// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Meeting queries added after the base: calendar binding, rejoin and summaries.
//!
//! - **Calendar binding** (`meetings.calendar_event_id`, partial unique index
//!   `idx_meetings_calendar_event_id`): one calendar event describes at most one
//!   meeting. [`DatabaseManager::bind_meeting_calendar_event`] only fills an
//!   empty column and reports an event that another meeting already owns as
//!   [`CalendarBinding::OwnedElsewhere`] instead of failing.
//! - **Room change**: [`DatabaseManager::switch_meeting_room`] ends the open
//!   meeting with `end_reason = 'room_changed'` and opens its successor in one
//!   write transaction, so the single-open invariant never sees two open rows.
//!   The rejoin lookup (`find_recent_meeting_for_app` in `db.rs`) skips rows
//!   that ended this way.
//! - **Summaries**: the `## Summary` section of a meeting note is written by
//!   [`DatabaseManager::save_meeting_summary`] (read, merge and write inside one
//!   transaction so a concurrent note edit is never lost), and the state of
//!   the summary pipe for a meeting is read from `pipe_executions` and
//!   `pipe_event_runs` by [`DatabaseManager::meeting_summary_run_snapshot`].
//!
//! The note and summary helpers are pure functions so the engine route, the
//! summary finalizer and tests share one implementation.

use chrono::{DateTime, Utc};
use sqlx::{Error as SqlxError, FromRow};

use crate::{DatabaseManager, MeetingRecord};

/// The meeting ended because the user moved the same audio session to a
/// different call and chose to start a new note. Like `explicit_stop`, the
/// rejoin lookup never reopens such a meeting.
pub const MEETING_END_REASON_ROOM_CHANGED: &str = "room_changed";

/// `detection_source` of the meeting opened by a room change.
pub const MEETING_DETECTION_SOURCE_ROOM_CHANGE: &str = "audio_process_room_change";

/// Heading of the summary section inside a meeting note.
pub const MEETING_SUMMARY_HEADING: &str = "## Summary";

/// Event the summary pipe is triggered by, as recorded in
/// `pipe_executions.trigger_event` and `pipe_event_runs.event_name`.
pub const MEETING_SUMMARY_TRIGGER_EVENT: &str = "meeting_ended";

/// A claim in `pipe_event_runs` without an execution row counts as a queued
/// run only for this long. The scheduler creates the execution right after
/// claiming, so an older claim without one belongs to a run that never
/// started (or whose history was pruned).
pub const UNSTARTED_CLAIM_WINDOW_SECS: i64 = 30 * 60;

// ── Calendar binding ─────────────────────────────────────────────────────

/// Outcome of attaching a calendar event to a meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalendarBinding {
    /// The event is now attached to this meeting.
    Bound,
    /// The meeting already carried this event id; nothing changed.
    AlreadyBound,
    /// Another meeting owns the event. `other_meeting_id` is known unless the
    /// conflict only surfaced as a unique-index violation.
    OwnedElsewhere { other_meeting_id: Option<i64> },
    /// The meeting is attached to a different event, which is kept.
    MeetingHasOtherEvent { existing: String },
    /// No meeting with this id.
    MeetingMissing,
}

impl CalendarBinding {
    /// True when the meeting carries the requested event after the call.
    pub fn is_bound(&self) -> bool {
        matches!(self, CalendarBinding::Bound | CalendarBinding::AlreadyBound)
    }
}

/// Trim a client-supplied calendar event id; blank means "no event".
pub fn normalize_calendar_event_id(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|s| !s.is_empty())
}

/// True when `e` is SQLite rejecting a second meeting for the same calendar
/// event (`idx_meetings_calendar_event_id`). Writers treat this as "already
/// bound elsewhere", never as a fatal error.
pub fn is_calendar_event_conflict(e: &SqlxError) -> bool {
    let SqlxError::Database(db_err) = e else {
        return false;
    };
    if !db_err.is_unique_violation() {
        return false;
    }
    let msg = db_err.message();
    msg.contains("meetings.calendar_event_id") || msg.contains("idx_meetings_calendar_event_id")
}

// ── Room change ──────────────────────────────────────────────────────────

/// Outcome of [`DatabaseManager::switch_meeting_room`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomSwitch {
    /// `ended_id` was closed with `room_changed`; `new_id` is the open successor.
    Switched {
        ended_id: i64,
        new_id: i64,
        meeting_app: String,
        switched_at: String,
    },
    /// The meeting exists but has already ended.
    NotActive,
    /// No meeting with this id.
    Missing,
}

// ── Summary text ─────────────────────────────────────────────────────────

/// Why a summary was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryRejection {
    /// Nothing but whitespace (or only the section heading).
    Empty,
    /// The text reports that no summary could be made (an apology, an error
    /// or "nothing to summarize") instead of describing the meeting.
    FailureText,
}

/// Apologies: the assistant speaking about itself. Matched at the start of
/// the first sentence, on a word boundary.
const APOLOGY_OPENERS: &[&str] = &[
    "sorry",
    "i'm sorry",
    "i am sorry",
    "apologies",
    "my apologies",
    "i apologize",
    "i apologise",
];

/// First-person reports of not managing something ("I couldn't ...").
/// Meeting summaries are written in the third person, so these open a note
/// about the summary run rather than about the meeting.
const SELF_INABILITY_OPENERS: &[&str] = &[
    "i couldn't",
    "i could not",
    "i can't",
    "i cannot",
    "i was unable",
    "i was not able",
    "i wasn't able",
    "i'm unable",
    "i am unable",
    "i'm not able",
    "i am not able",
];

/// An error label at the very start ("Error: ...").
const ERROR_LABEL: &str = "error:";

/// Subjectless openers that report the run itself failing. They count only
/// when a task verb follows ("Unable to summarize", "Failed to fetch the
/// transcript"), so "Failed to ship the audio feature, ..." is a summary.
const TASK_FAILURE_OPENERS: &[&str] = &["unable to", "failed to"];

/// Verbs that follow [`TASK_FAILURE_OPENERS`] in a failure notice.
const TASK_VERBS: &[&str] = &[
    "summarize",
    "summarise",
    "generate",
    "produce",
    "create",
    "write",
    "access",
    "find",
    "fetch",
    "load",
    "read",
    "retrieve",
    "process",
    "transcribe",
];

/// "Unfortunately" also opens ordinary sentences about the meeting
/// ("Unfortunately the launch slipped", "Unfortunately, the recording of the
/// demo failed, so ..."). It counts only in a sentence about producing the
/// summary, or in a short one-clause text that says the input is missing.
const SOFT_OPENER: &str = "unfortunately";

/// Words naming what a summary is made from (substring match).
const INPUT_WORDS: &[&str] = &["transcript", "recording", "audio", "speech", "captured"];

/// Words that say something was missing or did not work (substring match).
/// Used only after an apology, a first-person opener or an error label.
const LACK_WORDS: &[&str] = &[
    "no ",
    "not",
    "n't",
    "unable",
    "nothing",
    "without",
    "missing",
    "empty",
    "insufficient",
    "failed",
    "error",
];

/// Words that say the summary could not be produced (whole words).
const INABILITY_WORDS: &[&str] = &[
    "unable",
    "couldn't",
    "could not",
    "can't",
    "cannot",
    "not able",
    "wasn't able",
    "not possible",
    "impossible",
    "no summary",
];

/// Words that say the input is missing altogether (whole words).
const MISSING_WORDS: &[&str] = &[
    "no",
    "none",
    "nothing",
    "empty",
    "missing",
    "unavailable",
    "not available",
    "wasn't available",
    "not captured",
    "wasn't captured",
    "not recorded",
    "wasn't recorded",
    "not found",
];

/// Short answers that are nothing but a placeholder.
const PLACEHOLDER_ANSWERS: &[&str] = &["none", "n/a", "na", "nothing", "no summary", "no content"];

/// Short answers that open by saying there was nothing to summarize.
const NOTHING_TO_SUMMARIZE: &[&str] = &[
    "no transcript",
    "no audio",
    "no speech",
    "no summary",
    "no content",
    "no meeting content",
    "nothing to summarize",
    "nothing to summarise",
    "nothing was said",
    "empty transcript",
];

/// Short answers opening with "not enough" count only when they name the input.
const NOT_ENOUGH_WORDS: &[&str] = &[
    "summar",
    "transcript",
    "audio",
    "speech",
    "content",
    "information",
    "context",
];

/// Only texts up to this many characters are checked against the
/// "nothing to summarize" openers; a real summary that merely starts with
/// "No audio issues..." goes on for longer.
const SHORT_TEXT_CHARS: usize = 160;

/// Words that join a second clause; a notice that has one says more than
/// "the input is missing".
const CLAUSE_JOINERS: &[&str] = &[
    "so", "but", "and", "which", "while", "because", "then", "although", "though", "yet",
];

/// Length of the "first sentence" window used by the failure check.
const FIRST_SENTENCE_CHARS: usize = 240;

/// Prepare a summary for saving: trim it, drop a leading summary heading the
/// caller may have included, and demote level-1/2 headings inside it to
/// level 3 so the section stays one block that a later save replaces whole.
pub fn clean_summary_body(summary: &str) -> String {
    let mut lines: Vec<&str> = summary.trim().lines().collect();
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    if lines
        .first()
        .and_then(|l| heading_of(l))
        .is_some_and(|(level, text)| level <= 3 && is_summary_title(text))
    {
        lines.remove(0);
    }
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut fence: Option<char> = None;
    for line in lines {
        if let Some(marker) = fence_marker(line) {
            fence = toggle_fence(fence, marker);
            out.push(line.to_string());
            continue;
        }
        match heading_of(line) {
            Some((level, text)) if fence.is_none() && level <= 2 => {
                out.push(format!("### {}", text));
            }
            _ => out.push(line.to_string()),
        }
    }
    out.join("\n").trim().to_string()
}

/// Check a cleaned summary before it is saved. `None` means it may be saved.
pub fn summary_rejection(summary: &str) -> Option<SummaryRejection> {
    let text = summary.trim();
    if text.is_empty() {
        return Some(SummaryRejection::Empty);
    }
    let lower = normalize_for_matching(text);
    let stripped = strip_leading_markup(&lower);
    if stripped.is_empty() {
        return Some(SummaryRejection::Empty);
    }

    if stripped.chars().count() <= SHORT_TEXT_CHARS {
        let bare = stripped.trim_end_matches(['.', '!', ' ']);
        if PLACEHOLDER_ANSWERS.contains(&bare)
            || NOTHING_TO_SUMMARIZE
                .iter()
                .any(|p| starts_with_word(stripped, p))
            || (starts_with_word(stripped, "not enough")
                && NOT_ENOUGH_WORDS.iter().any(|w| stripped.contains(w)))
        {
            return Some(SummaryRejection::FailureText);
        }
    }

    let first = first_sentence(stripped);
    if is_self_report_of_failure(first)
        || is_report_that_summary_was_not_made(first)
        || (stripped.chars().count() <= SHORT_TEXT_CHARS && is_missing_input_notice(stripped))
    {
        return Some(SummaryRejection::FailureText);
    }
    None
}

/// The first sentence is the assistant talking about itself (an apology, a
/// first-person "I couldn't", an "Error:" label, or "Unfortunately, I ...")
/// and says the summary or its input was missing or did not work: "I'm
/// sorry, but there is no transcript", "Error: failed to read the audio".
fn is_self_report_of_failure(sentence: &str) -> bool {
    let opens_as_self = |s: &str| {
        APOLOGY_OPENERS
            .iter()
            .chain(SELF_INABILITY_OPENERS)
            .any(|o| starts_with_word(s, o))
    };
    let speaks_as_assistant = opens_as_self(sentence)
        || sentence.starts_with(ERROR_LABEL)
        || sentence
            .strip_prefix(SOFT_OPENER)
            .map(|rest| rest.trim_start_matches([',', ' ', '-']))
            .is_some_and(|rest| {
                SELF_INABILITY_OPENERS
                    .iter()
                    .any(|o| starts_with_word(rest, o))
            });
    speaks_as_assistant
        && (sentence.contains("summar") || mentions_any(sentence, INPUT_WORDS))
        && mentions_any(sentence, LACK_WORDS)
}

/// The first sentence says, without an "I", that the summary was not made:
/// "Unfortunately, no summary could be produced", "Unable to summarize this
/// call", "Failed to fetch the transcript".
fn is_report_that_summary_was_not_made(sentence: &str) -> bool {
    if starts_with_word(sentence, SOFT_OPENER) {
        return sentence.contains("summar")
            && INABILITY_WORDS.iter().any(|w| has_word(sentence, w));
    }
    TASK_FAILURE_OPENERS.iter().any(|opener| {
        sentence
            .strip_prefix(opener)
            .map(str::trim_start)
            .is_some_and(|rest| TASK_VERBS.iter().any(|v| starts_with_word(rest, v)))
    }) && (sentence.contains("summar") || mentions_any(sentence, INPUT_WORDS))
}

/// A short text that is one clause opening with "unfortunately" and only says
/// the input is missing: "Unfortunately, no transcript was available."
/// Anything after a full stop or semicolon ("...; the second half covered
/// the roadmap"), a further comma or a joining word ("..., so the team will
/// redo it") makes it a summary: refusing a real summary loses it, while
/// saving a rare failure notice only costs the user one edit.
fn is_missing_input_notice(text: &str) -> bool {
    let Some(rest) = text.strip_prefix(SOFT_OPENER) else {
        return false;
    };
    let rest = rest.trim_start_matches([',', ' ', '-']);
    starts_with_word(text, SOFT_OPENER)
        && is_single_clause(text)
        && !rest.contains(',')
        && !CLAUSE_JOINERS.iter().any(|w| has_word(rest, w))
        && mentions_any(text, INPUT_WORDS)
        && MISSING_WORDS.iter().any(|w| has_word(text, w))
}

/// Replace the last `## Summary` section of `note` with `summary` (a cleaned
/// body), or append a new section when there is none. The section runs from
/// its heading to the next heading of level 1 or 2 (outside code fences) or
/// to the end of the note; everything else in the note is kept as written.
pub fn merge_summary_into_note(note: Option<&str>, summary: &str) -> String {
    let section = format!("{}\n{}", MEETING_SUMMARY_HEADING, summary.trim());
    let note = note.unwrap_or("");
    if note.trim().is_empty() {
        return section;
    }

    let lines: Vec<&str> = note.split('\n').collect();
    // (line index, level, is a summary heading) of every heading outside code.
    let mut headings: Vec<(usize, usize, bool)> = Vec::new();
    let mut fence: Option<char> = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some(marker) = fence_marker(line) {
            fence = toggle_fence(fence, marker);
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if let Some((level, text)) = heading_of(line) {
            headings.push((i, level, level == 2 && is_summary_title(text)));
        }
    }

    let Some(pos) = headings.iter().rposition(|(_, _, is_summary)| *is_summary) else {
        return format!("{}\n\n{}", note.trim_end(), section);
    };
    let start = headings[pos].0;
    let end = headings[pos + 1..]
        .iter()
        .find(|(_, level, _)| *level <= 2)
        .map(|(i, _, _)| *i)
        .unwrap_or(lines.len());

    let before = lines[..start].join("\n");
    let after = lines[end..].join("\n");
    let mut out = String::new();
    if !before.trim().is_empty() {
        out.push_str(before.trim_end());
        out.push_str("\n\n");
    }
    out.push_str(&section);
    if !after.trim().is_empty() {
        out.push_str("\n\n");
        out.push_str(&after);
    }
    out
}

/// ATX heading level and text of `line` (up to three leading spaces).
fn heading_of(line: &str) -> Option<(usize, &str)> {
    let line = line.trim_end_matches('\r');
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let level = rest.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let after = &rest[level..];
    if !after.is_empty() && !after.starts_with([' ', '\t']) {
        return None;
    }
    let text = after.trim().trim_end_matches('#').trim();
    Some((level, text))
}

fn is_summary_title(text: &str) -> bool {
    text.trim_end_matches(':')
        .trim()
        .eq_ignore_ascii_case("summary")
}

/// Opening or closing marker of a fenced code block (``` or ~~~).
fn fence_marker(line: &str) -> Option<char> {
    let t = line.trim_start();
    if t.starts_with("```") {
        Some('`')
    } else if t.starts_with("~~~") {
        Some('~')
    } else {
        None
    }
}

/// A fence closes only with the marker that opened it.
fn toggle_fence(open: Option<char>, marker: char) -> Option<char> {
    match open {
        Some(current) if current == marker => None,
        Some(current) => Some(current),
        None => Some(marker),
    }
}

fn normalize_for_matching(text: &str) -> String {
    text.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'")
}

/// Skip markdown decoration (headings, bullets, quotes, emphasis) at the start.
fn strip_leading_markup(text: &str) -> &str {
    text.trim_start_matches(|c: char| {
        c.is_whitespace() || matches!(c, '#' | '*' | '_' | '-' | '>' | '"' | '\'' | '`')
    })
}

fn first_sentence(text: &str) -> &str {
    let mut end = text.len();
    for (count, (i, c)) in text.char_indices().enumerate() {
        if count >= FIRST_SENTENCE_CHARS || matches!(c, '.' | '!' | '?' | '\n') {
            end = i;
            break;
        }
    }
    &text[..end]
}

/// Nothing but punctuation and spaces follows the first full stop,
/// question mark, exclamation mark, semicolon or line break.
fn is_single_clause(text: &str) -> bool {
    match text.find(['.', '!', '?', ';', '\n']) {
        Some(i) => text[i..].chars().all(|c| !c.is_alphanumeric()),
        None => true,
    }
}

fn mentions_any(text: &str, words: &[&str]) -> bool {
    words.iter().any(|w| text.contains(w))
}

/// `word` occurs in `text` with no letter or digit right before or after it.
fn has_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        before.is_none_or(|c| !c.is_alphanumeric()) && after.is_none_or(|c| !c.is_alphanumeric())
    })
}

/// `text` starts with `prefix`, followed by a non-alphanumeric character or the end.
fn starts_with_word(text: &str, prefix: &str) -> bool {
    match text.strip_prefix(prefix) {
        Some(rest) => rest.chars().next().is_none_or(|c| !c.is_alphanumeric()),
        None => false,
    }
}

// ── Summary run state ────────────────────────────────────────────────────

/// The newest summary-pipe execution recorded for a meeting.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct MeetingSummaryExecution {
    pub id: i64,
    pub status: String,
    pub error_type: Option<String>,
    pub error_message: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

/// What the database knows about summary runs for one meeting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeetingSummaryRunSnapshot {
    /// Newest matching execution started at or after `not_before`.
    pub latest_execution: Option<MeetingSummaryExecution>,
    /// A recent `pipe_event_runs` claim exists whose run has no execution yet.
    pub has_unstarted_claim: bool,
}

/// Summarisation state reported by `GET /meetings/:id/summary-status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeetingSummaryState {
    /// No run for this meeting.
    Idle,
    /// A run is claimed or queued but has not started.
    Pending,
    /// The pipe is running.
    Running,
    /// The newest run completed.
    Ready,
    /// The newest run failed, timed out or was cancelled.
    Failed,
}

impl MeetingSummaryState {
    pub fn as_str(self) -> &'static str {
        match self {
            MeetingSummaryState::Idle => "idle",
            MeetingSummaryState::Pending => "pending",
            MeetingSummaryState::Running => "running",
            MeetingSummaryState::Ready => "ready",
            MeetingSummaryState::Failed => "failed",
        }
    }
}

impl MeetingSummaryRunSnapshot {
    /// Derive the reported state. An unstarted claim wins: it is a newer run
    /// than any execution that already exists.
    pub fn state(&self) -> MeetingSummaryState {
        if self.has_unstarted_claim {
            return MeetingSummaryState::Pending;
        }
        match self.latest_execution.as_ref().map(|e| e.status.as_str()) {
            None => MeetingSummaryState::Idle,
            Some("queued") => MeetingSummaryState::Pending,
            Some("running") => MeetingSummaryState::Running,
            Some("completed") => MeetingSummaryState::Ready,
            Some(_) => MeetingSummaryState::Failed,
        }
    }
}

/// Parse an RFC 3339 `not_before` bound into the UTC form bound in queries.
pub fn parse_not_before(raw: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc).to_rfc3339())
}

/// "Now" in the two shapes the base writes: `meeting_start` as `...Z` with
/// milliseconds (`insert_meeting`), `meeting_end` normalised by `to_rfc3339`
/// (`end_meeting`). Both name the same instant.
fn now_meeting_timestamps() -> (String, String) {
    let start = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let end = DateTime::parse_from_rfc3339(&start)
        .map(|dt| dt.with_timezone(&Utc).to_rfc3339())
        .unwrap_or_else(|_| start.clone());
    (start, end)
}

impl DatabaseManager {
    /// The calendar event attached to a meeting (`Ok(None)` when the meeting
    /// has none, `Err(RowNotFound)` when the meeting does not exist).
    pub async fn get_meeting_calendar_event_id(
        &self,
        meeting_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT calendar_event_id FROM meetings WHERE id = ?1")
                .bind(meeting_id)
                .fetch_optional(&self.pool)
                .await?;
        match row {
            Some((value,)) => Ok(value.filter(|v| !v.trim().is_empty())),
            None => Err(SqlxError::RowNotFound),
        }
    }

    /// Attach `event_id` to `meeting_id` when the meeting has no event yet and
    /// no other meeting owns it. Never overwrites an existing binding and never
    /// fails because of a conflict; the outcome says what happened.
    pub async fn bind_meeting_calendar_event(
        &self,
        meeting_id: i64,
        event_id: &str,
    ) -> Result<CalendarBinding, SqlxError> {
        let event_id = event_id.trim();
        if event_id.is_empty() {
            return Err(SqlxError::Protocol(
                "calendar_event_id must not be blank".to_string(),
            ));
        }
        let mut tx = self.begin_immediate_with_retry().await?;

        let current: Option<(Option<String>,)> =
            sqlx::query_as("SELECT calendar_event_id FROM meetings WHERE id = ?1")
                .bind(meeting_id)
                .fetch_optional(&mut **tx.conn())
                .await?;
        let Some((current,)) = current else {
            tx.rollback().await?;
            return Ok(CalendarBinding::MeetingMissing);
        };
        if let Some(existing) = current.filter(|v| !v.trim().is_empty()) {
            tx.rollback().await?;
            return Ok(if existing == event_id {
                CalendarBinding::AlreadyBound
            } else {
                CalendarBinding::MeetingHasOtherEvent { existing }
            });
        }

        // The extra predicates let SQLite use the partial unique index.
        let owner: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM meetings \
             WHERE calendar_event_id = ?1 \
               AND calendar_event_id IS NOT NULL AND calendar_event_id != '' \
               AND id != ?2 \
             LIMIT 1",
        )
        .bind(event_id)
        .bind(meeting_id)
        .fetch_optional(&mut **tx.conn())
        .await?;
        if let Some((other,)) = owner {
            tx.rollback().await?;
            return Ok(CalendarBinding::OwnedElsewhere {
                other_meeting_id: Some(other),
            });
        }

        let result = sqlx::query(
            "UPDATE meetings SET calendar_event_id = ?1 \
             WHERE id = ?2 AND (calendar_event_id IS NULL OR calendar_event_id = '')",
        )
        .bind(event_id)
        .bind(meeting_id)
        .execute(&mut **tx.conn())
        .await;
        match result {
            Ok(done) if done.rows_affected() == 1 => {
                tx.commit().await?;
                Ok(CalendarBinding::Bound)
            }
            Ok(_) => {
                // Not expected under the write lock; report what the row holds.
                tx.rollback().await?;
                match self.get_meeting_calendar_event_id(meeting_id).await {
                    Ok(Some(existing)) if existing == event_id => Ok(CalendarBinding::AlreadyBound),
                    Ok(Some(existing)) => Ok(CalendarBinding::MeetingHasOtherEvent { existing }),
                    Ok(None) | Err(SqlxError::RowNotFound) => Ok(CalendarBinding::MeetingMissing),
                    Err(e) => Err(e),
                }
            }
            Err(e) if is_calendar_event_conflict(&e) => {
                tx.rollback().await?;
                Ok(CalendarBinding::OwnedElsewhere {
                    other_meeting_id: None,
                })
            }
            Err(e) => Err(e),
        }
    }

    /// End the open meeting `meeting_id` with `room_changed` and open a new
    /// meeting for the same app (`detection_source =
    /// 'audio_process_room_change'`), both in one write transaction. The new
    /// meeting starts at the instant the old one ends and carries no title,
    /// attendees, note or calendar event: it is a different call.
    pub async fn switch_meeting_room(&self, meeting_id: i64) -> Result<RoomSwitch, SqlxError> {
        let mut tx = self.begin_immediate_with_retry().await?;
        let row: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT meeting_app, meeting_end FROM meetings WHERE id = ?1")
                .bind(meeting_id)
                .fetch_optional(&mut **tx.conn())
                .await?;
        let Some((meeting_app, meeting_end)) = row else {
            tx.rollback().await?;
            return Ok(RoomSwitch::Missing);
        };
        if meeting_end.is_some() {
            tx.rollback().await?;
            return Ok(RoomSwitch::NotActive);
        }

        let (start, end) = now_meeting_timestamps();
        sqlx::query(
            "UPDATE meetings SET meeting_end = ?1, end_reason = ?2 \
             WHERE id = ?3 AND meeting_end IS NULL",
        )
        .bind(&end)
        .bind(MEETING_END_REASON_ROOM_CHANGED)
        .bind(meeting_id)
        .execute(&mut **tx.conn())
        .await?;
        let new_id = sqlx::query(
            "INSERT INTO meetings (meeting_start, meeting_app, detection_source) \
             VALUES (?1, ?2, ?3)",
        )
        .bind(&start)
        .bind(&meeting_app)
        .bind(MEETING_DETECTION_SOURCE_ROOM_CHANGE)
        .execute(&mut **tx.conn())
        .await?
        .last_insert_rowid();
        tx.commit().await?;

        Ok(RoomSwitch::Switched {
            ended_id: meeting_id,
            new_id,
            meeting_app,
            switched_at: end,
        })
    }

    /// Write `summary` (already cleaned and checked, see [`clean_summary_body`]
    /// and [`summary_rejection`]) into the meeting note's `## Summary` section
    /// and set `title` when one is given. Returns `Ok(None)` when the meeting
    /// does not exist and `Err(RowNotFound)` when it vanished right after the
    /// update.
    pub async fn save_meeting_summary(
        &self,
        meeting_id: i64,
        summary: &str,
        title: Option<&str>,
    ) -> Result<Option<MeetingRecord>, SqlxError> {
        let title = title.map(str::trim).filter(|t| !t.is_empty());
        let mut tx = self.begin_immediate_with_retry().await?;
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT note FROM meetings WHERE id = ?1")
                .bind(meeting_id)
                .fetch_optional(&mut **tx.conn())
                .await?;
        let Some((note,)) = row else {
            tx.rollback().await?;
            return Ok(None);
        };
        let merged = merge_summary_into_note(note.as_deref(), summary);
        match title {
            Some(title) => {
                sqlx::query("UPDATE meetings SET note = ?1, title = ?2 WHERE id = ?3")
                    .bind(&merged)
                    .bind(title)
                    .bind(meeting_id)
                    .execute(&mut **tx.conn())
                    .await?;
            }
            None => {
                sqlx::query("UPDATE meetings SET note = ?1 WHERE id = ?2")
                    .bind(&merged)
                    .bind(meeting_id)
                    .execute(&mut **tx.conn())
                    .await?;
            }
        }
        tx.commit().await?;
        self.get_meeting_by_id(meeting_id).await.map(Some)
    }

    /// Summary-pipe runs for one meeting: the newest `pipe_executions` row of
    /// `pipe` triggered by `meeting_ended` for this meeting (trigger key = the
    /// meeting id, or `<id>:<suffix>` for recovery runs) started at or after
    /// `not_before`, and whether a recent `pipe_event_runs` claim for it has
    /// no execution yet. `not_before` must be RFC 3339 (see [`parse_not_before`]).
    pub async fn meeting_summary_run_snapshot(
        &self,
        meeting_id: i64,
        pipe: &str,
        not_before: Option<&str>,
    ) -> Result<MeetingSummaryRunSnapshot, SqlxError> {
        let key = meeting_id.to_string();
        let latest_execution = sqlx::query_as::<_, MeetingSummaryExecution>(
            "SELECT id, status, error_type, error_message, started_at, finished_at \
             FROM pipe_executions \
             WHERE pipe_name = ?1 \
               AND trigger_event = ?2 \
               AND (trigger_key = ?3 OR trigger_key LIKE ?3 || ':%') \
               AND (?4 IS NULL \
                    OR julianday(COALESCE(started_at, finished_at)) >= julianday(?4)) \
             ORDER BY id DESC \
             LIMIT 1",
        )
        .bind(pipe)
        .bind(MEETING_SUMMARY_TRIGGER_EVENT)
        .bind(&key)
        .bind(not_before)
        .fetch_optional(&self.pool)
        .await?;

        let claim_cutoff =
            (Utc::now() - chrono::Duration::seconds(UNSTARTED_CLAIM_WINDOW_SECS)).to_rfc3339();
        let unstarted: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM pipe_event_runs r \
             WHERE r.pipe_name = ?1 \
               AND r.event_name = ?2 \
               AND (r.event_key = ?3 OR r.event_key LIKE ?3 || ':%') \
               AND (?4 IS NULL OR julianday(r.claimed_at) >= julianday(?4)) \
               AND julianday(r.claimed_at) >= julianday(?5) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM pipe_executions e \
                   WHERE e.pipe_name = r.pipe_name \
                     AND e.trigger_event = r.event_name \
                     AND e.trigger_key = r.event_key \
               ) \
             LIMIT 1",
        )
        .bind(pipe)
        .bind(MEETING_SUMMARY_TRIGGER_EVENT)
        .bind(&key)
        .bind(not_before)
        .bind(&claim_cutoff)
        .fetch_optional(&self.pool)
        .await?;

        Ok(MeetingSummaryRunSnapshot {
            latest_execution,
            has_unstarted_claim: unstarted.is_some(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_appends_section_to_existing_note() {
        let note = "my notes\n- point one";
        let merged = merge_summary_into_note(Some(note), "the summary");
        assert_eq!(merged, "my notes\n- point one\n\n## Summary\nthe summary");
    }

    #[test]
    fn merge_into_empty_note_is_just_the_section() {
        assert_eq!(merge_summary_into_note(None, "s"), "## Summary\ns");
        assert_eq!(merge_summary_into_note(Some("  \n"), "s"), "## Summary\ns");
    }

    #[test]
    fn merge_replaces_existing_section_and_keeps_following_sections() {
        let note = "# Standup\nmine\n\n## Summary\nold text\n### detail\nold detail\n\n## Action items\n- ship it";
        let merged = merge_summary_into_note(Some(note), "new text");
        assert_eq!(
            merged,
            "# Standup\nmine\n\n## Summary\nnew text\n\n## Action items\n- ship it"
        );
    }

    #[test]
    fn merge_replaces_section_at_end_of_note() {
        let note = "notes\n\n## Summary\nfirst";
        let once = merge_summary_into_note(Some(note), "second");
        assert_eq!(once, "notes\n\n## Summary\nsecond");
        assert_eq!(merge_summary_into_note(Some(&once), "second"), once);
    }

    #[test]
    fn merge_targets_the_last_summary_heading() {
        let note = "## Summary\nwritten by me\n\n## Agenda\nx\n\n## Summary\nold ai";
        let merged = merge_summary_into_note(Some(note), "new ai");
        assert_eq!(
            merged,
            "## Summary\nwritten by me\n\n## Agenda\nx\n\n## Summary\nnew ai"
        );
    }

    #[test]
    fn merge_ignores_headings_inside_code_fences() {
        let note = "intro\n```\n## Summary\nnot a heading\n```";
        let merged = merge_summary_into_note(Some(note), "real");
        assert_eq!(merged, format!("{}\n\n## Summary\nreal", note));
    }

    #[test]
    fn merge_matches_heading_case_insensitively() {
        let note = "a\n\n## summary:\nold";
        assert_eq!(
            merge_summary_into_note(Some(note), "new"),
            "a\n\n## Summary\nnew"
        );
    }

    #[test]
    fn clean_drops_leading_heading_and_demotes_inner_headings() {
        let raw = "\n## Summary\nWe agreed on the plan.\n## Action items\n- Ana: draft\n```\n# kept in code\n```\n";
        assert_eq!(
            clean_summary_body(raw),
            "We agreed on the plan.\n### Action items\n- Ana: draft\n```\n# kept in code\n```"
        );
    }

    #[test]
    fn clean_leaves_plain_text_alone() {
        assert_eq!(clean_summary_body("  plain  "), "plain");
    }

    #[test]
    fn rejection_flags_empty_text() {
        assert_eq!(summary_rejection(""), Some(SummaryRejection::Empty));
        assert_eq!(summary_rejection("  \n\t"), Some(SummaryRejection::Empty));
        assert_eq!(summary_rejection("## "), Some(SummaryRejection::Empty));
    }

    #[test]
    fn rejection_flags_failure_messages() {
        for text in [
            "I'm sorry, but there is no transcript for this meeting.",
            "Sorry - I couldn't produce a summary because no audio was captured.",
            "Unfortunately I was unable to summarize this recording.",
            "I can\u{2019}t summarize this meeting: the transcript is empty.",
            "Error: failed to read the transcript",
            "Unfortunately, no transcript was available for this meeting.",
            "Unfortunately, I couldn't access the transcript for this meeting.",
            "Unfortunately, the summary could not be generated because the recording is empty.",
            "Unable to summarize: the transcript is empty.",
            "Failed to fetch the transcript.",
            "No transcript available.",
            "Nothing to summarize.",
            "Not enough transcript content to summarize.",
            "N/A",
            "None.",
        ] {
            assert_eq!(
                summary_rejection(text),
                Some(SummaryRejection::FailureText),
                "{text}"
            );
        }
    }

    #[test]
    fn rejection_accepts_real_summaries() {
        for text in [
            "The team agreed to ship on Friday.\n- Ana drafts the release notes",
            "Unfortunately the launch slipped; the meeting moved it to May.",
            "Unfortunately the meeting could not reach a decision on pricing.",
            "Errors in the deploy pipeline were the main topic.",
            "None of the blockers were resolved; Bo follows up.",
            "Not enough budget for Q3, so the scope was cut.",
            "No audio issues this week. The group reviewed the roadmap, picked the Q3 themes, \
             assigned owners for each theme and agreed to revisit the hiring plan next Tuesday \
             after the budget review.",
            "Sorry state of the build was discussed; Bo will fix CI.",
            "Unfortunately, the recording of the demo failed, so the team rescheduled it.",
            "Unfortunately there's no recording of the first half; the second half covered \
             the Q3 roadmap.",
            "Unfortunately, the summary from last week was not shared, so Ana will resend it.",
            "Unfortunately the summary pipeline failed last week; Bo is on it.",
            "Failed to ship the audio feature, so the team cut scope.",
            "Error budget for the audio service was not met; the team adds alerts.",
            "Unfortunately, the recording of the demo was not captured, so the team will redo it.",
            "Unfortunately no recording exists for the kickoff but the notes cover it.",
            "Unfortunately, no transcript was kept, and Ana will write up the decisions.",
        ] {
            assert_eq!(summary_rejection(text), None, "{text}");
        }
    }

    #[test]
    fn state_derivation_covers_every_status() {
        let exec = |status: &str| MeetingSummaryExecution {
            id: 1,
            status: status.to_string(),
            error_type: None,
            error_message: None,
            started_at: None,
            finished_at: None,
        };
        let snap = |e: Option<MeetingSummaryExecution>, claim: bool| MeetingSummaryRunSnapshot {
            latest_execution: e,
            has_unstarted_claim: claim,
        };
        assert_eq!(snap(None, false).state(), MeetingSummaryState::Idle);
        assert_eq!(snap(None, true).state(), MeetingSummaryState::Pending);
        assert_eq!(
            snap(Some(exec("queued")), false).state(),
            MeetingSummaryState::Pending
        );
        assert_eq!(
            snap(Some(exec("running")), false).state(),
            MeetingSummaryState::Running
        );
        assert_eq!(
            snap(Some(exec("completed")), false).state(),
            MeetingSummaryState::Ready
        );
        for status in ["failed", "timed_out", "cancelled"] {
            assert_eq!(
                snap(Some(exec(status)), false).state(),
                MeetingSummaryState::Failed
            );
        }
        assert_eq!(
            snap(Some(exec("completed")), true).state(),
            MeetingSummaryState::Pending
        );
    }

    #[test]
    fn not_before_must_be_rfc3339() {
        assert_eq!(
            parse_not_before("2026-10-20T10:00:00+02:00").as_deref(),
            Some("2026-10-20T08:00:00+00:00")
        );
        assert_eq!(parse_not_before("yesterday"), None);
    }

    #[test]
    fn calendar_event_ids_are_trimmed() {
        assert_eq!(normalize_calendar_event_id(Some("  evt ")), Some("evt"));
        assert_eq!(normalize_calendar_event_id(Some("  ")), None);
        assert_eq!(normalize_calendar_event_id(None), None);
    }
}
