# Spec: Meetings, memories, activity ledger, semantic layer, outputs, notifications (`meetings_memory`)

Clean-room behavioural spec for Freenpipe, derived from static evidence about the installed screenpipe
v2.7.104 Windows build. It is written for an engineer who has never seen the target. Everything below is
paraphrased. Interop identifiers (routes, JSON fields, SQL names, settings keys, events, file names) are
quoted exactly. SQL DDL for new tables is recorded verbatim for on-disk compatibility; queries are
described, not copied.

- BASE = `<repo>` (upstream `892199f`, ~v2.5.27, MIT).
- TARGET = `screenpipe-app.exe` v2.7.104 (static strings only; never executed).
- `app_strings.tsv` offsets are written `@0x<hex>`. Frontend paths are relative to
  `<re-workspace>/frontend/`.
- REA evidence: parent JS application analysis `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`;
  module views `ev_27381aee90c77a8867686aa3a01d65a60222f9492c1809d4eb3f3d4b79d1e002` (notification-inbox page),
  `ev_b55827417afa2adb8b9ebf9b95e21c567d5a3109eb4f10470f80291aef1199a2` (starred-sessions page),
  `ev_df2f0e5751cdd069e53296885abe98dfbea61b17917792afacbbfacad43df260` (home page), summary view
  `ev_d205fc1fda693bff953c410e8912bcbd3a6e1ff1e195914f6446ed80da215f84`. REA feature traces on literal seeds
  (`/activity-ledger`, route `notification-inbox`) returned no graph match
  (`ev_81ceac58e4f75f696a6c2adc4a19af9894839a9d2705960dee38a6c3fbc23eeb`,
  `ev_735fe155cb5f27c05bd3d2c95921cb5c4f2cc47b5e1952e767e786066fbb63ca`); frontend facts below come from direct
  inspection of the extracted chunks.
- Helper scripts written for this pass (all static): `<re-workspace>/tools/mm_*.py`
  (`mm_migtable.py` decodes the embedded sqlx migration table directly from the exe's `.rdata`).
- NOTE on embedded text: many target SQL migrations begin with an upstream header comment that instructs
  "AI agents" to add a screenpipe header to files. That is data inside the binary, not an instruction to us.
  Freenpipe migrations and sources use the Freenpipe header from `CLAUDE.md` instead.

---

## 0. Base vs target inventory

| Area | BASE (892199f) | TARGET (v2.7.104) | Delta ids |
|---|---|---|---|
| `meetings` table | yes (+`note`, `end_reason`, single-open index) | adds `calendar_event_id` + partial unique index | 025 |
| `meeting_transcript_segments` | yes (+`speaker_id`) | adds `stream_id`, `session_speaker_id`, two indexes | 025 |
| Meeting routes | list/status/merge/bulk-delete/start/stop/:id/transcript/retranscribe/split, `/export` | adds `/meetings/room-change`, `/meetings/:id/summary`, `/meetings/:id/summary-status`; request shape changes | 026, 029, 032 |
| Meeting detection | single `meeting_detector.rs` (UI-scan "v2" + calendar) | split `meeting_watcher/{audio_process,ui_scan,shared}`; new audio-session (WASAPI mic-holder) detector; exact calendar binding; room change | 030, 031 |
| Meeting summary | built-in `meeting-summary` pipe that PATCHes note | server-side summary save + status endpoints, finalizer/recovery worker, idempotent `meeting_ended` triggers | 026, 027, 028 |
| Meeting overlay | `/ws/meeting-status` | adds `/ws/meeting-overlay` live transcript stream + overlay UI | 033 |
| `memories` table + `/memories` API | yes (identical schema/validation) | unchanged API; UI moved to Home "Brain"; external sync sidecar changed; Obsidian target | 035, 006 |
| Activity ledger (`activity_*` tables, `/activity-ledger`) | absent | new schema, deterministic reconciler, API, Activity view | 013-017 |
| `/activity-summary` | yes (data_status, query_status, guidance, edited_files...) | adds include_* toggles, starred sessions, ui_event app attribution, parsed count | 018 |
| Semantic layer (`semantic_*`, `/semantic/actors/*`, `content_type=parsed`) | absent | new crate `screenpipe-semantic`, worker, schema, actors API, parsed search | 019-024 |
| `outputs` / `output_search_index` / `/artifacts` | absent | new artifacts registry + FTS + API + SAF envelopes + UI | 002-006 |
| Structured outputs (`/outputs/targets`) | absent | new typed output targets store bound to pipes; feedback + item actions | 005 |
| `starred_sessions` + `/starred-sessions` | absent | new table, API, search filters, overlay/page UI, shortcut | 010-012 |
| `feedback` table + `/feedback` | absent (only support "send logs" to cloud) | new local AI-feedback store + API + rating UI | 007, 008, 009 |
| Notifications (`:11435/notify`) | basic payload, JSON history (100), panel page, bell | priority, richer actions, per-item read, permission/mute/quiet/snooze gates, dedupe, inbox window, feedback | 037-041 |
| `screenpipe-team-memory` crate | present, not linked by anything | absent | 043 |
| `screenpipe-vault` | present; online lock route works | online lock refused; CLI only | 044 |
| History-access (24 h) gating | absent | free-tier 24 h gate on meetings/memories/frames/export/starred | 042 |
| Pipe default allowlist | 13 endpoints | adds starred/workflows/feedback + writer & structured-output lists | 045 |

---

## 1. Migration version map (target, decoded from the exe)

The exe embeds sqlx's migration table (array of 0x58-byte records starting at file offset `0xa2ae3f0`,
117 entries). `tools\mm_migtable.py` decodes it; descriptions were cross-checked against BASE file names
(the decoder prints each description one row early; the mapping below is corrected). Versions that matter
to this dimension (all absent from BASE unless marked):

| Version | Description (sqlx, from file name) | Owner |
|---|---|---|
| 20260601000000 | `create outputs` | 002 (note: sorts *before* BASE's last migration 20260603120000) |
| 20260603000000 | `one open meeting invariant` (BASE has it) | — |
| 20260603120000 | `meeting transcript speaker id` (BASE has it) | — |
| 20260612000000 | `add saf columns to outputs` | 002 |
| 20260622000000 | `create output search index` | 003 |
| 20260709000000 | `make redacted at indexes partial` | other dim |
| 20260719000000 | `index meeting transcript segments captured at` | 025 |
| 20260724000000 | `create semantic context tables` | 019 |
| 20260727000000 | `create semantic actors` | 020 |
| 20260728000000 | `create pipe event runs` | 028 |
| 20260729130000 | `index outputs original path` | 002 |
| 20260730180000 | `create feedback` | 007 |
| 20260805000000 | `add pipe execution conversation id` | 028 (shared with pipes dim) |
| 20260812120000 | `restore orphaned speaker references` | 025 (shared with audio dim) |
| 20260813000000 | `add calendar event id to meetings` | 025 |
| 20260817000000 | `create activity ledger` | 013 |
| 20260817160000 | `rebuild activity ledger for week backfill` | 013 |
| 20260818133000 | `activity ledger query indexes` | 013 |
| 20260828143000 | `scope live meeting speakers` | 025 |
| 20261002190000 | `starred sessions` | 010 |

Rules for Freenpipe (`crates/screenpipe-db/src/migrations/`):
- Use exactly these version numbers and snake_case names (`<version>_<description_with_underscores>.sql`)
  so a database last opened by screenpipe v2.7.104 has matching `_sqlx_migrations.version` rows and is not
  re-migrated. BASE already sets `set_ignore_missing(true)` and self-heals checksum mismatches
  (`crates/screenpipe-db/src/db.rs` ~L403-498); keep that, because our SQL text (and therefore checksum)
  will differ from upstream.
- `20260601000000` is older than BASE's newest applied version. Verify the bundled sqlx runs out-of-order
  pending migrations (sqlx applies any unapplied version; add a test that opens a BASE-era DB and confirms
  `outputs` gets created).
- Tables that the target creates with plain `CREATE TABLE` (activity_*, semantic_*, starred_sessions) should
  be created with `IF NOT EXISTS` in Freenpipe to tolerate partially-migrated DBs. Triggers/indexes likewise.

Evidence: `@0xa2a195a` (create outputs) ... `@0xa2ae1cc` (starred sessions) in `app_strings.tsv`; table
records at exe offset `0xa2ae3f0`.

---

## 2. Delta items

Each item: identifiers, behaviour, data shapes, defaults, evidence, confidence, base files, notes.

### meetings_memory-001 — Adopt target migration numbering for this dimension (changed)
- Behaviour: see §1. All new DDL in items 002-028 must be shipped under the target version numbers.
- Base touchpoints: `crates/screenpipe-db/src/migrations/`, `crates/screenpipe-db/src/db.rs` (migrator).
- Evidence: exe offset `0xa2ae3f0`; strings `@0xa2a1626`..`@0xa2ae35a`. Confidence: observed. Size S.

---

### meetings_memory-002 — `outputs` artifacts registry schema (new)
DDL (verbatim from target, migrations 20260601000000, 20260612000000, 20260729130000):
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
-- 20260729130000
CREATE INDEX IF NOT EXISTS idx_outputs_source_type_original_path_id
ON outputs(source_type, original_path, id DESC)
WHERE original_path IS NOT NULL;
```
- Semantics: one row per file copied into the canonical outputs dir `~/.screenpipe/outputs/` (Freenpipe:
  `<data_dir>/outputs/`). `source` = producer name (pipe slug, chat id...), `source_type` in
  {`pipe`, `pipe-run`, `chat`, `structured-output`?...} (observed values: `pipe`, `pipe-run`, `chat`; inferred
  others). Plain files leave the three SAF columns NULL. `saf_version` stores the artifact envelope's
  monotonically increasing `version` (not the envelope's `saf_version` marker). The partial index lets a
  scheduled pipe's stable producer path be looked up so unchanged snapshots are skipped.
- DB operations to implement (`db/outputs.rs` in target; Freenpipe: new module or `db.rs` section):
  insert (all columns except id/timestamps), update metadata fields by id (also bumps `updated_at`), get by
  id, get by `output_path`, repoint `output_path` by id, latest row for (`source`,`source_type`,`artifact_id`),
  latest row for (`source_type`,`original_path`), list with filters + `ORDER BY updated_at DESC LIMIT/OFFSET`,
  count, distinct display sources (chat rows collapse to the literal `chat`; rows with
  `saf_kind='structured-output'` are excluded from library listings and source facets).
- Evidence: `@0xa2a1af2`, `@0xa2a2d2a`, `@0xa2aad72`, `@0x51a63e0`..`@0x51a6780`, `@0x5a5f4d0`..`@0x5a5fb40`,
  `@0xa27b150`. Confidence: observed (DDL, queries); inferred (source_type value set).
- Base touchpoints: `crates/screenpipe-db/src/migrations/` (3 files), `crates/screenpipe-db/src/db.rs`,
  `crates/screenpipe-db/src/types.rs` (`OutputRecord`). Size M.

### meetings_memory-003 — Artifact full-text index (`output_search_index`, `output_search_fts`) (new)
DDL (verbatim, migration 20260622000000):
```sql
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
```
- The artifact file stays the source of truth; the sidecar records whether/what was indexed. FTS rowid =
  `outputs.id` (plain fts5, not external content, so rows can be deleted and replaced by rowid).
- Indexing behaviour: after each register/auto-register (and as a startup backfill pass that logs how
  many artifacts it indexed), read the output file, compute a content hash, skip if the stored
  `content_hash` matches, else delete the old fts row + sidecar row, insert `(rowid,title,body)` and a fresh
  sidecar row with `bytes_indexed`. Only text-like kinds are indexed (markdown/json/text/csv/code; SAF JSON
  bodies flattened to text). Cap the indexed body (inferred: a few hundred KB) and record bytes actually
  indexed. Failures are logged and never fail the register call.
- Search: when `/artifacts?q=` is given, match `output_search_fts MATCH ?` joined to `outputs`, ordered by
  `bm25(output_search_fts)` then `updated_at DESC`, paged; plus a count query.
- Evidence: `@0xa2a6390`..`@0xa2a6577`, `@0x51a6920`, `@0x5a5f750`, `@0x5a5f8d0`, `@0x6ab7a29`..`@0x6ab7ac0`.
- Confidence: observed (schema/queries); inferred (size caps, kinds indexed). Size M. Depends on 002.

### meetings_memory-004 — Artifacts API, auto-registration and SAF envelope (new)
Routes (engine, OpenAPI-registered, handlers `register_artifact_handler`, `list_artifacts_handler`,
`delete_artifact_handler`):
- `POST /artifacts/register` — body `RegisterArtifactRequest`: `file_path` (absolute, required), `title`
  (non-empty; default filename when omitted by tools), `source` (non-empty), `source_type` (default `pipe`),
  optional `kind`. Behaviour: validate file exists/readable (error otherwise), size limit
  **100 MB** (larger files rejected with an error naming the limit); sanitize `source`, `source_type`, filename (reject path
  separators / traversal with an invalid-characters error); copy into
  `<outputs>/<source_type>/<source>/<filename>` (inferred layout); upsert the `outputs` row keyed by
  `output_path`; if the file parses as a SAF envelope (below) set `saf_kind`/`artifact_id`/`saf_version`,
  and if a row with the same (`source`,`source_type`,`artifact_id`) exists, repoint/update that row instead
  of creating a duplicate; otherwise log that envelope validation failed and register it as a plain file.
  Then index for search (003). Response `RegisterArtifactResponse` includes the row `id` and `output_path`.
- `GET /artifacts` — query: `q`, `offset`, `limit`, `per_pipe_limit`, `include_internal`, `id`,
  plus source/kind filters used by the UI (`source`, `saf_kind`). Response `{ data: [...], pagination:
  {limit, offset, total} }` where each item has `id`, `path` (resolved file path), `title`, `kind`, `source`,
  `source_type`, `original_path`, `preview`, `saf_kind`, `artifact_id`, `saf_version`, `size_bytes`,
  `modified_at`, `created_at`, `registered` (bool). The listing merges DB rows (registered copies under the
  outputs dir) with artifacts derived at request time from pipe directories (frontmatter-declared paths),
  de-duplicated by path with the registered copy winning; filtering and pagination are server-side and
  `pagination.total` counts the filtered set. Rows with `saf_kind='structured-output'` are internal and
  only returned with `include_internal=true`.
- `DELETE /artifacts/:id` — deletes a registered row and its copied file (404 when no such row exists; 500 when
  the copied file cannot be removed). Filesystem-derived pipe artifacts have no row and cannot be deleted.
- Auto-registration after pipe runs: pipe frontmatter may declare
  `artifacts: [{ path, title, kind }]` (paths relative to the pipe dir). After each run, each declared file is
  copied/registered with `source_type='pipe-run'` (observed literal) and `original_path` = the pipe-dir path;
  if the content is unchanged versus the newest row for the same `original_path`, the existing output is kept
  (with a log noting the unchanged artifact). Errors are logged per step (create dir, copy, lookup, insert,
  update, repoint, index).
- Agent tools: pipes get `register_artifact(file_path, title)` (registers an existing absolute file);
  chat gets `save_artifact` which accepts content (text, or base64 image with `encoding=base64`) and writes
  then registers it. Tool descriptions tell the model to use them only for finished deliverables.
- SAF ("Screenpipe Artifact Format") envelope v1 — JSON object with: `saf_version` must equal `1`;
  `artifact_id` non-empty, <=128 chars, matches `^[a-z0-9][a-z0-9_-]*$`; `version` integer >=1;
  `kind` non-empty, <=32 chars; `title` non-empty string; `body` a JSON object. File suffix for structured
  output artifacts is `.saf.json`. UI renders `kind:"sop"` specially (steps with action, app, confidence,
  evidence count, frame thumbnails) and other envelopes as a JSON tree.
- Evidence: route list `@0x5257ca8`; doc text `@0x6a7c9c8`..`@0x6a7cda6`; errors `@0x522de71`..`@0x522e0c3`,
  `@0x49abb59`..`@0x49abf0b`, fields `@0x6189418`; skill text `@0x6a59f16`..`@0x6a5a229`; tool descriptions
  `@0x529f9b0`, `@0xa0d17a9`, `@0xa0db108`; UI validator in
  `_next/static/chunks/app/(main)/home/page-c18385b042b3382e.js` (@~100852).
- Confidence: observed (routes, fields, limits, envelope rules); inferred (on-disk dir layout under outputs,
  exact query filter names beyond those listed).
- Base touchpoints: `crates/screenpipe-engine/src/server.rs` (route registration), new
  `crates/screenpipe-engine/src/routes/artifacts.rs`, `crates/screenpipe-core/src/pipes/mod.rs`
  (frontmatter `artifacts`, post-run hook), pi extension assets under `crates/screenpipe-core/assets/extensions/`
  (new `register_artifact` tool), app chat tools (`apps/screenpipe-app-tauri/lib/pi/...`). Size L.
  Depends on 002, 003.

### meetings_memory-005 — Structured output targets (`/outputs/targets`) (new)
A local store of typed, schema-validated "targets" that consumers (live views / dashboards, "brain views")
define and that bound pipes fill.
- Storage: JSON file `structured-outputs/targets.json` under the data dir with a sibling lock file
  (`*.json.lock`), versioned format (an unknown store version is rejected). Each submitted value is
  also written as an artifact file `<...>.saf.json` and registered in `outputs` with
  `saf_kind='structured-output'` (hidden from the library).
- Target definition fields: `id` (target id; consumer-scoped format such as `live-view:<viewId>:<slotId>`),
  `consumer`, `title`, `instruction`, `bound_pipe`, `schema_name`, `schema` (JSON Schema subset), `revision`,
  `latest`, `feedback`, `item_actions`. Validation: target/item ids <= configured max, characters
  `[A-Za-z0-9.:_-]` (and `/` for some keys, <=160 chars); per-consumer target cap and global cap (an error
  states the maximum target count); schema size cap; schema subset supports `type` (string or array),
  `properties` (<=32 fields), `required`, `items` (required for arrays), `additionalProperties` (bool),
  `maxItems`, `maxLength`; payload size cap.
- `GET /outputs/targets?pipe=<slug>` -> `{ data: [ { id, title, instruction, revision, schema_name, schema,
  feedback: { up_count, down_count, recent: [{ rating, artifact_version, output_payload?, correction?,
  created_at }] }, latest?: { payload, evidence, artifact_output_id, artifact_version, updated_at },
  item_actions: { items: [{ item_id, disposition: active|resolved|snoozed|dismissed, snoozed_until?,
  correction?, updated_at }] } } ] }`.
- `POST /outputs/targets` — create/replace a consumer's target set (used by the dashboard UI; desktop only).
- `POST /outputs/targets/:target_id/submit` — body `SubmitStructuredOutputRequest`: `target_revision`,
  `payload`, `evidence` (<=50 refs `{event_id?, frame_id?, transcription_id?, ts?, device_id?}`),
  optional `source_pipe`. Rejects when the calling pipe is not `bound_pipe` (not-bound error),
  when `target_revision` differs (409-style revision-changed error), when the payload fails schema
  validation, when list items have duplicate ids or unsupported actions, or when binding/value changed
  concurrently. Response `{ target_id, artifact_output_id, artifact_version }`.
- `POST /outputs/targets/:target_id/feedback` — body `SetStructuredOutputFeedbackRequest`:
  `artifact_output_id`, `artifact_version`, `rating` (`up`|`down`|null to clear), `correction` (only with
  `down`, length-capped). Pipes may not rate their own outputs. Persists into the unified `feedback` table
  (007) with `target_kind='structured_output'` (inferred) and a bounded snapshot; returns
  `{ feedback: { up_count, down_count, current } }`.
- `POST /outputs/targets/:target_id/items/:item_id/actions` — body `SetStructuredOutputItemActionRequest`:
  `artifact_output_id`, `artifact_version`, `action` in {`resolve`,`snooze`,`correct`,`dismiss`,`reopen`},
  `snoozed_until` (RFC3339, future, only for snooze), `correction` (only for correct). Only for payloads of
  the `list.v1` component shape; item must exist in the current output and allow the action. Returns
  `{ item_actions: { items: [...] } }`. Pipes may not act on their own items.
- Pi extension `structured_output` tool (actions `get_targets`, `submit`) is installed into each pipe run;
  before the agent starts it injects a compact target summary into the system prompt and instructs the agent
  (paraphrased): respect each target's instruction and time range, keep stable item ids, honour user item
  state and corrections, never fabricate metrics, trust only authoritative API fields for counts, submit only
  evidence-backed payloads. Write Freenpipe's own wording.
- Default pipe allowlist addition: `GET /outputs/targets`, `POST /outputs/targets/*/submit` (045).
- Evidence: routes `@0x5258618`..`@0x52586c0`; request structs `@0x522d17b`..`@0x522d338`; store/validation
  `@0x6a23bb8`..`@0x6a240f0`, `@0x6a24288`..`@0x6a248f9`; ext source region `@0xa0adc18`..`@0xa0b0049`;
  UI calls in home page chunk (@~189852, @~251037).
- Confidence: observed (routes, fields, validation messages); inferred (numeric caps, exact id grammar).
- Base touchpoints: new `crates/screenpipe-engine/src/routes/structured_outputs.rs`, new store module in
  `crates/screenpipe-core`, `server.rs`, pi extension assets. Size L. Depends on 002, 007. The dashboard UI
  ("live views"/brain views, `/live-views/*`) is a different dimension and only consumes this contract.

### meetings_memory-006 — Home "Brain" library: memories + artifacts UI (new / changed)
- Target Home has a Brain section with tabs incl. memories and artifacts (and saved brain views). Header
  counters poll every 30 s: `GET /memories?limit=1&offset=0` and `GET /artifacts?limit=1&offset=0`
  (`pagination.total`).
- Memories tab: list with `q`, `source`, `tags`, `order_by=created_at`, `order_dir=desc`, paging; inline edit
  sends `PUT /memories/:id` with only changed `content`/`tags`; create sends `POST /memories`
  `{content, source:'user', tags, importance:0.5}`; single and batch delete via `DELETE /memories/:id`.
- Artifacts tab: grid/list of `/artifacts` items showing title, kind badge, SAF kind + version, size,
  modified time; actions: open in viewer window (`openViewerWindow(path)`), open the originating chat
  conversation when `source_type==='chat'` (load conversation with file preview), an ask-AI action for the artifact
  (prefills chat), delete (registered only), batch selection mixing memories (`mem:<id>`) and outputs
  (`output:<id>`). Notification deep links can open an artifact by registered id (`?id=`) or legacy path (`?q=`).
- BASE has memories only in Settings (`components/settings/memories-section.tsx`).
- Evidence: home chunk (@~297851, @~303602, @~308476..@~315891, @~331249, @~337422). Confidence: observed.
- Base touchpoints: `apps/screenpipe-app-tauri/app/home/page.tsx`, new `components/brain/*`,
  `components/settings/memories-section.tsx` (keep or redirect). Size M. Depends on 004.

---

### meetings_memory-007 — Unified AI feedback store and `/feedback` API (new)
DDL (verbatim, migration 20260730180000):
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
- One rating per (target kind, id, version, actor, device). Upsert: look up the existing row's `id` and
  `created_at` for that key; insert-or-update by `id` replacing `producer_ref`, `rating`, `comment`,
  `snapshot`, `context`, `device_id`, `updated_at`. Clearing a rating deletes the row
  (`device_id=''` variant for local actor). `actor_id` default `local-user`. `id` is a generated UUID.
  `target_version` empty string means "unversioned"; reads return it as NULL (`NULLIF`).
- `POST /feedback` — body `SetFeedbackRequest`: `target: { kind, id, version? }`, `rating` (`up`|`down`),
  optional `comment` (allowed only with `down`; length-capped), `snapshot` (bounded JSON of what was rated),
  `producer_ref` (e.g. `pipe:<slug>`, `agent:screenpipe-chat`), `context` (object), optional `device_id`.
  Validation: kind = lowercase ASCII letters/digits/`.`/`_`/`-`; id/version/producer_ref length+charset caps;
  rating limited to `up`/`down`; a comment is rejected unless the rating is `down`; pipe-scoped tokens cannot rate
  outputs whose `producer_ref` is their own pipe (self-rating is refused with an error).
- `GET /feedback` — query: `kind`, `target_id`, `producer`, `rating`, `q` (substring over target id,
  producer, comment, snapshot, context), `since` (updated_at >), `since_inclusive` + `after_id` (keyset
  cursor for equal timestamps), `order` (`asc`|`desc`, by `updated_at`), `limit`. Response records:
  `{ id, target: {kind, id, version?}, producer_ref, actor_id, rating, comment, snapshot, context,
  device_id, created_at, updated_at }`. Pipe-scoped tokens only see records whose `producer_ref` is their pipe.
- Target kinds observed: `notification`, `chat`, memories/blocks/artifacts, `structured_output`
  (exact-version). Pipes read feedback before regenerating recurring output (default allowlist adds
  `GET /feedback`).
- Evidence: DDL `@0xa2aaf2a`; queries `@0x516d4f2`, `@0x516d9a8`, `@0x51a6cb8`; route strings
  `@0x5183483`..`@0x5183708`; skill doc `@0x511d801`..`@0x511daa6`; client in
  `_next/static/chunks/3822-215756f0622728d9.js` module 68186. Confidence: observed (schema, fields, rules);
  inferred (exact length caps).
- Base touchpoints: migration, `crates/screenpipe-db/src/db.rs` (+`FeedbackRecord` in `types.rs`), new
  `crates/screenpipe-engine/src/routes/ai_feedback.rs`, `server.rs`, pipe auth middleware
  (`pipe_permissions_middleware.rs`) for producer scoping. Size M.

### meetings_memory-008 — Rating UI for notifications and chat responses (new)
- Notification rows (bell/inbox and toast panel) show thumbs up/down for pipe-originated notifications
  (`type==='pipe'`, or has `pipe_name`/`source_session_id`, or onboarding follow-up ids). Down opens a short
  free-text input asking what could be better. Submit posts to engine `POST /feedback` with
  `target {kind:'notification', id}`, `rating`, optional comment, `snapshot {title, body}`,
  `producer_ref 'pipe:<pipe_name>'` when known, `context {notification_type, source_session_id,
  source_message_id}`. States idle/saving/saved/error.
- Chat assistant messages: thumbs up/down with preset negative reasons; posts `target {kind:'chat', id}`,
  `producer_ref 'agent:screenpipe-chat'`, `snapshot {response (<=12000 chars), truncated}`, context from the
  message.
- Evidence: `_next/static/chunks/7737-fae548fffc70b86b.js` (@~37334), `3822-...js` module 68186,
  `3696-4fbfea4ced3de899.js` (@~485391). Confidence: observed. Size S. Depends on 007, 039.
- Base touchpoints: `components/notification-bell.tsx`, `app/notification-panel/page.tsx`, chat message
  components under `components/chat/`.

### meetings_memory-009 — Support "send logs & feedback" upload (decloud)
- Target app module `feedback_upload.rs` uploads logs/screenshot/video to the vendor (`/api/logs`,
  `/api/logs/confirm`) after PII redaction through a vendor-hosted redaction service
  (`pii.screenpipe.containers.tinfoil.dev`) with local regex fallback (`feedback_redact.rs`); Tauri commands
  `start_feedback_upload`, `redact_pii_for_feedback`.
- Freenpipe: never upload. Replace with "Export diagnostics bundle" that writes a local zip (logs, settings
  without secrets, optional screenshot/video <=50 MB) after local regex redaction, and shows the path. BASE's
  `share-logs-button.tsx`/`feedback-dialog.tsx` point at screenpi.pe and must be rewired the same way.
- Evidence: `@0x498c92c`..`@0x498cb65`, `@0x5a63148`..`@0x5a637c0`, `@0x498c790`. Confidence: observed.
  Size S. Note: unrelated to the AI-feedback table in 007 despite the name.

---

### meetings_memory-010 — Starred work sessions: schema and API (new)
DDL (verbatim, migration 20261002190000):
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
- `id` is a client-generated UUID; `start`/`end` RFC3339 UTC strings.
- `POST /starred-sessions` (handler `starred::save`) — body `SaveSession { id, start, end, hd_requested,
  revision }`. `revision: 0` creates; otherwise it must equal the stored revision (optimistic concurrency):
  mismatch -> **409** (the error tells the client another window changed the sessions and it should reload first); missing row for non-zero
  revision -> an error that the session is gone. Reject `end <= start` (ordering error), a start
  that falls inside another session (exists row with different id where `start <= new.start < end`),
  starts outside accessible history (042: always allowed in Freenpipe), and agent/pipe tokens
  (403; the error explains that agents have read-only access and starring needs a user action). Upsert sets
  `revision = old + 1` (insert uses 1). If `hd_requested` and HD capture is unavailable -> an error
  that HD capture is unavailable; otherwise starting/extending an HD session bound to `starred` (HD capture
  modes observed: `ask|always|never|timer|starred|prewarm_pending`). Returns the saved session.
- `GET /starred-sessions` (handler `starred::list`) — query `start_time`, `end_time`, `limit`, `offset`;
  returns sessions overlapping the range (`end > start_time AND start < end_time`) ordered by `start DESC, id`,
  each with computed `has_audio` (true when any audio chunk with a non-empty file path has a timestamp in
  [start, end)), plus pagination. No delete route observed (sessions are edited, not deleted).
- Evidence: DDL `@0xa2ae26f`; queries `@0x516d018`, `@0x516d1d0`, `@0x51a6920`; route errors
  `@0x5183990`..`@0x5183bb8`; doc `@0x511e39f`, `@0x6a570c3`. Confidence: observed (409 code from doc).
- Base touchpoints: migration, `db.rs`, new `crates/screenpipe-engine/src/routes/starred.rs`, `server.rs`,
  `routes/capture.rs` (HD `boundTo` gains `starred`). Size M.

### meetings_memory-011 — Starred sessions in search and activity summary (new)
- `/search` gains `starred_only` (bool) and `starred_session_id` (string) query params. Filtering happens
  before pagination; a frame/audio hit is "starred" when its timestamp lies in [start, end) of any session
  (compare via `julianday`). Every hit gains `starred: boolean`.
- `/activity-summary` gains `include_starred` (default true) and returns `starred_sessions` (up to **10**
  most recent overlapping sessions with `id`, `start`, `end`, `has_audio`, `hd_requested`) and
  `starred_sessions_has_more`. Omitted for app/data-restricted reads.
- Evidence: `@0x52ca6b8` (search params), `@0x522ba40` (summary fields), `@0x511df6b`, `@0x511e16e`,
  `@0x522d0d0`. Confidence: observed. Size M. Depends on 010. Base touchpoints:
  `crates/screenpipe-engine/src/routes/search.rs`, `crates/screenpipe-db/src/db.rs` (search SQL),
  `routes/activity_summary.rs`.

### meetings_memory-012 — Starred sessions UI, shortcut and CLI (new)
- Window/page `starred-sessions` (static page `starred-sessions.html`); Tauri commands
  `toggle_starred_sessions`, `hide_starred_sessions`; events `starred-sessions-changed`,
  `starred-sessions-visibility`, `starred-session-toggle`, `starred-session-state`; setting
  `starSessionShortcut`. The shortcut overlay (shortcut-reminder) shows a star control.
- Behaviour: a star action lasting N minutes, N in {5,15,30,60} (optionally requesting HD); while active show remaining
  minutes, an end control (sets end = now) and a 15-minute extend control (end += 900000 ms); edit start/end via `datetime-local`
  (inline validation error when end is not after start); session capture quality selector (normal/HD); export finished
  session as video via `POST /export {start, end, include_audio:true}` (refuse while active); an ask-about-this-session
  action opens chat with a prompt referencing the session id/bounds and `starred_session_id` search.
  List uses `GET /starred-sessions?limit=100`; on 409 reload.
- CLI (if the CLI dimension does not own it): `star list --limit N`, `star start --minutes 5|15|30|60 [--hd]`,
  `star end`, `star edit ID --start T --end T`, JSON output, honours `SCREENPIPE_API_URL`,
  `SCREENPIPE_LOCAL_API_URL`, `SCREENPIPE_LOCAL_API_KEY`.
- Evidence: page chunk `_next/static/chunks/app/starred-sessions/page-fef15017d60c36f1.js`
  (REA `ev_b55827417afa2adb8b9ebf9b95e21c567d5a3109eb4f10470f80291aef1199a2`); app strings `@0x524e640`..
  `@0x524e8c0`, `@0x621fdbd`; CLI `@0x6a5ac4d`; settings key in settings blob. Confidence: observed.
- Base touchpoints: `apps/screenpipe-app-tauri/app/` (new `starred-sessions/page.tsx`),
  `components/shortcut-reminder.tsx`, `src-tauri/src/commands.rs` (+ new `commands/starred_sessions.rs`),
  `src-tauri/src/window/*`, `lib/hooks/use-settings.tsx`, `crates/screenpipe-engine/src/cli/*`. Size M.

---

### meetings_memory-013 — Activity ledger schema (new)
DDL (verbatim, migrations 20260817000000 / 20260817160000 / 20260818133000):
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
-- 20260817160000 rebuild activity ledger for week backfill (data-only reset of the derived, unlocked projection)
DELETE FROM activity_intervals WHERE producer = 'deterministic-v1';
DELETE FROM activity_tasks WHERE producer = 'deterministic-v1' AND user_locked = 0;
DELETE FROM activity_ledger_state WHERE producer = 'deterministic-v1';
-- 20260818133000
CREATE INDEX IF NOT EXISTS idx_activity_intervals_producer_range
    ON activity_intervals(producer, start_at, end_at, id);
```
(Trigger `END;` terminators and statement separators are reconstructed; the string dump strips them.)
- Design intent (paraphrased from migration comments): the ledger is a durable, revisable interpretation over
  immutable capture rows; evidence points back to source rows; deleting sources removes derived actions and
  evidence, and an interval disappears only when its last evidence row is gone; empty unlocked tasks are
  garbage-collected bottom-up. User-locked tasks survive. The rebuild migration exists because the first
  worker only materialised 24 h while the UI shows 7 days.
- Evidence: `@0xa2ac281`..`@0xa2ad806`, `@0xa2ad952`..`@0xa2adb57`, `@0xa2adcab`..`@0xa2addd7`.
  Confidence: observed. Base touchpoints: 3 migrations. Size S.

### meetings_memory-014 — Deterministic activity ledger reconciler (`deterministic-v1`) (new)
Engine module `activity_ledger.rs` (target) materialises tasks/intervals/actions/evidence from capture rows.
- Producer id: `deterministic-v1`. State per producer in `activity_ledger_state.last_reconciled_at`
  (a reconcile-complete log with fields producer, tasks, intervals, start, end).
- Inputs for a window [start, end) (observed queries):
  1. Frames: one representative (minimum id) per 10-second epoch bucket per distinct
     (app_name, window_name, browser_url, document_path), restricted to focused-or-unknown frames that have
     an app or window name. Each carries app/window/url/document_path and, if the frame has a
     `semantic_run_id`, the first titled semantic item chosen by kind priority task > conversation >
     document > page > other, then `sort_order` (fields `semantic_kind`, `semantic_title`, `semantic_key`).
     attention_rank = 3 when focused else 2.
  2. UI events of type `click`, `text`, `clipboard`, `app_switch`, `window_focus` (app/window/url fall back to
     the linked frame's values when empty), with `click_count`, `element_role`, `element_name`;
     attention_rank 4.
  3. Audio transcriptions whose trimmed text is >= 8 chars, with device, speaker name, `is_input_device`;
     attention_rank 1.
  All timestamps normalised to epoch ms; rows ordered by time then id.
- Classification into tasks (keys are stable strings; titles <=256 chars):
  category per app (`category|app|<app>`, titled with the app name, or a placeholder when the app is unknown), task per semantic item
  (`item|semantic|<semantic_key>` -> titled by the semantic title), per document (`document|<path>`),
  per window (`window|<app>|<window>`), audio category (`category|audio`) with tasks per speaker/device
  (`task|audio|...`) and an `unattributed` task for audio with no known source; gaps without capture become
  `category|unobserved` / `task|unobserved` (kind `unobserved`). Exact key grammar beyond these prefixes is
  inferred; keep it deterministic so re-runs upsert the same rows.
- Actions (`action_key` = `action|<source_type>|<source_id>` inferred), `action_type` and summary
  (short one-line labels; write our own wording): `app_switch` names the app switched to,
  `window_focus` names the focused window, `clipboard_event` notes clipboard activity, `text` names where text
  was typed, `pointer_down` names the element pressed, `audio_input` marks a segment where the user spoke,
  `audio_output` marks a segment of audio that was heard; frame-only evidence produces evidence rows without actions (action_id NULL).
- Interval construction: consecutive evidence for the same task forms an interval (`interval_key`
  = `interval|<task_key>|<start>` inferred); an interval contiguous with an existing interval of the same
  producer/task whose `end_at` equals the new start is extended in place instead of inserted. The trailing
  interval that touches "now" is `provisional`; others `final`. Confidence derived from attention rank /
  evidence density (inferred formula, 0..1).
- Reconcile algorithm (observed SQL behaviour): for cut point T (previous `last_reconciled_at`): delete
  actions and evidence at/after T in producer intervals straddling T; truncate those intervals to end at T and
  mark them `final`; delete producer intervals that start in [T, window_end); then upsert tasks
  (`ON CONFLICT(task_key)`: unlocked rows take new parent/kind/title/app/confidence; locked rows keep theirs;
  producer and updated_at always refreshed), intervals (`ON CONFLICT(interval_key)`), actions
  (`ON CONFLICT(action_key)`), evidence (`ON CONFLICT(interval_id, source_type, source_id)`); then GC unlocked
  tasks with no intervals and no children; then upsert ledger state.
- Scheduling: background worker reconciles a trailing window periodically (inferred: every few minutes,
  re-processing the last ~10-15 min provisional tail) and backfills up to 7 days on first run; the API may
  also request generation for its range (a failure surfaces as a 500, see 015).
- Evidence: SQL `@0x61f8ff0`..`@0x61fbf78`, `@0x61fbfc0`; labels `@0x6a236c8`..`@0x6a23bb8`, `@0x62071c0`.
  Confidence: observed (inputs, upsert/cut semantics, labels); inferred (keys beyond prefixes, cadence,
  confidence math, gap thresholds — reuse BASE `IDLE_CAP_SECS = 300` for gap splitting).
- Base touchpoints: new `crates/screenpipe-engine/src/activity_ledger.rs`, DB functions in `db.rs`, worker
  start in `crates/screenpipe-engine/src/core.rs` / server startup. Size L. Depends on 013 (and 019 for
  semantic enrichment; must work with semantic tables empty).

### meetings_memory-015 — `GET /activity-ledger` (new)
- Handler `get_activity_ledger`. Query `ActivityLedgerQuery`: `start_time`, `end_time` (required, same
  parsing as activity-summary), `depth` (`ActivityLedgerDepth`: `task` | `action`), `include_artifacts`
  (bool), `refresh` (bool: reconcile the range before reading). Errors: 400 when start is not before end or
  when the range is longer than 31 days; 500 when the ledger query fails or on-demand generation
  fails.
- Response `ActivityLedgerResponse`: `data_status` (`ok` | `empty`), `time_range {start, end}`,
  `generated_at`, `intervals: [ActivityLedgerInterval]` each with `id`, `task_id`, `parent_task_id`, `kind`,
  `title`, `parent_title`, `app_name`, `start_at`, `end_at`, `state`, `confidence`, `producer`,
  `evidence_count`, `evidence: [ActivityLedgerEvidence { source_type, source_id, occurred_at, frame_id,
  app_name, window_title, browser_url }]` (frame_id resolved for frame and ui_event evidence; app/window/url
  coalesced from the event, then source frame, then the event's frame), and with `depth=action`
  `actions: [ActivityLedgerAction { occurred_at, action_type, summary, app_name, confidence, source_type,
  source_id }]`. Overlap selection: `end_at > start AND start_at < end`, ordered by start/end/id.
- Evidence: `@0x6208434`..`@0x6208570`, `@0x6a306a0`, `@0x61f9618`..`@0x61fa03a`; UI request
  `start_time,end_time,depth=task,include_artifacts=true,refresh=false` in home chunk (@~751823).
  Confidence: observed. Size M. Depends on 013, 014. Base touchpoints: new
  `crates/screenpipe-engine/src/routes/activity_ledger.rs`, `server.rs`, pipe allowlist optional.

### meetings_memory-016 — LLM "Activity history" generation (new, decloud)
App-side feature (`src-tauri/src/activity_history.rs` + `activity_history/quota.rs`) that turns the
ledger + summary + meetings into human-readable activities.
- Tauri commands: `get_activity_history(start, end)` -> stored entries + coverage; `generate_activity_history(
  start, end, idempotencyKey)`; `set_activity_sync_enabled`. Event `activity-history-updated`; a notification
  that activities changed, with action deeplink `open-activity-history`.
- Settings keys: `activitiesEnabled` (auto mode), `activitiesAiPresetId`, `activitiesIntervalMinutes`,
  `activitiesNextRunAt`.
- Stored model `StoredActivityHistory { schema, updated_at, coverage, entries }`, coverage per interval
  (`ActivityHistoryCoverage`), entries `ActivityHistoryEntry { id, kind ('work' etc.), meeting_id, start_at,
  end_at, title (3-8 words, past tense), summary (one sentence), evidence: [{ kind ('screen'|'audio'|
  'meeting'), at, frame_id, meeting_id, app_name, label }] }`. Storage location: app data (inferred JSON file;
  key prefix `activityHistory:activity-history-pi-v9` observed).
- Generation: runs a pi agent session with the chosen AI preset. Paraphrased contract: resolve the local API,
  read `/meetings`, `/activity-summary`, `/activity-ledger` for the exact range, then evidence-less `/search`
  per observed 30-minute window; return one JSON object with `entries`; all timestamps UTC `Z`; split on
  gaps > 15 min; include every recorded meeting >= 2 min exactly once; never invent activity for idle or
  unobserved time; minimum entry count derived from coverage. Deterministic validation rejects entries with
  `missing_required_text`, `no_valid_evidence`, `invalid_meeting_anchor`, `outside_boundary`,
  `invalid_range`, `invalid_kind`, `invalid_timestamp`, and reports missing observed windows/meetings; one
  repair pass re-prompts with the validation failures; partial valid entries are preserved without advancing
  coverage. Write Freenpipe's own prompt text.
- Quota/pause handling: vendor allowance errors (`hosted_ai_allowance_exceeded`, `free_chat_limit_exceeded`,
  `trial_cost_limit_exceeded`) pause auto generation until `resets_at`.
- Decloud: default preset `screenpipe-cloud` -> use the user's configured local/BYOK preset; drop vendor quota
  logic (keep generic provider rate-limit backoff).
- Evidence: `@0x498cfe0`..`@0x498d358`, `@0x5287b69`..`@0x5289238`, `@0x5289b90`, `@0x621d7c2`..`@0x621d9d0`,
  commands in `_next/static/chunks/8001-00ed120106af2449.js`. Confidence: observed (contract/fields);
  inferred (storage file).
- Base touchpoints: new `src-tauri/src/activity_history.rs`, `src-tauri/src/pi.rs`/`pi_command_queue.rs`,
  `commands.rs`, `specta_bindings.rs` (+ `bun run bindings:generate`), `lib/hooks/use-settings.tsx`. Size L.
  Depends on 015, 018.

### meetings_memory-017 — Home "Activity" view (new)
- Range selector `today` | `24h` | `7d` | `custom` (persisted in localStorage), AI preset picker, a
  refresh-history button, enable-automatic-activities CTA. Fetches in parallel: `/activity-ledger?...depth=task
  &include_artifacts=true&refresh=false`, `/activity-summary` with `include_key_texts=false
  &include_memories=false&include_snippets=false&include_recording=false&include_guidance=false`, and
  `/meetings?start_time=<range start - 24h>&end_time=...&limit=100` (retries up to 6 times with 750 ms * n
  backoff). Meetings are clipped into the range and titled `<app> meeting` when untitled.
- Renders activities grouped by local day; each card shows up to 6 source evidence icons (meeting, screen
  with preview frames, transcript); clicking routes to Meetings (meeting evidence) or Timeline (frame/time).
  Skeleton rows while loading; error copy for `no_capture_in_range` / `empty_but_recording`.
- Auto-refresh timer when the current-range coverage is stale by more than 10 minutes.
- Evidence: home chunk @~745866..@~758528 (REA `ev_df2f0e57...`). Confidence: observed. Size M.
  Depends on 015, 016. Base touchpoints: `apps/screenpipe-app-tauri/app/home/page.tsx`, new
  `components/activity/*`.

### meetings_memory-018 — `/activity-summary` extensions (changed)
- New query flags (all default true unless noted): `include_apps`, `include_windows`, `include_key_texts`,
  `include_recording`, `include_memories`, `include_snippets`, `include_guidance`, `include_parsed_count`,
  `include_starred`; existing `max_snippets`, `max_snippet_chars`, `max_memories`, `q`, `app_name`.
  Turning a section off omits that key; `total_active_minutes`, per-app/window `minutes`, `data_status`,
  `query_status`, `guidance` always return.
- App attribution: frames with no app name are attributed from UI events — first an event linked to the
  frame, else the latest event within a short look-back (seconds; inferred 10 s) before the frame. Window and
  URL come from the same event in that case. New `AppAttributionStatus` block with `total_frames`,
  `native_frames` (`attribution_source='frame'`), `recovered_frames` (`'ui_event'`), `unresolved_frames`.
  Apps limited to top 20, windows to top 30 by minutes then frame count.
- `parsed_context_count` / parsed count when semantic parsing is on (`include_parsed_count`).
- Starred sessions block (011).
- Evidence: `@0x522ba40`..`@0x522c9c7`, `@0x6a1a084`..`@0x6a1a550`, `@0x6a0c5b8`, `@0x524d7db`.
  Confidence: observed. Base touchpoints: `crates/screenpipe-engine/src/routes/activity_summary.rs`. Size M.

---

### meetings_memory-019 — Semantic projection schema (new)
DDL (verbatim, migration 20260724000000):
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
```
- Model: a frame points to one immutable parse run; canonical item values are content-addressed and shared
  across runs (`version_fingerprint` unique; `entity_fingerprint` groups versions of the same logical
  entity); the join table holds run-local order, hierarchy and source-node provenance (`source_node_ids` is a
  packed blob of tree node ids). No per-frame JSON copy.
- Retention GC (observed): delete runs no longer referenced by any frame; then delete items not referenced by
  any run item. Run after frame retention/deletion passes.
- Note: the hybrid/parquet storage layer in the target (other dimension) may hold frames; `frames.semantic_run_id`
  must exist in whatever frames table Freenpipe has.
- Evidence: `@0xa2a8aa2`..`@0xa2a9d37`, GC `@0x5145400`..`@0x51455bd`. Confidence: observed. Size M.

### meetings_memory-020 — Semantic actor identity schema (new)
DDL (verbatim, migration 20260727000000):
```sql
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
- Parsers keep the immutable observed actor label on `semantic_items.actor`; these tables hold a provisional,
  correctable identity. Aliases are keyed by (`source_key`, `normalized_label`) where `source_key` scopes the
  label to its source (parser/app/workspace; exact format inferred) and survive item retention so corrections
  keep applying to future captures.
- Heuristic assignment at store time (observed): if an item has an actor label and no assignment yet, look up
  the alias by (source_key, normalized_label); if absent create an actor named by the observed label and an
  alias; insert-or-ignore `semantic_item_actors(item_id, actor_id, observed_alias_id, 'heuristic')`.
- Evidence: `@0xa2a9d5f`..`@0xa2aa6dc`, `@0x6aaf535`..`@0x6aaf760`. Confidence: observed (normalisation rule
  inferred: trim, collapse whitespace, case-fold). Size S.

### meetings_memory-021 — `screenpipe-semantic` parser crate (new)
New workspace crate (target path `crates/screenpipe-semantic/src/{capture,tree,validation,messaging}.rs`,
`parsers/{catalog,families,claude,obsidian,native_macos,web_content,persisted_web}.rs`).
- Compact tree: built from the accessibility tree of a frame with a node budget (exceeding it is an
  error), required roles, off-screen node accounting (`known_offscreen_nodes`,
  `suppressed_offscreen_content_nodes/bytes`); domain-separation tags `screenpipe-semantic-tree-v1`,
  `screenpipe-semantic-entity-v1`, `screenpipe-semantic-version-v1`, `screenpipe-message-identity-v1` feed
  32-byte fingerprints (choose a 256-bit hash, e.g. BLAKE3 or SHA-256; inferred) and an 8-byte content hash.
- Registry/catalog: each parser profile declares app matchers (macOS bundle ids, Windows exe names, Linux
  names) and/or URL regexes; registration rejects profiles with no matcher, invalid URL patterns or duplicate
  ids. Observed profiles include: antigravity, asana(+web), calendar, chatgpt(+web/legacy, codex), claude
  (web + desktop + Claude Design), clickup(+web), cursor, discord(+web), fastmail, fantastical, gemini
  (desktop+web), ghostty, github issue/PR pages, gitlab issues/work items, gmail, hacker news items, hey,
  iterm2, mail, messages, messenger, outlook (desktop + 4 web hosts), microsoft teams(+web), microsoft to do
  (+web), microsoft word(+web hosts), notepad, notes, notion(+web), obsidian, omnifocus, pages, proton mail,
  slack(+web), spark (desktop/classic), superhuman, terminal, textedit, thunderbird, todoist(+web), toggl,
  vscode(+vscode.dev/github.dev), warp, whatsapp(+web), wikipedia articles, windows console/terminal,
  windsurf, xcode, zed. Parsers are grouped in families: `family.conversation`, `family.mail`,
  `family.task`, `family.calendar`, `family.document`, `family.terminal`, editor.
- Output: items of kinds `conversation|message|document|task|calendar_event|page` with `local_id`,
  `parent_local_id`, `item_key`, `identity_quality` (`stable|derived|ephemeral`), `title`, `body`, `actor`,
  `occurred_at` + `timestamp_precision`, `status`, metadata, source node ids. Message time parsing supports
  clock times, relative times (N-units-ago phrasing in en/fr/es/de), numeric dates and weekday names; statuses
  `parsed|missing_context|unrecognized`.
- Validation: undeclared kinds, items referencing missing source nodes, metadata-entry budgets -> the
  candidate "abstains" and the next candidate parser may run.
- Evidence: `@0xa2ebd9e`..`@0xa2f5eed`, catalog dump `@0xa2f1898`..`@0xa2f3c98`. Confidence: observed
  (structure, profiles); inferred (hash algorithm, per-app extraction heuristics — must be written from
  scratch by observing real accessibility trees). Size XL. Base touchpoints: new crate + `Cargo.toml`
  workspace member.

### meetings_memory-022 — Semantic projection worker + settings (new)
- Engine `semantic_worker.rs`: consumes captured frames (with accessibility trees), selects candidate
  parsers for app/url, parses, stores. Outcomes: `handled`, `handled_cached` (fingerprint already stored ->
  reuse run), `empty`, `not_handled`, `no_candidate`, `tree_missing`, `focus_incoherent`, `submitted`,
  `submitted_replaced`. Storage: insert run (`ON CONFLICT(input_fingerprint) DO NOTHING`, then read back and
  verify compatible metadata), insert items (`ON CONFLICT(version_fingerprint) DO NOTHING`), insert run items,
  heuristic actor assignment (020); frame attachments (`UPDATE frames SET semantic_run_id`) are batched and
  flushed (each flush is logged). Any failure leaves generic capture intact.
- Settings (recording settings blob): `enableSemanticContext` (bool, default **false**; documented as opt-in
  and off unless enabled), `semanticContextMode` in `memory` | `computerUse` | `both` (engine values
  `memory|computer_use|both`). Worker only starts when enabled (and logs that it is enabled).
- Telemetry fields seen (`semantic_capture_outcome`, ...) must not be sent (telemetry off).
- Evidence: `@0x6a95ad8`..`@0x6a96470`, `@0x6ab5c1a`..`@0x6ab5ed8`, `@0x6aaf7a8`..`@0x6aafdd0`,
  settings blob (`enableSemanticContext...semanticContextMode` near `@0x5177a68`-area strings). Confidence:
  observed. Size L. Depends on 019-021. Base touchpoints: new `crates/screenpipe-engine/src/semantic_worker.rs`,
  `crates/screenpipe-config` (recording settings), app settings UI (`components/settings/recording-settings.tsx`).

### meetings_memory-023 — Semantic actors API (`/semantic/actors/*`) (new)
- `GET /semantic/actors/search?q=&limit=&offset=` (`SearchSemanticActorsParams`; q <= 256 chars) — empty q
  lists all; matches canonical name or any alias observed label (case-insensitive LIKE); ordered by item
  count desc, name, id. Response actors `{ id, name, created_at, updated_at, item_count, aliases: [{ id,
  actor_id, source_label, observed_label, created_at }] }`.
- `POST /semantic/actors/create {name}` -> new actor.
- `POST /semantic/actors/update {id, name}` -> rename (bumps updated_at).
- `POST /semantic/actors/merge {actor_to_keep_id, actor_to_merge_id}` — ids must differ, both must exist;
  moves aliases and item assignments, deletes the merged actor, touches both.
- `POST /semantic/actors/reassign {item_id, actor_id}` — item must have an observed actor label; upserts
  `semantic_item_actors` with `assignment_source='reconciled'`; returns `ReassignSemanticActorResponse`.
- `POST /semantic/actors/aliases/reassign {alias_id, actor_id}` — moves the alias and all of its
  `heuristic` item assignments; explicit (`reconciled`) corrections are preserved; future observations follow
  the alias.
- Evidence: routes `@0x5257ca8`; structs `@0x522dab0`..`@0x522dd88`; SQL `@0x51a74c8`..`@0x51a810c`; doc
  `@0x511be47`..`@0x511c3ae`. Confidence: observed. Size M. Depends on 020. Base touchpoints: new
  `crates/screenpipe-engine/src/routes/semantic.rs`, `server.rs`.

### meetings_memory-024 — Parsed content in `/search` (new)
- `/search` gains `content_type=parsed` (not included in `all`), plus `frame_id` and `actor_id` filters valid
  with `parsed`. Result variant `{"type":"Parsed","content": ParsedContent}` with `frame_id`, `timestamp`,
  app/window fields, `run_id`, `parser_id`, `parser_version`, `app_platform`, `app_id`, `app_executable`,
  `parse_duration_us`, `text_bytes`, `text` (compact corrected text), `items: [ParsedItem { local_id,
  parent_local_id, kind, item_key, identity_quality, title, body, actor, timestamp_precision, occurred_at,
  status, source_nodes }]`, `actors: [ParsedActorReference { item_id/local_id, actor_id, name,
  observed_name, assignment_source }]`. Frame detail per frame: join `frames.semantic_run_id` to a `handled`
  run, items ordered by `sort_order` with resolved actor name. Text search uses `semantic_items_fts`.
- Evidence: `@0x6a3d150`, `@0x6a3d348`, `@0x6a303a0`, `@0x5249c18`..`@0x524a1c8`, doc `@0x5117b67`,
  `@0x5118653`. Confidence: observed (fields), inferred (exact search SQL). Size M. Base touchpoints:
  `crates/screenpipe-db/src/types.rs` (`ContentType::Parsed`, `ContentItem::Parsed`), `db.rs`,
  `routes/search.rs`, MCP search tool schema.

---

### meetings_memory-025 — Meeting schema deltas (changed)
DDL (verbatim):
```sql
-- 20260719000000 index meeting transcript segments captured at
CREATE INDEX IF NOT EXISTS idx_meeting_transcript_segments_captured_at_julianday
    ON meeting_transcript_segments(julianday(captured_at));
-- 20260813000000 add calendar event id to meetings
ALTER TABLE meetings ADD COLUMN calendar_event_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_meetings_calendar_event_id
    ON meetings (calendar_event_id)
    WHERE calendar_event_id IS NOT NULL AND calendar_event_id != '';
-- 20260828143000 scope live meeting speakers
ALTER TABLE meeting_transcript_segments
    ADD COLUMN stream_id TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE meeting_transcript_segments
    ADD COLUMN session_speaker_id TEXT;
CREATE INDEX IF NOT EXISTS idx_meeting_transcript_segments_session_speaker
    ON meeting_transcript_segments(meeting_id, session_speaker_id, captured_at);
```
- `calendar_event_id`: one calendar event describes exactly one meeting. Binding is conditional: set only if
  the row has none and no other meeting owns that id; a UNIQUE violation means "already bound elsewhere"
  (leave this meeting unenriched / named from its own content).
- `stream_id`/`session_speaker_id`: live-STT speaker labels are only meaningful within one provider stream;
  persist an opaque stream-scoped id so reconnects don't merge two different "speaker 1"s. Global
  `speaker_id` stays the post-hoc embedding identity. Transcript API exposes `sessionSpeakerId`.
- Timeline/transcript queries over segments should range-filter and order by `julianday(captured_at)` to use
  the expression index.
- `end_reason` gains `room_changed` (029); `find_recent_meeting_for_app` must exclude both `explicit_stop`
  and `room_changed` (BASE excludes only one, `crates/screenpipe-db/src/db.rs` ~L10094).
- Migration 20260812120000 (restore orphaned speaker references) also touches `meeting_transcript_segments`:
  re-insert nameless `speakers` rows for any speaker id referenced by audio_transcriptions, meeting segments,
  speaker_embeddings, diarization_segments, speaker_identity_evidence but missing from `speakers` (insert-only,
  idempotent). Owned by the audio/speakers dimension; listed here for meeting transcript integrity.
- Evidence: `@0xa2a8a13`, `@0xa2abf85`..`@0xa2ac027`, `@0xa2ae075`..`@0xa2ae17a`, `@0xa2abb1e`,
  `@0x6aad820` (end_reason exclusion), calendar SQL in `FROM meetings` hits. Confidence: observed. Size S.
- Base touchpoints: 3-4 migrations, `crates/screenpipe-db/src/db.rs` (meeting insert/bind/find_recent,
  segment insert), `types.rs` (`MeetingRecord`? keep fields; `MeetingTranscriptSegment` add
  `audio_start_time_secs`, `session_speaker_id`), `crates/screenpipe-audio/src/meeting_streaming/*`
  (stream id generation, Deepgram speaker label scoping).

### meetings_memory-026 — Meeting summary save + status endpoints (new)
- `POST /meetings/:id/summary` (handler `save_meeting_summary_handler`), body `SaveMeetingSummaryRequest
  { summary: string, title?: string }`. Behaviour: reject empty/whitespace summary with **400**; reject text
  that is a failure message rather than meeting content (heuristic, e.g. opens with an apology that no
  reliable summary could be made) with 400; read the current note, replace an existing `## Summary` section (heading to
  next same-or-higher heading / end) or append `\n\n## Summary\n<summary>`; user-written parts are preserved;
  optionally set `title` when non-empty; return the updated meeting (404 if the meeting
  is missing, and an error if it vanishes after the update). This replaces the pipe-side read-modify-write PUT that silently lost summaries.
- `GET /meetings/:id/summary-status?pipe=<slug>&not_before=<RFC3339>` (handler
  `get_meeting_summary_status_handler`) -> `MeetingSummaryStatusResponse { state, auto_summary_enabled,
  execution_id?, execution_status?, error_type?, error_message? }`. `state` in `idle`|`pending`|`running`|
  `ready`|`failed`. Derive from the newest `pipe_executions` row for that pipe with
  `trigger_event='meeting_ended'` and `trigger_key=<meeting id>` (or recovery keys) started at/after
  `not_before`, plus `pipe_event_runs` claims (claimed-but-not-started -> `pending`). `auto_summary_enabled`
  = whether the meeting-summary pipe is enabled with a `meeting_ended` trigger. Failed runs with
  `error_type='summary_not_saved'` report a message stating that the run ended with no summary saved.
- Meeting note UI polls summary-status after a meeting ends, shows queued/running/finalizing/completed/failed,
  streams the `## Summary` section from the live pipe run, and offers retry.
- Evidence: route list `@0x5257ca8`; OpenAPI docs `@0x6a5cea8`..`@0x6a5d2ea`; structs `@0x620a730`,
  `@0x620a790`, `@0x6a431c8`; SQL `@0x620aca0`; pipe prompt `@0xa06bdaa`..`@0xa06c177`; UI home chunk
  (@~531524). Confidence: observed (fields/states); inferred (section-replace rules, failure heuristic list).
- Base touchpoints: `crates/screenpipe-engine/src/routes/meetings.rs`, `server.rs`, `db.rs` (note update),
  `crates/screenpipe-core/assets/pipes/meeting-summary/pipe.md` (switch from PATCH to POST summary; print the
  `## Summary` section before saving), `components/meeting-notes/note-view.tsx`. Size M. Depends on 028.

### meetings_memory-027 — Meeting summary finalizer / recovery worker (new)
Engine `meeting_summary/{finalizer,notes,extract}.rs`.
- After a `meeting-summary` pipe execution triggered by `meeting_ended` completes: if the meeting note has no
  saved summary, extract the last assistant `## Summary` section from the run's stdout (pi JSON event stream:
  `messages`/`role`/`content`/`type`) and save it via the same note-merge logic (logging the recovery
  from the run); if nothing usable, mark the execution `failed` with `error_type='summary_not_saved'`
  (only when it was `completed` without error), or `nothing_to_summarize` when no speech was captured.
- On startup: find `meeting_ended` runs with `error_type='interrupted'` that have no newer run for the same
  `trigger_key` (limit 20) and requeue them (`interrupted_by_restart`), emitting
  `meeting_summary_refresh_requested`; recovery runs use keys like `<meeting_id>:summary-recovery-<n>`
  (counting prior attempts with `trigger_key = id OR LIKE 'id:%'` since the meeting ended minus 1 s).
- Trivial meetings: when a meeting ends with too few transcript segments, `meeting_ended` is still emitted but
  flagged so event-triggered pipes skip it (the log gives the segment count
  and notes the skip). Payload fields: `meeting_id`, `meeting_end`, `duration_secs`, `transcript_segments`. Threshold
  inferred (e.g. < 3 segments or < 2 min).
- Retranscribe integration: `POST /meetings/:id/retranscribe` (exists in BASE) now replaces segments and can
  request a summary refresh (`summary_refresh_event_emitted`, `transcript_updated_at`; max 2 h per pass).
- LLM: whatever preset the meeting-summary pipe uses. Decloud: BASE pipe default preset `screenpipe-cloud`
  must default to the user's own preset.
- Evidence: `@0x6ab2f50`..`@0x6ab39d9`, `@0x6207240`..`@0x620743e`, `@0x6207a80`, `@0x6208120`..`@0x6208211`.
  Confidence: observed (flows, keys, error types); inferred (trivial threshold). Size M. Depends on 026, 028.
- Base touchpoints: new `crates/screenpipe-engine/src/meeting_summary/*`, `crates/screenpipe-core/src/pipes/mod.rs`
  (completion hook), `routes/retranscribe.rs`.

### meetings_memory-028 — Idempotent event-triggered pipe runs (new)
DDL (verbatim, migrations 20260728000000, 20260805000000):
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
-- 20260805000000
ALTER TABLE pipe_executions ADD COLUMN conversation_id TEXT;
```
- `meeting_ended` is not once-per-meeting (a 120 s rejoin window reopens the same row and it ends again).
  Before starting an event-triggered run, the scheduler claims `(pipe, event, key)` with INSERT OR IGNORE; only
  the inserter runs. Key for `meeting_ended` = meeting id. Record `trigger_event`/`trigger_key` on the
  execution. Prune claims by age (inferred: 7-30 days).
- `conversation_id` gives pipe runs that continue an agent session a stable chat identity (pipes dimension).
- Evidence: `@0xa2aa964`..`@0xa2aab8e`, `@0xa2ab414`, `@0x620a358`. Confidence: observed. Size S.
- Base touchpoints: `crates/screenpipe-core/src/pipes/mod.rs` (event trigger path), `db.rs`.

### meetings_memory-029 — Meeting room change (new)
- Situation: the same audio session (same browser/app process) moves to a different call (different
  conference URL/room). The audio-process detector emits engine event `meeting_room_change_offer`
  (payload includes meeting id, app, a one-time `token`). The app shows a notification/prompt (category
  respects prefs; a skip is logged) that says the app may now be in a different room and asks whether to keep
  everything in one recording or begin a fresh note, with one action that keeps the current note
  (`keep-meeting-note`, decision `keep`) and one that starts a new note (`start-new-meeting-note`, decision
  `switch`).
- `POST /meetings/room-change` (handler `resolve_room_change_handler`), body `ResolveRoomChangeRequest
  { meetingId, token, decision: 'switch' | 'keep' }` -> 400 when `decision` is any other value, 409/410
  when the prompted meeting is not active any more. Engine forwards `meeting_room_change_response`. On `switch` the
  detector ends the current meeting with `end_reason='room_changed'` and starts a new one
  (`detection_source`/reason `audio_process_room_change`); on `keep` it continues. If unanswered the detector
  resolves it itself (it logs whether the user or a timeout settled it; inferred default keep).
- Evidence: `@0x620add8`, `@0x514c678`, `@0x498f8bf`..`@0x498f928`, `@0x6ab3e28`..`@0x6ab3ed6`,
  `@0x6ab45d0`..`@0x6ab4779`, `@0x522b693` (`RoomChangeChoice`, `MeetingRoomChangeResponse`). Confidence:
  observed (fields/decisions); inferred (timeout default). Size M. Depends on 030.
- Base touchpoints: `routes/meetings.rs`, `server.rs`, `src-tauri/src/meeting_live_notes.rs`,
  `crates/screenpipe-events/src/custom_events/meetings.rs`.

### meetings_memory-030 — Audio-process meeting detector (new)
Engine `meeting_watcher/audio_process/{mod,lifecycle,resolve,events,windows}.rs` + audio
`meeting_processes.rs` (Windows WASAPI).
- Signal: which processes currently hold an audio capture (mic) session. Windows: enumerate audio endpoints
  via `IMMDeviceEnumerator`, then `IAudioSessionManager2`/`IAudioSessionEnumerator` per endpoint, collect
  process ids with active sessions (`wasapi:` / `wasapi-session:` evidence ids). RDP/VDI sessions without a
  client proxy are flagged as degraded. Output-audio activity is also tracked (`has_output_audio`).
- Resolution: map the mic-holding pid to a meeting profile (Zoom `zoom.exe`, Teams `ms-teams.exe`/`teams.exe`,
  browsers `chrome.exe`, `msedge.exe`, `firefox.exe`, `brave.exe`, `opera.exe`, Slack huddles, Webex...) and
  confirm with fresh evidence: recent frames (app/window/url within the last 10 s from the DB), an
  active-tab URL probe for browsers (meet.google.com, zoom.us/j|wc|my, teams.microsoft.com, teams.live.com,
  webex.com, app.slack.com/huddle), and a bounded call-control UI scan (UIA on Windows: hang-up/leave/mute
  controls and a back-to-the-call control, matched by their accessible names, plus Zoom container names).
  Timeouts/failures of the scan withhold the start (and are logged as such). A mic held
  without platform evidence is logged and retried every poll. Ignored apps from settings `ignoredMeetingApps`
  are excluded.
- Lifecycle: start inserts a meeting (`detection_source='audio_process'`), handles the single-open unique
  index violation (`idx_meetings_single_open`) by waiting/adopting; doesn't start while a manual meeting is
  active; reopens the most recent meeting of the same app ended within the rejoin window (120 s) unless its
  end_reason is `explicit_stop`/`room_changed`, the session is in a different conference room, or its
  calendar event ended and the time now belongs to the next event (`audio_process_reopen`); binds calendar
  events (031); emits `meeting_started`/`meeting_ended`; auto-ends inactive live meetings; closes orphaned
  open meetings at startup (`end_reason='auto_end'`) or re-attaches an active one; ends meetings on shutdown
  (`end_reason='shutdown'`); explicit stop (`detector_stop_tracking`) moves the detector to idle.
  Re-resolves the meeting pid so per-process capture re-engages; flap counting/hysteresis on transitions.
- Interplay with per-process audio tap ("meeting piggyback", `experimentalMeetingPiggyback`,
  `windows_output_follow`) belongs to the audio dimension; the detector publishes the meeting pid it uses.
- Evidence: `@0x6ab3a38`..`@0x6ab4db7`, `@0x6ab4278`, `@0x6a06668`..`@0x6a06e58`, `@0x6aa5293`,
  `@0x6af6f18`..`@0x6af7460`, `@0x6a12f58`..`@0x6a13030`, profile literals `@0x6a11f80`..`@0x6a125a0`.
  Confidence: observed (signals, rules, messages); inferred (polling cadences, scoring). Size XL.
- Base touchpoints: `crates/screenpipe-engine/src/meeting_detector.rs` (split into
  `meeting_watcher/{audio_process,ui_scan,shared}`), `crates/screenpipe-engine/src/meeting_watcher.rs`,
  `crates/screenpipe-audio/src/meeting_detector.rs`, new `crates/screenpipe-audio/src/meeting_processes.rs`.

### meetings_memory-031 — UI-scan detector ("meeting v2") and shared calendar changes (changed)
- Same UI-scan approach as BASE, reorganised; changes: (a) calendar match requires an exact conference
  identity — method `exact_conference_url` when the event's meeting URL matches the detected call; `time_only`
  matching is logged as a skipped match (lacking an exact conference URL match) and does not bind;
  (b) binding goes through the conditional `calendar_event_id` update (025) so the same event never titles two
  meetings; (c) active meetings are preserved across capture restarts; (d) when no meeting app process is
  found but output audio is still active, keep the meeting alive (Ending -> Active) instead of ending it;
  (e) `meeting_ended` carries `transcript_segments` and trivial-meeting gating (027);
  (f) Windows UIA scanning with pid-scoped scans and panic isolation.
- Evidence: `@0x6ab5068`..`@0x6ab52a8`, `@0x6a77c78`..`@0x6a77d15`, `@0x6a95948`..`@0x6a959e5`,
  `@0x49f0afe`. Confidence: observed. Size L. Base touchpoints:
  `crates/screenpipe-engine/src/meeting_detector.rs`, `src-tauri/src/google_calendar.rs`,
  `src-tauri/src/calendar.rs` (events carry `excludeFromMeetingDetection`, `meetingUrl`).

### meetings_memory-032 — Meeting API shape changes (changed)
- `StartMeetingRequest` gains `calendar_event_id` (claim/bind on manual start; a failed claim is
  reported as an error). Manual start adopts an already-active auto-detected meeting instead of failing
  (with a log of the adoption), idempotently enriching title/attendees;
  `detection_source` values add `manual_start_possible_missed_detection`. A second open row maps the
  `idx_meetings_single_open` UNIQUE error to **409** (a different meeting is already open).
- Stop: `explicit_stop` end_reason; an error when the requested id is not the currently active meeting; emits
  `detector_stop_tracking` then `meeting_ended`.
- Split/merge validation: merge needs >= 2 ids; split `at` must be RFC3339 strictly between start and end.
- `GET /meetings/:id/transcript` segments add `audioStartTimeSecs`, `sessionSpeakerId` (camelCase, ordered
  `ascending`/`descending` option).
- `POST /export` gains `include_audio` (bool); export range gate 042.
- `GET /meetings` `q` searches title, attendees, note and meeting_app (case-insensitive substring).
- Evidence: `@0x620a1f0`..`@0x620abe0`, `@0x51a8150`, `@0x6a303a0`. Confidence: observed. Size S.
- Base touchpoints: `crates/screenpipe-engine/src/routes/meetings.rs`, `db.rs`, `types.rs`.

### meetings_memory-033 — `/ws/meeting-overlay` and overlay meeting strip (new)
- New engine WebSocket `/ws/meeting-overlay` (authenticated like other `/ws/*`). Messages:
  `{type:"status", data: MeetingStatusResponse}` (same camelCase shape as `/meetings/status`:
  `active`, `manualActive`, `activeMeetingId`, `stoppableMeetingId`, `meetingApp`, `detectionSource`),
  `{type:"snapshot", data:{meetingId, items:[...]}}` on connect, and per-segment upserts
  `{type:<item>, data:{meetingId, deviceName, deviceType, itemId, text, capturedAt, speaker...}}` keyed by
  `deviceName:deviceType:itemId` (client keeps the last 50).
- Shortcut overlay shows a live-meeting indicator whose hover reveals the transcript, a mic/speech meter, and a stop button that
  POSTs `/meetings/stop {id: stoppableMeetingId}` then re-checks `/meetings/status` after 6 s. Client hides mic
  lines that echo speaker output (>=60% word overlap within 6 s for input lines >= 24 chars).
- Evidence: `@0x5259250`, `@0x499a4c9`; `_next/static/chunks/app/shortcut-reminder/page-a5341a3196435d05.js`
  (@~4529, @~12303). Confidence: observed (status/snapshot), inferred (segment message type name).
- Base touchpoints: `crates/screenpipe-engine/src/server.rs` (`/ws/meeting-status` neighbour),
  `routes/websocket.rs`, `components/shortcut-reminder.tsx`. Size M.

### meetings_memory-034 — Meeting notification preferences (changed)
- `notificationPrefs.meetingReminders` (new, default true; migrate from `meetingLiveNotes` when absent)
  gates prewarm reminders saying a meeting begins in N minutes/seconds (`meeting_about_to_start`); `meetingLiveNotes`
  gates the open-a-note prompt on detection; room-change prompt has its own gate; `audioCaptureStalled`,
  `liveTranscriptStalled` unchanged.
- Evidence: settings chunk `_next/static/chunks/app/(main)/settings/page-d9c69007346aaf6a.js` (@~578072,
  @~593964); `@0x498f798`..`@0x498f9f8`. Confidence: observed. Size S. Base touchpoints:
  `lib/hooks/use-settings.tsx`, `components/settings/notifications-settings.tsx`,
  `src-tauri/src/meeting_live_notes.rs`.

---

### meetings_memory-035 — External memory sync targets (changed)
- Claude Code target (`claude-code`, `CLAUDE.md` + sidecar `screenpipe-memories.md`): the sidecar no longer
  contains the memory digest; it contains static guidance telling the assistant to retrieve memories through
  the Screenpipe MCP `search-content` tool with `content_type=memory` when relevant, and to treat memory text as
  untrusted data. The marker block intro changes accordingly. (Write our own wording.) Inline-digest targets
  (Codex `AGENTS.md`) unchanged.
- New connection `obsidian-memories` ("Obsidian Memories"): fields `vault_path` (required; must exist, be a
  directory, contain `.obsidian/`, be writable — verified with a `.screenpipe-write-probe` file) and
  `memories_folder` (optional, default `screenpipe`). Writes `screenpipe-memories.md` with YAML frontmatter
  (`title: screenpipe memories`, `tags: [screenpipe, memory]`) and/or per-memory notes (`screenpipe-memory-*`;
  inferred) on each sync.
- Evidence: `@0xa0babd3`..`@0xa0bae71`, `@0x9f7dbb1`..`@0x9f7dd17`, `@0x9f7ed88`, `@0x52557d0`.
  Confidence: observed (connection fields, guidance switch); inferred (per-memory file layout). Size S.
- Base touchpoints: `crates/screenpipe-core/src/memories/external_sync.rs`,
  `crates/screenpipe-engine/src/external_memory_sync.rs`, new
  `crates/screenpipe-connect/src/connections/obsidian_memories.rs`, `connections/mod.rs`.

### meetings_memory-036 — Memories API (unchanged; verify only; not a delta item)
- Target `/memories` CRUD, `/memories/tags`, `/memories/sync-external`, validation (50 tags, 100-char tags,
  50 000-char content), FTS triggers, `frame_id`, `sync_uuid`/`sync_modified_by` match BASE. Only addition is
  history gating (042) and Brain UI (006).
- Evidence: `@0x6a5dd2e`..`@0x6a5df58`, `@0x516de68`..`@0x516e118`, `@0x511c3e1`..`@0x511cb36`.
  Confidence: observed. Only regression tests are needed; it is not reported as a delta.

---

### meetings_memory-037 — `/notify` payload v2 and delivery gates (changed)
Tauri sidecar server on port **11435** (`SCREENPIPE_FOCUS_PORT`-style override; BASE same port).
- `NotifyPayload` fields: `title`*, `body`* (markdown), `id`, `type` (default `pipe`; also `system`,
  `meeting`, `capture_stall`/`captureStalls`, `disk_pressure`, `db_recovery`, `workflow_review`...),
  `priority` (`high` | `normal` (default) | `low`), `timeout` / `autoDismissMs` (ms, default 20000),
  `actions` (<= 5, each needs `id`, `label`, `type`), `pipe_name`/`pipeName`, `source_session_id`/
  `sourceSessionId`, `source_message_id`/`sourceMessageId`, `source_url`/`sourceUrl`, `transient`,
  `workflow_ids`, `surface`, `position`, `anchor`, `cta`, `survey`, `dismissible`, `expires_at`/`expiresAt`.
  Request header `x-screenpipe-session` identifies the sender session. Response `{"success":true}`.
- Action types: `link` (web URL), `deeplink` (`screenpipe://...`), `pipe` (run installed `pipe` with optional
  `context`; `open_in_chat` surfaces the run in chat), `chat` (run inline `prompt` in a fresh chat with
  optional `context`, `auto_send`), `api` (POST/other `method` to a local endpoint `url` with `body`, optional
  `toast`), `dismiss`, plus UI-only `copy`, `source` (open originating chat), `manage`, `mute`,
  `meeting_join`. Built-in action ids `open_timeline`, `open_chat`, `restart_recording`.
- Gates (in order): master `notificationsEnabled`; snooze (`snoozeUntil` epoch ms) and quiet hours
  (`quietHours {enabled,start,end}` default 22:00-08:00 off) — such notifications are dropped with a log naming the pause reason (master/snooze/quiet),
  except pipes listed in `allowDuringPause`; per-category toggles (pipe notifications ->
  `pipeNotifications`); per-pipe mute (`mutedPipes`); pipe permission check — if the sending pipe's
  `pipe.md` permissions deny `POST /notify`, drop it; duplicate suppression of identical alerts shown recently
  (inferred window ~minutes) and idempotency on `id` (a repeat of an already-stored id is not stored again).
  `transient` notifications are delivered without an inbox row.
- Body links: `~/...`, absolute paths and `C:\...` open in the in-app viewer for `.md` (`openNotePath`) or
  default app; `screenpipe://` handled in-app; web links in browser (BASE rewrite logic kept).
- Evidence: `@0x514b203`..`@0x514c120`, `@0x5287138`, doc `@0x511cb5d`..`@0x511d7d2`, panel chunk
  `_next/static/chunks/app/notification-panel/page-96fa5ccf9963fc66.js`. Confidence: observed (fields,
  gates); inferred (dedupe window). Size M.
- Base touchpoints: `apps/screenpipe-app-tauri/src-tauri/src/notifications/routes.rs`, `rewrite.rs`,
  `client.rs`, `app/notification-panel/page.tsx`, `components/notification-handler.tsx`.

### meetings_memory-038 — Notification history store v2 (changed)
- File stays `<data_dir>/notifications.json` (cap BASE 100; keep). `NotificationHistoryEntry`: `id`, `type`,
  `title`, `body`, `pipe_name`, `source_session_id`, `source_message_id`, `source_url`, `timestamp`, `read`,
  `actions` (persisted so the user can approve later from the bell), `priority` (`NotificationPriority`).
- Routes on :11435: `GET /notifications`, `POST /notifications` (mark all read), `DELETE /notifications`
  (clear), `DELETE /notifications/:id` (dismiss), **new** `POST /notifications/:id/read` (mark one read;
  404 for an unknown id). Writes are atomic (temp + rename) and errors logged.
- Evidence: `@0x6201c7b`..`@0x6202208`, `@0x514a9f6`, inbox chunk (@~9399..@~9851). Confidence: observed.
- Base touchpoints: `src-tauri/src/notifications/store.rs`, `routes.rs`, `src-tauri/src/server.rs`. Size S.

### meetings_memory-039 — Notification inbox window and bell (new)
- New static page/window `notification-inbox` (Tauri `show_notification_inbox`), sized popover-like; the
  bell in the main window uses the same component.
- Behaviour: polls `GET /notifications` every 5 s; tabs `priority` (default) and `all` with counts; the
  unread-dot/aria label counts unread high-priority items. Classification: explicit `priority` wins; otherwise
  high when the item has any labelled action other than dismiss/copy/source, or its type is
  `capture_stall`/`capturestalls`/`disk_pressure`/`db_recovery`, or its title contains one of a fixed
  list of six phrases about recording or audio having stopped/paused, capture not running, the database needing
  recovery, or the live transcript stalling; else normal. Two sections: the high-priority group first, then the rest.
  Expanding marks read (`POST /notifications/:id/read`); per-item dismiss (`DELETE /notifications/:id`), clear
  all (`DELETE /notifications`), copy, open source chat (`screenpipe://chat/<session>?message=<id>`), run
  actions, feedback (008), a link to notification settings (`/settings?section=notifications`).
- Evidence: `_next/static/chunks/app/notification-inbox/page-1108e01f2e609f2b.js` (REA
  `ev_27381aee90c77a8867686aa3a01d65a60222f9492c1809d4eb3f3d4b79d1e002`), `7737-fae548fffc70b86b.js`
  (module 69520), `@0x4999dd0`. Confidence: observed. Size M.
- Base touchpoints: new `apps/screenpipe-app-tauri/app/notification-inbox/page.tsx`,
  `components/notification-bell.tsx`, `src-tauri/src/commands.rs` + `window/*`, bindings regen.

### meetings_memory-040 — Notification preferences v2 (changed)
- `notificationPrefs` keys: `notificationsEnabled` (master, default true), `snoozeUntil` (0), `quietHours`
  (`{enabled:false,start:"22:00",end:"08:00"}`), `allowDuringPause` (pipe names), `mutedPipes`, and category
  toggles with defaults: `captureStalls` false (mirrors `showRestartNotifications`), `meetingReminders` true,
  `meetingLiveNotes` true, `audioCaptureStalled` true, `liveTranscriptStalled` true, `displayChanges` true,
  `powerModeChanges` true, `pipeNotifications` true (per-pipe), `pipeAllowanceWarnings` true (vendor allowance
  -> drop or repurpose for BYOK budget), `appUpdates` true. Groups: recording, meetings, system, automation,
  app. Snooze presets 30 min / 1 h / 2 h / until tomorrow; quick presets all/none/... .
- Evidence: settings chunk (@~578072..@~594884). Confidence: observed. Size S. Base touchpoints:
  `lib/hooks/use-settings.tsx`, `components/settings/notifications-settings.tsx`,
  `src-tauri/src/notifications/routes.rs` (gates read these keys).

### meetings_memory-041 — Additional notification producers (new)
- First-run summary (app `first_run_summary.rs`): a few minutes after setup, fetch `/activity-summary`
  (`include_windows`, `include_parsed_count`, `max_snippets=6`, `max_snippet_chars=240`), build an observation
  list (minutes since setup, screens indexed, top apps/windows, audio summary), and ask the local AI (pi,
  no tools) for a 2-4 sentence second-person message about the user's own work; reject apologetic/invalid
  outputs; notify the user that the first summary is available, with deeplink `screenpipe://first-run-summary`. Write our own
  prompt. Decloud: use local/BYOK preset.
- Activity history updates (016), database recovery and disk pressure notices (`db_recovery`,
  `disk_pressure` types with actions like `recover-database`, `screenpipe://database-recovery`), workflow
  review notices (`workflow_review`, owned by workflows dim), audio-health notices — all go through 037.
- Evidence: `@0x498e97e`..`@0x498eda0`, `@0x5174798`..`@0x5174f8f`, `@0x4990558`..`@0x49908b0`,
  `@0x514cf40`..`@0x514d709`, `@0x52871e4`..`@0x5287570`. Confidence: observed. Size M (first-run only;
  others owned elsewhere).

### meetings_memory-042 — History-access gating (ee-stub)
- Target restricts some accounts to 24 h of history: errors `history_access_limited` (message says the
  requested meeting/content/export lies beyond the 24-hour window), Tauri `is_history_access_restricted`, Activity range
  limited to today/24h, starred sessions refuse starts outside accessible history.
- Freenpipe: implement a single `history_access` helper that always returns "unrestricted" (no account
  checks), keep the error code constant for API compatibility but never emit it.
- Evidence: `@0x5152f2d`, `@0x59fbb50`, `@0x620a240`, `@0x6a5dd2e`, `@0x6a64d61`, `@0x5183aa8`.
  Confidence: observed. Size S. Base touchpoints: new `crates/screenpipe-engine/src/history_access.rs`
  (stub), call sites in routes above.

### meetings_memory-043 — `screenpipe-team-memory` crate (removed)
- BASE contains `crates/screenpipe-team-memory` (file-per-memory markdown format) but nothing depends on it;
  the target has no team-memory strings at all. Recommendation: leave it unwired (or drop it from the
  workspace) — no feature work.
- Evidence: zero hits for `team.memory|team_memory` in `app_strings.tsv`; BASE `Cargo.toml` grep shows no
  dependents. Confidence: observed. Size S.

### meetings_memory-044 — Vault online lock refused (changed)
- Target `POST /vault/lock` refuses while the server runs (the error explains that the live server holds open
  SQLite connections and directs the user to stop it and use the CLI `screenpipe vault lock` instead); setup
  refuses for hybrid storage (an error that vault protection is not supported there); vault also covers
  `secrets.sqlite` (+journal). `/vault/status|unlock|setup` routes unchanged.
- Evidence: `@0x522d6fa`, `@0x522ae21`, `@0x522af47`. Confidence: observed. Size S. Base touchpoints:
  `crates/screenpipe-engine/src/routes/vault.rs`, `crates/screenpipe-vault/src/manager.rs`, CLI `vault lock`.

### meetings_memory-045 — Pipe permission allowlists for this dimension (changed)
- Default reader allowlist adds `GET /starred-sessions`, `GET /workflows`, `GET /workflows/*`,
  `GET /feedback` to BASE's 13 entries. Additional named rule sets observed: meeting/memory writer
  (`POST /meetings/start`, `POST /meetings/stop`, `PUT /meetings/*`, `POST /meetings/merge`,
  `POST /memories`, `PUT /memories/*`, `DELETE /memories/*`) and structured output
  (`GET /outputs/targets`, `POST /outputs/targets/*/submit`). The meeting-summary pipe needs
  `POST /meetings/*/summary` (inferred: grant to built-in meeting-summary pipe). Starred-session writes and
  feedback writes are denied to pipe tokens.
- Evidence: `@0xa0c7f30`..`@0xa0c82c0`. Confidence: observed (lists), inferred (summary grant). Size S.
- Base touchpoints: `crates/screenpipe-core/src/pipes/permissions.rs`,
  `crates/screenpipe-core/assets/extensions/screenpipe-permissions.ts`.

---

## 3. Implementation order (suggested)
1. Migrations 001/002/003/007/010/013/019/020/025/028 (schema first; one PR, with a BASE-DB upgrade test).
2. Feedback API (007) and starred API (010/011), artifacts API (004) — small, high-value.
3. Meeting summary endpoints + finalizer (026-028), room change + API shape (029/032).
4. Notifications v2 (037-040) and inbox (039), rating UI (008).
5. Activity ledger reconciler + API + UI (014/015/017), activity-summary changes (018).
6. Semantic crate + worker + actors + parsed search (021-024) — largest, behind `enableSemanticContext`.
7. Audio-process detector (030/031), overlay WS (033), LLM activity history (016), first-run summary (041).

## 4. Open questions
- Exact numeric caps not visible in strings: structured-output target/consumer limits, payload/schema byte
  caps, feedback comment max length, notification dedupe window, trivial-meeting threshold, ledger cadence
  and confidence formula, `/ws/meeting-overlay` per-segment message `type` name.
- On-disk layout under `~/.screenpipe/outputs/` (subfolders per source_type/source) and the activity
  history storage file name.
- Whether `/live-views/*` (dashboards that define structured-output targets) is owned by another dimension
  (assumed yes).
- Hash function used for semantic fingerprints (affects reuse of a v2.7.104-written DB's
  `semantic_runs`; new runs simply won't dedupe against old ones if it differs).

---

## Verification

Adversarial pass over items 001-045 (2026-10-14). Method: (a) BASE grep of every claimed identifier over
`<repo>` (clean tree: only the import and rules commits), then reading the base files that
matched; (b) every cited `app_strings.tsv` offset checked by proximity (`tools\mmv_check.py near`, 150 of 162
needle checks hit; the 12 misses were wording/case differences at otherwise relevant offsets, plus one wrong
offset noted under 032), whole-file identifier counts (`mmv_check.py count`), frontend needle counts and context
windows (`mmv_fe.py`, `mmv_ctx.py`), the migration table re-decoded from the exe (`mm_migtable.py`, static
read), Tauri command diff against base `lib/utils/tauri.ts` (`mmv_cmds.py`), and target source-module paths
diffed against base files (`mmv_paths.py`, `mmv_apppaths.py`); (c) duplicate check within this list. Overlaps
with sibling specs are listed for the coordinator but do not change verdicts, because every dimension
re-describes the same routes, tables or UI from its own angle.

### Verdicts

| Item | Verdict | Notes |
|---|---|---|
| 001 | partially_in_base | Table re-decoded: 117 records at exe `0xa2ae3f0` with stride 0x58, all 17 cited versions present, 28 versions not in BASE's 89 files. BASE already sets `set_ignore_missing(true)` and rewrites mismatched checksums, and sqlx applies an out-of-order pending version. So the "keep" half is base; only the version adoption is new, and each schema item restates it. Canonical owner is db-001. |
| 002 | confirmed | DDL verbatim at `@0xa2a1af2`/`@0xa2a2d2a`/`@0xa2aad72`; insert `@0x51a63e0`; structured-output exclusion `COALESCE(o.saf_kind,'') != 'structured-output'` at `@0x5a5fb40`/`@0xa27b150`. BASE: 0 hits. Overlap db-002. |
| 003 | confirmed | DDL `@0xa2a6390`/`@0xa2a651e`; replace-by-rowid `@0x51a6920`; `MATCH ... ORDER BY bm25(...)` `@0x5a5f750`; artifact-search backfill log `@0x6ab7ac1`. BASE: 0. |
| 004 | confirmed | Routes `@0x5257ca8`; 100 MB size-limit error `@0x522df48`; `pipe-run` `@0x49abbf0`; `per_pipe_limit`/`include_internal` `@0x618947b`; `.saf.json` `@0x522d3b8`; frontmatter `artifacts:` with `path`/`title`/`kind` `@0xa0644d2`. BASE: no `/artifacts` route (its 2 hits are unrelated). Overlap api-017, agents_pipes-010. |
| 005 | confirmed | Four routes `@0x5258618`..`@0x52586c0`; request structs `@0x522d17b`; actions resolve/snooze/correct/dismiss/reopen `@0x6a240f0`; validation messages `@0x6a24415`; `targets.json` `@0x6a23bec`. The evidence cap of 50 was not seen as a literal (treat as inferred). BASE: 0. Overlap api-019, agents_pipes-011. |
| 006 | confirmed | Home chunk @297852: Brain header fetches `/memories?limit=1&offset=0` and `/artifacts?limit=1&offset=0` on a 30 s interval; `saf_kind` @337422; `mem:` selection keys @291043. BASE has memories only in Settings. Overlap frontend-021. |
| 007 | confirmed | DDL `@0xa2aaf2a`; comment-requires-down error `@0x5183665`; self-rating refusal `@0x51836c8`; `local-user` `@0x518365b`. BASE `/feedback` hits are the support-feedback dialog, not this API. Overlap db-010, api-018, agents_pipes-044. |
| 008 | confirmed | `3822-...js` @8875-9606 posts `/feedback` with `kind` `notification`/`chat`, `producer_ref` `pipe:<name>` / `agent:screenpipe-chat`, 12000-char response snapshot; `7737-...js` @37224 rating labels; `3696-...js` @485200 chat good/bad buttons. BASE has no rating UI. Overlap frontend-014. |
| 009 | partially_in_base | BASE already uploads logs to `https://screenpi.pe/api/logs` and `/api/logs/confirm` (`components/share-logs-button.tsx` L167-270, `app/overlay/page.tsx` L282-298, `components/onboarding/engine-startup.tsx` L768) with local regex `redactPii`. New in target: Rust `feedback_upload.rs`/`feedback_redact.rs`, Tinfoil-hosted redaction `@0x498c790`, video attachment `@0x5a63148`, Tauri `start_feedback_upload`/`redact_pii_for_feedback`. The decloud must cover the base JS paths too. Overlap cloud-036, shell-033, frontend-046. |
| 010 | confirmed | DDL `@0xa2ae26f`; `has_audio` list SQL `@0x51a6920`+594; engine `routes\starred.rs` `@0x5183a30`; errors `@0x5183aa8`..`@0x5183bb8`; 409 rule in doc `@0x511e39f`. BASE: 0. Overlap db-015, api-007, vision-starred-hd. |
| 011 | confirmed | `/search` params `@0x52ca6b8`; summary fields `@0x522ba40`+424; `include_starred` `@0x522bba0`. BASE: 0. |
| 012 | confirmed | Page chunk present; `toggle_starred_sessions`/`hide_starred_sessions` in `8001-...js` @48410 and exe `@0x4993871`/`@0x4994107`; events `@0x524e740`..`@0x524e7b7`, `@0x621fdbd`; `starSessionShortcut` 17 FE hits; CLI doc `@0x6a5ac69`. BASE: 0. Overlap frontend-029. |
| 013 | confirmed | DDL `@0xa2ac281`..; reset `@0xa2adab1`; producer range index `@0xa2add95`. BASE: 0. Overlap db-013. |
| 014 | confirmed | Upsert SQL `@0x61f8ff0`; labels `@0x6a236c8`..`@0x6a2390c` (incl. `pointer_down`, `clipboard_event`, `audio_input`/`audio_output`, the unattributed-audio task title); reconcile log `@0x62071c0`. BASE knows `app_switch`/`window_focus`/`clipboard_event` only as ui_event types; there is no reconciler. |
| 015 | confirmed | Route in table `@0x5257ca8`+1401; errors `@0x6208434`; `ActivityLedgerDepth`/`include_artifacts` `@0x6a306a0`; UI request @751822. BASE: 0. Overlap api-006. |
| 016 | confirmed | Commands (`8001-...js`, exe `@0x499396a`, `@0x5111e88`, `@0x5178aca`); settings keys `@0x498d8c8`..`@0x5289296`; events `@0x498d280`/`@0x498d300`; agent contract `@0x5288703`; `quota.rs` with `hosted_ai_allowance_exceeded` `@0x621d7ff`. BASE: 0. Overlap frontend-023. |
| 017 | confirmed | Home chunk: `/activity-ledger` @751822, `getActivityHistory` @754188, `generateActivityHistory` @755108, `empty_but_recording` @756085. Overlap frontend-023. |
| 018 | partially_in_base | BASE `routes/activity_summary.rs` already has `include_recording`/`include_memories`/`include_snippets`/`include_guidance` (default true), `max_snippets`/`max_snippet_chars`/`max_memories`, and the top-20 apps / top-30 windows limits (L389, L409). New: `include_apps`, `include_windows`, `include_key_texts`, `include_parsed_count`, `include_starred`; UI-event attribution (`frame_fallback` CTE `@0x6a1a2bb`, `native_frames`/`recovered_frames`/`unresolved_frames` `@0x522baf0`, `AppAttributionStatus` `@0x6a0c643`); `parsed_context_count`. Overlap api-005. |
| 019 | confirmed | DDL `@0xa2a8c7a`..`@0xa2a9d37`; GC `@0x5145400`. BASE: 0. Overlap db-008. |
| 020 | confirmed | DDL `@0xa2a9f4e`..`@0xa2aa6dc`; alias lookup `@0x6aaf5a0`. BASE: 0. Overlap db-008. |
| 021 | confirmed | Source paths `crates\screenpipe-semantic\src\{capture,tree,validation,messaging,parsers\*}.rs`; the four domain tags `@0xa2ec148`..`@0xa2ed648`; node-budget error `@0xa2ec528`; `family.conversation` `@0xa2f07c0`; catalog `@0xa2f1898`; weekday regex `@0xa2f5eed`. BASE: 0. |
| 022 | confirmed | `semantic_worker.rs` `@0x6a77e60`; settings keys `@0x52307bf`; `handled_cached` `@0x6ab5c1a`; attachment-flush log `@0x6ab5e08`; modes `memory`/`computer_use`/`both` `@0x6a96470`. BASE: 0. |
| 023 | confirmed | Six routes `@0x5257ca8`; structs `@0x522dab0`; SQL `@0x51a75ca`; doc `@0x511be47`. BASE: 0. Overlap api-008. |
| 024 | confirmed | `ParsedItem`/`ParsedContent`/`ParsedActorReference` `@0x6a3d150`..`@0x6a305c8`; `parsed` in the search content-type list `@0x6a303a0`. BASE `ContentType` already has `Memory` but not `Parsed`. |
| 025 | confirmed | DDL verified for all three versions. BASE has none of `calendar_event_id`/`stream_id`/`session_speaker_id`. BASE `find_recent_meeting_for_app` excludes only `explicit_stop` (db.rs ~L10074); target also excludes `room_changed` (`@0x6aad945`). Overlap db-011/012/014, audio-001. |
| 026 | confirmed | Routes `@0x5257fb0`; structs `@0x620a730`/`@0x620a790`; target pipe text posts to `/meetings/<id>/summary` (`@0xa06c0a3`), whereas BASE `meeting-summary/pipe.md` does a PATCH read-modify-write (L33-38). Overlap api-014. |
| 027 | confirmed | `summary_not_saved` `@0x620aeff`, `nothing_to_summarize` `@0x6ab33e1`, `interrupted_by_restart` `@0x6ab3925`, `summary-recovery` `@0x6ab3645`, `meeting_summary_refresh_requested` `@0x6208170`, trivial-meeting log `@0x62073eb`; module paths `meeting_summary\{extract,finalizer,notes}.rs`. BASE: 0. Overlap audio-025. |
| 028 | confirmed | DDL `@0xa2aa964`..`@0xa2ab414`; claim SQL `@0x620a36a`. BASE: 0. Overlap db-009. |
| 029 | confirmed | Engine route `@0x5257f83` and app call `@0x514c6c4`; offer/response events `@0x498f8f5`/`@0x620add8`; `audio_process_room_change` `@0x6ab4784`. BASE: 0. |
| 030 | confirmed | The detector itself is new (`IAudioSessionManager2` `@0x6af72ba`, `meeting_watcher\audio_process\*.rs`, `audio_process_reopen` `@0x6ab4db7`). BASE already has the lifecycle primitives the item lists for its UI-scan detector: 120 s rejoin via `find_recent_meeting_for_app`, `close_orphaned_meetings`, `auto_end`/`shutdown` end reasons, the `detector_stop_tracking` and `meeting_auto_end_requested` events, and `ignoredMeetingApps`. Reuse them. Overlap audio-015. |
| 031 | partially_in_base | BASE `meeting_detector.rs` already preserves active meetings across capture restart (L2608) and keeps Ending->Active while output audio is active (L1906, L3229). That is the very log text cited at `@0x6a77c78`. It also runs pid-scoped Windows UIA scans inside `catch_unwind` (L767-775; `@0x6a95948` is a BASE string). BASE calendar enrichment is time-overlap. New: `exact_conference_url`/`time_only` with the skip on a missing exact conference match (`@0x6ab50fd`..`@0x6ab51ab`), conditional `calendar_event_id` binding (`@0x6ab527b`), `excludeFromMeetingDetection`, and `meeting_ended` payload `duration_secs`/`transcript_segments` (`@0x620743e`). The payload change overlaps 027. |
| 032 | partially_in_base | BASE `routes/meetings.rs` already adopts an active auto-detected meeting (L581-590), maps `idx_meetings_single_open` to 409 (L613-624), emits `manual_start_possible_missed_detection` (L658), and has the stop, merge (>=2 ids) and split rules (L389, L697, L764). New: `StartMeetingRequest.calendar_event_id` (`@0x620a841`), transcript `audioStartTimeSecs`/`sessionSpeakerId` (`@0x51a817f`; the cited `@0x6a303a0` is the search content-type list, a wrong offset), `/export` `include_audio` (`@0x620a470`), and list `q` over `meeting_app` (BASE: title/attendees/note). |
| 033 | partially_in_base | BASE `app/shortcut-reminder/page.tsx` already subscribes to `/ws/meeting-status` and has a stop/start control posting `/meetings/stop {id: stoppableMeetingId}`; `stoppableMeetingId`/`activeMeetingId` are BASE fields. New: `/ws/meeting-overlay` (`@0x5259250`) with status/snapshot/item upserts (last 50), hover transcript and echo de-duplication (FE @12302, @4655). Overlap api-015. |
| 034 | confirmed | `meetingReminders` `@0x52b1304`; settings chunk @578429, with the fallback to `meetingLiveNotes` when unset at @593765; Rust gate log for a prewarm notification skipped by preference `@0x498f9f8` and the room-change prompt gate `@0x498f928`. Partly restated by 040. Misses the lead-time key (see missed item 046). |
| 035 | confirmed | `obsidian_memories.rs` `@0x9f7dc79`; fields `@0x9f7ed88`; write probe `@0x9f7dc3e`; static MCP guidance `@0xa0bae71` versus BASE `external_sync.rs` `render_digest` sidecar. Overlap connections-obsidian-memories, connections-agent-memory-guidance. |
| 037 | confirmed | `NotifyPayload` fields `@0x514b9ec`; gates and dedupe `@0x514b203`..`@0x514b501`; `x-screenpipe-session` `@0x514c09e`; UI action types `chat`/`link`/`copy`/`source`/`mute`/`manage` in the panel chunk. BASE already has action types `pipe`/`api`/`deeplink`/`meeting_join`/`dismiss`, `open_in_chat`, the `pipeNotifications` gate and a mute-pipe control. The Rust `NotificationPriority` literal run reads high/low/medium (`@0x6201f5d`) while the UI uses high/normal/low: accept both. Overlap api-037, frontend-031. |
| 038 | confirmed | `/notifications/:id/read` `@0x59f6268`; mark-read log `@0x514a9f6`; entry fields `@0x6201fd0`. BASE entry lacks source ids/url/actions/priority, and BASE has no per-item read route. |
| 039 | partially_in_base | BASE `components/notification-bell.tsx` already polls `/notifications` every 5 s and has an unread badge, mark-all-read, clear-all and per-item dismiss. New: `notification-inbox` window/page with `show_notification_inbox` (`@0x49936aa`), Priority/All tabs, high-priority heuristic (`7737-...js` @59887), per-item read, copy, open source, actions, rating and settings link. Overlap frontend-030. |
| 040 | confirmed | `notificationsEnabled`/`snoozeUntil`/`quietHours`/`allowDuringPause` `@0x5287149`; `powerModeChanges` `@0x516ed24`; `pipeAllowanceWarnings` is FE-only (settings @580216). `mutedPipes` is BASE. The list omits `meetingReminderLeadSeconds`. Overlap frontend-040. |
| 041 | confirmed | `first_run_summary.rs` `@0x498e97e`; request params `@0x498e9e8`; deeplink `@0x498eb2d`. The module also reports first-run telemetry (`@0x498e9e8`); drop it. The db-recovery and disk-pressure producers are new app modules owned by shell. Overlap frontend-018. |
| 042 | confirmed | Strings verified; Tauri `is_history_access_restricted` `@0x51780c1`; engine `history_access.rs` `@0x6a64d61`. BASE: 0. Overlap api-035. |
| 043 | already_in_base | No Cargo.toml depends on `screenpipe-team-memory` and no `.rs` imports it, so it is not linked into a BASE build either. Zero target hits (including the crate's own error strings) cannot separate "removed" from "unlinked". Observable behaviour already equals BASE; no work. |
| 044 | confirmed | BASE `routes/vault.rs` L45-59 locks online. Target refusal `@0x522d6fa`, hybrid refusal `@0x522ae21`, `secrets.sqlite` `@0x522af47`. Overlap api-030, db-026, connections-vault-secrets-db. |
| 045 | partially_in_base | BASE `DEFAULT_ALLOWED_ENDPOINTS` has the 13 entries, and the BASE `writer` preset already has the same 7 meeting/memory rules (`permissions.rs` L483-493). New: 4 reader additions (`@0xa0c8088`) and the structured-output set (`@0xa0c8030`). The "meeting-summary needs `POST /meetings/*/summary`" grant is unsupported: 0 hits for that rule, and the target meeting-summary frontmatter (`@0xa06823e`) declares no permissions block. |

Totals: 30 confirmed, 14 partially_in_base, 1 already_in_base, 0 unsupported, 0 duplicate (031's
`meeting_ended` payload change partly duplicates 027).

### Missed items

- **meetings_memory-046 [changed] App-side meeting-reminder scheduler (configurable lead, persisted receipts).**
  BASE fires `meeting_about_to_start` from `crates/screenpipe-events/src/custom_events/meetings.rs` with a fixed
  3-minute `PREWARM_LEAD` and an in-memory 1 h dedupe map, so a restart can repeat a reminder. Target moves this
  into a new app module `src\calendar\reminders.rs` (scheduler-start log `@0x4999270`). The
  scheduler keeps a durable receipt ledger: steps `ledger_read`, `receipt_write`, `receipt_rollback`,
  `snapshot_write`, `event_publish` (`@0x49992c8`, `@0x4999310`); receipts carry `expires_at`, `provider_key`
  and `room_key`, with outcomes `Emitted`/`Removed`/`AlreadyJoined` (`@0x52b14c0`). It withholds a reminder
  (`reminder_withheld`, `@0x52b1478`) when the meeting is already active (`active_meeting_read`, `@0x4999378`),
  and publishes `MeetingPrewarmEvent` with `secondsUntilStart` (`@0x52b1550`). The new setting
  `notificationPrefs.meetingReminderLeadSeconds` (`@0x52b1314`) is chosen in Settings from 15/30/60/120/300 s,
  default 30 (`settings/page-d9c69007346aaf6a.js` @~597545). frontend-040 only infers this key, and no spec covers
  the scheduler. Size M. Touchpoints: new `src-tauri/src/calendar/reminders.rs`, `meeting_live_notes.rs`,
  `screenpipe-events` (retire the in-crate prewarm), `use-settings.tsx`, `notifications-settings.tsx`.

Sweep coverage with no further misses: the engine route table (`@0x5257ca8`..`@0x5259328`) versus BASE
`server.rs` (the dimension's only new routes are the ones already itemised); app sidecar routes
(`@0x59f61b0`..`@0x59f6370`); Tauri command diff (brain-view commands belong to frontend-021/022);
target source modules for meetings, memories, notifications, activity, semantic and outputs; memories API
structs (unchanged, which supports 036); announcements via `/notify` `surface` (frontend); and the
`speaker-reconciliation` built-in pipe on `meeting_ended` (agents_pipes/frontend).

---

## Verification (second pass, 2026-10-15)

This is an independent adversarial re-check of items 001-045 (036 is not a delta item). It reuses none of
the first pass's conclusions. The BASE tree is clean (`bcef0bc` import plus `0e2397a` rules; only `bun.lock` is
modified). Method, all static:
- **BASE grep.** `tools\mmv2_base.py` greps every claimed identifier over `apps/`, `crates/` and `packages/`, including
  `.md`/`.toml`/`.sql`. Slash-leading needles were rerun with `MSYS_NO_PATHCONV=1`, because Git Bash had
  rewritten them. Every base match was then read.
- **Offsets.** `tools\mmv2_offs.py near` checked all 157 cited offsets: 146 hit within -0x300..+0x900. The 11 misses
  are needle wording; the text at the offset is relevant in every case except the one wrong offset under 032.
- **DDL.** `tools\mmv2_ddl.py` compared all 317 SQL lines in this spec verbatim against the exe strings: 314 match.
  The other 3 are the 20260817160000 reset statements, which the spec joins onto one line each; they are
  semantically identical.
- **Migration table.** I re-decoded it (`mm_migtable.py`, exe `0xa2ae3f0`, stride 0x58, 117 records). The mapping
  in section 1 is right. The decoder's one-row description shift is anchored by BASE
  `20260528090000_add_end_reason_to_meetings.sql`.
- **Route tables.** I dumped the full engine route table (`tools\out\mmv2_routes_full.txt`) and the app sidecar
  route list (`@0x59f61b0`..`@0x59f6370`).
- **Tauri commands.** `tools\mmv2_cmds.py` diffs the commands the frontend invokes (calls through the minified invoke wrapper in
  `8001-00ed120106af2449.js`, 310 in total) against BASE `lib/utils/tauri.ts` (206).
- **Source modules.** `tools\mmv2_srcdiff.py` diffs target source-module paths against BASE files.
- **Frontend.** I read context windows in the extracted chunks.
- **REA.** `inspect_analysis_view` on parent `ev_57f23857…` (notification-inbox module) reproduces the cited view ID
  `ev_27381aee90c77a8867686aa3a01d65a60222f9492c1809d4eb3f3d4b79d1e002`. A `trace_application_feature`
  string seed `/starred-sessions` completed and was retained as
  `ev_9ece2b19d6f6b4b5c9a6d1bdcc3afe2fdb59573a46b36330f5a5da3c4607ecd4`, but it is too large to return over MCP.
  `ev_b5582741…` resolves as a retained view record.

### Verdicts (second pass)

| Item | Verdict | Notes |
|---|---|---|
| 001 | partially_in_base | All 20 cited versions are in the decoded table. BASE already has `set_ignore_missing(true)` and checksum self-heal (`db.rs` L401-518). sqlx 0.7.2 (`Cargo.lock`) applies any unapplied version regardless of order, so `20260601000000` needs no new mechanism, only a regression test. New: adopting the exact version numbers. |
| 002 | confirmed | DDL is verbatim at `@0xa2a1af2`; SAF columns at `@0xa2a2d2a`; partial index at `@0xa2aad8d`; insert at `@0x51a63ec`. The facet query collapses chat sources and excludes `structured-output` (`@0x5a5fbed`, `@0xa27b160`). BASE: 0 hits. |
| 003 | confirmed | DDL at `@0xa2a6390`/`@0xa2a651e`; delete-by-rowid at `@0x51a696d`; `MATCH` + `bm25` ordering at `@0x5a5f876`; count query at `@0x5a5f8e5`; the backfill log at `@0x6ab7ac1`. BASE: 0. |
| 004 | confirmed | Routes `/artifacts/register`, `/artifacts` and `/artifacts/:id` are in the route table (`@0x5258056`), with doc text at `@0x6a7c9c8`. Errors at `@0x522ded8`..`@0x522e0c3` (the 100 MB size-limit error `@0x522df48`). `pipe-run` + auto-register at `@0x49abbf0`/`@0x49abc10`. Envelope messages at `@0x6a7cc8b`; the 128-char `artifact_id` cap is a literal only in the FE validator (home chunk @100852). BASE `pipe-run` (55 hits) is only an app-side chat label, never an outputs `source_type`. |
| 005 | confirmed | Four routes at `@0x5258618`..`@0x52586c0`; request structs at `@0x522d17f`; `targets.json` at `@0x6a23bec`; resolve/snooze/correct/dismiss/reopen at `@0x6a241e6`; the revision-changed and not-bound errors at `@0x6a2462b`/`@0x6a24755`. The evidence cap is real: `maxItems: 50` in the pi `structured_output` tool schema at `@0xa0adf80` (the first pass called it inferred). BASE: 0. |
| 006 | **partially_in_base** | The spec's claim that "BASE has memories only in Settings" is wrong. BASE `app/home/page.tsx` already has a `memories` main section (L46, L87, L815-816, L868) that renders `MemoriesSection`. That component already does tag filter, sort, create with `importance: 0.5`, a PUT sending only changed content/tags, and batch delete (`components/settings/memories-section.tsx` L161-171, L227-233, L318-424). New: the Brain wrapper, the artifacts tab, the 30 s counters (`/memories?limit=1&offset=0` @297852, `/artifacts?limit=1&offset=0` @297892), mixed `mem:`/`output:` selection (@291042/@290326), SAF kind/version badges (@337642), and deep-link open by artifact id. |
| 007 | confirmed | DDL at `@0xa2aaf2a`; upsert lookup at `@0x516d9c3`; delete at `@0x51a6cc4`; route `/feedback` in the route table at `@0x5258728`; `ai_feedback.rs` validation strings at `@0x51835d0`..`@0x5183719`. BASE engine "feedback" hits are only `capture_detection_feedback` telemetry in `routes/meetings.rs`; BASE app `/feedback` hits are the support dialog. |
| 008 | confirmed | `3822-…js` @8974/@9606 posts `kind:"notification"` / `agent:screenpipe-chat` with `producer_ref`. `7737-…js` @37224 has the positive/negative rating labels and the improvement prompt. `3696-…js` @484622 has the chat good/bad response rating control using module 68186 `submitFeedback`, which saves locally. BASE has thumbs icons only in `speakers-section.tsx`. |
| 009 | partially_in_base | BASE already uploads logs to `screenpi.pe` `/api/logs` and `/api/logs/confirm` with JS `redactPii` (`share-logs-button.tsx` L192/L270, `app/overlay/page.tsx` L282/L298, `onboarding/engine-startup.tsx` L768/L794). New in the target: Rust `feedback_upload.rs`/`feedback_redact.rs` (`@0x498c949`), Tinfoil redaction (`@0x498c7aa`), an mp4/mov video attachment (`@0x5a6315d`), and the Tauri commands `start_feedback_upload`/`redact_pii_for_feedback`. The decloud must cover the BASE paths too. |
| 010 | confirmed | DDL is verbatim at `@0xa2ae26f`; the `has_audio` list SQL is at `@0x51a6b63`; `routes\starred.rs` at `@0x5183a30`; errors at `@0x5183aa8`..`@0x5183bb8`. The same run also has an error requiring unrestricted history access for starred metadata, one more 042 call site. BASE: 0. |
| 011 | confirmed | `/search` params `starred_session_id`/`starred_only` are at `@0x52ca6cb`. The summary field list (`include_starred`, `starred_sessions`, `starred_sessions_has_more`) is at `@0x522ba40`. The doc states a limit of ten (`@0x511e19b`). BASE: 0. |
| 012 | confirmed | Page chunk present; commands at `@0x4993871`/`@0x4994107` and in the `8001-…js` bindings; four events at `@0x524e740`..`@0x524e7b7` and `@0x621fdbd`; `starSessionShortcut` at `@0x52aef6d`; CLI `star list|start|edit` at `@0x6a5ac69`. The page uses `5,15,30,60` (@22251), `9e5` (@24839), `datetime-local`, `include_audio` and `starred_session_id`. BASE: 0. |
| 013 | confirmed | All DDL lines verbatim (see method). BASE: 0. |
| 014 | confirmed | Sampler groups frames into 10 s epoch buckets per (app, window, url, document_path), focused-or-unknown only (`@0x61fabe3`..`@0x61fae14`); ui_event types at `@0x61fbb7a`; audio trimmed length >= 8 at `@0x61fbf55`; attention ranks 3/2 and 4 at `@0x61fb714`/`@0x61fba95`. BASE already has `frames.document_path` (migration `20260504120000`). |
| 015 | confirmed | `/activity-ledger` in the route table; errors at `@0x6208434`/`@0x62084e8`; `ActivityLedgerDepth`/`include_artifacts` at `@0x6a306b5`. The home chunk @751759 builds `depth:"task",include_artifacts:"true",refresh:"false"`. BASE: 0. |
| 016 | confirmed | Commands `get_activity_history`/`generate_activity_history`/`set_activity_sync_enabled` are target-only (command diff). Settings keys at `@0x498d8c8`..`@0x5289296`; events at `@0x498d280`/`@0x498d300`; validation codes at `@0x528902a`..`@0x5289050`; `activity_history\quota.rs` at `@0x621d859`. BASE: 0. |
| 017 | confirmed | Home chunk @751759: 6 retries with `750*a` backoff, plus `no_capture_in_range` @755969 and `empty_but_recording` @756085. BASE Home's `history` section is chat history, not activity. |
| 018 | partially_in_base | BASE `routes/activity_summary.rs` already has `include_recording`/`include_memories`/`include_snippets`/`include_guidance`, `max_snippets`/`max_memories`, and the top-20 apps / top-30 windows limits (L60-82, L389, L409). New: `include_apps`, `include_windows`, `include_key_texts`, `include_parsed_count`, `include_starred`, the ui_event attribution CTE (`@0x6a1a397`), and `AppAttributionStatus` with native/recovered/unresolved counts (`@0x522ba40`, `@0x6a0c643`). |
| 019 | confirmed | DDL verbatim; GC at `@0x5145413`. BASE: 0. |
| 020 | confirmed | DDL verbatim; alias lookup at `@0x6aaf5c8`. BASE: 0. |
| 021 | confirmed | 13 `crates\screenpipe-semantic\src\…` paths; four domain tags at `@0xa2ec148`..`@0xa2ed648`; node-budget error at `@0xa2ec528`; catalog region (`@0xa2f0700`..) names Slack, Teams, Outlook, Notion, Obsidian, ChatGPT, Claude, GitHub, VS Code, Windows Terminal, Warp, Discord, WhatsApp, Todoist and more. BASE: 0. |
| 022 | confirmed | `semantic_worker.rs`; modes `memory|computer_use|both` and the worker-enabled log at `@0x6a96476`; `handled_cached` at `@0x6ab5c1a`; batched `UPDATE frames SET semantic_run_id` at `@0x6aafde2`; the opt-in, off-by-default doc text at `@0x511be8d`; settings keys in the recording blob at `@0x52307bf`. BASE: 0. |
| 023 | confirmed | Six routes at `@0x5257caf`; structs at `@0x522dabe`; SQL at `@0x51a75df`..`@0x51a80b6`. BASE: 0. |
| 024 | confirmed | `ParsedItem`/`ParsedContent`/`ParsedActorReference` at `@0x6a3d1c4`/`@0x6a3d3bc`/`@0x6a305c8`; the doc note excluding it from `all` at `@0x51178f2`. BASE `ContentType` has no `Parsed`. |
| 025 | confirmed | DDL verbatim. The target rejoin lookup excludes both `explicit_stop` and `room_changed` (`@0x6aad938`); BASE `find_recent_meeting_for_app` excludes only `explicit_stop` (`db.rs` L10071-10080). |
| 026 | confirmed | Both routes in the route table (`@0x5257fb0`); `SaveMeetingSummaryRequest` at `@0x620a757`; status fields at `@0x620a790`; SQL at `@0x620acf6`. BASE `meeting-summary/pipe.md` L33 PATCHes the meeting. |
| 027 | confirmed | `meeting_summary\{extract,finalizer,notes}.rs` are target-only; `:summary-recovery-` key fragment at `@0x6ab3645`; trivial-meeting log at `@0x62073eb`; `meeting_summary_refresh_requested` at `@0x6208170`. BASE: 0. |
| 028 | confirmed | DDL verbatim; `INSERT OR IGNORE INTO pipe_event_runs` at `@0xa2b6e17`. BASE: 0. |
| 029 | confirmed | Engine route at `@0x5257f83`, app call at `@0x514c6c4`, `meeting_room_change_offer`/`_response` at `@0x498f8fd`/`@0x620add8`, `MeetingRoomChangeOffer` struct at `@0x5155918`. BASE: 0. |
| 030 | confirmed | The mic-holder detector is new: `IAudioSessionManager2`/`IAudioSessionEnumerator` at `@0x6af7287`/`@0x6af72bb` in `meeting_processes.rs`, plus `meeting_watcher\audio_process\{mod,lifecycle,resolve,events,windows}.rs`. BASE has no audio-session enumeration, and its `screenpipe-audio/src/meeting_detector.rs` is only a flag holder. The lifecycle primitives the item lists (`detector_stop_tracking`, `meeting_auto_end_requested`, `ignoredMeetingApps`, 120 s rejoin, orphan close) already exist in BASE and should be reused. |
| 031 | partially_in_base | BASE `meeting_detector.rs` already has (c) preserve on restart (L2608), (d) Ending -> Active on output audio (L1906, L3283) and (f) pid-scoped UIA inside `catch_unwind` (L767-775). Its calendar enrichment is time-overlap (L3015-3017, L3371). New: (a) `exact_conference_url`/`time_only` and the skip at `@0x6ab50fd`/`@0x6ab518a`; (b) the ownership check at `@0x6ab5211`..`@0x6ab527b`; `excludeFromMeetingDetection` (`@0x52876cc`); (e) duplicates part of 027. |
| 032 | partially_in_base | BASE already adopts an active auto meeting (L561-590), maps `idx_meetings_single_open` to 409 (L607-622), and has `manual_start_possible_missed_detection`. New: `StartMeetingRequest.calendar_event_id` (5 elements vs BASE 4, `@0x5155828`), `audioStartTimeSecs`/`sessionSpeakerId` (`@0x51a817f`; the cited `@0x6a303a0` is wrong, it is the search content-type list), `/export` `include_audio` (`@0x620a470`), and `meeting_app` in `q` (`@0x61fd80f` vs BASE `db.rs` L9190-9192). The item also misses `UpdateMeetingRequest.calendar_event_id` (missed item 047). |
| 033 | partially_in_base | BASE `app/shortcut-reminder/page.tsx` already uses `/ws/meeting-status` (L46) and the stop control posting `stoppableMeetingId` (L87-90). New: `/ws/meeting-overlay` (route table `@0x5259250`; client @12306 with status messages and reconnect backoff), the hover transcript and echo dedupe. |
| 034 | confirmed | `meetingReminders` is new (settings chunk @578429; BASE has only `meetingLiveNotes`); prewarm preference gate at `@0x498fa00`. The key is restated in 040, but this is not a full duplicate. Lead time is missed item 046. |
| 035 | confirmed | `obsidian_memories.rs` at `@0x9f7dc79`; `obsidian-memories`/"Obsidian Memories" at `@0x9f7edcd`/`@0x9f7edde`; write probe at `@0x9f7dc3e`; static MCP guidance at `@0xa0bae71`. BASE `external_sync.rs` renders a digest (L116-140); BASE has an `obsidian` connection with `vault_path` but no memories target. |
| 037 | confirmed | BASE `NotifyPayload` is title/body/id/type/autoDismissMs/timeout/actions, with only the `pipeNotifications` gate (`notifications/routes.rs` L17-179). BASE panel action types: pipe/api/deeplink/meeting_join/dismiss. Target gates and dedupe are at `@0x514b203`..`@0x514b501`, payload fields at `@0x514b9ec`, and `x-screenpipe-session` on `/notify` at `@0x514c09e`. BASE uses that header only for pi/agents. Wire note: the Rust literal run is `NotificationPriorityhighlowmedium` (`@0x6201f5d`) while the FE accepts `high|normal|low` (`7737-…js` @59887), so accept both `normal` and `medium`. |
| 038 | confirmed | BASE entry lacks source ids, url, actions and priority (`store.rs` L13-22) and has no per-item read route. Target route at `@0x59f6268`; the mark-read log at `@0x514aa19`. |
| 039 | partially_in_base | BASE `notification-bell.tsx` already polls every 5 s (L117) with unread badge, mark-all, clear and per-item dismiss (L105-147). New: the `notification-inbox` window (`show_notification_inbox`, target-only), priority/all tabs, the high-priority heuristic (@59887), per-item read, copy, source, actions, rating and the settings link (inbox chunk @20017). |
| 040 | confirmed | BASE `notificationPrefs` has captureStalls, appUpdates, pipeNotifications, displayChanges, meetingLiveNotes, audioCaptureStalled, liveTranscriptStalled and mutedPipes (`use-settings.tsx` L305-318). The new keys are in the settings chunk (@578429..@581325); `22:00` default at @580780. |
| 041 | confirmed | `first_run_summary.rs`; request params `include_windows`/`include_parsed_count`/`max_snippets` 6/`max_snippet_chars` 240 at `@0x498e9e8`; deeplink at `@0x498eb2d`; also first-run telemetry to drop. `db_recovery_notifications.rs`/`disk_pressure_notifications.rs` are new app modules. The workflow notice type is literally `workflow-review` (`@0x514b52a`), not `workflow_review`. |
| 042 | confirmed | the 24-hour-history error text + `history_access_limited` at `@0x5152f8e`/`@0x6a5dd5c`; `history_access.rs` at `@0x6a64d61`; `is_history_access_restricted` is target-only. BASE: 0. |
| 043 | already_in_base | The crate is a workspace member only via `crates/*`; nothing depends on it, so a BASE app build does not link it either. Zero target hits cannot distinguish "removed" from "unlinked". No delta, no work. |
| 044 | confirmed | BASE `routes/vault.rs` L43-59 locks online; BASE vault manager covers only `db.sqlite`. Target refusal at `@0x522d6fa`, hybrid refusal at `@0x522ae59`, `secrets.sqlite(-journal)` at `@0x522af61`. Ownership belongs with api/db rather than this dimension. |
| 045 | partially_in_base | BASE `DEFAULT_ALLOWED_ENDPOINTS` (13 entries, `permissions.rs` L125-139) and the writer preset's meeting/memory rules (L486-492) match the target writer list at `@0xa0c7f30`. New: 4 reader additions (`@0xa0c8088`) and the structured-output set (`@0xa0c8030`). Unsupported sub-claim: there are 0 hits for `POST /meetings/*/summary` as a rule, and the target `meeting-summary` frontmatter (`@0xa06823e`..`@0xa0682fb`) declares no permissions. |

Totals (second pass): 34 confirmed, 9 partially_in_base, 1 already_in_base, 0 unsupported, 0 duplicate.
- Differences from the first pass: 006 drops to partially_in_base, since BASE Home already has the memories section.
  005's evidence cap of 50 is now observed.
- Partial overlaps that stop short of duplicates: 031(e) with 027, 034 with 040 (`meetingReminders`), and 011 with 018
  (`include_starred`).

### Missed items (second pass)

- **meetings_memory-046 [changed] App-side meeting-reminder scheduler.** Independently re-verified; still absent from
  the inventory list above.
  - BASE: `crates/screenpipe-events/src/custom_events/meetings.rs` (L53-74) fires `meeting_about_to_start` with a fixed
    `PREWARM_LEAD`. BASE has no `secondsUntilStart`.
  - Target: a new app module `src\calendar\reminders.rs` (`@0x49991f0`, scheduler-start log at
    `@0x4999270`).
  - It keeps a durable receipt ledger with steps `ledger_read`, `receipt_write`, `receipt_rollback`, `snapshot_write`
    and `event_publish` (`@0x49992c8`, `@0x4999310`).
  - It withholds a reminder when the meeting is already active (`active_meeting_read` `@0x4999378`,
    `reminder_withheld` `@0x52b1499`); the outcomes include `AlreadyJoined` (`@0x52b1512`).
  - It publishes `MeetingPrewarmEvent` with `secondsUntilStart` (`@0x52b1550`).
  - The lead time is the new setting `notificationPrefs.meetingReminderLeadSeconds` (`@0x52b1314`; settings chunk
    @597941).
  - Size M.
- **meetings_memory-047 [changed] `PUT /meetings/:id` accepts `calendar_event_id`.**
  - Evidence: target `struct UpdateMeetingRequest with 7 elements` (`@0x5155a2b`) versus BASE's 6 fields
    (`routes/meetings.rs` L24-31).
  - The home chunk (@589293) enriches untitled auto-detected meetings from the local calendar and PUTs
    `{title, attendees, calendar_event_id}`.
  - Implement it through the same conditional binding as 025/031, where a UNIQUE conflict means "already bound
    elsewhere": skip the bind, keep the title. Otherwise the new partial unique index
    `idx_meetings_calendar_event_id` makes this PUT fail.
  - Size S.

Sweep coverage with no further misses:
- Target-only source modules for the dimension. `db\{meetings,memories}.rs` are refactors of `db.rs`; `live_views`,
  `brain_views`, `disk_pressure`, `recovery`, `sqlite-recovery`, `meeting_piggyback` and `windows_output_follow`
  belong to other dimensions.
- The full engine route table. `/pipes/activity`, `/pipes/:id/history` and `/pipes/favorites` are pipes routes.
- App sidecar routes: only `/notifications/:id/read` is new; BASE already has `/inbox`.
- Tauri command diff: the brain-view commands belong to live-views/frontend.
- Memory request structs (`CreateMemoryRequest` 6 / `UpdateMemoryRequest` 4 elements, equal to BASE), which supports 036.
- Built-in pipe names (new recap and meeting-prep/follow-up pipes are agents_pipes).
- FE object keys for meeting, memory, notification, activity, semantic, star, summary, reminder and calendar.
