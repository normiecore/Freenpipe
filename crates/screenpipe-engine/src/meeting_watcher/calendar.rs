// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Calendar events as the meeting detectors see them, and the rule that ties
//! a detected call to one of them.
//!
//! - **Exact conference identity only.** A detected meeting is tied to a
//!   calendar event only when a conference URL seen for the call and the
//!   event's `meetingUrl` name the same conference ([`conference_identity`]).
//!   Overlapping in time is not enough: such a match is reported as
//!   [`CalendarMatch::TimeOnly`] and leaves the meeting unbound and untitled.
//!   The binding itself goes through
//!   `DatabaseManager::bind_meeting_calendar_event`, which only fills an empty
//!   `calendar_event_id` and refuses an event another meeting already owns.
//! - **Excluded events are dropped on arrival.** Entries flagged
//!   `excludeFromMeetingDetection` (declined, marked free, cancelled), all-day
//!   entries and entries whose times do not parse never reach the detectors,
//!   so they neither bind nor keep a meeting alive.
//! - **One snapshot per publisher.** Each publisher on the `calendar_events`
//!   topic (OS calendar, Google, ICS feeds) sends its whole list every time.
//!   [`CalendarBook`] keeps the latest list of each source side by side
//!   instead of letting one publisher's list replace another's.
//!
//! Everything here works on data the user's own calendar connections already
//! published locally; nothing in this module makes a network request.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Event-bus topic the calendar publishers write to.
pub const CALENDAR_EVENTS_TOPIC: &str = "calendar_events";

/// Source name of the OS calendar publisher, also assumed for entries that
/// carry no `source`.
pub const NATIVE_SOURCE: &str = "native";

/// A source's list is forgotten when its publisher has been silent this long.
/// The slowest publisher (ICS feeds) refreshes every 10 minutes.
pub const SOURCE_SNAPSHOT_TTL: Duration = Duration::from_secs(15 * 60);

/// A call may be joined this long before its event starts, or run on this
/// long after it ends, and still be tied to it by its conference URL.
pub const JOIN_WINDOW_SLACK: chrono::Duration = chrono::Duration::minutes(15);

/// Value of [`CalendarMatch::method`] for a match on the conference URL.
pub const MATCH_METHOD_EXACT_CONFERENCE_URL: &str = "exact_conference_url";
/// Value of [`CalendarMatch::method`] for events that only overlap in time.
pub const MATCH_METHOD_TIME_ONLY: &str = "time_only";
/// Value of [`CalendarMatch::method`] when no event is around.
pub const MATCH_METHOD_NONE: &str = "none";

/// One calendar entry as published on [`CALENDAR_EVENTS_TOPIC`]
/// (`CalendarEventItem` in the desktop app). Every field is optional on the
/// wire so an older or newer publisher never makes a whole list unreadable.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEventSignal {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// RFC 3339.
    #[serde(default)]
    pub start: String,
    /// RFC 3339.
    #[serde(default)]
    pub end: String,
    #[serde(default)]
    pub attendees: Vec<String>,
    #[serde(default, alias = "is_all_day")]
    pub is_all_day: bool,
    #[serde(default, alias = "meeting_url")]
    pub meeting_url: Option<String>,
    /// Set by the publisher for entries the user is not attending as a
    /// meeting (declined, shown as free, cancelled).
    #[serde(default, alias = "exclude_from_meeting_detection")]
    pub exclude_from_meeting_detection: bool,
    /// `native`, `google`, `ics`; absent means `native`.
    #[serde(default)]
    pub source: Option<String>,
}

/// A calendar entry the detectors may use.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarEntry {
    pub id: String,
    pub title: String,
    pub attendees: Vec<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub source: String,
    /// [`conference_identity`] of the entry's meeting URL.
    pub conference: Option<String>,
}

impl CalendarEntry {
    /// The usable entry for `signal`, or `None` for entries the detectors
    /// ignore: flagged `excludeFromMeetingDetection`, all-day, or with times
    /// that do not parse or run backwards.
    pub fn from_signal(signal: CalendarEventSignal) -> Option<Self> {
        if signal.exclude_from_meeting_detection || signal.is_all_day {
            return None;
        }
        let start = parse_utc(&signal.start)?;
        let end = parse_utc(&signal.end)?;
        if end < start {
            return None;
        }
        let conference = signal.meeting_url.as_deref().and_then(conference_identity);
        Some(Self {
            id: signal.id.trim().to_string(),
            title: signal.title.trim().to_string(),
            attendees: signal
                .attendees
                .into_iter()
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty())
                .collect(),
            start,
            end,
            source: source_name(signal.source.as_deref()),
            conference,
        })
    }

    /// The entry is scheduled at `now`.
    pub fn is_happening(&self, now: DateTime<Utc>) -> bool {
        self.start <= now && now <= self.end
    }

    /// A call at `now` can belong to this entry: joined a little early, or
    /// still going a little after the scheduled end.
    pub fn in_join_window(&self, now: DateTime<Utc>) -> bool {
        self.start - JOIN_WINDOW_SLACK <= now && now <= self.end + JOIN_WINDOW_SLACK
    }

    /// The entry can be bound: it has an id and a conference URL.
    fn is_bindable(&self) -> bool {
        !self.id.is_empty() && self.conference.is_some()
    }
}

fn parse_utc(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn source_name(raw: Option<&str>) -> String {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| NATIVE_SOURCE.to_string())
}

/// Order in which sources win when two of them list the same conference:
/// ids that are stable across devices first.
fn source_rank(source: &str) -> u8 {
    match source {
        "google" => 0,
        "ics" => 1,
        NATIVE_SOURCE => 2,
        _ => 3,
    }
}

/// How a detected call relates to the calendar.
#[derive(Debug, Clone, PartialEq)]
pub enum CalendarMatch<'a> {
    /// An entry's conference URL names the detected call. Only this match
    /// binds the meeting to the entry.
    ExactConferenceUrl(&'a CalendarEntry),
    /// Entries are scheduled now, but none names the call's conference. The
    /// meeting stays unbound.
    TimeOnly { overlapping: usize },
    /// Nothing is scheduled around now.
    NoEvent,
}

impl CalendarMatch<'_> {
    /// Name of the matching method, as written to the logs.
    pub fn method(&self) -> &'static str {
        match self {
            CalendarMatch::ExactConferenceUrl(_) => MATCH_METHOD_EXACT_CONFERENCE_URL,
            CalendarMatch::TimeOnly { .. } => MATCH_METHOD_TIME_ONLY,
            CalendarMatch::NoEvent => MATCH_METHOD_NONE,
        }
    }

    /// The entry to bind, for an exact match only.
    pub fn bindable_entry(&self) -> Option<&CalendarEntry> {
        match self {
            CalendarMatch::ExactConferenceUrl(entry) => Some(entry),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct SourceSnapshot {
    refreshed: Instant,
    entries: Vec<CalendarEntry>,
}

/// The latest calendar list of every publisher, as one view.
#[derive(Debug, Clone, Default)]
pub struct CalendarBook {
    sources: HashMap<String, SourceSnapshot>,
}

impl CalendarBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take one publication from [`CALENDAR_EVENTS_TOPIC`].
    ///
    /// A publication is the full list of the sources it contains, so each of
    /// those sources is replaced as a whole (and a source whose entries were
    /// all excluded ends up empty). Only the OS calendar publisher announces
    /// "nothing scheduled" with an empty list; the other publishers stay
    /// silent instead, so an empty list clears the native source.
    pub fn ingest(&mut self, events: Vec<CalendarEventSignal>, now: Instant) {
        if events.is_empty() {
            self.sources.insert(
                NATIVE_SOURCE.to_string(),
                SourceSnapshot {
                    refreshed: now,
                    entries: Vec::new(),
                },
            );
            return;
        }
        let mut grouped: HashMap<String, Vec<CalendarEntry>> = HashMap::new();
        for signal in events {
            let source = source_name(signal.source.as_deref());
            let slot = grouped.entry(source).or_default();
            if let Some(entry) = CalendarEntry::from_signal(signal) {
                slot.push(entry);
            }
        }
        for (source, entries) in grouped {
            self.sources.insert(
                source,
                SourceSnapshot {
                    refreshed: now,
                    entries,
                },
            );
        }
    }

    /// Forget the lists of sources that stopped publishing.
    pub fn expire(&mut self, now: Instant) {
        self.sources.retain(|_, snapshot| {
            now.saturating_duration_since(snapshot.refreshed) < SOURCE_SNAPSHOT_TTL
        });
    }

    /// Every usable entry, all sources together.
    pub fn entries(&self) -> impl Iterator<Item = &CalendarEntry> {
        self.sources.values().flat_map(|s| s.entries.iter())
    }

    pub fn is_empty(&self) -> bool {
        self.entries().next().is_none()
    }

    /// Some entry is scheduled at `now`. Keeps an already detected meeting
    /// alive while its controls are hidden; it never starts or binds one.
    pub fn has_event_in_progress(&self, now: DateTime<Utc>) -> bool {
        self.entries().any(|e| e.is_happening(now))
    }

    /// Some entry with a conference URL could be bound at `now`. When false,
    /// looking up the call's conference URLs is pointless.
    pub fn has_bindable_entry(&self, now: DateTime<Utc>) -> bool {
        self.entries()
            .any(|e| e.is_bindable() && e.in_join_window(now))
    }

    /// Relate a call to the calendar. `call_conferences` are the
    /// [`conference_identity`] values seen for the call.
    ///
    /// With several exact matches (the same meeting listed by two
    /// publishers, or two entries sharing a recurring link), the entry
    /// scheduled at `now` wins, then the one starting closest to `now`, then
    /// the source with ids that are stable across devices, then the id.
    pub fn match_call(&self, call_conferences: &[String], now: DateTime<Utc>) -> CalendarMatch<'_> {
        let best = self
            .entries()
            .filter(|e| e.is_bindable() && e.in_join_window(now))
            .filter(|e| {
                e.conference
                    .as_ref()
                    .is_some_and(|c| call_conferences.iter().any(|call| call == c))
            })
            .min_by(|a, b| {
                let key = |e: &CalendarEntry| {
                    (
                        !e.is_happening(now),
                        (e.start - now).num_seconds().unsigned_abs(),
                        source_rank(&e.source),
                    )
                };
                key(a).cmp(&key(b)).then_with(|| a.id.cmp(&b.id))
            });
        if let Some(entry) = best {
            return CalendarMatch::ExactConferenceUrl(entry);
        }
        let overlapping = self.entries().filter(|e| e.is_happening(now)).count();
        if overlapping > 0 {
            CalendarMatch::TimeOnly { overlapping }
        } else {
            CalendarMatch::NoEvent
        }
    }
}

/// The conference a meeting URL points at, as `host/path`, or `None` when the
/// text is not an http(s) URL with a path.
///
/// Two URLs name the same conference when these strings are equal; there is
/// no substring or prefix matching. Normalisation covers what differs between
/// a calendar invite and the browser's address bar for the same room:
/// - the scheme (`http`, `https` or none, as browsers hide it) is dropped;
/// - the host is compared in lower case, without a leading `www.`, a
///   trailing dot or a default port;
/// - the query and fragment (`?pwd=`, `?authuser=`, `#success`) are dropped;
/// - trailing slashes are dropped and percent escapes are compared in upper
///   case. The rest of the path keeps its case.
///
/// A bare host (`https://meet.google.com/`) has no conference in it and
/// yields `None`.
pub fn conference_identity(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '<' | '>' | '"' | '\''));
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed.trim_start_matches('/'))
    };
    let url = url::Url::parse(&with_scheme).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    if host.is_empty() {
        return None;
    }
    let path = url.path().trim_end_matches('/');
    if path.is_empty() {
        return None;
    }
    Some(format!("{}{}", host, uppercase_percent_escapes(path)))
}

fn uppercase_percent_escapes(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = String::with_capacity(path.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            out.push('%');
            out.push(bytes[i + 1].to_ascii_uppercase() as char);
            out.push(bytes[i + 2].to_ascii_uppercase() as char);
            i += 3;
            continue;
        }
        // `path` comes from `Url::path`, which is ASCII (non-ASCII is
        // percent-encoded), so pushing bytes as chars is lossless.
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Conference identities among `urls` (browser URLs recently on screen) that
/// belong to the meeting service that was detected. `service_patterns` are
/// the detection profile's URL patterns and only decide which service a URL
/// belongs to; whether it is the scheduled call is decided by
/// [`CalendarBook::match_call`] on the exact identity.
pub fn call_conferences<'a>(
    urls: impl IntoIterator<Item = &'a str>,
    service_patterns: &[&str],
) -> Vec<String> {
    if service_patterns.is_empty() {
        return Vec::new();
    }
    let patterns: Vec<String> = service_patterns
        .iter()
        .map(|p| p.trim().to_ascii_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    let mut out: Vec<String> = Vec::new();
    for url in urls {
        let lower = url.to_ascii_lowercase();
        if !patterns.iter().any(|p| lower.contains(p.as_str())) {
            continue;
        }
        if let Some(identity) = conference_identity(url) {
            if !out.contains(&identity) {
                out.push(identity);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(
        id: &str,
        url: Option<&str>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> CalendarEventSignal {
        CalendarEventSignal {
            id: id.to_string(),
            title: format!("event {id}"),
            start: start.to_rfc3339(),
            end: end.to_rfc3339(),
            attendees: vec!["a@example.com".into()],
            meeting_url: url.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn identity_normalises_scheme_host_case_and_trailing_slashes() {
        let expected = Some("meet.google.com/abc-defg-hij".to_string());
        assert_eq!(
            conference_identity("https://meet.google.com/abc-defg-hij"),
            expected
        );
        assert_eq!(
            conference_identity("http://MEET.Google.com/abc-defg-hij/"),
            expected
        );
        assert_eq!(
            conference_identity("meet.google.com/abc-defg-hij//"),
            expected
        );
        assert_eq!(
            conference_identity("https://www.meet.google.com/abc-defg-hij"),
            expected
        );
        assert_eq!(
            conference_identity("https://meet.google.com:443/abc-defg-hij?authuser=1#x"),
            expected
        );
        assert_eq!(
            conference_identity(" <https://meet.google.com/abc-defg-hij> "),
            expected
        );
    }

    #[test]
    fn identity_keeps_path_and_rejects_non_urls() {
        assert_ne!(
            conference_identity("https://meet.google.com/abc-defg-hij"),
            conference_identity("https://meet.google.com/abc-defg-hik")
        );
        assert_eq!(conference_identity("https://meet.google.com/"), None);
        assert_eq!(conference_identity("zoommtg://zoom.us/join?confno=1"), None);
        assert_eq!(conference_identity("not a url"), None);
        assert_eq!(conference_identity(""), None);
        assert_eq!(
            conference_identity(
                "https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0"
            ),
            conference_identity(
                "https://teams.microsoft.com/l/meetup-join/19%3Ameeting_x%40thread.v2/0/"
            )
        );
    }

    #[test]
    fn identity_is_never_a_substring_match() {
        let call = conference_identity("https://zoom.us/j/12345").unwrap();
        assert_ne!(
            Some(call.clone()),
            conference_identity("https://zoom.us/j/123456")
        );
        assert_ne!(
            Some(call.clone()),
            conference_identity("https://zoom.us/j/1234")
        );
        assert_ne!(
            Some(call),
            conference_identity("https://us02web.zoom.us/j/12345")
        );
    }

    #[test]
    fn excluded_all_day_and_broken_entries_are_dropped() {
        let now = Utc::now();
        let mut excluded = signal("x", Some("https://meet.google.com/aaa-bbbb-ccc"), now, now);
        excluded.exclude_from_meeting_detection = true;
        assert_eq!(CalendarEntry::from_signal(excluded), None);

        let mut all_day = signal("d", None, now, now);
        all_day.is_all_day = true;
        assert_eq!(CalendarEntry::from_signal(all_day), None);

        let mut broken = signal("b", None, now, now);
        broken.start = "yesterday".into();
        assert_eq!(CalendarEntry::from_signal(broken), None);

        let backwards = signal("r", None, now, now - chrono::Duration::minutes(5));
        assert_eq!(CalendarEntry::from_signal(backwards), None);
    }

    #[test]
    fn wire_names_from_the_app_are_read() {
        let raw = serde_json::json!([{
            "id": "evt-1",
            "title": "Planning",
            "start": "2026-10-26T10:00:00Z",
            "end": "2026-10-26T11:00:00Z",
            "startDisplay": "10:00 AM",
            "attendees": ["x@example.com"],
            "meetingUrl": "https://meet.google.com/abc-defg-hij",
            "isAllDay": false,
            "excludeFromMeetingDetection": true,
            "source": "google"
        }]);
        let parsed: Vec<CalendarEventSignal> = serde_json::from_value(raw).unwrap();
        assert_eq!(
            parsed[0].meeting_url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert!(parsed[0].exclude_from_meeting_detection);
        assert_eq!(parsed[0].source.as_deref(), Some("google"));

        // Older publishers send neither the flag nor a source.
        let old: Vec<CalendarEventSignal> = serde_json::from_value(serde_json::json!([
            {"title": "x", "start": "2026-10-26T10:00:00Z", "end": "2026-10-26T11:00:00Z"}
        ]))
        .unwrap();
        assert!(!old[0].exclude_from_meeting_detection);
        assert_eq!(old[0].source, None);
    }

    #[test]
    fn publishers_keep_separate_lists() {
        let now = Utc::now();
        let t = Instant::now();
        let mut book = CalendarBook::new();
        let mut google = signal(
            "g1",
            None,
            now - chrono::Duration::minutes(5),
            now + chrono::Duration::minutes(25),
        );
        google.source = Some("google".into());
        book.ingest(vec![google], t);
        let native = signal(
            "n1",
            None,
            now - chrono::Duration::minutes(5),
            now + chrono::Duration::minutes(25),
        );
        book.ingest(vec![native], t);
        assert_eq!(book.entries().count(), 2);

        // The OS calendar publishing "nothing" clears its own list only.
        book.ingest(Vec::new(), t);
        let ids: Vec<_> = book.entries().map(|e| e.id.clone()).collect();
        assert_eq!(ids, vec!["g1".to_string()]);

        // A silent publisher's list ages out.
        book.expire(t + SOURCE_SNAPSHOT_TTL + Duration::from_secs(1));
        assert!(book.is_empty());
    }

    #[test]
    fn exact_conference_url_wins_and_time_only_is_reported() {
        let now = Utc::now();
        let mut book = CalendarBook::new();
        book.ingest(
            vec![
                signal(
                    "standup",
                    Some("https://meet.google.com/abc-defg-hij"),
                    now - chrono::Duration::minutes(5),
                    now + chrono::Duration::minutes(25),
                ),
                signal(
                    "lunch",
                    None,
                    now - chrono::Duration::minutes(5),
                    now + chrono::Duration::minutes(55),
                ),
            ],
            Instant::now(),
        );

        let call = vec![conference_identity("meet.google.com/abc-defg-hij?authuser=0").unwrap()];
        let m = book.match_call(&call, now);
        assert_eq!(m.method(), MATCH_METHOD_EXACT_CONFERENCE_URL);
        assert_eq!(m.bindable_entry().map(|e| e.id.as_str()), Some("standup"));

        let other_call = vec![conference_identity("https://meet.google.com/zzz-zzzz-zzz").unwrap()];
        let m = book.match_call(&other_call, now);
        assert_eq!(m, CalendarMatch::TimeOnly { overlapping: 2 });
        assert_eq!(m.bindable_entry(), None);

        let m = book.match_call(&[], now);
        assert_eq!(m.method(), MATCH_METHOD_TIME_ONLY);

        let later = now + chrono::Duration::hours(3);
        assert_eq!(book.match_call(&call, later), CalendarMatch::NoEvent);
    }

    #[test]
    fn early_join_binds_and_overlapping_matches_prefer_current_event() {
        let now = Utc::now();
        let url = "https://zoom.us/j/999";
        let mut book = CalendarBook::new();
        let mut google_copy = signal(
            "g-next",
            Some(url),
            now + chrono::Duration::minutes(10),
            now + chrono::Duration::minutes(40),
        );
        google_copy.source = Some("google".into());
        book.ingest(vec![google_copy], Instant::now());
        let call = vec![conference_identity(url).unwrap()];
        // Joined ten minutes early.
        assert_eq!(
            book.match_call(&call, now)
                .bindable_entry()
                .map(|e| e.id.as_str()),
            Some("g-next")
        );

        // A current entry with the same recurring link wins over the next one.
        book.ingest(
            vec![signal(
                "n-now",
                Some(url),
                now - chrono::Duration::minutes(20),
                now + chrono::Duration::minutes(5),
            )],
            Instant::now(),
        );
        assert_eq!(
            book.match_call(&call, now)
                .bindable_entry()
                .map(|e| e.id.as_str()),
            Some("n-now")
        );

        // Far outside the join window, the link alone binds nothing.
        assert_eq!(
            book.match_call(&call, now + chrono::Duration::hours(2)),
            CalendarMatch::NoEvent
        );
    }

    #[test]
    fn excluded_entry_never_matches_or_keeps_alive() {
        let now = Utc::now();
        let mut declined = signal(
            "declined",
            Some("https://meet.google.com/abc-defg-hij"),
            now - chrono::Duration::minutes(5),
            now + chrono::Duration::minutes(25),
        );
        declined.exclude_from_meeting_detection = true;
        let mut book = CalendarBook::new();
        book.ingest(vec![declined], Instant::now());
        let call = vec![conference_identity("https://meet.google.com/abc-defg-hij").unwrap()];
        assert_eq!(book.match_call(&call, now), CalendarMatch::NoEvent);
        assert!(!book.has_event_in_progress(now));
        assert!(!book.has_bindable_entry(now));
    }

    #[test]
    fn call_conferences_keep_only_the_detected_service() {
        let urls = [
            "https://meet.google.com/abc-defg-hij?authuser=0",
            "meet.google.com/abc-defg-hij",
            "https://zoom.us/j/12345",
            "https://meet.google.com/",
        ];
        let meet = call_conferences(urls.iter().copied(), &["meet.google.com"]);
        assert_eq!(meet, vec!["meet.google.com/abc-defg-hij".to_string()]);
        let zoom = call_conferences(urls.iter().copied(), &["zoom.us/j", "zoom.us/wc"]);
        assert_eq!(zoom, vec!["zoom.us/j/12345".to_string()]);
        assert!(call_conferences(urls.iter().copied(), &[]).is_empty());
    }
}
