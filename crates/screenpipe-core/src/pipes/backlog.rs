// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Deferred event runs: the bounded backlog of triggers that could not start
//! on the pass they arrived on.
//!
//! An event for a pipe that is busy (a run queued or running, or another
//! event run chosen this pass) is not dropped. It waits here and is tried
//! again on each scheduler pass until the pipe is free. The backlog keeps it
//! bounded and free of repeats:
//!
//! - **Coalescing.** Runs are identified by pipe, event name and instance
//!   key. A second run with the same identity is folded into the one already
//!   waiting: the newer payload replaces the older one (the newest
//!   generation wins), the entry keeps its place in the queue and its first
//!   wait time. Five `workflow_ready:<pipe>` wake-ups during a long run
//!   therefore produce one run afterwards, and a meeting id still runs once
//!   (different meetings have different keys and are never folded together).
//! - **Cap.** Each pipe holds at most [`DEFERRED_BACKLOG_CAP`] entries. When
//!   a new entry would go over, the oldest one is evicted and handed back to
//!   the scheduler, which logs it with a reason and, for a run with an
//!   instance key, records it as a skipped execution.
//! - **Deadline.** A run that cannot start for a reason other than a busy
//!   pipe (the agent is still installing, setup mode, another process holds
//!   the pipe, its trigger context could not be written) is given up after
//!   [`DEFERRED_REQUEST_TTL`] from when it was first deferred. Waiting behind
//!   a busy pipe does not count against it: the run ahead finishes within its
//!   own timeout.
//!
//! The backlog lives in memory; restart recovery of interrupted meeting
//! summaries (the finalizer) covers the case that matters across restarts.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::event_runs::EventRun;

pub use super::event_runs::DEFERRED_REQUEST_TTL;

/// Most deferred runs kept per pipe.
pub const DEFERRED_BACKLOG_CAP: usize = 32;

/// `error_type` of a skipped execution evicted from a full backlog.
pub const SKIP_BACKLOG_FULL: &str = "deferred_backlog_full";
/// `error_type` of a skipped execution that could not start in time.
pub const SKIP_DEFERRED_EXPIRED: &str = "deferred_expired";

/// What makes two deferred runs the same run.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeferredId {
    pub pipe: String,
    pub event: String,
    pub key: Option<String>,
}

impl DeferredId {
    pub fn of(run: &EventRun) -> Self {
        Self {
            pipe: run.pipe.clone(),
            event: run.event.clone(),
            key: run.key.clone(),
        }
    }
}

/// Bookkeeping a deferred run carries from pass to pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    /// When the run was first deferred.
    pub since: Instant,
    /// Queue position: lower is older.
    pub seq: u64,
    /// Generation of the payload the entry holds; a coalesced newer run
    /// raises it.
    pub generation: u64,
    /// How many runs were folded into this one.
    pub coalesced: u32,
}

/// A run waiting in the backlog.
#[derive(Debug, Clone)]
pub struct Deferred {
    pub run: EventRun,
    pub origin: Origin,
}

/// Running totals, for logs and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BacklogStats {
    /// Runs that entered the backlog.
    pub deferred: u64,
    /// Runs folded into a run with the same identity (in the backlog or
    /// already starting this pass).
    pub coalesced: u64,
    /// Runs evicted because their pipe's backlog was full.
    pub dropped_full: u64,
    /// Runs given up after the deadline.
    pub expired: u64,
    /// Runs dropped because the user stopped that event instance.
    pub dropped_stopped: u64,
    /// Runs dropped because their pipe went away, was turned off or no
    /// longer listens to the event.
    pub dropped_inactive: u64,
}

impl BacklogStats {
    /// One line for the scheduler log.
    pub fn summary(&self) -> String {
        format!(
            "deferred={} coalesced={} dropped_full={} expired={} dropped_stopped={} dropped_inactive={}",
            self.deferred,
            self.coalesced,
            self.dropped_full,
            self.expired,
            self.dropped_stopped,
            self.dropped_inactive
        )
    }
}

/// What [`TriggerBacklog::defer`] did with a run.
#[derive(Debug)]
pub enum DeferOutcome {
    /// The run waits in the backlog.
    Queued,
    /// The run was folded into a waiting run with the same identity.
    Coalesced,
    /// The run waits, and the pipe's oldest entry was evicted to make room
    /// (it may be the run itself, when it was the oldest).
    Evicted(Box<Deferred>),
}

/// Why a deferred run left the backlog without running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    BacklogFull,
    Expired,
}

impl DropReason {
    /// `error_type` recorded on the skipped execution.
    pub fn error_type(self) -> &'static str {
        match self {
            DropReason::BacklogFull => SKIP_BACKLOG_FULL,
            DropReason::Expired => SKIP_DEFERRED_EXPIRED,
        }
    }
}

/// Per-pipe queues of deferred runs.
#[derive(Debug)]
pub struct TriggerBacklog {
    queues: HashMap<String, VecDeque<Deferred>>,
    cap: usize,
    ttl: Duration,
    next_seq: u64,
    stats: BacklogStats,
}

impl Default for TriggerBacklog {
    fn default() -> Self {
        Self::new(DEFERRED_BACKLOG_CAP, DEFERRED_REQUEST_TTL)
    }
}

impl TriggerBacklog {
    /// A backlog holding at most `cap` runs per pipe (at least one) and
    /// giving up on runs that cannot start after `ttl`.
    pub fn new(cap: usize, ttl: Duration) -> Self {
        Self {
            queues: HashMap::new(),
            cap: cap.max(1),
            ttl,
            next_seq: 0,
            stats: BacklogStats::default(),
        }
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Runs waiting, all pipes together.
    pub fn len(&self) -> usize {
        self.queues.values().map(VecDeque::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Runs waiting for `pipe`.
    pub fn pending(&self, pipe: &str) -> usize {
        self.queues.get(pipe).map_or(0, VecDeque::len)
    }

    pub fn stats(&self) -> BacklogStats {
        self.stats
    }

    /// Whether a run deferred at `origin` has used up its deadline at `now`.
    pub fn is_expired(&self, origin: &Origin, now: Instant) -> bool {
        now.saturating_duration_since(origin.since) >= self.ttl
    }

    /// Note a run folded into another one outside the backlog (a duplicate
    /// of a run already starting this pass).
    pub fn note_coalesced(&mut self) {
        self.stats.coalesced += 1;
    }

    /// Note a run given up after its deadline.
    pub fn note_expired(&mut self) {
        self.stats.expired += 1;
    }

    /// Note a run dropped because its event instance was stopped.
    pub fn note_stopped(&mut self) {
        self.stats.dropped_stopped += 1;
    }

    /// Note a run dropped because its pipe no longer takes it.
    pub fn note_inactive(&mut self) {
        self.stats.dropped_inactive += 1;
    }

    /// Every waiting run, oldest first. The backlog is empty afterwards;
    /// runs that still cannot start go back through [`defer`](Self::defer)
    /// with their [`Origin`].
    pub fn take_all(&mut self) -> Vec<Deferred> {
        let mut all: Vec<Deferred> = self
            .queues
            .drain()
            .flat_map(|(_, queue)| queue.into_iter())
            .collect();
        all.sort_by_key(|d| d.origin.seq);
        all
    }

    /// Keep `run` for a later pass. `origin` is the bookkeeping of a run
    /// taken from the backlog this pass (it keeps its place, wait time and
    /// generation); a new run passes `None`.
    pub fn defer(&mut self, mut run: EventRun, origin: Option<Origin>, now: Instant) -> DeferOutcome {
        // A kept run is aimed at its pipe from now on: it does not go
        // through trigger matching or the event gate again.
        run.directed = true;
        let origin = match origin {
            Some(origin) => origin,
            None => {
                self.stats.deferred += 1;
                let seq = self.next_seq;
                self.next_seq += 1;
                Origin {
                    since: now,
                    seq,
                    generation: seq,
                    coalesced: 0,
                }
            }
        };
        let id = DeferredId::of(&run);
        let queue = self.queues.entry(run.pipe.clone()).or_default();

        if let Some(waiting) = queue.iter_mut().find(|d| DeferredId::of(&d.run) == id) {
            if origin.generation >= waiting.origin.generation {
                waiting.run = run;
                waiting.origin.generation = origin.generation;
            }
            waiting.origin.since = waiting.origin.since.min(origin.since);
            waiting.origin.seq = waiting.origin.seq.min(origin.seq);
            waiting.origin.coalesced = waiting
                .origin
                .coalesced
                .saturating_add(origin.coalesced)
                .saturating_add(1);
            self.stats.coalesced += 1;
            // A lower seq may have moved this entry forward.
            queue.make_contiguous().sort_by_key(|d| d.origin.seq);
            return DeferOutcome::Coalesced;
        }

        let at = queue
            .iter()
            .position(|d| d.origin.seq > origin.seq)
            .unwrap_or(queue.len());
        queue.insert(at, Deferred { run, origin });
        if queue.len() > self.cap {
            if let Some(oldest) = queue.pop_front() {
                self.stats.dropped_full += 1;
                return DeferOutcome::Evicted(Box::new(oldest));
            }
        }
        DeferOutcome::Queued
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(pipe: &str, event: &str, key: Option<&str>, n: i64) -> EventRun {
        EventRun {
            pipe: pipe.into(),
            event: event.into(),
            key: key.map(str::to_string),
            data: json!({ "n": n }),
            directed: false,
        }
    }

    #[test]
    fn repeats_of_one_trigger_fold_into_the_newest_payload() {
        let mut b = TriggerBacklog::new(32, Duration::from_secs(600));
        let t0 = Instant::now();
        for n in 0..5 {
            let out = b.defer(run("p", "workflow_ready:p", None, n), None, t0);
            if n == 0 {
                assert!(matches!(out, DeferOutcome::Queued));
            } else {
                assert!(matches!(out, DeferOutcome::Coalesced));
            }
        }
        assert_eq!(b.pending("p"), 1);
        let taken = b.take_all();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].run.data["n"], 4, "newest generation wins");
        assert_eq!(taken[0].origin.coalesced, 4);
        assert!(taken[0].run.directed);
        assert_eq!(b.stats().deferred, 5);
        assert_eq!(b.stats().coalesced, 4);
        assert!(b.is_empty());
    }

    #[test]
    fn different_instances_are_never_folded() {
        let mut b = TriggerBacklog::default();
        let t0 = Instant::now();
        b.defer(run("p", "meeting_ended", Some("1"), 0), None, t0);
        b.defer(run("p", "meeting_ended", Some("2"), 0), None, t0);
        b.defer(run("p", "meeting_ended", Some("1"), 1), None, t0);
        b.defer(run("q", "meeting_ended", Some("1"), 0), None, t0);
        assert_eq!(b.pending("p"), 2);
        assert_eq!(b.pending("q"), 1);
        let keys: Vec<_> = b
            .take_all()
            .into_iter()
            .map(|d| (d.run.pipe, d.run.key.unwrap()))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("p".to_string(), "1".to_string()),
                ("p".to_string(), "2".to_string()),
                ("q".to_string(), "1".to_string())
            ]
        );
    }

    #[test]
    fn a_full_pipe_backlog_evicts_its_oldest_run() {
        let mut b = TriggerBacklog::new(3, Duration::from_secs(600));
        let t0 = Instant::now();
        for k in 0..3 {
            b.defer(run("p", "e", Some(&k.to_string()), k), None, t0);
        }
        // Another pipe has its own room.
        b.defer(run("q", "e", Some("x"), 0), None, t0);
        match b.defer(run("p", "e", Some("3"), 3), None, t0) {
            DeferOutcome::Evicted(oldest) => assert_eq!(oldest.run.key.as_deref(), Some("0")),
            other => panic!("expected an eviction, got {other:?}"),
        }
        assert_eq!(b.pending("p"), 3);
        assert_eq!(b.pending("q"), 1);
        assert_eq!(b.stats().dropped_full, 1);
        assert!(b.len() <= 3 + 1);
    }

    #[test]
    fn requeued_runs_keep_their_place_age_and_deadline() {
        let mut b = TriggerBacklog::new(32, Duration::from_secs(600));
        let t0 = Instant::now();
        b.defer(run("p", "e", Some("old"), 0), None, t0);
        let later = t0 + Duration::from_secs(30);
        let taken = b.take_all();
        // A new run arrives in the same pass, then the old one is put back.
        b.defer(run("p", "e", Some("new"), 1), None, later);
        b.defer(taken[0].run.clone(), Some(taken[0].origin), later);
        let again = b.take_all();
        assert_eq!(again[0].run.key.as_deref(), Some("old"));
        assert_eq!(again[0].origin.since, t0);
        assert_eq!(again[1].run.key.as_deref(), Some("new"));
        // The deadline counts from the first deferral.
        assert!(!b.is_expired(&again[0].origin, t0 + Duration::from_secs(599)));
        assert!(b.is_expired(&again[0].origin, t0 + Duration::from_secs(600)));
        // Put back twice, counted once as deferred.
        assert_eq!(b.stats().deferred, 2);
    }

    #[test]
    fn folding_into_an_older_entry_keeps_the_earliest_deadline() {
        let mut b = TriggerBacklog::new(32, Duration::from_secs(600));
        let t0 = Instant::now();
        b.defer(run("p", "e", Some("1"), 0), None, t0);
        let first = b.take_all().remove(0);
        // A fresh copy is deferred first, then the taken one comes back.
        b.defer(run("p", "e", Some("1"), 1), None, t0 + Duration::from_secs(5));
        b.defer(first.run, Some(first.origin), t0 + Duration::from_secs(5));
        let only = b.take_all();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].origin.since, t0);
        // The fresh copy is the newer generation, so its payload stays.
        assert_eq!(only[0].run.data["n"], 1);
    }
}
