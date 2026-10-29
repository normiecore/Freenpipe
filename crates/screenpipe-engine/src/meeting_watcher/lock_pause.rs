// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Screen-lock bookkeeping for the meeting detectors' scan loops.
//!
//! While the screen is locked a detector skips its scan iterations entirely:
//! no process enumeration, no accessibility walk, no state transition. When
//! the screen unlocks, the detector learns how long it was paused so it can
//! move its timers forward by that much. Without that, a meeting in its end
//! grace period before the lock would be ended by the first scan after the
//! unlock, purely because the clock ran while nobody looked.

use std::time::{Duration, Instant};

/// What a scan loop does with one wake-up, as far as the lock is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockGate {
    /// The screen is locked: skip this iteration and change nothing.
    Skip,
    /// First iteration after an unlock. Scan, after moving the detector's
    /// timers forward by `paused_for`.
    Resume { paused_for: Duration },
    /// Not locked and not just unlocked: scan as usual.
    Run,
}

/// Remembers when the current lock was first seen.
#[derive(Debug, Clone, Default)]
pub struct LockPause {
    locked_since: Option<Instant>,
}

impl LockPause {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the lock state read at the start of an iteration.
    ///
    /// The pause is measured from the first iteration that saw the lock to
    /// the first that sees the screen unlocked, which is the stretch during
    /// which the loop scanned nothing.
    pub fn observe(&mut self, screen_locked: bool, now: Instant) -> LockGate {
        match (screen_locked, self.locked_since) {
            (true, None) => {
                self.locked_since = Some(now);
                LockGate::Skip
            }
            (true, Some(_)) => LockGate::Skip,
            (false, Some(since)) => {
                self.locked_since = None;
                LockGate::Resume {
                    paused_for: now.saturating_duration_since(since),
                }
            }
            (false, None) => LockGate::Run,
        }
    }

    /// The loop is currently paused by a lock.
    pub fn is_paused(&self) -> bool {
        self.locked_since.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_skips_until_unlock_then_reports_the_pause_once() {
        let t0 = Instant::now();
        let mut pause = LockPause::new();
        assert_eq!(pause.observe(false, t0), LockGate::Run);
        assert_eq!(
            pause.observe(true, t0 + Duration::from_secs(5)),
            LockGate::Skip
        );
        assert!(pause.is_paused());
        assert_eq!(
            pause.observe(true, t0 + Duration::from_secs(600)),
            LockGate::Skip
        );
        assert_eq!(
            pause.observe(false, t0 + Duration::from_secs(905)),
            LockGate::Resume {
                paused_for: Duration::from_secs(900)
            }
        );
        assert!(!pause.is_paused());
        assert_eq!(
            pause.observe(false, t0 + Duration::from_secs(910)),
            LockGate::Run
        );
    }
}
