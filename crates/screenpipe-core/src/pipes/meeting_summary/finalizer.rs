// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Checks the outcome of every meeting-summary run.
//!
//! The finalizer is a [`PipeRunHook`]. For each run of the summary pipe that
//! a `meeting_ended` trigger started (directly, or as a recovery or refresh
//! run keyed `<meeting id>:...`):
//!
//! 1. before the run, it notes the meeting's current `## Summary` section;
//! 2. after a completed run, a section that changed means the agent saved
//!    its summary, and nothing more happens;
//! 3. otherwise the last `## Summary` the agent printed is taken from the
//!    run's output and saved through the same note merge the summary route
//!    uses (the save refuses text that reads as an error message);
//! 4. when there is nothing usable, the run is marked failed:
//!    `nothing_to_summarize` when the meeting has no transcript at all,
//!    `summary_not_saved` otherwise. Only runs that completed without an
//!    error are changed.
//!
//! When the scheduler starts, summary runs that the last shutdown or crash
//! interrupted (`error_type = 'interrupted'`, no newer run for the same
//! meeting) are asked for again with [`SUMMARY_REFRESH_EVENT`], under the key
//! `<meeting id>:summary-recovery-<n>`. A meeting gets at most
//! [`MAX_SUMMARY_ATTEMPTS`] runs counted from its end, so a run that crashes
//! the engine every time stops being retried.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use super::extract::{summary_from_run_output, summary_section};
use super::{
    meeting_id_from_trigger_key, recovery_trigger_key, SummaryRefreshRequest,
    MEETING_ENDED_EVENT, MEETING_SUMMARY_PIPE, REFRESH_REASON_INTERRUPTED, SUMMARY_REFRESH_EVENT,
};
use crate::pipes::hooks::{PipeRunHook, RunEnd, RunStart};
use crate::pipes::PipeStore;

/// `error_type` of a completed summary run that left no summary behind.
pub const ERROR_SUMMARY_NOT_SAVED: &str = "summary_not_saved";
/// `error_type` of a summary run for a meeting with no transcript.
pub const ERROR_NOTHING_TO_SUMMARIZE: &str = "nothing_to_summarize";

/// Runs (the first one included) a meeting may get before interrupted runs
/// stop being asked for again.
pub const MAX_SUMMARY_ATTEMPTS: u32 = 4;
/// Interrupted runs looked at per scheduler start.
pub const RECOVERY_SCAN_LIMIT: u32 = 20;
/// Interrupted runs older than this are left alone.
pub const RECOVERY_LOOKBACK_HOURS: i64 = 24;

/// A meeting as the finalizer needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeetingSnapshot {
    pub note: Option<String>,
    pub meeting_start: Option<String>,
    pub meeting_end: Option<String>,
}

/// Result of saving a recovered summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummarySave {
    Saved,
    /// The text read as an error message or placeholder, not a summary.
    Rejected(String),
    MeetingMissing,
}

/// Meeting storage, implemented where the database lives (the engine).
#[async_trait::async_trait]
pub trait MeetingSummaryNotes: Send + Sync {
    /// The meeting, or `None` when it does not exist.
    async fn meeting(&self, meeting_id: i64) -> Result<Option<MeetingSnapshot>>;

    /// Transcript segments recorded for the meeting (live and background).
    async fn transcript_segment_count(&self, meeting_id: i64) -> Result<u64>;

    /// Save `summary` into the meeting note's summary section, after the
    /// same cleaning and checks as `POST /meetings/:id/summary`.
    async fn save_summary(&self, meeting_id: i64, summary: &str) -> Result<SummarySave>;
}

/// The run-lifecycle hook described in the module docs.
pub struct MeetingSummaryFinalizer {
    store: Arc<dyn PipeStore>,
    notes: Arc<dyn MeetingSummaryNotes>,
    pipe_name: String,
    /// Summary section before each run, keyed by execution id.
    before: Mutex<HashMap<i64, Option<String>>>,
}

impl MeetingSummaryFinalizer {
    pub fn new(store: Arc<dyn PipeStore>, notes: Arc<dyn MeetingSummaryNotes>) -> Self {
        Self::for_pipe(store, notes, MEETING_SUMMARY_PIPE)
    }

    pub fn for_pipe(
        store: Arc<dyn PipeStore>,
        notes: Arc<dyn MeetingSummaryNotes>,
        pipe_name: &str,
    ) -> Self {
        Self {
            store,
            notes,
            pipe_name: pipe_name.to_string(),
            before: Mutex::new(HashMap::new()),
        }
    }

    /// The meeting a run summarises, when it is one of ours.
    fn meeting_of(&self, run: &RunStart<'_>) -> Option<(i64, i64)> {
        if run.pipe_name != self.pipe_name || run.trigger_event != Some(MEETING_ENDED_EVENT) {
            return None;
        }
        let execution_id = run.execution_id?;
        let meeting_id = meeting_id_from_trigger_key(run.trigger_key?)?;
        Some((execution_id, meeting_id))
    }

    async fn current_section(&self, meeting_id: i64) -> Result<Option<Option<String>>> {
        Ok(self
            .notes
            .meeting(meeting_id)
            .await?
            .map(|m| m.note.as_deref().and_then(summary_section)))
    }

    /// Steps 2 to 4 of the module docs for one finished run.
    pub async fn finalize(
        &self,
        execution_id: i64,
        meeting_id: i64,
        before: Option<Option<String>>,
        stdout: &str,
    ) -> Result<Finalized> {
        let Some(after) = self.current_section(meeting_id).await? else {
            return Ok(Finalized::MeetingMissing);
        };
        let saved = match (&before, &after) {
            (_, None) => false,
            (Some(previous), Some(now)) => previous.as_ref() != Some(now),
            // No snapshot (the run started before this hook was attached):
            // any section counts.
            (None, Some(_)) => true,
        };
        if saved {
            return Ok(Finalized::SummarySaved);
        }

        if let Some(text) = summary_from_run_output(stdout) {
            match self.notes.save_summary(meeting_id, &text).await? {
                SummarySave::Saved => {
                    info!(
                        "meeting summary: saved the summary run {} printed but did not store (meeting {})",
                        execution_id, meeting_id
                    );
                    return Ok(Finalized::Recovered);
                }
                SummarySave::MeetingMissing => return Ok(Finalized::MeetingMissing),
                SummarySave::Rejected(why) => {
                    debug!(
                        "meeting summary: printed section of run {} not usable: {}",
                        execution_id, why
                    );
                }
            }
        }

        let segments = self.notes.transcript_segment_count(meeting_id).await?;
        let (error_type, message) = if segments == 0 {
            (
                ERROR_NOTHING_TO_SUMMARIZE,
                "no speech was captured in this meeting, so there was nothing to summarize",
            )
        } else {
            (
                ERROR_SUMMARY_NOT_SAVED,
                "the summary run finished without saving a summary to the meeting note",
            )
        };
        let changed = self
            .store
            .fail_completed_execution(execution_id, error_type, message)
            .await?;
        if changed {
            warn!(
                "meeting summary: run {} for meeting {} marked {}",
                execution_id, meeting_id, error_type
            );
            Ok(if segments == 0 {
                Finalized::NothingToSummarize
            } else {
                Finalized::NotSaved
            })
        } else {
            Ok(Finalized::LeftAsIs)
        }
    }

    /// Ask again for summary runs a restart interrupted. Returns the
    /// requests sent.
    pub async fn requeue_interrupted(&self) -> Result<Vec<SummaryRefreshRequest>> {
        let since = Utc::now() - Duration::hours(RECOVERY_LOOKBACK_HOURS);
        let runs = self
            .store
            .interrupted_event_runs(&self.pipe_name, MEETING_ENDED_EVENT, since, RECOVERY_SCAN_LIMIT)
            .await?;
        let mut sent = Vec::new();
        for run in runs {
            let Some(key) = run.trigger_key.as_deref() else {
                continue;
            };
            let Some(meeting_id) = meeting_id_from_trigger_key(key) else {
                continue;
            };
            let Some(meeting) = self.notes.meeting(meeting_id).await? else {
                continue;
            };
            let counted_from = meeting
                .meeting_end
                .as_deref()
                .or(meeting.meeting_start.as_deref())
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&Utc) - Duration::seconds(1))
                .unwrap_or(since);
            let attempts = self
                .store
                .count_event_runs(
                    &self.pipe_name,
                    MEETING_ENDED_EVENT,
                    &meeting_id.to_string(),
                    counted_from,
                )
                .await?;
            if attempts >= MAX_SUMMARY_ATTEMPTS {
                warn!(
                    "meeting summary: meeting {} already had {} summary runs; not retrying the one a restart interrupted",
                    meeting_id, attempts
                );
                continue;
            }
            let request = SummaryRefreshRequest {
                meeting_id,
                reason: REFRESH_REASON_INTERRUPTED.to_string(),
                trigger_key: Some(recovery_trigger_key(meeting_id, attempts.max(1))),
                pipe: Some(self.pipe_name.clone()),
                transcript_updated_at: None,
            };
            if let Err(e) = screenpipe_events::send_event(SUMMARY_REFRESH_EVENT, request.clone()) {
                warn!(
                    "meeting summary: could not ask again for meeting {}: {}",
                    meeting_id, e
                );
                continue;
            }
            info!(
                "meeting summary: asking again for meeting {} (run {} was interrupted by a restart)",
                meeting_id, run.id
            );
            sent.push(request);
        }
        Ok(sent)
    }
}

/// What the finalizer did with one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finalized {
    SummarySaved,
    Recovered,
    NotSaved,
    NothingToSummarize,
    /// The run was not a clean completion, so its status was kept.
    LeftAsIs,
    MeetingMissing,
}

#[async_trait::async_trait]
impl PipeRunHook for MeetingSummaryFinalizer {
    fn name(&self) -> &str {
        "meeting-summary-finalizer"
    }

    async fn before_run(&self, run: &RunStart<'_>) {
        let Some((execution_id, meeting_id)) = self.meeting_of(run) else {
            return;
        };
        match self.current_section(meeting_id).await {
            Ok(Some(section)) => {
                self.before.lock().await.insert(execution_id, section);
            }
            Ok(None) => {}
            Err(e) => warn!(
                "meeting summary: could not read meeting {} before run {}: {}",
                meeting_id, execution_id, e
            ),
        }
    }

    async fn after_run(&self, run: &RunEnd<'_>) {
        let Some((execution_id, meeting_id)) = self.meeting_of(&run.start) else {
            return;
        };
        let before = self.before.lock().await.remove(&execution_id);
        if !run.log.success || run.error_type.is_some() {
            return;
        }
        if let Err(e) = self
            .finalize(execution_id, meeting_id, before, &run.log.stdout)
            .await
        {
            warn!(
                "meeting summary: could not check run {} for meeting {}: {}",
                execution_id, meeting_id, e
            );
        }
    }

    async fn on_scheduler_start(&self) {
        if let Err(e) = self.requeue_interrupted().await {
            warn!("meeting summary: interrupted-run recovery failed: {}", e);
        }
    }
}
