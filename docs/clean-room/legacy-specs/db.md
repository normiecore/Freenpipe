# Freenpipe RE spec — Database schema, migrations and storage layout (dimension `db`)

Target: screenpipe v2.7.104 Windows build (`screenpipe-app.exe`, static evidence only).
Base: Freenpipe @ upstream 892199f (`crates/screenpipe-db`, 89 sqlx migrations).

Clean-room note: SQL DDL below is recorded verbatim for schema compatibility (interop
identifiers). The embedded migration files in the target carry long explanatory comments;
those are NOT reproduced here, only paraphrased. Do not copy comment prose into Freenpipe
migrations; write your own.

## 0. Evidence and method

* `app_strings.tsv` hex offsets are file offsets into the exe (`<re-workspace>/app_strings.tsv`).
* Helper scripts written for this dimension (all in `<re-workspace>/tools/`):
  * `region_dump.py` — dumps a byte range of the exe as text, keeping newlines (multi-line SQL survives).
  * `mig_match.py` — locates every base migration in the exe by SHA-384 digest, SQL text and LE-i64 version.
  * `mig_array.py` — walks the sqlx `Migration` static array (stride 0x58, starts at file offset `0xa2ae3a8`, version at +0x48, flags at +0x50) and dumps each embedded SQL body to `<re-workspace>/work/target_migrations/`.
  * `src_paths.py`, `app_src_paths.py`, `region_strings_diff.py` — module inventory and cross-copy string diffs.
* Working dumps: `<re-workspace>/work/` (`migrations_region.txt`, `hybrid_region.txt`, `db_region_a/b/c.txt`, `mig_array.txt`, `serde_structs.txt`, `settings_migration_ctx.txt`).
* The db crate's strings appear in several copies (e.g. `0x4980000..`, `0x49d9000..0x49ef000`, `0x5140000..`, `0x51a4000..0x51b3000`, `0x5240000..`, `0x52d3000..`, `0xa274000..0xa2cb000`); copies are near-identical, offsets below cite one.
* Target source module inventory (from panic paths) for `screenpipe-db`: `cancellable_query.rs`, `db/{mod,setup,maintenance,frames,search,elements,audio,meetings,memories,speakers,tags,outputs,feedback,semantic,starred,display_layout,activity_ledger,accessibility,write_ops,text_positions}.rs`, `planner_stats.rs`, `recovery.rs`, `write_queue.rs`, `storage/{mod,lifecycle,in_place,schema,read_schema,reader,snapshot,sealing,reclaim,maintenance,import,parity,diagnostics,backup,bindings,codec,sql}.rs`, `storage/bulk/{mod,schema,codec,cache,connection,lifecycle,elements,element_index}.rs`, `storage/bulk/elements/{lifecycle,vtab}.rs`. New crates: `screenpipe-sqlite-coordinator` (`lib.rs`, `quarantine.rs`, `verification.rs`), `screenpipe-sqlite-recovery` (`lib.rs`). App-level: `storage_migration.rs`, `db_relaunch.rs`, `db_self_heal.rs`, `db_recovery_notifications.rs`, `search_only.rs`, `recording/db_wedge.rs`, `recording/recovery_log.rs`, `disk_usage.rs`.
* Dependency deltas (`crates.txt`): `sqlx-core/sqlx-sqlite 0.9.0` (base 0.7.2), `libsqlite3-sys 0.37.0` (base 0.26.0), `rusqlite 0.39.0` (base 0.29.0, now used for virtual tables), `parquet 59.3.0` (new), `zstd-safe 7.3.0`, `sha2 0.10.9`. Bundled SQLite source id dated `2026-03-13` (`0xa32f090`).
* REA: parent JS-application evidence `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`; settings-module trace `ev_41e05f198918913c013943637857329ba6946afd02f6a403aa363c58e99412df`. API/string traces for `get_storage_migration_status` / `storage_migration` returned no-match (`ev_35274a14896383306630eef6098f3dcf360e9cfe6967ae6468588aa86d4e3cad`, `ev_556a969a963724184fa90048d6c626c853d9a407a6df72dc96da208d0b5f13f9`), so frontend evidence below is cited by file path.

## 1. Base vs target inventory

| Area | Base (892199f) | Target (v2.7.104) | Delta id |
|---|---|---|---|
| sqlx migrations | 89 (last `20260603120000`) | 117 (last `20261002190000`); all 89 base migrations embedded byte-identical (same SHA-384) | db-001 |
| Migration runner | `set_ignore_missing(true)`; on sqlx's `previously applied but has been modified` error (exact string, client-visible) rewrite `_sqlx_migrations.checksum` and retry | same behaviour (`0x49dadb0`, `0x49db6d0`), plus pre-migrate notices for frames_fts and ocr_text retirement (`0x49dbe02`) | db-001 |
| `outputs`, `output_search_index`, `output_search_fts` | absent | new | db-002 |
| `elements.redacted_at` | absent | new + index | db-003 |
| frames redaction watermarks | `accessibility_redacted_at`, `image_redacted_at` | + `full_text_redacted_at`, `accessibility_tree_redacted_at`, `window_name_redacted_at`, `browser_url_redacted_at`, `text_json_redacted_at`; all watermark indexes partial | db-004 |
| `ocr_text` table | present, written per frame, read by search/timeline/sync/redact | DROPPED; `frames.text_json` added; backfills | db-005 |
| Perf indexes | — | `idx_speakers_hallucination_name`, `idx_tags_name_nocase`, `idx_frames_app_name_timestamp`, `idx_frames_snapshot_pending`, `idx_meeting_transcript_segments_captured_at_julianday` | db-006 |
| `ui_events.element_ancestors`, `display_layout` | absent | new | db-007 |
| semantic tables | absent | `semantic_runs`, `semantic_items`, `semantic_run_items`, `semantic_items_fts`, `semantic_actors`, `semantic_actor_aliases`, `semantic_item_actors`, `frames.semantic_run_id` | db-008 |
| `pipe_event_runs`, pipe_executions columns | absent | new table; `trigger_event`, `trigger_key`, `conversation_id` | db-009 |
| `feedback` | absent | new | db-010 |
| speakers integrity | orphan rows NULLed (20260210100000) | orphan speaker ids re-created; startup self-heal of `speakers`/`speaker_embeddings` | db-011 |
| `meetings.calendar_event_id` | absent | new + partial unique index | db-012 |
| activity ledger | absent | 5 tables + 6 triggers + index | db-013 |
| `meeting_transcript_segments.stream_id/session_speaker_id` | absent | new + index | db-014 |
| `starred_sessions` | absent | new | db-015 |
| Pragmas / startup | WAL, NORMAL, `wal_autocheckpoint=4000`, TRUNCATE checkpoint at boot and every 5 min | WAL, NORMAL, `wal_autocheckpoint=0`(inferred), RESTART at boot, PASSIVE maintenance loop, read pool `query_only=ON`, header preflight, SQLite ≥ 3.51.3 guard | db-016 |
| Planner statistics | none | `ANALYZE` with `analysis_limit`, `PRAGMA main.optimize=0x10003`, `sqlite-planner-maintenance.json` | db-017 |
| Cancellable reads | none | `cancellable_query.rs` | db-018 |
| Hard-fault quarantine | write_queue recreates missing DB file; startup `quick_check` | coordinator crate: quarantine marker sidecars, fail-closed writer admission, verification, protected repair | db-019 |
| `screenpipe db recover` | shells out to `sqlite3 .recover` | embedded page-level recovery, manifest state machine, FTS rebuild, write canary | db-020 |
| Storage format | single `db.sqlite` | optional "hybrid-parquet-v1": SQLite index + Parquet payload files | db-021 |
| Bulk archival of other tables & elements | none | `_bulk_*` catalog, `_archive_*` columns, `screenpipe_elements` vtab | db-022 |
| Storage migration lifecycle | none | user-initiated migration (offline copy or in-place), journals, parity, diagnostics, delete-original | db-023 |
| Hybrid compact/reclaim/backup | none | offline compact, sparse free-page reclamation, hybrid backup bundle | db-024 |
| Full VACUUM endpoint | `/data/checkpoint`, `/data/backup` only | + `POST /data/compact` | db-025 |
| Secret store | `secrets` table in main `db.sqlite` | separate `secrets.sqlite` + `credential_refresh_leases` + `secret_store_metadata`, legacy copy | db-026 |
| `MigrationWorker` / `migration_progress` | present (`migration_worker.rs`) | removed | db-027 |
| Data-dir layout | `db.sqlite(+wal/shm)`, `.db_recovery.lock`, `vault.meta`, media dirs | many new sidecars and directories (see db-028) | db-028 |
| Toolchain | sqlx 0.7 / SQLite bundled by libsqlite3-sys 0.26 | sqlx 0.9 / libsqlite3-sys 0.37, rusqlite vtab, parquet | db-029 |

## 2. Delta items

### db-001 — Migration set: 28 new sqlx migrations (order, versions, checksums)

**Kind:** changed. **Confidence:** observed. **Evidence:** migration array `0xa2ae3a8..0xa2b0bd8` (117 entries, stride 0x58), SQL bodies `0xa2a1998..0xa2ae3a0`, `work/mig_array.txt`, `work/mig_match.txt`.

Embedded migration list (all `migration_type` = simple, `no_tx` = false; descriptions are sqlx's filename-derived descriptions). New entries in version order (the first one sorts *before* the base's last two):

| # | version | description (filename suffix) | SQL bytes | SHA-384 checksum (hex) |
|---|---|---|---|---|
| 1 | 20260601000000 | create_outputs | 1191 | 30d19eaa97f4b767c909049454ce37c11e9b9a5696230dfbb244c0cc3a603b36c48d2c3ee1c8e18bd4d21826b2906240 |
| (base) | 20260603000000 | one_open_meeting_invariant | 2317 | identical to base |
| (base) | 20260603120000 | meeting_transcript_speaker_id | 729 | identical to base |
| 2 | 20260612000000 | add_saf_columns_to_outputs | 698 | 62e96663f0416322aaedf798be8e2b0ce9a4488380394c186c85599daf90fff8211648d278116003e22e6d944a619433 |
| 3 | 20260613000000 | add_elements_redacted_at | 1382 | 5edabb504f7815c62a4efe7d9b596396dc322eb67947643cb851fb72137595d99fe3eaad57e97f07ba506e9ea33e2eba |
| 4 | 20260613000001 | add_frames_full_text_redacted_at | 1899 | 81612f52543f2ecfb2fbc41b1ab830dd23e079d60dd5ec1174cd0e623f63f9a49bdf815dccee73ef366995cef0f0a555 |
| 5 | 20260613130000 | unify_ocr_text_into_frames | 4666 | 302fcffa07df07ebb6b33e9f3719d949dbf90c3d503723e74040180912b145c9c48545fedac5b48555248ac4e1e6c5dd |
| 6 | 20260616120000 | speaker_filter_autocomplete_indexes | 255 | 1767ab1e3e5abe08adcd147b525c42d036a82db92c29e55f35d7c63e33c9a8c1f25ccc16eee7d0a42be2de16f9dd0ce3 |
| 7 | 20260618120000 | frames_app_name_timestamp_index | 1197 | 364a179becd5fda84dc485d96573ccc653a856f58708a8322a5d7cf965de8a1d3a3ed1d58c79400a77f91fca4b33d35a |
| 8 | 20260619000000 | add_frames_derived_text_redacted_at | 2300 | b4e49096aa88e2a5a6ca8b6152ae069999569c28a736b0bdcc760b7ec345764cc10f159c5e284a9e6d9ceaeafae9d4b0 |
| 9 | 20260619120000 | add_frames_browser_url_redacted_at | 1109 | 43dd4916e058cdf9a23d3237274b30ce48dc0d577e608a98e1f9f1cbd0bca54e34210403b5923577132be0da81669846 |
| 10 | 20260622000000 | create_output_search_index | 858 | fb70062178eb58c2b0581319e5f8c4dd83bf5cd424e80950ae17b2c7920333ff69be69529e8ce4abad5a282ea31cc8f7 |
| 11 | 20260627000000 | add_frames_text_json_redacted_at | 2367 | aedd5fe505370dbcf129aede33110b76da05d12dee82a9522d341054598fb2eb67c54fe103a846a439532acfdb736fb9 |
| 12 | 20260702000000 | add_frames_snapshot_pending_index | 1659 | 36e81f295d9c9e63cd8389bf29df6a7d251ae54c975296eaf873485a8f8149247a332b27a456a1e6f07324dd9a981251 |
| 13 | 20260702130000 | capture_action_fidelity | 1821 | ea637eb9895076823c2f37b580efcae4c3258211b95212975b9d11fe5dc24c26d315401f4749b522b1a69e0c7e07681d |
| 14 | 20260709000000 | make_redacted_at_indexes_partial | 2623 | 467d6aa5192816c666fe47185957023083b27405e15920169f9dc3704bfc243826aaef713fe4fe44cacd5222e8ab2b9e |
| 15 | 20260719000000 | index_meeting_transcript_segments_captured_at | 610 | 28fd070c5d46cab85d19e4772ae805c1df00a53e9385b22da444acad6c631d1db869885fe97f19e0e3ccdbe0b4c71e5d |
| 16 | 20260724000000 | create_semantic_context_tables | 4719 | d7bc7b69094021dd0d5a4cb6b5b4283397b47ba2dfcb467d62ca35a771e2407504936e660014d43dd99834f7cd2cd482 |
| 17 | 20260727000000 | create_semantic_actors | 2408 | af1bd0dcc6968d575f42c5e86f87da756378e46a5c4df60e9c6934cef3da4371c55a877c15accb11f632956f83b06ab8 |
| 18 | 20260728000000 | create_pipe_event_runs | 1141 | c71ca8f1f0522c20326f9380ef77cd56c6473d775f516b6475e31b4cb26ea801ab013b28a78fec2c4533c6ca95e3c3d2 |
| 19 | 20260729130000 | index_outputs_original_path | 503 | 0f419ca2729c99b515535dfc2531632c80c5df4cb8e448598b60a4848ccc45d16c68a436a40e61bd9121cf42ba74779f |
| 20 | 20260730180000 | create_feedback | 1093 | 056701af08e132d9f79656b32de4204bc28d79644482297e4bf837443dd778d1a5e54e8a47a0ca10eedbb527bed74b93 |
| 21 | 20260805000000 | add_pipe_execution_conversation_id | 370 | 33cbe2af6c5bdf6f96e47ea23f2c3132c9fe6b7915cf196cc09bfceb75ac8b27a7d68590492be60ef44df1e237dadfca |
| 22 | 20260812120000 | restore_orphaned_speaker_references | 2254 | d3a963d755335c8dd3e44090e4b9c356b17ac582739a78a92e9dea4c755dd8064264da55b2514f996c55af89456272be |
| 23 | 20260813000000 | add_calendar_event_id_to_meetings | 681 | 0dd9db9c16d1f956d66acef81f502d1410091b3b231a3053ca206dcd6bd3208aa773561d5090c3381d4fd740e39ff5fd |
| 24 | 20260817000000 | create_activity_ledger | 5989 | 820278e6c67ab85c38315d6378e53bf2aa853700a524299501768f332886ff6213a36aefe05122ba5bf577604d0a66fd |
| 25 | 20260817160000 | rebuild_activity_ledger_for_week_backfill | 780 | 897c85164aa712d73d83ea1ec0888bbdbe969e525a3da862db8af4e1ba08cdf2fca2414fb1a63d9daa714d7d476a5b43 |
| 26 | 20260818133000 | activity_ledger_query_indexes | 585 | 87b24280229975913f1ac27476f7b5bf097586e7f2ba4e2f79bda3db794ccc762a5311d73af76e61a7574263bbc8eefb |
| 27 | 20260828143000 | scope_live_meeting_speakers | 878 | 61087cd7c19be5e1ba990d6f0cc1f2a13d7b1e2ff3cd0fa659bea28e3098f58f89759aab5233b8d8411371c3a2968b7a |
| 28 | 20261002190000 | starred_sessions | 412 | 788d49f20afbf8325938e183d8e81bea449bd55775f512512702c4b44a7059dac3ceab3512eed68e9c01920a27eca2d0 |

Behaviour:
* sqlx applies every migration not yet present in `_sqlx_migrations`, regardless of order, so `20260601000000` will be applied on an existing base DB even though its version is lower than the base's newest. Keep `set_ignore_missing(true)` (target DBs created by screenpipe contain all 117).
* Checksum interop: Freenpipe must write its *own* migration text (comments are prose). When Freenpipe opens a DB that screenpipe already migrated, sqlx will report a checksum mismatch for these 28 versions; the base's existing `fix_migration_checksums` path (rewrite stored checksum, retry) already handles this and the target has the same fallback, so switching back and forth stays safe. Optional hardening: build the `Migrator` manually and, for these 28 versions, set the `checksum` field to the target's SHA-384 above so no rewrite is ever needed.
* Pre-migration log notices: before running, the target checks whether `20260415000000` (frames_fts external content) and `20260613130000` (ocr_text retirement) are pending (via `_sqlx_migrations` or, if absent, `SELECT 1 FROM frames LIMIT 1`) and logs a notice warning that the step can be slow on big databases (`0x49dbe02`). Base already does this for the FTS one; add the ocr_text one.
* Post-migration self-heal kept from base (`ensure_event_driven_columns`, `ensure_memories_sync_columns`; strings at `0x49db978`, `0x49dbc48`), plus new speakers self-heal (db-011).

Base files: `crates/screenpipe-db/src/migrations/` (add 28 files), `crates/screenpipe-db/src/db.rs` (`run_migrations`, `log_pending_search_index_migration`).

### db-002 — Outputs registry tables

**Kind:** new. **Confidence:** observed. **Evidence:** migrations 20260601000000, 20260612000000, 20260622000000, 20260729130000; queries at `0x4983210`..`0x4983678` (`db/outputs.rs`), `0x51a63e0..0x51a6920`.

DDL:
```sql
CREATE TABLE IF NOT EXISTS outputs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    source_type TEXT NOT NULL DEFAULT 'pipe',
    title TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'text',
    original_path TEXT,
    output_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    preview TEXT,
    metadata TEXT DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_outputs_source ON outputs(source);
CREATE INDEX IF NOT EXISTS idx_outputs_source_type ON outputs(source_type);
CREATE INDEX IF NOT EXISTS idx_outputs_created_at ON outputs(created_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS idx_outputs_output_path ON outputs(output_path);
-- 20260612000000
ALTER TABLE outputs ADD COLUMN saf_kind TEXT;
ALTER TABLE outputs ADD COLUMN artifact_id TEXT;
ALTER TABLE outputs ADD COLUMN saf_version INTEGER;
-- 20260622000000
CREATE TABLE IF NOT EXISTS output_search_index (
    output_id INTEGER PRIMARY KEY REFERENCES outputs(id) ON DELETE CASCADE,
    content_hash TEXT NOT NULL,
    bytes_indexed INTEGER NOT NULL DEFAULT 0,
    indexed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE VIRTUAL TABLE IF NOT EXISTS output_search_fts USING fts5(
    title,
    body,
    tokenize='unicode61'
);
-- 20260729130000
CREATE INDEX IF NOT EXISTS idx_outputs_source_type_original_path_id
ON outputs(source_type, original_path, id DESC)
WHERE original_path IS NOT NULL;
```
Semantics (paraphrased): one row per file a pipe/chat/agent produced and that was copied into `~/.screenpipe/outputs/`. `saf_kind`/`artifact_id`/`saf_version` are NULL for plain files and filled for "Screenpipe Artifact Format" envelopes (`saf_version` holds the artifact's ever-increasing version number). `output_search_index` is a sidecar recording which content hash was indexed into the plain (non-external-content) FTS5 table `output_search_fts` (rowid = `outputs.id`); re-indexing deletes the FTS row by rowid and re-inserts. Query shapes observed: insert of all 12 columns; update of title/kind/original_path/size/preview/metadata/saf_* with `updated_at` refresh; lookup by `id`, by `output_path`, latest by `(source, source_type, artifact_id)`; listing filters exclude `saf_kind = 'structured-output'` (`0xa27b150`).
Base files: new `crates/screenpipe-db/src/db/outputs.rs` (or section in `db.rs`), types in `types.rs`. Routes/UI are owned by the outputs/artifacts dimension.

### db-003 — `elements.redacted_at`

**Kind:** changed. **Confidence:** observed. **Evidence:** migration 20260613000000.
```sql
ALTER TABLE elements ADD COLUMN redacted_at INTEGER;
CREATE INDEX IF NOT EXISTS idx_elements_redacted_at ON elements(redacted_at);
```
Behaviour: the async text-PII worker now rewrites `elements.text` in place and stamps `redacted_at`; `elements_fts` follows via the existing `elements_au` trigger. Rows with `redacted_at IS NULL` are the worker backlog (newest first). In hybrid mode, `redacted_at` also gates element archival (db-022).
Base files: migrations; `crates/screenpipe-redact/src/worker/tables.rs` (add elements surface).

### db-004 — Frame redaction watermark columns and partial indexes

**Kind:** changed. **Confidence:** observed. **Evidence:** migrations 20260613000001, 20260619000000, 20260619120000, 20260627000000, 20260709000000.
```sql
ALTER TABLE frames ADD COLUMN full_text_redacted_at INTEGER;
ALTER TABLE frames ADD COLUMN accessibility_tree_redacted_at INTEGER;
ALTER TABLE frames ADD COLUMN window_name_redacted_at INTEGER;
ALTER TABLE frames ADD COLUMN browser_url_redacted_at INTEGER;
ALTER TABLE frames ADD COLUMN text_json_redacted_at INTEGER;
-- final index shape after 20260709000000 (each was first created as a full index, then dropped and recreated partial)
CREATE INDEX IF NOT EXISTS idx_frames_accessibility_redacted_at ON frames(accessibility_redacted_at) WHERE accessibility_redacted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_frames_full_text_redacted_at ON frames(full_text_redacted_at) WHERE full_text_redacted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_frames_accessibility_tree_redacted_at ON frames(accessibility_tree_redacted_at) WHERE accessibility_tree_redacted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_frames_window_name_redacted_at ON frames(window_name_redacted_at) WHERE window_name_redacted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_frames_browser_url_redacted_at ON frames(browser_url_redacted_at) WHERE browser_url_redacted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_frames_text_json_redacted_at ON frames(text_json_redacted_at) WHERE text_json_redacted_at IS NULL;
```
Migration sequence to reproduce exactly: 20260613000001 creates `idx_frames_full_text_redacted_at` (full); 20260619000000 creates the tree/window full indexes; 20260619120000 the browser_url one; 20260627000000 the text_json one; 20260709000000 does `DROP INDEX IF EXISTS` + `CREATE INDEX IF NOT EXISTS ... WHERE col IS NULL` for all six (including the pre-existing `idx_frames_accessibility_redacted_at` from base 20260501000000).
Behaviour (paraphrased): every per-frame derived text surface gets its own "processed" watermark. `full_text` is redacted by detection; `accessibility_tree_json`, `window_name`, `browser_url`, `text_json` are scrubbed by propagating the `full_text` detection map (in `text_json` only the `text` field of each block is rewritten; geometry unchanged). Go-forward only: frames whose `full_text` was redacted before a column existed keep the raw derived copy. `window_name` and `browser_url` are `frames_fts` columns; the existing `frames_au` trigger re-indexes them.
Base files: migrations; `crates/screenpipe-redact/src/worker/*` (add surfaces); `db.rs` redaction fetch queries.

### db-005 — Retire `ocr_text`; OCR text and boxes move onto `frames`

**Kind:** removed (table) + changed (frames). **Confidence:** observed. **Evidence:** migration 20260613130000 (`0xa2a3b73`), timeline/search queries reading `COALESCE(f.full_text, f.accessibility_text, '')` and `COALESCE(f.text_json, '')` (`0x49df75b`, `0x49dd292`, `0x49ddcc8`), late-OCR update `UPDATE frames SET full_text = CASE WHEN ?2 != '' ... text_json = CASE WHEN ?3 != '' ...` (`0x5143218`), search result column `'' as ocr_engine` (`0x49dd353`).

Migration steps (paraphrased; DDL/DML identifiers exact):
1. `ALTER TABLE frames ADD COLUMN text_json TEXT DEFAULT NULL;` (must not re-add `full_text_redacted_at`, which 20260613000001 owns and which sorts earlier).
2. Backfill, for frames whose value is NULL/empty, from any `ocr_text` row of the same `frame_id` with a non-empty value: `frames.app_name` (when NULL or ''), `frames.window_name` (when NULL), `frames.focused` (when NULL), `frames.full_text` (when NULL or ''), `frames.text_json` (when NULL). Each is an `UPDATE frames SET col = (SELECT ot.col FROM ocr_text ot WHERE ot.frame_id = frames.id AND <non-empty> LIMIT 1) WHERE <target empty> AND EXISTS(...)`.
3. Leave `full_text_redacted_at` NULL (so the redaction worker scrubs `full_text` again); do not copy `ocr_text.redacted_at`.
4. Drop: `DROP TRIGGER IF EXISTS ocr_text_ai; ocr_text_au; ocr_text_ad; ocr_text_update; ocr_text_delete; DROP TABLE IF EXISTS ocr_text_fts; DROP TABLE IF EXISTS ocr_text;`
5. `ocr_engine` is dropped (search responses emit an empty string).

Consequences for Freenpipe code (base touchpoints, 229 `ocr_text` references across 36 files): `crates/screenpipe-db/src/db.rs` (inserts, search, timeline, delete_time_range, sync), `write_queue.rs` (frame/OCR insert batches and the empty-DB recreate DDL at ~line 1810), `types.rs`, `migration_worker.rs` (delete, see db-027), `crates/screenpipe-redact/src/worker/tables.rs` (stop treating `ocr_text` as a surface; use `frames.full_text` + `full_text_redacted_at`), `crates/screenpipe-engine/src/routes/{frames,streaming,content,search}.rs`, `archive.rs`, `hot_frame_cache.rs`, `video_cache.rs`, `sync_provider.rs`, `crates/screenpipe-connect/src/remote_sync.rs`, `crates/screenpipe-capture/src/paired_capture.rs`, tests/benches. OCR output path becomes: write `full_text` (plain text) and `text_json` (per-word boxes JSON) on the frame row; per-element rows still go to `elements` (`source='ocr'`, roles `page|block|paragraph|line|word`, `0xa2b8512`). Delete counters returned by time-range deletion no longer have an `ocr_text` count in the target strings (`0x5140e3a`: frames, audio_transcriptions, audio_chunks, video_chunks, accessibility, ui_events).

### db-006 — Query-performance indexes

**Kind:** changed. **Confidence:** observed. **Evidence:** migrations 20260616120000, 20260618120000, 20260702000000, 20260719000000.
```sql
CREATE INDEX IF NOT EXISTS idx_speakers_hallucination_name
  ON speakers(hallucination, name COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS idx_tags_name_nocase
  ON tags(name COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS idx_frames_app_name_timestamp
    ON frames(app_name, timestamp);
CREATE INDEX IF NOT EXISTS idx_frames_snapshot_pending
    ON frames(device_name, timestamp)
    WHERE snapshot_path IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_meeting_transcript_segments_captured_at_julianday
    ON meeting_transcript_segments(julianday(captured_at));
```
Purpose (paraphrased): people-filter/speaker autocomplete; per-app capped search window (`ROW_NUMBER() OVER (PARTITION BY app_name ORDER BY timestamp)`); snapshot compaction + media retention scan for frames still holding an uncompacted JPEG (`snapshot_path IS NOT NULL AND timestamp < ? ORDER BY device_name, timestamp LIMIT 5000`); meeting timeline query that filters/orders by `julianday(captured_at)`. Queries must use the exact `julianday(captured_at)` expression to hit the expression index.
Base files: migrations; `db.rs` timeline/meeting queries (rewrite comparisons to `julianday(...)`).

### db-007 — `ui_events.element_ancestors` and `display_layout`

**Kind:** new. **Confidence:** observed. **Evidence:** migration 20260702130000; `INSERT INTO display_layout (reason, layout_json) VALUES (?1, ?2)` and `SELECT layout_json FROM display_layout ORDER BY id DESC LIMIT 1` (`0x52435d0`); ui_events select list incl. `element_ancestors` (`0x49d9699`).
```sql
ALTER TABLE ui_events ADD COLUMN element_ancestors TEXT;
CREATE TABLE IF NOT EXISTS display_layout (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    reason TEXT NOT NULL DEFAULT 'change',
    layout_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_display_layout_timestamp
    ON display_layout(timestamp);
```
Data shapes: `element_ancestors` = compact JSON array, root first, of `{"role":..., "name":...}` objects (name optional) for the clicked element's container chain. `reason` ∈ `startup|change`. `layout_json` = canonical JSON array sorted by display id, each `{"id":int,"stable_id":str,"name":str,"x":num,"y":num,"width":num,"height":num,"is_primary":bool}` (global-desktop origin in points). Write a row when capture begins and again on every layout change; skip if identical to the latest row (inferred from the "latest" query).
Base files: migrations; `crates/screenpipe-db/src/db.rs` (+ insert/select fns); `crates/screenpipe-engine/src/ui_recorder.rs` and monitor watcher (callers, owned by capture dimension).

### db-008 — Semantic context and actor tables

**Kind:** new. **Confidence:** observed (DDL), inferred (writer behaviour). **Evidence:** migrations 20260724000000, 20260727000000; queries in `db/semantic.rs` (`0x5145400`, `0x51a74c8..0x51a7f20`, `0x5249b08`, `0x5249e18`, `0xa275fa9`); parser registry strings in `screenpipe-semantic` (`0xa2ebcef`).
```sql
CREATE TABLE semantic_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    input_fingerprint BLOB NOT NULL UNIQUE
        CHECK (length(input_fingerprint) = 32),
    parser_id TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    app_platform TEXT NOT NULL
        CHECK (app_platform IN ('macos', 'windows', 'linux')),
    app_id TEXT,
    app_executable TEXT,
    app_version TEXT,
    input_content_hash BLOB NOT NULL
        CHECK (length(input_content_hash) = 8),
    status TEXT NOT NULL
        CHECK (status IN ('handled', 'empty')),
    item_count INTEGER NOT NULL DEFAULT 0 CHECK (item_count >= 0),
    text_bytes INTEGER NOT NULL DEFAULT 0 CHECK (text_bytes >= 0),
    parse_duration_us INTEGER NOT NULL DEFAULT 0 CHECK (parse_duration_us >= 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX idx_semantic_runs_parser
    ON semantic_runs(parser_id, parser_version, schema_version);
CREATE TABLE semantic_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_fingerprint BLOB NOT NULL CHECK (length(entity_fingerprint) = 32),
    version_fingerprint BLOB NOT NULL UNIQUE
        CHECK (length(version_fingerprint) = 32),
    kind TEXT NOT NULL
        CHECK (kind IN ('conversation', 'message', 'document', 'task', 'calendar_event', 'page')),
    item_key TEXT NOT NULL,
    identity_quality TEXT NOT NULL
        CHECK (identity_quality IN ('stable', 'derived', 'ephemeral')),
    title TEXT,
    body TEXT,
    actor TEXT,
    occurred_at TEXT,
    timestamp_precision TEXT,
    status TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(metadata_json)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX idx_semantic_items_entity
    ON semantic_items(entity_fingerprint);
CREATE INDEX idx_semantic_items_kind_time
    ON semantic_items(kind, occurred_at);
CREATE TABLE semantic_run_items (
    run_id INTEGER NOT NULL,
    local_id TEXT NOT NULL,
    item_id INTEGER NOT NULL,
    parent_local_id TEXT,
    sort_order INTEGER NOT NULL CHECK (sort_order >= 0),
    source_node_ids BLOB NOT NULL DEFAULT X'',
    PRIMARY KEY (run_id, local_id),
    UNIQUE (run_id, sort_order),
    FOREIGN KEY (run_id) REFERENCES semantic_runs(id) ON DELETE CASCADE,
    FOREIGN KEY (item_id) REFERENCES semantic_items(id) ON DELETE RESTRICT,
    FOREIGN KEY (run_id, parent_local_id)
        REFERENCES semantic_run_items(run_id, local_id)
        DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX idx_semantic_run_items_item
    ON semantic_run_items(item_id);
CREATE VIRTUAL TABLE semantic_items_fts USING fts5(
    title,
    body,
    actor,
    status,
    metadata_json,
    content='semantic_items',
    content_rowid='id',
    tokenize='unicode61'
);
CREATE TRIGGER semantic_items_ai AFTER INSERT ON semantic_items BEGIN
    INSERT INTO semantic_items_fts(rowid, title, body, actor, status, metadata_json)
    VALUES (new.id, new.title, new.body, new.actor, new.status, new.metadata_json);
END;
CREATE TRIGGER semantic_items_ad AFTER DELETE ON semantic_items BEGIN
    INSERT INTO semantic_items_fts(
        semantic_items_fts, rowid, title, body, actor, status, metadata_json
    ) VALUES (
        'delete', old.id, old.title, old.body, old.actor, old.status, old.metadata_json
    );
END;
CREATE TRIGGER semantic_items_au AFTER UPDATE ON semantic_items BEGIN
    INSERT INTO semantic_items_fts(
        semantic_items_fts, rowid, title, body, actor, status, metadata_json
    ) VALUES (
        'delete', old.id, old.title, old.body, old.actor, old.status, old.metadata_json
    );
    INSERT INTO semantic_items_fts(rowid, title, body, actor, status, metadata_json)
    VALUES (new.id, new.title, new.body, new.actor, new.status, new.metadata_json);
END;
ALTER TABLE frames ADD COLUMN semantic_run_id INTEGER
    REFERENCES semantic_runs(id) ON DELETE SET NULL;
CREATE INDEX idx_frames_semantic_run_id
    ON frames(semantic_run_id)
    WHERE semantic_run_id IS NOT NULL;
-- 20260727000000
CREATE TABLE semantic_actors (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL
        CHECK (length(trim(name)) BETWEEN 1 AND 256),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX idx_semantic_actors_name
    ON semantic_actors(name COLLATE NOCASE);
CREATE TABLE semantic_actor_aliases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_id INTEGER NOT NULL,
    source_key TEXT NOT NULL CHECK (length(source_key) BETWEEN 1 AND 1024),
    source_label TEXT NOT NULL CHECK (length(source_label) BETWEEN 1 AND 256),
    normalized_label TEXT NOT NULL
        CHECK (length(normalized_label) BETWEEN 1 AND 512),
    observed_label TEXT NOT NULL
        CHECK (length(trim(observed_label)) BETWEEN 1 AND 256),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (source_key, normalized_label),
    FOREIGN KEY (actor_id) REFERENCES semantic_actors(id) ON DELETE CASCADE
);
CREATE INDEX idx_semantic_actor_aliases_actor
    ON semantic_actor_aliases(actor_id);
CREATE INDEX idx_semantic_actor_aliases_observed_label
    ON semantic_actor_aliases(observed_label COLLATE NOCASE);
CREATE TABLE semantic_item_actors (
    item_id INTEGER PRIMARY KEY,
    actor_id INTEGER NOT NULL,
    observed_alias_id INTEGER,
    assignment_source TEXT NOT NULL DEFAULT 'heuristic'
        CHECK (assignment_source IN ('heuristic', 'reconciled')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (item_id) REFERENCES semantic_items(id) ON DELETE CASCADE,
    FOREIGN KEY (actor_id) REFERENCES semantic_actors(id) ON DELETE RESTRICT,
    FOREIGN KEY (observed_alias_id)
        REFERENCES semantic_actor_aliases(id) ON DELETE SET NULL
);
CREATE INDEX idx_semantic_item_actors_actor
    ON semantic_item_actors(actor_id, item_id);
```
Model (paraphrased): a frame points to one immutable parse run (`frames.semantic_run_id`); runs are deduplicated by a 32-byte input fingerprint; items are canonical, deduplicated by a 32-byte version fingerprint and grouped by entity fingerprint; the run→item join holds run-local order, hierarchy (`parent_local_id`) and source-node provenance blob. Actors: parser output keeps the observed `semantic_items.actor`; `semantic_item_actors` holds the assigned identity (heuristic or user/pipe "reconciled"); aliases persist across item retention.
DB-level maintenance observed: garbage collection of runs no frame references and items no run references (`DELETE FROM semantic_runs WHERE NOT EXISTS (SELECT 1 FROM frames WHERE frames.semantic_run_id = semantic_runs.id)`, same for items vs `semantic_run_items`, `0x5145400`); actor merge = move aliases + item assignments to the kept id then delete the other; rename updates `name`/`updated_at`; alias reassignment updates heuristic assignments only. Search joins `frames → semantic_runs (status='handled') → semantic_run_items → semantic_items_fts MATCH` with optional actor filter. Stored platform/kind/identity-quality strings are validated on read; an unrecognised stored value is an error (`0xa275d40`).
Base files: migrations; new `crates/screenpipe-db/src/db/semantic.rs`. The parser/worker (`screenpipe-semantic`, `semantic_worker.rs`) and `/search/semantic/*` routes belong to the semantic dimension.

### db-009 — Event-run idempotency and pipe_executions columns

**Kind:** new. **Confidence:** observed. **Evidence:** migrations 20260728000000, 20260805000000; `INSERT INTO pipe_executions (pipe_name, status, trigger_type, model, provider, started_at, trigger_event, trigger_key)` (`0xa2b6bb8`).
```sql
CREATE TABLE IF NOT EXISTS pipe_event_runs (
    pipe_name  TEXT NOT NULL,
    event_name TEXT NOT NULL,
    event_key  TEXT NOT NULL,
    claimed_at TEXT NOT NULL,
    PRIMARY KEY (pipe_name, event_name, event_key)
);
CREATE INDEX IF NOT EXISTS idx_pipe_event_runs_claimed_at ON pipe_event_runs(claimed_at);
ALTER TABLE pipe_executions ADD COLUMN trigger_event TEXT;
ALTER TABLE pipe_executions ADD COLUMN trigger_key TEXT;
ALTER TABLE pipe_executions ADD COLUMN conversation_id TEXT;
```
Behaviour: before starting an event-triggered run the scheduler claims `(pipe, event, key)` with `INSERT OR IGNORE`; zero rows inserted means already claimed (e.g. `meeting_ended` firing twice for a rejoined meeting within the 120 s rejoin window). Old claims are pruned by `claimed_at`. `trigger_event`/`trigger_key` record which event instance produced an execution; `conversation_id` is the stable chat id when a pipe run continues an agent session.
Base files: migrations; `crates/screenpipe-engine/src/pipe_store.rs` (it also has inline `CREATE TABLE IF NOT EXISTS pipe_executions` — keep both in sync).

### db-010 — `feedback` table

**Kind:** new. **Confidence:** observed. **Evidence:** migration 20260730180000; `db/feedback.rs` queries (`0x51a6cb8`, `0x51a6d50`).
```sql
CREATE TABLE IF NOT EXISTS feedback (
    id TEXT PRIMARY KEY,
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    target_version TEXT NOT NULL DEFAULT '',
    producer_ref TEXT,
    actor_id TEXT NOT NULL,
    rating TEXT NOT NULL CHECK (rating IN ('up', 'down')),
    comment TEXT,
    snapshot TEXT,
    context TEXT NOT NULL DEFAULT '{}',
    device_id TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (target_kind, target_id, target_version, actor_id, device_id)
);
CREATE INDEX IF NOT EXISTS idx_feedback_target
    ON feedback (target_kind, target_id, target_version);
CREATE INDEX IF NOT EXISTS idx_feedback_producer_updated
    ON feedback (producer_ref, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_feedback_rating_updated
    ON feedback (rating, updated_at DESC);
```
Behaviour: upsert keyed by `id` (`ON CONFLICT(id) DO UPDATE SET producer_ref, rating, comment, ...`), with an existence lookup by the natural key `(target_kind, target_id, target_version, actor_id, device_id)` to reuse the existing id/created_at; clearing feedback deletes the row for the local device (`device_id = ''`). Routes (`/ai-feedback`, `SetFeedbackRequest`) belong to the feedback dimension.

### db-011 — Speaker integrity: orphan restore and table self-heal

**Kind:** changed. **Confidence:** observed. **Evidence:** migration 20260812120000; setup strings `0x49db209..0x49db6d0`; speakers deletion strings `0x5145680..0x5145a14`; `INSERT INTO speakers (id, name) VALUES (?, '')` (`0x5147988`).
Migration DML (verbatim):
```sql
INSERT INTO speakers (id, name)
SELECT DISTINCT speaker_id, ''
FROM (
    SELECT speaker_id FROM audio_transcriptions
    WHERE speaker_id IS NOT NULL
    UNION
    SELECT speaker_id FROM meeting_transcript_segments
    WHERE speaker_id IS NOT NULL
    UNION
    SELECT speaker_id FROM speaker_embeddings
    WHERE speaker_id IS NOT NULL
    UNION
    SELECT speaker_id FROM diarization_segments
    WHERE speaker_id IS NOT NULL
    UNION
    SELECT speaker_id FROM speaker_identity_evidence
    WHERE speaker_id IS NOT NULL
)
WHERE speaker_id NOT IN (SELECT id FROM speakers);
```
Rationale (paraphrased): re-create nameless speaker rows for ids that rows still reference, preserving voice grouping; safe because `speakers` is AUTOINCREMENT (ids never reissued), insert-only and idempotent.
Startup self-heal (new in `db/setup.rs`): after migrations, if `sqlite_master` lacks table `speakers`, create it with columns `id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, metadata JSON, hallucination BOOLEAN DEFAULT FALSE, centroid FLOAT[512], embedding_count INTEGER DEFAULT 0` plus `idx_speakers_hallucination_name`; if `speaker_embeddings` is missing create `id INTEGER PRIMARY KEY AUTOINCREMENT, embedding FLOAT[512] NOT NULL check(typeof(embedding) == 'blob' and vec_length(embedding) == 512), speaker_id INTEGER REFERENCES speakers(id)` plus `idx_speaker_embeddings_speaker_id`. Speaker deletion in the target re-points or keeps rows in all five referencing tables (`audio_transcriptions, meeting_transcript_segments, speaker_embeddings, diarization_segments, speaker_identity_evidence`) and refuses to delete a speaker that still has rows (it logs that the speaker is kept and which table still references it).
Base files: migrations; `db.rs` (`run_migrations` self-heal; speaker delete/merge).

### db-012 — `meetings.calendar_event_id`

**Kind:** changed. **Confidence:** observed. **Evidence:** migration 20260813000000; `SELECT calendar_event_id FROM meetings WHERE id = ?1` (`0x4983bc8`); unique-violation handling `UNIQUE constraint failed meetings.calendar_event_id idx_meetings_calendar_event_id` (SQLite constraint-error text plus identifiers; exact string, client-visible) (`0xa2b8742`).
```sql
ALTER TABLE meetings ADD COLUMN calendar_event_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_meetings_calendar_event_id
    ON meetings (calendar_event_id)
    WHERE calendar_event_id IS NOT NULL AND calendar_event_id != '';
```
Behaviour: a calendar event can be attached to at most one meeting; writers (detector, reopen enrichment, client PUT) must treat a unique violation on this index as meaning the event is already attached to another meeting, not as a fatal error.

### db-013 — Activity ledger

**Kind:** new. **Confidence:** observed. **Evidence:** migrations 20260817000000, 20260817160000, 20260818133000; module `db/activity_ledger.rs` (`0x52d3ba0`).
```sql
CREATE TABLE activity_tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_key TEXT NOT NULL UNIQUE,
    parent_task_id INTEGER REFERENCES activity_tasks(id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK (kind IN ('category', 'task', 'unobserved')),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 256),
    app_name TEXT,
    confidence REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
    producer TEXT NOT NULL,
    user_locked INTEGER NOT NULL DEFAULT 0 CHECK (user_locked IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX idx_activity_tasks_parent ON activity_tasks(parent_task_id, id);
CREATE INDEX idx_activity_tasks_app ON activity_tasks(app_name, id);
CREATE TABLE activity_intervals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    interval_key TEXT NOT NULL UNIQUE,
    task_id INTEGER NOT NULL REFERENCES activity_tasks(id) ON DELETE CASCADE,
    start_at TEXT NOT NULL,
    end_at TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('provisional', 'final')),
    confidence REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
    producer TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (julianday(end_at) > julianday(start_at))
);
CREATE INDEX idx_activity_intervals_range
    ON activity_intervals(start_at, end_at, id);
CREATE INDEX idx_activity_intervals_task
    ON activity_intervals(task_id, start_at, id);
CREATE TABLE activity_actions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    action_key TEXT NOT NULL UNIQUE,
    interval_id INTEGER NOT NULL REFERENCES activity_intervals(id) ON DELETE CASCADE,
    occurred_at TEXT NOT NULL,
    action_type TEXT NOT NULL,
    summary TEXT NOT NULL CHECK (length(summary) BETWEEN 1 AND 512),
    app_name TEXT,
    confidence REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
    source_type TEXT NOT NULL CHECK (source_type IN ('frame', 'ui_event', 'audio')),
    source_id INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX idx_activity_actions_interval
    ON activity_actions(interval_id, occurred_at, id);
CREATE INDEX idx_activity_actions_source
    ON activity_actions(source_type, source_id);
CREATE TABLE activity_evidence (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    interval_id INTEGER NOT NULL REFERENCES activity_intervals(id) ON DELETE CASCADE,
    action_id INTEGER REFERENCES activity_actions(id) ON DELETE CASCADE,
    source_type TEXT NOT NULL CHECK (source_type IN ('frame', 'ui_event', 'audio')),
    source_id INTEGER NOT NULL,
    occurred_at TEXT NOT NULL,
    UNIQUE(interval_id, source_type, source_id)
);
CREATE INDEX idx_activity_evidence_interval
    ON activity_evidence(interval_id, occurred_at, id);
CREATE INDEX idx_activity_evidence_source
    ON activity_evidence(source_type, source_id);
CREATE TABLE activity_ledger_state (
    producer TEXT PRIMARY KEY,
    last_reconciled_at TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE TRIGGER activity_frames_ad AFTER DELETE ON frames BEGIN
    DELETE FROM activity_actions
      WHERE source_type = 'frame' AND source_id = OLD.id;
    DELETE FROM activity_evidence
      WHERE source_type = 'frame' AND source_id = OLD.id;
END;
CREATE TRIGGER activity_ui_events_ad AFTER DELETE ON ui_events BEGIN
    DELETE FROM activity_actions
      WHERE source_type = 'ui_event' AND source_id = OLD.id;
    DELETE FROM activity_evidence
      WHERE source_type = 'ui_event' AND source_id = OLD.id;
END;
CREATE TRIGGER activity_audio_ad AFTER DELETE ON audio_transcriptions BEGIN
    DELETE FROM activity_actions
      WHERE source_type = 'audio' AND source_id = OLD.id;
    DELETE FROM activity_evidence
      WHERE source_type = 'audio' AND source_id = OLD.id;
END;
CREATE TRIGGER activity_evidence_ad AFTER DELETE ON activity_evidence BEGIN
    DELETE FROM activity_intervals
      WHERE id = OLD.interval_id
        AND NOT EXISTS (
            SELECT 1 FROM activity_evidence
            WHERE interval_id = OLD.interval_id
        );
END;
CREATE TRIGGER activity_intervals_ad AFTER DELETE ON activity_intervals BEGIN
    DELETE FROM activity_tasks
      WHERE id = OLD.task_id
        AND user_locked = 0
        AND NOT EXISTS (
            SELECT 1 FROM activity_intervals WHERE task_id = OLD.task_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM activity_tasks WHERE parent_task_id = OLD.task_id
        );
END;
CREATE TRIGGER activity_tasks_ad AFTER DELETE ON activity_tasks
WHEN OLD.parent_task_id IS NOT NULL BEGIN
    DELETE FROM activity_tasks
      WHERE id = OLD.parent_task_id
        AND user_locked = 0
        AND NOT EXISTS (
            SELECT 1 FROM activity_intervals
            WHERE task_id = OLD.parent_task_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM activity_tasks
            WHERE parent_task_id = OLD.parent_task_id
        );
END;
-- 20260817160000 (data reset)
DELETE FROM activity_intervals
WHERE producer = 'deterministic-v1';
DELETE FROM activity_tasks
WHERE producer = 'deterministic-v1'
  AND user_locked = 0;
DELETE FROM activity_ledger_state
WHERE producer = 'deterministic-v1';
-- 20260818133000
CREATE INDEX IF NOT EXISTS idx_activity_intervals_producer_range
    ON activity_intervals(producer, start_at, end_at, id);
```
Retention/deletion consequence: deleting capture rows (frames/ui_events/audio_transcriptions) cascades through these triggers; an interval disappears only when its last evidence row is gone; unlocked tasks are collected when they have no intervals or children. Producer `deterministic-v1` is the built-in reconciler; it rewrites a trailing window per reconciliation and records `last_reconciled_at`. Ledger semantics/routes belong to the activity dimension.

### db-014 — Stream-scoped live meeting speakers

**Kind:** changed. **Confidence:** observed. **Evidence:** migration 20260828143000; meeting segment union query `0x49e0238` (`mts.session_speaker_id`); live-evidence CTE `0x524a908` (`m.stream_id`).
```sql
ALTER TABLE meeting_transcript_segments
    ADD COLUMN stream_id TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE meeting_transcript_segments
    ADD COLUMN session_speaker_id TEXT;
CREATE INDEX IF NOT EXISTS idx_meeting_transcript_segments_session_speaker
    ON meeting_transcript_segments(meeting_id, session_speaker_id, captured_at);
```
Behaviour: a live provider's speaker labels have meaning only inside a single streaming connection; each segment records an opaque `stream_id` (existing rows `'legacy'`) and the provider label as `session_speaker_id`; `speaker_id` stays the global cross-meeting identity filled later by embedding matching. Display name precedence: resolved global speaker name, else `speaker_name`.
Base: `NewMeetingTranscriptSegment` in `db.rs` (add `stream_id`, `session_speaker_id`).

### db-015 — `starred_sessions`

**Kind:** new. **Confidence:** observed. **Evidence:** migration 20261002190000; `db/starred.rs` queries (`0x52484d0`, `0x5248260`, `0x51a6920`).
```sql
CREATE TABLE starred_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    start TEXT NOT NULL,
    end TEXT NOT NULL,
    hd_requested INTEGER NOT NULL DEFAULT 0,
    revision INTEGER NOT NULL DEFAULT 1,
    CHECK (end > start)
);
CREATE INDEX idx_starred_sessions_start_end ON starred_sessions(start, end);
```
Queries: membership test for a JSON array of timestamps (`EXISTS(... s.start <= j.value AND s.end > j.value)` over `json_each`, ordered by array key); merged coverage islands inside a window (clip to window, window function to find gaps, group into islands, return `MIN(start), MAX(end)` per island); listing with `has_audio` computed by checking `audio_chunks` timestamps within the session (string prefix range on `substr(...,1,19)` plus `julianday` exact compare, excluding empty `file_path`). Owned by the starred-sessions dimension for routes/UI.

### db-016 — Connection setup, pragmas, WAL maintenance and startup preflight

**Kind:** changed. **Confidence:** observed unless marked. **Evidence:** setup literals `0x49daec1` (`cache_size mmap_size journal_mode WAL synchronous NORMAL temp_store MEMORY wal_autocheckpoint 0 DELETE FULL locking_mode EXCLUSIVE`), `0x49daf37` (`PRAGMA journal_mode=`, `query_only ON`), `0x49daf78` (`PRAGMA wal_checkpoint(RESTART)`), `0x49db0c0` (`PRAGMA temp_store=FILE`), maintenance `0x5140340`/`0xa2ca98e` (`PRAGMA wal_checkpoint(PASSIVE)`), `0xa278527..0xa278a00`, preflight `0xa279e40..0xa279ff6`, version guard `0xa2ea870..0xa2ea907`.

Target behaviour:
1. **Connection pragmas (normal mode):** `journal_mode=WAL`, `synchronous=NORMAL`, `cache_size=-<KB>`, `mmap_size=<bytes>` (from DbConfig tier, unchanged), `temp_store=MEMORY`, `wal_autocheckpoint=0` (inferred from the literal `0` packed directly after `wal_autocheckpoint`; i.e. writers never checkpoint inline). Read pool connections additionally set `query_only=ON`.
2. **Offline/exclusive mode** (storage conversion, recovery verification): `journal_mode=DELETE`, `synchronous=FULL`, `locking_mode=EXCLUSIVE`; `temp_store=FILE` when building large indexes so temp B-trees do not live in RAM.
3. **Startup checkpoint:** `PRAGMA wal_checkpoint(RESTART)` (base used TRUNCATE); log `busy/pages/checkpointed` and continue on failure.
4. **WAL maintenance task** (`db/maintenance.rs`): periodic `PRAGMA wal_checkpoint(PASSIVE)` that never blocks recording; tracks `pending_pages, log_pages, checkpointed, busy, backlog_seconds`; logs a deferral when reader snapshots pin the WAL and a separate event once the backlog has drained; after a SQLite hard fault (db-019) it stops for that database generation; it acquires the write coordinator before checkpointing and exits on shutdown. Base's 5-minute TRUNCATE loop is replaced.
5. **Bounded startup preflight** on an existing DB file before opening pools: read the 100-byte SQLite header and fail fast with SQLite-style errors: bad magic (`SQLite format 3\0`) → `(code: 26) file is not a database` (SQLite's own NOTADB text; exact string, client-visible); invalid page size; file length that is not a whole number of pages; invalid write/read versions (bytes 18/19 must be 1 or 2); invalid payload fractions (bytes 21..23 must be 64/32/32); invalid schema format (1..4); invalid text encoding (1..3). Malformed → `(code: 11) database disk image is malformed` (SQLite's own CORRUPT text; exact string, client-visible).
6. **SQLite runtime guard:** log `sqlite_version`, `sqlite_version_number`, `sqlite_source_id`; refuse to run if the linked SQLite is older than 3.51.3 (the WAL-reset corruption fix). Requires the bundled SQLite upgrade (db-029).
7. **sqlite-vec registration** via `sqlite3_auto_extension` now checks the return code and fails with an error that reports the SQLite result code when registration does not succeed.
8. **Process ownership:** a process may own only one live `DatabaseManager` per database path; an ownership file guards cross-process use (a second process gets an error telling it the database is owned elsewhere and must be closed before storage is changed, `0x5232229`).
9. Startup integrity check kept (`PRAGMA quick_check(1)`, `0xa2cae70`) with CLI hint text pointing to `screenpipe db recover`.
Base files: `crates/screenpipe-db/src/db.rs` (`DatabaseManager::new`, `spawn_wal_checkpoint`-style task near line 8665, `spawn_startup_integrity_check`), `crates/screenpipe-db/src/write_queue.rs`.

### db-017 — Query-planner statistics maintenance

**Kind:** new. **Confidence:** observed (statements), inferred (schedule). **Evidence:** `planner_stats.rs` strings `0xa2ca410..0xa2ca7f0`, `0xa280b9f..0xa281150`; report struct fields `0xa2b3968`, `0xa2b39b8`.
Behaviour:
* Periodically (once per maintenance tick), on the write connection with a short `PRAGMA busy_timeout` (restored afterwards; uses `busy_timeout=0` for a non-blocking attempt), set `PRAGMA analysis_limit=<n>` and run `ANALYZE` on selected tables, then `PRAGMA main.optimize=0x10003`, then force other connections to reload statistics by bumping the schema cookie: `BEGIN IMMEDIATE; CREATE TABLE main._screenpipe_planner_refresh(dummy); DROP TABLE main._screenpipe_planner_refresh; COMMIT`. `ANALYZE main.sqlite_schema` is also used.
* Outcomes: writer busy → retry next tick; `SQLITE_INTERRUPT` → retry next tick; shutdown → retry after restart; stages include `selecting_tables`; recorded failure categories cover I/O errors during maintenance and the database pool being unavailable or already closed.
* Persists `sqlite-planner-maintenance.json` in the data dir: `{ "latest": Attempt, "last_success": Attempt, "last_failure": Attempt }`, `Attempt = { started_at, status, duration_ms, analyzed_tables, stage, sqlite_code, detail }`.
* The literals `PRAGMA analysis_limit = ` / `PRAGMA optimize;` also recur many times in the binary; they belong to sqlx-sqlite's optional optimize-on-close support, so pooled connections probably enable it (inferred; set `optimize_on_close` on the connect options).
Base files: new `crates/screenpipe-db/src/planner_stats.rs`; scheduler hook in engine startup.

### db-018 — Cancellable read queries

**Kind:** new. **Confidence:** inferred (behaviour from messages). **Evidence:** `cancellable_query.rs` strings `0x49d9358`, `0x49d93c9`, `0xa2b5858`, `0xa280440..0xa280650`; write_queue `0xa2b8188`.
Behaviour: long read queries (search, timeline, activity) run on a pooled read connection wrapped in a guard that, when the HTTP request is dropped or times out, calls `sqlite3_interrupt` on that connection and spawns a cleanup task; if cleanup fails the connection is closed instead of returned (and a log line records that). Before a read connection is reused, any abandoned read transaction is rolled back so its WAL snapshot does not pin the WAL (logged as an informational event; recording is not interrupted).
Base files: new `crates/screenpipe-db/src/cancellable_query.rs`; `db.rs` heavy-read paths (`heavy_read_semaphore`).

### db-019 — SQLite hard-fault quarantine, verification and protected repair (coordinator)

**Kind:** new. **Confidence:** observed (identifiers), inferred (exact control flow). **Evidence:** `screenpipe-sqlite-coordinator` strings `0xa2e8c67..0xa2ea907`, `0x518c0c0..0x518c490`, `0x52321a0..0x52322d1`, `0xa281bd0`; write_queue `0xa2b6058`, `0xa2b5d40..0xa2b5e8d`; app `db_recovery_notifications.rs` `0x514cf40..0x514d268`; app commands in `frontend/_next/static/chunks/8001-00ed120106af2449.js` (`start_database_recovery`, `restart_database_verification`); serde structs `0xa2b3dc0`, `0xa2bdeb8`.

Concepts:
* **Hard fault:** a SQLite extended result code that indicates physical damage (CORRUPT, NOTADB, IOERR family, FULL) observed on any connection (logged as a hard fault together with the extended result code). The write queue then closes writer admission for that *database generation*, stops WAL maintenance, and requests engine recovery; recording pauses; search may continue ("search_only" mode: capture stopped, API retained).
* **Quarantine marker** sidecar next to the DB file: `<db>.quarantine.json` with schema
  `SqliteQuarantineMarker { schema_version, database_path, file_identity, sqlite_code, detected_at_unix_ms, reason, confirmed_damage, new_database }` where `file_identity` is an internally tagged enum `SqliteFileIdentity` with tag field `platform`; Windows variant `{ "platform":"windows", "volume_serial_number", "file_index" }` (unix variant inferred: device + inode). The marker's `database_path` must match its sidecar location; unknown `schema_version` is rejected; writes are atomic via `.tmp-*` + rename (a marker path without a parent directory is an error; literals `sqlite-quarantine`, `.tmp-`).
* `<db>.quarantine.reserve.json` (pre-armed reserve for a pending verification) and `<db>.verification-pending.json` (verification requested; detailed diagnostics optional).
* **Verification:** a fresh process opens the generation read-only with checkpoints disabled, runs `PRAGMA integrity_check` and `SELECT count(*) FROM sqlite_schema`; if no damage is confirmed the quarantine is resolved; if damage is confirmed, live journal recovery is forbidden and the user must run protected repair. Resolution will not touch the damaged physical generation named in the marker, nor a forbidden generation; installed identity must equal the verified replacement.
* **App surface:** a notification telling the user that a repair is required, with actions `recover-database` / `dismiss-database-recovery`, deep link `screenpipe://database-recovery`; Tauri commands `start_database_recovery` (runs the embedded recovery, db-020, from the app) and `restart_database_verification` (user approves fresh-process verification). Event `DbRecoveryEvent` (1 field) emitted to the UI. `recording-recovery.log` diagnostic file written by the app. The same quarantine machinery protects `secrets.sqlite` (db-026).
* Missing parent directory: do not recreate the DB if the parent tree is gone (likely an unmounted volume); otherwise create the parent and an empty DB (base already has the recreate path; the target adds the unmounted-volume refusal, `0xa2856b1`).
Base files: new crate `crates/screenpipe-sqlite-coordinator` (or module in `screenpipe-db`); `write_queue.rs` (classify errors, admission), `db.rs` maintenance tasks; app `src-tauri` (commands, notification). Some of this overlaps the recording-resilience dimension; this spec owns the on-disk marker formats.

### db-020 — `screenpipe db recover` v2 (embedded page-level recovery)

**Kind:** changed. **Confidence:** observed (identifiers/phases), inferred (ordering details). **Evidence:** `cli/db.rs` strings `0x49aae20..0x49ab898`, `0x6a9800d..0x6a99145`; `screenpipe-sqlite-recovery` `0x6ae4940..0x6ae5188`; `screenpipe-db/src/recovery.rs` `0x49866b8..0x49871aa`, `0x49ec280..0x49ecc36`.
Base: shells out to the `sqlite3` CLI `.recover`, requires sqlite3 ≥ 3.38, lock file `.db_recovery.lock` with `{pid, host, started_at, op}`.
Target:
1. **Gate:** recovery runs only when a durable quarantine marker exists (without one, the command reports that no recovery is needed); if the DB verifies healthy, it reports that and skips the rebuild. Hybrid storage is excluded (the command points the user to storage verification or a restore instead, because a hybrid root's descriptor and payload files must be left as they are). Refuses while the app is running (port 3030 open) unless `--force`.
2. **Lock:** `.db_recovery.lock` payload gains `host_id` (`LockPayload { pid, host, host_id, started_at, op }`); foreign-host locks never auto-clear; corrupt-but-recent (<1 h) locks require `screenpipe db unlock --force`; orphaned locks with a confirmed-dead owner are reclaimed. While the lock is held, recording startup stays blocked.
3. **Working area:** `<data>/db-recovery-<YYYYMMDD-HHMMSS>/` containing `working-copy` (read-only hard/sym link to the quarantined DB when possible, else a private copy; sidecars WAL/SHM copied), `candidate.sqlite`, `recovery-manifest.json`. Disk check: need about 2× DB size free outside the quarantine evidence.
4. **Recovery engine:** in-process SQLite recover API (`sqlite3_recover_init` page-level, writes to a destination that must not exist). Then rebuild every external-content FTS5 index discovered via `SELECT name, sql FROM sqlite_schema WHERE type = 'table' AND sql IS NOT NULL AND lower(sql) LIKE '%using fts5%'` using `INSERT INTO <fts>(<fts>) VALUES('rebuild')`; for `frames_fts` specifically: `delete-all` then repopulate from `frames` (rowid, full_text, COALESCE(app_name,''), COALESCE(window_name,''), COALESCE(browser_url,'')) where full_text non-empty.
5. **Verification of the candidate:** switch candidate to `journal_mode=DELETE` (self-contained); fresh file identity must differ from the quarantined identity; `PRAGMA integrity_check` (or `quick_check`), FTS `integrity-check` command per FTS table, initial and final `PRAGMA foreign_key_check` (violations fail), table-count parity vs source (recovery aborts if too many source tables are absent from the candidate), and a **write canary**:
   ```sql
   CREATE TABLE IF NOT EXISTS __screenpipe_recovery_write_canary (token TEXT PRIMARY KEY NOT NULL)
   ```
   delete all rows, insert a random token, commit; if `frames_fts` exists, exercise its update trigger with a no-op `UPDATE frames SET full_text = full_text WHERE id = (latest frame with non-empty full_text)`; close; reopen the file and read the token back (must match); then `DROP TABLE __screenpipe_recovery_write_canary`. Refuse if a live `-wal`/`-shm` sidecar remains next to the verified candidate (installing it would mix two generations).
6. **Install state machine** persisted in `recovery-manifest.json`: `RecoveryManifest { schema_version, phase, live_path, original_identity, candidate_identity, started_at_unix_ms }`, phases (kebab-case): `preparing → candidate-verified → original-archived → candidate-installed → installed-verified → complete`, or `restored-after-failure`. The exact original generation (DB/WAL/SHM) is archived (`source-generation`), the candidate moved into place; on post-install verification failure the original is restored and the failed install archived (`failed-installed-generation`). Interrupted recoveries are resumed/rolled back on next run using the manifest.
7. On success write `resolved-quarantine.json` and remove the quarantine marker; print next steps (start screenpipe; later `screenpipe db cleanup --apply` to reclaim snapshots/archives). `db cleanup` refuses to delete recovery evidence while quarantine is active.
Base files: `crates/screenpipe-engine/src/cli/db.rs` (rewrite `recover`, extend lock payload), new `crates/screenpipe-sqlite-recovery` (needs `libsqlite3-sys` built with the recover extension; SQLite ships it in `ext/recover`, compile flag/feature required), `crates/screenpipe-db/src/recovery.rs`.

### db-021 — Hybrid storage format ("hybrid-parquet-v1")

**Kind:** new. **Confidence:** observed (DDL, identifiers), inferred (file layout details, defaults). **Size:** XL.
**Evidence:** `storage/schema.rs` DDL `0x49e7698..0x49e8251`; frame triggers `0xa2b8bb0..0xa2b9ecf`; revocation `0x49e1e76..0x49e22d0`; reader `0x49e6f48..0x49e752e`; sealing `0x49e9760..0x49e9f76`, `0x51491e8..0x5149310`; descriptor/budget/privacy serde lists `0xa27c4a8`, `0xa27c658`, `0xa2b37f0`, `0xa2b38d8`; format/capability literals `0xa27c0a6..0xa27c3ad`; Parquet schemas `0xa2c46c8`; checks `0x49e2b68`.

Overview (paraphrased): an alternative, opt-in storage format in which the SQLite file becomes an *index* (metadata, FTS, small columns) and the bulky per-frame text payloads are sealed into immutable, checksummed Parquet files. Plain `sqlite` remains the default format; the descriptor's `format` is `"sqlite"` or `"hybrid-parquet-v1"`.

**Descriptor** `storage.json` at the storage root: `StorageDescriptor { format, mode, database_id, generation, source_continuity, index, payloads, capabilities, budget, privacy }`:
* `capabilities` (all required for hybrid): `"parquet-frame-v1"`, `"contentless-delete-fts5"`, `"durable-wal"`, `"parquet-bulk-v1"`.
* `index`: relative path of the SQLite index (`index.sqlite`, or the original `db.sqlite` for in-place migrations); `payloads`: relative payload directory (`payloads`); paths must be relative, confined to the root, and must not contain symlinks.
* `budget` = `StorageBudget { row_group_rows, file_rows, file_bytes, record_bytes, decode_bytes, response_bytes, concurrent_decodes, staging_bytes, disk_reserve_bytes, operation_timeout_secs, lifecycle_timeout_secs, read_attempts }` (numeric defaults not recovered).
* `privacy` = `PrivacyPolicy { identity, required_surfaces }` (identity string of the redaction policy; bitmask of surfaces that must be redaction-complete before sealing).
* The SQLite index stores a copy (`storage_metadata.descriptor`) and must match (a mismatch between descriptor and catalog identity is an open error).
* Root lock `.storage.lock`; `storage-init.json` for fresh hybrid initialization (only allowed on an empty root).

**Hybrid schema migrations** are tracked separately from sqlx in `_hybrid_migrations(version INTEGER PRIMARY KEY, checksum TEXT NOT NULL)`; each version stores a checksum of its schema text and is verified on open (a mismatch is an error, reported separately for the hybrid, bulk, read, resident-frame and recording schemas). Versions observed: 1 (catalog, below), 2 (bulk schema, label prefix `bulk-schema-`, capability string `bulk-v1:element-ranges:frame-group-counts:fts-columnsize-0:column-overrides:audio-sha256-key`), 3 (`:resident-delete-and-redacted-update-v1`, read schema), 4 (`:resident-frames-v1`), 6 (`:recording-v6`, frame triggers). Version 5 not observed. Checksum recipe not recovered (see open questions).

Version 1 DDL (verbatim):
```sql
CREATE TABLE _hybrid_migrations(version INTEGER PRIMARY KEY, checksum TEXT NOT NULL);
CREATE TABLE storage_metadata (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1), descriptor TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0, maintenance INTEGER NOT NULL DEFAULT 0,
    staging_bytes INTEGER NOT NULL DEFAULT 0, staging_limit INTEGER NOT NULL,
    record_limit INTEGER NOT NULL, policy TEXT NOT NULL, required_surfaces INTEGER NOT NULL,
    writer_version TEXT NOT NULL
);
CREATE TABLE upload_bindings (
    destination TEXT PRIMARY KEY, checkpoint TEXT NOT NULL
);
CREATE TABLE payload_files (
    id TEXT PRIMARY KEY, search_path TEXT NOT NULL UNIQUE, detail_path TEXT NOT NULL UNIQUE,
    search_checksum TEXT, detail_checksum TEXT, schema_version INTEGER NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('encoding','published','dirty','retired')),
    row_count INTEGER NOT NULL, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE frame_payloads (
    frame_id INTEGER PRIMARY KEY REFERENCES frames(id) ON DELETE CASCADE,
    generation INTEGER NOT NULL CHECK(generation > 0),
    state TEXT NOT NULL CHECK(state IN ('staged','sealed')),
    file_id TEXT REFERENCES payload_files(id), bytes INTEGER NOT NULL,
    policy TEXT NOT NULL, completed_surfaces INTEGER NOT NULL DEFAULT 0,
    capture_version TEXT, archive_writer_version TEXT,
    attempts INTEGER NOT NULL DEFAULT 0, retry_at INTEGER, last_error TEXT,
    CHECK((state='staged' AND file_id IS NULL) OR (state='sealed' AND file_id IS NOT NULL))
);
CREATE INDEX frame_payloads_file ON frame_payloads(file_id);
CREATE INDEX frame_payloads_staging ON frame_payloads(state,frame_id);
ALTER TABLE frames ADD COLUMN payload_full_text_length INTEGER NOT NULL DEFAULT 0;
ALTER TABLE frames ADD COLUMN payload_accessibility_length INTEGER NOT NULL DEFAULT 0;
ALTER TABLE frames ADD COLUMN payload_full_text_present INTEGER NOT NULL DEFAULT 0;
ALTER TABLE frames ADD COLUMN payload_accessibility_present INTEGER NOT NULL DEFAULT 0;
ALTER TABLE frames ADD COLUMN payload_detail_present INTEGER NOT NULL DEFAULT 0;
DROP TRIGGER IF EXISTS frames_ai;
DROP TRIGGER IF EXISTS frames_au;
DROP TRIGGER IF EXISTS frames_ad;
DROP TABLE frames_fts;
CREATE VIRTUAL TABLE frames_fts USING fts5(full_text,app_name,window_name,browser_url,
    content='',contentless_delete=1,tokenize='unicode61');
INSERT INTO frames_fts(rowid,full_text,app_name,window_name,browser_url)
SELECT id,full_text,COALESCE(app_name,''),COALESCE(window_name,''),COALESCE(browser_url,'')
FROM frames WHERE full_text IS NOT NULL AND full_text != '';
UPDATE frames SET payload_full_text_length=length(COALESCE(full_text,'')),
    payload_accessibility_length=length(COALESCE(accessibility_text,'')),
    payload_full_text_present=full_text IS NOT NULL,
    payload_accessibility_present=accessibility_text IS NOT NULL,
    payload_detail_present=accessibility_tree_json IS NOT NULL OR text_json IS NOT NULL;
```
After v1, a `storage_metadata` row is inserted: `INSERT INTO storage_metadata(singleton,descriptor,staging_limit,record_limit,policy,required_surfaces,writer_version) VALUES(1,?,?,?,?,?,?)`.

**Frame payload accounting:** `bytes` = sum of `length(CAST(col AS BLOB))` over `full_text, accessibility_text, accessibility_tree_json, text_json`. `completed_surfaces` initial value is a bitmask of surfaces that are *empty* (nothing to redact): 1 full_text, 2 accessibility_text, 4 accessibility_tree_json, 8 text_json, 16 window_name, 32 browser_url; the redaction worker ORs in bits as it finishes surfaces.

**Frame triggers (v4/v6 "resident frames"/"recording"):** paraphrase of observed bodies (identifiers exact):
* `hybrid_frame_insert` AFTER INSERT ON frames WHEN `storage_metadata.maintenance=0`: abort with `RAISE(ABORT,'frame storage: record budget exceeded')` (exact string, client-visible) if the row's payload bytes > `record_limit`, and with `'frame storage: staging budget reached; capture admission paused'` (exact string, client-visible) if `staging_bytes + bytes > staging_limit`; insert `frame_payloads(frame_id, generation=1, state='staged', bytes, policy, completed_surfaces, capture_version=writer_version)`; add bytes to `staging_bytes` and bump `revision`; set the five `payload_*` columns; insert into contentless `frames_fts` when `full_text` non-empty.
* `hybrid_frame_sealed_guard` BEFORE UPDATE OF `full_text,accessibility_text,accessibility_tree_json,text_json,app_name,window_name,browser_url` WHEN not in maintenance and the frame's payload is `sealed`: `RAISE(ABORT,'frame storage: sealed frame requires coordinated payload replacement')` (exact string, client-visible).
* `hybrid_frame_update` AFTER UPDATE OF the same columns: adjust `staging_bytes` by new−old bytes, bump `revision`; `frame_payloads.generation += 1`, `bytes` = new, `policy` = current, recompute `completed_surfaces`, reset `attempts/retry_at/last_error`; refresh `payload_*`; re-index `frames_fts` (delete rowid then insert).
* `hybrid_frame_metadata` AFTER UPDATE OF `timestamp,offset_index,device_name,focused,video_chunk_id,frame_name`: bump `revision`.
* `hybrid_frame_delete` BEFORE DELETE ON frames: delete the `frames_fts` row, mark the frame's payload file `dirty`, bump `revision`, subtract staged bytes, delete its `frame_payloads` row.
* Every other user table (non-underscore, non-FTS, non-virtual, excluding `frames`, `frame_payloads`, `payload_files`, `storage_metadata`, `upload_bindings`) gets `hybrid_revision_<table>_INSERT/_UPDATE/_DELETE` triggers that do `UPDATE storage_metadata SET revision=revision+1`, plus `hybrid_read_revoke_delete_<table>` (v3).

**Read revocation (v3):**
```sql
CREATE TABLE IF NOT EXISTS _storage_revocation(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL);
INSERT OR IGNORE INTO _storage_revocation VALUES(1,0);
CREATE TRIGGER IF NOT EXISTS hybrid_read_revoke_policy AFTER UPDATE OF policy,required_surfaces ON storage_metadata
WHEN NEW.policy IS NOT OLD.policy OR NEW.required_surfaces IS NOT OLD.required_surfaces
BEGIN UPDATE _storage_revocation SET revision=revision+1; END;
CREATE TRIGGER IF NOT EXISTS hybrid_read_revoke_payload AFTER UPDATE OF generation ON frame_payloads
WHEN (SELECT maintenance FROM storage_metadata)=1 AND NEW.generation!=OLD.generation
AND (NEW.completed_surfaces!=0 OR NEW.policy IS NOT OLD.policy)
BEGIN UPDATE _storage_revocation SET revision=revision+1; END;
```
plus per-table `hybrid_read_revoke_delete_<table>` AFTER DELETE and `hybrid_read_revoke_redaction_<table>` AFTER UPDATE (WHEN not in maintenance and a redaction column changed) that bump `_storage_revocation.revision`. Readers snapshot `SELECT revision,(SELECT revision FROM _storage_revocation WHERE id=1) FROM storage_metadata WHERE singleton=1` before and after reading Parquet payloads; if either changed the whole query is retried from the start, bounded by `read_attempts`.

**Parquet payload files:** two projections per sealed batch (written with the `parquet` crate; zstd likely):
```
message screenpipe_search_v1 { REQUIRED INT64 id; REQUIRED INT64 generation; OPTIONAL BYTE_ARRAY full_text (UTF8); OPTIONAL BYTE_ARRAY accessibility_text (UTF8); }
message screenpipe_detail_v1 { REQUIRED INT64 id; REQUIRED INT64 generation; OPTIONAL BYTE_ARRAY accessibility_tree_json (UTF8); OPTIONAL BYTE_ARRAY text_json (UTF8); }
```
Files live under the payload root in a date directory (`%Y-%m-%d`) named `search.parquet` / `detail.parquet` (exact nesting inferred: `payloads/<YYYY-MM-DD>/<payload_file id>/{search,detail}.parquet`). Rows sorted by id; each `(id, generation)` must match `frame_payloads`.

**Sealing worker** (`storage/sealing.rs`): select staged frames whose `bytes <= ?` and whose policy equals the current policy and `(completed_surfaces & required_surfaces) = required_surfaces`, ordered by `frame_id`, limited per batch; read payloads; reserve disk (`disk_reserve_bytes`; hitting the temporary reserve pauses sealing and reports a free-space problem); `INSERT INTO payload_files(id,search_path,detail_path,schema_version,state,row_count) VALUES(?,?,?,?,'encoding',?)`; encode and verify; fsync; publish (`state='published'`, checksums set); per frame, re-check generation/state/policy, then `UPDATE frame_payloads SET state='sealed',file_id=?,archive_writer_version=?` and `UPDATE frames SET full_text=NULL,accessibility_text=NULL,accessibility_tree_json=NULL,text_json=NULL`; retire files left without rows. Telemetry fields: `archived_rows, reclaimed_files, cpu_wait_ms, worked_ms, cooldown_ms, active_cpu_percent` (CPU-throttled background lane, `screenpipe-resource` crate). Garbage collection: dirty files are rewritten/retired; `retired` files without references are deleted from disk then `DELETE FROM payload_files WHERE id=? AND state='retired'` (max 32 per pass).
**Payload replacement** (redaction of a sealed frame): within maintenance mode (`maintenance=1`), write new text back to the frame row, recompute `payload_*`, set `frame_payloads` back to `staged` with `generation+1`, `file_id=NULL`, mark the old file dirty (`0x5149310`).
**Policy change:** `UPDATE storage_metadata SET policy=?,required_surfaces=?,revision=revision+1` and reset `frame_payloads.policy/completed_surfaces/attempts/retry_at/last_error`.

**Reader:** frame text APIs fetch by id list: resident columns from `frames` when `staged`, otherwise from Parquet (`search` projection for full_text/accessibility_text, `detail` for tree/text_json) located via `payload_files`; response size capped by `response_bytes` (exceeding it is an error) and decode by `decode_bytes`/`concurrent_decodes`; an estimated size per frame uses `MIN(p.bytes, 4*(payload_full_text_length+payload_accessibility_length))`. Search/timeline queries in hybrid mode replace `f.accessibility_text IS NOT NULL AND f.accessibility_text != ''` with `f.payload_accessibility_length > 0` and `LENGTH(COALESCE(frames.full_text,''))` with `frames.payload_full_text_length` (`0x49d9ba0`, `0x49dcca0`). `/raw_sql` rejects queries needing archived payload columns with an `unsupported-storage-query:` error (`0x49e2d98`).
**Upload bindings:** `SELECT checkpoint FROM upload_bindings WHERE destination=?` / upsert; per-destination sync checkpoint; an independent (restored/forked) database refuses to reuse another database's destination namespace.
**Integrity checks on open:** every frame must have a `frame_payloads` row; every sealed payload's files must exist with supported `schema_version` and `published|dirty` state.
**Restrictions:** vault encryption is unavailable for hybrid roots; SSH remote directory sync requires SQLite format (hybrid transfers via verified backup or SQLite export).

Implementation notes: this is an opt-in subsystem. Minimum viable Freenpipe support: (a) detect `storage.json`/`_hybrid_migrations` and refuse to open with a clear message instead of corrupting; (b) later, full implementation behind a feature flag. All triggers above must be byte-compatible in *behaviour*; names must match exactly so a screenpipe-written hybrid root remains readable.
Base files: new `crates/screenpipe-db/src/storage/` module tree; `Cargo.toml` (parquet, zstd, sha2); `db.rs` read paths.

### db-022 — Bulk archival of large text columns and the `elements` virtual table

**Kind:** new. **Confidence:** observed (DDL/identifiers), inferred (exact column-to-bit mapping). **Size:** XL.
**Evidence:** `storage/bulk/*` strings `0x49e2e78..0x49e6d98`, `0xa2bf078..0xa2c08d8`, `0xa2c37c7..0xa2c4100`; element lifecycle DDL `0x49e4288..0x49e50a1`; vtab `0xa2be80f..0xa2be9e8`, `0xa2c15e0`.

Generic catalog (bulk schema, hybrid version 2):
```sql
CREATE TABLE IF NOT EXISTS _bulk_files(id INTEGER PRIMARY KEY,table_name TEXT NOT NULL,path TEXT NOT NULL UNIQUE,checksum TEXT,state TEXT NOT NULL CHECK(state IN ('encoding','published','dirty','retired')),rows INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS _bulk_legacy_sql(kind TEXT NOT NULL,name TEXT PRIMARY KEY,sql TEXT NOT NULL);
```
For each archivable table: `ALTER TABLE <t> ADD COLUMN _archive_file INTEGER REFERENCES _bulk_files(id)`, `_archive_mask INTEGER NOT NULL DEFAULT <all-bits>`, `_archive_generation INTEGER NOT NULL DEFAULT 1`; indexes on `(_archive_file) WHERE _archive_file IS NOT NULL` and `<t>..._pending ON <t>(id) WHERE _archive_mask!=0`. Triggers and FTS that reference archived columns are saved into `_bulk_legacy_sql` and dropped (`SELECT type,name,sql FROM sqlite_master WHERE (tbl_name=? AND type='trigger' AND instr(sql,?)>0) OR (type='table' AND name=?) OR (?='audio_transcriptions' AND name IN ('idx_audio_transcription_chunk_text','idx_audio_transcriptions_transcription'))`); a contentless FTS `<t>_fts USING fts5(<cols>, content='', contentless_delete=1, tokenize='unicode61')` replaces any content FTS. Per-table triggers `hybrid_bulk_<t>_id` (archived ids immutable: `RAISE(ABORT,'archived record IDs are immutable')` (exact string, client-visible)), `_insert` (staging accounting), `_delete` (mark file dirty, subtract staging), `_revision`, `_fts_metadata`, and a per-column update trigger that sets `_archive_mask |= bit`, `_archive_generation+1`, marks file dirty and adjusts staging bytes. A view `_bulk_logical_<t>` presents the logical row (resident + Parquet values), read through the `screenpipe_bulk` table-valued function / virtual module registered on each connection together with SQL function `screenpipe_payload_sha256(text)`; read connections toggle `PRAGMA query_only` around registration and create `TEMP VIEW`s.
Archivable tables/columns and seal predicates (observed lists, `0xa2c3b55..0xa2c4100`):
| table | archived columns | sealable when |
|---|---|---|
| audio_transcriptions | transcription | `redacted_at IS NOT NULL OR (SELECT required_surfaces=0 FROM storage_metadata) OR transcription=''`; adds `_archive_digest BLOB` and `CREATE UNIQUE INDEX _bulk_audio_identity ON audio_transcriptions(audio_chunk_id,CASE WHEN (_archive_mask&1)!=0 THEN screenpipe_payload_sha256(transcription) ELSE _archive_digest END)` replacing the base dedupe index |
| ui_events | text_content, element_value, element_description, element_ancestors, element_bounds, window_title, browser_url, element_name, element_automation_id | `redacted_at IS NOT NULL OR (SELECT required_surfaces=0 FROM storage_metadata)` |
| semantic_items | body, metadata_json (default `'{}'`), title, actor, status | (not shown) |
| pipe_executions | stdout, stderr, error_message, trigger_event | `finished_at IS NOT NULL` |
| meeting_transcript_segments | transcript | (not shown) |
| outputs | preview, metadata | (not shown) |
Parquet schema per table: `message screenpipe_bulk_<table>_v1 { REQUIRED INT64 id; REQUIRED INT64 generation; OPTIONAL BYTE_ARRAY <col> (UTF8) ...; }` (INT64/DOUBLE for numeric overrides). Files under `bulk-v1/<table>/...parquet`.

Elements conversion (element-specific bulk path):
* Rename original: `DROP TABLE elements_fts; ALTER TABLE elements RENAME TO _bulk_elements_source;` (with `PRAGMA legacy_alter_table=ON`).
* `CREATE TABLE _bulk_element_rows(id INTEGER PRIMARY KEY, <all element columns>, _archive_generation INTEGER NOT NULL,_archive_deleted INTEGER NOT NULL DEFAULT 0,_archive_file INTEGER REFERENCES _bulk_files(id));` copy rows with generation 1, then drop the source (`PRAGMA foreign_keys=OFF/ON` around it).
* Verbatim DDL that follows:
```sql
CREATE TABLE _bulk_element_kinds(id INTEGER PRIMARY KEY,source TEXT NOT NULL,role TEXT NOT NULL,UNIQUE(source,role));
CREATE INDEX _bulk_element_staged_frame ON _bulk_element_rows(frame_id);
CREATE INDEX _bulk_element_staged_file ON _bulk_element_rows(_archive_file) WHERE _archive_file IS NOT NULL;
CREATE TABLE _bulk_element_ranges(first_id INTEGER PRIMARY KEY,last_id INTEGER NOT NULL,file_id INTEGER NOT NULL UNIQUE REFERENCES _bulk_files(id));
CREATE TABLE _bulk_element_frames(frame_id INTEGER NOT NULL,file_id INTEGER NOT NULL REFERENCES _bulk_files(id),PRIMARY KEY(frame_id,file_id));
CREATE INDEX _bulk_element_frames_file ON _bulk_element_frames(file_id);
INSERT INTO _bulk_element_kinds(source,role) SELECT DISTINCT source,role FROM _bulk_element_rows;
CREATE TABLE _bulk_element_groups(frame_id INTEGER NOT NULL REFERENCES frames(id),kind_id INTEGER NOT NULL REFERENCES _bulk_element_kinds(id),visibility INTEGER NOT NULL,is_null INTEGER NOT NULL,rows INTEGER NOT NULL,PRIMARY KEY(frame_id,kind_id,visibility,is_null));
INSERT INTO _bulk_element_groups SELECT e.frame_id,k.id,COALESCE(e.on_screen,0),e.on_screen IS NULL,count(*) FROM _bulk_element_rows e JOIN _bulk_element_kinds k ON k.source=e.source AND k.role=e.role GROUP BY e.frame_id,k.id,COALESCE(e.on_screen,0),e.on_screen IS NULL;
CREATE VIEW _bulk_element_counts AS SELECT g.frame_id,k.source,k.role,CASE WHEN g.is_null=1 THEN NULL ELSE g.visibility END AS on_screen,g.rows FROM _bulk_element_groups g JOIN _bulk_element_kinds k ON k.id=g.kind_id;
CREATE TABLE _bulk_element_checks(parent_id INTEGER PRIMARY KEY);
CREATE TABLE _bulk_element_parent_refs(parent_id INTEGER PRIMARY KEY,rows INTEGER NOT NULL);
INSERT INTO _bulk_element_parent_refs SELECT parent_id,count(*) FROM _bulk_element_rows WHERE parent_id IS NOT NULL GROUP BY parent_id;
CREATE TABLE _bulk_element_state(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL);
INSERT INTO _bulk_element_state VALUES(1,1);
CREATE VIRTUAL TABLE elements USING screenpipe_elements;
CREATE VIEW _bulk_logical_elements AS SELECT * FROM elements;
CREATE VIEW _bulk_element_search_content AS SELECT id,text,role,frame_id FROM elements WHERE text IS NOT NULL AND text!='';
CREATE VIRTUAL TABLE elements_fts USING fts5(text,role,frame_id UNINDEXED,content='_bulk_element_search_content',content_rowid='id',columnsize=0,tokenize='unicode61');
INSERT INTO elements_fts(rowid,text,role,frame_id) SELECT id,text,role,frame_id FROM _bulk_element_rows WHERE text IS NOT NULL AND text!='';
```
  and `storage_metadata.staging_bytes` grows by the staged element bytes. The `sqlite_sequence` row for `elements` is preserved/renamed so AUTOINCREMENT ids continue.
* Read schema (v3) adds `_bulk_element_lookup(id INTEGER PRIMARY KEY,frame_id INTEGER NOT NULL,sort_order INTEGER,kind_id INTEGER NOT NULL,on_screen INTEGER)`, view `_bulk_element_search` (lookup ⨝ kinds), `_storage_read_upgrade(id INTEGER PRIMARY KEY CHECK(id=1),last_id INTEGER)` (backfill cursor, batches of 4096, dropped when complete), triggers `hybrid_read_element_lookup_INSERT/UPDATE` on `_bulk_element_rows`, and `hybrid_read_revoke_element` (bump revocation when `_archive_generation>1`).
* `screenpipe_elements` vtab (rusqlite): declared as `CREATE TABLE x(id INTEGER PRIMARY KEY, <element columns>, _archive_generation INTEGER HIDDEN)`; reads merge staged rows with Parquet ranges found via `_bulk_element_ranges`/`_bulk_element_frames`; supports constraints on `id` and `frame_id`; writes enforce: new id must not exist and must not be reused (ids from `sqlite_sequence`), ids immutable, frame must exist, parent must exist (deferred via `_bulk_element_checks`), NOT NULL/affinity checks; maintain kinds/groups/parent_refs counters and `elements_fts` (`'delete'` command then insert); bump `_bulk_element_state.version`.
* Element sealing: staged rows qualify when `redacted_at IS NOT NULL OR (SELECT required_surfaces=0 FROM storage_metadata) OR (COALESCE(text,'')='' AND properties IS NULL)`; contiguous id ranges are encoded to `bulk-v1/elements/<..>.parquet`, recorded in `_bulk_files` (`table_name='elements'`), `_bulk_element_ranges`, `_bulk_element_frames`, then staged rows deleted. Integrity checks verify ranges are non-overlapping and reference published/dirty files, and that `_bulk_element_groups`/parent refs equal recomputed counts.
* Element queries in hybrid mode read counts from `_bulk_element_counts` and searches from `_bulk_element_search` (`0x49df29d`).
Base files: new `crates/screenpipe-db/src/storage/bulk/`; `db.rs` element insert paths (`flush_level0_bulk`, AX bulk insert) must reserve ids from `sqlite_sequence` (`SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'elements'), 0)`, `0xa2b8625`) and insert with explicit `id` (`INSERT INTO elements (id, frame_id, source, role, ...) VALUES (?,?,'accessibility',...)`, `0xa2b8218`) so the same code works on table and vtab.

### db-023 — Storage migration lifecycle (legacy SQLite → hybrid)

**Kind:** new. **Confidence:** observed (identifiers/messages), inferred (sequence). **Size:** XL.
**Evidence:** `storage/lifecycle.rs` `0x49eaef8..0x49ec1e0`, `0xa276bd8..0xa277270`, `0xa27c748`; `storage/in_place.rs` `0x49ea024..0x49eab88`; `storage/diagnostics.rs` `0x49e1b10..0x49e1c80`, `0xa281150..0xa2818d8`; `storage/parity.rs` `0x49eb0e4`; app `storage_migration.rs` `0x498edf0..0x498f1a8`; frontend `frontend/_next/static/chunks/app/layout-3ec03e167c097e07.js` (startup prompt) and `frontend/_next/static/chunks/app/(main)/settings/page-d9c69007346aaf6a.js` (the database-storage settings card, `data-testid="storage-migration-card"`, `storage-migration-prompt`); bindings `frontend/_next/static/chunks/8001-00ed120106af2449.js`; REA `ev_41e05f198918913c013943637857329ba6946afd02f6a403aa363c58e99412df`.

Tauri commands (exact names/args): `get_storage_migration_status()`, `get_storage_migration_activity()`, `start_storage_migration({root})`, `cancel_storage_migration({root})`, `delete_original_storage_database({root, generation, confirmPermanentDeletion})`, `get_active_data_dir()`.
Status object fields used by the UI: `root, app_session_id, busy, pending, completed, using_new_storage, in_place, can_migrate, can_cancel, can_delete_source, blocked_reason, error, generation, migrated_bytes, bytes_saved, source_bytes`. Activity object: `busy, completed, error, root, message, elapsed_seconds`. The app polls status every 1 s (settings) / 5 s (global prompt) and remembers a per-root postpone choice in localStorage keyed with `app_session_id`.
Two modes:
* **Offline copy**: build a new generation (`index.sqlite` + `payloads/`) beside the original `db.sqlite`; original kept as recovery copy until the user explicitly deletes it.
* **In-place**: convert `db.sqlite` itself into the index (journal tables `_storage_conversion_steps(step TEXT PRIMARY KEY,last_id INTEGER)`, `_storage_conversion_schema(type,name,table_name,sql)`, `_storage_conversion_triggers(name TEXT PRIMARY KEY,sql TEXT NOT NULL,restore INTEGER NOT NULL DEFAULT 0)`; steps `original-triggers`, `suspend-original-triggers`, `conversion-complete`, per-table resumable cursors; stages `building_migration_schema, selecting_resident_frames, waiting_for_frame_writer, waiting_for_frame_connection, staging_frame_payloads, staging_element_rows, deleting_staged_source_elements, committing_element_staging, dropping_empty_element_source, building_final_indexes, counting_converted_records, measuring_reclaimed_space, waiting_for_reclamation_writer/connection`); progress survives interruption (finished steps are not redone).
Preconditions/blocks: not while vault protection is enabled; not while restarting/restoring recording; the data-directory change must be applied first; sleep is prevented during migration; automatic retries disabled after a failure (explicit retry only); insufficient working space leaves the DB unchanged.
Journals and receipts at the root: `storage-migration.json` (`Journal { descriptor, phase, source, snapshot, report, allocated_before_bytes, search_receipts, ... }`, phases `building|ready|paused|active|complete`), `storage-migration-complete.json` (receipt), `storage-migration-diagnostics` persistence (`Snapshot` of 23 fields incl. `schema_version, process_id, database_version, app_version, attempt_id, kind (started|heartbeat|stalled|event), observed_at, table, first_id, last_id, batch_rows, batch_bytes, completed_records, total_records, table_records, elapsed_seconds, idle_seconds, failure_stage, error, source_state`; `SourceState { journal_phase, source_exists, index_exists, active_descriptor_present, active_descriptor_matches, expected_source_bytes, actual_source_bytes, source_modified_matches, source_file_id_matches }`), `MigrationReport { database_id, generation, frames, tables, source_bytes, index_bytes, payload_bytes, source_identity, allocated_after_bytes, ... }`, `RetainedSourceIdentity { bytes, modified, file_id }`, `TableParity { table, rows, sha256 }`.
Verification: per-table logical parity (rows read in id order in batches of 128 and hashed with SHA-256, compared source vs converted), FTS search parity (sample `full_text` of min/max frame, tokenize `[\p{L}\p{N}]+`, run `frames_fts MATCH` and compare the top-32 ids), `PRAGMA integrity_check`, source must have no live WAL/journal and unchanged identity (size, mtime, file id) before rename; on a mismatch neither file is removed, so both remain available for diagnosis. Activation writes `storage.json`, reopens on the migrated storage (`SELECT id FROM frames LIMIT 1` probe), and saves the completion receipt. Secrets present in the source DB are recovered into the secret store during migration (`recovering_migration_secrets`). Deleting the original requires the running storage to be the new one, the receipt to match, no WAL/journal on the original, and explicit confirmation.
Implementation note: Freenpipe can defer this (keep SQLite-only) while still recognising a migrated root (db-021 detection). If implemented, keep it opt-in and local-only; no telemetry (the target's migration-telemetry upload path must not be ported).

### db-024 — Hybrid maintenance: compact, reclaim, backup

**Kind:** new. **Confidence:** observed (identifiers), inferred (algorithms). **Evidence:** `storage/maintenance.rs` `0x49e1c80`, `storage/reclaim.rs` `0x49e943f..0x49e9698`, `0xa27cbf8`; `storage/backup.rs` `0x51487f8..0x5148a08`, `0x52d5e70`.
* **Compact (offline index maintenance):** builds `compacting.sqlite` as a candidate, journal `storage-maintenance.json` (`CompactJournal { candidate, ... }`), completion `storage-maintenance-complete.json`; while pending, the index cannot be opened (the error tells the user to either finish the compaction or cancel the candidate that was never activated); cancellable before activation (`compact_cancel`). Online `/data/compact` VACUUM is refused for hybrid indexes (the refusal points to the offline storage-compact command instead).
* **Reclaim (in-place):** requires exclusive offline ownership (`locking_mode=EXCLUSIVE`); checkpoint `PRAGMA wal_checkpoint(FULL)` must complete; reads `PRAGMA page_count`/`freelist_count`, walks the freelist trunk pages (validating trunk lengths, no repeats), and punches sparse holes over free pages (`scanning_free_pages, reclaiming_disk_extents, syncing_reclaimed_database`); if the filesystem does not support/report sparse savings, keeps free pages and enforces the migration disk reserve.
* **Backup:** hybrid backup bundle directory `screenpipe-backup-*` containing `VACUUM INTO` snapshot of the index plus copies of all referenced payload files (`SELECT DISTINCT pf.search_path,pf.detail_path FROM payload_files pf JOIN frame_payloads p ...`) and published/dirty bulk files, `storage.json` and `manifest.json` (path, bytes, sha256 per file); destination must not exist; bounded by a deadline.
Base files: new `storage/{maintenance,reclaim,backup}.rs`.

### db-025 — `POST /data/compact` (full VACUUM)

**Kind:** new. **Confidence:** observed. **Evidence:** route doc `0x6a6c9a6..0x6a6cb13`; handler strings `0x5151ab3..0x5151c7b`, `0x5141f60` (`PRAGMA busy_timeout = 60000`, `VACUUM`, `PRAGMA busy_timeout = 5000`); `CompactResponse` (`0x6a6c4f0`).
Behaviour: explicit user action to shrink `db.sqlite` after deletions. Pre-check free space ≈ 2× current DB size (`required_free_space`, `available_free_space`; refuse if insufficient), raise `busy_timeout` to 60000, run `VACUUM` (exclusive; recording writes pause), restore `busy_timeout` 5000; respond with `size_bytes`/`bytes_before`/`bytes_after`/`bytes_reclaimed`. Rejected for hybrid indexes (db-024). Existing base routes `/data/checkpoint`, `/data/backup`, `/data/evict-media`, `/data/storage-preview`, `/data/delete-range`, `/data/device-storage` remain.
Base files: `crates/screenpipe-engine/src/routes/data.rs`, `crates/screenpipe-db/src/db.rs` (vacuum fn), server router.

### db-026 — Secret store moved to `secrets.sqlite`

**Kind:** changed. **Confidence:** observed. **Evidence:** `screenpipe-secrets/src/store.rs` DDL `0x49af280..0x49afa1c`; file names `0x49873c0`, `0xa23c560`; quarantine messages `0x49aff48`.
DDL in the separate `secrets.sqlite` (rollback-journal mode, `secrets.sqlite-journal`):
```sql
CREATE TABLE IF NOT EXISTS secrets (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL,
                nonce BLOB NOT NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                expires_at TEXT
            )
CREATE TABLE IF NOT EXISTS credential_refresh_leases (
                    key TEXT PRIMARY KEY,
                    owner TEXT NOT NULL,
                    expires_at INTEGER NOT NULL
                )
CREATE TABLE IF NOT EXISTS secret_store_metadata (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                )
```
Behaviour: on first open, if `secret_store_metadata` lacks key `legacy_main_db_migrated_v1`, copy every row of the legacy `secrets` table in `db.sqlite` (if present and the main DB is not quarantined) with an upsert that keeps the newer `updated_at`, then insert `('legacy_main_db_migrated_v1','complete')`. Refresh leases: acquire with upsert that succeeds only if the existing lease expired (`expires_at <= unixepoch()`) or is owned by the caller; release deletes by key+owner. The secrets DB participates in quarantine (db-019). Vault encrypts `secrets.sqlite` and its journal too.
Base files: `crates/screenpipe-secrets/src/store.rs` (open own pool on `<data>/secrets.sqlite`), vault manager file list. Overlaps the secrets dimension; schema recorded here.

### db-027 — Removed: `MigrationWorker` / `migration_progress`

**Kind:** removed. **Confidence:** observed (absence of `migration_progress` string; target migration comment states the worker was never started). **Evidence:** no `migration_progress` in `app_strings.tsv`; `0xa2a3f29`.
Action: delete `crates/screenpipe-db/src/migration_worker.rs` and its exports; its backfill (ocr_text → frames) is performed by migration 20260613130000 (db-005).

### db-028 — Data-directory layout additions

**Kind:** changed. **Confidence:** observed names, inferred placement where noted.
Paths relative to the data dir (`~/.screenpipe` or the configured data dir; `get_active_data_dir`):
| path | purpose | item |
|---|---|---|
| `db.sqlite`, `db.sqlite-wal`, `db.sqlite-shm` | main DB (unchanged) | — |
| `secrets.sqlite`, `secrets.sqlite-journal` | secret store DB | db-026 |
| `db.sqlite.quarantine.json`, `.quarantine.reserve.json`, `.verification-pending.json` (suffixes appended to the DB path, inferred) | hard-fault quarantine | db-019 |
| `.db_recovery.lock` | recovery lock (adds `host_id`) | db-020 |
| `db-recovery-<YYYYMMDD-HHMMSS>/{working-copy,candidate.sqlite,recovery-manifest.json}`, `resolved-quarantine.json` | protected repair | db-020 |
| `recording-recovery.log` | app recovery diagnostics | db-019 |
| `sqlite-planner-maintenance.json` | planner stats report | db-017 |
| `storage.json`, `.storage.lock`, `storage-init.json`, `storage-migration.json`, `storage-migration-complete.json`, `storage-migration-diagnostics`, `storage-maintenance.json`, `storage-maintenance-complete.json` | hybrid descriptor and journals | db-021/023/024 |
| `index.sqlite`, `payloads/<YYYY-MM-DD>/.../{search,detail}.parquet`, `payloads/bulk-v1/<table>/*.parquet`, `compacting.sqlite` (under a generation directory, inferred `storage/<generation>/`) | hybrid generation | db-021/022/024 |
| `recovered-<..>-source-<..>.sqlite` | retained source after migration recovery | db-023 |
| `screenpipe-backup-*/{storage.json,manifest.json,...}`; `backup.screenpipe`/`backup.sqlite` | backups | db-024 |
| `outputs/` | outputs registry file copies | db-002 |
Base files: `crates/screenpipe-config` (path helpers), app `disk_usage.rs` (count `secrets.sqlite*` in database size; skip hybrid payloads under media), `cli/db.rs` cleanup (never delete active quarantine evidence).

### db-029 — SQLite toolchain upgrade

**Kind:** changed. **Confidence:** observed versions; inferred necessity. **Evidence:** `crates.txt`; source id `0xa32f090`; version guard `0xa2ea907`; FTS5 `contentless_delete=1` usage.
* `libsqlite3-sys` 0.26 → a version bundling SQLite ≥ 3.51.3 (target uses 0.37.0, source id 2026-03-13). Needed for: WAL-reset bug fix (guard in db-016), FTS5 `contentless_delete=1` (SQLite ≥ 3.43), `sqlite3_recover` (db-020; enable the recover extension), `sqlite_dbpage` vtab.
* `sqlx` 0.7.2 → 0.9.0 (target); check `_sqlx_migrations` compatibility (unchanged columns) and that the checksum-mismatch error text used by `fix_migration_checksums` (sqlx's `was previously applied but has been modified`; exact string, client-visible) still matches.
* `rusqlite` 0.39 with `vtab` feature for `screenpipe_elements`/`screenpipe_bulk`; `parquet` 59.3 (+ zstd) for payload files; `sha2` for receipts/digests. All must share one SQLite build.
Base files: `crates/screenpipe-db/Cargo.toml`, workspace `Cargo.toml`/`Cargo.lock`.

## 3. Implementation order (recommendation)

1. db-029 (toolchain), db-001 (runner), then schema migrations db-002..db-015 in version order (pure DDL, low risk), with db-005 code changes and db-027 removal.
2. db-016, db-018, db-017 (runtime robustness).
3. db-019, db-020, db-026 (damage handling).
4. db-025, db-028.
5. db-021..db-024 behind a feature flag; until then, detect hybrid roots and refuse to open them.

## 4. Open questions

* `_hybrid_migrations.checksum` recipe (hash function and exact input text per version) — needed to open screenpipe-written hybrid roots without a checksum-mismatch failure.
* Numeric defaults of `StorageBudget` and `storage_metadata.staging_limit`/`record_limit`; policy `identity` string format; `required_surfaces` default bits.
* Exact on-disk nesting of generation and payload directories (`storage/<generation>/...`) and payload file naming.
* `StorageDescriptor.mode` value set and `source_continuity` semantics.
* Exact column→bit mapping of `_archive_mask` per bulk table and seal predicates for semantic_items/meeting_transcript_segments/outputs.
* WAL maintenance thresholds (checkpoint interval, backlog limit) and planner-maintenance cadence/`analysis_limit` value.
* Whether a fresh install in v2.7.104 initialises hybrid storage by default (`storage-init.json` exists) or only via the migration prompt.
* Quarantine sidecar exact naming (suffix appended to `db.sqlite` vs separate directory `sqlite-quarantine`).

## Verification

Adversarial verifier pass, 2026-10-14. For every item I checked (a) whether the base (`<repo>` @ 892199f) already has it, by grepping identifiers across `crates/` and `apps/`, and (b) whether the cited target evidence exists, using raw-byte dumps of the exe at each cited offset (not only `app_strings.tsv`, which splits multi-line SQL at newlines, so several cited offsets land 1 byte before the line start; the statements are there).

Independent tooling, all in `<re-workspace>/tools/`:
* `verify_migs.py`: my own PE parser (VA to file offset) that walks the sqlx array at `0xa2ae3a8` (stride 0x58). Result in `work/verify_migs.txt`, SQL bodies in `work/verify_migs/`. It found 117 entries, sorted and unique. 89 are byte-identical to base (SHA-384), none of the base versions is missing, and 28 are new. For all 28, the embedded 48-byte checksum equals SHA-384 of the embedded SQL and matches the table in db-001. Every DDL statement quoted in db-002..db-015 matches the embedded SQL once comments are stripped.
* `verify_ddl_sweep.py`: every `CREATE TABLE/INDEX/TRIGGER/VIEW` and `ALTER TABLE ... ADD COLUMN` object name in the raw exe, minus every object defined anywhere in the base. That leaves 146 target-only objects, all named in this spec except SQLite-internal `t1`/`recovery`/`x` (recover extension and vtab declarations). Output: `work/ddl_sweep.txt`.
* `verify_ddl_reverse.py`: base DDL objects whose names never occur in the exe. Only `migration_progress` (db-027) and the cloud-side `packages/ai-gateway` tables (not shipped in the desktop app) come up. Output: `work/ddl_reverse.txt`.
* `verify_offs.py` and `verify_raw.py`: offset spot checks. Dumps are in `work/vr*.txt`.
* REA: parent `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`. The string traces for `/data/compact` (`ev_31ee53fc54951d1498d1c7900aba94995d52dfebf877e4399b7b754f3900dc1d`) and `start_database_recovery` (`ev_97d539f7d61628bd54067ff6a6dd3f0ce07d40aa17724fb7a7d75062f105f2a2`) returned no-match, because the graph does not index those literals. A module trace of the bindings chunk `8001-00ed120106af2449.js` (`ev_726906e03c96fa69a5c86e375c5dd06b9079ae2d24db619b1ed8dd974ed6f8b9`, 17 nodes / 31 edges) confirms that the shared module carrying the storage-migration and recovery commands is imported across the app. A direct grep of `frontend/` finds `start/cancel/get_storage_migration_*`, `delete_original_storage_database`, `get_active_data_dir`, `start_database_recovery` and `restart_database_verification` in that chunk, and `/data/compact` in `_next/static/chunks/app/(main)/settings/page-d9c69007346aaf6a.js`.

| id | verdict | note |
|---|---|---|
| db-001 | confirmed | Array re-walked independently (see above). Base already has `set_ignore_missing(true)`, the checksum rewrite-and-retry (`db.rs:403-418`, `fix_migration_checksums` at :501) and the frames_fts notice (:494). The new parts are the 28 files plus the ocr_text notice (`0x49dbe02`, string verified). Small correction: the last entry *starts* at `0xa2b0b88`, and the array ends at `0xa2b0be0`. |
| db-002 | confirmed | 4 migrations match verbatim; no `outputs`/`output_search_*` in the base. Query evidence `0x51a6920` (output_search_index/fts delete+insert) was verified. |
| db-003 | confirmed | Migration 20260613000000 verbatim. The base redact worker (`crates/screenpipe-redact/src/worker/tables.rs`) has no `elements` surface. |
| db-004 | confirmed | 20260613000001/20260619000000/20260619120000/20260627000000/20260709000000 verbatim; the six partial indexes are exactly as listed. None of the 5 columns is in the base. |
| db-005 | confirmed | 20260613130000 verbatim (5 backfills + 5 trigger drops + fts/table drop). Late-OCR `UPDATE frames SET full_text = CASE ... text_json = CASE ...` was verified at `0x5143218`. Nuance: the base *already* writes `frames.full_text` for OCR and AX frames (`write_queue.rs:882,1303`), so the code delta is `frames.text_json` + removing all `ocr_text` writers/readers (40 refs in `db.rs` alone). `'' as ocr_engine` is actually at `0x49dd429`; the cited `0x49dd353` is in the same statement, a bit earlier. |
| db-006 | confirmed | 4 migrations verbatim; none of the indexes is in the base. |
| db-007 | confirmed | Migration verbatim. `0x52435d0` insert/latest-select verified. The `element_ancestors` select list is at `0x49d969a` (cited off by one). Missed detail: time-range deletion prunes `display_layout` but keeps the newest in-range row (see db-m02). |
| db-008 | confirmed | Migrations verbatim (20 objects). GC deletes for runs/items verified at `0x5145400`/`0x5145510`. |
| db-009 | confirmed | Migrations verbatim. Insert with `trigger_event, trigger_key` verified at `0xa2b6bb8`, prune `DELETE FROM pipe_event_runs WHERE claimed_at < ?` at `0xa2b6ec5`. |
| db-010 | confirmed | Migration verbatim. `0x51a6cb8` (local-device delete) and `0x51a6d50` (natural-key lookup) verified. |
| db-011 | confirmed | Migration verbatim. Self-heal strings verified at `0x49db209`/`0x49db270`/`0x49db3cd`, and the speaker-kept log message at `0x514584a`/`0x5145860`. The base `delete_speaker` (`db.rs:6146`) unconditionally deletes transcriptions, embeddings and the speaker, so the behaviour really changed. |
| db-012 | confirmed | Migration verbatim; `0x4983bc8` and the unique-violation literals at `0xa2b8742` verified. |
| db-013 | confirmed | 3 migrations verbatim (5 tables, 9 indexes incl. producer_range, 6 triggers). The reconciler's windowed deletes are at `0x61fa088`. |
| db-014 | confirmed | Migration verbatim. `mts.session_speaker_id` at `0x49e0239` and the `m.stream_id` live-evidence CTE at `0x524a909` were verified (both cited 1 byte early). |
| db-015 | confirmed | Migration verbatim. The json_each membership query and the `WITH clipped AS` islands query were verified at `0x5248261`..`0x52484d0`. |
| db-016 | confirmed | Literals verified (`0x49daec1` pragma pack with `wal_autocheckpoint`,`0`; `query_only`,`ON`; `wal_checkpoint(RESTART)`; `temp_store=FILE`; PASSIVE in `db/maintenance.rs` at `0x5140340`; preflight messages at `0xa279e40`; the 3.51.3 guard at `0xa2ea907`). The base matches the "base" column (`db.rs:307-320`, TRUNCATE at :372, 5-min loop at :8665). The base calls `sqlite3_auto_extension` without checking rc (:282). Point 9 (startup `quick_check(1)`) is already in the base (:8716), so it is not a delta. The spec misses the ownership-file name: the literal `.owner` at `0x5232222` sits next to the ownership messages (`0x523222a`), so the sidecar is `<db>.owner` (exact join inferred). |
| db-017 | confirmed | `planner_stats.rs` strings verified (`0xa2ca410` analysis_limit/busy_timeout=0/`ANALYZE main.sqlite_schema`, `0xa2ca718` optimize=0x10003, `0xa2ca750` planner_refresh cookie bump), and the report fields at `0xa280d88`. Nothing comparable exists in the base (`analysis_limit` 0 hits). |
| db-018 | confirmed | Messages verified (`0x49d9358` connection-already-released message, `0x49d93c9` cleanup-task failure, `0xa2b5858` connection closed, `0xa2b8188` abandoned read txn rolled back). The base only has `heavy_read_semaphore`. The `sqlite3_interrupt` call is inferred; no literal evidence. |
| db-019 | confirmed | Coordinator strings verified (`0xa2e8c67`, marker fields at `0xa2e9500`, `.quarantine.json` at `0xa2e97b6`, `.verification-pending.json` at `0xa2e98e8`, sidecar trio at `0xa281bd0`). **Correction:** the "unmounted-volume refusal" bullet (`0xa2856b1`) is **already in the base**: `write_queue.rs:1562-1586` `ensure_db_parent_dir` emits the identical message. The base also already classifies malformed/IOERR/NOTADB as fatal and recycles the connection (`sqlite_error.rs::is_fatal_sqlite_message`). New: durable marker sidecars, per-generation writer admission, fresh-process verification, app commands and notification. |
| db-020 | confirmed | Strings verified (`0x49aae20` working-dir names, `0x49ab261` canary/identity verification, `0x6a982a9` phase list, `0x6a984bf` manifest + `LockPayload` fields, `0x49ab4c8` resolved-quarantine.json). **Correction:** spec steps 1-2 restate base behaviour. The base `cli/db.rs` already refuses while port 3030 is open unless `--force` (:299-308), treats locks older than 1 h as stale (:56), handles foreign-host locks (:99,:140), has `db unlock --force` and `db cleanup --apply`, and needs about 2x disk (:333-339). The real deltas are `host_id`, the quarantine gate, orphan-owner verification, in-process recover, the manifest state machine, the canary and FTS rebuild, and removal of the `SCREENPIPE_IGNORE_DB_LOCK` bypass (db-m01). |
| db-021 | confirmed | v1 DDL verified at `0x49e7698`; byte accounting at `0xa2b8bb0`, `hybrid_frame_insert` at `0xa2b90db`, revocation DDL at `0x49e1fd6`. None of it is in the base. |
| db-022 | confirmed | `_bulk_audio_identity` at `0x49e2e78`, `_bulk_files` at `0x49e3586`, element kinds at `0x49e4288`, staged indexes/ranges at `0x49e4630` all verified; none in the base. |
| db-023 | confirmed | `_storage_conversion_*` at `0x49ea368`, `index.sqlite`/`payloads` at `0x49eb390`, FTS parity sampling at `0x49eb758`, `storage-migration.json` at `0xa276bd8`. The frontend command set is confirmed (see REA/grep above). None of it is in the base. |
| db-024 | confirmed | `storage-maintenance.json` at `0x49e1c80`, freelist reclaim stages at `0x49e94c8`, sparse-probe messages at `0xa27cbf8`, `screenpipe-backup-` at `0x51487f8`. None of it is in the base. |
| db-025 | confirmed | The route doc (`0x6a6c9a6`), free-space messages (`0x5151ab3`/`0x5151b35`, incl. the hybrid refusal) and pragma triple (`0x5141f60`) were verified, and the settings page calls `/data/compact`. The base router (`server.rs:723-730`) has no compact route. The base's only VACUUM is inside `repair_database()` (`db.rs:7405`), which is dead code with no callers; the target dropped it (0 hits for its log strings). The target's `busy_timeout = 60000 / VACUUM / 5000` triple is a new literal sequence without semicolons, not the old one. |
| db-026 | confirmed | DDL verified (`0x49af280`, `0x49af868`, legacy-copy gate at `0x49afa1c`, `secrets.sqlite(-journal)` at `0x49873c0`). The base `SecretStore::new` (`crates/screenpipe-secrets/src/store.rs:18-31`) creates the identical `secrets` table on the *main* pool, and the base has no leases or metadata tables. |
| db-027 | confirmed | Raw-byte search: 0 occurrences of `migration_progress`. `MigrationWorker` appears only in the 20260613130000 comment (`0xa2a3f29`). In the base it is exported (`lib.rs:18-20`) but never constructed outside its own module, which is consistent with "never started". |
| db-028 | duplicate | This is a roll-up of paths that db-002/017/019/020/021/023/024/026 already own. Its only extra point (disk-usage counting `secrets.sqlite*`, `0x49873c0`) is already cited under db-026. Fold it into those items; also add the `<db>.owner` sidecar to db-016. |
| db-029 | confirmed | `crates.txt`: libsqlite3-sys-0.37.0, sqlx-core/sqlx-sqlite-0.9.0, rusqlite-0.39.0, parquet-59.3.0, zstd-safe-7.3.0. Base: sqlx 0.7 / libsqlite3-sys 0.26 (`crates/screenpipe-db/Cargo.toml:13,20`), Cargo.lock rusqlite 0.29.0, sqlx-core 0.7.2. The source id `2026-03-13 10:38:09` at `0xa32f090` and the 3.51.3 guard at `0xa2ea907` were verified. Nuance: `sha2 0.10.9` is already in the base Cargo.lock, so only wiring it into screenpipe-db is new; `parquet`/`zstd` are genuinely new. |

### Missed items

* **db-m01 [removed] `SCREENPIPE_IGNORE_DB_LOCK` startup bypass removed; recovery-lock gate hardened.** The base app (`apps/screenpipe-app-tauri/src-tauri/src/main.rs:382-404`) lets `SCREENPIPE_IGNORE_DB_LOCK=1` bypass the `.db_recovery.lock` startup check. The target exe contains no `IGNORE_DB_LOCK` at all (raw bytes, ASCII and UTF-16 both 0 hits). Its gate instead verifies orphaned locks: it removes a lock only when the owner is confirmed dead, re-checks that the owner did not change during verification, never auto-clears foreign-host or unverifiable locks, and keeps recording startup blocked so the current database generation stays protected. Evidence: `0x6a98c00..0x6a98e49` (log messages for removing a verified orphan lock, for an owner whose death cannot be confirmed, and for refusing to clear a lock from another host). Size S.
* **db-m02 [changed] Time-range deletion also prunes `display_layout` (keeps the newest in-range row) and drops the OCR counter.** The target's `delete_time_range` deletes `display_layout` rows in the range except the most recent one (`... AND id NOT IN (SELECT id FROM display_layout WHERE timestamp BETWEEN ?1 AND ?2 ORDER BY timestamp DESC, id DESC LIMIT 1)`), so the layout context stays reconstructible after a deletion. The commit log no longer reports `ocr=` (the base logs it at `db.rs:6388`). Evidence: `0x5140d13`, region `0x5140780..0x5140f00`, log string `0x5140e14`. Size S.
* **db-m03 [new] DatabaseManager ownership sidecar `<db>.owner`.** db-016 point 8 describes the guard but not the file. The literal `.owner` is packed next to three ownership error messages: same process already holding a live `DatabaseManager` for the path, ownership file cannot be opened, and database owned by another process that must close it before storage changes (`.owner` at `0x5232222`, messages from `0x523222a`). Add it to the data-dir layout and to cleanup exclusions. Size XS.

## Verification (second pass, 2026-10-15)

Second adversarial pass, done independently of the first-pass section above. For each item I checked (a) whether the base already has it, by grepping `<repo>` (crates, apps, packages; build output skipped) for its identifiers, and (b) whether the cited target evidence exists, by dumping raw exe bytes at every cited offset. I did not rely on `app_strings.tsv` alone.

New tooling (all in `<re-workspace>/tools/`, outputs in `work/`):
* `v2db_migs.py`: re-finds the sqlx array *without* a given start offset. It searches for the LE-i64 of the first base version and gets a single hit, the version field at `0xa2ae3f0`, which puts the struct at `0xa2ae3a8`. It then probes strides, keeps 0x58, and walks the array: 117 entries, sorted and unique. 89 match base byte-for-byte (SHA-384), no base version is missing, and 28 are new. Every embedded 48-byte checksum equals SHA-384(SQL) and matches the db-001 table. SQL is in `work\v2db_migs\`.
* `v2db_ddlcheck.py`: takes every SQL statement quoted in db-002..db-015, strips comments, normalises whitespace and looks it up in the extracted migrations. All 92 statements were found. In reverse, every DDL object in the 28 migrations is named in the spec (0 unquoted).
* `v2db_basegrep.py` + `work\v2db_idents.txt`: base hit counts for about 110 delta identifiers.
* `v2db_sweep.py` and `v2db_sqlidents.py` (outputs `work\v2db_sweep.txt`, `work\v2db_sqlidents.txt`): sweep the target strings for DDL, `ADD COLUMN`, PRAGMAs, file names and snake_case identifiers used in SQL, then subtract base and spec. Every remaining target-only DDL/column name is already in the spec. What is left is SQLite-internal (recover extension, FTS3/RTree, sqlite-vec shadow tables) or query CTE aliases owned by other dimensions (activity-summary attribution `0x522bc2c..0x522c9c7`, tag facets `0x5246629`).
* `v2db_reverse.py`: lists base tables, indexes and triggers that the target references only inside migration SQL. Besides migration-only indexes and triggers (expected), only `migration_progress` (0 bytes anywhere) and the `ocr_text*` family come up.
* `v2db_rawfind.py` / `v2db_offs.py`: raw presence/absence checks (ASCII and UTF-16LE) and offset dumps.
* REA: the module trace of the settings page `app/(main)/settings/page-d9c69007346aaf6a.js` is `ev_f95e14984a9317611427c97745e4a4d093b2ad959845fe7120198b94f4fdd526` (3 nodes / 2 edges, child of `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`; saved to `work\v2db_rea_settings_trace.json`). It links the page to the shared bindings chunk `8001-00ed120106af2449.js`. The graph does not index the command literals, so I grepped `frontend/` directly. Chunk 8001 contains each of `get_storage_migration_status`, `get_storage_migration_activity`, `start_storage_migration`, `cancel_storage_migration`, `delete_original_storage_database`, `get_active_data_dir`, `start_database_recovery` and `restart_database_verification` once. The settings page contains `data/compact`, `bytes_reclaimed` and `storage-migration-card`. The base has 0 hits for all of these.

| id | verdict | note |
|---|---|---|
| db-001 | confirmed | Array re-found independently (see above). The base already has `set_ignore_missing(true)` (db.rs:403), checksum rewrite+retry (:413-420, `fix_migration_checksums` :501) and the frames_fts notice (:494). New: the 28 migrations and the ocr_text notice (`0x49dbe02`). Each new body starts with an upstream header comment that sits inside the checksummed text (`0xa2a1998`), so Freenpipe cannot reproduce the target checksums with its own text. The rewrite path (or hand-pinned checksums) is the only interop route, as the item says. |
| db-002 | confirmed | DDL verbatim in 20260601000000/20260612000000/20260622000000/20260729130000 (ddlcheck). Base: 0 hits for `outputs` DDL, `output_search_index`, `output_search_fts`, `saf_kind`. Sidecar/FTS reindex statements verified at `0x51a6920`. |
| db-003 | confirmed | 20260613000000 verbatim. The base worker surfaces are `ocr_text`, `audio_transcriptions`, `frames.accessibility_text` and `ui_events` x2 (`crates/screenpipe-redact/src/worker/tables.rs:62-68`); there is no `elements` surface and 0 hits for `idx_elements_redacted_at`. |
| db-004 | confirmed | The 5 columns have 0 base hits. 20260709000000 (extracted SQL) does DROP + CREATE ... `WHERE <col> IS NULL` for all six indexes, including base's full `idx_frames_accessibility_redacted_at` (20260501000000). The derived-copy propagation is paraphrased from the 20260619000000 comment (`0xa2a544e`). |
| db-005 | confirmed | 20260613130000 = `ADD COLUMN text_json` + 5 backfills + 5 trigger drops + `ocr_text_fts`/`ocr_text` drops (extracted SQL). Raw scan: 216 `ocr_text` hits; only 21 are outside migration bodies, and all of those are result aliases (`... as ocr_text`), field-name lists, the retirement log notice or stale LLM-prompt text (`0xa0cf1a0`, do not copy). There is no runtime INSERT/SELECT on the table. Base nuance: it already writes `frames.full_text` (write_queue.rs:882,1303) but also inserts into `ocr_text` (:935,:1130,:1323) for `text_json`/`ocr_engine`. So the code delta is to move `text_json` onto `frames` and delete every `ocr_text` writer and reader. |
| db-006 | confirmed | The 5 indexes have 0 base hits. **Correction:** the base meeting-segment queries *already* use `julianday(captured_at)` (db.rs:4284-4290, 4372-4373, 9613, 9765), so the "Base files: rewrite comparisons" note is moot and only the expression index is new. |
| db-007 | confirmed | Migration verbatim; `0x52435d0` insert/latest verified; `element_ancestors`/`display_layout` have 0 base hits. Cross-dimension overlap: `vision.md` (`vision-display-layout`) carries the same DDL plus the delete-range rule that keeps the newest in-range row (`0x5140d00` verified). Pick one owner when implementing. |
| db-008 | confirmed | 20 objects verbatim. GC deletes at `0x5145400` (runs) and `0x5145510` (items); actor alias/assignment updates at `0x51a7749`/`0x51a778c`. Base: 0 hits for `semantic_*`. |
| db-009 | confirmed | `INSERT OR IGNORE INTO pipe_event_runs` at `0xa2b6e17`, prune `DELETE ... WHERE claimed_at` at `0xa2b6f10`, executions insert at `0xa2b6bb8`. Base: 0 hits for all four identifiers. Extra detail the item omits: a claim can also be released by exact key (`DELETE FROM pipe_event_runs WHERE pipe_name = ? AND event_name = ? AND event_key = ?`, `0xa2b6ea9`), presumably when the run fails to start (inferred). |
| db-010 | confirmed | `0x51a6cb8` (local-device delete), `0x51a6d50` (natural-key lookup), `ON CONFLICT(id) DO UPDATE` upsert at `0x51a6f00`. Base: no `feedback` table. |
| db-011 | confirmed | Self-heal at `0x49db209`/`0x49db270`; five-table reference list at `0x5145680`. Base `delete_speaker` (db.rs:6146) unconditionally deletes the transcriptions, embeddings and speaker row, so the behaviour really changed. |
| db-012 | confirmed | `0xa2abdc4` migration, `0x4983bc8` lookup, `0xa2b8742` unique-violation literals in `db/meetings.rs`. Base: 0 hits for `calendar_event_id`. |
| db-013 | confirmed | 3 migrations verbatim (ddlcheck), module path at `0x52d3ba0`. Base: 0 hits for `activity_tasks/intervals/ledger` and `deterministic-v1`. |
| db-014 | confirmed | Migration verbatim; `m.stream_id` live-evidence CTE at `0x524a908`. Base: 0 hits for `stream_id`/`session_speaker_id`. |
| db-015 | confirmed | `json_each` membership at `0x524842b`, islands at `0x52484d0`, `has_audio` listing at `0x51a6b60` (the cited `0x51a6920` is the output-search run that comes just before it). Base: 0 hits. |
| db-016 | confirmed | Verified: pragma pack at `0x49daec1` (`wal_autocheckpoint` followed by the literal `0`; the value is inferred), `query_only`/`ON` at `0x49daf37`, RESTART at `0x49daf78`, `temp_store=FILE` at `0x49db0c0`, PASSIVE in `db/maintenance.rs` at `0x5140340`, preflight and sqlite-vec rc messages at `0xa279e40`, and the 3.51.3 guard at `0xa2ea870`. The ownership sidecar is named `.owner` (`0x52321a0` region). Base: `wal_autocheckpoint` 4000 (db.rs:320), TRUNCATE at boot (:372) and every 5 min (:8665-8673), `sqlite3_auto_extension` rc ignored (:282). Point 9 (`quick_check(1)`) is already in base (:8716) and is not a delta. |
| db-017 | confirmed | `0xa2ca410`, `0xa2ca718`, `0xa2ca750` and the report fields at `0xa280d88` verified. Base: 0 hits for `analysis_limit`, `_screenpipe_planner_refresh` and `sqlite-planner-maintenance`. The `optimize_on_close` remark is inference only (0 hits for that name). |
| db-018 | confirmed | Messages verified (`0x49d9358`, `0x49d93c9`, `0xa2b5858`, `0xa2b8188`). The interrupt mechanism is inferred (SQLite is statically linked, so no symbol name is expected). Base: only `heavy_read_semaphore`. |
| db-019 | confirmed | Coordinator strings verified (`0xa2e8c67`, marker fields `0xa2e9500`, `.quarantine.json` `0xa2e97b6`, `.verification-pending.json` `0xa2e98e8`). Base `sqlite_error.rs` only classifies fatal errors for connection recycling. As the first pass found, the missing-parent "unmounted volume" refusal is already in base (`write_queue.rs:1567` `ensure_db_parent_dir`). |
| db-020 | confirmed | Base `cli/db.rs` shells out to `sqlite3 .recover` (>= 3.38) with `LockPayload {pid, host, started_at, op}` (:61-67). Target: `0x49aae20` (healthy short-circuit, `db-recovery-%Y%m%d-%H%M%S`), `0x49aaf38` (`working-copy`, `candidate.sqlite`, `recovery-manifest.json`), `0x49ab261` (canary/identity verification), `0x6a982a9` (phases), `0x6a984bf` (manifest + lock fields incl. `host_id`). Port-3030 refusal, `unlock --force`, `cleanup --apply` and the 2x disk check are base behaviour (first-pass note stands). |
| db-021 | confirmed | v1 DDL `0x49e7698`, byte accounting `0xa2b8bb0`, `hybrid_frame_insert` `0xa2b90db`, revocation `0x49e1fd6`. Version labels `:resident-delete-and-redacted-update-v1`, `:resident-frames-v1`, `:recording-v6`, `bulk-schema-` and capability strings `parquet-frame-v1`/`contentless-delete-fts5`/`durable-wal`/`parquet-bulk-v1` all present. No `-v5` label exists anywhere (consistent with "v5 not observed"). Base: 0 hits. |
| db-022 | confirmed | `0x49e2e78`, `0x49e3586`, `0x49e4288`, `0x49e4630` verified; `CREATE VIRTUAL TABLE elements USING screenpipe_elements` at `0x49e4dde`; `screenpipe_payload_sha256` 8 hits. Base: 0 hits. |
| db-023 | confirmed | `0x49ea368`, `0x49eb390`, `0x49eb758`, `0xa276bd8` verified; frontend commands confirmed (above). Base: 0 hits. |
| db-024 | confirmed | `0x49e1c80`, `0x49e94c8`, `0xa27cbf8`, `0x51487f8` verified. The base has single-file `VACUUM INTO` backups (`db.rs:8757`, `/data/backup`, `backup.sqlite`); the bundle format and the `backup.screenpipe` name (`0x5151ab3`) are new. |
| db-025 | confirmed | Route doc `0x6a6c9a6`, messages `0x5151ab3`/`0x5151b35` (incl. the hybrid refusal), pragma triple `0x5141f60`. Base router has no compact route; the base's only full VACUUM is in the uncalled `repair_database()` (db.rs:7405; its log strings have 0 target hits). |
| db-026 | confirmed | `0x49af280`, `0x49af868`, `0x49afa1c`, `0x49873c0` verified. Base `SecretStore` lives on the main pool. Overlaps `connections.md`, which already records the lease-acquire SQL. |
| db-027 | confirmed | `migration_progress` has 0 raw bytes (ASCII or UTF-16). `MigrationWorker` appears only inside migration comment bytes (`0xa2a3f5b`, `0xa2a42f5`). Base exports it (`lib.rs:20`). |
| db-028 | duplicate | A roll-up of paths owned by db-002/016/017/019/020/021/023/024/026. `backup.sqlite` and `disk_usage.json` are already in base (`routes/data.rs:447`, `disk_usage.rs:133`). The only extra fact (DB-size accounting includes `secrets.sqlite*`, `0x49873c0`) belongs with db-026. |
| db-029 | confirmed | `crates.txt`: `libsqlite3-sys-0.37.0`, `sqlx-core/sqlx-sqlite-0.9.0`, `rusqlite-0.39.0`, `parquet-59.3.0`, `zstd-safe-7.3.0`, `sha2-0.10.9`. Base `Cargo.lock`: libsqlite3-sys 0.26.0, sqlx 0.7.2, rusqlite 0.29.0, sha2 0.10.9 (already present), no parquet/zstd. Source id at `0xa32f090`. |

### Missed items (second pass)

* **db-v2-01 [changed] The on-demand WAL checkpoint no longer uses TRUNCATE (no TRUNCATE checkpoint anywhere).** In the base, `DatabaseManager::wal_checkpoint()` (db.rs:8747-8751) runs `PRAGMA wal_checkpoint(TRUNCATE)` for `POST /data/checkpoint` (`routes/data.rs:398-403`). The target still ships `routes::data::checkpoint_handler` (`0x52ce5df`) and the route (`/data/checkpoint` in the route table at `0x5257ca8`), but the exe contains **0** occurrences of `(TRUNCATE)` (raw scan). The only checkpoint modes present are RESTART (startup, `0x49daf78`), PASSIVE (maintenance, `0x5140340`) and FULL (offline construction/reclaim, `0x49e8be8`). Together with the WAL-reset guard (db-016), this suggests the target never truncates the WAL. The on-demand mode is inferred (RESTART or FULL). Freenpipe: move `wal_checkpoint()` off TRUNCATE and keep the `busy/log/checkpointed` response shape. Size XS.

Review of the first-pass missed items. I do not re-list them as separate deltas:
* db-m01 (`SCREENPIPE_IGNORE_DB_LOCK` removal): the env var is absent (0 raw hits, confirmed). It is already tracked in `shell.md` ("base vars not observed"), and the orphan-lock verification gate is described in db-020 step 2.
* db-m02 (delete-range prunes `display_layout`, OCR counter dropped): the pruning rule is in `vision.md`, and the dropped counter is in db-005 (`0x5140e3a`).
* db-m03 (`.owner`): this is a detail of db-016 point 8 (the guard is claimed there, only the file name was missing). It is noted in the db-016 row above.
* A candidate I checked and dropped: the target has no `ui_events_fts MATCH` query (0 hits). The base's only FTS user, `search_ui_text_events` (db.rs:8619), has no callers, so this is not a behaviour delta.

