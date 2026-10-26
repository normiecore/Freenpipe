// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! The `meeting_ended` event, shared by everything that ends a meeting (the
//! detectors and the meeting routes).
//!
//! Payload: `meeting_id`, `meeting_end`, `duration_secs` and
//! `transcript_segments` (live segments plus background transcriptions inside
//! the meeting window, as `GET /meetings/:id/transcript` returns them), and
//! `end_reason` when the end has one. Subscribers that gate on short or silent
//! meetings (the summary pipe trigger) read the last two fields. A field that
//! could not be read is `null`, never a guess: the event itself always goes
//! out, because the audio capture gate and live notes wait for it.

use chrono::{DateTime, Utc};
use screenpipe_db::DatabaseManager;
use serde_json::{json, Value};
use tracing::{debug, warn};

/// Event name on the engine bus.
pub const MEETING_ENDED_EVENT: &str = "meeting_ended";

/// Build the payload from what is known about the meeting.
///
/// `duration_secs` is whole seconds between `meeting_start` and
/// `meeting_end` (never negative), or `null` when either is missing or does
/// not parse.
pub fn meeting_ended_payload(
    meeting_id: i64,
    meeting_start: Option<&str>,
    meeting_end: Option<&str>,
    transcript_segments: Option<usize>,
    end_reason: Option<&str>,
) -> Value {
    let duration_secs = match (meeting_start.and_then(parse_ts), meeting_end.and_then(parse_ts)) {
        (Some(start), Some(end)) => Some((end - start).num_seconds().max(0)),
        _ => None,
    };
    let mut payload = json!({
        "meeting_id": meeting_id,
        "meeting_end": meeting_end,
        "duration_secs": duration_secs,
        "transcript_segments": transcript_segments,
    });
    if let Some(reason) = end_reason.map(str::trim).filter(|r| !r.is_empty()) {
        payload["end_reason"] = json!(reason);
    }
    payload
}

fn parse_ts(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Read the ended meeting and count its transcript, then build the payload.
/// Call after the end was written.
pub async fn load_meeting_ended_payload(
    db: &DatabaseManager,
    meeting_id: i64,
    end_reason: Option<&str>,
) -> Value {
    let meeting = match db.get_meeting_by_id(meeting_id).await {
        Ok(meeting) => Some(meeting),
        Err(e) => {
            warn!(
                "meeting_ended: could not read meeting {} for the event payload: {}",
                meeting_id, e
            );
            None
        }
    };
    let segments = match db.list_meeting_transcript_segments(meeting_id).await {
        Ok(rows) => Some(rows.len()),
        Err(e) => {
            warn!(
                "meeting_ended: could not count transcript segments of meeting {}: {}",
                meeting_id, e
            );
            None
        }
    };
    meeting_ended_payload(
        meeting_id,
        meeting.as_ref().map(|m| m.meeting_start.as_str()),
        meeting.as_ref().and_then(|m| m.meeting_end.as_deref()),
        segments,
        end_reason,
    )
}

/// Publish `meeting_ended` for a meeting whose end was just written.
pub async fn emit_meeting_ended(db: &DatabaseManager, meeting_id: i64, end_reason: Option<&str>) {
    let payload = load_meeting_ended_payload(db, meeting_id, end_reason).await;
    debug!("meeting_ended: {}", payload);
    if let Err(e) = screenpipe_events::send_event(MEETING_ENDED_EVENT, payload) {
        warn!("failed to emit meeting_ended event: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_carries_duration_and_segment_count() {
        let p = meeting_ended_payload(
            7,
            Some("2026-10-26T10:00:00.000Z"),
            Some("2026-10-26T10:31:15.500Z"),
            Some(42),
            None,
        );
        assert_eq!(p["meeting_id"], 7);
        assert_eq!(p["meeting_end"], "2026-10-26T10:31:15.500Z");
        assert_eq!(p["duration_secs"], 1875);
        assert_eq!(p["transcript_segments"], 42);
        assert!(p.get("end_reason").is_none());
    }

    #[test]
    fn unknown_parts_are_null_and_reason_is_kept() {
        let p = meeting_ended_payload(3, Some("garbage"), None, None, Some("auto_end"));
        assert_eq!(p["meeting_id"], 3);
        assert!(p["meeting_end"].is_null());
        assert!(p["duration_secs"].is_null());
        assert!(p["transcript_segments"].is_null());
        assert_eq!(p["end_reason"], "auto_end");

        // A clock that ran backwards never yields a negative duration.
        let p = meeting_ended_payload(
            4,
            Some("2026-10-26T10:00:05Z"),
            Some("2026-10-26T10:00:00Z"),
            Some(0),
            Some("  "),
        );
        assert_eq!(p["duration_secs"], 0);
        assert!(p.get("end_reason").is_none());
    }
}
