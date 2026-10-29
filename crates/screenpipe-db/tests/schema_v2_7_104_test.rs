// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! The v2.7.104 schema (W1-01): which migrations a database ends up with, what
//! they create, how a base-schema database with `ocr_text` rows moves forward,
//! how a database screenpipe already migrated is adopted, and where OCR output
//! is stored now that `ocr_text` is gone.
//!
//! Every test works on its own temporary database.

use std::collections::BTreeSet;
use std::path::Path;

use screenpipe_config::DbConfig;
use screenpipe_db::schema_migrations::{
    self, upstream_checksum, OCR_TEXT_RETIREMENT_VERSION, UPSTREAM_MIGRATIONS,
};
use screenpipe_db::{ContentType, DatabaseManager, FrameWindowData, SearchResult};
use sha2::{Digest, Sha384};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection, SqlitePoolOptions};
use sqlx::{ConnectOptions, Connection};

/// Every migration version a v2.7.104 database records, in order: the 89 of
/// the base schema plus the 28 that v2.7.104 adds (db.md, db-001).
#[rustfmt::skip]
const TARGET_VERSIONS: [i64; 117] = [
    20240703111257, 20240715121444, 20240716120000, 20240716120001, 20240716120002,
    20240716120003, 20240716120004, 20240805120005, 20240808120006, 20240808120007,
    20240808120008, 20240808120009, 20240808120010, 20240811010614, 20240812202755,
    20240812205501, 20240815201445, 20240815214646, 20240815215557, 20240821074222,
    20240917213422, 20241024181218, 20241103222151, 20241106031800, 20241108202826,
    20241127172038, 20241127220428, 20241204191424, 20241207202831, 20241210111055,
    20241210205142, 20241213220649, 20250117134500, 20250117180530, 20250130101751,
    20250131000000, 20250131232938, 20250202000000, 20250219212616, 20250223204844,
    20260126200000, 20260206200000, 20260207000000, 20260207100000, 20260209000000,
    20260209000001, 20260210000000, 20260210100000, 20260210200000, 20260213000000,
    20260218000000, 20260220000000, 20260221000000, 20260224000000, 20260225000000,
    20260301000000, 20260301100000, 20260301200000, 20260306000000, 20260309000000,
    20260310000000, 20260311000000, 20260312000000, 20260312000001, 20260312000002,
    20260315000000, 20260316000000, 20260317000000, 20260318000000, 20260319000000,
    20260320000000, 20260324000000, 20260326000000, 20260411000000, 20260415000000,
    20260423000000, 20260429000000, 20260501000000, 20260502000000, 20260503000000,
    20260504120000, 20260506120000, 20260507000000, 20260514000000, 20260515000000,
    20260520180000, 20260528090000, 20260601000000, 20260603000000, 20260603120000,
    20260612000000, 20260613000000, 20260613000001, 20260613130000, 20260616120000,
    20260618120000, 20260619000000, 20260619120000, 20260622000000, 20260627000000,
    20260702000000, 20260702130000, 20260709000000, 20260719000000, 20260724000000,
    20260727000000, 20260728000000, 20260729130000, 20260730180000, 20260805000000,
    20260812120000, 20260813000000, 20260817000000, 20260817160000, 20260818133000,
    20260828143000, 20261002190000,
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

async fn schema_names(conn: &mut SqliteConnection, kind: &str) -> BTreeSet<String> {
    sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = ?1")
        .bind(kind)
        .fetch_all(conn)
        .await
        .unwrap()
        .into_iter()
        .collect()
}

async fn column_names(conn: &mut SqliteConnection, table: &str) -> BTreeSet<String> {
    sqlx::query_scalar("SELECT name FROM pragma_table_info(?1)")
        .bind(table)
        .fetch_all(conn)
        .await
        .unwrap()
        .into_iter()
        .collect()
}

/// (version, description, success, checksum, installed_on, execution_time)
type MigrationRow = (i64, String, bool, Vec<u8>, String, i64);

async fn migration_rows(conn: &mut SqliteConnection) -> Vec<MigrationRow> {
    sqlx::query_as(
        "SELECT version, description, success, checksum, installed_on, execution_time \
         FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(conn)
    .await
    .unwrap()
}

async fn stored_checksum(conn: &mut SqliteConnection, version: i64) -> Vec<u8> {
    sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = ?1")
        .bind(version)
        .fetch_one(conn)
        .await
        .unwrap()
}

fn assert_contains_all(have: &BTreeSet<String>, want: &[&str], what: &str) {
    let missing: Vec<&&str> = want.iter().filter(|n| !have.contains(**n)).collect();
    assert!(missing.is_empty(), "missing {what}: {missing:?}");
}

#[tokio::test]
async fn fresh_database_applies_exactly_the_target_versions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    drop(open_manager(&path).await);

    let mut side = side_connection(&path).await;
    let rows = migration_rows(&mut side).await;
    side.close().await.unwrap();

    let versions: Vec<i64> = rows.iter().map(|r| r.0).collect();
    assert_eq!(versions, TARGET_VERSIONS.to_vec());
    assert!(rows.iter().all(|r| r.2), "every migration should succeed");

    // The 28 additions carry screenpipe's descriptions and checksums.
    let added: Vec<i64> = versions
        .iter()
        .copied()
        .filter(|v| upstream_checksum(*v).is_some())
        .collect();
    assert_eq!(added.len(), 28);
    for upstream in UPSTREAM_MIGRATIONS {
        let row = rows.iter().find(|r| r.0 == upstream.version).unwrap();
        assert_eq!(row.1, upstream.description, "{}", upstream.version);
        assert_eq!(
            Some(row.3.clone()),
            upstream_checksum(upstream.version),
            "{}",
            upstream.version
        );
    }
}

#[tokio::test]
async fn fresh_database_has_the_target_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    drop(open_manager(&path).await);
    let mut side = side_connection(&path).await;

    // The acceptance query from the plan: nothing left of ocr_text.
    let ocr_objects: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE name LIKE 'ocr_text%'")
            .fetch_all(&mut side)
            .await
            .unwrap();
    assert!(ocr_objects.is_empty(), "{ocr_objects:?}");

    let tables = schema_names(&mut side, "table").await;
    assert_contains_all(
        &tables,
        &[
            "outputs",
            "output_search_index",
            "output_search_fts",
            "display_layout",
            "semantic_runs",
            "semantic_items",
            "semantic_run_items",
            "semantic_items_fts",
            "semantic_actors",
            "semantic_actor_aliases",
            "semantic_item_actors",
            "pipe_event_runs",
            "feedback",
            "activity_tasks",
            "activity_intervals",
            "activity_actions",
            "activity_evidence",
            "activity_ledger_state",
            "starred_sessions",
        ],
        "tables",
    );
    assert!(!tables.contains("migration_progress"));

    let new_columns: &[(&str, &[&str])] = &[
        (
            "frames",
            &[
                "text_json",
                "full_text_redacted_at",
                "accessibility_tree_redacted_at",
                "window_name_redacted_at",
                "browser_url_redacted_at",
                "text_json_redacted_at",
                "semantic_run_id",
            ],
        ),
        ("elements", &["redacted_at"]),
        ("outputs", &["saf_kind", "artifact_id", "saf_version"]),
        ("ui_events", &["element_ancestors"]),
        (
            "pipe_executions",
            &["trigger_event", "trigger_key", "conversation_id"],
        ),
        ("meetings", &["calendar_event_id"]),
        (
            "meeting_transcript_segments",
            &["stream_id", "session_speaker_id"],
        ),
    ];
    for (table, columns) in new_columns {
        let have = column_names(&mut side, table).await;
        assert_contains_all(&have, columns, &format!("{table} columns"));
    }

    let indexes = schema_names(&mut side, "index").await;
    assert_contains_all(
        &indexes,
        &[
            "idx_outputs_source",
            "idx_outputs_source_type",
            "idx_outputs_created_at",
            "idx_outputs_output_path",
            "idx_outputs_source_type_original_path_id",
            "idx_elements_redacted_at",
            "idx_frames_accessibility_redacted_at",
            "idx_frames_full_text_redacted_at",
            "idx_frames_accessibility_tree_redacted_at",
            "idx_frames_window_name_redacted_at",
            "idx_frames_browser_url_redacted_at",
            "idx_frames_text_json_redacted_at",
            "idx_speakers_hallucination_name",
            "idx_tags_name_nocase",
            "idx_frames_app_name_timestamp",
            "idx_frames_snapshot_pending",
            "idx_meeting_transcript_segments_captured_at_julianday",
            "idx_display_layout_timestamp",
            "idx_semantic_runs_parser",
            "idx_semantic_items_entity",
            "idx_semantic_items_kind_time",
            "idx_semantic_run_items_item",
            "idx_frames_semantic_run_id",
            "idx_semantic_actors_name",
            "idx_semantic_actor_aliases_actor",
            "idx_semantic_actor_aliases_observed_label",
            "idx_semantic_item_actors_actor",
            "idx_pipe_event_runs_claimed_at",
            "idx_feedback_target",
            "idx_feedback_producer_updated",
            "idx_feedback_rating_updated",
            "idx_meetings_calendar_event_id",
            "idx_activity_tasks_parent",
            "idx_activity_tasks_app",
            "idx_activity_intervals_range",
            "idx_activity_intervals_task",
            "idx_activity_intervals_producer_range",
            "idx_activity_actions_interval",
            "idx_activity_actions_source",
            "idx_activity_evidence_interval",
            "idx_activity_evidence_source",
            "idx_meeting_transcript_segments_session_speaker",
            "idx_starred_sessions_start_end",
        ],
        "indexes",
    );

    // The redaction watermark indexes only cover the backlog (db-004).
    for column in [
        "accessibility_redacted_at",
        "full_text_redacted_at",
        "accessibility_tree_redacted_at",
        "window_name_redacted_at",
        "browser_url_redacted_at",
        "text_json_redacted_at",
    ] {
        let sql: String = sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE name = ?1")
            .bind(format!("idx_frames_{column}"))
            .fetch_one(&mut side)
            .await
            .unwrap();
        let normalized = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            normalized.ends_with(&format!("WHERE {column} IS NULL")),
            "{column}: {normalized}"
        );
    }

    let triggers = schema_names(&mut side, "trigger").await;
    assert_contains_all(
        &triggers,
        &[
            "semantic_items_ai",
            "semantic_items_ad",
            "semantic_items_au",
            "activity_frames_ad",
            "activity_ui_events_ad",
            "activity_audio_ad",
            "activity_evidence_ad",
            "activity_intervals_ad",
            "activity_tasks_ad",
        ],
        "triggers",
    );
    side.close().await.unwrap();
}

/// Register sqlite-vec for every new SQLite connection in this process, as
/// `DatabaseManager::new` does. The raw pool below runs base migrations that
/// call `vec_length`, so it must not depend on another test having opened a
/// `DatabaseManager` first.
fn register_sqlite_vec() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        type ExtensionInit = unsafe extern "C" fn(
            *mut libsqlite3_sys::sqlite3,
            *mut *mut std::os::raw::c_char,
            *const libsqlite3_sys::sqlite3_api_routines,
        ) -> std::os::raw::c_int;
        libsqlite3_sys::sqlite3_auto_extension(Some(
            std::mem::transmute::<*const (), ExtensionInit>(
                sqlite_vec::sqlite3_vec_init as *const (),
            ),
        ));
    });
}

/// A database at the base schema (upstream 892199f): the 89 base migrations
/// with the checksums the base recorded.
async fn create_base_schema_database(path: &Path) {
    register_sqlite_vec();
    let base: Vec<_> = schema_migrations::migrator()
        .iter()
        .filter(|m| upstream_checksum(m.version).is_none())
        .cloned()
        .collect();
    assert_eq!(base.len(), 89);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    Migrator::with_migrations(base).run(&pool).await.unwrap();

    // Frames the way the base wrote them, each paired with ocr_text rows that
    // exercise one rule of the fold.
    sqlx::raw_sql(
        r#"
        INSERT INTO video_chunks (id, file_path, device_name)
            VALUES (1, '/tmp/fixture/monitor_1.mp4', 'monitor_1');

        -- 1: legacy frame, every value only on the ocr_text row
        INSERT INTO frames (id, video_chunk_id, offset_index, timestamp, device_name)
            VALUES (1, 1, 0, '2026-05-01T10:00:00Z', 'monitor_1');
        INSERT INTO ocr_text (frame_id, text, text_json, app_name, ocr_engine, window_name, focused, redacted_at)
            VALUES (1, 'legacy invoice 4711', '[{"text":"legacy"}]', 'Ledger', 'Tesseract', 'Invoices', 1, 123);

        -- 2: frame that already has its own values; only text_json is new
        INSERT INTO frames (id, video_chunk_id, offset_index, timestamp, device_name,
                            app_name, window_name, focused, full_text)
            VALUES (2, 1, 1, '2026-05-01T10:00:01Z', 'monitor_1',
                    'Editor', 'Draft', 0, 'kept accessibility text');
        INSERT INTO ocr_text (frame_id, text, text_json, app_name, ocr_engine, window_name, focused)
            VALUES (2, 'ocr words', '[{"text":"ocr"}]', 'Other', 'Tesseract', 'Other window', 1);

        -- 3: empty strings; app_name and full_text count as missing, window_name does not
        INSERT INTO frames (id, video_chunk_id, offset_index, timestamp, device_name,
                            app_name, window_name, full_text)
            VALUES (3, 1, 2, '2026-05-01T10:00:02Z', 'monitor_1', '', '', '');
        INSERT INTO ocr_text (frame_id, text, text_json, app_name, ocr_engine, window_name, focused)
            VALUES (3, 'terminal output', '', 'Terminal', 'Tesseract', 'zsh', 0);

        -- 4: ocr_text row with nothing in it
        INSERT INTO frames (id, video_chunk_id, offset_index, timestamp, device_name)
            VALUES (4, 1, 3, '2026-05-01T10:00:03Z', 'monitor_1');
        INSERT INTO ocr_text (frame_id, text, text_json, app_name, ocr_engine)
            VALUES (4, '', '', '', 'Tesseract');

        -- 5: no ocr_text row at all
        INSERT INTO frames (id, video_chunk_id, offset_index, timestamp, device_name, app_name)
            VALUES (5, 1, 4, '2026-05-01T10:00:04Z', 'monitor_1', 'Browser');

        -- 6: two ocr_text rows, the first one empty
        INSERT INTO frames (id, video_chunk_id, offset_index, timestamp, device_name)
            VALUES (6, 1, 5, '2026-05-01T10:00:05Z', 'monitor_1');
        INSERT INTO ocr_text (frame_id, text, text_json, app_name, ocr_engine)
            VALUES (6, '', '', '', 'Tesseract');
        INSERT INTO ocr_text (frame_id, text, text_json, app_name, ocr_engine)
            VALUES (6, 'second row wins', '[{"text":"second"}]', 'Notes', 'Tesseract');
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
}

/// (full_text, text_json, app_name, window_name, focused, full_text_redacted_at)
type FrameValues = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<bool>,
    Option<i64>,
);

async fn frame_values(conn: &mut SqliteConnection, id: i64) -> FrameValues {
    sqlx::query_as(
        "SELECT full_text, text_json, app_name, window_name, focused, full_text_redacted_at \
         FROM frames WHERE id = ?1",
    )
    .bind(id)
    .fetch_one(conn)
    .await
    .unwrap()
}

fn s(value: &str) -> Option<String> {
    Some(value.to_owned())
}

#[tokio::test]
async fn base_schema_database_folds_ocr_text_into_frames() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    create_base_schema_database(&path).await;

    let db = open_manager(&path).await;
    let mut side = side_connection(&path).await;

    let versions: Vec<i64> = migration_rows(&mut side)
        .await
        .into_iter()
        .map(|r| r.0)
        .collect();
    assert_eq!(
        versions,
        TARGET_VERSIONS.to_vec(),
        "all 28 additions applied, including the one older than the base's newest"
    );

    assert_eq!(
        frame_values(&mut side, 1).await,
        (
            s("legacy invoice 4711"),
            s(r#"[{"text":"legacy"}]"#),
            s("Ledger"),
            s("Invoices"),
            Some(true),
            None
        )
    );
    assert_eq!(
        frame_values(&mut side, 2).await,
        (
            s("kept accessibility text"),
            s(r#"[{"text":"ocr"}]"#),
            s("Editor"),
            s("Draft"),
            Some(false),
            None
        )
    );
    assert_eq!(
        frame_values(&mut side, 3).await,
        (
            s("terminal output"),
            None,
            s("Terminal"),
            s(""),
            Some(false),
            None
        )
    );
    // ocr_text.focused defaults to FALSE, so even an otherwise empty row
    // fills a missing focused flag.
    assert_eq!(
        frame_values(&mut side, 4).await,
        (None, None, None, None, Some(false), None)
    );
    assert_eq!(
        frame_values(&mut side, 5).await,
        (None, None, s("Browser"), None, None, None)
    );
    assert_eq!(
        frame_values(&mut side, 6).await,
        (
            s("second row wins"),
            s(r#"[{"text":"second"}]"#),
            s("Notes"),
            None,
            Some(false),
            None
        )
    );

    let ocr_objects: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name LIKE 'ocr_text%'")
            .fetch_one(&mut side)
            .await
            .unwrap();
    assert_eq!(ocr_objects, 0);

    // The folded text is indexed through the frames_au trigger.
    let hits: Vec<i64> =
        sqlx::query_scalar("SELECT rowid FROM frames_fts WHERE frames_fts MATCH 'invoice'")
            .fetch_all(&mut side)
            .await
            .unwrap();
    assert_eq!(hits, vec![1]);
    side.close().await.unwrap();

    // ... and search finds it, with no OCR engine to report.
    let results = db
        .search(
            "invoice",
            ContentType::OCR,
            10,
            0,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let ocr: Vec<_> = results
        .iter()
        .filter_map(|r| match r {
            SearchResult::OCR(ocr) => Some(ocr),
            _ => None,
        })
        .collect();
    assert_eq!(ocr.len(), 1, "{results:?}");
    assert_eq!(ocr[0].frame_id, 1);
    assert_eq!(ocr[0].ocr_text, "legacy invoice 4711");
    assert_eq!(ocr[0].text_json, r#"[{"text":"legacy"}]"#);
    assert_eq!(ocr[0].app_name, "Ledger");
    assert_eq!(ocr[0].ocr_engine, "");
}

#[tokio::test]
async fn database_screenpipe_migrated_is_adopted_without_rerunning_anything() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    // A database migrated to v2.7.104 records screenpipe's checksum for every
    // version, which is what this build stores too.
    drop(open_manager(&path).await);

    let mut side = side_connection(&path).await;
    let before = migration_rows(&mut side).await;
    for upstream in UPSTREAM_MIGRATIONS {
        assert_eq!(
            stored_checksum(&mut side, upstream.version).await,
            upstream_checksum(upstream.version).unwrap()
        );
    }
    // screenpipe releases after v2.7.104 may have added versions of their own.
    const NEWER_SCREENPIPE_VERSION: i64 = 20261101000000;
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (?1, 'from a newer screenpipe', 1, X'00', 0)",
    )
    .bind(NEWER_SCREENPIPE_VERSION)
    .execute(&mut side)
    .await
    .unwrap();

    // Re-running any of the 28 would fail (the ALTER TABLE statements are not
    // repeatable), so opening at all shows nothing was re-applied; the
    // bookkeeping rows prove nothing was rewritten either.
    drop(open_manager(&path).await);
    let after: Vec<MigrationRow> = migration_rows(&mut side)
        .await
        .into_iter()
        .filter(|r| r.0 != NEWER_SCREENPIPE_VERSION)
        .collect();
    assert_eq!(after, before);
    side.close().await.unwrap();
}

#[tokio::test]
async fn checksum_of_our_own_migration_text_is_repaired_to_screenpipes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    drop(open_manager(&path).await);

    // A build that recorded the hash of Handvel's own file text instead of
    // the pinned upstream checksum.
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/migrations/20260613130000_unify_ocr_text_into_frames.sql");
    let own = Sha384::digest(std::fs::read(file).unwrap()).to_vec();
    let upstream = upstream_checksum(OCR_TEXT_RETIREMENT_VERSION).unwrap();
    assert_ne!(own, upstream);

    let mut side = side_connection(&path).await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum = ?1 WHERE version = ?2")
        .bind(&own)
        .bind(OCR_TEXT_RETIREMENT_VERSION)
        .execute(&mut side)
        .await
        .unwrap();

    drop(open_manager(&path).await);
    assert_eq!(
        stored_checksum(&mut side, OCR_TEXT_RETIREMENT_VERSION).await,
        upstream
    );
    side.close().await.unwrap();
}

fn ocr_boxes(word: &str) -> String {
    format!(
        r#"[{{"block_num":"1","conf":"96.5","page_num":"1","left":"10","height":"12","level":"5","text":"{word}","par_num":"1","top":"20","word_num":"1","width":"40","line_num":"1"}}]"#
    )
}

#[tokio::test]
async fn ocr_output_is_stored_on_the_frame_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let db = open_manager(&path).await;
    db.insert_video_chunk("/tmp/fixture/monitor_2.mp4", "monitor_2")
        .await
        .unwrap();

    // Batch capture path: text into full_text, boxes into text_json.
    let windows = [
        FrameWindowData {
            app_name: Some("Terminal".into()),
            window_name: Some("build".into()),
            browser_url: None,
            focused: true,
            text: "cargo build finished".into(),
            text_json: ocr_boxes("cargo"),
        },
        FrameWindowData {
            app_name: Some("Viewer".into()),
            window_name: Some("blank".into()),
            browser_url: None,
            focused: false,
            text: String::new(),
            text_json: "[]".into(),
        },
    ];
    let inserted = db
        .insert_frames_with_ocr_batch("monitor_2", None, 0, &windows)
        .await
        .unwrap();
    assert_eq!(inserted.len(), 2);
    let (with_text, blank) = (inserted[0].0, inserted[1].0);

    // Late OCR on frames that already exist.
    let late = db
        .insert_frame(
            "monitor_2",
            None,
            None,
            Some("Game"),
            Some("menu"),
            true,
            Some(1),
        )
        .await
        .unwrap();
    db.insert_ocr_text(late, "press start", &ocr_boxes("press"))
        .await
        .unwrap();
    let has_text = db
        .insert_frame(
            "monitor_2",
            None,
            None,
            Some("Docs"),
            Some("page"),
            true,
            Some(2),
        )
        .await
        .unwrap();
    let mut side = side_connection(&path).await;
    sqlx::query("UPDATE frames SET full_text = 'text from the accessibility tree' WHERE id = ?1")
        .bind(has_text)
        .execute(&mut side)
        .await
        .unwrap();
    db.insert_ocr_text(has_text, "ocr disagrees", &ocr_boxes("ocr"))
        .await
        .unwrap();
    db.insert_ocr_text(blank, "", "[]").await.unwrap();

    let stored = |id: i64| {
        sqlx::query_as::<_, (Option<String>, Option<String>)>(
            "SELECT full_text, text_json FROM frames WHERE id = ?1",
        )
        .bind(id)
    };
    assert_eq!(
        stored(with_text).fetch_one(&mut side).await.unwrap(),
        (s("cargo build finished"), Some(ocr_boxes("cargo")))
    );
    assert_eq!(
        stored(blank).fetch_one(&mut side).await.unwrap(),
        (None, None),
        "an empty OCR result stores nothing"
    );
    assert_eq!(
        stored(late).fetch_one(&mut side).await.unwrap(),
        (s("press start"), Some(ocr_boxes("press")))
    );
    assert_eq!(
        stored(has_text).fetch_one(&mut side).await.unwrap(),
        (
            s("text from the accessibility tree"),
            Some(ocr_boxes("ocr"))
        ),
        "existing text is kept; the boxes are still recorded"
    );
    side.close().await.unwrap();

    assert_eq!(
        db.get_frame_ocr_text_json(late).await.unwrap(),
        Some(ocr_boxes("press"))
    );
    assert_eq!(db.get_frame_ocr_text_json(blank).await.unwrap(), None);

    let results = db
        .search(
            "cargo",
            ContentType::OCR,
            10,
            0,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let ocr: Vec<_> = results
        .iter()
        .filter_map(|r| match r {
            SearchResult::OCR(ocr) => Some(ocr),
            _ => None,
        })
        .collect();
    assert_eq!(ocr.len(), 1, "{results:?}");
    assert_eq!(ocr[0].frame_id, with_text);
    assert_eq!(ocr[0].text_json, ocr_boxes("cargo"));
    assert_eq!(ocr[0].ocr_engine, "");
}

async fn query_plan(conn: &mut SqliteConnection, sql: &'static str) -> String {
    let rows: Vec<(i64, i64, i64, String)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
            .fetch_all(conn)
            .await
            .unwrap();
    rows.into_iter()
        .map(|(_, _, _, detail)| detail)
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn performance_indexes_serve_the_queries_they_were_added_for() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    drop(open_manager(&path).await);
    let mut side = side_connection(&path).await;

    // The meeting timeline filters and orders on julianday(captured_at)
    // exactly as the expression index spells it.
    let plan = query_plan(
        &mut side,
        "SELECT mts.id FROM meeting_transcript_segments mts \
         WHERE julianday(mts.captured_at) >= julianday('2026-01-01') \
           AND julianday(mts.captured_at) <= julianday('2026-01-02') \
         ORDER BY julianday(mts.captured_at) DESC",
    )
    .await;
    assert!(
        plan.contains("idx_meeting_transcript_segments_captured_at_julianday"),
        "{plan}"
    );

    // Snapshot compaction scans frames that still hold an uncompacted image.
    let plan = query_plan(
        &mut side,
        "SELECT id FROM frames \
         WHERE snapshot_path IS NOT NULL AND timestamp < '2026-01-01' \
         ORDER BY device_name, timestamp LIMIT 5000",
    )
    .await;
    assert!(plan.contains("idx_frames_snapshot_pending"), "{plan}");
    side.close().await.unwrap();
}
