// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Read queries that stop when nobody waits for them any more.
//!
//! A long read (search, timeline, raw SQL from a pipe) runs on a read-pool
//! connection held by a [`CancellableConnection`]. When the future that runs
//! the query is dropped (the HTTP client went away, a timeout fired), the
//! guard calls `sqlite3_interrupt` on *its own* connection, so SQLite stops
//! the statement at its next opcode instead of finishing a query whose result
//! would be thrown away, and hands the connection to a cleanup task. The task
//! keeps interrupting until the connection's worker has moved past the
//! statement (an interrupt that lands before the statement starts is lost,
//! because SQLite clears the flag when a statement starts while no other is
//! active), waits for it to end, finalizes cached statements,
//! rolls back any read transaction left open, and only then lets the
//! connection go back to the pool. A connection that cannot be cleaned up is
//! closed instead of returned.
//!
//! The interrupt can only ever reach the guard's own connection: the guard
//! owns it from acquire to cleanup, so no other request can be running a
//! statement on it, and `sqlite3_interrupt` is a no-op when nothing runs.
//!
//! [`rollback_abandoned_read`] covers ordinary pooled reads: the read pool
//! runs it whenever a connection is released, so a transaction a caller left
//! open never keeps its WAL snapshot pinned.

use std::ptr::NonNull;
use std::time::Duration;

use sqlx::pool::PoolConnection;
use sqlx::sqlite::SqliteConnection;
use sqlx::{Connection, Sqlite, SqlitePool};
use tracing::{debug, info, warn};

/// Longest a cleanup task waits for an interrupted connection to settle.
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);

/// How often a cleanup task repeats the interrupt while the abandoned
/// statement may still be queued or starting.
const INTERRUPT_REPEAT: Duration = Duration::from_millis(50);

/// Raw handle of one connection, used only for `sqlite3_interrupt`.
#[derive(Clone, Copy)]
struct InterruptHandle(NonNull<libsqlite3_sys::sqlite3>);

// SAFETY: sqlite3_interrupt is documented as safe to call from any thread
// while another thread runs a statement on the connection, as long as the
// connection stays open; the guard keeps the connection alive for as long as
// it holds the handle.
unsafe impl Send for InterruptHandle {}
unsafe impl Sync for InterruptHandle {}

impl InterruptHandle {
    fn interrupt(self) {
        // SAFETY: see the Send/Sync note; the owning connection is open.
        unsafe { libsqlite3_sys::sqlite3_interrupt(self.0.as_ptr()) }
    }
}

/// Whether the connection has a transaction open.
async fn in_transaction(conn: &mut SqliteConnection) -> Result<bool, sqlx::Error> {
    let mut handle = conn.lock_handle().await?;
    // SAFETY: the locked handle guarantees the worker thread is not using the
    // connection while we read its autocommit flag.
    let autocommit =
        unsafe { libsqlite3_sys::sqlite3_get_autocommit(handle.as_raw_handle().as_ptr()) };
    Ok(autocommit == 0)
}

/// Roll back a read transaction left open on a connection that is being
/// released. `Ok(false)` tells the pool to close the connection.
pub async fn rollback_abandoned_read(conn: &mut SqliteConnection) -> Result<bool, sqlx::Error> {
    if !in_transaction(conn).await? {
        return Ok(true);
    }
    match sqlx::query("ROLLBACK").execute(&mut *conn).await {
        Ok(_) => {
            info!("rolled back an abandoned read transaction before reusing its connection");
            Ok(true)
        }
        Err(e) => {
            warn!(
                "rolling back an abandoned read transaction failed ({}); closing the connection",
                e
            );
            Ok(false)
        }
    }
}

/// A read-pool connection whose statement is interrupted if the guard is
/// dropped before [`CancellableConnection::finish`].
pub struct CancellableConnection {
    conn: Option<PoolConnection<Sqlite>>,
    handle: InterruptHandle,
}

impl CancellableConnection {
    /// Acquire a connection from `pool`.
    pub async fn acquire(pool: &SqlitePool) -> Result<Self, sqlx::Error> {
        let mut conn = pool.acquire().await?;
        let handle = {
            let mut locked = conn.lock_handle().await?;
            InterruptHandle(locked.as_raw_handle())
        };
        Ok(Self {
            conn: Some(conn),
            handle,
        })
    }

    /// The connection, to run queries on.
    pub fn conn(&mut self) -> &mut SqliteConnection {
        self.conn
            .as_deref_mut()
            .expect("CancellableConnection used after finish")
    }

    /// The query finished: return the connection to the pool normally.
    pub fn finish(mut self) {
        drop(self.conn.take());
    }
}

impl Drop for CancellableConnection {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else {
            return;
        };
        // Dropped before finish(): a statement may still be running on this
        // connection. Stop it, then clean up off the caller's path.
        self.handle.interrupt();
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(cleanup(conn, self.handle));
            }
            Err(_) => {
                // No runtime to clean up on: never return a connection that
                // may still be mid-statement.
                let _ = conn.detach();
            }
        }
    }
}

/// Make an interrupted connection reusable, or close it. `handle` belongs to
/// `conn`, which this task owns, so it stays valid throughout.
async fn cleanup(mut conn: PoolConnection<Sqlite>, handle: InterruptHandle) {
    let settled = tokio::time::timeout(CLEANUP_TIMEOUT, async {
        // Queued behind the abandoned statement on the connection's worker,
        // so this returns once that statement has ended. The statement may
        // not have started when the guard's interrupt landed (still queued,
        // or being prepared), and starting clears the flag, so interrupt
        // again until then. Finalizing cached statements leaves none
        // mid-step; a flag set after that is cleared when the next statement
        // starts.
        {
            let mut clear = std::pin::pin!(conn.clear_cached_statements());
            let mut repeat = tokio::time::interval(INTERRUPT_REPEAT);
            repeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // The first tick fires at once; the guard has just interrupted.
            repeat.tick().await;
            loop {
                tokio::select! {
                    cleared = &mut clear => {
                        cleared?;
                        break;
                    }
                    _ = repeat.tick() => handle.interrupt(),
                }
            }
        }
        let in_txn = in_transaction(&mut conn).await?;
        if in_txn {
            sqlx::query("ROLLBACK").execute(&mut *conn).await?;
            info!("rolled back the read transaction of an interrupted query");
        }
        Ok::<_, sqlx::Error>(())
    })
    .await;
    match settled {
        Ok(Ok(())) => {
            debug!("interrupted read connection cleaned up and returned to the pool");
        }
        Ok(Err(e)) => {
            warn!(
                "cleaning up an interrupted read connection failed ({}); closing it instead of returning it",
                e
            );
            if let Err(e) = conn.close().await {
                debug!("closing the interrupted read connection: {}", e);
            }
        }
        Err(_) => {
            warn!(
                "interrupted read connection did not settle within {}s; detaching it from the pool",
                CLEANUP_TIMEOUT.as_secs()
            );
            let _ = conn.detach();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::time::Instant;

    /// A statement that runs far longer than any test.
    const ENDLESS: &str = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) \
                           SELECT count(*) FROM n";

    async fn single_connection_pool(dir: &tempfile::TempDir) -> SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(dir.path().join("t.sqlite"))
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        SqlitePoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(2))
            .after_release(|conn, _| Box::pin(async move { rollback_abandoned_read(conn).await }))
            .connect_with(options)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn dropping_a_running_query_interrupts_it_and_frees_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        let pool = single_connection_pool(&dir).await;

        let query = async {
            let mut guard = CancellableConnection::acquire(&pool).await?;
            let n: i64 = sqlx::query_scalar(ENDLESS).fetch_one(guard.conn()).await?;
            guard.finish();
            Ok::<_, sqlx::Error>(n)
        };
        let timed_out = tokio::time::timeout(Duration::from_millis(300), query).await;
        assert!(timed_out.is_err(), "the endless query cannot finish");

        // The only connection must come back, usable, within a second.
        let dropped_at = Instant::now();
        let mut conn = tokio::time::timeout(Duration::from_secs(1), pool.acquire())
            .await
            .expect("connection returned within 1s")
            .expect("acquire");
        assert!(dropped_at.elapsed() < Duration::from_secs(1));
        let one: i64 = sqlx::query_scalar("SELECT 1")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        assert_eq!(one, 1);
        assert!(!in_transaction(&mut conn).await.unwrap());
    }

    #[tokio::test]
    async fn query_still_queued_when_dropped_is_interrupted_once_it_starts() {
        // An earlier statement whose future was dropped (as try_join does to
        // its siblings) still runs on the worker, so the endless one is only
        // queued when the guard is dropped. The guard's single interrupt
        // reaches the earlier statement or nothing, and SQLite clears the
        // flag when the queued one starts; the cleanup task's repeated
        // interrupt must stop it anyway.
        const EARLIER: &str = "WITH RECURSIVE n(i) AS \
                               (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3000000) \
                               SELECT count(*) FROM n";
        let dir = tempfile::tempdir().unwrap();
        let pool = single_connection_pool(&dir).await;
        for _ in 0..3 {
            let mut guard = CancellableConnection::acquire(&pool).await.unwrap();
            {
                let earlier = sqlx::query_scalar::<_, i64>(EARLIER).fetch_one(guard.conn());
                let _ = tokio::time::timeout(Duration::ZERO, earlier).await;
            }
            {
                let queued = sqlx::query_scalar::<_, i64>(ENDLESS).fetch_one(guard.conn());
                let _ = tokio::time::timeout(Duration::ZERO, queued).await;
            }
            drop(guard);
            let mut conn = tokio::time::timeout(Duration::from_secs(2), pool.acquire())
                .await
                .expect("connection returned once the queued statement was stopped")
                .expect("acquire");
            let one: i64 = sqlx::query_scalar("SELECT 1")
                .fetch_one(&mut *conn)
                .await
                .unwrap();
            assert_eq!(one, 1);
        }
    }

    #[tokio::test]
    async fn finished_query_is_not_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let pool = single_connection_pool(&dir).await;
        let mut guard = CancellableConnection::acquire(&pool).await.unwrap();
        let v: i64 = sqlx::query_scalar("SELECT 41 + 1")
            .fetch_one(guard.conn())
            .await
            .unwrap();
        guard.finish();
        assert_eq!(v, 42);
        let v: i64 = sqlx::query_scalar("SELECT 7")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(v, 7);
    }

    #[tokio::test]
    async fn abandoned_read_transaction_is_rolled_back_on_release() {
        let dir = tempfile::tempdir().unwrap();
        let pool = single_connection_pool(&dir).await;
        sqlx::query("CREATE TABLE t(x)")
            .execute(&pool)
            .await
            .unwrap();
        {
            let mut conn = pool.acquire().await.unwrap();
            sqlx::query("BEGIN").execute(&mut *conn).await.unwrap();
            let _: Option<i64> = sqlx::query_scalar("SELECT x FROM t")
                .fetch_optional(&mut *conn)
                .await
                .unwrap();
            assert!(in_transaction(&mut conn).await.unwrap());
        }
        let mut conn = pool.acquire().await.unwrap();
        assert!(!in_transaction(&mut conn).await.unwrap());
    }

    #[tokio::test]
    async fn interrupt_only_reaches_the_guarded_connection() {
        let dir = tempfile::tempdir().unwrap();
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(dir.path().join("t.sqlite"))
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .unwrap();

        // A bounded but slow-ish query on the other connection must complete.
        let mut other = pool.acquire().await.unwrap();
        let counting = async {
            sqlx::query_scalar::<_, i64>(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 2000000) \
                 SELECT count(*) FROM n",
            )
            .fetch_one(&mut *other)
            .await
        };
        let interrupting = async {
            let mut guard = CancellableConnection::acquire(&pool).await.unwrap();
            let _ = tokio::time::timeout(
                Duration::from_millis(50),
                sqlx::query_scalar::<_, i64>(ENDLESS).fetch_one(guard.conn()),
            )
            .await;
            drop(guard);
        };
        let (count, ()) = tokio::join!(counting, interrupting);
        assert_eq!(count.unwrap(), 2_000_000);
    }
}
