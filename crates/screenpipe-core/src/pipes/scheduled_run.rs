// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! One run the scheduler queued (scheduled or event-triggered), from the
//! moment it is queued until its result is stored.
//!
//! A run makes one or more attempts. The first uses the preset the
//! scheduler chose when it queued the run. When an attempt fails and the
//! pipe's preset chain has another slot (see
//! [`PresetChain`](super::preset_fallback::PresetChain)), the run tries the
//! next slot at once, as a manual run does: the failed attempt keeps its
//! execution row, the next one gets a new row for the same trigger, and a
//! `model_fallback` status line goes to the output stream. A run without a
//! usable preset (`ai_preset_unavailable`) makes one attempt that fails
//! without starting the agent.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use super::event_runs::RunSlot;
use super::preset_fallback::{
    PresetFallbackRegistry, ERROR_AI_PRESET_UNAVAILABLE, MAX_FALLBACK_DEPTH,
};
use super::trajectory::{RunTrajectory, TrajectoryConfig, TrajectoryEnd, TrajectoryMeta};
use super::{
    choose_run_model, cleanup_pipe_token, delete_pi_sessions, filter_ndjson_stdout,
    find_latest_pi_session, hooks, log_archive, mark_run_slot_started, outcome,
    record_conversation_id, release_claim, remove_pid_file, remove_run_slot,
    render_pipe_system_prompt, render_prompt_with_port, write_pid_file, OnPipeOutputLine,
    OnPipeRunComplete, PipeConfig, PipeRunLog, PipeStore, PresetUnavailable, RunModel,
    DEFAULT_TIMEOUT_SECS, SCHEDULED_RUN_SPACING_SECS,
};
use crate::agents::pi::{PiExecutor, SCREENPIPE_API_URL};
use crate::agents::{AgentExecutor, ExecutionHandle};

/// Everything a queued run needs, moved into its task.
pub(super) struct ScheduledRun {
    pub pipe_name: String,
    pub config: PipeConfig,
    pub body: String,
    pub pipes_dir: PathBuf,
    pub executor: Arc<dyn AgentExecutor>,
    pub store: Option<Arc<dyn PipeStore>>,
    pub api_port: u16,
    /// Extra prompt context (connected apps, the triggering event).
    pub run_context: Option<String>,
    pub connections_context: Option<String>,
    pub local_api_key: Option<String>,
    /// `scheduled`, `event` or `connection`.
    pub trigger: &'static str,
    pub is_event_triggered: bool,
    pub trigger_event: Option<String>,
    pub trigger_key: Option<String>,
    /// Row of the first attempt, created when the run was queued.
    pub exec_id: Option<i64>,
    pub slot: RunSlot,
    pub run_model: Result<RunModel, PresetUnavailable>,
    pub pipe_token: Option<String>,
    pub run_hooks: hooks::PipeHooks,
    pub logs: Arc<Mutex<HashMap<String, VecDeque<PipeRunLog>>>>,
    pub running: Arc<Mutex<HashMap<String, ExecutionHandle>>>,
    pub running_execution_ids: Arc<Mutex<HashMap<String, i64>>>,
    pub queued_or_running: Arc<Mutex<HashSet<String>>>,
    pub run_slots: Arc<Mutex<HashMap<String, RunSlot>>>,
    pub on_complete: Option<OnPipeRunComplete>,
    pub on_output: Option<OnPipeOutputLine>,
    pub token_registry: Option<Arc<dyn super::permissions::PipeTokenRegistry>>,
    pub semaphore: Arc<tokio::sync::Semaphore>,
    pub fallback_registry: Arc<PresetFallbackRegistry>,
    pub trajectories: Option<TrajectoryConfig>,
}

/// What one attempt produced.
struct Attempt {
    log: PipeRunLog,
    error_type: Option<String>,
    /// The agent process was started.
    launched: bool,
}

impl ScheduledRun {
    pub(super) async fn execute(self) {
        let name = self.pipe_name.clone();
        // Event-triggered pipes skip the queue for low-latency response.
        // Scheduled pipes wait for the previous one to finish.
        let _permit = if !self.is_event_triggered {
            match self.semaphore.clone().acquire_owned().await {
                Ok(permit) => Some(permit),
                Err(_) => {
                    warn!(
                        "scheduler: execution queue closed; run of '{}' dropped",
                        name
                    );
                    self.queued_or_running.lock().await.remove(&name);
                    remove_run_slot(&self.run_slots, &name, &self.slot).await;
                    self.drop_token();
                    return;
                }
            }
        } else {
            None
        };

        // Stopped while queued: `stop_pipe` already cancelled the row and
        // gave the claim back.
        if self.slot.is_cancelled() {
            info!("scheduler: queued run of pipe '{}' was cancelled", name);
            self.queued_or_running.lock().await.remove(&name);
            remove_run_slot(&self.run_slots, &name, &self.slot).await;
            self.drop_token();
            return;
        }

        // Mark running + write PID file only after acquiring the permit, so
        // the UI shows accurate state (not "running" while queued).
        self.running
            .lock()
            .await
            .insert(name.clone(), ExecutionHandle { pid: 0 });
        // Sentinel 0 — see start_pipe_background.
        write_pid_file(&self.pipes_dir, &name, 0);
        info!("scheduler: running pipe '{}'", name);
        if let Some(id) = self.exec_id {
            self.running_execution_ids
                .lock()
                .await
                .insert(name.clone(), id);
        }

        // The transition only applies to a row that is still queued; one
        // cancelled or finished meanwhile keeps its state and this run does
        // not start.
        let may_start = match (&self.store, self.exec_id) {
            (Some(store), Some(id)) => match store.set_execution_running(id, None).await {
                Ok(moved) => moved,
                Err(e) => {
                    warn!("failed to mark execution {} running: {}", id, e);
                    true
                }
            },
            _ => true,
        };
        if !may_start || self.slot.is_cancelled() {
            info!(
                "scheduler: run of pipe '{}' was cancelled or finished before it started",
                name
            );
            self.clear_running_state().await;
            if let Some(ref claim) = self.slot.claim {
                release_claim(self.store.as_ref(), claim, "the run did not start").await;
            }
            remove_run_slot(&self.run_slots, &name, &self.slot).await;
            self.drop_token();
            return;
        }
        mark_run_slot_started(&self.run_slots, &name, &self.slot).await;

        let first_started = Utc::now();
        let mut exec_id = self.exec_id;
        let mut model = self.run_model.clone();
        let mut note = model
            .as_ref()
            .ok()
            .and_then(|m| m.fallback_note(None, None));
        let mut attempt_no = 1u32;
        let mut any_launched = false;
        let last = loop {
            let attempt = self.attempt(exec_id, &model, attempt_no, note.take()).await;
            any_launched |= attempt.launched;

            let next_floor = match &model {
                Ok(m) if !attempt.log.success && !self.slot.is_cancelled() => m
                    .slot
                    .map(|i| i + 1)
                    .filter(|next| *next < m.slots.min(MAX_FALLBACK_DEPTH)),
                _ => None,
            };
            let Some(floor) = next_floor else {
                break attempt;
            };
            let next = match choose_run_model(
                &self.pipes_dir,
                &self.config,
                &self.fallback_registry,
                floor,
            ) {
                Ok(next) => next,
                Err(_) => break attempt,
            };
            let previous = model.as_ref().ok().cloned();
            info!(
                "pipe '{}': preset '{}' failed ({}), retrying with fallback preset '{}' (attempt {}/{})",
                name,
                previous
                    .as_ref()
                    .and_then(|p| p.preset_id.as_deref())
                    .unwrap_or("?"),
                attempt.error_type.as_deref().unwrap_or("failed"),
                next.preset_id.as_deref().unwrap_or("?"),
                attempt_no + 1,
                next.slots.min(MAX_FALLBACK_DEPTH)
            );
            let next_id = self.open_retry_row(&next).await;
            if self.slot.is_cancelled() {
                // Stopped while the retry was being set up: its row never
                // ran.
                if let (Some(store), Some(id)) = (&self.store, next_id) {
                    let _ = store
                        .finish_execution(
                            id,
                            "cancelled",
                            "",
                            "",
                            None,
                            Some("cancelled"),
                            None,
                            None,
                        )
                        .await;
                }
                break attempt;
            }
            note = next.fallback_note(previous.as_ref(), attempt.error_type.as_deref());
            exec_id = next_id;
            model = Ok(next);
            attempt_no += 1;
        };

        self.clear_running_state().await;

        // No attempt started the agent (no usable preset, a launch error, a
        // timeout before the process came up): the event instance was not
        // handled, so its claim goes back and a later delivery may run it.
        if !any_launched && !last.log.success {
            if let Some(ref claim) = self.slot.claim {
                let reason = match last.error_type.as_deref() {
                    Some(ERROR_AI_PRESET_UNAVAILABLE) => "no usable AI preset",
                    _ => "the agent could not start",
                };
                release_claim(self.store.as_ref(), claim, reason).await;
            }
        }
        if let Some(ref store) = self.store {
            let _ = store.upsert_scheduler_state(&name, last.log.success).await;
        }

        // Auto-clear Pi session on context overflow so the next run starts fresh
        if last.error_type.as_deref() == Some("context_overflow") {
            let pipe_dir = self.pipes_dir.join(&name);
            if let Err(e) = delete_pi_sessions(&pipe_dir) {
                warn!(
                    "pipe '{}' overflowed its context, but its Pi session could not be reset: {}",
                    name, e
                );
            } else {
                info!(
                    "cleared Pi session for '{}' after context overflow — next run starts fresh",
                    name
                );
            }
        }

        // Emit pipe_completed event so other pipes can chain
        let success = last.log.success;
        let duration_secs =
            (last.log.finished_at - first_started).num_milliseconds() as f64 / 1000.0;
        let _ = screenpipe_events::send_event(
            format!("{}{}", super::event_runs::PIPE_COMPLETED_PREFIX, name),
            screenpipe_events::PipeCompletedEvent {
                pipe_name: name.clone(),
                success,
                duration_secs,
                timestamp: Utc::now(),
            },
        );

        // Fire run-complete callback (analytics, etc.)
        if let Some(ref cb) = self.on_complete {
            cb(&name, success, duration_secs, last.error_type.as_deref());
        }

        self.drop_token();
        remove_run_slot(&self.run_slots, &name, &self.slot).await;

        // Stagger consecutive scheduled runs: if other pipes are still
        // queued behind the semaphore, hold this permit a few seconds longer
        // before releasing it, so their requests do not land in the same
        // rate-limit window. Skipped for event-triggered pipes.
        if !self.is_event_triggered {
            let others_waiting = !self.queued_or_running.lock().await.is_empty();
            if others_waiting {
                tokio::time::sleep(std::time::Duration::from_secs(SCHEDULED_RUN_SPACING_SECS))
                    .await;
            }
        }
    }

    /// Create and open the execution row of a retry with `next`.
    async fn open_retry_row(&self, next: &RunModel) -> Option<i64> {
        let store = self.store.as_ref()?;
        let id = match store
            .create_execution(
                &self.pipe_name,
                self.trigger,
                &next.model,
                next.provider.as_deref(),
                self.trigger_event.as_deref(),
                self.trigger_key.as_deref(),
            )
            .await
        {
            Ok(id) => id,
            Err(e) => {
                warn!("failed to create execution row for a retry: {}", e);
                return None;
            }
        };
        if let Err(e) = store.set_execution_running(id, None).await {
            warn!("failed to mark execution {} running: {}", id, e);
        }
        self.running_execution_ids
            .lock()
            .await
            .insert(self.pipe_name.clone(), id);
        Some(id)
    }

    /// Forget an attempt once its agent has exited (or never started). The
    /// pipe stays claimed for a possible retry, but without a process id:
    /// the handle and `run.pid` go back to the sentinel 0, so a stop (or the
    /// orphan cleanup after a crash) never signals a process id the system
    /// may already have handed to another program. `finished` tells the
    /// attempt's PID watcher not to record the id after this point.
    ///
    /// The attempt's row leaves `running_execution_ids` here too. Returns
    /// whether this worker took it out, and so still owns the row: when
    /// `stop_pipe` got there first it finishes the row as cancelled, and the
    /// worker must not overwrite that.
    async fn end_attempt(&self, exec_id: Option<i64>, finished: Option<&AtomicBool>) -> bool {
        {
            // The same lock the watcher holds while it records a pid, so the
            // two never interleave.
            let mut running = self.running.lock().await;
            if let Some(flag) = finished {
                flag.store(true, Ordering::SeqCst);
            }
            if let Some(handle) = running.get_mut(&self.pipe_name) {
                handle.pid = 0;
            }
            write_pid_file(&self.pipes_dir, &self.pipe_name, 0);
        }
        let Some(id) = exec_id else {
            return false;
        };
        let mut ids = self.running_execution_ids.lock().await;
        if ids.get(&self.pipe_name) == Some(&id) {
            ids.remove(&self.pipe_name);
            true
        } else {
            false
        }
    }

    async fn clear_running_state(&self) {
        self.running.lock().await.remove(&self.pipe_name);
        self.running_execution_ids
            .lock()
            .await
            .remove(&self.pipe_name);
        self.queued_or_running.lock().await.remove(&self.pipe_name);
        remove_pid_file(&self.pipes_dir, &self.pipe_name);
    }

    fn drop_token(&self) {
        if let Some(ref token) = self.pipe_token {
            cleanup_pipe_token(token, self.token_registry.as_ref());
        }
    }

    /// Finish an attempt's execution row. `exec_id` is `None` when the
    /// worker does not own the row (see [`Self::end_attempt`]).
    #[allow(clippy::too_many_arguments)]
    async fn finish_row(
        &self,
        exec_id: Option<i64>,
        status: &str,
        stdout: &str,
        stderr: &str,
        error_type: Option<&str>,
        message: Option<&str>,
        session_path: Option<&str>,
    ) {
        if let (Some(store), Some(id)) = (&self.store, exec_id) {
            let _ = store
                .finish_execution(
                    id,
                    status,
                    stdout,
                    stderr,
                    None,
                    error_type,
                    message,
                    session_path,
                )
                .await;
        }
    }

    /// Store an attempt's log file and in-memory log, then run the
    /// `after_run` hooks.
    async fn record(&self, hook_run: hooks::RunStart<'_>, attempt: &Attempt) {
        let pipe_dir = self.pipes_dir.join(&self.pipe_name);
        if let Ok(body) = serde_json::to_string_pretty(&attempt.log) {
            if let Err(e) = log_archive::write_run_log(&pipe_dir, attempt.log.started_at, &body) {
                warn!("pipe '{}': could not write run log: {}", self.pipe_name, e);
            }
        }
        run_hooks_after(&self.run_hooks, hook_run, attempt).await;
        let mut logs = self.logs.lock().await;
        let entry = logs.entry(self.pipe_name.clone()).or_default();
        entry.push_back(attempt.log.clone());
        if entry.len() > 50 {
            entry.pop_front();
        }
    }

    /// One attempt with `model`.
    async fn attempt(
        &self,
        exec_id: Option<i64>,
        model: &Result<RunModel, PresetUnavailable>,
        attempt_no: u32,
        note: Option<Value>,
    ) -> Attempt {
        let name = &self.pipe_name;
        let pipe_dir = self.pipes_dir.join(name);
        let started_at = Utc::now();
        let hook_run = hooks::RunStart {
            pipe_name: name,
            trigger: self.trigger,
            trigger_event: self.trigger_event.as_deref(),
            trigger_key: self.trigger_key.as_deref(),
            execution_id: exec_id,
            pipe_dir: &pipe_dir,
        };

        let m = match model {
            Ok(m) => m,
            Err(unavailable) => {
                // Logged when the run was queued.
                let attempt = Attempt {
                    log: PipeRunLog {
                        pipe_name: name.clone(),
                        started_at,
                        finished_at: Utc::now(),
                        success: false,
                        stdout: String::new(),
                        stderr: unavailable.message.clone(),
                    },
                    error_type: Some(ERROR_AI_PRESET_UNAVAILABLE.to_string()),
                    launched: false,
                };
                self.run_hooks.before_run(&hook_run).await;
                let owns_row = self.end_attempt(exec_id, None).await;
                self.finish_row(
                    exec_id.filter(|_| owns_row),
                    "failed",
                    "",
                    &unavailable.message,
                    Some(ERROR_AI_PRESET_UNAVAILABLE),
                    Some(&unavailable.message),
                    None,
                )
                .await;
                self.record(hook_run, &attempt).await;
                return attempt;
            }
        };

        // Pre-configure pi with this attempt's provider.
        if self.config.agent == "pi" {
            let cloud_token = self.executor.user_token();
            if let Err(e) = PiExecutor::ensure_pi_config(
                cloud_token.as_deref(),
                SCREENPIPE_API_URL,
                m.provider.as_deref(),
                Some(&m.model),
                m.url.as_deref(),
            )
            .await
            {
                warn!("scheduler: failed to pre-configure pi provider: {}", e);
            }
        }

        let system_prompt = render_pipe_system_prompt(
            &self.config,
            &self.body,
            self.api_port,
            m.prompt.as_deref(),
            self.connections_context.as_deref(),
            self.local_api_key.as_deref(),
        );
        let prompt = render_prompt_with_port(
            &self.config,
            &self.body,
            self.api_port,
            m.prompt.as_deref(),
            self.run_context.as_deref(),
        );

        // Stopped while this attempt was being set up (a retry after a
        // failed slot): the agent does not start.
        if self.slot.is_cancelled() {
            let owns_row = self.end_attempt(exec_id, None).await;
            self.finish_row(
                exec_id.filter(|_| owns_row),
                "cancelled",
                "",
                "",
                Some("cancelled"),
                None,
                None,
            )
            .await;
            return Attempt {
                log: PipeRunLog {
                    pipe_name: name.clone(),
                    started_at,
                    finished_at: Utc::now(),
                    success: false,
                    stdout: String::new(),
                    stderr: "stopped before the agent started".to_string(),
                },
                error_type: Some("cancelled".to_string()),
                launched: false,
            };
        }

        // Shared PID — set synchronously by the executor right after spawn.
        let shared_pid = Arc::new(AtomicU32::new(0));
        // Set by `end_attempt` once the agent has exited.
        let finished = Arc::new(AtomicBool::new(false));
        {
            let running = self.running.clone();
            let store = self.store.clone();
            let pipes_dir = self.pipes_dir.clone();
            let name = name.clone();
            let shared_pid = shared_pid.clone();
            let finished = finished.clone();
            tokio::spawn(async move {
                // Brief wait for the synchronous PID store
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let pid = shared_pid.load(Ordering::SeqCst);
                if pid == 0 {
                    return;
                }
                {
                    let mut running = running.lock().await;
                    // An attempt that already ended keeps the sentinel: its
                    // process is gone and the id may belong to another
                    // program by now.
                    if finished.load(Ordering::SeqCst) {
                        return;
                    }
                    if let Some(handle) = running.get_mut(&name) {
                        handle.pid = pid;
                    }
                    write_pid_file(&pipes_dir, &name, pid);
                }
                if let (Some(store), Some(id)) = (&store, exec_id) {
                    let _ = store.set_execution_running(id, Some(pid)).await;
                }
            });
        }

        let trajectory = RunTrajectory::start(
            self.trajectories.as_ref(),
            &TrajectoryMeta {
                pipe: name.clone(),
                execution_id: exec_id,
                trigger: self.trigger.to_string(),
                trigger_event: self.trigger_event.clone(),
                trigger_key: self.trigger_key.clone(),
                model: m.model.clone(),
                provider: m.provider.clone(),
                preset: m.preset_id.clone(),
                attempt: attempt_no,
                prompt: prompt.clone(),
                system_prompt: Some(system_prompt.clone()),
            },
        );

        // Stream the agent's lines to listeners (and the trajectory).
        let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let on_output = self.on_output.clone();
        let drain_name = name.clone();
        let drain_id = exec_id.unwrap_or(0);
        let drain_trajectory = trajectory.clone();
        if let Some(ref note) = note {
            let line = note.to_string();
            if let Some(ref cb) = on_output {
                cb(&drain_name, drain_id, &line);
            }
            if let Some(ref t) = trajectory {
                t.status(note);
            }
        }
        let drainer = tokio::spawn(async move {
            while let Some(line) = line_rx.recv().await {
                if let Some(ref t) = drain_trajectory {
                    t.agent_event(&line);
                }
                if let Some(ref cb) = on_output {
                    cb(&drain_name, drain_id, &line);
                }
            }
            // Channel closed — pipe process exited. Emit a done sentinel.
            if let Some(ref cb) = on_output {
                cb(&drain_name, drain_id, r#"{"type":"pipe_done"}"#);
            }
        });

        self.run_hooks.before_run(&hook_run).await;

        let timeout_secs = self.config.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS);
        let run_result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            self.executor.run_streaming(
                &prompt,
                &m.model,
                &pipe_dir,
                m.provider.as_deref(),
                m.url.as_deref(),
                m.api_key.as_deref(),
                Some(shared_pid.clone()),
                line_tx,
                self.config.history,
                Some(&system_prompt),
                // Owner tag — see run_pipe_with_trigger.
                Some(format!("pipe:{name}").as_str()),
                self.config.effort.map(|e| e.as_str()),
            ),
        )
        .await;
        let finished_at = Utc::now();
        if run_result.is_err() {
            // Timed out: the agent may still be running.
            let pid = shared_pid.load(Ordering::SeqCst);
            if pid != 0 {
                let _ = crate::agents::pi::kill_process_group(pid);
            }
        }
        let owns_row = self.end_attempt(exec_id, Some(&finished)).await;
        // The agent ran when it reported a process id or returned an output.
        let launched = shared_pid.load(Ordering::SeqCst) != 0 || matches!(run_result, Ok(Ok(_)));
        if trajectory.is_some() {
            // Let the drainer write the last lines before the end record.
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), drainer).await;
        }

        let (attempt, row_status, stdout, stderr, error_type, message, session_path) =
            match run_result {
                Ok(Ok(output)) => {
                    let filtered_stdout = filter_ndjson_stdout(&output.stdout);
                    // A clean exit still needs a verified final answer.
                    let verdict =
                        outcome::assess_run(output.success, &output.stdout, &output.stderr);
                    let ok = verdict.success;
                    if let Some(ref preset) = m.preset_id {
                        if ok {
                            self.fallback_registry.record_success(preset);
                        } else {
                            self.fallback_registry.record_failure_from_output(
                                preset,
                                &output.stderr,
                                &filtered_stdout,
                            );
                        }
                    }
                    if ok {
                        info!("pipe '{}' completed successfully", name);
                    } else {
                        warn!(
                            "pipe '{}' failed ({}): {}",
                            name,
                            verdict.error_type.as_deref().unwrap_or("unknown"),
                            verdict
                                .error_message
                                .as_deref()
                                .unwrap_or(output.stderr.as_str())
                        );
                    }
                    let error_type = (!ok).then(|| {
                        verdict
                            .error_type
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string())
                    });
                    let session_path =
                        find_latest_pi_session(&pipe_dir).map(|p| p.to_string_lossy().to_string());
                    (
                        Attempt {
                            log: PipeRunLog {
                                pipe_name: name.clone(),
                                started_at,
                                finished_at,
                                success: ok,
                                stdout: filtered_stdout.clone(),
                                stderr: output.stderr.clone(),
                            },
                            error_type,
                            launched,
                        },
                        verdict.status().to_string(),
                        filtered_stdout,
                        output.stderr,
                        verdict.error_type,
                        verdict.error_message,
                        session_path,
                    )
                }
                Ok(Err(e)) => {
                    error!("pipe '{}' error: {}", name, e);
                    let text = e.to_string();
                    (
                        Attempt {
                            log: PipeRunLog {
                                pipe_name: name.clone(),
                                started_at,
                                finished_at,
                                success: false,
                                stdout: String::new(),
                                stderr: text.clone(),
                            },
                            error_type: Some("crash".to_string()),
                            launched,
                        },
                        "failed".to_string(),
                        String::new(),
                        text.clone(),
                        Some("crash".to_string()),
                        Some(text),
                        None,
                    )
                }
                Err(_elapsed) => {
                    warn!("pipe '{}' timed out after {}s", name, timeout_secs);
                    let text = format!("execution timed out after {}s", timeout_secs);
                    (
                        Attempt {
                            log: PipeRunLog {
                                pipe_name: name.clone(),
                                started_at,
                                finished_at,
                                success: false,
                                stdout: String::new(),
                                stderr: text.clone(),
                            },
                            error_type: Some("timeout".to_string()),
                            launched,
                        },
                        "timed_out".to_string(),
                        String::new(),
                        String::new(),
                        Some("timeout".to_string()),
                        Some(text),
                        None,
                    )
                }
            };

        let owned_id = exec_id.filter(|_| owns_row);
        self.finish_row(
            owned_id,
            &row_status,
            &stdout,
            &stderr,
            error_type.as_deref(),
            message.as_deref(),
            session_path.as_deref(),
        )
        .await;
        if owned_id.is_some() && session_path.is_some() {
            record_conversation_id(
                self.store.as_ref(),
                owned_id,
                self.config.history,
                session_path.as_deref(),
            )
            .await;
        }
        if let Some(ref t) = trajectory {
            t.finish(&TrajectoryEnd {
                success: attempt.log.success,
                error_type: attempt.error_type.clone(),
                duration_ms: (finished_at - started_at).num_milliseconds(),
            });
        }
        self.record(hook_run, &attempt).await;
        attempt
    }
}

async fn run_hooks_after(hooks: &hooks::PipeHooks, start: hooks::RunStart<'_>, attempt: &Attempt) {
    hooks
        .after_run(&hooks::RunEnd {
            start,
            log: &attempt.log,
            error_type: attempt.error_type.as_deref(),
        })
        .await;
}

impl RunModel {
    /// The `model_fallback` status line for an attempt that does not use
    /// the first preset the pipe asked for: after a failed attempt
    /// (`previous`), or because earlier presets were missing or cooling
    /// down. `None` when nothing fell back.
    pub(super) fn fallback_note(
        &self,
        previous: Option<&RunModel>,
        reason: Option<&str>,
    ) -> Option<Value> {
        let requested = match previous {
            Some(p) => p.preset_id.clone().unwrap_or_else(|| p.model.clone()),
            None => {
                let first = self.requested.clone()?;
                if self.preset_id.as_deref() == Some(first.as_str()) {
                    return None;
                }
                first
            }
        };
        Some(json!({
            "type": "status",
            "kind": "model_fallback",
            "requested": requested,
            "used": self.preset_id.clone().unwrap_or_else(|| self.model.clone()),
            "model": self.model,
            "reason": reason.unwrap_or("unavailable"),
        }))
    }
}
