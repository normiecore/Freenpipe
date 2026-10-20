// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Health counters for the write coalescing queue.
//!
//! The drain loop reports how each batch ended; `/health` reads a snapshot
//! to tell a user whether captured data is actually reaching the database.
//! Everything here is in-process state: nothing is persisted or sent anywhere.
//!
//! - A *fatal* batch failure is one where the writer connection itself broke
//!   (I/O error, corruption signal, closed pool, failed COMMIT), or where the
//!   batch committed without storing a single write because the database
//!   could not take more data (disk full, read-only file).
//! - A *contention* failure is one where the batch could not get the write
//!   lock in time (semaphore or pool acquire timeout, `SQLITE_BUSY`).
//! - Both run counters count failed batches since the last committed batch.
//! - A *reopen* is a writer connection that was discarded so the pool opens a
//!   fresh one, or a database file that had to be recreated before reopening.
//! - A *persistent failure signal* is raised once each time a run of failures
//!   reaches its threshold, so a reader can tell a blip from a wedged writer.

use std::sync::Mutex;
use tracing::warn;

/// Fatal batches in a row that count as a persistent failure.
pub const PERSISTENT_FATAL_BATCHES: u64 = 3;
/// Contended batches in a row that count as a persistent failure.
pub const PERSISTENT_CONTENTION_BATCHES: u64 = 5;
/// Contended batches in a row before the queue reports itself degraded. One
/// slow batch under a burst of writes is normal; two in a row are not.
pub const DEGRADED_CONTENTION_BATCHES: u64 = 2;

/// How a failed batch failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchFailure {
    Fatal,
    Contention,
}

/// How a batch that got its transaction ended. Each batch reports exactly one
/// of these, so a rolled-back batch can never also count as committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchEnd {
    /// A write broke the connection partway through; the batch was rolled back.
    RolledBack,
    /// Every write ran but COMMIT failed.
    CommitFailed,
    /// COMMIT succeeded. `writes_ok` writes were stored; `storage_errors` of
    /// the failed writes failed because the database could not store data.
    Committed {
        writes_ok: usize,
        storage_errors: usize,
    },
}

impl BatchEnd {
    /// The failure this ending counts as, or `None` when the writer made
    /// progress. A committed batch in which writes failed only for their own
    /// reasons (constraint violations, a missing row) is progress: the writer
    /// works and a retry of other data would be stored.
    pub fn failure(self) -> Option<BatchFailure> {
        match self {
            BatchEnd::RolledBack | BatchEnd::CommitFailed => Some(BatchFailure::Fatal),
            BatchEnd::Committed {
                writes_ok: 0,
                storage_errors,
            } if storage_errors > 0 => Some(BatchFailure::Fatal),
            BatchEnd::Committed { .. } => None,
        }
    }
}

/// Point-in-time view of the write queue's health.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteQueueHealth {
    /// True while recent batches are failing (see [`WriteQueueHealth::is_degraded`]).
    pub degraded: bool,
    /// Fatal batch failures since the last committed batch.
    pub consecutive_fatal: u64,
    /// Contention batch failures since the last committed batch.
    pub consecutive_contention: u64,
    /// Writer connections discarded and reopened since the engine started.
    pub pool_reopens: u64,
    /// Times a failure run reached its persistent-failure threshold.
    pub persistent_failure_signals: u64,
}

/// The counters behind [`WriteQueueHealth`], as a plain state machine so the
/// rules are testable without a database.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteQueueHealthState {
    consecutive_fatal: u64,
    consecutive_contention: u64,
    pool_reopens: u64,
    persistent_failure_signals: u64,
}

impl WriteQueueHealthState {
    pub const fn new() -> Self {
        Self {
            consecutive_fatal: 0,
            consecutive_contention: 0,
            pool_reopens: 0,
            persistent_failure_signals: 0,
        }
    }

    /// A batch committed: the writer works, so both failure runs end.
    pub fn on_commit(&mut self) {
        self.consecutive_fatal = 0;
        self.consecutive_contention = 0;
    }

    /// A batch failed. Returns true when this failure made the run reach its
    /// persistent-failure threshold (reported once per run).
    pub fn on_failure(&mut self, kind: BatchFailure) -> bool {
        let (run, threshold) = match kind {
            BatchFailure::Fatal => (&mut self.consecutive_fatal, PERSISTENT_FATAL_BATCHES),
            BatchFailure::Contention => (
                &mut self.consecutive_contention,
                PERSISTENT_CONTENTION_BATCHES,
            ),
        };
        *run = run.saturating_add(1);
        let crossed = *run == threshold;
        if crossed {
            self.persistent_failure_signals = self.persistent_failure_signals.saturating_add(1);
        }
        crossed
    }

    /// A batch that got its transaction ended. Returns true when this ending
    /// made a failure run reach its persistent-failure threshold.
    pub fn on_batch_end(&mut self, end: BatchEnd) -> bool {
        match end.failure() {
            Some(kind) => self.on_failure(kind),
            None => {
                self.on_commit();
                false
            }
        }
    }

    /// A writer connection was discarded (or the file recreated) so the pool
    /// has to open a fresh one.
    pub fn on_reopen(&mut self) {
        self.pool_reopens = self.pool_reopens.saturating_add(1);
    }

    pub fn snapshot(&self) -> WriteQueueHealth {
        WriteQueueHealth {
            degraded: WriteQueueHealth::is_degraded(
                self.consecutive_fatal,
                self.consecutive_contention,
            ),
            consecutive_fatal: self.consecutive_fatal,
            consecutive_contention: self.consecutive_contention,
            pool_reopens: self.pool_reopens,
            persistent_failure_signals: self.persistent_failure_signals,
        }
    }
}

impl WriteQueueHealth {
    /// Degraded when the latest batch broke the writer, or when the write lock
    /// could not be had for several batches in a row.
    pub fn is_degraded(consecutive_fatal: u64, consecutive_contention: u64) -> bool {
        consecutive_fatal > 0 || consecutive_contention >= DEGRADED_CONTENTION_BATCHES
    }
}

static STATE: Mutex<WriteQueueHealthState> = Mutex::new(WriteQueueHealthState::new());

/// Current write queue health. Cheap; safe to call on every `/health` poll.
pub fn write_queue_health() -> WriteQueueHealth {
    STATE
        .lock()
        .map(|state| state.snapshot())
        .unwrap_or_default()
}

/// Report how a batch that got its transaction ended. Call once per batch.
pub(crate) fn record_batch_end(end: BatchEnd) {
    match end.failure() {
        Some(kind) => record_batch_failed(kind),
        None => {
            if let Ok(mut state) = STATE.lock() {
                state.on_commit();
            }
        }
    }
}

pub(crate) fn record_batch_failed(kind: BatchFailure) {
    let crossed = match STATE.lock() {
        Ok(mut state) => state.on_failure(kind).then(|| state.snapshot()),
        Err(_) => None,
    };
    if let Some(health) = crossed {
        warn!(
            "write_queue: writes keep failing ({:?}; fatal run={}, contention run={}, connection reopens={}) — captured data is not reaching the database",
            kind, health.consecutive_fatal, health.consecutive_contention, health.pool_reopens
        );
    }
}

pub(crate) fn record_writer_reopen() {
    if let Ok(mut state) = STATE.lock() {
        state.on_reopen();
    }
}

/// The [`BatchEnd`] of a batch whose COMMIT succeeded, from its per-write
/// results.
pub(crate) fn committed_batch_end<T>(results: &[Result<T, sqlx::Error>]) -> BatchEnd {
    BatchEnd::Committed {
        writes_ok: results.iter().filter(|r| r.is_ok()).count(),
        storage_errors: results
            .iter()
            .filter(|r| matches!(r, Err(e) if is_storage_error(e)))
            .count(),
    }
}

/// A write failed because SQLite could not store data: the disk (or the
/// database's page limit) is full, or the database file became read-only.
/// SQLite reports these per statement and keeps the transaction open, so
/// they arrive as per-row errors rather than connection errors.
pub(crate) fn is_storage_error(error: &sqlx::Error) -> bool {
    let message = match error {
        sqlx::Error::Database(db) => db.message().to_lowercase(),
        sqlx::Error::Protocol(text) => text.to_lowercase(),
        _ => return false,
    };
    message.contains("database or disk is full") || message.contains("readonly database")
}

/// Classify the error a whole batch failed with.
pub(crate) fn classify_batch_error(error: &sqlx::Error) -> BatchFailure {
    match error {
        sqlx::Error::PoolTimedOut => BatchFailure::Contention,
        e if crate::sqlite_error::is_sqlite_busy_error(e) => BatchFailure::Contention,
        _ => BatchFailure::Fatal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_clears_both_runs_but_keeps_lifetime_counters() {
        let mut s = WriteQueueHealthState::new();
        s.on_failure(BatchFailure::Fatal);
        s.on_failure(BatchFailure::Contention);
        s.on_reopen();
        assert!(s.snapshot().degraded);
        s.on_commit();
        let h = s.snapshot();
        assert_eq!(h.consecutive_fatal, 0);
        assert_eq!(h.consecutive_contention, 0);
        assert_eq!(h.pool_reopens, 1);
        assert!(!h.degraded);
    }

    #[test]
    fn one_fatal_batch_degrades_but_one_contended_batch_does_not() {
        let mut s = WriteQueueHealthState::new();
        s.on_failure(BatchFailure::Contention);
        assert!(!s.snapshot().degraded);
        s.on_failure(BatchFailure::Contention);
        assert!(s.snapshot().degraded);

        let mut s = WriteQueueHealthState::new();
        s.on_failure(BatchFailure::Fatal);
        assert!(s.snapshot().degraded);
    }

    #[test]
    fn persistent_signal_fires_once_per_run() {
        let mut s = WriteQueueHealthState::new();
        let fired: Vec<bool> = (0..6).map(|_| s.on_failure(BatchFailure::Fatal)).collect();
        assert_eq!(fired, vec![false, false, true, false, false, false]);
        assert_eq!(s.snapshot().persistent_failure_signals, 1);
        assert_eq!(s.snapshot().consecutive_fatal, 6);

        s.on_commit();
        for _ in 0..PERSISTENT_FATAL_BATCHES {
            s.on_failure(BatchFailure::Fatal);
        }
        assert_eq!(s.snapshot().persistent_failure_signals, 2);

        for _ in 0..PERSISTENT_CONTENTION_BATCHES {
            s.on_failure(BatchFailure::Contention);
        }
        assert_eq!(s.snapshot().persistent_failure_signals, 3);
    }

    /// A writer that rolls back every batch (a corrupt file, an I/O error in
    /// the middle of a batch) stays degraded and reaches the persistent
    /// threshold; nothing in the rollback path counts as a commit.
    #[test]
    fn rolled_back_batches_keep_the_queue_degraded() {
        let mut s = WriteQueueHealthState::new();
        assert!(!s.on_batch_end(BatchEnd::RolledBack));
        let h = s.snapshot();
        assert_eq!(h.consecutive_fatal, 1);
        assert!(h.degraded);

        assert!(!s.on_batch_end(BatchEnd::CommitFailed));
        assert!(s.on_batch_end(BatchEnd::RolledBack));
        let h = s.snapshot();
        assert_eq!(h.consecutive_fatal, PERSISTENT_FATAL_BATCHES);
        assert_eq!(h.persistent_failure_signals, 1);
        assert!(h.degraded);

        // One batch that stores something ends the run.
        s.on_batch_end(BatchEnd::Committed {
            writes_ok: 1,
            storage_errors: 0,
        });
        assert!(!s.snapshot().degraded);
    }

    #[test]
    fn a_committed_batch_counts_by_what_it_stored() {
        let full = sqlx::Error::Protocol("database or disk is full".into());
        let constraint = sqlx::Error::Protocol("UNIQUE constraint failed: t.id".into());

        // Nothing stored because the disk is full: a failure.
        let end = committed_batch_end::<()>(&[Err(full)]);
        assert_eq!(
            end,
            BatchEnd::Committed {
                writes_ok: 0,
                storage_errors: 1
            }
        );
        assert_eq!(end.failure(), Some(BatchFailure::Fatal));

        // Writes rejected for their own reasons: the writer still works.
        let end = committed_batch_end::<()>(&[Err(constraint)]);
        assert_eq!(end.failure(), None);

        // Something was stored: progress, even if another write hit the limit.
        let full = sqlx::Error::Protocol("database or disk is full".into());
        let end = committed_batch_end(&[Ok(()), Err(full)]);
        assert_eq!(end.failure(), None);

        let mut s = WriteQueueHealthState::new();
        s.on_batch_end(BatchEnd::Committed {
            writes_ok: 0,
            storage_errors: 2,
        });
        assert!(s.snapshot().degraded);
    }

    /// The storage-error test matches the messages SQLite really returns.
    #[tokio::test]
    async fn storage_errors_match_what_sqlite_reports() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE t (b BLOB)")
            .execute(&pool)
            .await
            .unwrap();

        // Cap the database at its current size (SQLite never sets the cap
        // below it) so the next insert finds it full.
        sqlx::query("PRAGMA max_page_count = 1")
            .execute(&pool)
            .await
            .unwrap();
        let full = sqlx::query("INSERT INTO t VALUES (zeroblob(1000000))")
            .execute(&pool)
            .await
            .unwrap_err();
        assert!(is_storage_error(&full), "{}", full);

        let missing_table = sqlx::query("INSERT INTO missing VALUES (1)")
            .execute(&pool)
            .await
            .unwrap_err();
        assert!(!is_storage_error(&missing_table), "{}", missing_table);

        sqlx::query("PRAGMA query_only = 1")
            .execute(&pool)
            .await
            .unwrap();
        let read_only = sqlx::query("INSERT INTO t VALUES (x'00')")
            .execute(&pool)
            .await
            .unwrap_err();
        assert!(is_storage_error(&read_only), "{}", read_only);
    }

    #[test]
    fn timeouts_and_busy_are_contention_everything_else_is_fatal() {
        assert_eq!(
            classify_batch_error(&sqlx::Error::PoolTimedOut),
            BatchFailure::Contention
        );
        assert_eq!(
            classify_batch_error(&sqlx::Error::PoolClosed),
            BatchFailure::Fatal
        );
        assert_eq!(
            classify_batch_error(&sqlx::Error::WorkerCrashed),
            BatchFailure::Fatal
        );
    }
}
