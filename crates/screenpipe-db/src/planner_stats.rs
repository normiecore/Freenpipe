// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Query-planner statistics maintenance.
//!
//! SQLite picks query plans from the statistics `ANALYZE` stores in
//! `sqlite_stat1`. A capture database grows by orders of magnitude, so stale
//! (or missing) statistics end up with plans that scan whole tables. This
//! module keeps them current without getting in the way of recording:
//!
//! 1. **selecting_tables**: `PRAGMA main.optimize=0x10003` in report-only
//!    mode lists the tables SQLite thinks would benefit, and every ordinary
//!    indexed table that has no statistics yet is added.
//! 2. **analyzing**: each table gets `ANALYZE` with a bounded
//!    `analysis_limit`. The write coordinator is taken per table and the
//!    connection runs with `busy_timeout=0`, so a busy writer makes the run
//!    stop and retry on the next tick instead of waiting.
//! 3. **refreshing_planner**: `ANALYZE main.sqlite_schema` reloads the
//!    statistics on this connection, and creating and dropping the scratch
//!    table `_screenpipe_planner_refresh` in one transaction bumps the schema
//!    cookie so every other pooled connection reloads them too.
//!
//! Each attempt is recorded in `<data dir>/sqlite-planner-maintenance.json`
//! (`latest`, `last_success`, `last_failure`), written atomically.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::pool::PoolConnection;
use sqlx::{Sqlite, SqlitePool};
use tokio::sync::{watch, Semaphore};
use tracing::{debug, info, warn};

use crate::maintenance::{shutdown_signalled, sqlite_primary_code, CONNECTION_BUSY_TIMEOUT};

/// Name of the report file in the data dir.
pub const PLANNER_REPORT_FILE: &str = "sqlite-planner-maintenance.json";

/// Overrides the planner maintenance schedule (seconds): the first run waits
/// this long and later runs follow at the same interval.
pub const PLANNER_MAINTENANCE_INTERVAL_ENV: &str = "SCREENPIPE_PLANNER_MAINTENANCE_INTERVAL_SECS";

/// Rows `ANALYZE` samples per index.
pub const ANALYSIS_LIMIT: u32 = 1000;

/// Scratch table whose create/drop bumps the schema cookie.
const REFRESH_TABLE: &str = "_screenpipe_planner_refresh";

/// Attempt outcomes as stored in the report.
pub mod status {
    pub const RUNNING: &str = "running";
    pub const SUCCEEDED: &str = "succeeded";
    /// The writer (or another process) held the write lock; retried next tick.
    pub const BUSY: &str = "busy";
    /// A statement was interrupted; retried next tick.
    pub const INTERRUPTED: &str = "interrupted";
    /// The database closed mid-run; retried after the next start.
    pub const SHUTDOWN: &str = "shutdown";
    pub const FAILED: &str = "failed";
}

/// Stages an attempt passes through.
pub mod stage {
    pub const SELECTING_TABLES: &str = "selecting_tables";
    pub const ANALYZING: &str = "analyzing";
    pub const REFRESHING_PLANNER: &str = "refreshing_planner";
    pub const DONE: &str = "done";
}

/// One maintenance attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerAttempt {
    /// RFC 3339 start time (UTC).
    pub started_at: String,
    pub status: String,
    pub duration_ms: u64,
    /// Tables `ANALYZE` ran on.
    #[serde(default)]
    pub analyzed_tables: Vec<String>,
    /// Stage the attempt ended in.
    pub stage: String,
    /// SQLite result code of the failure, when there was one.
    #[serde(default)]
    pub sqlite_code: Option<i32>,
    #[serde(default)]
    pub detail: Option<String>,
}

/// Contents of the report file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerMaintenanceReport {
    #[serde(default)]
    pub latest: Option<PlannerAttempt>,
    #[serde(default)]
    pub last_success: Option<PlannerAttempt>,
    #[serde(default)]
    pub last_failure: Option<PlannerAttempt>,
}

impl PlannerMaintenanceReport {
    /// Fold a finished (or started) attempt into the report.
    pub fn record(&mut self, attempt: PlannerAttempt) {
        match attempt.status.as_str() {
            status::SUCCEEDED => self.last_success = Some(attempt.clone()),
            status::RUNNING => {}
            _ => self.last_failure = Some(attempt.clone()),
        }
        self.latest = Some(attempt);
    }
}

/// Path of the report for a database file.
pub fn report_path(database: &Path) -> PathBuf {
    database
        .parent()
        .map(|dir| dir.join(PLANNER_REPORT_FILE))
        .unwrap_or_else(|| PathBuf::from(PLANNER_REPORT_FILE))
}

/// Read a report; a missing or unreadable file reads as empty.
pub fn read_report(path: &Path) -> PlannerMaintenanceReport {
    std::fs::read(path)
        .ok()
        .and_then(|body| serde_json::from_slice(&body).ok())
        .unwrap_or_default()
}

/// Write `report` to `path` atomically (temp file in the same directory, then
/// rename over the old file).
pub fn write_report(path: &Path, report: &PlannerMaintenanceReport) -> std::io::Result<()> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .ok_or_else(|| std::io::Error::other("report path has no parent directory"))?;
    let body = serde_json::to_vec_pretty(report).map_err(std::io::Error::other)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| PLANNER_REPORT_FILE.to_string());
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{name}.tmp-{}-{nonce}", std::process::id()));
    let written = (|| {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&body)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// When planner maintenance runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannerSchedule {
    /// Delay after start-up, so the first run does not compete with
    /// migrations and the first capture writes.
    pub initial_delay: Duration,
    /// Time between runs.
    pub interval: Duration,
    /// Wait before trying again after a run that found the writer busy or
    /// was interrupted.
    pub retry_after: Duration,
}

impl Default for PlannerSchedule {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(10 * 60),
            interval: Duration::from_secs(6 * 60 * 60),
            retry_after: Duration::from_secs(10 * 60),
        }
    }
}

impl PlannerSchedule {
    /// Defaults, or both values from [`PLANNER_MAINTENANCE_INTERVAL_ENV`].
    pub fn from_env() -> Self {
        match std::env::var(PLANNER_MAINTENANCE_INTERVAL_ENV)
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|s| *s > 0)
        {
            Some(secs) => Self {
                initial_delay: Duration::from_secs(secs),
                interval: Duration::from_secs(secs),
                retry_after: Duration::from_secs(secs),
            },
            None => Self::default(),
        }
    }
}

impl PlannerSchedule {
    /// Wait before the next run after an attempt that ended with `status`:
    /// a busy writer or an interrupt is retried sooner than the interval.
    pub fn next_wait(&self, status_text: &str) -> Duration {
        match status_text {
            status::BUSY | status::INTERRUPTED => self.retry_after.min(self.interval),
            _ => self.interval,
        }
    }
}

/// Why an attempt stopped early.
struct Stop {
    status: &'static str,
    sqlite_code: Option<i32>,
    detail: String,
}

impl Stop {
    fn from_error(err: &sqlx::Error) -> Self {
        let code = sqlite_primary_code(err);
        let status = match (err, code) {
            (_, Some(5)) | (_, Some(6)) => status::BUSY,
            (_, Some(9)) => status::INTERRUPTED,
            (sqlx::Error::PoolClosed, _) => status::SHUTDOWN,
            _ => status::FAILED,
        };
        let detail = match err {
            sqlx::Error::PoolClosed => "database pool already closed".to_string(),
            sqlx::Error::PoolTimedOut => {
                "database pool unavailable (acquire timed out)".to_string()
            }
            sqlx::Error::Io(e) => format!("I/O error during planner maintenance: {e}"),
            other => other.to_string(),
        };
        Self {
            status,
            sqlite_code: match err {
                sqlx::Error::Database(db) => db.code().and_then(|c| c.parse().ok()),
                _ => None,
            },
            detail,
        }
    }
}

/// Parse a table name out of one line of `PRAGMA optimize` report output,
/// such as `ANALYZE "main"."frames"`.
pub fn table_from_optimize_line(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("ANALYZE")?.trim();
    let last = rest.rsplit('.').next()?.trim();
    let name = last.trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']');
    if name.is_empty() {
        None
    } else {
        Some(name.replace("\"\"", "\""))
    }
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Everything one attempt needs.
pub(crate) struct PlannerJob {
    pub write_pool: SqlitePool,
    pub coordinator: Arc<Semaphore>,
    pub report_path: PathBuf,
    pub coordinator_wait: Duration,
}

impl PlannerJob {
    /// Run one attempt and record it. Returns the finished attempt.
    pub async fn run_once(&self, shutdown: &watch::Receiver<()>) -> PlannerAttempt {
        let started = Instant::now();
        let started_at = Utc::now().to_rfc3339();
        let mut report = read_report(&self.report_path);
        report.record(PlannerAttempt {
            started_at: started_at.clone(),
            status: status::RUNNING.into(),
            duration_ms: 0,
            analyzed_tables: Vec::new(),
            stage: stage::SELECTING_TABLES.into(),
            sqlite_code: None,
            detail: None,
        });
        self.save(&report);

        let mut analyzed = Vec::new();
        let mut current_stage = stage::SELECTING_TABLES;
        let outcome = self
            .attempt(&mut analyzed, &mut current_stage, shutdown)
            .await;
        let (status_text, sqlite_code, detail) = match outcome {
            Ok(()) => (status::SUCCEEDED, None, None),
            Err(stop) => (stop.status, stop.sqlite_code, Some(stop.detail)),
        };
        let attempt = PlannerAttempt {
            started_at,
            status: status_text.into(),
            duration_ms: started.elapsed().as_millis() as u64,
            analyzed_tables: analyzed,
            stage: current_stage.into(),
            sqlite_code,
            detail,
        };
        match attempt.status.as_str() {
            status::SUCCEEDED => info!(
                tables = attempt.analyzed_tables.len(),
                duration_ms = attempt.duration_ms,
                "sqlite planner statistics refreshed"
            ),
            status::FAILED => warn!(
                stage = %attempt.stage,
                "sqlite planner maintenance failed: {}",
                attempt.detail.as_deref().unwrap_or("")
            ),
            other => debug!(
                status = other,
                stage = %attempt.stage,
                "sqlite planner maintenance deferred: {}",
                attempt.detail.as_deref().unwrap_or("")
            ),
        }
        report.record(attempt.clone());
        self.save(&report);
        attempt
    }

    fn save(&self, report: &PlannerMaintenanceReport) {
        if let Err(e) = write_report(&self.report_path, report) {
            warn!(
                path = %self.report_path.display(),
                "writing the planner maintenance report failed: {}",
                e
            );
        }
    }

    async fn coordinator(&self) -> Result<tokio::sync::OwnedSemaphorePermit, Stop> {
        match tokio::time::timeout(
            self.coordinator_wait,
            Arc::clone(&self.coordinator).acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(Stop {
                status: status::SHUTDOWN,
                sqlite_code: None,
                detail: "write coordinator closed".into(),
            }),
            Err(_) => Err(Stop {
                status: status::BUSY,
                sqlite_code: None,
                detail: "writer busy".into(),
            }),
        }
    }

    async fn attempt(
        &self,
        analyzed: &mut Vec<String>,
        current_stage: &mut &'static str,
        shutdown: &watch::Receiver<()>,
    ) -> Result<(), Stop> {
        if crate::write_queue_health::write_queue_health().degraded {
            return Err(Stop {
                status: status::BUSY,
                sqlite_code: None,
                detail: "write queue degraded".into(),
            });
        }
        let mut conn = self
            .write_pool
            .acquire()
            .await
            .map_err(|e| Stop::from_error(&e))?;
        // Non-blocking: if anyone else holds the write lock, give up at once.
        let configured = set_busy_timeout(&mut conn, Duration::ZERO).await;
        let result = async {
            configured.map_err(|e| Stop::from_error(&e))?;
            let tables = {
                let _permit = self.coordinator().await?;
                select_tables(&mut conn)
                    .await
                    .map_err(|e| Stop::from_error(&e))?
            };

            *current_stage = stage::ANALYZING;
            let limit = format!("PRAGMA analysis_limit = {ANALYSIS_LIMIT}");
            sqlx::query(sqlx::AssertSqlSafe(limit))
                .execute(&mut *conn)
                .await
                .map_err(|e| Stop::from_error(&e))?;
            for table in tables {
                if shutdown.has_changed().is_err() {
                    return Err(Stop {
                        status: status::SHUTDOWN,
                        sqlite_code: None,
                        detail: "database closed during planner maintenance".into(),
                    });
                }
                let _permit = self.coordinator().await?;
                let sql = format!("ANALYZE main.{}", quote_ident(&table));
                sqlx::query(sqlx::AssertSqlSafe(sql))
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| Stop::from_error(&e))?;
                analyzed.push(table);
            }

            *current_stage = stage::REFRESHING_PLANNER;
            if !analyzed.is_empty() {
                let _permit = self.coordinator().await?;
                refresh_planner(&mut conn)
                    .await
                    .map_err(|e| Stop::from_error(&e))?;
            }
            *current_stage = stage::DONE;
            Ok(())
        }
        .await;

        // Never hand a writer with busy_timeout=0 back to the pool.
        if set_busy_timeout(&mut conn, CONNECTION_BUSY_TIMEOUT)
            .await
            .is_err()
        {
            warn!("planner maintenance: restoring busy_timeout failed; closing the connection");
            conn.close_on_drop();
        }
        result
    }
}

async fn set_busy_timeout(
    conn: &mut PoolConnection<Sqlite>,
    timeout: Duration,
) -> Result<(), sqlx::Error> {
    let sql = format!("PRAGMA busy_timeout = {}", timeout.as_millis());
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .execute(&mut **conn)
        .await
        .map(|_| ())
}

/// Tables that would benefit from `ANALYZE`, in a stable order.
async fn select_tables(conn: &mut PoolConnection<Sqlite>) -> Result<Vec<String>, sqlx::Error> {
    let mut tables: Vec<String> = Vec::new();
    // Report-only optimize: lists the ANALYZE statements it would run.
    let lines: Vec<String> = sqlx::query_scalar("PRAGMA main.optimize=0x10003")
        .fetch_all(&mut **conn)
        .await?;
    for line in &lines {
        if let Some(table) = table_from_optimize_line(line) {
            if !table.starts_with("sqlite_") && table != REFRESH_TABLE && !tables.contains(&table) {
                tables.push(table);
            }
        }
    }

    // Ordinary tables with an index of their own but no statistics yet.
    let has_stat1: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM main.sqlite_schema WHERE type = 'table' AND name = 'sqlite_stat1')",
    )
    .fetch_one(&mut **conn)
    .await?;
    let unanalyzed_sql = if has_stat1 {
        "SELECT DISTINCT t.name FROM main.sqlite_schema t \
         JOIN main.sqlite_schema i ON i.type = 'index' AND i.tbl_name = t.name \
         WHERE t.type = 'table' AND t.name NOT LIKE 'sqlite_%' AND t.sql NOT LIKE 'CREATE VIRTUAL%' \
           AND t.name NOT IN (SELECT tbl FROM main.sqlite_stat1) \
         ORDER BY t.name"
    } else {
        "SELECT DISTINCT t.name FROM main.sqlite_schema t \
         JOIN main.sqlite_schema i ON i.type = 'index' AND i.tbl_name = t.name \
         WHERE t.type = 'table' AND t.name NOT LIKE 'sqlite_%' AND t.sql NOT LIKE 'CREATE VIRTUAL%' \
         ORDER BY t.name"
    };
    let unanalyzed: Vec<String> = sqlx::query_scalar(unanalyzed_sql)
        .fetch_all(&mut **conn)
        .await?;
    for table in unanalyzed {
        if table != REFRESH_TABLE && !tables.contains(&table) {
            tables.push(table);
        }
    }

    // An empty table without statistics has nothing to gain from ANALYZE
    // (it would only be selected again on every run); one that still has
    // statistics from when it held rows is analyzed so they are cleared.
    let mut selected = Vec::with_capacity(tables.len());
    for table in tables {
        let has_rows: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT EXISTS(SELECT 1 FROM main.{})",
            quote_ident(&table)
        )))
        .fetch_one(&mut **conn)
        .await?;
        let has_stats: bool = has_stat1
            && sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM main.sqlite_stat1 WHERE tbl = ?1)")
                .bind(&table)
                .fetch_one(&mut **conn)
                .await?;
        if has_rows || has_stats {
            selected.push(table);
        }
    }
    Ok(selected)
}

/// Reload statistics here and make every other connection reload them.
async fn refresh_planner(conn: &mut PoolConnection<Sqlite>) -> Result<(), sqlx::Error> {
    sqlx::query("ANALYZE main.sqlite_schema")
        .execute(&mut **conn)
        .await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut **conn).await?;
    let bump = async {
        let create = format!("CREATE TABLE IF NOT EXISTS main.{REFRESH_TABLE}(dummy)");
        sqlx::query(sqlx::AssertSqlSafe(create))
            .execute(&mut **conn)
            .await?;
        let drop = format!("DROP TABLE IF EXISTS main.{REFRESH_TABLE}");
        sqlx::query(sqlx::AssertSqlSafe(drop))
            .execute(&mut **conn)
            .await?;
        Ok::<_, sqlx::Error>(())
    }
    .await;
    match bump {
        Ok(()) => match sqlx::query("COMMIT").execute(&mut **conn).await {
            Ok(_) => Ok(()),
            Err(e) => {
                // A failed COMMIT (disk full, I/O error) can leave the
                // transaction open, and with it the write lock, on a writer
                // that goes back to the pool: roll back, and never reuse it.
                if let Err(rb) = sqlx::query("ROLLBACK").execute(&mut **conn).await {
                    debug!(
                        "planner maintenance: rollback after a failed commit: {}",
                        rb
                    );
                }
                conn.close_on_drop();
                Err(e)
            }
        },
        Err(e) => {
            if let Err(rb) = sqlx::query("ROLLBACK").execute(&mut **conn).await {
                warn!(
                    "planner maintenance: rollback after a failed refresh: {}",
                    rb
                );
                conn.close_on_drop();
            }
            Err(e)
        }
    }
}

/// Spawn the periodic planner maintenance for one database.
pub(crate) fn spawn_planner_maintenance(
    job: PlannerJob,
    schedule: PlannerSchedule,
    mut shutdown: watch::Receiver<()>,
) {
    tokio::spawn(async move {
        let mut wait = schedule.initial_delay;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = shutdown_signalled(&mut shutdown) => break,
            }
            if job.write_pool.is_closed() {
                break;
            }
            let attempt = job.run_once(&shutdown).await;
            wait = schedule.next_wait(&attempt.status);
        }
        debug!("planner maintenance stopped");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(status_text: &str) -> PlannerAttempt {
        PlannerAttempt {
            started_at: "2026-10-20T00:00:00Z".into(),
            status: status_text.into(),
            duration_ms: 3,
            analyzed_tables: vec!["frames".into()],
            stage: stage::DONE.into(),
            sqlite_code: None,
            detail: None,
        }
    }

    #[test]
    fn report_keeps_latest_success_and_failure() {
        let mut report = PlannerMaintenanceReport::default();
        report.record(attempt(status::RUNNING));
        assert!(report.last_success.is_none() && report.last_failure.is_none());
        report.record(attempt(status::SUCCEEDED));
        report.record(attempt(status::BUSY));
        assert_eq!(report.latest.as_ref().unwrap().status, status::BUSY);
        assert_eq!(
            report.last_success.as_ref().unwrap().status,
            status::SUCCEEDED
        );
        assert_eq!(report.last_failure.as_ref().unwrap().status, status::BUSY);

        let json = serde_json::to_value(&report).unwrap();
        for key in ["latest", "last_success", "last_failure"] {
            assert!(json.get(key).is_some(), "{key}");
        }
        let latest = &json["latest"];
        for key in [
            "started_at",
            "status",
            "duration_ms",
            "analyzed_tables",
            "stage",
            "sqlite_code",
            "detail",
        ] {
            assert!(latest.get(key).is_some(), "{key}");
        }
    }

    #[test]
    fn report_is_written_atomically_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PLANNER_REPORT_FILE);
        assert_eq!(read_report(&path), PlannerMaintenanceReport::default());
        let mut report = PlannerMaintenanceReport::default();
        report.record(attempt(status::SUCCEEDED));
        write_report(&path, &report).unwrap();
        write_report(&path, &report).unwrap();
        assert_eq!(read_report(&path), report);
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());
        // A file in another shape reads as empty instead of failing.
        std::fs::write(&path, b"{\"latest\": 3}").unwrap();
        assert_eq!(read_report(&path), PlannerMaintenanceReport::default());
    }

    #[test]
    fn optimize_lines_name_their_tables() {
        assert_eq!(
            table_from_optimize_line("ANALYZE \"main\".\"frames\""),
            Some("frames".into())
        );
        assert_eq!(
            table_from_optimize_line("ANALYZE main.audio_chunks"),
            Some("audio_chunks".into())
        );
        assert_eq!(table_from_optimize_line("PRAGMA something"), None);
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
    }

    async fn database(dir: &tempfile::TempDir) -> (PathBuf, SqlitePool) {
        let path = dir.path().join("db.sqlite");
        let options = crate::maintenance::normal_mode_options(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
            &screenpipe_config::DbConfig::default(),
            crate::maintenance::ConnectionRole::Writer,
        );
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .unwrap();
        for sql in [
            "CREATE TABLE frames(id INTEGER PRIMARY KEY, app TEXT, ts TEXT)",
            "CREATE INDEX idx_frames_app ON frames(app)",
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT)",
            "CREATE TABLE tags(id INTEGER PRIMARY KEY, name TEXT)",
            "CREATE INDEX idx_tags_name ON tags(name)",
            "INSERT INTO frames(app, ts) WITH RECURSIVE s(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM s WHERE i < 500) \
             SELECT 'app' || (i % 7), datetime('now') FROM s",
            "INSERT INTO tags(name) VALUES ('a'), ('b')",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(sql)).execute(&pool).await.unwrap();
        }
        (path, pool)
    }

    fn job(path: &Path, pool: &SqlitePool) -> PlannerJob {
        PlannerJob {
            write_pool: pool.clone(),
            coordinator: Arc::new(Semaphore::new(1)),
            report_path: report_path(path),
            coordinator_wait: Duration::from_secs(2),
        }
    }

    async fn busy_timeouts(pool: &SqlitePool) -> Vec<i64> {
        let mut out = Vec::new();
        let mut conns = Vec::new();
        for _ in 0..2 {
            let mut c = pool.acquire().await.unwrap();
            out.push(
                sqlx::query_scalar("PRAGMA busy_timeout")
                    .fetch_one(&mut *c)
                    .await
                    .unwrap(),
            );
            conns.push(c);
        }
        out
    }

    #[tokio::test]
    async fn run_analyzes_indexed_tables_and_records_success() {
        let dir = tempfile::tempdir().unwrap();
        let (path, pool) = database(&dir).await;
        let (_tx, rx) = watch::channel(());
        let attempt = job(&path, &pool).run_once(&rx).await;
        assert_eq!(attempt.status, status::SUCCEEDED, "{attempt:?}");
        assert_eq!(attempt.stage, stage::DONE);
        assert!(attempt.analyzed_tables.contains(&"frames".to_string()));
        assert!(attempt.analyzed_tables.contains(&"tags".to_string()));

        let stats: i64 =
            sqlx::query_scalar("SELECT count(*) FROM sqlite_stat1 WHERE tbl = 'frames'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(stats > 0);
        let scratch: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_schema WHERE name = '_screenpipe_planner_refresh'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(scratch, 0);

        let report = read_report(&report_path(&path));
        assert_eq!(report.latest.as_ref().unwrap().status, status::SUCCEEDED);
        assert!(report.last_success.is_some());
        assert!(report.last_failure.is_none());
        assert_eq!(busy_timeouts(&pool).await, vec![5000, 5000]);

        // A second run finds nothing new to analyze and still succeeds.
        let again = job(&path, &pool).run_once(&rx).await;
        assert_eq!(again.status, status::SUCCEEDED);
        assert!(again.analyzed_tables.is_empty(), "{again:?}");
    }

    #[tokio::test]
    async fn run_backs_off_when_another_connection_holds_the_write_lock() {
        use sqlx::ConnectOptions;
        let dir = tempfile::tempdir().unwrap();
        let (path, pool) = database(&dir).await;
        let mut other = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .connect()
            .await
            .unwrap();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut other)
            .await
            .unwrap();

        let (_tx, rx) = watch::channel(());
        let started = Instant::now();
        let attempt = job(&path, &pool).run_once(&rx).await;
        assert_eq!(attempt.status, status::BUSY, "{attempt:?}");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "never waits for the lock"
        );
        let report = read_report(&report_path(&path));
        assert_eq!(report.last_failure.as_ref().unwrap().status, status::BUSY);
        assert!(report.last_success.is_none());
        assert_eq!(busy_timeouts(&pool).await, vec![5000, 5000]);

        sqlx::query("ROLLBACK").execute(&mut other).await.unwrap();
        let attempt = job(&path, &pool).run_once(&rx).await;
        assert_eq!(attempt.status, status::SUCCEEDED, "{attempt:?}");
    }

    #[test]
    fn busy_runs_are_retried_sooner() {
        let schedule = PlannerSchedule::default();
        assert_eq!(schedule.next_wait(status::SUCCEEDED), schedule.interval);
        assert_eq!(schedule.next_wait(status::FAILED), schedule.interval);
        assert_eq!(schedule.next_wait(status::BUSY), schedule.retry_after);
        assert_eq!(
            schedule.next_wait(status::INTERRUPTED),
            schedule.retry_after
        );
        assert!(schedule.retry_after < schedule.interval);
    }

    #[test]
    fn report_lives_next_to_the_database() {
        assert_eq!(
            report_path(Path::new("/data/db.sqlite")),
            PathBuf::from("/data/sqlite-planner-maintenance.json")
        );
    }
}
