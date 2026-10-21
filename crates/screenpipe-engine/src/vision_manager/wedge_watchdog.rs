// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Capture-loop wedge watchdog, run by the monitor watcher on every pass.
//!
//! Each capture loop publishes its stage, a heartbeat at the top of every
//! iteration, and when its capture attempts start and end
//! ([`screenpipe_screen::LoopLiveness`]). While the VisionManager is running,
//! the watchdog looks at every monitor that should be capturing and reports
//! two kinds of trouble:
//!
//! - **gone-silent stall**: the loop's heartbeat is older than the stall
//!   threshold while it sits in a stage that should never take that long (a
//!   screenshot, a probe, the hot-cache push...). Stages where a loop
//!   legitimately waits (cold focus wait, pause gate, trigger wait) never
//!   count. A running capture task whose loop never registered counts too.
//! - **still-attempting wedge**: the loop keeps turning and keeps starting
//!   capture attempts, but none of them has come back with an answer (a frame
//!   written or skipped, or an error) for longer than the wedge threshold;
//!   they all run into their deadline. A failing database writer explains
//!   that on its own, so it never triggers a restart.
//!
//! On a finding the watchdog logs one diagnostic line and restarts the
//! VisionManager (stop, then start), which builds fresh capture sessions for
//! every monitor. Restarts are bounded: a per-monitor cooldown that doubles
//! with each repeat (5 minutes up to an hour), and at most three restarts in
//! any hour across all monitors. Everything stays in the local log; nothing
//! is sent anywhere.
//!
//! Monitors the user paused, monitors without a capture task, and the time
//! the whole pipeline is paused (screen locked, DRM content, outside the
//! work-hours schedule) are never judged. The restart goes through
//! `VisionManager::stop`/`start`, which serialise with per-monitor pause and
//! resume and wait for the old capture tasks before new ones start.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use screenpipe_screen::{CaptureLoopStage, LoopLiveness};
use tracing::{debug, info, warn};

use super::manager::{VisionManager, VisionManagerStatus};

/// No answered capture attempt for this long, while attempts keep starting,
/// is a still-attempting wedge.
pub(crate) const WEDGE_AFTER: Duration = Duration::from_secs(120);
/// A loop heartbeat this old, outside an expected wait, is a gone-silent
/// stall.
pub(crate) const STALL_AFTER: Duration = Duration::from_secs(90);
/// Cooldown after a monitor's first restart; it doubles with each repeat.
pub(crate) const RESTART_COOLDOWN: Duration = Duration::from_secs(5 * 60);
/// Longest per-monitor cooldown.
pub(crate) const RESTART_COOLDOWN_MAX: Duration = Duration::from_secs(60 * 60);
/// Most restarts in any [`RESTART_BUDGET_WINDOW`], over all monitors.
pub(crate) const RESTART_BUDGET: usize = 3;
pub(crate) const RESTART_BUDGET_WINDOW: Duration = Duration::from_secs(60 * 60);

/// Thresholds and limits of the watchdog.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WatchdogConfig {
    pub wedge_after: Duration,
    pub stall_after: Duration,
    pub cooldown: Duration,
    pub cooldown_max: Duration,
    pub budget: usize,
    pub budget_window: Duration,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            wedge_after: WEDGE_AFTER,
            stall_after: STALL_AFTER,
            cooldown: RESTART_COOLDOWN,
            cooldown_max: RESTART_COOLDOWN_MAX,
            budget: RESTART_BUDGET,
            budget_window: RESTART_BUDGET_WINDOW,
        }
    }
}

/// The two kinds of stuck loop the watchdog restarts for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WedgeClass {
    StillAttempting,
    GoneSilent,
}

impl WedgeClass {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::StillAttempting => "still-attempting wedge",
            Self::GoneSilent => "gone-silent stall",
        }
    }
}

/// One stuck monitor, with the facts the diagnostic reports.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WedgeFinding {
    pub monitor_id: u32,
    pub class: WedgeClass,
    /// Stage and time spent in it; `None` when the loop never registered.
    pub stage: Option<(CaptureLoopStage, Duration)>,
    pub heartbeat_age: Option<Duration>,
    pub last_attempt_age: Option<Duration>,
    /// Time without an answered capture attempt.
    pub unanswered_for: Duration,
}

fn secs(d: Duration) -> String {
    format!("{}s", d.as_secs())
}

fn secs_or_never(d: Option<Duration>) -> String {
    d.map(secs).unwrap_or_else(|| "never".to_string())
}

impl std::fmt::Display for WedgeFinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stage = match self.stage {
            Some((stage, age)) => format!("stage {} frozen for {}", stage, secs(age)),
            None => "no loop stage ever entered".to_string(),
        };
        write!(
            f,
            "monitor {}: {} (status=Running, {}, loop heartbeat age {}, last attempt age {}, \
             no answered capture for {})",
            self.monitor_id,
            self.class.label(),
            stage,
            secs_or_never(self.heartbeat_age),
            secs_or_never(self.last_attempt_age),
            secs(self.unanswered_for),
        )
    }
}

/// Judge one tracked loop.
pub(crate) fn classify(
    l: &LoopLiveness,
    cfg: &WatchdogConfig,
    writer_failing: bool,
) -> Option<WedgeFinding> {
    let finding = |class| WedgeFinding {
        monitor_id: l.monitor_id,
        class,
        stage: Some((l.stage, l.stage_age)),
        heartbeat_age: Some(l.heartbeat_age),
        last_attempt_age: l.last_attempt_age,
        unanswered_for: l.unanswered_for(),
    };
    if l.heartbeat_age >= cfg.stall_after && !l.stage.is_expected_wait() {
        return Some(finding(WedgeClass::GoneSilent));
    }
    let attempting =
        l.open_attempt_age.is_some() || l.last_attempt_age.is_some_and(|a| a < cfg.wedge_after);
    if attempting && l.unanswered_for() >= cfg.wedge_after && !writer_failing {
        return Some(finding(WedgeClass::StillAttempting));
    }
    None
}

/// Why a finding did not lead to a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Deferral {
    /// This monitor was restarted recently; it may restart again in `remaining`.
    CoolingDown { remaining: Duration },
    /// The hourly restart budget is used up for `remaining`.
    BudgetExhausted { remaining: Duration },
}

impl std::fmt::Display for Deferral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CoolingDown { remaining } => write!(
                f,
                "the monitor was restarted recently (next restart allowed in {})",
                secs(*remaining)
            ),
            Self::BudgetExhausted { remaining } => write!(
                f,
                "the restart budget is used up (next restart allowed in {})",
                secs(*remaining)
            ),
        }
    }
}

/// Outcome of one assessment.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Verdict {
    Healthy,
    Restart(WedgeFinding),
    Deferred(WedgeFinding, Deferral),
}

#[derive(Debug, Clone, Copy)]
struct MonitorRestarts {
    last: Instant,
    /// Restarts in the current run (a quiet stretch of twice the longest
    /// cooldown ends the run).
    run: u32,
}

/// Something the watchdog can restart. `VisionManager` in production; tests
/// substitute their own.
#[allow(async_fn_in_trait)]
pub(crate) trait VisionRestartTarget {
    async fn stop_capture(&self) -> anyhow::Result<()>;
    async fn start_capture(&self) -> anyhow::Result<()>;
}

impl VisionRestartTarget for VisionManager {
    async fn stop_capture(&self) -> anyhow::Result<()> {
        self.stop().await
    }

    async fn start_capture(&self) -> anyhow::Result<()> {
        self.start().await
    }
}

/// Watchdog state kept by the monitor watcher across passes.
#[derive(Debug)]
pub(crate) struct WedgeWatchdog {
    cfg: WatchdogConfig,
    /// Monitors with a running capture task but no registered loop, and
    /// since when the watchdog has seen them that way.
    untracked_since: HashMap<u32, Instant>,
    per_monitor: HashMap<u32, MonitorRestarts>,
    recent_restarts: VecDeque<Instant>,
    /// Monitors whose deferred finding was already logged at info.
    deferral_logged: HashSet<u32>,
    /// Restarts performed by this watchdog.
    restarts: u64,
    /// Whether the whole pipeline is paused on purpose right now.
    paused_globally: fn() -> bool,
}

impl Default for WedgeWatchdog {
    fn default() -> Self {
        Self::new(WatchdogConfig::default())
    }
}

impl WedgeWatchdog {
    pub(crate) fn new(cfg: WatchdogConfig) -> Self {
        Self {
            cfg,
            untracked_since: HashMap::new(),
            per_monitor: HashMap::new(),
            recent_restarts: VecDeque::new(),
            deferral_logged: HashSet::new(),
            restarts: 0,
            paused_globally: capture_paused_globally,
        }
    }

    /// Use `check` instead of the process-wide pause flags (tests run beside
    /// others that flip those flags).
    #[cfg(test)]
    fn with_pause_check(mut self, check: fn() -> bool) -> Self {
        self.paused_globally = check;
        self
    }

    /// Restarts performed so far.
    #[cfg(test)]
    pub(crate) fn restarts(&self) -> u64 {
        self.restarts
    }

    fn cooldown_for(&self, run: u32) -> Duration {
        let factor = 1u32.checked_shl(run.saturating_sub(1)).unwrap_or(u32::MAX);
        self.cfg
            .cooldown
            .checked_mul(factor)
            .unwrap_or(self.cfg.cooldown_max)
            .min(self.cfg.cooldown_max)
    }

    fn deferral(&mut self, now: Instant, monitor_id: u32) -> Option<Deferral> {
        while let Some(&oldest) = self.recent_restarts.front() {
            if now.saturating_duration_since(oldest) >= self.cfg.budget_window {
                self.recent_restarts.pop_front();
            } else {
                break;
            }
        }
        if self.recent_restarts.len() >= self.cfg.budget {
            let oldest = self.recent_restarts.front().copied().unwrap_or(now);
            let remaining = self
                .cfg
                .budget_window
                .saturating_sub(now.saturating_duration_since(oldest));
            return Some(Deferral::BudgetExhausted { remaining });
        }
        if let Some(r) = self.per_monitor.get(&monitor_id) {
            let cooldown = self.cooldown_for(r.run);
            let since = now.saturating_duration_since(r.last);
            if since < cooldown {
                return Some(Deferral::CoolingDown {
                    remaining: cooldown - since,
                });
            }
        }
        None
    }

    /// Judge the loops of the monitors in `active` (those with a running
    /// capture task that the user has not paused). Loops of other monitors
    /// are ignored. At most one restart is asked for per pass.
    pub(crate) fn assess(
        &mut self,
        now: Instant,
        liveness: &[LoopLiveness],
        active: &[u32],
        writer_failing: bool,
    ) -> Verdict {
        let active_set: HashSet<u32> = active.iter().copied().collect();
        self.untracked_since.retain(|id, _| active_set.contains(id));
        let by_id: HashMap<u32, &LoopLiveness> =
            liveness.iter().map(|l| (l.monitor_id, l)).collect();

        let mut findings = Vec::new();
        let mut ids: Vec<u32> = active_set.iter().copied().collect();
        ids.sort_unstable();
        for id in ids {
            match by_id.get(&id) {
                Some(l) => {
                    self.untracked_since.remove(&id);
                    if let Some(f) = classify(l, &self.cfg, writer_failing) {
                        findings.push(f);
                    } else if writer_failing {
                        if let Some(f) = classify(l, &self.cfg, false) {
                            debug!(
                                "vision capture watchdog: {}; the database writer is failing, \
                                 so this is not treated as a capture wedge",
                                f
                            );
                        }
                    }
                }
                None => {
                    let since = *self.untracked_since.entry(id).or_insert(now);
                    let missing_for = now.saturating_duration_since(since);
                    if missing_for >= self.cfg.stall_after {
                        findings.push(WedgeFinding {
                            monitor_id: id,
                            class: WedgeClass::GoneSilent,
                            stage: None,
                            heartbeat_age: None,
                            last_attempt_age: None,
                            unanswered_for: missing_for,
                        });
                    }
                }
            }
        }

        let stuck: HashSet<u32> = findings.iter().map(|f| f.monitor_id).collect();
        self.deferral_logged.retain(|id| stuck.contains(id));

        let mut deferred = None;
        for finding in findings {
            match self.deferral(now, finding.monitor_id) {
                None => return Verdict::Restart(finding),
                Some(d) => {
                    if deferred.is_none() {
                        deferred = Some((finding, d));
                    }
                }
            }
        }
        match deferred {
            Some((finding, d)) => {
                if self.deferral_logged.insert(finding.monitor_id) {
                    info!(
                        "vision capture watchdog: {}; not restarting yet: {}",
                        finding, d
                    );
                } else {
                    debug!("vision capture watchdog: {}; deferred: {}", finding, d);
                }
                Verdict::Deferred(finding, d)
            }
            None => Verdict::Healthy,
        }
    }

    /// Book a restart for `monitor_id` at `now`.
    pub(crate) fn record_restart(&mut self, now: Instant, monitor_id: u32) {
        let quiet_resets_after = self.cfg.cooldown_max * 2;
        let run = match self.per_monitor.get(&monitor_id) {
            Some(r) if now.saturating_duration_since(r.last) < quiet_resets_after => {
                r.run.saturating_add(1)
            }
            _ => 1,
        };
        self.per_monitor
            .insert(monitor_id, MonitorRestarts { last: now, run });
        self.recent_restarts.push_back(now);
        self.untracked_since.clear();
        self.deferral_logged.remove(&monitor_id);
        self.restarts += 1;
    }

    /// One watchdog pass against the running VisionManager. Returns true when
    /// it restarted the manager.
    pub(crate) async fn run_pass(&mut self, vm: &VisionManager) -> bool {
        if vm.status().await != VisionManagerStatus::Running {
            self.untracked_since.clear();
            return false;
        }
        if (self.paused_globally)() {
            // Paused loops sit in the pause gate; nothing to judge.
            self.untracked_since.clear();
            return false;
        }
        let active: Vec<u32> = vm
            .active_monitors()
            .await
            .into_iter()
            .filter(|id| !vm.is_user_paused(*id))
            .collect();
        let liveness = vm.vision_metrics().loop_stages.liveness();
        let writer_failing = screenpipe_db::write_queue_health::write_queue_health().degraded;
        match self.assess(Instant::now(), &liveness, &active, writer_failing) {
            Verdict::Restart(finding) => {
                restart_vision(vm, &finding).await;
                self.record_restart(Instant::now(), finding.monitor_id);
                true
            }
            Verdict::Healthy | Verdict::Deferred(..) => false,
        }
    }
}

/// True while every capture loop is held in its pause gate on purpose.
fn capture_paused_globally() -> bool {
    crate::sleep_monitor::screen_is_locked()
        || crate::drm_detector::drm_content_paused()
        || crate::schedule_monitor::schedule_paused()
}

/// Stop and start `target` because of `finding`. Logs locally only.
pub(crate) async fn restart_vision<T: VisionRestartTarget>(target: &T, finding: &WedgeFinding) {
    warn!(
        monitor_id = finding.monitor_id,
        "vision capture watchdog: {}; restarting the VisionManager", finding
    );
    if let Err(e) = target.stop_capture().await {
        warn!(
            monitor_id = finding.monitor_id,
            "vision capture watchdog: stopping the VisionManager after a {} on monitor {} \
             failed: {:#}",
            finding.class.label(),
            finding.monitor_id,
            e
        );
    }
    match target.start_capture().await {
        Ok(()) => info!(
            monitor_id = finding.monitor_id,
            "vision capture watchdog: VisionManager restarted after a {} on monitor {}",
            finding.class.label(),
            finding.monitor_id
        ),
        Err(e) => warn!(
            monitor_id = finding.monitor_id,
            "vision capture watchdog: restart after a {} on monitor {} could not start \
             capture ({:#}); the monitor watcher keeps retrying",
            finding.class.label(),
            finding.monitor_id,
            e
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use screenpipe_screen::{AttemptEnd, PipelineMetrics};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::task::JoinHandle;

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn live(monitor_id: u32) -> LoopLiveness {
        LoopLiveness {
            monitor_id,
            stage: CaptureLoopStage::TriggerWait,
            stage_age: S(0),
            loop_age: S(3600),
            heartbeat_age: S(0),
            last_attempt_age: Some(S(5)),
            open_attempt_age: None,
            last_answer_age: Some(S(5)),
        }
    }

    #[test]
    fn a_turning_loop_is_healthy() {
        let cfg = WatchdogConfig::default();
        assert_eq!(classify(&live(1), &cfg, false), None);
        // Long waits in the expected stages are fine.
        for stage in [
            CaptureLoopStage::ColdWait,
            CaptureLoopStage::PauseGate,
            CaptureLoopStage::TriggerWait,
        ] {
            let l = LoopLiveness {
                stage,
                stage_age: S(600),
                heartbeat_age: S(600),
                last_attempt_age: Some(S(600)),
                last_answer_age: Some(S(600)),
                ..live(1)
            };
            assert_eq!(classify(&l, &cfg, false), None, "{stage}");
        }
        // A static screen: attempts answered by dedup are answers.
        let l = LoopLiveness {
            last_attempt_age: Some(S(2)),
            last_answer_age: Some(S(2)),
            ..live(1)
        };
        assert_eq!(classify(&l, &cfg, false), None);
        // Not attempting for a long time (idle, focus away) is not a wedge.
        let l = LoopLiveness {
            last_attempt_age: Some(S(900)),
            last_answer_age: Some(S(900)),
            ..live(1)
        };
        assert_eq!(classify(&l, &cfg, false), None);
    }

    #[test]
    fn a_loop_frozen_in_a_working_stage_is_a_gone_silent_stall() {
        let cfg = WatchdogConfig::default();
        for stage in [
            CaptureLoopStage::Capture,
            CaptureLoopStage::VisualProbe,
            CaptureLoopStage::ExclusionProbe,
            CaptureLoopStage::ReleaseStream,
            CaptureLoopStage::HotCachePush,
            CaptureLoopStage::Heartbeat,
        ] {
            let l = LoopLiveness {
                stage,
                stage_age: S(95),
                heartbeat_age: S(95),
                ..live(2)
            };
            let f = classify(&l, &cfg, false).expect("stall");
            assert_eq!(f.class, WedgeClass::GoneSilent, "{stage}");
        }
        // Just below the threshold: not yet.
        let l = LoopLiveness {
            stage: CaptureLoopStage::Capture,
            heartbeat_age: S(89),
            ..live(2)
        };
        assert_eq!(classify(&l, &cfg, false), None);
    }

    #[test]
    fn attempts_that_never_answer_are_a_still_attempting_wedge() {
        let cfg = WatchdogConfig::default();
        let l = LoopLiveness {
            stage: CaptureLoopStage::Capture,
            stage_age: S(3),
            heartbeat_age: S(20),
            last_attempt_age: Some(S(3)),
            open_attempt_age: Some(S(3)),
            last_answer_age: Some(S(150)),
            ..live(3)
        };
        let f = classify(&l, &cfg, false).expect("wedge");
        assert_eq!(f.class, WedgeClass::StillAttempting);
        assert_eq!(f.unanswered_for, S(150));
        // A failing database writer explains it: no restart.
        assert_eq!(classify(&l, &cfg, true), None);
        // A loop that never got an answer counts from its start.
        let fresh = LoopLiveness {
            loop_age: S(60),
            last_answer_age: None,
            ..l
        };
        assert_eq!(classify(&fresh, &cfg, false), None);
        let old = LoopLiveness {
            loop_age: S(130),
            last_answer_age: None,
            ..l
        };
        assert!(classify(&old, &cfg, false).is_some());
    }

    #[test]
    fn the_diagnostic_names_monitor_class_stage_and_ages() {
        let f = WedgeFinding {
            monitor_id: 65537,
            class: WedgeClass::GoneSilent,
            stage: Some((CaptureLoopStage::VisualProbe, S(97))),
            heartbeat_age: Some(S(97)),
            last_attempt_age: Some(S(130)),
            unanswered_for: S(130),
        };
        let text = f.to_string();
        for part in [
            "monitor 65537",
            "gone-silent stall",
            "status=Running",
            "visual-probe frozen for 97s",
            "heartbeat age 97s",
            "last attempt age 130s",
            "no answered capture for 130s",
        ] {
            assert!(text.contains(part), "{part} missing in {text}");
        }
        let never = WedgeFinding {
            stage: None,
            heartbeat_age: None,
            last_attempt_age: None,
            ..f
        };
        assert!(never.to_string().contains("no loop stage ever entered"));
        assert!(never.to_string().contains("heartbeat age never"));
    }

    #[test]
    fn only_active_monitors_are_judged() {
        let mut wd = WedgeWatchdog::default();
        let now = Instant::now();
        let stuck = LoopLiveness {
            stage: CaptureLoopStage::Capture,
            heartbeat_age: S(300),
            ..live(4)
        };
        // Monitor 4 has no running task (paused by the user, or stopped).
        assert_eq!(wd.assess(now, &[stuck], &[], false), Verdict::Healthy);
        assert!(matches!(
            wd.assess(now, &[stuck], &[4], false),
            Verdict::Restart(f) if f.monitor_id == 4
        ));
    }

    #[test]
    fn a_task_whose_loop_never_registered_is_reported_after_the_stall_threshold() {
        let mut wd = WedgeWatchdog::default();
        let t0 = Instant::now();
        assert_eq!(wd.assess(t0, &[], &[9], false), Verdict::Healthy);
        assert_eq!(wd.assess(t0 + S(60), &[], &[9], false), Verdict::Healthy);
        match wd.assess(t0 + S(91), &[], &[9], false) {
            Verdict::Restart(f) => {
                assert_eq!(f.class, WedgeClass::GoneSilent);
                assert_eq!(f.stage, None);
            }
            other => panic!("expected a restart, got {other:?}"),
        }
        // Once the loop shows up, the clock is forgotten.
        assert_eq!(
            wd.assess(t0 + S(92), &[live(9)], &[9], false),
            Verdict::Healthy
        );
        assert_eq!(wd.assess(t0 + S(93), &[], &[9], false), Verdict::Healthy);
    }

    #[test]
    fn restarts_back_off_per_monitor_and_respect_the_hourly_budget() {
        let mut wd = WedgeWatchdog::default();
        let stuck = |id| LoopLiveness {
            stage: CaptureLoopStage::Capture,
            heartbeat_age: S(100),
            ..live(id)
        };
        let t0 = Instant::now();
        let restart_at = |wd: &mut WedgeWatchdog, t: Instant, id: u32| -> Verdict {
            let v = wd.assess(t, &[stuck(id)], &[id], false);
            if let Verdict::Restart(ref f) = v {
                wd.record_restart(t, f.monitor_id);
            }
            v
        };

        assert!(matches!(restart_at(&mut wd, t0, 1), Verdict::Restart(_)));
        // Within the 5-minute cooldown: deferred, however often it is asked.
        for s in [5, 60, 299] {
            assert!(matches!(
                restart_at(&mut wd, t0 + S(s), 1),
                Verdict::Deferred(_, Deferral::CoolingDown { .. })
            ));
        }
        assert!(matches!(
            restart_at(&mut wd, t0 + S(300), 1),
            Verdict::Restart(_)
        ));
        // The second cooldown is twice as long.
        assert!(matches!(
            restart_at(&mut wd, t0 + S(300 + 599), 1),
            Verdict::Deferred(_, Deferral::CoolingDown { .. })
        ));
        assert!(matches!(
            restart_at(&mut wd, t0 + S(900), 1),
            Verdict::Restart(_)
        ));
        // Three restarts in the hour: everything waits for the window.
        assert!(matches!(
            restart_at(&mut wd, t0 + S(1000), 2),
            Verdict::Deferred(_, Deferral::BudgetExhausted { .. })
        ));
        assert!(matches!(
            restart_at(&mut wd, t0 + S(3600), 2),
            Verdict::Restart(_)
        ));
        assert_eq!(wd.restarts(), 4);
    }

    #[test]
    fn the_cooldown_doubles_up_to_the_cap_and_resets_after_a_quiet_stretch() {
        let wd = WedgeWatchdog::default();
        assert_eq!(wd.cooldown_for(1), S(300));
        assert_eq!(wd.cooldown_for(2), S(600));
        assert_eq!(wd.cooldown_for(4), S(2400));
        assert_eq!(wd.cooldown_for(5), S(3600));
        assert_eq!(wd.cooldown_for(40), S(3600));

        let mut wd = WedgeWatchdog::default();
        let t0 = Instant::now();
        wd.record_restart(t0, 1);
        wd.record_restart(t0 + S(400), 1);
        assert_eq!(wd.per_monitor[&1].run, 2);
        wd.record_restart(t0 + S(400) + S(7200), 1);
        assert_eq!(wd.per_monitor[&1].run, 1);
    }

    // --- Simulated hang through the restart path --------------------------

    /// Stands in for the VisionManager: "starting" spawns one capture loop
    /// per monitor that registers with the metrics and either hangs inside
    /// the capture stage (the simulated wedge) or keeps turning.
    struct FakeVision {
        metrics: Arc<PipelineMetrics>,
        tasks: Mutex<Vec<(u32, JoinHandle<()>)>>,
        hang_on_next_start: std::sync::atomic::AtomicBool,
        stops: AtomicU32,
        starts: AtomicU32,
    }

    impl FakeVision {
        fn new() -> Self {
            Self {
                metrics: Arc::new(PipelineMetrics::new()),
                tasks: Mutex::new(Vec::new()),
                hang_on_next_start: std::sync::atomic::AtomicBool::new(true),
                stops: AtomicU32::new(0),
                starts: AtomicU32::new(0),
            }
        }

        fn spawn_loop(&self, monitor_id: u32, hang: bool) {
            let metrics = self.metrics.clone();
            let task = tokio::spawn(async move {
                let stage = metrics.track_capture_loop(monitor_id);
                if hang {
                    stage.attempt_started();
                    stage.enter(CaptureLoopStage::Capture);
                    // The test hook: a capture that never returns.
                    std::future::pending::<()>().await;
                }
                loop {
                    stage.enter(CaptureLoopStage::Heartbeat);
                    stage.attempt_started();
                    stage.enter(CaptureLoopStage::Capture);
                    stage.attempt_finished(AttemptEnd::Answered);
                    stage.enter(CaptureLoopStage::TriggerWait);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            });
            self.tasks.lock().unwrap().push((monitor_id, task));
        }

        fn active(&self) -> Vec<u32> {
            self.tasks
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, t)| !t.is_finished())
                .map(|(id, _)| *id)
                .collect()
        }
    }

    impl VisionRestartTarget for FakeVision {
        async fn stop_capture(&self) -> anyhow::Result<()> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            let tasks: Vec<_> = self.tasks.lock().unwrap().drain(..).collect();
            for (_, t) in tasks {
                t.abort();
                let _ = t.await;
            }
            Ok(())
        }

        async fn start_capture(&self) -> anyhow::Result<()> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            let hang = self.hang_on_next_start.swap(false, Ordering::SeqCst);
            self.spawn_loop(1, hang);
            Ok(())
        }
    }

    fn fast_config() -> WatchdogConfig {
        WatchdogConfig {
            wedge_after: Duration::from_millis(400),
            stall_after: Duration::from_millis(150),
            cooldown: Duration::from_secs(60),
            ..WatchdogConfig::default()
        }
    }

    #[tokio::test]
    async fn a_hang_in_the_capture_stage_restarts_capture_once() {
        let vision = FakeVision::new();
        vision.start_capture().await.unwrap();
        let mut wd = WedgeWatchdog::new(fast_config());

        let deadline = Instant::now() + Duration::from_secs(10);
        let finding = loop {
            assert!(Instant::now() < deadline, "the hang was never detected");
            let liveness = vision.metrics.loop_stages.liveness();
            match wd.assess(Instant::now(), &liveness, &vision.active(), false) {
                Verdict::Restart(f) => break f,
                Verdict::Healthy => tokio::time::sleep(Duration::from_millis(20)).await,
                Verdict::Deferred(f, d) => panic!("unexpected deferral {f}: {d}"),
            }
        };
        assert_eq!(finding.monitor_id, 1);
        assert_eq!(finding.class, WedgeClass::GoneSilent);
        assert_eq!(
            finding.stage.map(|(s, _)| s),
            Some(CaptureLoopStage::Capture)
        );

        restart_vision(&vision, &finding).await;
        wd.record_restart(Instant::now(), finding.monitor_id);
        assert_eq!(vision.stops.load(Ordering::SeqCst), 1);
        assert_eq!(vision.starts.load(Ordering::SeqCst), 2);
        // Exactly one capture loop runs for the monitor afterwards.
        assert_eq!(vision.active(), vec![1]);

        // The replacement loop turns; repeated passes find nothing to do.
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let liveness = vision.metrics.loop_stages.liveness();
            assert_eq!(
                wd.assess(Instant::now(), &liveness, &vision.active(), false),
                Verdict::Healthy
            );
        }
        assert_eq!(wd.restarts(), 1);
        vision.stop_capture().await.unwrap();
        assert!(!vision.metrics.loop_stages.is_tracked(1));
    }

    #[tokio::test]
    async fn a_hang_that_comes_back_never_loops_restarts() {
        let vision = FakeVision::new();
        vision.start_capture().await.unwrap();
        let mut wd = WedgeWatchdog::new(fast_config());
        let mut restarts = 0;
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(1500) {
            // Every replacement hangs too.
            vision.hang_on_next_start.store(true, Ordering::SeqCst);
            let liveness = vision.metrics.loop_stages.liveness();
            if let Verdict::Restart(f) =
                wd.assess(Instant::now(), &liveness, &vision.active(), false)
            {
                restart_vision(&vision, &f).await;
                wd.record_restart(Instant::now(), f.monitor_id);
                restarts += 1;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // The 60 s cooldown allows a single restart in this window.
        assert_eq!(restarts, 1);
        vision.stop_capture().await.unwrap();
    }

    /// The real VisionManager: a capture task hung inside the capture stage
    /// is detected, the manager is stopped (ending the hung task and freeing
    /// its stage cell) and started again.
    #[tokio::test]
    async fn a_hung_capture_task_restarts_the_vision_manager() {
        use super::super::manager::{VisionDeviceControl, VisionManagerConfig};
        use screenpipe_db::DatabaseManager;
        use std::sync::Arc;

        let db = Arc::new(
            DatabaseManager::new("sqlite::memory:", Default::default())
                .await
                .unwrap(),
        );
        let tmp = tempfile::tempdir().unwrap();
        let config = VisionManagerConfig {
            output_path: tmp.path().to_string_lossy().into_owned(),
            ignored_windows: vec![],
            included_windows: vec![],
            ignored_urls: vec![],
            vision_metrics: Arc::new(PipelineMetrics::default()),
            use_pii_removal: false,
            monitor_ids: vec![],
            use_all_monitors: true,
            ignore_incognito_windows: false,
            pause_on_drm_content: false,
            languages: vec![],
            video_quality: "balanced".to_string(),
            idle_capture_interval_ms: None,
            visual_check_interval_ms: None,
            visual_change_threshold: None,
            min_capture_interval_ms: None,
            capture_on_keystroke: None,
            capture_on_clipboard: None,
        };
        // Every display a CI runner may have is paused, so the real start()
        // inside the restart records nothing.
        let control = Arc::new(VisionDeviceControl::new());
        for monitor in screenpipe_screen::monitor::list_monitors().await {
            control.mark_paused(monitor.id());
        }
        let vm = VisionManager::new(config, db, tokio::runtime::Handle::current())
            .with_device_control(control);
        vm.set_status_for_test(VisionManagerStatus::Running).await;

        let metrics = vm.vision_metrics().clone();
        let (gone_tx, gone_rx) = tokio::sync::oneshot::channel::<()>();
        vm.insert_capture_task_for_test(
            42,
            tokio::spawn(async move {
                let _gone = gone_tx;
                let stage = metrics.track_capture_loop(42);
                stage.attempt_started();
                stage.enter(CaptureLoopStage::Capture);
                std::future::pending::<()>().await;
            }),
        );

        let mut wd = WedgeWatchdog::new(fast_config()).with_pause_check(|| false);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!wd.run_pass(&vm).await, "too early to call it a stall");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(wd.run_pass(&vm).await, "the hung task triggers a restart");
        assert_eq!(wd.restarts(), 1);

        // The hung task was aborted, not left running beside a new one.
        let ended = tokio::time::timeout(Duration::from_secs(5), gone_rx).await;
        assert!(matches!(ended, Ok(Err(_))), "hung capture task ended");
        assert!(!vm.vision_metrics().loop_stages.is_tracked(42));
        assert!(!vm.active_monitors().await.contains(&42));
        // A second pass right away does not restart again.
        assert!(!wd.run_pass(&vm).await);
        vm.stop().await.unwrap();
    }
}
