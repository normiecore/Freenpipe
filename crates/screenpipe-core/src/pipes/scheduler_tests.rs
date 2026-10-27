// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Scheduler v2 tests through the real scheduler: deferred and coalesced
//! triggers, the backlog cap, preset chains (`"*"`, missing presets,
//! fallback slots), trigger context files, stale one-offs, trajectories and
//! the per-run log archive.
//!
//! The event bus is process-wide and tests run in parallel, so every test
//! uses its own pipe names, event names and meeting ids.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use chrono::Utc;
use serde_json::{json, Value};

use super::backlog::{SKIP_BACKLOG_FULL, SKIP_DEFERRED_EXPIRED};
use super::event_runs::EventRun;
use super::preset_fallback::{PresetFallbackRegistry, ERROR_AI_PRESET_UNAVAILABLE};
use super::test_support::{finished_answer, MemoryPipeStore, SchedulerReady, ScriptedAgent};
use super::trajectory::TrajectoryConfig;
use super::{
    choose_run_model, pipe_accepts_run, PipeConfig, PipeManager, PipeStore, SKIP_STALE_ONE_OFF,
};
use crate::agents::AgentExecutor;

static NEXT_ID: AtomicI64 = AtomicI64::new(0);

/// An id no other test uses.
fn unique_id() -> i64 {
    let base = (std::process::id() as i64 % 10_000) * 1_000_000 + 900_000_000;
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

/// Write `<dir>/pipes/<name>/pipe.md` with `frontmatter` (YAML lines after
/// the common ones) and return the pipes directory.
fn write_pipe(dir: &Path, name: &str, frontmatter: &str) -> PathBuf {
    let pipes_dir = dir.join("pipes");
    let pipe_dir = pipes_dir.join(name);
    std::fs::create_dir_all(&pipe_dir).unwrap();
    std::fs::write(
        pipe_dir.join("pipe.md"),
        format!("---\nenabled: true\nagent: scripted\n{frontmatter}---\n\nDo the task.\n"),
    )
    .unwrap();
    pipes_dir
}

/// Write the AI settings (`<dir>/store.bin`) with `presets`.
fn write_store(dir: &Path, presets: Value) {
    std::fs::write(
        dir.join("store.bin"),
        serde_json::to_string(&json!({"settings": {"aiPresets": presets}})).unwrap(),
    )
    .unwrap();
}

fn two_presets() -> Value {
    json!([
        {"id": "work", "model": "m-work", "provider": "openai", "url": "https://api.example.invalid/v1"},
        {"id": "local", "model": "m-local", "provider": "native-ollama",
         "url": "http://localhost:11434/v1", "defaultPreset": true}
    ])
}

/// Gate that only lets events carrying one of `owned` meeting ids start
/// pipes (other tests send `meeting_ended` too). Events without a meeting id
/// pass.
fn only_meetings(owned: Vec<i64>) -> super::EventGate {
    let owned = Arc::new(owned);
    Arc::new(move |_name, data| {
        let owned = owned.clone();
        Box::pin(async move {
            match data.get("meeting_id").and_then(|v| v.as_i64()) {
                Some(id) if !owned.contains(&id) => Some("another test's meeting".to_string()),
                _ => None,
            }
        })
    })
}

/// Output lines the manager streamed, as (pipe, line).
type Lines = Arc<StdMutex<Vec<(String, String)>>>;

/// A started scheduler over `pipes_dir` with `agent` as the `scripted`
/// agent.
async fn start(
    pipes_dir: PathBuf,
    store: Arc<MemoryPipeStore>,
    agent: Arc<ScriptedAgent>,
    owned: Vec<i64>,
    configure: impl FnOnce(&mut PipeManager),
) -> (PipeManager, Lines) {
    let mut executors: HashMap<String, Arc<dyn AgentExecutor>> = HashMap::new();
    executors.insert("scripted".to_string(), agent);
    let store: Arc<dyn PipeStore> = store;
    let mut manager = PipeManager::new(pipes_dir, executors, Some(store), 3035);
    manager.scheduler_tick = Duration::from_millis(40);
    manager.set_trajectories(None);
    manager.set_event_gate(only_meetings(owned));
    let lines: Lines = Arc::new(StdMutex::new(Vec::new()));
    let sink = lines.clone();
    manager.set_on_output_line(Arc::new(move |pipe, _id, line| {
        sink.lock()
            .unwrap()
            .push((pipe.to_string(), line.to_string()));
    }));
    configure(&mut manager);
    let ready = Arc::new(SchedulerReady::default());
    manager.add_run_hook(ready.clone());
    manager.load_pipes().await.unwrap();
    manager.start_scheduler().await.unwrap();
    assert!(
        wait_until(Duration::from_secs(20), || ready
            .ready
            .load(Ordering::SeqCst))
        .await,
        "scheduler never subscribed"
    );
    (manager, lines)
}

fn finished_rows(store: &MemoryPipeStore, pipe: &str) -> usize {
    store
        .executions_for_pipe(pipe)
        .iter()
        .filter(|e| e.finished_at.is_some())
        .count()
}

fn meeting_ended(meeting_id: i64) {
    screenpipe_events::send_event("meeting_ended", json!({ "meeting_id": meeting_id })).unwrap();
}

// ── Defer and coalesce ───────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_busy_pipe_defers_wakeups_then_runs_once_coalesced() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w402-wake-{}", unique_id());
    let event = format!("workflow_ready:{pipe}");
    let pipes_dir = write_pipe(
        dir.path(),
        &pipe,
        &format!("schedule: manual\ntrigger:\n  events:\n    - {event}\n"),
    );
    let store = Arc::new(MemoryPipeStore::default());
    let agent = Arc::new(ScriptedAgent::new(&finished_answer("done"), true));
    let (mut manager, _) = start(pipes_dir, store.clone(), agent.clone(), vec![], |_| {}).await;

    // Hold the first run in its queued state: the pipe stays busy.
    let gate = store.running_gate.write().await;
    screenpipe_events::send_event(&event, json!({"reason": "first"})).unwrap();
    assert!(
        wait_until(Duration::from_secs(10), || !store
            .executions_for_pipe(&pipe)
            .is_empty())
        .await,
        "the first wake-up never queued a run"
    );
    // Four more wake-ups while the pipe is busy: one more run afterwards.
    for n in 0..4 {
        screenpipe_events::send_event(&event, json!({"reason": "again", "n": n})).unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    assert!(
        wait_until(Duration::from_secs(10), || {
            let stats = manager.backlog_stats();
            stats.deferred == 4 && stats.coalesced == 3
        })
        .await,
        "{:?}",
        manager.backlog_stats()
    );
    drop(gate);

    assert!(
        wait_until(Duration::from_secs(15), || finished_rows(&store, &pipe)
            >= 2)
        .await,
        "the deferred wake-up never ran"
    );
    // Nothing else is waiting.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let rows = store.executions_for_pipe(&pipe);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows.iter().all(|r| r.status == "completed"));
    assert!(rows
        .iter()
        .all(|r| r.trigger_event.as_deref() == Some(event.as_str())));
    // The second run handled the newest wake-up.
    let prompts = agent.prompts.lock().unwrap().clone();
    assert!(prompts[1].contains("\"n\":3"), "{}", prompts[1]);

    manager.stop_scheduler().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_coalescing_keeps_one_run_per_meeting() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w402-meet-{}", unique_id());
    let pipes_dir = write_pipe(
        dir.path(),
        &pipe,
        "schedule: manual\ntrigger:\n  events:\n    - meeting_ended\n",
    );
    let store = Arc::new(MemoryPipeStore::default());
    let agent = Arc::new(ScriptedAgent::new(&finished_answer("Saved."), true));
    let first = unique_id();
    let second = unique_id();
    let (mut manager, _) = start(
        pipes_dir,
        store.clone(),
        agent.clone(),
        vec![first, second],
        |_| {},
    )
    .await;

    let gate = store.running_gate.write().await;
    meeting_ended(first);
    let first_key = first.to_string();
    assert!(
        wait_until(Duration::from_secs(10), || !store
            .executions_for_key(&first_key)
            .is_empty())
        .await
    );
    // The second meeting ends three times while the first is summarised
    // (a rejoin window), and the first again.
    for _ in 0..3 {
        meeting_ended(second);
    }
    meeting_ended(first);
    assert!(
        wait_until(Duration::from_secs(10), || {
            let stats = manager.backlog_stats();
            stats.deferred == 4 && stats.coalesced == 2
        })
        .await,
        "{:?}",
        manager.backlog_stats()
    );
    drop(gate);

    let second_key = second.to_string();
    assert!(
        wait_until(Duration::from_secs(15), || store
            .executions_for_key(&second_key)
            .iter()
            .any(|e| e.finished_at.is_some()))
        .await
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    // One run per meeting: the kept copy of the first meeting is a
    // duplicate its claim stops.
    assert_eq!(store.executions_for_key(&first_key).len(), 1);
    assert_eq!(store.executions_for_key(&second_key).len(), 1);
    assert_eq!(
        agent.runs_mentioning(&format!("instance `{second_key}`")),
        1
    );
    assert_eq!(agent.runs_mentioning(&format!("instance `{first_key}`")), 1);

    manager.stop_scheduler().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_backlog_cap_evicts_the_oldest_and_records_it() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w402-cap-{}", unique_id());
    let pipes_dir = write_pipe(
        dir.path(),
        &pipe,
        "schedule: manual\ntrigger:\n  events:\n    - meeting_ended\n",
    );
    let store = Arc::new(MemoryPipeStore::default());
    let agent = Arc::new(ScriptedAgent::new(&finished_answer("Saved."), true));
    let meetings: Vec<i64> = (0..5).map(|_| unique_id()).collect();
    let (mut manager, _) = start(
        pipes_dir,
        store.clone(),
        agent.clone(),
        meetings.clone(),
        |m| m.backlog_cap = 2,
    )
    .await;

    let gate = store.running_gate.write().await;
    meeting_ended(meetings[0]);
    assert!(
        wait_until(Duration::from_secs(10), || !store
            .executions_for_key(&meetings[0].to_string())
            .is_empty())
        .await
    );
    // Four more meetings end while the first is held: the backlog keeps
    // two, so the two oldest are evicted and recorded.
    for m in &meetings[1..] {
        meeting_ended(*m);
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    assert!(
        wait_until(Duration::from_secs(10), || manager
            .backlog_stats()
            .dropped_full
            == 2)
        .await,
        "{:?}",
        manager.backlog_stats()
    );
    drop(gate);

    for m in &meetings[3..] {
        let key = m.to_string();
        assert!(
            wait_until(Duration::from_secs(20), || store
                .executions_for_key(&key)
                .iter()
                .any(|e| e.status == "completed"))
            .await,
            "kept meeting {key} never ran"
        );
    }
    for m in &meetings[1..3] {
        let rows = store.executions_for_key(&m.to_string());
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].status, "skipped");
        assert_eq!(rows[0].error_type.as_deref(), Some(SKIP_BACKLOG_FULL));
        assert!(rows[0].error_message.as_deref().unwrap().contains("busy"));
        assert_eq!(agent.runs_mentioning(&format!("instance `{m}`")), 0);
    }
    let stats = manager.backlog_stats();
    assert_eq!(stats.dropped_full, 2, "{stats:?}");
    assert_eq!(stats.deferred, 4, "{stats:?}");

    manager.stop_scheduler().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_gives_up_a_deferred_run_after_its_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w402-ttl-{}", unique_id());
    let pipes_dir = write_pipe(
        dir.path(),
        &pipe,
        "schedule: manual\ntrigger:\n  events:\n    - meeting_ended\n",
    );
    let store = Arc::new(MemoryPipeStore::default());
    let agent = Arc::new(ScriptedAgent::new(&finished_answer("Saved."), true));
    // The agent never becomes ready.
    agent.available.store(false, Ordering::SeqCst);
    let meeting = unique_id();
    let (mut manager, _) = start(
        pipes_dir,
        store.clone(),
        agent.clone(),
        vec![meeting],
        |m| m.backlog_ttl = Duration::from_millis(300),
    )
    .await;

    meeting_ended(meeting);
    let key = meeting.to_string();
    assert!(
        wait_until(Duration::from_secs(10), || !store
            .executions_for_key(&key)
            .is_empty())
        .await,
        "the expired run was not recorded"
    );
    let rows = store.executions_for_key(&key);
    assert_eq!(rows[0].status, "skipped");
    assert_eq!(rows[0].error_type.as_deref(), Some(SKIP_DEFERRED_EXPIRED));
    assert!(!store.is_claimed(&pipe, "meeting_ended", &key));
    assert_eq!(manager.backlog_stats().expired, 1);
    assert!(agent.prompts.lock().unwrap().is_empty());

    manager.stop_scheduler().await;
}

// ── Connection triggers and trigger context ──────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_connection_triggers_run_their_pipe_with_a_private_context() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w402-conn-{}", unique_id());
    let other = format!("w402-conn-other-{}", unique_id());
    let frontmatter =
        "schedule: manual\ntrigger:\n  sources:\n    - app: slack\n      path: general\n";
    let pipes_dir = write_pipe(dir.path(), &pipe, frontmatter);
    write_pipe(dir.path(), &other, frontmatter);
    let store = Arc::new(MemoryPipeStore::default());
    let agent = Arc::new(ScriptedAgent::new(&finished_answer("done"), true));
    let (mut manager, _) = start(
        pipes_dir.clone(),
        store.clone(),
        agent.clone(),
        vec![],
        |_| {},
    )
    .await;

    let delivery = format!("slack:general:{}", unique_id());
    let payload = json!({
        "pipe_name": pipe,
        "source_delivery_id": delivery,
        "item": {"text": "deploy is broken", "permalink": "https://x.slack.com/a?token=abc123"},
        "auth": {"access_token": "xoxb-1111111111-2222222222"}
    });
    screenpipe_events::send_event("connection_trigger", payload.clone()).unwrap();
    assert!(
        wait_until(Duration::from_secs(10), || finished_rows(&store, &pipe)
            == 1)
        .await,
        "the connection trigger never ran its pipe"
    );
    let row = &store.executions_for_pipe(&pipe)[0];
    assert_eq!(row.trigger_type, "connection");
    assert_eq!(row.trigger_event.as_deref(), Some("connection_trigger"));
    assert_eq!(row.trigger_key.as_deref(), Some(delivery.as_str()));
    assert!(store.is_claimed(&pipe, "connection_trigger", &delivery));
    // Only the named pipe ran.
    assert!(store.executions_for_pipe(&other).is_empty());

    let context_path = pipes_dir.join(&pipe).join(".trigger-context.json");
    let text = std::fs::read_to_string(&context_path).unwrap();
    assert!(!text.contains("xoxb-1111111111"), "{text}");
    assert!(!text.contains("abc123"), "{text}");
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["event"], "connection_trigger");
    assert_eq!(doc["key"], delivery);
    assert_eq!(doc["data"]["item"]["text"], "deploy is broken");
    // The prompt does not carry the token either.
    let prompts = agent.prompts.lock().unwrap().clone();
    assert!(!prompts[0].contains("xoxb-1111111111"));

    // The same delivery again is a duplicate.
    screenpipe_events::send_event("connection_trigger", payload).unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(store.executions_for_pipe(&pipe).len(), 1);

    // A manual run is not about an event: the old context goes.
    manager.stop_scheduler().await;
    manager.run_pipe(&pipe).await.unwrap();
    assert!(!context_path.exists());
}

#[test]
fn scheduler_directed_runs_need_a_pipe_that_takes_them() {
    let run = |event: &str| EventRun {
        pipe: "p".into(),
        event: event.into(),
        key: Some("k".into()),
        data: json!({}),
        directed: true,
    };
    let parse = |yaml: &str| -> PipeConfig {
        super::parse_frontmatter(&format!("---\n{yaml}---\nbody\n"))
            .unwrap()
            .0
    };
    let sources = parse("enabled: true\ntrigger:\n  sources:\n    - app: github\n");
    assert!(pipe_accepts_run(&sources, &run("connection_trigger")));
    assert!(!pipe_accepts_run(&sources, &run("meeting_ended")));
    let listed = parse("enabled: true\ntrigger:\n  events:\n    - connection_trigger\n");
    assert!(pipe_accepts_run(&listed, &run("connection_trigger")));
    let off = parse("enabled: false\ntrigger:\n  sources:\n    - app: github\n");
    assert!(!pipe_accepts_run(&off, &run("connection_trigger")));
    let none = parse("enabled: true\n");
    assert!(!pipe_accepts_run(&none, &run("connection_trigger")));
}

// ── Presets ──────────────────────────────────────────────────────────────

#[test]
fn scheduler_star_preset_expands_to_the_users_presets() {
    let dir = tempfile::tempdir().unwrap();
    let pipes_dir = write_pipe(dir.path(), "p", "schedule: manual\npreset: [\"*\"]\n");
    write_store(dir.path(), two_presets());
    let registry = PresetFallbackRegistry::new(dir.path());
    let (config, _) =
        super::parse_frontmatter(&std::fs::read_to_string(pipes_dir.join("p/pipe.md")).unwrap())
            .unwrap();
    let first = choose_run_model(&pipes_dir, &config, &registry, 0).unwrap();
    // The default preset first, then the others.
    assert_eq!(first.preset_id.as_deref(), Some("local"));
    assert_eq!(first.model, "m-local");
    assert_eq!(first.provider.as_deref(), Some("ollama"));
    assert_eq!(first.slots, 2);
    let second = choose_run_model(&pipes_dir, &config, &registry, 1).unwrap();
    assert_eq!(second.preset_id.as_deref(), Some("work"));
    assert_eq!(second.model, "m-work");
}

#[test]
fn scheduler_missing_named_preset_fails_without_a_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let pipes_dir = write_pipe(
        dir.path(),
        "p",
        "schedule: manual\npreset: deleted\nmodel: some-cloud-model\n",
    );
    write_store(dir.path(), two_presets());
    let registry = PresetFallbackRegistry::new(dir.path());
    let (mut config, _) =
        super::parse_frontmatter(&std::fs::read_to_string(pipes_dir.join("p/pipe.md")).unwrap())
            .unwrap();
    config.name = "p".into();
    let err = choose_run_model(&pipes_dir, &config, &registry, 0).unwrap_err();
    assert!(
        err.message.contains("'deleted' is not in your AI settings"),
        "{err}"
    );
    assert!(
        err.message.contains("available presets: work, local"),
        "{err}"
    );
    // Not even the pipe's own `model` is used.
    assert!(!err.message.contains("some-cloud-model"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_event_run_with_a_missing_preset_fails_and_runs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w402-nopreset-{}", unique_id());
    let pipes_dir = write_pipe(
        dir.path(),
        &pipe,
        "schedule: manual\npreset: deleted\ntrigger:\n  events:\n    - meeting_ended\n",
    );
    write_store(dir.path(), two_presets());
    let store = Arc::new(MemoryPipeStore::default());
    let agent = Arc::new(ScriptedAgent::new(&finished_answer("Saved."), true));
    let meeting = unique_id();
    let (mut manager, _) = start(
        pipes_dir,
        store.clone(),
        agent.clone(),
        vec![meeting],
        |_| {},
    )
    .await;

    meeting_ended(meeting);
    let key = meeting.to_string();
    assert!(
        wait_until(Duration::from_secs(10), || store
            .executions_for_key(&key)
            .iter()
            .any(|e| e.finished_at.is_some()))
        .await
    );
    let rows = store.executions_for_key(&key);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "failed");
    assert_eq!(
        rows[0].error_type.as_deref(),
        Some(ERROR_AI_PRESET_UNAVAILABLE)
    );
    assert!(rows[0]
        .error_message
        .as_deref()
        .unwrap()
        .contains("'deleted'"));
    assert!(
        agent.prompts.lock().unwrap().is_empty(),
        "the agent must not run"
    );
    // The pipe is free again afterwards.
    assert!(!manager.run_slots.lock().await.contains_key(&pipe));

    // A manual run gets the same reason and leaves the pipe runnable.
    manager.stop_scheduler().await;
    let err = manager.run_pipe(&pipe).await.unwrap_err().to_string();
    assert!(err.contains("'deleted'"), "{err}");
    let again = manager.run_pipe(&pipe).await.unwrap_err().to_string();
    assert!(!again.contains("already running"), "{again}");
    let manual: Vec<_> = store
        .executions_for_pipe(&pipe)
        .into_iter()
        .filter(|e| e.trigger_type == "manual")
        .collect();
    assert_eq!(manual.len(), 2);
    assert!(manual
        .iter()
        .all(|e| e.error_type.as_deref() == Some(ERROR_AI_PRESET_UNAVAILABLE)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_star_preset_runs_on_the_users_presets_and_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w402-star-{}", unique_id());
    // The first named preset is gone; `*` brings the user's own.
    let pipes_dir = write_pipe(
        dir.path(),
        &pipe,
        "schedule: manual\npreset: [\"gone\", \"*\"]\ntrigger:\n  events:\n    - meeting_ended\n",
    );
    write_store(dir.path(), two_presets());
    let store = Arc::new(MemoryPipeStore::default());
    let agent = Arc::new(ScriptedAgent::new(&finished_answer("Saved."), true));
    // The default preset's model fails: the run moves to the next slot.
    agent
        .failing_models
        .lock()
        .unwrap()
        .insert("m-local".to_string());
    let meeting = unique_id();
    let (mut manager, lines) = start(
        pipes_dir,
        store.clone(),
        agent.clone(),
        vec![meeting],
        |_| {},
    )
    .await;

    meeting_ended(meeting);
    let key = meeting.to_string();
    assert!(
        wait_until(Duration::from_secs(10), || store
            .executions_for_key(&key)
            .iter()
            .any(|e| e.status == "completed"))
        .await,
        "the fallback slot never completed: {:?}",
        store.executions_for_key(&key)
    );
    let rows = store.executions_for_key(&key);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].status, "failed");
    assert_eq!(rows[0].model.as_deref(), Some("m-local"));
    assert_eq!(rows[1].status, "completed");
    assert_eq!(rows[1].model.as_deref(), Some("m-work"));
    assert_eq!(rows[1].trigger_event.as_deref(), Some("meeting_ended"));
    let models: Vec<String> = agent
        .models
        .lock()
        .unwrap()
        .iter()
        .map(|m| m.0.clone())
        .collect();
    assert_eq!(models, vec!["m-local".to_string(), "m-work".to_string()]);
    // One claim for the instance, held by the run.
    assert!(store.is_claimed(&pipe, "meeting_ended", &key));
    // The fallback is announced on the output stream.
    let fallbacks: Vec<Value> = lines
        .lock()
        .unwrap()
        .iter()
        .filter(|(p, _)| *p == pipe)
        .filter_map(|(_, l)| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["kind"] == "model_fallback")
        .collect();
    assert!(
        fallbacks
            .iter()
            .any(|v| v["requested"] == "local" && v["used"] == "work"),
        "{fallbacks:?}"
    );

    manager.stop_scheduler().await;
}

// ── Stale one-offs ───────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_records_a_stale_one_off_as_skipped_and_switches_it_off() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w402-stale-{}", unique_id());
    let at = (Utc::now() - chrono::Duration::hours(3)).to_rfc3339();
    let pipes_dir = write_pipe(dir.path(), &pipe, &format!("schedule: at {at}\n"));
    let store = Arc::new(MemoryPipeStore::default());
    let agent = Arc::new(ScriptedAgent::new(&finished_answer("done"), true));
    let (mut manager, _) = start(
        pipes_dir.clone(),
        store.clone(),
        agent.clone(),
        vec![],
        |_| {},
    )
    .await;

    assert!(
        wait_until(Duration::from_secs(10), || !store
            .executions_for_pipe(&pipe)
            .is_empty())
        .await
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let rows = store.executions_for_pipe(&pipe);
    assert_eq!(rows.len(), 1, "recorded once: {rows:?}");
    assert_eq!(rows[0].status, "skipped");
    assert_eq!(rows[0].trigger_type, "scheduled");
    assert_eq!(rows[0].error_type.as_deref(), Some(SKIP_STALE_ONE_OFF));
    assert!(agent.prompts.lock().unwrap().is_empty());
    assert!(!manager.get_pipe(&pipe).await.unwrap().config.enabled);
    assert_eq!(
        super::load_local_overrides(&pipes_dir).get(&pipe),
        Some(&false)
    );

    manager.stop_scheduler().await;
}

// ── Trajectories ─────────────────────────────────────────────────────────

fn trajectory_records(dir: &Path) -> Vec<Vec<Value>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default();
    files.sort();
    files
        .iter()
        .map(|f| {
            std::fs::read_to_string(f)
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect()
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_trajectories_dir_set_records_one_jsonl_file_per_run() {
    let dir = tempfile::tempdir().unwrap();
    let traj_dir = dir.path().join("traj");
    let pipe = format!("w402-traj-{}", unique_id());
    let event = format!("workflow_ready:{pipe}");
    let pipes_dir = write_pipe(
        dir.path(),
        &pipe,
        &format!("schedule: manual\ntrigger:\n  events:\n    - {event}\n"),
    );
    let store = Arc::new(MemoryPipeStore::default());
    let stdout = format!(
        "{}\n{}",
        json!({"type": "tool_execution_start", "args": {"api_key": "sk-secret-0123456789abcdef"}}),
        finished_answer("all done")
    );
    let agent = Arc::new(ScriptedAgent::new(&stdout, true));
    // As if SCREENPIPE_TRAJECTORIES_DIR were set for the engine.
    let traj_value = traj_dir.to_string_lossy().to_string();
    let config = TrajectoryConfig::from_lookup(dir.path(), |name| {
        (name == crate::env::SCREENPIPE_TRAJECTORIES_DIR).then(|| traj_value.clone())
    })
    .unwrap();
    assert_eq!(config.dir, traj_dir);
    let (mut manager, _) = start(pipes_dir, store.clone(), agent.clone(), vec![], move |m| {
        m.set_trajectories(Some(config))
    })
    .await;

    screenpipe_events::send_event(&event, json!({"why": "test"})).unwrap();
    assert!(
        wait_until(Duration::from_secs(10), || finished_rows(&store, &pipe)
            == 1)
        .await
    );
    assert!(
        wait_until(Duration::from_secs(5), || trajectory_records(&traj_dir)
            .first()
            .is_some_and(|r| r.last().is_some_and(|l| l["type"] == "run_end")))
        .await,
        "no finished trajectory"
    );
    let files = trajectory_records(&traj_dir);
    assert_eq!(files.len(), 1);
    let records = &files[0];
    assert_eq!(records[0]["type"], "run_start");
    assert_eq!(records[0]["pipe"], pipe);
    assert_eq!(records[0]["trigger"], "event");
    assert_eq!(records[0]["trigger_event"], event);
    let events: Vec<&Value> = records
        .iter()
        .filter(|r| r["type"] == "agent_event")
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"]["args"]["api_key"], "[redacted]");
    let end = records.last().unwrap();
    assert_eq!(end["type"], "run_end");
    assert_eq!(end["success"], true);
    assert_eq!(end["events"], 2);

    // A manual run records its own file.
    manager.stop_scheduler().await;
    manager.run_pipe(&pipe).await.unwrap();
    let files = trajectory_records(&traj_dir);
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|r| r[0]["trigger"] == "manual"));
}

// ── Log archive ──────────────────────────────────────────────────────────

/// Collects the message of every log event on the current thread.
#[derive(Default, Clone)]
struct LogCapture {
    lines: Arc<StdMutex<Vec<String>>>,
}

struct MessageVisitor<'a>(&'a mut String);

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0.push_str(&format!("{value:?}"));
        }
    }
}

impl tracing::Subscriber for LogCapture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut text = String::new();
        event.record(&mut MessageVisitor(&mut text));
        self.lines.lock().unwrap().push(text);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test]
async fn scheduler_log_archive_sweep_reports_its_counts_in_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = format!("w402-arch-{}", unique_id());
    let pipes_dir = write_pipe(dir.path(), &pipe, "schedule: manual\n");
    let logs = pipes_dir.join(&pipe).join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let now = std::time::SystemTime::now();
    // 60 old run logs: the newest 50 stay, 10 are archived.
    for i in 0..60u32 {
        let age = Duration::from_secs(86_400 * (30 + i as u64));
        let started: chrono::DateTime<Utc> = (now - age).into();
        let path = logs.join(format!("{}.json", started.format("%Y%m%d_%H%M%S")));
        std::fs::write(
            &path,
            json!({"pipe_name": pipe, "success": true}).to_string(),
        )
        .unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(now - age)
            .unwrap();
    }
    let manager = PipeManager::new(pipes_dir.clone(), HashMap::new(), None, 3035);
    manager.load_pipes().await.unwrap();

    let capture = LogCapture::default();
    let guard = tracing::subscriber::set_default(capture.clone());
    let report = manager.archive_pipe_logs().await.expect("sweep ran");
    drop(guard);

    assert_eq!(report.scanned, 60);
    assert_eq!(report.archived, 10);
    assert_eq!(report.failed, 0);
    let remaining = std::fs::read_dir(&logs)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "json")
        })
        .count();
    assert_eq!(remaining, 50);
    let lines = capture.lines.lock().unwrap().clone();
    assert!(
        lines.iter().any(|l| l.contains("pipe log archive")
            && l.contains("scanned=60")
            && l.contains("archived=10")),
        "{lines:?}"
    );
}
