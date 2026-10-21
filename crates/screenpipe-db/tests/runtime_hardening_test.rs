// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! W2-01 acceptance checks through `DatabaseManager`: connection pragmas,
//! start-up preflight, ownership, the on-demand checkpoint and cancellable
//! reads.

use std::path::Path;
use std::time::{Duration, Instant};

use screenpipe_config::DbConfig;
use screenpipe_db::cancellable_query::CancellableConnection;
use screenpipe_db::ownership::{is_ownership_error, owner_file_path};
use screenpipe_db::DatabaseManager;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection};

async fn open(path: &Path) -> DatabaseManager {
    DatabaseManager::new(path.to_str().unwrap(), DbConfig::default())
        .await
        .expect("DatabaseManager::new")
}

async fn pragma_i64(pool: &sqlx::SqlitePool, name: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("PRAGMA {name}")))
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn writer_and_reader_pragmas() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir.path().join("db.sqlite")).await;

    // Writers: never checkpoint inline; WAL file capped after a restart.
    assert_eq!(pragma_i64(db.write_pool(), "wal_autocheckpoint").await, 0);
    assert_eq!(pragma_i64(db.write_pool(), "query_only").await, 0);
    assert_eq!(pragma_i64(db.write_pool(), "synchronous").await, 1);
    assert_eq!(pragma_i64(db.write_pool(), "temp_store").await, 2);
    assert_eq!(
        pragma_i64(db.write_pool(), "journal_size_limit").await as u64,
        screenpipe_db::maintenance::JOURNAL_SIZE_LIMIT_BYTES
    );

    // Readers: query_only on every connection, same WAL settings.
    for _ in 0..3 {
        assert_eq!(pragma_i64(&db.pool, "query_only").await, 1);
        assert_eq!(pragma_i64(&db.pool, "wal_autocheckpoint").await, 0);
    }
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(mode.to_lowercase(), "wal");

    // A write on the read pool is refused; the write pool takes it.
    let refused = sqlx::query("INSERT INTO tags (name) VALUES ('nope')")
        .execute(&db.pool)
        .await
        .unwrap_err();
    assert!(
        refused.to_string().to_lowercase().contains("readonly")
            || refused.to_string().to_lowercase().contains("read-only")
            || refused.to_string().to_lowercase().contains("read only"),
        "{refused}"
    );
    sqlx::query("INSERT INTO tags (name) VALUES ('yes')")
        .execute(db.write_pool())
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM tags WHERE name = 'yes'")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn preflight_rejects_a_file_that_is_not_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    std::fs::write(&path, "definitely not a database\n".repeat(200)).unwrap();
    let err = DatabaseManager::new(path.to_str().unwrap(), DbConfig::default())
        .await
        .err()
        .expect("a text file must not open");
    assert!(
        err.to_string().contains("(code: 26) file is not a database"),
        "{err}"
    );
    // Nothing was rewritten.
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .starts_with("definitely not a database"));
}

#[tokio::test]
async fn preflight_rejects_a_truncated_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    drop(open(&path).await);
    // Cut the file mid-page, as an interrupted copy would.
    let len = std::fs::metadata(&path).unwrap().len();
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(len - 100).unwrap();
    drop(file);
    let _ = std::fs::remove_file(dir.path().join("db.sqlite-wal"));
    let err = DatabaseManager::new(path.to_str().unwrap(), DbConfig::default())
        .await
        .err()
        .expect("a truncated file must not open");
    assert!(
        err.to_string()
            .contains("(code: 11) database disk image is malformed"),
        "{err}"
    );
}

#[tokio::test]
async fn existing_databases_keep_opening() {
    let dir = tempfile::tempdir().unwrap();

    // A schema-less WAL database, as left by a crash before the first
    // migration ran.
    let bare = dir.path().join("bare.sqlite");
    {
        let mut conn = SqliteConnectOptions::new()
            .filename(&bare)
            .create_if_missing(true)
            .connect()
            .await
            .unwrap();
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&mut conn)
            .await
            .unwrap();
        conn.close().await.unwrap();
    }
    drop(open(&bare).await);

    // A database in rollback-journal mode (another tool converted it).
    let rollback = dir.path().join("rollback.sqlite");
    {
        let mut conn = SqliteConnectOptions::new()
            .filename(&rollback)
            .create_if_missing(true)
            .connect()
            .await
            .unwrap();
        sqlx::query("CREATE TABLE keep_me(x)")
            .execute(&mut conn)
            .await
            .unwrap();
        sqlx::query("INSERT INTO keep_me VALUES (1)")
            .execute(&mut conn)
            .await
            .unwrap();
        conn.close().await.unwrap();
    }
    let db = open(&rollback).await;
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM keep_me")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(kept, 1);
    drop(db);

    // Reopening our own database right after dropping it.
    let ours = dir.path().join("db.sqlite");
    drop(open(&ours).await);
    drop(open(&ours).await);
}

#[tokio::test]
async fn ownership_sidecar_guards_against_another_process() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let db = open(&path).await;
    let sidecar = owner_file_path(&std::fs::canonicalize(&path).unwrap());
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&sidecar).unwrap_or_default()).unwrap_or_default();
    if !cfg!(windows) {
        // Windows keeps a locked file unreadable to other handles.
        assert_eq!(record["pid"], serde_json::json!(std::process::id()));
    }

    // Tools that only read a live database attach without claiming it.
    let attached = DatabaseManager::attach(path.to_str().unwrap(), DbConfig::default())
        .await
        .expect("attach next to the owner");
    let _: i64 = sqlx::query_scalar("SELECT count(*) FROM frames")
        .fetch_one(&attached.pool)
        .await
        .unwrap();
    drop(attached);
    drop(db);

    // A lock held through another handle stands in for another process.
    let foreign = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&sidecar)
        .unwrap();
    foreign.try_lock().unwrap();
    let started = Instant::now();
    let err = DatabaseManager::new(path.to_str().unwrap(), DbConfig::default())
        .await
        .err()
        .expect("owned elsewhere");
    assert!(is_ownership_error(&err), "{err}");
    assert!(
        err.to_string().contains("owned by another running process"),
        "{err}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));

    // Once that process is gone (its lock released), the file is taken over
    // at once, with no manual cleanup.
    foreign.unlock().unwrap();
    drop(foreign);
    drop(open(&path).await);
}

#[tokio::test]
async fn on_demand_checkpoint_uses_restart_and_restarts_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let db = open(&path).await;
    for i in 0..200 {
        sqlx::query("INSERT INTO tags (name) VALUES (?1)")
            .bind(format!("tag-{i}"))
            .execute(db.write_pool())
            .await
            .unwrap();
    }
    let (busy, log_pages, checkpointed) = db.wal_checkpoint().await.unwrap();
    assert_eq!(busy, 0);
    assert!(log_pages > 0);
    assert_eq!(log_pages, checkpointed);

    // RESTART: the next write starts the log from the beginning.
    sqlx::query("INSERT INTO tags (name) VALUES ('after')")
        .execute(db.write_pool())
        .await
        .unwrap();
    let (_, log_after, _) = db.wal_checkpoint().await.unwrap();
    assert!(log_after < log_pages, "{log_after} >= {log_pages}");
}

#[tokio::test]
async fn dropped_read_is_interrupted_and_the_connection_reused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let config = DbConfig {
        read_pool_max: 1,
        read_pool_min: 1,
        ..DbConfig::default()
    };
    let db = DatabaseManager::new(path.to_str().unwrap(), config)
        .await
        .unwrap();

    let long_query = async {
        let mut reader = db.cancellable_read().await?;
        sqlx::query("BEGIN").execute(reader.conn()).await?;
        let n: i64 = sqlx::query_scalar(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n",
        )
        .fetch_one(reader.conn())
        .await?;
        reader.finish();
        Ok::<_, sqlx::Error>(n)
    };
    assert!(
        tokio::time::timeout(Duration::from_millis(300), long_query)
            .await
            .is_err()
    );

    // The single read connection is back within a second, outside any
    // transaction, and serves the next query.
    let dropped_at = Instant::now();
    let mut conn = tokio::time::timeout(Duration::from_secs(1), db.pool.acquire())
        .await
        .expect("read connection returned within 1s")
        .unwrap();
    assert!(dropped_at.elapsed() < Duration::from_secs(1));
    let one: i64 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(one, 1);
    drop(conn);

    // The abandoned read transaction does not pin the WAL.
    sqlx::query("INSERT INTO tags (name) VALUES ('w')")
        .execute(db.write_pool())
        .await
        .unwrap();
    let (busy, log, done) = db.wal_checkpoint().await.unwrap();
    assert_eq!((busy, log), (0, done));

    // The same guard is what the heavy-read paths use.
    let mut reader = CancellableConnection::acquire(&db.pool).await.unwrap();
    let v: i64 = sqlx::query_scalar("SELECT 2")
        .fetch_one(reader.conn())
        .await
        .unwrap();
    reader.finish();
    assert_eq!(v, 2);
    let json = db
        .execute_raw_sql("SELECT count(*) AS c FROM tags")
        .await
        .unwrap();
    assert_eq!(json[0]["c"], serde_json::json!(1));
}

#[tokio::test]
async fn backup_and_wal_status_work_on_a_live_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let db = open(&path).await;
    let dest = dir.path().join("backup.sqlite");
    db.backup_to(dest.to_str().unwrap()).await.unwrap();
    assert!(dest.exists());
    let status = db.wal_maintenance_status();
    assert!(status.stopped_reason.is_none(), "{status:?}");
}
