// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Ids that name one occurrence of a calendar event.
//!
//! The OS calendars give every occurrence of a recurring event the id of the
//! whole series: EventKit's `eventIdentifier` and the Windows appointment
//! `LocalId` repeat for each week of a weekly meeting, and an ICS `UID` repeats
//! for every `RECURRENCE-ID` instance a feed lists. Meetings are bound to a
//! calendar entry by id (`meetings.calendar_event_id`, one meeting per id), so
//! a series id would let the first occurrence claim every later one.
//! [`occurrence_event_id`] appends the occurrence's start, which is how the
//! Google publisher's instance ids already behave.

use chrono::{DateTime, Utc};

/// Id of the occurrence of `series_id` that starts at `start`:
/// `<series id>_<start as UTC, YYYYMMDDTHHMMSSZ>`.
///
/// A blank `series_id` stays blank: an entry without an id has no identity
/// to bind, and the start alone must not become one.
pub fn occurrence_event_id(series_id: &str, start: DateTime<Utc>) -> String {
    let series_id = series_id.trim();
    if series_id.is_empty() {
        return String::new();
    }
    format!("{}_{}", series_id, start.format("%Y%m%dT%H%M%SZ"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn occurrences_of_one_series_get_distinct_ids() {
        let monday = Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap();
        let next_monday = Utc.with_ymd_and_hms(2026, 11, 2, 9, 0, 0).unwrap();
        let a = occurrence_event_id("EK-SERIES:42", monday);
        let b = occurrence_event_id("EK-SERIES:42", next_monday);
        assert_eq!(a, "EK-SERIES:42_20261026T090000Z");
        assert_eq!(b, "EK-SERIES:42_20261102T090000Z");
        assert_ne!(a, b);
    }

    #[test]
    fn the_same_occurrence_always_gets_the_same_id() {
        // Publishers resend their list every minute; the id must not drift.
        let start = Utc.with_ymd_and_hms(2026, 10, 27, 14, 30, 0).unwrap();
        assert_eq!(
            occurrence_event_id(" 123.456 ", start),
            occurrence_event_id("123.456", start)
        );
        // Sub-second noise in the start does not change the id either.
        let jittered = start + chrono::Duration::milliseconds(250);
        assert_eq!(
            occurrence_event_id("123.456", jittered),
            occurrence_event_id("123.456", start)
        );
    }

    #[test]
    fn a_blank_series_id_has_no_occurrence_id() {
        let start = Utc.with_ymd_and_hms(2026, 10, 27, 14, 30, 0).unwrap();
        assert_eq!(occurrence_event_id("", start), "");
        assert_eq!(occurrence_event_id("   ", start), "");
    }
}
