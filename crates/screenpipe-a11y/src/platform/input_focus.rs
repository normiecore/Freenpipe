// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Which window input went to, kept in input order.
//!
//! The input worker handles raw input some time after the hook saw it.
//! Normally that is a fraction of a millisecond, but the worker can fall
//! behind, and judging queued input against whatever window is in front by
//! then would attribute it to the wrong app. Worse, keys typed into an
//! excluded app (a password manager) would be checked against the app that
//! got the focus afterwards and could be stored as its text.
//!
//! So the app observer publishes each focus change here under a new epoch,
//! together with its capture decision; the hook callback stamps every raw
//! input with the epoch current at that moment (one atomic load, no lock);
//! and the worker looks up the window of that epoch. Input whose epoch has
//! dropped out of the bounded history is not recorded.

use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// How many focus changes the history keeps. The worker would have to fall
/// behind by this many window switches before input is dropped for being
/// too old to attribute.
pub const FOCUS_HISTORY_LEN: usize = 64;

/// A focused window as the input worker needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusTarget {
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    /// Whether input to this window may be recorded (the capture filters,
    /// decided once when the focus changed).
    pub capture: bool,
}

/// Focus changes by epoch; shared by the app observer (writer), the hook
/// callback (reads the epoch) and the input worker (looks epochs up).
#[derive(Debug)]
pub struct FocusTracker {
    current: AtomicU64,
    history: Mutex<VecDeque<(u64, Arc<FocusTarget>)>>,
    capacity: usize,
}

impl FocusTracker {
    /// A tracker whose epoch 0 is `initial` (in effect until the observer
    /// reports the first focused window).
    pub fn new(initial: FocusTarget) -> Self {
        Self::with_capacity(initial, FOCUS_HISTORY_LEN)
    }

    /// Like [`FocusTracker::new`] with a history of `capacity` entries (at
    /// least one).
    pub fn with_capacity(initial: FocusTarget, capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let mut history = VecDeque::with_capacity(capacity);
        history.push_back((0, Arc::new(initial)));
        Self {
            current: AtomicU64::new(0),
            history: Mutex::new(history),
            capacity,
        }
    }

    /// Epoch of the window in front right now. Lock-free; this is what the
    /// hook callback stamps on raw input.
    pub fn current_epoch(&self) -> u64 {
        self.current.load(Ordering::Acquire)
    }

    /// Record a focus change and return its epoch. The entry is in the
    /// history before its epoch becomes current, so any epoch a callback
    /// reads can be looked up.
    pub fn publish(&self, target: FocusTarget) -> u64 {
        let mut history = self.history.lock();
        let epoch = history.back().map_or(0, |(epoch, _)| *epoch) + 1;
        history.push_back((epoch, Arc::new(target)));
        while history.len() > self.capacity {
            history.pop_front();
        }
        self.current.store(epoch, Ordering::Release);
        epoch
    }

    /// The window of `epoch`, or `None` when it has left the history.
    pub fn lookup(&self, epoch: u64) -> Option<Arc<FocusTarget>> {
        self.history
            .lock()
            .iter()
            .rev()
            .find(|(e, _)| *e == epoch)
            .map(|(_, target)| target.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(app: &str, capture: bool) -> FocusTarget {
        FocusTarget {
            app_name: Some(app.to_string()),
            window_title: None,
            capture,
        }
    }

    #[test]
    fn input_keeps_the_window_it_was_typed_in() {
        let tracker = FocusTracker::new(target("start", true));
        assert_eq!(tracker.current_epoch(), 0);

        let vault = tracker.publish(target("KeePassXC.exe", false));
        // Keys typed now carry the vault's epoch ...
        let stamped = tracker.current_epoch();
        assert_eq!(stamped, vault);
        // ... and keep it after the user moved on to the browser.
        let browser = tracker.publish(target("chrome.exe", true));
        assert_ne!(browser, vault);
        let typed_in = tracker.lookup(stamped).unwrap();
        assert_eq!(typed_in.app_name.as_deref(), Some("KeePassXC.exe"));
        assert!(!typed_in.capture);
        assert!(tracker.lookup(browser).unwrap().capture);
        assert!(tracker.lookup(0).is_some());
    }

    #[test]
    fn epochs_older_than_the_history_are_unknown() {
        let tracker = FocusTracker::with_capacity(target("start", true), 4);
        let first = tracker.publish(target("a", true));
        for i in 0..4 {
            tracker.publish(target(&format!("app{i}"), true));
        }
        assert!(tracker.lookup(first).is_none());
        assert!(tracker.lookup(0).is_none());
        assert!(tracker.lookup(tracker.current_epoch()).is_some());
        assert!(tracker.lookup(u64::MAX).is_none());
    }

    #[test]
    fn concurrent_readers_always_find_the_current_epoch() {
        let tracker = Arc::new(FocusTracker::with_capacity(target("start", true), 8));
        let writer = {
            let tracker = tracker.clone();
            std::thread::spawn(move || {
                for i in 0..5_000 {
                    tracker.publish(target(&format!("app{i}"), i % 2 == 0));
                }
            })
        };
        for _ in 0..5_000 {
            let epoch = tracker.current_epoch();
            // An epoch read while the writer runs is in the history unless
            // eight newer changes pushed it out in the meantime.
            if let Some(found) = tracker.lookup(epoch) {
                assert!(found.app_name.is_some());
            } else {
                assert!(tracker.current_epoch() >= epoch + 8);
            }
        }
        writer.join().unwrap();
        assert_eq!(tracker.current_epoch(), 5_000);
    }
}
