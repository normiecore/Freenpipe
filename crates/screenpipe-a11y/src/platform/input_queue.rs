// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Hand-off queue between a low-level input hook and its worker thread.
//!
//! A low-level hook callback has to return within `LowLevelHooksTimeout` or
//! Windows drops the hook, so the callback only records the raw input and
//! pushes it here; a worker thread turns raw input into UI events (window
//! lookups, text aggregation, clipboard reads, PII scrubbing).
//!
//! The queue never blocks the hook. It has a soft cap: once that many items
//! are waiting, pointer-motion items are dropped first, and keys, clicks and
//! wheel ticks still get in until the hard capacity (a quarter above the soft
//! cap) is reached. Every drop is counted so the worker can report that it
//! fell behind, without saying anything about the dropped input itself.

use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender, TrySendError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How much an item matters when the worker falls behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPriority {
    /// Pointer motion: plentiful and cheap to lose; dropped at the soft cap.
    Motion,
    /// Keys, clicks, wheel ticks: dropped only at the hard capacity.
    Normal,
}

/// Producer side, used by the hook callback. `push` never blocks.
pub struct InputQueueSender<T> {
    tx: Sender<T>,
    soft_cap: usize,
    dropped: Arc<AtomicU64>,
}

/// Consumer side, used by the worker thread.
pub struct InputQueueReceiver<T> {
    rx: Receiver<T>,
    dropped: Arc<AtomicU64>,
}

/// A queue with the given soft cap (at least 1).
pub fn input_queue<T>(soft_cap: usize) -> (InputQueueSender<T>, InputQueueReceiver<T>) {
    let soft_cap = soft_cap.max(1);
    let capacity = soft_cap + (soft_cap / 4).max(1);
    let (tx, rx) = bounded(capacity);
    let dropped = Arc::new(AtomicU64::new(0));
    (
        InputQueueSender {
            tx,
            soft_cap,
            dropped: dropped.clone(),
        },
        InputQueueReceiver { rx, dropped },
    )
}

impl<T> InputQueueSender<T> {
    /// Queue `item`; returns false (and counts a drop) when it was refused.
    /// A disconnected worker refuses everything without counting drops.
    pub fn push(&self, item: T, priority: InputPriority) -> bool {
        if priority == InputPriority::Motion && self.tx.len() >= self.soft_cap {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        match self.tx.try_send(item) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    /// Items waiting for the worker.
    pub fn len(&self) -> usize {
        self.tx.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tx.is_empty()
    }
}

impl<T> InputQueueReceiver<T> {
    /// Wait up to `timeout` for the next item.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }

    /// Next item if one is waiting.
    pub fn try_recv(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    /// Drops counted since the previous call.
    pub fn take_dropped(&self) -> u64 {
        self.dropped.swap(0, Ordering::Relaxed)
    }
}

/// Rate limit for "the worker fell behind" reports: collects drop counts and
/// releases a total at most once per interval.
#[derive(Debug)]
pub struct DropReporter {
    interval: Duration,
    pending: u64,
    last_report: Option<Instant>,
}

impl DropReporter {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            pending: 0,
            last_report: None,
        }
    }

    /// Add `newly_dropped`; returns the total to report when a report is due.
    pub fn add(&mut self, newly_dropped: u64, now: Instant) -> Option<u64> {
        self.pending = self.pending.saturating_add(newly_dropped);
        if self.pending == 0 {
            return None;
        }
        let due = self
            .last_report
            .is_none_or(|at| now.saturating_duration_since(at) >= self.interval);
        if !due {
            return None;
        }
        self.last_report = Some(now);
        Some(std::mem::take(&mut self.pending))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_is_dropped_at_the_soft_cap_but_keys_still_fit() {
        let (tx, rx) = input_queue::<u32>(8);
        for i in 0..8 {
            assert!(tx.push(i, InputPriority::Motion));
        }
        // Soft cap reached: motion is refused, keys and clicks are not.
        assert!(!tx.push(100, InputPriority::Motion));
        assert!(tx.push(200, InputPriority::Normal));
        assert!(tx.push(201, InputPriority::Normal));
        // Hard capacity is 8 + 2: the next key is refused too.
        assert!(!tx.push(202, InputPriority::Normal));
        assert_eq!(rx.take_dropped(), 2);
        assert_eq!(rx.take_dropped(), 0);

        // Everything that got in comes out in order.
        let drained: Vec<u32> = std::iter::from_fn(|| rx.try_recv()).collect();
        assert_eq!(drained, vec![0, 1, 2, 3, 4, 5, 6, 7, 200, 201]);
        assert!(tx.push(1, InputPriority::Motion));
    }

    #[test]
    fn push_never_blocks_and_ignores_a_gone_worker() {
        let (tx, rx) = input_queue::<u32>(1);
        drop(rx);
        let start = Instant::now();
        for i in 0..10_000 {
            assert!(!tx.push(i, InputPriority::Normal));
        }
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn worker_receives_with_timeout() {
        let (tx, rx) = input_queue::<&'static str>(4);
        assert!(matches!(
            rx.recv_timeout(Duration::from_millis(5)),
            Err(RecvTimeoutError::Timeout)
        ));
        tx.push("click", InputPriority::Normal);
        assert_eq!(rx.recv_timeout(Duration::from_millis(5)), Ok("click"));
        drop(tx);
        assert!(matches!(
            rx.recv_timeout(Duration::from_millis(5)),
            Err(RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn drop_reports_are_rate_limited() {
        let start = Instant::now();
        let mut reporter = DropReporter::new(Duration::from_secs(10));
        assert_eq!(reporter.add(0, start), None);
        assert_eq!(reporter.add(3, start), Some(3));
        assert_eq!(reporter.add(4, start + Duration::from_secs(1)), None);
        assert_eq!(reporter.add(1, start + Duration::from_secs(5)), None);
        assert_eq!(reporter.add(0, start + Duration::from_secs(10)), Some(5));
        assert_eq!(reporter.add(0, start + Duration::from_secs(30)), None);
    }
}
