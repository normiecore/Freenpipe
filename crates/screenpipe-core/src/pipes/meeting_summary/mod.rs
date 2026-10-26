// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Meeting summaries produced by the `meeting-summary` pipe.
//!
//! When a meeting ends, the scheduler runs the summary pipe once per meeting
//! (the run is claimed in `pipe_event_runs` under the meeting id). This
//! module holds what happens around those runs:
//!
//! - [`extract`] finds a `## Summary` section in a note or in a run's output;
//! - [`finalizer`] checks every finished summary run: it saves a summary the
//!   agent printed but never stored, or marks the run `summary_not_saved` /
//!   `nothing_to_summarize`, and on startup asks again for summaries whose
//!   run was cut short by a restart;
//! - the shared names below: trigger keys, the refresh request event and the
//!   rule that keeps trivial meetings from starting event-triggered pipes.
//!
//! The summary is written by whatever AI preset the pipe uses (the user's own
//! default unless the pipe names one). Nothing here talks to a model.

pub mod extract;
pub mod finalizer;

pub use finalizer::{MeetingSnapshot, MeetingSummaryFinalizer, MeetingSummaryNotes, SummarySave};

use serde::{Deserialize, Serialize};

/// Name of the bundled meeting-summary pipe.
pub const MEETING_SUMMARY_PIPE: &str = "meeting-summary";

/// Event the summary pipe is triggered by. Recovery and refresh runs record
/// it as their `trigger_event` too, under a suffixed key.
pub const MEETING_ENDED_EVENT: &str = "meeting_ended";

/// Event that asks the scheduler to run the summary pipe again for one
/// meeting (after a retranscription, or for a run a restart interrupted).
pub const SUMMARY_REFRESH_EVENT: &str = "meeting_summary_refresh_requested";

/// `reason` of a refresh asked for after a run was interrupted by a restart.
pub const REFRESH_REASON_INTERRUPTED: &str = "interrupted_by_restart";
/// `reason` of a refresh asked for after the transcript was replaced.
pub const REFRESH_REASON_RETRANSCRIBED: &str = "retranscribed";

/// A meeting this short with this few transcript segments is trivial:
/// `meeting_ended` still reaches every listener, but event-triggered pipes
/// do not run for it. Both limits must hold.
pub const TRIVIAL_MEETING_MAX_SECS: f64 = 120.0;
/// See [`TRIVIAL_MEETING_MAX_SECS`].
pub const TRIVIAL_MEETING_MAX_SEGMENTS: u64 = 2;

/// Body of [`SUMMARY_REFRESH_EVENT`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryRefreshRequest {
    pub meeting_id: i64,
    /// Why the refresh is asked for ([`REFRESH_REASON_INTERRUPTED`],
    /// [`REFRESH_REASON_RETRANSCRIBED`], or another short tag).
    pub reason: String,
    /// Trigger key the run is claimed and recorded under. Derived from the
    /// meeting id and the time when missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_key: Option<String>,
    /// Pipe to run; the bundled summary pipe when missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipe: Option<String>,
    /// When the transcript changed, for refreshes after a retranscription.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_updated_at: Option<String>,
}

impl SummaryRefreshRequest {
    pub fn pipe_name(&self) -> &str {
        self.pipe
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .unwrap_or(MEETING_SUMMARY_PIPE)
    }

    /// The key this request's run is claimed under.
    pub fn key(&self) -> String {
        match self
            .trigger_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            Some(k) => k.to_string(),
            None => refresh_trigger_key(
                self.meeting_id,
                chrono::Utc::now().timestamp_millis(),
            ),
        }
    }
}

/// Meeting id of a summary run's trigger key: the key itself, or the part
/// before the first `:` for recovery and refresh keys.
pub fn meeting_id_from_trigger_key(key: &str) -> Option<i64> {
    key.split(':').next()?.trim().parse().ok()
}

/// Trigger key of the `attempt`-th run asked for again after a restart.
pub fn recovery_trigger_key(meeting_id: i64, attempt: u32) -> String {
    format!("{meeting_id}:summary-recovery-{attempt}")
}

/// Trigger key of a refresh asked for at `at_ms` (Unix milliseconds).
pub fn refresh_trigger_key(meeting_id: i64, at_ms: i64) -> String {
    format!("{meeting_id}:summary-refresh-{at_ms}")
}

/// Why a meeting is too slight to start event-triggered pipes, or `None`.
/// Unknown values never make a meeting trivial: a batch transcription can
/// still be catching up when the meeting ends, so a missing count is not
/// read as silence.
pub fn trivial_meeting_reason(duration_secs: Option<f64>, segments: Option<u64>) -> Option<String> {
    let duration = duration_secs.filter(|d| d.is_finite() && *d >= 0.0)?;
    let segments = segments?;
    (duration < TRIVIAL_MEETING_MAX_SECS && segments <= TRIVIAL_MEETING_MAX_SEGMENTS).then(|| {
        format!(
            "trivial meeting ({} transcript segment{} in {:.0}s)",
            segments,
            if segments == 1 { "" } else { "s" },
            duration
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_keys_round_trip_to_the_meeting_id() {
        assert_eq!(meeting_id_from_trigger_key("42"), Some(42));
        assert_eq!(
            meeting_id_from_trigger_key(&recovery_trigger_key(42, 2)),
            Some(42)
        );
        assert_eq!(recovery_trigger_key(42, 2), "42:summary-recovery-2");
        assert_eq!(
            meeting_id_from_trigger_key(&refresh_trigger_key(7, 1_700_000_000_000)),
            Some(7)
        );
        assert_eq!(meeting_id_from_trigger_key("abc"), None);
        assert_eq!(meeting_id_from_trigger_key(""), None);
    }

    #[test]
    fn trivial_needs_both_a_short_meeting_and_few_segments() {
        assert!(trivial_meeting_reason(Some(40.0), Some(0)).is_some());
        assert!(trivial_meeting_reason(Some(119.0), Some(2)).is_some());
        assert_eq!(trivial_meeting_reason(Some(119.0), Some(3)), None);
        assert_eq!(trivial_meeting_reason(Some(600.0), Some(0)), None);
        // Unknown inputs are never trivial.
        assert_eq!(trivial_meeting_reason(None, Some(0)), None);
        assert_eq!(trivial_meeting_reason(Some(10.0), None), None);
        assert_eq!(trivial_meeting_reason(Some(f64::NAN), Some(0)), None);
        assert_eq!(
            trivial_meeting_reason(Some(30.0), Some(1)).unwrap(),
            "trivial meeting (1 transcript segment in 30s)"
        );
    }

    #[test]
    fn refresh_request_defaults() {
        let req: SummaryRefreshRequest =
            serde_json::from_value(serde_json::json!({"meeting_id": 5, "reason": "retranscribed"}))
                .unwrap();
        assert_eq!(req.pipe_name(), MEETING_SUMMARY_PIPE);
        assert!(req.key().starts_with("5:summary-refresh-"));
        let req = SummaryRefreshRequest {
            meeting_id: 5,
            reason: REFRESH_REASON_INTERRUPTED.into(),
            trigger_key: Some(recovery_trigger_key(5, 1)),
            pipe: Some("my-summary".into()),
            transcript_updated_at: None,
        };
        assert_eq!(req.key(), "5:summary-recovery-1");
        assert_eq!(req.pipe_name(), "my-summary");
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("transcript_updated_at").is_none());
    }
}
