// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Structured pipe schedules (`schedule_config` in pipe.md).
//!
//! Instead of a schedule string, a pipe can describe its recurrence as data:
//!
//! ```yaml
//! schedule_config:
//!   frequency: weeks          # minutes | hours | days | weeks | months
//!   interval: 1               # every N of those units (at least 1)
//!   days_of_week: [1, 4]      # weeks only: 0 = Sunday .. 6 = Saturday (7 is Sunday too)
//!   day_of_month: null        # months only: 1..=31
//!   at_hour: 9                # days, weeks, months
//!   at_minute: 0              # hours, days, weeks, months
//!   timezone: Europe/Berlin   # IANA name; leave it out to use this machine's zone
//!   starting: 2026-10-20T00:00:00+02:00
//!   ending: null              # last moment a run may happen, or null
//!   max_occurrences: null     # total runs counted from `starting`, or null
//! ```
//!
//! How each frequency recurs:
//!
//! - `minutes`: every `interval` minutes of elapsed time, measured from
//!   `starting`. Without a start the count begins at midnight on 2000-01-01
//!   in the schedule's zone, so an interval that divides an hour lands on
//!   round clock times, and stays there across daylight-saving changes,
//!   which move clocks by whole hours (in all but a few zones).
//! - `hours`: every `interval` hours at `at_minute` past the hour, counted
//!   in wall-clock hours from the hour `starting` falls in (from midnight on
//!   2000-01-01 without a start), so "every 2 hours" keeps to the same clock
//!   hours after a daylight-saving change. A repeated hour runs once.
//! - `days`: at `at_hour:at_minute` on every `interval`-th day, counted from
//!   the day of `starting`.
//! - `weeks`: at `at_hour:at_minute` on each listed weekday of every
//!   `interval`-th week, counted from the week of `starting`. Weeks run
//!   Sunday to Saturday.
//! - `months`: at `at_hour:at_minute` on `day_of_month` (default: the day of
//!   `starting`, else the 1st) of every `interval`-th month. A day beyond the
//!   end of a short month means that month's last day.
//!
//! Wall-clock times are resolved in the schedule's zone: a time that falls in
//! a daylight-saving gap moves forward in quarter-hour steps until it exists,
//! and a time that happens twice uses the first. `ending` and `max_occurrences` end the
//! schedule; the pipe runtime then disables the pipe, as it does for a fired
//! one-off.
//!
//! [`ScheduleConfig`] is the wire form. It is deliberately loose (plain
//! integers and strings) so hand-written values read; a `schedule_config` of
//! another shape is set aside when pipe.md loads, and the pipe still loads.
//! [`ScheduleConfig::compile`] checks the values and returns a
//! [`CompiledSchedule`] that computes run times and a one-line summary. A
//! schedule that is set aside or does not compile is ignored: the pipe
//! follows its `schedule` string.

use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime,
    SecondsFormat, TimeZone, Timelike, Utc,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Largest accepted `interval`.
pub const MAX_INTERVAL: i64 = 1000;
/// Largest accepted `max_occurrences`.
pub const MAX_OCCURRENCES: i64 = 1_000_000;
/// How many upcoming run times a preview lists.
pub const PREVIEW_COUNT: usize = 5;

/// Unit a structured schedule repeats in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScheduleFrequency {
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
}

impl ScheduleFrequency {
    /// The canonical (plural, lowercase) name written to pipe.md.
    pub fn as_str(self) -> &'static str {
        match self {
            ScheduleFrequency::Minutes => "minutes",
            ScheduleFrequency::Hours => "hours",
            ScheduleFrequency::Days => "days",
            ScheduleFrequency::Weeks => "weeks",
            ScheduleFrequency::Months => "months",
        }
    }

    /// Read a frequency name. Case and a trailing `s` do not matter.
    pub fn parse(raw: &str) -> Option<Self> {
        let lower = raw.trim().to_ascii_lowercase();
        let unit = lower.strip_suffix('s').unwrap_or(&lower);
        match unit {
            "minute" => Some(ScheduleFrequency::Minutes),
            "hour" => Some(ScheduleFrequency::Hours),
            "day" => Some(ScheduleFrequency::Days),
            "week" => Some(ScheduleFrequency::Weeks),
            "month" => Some(ScheduleFrequency::Months),
            _ => None,
        }
    }
}

impl fmt::Display for ScheduleFrequency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ScheduleFrequency {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ScheduleFrequency {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        ScheduleFrequency::parse(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unknown schedule frequency '{raw}': use minutes, hours, days, weeks or months"
            ))
        })
    }
}

/// `schedule_config` as written in pipe.md and sent to
/// `POST /pipes/schedule/preview`. Field names match screenpipe's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleConfig {
    pub frequency: ScheduleFrequency,
    /// Every N units. Missing means 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<i64>,
    /// Weekdays for `weeks`: 0 = Sunday .. 6 = Saturday; 7 also means Sunday.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub days_of_week: Vec<i64>,
    /// Day of the month for `months` (1..=31).
    #[serde(default)]
    pub day_of_month: Option<i64>,
    /// Hour of the day (0..=23). Missing means 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_hour: Option<i64>,
    /// Minute of the hour (0..=59). Missing means 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_minute: Option<i64>,
    /// IANA zone name. Missing or empty means the machine's local zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// When the schedule begins (RFC 3339). A timestamp without an offset,
    /// or a bare date, is read in the schedule's zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starting: Option<String>,
    /// Last moment a run may happen (RFC 3339), or null for never. A bare date
    /// includes the whole day.
    #[serde(default)]
    pub ending: Option<String>,
    /// Stop after this many runs, counted from `starting`.
    #[serde(default)]
    pub max_occurrences: Option<i64>,
}

fn null_as_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<i64>, D::Error> {
    Ok(Option::<Vec<i64>>::deserialize(deserializer)?.unwrap_or_default())
}

impl ScheduleConfig {
    /// A schedule of the given frequency with every other field unset.
    pub fn new(frequency: ScheduleFrequency) -> Self {
        Self {
            frequency,
            interval: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            at_hour: None,
            at_minute: None,
            timezone: None,
            starting: None,
            ending: None,
            max_occurrences: None,
        }
    }

    /// Check every field and build the schedule the runtime evaluates.
    pub fn compile(&self) -> Result<CompiledSchedule, ScheduleConfigError> {
        let interval = match self.interval {
            None => 1,
            Some(n) if (1..=MAX_INTERVAL).contains(&n) => n as u32,
            Some(n) => {
                return Err(ScheduleConfigError::new(format!(
                    "interval must be a whole number from 1 to {MAX_INTERVAL}, got {n}"
                )))
            }
        };
        let hour = in_range("at_hour", self.at_hour, 0, 23)?.unwrap_or(0);
        let minute = in_range("at_minute", self.at_minute, 0, 59)?.unwrap_or(0);
        let time = NaiveTime::from_hms_opt(hour, minute, 0)
            .ok_or_else(|| ScheduleConfigError::new("invalid at_hour/at_minute"))?;

        let tz = ScheduleTz::from_name(self.timezone.as_deref())?;
        let starting = parse_moment("starting", self.starting.as_deref(), tz, Bound::Start)?;
        let ending = parse_moment("ending", self.ending.as_deref(), tz, Bound::End)?;
        if let (Some(start), Some(end)) = (starting, ending) {
            if end < start {
                return Err(ScheduleConfigError::new(
                    "ending is earlier than starting, so the schedule would never run",
                ));
            }
        }

        let max_occurrences = match self.max_occurrences {
            None => None,
            Some(n) if (1..=MAX_OCCURRENCES).contains(&n) => Some(n as u64),
            Some(n) => {
                return Err(ScheduleConfigError::new(format!(
                    "max_occurrences must be a whole number from 1 to {MAX_OCCURRENCES} or null, got {n}"
                )))
            }
        };
        if max_occurrences.is_some() && starting.is_none() {
            return Err(ScheduleConfigError::new(
                "max_occurrences counts runs from starting, so starting must be set too",
            ));
        }

        let days_of_week = if self.frequency == ScheduleFrequency::Weeks {
            let mut days = Vec::with_capacity(self.days_of_week.len());
            for &d in &self.days_of_week {
                if !(0..=7).contains(&d) {
                    return Err(ScheduleConfigError::new(format!(
                        "days_of_week entries run from 0 (Sunday) to 6 (Saturday), got {d}"
                    )));
                }
                days.push((d % 7) as u32);
            }
            days.sort_unstable();
            days.dedup();
            if days.is_empty() {
                return Err(ScheduleConfigError::new(
                    "a weekly schedule needs at least one day in days_of_week",
                ));
            }
            days
        } else {
            Vec::new()
        };

        let day_of_month = if self.frequency == ScheduleFrequency::Months {
            in_range("day_of_month", self.day_of_month, 1, 31)?
        } else {
            None
        };

        let anchor = match starting {
            Some(start) => tz.wall_clock(start),
            None => reference_anchor(),
        };
        let mut compiled = CompiledSchedule {
            frequency: self.frequency,
            interval,
            days_of_week,
            day_of_month,
            time,
            tz,
            starting,
            ending,
            max_occurrences,
            anchor,
            skipped_at_start: 0,
        };
        compiled.skipped_at_start = match (compiled.candidates(0), starting) {
            (Some(first), Some(start)) => first.iter().filter(|c| **c < start).count() as u64,
            _ => 0,
        };
        Ok(compiled)
    }

    /// Check the schedule without keeping the compiled form.
    pub fn validate(&self) -> Result<(), ScheduleConfigError> {
        self.compile().map(|_| ())
    }

    /// The summary and the next `count` run times after `now`, as returned by
    /// `POST /pipes/schedule/preview`.
    pub fn preview(
        &self,
        now: DateTime<Utc>,
        count: usize,
    ) -> Result<SchedulePreview, ScheduleConfigError> {
        let compiled = self.compile()?;
        Ok(SchedulePreview {
            summary: compiled.summary(now),
            next: compiled
                .upcoming(now, count)
                .into_iter()
                .map(|t| compiled.format_time(t))
                .collect(),
        })
    }
}

/// Response body of `POST /pipes/schedule/preview`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchedulePreview {
    /// One sentence describing the schedule.
    pub summary: String,
    /// Upcoming run times, RFC 3339 in the schedule's zone.
    pub next: Vec<String>,
}

/// Why a `schedule_config` was refused. The message names the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleConfigError(String);

impl ScheduleConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ScheduleConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid schedule_config: {}", self.0)
    }
}

impl std::error::Error for ScheduleConfigError {}

fn in_range(
    field: &str,
    value: Option<i64>,
    min: i64,
    max: i64,
) -> Result<Option<u32>, ScheduleConfigError> {
    match value {
        None => Ok(None),
        Some(v) if (min..=max).contains(&v) => Ok(Some(v as u32)),
        Some(v) => Err(ScheduleConfigError::new(format!(
            "{field} must be from {min} to {max}, got {v}"
        ))),
    }
}

/// Counting origin when a schedule has no `starting`.
fn reference_anchor() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2000, 1, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .expect("2000-01-01 is a valid date")
}

#[derive(Clone, Copy)]
enum Bound {
    Start,
    End,
}

fn parse_moment(
    field: &str,
    raw: Option<&str>,
    tz: ScheduleTz,
    bound: Bound,
) -> Result<Option<DateTime<Utc>>, ScheduleConfigError> {
    let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if let Ok(t) = DateTime::parse_from_rfc3339(text) {
        return Ok(Some(t.with_timezone(&Utc)));
    }
    let unresolvable =
        || ScheduleConfigError::new(format!("{field} '{text}' does not exist in that timezone"));
    const NAIVE_FORMATS: [&str; 6] = [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
    ];
    for format in NAIVE_FORMATS {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return tz.resolve(naive).map(Some).ok_or_else(unresolvable);
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        let naive = match bound {
            Bound::Start => date.and_hms_opt(0, 0, 0),
            Bound::End => date.and_hms_opt(23, 59, 59),
        }
        .ok_or_else(unresolvable)?;
        return tz.resolve(naive).map(Some).ok_or_else(unresolvable);
    }
    Err(ScheduleConfigError::new(format!(
        "{field} must be an RFC 3339 timestamp such as 2026-10-20T09:00:00Z, got '{text}'"
    )))
}

/// The zone a schedule's wall-clock times are read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleTz {
    /// The machine's current local zone.
    Local,
    Named(chrono_tz::Tz),
}

impl ScheduleTz {
    fn from_name(name: Option<&str>) -> Result<Self, ScheduleConfigError> {
        match name.map(str::trim).filter(|s| !s.is_empty()) {
            None => Ok(ScheduleTz::Local),
            Some(n) => n
                .parse::<chrono_tz::Tz>()
                .map(ScheduleTz::Named)
                .map_err(|_| {
                    ScheduleConfigError::new(format!(
                        "unknown timezone '{n}': use an IANA name such as Europe/Berlin"
                    ))
                }),
        }
    }

    /// The wall-clock reading of `t` in this zone.
    fn wall_clock(self, t: DateTime<Utc>) -> NaiveDateTime {
        match self {
            ScheduleTz::Local => t.with_timezone(&Local).naive_local(),
            ScheduleTz::Named(tz) => t.with_timezone(&tz).naive_local(),
        }
    }

    /// The instant a wall-clock reading in this zone stands for.
    fn resolve(self, naive: NaiveDateTime) -> Option<DateTime<Utc>> {
        match self {
            ScheduleTz::Local => resolve_in(&Local, naive),
            ScheduleTz::Named(tz) => resolve_in(&tz, naive),
        }
    }
}

/// Map a wall-clock time to an instant. A repeated time (clocks going back)
/// takes its first occurrence; a skipped time (clocks going forward) moves to
/// the first valid quarter hour after it.
fn resolve_in<Z: TimeZone>(tz: &Z, naive: NaiveDateTime) -> Option<DateTime<Utc>> {
    let mut probe = naive;
    // Daylight-saving gaps are at most a couple of hours; 16 quarter-hour
    // steps cover four.
    for _ in 0..=16 {
        match tz.from_local_datetime(&probe) {
            LocalResult::Single(t) => return Some(t.with_timezone(&Utc)),
            LocalResult::Ambiguous(first, _) => return Some(first.with_timezone(&Utc)),
            LocalResult::None => probe = probe.checked_add_signed(Duration::minutes(15))?,
        }
    }
    None
}

/// A checked `schedule_config`, ready to compute run times.
#[derive(Debug, Clone)]
pub struct CompiledSchedule {
    frequency: ScheduleFrequency,
    interval: u32,
    /// Weekdays (0 = Sunday), sorted and unique. Weekly schedules only.
    days_of_week: Vec<u32>,
    day_of_month: Option<u32>,
    time: NaiveTime,
    tz: ScheduleTz,
    starting: Option<DateTime<Utc>>,
    ending: Option<DateTime<Utc>>,
    max_occurrences: Option<u64>,
    /// `starting` read in the schedule's zone, or the reference origin.
    anchor: NaiveDateTime,
    /// Slots of the first period that fall before `starting` and so do not
    /// count as occurrences.
    skipped_at_start: u64,
}

impl CompiledSchedule {
    pub fn frequency(&self) -> ScheduleFrequency {
        self.frequency
    }

    pub fn timezone(&self) -> ScheduleTz {
        self.tz
    }

    /// The first run strictly after `t`, or `None` once the schedule has
    /// ended (past `ending` or out of occurrences).
    pub fn next_after(&self, t: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let mut period = self.first_period_after(t);
        // `first_period_after` starts at most two periods early, so a short
        // scan always reaches the answer.
        for _ in 0..64 {
            let slots = self.candidates(period)?;
            for (index, slot) in slots.iter().copied().enumerate() {
                if self.starting.is_some_and(|start| slot < start) || slot <= t {
                    continue;
                }
                if self.ending.is_some_and(|end| slot > end) {
                    return None;
                }
                if let Some(max) = self.max_occurrences {
                    if self.occurrence_number(period, index) > max {
                        return None;
                    }
                }
                return Some(slot);
            }
            period = period.checked_add(1)?;
        }
        None
    }

    /// Up to `count` run times after `t`, in order.
    pub fn upcoming(&self, t: DateTime<Utc>, count: usize) -> Vec<DateTime<Utc>> {
        let mut out = Vec::with_capacity(count);
        let mut cursor = t;
        while out.len() < count {
            match self.next_after(cursor) {
                Some(next) => {
                    out.push(next);
                    cursor = next;
                }
                None => break,
            }
        }
        out
    }

    /// True when no run remains after `t`.
    pub fn has_ended_after(&self, t: DateTime<Utc>) -> bool {
        self.next_after(t).is_none()
    }

    /// Typical gap between two runs, used to size a run's lookback window.
    pub fn approximate_period(&self) -> std::time::Duration {
        let n = u64::from(self.interval);
        let secs = match self.frequency {
            ScheduleFrequency::Minutes => n * 60,
            ScheduleFrequency::Hours => n * 3_600,
            ScheduleFrequency::Days => n * 86_400,
            ScheduleFrequency::Weeks => {
                let per_week = self.days_of_week.len().max(1) as u64;
                n * 7 * 86_400 / per_week
            }
            ScheduleFrequency::Months => n * 30 * 86_400,
        };
        std::time::Duration::from_secs(secs)
    }

    /// `t` as RFC 3339 in the schedule's zone.
    pub fn format_time(&self, t: DateTime<Utc>) -> String {
        match self.tz {
            ScheduleTz::Local => t
                .with_timezone(&Local)
                .to_rfc3339_opts(SecondsFormat::Secs, true),
            ScheduleTz::Named(tz) => t
                .with_timezone(&tz)
                .to_rfc3339_opts(SecondsFormat::Secs, true),
        }
    }

    /// One sentence describing the schedule, e.g. "Every week on Monday and
    /// Thursday at 09:00". The start date is mentioned only while it is still
    /// ahead of `now`.
    pub fn summary(&self, now: DateTime<Utc>) -> String {
        let n = self.interval;
        let clock = format!("{:02}:{:02}", self.time.hour(), self.time.minute());
        let mut text = match self.frequency {
            ScheduleFrequency::Minutes => {
                if n == 1 {
                    "Every minute".to_string()
                } else {
                    format!("Every {n} minutes")
                }
            }
            ScheduleFrequency::Hours => {
                let minute = self.time.minute();
                if n == 1 {
                    format!("Every hour at :{minute:02}")
                } else {
                    format!("Every {n} hours at :{minute:02}")
                }
            }
            ScheduleFrequency::Days => {
                if n == 1 {
                    format!("Every day at {clock}")
                } else {
                    format!("Every {n} days at {clock}")
                }
            }
            ScheduleFrequency::Weeks => {
                if n == 1 && self.days_of_week.len() == 7 {
                    format!("Every day at {clock}")
                } else {
                    let days = describe_weekdays(&self.days_of_week);
                    if n == 1 {
                        format!("Every week on {days} at {clock}")
                    } else {
                        format!("Every {n} weeks on {days} at {clock}")
                    }
                }
            }
            ScheduleFrequency::Months => {
                let day = self.month_day();
                let unit = if n == 1 {
                    "Every month".to_string()
                } else {
                    format!("Every {n} months")
                };
                let short_months = if day > 28 {
                    " (the last day in shorter months)"
                } else {
                    ""
                };
                format!("{unit} on the {}{short_months} at {clock}", ordinal(day))
            }
        };
        if let ScheduleTz::Named(tz) = self.tz {
            text.push_str(&format!(" ({})", tz.name()));
        }
        if let Some(start) = self.starting.filter(|start| *start > now) {
            text.push_str(&format!(", starting {}", self.describe_moment(start)));
        }
        if let Some(end) = self.ending {
            text.push_str(&format!(", until {}", self.describe_moment(end)));
        }
        match self.max_occurrences {
            Some(1) => text.push_str(", once"),
            Some(max) => text.push_str(&format!(", {max} times in total")),
            None => {}
        }
        text
    }

    fn describe_moment(&self, t: DateTime<Utc>) -> String {
        self.tz.wall_clock(t).format("%Y-%m-%d %H:%M").to_string()
    }

    fn month_day(&self) -> u32 {
        self.day_of_month.unwrap_or(if self.starting.is_some() {
            self.anchor.day()
        } else {
            1
        })
    }

    /// 1-based count of the slot (`period`, `index`) among the schedule's
    /// occurrences.
    fn occurrence_number(&self, period: u64, index: usize) -> u64 {
        let per_period = match self.frequency {
            ScheduleFrequency::Weeks => self.days_of_week.len() as u64,
            _ => 1,
        };
        (period.saturating_mul(per_period) + index as u64 + 1).saturating_sub(self.skipped_at_start)
    }

    /// The candidate run times of period `k` (k = 0 is the period holding
    /// the anchor). Weekly schedules have one slot per listed weekday, the
    /// others exactly one. `None` once the dates leave chrono's range.
    fn candidates(&self, k: u64) -> Option<Vec<DateTime<Utc>>> {
        let n = u64::from(self.interval);
        let steps = i64::try_from(k.checked_mul(n)?).ok()?;
        match self.frequency {
            ScheduleFrequency::Minutes => {
                let base = self.anchor_instant()?;
                Some(vec![base.checked_add_signed(Duration::try_minutes(steps)?)?])
            }
            ScheduleFrequency::Hours => {
                let wall = self
                    .hour_origin()?
                    .checked_add_signed(Duration::try_hours(steps)?)?;
                Some(vec![self.tz.resolve(wall)?])
            }
            ScheduleFrequency::Days => {
                let date = self
                    .anchor
                    .date()
                    .checked_add_signed(Duration::try_days(steps)?)?;
                Some(vec![self.tz.resolve(date.and_time(self.time))?])
            }
            ScheduleFrequency::Weeks => {
                let week = self
                    .week_start(self.anchor.date())
                    .checked_add_signed(Duration::try_days(steps.checked_mul(7)?)?)?;
                self.days_of_week
                    .iter()
                    .map(|&d| {
                        let date = week.checked_add_signed(Duration::days(i64::from(d)))?;
                        self.tz.resolve(date.and_time(self.time))
                    })
                    .collect()
            }
            ScheduleFrequency::Months => {
                let origin = i64::from(self.anchor.year()) * 12 + i64::from(self.anchor.month0());
                let index = origin.checked_add(steps)?;
                let year = i32::try_from(index.div_euclid(12)).ok()?;
                let month = index.rem_euclid(12) as u32 + 1;
                let day = self.month_day().min(days_in_month(year, month)?);
                let date = NaiveDate::from_ymd_opt(year, month, day)?;
                Some(vec![self.tz.resolve(date.and_time(self.time))?])
            }
        }
    }

    /// Wall-clock time of the first hourly slot: the anchor's hour at
    /// `at_minute` (hours frequency).
    fn hour_origin(&self) -> Option<NaiveDateTime> {
        self.anchor
            .date()
            .and_hms_opt(self.anchor.hour(), self.time.minute(), 0)
    }

    /// The anchor as an instant (minutes frequency).
    fn anchor_instant(&self) -> Option<DateTime<Utc>> {
        self.starting.or_else(|| self.tz.resolve(self.anchor))
    }

    /// The Sunday that starts the week holding `date`.
    fn week_start(&self, date: NaiveDate) -> NaiveDate {
        date - Duration::days(i64::from(date.weekday().num_days_from_sunday()))
    }

    /// A period index no later than the first period that has a slot after
    /// `t` (at most two periods early).
    fn first_period_after(&self, t: DateTime<Utc>) -> u64 {
        let n = i64::from(self.interval);
        let periods: i64 = match self.frequency {
            ScheduleFrequency::Minutes => match self.anchor_instant() {
                Some(base) if t > base => (t - base).num_seconds() / (60 * n) - 1,
                _ => 0,
            },
            ScheduleFrequency::Hours => {
                // Counted on the wall clock, like the slots. Starting a
                // period early also covers a slot that a daylight-saving gap
                // pushed past `t`'s wall time.
                match self.hour_origin() {
                    Some(origin) => (self.tz.wall_clock(t) - origin).num_hours() / n - 1,
                    None => 0,
                }
            }
            ScheduleFrequency::Days => {
                let today = self.tz.wall_clock(t).date();
                (today - self.anchor.date()).num_days() / n - 1
            }
            ScheduleFrequency::Weeks => {
                let this_week = self.week_start(self.tz.wall_clock(t).date());
                let first_week = self.week_start(self.anchor.date());
                (this_week - first_week).num_days() / 7 / n - 1
            }
            ScheduleFrequency::Months => {
                let now = self.tz.wall_clock(t);
                let months = (i64::from(now.year()) - i64::from(self.anchor.year())) * 12
                    + i64::from(now.month0())
                    - i64::from(self.anchor.month0());
                months / n - 1
            }
        };
        u64::try_from(periods).unwrap_or(0)
    }
}

fn days_in_month(year: i32, month: u32) -> Option<u32> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year.checked_add(1)?, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };
    Some((next - first).num_days() as u32)
}

const WEEKDAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

fn describe_weekdays(days: &[u32]) -> String {
    match days {
        [1, 2, 3, 4, 5] => return "weekdays".to_string(),
        [0, 6] => return "weekends".to_string(),
        _ => {}
    }
    let names: Vec<&str> = days
        .iter()
        .filter_map(|&d| WEEKDAY_NAMES.get(d as usize).copied())
        .collect();
    match names.as_slice() {
        [] => String::new(),
        [only] => only.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn config(value: serde_json::Value) -> ScheduleConfig {
        serde_json::from_value(value).expect("schedule_config parses")
    }

    fn next_n(value: serde_json::Value, after: &str, n: usize) -> Vec<String> {
        let compiled = config(value).compile().expect("schedule compiles");
        compiled
            .upcoming(utc(after), n)
            .into_iter()
            .map(|t| compiled.format_time(t))
            .collect()
    }

    #[test]
    fn daily_at_nine_utc() {
        let next = next_n(
            json!({"frequency": "days", "interval": 1, "at_hour": 9, "at_minute": 0, "timezone": "UTC"}),
            "2026-10-20T10:00:00Z",
            3,
        );
        assert_eq!(
            next,
            [
                "2026-10-21T09:00:00Z",
                "2026-10-22T09:00:00Z",
                "2026-10-23T09:00:00Z"
            ]
        );
    }

    #[test]
    fn preview_matches_the_acceptance_request() {
        // The exact body the plan's harness check posts (no timezone, so the
        // machine's zone is used).
        let preview =
            config(json!({"frequency": "days", "interval": 1, "at_hour": 9, "at_minute": 0}))
                .preview(Utc::now(), PREVIEW_COUNT)
                .unwrap();
        assert_eq!(preview.summary, "Every day at 09:00");
        assert_eq!(preview.next.len(), PREVIEW_COUNT);
        let times: Vec<DateTime<Utc>> = preview
            .next
            .iter()
            .map(|s| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc))
            .collect();
        for pair in times.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        for t in &times {
            let local = t.with_timezone(&Local);
            assert_eq!((local.minute(), local.second()), (0, 0));
        }
    }

    #[test]
    fn daily_follows_wall_clock_across_dst() {
        // Berlin leaves summer time on 2026-10-25.
        let next = next_n(
            json!({"frequency": "days", "at_hour": 9, "timezone": "Europe/Berlin"}),
            "2026-10-24T12:00:00Z",
            2,
        );
        assert_eq!(
            next,
            ["2026-10-25T09:00:00+01:00", "2026-10-26T09:00:00+01:00"]
        );
        let before = next_n(
            json!({"frequency": "days", "at_hour": 9, "timezone": "Europe/Berlin"}),
            "2026-10-23T12:00:00Z",
            1,
        );
        assert_eq!(before, ["2026-10-24T09:00:00+02:00"]);
    }

    #[test]
    fn time_in_a_dst_gap_moves_forward() {
        // 02:30 does not exist in New York on 2026-03-08.
        let next = next_n(
            json!({"frequency": "days", "at_hour": 2, "at_minute": 30, "timezone": "America/New_York"}),
            "2026-03-07T12:00:00Z",
            2,
        );
        assert_eq!(
            next,
            ["2026-03-08T03:00:00-04:00", "2026-03-09T02:30:00-04:00"]
        );
    }

    #[test]
    fn every_two_days_counts_from_starting() {
        let next = next_n(
            json!({"frequency": "days", "interval": 2, "at_hour": 8, "timezone": "UTC",
                   "starting": "2026-10-01T00:00:00Z"}),
            "2026-10-20T09:00:00Z",
            3,
        );
        // Oct 1, 3, 5 ... are run days; the 21st is the next one after the 20th.
        assert_eq!(
            next,
            [
                "2026-10-21T08:00:00Z",
                "2026-10-23T08:00:00Z",
                "2026-10-25T08:00:00Z"
            ]
        );
    }

    #[test]
    fn weekly_on_listed_days() {
        // 2026-10-20 is a Tuesday.
        let next = next_n(
            json!({"frequency": "weeks", "days_of_week": [5, 1], "at_hour": 9, "timezone": "UTC"}),
            "2026-10-20T00:00:00Z",
            3,
        );
        assert_eq!(
            next,
            [
                "2026-10-23T09:00:00Z",
                "2026-10-26T09:00:00Z",
                "2026-10-30T09:00:00Z"
            ]
        );
    }

    #[test]
    fn biweekly_skips_alternate_weeks() {
        let next = next_n(
            json!({"frequency": "weeks", "interval": 2, "days_of_week": [1], "at_hour": 9,
                   "timezone": "UTC", "starting": "2026-10-19T00:00:00Z"}),
            "2026-10-18T00:00:00Z",
            3,
        );
        assert_eq!(
            next,
            [
                "2026-10-19T09:00:00Z",
                "2026-11-02T09:00:00Z",
                "2026-11-16T09:00:00Z"
            ]
        );
    }

    #[test]
    fn seven_means_sunday() {
        let compiled =
            config(json!({"frequency": "weeks", "days_of_week": [7, 0], "timezone": "UTC"}))
                .compile()
                .unwrap();
        assert_eq!(compiled.days_of_week, vec![0]);
        let next = compiled.next_after(utc("2026-10-20T00:00:00Z")).unwrap();
        assert_eq!(compiled.format_time(next), "2026-10-25T00:00:00Z");
    }

    #[test]
    fn monthly_clamps_to_short_months() {
        let next = next_n(
            json!({"frequency": "months", "day_of_month": 31, "at_hour": 18, "timezone": "UTC"}),
            "2027-01-31T19:00:00Z",
            3,
        );
        assert_eq!(
            next,
            [
                "2027-02-28T18:00:00Z",
                "2027-03-31T18:00:00Z",
                "2027-04-30T18:00:00Z"
            ]
        );
    }

    #[test]
    fn monthly_defaults_to_the_start_day() {
        let next = next_n(
            json!({"frequency": "months", "interval": 3, "at_hour": 7, "timezone": "UTC",
                   "starting": "2026-01-15T00:00:00Z"}),
            "2026-02-01T00:00:00Z",
            2,
        );
        assert_eq!(next, ["2026-04-15T07:00:00Z", "2026-07-15T07:00:00Z"]);
    }

    #[test]
    fn minutes_count_from_starting() {
        let next = next_n(
            json!({"frequency": "minutes", "interval": 45, "timezone": "UTC",
                   "starting": "2026-10-20T10:10:00Z"}),
            "2026-10-20T11:00:00Z",
            2,
        );
        assert_eq!(next, ["2026-10-20T11:40:00Z", "2026-10-20T12:25:00Z"]);
    }

    #[test]
    fn minutes_without_start_line_up_with_the_clock() {
        let next = next_n(
            json!({"frequency": "minutes", "interval": 15, "timezone": "UTC"}),
            "2026-10-20T11:07:30Z",
            2,
        );
        assert_eq!(next, ["2026-10-20T11:15:00Z", "2026-10-20T11:30:00Z"]);
    }

    #[test]
    fn hours_run_at_the_given_minute() {
        let next = next_n(
            json!({"frequency": "hours", "interval": 2, "at_minute": 15, "timezone": "UTC",
                   "starting": "2026-10-20T10:30:00Z"}),
            "2026-10-20T10:00:00Z",
            3,
        );
        // 10:15 is before the start, so the first run is two hours later.
        assert_eq!(
            next,
            [
                "2026-10-20T12:15:00Z",
                "2026-10-20T14:15:00Z",
                "2026-10-20T16:15:00Z"
            ]
        );
    }

    #[test]
    fn hours_keep_their_clock_hours_across_dst() {
        // Berlin leaves summer time at 03:00 on 2026-10-25; 02:15 happens
        // twice and runs once. Every 2 hours stays on even clock hours.
        let next = next_n(
            json!({"frequency": "hours", "interval": 2, "at_minute": 15, "timezone": "Europe/Berlin"}),
            "2026-10-24T20:00:00Z",
            5,
        );
        assert_eq!(
            next,
            [
                "2026-10-24T22:15:00+02:00",
                "2026-10-25T00:15:00+02:00",
                "2026-10-25T02:15:00+02:00",
                "2026-10-25T04:15:00+01:00",
                "2026-10-25T06:15:00+01:00"
            ]
        );
        // Asked during the repeated hour, the next run is still 04:15.
        let during = next_n(
            json!({"frequency": "hours", "interval": 2, "at_minute": 15, "timezone": "Europe/Berlin"}),
            "2026-10-25T01:20:00Z",
            1,
        );
        assert_eq!(during, ["2026-10-25T04:15:00+01:00"]);
    }

    #[test]
    fn hourly_slot_in_a_dst_gap_still_runs() {
        // New York skips 02:00-03:00 on 2026-03-08.
        let next = next_n(
            json!({"frequency": "hours", "at_minute": 30, "timezone": "America/New_York"}),
            "2026-03-08T05:00:00Z",
            5,
        );
        assert_eq!(
            next,
            [
                "2026-03-08T00:30:00-05:00",
                "2026-03-08T01:30:00-05:00",
                "2026-03-08T03:00:00-04:00",
                "2026-03-08T03:30:00-04:00",
                "2026-03-08T04:30:00-04:00"
            ]
        );
    }

    #[test]
    fn minutes_keep_elapsed_time_and_round_times_across_dst() {
        let next = next_n(
            json!({"frequency": "minutes", "interval": 30, "timezone": "Europe/Berlin"}),
            "2026-10-25T00:10:00Z",
            3,
        );
        assert_eq!(
            next,
            [
                "2026-10-25T02:30:00+02:00",
                "2026-10-25T02:00:00+01:00",
                "2026-10-25T02:30:00+01:00"
            ]
        );
    }

    #[test]
    fn ending_stops_the_schedule() {
        let compiled = config(json!({"frequency": "days", "at_hour": 9, "timezone": "UTC",
                                     "ending": "2026-10-22"}))
        .compile()
        .unwrap();
        let runs = compiled.upcoming(utc("2026-10-20T10:00:00Z"), 10);
        assert_eq!(runs.len(), 2, "the bare end date includes its own day");
        assert!(compiled.has_ended_after(utc("2026-10-22T09:00:00Z")));
        assert!(!compiled.has_ended_after(utc("2026-10-22T08:59:00Z")));
    }

    #[test]
    fn max_occurrences_count_from_starting() {
        // Starts 10:00, runs at 09:00: the start day's slot is skipped and
        // does not use up an occurrence.
        let compiled = config(json!({"frequency": "days", "at_hour": 9, "timezone": "UTC",
                                     "starting": "2026-10-20T10:00:00Z", "max_occurrences": 3}))
        .compile()
        .unwrap();
        let runs: Vec<String> = compiled
            .upcoming(utc("2026-10-01T00:00:00Z"), 10)
            .into_iter()
            .map(|t| compiled.format_time(t))
            .collect();
        assert_eq!(
            runs,
            [
                "2026-10-21T09:00:00Z",
                "2026-10-22T09:00:00Z",
                "2026-10-23T09:00:00Z"
            ]
        );
        // Asking late still respects the count.
        assert_eq!(compiled.next_after(utc("2026-10-23T09:00:00Z")), None);
    }

    #[test]
    fn max_occurrences_on_weekly_counts_each_day() {
        // Starts Wednesday 2026-10-21; Monday of that week does not count.
        let compiled = config(
            json!({"frequency": "weeks", "days_of_week": [1, 5], "timezone": "UTC",
                                     "starting": "2026-10-21T00:00:00Z", "max_occurrences": 3}),
        )
        .compile()
        .unwrap();
        let runs: Vec<String> = compiled
            .upcoming(utc("2026-10-01T00:00:00Z"), 10)
            .into_iter()
            .map(|t| compiled.format_time(t))
            .collect();
        assert_eq!(
            runs,
            [
                "2026-10-23T00:00:00Z",
                "2026-10-26T00:00:00Z",
                "2026-10-30T00:00:00Z"
            ]
        );
    }

    #[test]
    fn naive_starting_is_read_in_the_schedule_zone() {
        let compiled = config(json!({"frequency": "minutes", "interval": 60,
                                     "timezone": "Asia/Tokyo", "starting": "2026-10-20T09:00"}))
        .compile()
        .unwrap();
        assert_eq!(compiled.starting, Some(utc("2026-10-20T00:00:00Z")));
    }

    #[test]
    fn rejects_bad_values_with_the_field_name() {
        let cases = [
            (json!({"frequency": "weeks"}), "days_of_week"),
            (
                json!({"frequency": "weeks", "days_of_week": [8]}),
                "days_of_week",
            ),
            (json!({"frequency": "days", "interval": 0}), "interval"),
            (json!({"frequency": "days", "at_hour": 24}), "at_hour"),
            (json!({"frequency": "days", "at_minute": 60}), "at_minute"),
            (
                json!({"frequency": "months", "day_of_month": 32}),
                "day_of_month",
            ),
            (
                json!({"frequency": "days", "timezone": "Mars/Olympus"}),
                "timezone",
            ),
            (
                json!({"frequency": "days", "starting": "tomorrow"}),
                "starting",
            ),
            (
                json!({"frequency": "days", "starting": "2026-10-20T00:00:00Z", "ending": "2026-10-19T00:00:00Z"}),
                "ending",
            ),
            (
                json!({"frequency": "days", "max_occurrences": 2}),
                "max_occurrences",
            ),
            (
                json!({"frequency": "days", "starting": "2026-10-20T00:00:00Z", "max_occurrences": 0}),
                "max_occurrences",
            ),
        ];
        for (value, field) in cases {
            let err = config(value.clone()).compile().unwrap_err();
            assert!(
                err.message().contains(field),
                "{value}: error {err} should name {field}"
            );
        }
    }

    #[test]
    fn unknown_frequency_fails_to_parse() {
        let err = serde_json::from_value::<ScheduleConfig>(json!({"frequency": "fortnights"}))
            .unwrap_err()
            .to_string();
        assert!(err.contains("fortnights"), "{err}");
        assert_eq!(
            ScheduleFrequency::parse(" Week "),
            Some(ScheduleFrequency::Weeks)
        );
        assert_eq!(
            ScheduleFrequency::parse("MONTHS"),
            Some(ScheduleFrequency::Months)
        );
    }

    #[test]
    fn nulls_are_accepted() {
        let parsed = config(json!({
            "frequency": "weeks", "interval": 1, "days_of_week": null, "day_of_month": null,
            "at_hour": 9, "at_minute": 0, "timezone": null, "starting": null, "ending": null,
            "max_occurrences": null
        }));
        assert!(parsed.days_of_week.is_empty());
        assert!(parsed.timezone.is_none());
    }

    #[test]
    fn yaml_round_trip_keeps_every_field() {
        let original = ScheduleConfig {
            frequency: ScheduleFrequency::Weeks,
            interval: Some(2),
            days_of_week: vec![1, 3],
            day_of_month: None,
            at_hour: Some(9),
            at_minute: Some(30),
            timezone: Some("Europe/Berlin".into()),
            starting: Some("2026-10-20T00:00:00+02:00".into()),
            ending: Some("2026-12-31T23:59:00+01:00".into()),
            max_occurrences: Some(20),
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        let back: ScheduleConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back, original);
        // An unquoted YAML timestamp still reads as text.
        let unquoted: ScheduleConfig =
            serde_yaml::from_str("frequency: days\nstarting: 2026-10-20T00:00:00Z\nending: null\n")
                .unwrap();
        assert_eq!(unquoted.starting.as_deref(), Some("2026-10-20T00:00:00Z"));
    }

    #[test]
    fn summaries() {
        let now = utc("2026-10-20T00:00:00Z");
        let summary = |value: serde_json::Value| config(value).compile().unwrap().summary(now);
        assert_eq!(summary(json!({"frequency": "minutes"})), "Every minute");
        assert_eq!(
            summary(json!({"frequency": "minutes", "interval": 30})),
            "Every 30 minutes"
        );
        assert_eq!(
            summary(json!({"frequency": "hours", "interval": 3, "at_minute": 5})),
            "Every 3 hours at :05"
        );
        assert_eq!(
            summary(json!({"frequency": "days", "interval": 2, "at_hour": 18, "at_minute": 30})),
            "Every 2 days at 18:30"
        );
        assert_eq!(
            summary(json!({"frequency": "weeks", "days_of_week": [1, 2, 3, 4, 5], "at_hour": 8})),
            "Every week on weekdays at 08:00"
        );
        assert_eq!(
            summary(
                json!({"frequency": "weeks", "interval": 2, "days_of_week": [4, 1], "at_hour": 8})
            ),
            "Every 2 weeks on Monday and Thursday at 08:00"
        );
        assert_eq!(
            summary(
                json!({"frequency": "weeks", "days_of_week": [0, 1, 2, 3, 4, 5, 6], "at_hour": 7})
            ),
            "Every day at 07:00"
        );
        assert_eq!(
            summary(json!({"frequency": "months", "day_of_month": 31, "at_hour": 9})),
            "Every month on the 31st (the last day in shorter months) at 09:00"
        );
        assert_eq!(
            summary(json!({"frequency": "months", "interval": 6, "day_of_month": 2})),
            "Every 6 months on the 2nd at 00:00"
        );
        assert_eq!(
            summary(json!({"frequency": "days", "at_hour": 9, "timezone": "Europe/Berlin",
                           "starting": "2026-11-01T00:00:00+01:00", "ending": "2026-11-30",
                           "max_occurrences": 1})),
            "Every day at 09:00 (Europe/Berlin), starting 2026-11-01 00:00, until 2026-11-30 23:59, once"
        );
        // A start already in the past is not mentioned.
        assert_eq!(
            summary(
                json!({"frequency": "days", "at_hour": 9, "starting": "2026-01-01T00:00:00Z",
                           "max_occurrences": 400})
            ),
            "Every day at 09:00, 400 times in total"
        );
    }

    #[test]
    fn ordinals() {
        let got: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 31]
            .into_iter()
            .map(ordinal)
            .collect();
        assert_eq!(
            got,
            ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "23rd", "31st"]
        );
    }

    #[test]
    fn far_future_lookups_stay_fast_and_correct() {
        let compiled = config(json!({"frequency": "minutes", "interval": 1, "timezone": "UTC"}))
            .compile()
            .unwrap();
        let t = utc("2090-06-15T12:34:20Z");
        assert_eq!(compiled.next_after(t), Some(utc("2090-06-15T12:35:00Z")));
        let monthly = config(json!({"frequency": "months", "day_of_month": 1, "timezone": "UTC"}))
            .compile()
            .unwrap();
        assert_eq!(monthly.next_after(t), Some(utc("2090-07-01T00:00:00Z")));
    }

    #[test]
    fn approximate_period_per_frequency() {
        let period =
            |value: serde_json::Value| config(value).compile().unwrap().approximate_period();
        assert_eq!(
            period(json!({"frequency": "minutes", "interval": 30})).as_secs(),
            1_800
        );
        assert_eq!(
            period(json!({"frequency": "hours", "interval": 2})).as_secs(),
            7_200
        );
        assert_eq!(
            period(json!({"frequency": "weeks", "days_of_week": [1, 4]})).as_secs(),
            7 * 86_400 / 2
        );
    }
}
