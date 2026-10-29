// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Run-lifecycle hooks for pipes.
//!
//! Features that react to pipe runs (artifact registration, run finalizers,
//! trajectory recording and similar) implement [`PipeRunHook`] in their own
//! module and register it on the [`PipeManager`](super::PipeManager) with
//! `add_run_hook`. The pipe runtime calls every registered hook at three
//! points, on all of its run paths (manual, scheduled and event-triggered):
//!
//! - [`PipeRunHook::before_run`] after the execution row exists and just
//!   before the agent starts;
//! - [`PipeRunHook::after_run`] once the run's result is stored in the
//!   execution record, before the run-complete callback fires;
//! - [`PipeRunHook::on_event`] for each event the scheduler receives, before
//!   it matches events to pipe triggers;
//! - [`PipeRunHook::on_scheduler_start`] each time the scheduler starts, once
//!   it listens for events (so events a hook sends from there are heard).
//!
//! Hooks run one after another in registration order and are awaited, so a
//! hook that has slow work to do should spawn it. A hook that panics is
//! logged and skipped; it never takes the run down with it. Hooks observe and
//! cannot cancel a run.

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::Arc;

use futures::FutureExt;
use tracing::warn;

use super::PipeRunLog;

/// A pipe run that is about to start.
#[derive(Debug, Clone, Copy)]
pub struct RunStart<'a> {
    pub pipe_name: &'a str,
    /// What started the run: `manual`, `scheduled` or `event`.
    pub trigger: &'a str,
    /// Event behind an event-triggered run (`pipe_executions.trigger_event`).
    pub trigger_event: Option<&'a str>,
    /// Instance key of that event, e.g. the meeting id
    /// (`pipe_executions.trigger_key`).
    pub trigger_key: Option<&'a str>,
    /// Row id in `pipe_executions`, when the run is persisted.
    pub execution_id: Option<i64>,
    /// The pipe's directory (`<data dir>/pipes/<name>`).
    pub pipe_dir: &'a Path,
}

/// A pipe run that has finished, successfully or not.
#[derive(Debug, Clone, Copy)]
pub struct RunEnd<'a> {
    pub start: RunStart<'a>,
    /// The run log as written to disk.
    pub log: &'a PipeRunLog,
    /// Sanitised failure category (`timeout`, `crash`, `rate_limited`, ...)
    /// for failed runs; `None` on success.
    pub error_type: Option<&'a str>,
}

/// An event received by the pipe scheduler.
#[derive(Debug, Clone, Copy)]
pub struct PipeEvent<'a> {
    /// The name triggers match against (for `workflow_event`, the inner
    /// event type).
    pub name: &'a str,
    pub data: &'a serde_json::Value,
}

/// Something that wants to hear about pipe runs. Every method has an empty
/// default, so a hook implements only what it needs.
#[async_trait::async_trait]
pub trait PipeRunHook: Send + Sync {
    /// Short name used in logs.
    fn name(&self) -> &str;

    async fn before_run(&self, _run: &RunStart<'_>) {}

    async fn after_run(&self, _run: &RunEnd<'_>) {}

    async fn on_event(&self, _event: &PipeEvent<'_>) {}

    async fn on_scheduler_start(&self) {}
}

/// The hooks registered on a pipe manager.
#[derive(Clone, Default)]
pub struct PipeHooks {
    hooks: Vec<Arc<dyn PipeRunHook>>,
}

impl PipeHooks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a hook. Hooks run in the order they were registered.
    pub fn register(&mut self, hook: Arc<dyn PipeRunHook>) {
        self.hooks.push(hook);
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// Names of the registered hooks, in order.
    pub fn names(&self) -> Vec<String> {
        self.hooks.iter().map(|h| h.name().to_string()).collect()
    }

    pub async fn before_run(&self, run: &RunStart<'_>) {
        for hook in &self.hooks {
            guard(hook.name(), "before_run", hook.before_run(run)).await;
        }
    }

    pub async fn after_run(&self, run: &RunEnd<'_>) {
        for hook in &self.hooks {
            guard(hook.name(), "after_run", hook.after_run(run)).await;
        }
    }

    pub async fn on_event(&self, event: &PipeEvent<'_>) {
        for hook in &self.hooks {
            guard(hook.name(), "on_event", hook.on_event(event)).await;
        }
    }

    pub async fn on_scheduler_start(&self) {
        for hook in &self.hooks {
            guard(hook.name(), "on_scheduler_start", hook.on_scheduler_start()).await;
        }
    }
}

impl std::fmt::Debug for PipeHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.names()).finish()
    }
}

/// Await one hook call, containing a panic so the caller carries on.
async fn guard<F>(hook: &str, stage: &str, call: F)
where
    F: std::future::Future<Output = ()>,
{
    if AssertUnwindSafe(call).catch_unwind().await.is_err() {
        warn!("pipe hook '{}' panicked in {}; continuing", hook, stage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder {
        label: &'static str,
        seen: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl PipeRunHook for Recorder {
        fn name(&self) -> &str {
            self.label
        }
        async fn before_run(&self, run: &RunStart<'_>) {
            self.seen.lock().unwrap().push(format!(
                "{}:before:{}:{}:{:?}",
                self.label, run.pipe_name, run.trigger, run.execution_id
            ));
        }
        async fn after_run(&self, run: &RunEnd<'_>) {
            self.seen.lock().unwrap().push(format!(
                "{}:after:{}:{}:{:?}",
                self.label, run.start.pipe_name, run.log.success, run.error_type
            ));
        }
        async fn on_event(&self, event: &PipeEvent<'_>) {
            self.seen
                .lock()
                .unwrap()
                .push(format!("{}:event:{}", self.label, event.name));
        }
        async fn on_scheduler_start(&self) {
            self.seen
                .lock()
                .unwrap()
                .push(format!("{}:scheduler_start", self.label));
        }
    }

    struct Panicker;

    #[async_trait::async_trait]
    impl PipeRunHook for Panicker {
        fn name(&self) -> &str {
            "panicker"
        }
        async fn before_run(&self, _run: &RunStart<'_>) {
            panic!("hook bug");
        }
    }

    struct Silent;

    #[async_trait::async_trait]
    impl PipeRunHook for Silent {
        fn name(&self) -> &str {
            "silent"
        }
    }

    fn log(success: bool) -> PipeRunLog {
        PipeRunLog {
            pipe_name: "digest".into(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            success,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    #[tokio::test]
    async fn hooks_run_in_registration_order_at_every_stage() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = PipeHooks::new();
        assert!(hooks.is_empty());
        hooks.register(Arc::new(Recorder {
            label: "a",
            seen: seen.clone(),
        }));
        hooks.register(Arc::new(Silent));
        hooks.register(Arc::new(Recorder {
            label: "b",
            seen: seen.clone(),
        }));
        assert_eq!(hooks.len(), 3);
        assert_eq!(hooks.names(), vec!["a", "silent", "b"]);

        let dir = Path::new("/tmp/pipes/digest");
        let start = RunStart {
            pipe_name: "digest",
            trigger: "scheduled",
            trigger_event: None,
            trigger_key: None,
            execution_id: Some(7),
            pipe_dir: dir,
        };
        hooks.before_run(&start).await;
        let finished = log(false);
        hooks
            .after_run(&RunEnd {
                start,
                log: &finished,
                error_type: Some("timeout"),
            })
            .await;
        let data = serde_json::json!({"meeting_id": 3});
        hooks
            .on_event(&PipeEvent {
                name: "meeting_ended",
                data: &data,
            })
            .await;
        hooks.on_scheduler_start().await;

        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                "a:before:digest:scheduled:Some(7)",
                "b:before:digest:scheduled:Some(7)",
                "a:after:digest:false:Some(\"timeout\")",
                "b:after:digest:false:Some(\"timeout\")",
                "a:event:meeting_ended",
                "b:event:meeting_ended",
                "a:scheduler_start",
                "b:scheduler_start",
            ]
        );
    }

    #[tokio::test]
    async fn a_panicking_hook_does_not_stop_the_others() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = PipeHooks::new();
        hooks.register(Arc::new(Panicker));
        hooks.register(Arc::new(Recorder {
            label: "after-panic",
            seen: seen.clone(),
        }));
        hooks
            .before_run(&RunStart {
                pipe_name: "p",
                trigger: "manual",
                trigger_event: None,
                trigger_key: None,
                execution_id: None,
                pipe_dir: Path::new("."),
            })
            .await;
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["after-panic:before:p:manual:None"]
        );
    }

    #[tokio::test]
    async fn no_hooks_is_a_no_op() {
        let hooks = PipeHooks::default();
        let finished = log(true);
        let start = RunStart {
            pipe_name: "p",
            trigger: "event",
            trigger_event: Some("meeting_ended"),
            trigger_key: Some("12"),
            execution_id: None,
            pipe_dir: Path::new("."),
        };
        hooks.before_run(&start).await;
        hooks
            .after_run(&RunEnd {
                start,
                log: &finished,
                error_type: None,
            })
            .await;
        assert_eq!(format!("{hooks:?}"), "[]");
    }
}
