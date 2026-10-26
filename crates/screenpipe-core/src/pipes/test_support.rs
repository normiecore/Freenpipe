// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! In-memory pipe store and a scripted agent for scheduler tests.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex as StdMutex;

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::agents::{AgentExecutor, AgentOutput, ExecutionHandle, SharedPid};

use super::{PipeExecution, PipeStore, SchedulerState};

/// Keeps executions and claims in memory, with the same guarded transitions
/// as the SQLite store.
#[derive(Default)]
pub(crate) struct MemoryPipeStore {
    pub executions: StdMutex<Vec<PipeExecution>>,
    pub claims: StdMutex<HashSet<(String, String, String)>>,
    pub claim_attempts: AtomicUsize,
    /// While a test holds the write side, `set_execution_running` waits,
    /// which keeps a scheduler run in its queued state.
    pub running_gate: tokio::sync::RwLock<()>,
}

impl MemoryPipeStore {
    pub fn executions_for_key(&self, key: &str) -> Vec<PipeExecution> {
        self.executions
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.trigger_key.as_deref() == Some(key))
            .cloned()
            .collect()
    }

    pub fn is_claimed(&self, pipe: &str, event: &str, key: &str) -> bool {
        self.claims
            .lock()
            .unwrap()
            .contains(&(pipe.to_string(), event.to_string(), key.to_string()))
    }

    /// Add a finished row directly (fixtures).
    pub fn insert_finished(
        &self,
        pipe: &str,
        status: &str,
        error_type: Option<&str>,
        trigger_key: Option<&str>,
        finished_at: DateTime<Utc>,
    ) -> i64 {
        let mut rows = self.executions.lock().unwrap();
        let id = rows.len() as i64 + 1;
        rows.push(PipeExecution {
            id,
            pipe_name: pipe.to_string(),
            status: status.to_string(),
            trigger_type: "event".to_string(),
            pid: None,
            model: None,
            provider: None,
            started_at: Some(finished_at.to_rfc3339()),
            finished_at: Some(finished_at.to_rfc3339()),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            error_type: error_type.map(str::to_string),
            error_message: None,
            duration_ms: None,
            session_path: None,
            trigger_event: trigger_key.map(|_| "meeting_ended".to_string()),
            trigger_key: trigger_key.map(str::to_string),
            conversation_id: None,
        });
        id
    }

    fn with_row<T>(&self, id: i64, f: impl FnOnce(&mut PipeExecution) -> T) -> Option<T> {
        let mut rows = self.executions.lock().unwrap();
        rows.iter_mut().find(|r| r.id == id).map(f)
    }
}

fn key_family(key: &str) -> &str {
    key.split(':').next().unwrap_or(key)
}

fn in_family(key: Option<&str>, family: &str) -> bool {
    key.is_some_and(|k| k == family || k.starts_with(&format!("{family}:")))
}

#[async_trait::async_trait]
impl PipeStore for MemoryPipeStore {
    async fn create_execution(
        &self,
        pipe_name: &str,
        trigger_type: &str,
        model: &str,
        provider: Option<&str>,
        trigger_event: Option<&str>,
        trigger_key: Option<&str>,
    ) -> Result<i64> {
        let mut rows = self.executions.lock().unwrap();
        let id = rows.len() as i64 + 1;
        rows.push(PipeExecution {
            id,
            pipe_name: pipe_name.to_string(),
            status: "queued".to_string(),
            trigger_type: trigger_type.to_string(),
            pid: None,
            model: Some(model.to_string()),
            provider: provider.map(str::to_string),
            started_at: Some(Utc::now().to_rfc3339()),
            finished_at: None,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            error_type: None,
            error_message: None,
            duration_ms: None,
            session_path: None,
            trigger_event: trigger_event.map(str::to_string),
            trigger_key: trigger_key.map(str::to_string),
            conversation_id: None,
        });
        Ok(id)
    }

    async fn set_execution_running(&self, id: i64, pid: Option<u32>) -> Result<bool> {
        let _wait = self.running_gate.read().await;
        Ok(self
            .with_row(id, |r| {
                let open = matches!(r.status.as_str(), "queued" | "running") && r.finished_at.is_none();
                if open {
                    r.status = "running".to_string();
                    if pid.is_some() {
                        r.pid = pid;
                    }
                }
                open
            })
            .unwrap_or(false))
    }

    async fn finish_execution(
        &self,
        id: i64,
        status: &str,
        stdout: &str,
        stderr: &str,
        exit_code: Option<i32>,
        error_type: Option<&str>,
        error_message: Option<&str>,
        session_path: Option<&str>,
    ) -> Result<()> {
        self.with_row(id, |r| {
            r.status = status.to_string();
            r.finished_at = Some(Utc::now().to_rfc3339());
            r.stdout = stdout.to_string();
            r.stderr = stderr.to_string();
            r.exit_code = exit_code;
            r.error_type = error_type.map(str::to_string);
            r.error_message = error_message.map(str::to_string);
            r.session_path = session_path.map(str::to_string);
        });
        Ok(())
    }

    async fn get_executions(&self, pipe_name: &str, limit: i32) -> Result<Vec<PipeExecution>> {
        let mut rows: Vec<PipeExecution> = self
            .executions
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.pipe_name == pipe_name)
            .cloned()
            .collect();
        rows.sort_by(|a, b| b.id.cmp(&a.id));
        rows.truncate(limit.max(0) as usize);
        Ok(rows)
    }

    async fn mark_orphaned_running(&self) -> Result<u32> {
        let mut n = 0;
        for r in self.executions.lock().unwrap().iter_mut() {
            if matches!(r.status.as_str(), "queued" | "running") {
                r.status = "failed".to_string();
                r.error_type = Some("interrupted".to_string());
                r.finished_at = Some(Utc::now().to_rfc3339());
                n += 1;
            }
        }
        Ok(n)
    }

    async fn get_scheduler_state(&self, _pipe_name: &str) -> Result<Option<SchedulerState>> {
        Ok(None)
    }

    async fn upsert_scheduler_state(&self, _pipe_name: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    async fn cleanup_old_executions(&self, _keep_per_pipe: i32) -> Result<u32> {
        Ok(0)
    }

    async fn get_all_scheduler_states(&self) -> Result<HashMap<String, SchedulerState>> {
        Ok(HashMap::new())
    }

    async fn get_all_executions(
        &self,
        limit_per_pipe: i32,
    ) -> Result<HashMap<String, Vec<PipeExecution>>> {
        let names: HashSet<String> = self
            .executions
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.pipe_name.clone())
            .collect();
        let mut out = HashMap::new();
        for name in names {
            let rows = self.get_executions(&name, limit_per_pipe).await?;
            out.insert(name, rows);
        }
        Ok(out)
    }

    async fn claim_event_run(&self, pipe_name: &str, event: &str, key: &str) -> Result<bool> {
        self.claim_attempts.fetch_add(1, Ordering::SeqCst);
        Ok(self.claims.lock().unwrap().insert((
            pipe_name.to_string(),
            event.to_string(),
            key.to_string(),
        )))
    }

    async fn release_event_run(&self, pipe_name: &str, event: &str, key: &str) -> Result<()> {
        self.claims.lock().unwrap().remove(&(
            pipe_name.to_string(),
            event.to_string(),
            key.to_string(),
        ));
        Ok(())
    }

    async fn cancel_queued_execution(&self, id: i64) -> Result<bool> {
        Ok(self
            .with_row(id, |r| {
                let queued = r.status == "queued" && r.finished_at.is_none();
                if queued {
                    r.status = "cancelled".to_string();
                    r.error_type = Some("cancelled".to_string());
                    r.finished_at = Some(Utc::now().to_rfc3339());
                }
                queued
            })
            .unwrap_or(false))
    }

    async fn fail_completed_execution(
        &self,
        id: i64,
        error_type: &str,
        error_message: &str,
    ) -> Result<bool> {
        Ok(self
            .with_row(id, |r| {
                let clean = r.status == "completed" && r.error_type.is_none();
                if clean {
                    r.status = "failed".to_string();
                    r.error_type = Some(error_type.to_string());
                    r.error_message = Some(error_message.to_string());
                }
                clean
            })
            .unwrap_or(false))
    }

    async fn get_execution(&self, id: i64) -> Result<Option<PipeExecution>> {
        Ok(self.with_row(id, |r| r.clone()))
    }

    async fn interrupted_event_runs(
        &self,
        pipe_name: &str,
        event: &str,
        since: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<PipeExecution>> {
        let rows = self.executions.lock().unwrap().clone();
        let mut out: Vec<PipeExecution> = rows
            .iter()
            .filter(|r| {
                r.pipe_name == pipe_name
                    && r.trigger_event.as_deref() == Some(event)
                    && r.error_type.as_deref() == Some("interrupted")
                    && r
                        .finished_at
                        .as_deref()
                        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                        .is_some_and(|t| t >= since)
            })
            .filter(|r| {
                let family = key_family(r.trigger_key.as_deref().unwrap_or_default());
                !rows.iter().any(|n| {
                    n.id > r.id
                        && n.pipe_name == r.pipe_name
                        && n.trigger_event == r.trigger_event
                        && in_family(n.trigger_key.as_deref(), family)
                })
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| b.id.cmp(&a.id));
        out.truncate(limit as usize);
        Ok(out)
    }

    async fn count_event_runs(
        &self,
        pipe_name: &str,
        event: &str,
        key: &str,
        since: DateTime<Utc>,
    ) -> Result<u32> {
        Ok(self
            .executions
            .lock()
            .unwrap()
            .iter()
            .filter(|r| {
                r.pipe_name == pipe_name
                    && r.trigger_event.as_deref() == Some(event)
                    && in_family(r.trigger_key.as_deref(), key)
                    && r
                        .started_at
                        .as_deref()
                        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                        .is_some_and(|t| t >= since)
            })
            .count() as u32)
    }
}

/// An agent that answers every run with a fixed output and records the
/// prompts it was given.
pub(crate) struct ScriptedAgent {
    pub output: AgentOutput,
    pub prompts: StdMutex<Vec<String>>,
    pub delay: std::time::Duration,
}

impl ScriptedAgent {
    pub fn new(stdout: &str, success: bool) -> Self {
        Self {
            output: AgentOutput {
                stdout: stdout.to_string(),
                stderr: String::new(),
                success,
                pid: None,
            },
            prompts: StdMutex::new(Vec::new()),
            delay: std::time::Duration::from_millis(20),
        }
    }

    /// Runs whose prompt mentions `needle`.
    pub fn runs_mentioning(&self, needle: &str) -> usize {
        self.prompts
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.contains(needle))
            .count()
    }
}

#[async_trait::async_trait]
impl AgentExecutor for ScriptedAgent {
    async fn run(
        &self,
        prompt: &str,
        _model: &str,
        _working_dir: &Path,
        _provider: Option<&str>,
        _provider_url: Option<&str>,
        _provider_api_key: Option<&str>,
        _shared_pid: Option<SharedPid>,
        _continue_session: bool,
    ) -> Result<AgentOutput> {
        self.prompts.lock().unwrap().push(prompt.to_string());
        tokio::time::sleep(self.delay).await;
        Ok(self.output.clone())
    }

    fn kill(&self, _handle: &ExecutionHandle) -> Result<()> {
        Ok(())
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn ensure_installed(&self) -> Result<()> {
        Ok(())
    }

    fn name(&self) -> &str {
        "scripted"
    }
}

/// A hook that reports when the scheduler has subscribed to events (its
/// `on_scheduler_start` runs right after the subscriptions exist).
#[derive(Default)]
pub(crate) struct SchedulerReady {
    pub ready: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl super::hooks::PipeRunHook for SchedulerReady {
    fn name(&self) -> &str {
        "test-scheduler-ready"
    }

    async fn on_scheduler_start(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }
}

/// A finished Pi-style answer, as one JSON line.
pub(crate) fn finished_answer(text: &str) -> String {
    serde_json::json!({
        "type": "message_end",
        "message": {"role": "assistant", "content": [{"type": "text", "text": text}], "stopReason": "stop"}
    })
    .to_string()
}

/// Write a pipe triggered by `meeting_ended` into `<dir>/pipes/<name>` and
/// return the pipes directory.
pub(crate) fn write_event_pipe(dir: &Path, name: &str, agent: &str) -> PathBuf {
    let pipes_dir = dir.join("pipes");
    let pipe_dir = pipes_dir.join(name);
    std::fs::create_dir_all(&pipe_dir).unwrap();
    std::fs::write(
        pipe_dir.join("pipe.md"),
        format!(
            "---\nschedule: manual\nenabled: true\nagent: {agent}\ntrigger:\n  events:\n    - meeting_ended\n---\n\nSummarize the meeting.\n"
        ),
    )
    .unwrap();
    pipes_dir
}
