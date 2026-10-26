// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Tests for event-run claims, run outcome checks and the meeting-summary
//! finalizer, through the real scheduler where it matters.
//!
//! The event bus is process-wide and tests run in parallel, so every test
//! uses its own pipe name and meeting id and only looks at its own rows.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use serde_json::json;

use super::event_runs::{EventClaim, RunSlot};
use super::hooks::{PipeHooks, RunEnd, RunStart};
use super::meeting_summary::finalizer::{
    Finalized, ERROR_NOTHING_TO_SUMMARIZE, ERROR_SUMMARY_NOT_SAVED, MAX_SUMMARY_ATTEMPTS,
};
use super::meeting_summary::{
    recovery_trigger_key, MeetingSnapshot, MeetingSummaryFinalizer, MeetingSummaryNotes,
    SummaryRefreshRequest, SummarySave, MEETING_SUMMARY_PIPE, SUMMARY_REFRESH_EVENT,
};
use super::outcome::ERROR_NO_FINAL_RESULT;
use super::test_support::{
    finished_answer, write_event_pipe, MemoryPipeStore, SchedulerReady, ScriptedAgent,
};
use super::{PipeManager, PipeRunLog, PipeStore};
use crate::agents::AgentExecutor;

static NEXT_ID: AtomicI64 = AtomicI64::new(0);

/// A meeting id no other test uses.
fn unique_id() -> i64 {
    let base = (std::process::id() as i64 % 10_000) * 1_000_000 + 700_000_000;
    base + NEXT_ID.fetch_add(1, Ordering::SeqCst)
}

async fn wait_until(limit: Duration, check: impl Fn() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    check()
}

fn meeting_ended(meeting_id: i64) {
    screenpipe_events::send_event("meeting_ended", json!({ "meeting_id": meeting_id })).unwrap();
}

/// Gate that only lets this test's meetings start pipes. Every test pipe
/// listens to `meeting_ended`, and a pipe busy with another test's meeting
/// would drop this test's event (a busy pipe skips events).
fn only_meetings(owned: &[i64]) -> super::EventGate {
    let owned: Arc<Vec<i64>> = Arc::new(owned.to_vec());
    Arc::new(move |_name, data| {
        let owned = owned.clone();
        Box::pin(async move {
            let id = data.get("meeting_id").and_then(|v| v.as_i64());
            if id.is_some_and(|id| owned.contains(&id)) {
                None
            } else {
                Some("another test's meeting".to_string())
            }
        })
    })
}

/// A started scheduler with one `meeting_ended` pipe on `agent` that only
/// reacts to the `owned` meetings.
async fn scheduler(
    dir: &std::path::Path,
    pipe: &str,
    store: Arc<MemoryPipeStore>,
    agent: Arc<ScriptedAgent>,
    owned: &[i64],
    configure: impl FnOnce(&mut PipeManager),
) -> PipeManager {
    let pipes_dir = write_event_pipe(dir, pipe, "scripted");
    let mut executors: HashMap<String, Arc<dyn AgentExecutor>> = HashMap::new();
    executors.insert("scripted".to_string(), agent);
    let store: Arc<dyn PipeStore> = store;
    let mut manager = PipeManager::new(pipes_dir, executors, Some(store), 3035);
    manager.scheduler_tick = Duration::from_millis(40);
    manager.set_event_gate(only_meetings(owned));
    configure(&mut manager);
    let ready = Arc::new(SchedulerReady::default());
    manager.add_run_hook(ready.clone());
    manager.load_pipes().await.unwrap();
    manager.start_scheduler().await.unwrap();
    // Events sent before the scheduler task subscribes are not delivered.
    assert!(
        wait_until(Duration::from_secs(20), || ready
            .ready
            .load(Ordering::SeqCst))
        .await,
        "scheduler never subscribed"
    );
    manager
}

fn finished(store: &MemoryPipeStore, key: &str) -> bool {
    store
        .executions_for_key(key)
        .iter()
        .any(|e| e.finished_at.is_some())
}

// ── Idempotent event runs ────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_meeting_ended_events_for_one_meeting_run_the_pipe_once() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w306-once-{}", unique_id());
    let store = Arc::new(MemoryPipeStore::default());
    let agent = Arc::new(ScriptedAgent::new(&finished_answer("Saved."), true));
    let meeting = unique_id();
    let key = meeting.to_string();
    let mut manager = scheduler(
        dir.path(),
        &pipe,
        store.clone(),
        agent.clone(),
        &[meeting],
        |_| {},
    )
    .await;

    // Both deliveries land in one pass ...
    meeting_ended(meeting);
    meeting_ended(meeting);
    assert!(wait_until(Duration::from_secs(10), || finished(&store, &key)).await);
    // ... and a rejoined meeting ending again after the run is done.
    meeting_ended(meeting);
    tokio::time::sleep(Duration::from_millis(400)).await;

    let runs = store.executions_for_key(&key);
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].status, "completed");
    assert_eq!(runs[0].trigger_type, "event");
    assert_eq!(runs[0].trigger_event.as_deref(), Some("meeting_ended"));
    assert_eq!(agent.runs_mentioning(&format!("instance `{key}`")), 1);
    assert!(store.is_claimed(&pipe, "meeting_ended", &key));

    manager.stop_scheduler().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_meeting_that_ends_while_the_pipe_is_busy_still_gets_its_run() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w306-busy-{}", unique_id());
    let store = Arc::new(MemoryPipeStore::default());
    let mut agent = ScriptedAgent::new(&finished_answer("Saved."), true);
    // Long enough for the second meeting to end while the first runs.
    agent.delay = Duration::from_millis(600);
    let agent = Arc::new(agent);
    let first = unique_id();
    let second = unique_id();
    let mut manager = scheduler(
        dir.path(),
        &pipe,
        store.clone(),
        agent.clone(),
        &[first, second],
        |_| {},
    )
    .await;

    meeting_ended(first);
    let first_key = first.to_string();
    assert!(
        wait_until(Duration::from_secs(10), || !store
            .executions_for_key(&first_key)
            .is_empty())
        .await
    );
    meeting_ended(second);
    let second_key = second.to_string();
    assert!(wait_until(Duration::from_secs(15), || finished(&store, &second_key)).await);
    assert!(finished(&store, &first_key));
    assert_eq!(store.executions_for_key(&first_key).len(), 1);
    assert_eq!(store.executions_for_key(&second_key).len(), 1);
    assert_eq!(
        agent.runs_mentioning(&format!("instance `{second_key}`")),
        1
    );

    manager.stop_scheduler().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_schedulers_racing_on_one_store_run_the_pipe_once() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let pipe = format!("w306-race-{}", unique_id());
    let store = Arc::new(MemoryPipeStore::default());
    let agent_a = Arc::new(ScriptedAgent::new(&finished_answer("A"), true));
    let agent_b = Arc::new(ScriptedAgent::new(&finished_answer("B"), true));
    let meeting = unique_id();
    let key = meeting.to_string();
    let mut a = scheduler(
        dir_a.path(),
        &pipe,
        store.clone(),
        agent_a.clone(),
        &[meeting],
        |_| {},
    )
    .await;
    let mut b = scheduler(
        dir_b.path(),
        &pipe,
        store.clone(),
        agent_b.clone(),
        &[meeting],
        |_| {},
    )
    .await;

    meeting_ended(meeting);
    assert!(wait_until(Duration::from_secs(10), || finished(&store, &key)).await);
    tokio::time::sleep(Duration::from_millis(400)).await;

    let needle = format!("instance `{key}`");
    assert_eq!(
        agent_a.runs_mentioning(&needle) + agent_b.runs_mentioning(&needle),
        1
    );
    assert_eq!(store.executions_for_key(&key).len(), 1);

    a.stop_scheduler().await;
    b.stop_scheduler().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_a_queued_run_releases_its_claim() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w306-cancel-{}", unique_id());
    let store = Arc::new(MemoryPipeStore::default());
    let agent = Arc::new(ScriptedAgent::new(&finished_answer("Saved."), true));
    let meeting = unique_id();
    let key = meeting.to_string();
    let needle = format!("instance `{key}`");
    let mut manager = scheduler(
        dir.path(),
        &pipe,
        store.clone(),
        agent.clone(),
        &[meeting],
        |_| {},
    )
    .await;

    // Hold the run in its queued state.
    let gate = store.running_gate.write().await;
    meeting_ended(meeting);
    let slots = manager.run_slots.clone();
    let queued = wait_until(Duration::from_secs(10), || {
        !store.executions_for_key(&key).is_empty()
            && slots
                .try_lock()
                .map(|s| s.contains_key(&pipe))
                .unwrap_or(false)
    })
    .await;
    assert!(queued, "run never queued");
    assert!(store.is_claimed(&pipe, "meeting_ended", &key));
    assert_eq!(store.executions_for_key(&key)[0].status, "queued");

    manager.stop_pipe(&pipe).await.unwrap();
    assert!(!store.is_claimed(&pipe, "meeting_ended", &key));
    drop(gate);

    tokio::time::sleep(Duration::from_millis(400)).await;
    let rows = store.executions_for_key(&key);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "cancelled");
    assert_eq!(agent.runs_mentioning(&needle), 0, "a cancelled run started");
    assert!(!manager.run_slots.lock().await.contains_key(&pipe));

    // The instance was given back, so the next delivery runs it.
    meeting_ended(meeting);
    assert!(
        wait_until(Duration::from_secs(10), || {
            store
                .executions_for_key(&key)
                .iter()
                .any(|e| e.status == "completed")
        })
        .await
    );
    assert_eq!(agent.runs_mentioning(&needle), 1);

    manager.stop_scheduler().await;
}

#[tokio::test]
async fn stopping_a_slot_that_has_not_started_cancels_its_row() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w306-slot-{}", unique_id());
    let pipes_dir = write_event_pipe(dir.path(), &pipe, "scripted");
    let store = Arc::new(MemoryPipeStore::default());
    let manager = PipeManager::new(pipes_dir, HashMap::new(), Some(store.clone()), 3035);
    manager.load_pipes().await.unwrap();

    assert!(store
        .claim_event_run(&pipe, "meeting_ended", "5")
        .await
        .unwrap());
    let id = store
        .create_execution(&pipe, "event", "m", None, Some("meeting_ended"), Some("5"))
        .await
        .unwrap();
    let slot = RunSlot::new(
        Some(id),
        Some(EventClaim {
            pipe: pipe.clone(),
            event: "meeting_ended".into(),
            key: "5".into(),
        }),
    );
    manager
        .run_slots
        .lock()
        .await
        .insert(pipe.clone(), slot.clone());

    manager.stop_pipe(&pipe).await.unwrap();
    assert!(slot.is_cancelled());
    assert!(!store.is_claimed(&pipe, "meeting_ended", "5"));
    assert_eq!(
        store.get_execution(id).await.unwrap().unwrap().status,
        "cancelled"
    );
    // A stale worker cannot reopen it.
    assert!(!store.set_execution_running(id, Some(9)).await.unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_clean_exit_without_a_final_answer_is_recorded_as_failed() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w306-nofinal-{}", unique_id());
    let store = Arc::new(MemoryPipeStore::default());
    // Exit code 0, but the stream stops after a tool call.
    let tool_only = json!({
        "type": "message_end",
        "message": {"role": "assistant", "content": [{"type": "toolCall", "id": "c1", "name": "bash", "arguments": {}}], "stopReason": "toolUse"}
    })
    .to_string();
    let agent = Arc::new(ScriptedAgent::new(&tool_only, true));
    let meeting = unique_id();
    let key = meeting.to_string();
    let mut manager = scheduler(dir.path(), &pipe, store.clone(), agent, &[meeting], |_| {}).await;

    meeting_ended(meeting);
    assert!(wait_until(Duration::from_secs(10), || finished(&store, &key)).await);
    let row = &store.executions_for_key(&key)[0];
    assert_eq!(row.status, "failed");
    assert_eq!(row.error_type.as_deref(), Some(ERROR_NO_FINAL_RESULT));

    manager.stop_scheduler().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_event_gate_keeps_trivial_events_from_starting_pipes() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w306-gate-{}", unique_id());
    let store = Arc::new(MemoryPipeStore::default());
    let agent = Arc::new(ScriptedAgent::new(&finished_answer("ok"), true));
    let meeting = unique_id();
    let key = meeting.to_string();
    let mut manager = scheduler(
        dir.path(),
        &pipe,
        store.clone(),
        agent.clone(),
        &[meeting],
        |m| {
            let own = only_meetings(&[meeting]);
            m.set_event_gate(Arc::new(move |name, data| {
                let own = own.clone();
                Box::pin(async move {
                    if data.get("trivial") == Some(&json!(true)) {
                        return Some("trivial meeting".to_string());
                    }
                    own(name, data).await
                })
            }));
        },
    )
    .await;

    screenpipe_events::send_event(
        "meeting_ended",
        json!({"meeting_id": meeting, "trivial": true}),
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(store.executions_for_key(&key).is_empty());
    // Nothing was claimed, so the same meeting can still run later.
    assert!(!store.is_claimed(&pipe, "meeting_ended", &key));

    meeting_ended(meeting);
    assert!(wait_until(Duration::from_secs(10), || finished(&store, &key)).await);

    manager.stop_scheduler().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refresh_request_runs_the_pipe_once_under_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w306-refresh-{}", unique_id());
    let store = Arc::new(MemoryPipeStore::default());
    let agent = Arc::new(ScriptedAgent::new(&finished_answer("ok"), true));
    // Refresh requests name the pipe; no meeting event may start it.
    let mut manager = scheduler(dir.path(), &pipe, store.clone(), agent.clone(), &[], |_| {}).await;

    let meeting = unique_id();
    let key = recovery_trigger_key(meeting, 1);
    let request = SummaryRefreshRequest {
        meeting_id: meeting,
        reason: "interrupted_by_restart".into(),
        trigger_key: Some(key.clone()),
        pipe: Some(pipe.clone()),
        transcript_updated_at: None,
    };
    screenpipe_events::send_event(SUMMARY_REFRESH_EVENT, request.clone()).unwrap();
    assert!(wait_until(Duration::from_secs(10), || finished(&store, &key)).await);
    // The same request again is a duplicate.
    screenpipe_events::send_event(SUMMARY_REFRESH_EVENT, request).unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;

    let rows = store.executions_for_key(&key);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].trigger_event.as_deref(), Some("meeting_ended"));
    assert_eq!(agent.runs_mentioning(&format!("instance `{key}`")), 1);
    assert!(store.is_claimed(&pipe, "meeting_ended", &key));

    manager.stop_scheduler().await;
}

// ── Meeting-summary finalizer ────────────────────────────────────────────

#[derive(Default)]
struct FakeNotes {
    meetings: StdMutex<HashMap<i64, MeetingSnapshot>>,
    segments: StdMutex<HashMap<i64, u64>>,
}

impl FakeNotes {
    fn with_meeting(id: i64, note: Option<&str>, segments: u64) -> Self {
        let notes = Self::default();
        notes.meetings.lock().unwrap().insert(
            id,
            MeetingSnapshot {
                note: note.map(str::to_string),
                meeting_start: Some((Utc::now() - chrono::Duration::minutes(40)).to_rfc3339()),
                meeting_end: Some((Utc::now() - chrono::Duration::minutes(10)).to_rfc3339()),
            },
        );
        notes.segments.lock().unwrap().insert(id, segments);
        notes
    }

    fn note(&self, id: i64) -> Option<String> {
        self.meetings.lock().unwrap().get(&id)?.note.clone()
    }

    fn set_note(&self, id: i64, note: &str) {
        if let Some(m) = self.meetings.lock().unwrap().get_mut(&id) {
            m.note = Some(note.to_string());
        }
    }
}

#[async_trait::async_trait]
impl MeetingSummaryNotes for FakeNotes {
    async fn meeting(&self, meeting_id: i64) -> Result<Option<MeetingSnapshot>> {
        Ok(self.meetings.lock().unwrap().get(&meeting_id).cloned())
    }

    async fn transcript_segment_count(&self, meeting_id: i64) -> Result<u64> {
        Ok(*self.segments.lock().unwrap().get(&meeting_id).unwrap_or(&0))
    }

    async fn save_summary(&self, meeting_id: i64, summary: &str) -> Result<SummarySave> {
        if summary.trim().is_empty() || summary.to_lowercase().starts_with("no transcript") {
            return Ok(SummarySave::Rejected("not a summary".into()));
        }
        let mut meetings = self.meetings.lock().unwrap();
        let Some(m) = meetings.get_mut(&meeting_id) else {
            return Ok(SummarySave::MeetingMissing);
        };
        let note = m.note.clone().unwrap_or_default();
        m.note = Some(format!("{note}\n\n## Summary\n{}", summary.trim()));
        Ok(SummarySave::Saved)
    }
}

/// Fixture: a summary run that printed its summary and stopped before the
/// save request.
fn printed_summary_stdout() -> String {
    [
        r#"{"type":"session","id":"s"}"#.to_string(),
        json!({
            "type": "message_end",
            "message": {"role": "assistant", "content": [
                {"type": "text", "text": "Reading the transcript."},
                {"type": "toolCall", "id": "t1", "name": "bash", "arguments": {"command": "curl /meetings/1/transcript"}}
            ], "stopReason": "toolUse"}
        })
        .to_string(),
        finished_answer("## Summary\nThe group chose the March launch date.\n\n**Action items**\n- Lee: draft the announcement"),
    ]
    .join("\n")
}

fn completed_run(store: &MemoryPipeStore, key: &str) -> i64 {
    store.insert_finished(
        MEETING_SUMMARY_PIPE,
        "completed",
        None,
        Some(key),
        Utc::now(),
    )
}

fn finalizer(store: &Arc<MemoryPipeStore>, notes: &Arc<FakeNotes>) -> MeetingSummaryFinalizer {
    let store: Arc<dyn PipeStore> = store.clone();
    let notes: Arc<dyn MeetingSummaryNotes> = notes.clone();
    MeetingSummaryFinalizer::new(store, notes)
}

#[tokio::test]
async fn finalizer_saves_a_summary_the_run_printed_but_did_not_store() {
    let meeting = unique_id();
    let store = Arc::new(MemoryPipeStore::default());
    let notes = Arc::new(FakeNotes::with_meeting(meeting, Some("my own notes"), 12));
    let id = completed_run(&store, &meeting.to_string());

    let result = finalizer(&store, &notes)
        .finalize(id, meeting, Some(None), &printed_summary_stdout())
        .await
        .unwrap();
    assert_eq!(result, Finalized::Recovered);
    let note = notes.note(meeting).unwrap();
    assert!(note.starts_with("my own notes"));
    assert!(note.contains("## Summary\nThe group chose the March launch date."));
    assert!(note.contains("- Lee: draft the announcement"));
    assert_eq!(
        store.get_execution(id).await.unwrap().unwrap().status,
        "completed"
    );
}

#[tokio::test]
async fn finalizer_marks_a_run_that_saved_nothing() {
    let meeting = unique_id();
    let store = Arc::new(MemoryPipeStore::default());
    let notes = Arc::new(FakeNotes::with_meeting(meeting, None, 30));
    let id = completed_run(&store, &meeting.to_string());

    let result = finalizer(&store, &notes)
        .finalize(id, meeting, Some(None), &finished_answer("I looked at it."))
        .await
        .unwrap();
    assert_eq!(result, Finalized::NotSaved);
    let row = store.get_execution(id).await.unwrap().unwrap();
    assert_eq!(row.status, "failed");
    assert_eq!(row.error_type.as_deref(), Some(ERROR_SUMMARY_NOT_SAVED));
}

#[tokio::test]
async fn finalizer_marks_a_silent_meeting_nothing_to_summarize() {
    let meeting = unique_id();
    let store = Arc::new(MemoryPipeStore::default());
    let notes = Arc::new(FakeNotes::with_meeting(meeting, None, 0));
    let id = completed_run(&store, &meeting.to_string());

    // The printed section is a placeholder the save refuses.
    let result = finalizer(&store, &notes)
        .finalize(
            id,
            meeting,
            Some(None),
            &finished_answer("## Summary\nNo transcript was captured."),
        )
        .await
        .unwrap();
    assert_eq!(result, Finalized::NothingToSummarize);
    let row = store.get_execution(id).await.unwrap().unwrap();
    assert_eq!(row.error_type.as_deref(), Some(ERROR_NOTHING_TO_SUMMARIZE));
    assert_eq!(notes.note(meeting), None);
}

#[tokio::test]
async fn finalizer_leaves_saved_summaries_and_failed_runs_alone() {
    let meeting = unique_id();
    let key = meeting.to_string();
    let store = Arc::new(MemoryPipeStore::default());
    let notes = Arc::new(FakeNotes::with_meeting(
        meeting,
        Some("## Summary\nold summary"),
        5,
    ));
    let hook = Arc::new(finalizer(&store, &notes));
    let mut hooks = PipeHooks::new();
    hooks.register(hook);

    // The agent replaces the summary during the run: nothing else happens.
    let id = completed_run(&store, &key);
    let dir = std::path::PathBuf::from("/tmp");
    let start = RunStart {
        pipe_name: MEETING_SUMMARY_PIPE,
        trigger: "event",
        trigger_event: Some("meeting_ended"),
        trigger_key: Some(&key),
        execution_id: Some(id),
        pipe_dir: &dir,
    };
    hooks.before_run(&start).await;
    notes.set_note(meeting, "## Summary\nnew summary");
    let log = PipeRunLog {
        pipe_name: MEETING_SUMMARY_PIPE.into(),
        started_at: Utc::now(),
        finished_at: Utc::now(),
        success: true,
        stdout: finished_answer("Saved the summary."),
        stderr: String::new(),
    };
    hooks
        .after_run(&RunEnd {
            start,
            log: &log,
            error_type: None,
        })
        .await;
    assert_eq!(
        store.get_execution(id).await.unwrap().unwrap().status,
        "completed"
    );

    // Same section before and after, nothing printed: not saved.
    let id2 = completed_run(&store, &key);
    let start2 = RunStart {
        execution_id: Some(id2),
        ..start
    };
    hooks.before_run(&start2).await;
    hooks
        .after_run(&RunEnd {
            start: start2,
            log: &log,
            error_type: None,
        })
        .await;
    assert_eq!(
        store
            .get_execution(id2)
            .await
            .unwrap()
            .unwrap()
            .error_type
            .as_deref(),
        Some(ERROR_SUMMARY_NOT_SAVED)
    );

    // A failed run keeps its own failure.
    let id3 = store.insert_finished(
        MEETING_SUMMARY_PIPE,
        "failed",
        Some("rate_limited"),
        Some(&key),
        Utc::now(),
    );
    let start3 = RunStart {
        execution_id: Some(id3),
        ..start
    };
    let failed_log = PipeRunLog {
        success: false,
        ..log.clone()
    };
    hooks.before_run(&start3).await;
    hooks
        .after_run(&RunEnd {
            start: start3,
            log: &failed_log,
            error_type: Some("rate_limited"),
        })
        .await;
    assert_eq!(
        store
            .get_execution(id3)
            .await
            .unwrap()
            .unwrap()
            .error_type
            .as_deref(),
        Some("rate_limited")
    );

    // Other pipes are not the finalizer's business.
    let other = store.insert_finished("slack-notes", "completed", None, Some(&key), Utc::now());
    let other_start = RunStart {
        pipe_name: "slack-notes",
        execution_id: Some(other),
        ..start
    };
    hooks.before_run(&other_start).await;
    hooks
        .after_run(&RunEnd {
            start: other_start,
            log: &log,
            error_type: None,
        })
        .await;
    assert_eq!(
        store.get_execution(other).await.unwrap().unwrap().status,
        "completed"
    );
}

#[tokio::test]
async fn interrupted_summary_runs_are_asked_for_again_a_bounded_number_of_times() {
    use futures::StreamExt;

    let mut refresh_rx =
        screenpipe_events::subscribe_to_event::<serde_json::Value>(SUMMARY_REFRESH_EVENT);

    // Meeting A: its only run was interrupted.
    let a = unique_id();
    // Meeting B: interrupted, but a recovery run already followed.
    let b = unique_id();
    // Meeting C: interrupted after using up its attempts.
    let c = unique_id();
    let store = Arc::new(MemoryPipeStore::default());
    let notes = Arc::new(FakeNotes::with_meeting(a, None, 10));
    for id in [b, c] {
        let snapshot = notes.meetings.lock().unwrap()[&a].clone();
        notes.meetings.lock().unwrap().insert(id, snapshot);
    }
    store.insert_finished(
        MEETING_SUMMARY_PIPE,
        "failed",
        Some("interrupted"),
        Some(&a.to_string()),
        Utc::now(),
    );
    store.insert_finished(
        MEETING_SUMMARY_PIPE,
        "failed",
        Some("interrupted"),
        Some(&b.to_string()),
        Utc::now(),
    );
    store.insert_finished(
        MEETING_SUMMARY_PIPE,
        "completed",
        None,
        Some(&recovery_trigger_key(b, 1)),
        Utc::now(),
    );
    for attempt in 0..MAX_SUMMARY_ATTEMPTS {
        let key = if attempt == 0 {
            c.to_string()
        } else {
            recovery_trigger_key(c, attempt)
        };
        store.insert_finished(
            MEETING_SUMMARY_PIPE,
            "failed",
            Some("interrupted"),
            Some(&key),
            Utc::now(),
        );
    }

    let sent = finalizer(&store, &notes)
        .requeue_interrupted()
        .await
        .unwrap();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].meeting_id, a);
    assert_eq!(sent[0].reason, "interrupted_by_restart");
    assert_eq!(
        sent[0].trigger_key.as_deref(),
        Some(recovery_trigger_key(a, 1).as_str())
    );

    let event = loop {
        let event = tokio::time::timeout(Duration::from_secs(5), refresh_rx.next())
            .await
            .expect("refresh event")
            .expect("stream open");
        if event.data["meeting_id"] == json!(a) {
            break event;
        }
    };
    assert_eq!(event.data["pipe"], json!(MEETING_SUMMARY_PIPE));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_summary_run_that_forgot_to_save_is_rescued_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w306-summary-{}", unique_id());
    let meeting = unique_id();
    let key = meeting.to_string();
    let store = Arc::new(MemoryPipeStore::default());
    let notes = Arc::new(FakeNotes::with_meeting(meeting, None, 20));
    let agent = Arc::new(ScriptedAgent::new(&printed_summary_stdout(), true));
    let hook_store: Arc<dyn PipeStore> = store.clone();
    let hook_notes: Arc<dyn MeetingSummaryNotes> = notes.clone();
    let mut manager = scheduler(dir.path(), &pipe, store.clone(), agent, &[meeting], |m| {
        m.add_run_hook(Arc::new(MeetingSummaryFinalizer::for_pipe(
            hook_store, hook_notes, &pipe,
        )));
    })
    .await;

    meeting_ended(meeting);
    assert!(
        wait_until(Duration::from_secs(10), || {
            notes
                .note(meeting)
                .is_some_and(|n| n.contains("March launch date"))
        })
        .await
    );
    let row = &store.executions_for_key(&key)[0];
    assert_eq!(row.status, "completed");

    manager.stop_scheduler().await;
}
