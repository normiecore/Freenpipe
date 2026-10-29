// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Connection pragmas and WAL maintenance.
//!
//! # Pragmas
//!
//! Every pooled connection runs in WAL mode with `synchronous=NORMAL`, the
//! cache and mmap sizes of the [`DbConfig`] tier, in-memory temp storage and
//! `wal_autocheckpoint=0`: a committing writer never stops to checkpoint.
//! Read-pool connections also set `query_only=ON`, so a read path can never
//! take the write lock. `journal_size_limit` caps the WAL file a writer leaves
//! behind once the log restarts.
//!
//! [`offline_connect_options`] builds the opposite profile for work done while
//! no engine runs (storage conversion, recovery verification): rollback
//! journal, `synchronous=FULL` and an exclusive lock.
//!
//! # WAL maintenance
//!
//! With inline checkpoints off, the WAL only shrinks through this module:
//!
//! - at start-up, one `RESTART` checkpoint folds a WAL left by the last run;
//! - a background loop runs a `PASSIVE` checkpoint every
//!   [`WalMaintenanceConfig::interval`]. It takes the write coordinator first
//!   so it never competes with a write batch, and PASSIVE never waits for a
//!   reader or writer, so recording is never blocked;
//! - when reader snapshots keep frames from being checkpointed, the loop logs
//!   a deferral and later a separate event once the backlog has drained;
//! - when the log grows past [`WalMaintenanceConfig::escalate_wal_bytes`] or a
//!   backlog lasts longer than [`WalMaintenanceConfig::escalate_after_backlog`],
//!   one `RESTART` checkpoint with a short busy timeout lets the next writer
//!   start the log from the beginning (the file is then cut back to
//!   `journal_size_limit`). A loop that cannot get the write coordinator for a
//!   while checkpoints without it, so a wedged writer cannot let the WAL grow
//!   unchecked either;
//! - the loop stops for good on a corruption result (`SQLITE_CORRUPT`,
//!   `SQLITE_NOTADB`), backs off on I/O and disk-full errors, and exits when
//!   its `DatabaseManager` is dropped.
//!
//! No checkpoint here ever uses `TRUNCATE`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use screenpipe_config::DbConfig;
use serde::Serialize;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqliteLockingMode, SqlitePool,
    SqliteSynchronous,
};
use sqlx::Row;
use tokio::sync::{watch, Semaphore};
use tracing::{debug, error, info, warn};

/// `busy_timeout` every pooled connection uses.
pub const CONNECTION_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// WAL size a connection leaves on disk after the log restarts (64 MiB).
pub const JOURNAL_SIZE_LIMIT_BYTES: u64 = 64 * 1024 * 1024;

/// Overrides [`WalMaintenanceConfig::interval`] (seconds); for tests and
/// diagnostics.
pub const WAL_MAINTENANCE_INTERVAL_ENV: &str = "SCREENPIPE_WAL_MAINTENANCE_INTERVAL_SECS";

/// Which pool a connection belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionRole {
    Writer,
    Reader,
}

/// Apply the normal-mode pragma pack for `role` to `options`.
pub fn normal_mode_options(
    options: SqliteConnectOptions,
    config: &DbConfig,
    role: ConnectionRole,
) -> SqliteConnectOptions {
    let options = options
        .busy_timeout(CONNECTION_BUSY_TIMEOUT)
        .pragma("journal_mode", "WAL")
        // NORMAL is safe in WAL mode: a commit waits for the WAL write, not
        // for an fsync of the main file.
        .pragma("synchronous", "NORMAL")
        .pragma("cache_size", format!("-{}", config.cache_size_kb))
        .pragma("mmap_size", config.mmap_size.to_string())
        .pragma("temp_store", "MEMORY")
        // Writers never checkpoint inline; the maintenance loop does.
        .pragma("wal_autocheckpoint", "0")
        .pragma("journal_size_limit", JOURNAL_SIZE_LIMIT_BYTES.to_string());
    match role {
        ConnectionRole::Writer => options,
        ConnectionRole::Reader => options.pragma("query_only", "ON"),
    }
}

/// Connect options for offline work on a database no engine has open:
/// rollback journal, `synchronous=FULL`, exclusive locking. With
/// `large_index_build`, temp B-trees go to disk (`temp_store=FILE`) so a big
/// index build does not hold them in RAM.
///
/// Check [`crate::ownership::ensure_not_owned_elsewhere`] before using these.
pub fn offline_connect_options(path: &Path, large_index_build: bool) -> SqliteConnectOptions {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .busy_timeout(CONNECTION_BUSY_TIMEOUT)
        .locking_mode(SqliteLockingMode::Exclusive)
        .journal_mode(SqliteJournalMode::Delete)
        .synchronous(SqliteSynchronous::Full);
    if large_index_build {
        options.pragma("temp_store", "FILE")
    } else {
        options
    }
}

// ---------------------------------------------------------------------------
// Checkpoints
// ---------------------------------------------------------------------------

/// Checkpoint modes Handvel uses. There is deliberately no TRUNCATE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckpointMode {
    Passive,
    Full,
    Restart,
}

impl CheckpointMode {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckpointMode::Passive => "PASSIVE",
            CheckpointMode::Full => "FULL",
            CheckpointMode::Restart => "RESTART",
        }
    }

    fn statement(self) -> &'static str {
        match self {
            CheckpointMode::Passive => "PRAGMA wal_checkpoint(PASSIVE)",
            CheckpointMode::Full => "PRAGMA wal_checkpoint(FULL)",
            CheckpointMode::Restart => "PRAGMA wal_checkpoint(RESTART)",
        }
    }
}

/// The row `PRAGMA wal_checkpoint` returns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CheckpointResult {
    /// The checkpoint could not finish (a reader or writer was in the way).
    pub busy: bool,
    /// Frames in the WAL; -1 when the database is not in WAL mode.
    pub log_pages: i64,
    /// Frames copied into the database file; -1 when not in WAL mode.
    pub checkpointed: i64,
}

impl CheckpointResult {
    /// Frames still waiting to be copied into the database file.
    pub fn pending_pages(&self) -> i64 {
        (self.log_pages - self.checkpointed).max(0)
    }

    /// Whether the database is in WAL mode at all.
    pub fn is_wal(&self) -> bool {
        self.log_pages >= 0
    }
}

/// Run one checkpoint on `conn`.
pub async fn checkpoint(
    conn: &mut SqliteConnection,
    mode: CheckpointMode,
) -> Result<CheckpointResult, sqlx::Error> {
    let row = sqlx::query(mode.statement()).fetch_one(&mut *conn).await?;
    Ok(CheckpointResult {
        busy: row.try_get::<i64, _>(0)? != 0,
        log_pages: row.try_get(1)?,
        checkpointed: row.try_get(2)?,
    })
}

/// Run a checkpoint with a temporary `busy_timeout`, restoring the pool's
/// value afterwards. A connection whose timeout cannot be restored is closed
/// instead of returned.
pub(crate) async fn checkpoint_with_busy_timeout(
    pool: &SqlitePool,
    mode: CheckpointMode,
    busy_timeout: Duration,
) -> Result<CheckpointResult, sqlx::Error> {
    let mut conn = pool.acquire().await?;
    let set = format!("PRAGMA busy_timeout = {}", busy_timeout.as_millis());
    sqlx::query(sqlx::AssertSqlSafe(set))
        .execute(&mut *conn)
        .await?;
    let result = checkpoint(&mut conn, mode).await;
    let restore = format!(
        "PRAGMA busy_timeout = {}",
        CONNECTION_BUSY_TIMEOUT.as_millis()
    );
    if let Err(e) = sqlx::query(sqlx::AssertSqlSafe(restore))
        .execute(&mut *conn)
        .await
    {
        warn!(
            "wal checkpoint: restoring busy_timeout failed ({}); closing the connection",
            e
        );
        conn.close_on_drop();
    }
    result
}

/// SQLite primary result code of an error, when it carries one.
pub fn sqlite_primary_code(err: &sqlx::Error) -> Option<i32> {
    match err {
        sqlx::Error::Database(db) => db
            .code()
            .and_then(|c| c.parse::<i32>().ok())
            .map(|c| c & 0xff),
        _ => None,
    }
}

/// Corruption results after which maintenance stops for this database
/// generation.
fn is_corruption(err: &sqlx::Error) -> bool {
    matches!(sqlite_primary_code(err), Some(11) | Some(26))
}

// ---------------------------------------------------------------------------
// WAL maintenance loop
// ---------------------------------------------------------------------------

/// Tuning of the WAL maintenance loop.
#[derive(Debug, Clone)]
pub struct WalMaintenanceConfig {
    /// Time between PASSIVE checkpoints.
    pub interval: Duration,
    /// How long one tick waits for the write coordinator.
    pub coordinator_wait: Duration,
    /// Ticks in a row without the coordinator after which the loop
    /// checkpoints without it.
    pub uncoordinated_after_skips: u32,
    /// Frames (in bytes) that trigger a RESTART checkpoint.
    pub escalate_wal_bytes: u64,
    /// Backlog age that triggers a RESTART checkpoint.
    pub escalate_after_backlog: Duration,
    /// Minimum time between two RESTART escalations.
    pub escalation_cooldown: Duration,
    /// Busy timeout of an escalated RESTART.
    pub escalation_busy_timeout: Duration,
    /// Time between repeated deferral warnings.
    pub deferral_log_every: Duration,
    /// Longest back-off after I/O errors.
    pub max_backoff: Duration,
}

impl Default for WalMaintenanceConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            coordinator_wait: Duration::from_secs(5),
            uncoordinated_after_skips: 3,
            escalate_wal_bytes: 128 * 1024 * 1024,
            escalate_after_backlog: Duration::from_secs(10 * 60),
            escalation_cooldown: Duration::from_secs(120),
            escalation_busy_timeout: Duration::from_secs(2),
            deferral_log_every: Duration::from_secs(10 * 60),
            max_backoff: Duration::from_secs(5 * 60),
        }
    }
}

impl WalMaintenanceConfig {
    /// Defaults, with [`WAL_MAINTENANCE_INTERVAL_ENV`] applied.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Some(secs) = std::env::var(WAL_MAINTENANCE_INTERVAL_ENV)
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|s| *s > 0)
        {
            config.interval = Duration::from_secs(secs);
        }
        config
    }
}

/// Point-in-time view of the WAL maintenance loop.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WalMaintenanceStatus {
    /// The loop is running.
    pub running: bool,
    /// Checkpoints run since start-up.
    pub checkpoints: u64,
    /// Mode of the last checkpoint.
    pub last_mode: Option<CheckpointMode>,
    pub last_checkpoint_unix_ms: Option<u64>,
    /// Last result.
    pub busy: bool,
    pub log_pages: i64,
    pub checkpointed: i64,
    pub pending_pages: i64,
    /// How long frames have been waiting to be checkpointed.
    pub backlog_seconds: u64,
    /// Size of the `-wal` file on disk.
    pub wal_file_bytes: u64,
    /// Ticks in a row that could not get the write coordinator.
    pub coordinator_skips: u32,
    /// RESTART checkpoints the loop escalated to.
    pub escalations: u64,
    /// Failed checkpoints since start-up.
    pub failures: u64,
    pub last_error: Option<String>,
    /// Why the loop stopped, when it did.
    pub stopped_reason: Option<String>,
}

#[derive(Debug, Default)]
struct WalTracker {
    status: WalMaintenanceStatus,
    backlog_since: Option<Instant>,
    last_escalation: Option<Instant>,
    last_deferral_log: Option<Instant>,
    page_size: i64,
    stop_requested: Option<String>,
}

/// Shared state of one database's WAL maintenance.
#[derive(Debug, Default)]
pub struct WalMaintenanceState {
    inner: Mutex<WalTracker>,
}

impl WalMaintenanceState {
    pub fn snapshot(&self) -> WalMaintenanceStatus {
        let tracker = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let mut status = tracker.status.clone();
        if let Some(since) = tracker.backlog_since {
            status.backlog_seconds = since.elapsed().as_secs();
        }
        status
    }

    /// Stop the loop for this database generation (after a hard fault).
    pub fn stop(&self, reason: impl Into<String>) {
        let mut tracker = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        tracker.stop_requested = Some(reason.into());
    }

    fn stop_requested(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .stop_requested
            .clone()
    }

    fn with<R>(&self, f: impl FnOnce(&mut WalTracker) -> R) -> R {
        let mut tracker = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut tracker)
    }
}

/// What the tracker decided after a checkpoint, for logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BacklogEvent {
    None,
    /// Frames are pinned by reader snapshots (first time, or a repeat warning).
    Deferred {
        first: bool,
    },
    /// A backlog that existed is gone.
    Drained {
        after: Duration,
    },
}

/// Pick the mode of the next checkpoint.
pub fn next_mode(
    last: &WalMaintenanceStatus,
    page_size: i64,
    backlog_age: Option<Duration>,
    since_last_escalation: Option<Duration>,
    config: &WalMaintenanceConfig,
) -> CheckpointMode {
    let cooled_down = since_last_escalation.is_none_or(|d| d >= config.escalation_cooldown);
    if !cooled_down {
        return CheckpointMode::Passive;
    }
    let log_bytes = (last.log_pages.max(0) as u64).saturating_mul(page_size.max(0) as u64);
    let too_big = log_bytes >= config.escalate_wal_bytes;
    let too_old = backlog_age.is_some_and(|age| age >= config.escalate_after_backlog);
    if too_big || too_old {
        CheckpointMode::Restart
    } else {
        CheckpointMode::Passive
    }
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Path of the `-wal` file of `database`.
pub fn wal_file_path(database: &Path) -> PathBuf {
    let mut name = database.as_os_str().to_os_string();
    name.push("-wal");
    PathBuf::from(name)
}

impl WalTracker {
    /// Record a checkpoint result and say what changed about the backlog.
    fn record(
        &mut self,
        mode: CheckpointMode,
        result: CheckpointResult,
        wal_file_bytes: u64,
        now: Instant,
        config: &WalMaintenanceConfig,
    ) -> BacklogEvent {
        let status = &mut self.status;
        status.checkpoints += 1;
        status.last_mode = Some(mode);
        status.last_checkpoint_unix_ms = Some(unix_ms_now());
        status.busy = result.busy;
        status.log_pages = result.log_pages;
        status.checkpointed = result.checkpointed;
        status.pending_pages = result.pending_pages();
        status.wal_file_bytes = wal_file_bytes;
        status.coordinator_skips = 0;
        status.last_error = None;
        if mode != CheckpointMode::Passive {
            status.escalations += 1;
            self.last_escalation = Some(now);
        }

        if result.pending_pages() > 0 {
            let first = self.backlog_since.is_none();
            let since = *self.backlog_since.get_or_insert(now);
            status.backlog_seconds = now.saturating_duration_since(since).as_secs();
            let repeat_due = self
                .last_deferral_log
                .is_none_or(|t| now.saturating_duration_since(t) >= config.deferral_log_every);
            if first || repeat_due {
                self.last_deferral_log = Some(now);
                return BacklogEvent::Deferred { first };
            }
            BacklogEvent::None
        } else {
            status.backlog_seconds = 0;
            self.last_deferral_log = None;
            match self.backlog_since.take() {
                Some(since) => BacklogEvent::Drained {
                    after: now.saturating_duration_since(since),
                },
                None => BacklogEvent::None,
            }
        }
    }
}

/// Handles a `DatabaseManager` keeps for its maintenance tasks. Dropping it
/// stops them.
#[derive(Debug)]
pub struct MaintenanceHandle {
    _shutdown: watch::Sender<()>,
    pub(crate) wal: Arc<WalMaintenanceState>,
}

impl MaintenanceHandle {
    pub(crate) fn new() -> (Self, watch::Receiver<()>) {
        let (tx, rx) = watch::channel(());
        (
            Self {
                _shutdown: tx,
                wal: Arc::new(WalMaintenanceState::default()),
            },
            rx,
        )
    }
}

/// Resolves when the owning `DatabaseManager` is dropped.
pub(crate) async fn shutdown_signalled(rx: &mut watch::Receiver<()>) {
    while rx.changed().await.is_ok() {}
}

/// Run the startup RESTART checkpoint; logs and continues on failure.
pub(crate) async fn startup_checkpoint(write_pool: &SqlitePool) {
    let result = async {
        let mut conn = write_pool.acquire().await?;
        checkpoint(&mut conn, CheckpointMode::Restart).await
    }
    .await;
    match result {
        Ok(r) if !r.is_wal() => debug!("startup wal checkpoint: database is not in WAL mode"),
        Ok(r) => {
            if r.log_pages > 0 || r.busy {
                info!(
                    "startup wal checkpoint (RESTART): busy={}, pages={}, checkpointed={}",
                    r.busy as i32, r.log_pages, r.checkpointed
                );
            } else {
                debug!("startup wal checkpoint (RESTART): wal empty");
            }
        }
        Err(e) => warn!(
            "startup wal checkpoint (RESTART) failed (continuing): {}",
            e
        ),
    }
}

/// Spawn the WAL maintenance loop for one database.
pub(crate) fn spawn_wal_maintenance(
    write_pool: SqlitePool,
    coordinator: Arc<Semaphore>,
    database: PathBuf,
    state: Arc<WalMaintenanceState>,
    config: WalMaintenanceConfig,
    mut shutdown: watch::Receiver<()>,
) {
    tokio::spawn(async move {
        state.with(|t| t.status.running = true);
        let page_size = page_size_of(&write_pool).await.unwrap_or(4096);
        state.with(|t| t.page_size = page_size);
        let wal_path = wal_file_path(&database);
        let mut backoff = Duration::ZERO;
        debug!(
            db = %database.display(),
            interval_secs = config.interval.as_secs(),
            "wal maintenance started"
        );

        let stop_reason = loop {
            let wait = config.interval + backoff;
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = shutdown_signalled(&mut shutdown) => break "database closed".to_string(),
            }
            if let Some(reason) = state.stop_requested() {
                break reason;
            }
            if write_pool.is_closed() {
                break "write pool closed".to_string();
            }

            // Take the write coordinator so the checkpoint never interleaves
            // with a write batch. If it stays unavailable, go on without it:
            // PASSIVE does not block writers and the WAL must not grow forever.
            let permit = match tokio::time::timeout(
                config.coordinator_wait,
                Arc::clone(&coordinator).acquire_owned(),
            )
            .await
            {
                Ok(Ok(permit)) => Some(permit),
                Ok(Err(_)) => break "write coordinator closed".to_string(),
                Err(_) => {
                    let skips = state.with(|t| {
                        t.status.coordinator_skips += 1;
                        t.status.coordinator_skips
                    });
                    if skips < config.uncoordinated_after_skips {
                        debug!("wal maintenance: writer busy, checkpoint deferred to next tick");
                        continue;
                    }
                    warn!(
                        "wal maintenance: write coordinator unavailable for {} ticks; \
                         checkpointing without it",
                        skips
                    );
                    None
                }
            };

            let now = Instant::now();
            let mode = state.with(|t| {
                next_mode(
                    &t.status,
                    t.page_size,
                    t.backlog_since.map(|s| now.saturating_duration_since(s)),
                    t.last_escalation.map(|e| now.saturating_duration_since(e)),
                    &config,
                )
            });
            let result = match mode {
                CheckpointMode::Passive => {
                    async {
                        let mut conn = write_pool.acquire().await?;
                        checkpoint(&mut conn, mode).await
                    }
                    .await
                }
                _ => {
                    checkpoint_with_busy_timeout(&write_pool, mode, config.escalation_busy_timeout)
                        .await
                }
            };
            drop(permit);

            match result {
                Ok(r) if !r.is_wal() => break "database is not in WAL mode".to_string(),
                Ok(r) => {
                    backoff = Duration::ZERO;
                    let wal_bytes = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);
                    let (event, backlog) = state.with(|t| {
                        let e = t.record(mode, r, wal_bytes, Instant::now(), &config);
                        (e, t.status.backlog_seconds)
                    });
                    log_checkpoint(mode, r, wal_bytes, event, backlog);
                }
                Err(e) if is_corruption(&e) => {
                    error!(
                        db = %database.display(),
                        "wal maintenance stopped for this database: checkpoint reported \
                         corruption ({}). Quit and run `screenpipe db recover`.",
                        e
                    );
                    break format!("corruption: {e}");
                }
                Err(sqlx::Error::PoolClosed) => break "write pool closed".to_string(),
                Err(e) => {
                    let failures = state.with(|t| {
                        t.status.failures += 1;
                        t.status.last_error = Some(e.to_string());
                        t.status.failures
                    });
                    backoff = (config.interval * 2u32.saturating_pow(failures.min(8) as u32))
                        .min(config.max_backoff);
                    warn!(
                        "wal checkpoint ({}) failed: {} (retrying in {}s)",
                        mode.as_str(),
                        e,
                        (config.interval + backoff).as_secs()
                    );
                }
            }
        };

        state.with(|t| {
            t.status.running = false;
            t.status.stopped_reason = Some(stop_reason.clone());
        });
        debug!(db = %database.display(), reason = %stop_reason, "wal maintenance stopped");
    });
}

fn log_checkpoint(
    mode: CheckpointMode,
    r: CheckpointResult,
    wal_bytes: u64,
    event: BacklogEvent,
    backlog_seconds: u64,
) {
    if mode != CheckpointMode::Passive {
        info!(
            "wal checkpoint ({}, escalated): busy={}, log_pages={}, checkpointed={}, wal_bytes={}",
            mode.as_str(),
            r.busy as i32,
            r.log_pages,
            r.checkpointed,
            wal_bytes
        );
    }
    match event {
        BacklogEvent::Deferred { first: true } => info!(
            "wal checkpoint deferred: {} of {} frames are pinned by open reader snapshots",
            r.pending_pages(),
            r.log_pages
        ),
        BacklogEvent::Deferred { first: false } => warn!(
            "wal checkpoint still deferred after {}s: {} of {} frames pinned by readers (wal_bytes={})",
            backlog_seconds,
            r.pending_pages(),
            r.log_pages,
            wal_bytes
        ),
        BacklogEvent::Drained { after } => info!(
            "wal checkpoint backlog drained after {}s ({} frames checkpointed)",
            after.as_secs(),
            r.checkpointed
        ),
        BacklogEvent::None => debug!(
            "wal checkpoint ({}): busy={}, log_pages={}, checkpointed={}",
            mode.as_str(),
            r.busy as i32,
            r.log_pages,
            r.checkpointed
        ),
    }
}

async fn page_size_of(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("PRAGMA page_size")
        .fetch_one(pool)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::ConnectOptions;

    fn status(log_pages: i64) -> WalMaintenanceStatus {
        WalMaintenanceStatus {
            log_pages,
            ..Default::default()
        }
    }

    #[test]
    fn passive_by_default_and_restart_when_the_log_is_large_or_old() {
        let config = WalMaintenanceConfig::default();
        assert_eq!(
            next_mode(&status(100), 4096, None, None, &config),
            CheckpointMode::Passive
        );
        let big = (config.escalate_wal_bytes / 4096) as i64;
        assert_eq!(
            next_mode(&status(big), 4096, None, None, &config),
            CheckpointMode::Restart
        );
        assert_eq!(
            next_mode(
                &status(10),
                4096,
                Some(config.escalate_after_backlog),
                None,
                &config
            ),
            CheckpointMode::Restart
        );
        // Cooling down after an escalation.
        assert_eq!(
            next_mode(
                &status(big),
                4096,
                None,
                Some(Duration::from_secs(5)),
                &config
            ),
            CheckpointMode::Passive
        );
        assert_eq!(
            next_mode(
                &status(big),
                4096,
                None,
                Some(config.escalation_cooldown),
                &config
            ),
            CheckpointMode::Restart
        );
    }

    #[test]
    fn tracker_reports_deferral_once_then_drain() {
        let config = WalMaintenanceConfig::default();
        let mut t = WalTracker::default();
        let t0 = Instant::now();
        let pinned = CheckpointResult {
            busy: false,
            log_pages: 50,
            checkpointed: 20,
        };
        assert_eq!(
            t.record(CheckpointMode::Passive, pinned, 0, t0, &config),
            BacklogEvent::Deferred { first: true }
        );
        assert_eq!(
            t.record(
                CheckpointMode::Passive,
                pinned,
                0,
                t0 + Duration::from_secs(30),
                &config
            ),
            BacklogEvent::None
        );
        assert_eq!(t.status.backlog_seconds, 30);
        assert_eq!(
            t.record(
                CheckpointMode::Passive,
                pinned,
                0,
                t0 + config.deferral_log_every + Duration::from_secs(1),
                &config
            ),
            BacklogEvent::Deferred { first: false }
        );
        let clear = CheckpointResult {
            busy: false,
            log_pages: 50,
            checkpointed: 50,
        };
        match t.record(
            CheckpointMode::Passive,
            clear,
            0,
            t0 + Duration::from_secs(900),
            &config,
        ) {
            BacklogEvent::Drained { after } => assert_eq!(after.as_secs(), 900),
            other => panic!("expected drain, got {other:?}"),
        }
        assert_eq!(t.status.pending_pages, 0);
        assert_eq!(t.status.checkpoints, 4);
    }

    #[test]
    fn checkpoint_modes_never_truncate() {
        for mode in [
            CheckpointMode::Passive,
            CheckpointMode::Full,
            CheckpointMode::Restart,
        ] {
            assert!(!mode.statement().contains("TRUNCATE"));
        }
    }

    #[test]
    fn wal_and_owner_paths_append_suffixes() {
        assert_eq!(
            wal_file_path(Path::new("/d/db.sqlite")),
            PathBuf::from("/d/db.sqlite-wal")
        );
    }

    /// A file database with a writer pool configured like the real one.
    async fn wal_database(dir: &tempfile::TempDir) -> (PathBuf, SqlitePool) {
        let path = dir.path().join("db.sqlite");
        let options = normal_mode_options(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
            &DbConfig::default(),
            ConnectionRole::Writer,
        );
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(3)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE t(x BLOB)")
            .execute(&pool)
            .await
            .unwrap();
        (path, pool)
    }

    async fn insert_rows(pool: &SqlitePool, n: usize) {
        for _ in 0..n {
            sqlx::query("INSERT INTO t(x) VALUES (randomblob(2000))")
                .execute(pool)
                .await
                .unwrap();
        }
    }

    async fn wait_for(
        state: &WalMaintenanceState,
        what: &str,
        mut done: impl FnMut(&WalMaintenanceStatus) -> bool,
    ) -> WalMaintenanceStatus {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = state.snapshot();
            if done(&status) {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {status:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn fast_config() -> WalMaintenanceConfig {
        WalMaintenanceConfig {
            interval: Duration::from_millis(50),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn writer_never_checkpoints_inline_and_the_loop_does() {
        let dir = tempfile::tempdir().unwrap();
        let (path, pool) = wal_database(&dir).await;
        let auto: i64 = sqlx::query_scalar("PRAGMA wal_autocheckpoint")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(auto, 0);
        insert_rows(&pool, 50).await;
        let before = std::fs::metadata(wal_file_path(&path)).unwrap().len();
        assert!(before > 0, "writes went to the WAL");

        let (handle, rx) = MaintenanceHandle::new();
        spawn_wal_maintenance(
            pool.clone(),
            Arc::new(Semaphore::new(1)),
            path.clone(),
            Arc::clone(&handle.wal),
            fast_config(),
            rx,
        );
        let status = wait_for(&handle.wal, "a checkpoint", |s| s.checkpoints > 0).await;
        assert_eq!(status.last_mode, Some(CheckpointMode::Passive));
        assert!(status.running);
        assert_eq!(status.pending_pages, 0);
        assert!(status.log_pages > 0 && status.log_pages == status.checkpointed);

        // Dropping the handle (what dropping the DatabaseManager does) stops it.
        let state = Arc::clone(&handle.wal);
        drop(handle);
        let stopped = wait_for(&state, "the loop to stop", |s| !s.running).await;
        assert_eq!(stopped.stopped_reason.as_deref(), Some("database closed"));
    }

    #[tokio::test]
    async fn pinned_reader_defers_the_checkpoint_until_it_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let (path, pool) = wal_database(&dir).await;
        insert_rows(&pool, 5).await;

        // A reader holding a snapshot from before the next writes.
        let mut reader = SqliteConnectOptions::new()
            .filename(&path)
            .connect()
            .await
            .unwrap();
        sqlx::query("BEGIN").execute(&mut reader).await.unwrap();
        let _: i64 = sqlx::query_scalar("SELECT count(*) FROM t")
            .fetch_one(&mut reader)
            .await
            .unwrap();
        insert_rows(&pool, 40).await;

        let (handle, rx) = MaintenanceHandle::new();
        spawn_wal_maintenance(
            pool.clone(),
            Arc::new(Semaphore::new(1)),
            path.clone(),
            Arc::clone(&handle.wal),
            fast_config(),
            rx,
        );
        let deferred = wait_for(&handle.wal, "a deferral", |s| s.pending_pages > 0).await;
        assert!(deferred.log_pages > deferred.checkpointed);

        sqlx::query("COMMIT").execute(&mut reader).await.unwrap();
        let drained = wait_for(&handle.wal, "the drain", |s| {
            s.checkpoints > deferred.checkpoints && s.pending_pages == 0
        })
        .await;
        assert_eq!(drained.backlog_seconds, 0);
        use sqlx::Connection;
        reader.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_large_log_escalates_to_restart_and_a_stalled_writer_cannot_block_it() {
        let dir = tempfile::tempdir().unwrap();
        let (path, pool) = wal_database(&dir).await;
        insert_rows(&pool, 20).await;

        // The write coordinator is held for good, as by a wedged writer.
        let coordinator = Arc::new(Semaphore::new(1));
        let _wedged = Arc::clone(&coordinator).acquire_owned().await.unwrap();
        let config = WalMaintenanceConfig {
            interval: Duration::from_millis(50),
            coordinator_wait: Duration::from_millis(20),
            uncoordinated_after_skips: 2,
            escalate_wal_bytes: 1,
            escalation_cooldown: Duration::ZERO,
            ..Default::default()
        };
        let (handle, rx) = MaintenanceHandle::new();
        spawn_wal_maintenance(
            pool.clone(),
            coordinator,
            path,
            Arc::clone(&handle.wal),
            config,
            rx,
        );
        let status = wait_for(&handle.wal, "an escalation", |s| s.escalations > 0).await;
        assert_eq!(status.last_mode, Some(CheckpointMode::Restart));
        assert!(!status.busy);
        // busy_timeout is back to the pool's value on every writer.
        for _ in 0..3 {
            let timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(timeout, CONNECTION_BUSY_TIMEOUT.as_millis() as i64);
        }
    }

    #[tokio::test]
    async fn offline_options_use_rollback_journal_and_exclusive_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("offline.sqlite");
        let mut conn = offline_connect_options(&path, true)
            .create_if_missing(true)
            .connect()
            .await
            .unwrap();
        let pragma = |name: &'static str| format!("PRAGMA {name}");
        let mut text = Vec::new();
        for name in ["journal_mode", "locking_mode"] {
            let v: String = sqlx::query_scalar(sqlx::AssertSqlSafe(pragma(name)))
                .fetch_one(&mut conn)
                .await
                .unwrap();
            text.push(v.to_lowercase());
        }
        assert_eq!(text, ["delete", "exclusive"]);
        let sync: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(sync, 2, "synchronous=FULL");
        let temp: i64 = sqlx::query_scalar("PRAGMA temp_store")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(temp, 1, "temp_store=FILE");
    }
}
