// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Watchdog for low-level input hooks.
//!
//! Windows calls a low-level keyboard or mouse hook synchronously for every
//! input event and waits at most `LowLevelHooksTimeout` for it. On Windows 7
//! and later a hook that misses that deadline is removed without any notice:
//! the hook handle still looks valid, the message loop keeps running, and
//! input capture simply stops. The only symptom is silence while the user is
//! clearly typing and clicking.
//!
//! [`HookWatchdog`] detects that symptom from two timestamps the hook thread
//! already has, the last hook callback and the system's last-input time, and
//! asks for a reinstall when input kept arriving but no callback did. The
//! system only reports its newest input, so the watchdog remembers the first
//! input it saw go unanswered: a user who never pauses still has hooks that
//! are found out once that input is old enough. It is
//! pure logic over millisecond timestamps, so it runs in every platform's
//! tests; the Win32 side (`platform::windows`) only feeds it numbers and
//! performs the reinstall through [`HookReinstaller`].
//!
//! Rules:
//! - A reinstall happens only from [`tick`], which the hook thread calls from
//!   its own message loop, so hooks are always replaced by the thread that
//!   owns them.
//! - Attempts in a streak back off exponentially up to a cap, so a hook that
//!   keeps failing (or keeps being removed) cannot make the thread spin. The
//!   streak ends once the hooks have delivered callbacks for a while.
//! - Input the hooks cannot see is set aside instead of counted: input to
//!   the secure desktop (lock screen, UAC prompt), and input to a window of
//!   a process running at a higher integrity level than ours (an elevated
//!   app while the engine is not elevated), which Windows withholds from
//!   lower-integrity low-level hooks. See [`hooks_can_see_input`].
//! - Logs carry durations and counts only, never input content.

use std::time::Duration;
use tracing::{debug, info, warn};

/// Tuning for [`HookWatchdog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookWatchdogConfig {
    /// How long the hooks must have been quiet, while the system reported
    /// newer input, before they count as removed.
    pub silence_threshold: Duration,
    /// How old an unanswered system input must be before its missing
    /// callback counts. Covers the moment between the system recording an
    /// input and the hook thread being called for it.
    pub callback_grace: Duration,
    /// System input counts as unanswered only when it is newer than the
    /// last callback by more than this. The system's input time comes from
    /// the coarse tick counter (about 16 ms steps), so the input a callback
    /// just answered can look a few milliseconds newer than that callback.
    pub clock_tolerance: Duration,
    /// Wait after the first attempt of a streak; doubles with each attempt.
    pub initial_backoff: Duration,
    /// Longest wait between two attempts.
    pub max_backoff: Duration,
    /// After an attempt, hooks that answer input and stay quiet-free this
    /// long end the streak, so the next problem starts from a short backoff.
    pub healthy_after: Duration,
}

impl Default for HookWatchdogConfig {
    fn default() -> Self {
        Self {
            silence_threshold: Duration::from_secs(3),
            callback_grace: Duration::from_secs(1),
            clock_tolerance: Duration::from_millis(100),
            initial_backoff: Duration::from_secs(5),
            max_backoff: Duration::from_secs(600),
            healthy_after: Duration::from_secs(60),
        }
    }
}

/// What the watchdog concluded from one look at the timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogVerdict {
    /// Callbacks keep up with system input (or there was no input).
    Healthy,
    /// The hooks look removed, but the backoff from the previous attempt
    /// has not run out yet.
    Waiting {
        silent_for_ms: u64,
        retry_in_ms: u64,
    },
    /// The hooks look removed and an attempt is due now.
    Reinstall { attempt: u32, silent_for_ms: u64 },
}

/// Detects silently removed hooks; see the module docs.
#[derive(Debug, Clone)]
pub struct HookWatchdog {
    config: HookWatchdogConfig,
    /// When the current hooks were installed. A callback older than this
    /// cannot answer input that came after it.
    installed_at_ms: u64,
    /// Attempts in the current streak.
    attempts: u32,
    last_attempt_at_ms: Option<u64>,
    next_attempt_at_ms: u64,
    /// System input at or before this time is ignored (see
    /// [`HookWatchdog::ignore_input_until`]).
    ignore_input_until_ms: u64,
    /// The first system input seen that no callback has answered yet. Kept
    /// across looks because the system only reports its newest input: while
    /// the user keeps typing, the newest input is always too fresh to count,
    /// but this one keeps ageing.
    first_unanswered_input_ms: Option<u64>,
    total_attempts: u64,
}

impl HookWatchdog {
    /// Watchdog for hooks installed at `installed_at_ms` (on the caller's
    /// millisecond clock; every later timestamp uses the same clock).
    pub fn new(config: HookWatchdogConfig, installed_at_ms: u64) -> Self {
        Self {
            config,
            installed_at_ms,
            attempts: 0,
            last_attempt_at_ms: None,
            next_attempt_at_ms: 0,
            ignore_input_until_ms: 0,
            first_unanswered_input_ms: None,
            total_attempts: 0,
        }
    }

    /// Attempts in the current streak.
    pub fn streak_attempts(&self) -> u32 {
        self.attempts
    }

    /// Attempts since the watchdog was created.
    pub fn total_attempts(&self) -> u64 {
        self.total_attempts
    }

    /// Wait before the next attempt once `attempts` attempts have been made
    /// in the current streak.
    pub fn backoff_after(&self, attempts: u32) -> Duration {
        if attempts == 0 {
            return Duration::ZERO;
        }
        let doublings = (attempts - 1).min(20);
        self.config
            .initial_backoff
            .saturating_mul(1u32 << doublings)
            .min(self.config.max_backoff)
    }

    /// Treat system input up to `now_ms` as input the hooks could not have
    /// seen (it went to another desktop), so it never triggers a reinstall.
    pub fn ignore_input_until(&mut self, now_ms: u64) {
        self.ignore_input_until_ms = self.ignore_input_until_ms.max(now_ms);
    }

    /// Look at the timestamps once. `last_callback_ms` is the newest hook
    /// callback (`None` if none yet); `last_input_ms` the newest system input
    /// (`None` if unknown).
    pub fn evaluate(
        &mut self,
        now_ms: u64,
        last_callback_ms: Option<u64>,
        last_input_ms: Option<u64>,
    ) -> WatchdogVerdict {
        let reference = last_callback_ms
            .unwrap_or(0)
            .max(self.installed_at_ms)
            .min(now_ms);
        let silent_for_ms = now_ms - reference;

        // Input up to here has been answered (or predates the current hooks,
        // or was set aside); only input after it can be missed.
        let answered_until = reference
            .saturating_add(millis(self.config.clock_tolerance))
            .max(self.ignore_input_until_ms);
        if self
            .first_unanswered_input_ms
            .is_some_and(|input| input <= answered_until)
        {
            self.first_unanswered_input_ms = None;
        }
        if self.first_unanswered_input_ms.is_none() {
            self.first_unanswered_input_ms =
                last_input_ms.filter(|&input| input > answered_until && input <= now_ms);
        }

        let missed_input = self.first_unanswered_input_ms.is_some_and(|input| {
            now_ms.saturating_sub(input) >= millis(self.config.callback_grace)
        });
        let silent = missed_input && silent_for_ms >= millis(self.config.silence_threshold);

        if !silent {
            self.maybe_end_streak(now_ms, last_callback_ms);
            return WatchdogVerdict::Healthy;
        }

        if now_ms < self.next_attempt_at_ms {
            return WatchdogVerdict::Waiting {
                silent_for_ms,
                retry_in_ms: self.next_attempt_at_ms - now_ms,
            };
        }
        WatchdogVerdict::Reinstall {
            attempt: self.attempts + 1,
            silent_for_ms,
        }
    }

    /// Record an attempt made at `now_ms`. A successful one makes `now_ms`
    /// the new install time; a failed one leaves the hooks counted as
    /// missing, so the next attempt follows after the backoff.
    pub fn record_attempt(&mut self, now_ms: u64, installed: bool) {
        self.attempts = self.attempts.saturating_add(1);
        self.total_attempts = self.total_attempts.saturating_add(1);
        self.last_attempt_at_ms = Some(now_ms);
        self.next_attempt_at_ms = now_ms.saturating_add(millis(self.backoff_after(self.attempts)));
        if installed {
            self.installed_at_ms = self.installed_at_ms.max(now_ms);
        }
    }

    fn maybe_end_streak(&mut self, now_ms: u64, last_callback_ms: Option<u64>) {
        let Some(attempt_at) = self.last_attempt_at_ms else {
            return;
        };
        if self.attempts == 0 {
            return;
        }
        let answered = last_callback_ms.is_some_and(|cb| cb > attempt_at);
        if answered && now_ms.saturating_sub(attempt_at) >= millis(self.config.healthy_after) {
            info!(
                attempts = self.attempts,
                "input hooks are delivering input again after reinstalling"
            );
            self.attempts = 0;
            self.next_attempt_at_ms = 0;
        }
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Relative identifier of the System mandatory integrity level. Windows
/// integrity levels are the last sub-authority of a token's mandatory label
/// SID: untrusted 0x0000, low 0x1000, medium 0x2000, high 0x3000 (elevated),
/// system 0x4000.
pub const SYSTEM_INTEGRITY_RID: u32 = 0x4000;

/// Where the newest input went, as far as the integrity check is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputTarget {
    /// No foreground window, or one whose process could not be named.
    Unknown,
    /// The foreground window's process runs at this integrity level.
    Integrity(u32),
    /// The foreground window's process exists but its integrity level could
    /// not be read, which in practice means it runs with more privileges
    /// than we do (elevated, system or protected).
    Unreadable,
}

/// Whether low-level hooks running at `own_integrity` are called for input
/// that goes to `target`.
///
/// Windows does not call a low-level hook for input that goes to a window of
/// a process with a higher integrity level than the hook's own process, so
/// while an elevated app has the focus a non-elevated engine's hooks stay
/// silent although the system reports input. That silence is no sign of a
/// removed hook. When our own level is unknown, the input is assumed to be
/// visible (the watchdog then behaves as if there were no check).
pub fn hooks_can_see_input(own_integrity: Option<u32>, target: InputTarget) -> bool {
    let Some(own) = own_integrity else {
        return true;
    };
    match target {
        InputTarget::Unknown => true,
        InputTarget::Integrity(level) => level <= own,
        InputTarget::Unreadable => own >= SYSTEM_INTEGRITY_RID,
    }
}

/// Replaces the low-level hooks. Implemented by the Win32 hook set, and by
/// test hooks.
pub trait HookReinstaller {
    /// Remove the current hooks (ignoring ones Windows already dropped) and
    /// install them again. Called only on the thread that owns the hooks.
    fn reinstall(&mut self) -> Result<(), String>;
}

/// What one [`tick`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickOutcome {
    Healthy,
    Waiting,
    /// The input went where the hooks cannot see it (the secure desktop, or
    /// a window of a more privileged process); nothing was done.
    InputHidden,
    Reinstalled {
        attempt: u32,
        silent_for_ms: u64,
    },
    ReinstallFailed {
        attempt: u32,
        silent_for_ms: u64,
        error: String,
    },
}

/// Evaluate the watchdog and, when an attempt is due, reinstall the hooks.
///
/// `input_visible_to_hooks` is asked only when an attempt is due: `false`
/// means the input went where the hooks cannot see it (another desktop, or
/// a window of a more privileged process), so it is set aside instead of
/// acted on.
pub fn tick(
    watchdog: &mut HookWatchdog,
    now_ms: u64,
    last_callback_ms: Option<u64>,
    last_input_ms: Option<u64>,
    hooks: &mut dyn HookReinstaller,
    input_visible_to_hooks: &dyn Fn() -> bool,
) -> TickOutcome {
    match watchdog.evaluate(now_ms, last_callback_ms, last_input_ms) {
        WatchdogVerdict::Healthy => TickOutcome::Healthy,
        WatchdogVerdict::Waiting {
            silent_for_ms,
            retry_in_ms,
        } => {
            debug!(
                silent_for_ms,
                retry_in_ms, "input hooks still silent; waiting before the next reinstall"
            );
            TickOutcome::Waiting
        }
        WatchdogVerdict::Reinstall {
            attempt,
            silent_for_ms,
        } => {
            if !input_visible_to_hooks() {
                debug!(
                    silent_for_ms,
                    "input went where the hooks cannot see it (secure desktop or a more privileged window); not reinstalling"
                );
                watchdog.ignore_input_until(now_ms);
                return TickOutcome::InputHidden;
            }
            match hooks.reinstall() {
                Ok(()) => {
                    watchdog.record_attempt(now_ms, true);
                    warn!(
                        attempt,
                        silent_for_ms,
                        next_retry_ms = millis(watchdog.backoff_after(attempt)),
                        "input hooks received no callbacks while the system reported input; reinstalled them"
                    );
                    TickOutcome::Reinstalled {
                        attempt,
                        silent_for_ms,
                    }
                }
                Err(error) => {
                    watchdog.record_attempt(now_ms, false);
                    warn!(
                        attempt,
                        silent_for_ms,
                        next_retry_ms = millis(watchdog.backoff_after(attempt)),
                        "reinstalling silent input hooks failed: {}",
                        error
                    );
                    TickOutcome::ReinstallFailed {
                        attempt,
                        silent_for_ms,
                        error,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test hook: counts reinstalls and can be told to fail.
    #[derive(Default)]
    struct TestHooks {
        reinstalls: u32,
        fail: bool,
    }

    impl HookReinstaller for TestHooks {
        fn reinstall(&mut self) -> Result<(), String> {
            self.reinstalls += 1;
            if self.fail {
                Err("SetWindowsHookExW failed (test)".into())
            } else {
                Ok(())
            }
        }
    }

    fn visible() -> bool {
        true
    }

    fn watchdog() -> HookWatchdog {
        HookWatchdog::new(HookWatchdogConfig::default(), 0)
    }

    #[test]
    fn simulated_hook_silence_with_active_input_triggers_reinstall() {
        let mut wd = watchdog();
        let mut hooks = TestHooks::default();
        // Hooks answered until t=10s, then Windows dropped them; the user
        // kept typing and the system kept recording input.
        let last_cb = Some(10_000);
        for now in (10_000..=12_500).step_by(500) {
            let input = Some(now);
            assert_eq!(
                tick(&mut wd, now, last_cb, input, &mut hooks, &visible),
                TickOutcome::Healthy,
                "too early at {now}"
            );
        }
        // Quiet for 3 s while the input of t=10.5s is still unanswered.
        let outcome = tick(&mut wd, 13_000, last_cb, Some(13_000), &mut hooks, &visible);
        assert_eq!(
            outcome,
            TickOutcome::Reinstalled {
                attempt: 1,
                silent_for_ms: 3_000
            }
        );
        assert_eq!(hooks.reinstalls, 1);

        // The fresh hooks answer the next keystrokes: no further reinstall.
        for now in (15_000..=120_000).step_by(500) {
            let outcome = tick(
                &mut wd,
                now,
                Some(now - 20),
                Some(now - 10),
                &mut hooks,
                &visible,
            );
            assert_eq!(outcome, TickOutcome::Healthy);
        }
        assert_eq!(hooks.reinstalls, 1);
        // Healthy for a minute after the attempt: the streak is over.
        assert_eq!(wd.streak_attempts(), 0);
    }

    #[test]
    fn removed_hooks_are_found_while_the_user_never_pauses() {
        let mut wd = watchdog();
        let mut hooks = TestHooks::default();
        // Hooks answered until t=10s and then went silent, while the user
        // kept typing: new input every 100 ms, so the newest input is never
        // older than the grace period.
        let last_cb = Some(10_000);
        let mut reinstalled_at = None;
        for now in (10_100..=20_000).step_by(100) {
            let outcome = tick(&mut wd, now, last_cb, Some(now - 30), &mut hooks, &visible);
            if matches!(outcome, TickOutcome::Reinstalled { attempt: 1, .. }) {
                reinstalled_at = Some(now);
                break;
            }
        }
        // Found once the hooks were quiet for the silence threshold (3 s),
        // not only at the user's first pause.
        assert_eq!(reinstalled_at, Some(13_000));
        assert_eq!(hooks.reinstalls, 1);

        // The new hooks answer, again with input every 100 ms: left alone.
        for now in (13_100..=200_000).step_by(100) {
            let outcome = tick(
                &mut wd,
                now,
                Some(now - 20),
                Some(now - 30),
                &mut hooks,
                &visible,
            );
            assert_eq!(outcome, TickOutcome::Healthy, "at {now}");
        }
        assert_eq!(hooks.reinstalls, 1);
    }

    #[test]
    fn coarse_input_clock_is_not_a_missed_callback() {
        let mut wd = watchdog();
        let mut hooks = TestHooks::default();
        // The system's input time is read from a ~16 ms tick counter, so
        // the last keystroke can look 15 ms newer than the callback that
        // answered it. An idle user afterwards must not cause reinstalls.
        for now in (5_000..600_000).step_by(500) {
            tick(&mut wd, now, Some(4_000), Some(4_015), &mut hooks, &visible);
        }
        assert_eq!(hooks.reinstalls, 0);
    }

    #[test]
    fn no_reinstall_without_input() {
        let mut wd = watchdog();
        let mut hooks = TestHooks::default();
        // Idle user: the system's last input is older than the last callback.
        for now in (0..3_600_000).step_by(1_000) {
            tick(&mut wd, now, Some(5_000), Some(4_990), &mut hooks, &visible);
        }
        // Unknown system input never triggers a reinstall either.
        for now in (0..600_000).step_by(1_000) {
            tick(&mut wd, now, None, None, &mut hooks, &visible);
        }
        assert_eq!(hooks.reinstalls, 0);
    }

    #[test]
    fn busy_but_answering_hooks_are_left_alone() {
        let mut wd = watchdog();
        let mut hooks = TestHooks::default();
        for now in (1_000..600_000).step_by(250) {
            // Input every tick, each answered within a few milliseconds,
            // and the newest input sometimes not yet answered.
            tick(
                &mut wd,
                now,
                Some(now - 240),
                Some(now - 5),
                &mut hooks,
                &visible,
            );
        }
        assert_eq!(hooks.reinstalls, 0);
    }

    #[test]
    fn input_not_yet_old_enough_is_not_a_missed_callback() {
        let mut wd = watchdog();
        // Hooks quiet for 10 s, then a keystroke 200 ms ago whose callback
        // may still be on its way.
        assert_eq!(
            wd.evaluate(20_000, Some(10_000), Some(19_800)),
            WatchdogVerdict::Healthy
        );
        assert!(matches!(
            wd.evaluate(21_000, Some(10_000), Some(19_800)),
            WatchdogVerdict::Reinstall { attempt: 1, .. }
        ));
    }

    #[test]
    fn hooks_that_keep_failing_back_off_and_cannot_spin() {
        let mut wd = watchdog();
        let mut hooks = TestHooks {
            fail: true,
            ..TestHooks::default()
        };
        // An hour of continuous input with hooks that never come back,
        // checked every 100 ms.
        let mut attempt_times = Vec::new();
        for now in (5_000..3_605_000).step_by(100) {
            let before = hooks.reinstalls;
            tick(
                &mut wd,
                now,
                Some(1_000),
                Some(now - 1_000),
                &mut hooks,
                &visible,
            );
            if hooks.reinstalls != before {
                attempt_times.push(now);
            }
        }
        // 5, 10, 20, ... 320 s, then every 600 s: 12 attempts in the hour.
        assert!(hooks.reinstalls <= 14, "{} attempts", hooks.reinstalls);
        assert!(hooks.reinstalls >= 8);
        let gaps: Vec<u64> = attempt_times.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.windows(2).all(|g| g[1] >= g[0]), "{gaps:?}");
        assert!(gaps.iter().all(|g| *g >= 5_000));
        assert_eq!(*gaps.last().unwrap(), 600_000);
    }

    #[test]
    fn hooks_removed_again_and_again_back_off_too() {
        let mut wd = watchdog();
        let mut hooks = TestHooks::default();
        // Each reinstall works for ten seconds, then the hook is dropped
        // again: the streak never ends, so attempts keep spreading out.
        let mut alive_until = 0u64;
        let mut last_cb = 1_000u64;
        for now in (5_000..1_805_000).step_by(100) {
            if now < alive_until {
                last_cb = now;
            }
            let before = hooks.reinstalls;
            tick(
                &mut wd,
                now,
                Some(last_cb),
                Some(now - 1_000),
                &mut hooks,
                &visible,
            );
            if hooks.reinstalls != before {
                alive_until = now + 10_000;
            }
        }
        assert!(hooks.reinstalls <= 12, "{} attempts", hooks.reinstalls);
        assert!(wd.streak_attempts() >= 5);
    }

    #[test]
    fn old_input_does_not_retrigger_after_a_reinstall() {
        let mut wd = watchdog();
        let mut hooks = TestHooks::default();
        assert!(matches!(
            tick(
                &mut wd,
                10_000,
                Some(1_000),
                Some(6_000),
                &mut hooks,
                &visible
            ),
            TickOutcome::Reinstalled { .. }
        ));
        // User went idle right after: the last input predates the reinstall.
        for now in (10_000..1_000_000).step_by(1_000) {
            tick(&mut wd, now, Some(1_000), Some(6_000), &mut hooks, &visible);
        }
        assert_eq!(hooks.reinstalls, 1);
    }

    #[test]
    fn input_on_a_hidden_desktop_is_set_aside() {
        let mut wd = watchdog();
        let mut hooks = TestHooks::default();
        let hidden = || false;
        // Lock screen: the user types a password the hooks cannot see.
        assert_eq!(
            tick(
                &mut wd,
                30_000,
                Some(1_000),
                Some(28_000),
                &mut hooks,
                &hidden
            ),
            TickOutcome::InputHidden
        );
        // Still the same (set-aside) input: nothing to do.
        assert_eq!(
            tick(
                &mut wd,
                31_000,
                Some(1_000),
                Some(28_000),
                &mut hooks,
                &visible
            ),
            TickOutcome::Healthy
        );
        assert_eq!(hooks.reinstalls, 0);
        // Back on the normal desktop, new unanswered input counts again.
        assert!(matches!(
            tick(
                &mut wd,
                40_000,
                Some(1_000),
                Some(38_000),
                &mut hooks,
                &visible
            ),
            TickOutcome::Reinstalled { attempt: 1, .. }
        ));
    }

    #[test]
    fn hooks_that_never_installed_are_retried_when_input_arrives() {
        // No callback has ever arrived (the first install failed).
        let mut wd = watchdog();
        let mut hooks = TestHooks::default();
        assert!(matches!(
            tick(&mut wd, 8_000, None, Some(6_500), &mut hooks, &visible),
            TickOutcome::Reinstalled { attempt: 1, .. }
        ));
    }

    #[test]
    fn input_to_more_privileged_windows_is_hidden_from_the_hooks() {
        const MEDIUM: u32 = 0x2000;
        const HIGH: u32 = 0x3000;
        // A normal engine and a normal app: the hooks see the input.
        assert!(hooks_can_see_input(
            Some(MEDIUM),
            InputTarget::Integrity(MEDIUM)
        ));
        assert!(hooks_can_see_input(
            Some(MEDIUM),
            InputTarget::Integrity(0x1000)
        ));
        // An elevated app in front of a normal engine: they do not.
        assert!(!hooks_can_see_input(
            Some(MEDIUM),
            InputTarget::Integrity(HIGH)
        ));
        assert!(!hooks_can_see_input(Some(MEDIUM), InputTarget::Unreadable));
        // An elevated engine sees elevated apps; only system-level hooks
        // are sure to see a process whose level cannot be read.
        assert!(hooks_can_see_input(
            Some(HIGH),
            InputTarget::Integrity(HIGH)
        ));
        assert!(!hooks_can_see_input(Some(HIGH), InputTarget::Unreadable));
        assert!(hooks_can_see_input(
            Some(SYSTEM_INTEGRITY_RID),
            InputTarget::Unreadable
        ));
        // Nothing to compare: behave as if there were no check.
        assert!(hooks_can_see_input(Some(MEDIUM), InputTarget::Unknown));
        assert!(hooks_can_see_input(None, InputTarget::Integrity(HIGH)));
        assert!(hooks_can_see_input(None, InputTarget::Unreadable));
    }

    #[test]
    fn typing_into_an_elevated_window_does_not_trigger_reinstalls() {
        let mut wd = watchdog();
        let mut hooks = TestHooks::default();
        let elevated_in_front =
            || hooks_can_see_input(Some(0x2000), InputTarget::Integrity(0x3000));
        // Ten minutes of typing into an elevated terminal: the system keeps
        // reporting input, the hooks hear nothing, and that is expected.
        let mut hidden = 0;
        for now in (5_000..605_000).step_by(500) {
            let outcome = tick(
                &mut wd,
                now,
                Some(1_000),
                Some(now - 1_200),
                &mut hooks,
                &elevated_in_front,
            );
            if outcome == TickOutcome::InputHidden {
                hidden += 1;
            }
        }
        assert_eq!(hooks.reinstalls, 0);
        assert!(hidden > 0);
        // Focus back on a normal window and the hooks still silent: now it
        // is a removed hook, and it is reinstalled.
        assert!(matches!(
            tick(
                &mut wd,
                610_000,
                Some(1_000),
                Some(608_500),
                &mut hooks,
                &visible
            ),
            TickOutcome::Reinstalled { attempt: 1, .. }
        ));
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        let wd = watchdog();
        assert_eq!(wd.backoff_after(0), Duration::ZERO);
        assert_eq!(wd.backoff_after(1), Duration::from_secs(5));
        assert_eq!(wd.backoff_after(2), Duration::from_secs(10));
        assert_eq!(wd.backoff_after(4), Duration::from_secs(40));
        assert_eq!(wd.backoff_after(8), Duration::from_secs(600));
        assert_eq!(wd.backoff_after(u32::MAX), Duration::from_secs(600));
    }
}
