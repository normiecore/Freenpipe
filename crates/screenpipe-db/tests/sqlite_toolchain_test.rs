// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Pins the SQLite toolchain that the storage layer relies on:
//!
//! * the linked library is SQLite 3.51.3 or newer (the release with the WAL
//!   reset fix that the startup guard will insist on);
//! * sqlx and rusqlite talk to the same SQLite build, so there is exactly one
//!   copy of the C library in the process;
//! * FTS5 accepts `contentless_delete=1`;
//! * the `sqlite_dbpage` virtual table, which the page-level recover extension
//!   reads from, is compiled in (set through `LIBSQLITE3_FLAGS` in
//!   `.cargo/config.toml`);
//! * rusqlite's `vtab` feature works against that shared build;
//! * the startup integrity check, which on this SQLite also checks FTS5
//!   indexes, does not report a healthy database as corrupt.

use std::os::raw::c_int;
use std::sync::Arc;

use rusqlite::vtab::{
    eponymous_only_module, sqlite3_vtab, sqlite3_vtab_cursor, Context, Filters, IndexInfo, VTab,
    VTabConnection, VTabCursor,
};
use screenpipe_config::DbConfig;
use screenpipe_db::{DatabaseManager, FrameWindowData};
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use sqlx::{ConnectOptions, Row};

/// Oldest SQLite release Handvel accepts, as (major, minor, patch).
const MIN_SQLITE: (u32, u32, u32) = (3, 51, 3);
/// The same floor in `SQLITE_VERSION_NUMBER` form.
const MIN_SQLITE_NUMBER: i32 = 3_051_003;

fn parse_version(text: &str) -> (u32, u32, u32) {
    let mut parts = text.trim().split('.').map(|p| {
        p.parse::<u32>()
            .unwrap_or_else(|_| panic!("unexpected sqlite_version() output: {text:?}"))
    });
    let major = parts.next().expect("major version");
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    (major, minor, patch)
}

async fn open_file_connection(dir: &tempfile::TempDir, name: &str) -> SqliteConnection {
    SqliteConnectOptions::new()
        .filename(dir.path().join(name))
        .create_if_missing(true)
        .connect()
        .await
        .expect("open scratch database")
}

#[test]
fn version_parser_orders_releases() {
    assert_eq!(parse_version("3.51.3"), (3, 51, 3));
    assert_eq!(parse_version("3.52"), (3, 52, 0));
    assert!(parse_version("3.51.2") < MIN_SQLITE);
    assert!(parse_version("3.51.3") >= MIN_SQLITE);
    assert!(parse_version("3.60.0") >= MIN_SQLITE);
}

#[tokio::test]
async fn sqlite_version_meets_floor_through_database_manager() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let db = DatabaseManager::new(path.to_str().unwrap(), DbConfig::default())
        .await
        .expect("open database");

    let version: String = sqlx::query_scalar("select sqlite_version()")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(
        parse_version(&version) >= MIN_SQLITE,
        "linked SQLite {version} is older than {}.{}.{}",
        MIN_SQLITE.0,
        MIN_SQLITE.1,
        MIN_SQLITE.2
    );

    // The C library reports the same thing without going through SQL.
    let number = unsafe { libsqlite3_sys::sqlite3_libversion_number() };
    assert!(
        number >= MIN_SQLITE_NUMBER,
        "sqlite3_libversion_number() = {number}"
    );
}

#[tokio::test]
async fn sqlx_and_rusqlite_share_one_sqlite_build() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_file_connection(&dir, "shared.sqlite").await;
    let via_sqlx: String = sqlx::query_scalar("select sqlite_source_id()")
        .fetch_one(&mut conn)
        .await
        .unwrap();

    let via_rusqlite: String = rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row("select sqlite_source_id()", [], |row| row.get(0))
        .unwrap();

    assert_eq!(via_sqlx, via_rusqlite);
    assert!(rusqlite::version_number() >= MIN_SQLITE_NUMBER);
}

#[tokio::test]
async fn fts5_contentless_delete_table_supports_delete() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_file_connection(&dir, "fts.sqlite").await;

    sqlx::query(
        "CREATE VIRTUAL TABLE probe_fts USING fts5(body, content='', contentless_delete=1)",
    )
    .execute(&mut conn)
    .await
    .expect("contentless_delete=1 should be accepted");

    for (rowid, body) in [(1_i64, "alpha beta"), (2, "beta gamma"), (3, "gamma delta")] {
        sqlx::query("INSERT INTO probe_fts(rowid, body) VALUES (?, ?)")
            .bind(rowid)
            .bind(body)
            .execute(&mut conn)
            .await
            .unwrap();
    }

    sqlx::query("DELETE FROM probe_fts WHERE rowid = 2")
        .execute(&mut conn)
        .await
        .expect("contentless_delete=1 tables accept DELETE");

    let beta: Vec<i64> = sqlx::query_scalar(
        "SELECT rowid FROM probe_fts WHERE probe_fts MATCH 'beta' ORDER BY rowid",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    assert_eq!(beta, vec![1]);

    let gamma: Vec<i64> = sqlx::query_scalar(
        "SELECT rowid FROM probe_fts WHERE probe_fts MATCH 'gamma' ORDER BY rowid",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    assert_eq!(gamma, vec![3]);

    // Without the option a contentless table still refuses DELETE, which shows
    // the behaviour above comes from contentless_delete and not from FTS5 in general.
    sqlx::query("CREATE VIRTUAL TABLE probe_plain USING fts5(body, content='')")
        .execute(&mut conn)
        .await
        .unwrap();
    sqlx::query("INSERT INTO probe_plain(rowid, body) VALUES (1, 'alpha')")
        .execute(&mut conn)
        .await
        .unwrap();
    assert!(sqlx::query("DELETE FROM probe_plain WHERE rowid = 1")
        .execute(&mut conn)
        .await
        .is_err());
}

#[tokio::test]
async fn recover_prerequisites_are_compiled_in() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_file_connection(&dir, "pages.sqlite").await;

    let options: Vec<String> = sqlx::query("PRAGMA compile_options")
        .fetch_all(&mut conn)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>(0))
        .collect();
    for wanted in ["ENABLE_DBPAGE_VTAB", "ENABLE_FTS5"] {
        assert!(
            options.iter().any(|o| o == wanted),
            "SQLite was built without {wanted}; compile options: {options:?}"
        );
    }

    sqlx::query("CREATE TABLE filler(id INTEGER PRIMARY KEY, payload BLOB)")
        .execute(&mut conn)
        .await
        .unwrap();
    for _ in 0..8 {
        sqlx::query("INSERT INTO filler(payload) VALUES (zeroblob(3000))")
            .execute(&mut conn)
            .await
            .unwrap();
    }

    let page_count: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    let dbpage_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_dbpage")
        .fetch_one(&mut conn)
        .await
        .expect("sqlite_dbpage virtual table should be available");
    assert!(page_count > 1);
    assert_eq!(dbpage_rows, page_count);

    let first_page: Vec<u8> = sqlx::query_scalar("SELECT data FROM sqlite_dbpage WHERE pgno = 1")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert!(first_page.starts_with(b"SQLite format 3\0"));
}

/// Records `count` one-window frames whose text contains "alpha", starting at
/// frame offset `first`. Each call to the batch insert is its own transaction,
/// so every frame adds a segment to `frames_fts` and FTS5 merges them as they
/// pile up.
async fn record_frames(db: &DatabaseManager, first: i64, count: i64) {
    let start = chrono::Utc::now() - chrono::Duration::hours(1);
    for offset in first..first + count {
        let windows = [FrameWindowData {
            app_name: Some(format!("App{}", offset % 5)),
            window_name: Some(format!("Window {offset}")),
            browser_url: None,
            focused: offset % 2 == 0,
            text: format!("alpha beta gamma {offset} {}", "filler words ".repeat(40)),
            text_json: String::new(),
        }];
        db.insert_frames_with_ocr_batch(
            "integrity_device",
            Some(start + chrono::Duration::seconds(offset)),
            offset,
            &windows,
        )
        .await
        .expect("insert frame");
    }
}

// Regression: a read-pool connection that ran an FTS5 query keeps that
// index's segment layout cached. After the writer merges segments, a
// quick_check on that connection (which since SQLite 3.44 also checks FTS5
// indexes) reported the healthy frames_fts as corrupt. The startup check
// therefore runs on its own connection and must say "ok" here.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn startup_integrity_check_passes_after_searches_and_fts_merges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let path = path.to_str().unwrap();
    let db = Arc::new(
        DatabaseManager::new(path, DbConfig::default())
            .await
            .expect("open database"),
    );
    db.insert_video_chunk("integrity.mp4", "integrity_device")
        .await
        .unwrap();
    record_frames(&db, 0, 100).await;

    // Concurrent searches spread the cached FTS5 layout over several
    // read-pool connections.
    let searches: Vec<_> = (0..8)
        .map(|_| {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM frames_fts WHERE frames_fts MATCH 'alpha'",
                )
                .fetch_one(&db.pool)
                .await
                .expect("search frames_fts")
            })
        })
        .collect();
    for search in searches {
        assert_eq!(search.await.unwrap(), 100);
    }

    // Recording continues; the new segments get merged into the old ones.
    record_frames(&db, 100, 100).await;

    let verdict = DatabaseManager::run_startup_integrity_check(path)
        .await
        .expect("integrity check runs");
    assert_eq!(verdict, "ok");

    // FTS5's own check agrees that the index is sound, so "ok" is the right
    // verdict and not a check that skipped the FTS tables.
    let mut conn = SqliteConnectOptions::new()
        .filename(path)
        .connect()
        .await
        .expect("open a separate connection");
    sqlx::query("INSERT INTO frames_fts(frames_fts) VALUES('integrity-check')")
        .execute(&mut conn)
        .await
        .expect("frames_fts integrity-check");
}

// A tiny eponymous virtual table, `probe_squares`, yielding (n, n*n) for
// n = 1..=limit. It exercises rusqlite's vtab support on the shared SQLite.

#[repr(C)]
struct SquaresTable {
    base: sqlite3_vtab,
    limit: i64,
}

#[repr(C)]
struct SquaresCursor {
    base: sqlite3_vtab_cursor,
    limit: i64,
    current: i64,
}

unsafe impl<'vtab> VTab<'vtab> for SquaresTable {
    type Aux = i64;
    type Cursor = SquaresCursor;

    fn connect(
        _db: &mut VTabConnection,
        aux: Option<&i64>,
        _args: &[&[u8]],
    ) -> rusqlite::Result<(String, Self)> {
        let table = SquaresTable {
            base: sqlite3_vtab::default(),
            limit: aux.copied().unwrap_or(0),
        };
        Ok((
            "CREATE TABLE x(n INTEGER, square INTEGER)".to_owned(),
            table,
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> rusqlite::Result<()> {
        info.set_estimated_cost(self.limit as f64);
        Ok(())
    }

    fn open(&'vtab mut self) -> rusqlite::Result<SquaresCursor> {
        Ok(SquaresCursor {
            base: sqlite3_vtab_cursor::default(),
            limit: self.limit,
            current: 1,
        })
    }
}

unsafe impl VTabCursor for SquaresCursor {
    fn filter(
        &mut self,
        _idx_num: c_int,
        _idx_str: Option<&str>,
        _args: &Filters<'_>,
    ) -> rusqlite::Result<()> {
        self.current = 1;
        Ok(())
    }

    fn next(&mut self) -> rusqlite::Result<()> {
        self.current += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        self.current > self.limit
    }

    fn column(&self, ctx: &mut Context, i: c_int) -> rusqlite::Result<()> {
        match i {
            0 => ctx.set_result(&self.current),
            _ => ctx.set_result(&(self.current * self.current)),
        }
    }

    fn rowid(&self) -> rusqlite::Result<i64> {
        Ok(self.current)
    }
}

#[test]
fn rusqlite_vtab_feature_runs_on_shared_sqlite() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.create_module(
        "probe_squares",
        eponymous_only_module::<SquaresTable>(),
        Some(5),
    )
    .unwrap();

    let (rows, total): (i64, i64) = conn
        .query_row(
            "SELECT count(*), sum(square) FROM probe_squares",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(rows, 5);
    assert_eq!(total, 1 + 4 + 9 + 16 + 25);

    let largest: i64 = conn
        .query_row("SELECT square FROM probe_squares WHERE n = 4", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(largest, 16);
}
