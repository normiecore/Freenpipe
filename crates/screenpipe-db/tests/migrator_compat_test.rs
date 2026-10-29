// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Migration-runner behaviour the schema plan depends on, re-checked on the
//! sqlx 0.9 toolchain:
//!
//! * a pending migration whose version is older than the newest applied one is
//!   still applied (the v2.7 schema adds versions that sort before the last two
//!   base migrations);
//! * `set_ignore_missing(true)` lets a database that a newer build migrated
//!   further still open;
//! * the checksum-mismatch error text that `fix_migration_checksums` keys on is
//!   unchanged, and the repair path still runs;
//! * the `_sqlx_migrations` bookkeeping table keeps the layout earlier sqlx
//!   releases created, and stored checksums are still the SHA-384 of each
//!   base migration file, so a database migrated by the sqlx 0.7 base opens
//!   without any checksum repair. The v2.7.104 versions store screenpipe's
//!   checksum instead (see `schema_migrations`), so a database screenpipe
//!   migrated opens without one either.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use screenpipe_config::DbConfig;
use screenpipe_db::schema_migrations::upstream_checksum;
use screenpipe_db::DatabaseManager;
use sha2::{Digest, Sha384};
use sqlx::migrate::{MigrateError, Migrator};
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection, SqlitePoolOptions};
use sqlx::{ConnectOptions, Connection};

/// `CREATE INDEX IF NOT EXISTS` + `ANALYZE` only, so re-running it is harmless.
const IDEMPOTENT_VERSION: i64 = 20260423000000;
const IDEMPOTENT_INDEXES: [&str; 2] = [
    "idx_speaker_embeddings_speaker_id",
    "idx_audio_transcriptions_speaker_id_timestamp",
];

async fn open_manager(path: &Path) -> DatabaseManager {
    DatabaseManager::new(path.to_str().unwrap(), DbConfig::default())
        .await
        .expect("DatabaseManager::new")
}

async fn side_connection(path: &Path) -> SqliteConnection {
    SqliteConnectOptions::new()
        .filename(path)
        .busy_timeout(std::time::Duration::from_secs(10))
        .connect()
        .await
        .expect("open side connection")
}

async fn applied_versions(conn: &mut SqliteConnection) -> Vec<i64> {
    sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
        .fetch_all(conn)
        .await
        .unwrap()
}

async fn index_exists(conn: &mut SqliteConnection, name: &str) -> bool {
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = ?")
            .bind(name)
            .fetch_one(conn)
            .await
            .unwrap();
    n == 1
}

#[test]
fn checksum_mismatch_text_still_matches_repair_trigger() {
    // db.rs repairs stored checksums when the migrator error contains this text.
    let message = MigrateError::VersionMismatch(IDEMPOTENT_VERSION).to_string();
    assert!(
        message.contains("was previously applied but has been modified"),
        "sqlx changed its checksum-mismatch message: {message}"
    );
}

#[tokio::test]
async fn embedded_migrator_applies_pending_version_older_than_newest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    drop(open_manager(&path).await);

    let mut side = side_connection(&path).await;
    let before = applied_versions(&mut side).await;
    let newest = *before.last().expect("migrations applied");
    assert!(before.contains(&IDEMPOTENT_VERSION));
    assert!(IDEMPOTENT_VERSION < newest);

    // Make IDEMPOTENT_VERSION pending again while every newer version stays applied.
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ?")
        .bind(IDEMPOTENT_VERSION)
        .execute(&mut side)
        .await
        .unwrap();
    for index in IDEMPOTENT_INDEXES {
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP INDEX {index}")))
            .execute(&mut side)
            .await
            .unwrap();
        assert!(!index_exists(&mut side, index).await);
    }

    let _db = open_manager(&path).await;

    let after = applied_versions(&mut side).await;
    assert_eq!(
        after, before,
        "the out-of-order version should be re-applied"
    );
    for index in IDEMPOTENT_INDEXES {
        assert!(
            index_exists(&mut side, index).await,
            "{index} not recreated"
        );
    }
    let success: bool =
        sqlx::query_scalar("SELECT success FROM _sqlx_migrations WHERE version = ?")
            .bind(IDEMPOTENT_VERSION)
            .fetch_one(&mut side)
            .await
            .unwrap();
    assert!(success);
    side.close().await.unwrap();
}

#[tokio::test]
async fn embedded_migrator_ignores_versions_from_a_newer_build() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    drop(open_manager(&path).await);

    const FUTURE_VERSION: i64 = 99991231000000;
    let mut side = side_connection(&path).await;
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (?, 'applied by a newer build', 1, X'00', 0)",
    )
    .bind(FUTURE_VERSION)
    .execute(&mut side)
    .await
    .unwrap();

    // Without set_ignore_missing(true) this would fail with VersionMissing.
    let _db = open_manager(&path).await;

    assert!(applied_versions(&mut side).await.contains(&FUTURE_VERSION));
    side.close().await.unwrap();
}

#[tokio::test]
async fn embedded_migrator_repairs_modified_checksum() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    drop(open_manager(&path).await);

    let mut side = side_connection(&path).await;
    let original: Vec<u8> =
        sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = ?")
            .bind(IDEMPOTENT_VERSION)
            .fetch_one(&mut side)
            .await
            .unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum = X'DEADBEEF' WHERE version = ?")
        .bind(IDEMPOTENT_VERSION)
        .execute(&mut side)
        .await
        .unwrap();

    let _db = open_manager(&path).await;

    let repaired: Vec<u8> =
        sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = ?")
            .bind(IDEMPOTENT_VERSION)
            .fetch_one(&mut side)
            .await
            .unwrap();
    assert_eq!(repaired, original);
    side.close().await.unwrap();
}

#[tokio::test]
async fn migrations_table_keeps_its_layout() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    drop(open_manager(&path).await);

    let mut side = side_connection(&path).await;
    let columns: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT name, type, \"notnull\", pk FROM pragma_table_info('_sqlx_migrations') ORDER BY cid",
    )
    .fetch_all(&mut side)
    .await
    .unwrap();
    side.close().await.unwrap();

    // (name, declared type, NOT NULL, primary-key position), as sqlx 0.7 created it.
    let expected: Vec<(String, String, i64, i64)> = [
        ("version", "BIGINT", 0, 1),
        ("description", "TEXT", 1, 0),
        ("installed_on", "TIMESTAMP", 1, 0),
        ("success", "BOOLEAN", 1, 0),
        ("checksum", "BLOB", 1, 0),
        ("execution_time", "BIGINT", 1, 0),
    ]
    .into_iter()
    .map(|(name, ty, not_null, pk)| (name.to_owned(), ty.to_owned(), not_null, pk))
    .collect();
    assert_eq!(columns, expected);
}

/// Embedded migration files by version, read from the source tree.
fn migration_files() -> BTreeMap<i64, PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/migrations");
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(&dir).expect("read migrations dir") {
        let path = entry.unwrap().path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".sql") {
            continue;
        }
        let version: i64 = name
            .split('_')
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("migration file without a numeric version: {name}"));
        assert!(
            files.insert(version, path.clone()).is_none(),
            "two migration files share version {version}"
        );
    }
    files
}

#[tokio::test]
async fn stored_checksums_match_files_or_pinned_upstream_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    drop(open_manager(&path).await);

    let mut side = side_connection(&path).await;
    let stored: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&mut side)
            .await
            .unwrap();
    side.close().await.unwrap();

    let files = migration_files();
    assert_eq!(
        stored.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
        files.keys().copied().collect::<Vec<_>>(),
        "every migration file should be applied exactly once"
    );
    let mut pinned = 0;
    for (version, checksum) in stored {
        if let Some(upstream) = upstream_checksum(version) {
            pinned += 1;
            assert_eq!(
                checksum, upstream,
                "{version} should record screenpipe's checksum so screenpipe \
                 accepts this database without a repair"
            );
            continue;
        }
        let text = std::fs::read_to_string(&files[&version]).unwrap();
        assert_eq!(
            checksum,
            Sha384::digest(text.as_bytes()).to_vec(),
            "checksum of {version} is not the SHA-384 of its file; databases \
             migrated by an earlier sqlx would need a checksum repair"
        );
    }
    assert_eq!(
        pinned, 28,
        "every v2.7.104 version carries a pinned checksum"
    );
}

/// The same guarantees, shown on the bare sqlx `Migrator` with migrations read
/// from a directory at runtime, independent of the embedded migration set.
#[tokio::test]
async fn runtime_migrator_out_of_order_and_ignore_missing() {
    let migrations = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let write = |name: &str, sql: &str| std::fs::write(migrations.path().join(name), sql).unwrap();

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(data.path().join("runtime.sqlite"))
                .create_if_missing(true),
        )
        .await
        .unwrap();

    write(
        "1_create_first.sql",
        "CREATE TABLE first_table (id INTEGER);",
    );
    write(
        "3_create_third.sql",
        "CREATE TABLE third_table (id INTEGER);",
    );
    Migrator::new(migrations.path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();

    // Version 2 arrives after 3 was applied.
    write(
        "2_create_second.sql",
        "CREATE TABLE second_table (id INTEGER);",
    );
    Migrator::new(migrations.path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .expect("an older pending version is applied, not rejected");
    let tables: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'second_table'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(tables, 1);

    // Version 1 disappears from the source (as a newer build's versions do
    // from the point of view of an older one).
    std::fs::remove_file(migrations.path().join("1_create_first.sql")).unwrap();
    let mut migrator = Migrator::new(migrations.path()).await.unwrap();
    match migrator.run(&pool).await {
        Err(MigrateError::VersionMissing(1)) => {}
        other => panic!("expected VersionMissing(1), got {other:?}"),
    }
    migrator.set_ignore_missing(true);
    migrator
        .run(&pool)
        .await
        .expect("set_ignore_missing(true) accepts unknown applied versions");

    pool.close().await;
}
