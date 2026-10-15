# Spec: HTTP / WebSocket API surface (dimension `api`)

Target: screenpipe v2.7.104 (Windows, Tauri 2.11, engine embedded in `screenpipe-app.exe`).
Base: Freenpipe @ upstream `892199f` (`crates/screenpipe-engine`, `apps/screenpipe-app-tauri/src-tauri/src/server.rs`).

Clean-room note: everything below is paraphrased from static evidence (binary strings, panic `Location` records,
serde/oasgen schema names, embedded agent-extension sources and the extracted Next.js frontend). Only
interoperability identifiers (paths, JSON keys, header names, enum values, file names) are quoted verbatim.
The embedded `CHANGELOG.md` in `frontend/` was deliberately NOT read (post-relicense changelog).

---

## 0. How the inventory was recovered (reproducible)

| Source | What it gives | Location |
|---|---|---|
| oasgen route-literal blob | every OpenAPI-registered path, in registration order, concatenated | `app_strings.tsv` @ `0x5257ca8` (one 2.1 KB line starting `/search/semantic/actors/search…`, ending `/data/compact`) |
| axum `.route()` / `.nest()` literals | non-OpenAPI routes (WebSockets, SSE, proxies, nested routers) | `0x52584e0` … `0x5259328` |
| `core::panic::Location` records (file ptr `0x145258608`, len 83 = target `server.rs`) | one record per `#[track_caller]` `.route/.nest/.merge` and per chained method (`.get().post()` adds a second record, col 22/26). Lines 1107–1533 of target `server.rs`. Records with no adjacent literal = a path literal deduplicated with an identical string elsewhere (`/`, second `/:id`, etc.) | `tools/loc_records.py`, `tools/route_locs.py` |
| oasgen operation names (`std::any::type_name` of each handler) | maps each oasgen path to its handler, proves hidden oasgen routes such as `/starred-sessions` (`routes::starred::{list,save}`) | e.g. `0x6a1a8b0`, `0x6a3b3d2`, `0x52cd6d0` (`screenpipe_engine::routes::…`) |
| oasgen schema names + doc comments | request/response type names, new fields, paraphrasable handler docs | `0x6a0bba0`–`0x6a0c890`, `0x6a1dde0`–`0x6a1e372`, `0x6a3d150`, `0x6a3fdb0`, `0x6a431c8`, `0x6a63968`, `0x6a72278`, `0x6a726a0` |
| serde `struct X with N elements` markers | field lists (partially deduplicated) of request structs | `0x52ca6b8` (SearchQuery) and neighbours, `0x5183483` (feedback), `0x522b978` (cloud agents), `0x620a458`… (meetings) |
| embedded `screenpipe-api` agent skill (markdown) | documented params/behaviour of the public REST surface (paraphrased here) | `0x5113095`–`0x511e39f` (copies at `0x6a4e…`, `0xa090…`) |
| embedded Pi agent extensions (TS sources) | exact client call shapes for `/agent/*`, `/outputs/targets*`, `/live-views*`, `/v1/connect-request`, `/v1/web-search` | `0x52919e4`, `0x52aa320`, `0xa0a782e`, `0xa0ae18d`, `0xa0d2a2f` |
| Frontend chunks | real call sites, methods, query params | `frontend/_next/static/chunks/**` (paths cited per item) |
| REA | parent JS application analysis `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`. `trace_application_feature` seeds `string:"localhost:3030"` → `ev_f86a8c54535aff7221d66a69c814c41ff2d988b9958479ffb01eeb8693595408` and `api:"fetch"` → `ev_6d93613d94a37e8cdcaa04bc122d091dd27a769f9484ce5162ac2b62e6660f1a` both returned `no-match` (graph does not index fetch literals), so the frontend inventory was done by direct literal scan (`tools/fe_routes.py`, `tools/api_fe_ctx.py`). |

Method attribution rule: oasgen literals do not carry the verb. Verbs below come from (a) the base for unchanged paths,
(b) frontend `fetch` call sites, (c) the embedded skill/extension sources, (d) the chained-method Location records
(two records ⇒ two verbs). Anything not covered by (a)–(d) is marked *inferred*.

---

## 1. Cross-cutting behaviour (applies to many routes)

### 1.1 Server, ports, CORS, limits — unchanged
- Engine HTTP+WS on `:3030` (axum 0.7.9; oasgen 0.22.0). App-side notification/utility server on `:11435`.
- CORS allow-list (localhost / 127.0.0.1 / tauri origins), `TraceLayer`, `/v1/audio/transcriptions` 250 MB body cap,
  `/v1/chat/completions` 50 MB cap: identical strings and structure to base.
- Auth middleware error body is byte-identical to base (`0x6189c50`): 403 + JSON `{"error":"unauthorized: …"}` (exact string prefix, client-visible),
  accepted credentials in order: `Authorization: Bearer`, cookie `screenpipe_auth`, `?token=`.

### 1.2 Auth exemption list — CHANGED (`api-011`)
Target exempt paths (skill doc `0x5115cfb`, plus 16-byte compare fragments at `0x6a91c00`):
`/health`, `/ws/health`, `/audio/device/status`, `/connections/oauth/callback`, `/mcp-servers/:id/oauth/callback`,
`/connections/browser/pair/start`, `/connections/browser/pair/status`, `/notify`, `/pipes/store*`.
**`/frames/*` is no longer exempt**: per the skill doc, every `/frames/*` read (image, thumbnail, text, context,
metadata) now has to present the bearer token, exactly like the other recording reads. The desktop frontend already sets the `screenpipe_auth` cookie
(`chunks/3258-*.js`) so `<img src>` thumbnails keep working.

### 1.3 Attribution headers — NEW (`api-012`)
Clients may send `X-Screenpipe-Client` (fixed value `api` for REST, MCP clients send their client id) and
`X-Screenpipe-Agent` (fixed installer-chosen agent id, else `unknown`). Parsed by `routes/request_origin.rs`
(`0x5a5eb30`: `x-screenpipe-client`, `x-screenpipe-agent`). Target uses them only to emit a PostHog
`qualified_value_event` when an external retrieval returns non-empty data. Also seen: `x-screenpipe-session`
(chat/pipe session id, used by the MCP-server access gate), `x-screenpipe-time-range` (structured-output schema hint).
Freenpipe: accept and ignore (debug log only). Never forward anywhere.

### 1.4 Flexible time parsing — CHANGED (`api-001`)
Every `deserialize_flexible_datetime*` param (`start_time`, `end_time`, `start`, `end`, export `start/end`, retranscribe range…)
now also accepts **machine-local calendar boundaries**: `today` (local midnight), `yesterday` (previous local midnight),
and bare `YYYY-MM-DD` (that local day's midnight). Error text updated to list the three forms (`0x6a08d13`).
Ordering of attempts: RFC3339 → calendar literal → relative (`16h ago`, `now`, `2d`, …).

### 1.5 Response format negotiation — NEW (`api-002`), `routes/response_format.rs` (`0x6a991ef`)
- `format=json` (default) | `csv` | `tsv` (alias `table`). Unknown → 400 ``unknown format `X` (expected json|csv|tsv)`` (exact string, client-visible) (`0x6a954b9`).
- `fields=a,b.c` — allow-list of dotted paths applied to each row before serialisation; JSON rows become **flat objects keyed by the dotted path** (`row["content.timestamp"]`).
- CSV: RFC-4180, lossless (quotes newlines). TSV: newlines/tabs collapsed to spaces. Content types
  `text/csv; charset=utf-8` and `text/tab-separated-values; charset=utf-8`. Header row written once.
- Applies to `/search` and `/elements` (and `/frames/:id/elements`). JSON keeps `pagination`; CSV/TSV drop it
  (caller cannot tell if more rows exist).

### 1.6 Pipe permission middleware — CHANGED (`api-038`)
Base already enforces endpoint allow-lists, content-type allow-lists and a 3-permit query semaphore with 30 s timeout.
Target adds (strings `0x6188c65`–`0x6188e97`, `0x6a3ba0a`–`0x6a3bb1a`):
- A second concurrency lane named "workflow operation" for `/workflows/workspace` and `/workflows/catalog`, with its own timeout message (`<op> timed out after N`) and generic `pipe request timed out` (both exact strings, client-visible).
- On `/search`: when a pipe has content-type restrictions, `content_type` must be explicit (400); app/window restrictions return 403 with an access-denied message that names the app (or window) the Pipe is not allowed to read; `include_related` is refused when data permissions are active.
- Structured outputs/feedback: a pipe cannot rate or act on its own outputs (403).
- Default permission template seen for scheduled pipes (`0xa0c8088`): `GET /search, /starred-sessions, /activity-summary, /workflows, /workflows/*, /elements, /frames/*, /meetings, /meetings/*, /meetings/status, /speakers, /pipes/info, /health, /connections/*, /feedback` and `POST /notify, /speakers/update`.

### 1.7 Search-only mode — NEW (`api-034`)
When the user quits the desktop app while the option to leave search running after quit is on (`keepSearchAvailableAfterQuit` setting), capture stops but the engine API keeps serving history (`0x49878…`: `search_only_ready outcome=capture_stopped; api=retained; ui=closed`). Features that need the live recorder answer with a message explaining that the app has been quit and is only serving previously saved history, and that reopening the app re-enables the feature (`0x6187fe0`, `routes`/`search_only.rs`). See §api-034 for the proposed gate.

### 1.8 History-access gating — DECLOUD (`api-035`)
Target can restrict reads to the last 24 h for non-entitled accounts: error code `history_access_limited`, with messages stating that the requested content, meeting or export lies beyond the 24-hour window the account may see (`0x6a1cbd8`, `0x6a5dd2e`, `0x620a240`, `0x59fbb50`), plus a refusal to return starred-session metadata without full history access, and `workflow_catalog_requires_unfiltered_history_permission`. This is a paywall. Freenpipe never limits history.

---

## 2. Route inventory: base vs target

Legend — **B** base-present unchanged, **C** changed, **N** new, **R** removed in target, **D** decloud needed, **X** not served by engine.
`oa` = registered through oasgen (`Server::axum()`), `ax` = plain axum route.

### 2.1 Engine `:3030` — oasgen routes (target order)

| # | Method | Path | St | Handler (target) | Delta |
|---|---|---|---|---|---|
| 1 | GET | `/search` | C | `routes::search::search` | api-003 |
| 2 | GET | `/semantic/actors/search` | N | `semantic::search_semantic_actors` | api-008 |
| 3 | POST | `/semantic/actors/create` | N | `semantic::create_semantic_actor` | api-008 |
| 4 | POST | `/semantic/actors/update` | N | `semantic::update_semantic_actor` | api-008 |
| 5 | POST | `/semantic/actors/merge` | N | `semantic::merge_semantic_actors` | api-008 |
| 6 | POST | `/semantic/actors/reassign` | N | `semantic::reassign_semantic_actor` | api-008 |
| 7 | POST | `/semantic/actors/aliases/reassign` | N | `semantic::reassign_semantic_actor_alias` | api-008 |
| 8 | GET | `/audio/list` | B | `audio::api_list_audio_devices` | – |
| 9 | GET | `/vision/list` | B | `health::api_list_monitors` | – |
| 10 | GET | `/tags/autocomplete` | N | `content::autocomplete_tags` | api-009 |
| 11 | POST | `/tags/vision/batch` | B | `content::get_tags_batch` | – |
| 12 | POST, DELETE | `/tags/:content_type/:id` | B | `add_tags` / `remove_tags` | – |
| 13 | GET | `/frames/preview-samples` | N | `frames::get_frame_preview_samples` | api-010 |
| 14 | GET | `/frames/preview-media/:video_chunk_id` | N | `frames::get_frame_preview_media` | api-010 |
| 15 | GET | `/frames/:frame_id` | C (auth) | `frames::get_frame_data` | api-011 |
| 16 | GET | `/frames/:frame_id/thumbnail` | N | `frames::get_frame_thumbnail` | api-010 |
| 17 | GET, POST | `/frames/:frame_id/text` (+ deprecated `/ocr` alias) | C (auth) | `get_frame_text_data` / `run_frame_ocr` | api-011 |
| 18 | GET | `/frames/:frame_id/context` | C (auth) | `get_frame_context` | api-011 |
| 19 | GET | `/frames/:frame_id/metadata` | C (auth) | `get_frame_metadata` | api-011 |
| 20 | GET | `/frames/next-valid` | B | `get_next_valid_frame` | – |
| 21 | GET | `/health` | C | `health::health_check` | api-033 |
| 22 | POST | `/raw_sql` | B | `content::execute_raw_sql` | – |
| 23 | POST | `/add` | B | `content::add_to_database` | – |
| 24 | GET | `/speakers/unnamed` | B | | – |
| 25 | POST | `/speakers/update` | B | | – |
| 26 | GET | `/speakers/search` | C | `search_speakers_handler` | api-013 |
| 27 | POST | `/speakers/delete` | B | | – |
| 28 | POST | `/speakers/hallucination` | B | | – |
| 29 | POST | `/speakers/merge` | B | | – |
| 30 | GET | `/speakers/similar` | B | | – |
| 31 | POST | `/speakers/reassign` | C | `reassign_speaker_handler` | api-013 |
| 32 | POST | `/speakers/undo-reassign` | B | | – |
| 33 | GET | `/meetings` | B | `list_meetings_handler` | – |
| 34 | GET | `/meetings/status` | B | | – |
| 35 | POST | `/meetings/merge` | B | | – |
| 36 | POST | `/export` | C | `meetings::export_handler` | api-014 |
| 37 | POST | `/meetings/bulk-delete` | B | | – |
| 38 | POST | `/meetings/start` | C | `start_meeting_handler` | api-014 |
| 39 | POST | `/meetings/stop` | B | | – |
| 40 | POST | `/meetings/room-change` | N | `resolve_room_change_handler` | api-014 |
| 41 | GET | `/meetings/:id/transcript` | B | | – |
| 42 | GET | `/meetings/:id/summary-status` | N | `get_meeting_summary_status_handler` | api-014 |
| 43 | POST | `/meetings/:id/summary` | N | `save_meeting_summary_handler` | api-014 |
| 44 | GET, DELETE, PUT | `/meetings/:id` | B | | – |
| 45 | POST | `/meetings/:id/retranscribe` | C | `retranscribe::retranscribe_meeting_handler` | api-039 |
| 46 | POST | `/meetings/:id/split` | B | | – |
| 47 | POST, GET | `/memories` | B | | – |
| 48 | GET | `/memories/tags` | B | | – |
| 49 | POST | `/memories/sync-external` | B | | – |
| 50 | GET, PUT, DELETE | `/memories/:id` | B | | – |
| 51 | POST | `/artifacts/register` | N | `artifacts::register_artifact_handler` | api-017 |
| 52 | GET | `/artifacts` | N | `artifacts::list_artifacts_handler` | api-017 |
| 53 | DELETE | `/artifacts/:id` | N | `artifacts::delete_artifact_handler` | api-017 |
| 54 | POST | `/experimental/frames/merge` | B | | – |
| 55 | GET | `/experimental/validate/media` | B | | – |
| 56 | POST | `/audio/start` | B | | – |
| 57 | POST | `/audio/stop` | B | | – |
| 58 | GET | `/search/keyword` | B | | – |
| 59 | POST | `/audio/device/start` | B | | – |
| 60 | POST | `/audio/device/stop` | B | | – |
| 61 | GET | `/audio/device/status` | B | | – |
| 62 | POST | `/vision/device/start` | N | `vision::start_vision_device` | api-016 |
| 63 | POST | `/vision/device/stop` | N | `vision::stop_vision_device` | api-016 |
| 64 | GET | `/vision/device/status` | N | `vision::vision_device_status` | api-016 |
| 65 | GET | `/elements` | C | `elements::search_elements` | api-004 |
| 66 | GET | `/frames/:frame_id/elements` | C | `elements::get_frame_elements` | api-004 |
| 67 | POST | `/workflows/rollout` | N | `workflow_catalog::rollout` | api-028 |
| 68 | GET, POST | `/workflows/catalog` | N | `workflow_catalog::{catalog,commit}` | api-028 |
| 69 | GET, POST | `/workflows/workspace` | N | `workflow_workspace::{context,update}` | api-028 |
| 70 | GET, POST | `/workflows/pipeline` | N | `workflow_pipeline::{context,commit}` | api-028 |
| 71 | GET | `/workflows/context` | N | `workflow_catalog::context` | api-028 |
| 72 | POST | `/workflows/edits` | N | `workflow_edits::edit` | api-028 |
| 73 | POST | `/workflows/corrections` | N | `workflow_catalog::correct` | api-028 |
| 74 | GET | `/workflows` | N | `workflows::list_workflows` | api-028 |
| 75 | GET | `/workflows/:id` | N | `workflows::get_workflow` | api-028 |
| 76 | GET | `/activity-summary` | C | `activity_summary::get_activity_summary` | api-005 |
| 77 | GET | `/activity-ledger` | N | `activity_ledger::get_activity_ledger` | api-006 |
| 78 | GET | `/cloud-agents/status` | N | `cloud_agents::cloud_agent_status` | api-027 |
| 79 | POST | `/cloud-agents/:provider/connect` | N | `cloud_agents::connect_cloud_agent` | api-027 |
| 80 | GET | `/cloud-agents/cursor-agents` | N | `cloud_agents::list_cursor_cloud_agents` | api-027 |
| 81 | GET | `/cloud-agents/:provider/codebases` | N | `cloud_agents::list_cloud_agent_codebases` | api-027 |
| 82 | PUT | `/cloud-agents/cursor-key` | N | `cloud_agents::set_cursor_cloud_key` | api-027 |
| 83 | GET | `/vault/status` | B | | – |
| 84 | POST | `/vault/lock` | C (minor) | | api-030 |
| 85 | POST | `/vault/unlock` | B | | – |
| 86 | POST | `/vault/setup` | B | | – |
| 87 | POST | `/sync/init` | B/D | | api-030 |
| 88 | GET | `/sync/status` | B/D | | – |
| 89 | POST | `/sync/trigger` | B/D | | – |
| 90 | POST | `/sync/lock` | B/D | | – |
| 91 | POST | `/sync/reset-account` | N/D | `sync_api::sync_reset_account` | api-030 |
| 92 | POST | `/sync/download` | B/D | | – |
| 93–98 | POST | `/sync/{pipes,connections,memories}/{push,pull}` | B/D | | – |
| 99 | POST | `/archive/init` | B/D | | – |
| 100 | POST | `/archive/configure` | B/D | | – |
| 101 | GET | `/archive/status` | B/D | | – |
| 102 | POST | `/archive/run` | B/D | | – |
| 103 | POST | `/archive/download` | N/D | `archive::archive_download` | api-030 |
| 104 | GET | `/archive/download/status` | N/D | `archive::archive_download_status` | api-030 |
| 105 | POST | `/retention/configure` | B | | – |
| 106 | GET | `/retention/status` | B | | – |
| 107 | POST | `/retention/run` | B | | – |
| 108 | POST | `/data/delete-range` | B | | – |
| 109 | POST | `/data/evict-media` | B | | – |
| 110 | GET | `/data/storage-preview` | B | | – |
| 111 | POST | `/data/delete-device` | B | | – |
| 112 | GET | `/data/device-storage` | B | | – |
| 113 | POST | `/data/checkpoint` | B | | – |
| 114 | GET | `/data/backup` | B | | – |
| 115 | POST | `/data/compact` | N | `data::compact_handler` | api-030 |
| 116 | GET, POST | `/starred-sessions` | N | `starred::{list,save}` (literal deduped with app crate) | api-007 |
| – | GET | `/openapi.yaml`, `/openapi.json` | R | (no literal anywhere in target) | api-036 |

### 2.2 Engine `:3030` — axum routes (target `server.rs` line → path)

| Line | Method | Path | St | Delta |
|---|---|---|---|---|
| 1111 | POST | `/agent/learning/chats` | N | api-021 |
| 1115 | POST | `/agent/skills/manage` | N | api-021 |
| 1119 | POST | `/agent/profile/manage` | N | api-021 |
| 1123 | GET | `/agent/self-improvement/context` | N | api-021 |
| 1134 | POST | `/internal/telemetry/mcp-value` | N/D | api-012 |
| 1138 | POST | `/internal/telemetry/mcp-call` | N/D | api-012 |
| 1142 | GET | `/outputs/targets` | N | api-019 |
| 1146 | POST | `/outputs/targets/:target_id/submit` | N | api-019 |
| 1152 | POST | `/outputs/targets/:target_id/feedback` | N | api-019 |
| 1158 | POST | `/outputs/targets/:target_id/items/:item_id/actions` | N | api-019 |
| 1167/1170 | GET + POST | `/feedback` | N | api-018 |
| 1175 | GET | `/live-views` | N | api-020 |
| 1179 | GET | `/live-views/schema` | N | api-020 |
| 1183 | GET | `/live-views/catalog` | N | api-020 |
| 1187/1190 | PUT + (DELETE, inferred) | `/live-views/:id` | N | api-020 |
| 1192 | GET | `/live-views/:id/template` | N | api-020 |
| 1196 | GET | `/speakers/sample/:audio_chunk_id` | N | api-013 |
| 1201 | GET | `/vision/status` | B | – |
| 1203 | GET | `/vision/metrics` | B | – |
| 1204 | GET | `/audio/metrics` | B | – |
| 1212–1225 | GET/POST | `/capture/hd`, `/capture/hd/{start,stop,extend,settings}` | B | – |
| 1230 | GET | `/audio/reconciliation/backlog` | B | – |
| 1234 | DELETE | `/audio/reconciliation/backlog/:audio_chunk_id` | B | – |
| 1240 | POST | `/audio/retranscribe` | B (doc'd engines) | api-039 |
| 1244 | POST | `/v1/audio/transcriptions` | B | – |
| 1253 | POST | `/v1/chat/completions` | B/D | api-024 |
| 1261 | POST | `/v1/web-search` | N/D | api-025 |
| 1267 | GET | `/data-sync/devices` | N/D | api-026 |
| 1271 | GET | `/data-sync/search` | N/D | api-026 |
| 1277 | POST | `/v1/connect-request` | N | api-022 |
| 1281 | POST | `/v1/connect-response` | N | api-022 |
| 1296–1393 | (nested) | `/pipes/*` (see 2.3) | C | api-029 |
| 1404 | – | `.nest("/pipes", …)` (+ one literal-less record at col 61) | – | – |
| 1406 | GET (SSE) | `/pipes/:id/stream` | N | api-029 |
| 1415 | ? | literal-less registration (unresolved; see open questions) | ? | – |
| 1417 | POST | `/v1/local/chat/completions` | N | api-023 |
| 1454 | – | `.nest("/connections", …)` | C | api-031 |
| 1475 | – | `.nest("/mcp-servers", …)` | C | api-032 |
| 1483/1486/1489 | GET + POST | `/power` (nested `/`) | B | – |
| 1496 | GET (WS) | `/stream/frames` | B | – |
| 1497 | GET (WS) | `/ws/events` | B | – |
| 1498 | GET (WS) | `/ws/health` | B | – |
| 1499 | GET (WS) | `/ws/meeting-overlay` | N | api-015 |
| 1500 | GET (WS) | `/ws/meeting-status` | B | – |
| 1501 | GET (WS) | `/ws/metrics` | C (field) | api-015 |
| 1507/1521/1533 | WS/POST/GET | `/browser/ws`, `/browser/eval`, `/browser/status` (deprecated aliases) | B | – |
| – | – | `/mcp` | X | MCP is served by the separate `screenpipe-mcp` process (`SCREENPIPE_MCP_TRANSPORT=http`, `SCREENPIPE_MCP_HOST=127.0.0.1`, `SCREENPIPE_MCP_PATH=/mcp`, `0xa117260`); an embedded comment explicitly says `:3030/mcp` only 404s (`0xa0cbb2d`). Same as base. |
| – | – | `/v1/models` | X | only appears as a Hermes connection probe (`0x9f845ba`), not an engine route. |
| – | – | `/ai/status`, `/ai/chat/completions` | n/a | Apple-Intelligence feature (macOS-only cfg) — cannot be observed in the Windows build; leave as in base. |

### 2.3 `/pipes` nested router (target lines 1296–1393)

| Method | Path | St | Notes |
|---|---|---|---|
| GET | `/pipes` (nested `/`, literal-less Location record at line 1296) | C | new list params `execution_limit`, `include_execution_counts` (api-029) |
| POST | `/pipes/bundled/:id/install` | N | api-029 |
| POST | `/pipes/install` | B | |
| GET | `/pipes/favorites` | B | |
| POST | `/pipes/schedule/preview` | N | api-029 |
| GET | `/pipes/activity` | N | api-029 |
| GET, DELETE | `/pipes/:id` | B | |
| POST | `/pipes/:id/enable` | B | |
| POST | `/pipes/:id/favorite` | B | |
| POST | `/pipes/:id/run` | C | richer body, returns `execution_id` (api-029) |
| GET | `/pipes/:id/logs` | B | |
| POST | `/pipes/:id/config` | B (+`cloud_agent` key) | |
| POST | `/pipes/:id/stop` | B | |
| GET | `/pipes/:id/executions` | C | `before_id`, `include_output` (api-029) |
| GET | `/pipes/:id/executions/:exec_id` | N | api-029 |
| DELETE | `/pipes/:id/history` | B | |
| GET | `/pipes/:id/session/:exec_id` | B | |
| GET | `/pipes/store` | B/D | public registry proxy (`/api/pipes/store` on screenpi.pe) |
| POST | `/pipes/store/{publish,install,update,auto-update}` | B/D | |
| GET | `/pipes/store/check-updates` | B/D | |
| GET, DELETE | `/pipes/store/:slug` | B/D | |
| POST | `/pipes/store/:slug/review` | B/D | |

### 2.4 `/connections` nested router (target `connections_api.rs` lines 3647–3724) — semantics owned by the connections dimension

| Method | Path | St |
|---|---|---|
| GET | `/connections` | B |
| GET | `/connections/browsers` | B |
| GET | `/connections/browsers/:id/status` | B |
| POST | `/connections/browsers/:id/navigate` | B |
| GET | `/connections/browsers/:id/snapshot` | B |
| POST | `/connections/browsers/:id/act` | **N** |
| POST | `/connections/browsers/:id/eval` | B |
| POST/GET/GET/POST | `/connections/browser/pair/{start,status,pending,approve}` | B |
| WS/POST/POST/GET | `/connections/browser/{ws,eval,cookies,status}` | B |
| GET | `/connections/oauth/callback` | B |
| GET | `/connections/{calendar,ics-calendar,google-calendar}/{events,status}` | B |
| DELETE | `/connections/google-calendar/disconnect` | B |
| GET | `/connections/imap/messages` | **N** |
| GET | `/connections/imap/messages/:uid` | **N** |
| GET | `/connections/imap/mailboxes` | **N** |
| POST | `/connections/slack/send` | B |
| GET | `/connections/slack/search` | **N** |
| GET | `/connections/slack/conversations` | **N** |
| GET | `/connections/slack/history` | **N** |
| POST | `/connections/telegram/send` | **N** |
| POST/GET/POST | `/connections/whatsapp/{pair,status,disconnect}` | B |
| POST | `/connections/bee/pair/start` | **N** |
| POST | `/connections/bee/pair/poll` | **N** |
| POST (any) | `/connections/:id/proxy` (no sub-path; webhook-style JSON forward) | **N** |
| ANY | `/connections/:id/proxy/*path` | B |
| GET | `/connections/:id/config` | B |
| GET | `/connections/:id/instances` | B |
| PUT, DELETE | `/connections/:id/instances/:instance` | B |
| GET, PUT, DELETE | `/connections/:id` | B |
| POST | `/connections/:id/test` | B |
| – | `/connections/gmail/{instances,messages,messages/:id,send}` | **R** (no `/gmail/` literal anywhere in target) |

### 2.5 `/mcp-servers` nested router (target `mcp_servers_api.rs` lines 747–761)

| Method | Path | St |
|---|---|---|
| GET | `/mcp-servers` | B |
| GET | `/mcp-servers/registry` | **N** (api-032) |
| POST | `/mcp-servers/test` | B |
| POST | `/mcp-servers/:id/test` | B |
| GET | `/mcp-servers/:id/tools` | B |
| POST | `/mcp-servers/:id/call` | B |
| GET/POST/GET/POST | `/mcp-servers/:id/oauth/{status,start,callback,disconnect}` | B |
| GET, PUT, DELETE | `/mcp-servers/:id` | B |

### 2.6 App server `:11435` (target literals `0x59f61b0`–`0x59f6370`)

| Method | Path | St |
|---|---|---|
| POST | `/notify` | C (payload fields, api-037) |
| GET, POST, DELETE | `/notifications` (list / mark-all-read / clear) | B |
| DELETE | `/notifications/:id` | B |
| POST | `/notifications/:id/read` | **N** (api-037) |
| POST | `/inbox` | B |
| POST | `/log` | B |
| POST | `/auth` | B |
| GET | `/app-icon` | B |
| GET | `/installed-apps` | B |
| POST | `/window-size` | B |
| POST | `/focus` | B |

Distinct path counts (computed from the tables above):
- engine oasgen: target 117 (incl. hidden `/starred-sessions`) vs base 80 (incl. 2 `/openapi.*`) → 39 new, 2 removed;
- engine top-level axum (WS/SSE/proxies, `/power` counted once): target 47 vs base 22 (excl. macOS-only `/ai/*`) → 25 new;
- `/pipes/*`: target 25 vs base 21 → 4 new;
- `/connections/*`: target 43 vs base 36 → 11 new, 4 removed (gmail);
- `/mcp-servers/*`: target 11 vs base 10 → 1 new;
- app server `:11435`: target 11 vs base 10 → 1 new.

---

## 3. Delta items

Each item: identifiers, behaviour, shapes, defaults, evidence, confidence, base files, implementation notes.

### api-001 — Local calendar boundaries in flexible time params (changed, S)
- **Behaviour:** extend the flexible parser so `today`, `yesterday` (case-insensitive) and `YYYY-MM-DD` resolve to the *local* (machine timezone) midnight of that day, converted to UTC. `end_time=today` therefore means "start of today". Keep RFC3339 first, relative last. Update error text to mention all three families.
- **Evidence:** `0x6a08c68` (`%Y-%m-%d` beside `routes\time.rs`), `0x6a08d13` (error text), skill table `0x5117a1b`, export doc `0x6a3fc6e`. Location records `routes\time.rs` lines 28–29. *Observed.*
- **Base:** `crates/screenpipe-engine/src/routes/time.rs` (`parse_flexible_datetime`). Add tests for DST edge (use `chrono::Local` and `earliest()` on ambiguous local times).

### api-002 — Tabular response formats + field projection (new, M)
- **Identifiers:** query `format` ∈ {`json`,`csv`,`tsv`,`table`}; `fields` (comma list of dotted paths, e.g. `type,content.app_name,content.text`).
- **Behaviour:** see §1.5. Projection runs per row against the serialised JSON (`type` plus `content.*` for `/search`, element keys plus `bounds.left` etc. for `/elements`). Missing paths emit empty cells / omitted keys. CSV is RFC-4180 (quote fields containing `,` `"` CR/LF). TSV replaces `\t`, `\r`, `\n` with a space.
- **Errors:** 400 `{"error":"unknown format `x` (expected json|csv|tsv)"}` (exact string, client-visible).
- **Evidence:** `0x6a99270` (`text/csv`, `text/tab-separated-values`, `; charset=utf-8`), `0x6a954b9`, skill `0x5116243`–`0x5116335`, `0x5117ddc`–`0x5117e42`, Location `response_format.rs:128`. *Observed (wire), inferred (exact escaping).*
- **Base:** new module `crates/screenpipe-engine/src/routes/response_format.rs`; hook into `routes/search.rs::search` and `routes/elements.rs`.

### api-003 — `/search` v2 (changed, L)
**New query params** (serde list `0x52ca6b8`): `starred_session_id` (uuid), `starred_only` (bool), `input_context_only` (bool), `frame_id` (i64; with `content_type=parsed` returns parsed data of one frame), `actor_id` (i64; parsed only), `include_related` (bool; requires `tags`), `fields`, `format` (api-002). Existing params kept (`q, content_type, start_time, end_time, app_name, window_name, frame_name, include_frames, min_length, max_length, speaker_ids, speaker_name, focused, on_screen, browser_url, include_cloud, max_content_length, device_name, machine_id, filter_pii, tags, limit, offset`).
**`content_type`** enum (`SearchContentType` `0x6a303a0`): `all` (default), `ocr`, `audio`, `input`, `accessibility`, `memory`, `parsed`. `memory` and `parsed` are **not** included in `all`.
**Documented limits:** `limit` 1–20 per skill (server default 20); `start_time` effectively required (unbounded queries time out). Server enforces a 30 s budget → a timeout error that reports the 30 s limit and suggests narrowing the time window or adding filters (status code not observed; use 504); saturation → `search is busy — retry shortly` (exact string, client-visible) (use 503) (`0x6a3b750`, `0x6a3b9f8`).
**Starred filter:** intervals come from the `starred_sessions` table; the hit set is restricted *before* pagination to frames/audio inside any starred interval; each hit gets `starred: true|false` (`StarredSearchHit`, `0x6a30250`). More than 100 disjoint intervals in range → 400 with an error telling the caller to tighten `start_time`/`end_time` because the starred filter would span over 100 separate intervals (`0x5257590`).
**Response:** `SearchResponse { data: ContentItem[], pagination: {limit, offset, total}, cloud?, related? }` (`0x6a3b3d2`). `related` (only with `include_related=true` and `tags`) = map namespace → list of co-occurring tags ordered by frequency (e.g. `{"person":[…],"project":[…]}`).
**New `ContentItem` variant** `{"type":"Parsed","content":ParsedContent}` (`0x6a3d348`): `frame_id, timestamp, app_name, window_name, text, run_id, parser_id, parser_version, app_platform, app_id, app_executable, parse_duration_us, text_bytes, items: ParsedItem[], actors: ParsedActorReference[]`.
`ParsedItem` (`0x6a3d150`): `local_id, parent_local_id, item_key, identity_quality, kind, text/body, actor (raw parser label), timestamp, timestamp_precision, source_nodes`. Kinds seen: `conversation, document, calendar_event, page, table, derived, ephemeral` (`0x6a3bb1a`).
`ParsedActorReference` (`0x6a303a0`): `item_id, local_id, actor_id, name (canonical), observed_name, assignment_source`.
Other additions: `UiContent.event_source`; `InputContent.click_count`.
**Pipe rules:** §1.6.
- **Evidence:** above offsets; skill §2 `0x511750d`–`0x5118653`, §11 `0x511be47`–`0x511c3ae`; frontend `chunks/app/search/page-*.js`, `chunks/9665-*.js` (`/search?start_time=…&content_type=all&limit=10&offset=0`), `(main)/home/page-*.js` (`content_type=input`, `content_type=audio`). *Observed (params, enum, errors), inferred (related map exact shape).*
- **Base:** `routes/search.rs` (SearchQuery, SearchResponse), `routes/content.rs` (ContentItem), `screenpipe-db` `ContentType` (add `Parsed`) and search SQL. Parsed data storage/schema belongs to the semantic-parsing dimension; when absent, `parsed` returns empty `data`.

### api-004 — `/elements` and `/frames/:id/elements` v2 (changed, M)
- **New params:** `format` ∈ {`json`,`csv`,`tsv`,`table`,`outline`,`automation`,`preferred`}, `fields`, `limit`, `offset` (ElementsQuery `0x6a0bba0`, `0x59fbc06`).
- **`ElementResponse.state`** (`ElementState`): compact flags parsed from the element's captured `properties` JSON, emitted only when present: `focused, selected, expanded, collapsed, disabled` (disabled = !`is_enabled`), `off-screen`. Raw property keys consulted: `is_password, placeholder, help_text, role_description, is_enabled, is_keyboard_focusable, is_focused, is_selected, is_expanded` (`0x6a1ea0a`–`0x6a1eff0`).
- **`format=outline`** → `text/plain; charset=utf-8`, header `x-total-count`, `Cache-Control: no-store` (`0x6a1f760`). Indented role/text outline; a first line giving the frame id and how many text elements it holds; when truncated, a closing hint suggesting `?q=`, `?role=`, `?on_screen=true`, `?limit=` to narrow. Empty → a single line saying the frame has no elements carrying text, pointing the caller to `format=json` for the unfiltered tree.
- **`format=automation`** → only interactive controls (role families: button, link, checkbox/check_box, radio, switch, tab, menuitem, disclosure, cell/row/listitem/treeitem, text fields/textarea/edit/entry/input/password, combobox/listbox/popupbutton/select, slider/spinbutton, scrollarea/scrollbar, webarea), each line `[ref=eN key=k_… key_quality=derived|strong|ambiguous actions=… state=… bounds=…]`, preceded by a header line that labels the output as an automation snapshot and carries a `revision=r_…` token, plus a caution that the refs are only valid inside that one response. Empty → a single line saying no actionable accessibility controls were found, pointing the caller to `format=outline` for readable text. Allowed actions vocabulary: `click, set_value/type, press_key, scroll`.
- **`format=preferred`** resolves to whichever format the desktop's AI-context preference selects (outline when unset).
- **Errors:** history gate (api-035), `elements search failed`, `get frame elements failed` (exact strings, client-visible).
- **Evidence:** above; skill §3 `0x5118c28`–`0x5118f72`. *Observed (identifiers), inferred (exact text layout — design your own).*
- **Base:** `routes/elements.rs`.

### api-005 — `/activity-summary` v2 (changed, M)
- **New query params** (`0x522ba40`, `0x6a0c5b8`): `include_apps`, `include_windows`, `include_key_texts`, `include_parsed_count`, `include_starred` (all default true), keep `include_recording/memories/snippets/guidance`, `max_snippets`, `max_snippet_chars`, `max_memories`, `q`, `app_name`. Disabled sections are omitted from JSON. `start_time`/`end_time` required; error `start_time must be before end_time` with hint `Try start_time=30m ago&end_time=now` (exact strings, client-visible).
- **New response fields** (`ActivitySummaryResponse` `0x6a0c748`): `starred_sessions` (≤10 most recent `{id,start,end,has_audio}` intersecting range), `starred_sessions_has_more`, `parsed_context_count`, `attribution` (`AppAttributionStatus{total_frames, native_frames, recovered_frames, unresolved_frames}`), `time_range`. Base already has `apps[].minutes`, `windows[].minutes`, `total_active_minutes`, `data_status` (`ok|empty_but_recording|no_capture_in_range|not_recording`), `query_status` (`not_requested|matched|no_query_matches`), `guidance{searched_endpoints,next_best_query}`.
- **App attribution recovery:** frames with empty `app_name` borrow app/window/url from the nearest `ui_events` row tied to the frame or within a short preceding window (CTE `frame_fallback`/`resolved_frames`, `0x6a1a2bb`); minute allocation runs on resolved frames.
- **Evidence:** SQL fragments `0x522bc2c`–`0x522c9c7`, `0x6a19d24`, doc `0x6a1ada8`. *Observed.*
- **Base:** `routes/activity_summary.rs`.

### api-006 — `/activity-ledger` (new, L; data model may be owned by an activity/ledger dimension)
- **GET** `start_time`, `end_time` (≤31 days, else 400 with an error stating the 31-day maximum), `depth` ∈ {`task`,`action`} (`ActivityLedgerDepth`), `include_artifacts` (bool), `refresh` (bool; false = use cached ledger).
- **Response** `ActivityLedgerResponse { time_range:{start,end}, data_status: "ok"|"empty", generated_at, intervals: ActivityLedgerInterval[] }`. Interval fields seen (`0x6208570`): `task_id, parent_task_id, start_at, end_at, producer ("deterministic-v1"), status (provisional|final), summary, confidence, category (app|window|document|semantic|audio|unattributed|unobserved), evidence_count, evidence: ActivityLedgerEvidence[]{occurred_at, action_type, summary, source_type, source_id, frame_id, window_title, browser_url}`. Action types include `app_switch, window_focus, clipboard_event, pointer_down, audio_input, audio_output` (`0x6a236c8`).
- **Frontend:** `(main)/home/page-*.js` requests `depth=task&include_artifacts=true&refresh=false`; on error it falls back to `{intervals:[]}`.
- **Evidence:** handler `0x6a3ad98`, strings `0x6208434`–`0x6208570`, `0x6a2372e`–`0x6a23bb8`. *Observed (identifiers), inferred (grouping algorithm).*
- **Base:** new `routes/activity_ledger.rs` + `activity_ledger.rs` (deterministic grouping of ui_events/frames/audio into task intervals).

### api-007 — Starred work sessions `/starred-sessions` (new, M)
- **GET** `?limit=&offset=&start_time=&end_time=` → `{ data: StarredSession[], pagination? }` (frontend reads `data`). Sessions sorted by `start` desc.
- **POST** body `SaveSession { id (UUID, client-generated), start (RFC3339), end (RFC3339), hd_requested (bool), revision (0 = create, else current revision) }` → saved session (incremented `revision`). `StarredSession` = `{id, start, end, hd_requested, revision, has_audio, …}`.
- **Validation** (quoted messages are exact strings, client-visible): `valid UUID and nonnegative revision required`; the interval has to begin before the current time, last less than 24 h, and finish no later than 2 h after the current time; `start_time must precede end_time`; 409 on revision mismatch, with a conflict error telling the user the sessions were edited elsewhere and must be reloaded first; 404 `session no longer exists`; `HD capture is unavailable`; `Screen recording is disabled in settings`; pipes/agents may read but **marking requires a user token** (403 for pipe tokens).
- **Side effects:** when `hd_requested`, start/extend a high-fps (HD) capture bound to the session (`HighFpsController` "Starred" binding, `0x6a707a0`); app emits a `starred-sessions-changed` window event.
- **Evidence:** handlers `0x6a7aff0`/`0x6a7b0c8`, strings `0x5183990`–`0x5183bb8`, `0x6a7b298`; skill `0x511e39f`; frontend `starred-sessions/page-*.js`, `(main)/home/page-*.js`, `shortcut-reminder/page-*.js`. *Observed.*
- **Base:** new `routes/starred.rs`; table `starred_sessions` (schema owned by DB dimension); wire into api-003/api-005; `high_fps_controller.rs`.

### api-008 — Semantic actor identities `/semantic/actors/*` (new, M)
| Method | Path | Body / query | Notes |
|---|---|---|---|
| GET | `/semantic/actors/search` | `q` (≤256 chars), `limit`, `offset` | canonical + observed names |
| POST | `/semantic/actors/create` | `{name}` (1–256 chars) | new separate identity |
| POST | `/semantic/actors/update` | `{id, name}` | rename |
| POST | `/semantic/actors/merge` | `{actor_to_keep_id, actor_to_merge_id}` | ids must differ; merges current and future aliases |
| POST | `/semantic/actors/reassign` | `{item_id, actor_id}` | explicit item correction (preserved over heuristics) |
| POST | `/semantic/actors/aliases/reassign` | `{alias_id, actor_id}` | moves an alias with its history and future observations |
- **Errors:** 400 `invalid semantic actor request`, `… must be positive`; 404 `semantic actor or item not found` (exact strings, client-visible).
- **Evidence:** `0x522dab0`–`0x522dd88`, `0x6a7b298`–`0x6a7bab2`, skill `0x511bf56`–`0x511c1db`. *Observed.*
- **Base:** new `routes/semantic.rs`; storage owned by semantic-parsing dimension.

### api-009 — `/tags/autocomplete` (new, S)
- **GET** `?q=&limit=` (frontend uses 50/100). Response: bare JSON array of `{name, frame_count, audio_count, memory_count}` ordered by usage (frontend renders `#name` and counts) (`chunks/2037-*.js`, `3696-*.js`).
- **Evidence:** handler `content::autocomplete_tags`, `TagAutocompleteRequest` `0x6a303a0`. *Observed (path/params), inferred (sort).*
- **Base:** `routes/content.rs`; union of frame/audio tag junction tables and memory JSON tags.

### api-010 — Frame thumbnails and preview media (new, M)
- **GET `/frames/:frame_id/thumbnail`** `?width=&quality=&retry=&fallback=` (frontend: width from a srcset list, quality 75, `fallback=false` to disable nearest-frame fallback). Returns `image/jpeg`, `Cache-Control: private, no-cache`, `ETag`, `x-screenpipe-thumbnail-cache: hit|miss`, `x-screenpipe-thumbnail-width`, `x-screenpipe-thumbnail-height`, `content-length` (`0x6a1cfa0`). On source failure it tries nearby frames unless `fallback=false` (in that case the target emits a message noting that the exact source failed and fallback was off).
- **GET `/frames/preview-samples`** `?start_time&end_time&app_name&limit&browser_domain` → `{ frames: FramePreviewSample[] }`, sample = `{frame_id, timestamp, source: "snapshot"|"video", video_chunk_id?, video_offset_seconds?}`. Limits: range ≤24 h, `limit` 1..N, `app_name` 1–256 chars, `browser_domain` must be a host (`0x6a1e951`, `0x6a1e96f`). Only lists sample locations; it does not decode or extract any frame.
- **GET `/frames/preview-media/:video_chunk_id`** — returns a video chunk that was previously compacted to MP4, honouring HTTP range requests (`Accept-Ranges`, `Content-Range`, `video/mp4`) so the browser can decode it itself; no transcoding.
- **Evidence:** handler docs `0x6a1dde0`–`0x6a1e06d`; frontend `(main)/home/page-*.js` (`cp()`), `chunks/*` thumbnail helper (`/thumbnail?width=…&quality=75`), `3308-*.js` (`/frames/preview-media/`). *Observed.*
- **Base:** `routes/frames.rs` (reuse `frame_extraction_semaphore`, image cache, `video_utils`).

### api-011 — Auth exemptions: frames now protected (changed, S)
- Remove the `path.starts_with("/frames/")` bypass from the auth middleware; keep `/health`, `/ws/health`, `/audio/device/status`, `/connections/oauth/callback`, `/mcp-servers/*/oauth/callback`, `/connections/browser/pair/{start,status}`, `/notify`, `/pipes/store*`.
- Keep 403 JSON for missing/invalid token (the skill's mention of "401" is loose wording; the middleware string is unchanged).
- **Evidence:** §1.2. *Inferred from documentation + unchanged cookie mechanism.*
- **Base:** `crates/screenpipe-engine/src/server.rs` auth layer. Frontend must keep setting `screenpipe_auth` cookie (already does).

### api-012 — Client attribution headers and internal telemetry routes (new + decloud, S)
- Accept `X-Screenpipe-Client`, `X-Screenpipe-Agent`, `x-screenpipe-session` on all routes; record them in request tracing spans only.
- `POST /internal/telemetry/mcp-call` and `POST /internal/telemetry/mcp-value`: target forwards MCP tool-call outcomes (`mcp_tool_call_completed`) and `qualified_value_event` to PostHog (`0x5a51b20`). Freenpipe: accept any JSON, return `{"success":true}` (200), drop it. Never send.
- **Evidence:** `0x5a5eb30`, `0x52585a8`, `0x52585e0`, frontend `chunks/3822-*.js` (`qualified_value_event` property names `metric_version, surface, action, value_strength`). *Observed.*
- **Base:** new `routes/request_origin.rs`, `routes/internal_telemetry.rs` (no-op).

### api-013 — Speakers additions (new/changed, S)
- **GET `/speakers/sample/:audio_chunk_id`** `?start=&end=` (seconds within the chunk) → `audio/wav` clip, `Cache-Control: private, max-age=3600`. Errors: 404 `audio chunk not found` (exact string, client-visible); 422/400 when the chunk has no local file that can be played, when the chunk lives only in the cloud, when the requested window falls outside the chunk, when `start` or `end` is not a finite number, when `end` is not after `start`, and when the requested clip exceeds the maximum length (`0x5153810`–`0x51539e0`).
- **`/speakers/search`** adds `limit` and `include_samples` (bool; frontend sends `include_samples=false` for autocomplete).
- **`/speakers/reassign`** body adds `scope` (inferred values: single segment vs whole speaker); response adds `renamed_whole_speaker`, `previous_speaker_name`, `old_segment_assignments` (`0x51535a8`).
- **Evidence:** above; frontend `(main)/settings/page-*.js` (`/speakers/sample/<id>?start=&end=`), `3696-*.js`. *Observed.*
- **Base:** `routes/speakers.rs`.

### api-014 — Meetings additions (new/changed, M)
- **POST `/meetings/room-change`** `ResolveRoomChangeRequest {meetingId, token, decision: "switch"|"keep"}` → `MeetingRoomChangeResponse {decision, …}`. Answers the user prompt raised when the browser looks like it has switched to another meeting room but the same audio session is still running; emits `meeting_room_change_response`. Errors: one when the meeting that raised the prompt has already ended, and `decision must be 'switch' or 'keep'` (exact string, client-visible).
- **GET `/meetings/:id/summary-status`** `?pipe=&not_before=` → `MeetingSummaryStatusResponse {auto_summary_enabled, status, execution_id, execution_status, error_type, error_message}`; the one endpoint clients should consult for a meeting's summarisation state, derived from `pipe_executions` rows with `trigger_event='meeting_ended'` and `trigger_key` = meeting key (SQL `0x620aca0`), plus `pipe_event_runs` claim rows.
- **POST `/meetings/:id/summary`** `SaveMeetingSummaryRequest {summary, title?}` (field names inferred) → appends or refreshes the `## Summary` section of the meeting note and optionally retitles. Empty/failure-like text → 400 with an error rejecting a summary that is blank or reads like an error report rather than actual meeting content.
- **POST `/meetings/start`** adds `calendar_event_id`; adopting an active auto-detected meeting is idempotent; `another meeting is already active` (exact string, client-visible) (409) guarded by unique index `idx_meetings_single_open`; detection sources `manual_start`, `manual_start_possible_missed_detection`.
- **POST `/export`** adds `include_audio` (bool, default true); 400 `provide either meeting_id or start/end` (exact string, client-visible), and a 400 when `end` is supplied without a `start`; output default `<data_dir>/exports/` with timestamped `meeting_<id>_…mp4` style names.
- **Evidence:** `0x620a458`–`0x620add8`, `0x6a5cea8`–`0x6a5d2ea`, `0x6a431c8`. *Observed (ids), inferred (summary body field names).*
- **Base:** `routes/meetings.rs`, `meeting_export.rs`.

### api-015 — `/ws/meeting-overlay` WebSocket (new, M) and `/ws/metrics` field
- **Messages (server → client)**, all JSON `{type, data}`:
  - `{"type":"status","data":MeetingStatusResponse}` (same camelCase shape as `/meetings/status`: `active, manual, manualActive, activeMeetingId, stoppableMeetingId, meetingApp, detectionSource`), sent on connect and on every `meeting_status_changed`.
  - `{"type":"snapshot","data":{"meetingId":N,"items":Item[]}}` once after connect while a meeting is active (last transcript items; client keeps last 50).
  - item upserts (type name not observed; any other `type` is treated as an item) with `data: Item`. `Item = {meetingId, itemId, deviceName, deviceType ("input"|"output"), speakerName, capturedAt (RFC3339), text, isFinal}`; client keys items by `deviceName:deviceType:itemId` and replaces on re-send. Sourced from `meeting_transcript_delta` / `meeting_transcript_final` events.
- Reconnect with exponential backoff (client side). Connection-count guard identical to other WS routes (`Too many WebSocket connections`, exact string, client-visible).
- `/ws/metrics` payload adds `mic_capture_active` (bool) (`0x512e850`).
- **Evidence:** `0x6189a20` (`MeetingOverlayMessage`, `snapshot`, field names), `0x512e970`, frontend `shortcut-reminder/page-*.js` (reducer `$`, key fn `R`). *Observed.*
- **Base:** `routes/websocket.rs`, `screenpipe-events` (`meeting_transcript_delta`/`_final` events owned by the meeting live-transcription dimension).

### api-016 — Per-monitor vision control (new, S)
- `POST /vision/device/start` / `POST /vision/device/stop` body `VisionDeviceControlRequest {monitor_id}` → `{success, message}` (the message confirms which monitor was paused or resumed). Pause is a user pause persisted across restarts (any attempt to start a user-paused monitor is skipped, with a log entry).
- `GET /vision/device/status` → `VisionDeviceStatusEntry[]` = one per monitor `{monitor_id, name, active, user_disabled}` (feeds the per-monitor recording popover in the UI).
- **Evidence:** `0x59fb855`–`0x59fb96f`, `0x6a72278`, frontend `(main)/home/page-*.js`. *Observed.*
- **Base:** new `routes/vision.rs`; `vision_manager/manager.rs` (pause/resume per monitor). Mirror of existing `/audio/device/*`.

### api-017 — Artifacts library `/artifacts*` (new, M)
- **POST `/artifacts/register`** `RegisterArtifactRequest {file_path, source, source_type?, title, kind?, …}` → copies the file (≤100 MB) into `~/.screenpipe/outputs/` and upserts a registration row; if the file is a `.saf.json` envelope (`saf_version` must be 1, `artifact_id` `^[a-z0-9][a-z0-9_-]*$`, `kind`, `title`, `body` object, `version` ≥1) it is validated, else registered as a plain file. Indexes text for search.
- **GET `/artifacts`** `?q&source&limit&offset&per_pipe_limit&include_internal` → `{data: ArtifactItem[], pagination:{limit,offset,total}, sources: string[]}`; merges registered outputs with artifacts derived on the fly from pipe output directories, deduped by path (registered wins); filtering/paging server-side. `ArtifactItem` keys: `id, registered, source, source_type, title, kind, original_path, output_path|file_path, preview, saf_kind, artifact_id, saf_version, modified_at, created_at`.
- **DELETE `/artifacts/:id`** removes a registered row and its copied file; filesystem-derived items cannot be deleted (404).
- **Evidence:** docs `0x6a7c9c8`–`0x6a7cc63`, validation `0x6a7cc8b`–`0x6a7cda6`, `0x6189418`; frontend `3696-*.js`. *Observed.*
- **Base:** new `routes/artifacts.rs`; table schema owned by DB dimension.

### api-018 — Unified AI feedback `/feedback` (new, M)
- **POST** `SetFeedbackRequest {target:{kind, id, version?}, rating:"up"|"down", comment?, snapshot, producer_ref?, context?}`. Rules: `kind` lowercase `[a-z0-9._-]`; comment only with `down`; comment length cap; a pipe cannot rate its own outputs (403); rater recorded as `local-user`. Frontend example: notification feedback with `snapshot {title, body}`, `producer_ref "pipe:<name>"`, `context {notification_type, source_session_id, source_message_id}`.
- **GET** `?kind&producer&rating&q&target_id&since&since_inclusive&after_id&order&limit` → records `{id, target:{kind,id,version?}, rating, comment?, snapshot, producer_ref, actor_ids?, context, device_id, created_at, updated_at}`. Pipe-scoped tokens only see records whose producer is that pipe.
- **Evidence:** `0x5183483`–`0x5183740`, skill §14 `0x511d801`, frontend `chunks/3822-*.js`. *Observed.*
- **Base:** new `routes/ai_feedback.rs`.

### api-019 — Structured output targets `/outputs/targets*` (new, L)
- **GET `/outputs/targets`** `?pipe=<name>` (and consumer filters) → `{data: OutputTarget[]}`; target = `{id, title, instruction, revision, schema_name, schema (JSON Schema subset), consumer, bound_pipe, feedback:{up_count, down_count, current?, recent[]{rating, artifact_version, output_payload?, correction?, created_at}}, latest?:{payload, evidence[], artifact_output_id, artifact_version, updated_at}, item_actions:{items?[]{item_id, disposition: active|resolved|snoozed|dismissed, snoozed_until?, correction?, updated_at}}, created_at, updated_at}`.
- **POST `/outputs/targets/:target_id/submit`** `{target_revision, payload, evidence[]}` → `{target_id, artifact_output_id, artifact_version}`. Validates payload against the target schema (supported keywords: `type` (string or array), `properties` (≤32), `required`, `additionalProperties` (bool), `items`, `maxItems`, `maxLength`); payload/schema byte caps; evidence refs `{event_id|frame_id|transcription_id, ts, device_id}` capped in count/length; only the bound pipe may submit; stale revision → 409 `target revision changed (current N, submitted M)` (exact string, client-visible). Writes `.saf.json` artifact under the outputs dir.
- **POST `/outputs/targets/:target_id/feedback`** `{artifact_output_id, artifact_version, rating, correction?}` → `{feedback:{up_count, down_count, current}}`; `correction` requires `down`.
- **POST `/outputs/targets/:target_id/items/:item_id/actions`** `{artifact_output_id, artifact_version, action, snoozed_until?, correction?}`; actions `resolve, snooze, correct, dismiss, reopen` (+ `up`/`down`); only `list.v1` outputs support item actions; `snooze` needs future RFC3339 `snoozed_until`; `correct` needs `correction` → `{item_actions:{…}}`.
- Target id convention for Live View blocks: `live-view:<viewId>:<blockId>`. Store: `<data_dir>/structured-outputs/targets.json` (+ `.lock`).
- **Evidence:** `0x6a23bb8`–`0x6a24960`, `0x522d0ff`–`0x522d61b`, embedded extension `0xa0ae18d`–`0xa0afe5c`, frontend `(main)/home/page-*.js` (`ay`, `t6`). *Observed.*
- **Base:** new `routes/structured_outputs.rs` + store module.

### api-020 — Live Views `/live-views*` (new, L; UI owned by the frontend/dashboard dimension)
- **GET `/live-views`** → `{views:[…]}`-style list of saved views with block slots and their latest values (extension reads `id, title, revision, blocks/slots[{id,title,component|kind,binding.pipeName|source.pipeName,value}]`).
- **GET `/live-views/schema`** → bundled JSON Schema for `live-view-template.v1`.
- **GET `/live-views/catalog`** → bundled kits (`live-view-kit.v1`: `id, title, description, version, pipes[{name, distribution:"bundled"}], template`). Kits seen: `daily-memory`, `meeting-follow-through`, `time-and-focus` (pipes `day-recap`, `missed-todos`, `meeting-summary`, `time-breakdown`).
- **GET `/live-views/:id/template`** → the view's template (`schema, id, title, revision, timeRange, periodPolicy, blocks, createdAt, updatedAt`).
- **PUT `/live-views/:id`** `SaveLiveViewRequest {id, title, expectedRevision (null to create), timeRange, periodPolicy, blocks}`; path id must match body id; creating with `expectedRevision` → 400; stale revision → 409. **DELETE `/live-views/:id`** (inferred second verb).
- Template rules: id `^[a-z0-9][a-z0-9_-]*$` ≤64; title 1–120; block `{id, title, kind ∈ metric.v1|list.v1|bar-chart.v1|line-chart.v1|table.v1|timeline.v1|markdown.v1, width ∈ 3|6|12, order, intent (1–800), source {type:"pipe.v1", pipeName}}`; `timeRange` ∈ `today|24h|7d|30d`; `periodPolicy` `{type:"fixed.v1", value}` or `{type:"selectable.v1", values[]}`; max views/blocks caps. Pipes may fill blocks but not edit templates (403).
- Store `<data_dir>/live-views/templates.json` (+ lock); migrates legacy `brain-overview-views.json`.
- **Evidence:** `0x6a84e91`–`0x6a85840`, kits `0x6a85a33`…, extension `0x5291c60`–`0x5295208`, `0xa0d3d1b`. *Observed (identifiers), inferred (list response wrapper key).*
- **Base:** new `live_views.rs`, `routes/live_views.rs`.

### api-021 — Agent self-improvement routes `/agent/*` (new, M)
- **GET `/agent/self-improvement/context`** → `{system_prompt: string}` — engine-rendered block containing frozen user-profile facts (wrapped as data, not instructions) and usage guidance for `user_profile`/`skill_manage`. Write your own wording.
- **POST `/agent/profile/manage`** internally-tagged `{action: "list"|"save"|"delete", id?, content?, tags?, importance?, confirmed?, source?}`; save stores one compact fact (client caps 2000 chars, server tag/length caps; entries matching injection/secret patterns are stored blocked with `blocked_reason`); delete needs `id` + `confirmed:true`. Returns profile items `{id, content, tags, importance, blocked_reason?, …}`.
- **POST `/agent/skills/manage`** `{action: "list"|"read"|"create"|"patch", name?, description?, instructions?, expected_sha256?, confirmed?, source}`; list → `{skills:[{key, description, instructions?, sha256, origin}]}`; create needs `confirmed:true`; patch only for agent-created skills with matching `sha256`; bundled/user/imported skills read-only; content scanned for unsafe patterns (prompt-injection, role hijack, secrets, hidden HTML comments); provenance in `.screenpipe-agent-skills.json`.
- **POST `/agent/learning/chats`** `{query}` → `{results:[{preview, updated_at, …}], warnings[]}` — bounded chat-history previews for the opt-in skill-learning pipe.
- **Evidence:** extension `0xa0a782e`–`0xa0ab399`; `0x6a0f0ea`–`0x6a0f519`, `0x6a43757`–`0x6a43c57`, `0x6221d58`, `0x6222288`. *Observed.*
- **Base:** new `agent_profile.rs`, `agent_skills.rs`; routes in `server.rs`. Profile storage may reuse `memories` with a dedicated `source`.

### api-022 — Connect broker `/v1/connect-request`, `/v1/connect-response` (new, S)
- **POST `/v1/connect-request`** `ConnectRequestBody {connection_id, name?, reason?, required_for?, session_id?}` (from an agent) → engine assigns `request_id`, emits event `acp_connect_request` to the desktop UI (which shows a card asking the user to connect the named integration before the agent can proceed), and **blocks** until a response or timeout → `{request_id, status: "connected"|"declined"|"timeout"}`.
- **POST `/v1/connect-response`** `ConnectResponseBody {request_id, status: "connected"|"declined"}` (from the UI) → completes the waiting request; also emits `acp_connect_response`.
- Timeout value not observed (choose ~5 min).
- **Evidence:** `0x5255a24`–`0x5255bb0`, `0x6a6aba0`; extension `0xa0d302b`; frontend `chunks/3696-*.js` (`iZ`). *Observed.*
- **Base:** new `routes/connect_broker.rs` (oneshot map keyed by request_id).

### api-023 — Local OpenAI-compatible proxy `/v1/local/chat/completions` (new, M)
- OpenAI chat-completions body passthrough to the user's **configured default AI preset** (BYOK): resolves base URL (e.g. `http://localhost:11434/v1` for Ollama, `https://api.openai.com/v1`, custom) + key + model, POSTs `<base>/chat/completions`, streams the upstream body back (`Content-Type` from upstream, `Cache-Control: no-cache`).
- Errors (OpenAI error shape `{error:{message,type:"invalid_request_error"}}`): no preset → a message that no default AI preset exists and the user should create one in AI settings; preset without URL → a message that the chosen preset is missing its base URL and the user should fill it in under AI settings; upstream down → 502 with a message that the AI provider could not be reached, followed by the underlying error.
- Presets of type `screenpipe-cloud` must be rejected or mapped to a local provider in Freenpipe.
- **Evidence:** `0x5253b30`–`0x5253c9c`, `0x6a64508`. *Observed (errors), inferred (preset resolution).*
- **Base:** new `local_chat.rs`; settings store access (`store.bin` aiPresets) — coordinate with settings dimension.

### api-024 — `/v1/chat/completions` cloud proxy (decloud, S)
- Base and target forward to `https://api.screenpipe.com/v1/chat/completions` with the cloud JWT (`0x5183758`). Freenpipe: re-implement as an alias of api-023 (BYOK preset) or return 501 `{"error":"cloud_disabled"}`. Never embed screenpipe endpoints.
- **Base:** `routes/cloud_proxy.rs`.

### api-025 — `/v1/web-search` (new + decloud, M)
- **Contract:** POST `{query}` → `{content: string, sources: [{title?, url?}]}`. Errors: 401 `{"error":"cloud_token_missing","message":…}`, 502 `upstream_unreachable`, 504 `upstream_timeout` (`0x6188eda`–`0x6189041`).
- Target proxies to `https://api.screenpipe.com/v1/web-search`. Freenpipe: implement with a user-configured provider (e.g. SearXNG URL or a BYO search API key) and synthesise `content` as a concise list of results; if unconfigured return 501 `{"error":"web_search_not_configured"}`.
- **Evidence:** extension `0x52aa320`–`0x52aae85`. *Observed.*
- **Base:** new `routes/web_search.rs`.

### api-026 — `/data-sync/devices`, `/data-sync/search` (new + decloud, M)
- Target proxies to `SCREENPIPE_WEB_URL` (default `https://screenpipe.com`) `/api/user/data-sync/{devices|search}` with the signed-in identity; 401 with a message asking the user to sign in before synced devices can be queried.
- **Search params:** `q, device_name, device_id, app_name, since, until, since_hours_ago, limit`. Devices response: list of devices with names/ids/last-seen.
- Freenpipe: back these with local multi-device data (the `device_name`/`machine_id` columns already filled by sync/import) or with the SSH remote-sync feature; else return 501 `{"error":"data_sync_unavailable"}`.
- **Evidence:** `0x6189041`–`0x6189148`, `0x6a9e4a8`, skill `0x5118799`–`0x5118bfd`. *Observed.*
- **Base:** new `routes/data_sync_proxy.rs`.

### api-027 — Cloud coding agents `/cloud-agents/*` (new, M)
- `GET /cloud-agents/status` → `{providers: CloudAgentProviderStatus[]}`; `POST /cloud-agents/:provider/connect` → `{providers}`; `GET /cloud-agents/:provider/codebases` → `{codebases: CloudCodebaseSummary[]}`; `GET /cloud-agents/cursor-agents` → `{agents: CursorCloudAgentSummary[]}`; `PUT /cloud-agents/cursor-key` `{key}` (stored in secret store `cloud_agents.cursor.api_key`, env override `CURSOR_API_KEY`; 503 `encrypted secret storage is unavailable`, exact string, client-visible).
- Providers: `cursor` (Cursor Cloud Agents API `https://api.cursor.com`), `codex`. Pipe config gains `cloud_agent: CloudAgentConfig {provider, environment_id, branch, session_id, agent_id, repository, starting_ref, model, send_screenpipe_context, context_lookback_hours, context_max_items}`.
- Third-party BYOK — allowed in Freenpipe (not a screenpipe service).
- **Evidence:** `0x522a0b8`–`0x522a7c8`, `0xa049a08`–`0xa049fa0`, frontend `chunks/5977-*.js`. *Observed.*
- **Base:** new `routes/cloud_agents.rs`; `screenpipe-core/src/agents` (cloud.rs equivalent).

### api-028 — Workflows API (new, L; semantics owned by the workflows dimension)
| Method | Path | Shape summary |
|---|---|---|
| GET | `/workflows` | `?q&limit&offset` → saved catalog search (a read does not trigger any analysis run); 404/409-ish `workflow_catalog_not_configured`, `workflow_catalog_unreadable` |
| GET | `/workflows/:id` | `?include_automation=true` → ordered stages, procedure, quotes, trigger/outcome, bounded `automationEvidence` (per captured frame: timestamp, app, match distance, role/text/depth, normalised bounds, `truncated`, `totalNodes`, `contextPath`) |
| GET | `/workflows/catalog` | saved catalog (`analysis`, `analyzedAt`, `checkedThrough`, `schemaVersion`, `workflows[]`) |
| POST | `/workflows/catalog` | `CommitRequest {expected_revision, pipeline_revision, workspace_revision?, draft_id?, checked_through, workflows}` → receipt with new `revision`/`checkedThrough` |
| GET | `/workflows/context` | catalog + user profile/corrections + `outputContract` |
| GET/POST | `/workflows/pipeline` | `?task=` stage input (`ready, window, revision, inputRevision, checkedThrough, input{items,coverage}, previous`) / `StageCommit {task, expected_revision, input_revision, checked_through, items, coverage}`; tasks `workflow-activity|patterns|procedures|timing|discovery` |
| GET/POST | `/workflows/workspace` | `?task=` agent workspace context / `Change {action, expected_revision, draft_id, assignee, payload, note}` (agents `workflow-discover|deepen|review|maintain`) |
| POST | `/workflows/edits` | `EditRequest {id, expected_revision, title, trigger, outcome, stages[StageEdit{description, procedure[ProcedureEdit{kind,text}]}]}` → `{workflow}` |
| POST | `/workflows/corrections` | `CorrectionRequest {id, correction, changes?, expected_revision?}` (≤20,000 bytes) → `{success, workflow}` |
| POST | `/workflows/rollout` | `RolloutRequest {enabled}` — desktop-only |
- Storage files `catalog.json`, `catalog.backup.json`, `catalog.lock`, `pipeline.json`, `pipeline.backup.json`, `pipeline.lock`, `profile.json`.
- **Evidence:** `0x6208570`–`0x620a1f0`, `0x6a6abf9`–`0x6a6b740`, skill `0x5114427`–`0x51169c6`, frontend `chunks/4656-*.js`, `dev/workflows-ux/page-*.js`. *Observed.*
- **Base:** new `routes/workflows*.rs`.

### api-029 — Pipes API extensions (changed/new, M; pipe runtime owned by the pipes dimension)
- `GET /pipes` adds `execution_limit` and `include_execution_counts` (frontend: `?include_executions=true&execution_limit=1&include_execution_counts=true`); `PipeStatus` gains `is_bundled_builtin`, `has_saved_context`, `next_run` (base already has `current_execution_id`, `consecutive_failures`, `locally_modified`); `PipeConfig` gains `cloud_agent`, `effort` (`low|medium|high`), `run_in` (`{mode:"existing_chat", chat_id}`), `artifacts` (`ArtifactDeclaration{kind,…}`), `schedule_config` (base already has `subagent`, `privacy_filter`). Full pipe config semantics belong to the pipes dimension (`0x61874e0`–`0x6187760`).
- `GET /pipes/:id/executions` adds `before_id` (cursor) and `include_output`.
- `GET /pipes/:id/executions/:exec_id` (new) → one `PipeExecution {id, pipe_name, status (queued|running|completed|failed|cancelled), started_at, finished_at, pid, exit_code, error_type, error_message, stdout, stderr, session_path, conversation_id, trigger_event, trigger_key}`; 404 `execution not found`.
- `POST /pipes/:id/run` body `RunPipeBody {run_context?, notification_context?, trigger_type?, meeting_summary?:{meeting_id, meeting_end}}` → `{success:true, execution_id}`; refuses to start when the execution-history store is unavailable, since such a run could not be tracked; `meeting_summary` requires positive id + end.
- `POST /pipes/bundled/:id/install` (new) copies a bundled pipe into the user pipes dir and loads it.
- `POST /pipes/schedule/preview` (new) body `ScheduleConfig {frequency (minutes|hours|days|weeks|months), interval, days_of_week, day_of_month, at_hour, at_minute, timezone, starting, ending, max_occurrences}` → `{summary: string, next: [RFC3339…]}`.
- `GET /pipes/activity` (new) `?limit&before_id` → `{data: PipeExecutionActivity[]{pipe_name, execution_count, latest_execution_id, last_run_at}}`.
- `GET /pipes/:id/stream` (new) — Server-Sent Events of a running pipe's output (`progress` events and `keep-alive`); 429/503 `too many active pipe streams` (exact string, client-visible).
- Error envelope `{error, error_code}` with codes `pipe_not_found, timeout, disk_full, invalid_state, io_error, pipe_setup_failed, retry_available, free_pipe_limit_reached (decloud: never emit), operation_failed, permission_denied`.
- **Evidence:** `0x5184e28`–`0x5185488`, `0x61874e0`–`0x6187760`, `0x6a6d648`, frontend `chunks/5977-*.js`, `(main)/home/page-*.js`, `3696-*.js`. *Observed.*
- **Base:** `crates/screenpipe-engine/src/pipes_api.rs`, `pipe_store.rs`, new `pipe_stream.rs`.

### api-030 — Data / sync / archive / vault additions (new + decloud, S)
- `POST /data/compact` → `CompactResponse` (bytes before/after, duration); runs a full `VACUUM` while holding an exclusive lock, so recording writes wait until it completes; needs ~2× DB size free.
- `POST /sync/reset-account` `{confirmation}` — destructive cloud reset (decloud: return 501).
- `POST /archive/download` `{output_dir, token}` → starts background export (decrypt every archived blob into `media/` and `metadata/`); `GET /archive/download/status` → `DownloadProgress {blobs_downloaded, …}` (decloud: archive is a screenpipe-cloud feature; return 501 or map to local backup).
- `/vault/lock` online: 409 with an error explaining that the vault cannot be locked over the API while the server holds open SQLite connections, and directing the user to shut screenpipe down and use the `screenpipe vault lock` CLI command instead; vault unavailable on hybrid storage.
- **Evidence:** `0x6a6c9a6`–`0x6a6cb13`, `0x6a28c98`–`0x6a28e7d`, `0x5184098`–`0x5184395`, `0x522d6fa`. *Observed.*
- **Base:** `routes/data.rs`, `sync_api.rs`, `archive.rs`, `routes/vault.rs`.

### api-031 — Connections router deltas (changed, M; deep semantics owned by connections dimension)
- New: `POST /connections/browsers/:id/act` `BrowserActBody {ref, action (click|fill|clear|check|uncheck|select|hover|focus|type), value?}` → `{ok, ref, action, tag, url}` or `{ok:false,error}`; acts on `data-sp-ref` refs stamped by `/snapshot`.
- New IMAP: `GET /connections/imap/messages`, `GET /connections/imap/messages/:uid`, `GET /connections/imap/mailboxes` (credentials via generic `PUT /connections/imap {credentials:{imap_host, imap_port, username, password}}` and `POST /connections/imap/test`).
- New Slack reads: `/slack/search` (→ `search.messages`, `query`), `/slack/conversations` (`limit`), `/slack/history` (→ `conversations.history`, `channel`, `limit`).
- New: `POST /connections/telegram/send` `{text, chat_id?}`.
- New Bee pairing: `POST /connections/bee/pair/start` → `{request_id, pairing_url}`; `POST /connections/bee/pair/poll` `{request_id}` → `{status: pending|completed|expired|unknown}`.
- New: `POST /connections/:id/proxy` (no sub-path) forwards a JSON body to a configured webhook target (n8n/Zapier/Make/Discord/Teams); errors `webhook_proxy_unavailable, invalid_webhook_target, connection_not_configured, upstream_rejected, transport_failed`.
- Removed: `/connections/gmail/*` (Gmail now via generic Google OAuth proxy / IMAP).
- **Evidence:** Location table `connections_api.rs:3647–3724` (`0x5182690`…), `0x517c750`–`0x517d238`, `0x6a320ee`…, frontend `chunks/5977-*.js`, `(main)/home/page-*.js`. *Observed.*
- **Base:** `crates/screenpipe-engine/src/connections_api.rs`, `crates/screenpipe-connect`.

### api-032 — `GET /mcp-servers/registry` (new, S)
- `?search&cursor&limit` (frontend limit 30) → proxies the public MCP registry `https://registry.modelcontextprotocol.io/v0/servers` and returns `{data:{servers:[…], nextCursor}}`; errors `mcp registry returned HTTP N`, `mcp registry unreachable`, `mcp registry response was not JSON` (exact strings, client-visible).
- Third-party public registry, not a screenpipe service; allowed (make the base URL configurable).
- **Evidence:** `0x6188550`–`0x6188608`, Location `mcp_servers_api.rs:750`, frontend `chunks/5977-*.js`. *Observed.*
- **Base:** `crates/screenpipe-engine/src/mcp_servers_api.rs`.

### api-033 — `/health` v2 (changed, M; diagnostics semantics may be owned by a health dimension)
- Serialization order in target (`0x5256cd6`): `status, status_code, dependencies, runtime, startup, unhealthy_reasons, last_frame_timestamp, last_audio_timestamp, frame_status, vision_reason, loop_stage, loop_stage_age_secs, audio_capture_mode, audio_status, message, verbose_instructions, device_status_details, capture_status, monitors, pipeline, audio_pipeline, accessibility, ui_recorder, recording_coverage, pool_stats, write_queue_degraded, write_queue_consecutive_fatal, write_queue_consecutive_contention, write_pool_reopens, persistent_failure_signals, vision_db_write_stalled, audio_db_write_stalled, drm_content_paused, schedule_paused, hostname, version`.
- New: `unhealthy_reasons[] {code, component, severity, message, remediation}` with codes `health_snapshot_timeout, vision_*, audio_*, database, vision_write_stalled, transcription_write_stalled, frame_drop_rate_high, write_latency_high, transcription_queue_full`; `vision_reason` ∈ `ok, disabled_by_setting, no_displays_expected, screenshots_disabled_by_config, screenshots_disabled_by_power_profile, permission_denied, capture_stalled, not_started, ocr_unavailable`; `audio_status` ∈ `ok, recording, waiting_for_voice, transcript_pending, transcript_paused, audio_stalled, audio_not_started, mic_paused, no_input_device, active_no_data, waiting_for_meeting, meeting_detector_unavailable, screen_locked, disabled`; `capture_status {status, reason, active_audio_devices, paused_audio_devices, pending_transcription_segments}`; `recording_coverage (CoverageSnapshot {recording_secs, active_secs, idle_secs, active_stalled_secs, active_paused_secs, coverage_ratio})`; `pipeline` adds `frames_dropped_timeout, frames_dropped_error, silent_loss, silent_loss_rate, capture_attempts, capture_loop_heartbeats, dedup_skips, frames_corrupt_black, frames_corrupt_green, last_capture_attempt_ts, last_capture_loop_heartbeat_ts, ocr_empty`.
- Health computation has a budget; on overrun it serves the last cached snapshot (a `health_check` log line records the overrun and the fallback).
- **Evidence:** `0x5256419`–`0x5257420`, `0x6a24af0`–`0x6a26478`, `0x6a22bc0`–`0x6a23150`. *Observed.*
- **Base:** `routes/health.rs`, `recording_coverage.rs` (new).

### api-034 — Search-only mode gate (new, M)
- When the engine runs in `search_only` state, read routes stay available; routes that need live capture or the desktop (capture/device control, meeting start/stop, `/v1/connect-request`, chat/agent routes, HD capture, starring with HD) return 503 `{"error":"<message>","code":"search_only"}` (code name inferred), where `<message>` tells the user the app has been quit, only previously saved history is being served, and reopening the app re-enables the feature (write your own wording).
- A 16-byte path-compare table near `0x6a84320` lists `/vision/device/status, /audio/device/status, /tags/autocomplete, /activity-ledger, /semantic/actors/search, /starred-sessions, /activity-summary, /tags/vision/batch` — probably part of this read allow-list (unconfirmed).
- **Evidence:** `0x6187fe0`, `0x49878…`, `0x517b3d3`. *Inferred.*
- **Base:** new `search_only.rs` middleware in `server.rs`; app-side lifecycle owned by the app/tray dimension.

### api-035 — History-access gating (decloud / ee-stub, S)
- Do **not** implement the 24 h limit. Optionally reserve the error code `history_access_limited` (403) for future local policy. Remove any entitlement checks.
- **Evidence:** §1.8. *Observed.*

### api-036 — Removed routes (removed, S)
- `/openapi.yaml`, `/openapi.json` (no literals in target). Recommendation: keep them in Freenpipe (harmless, useful) but they are not part of target parity.
- `/connections/gmail/*` (see api-031).

### api-037 — App server `:11435` notification updates (changed, S)
- New `POST /notifications/:id/read` → marks one history entry read (frontend `notification-inbox/page-*.js`, `7737-*.js`).
- `NotifyPayload` (`0x52cadd8`) adds: `priority` (`high|normal|low`, default normal; low is toast-only), `transient`, `pipe_name`, `workflow_ids`, `source_session_id`, `source_message_id`, `source_url`, `surface`, `position`, `anchor`, `cta`, `survey`, `dismissible`, `expires_at`, `calendar_title`, `detection_source`. Action types `link | deeplink | pipe | chat | api | dismiss`, ≤5 actions, each `{id,label,type,…}`; `pipe` actions take `pipe, context, open_in_chat`; `chat` actions take `prompt, context, auto_send`; `api` actions take `url, method, body`. Default `autoDismissMs`/`timeout` 20000 ms. Response unchanged `{"success":true,"message":"Notification sent successfully"}` (exact string, client-visible; pipes test for this message to detect delivery).
- **Evidence:** skill §13 `0x511cb5d`–`0x511d7d2`; `0x59f6268`. *Observed.*
- **Base:** `apps/screenpipe-app-tauri/src-tauri/src/server.rs`, `notifications/routes.rs`, `notifications/store.rs`.

### api-038 — Pipe permission middleware lanes (changed, S) — see §1.6
- **Base:** `crates/screenpipe-engine/src/pipe_permissions_middleware.rs`, `screenpipe-core/src/pipes/permissions.rs`.

### api-039 — Retranscription updates (changed, S)
- `/audio/retranscribe` documented engines: `deepgram, screenpipe-cloud (decloud: reject), whisper-large, whisper-large-v3-turbo, whisper-large-v3-turbo-quantized, qwen3-asr, parakeet, parakeet-mlx, openai-compatible`.
- `POST /meetings/:id/retranscribe`: max 2 h per pass; meeting must be stopped; requires a configured engine; response adds `transcript_updated_at`, `summary_refresh_event_emitted` (after replacing segments it requests a meeting-summary refresh via `meeting_summary_refresh_requested`).
- **Evidence:** `0x6207e90`–`0x620814b`, `0x6a0c748`, skill §6. *Observed.*
- **Base:** `routes/retranscribe.rs`.

### api-040 — PII filter and pipe-store registry (decloud, S; base-present)
- `filter_pii=true` (search) routes text through a remote attested enclave (`SCREENPIPE_PRIVACY_FILTER_ENCLAVE`, `pii.screenpipe.containers.tinfoil.dev`, `0x6a92200`). Freenpipe: implement locally (`screenpipe-redact` regex/NER) or return text unchanged with a warning.
- `/pipes/store*` proxies the screenpi.pe registry (`/api/pipes/store`). Freenpipe: point at a configurable registry URL (default: disabled/empty list).
- **Base:** `privacy_filter.rs`, `routes/pipe_store.rs`.

---

## 4. Implementation notes (engine)
1. Keep oasgen registration for new read/write routes that are oasgen in target (§2.1) so a future `/openapi.json` stays complete; WS/SSE/proxy routes go through plain axum (§2.2).
2. Route order matters in nested routers: register literal segments (`/pipes/activity`, `/pipes/schedule/preview`, `/pipes/bundled/:id/install`, `/connections/imap/*`, `/connections/bee/*`, `/mcp-servers/registry`) **before** `/:id`.
3. All new mutating routes must pass the pipe-permission middleware; add endpoint descriptors for them in `screenpipe-core/src/pipes/permissions.rs`.
4. Every `decloud` item must have zero network calls to screenpipe/mediar hosts; return 501 JSON `{"error":"…_unavailable"}` when no local equivalent is configured.
5. Error envelope convention used across new routes: `{"error": "<human message>", "code"?: "<snake_case>"}`; conflicts → 409, validation → 400, not found → 404, permission → 403, unavailable subsystem → 503, upstream failures → 502/504.

---

## Verification

Adversarial pass, 2026-10-15. For every item I (a) grepped the BASE (`crates/`, `apps/`) for its identifiers and read the base structs and handlers involved, and (b) spot-checked every cited offset in `app_strings.tsv`, the cited frontend chunks, and where needed raw `.text` bytes of the target exe. All reads were static; nothing was executed. Inline strings were decoded from `movabs`/`mov` immediates and RIP-relative operands. The PE section table shows `.text` and `.rdata` both at file-to-RVA delta `0xc00`, so file offsets resolve directly.

Verifier tools (new, in `tools/`): `vapi_ident.py` (multi-regex hit counter), `vapi_hexdump.py`, `vapi_imm.py` (printable 8-byte immediates), `vapi_riprefs.py` (resolve RIP-relative lea/pcmpeqb/movups/movdq targets), `vapi_xref.py`, `vapi_srcmods.py` (new engine source modules vs base), `vapi_fe_unknown.py` (frontend paths with unknown first segment), `vapi_fe_vs_spec.py` (frontend paths not mentioned in this spec).

REA: `trace_application_feature` seed `string:"/activity-ledger"` → `ev_81ceac58e4f75f696a6c2adc4a19af9894839a9d2705960dee38a6c3fbc23eeb` (no-match). Seed `module:"page-c18385b042b3382e.js"` → `ev_02aa9861556529ce7fa974d5d529f9d7c23a7022552c94d855a1e5e3efbbdc98` (one module matched, 3 nodes, no route literals indexed). Both are against parent `ev_57f23857…6216d`. This confirms the inventory's note that the graph does not carry fetch literals, so direct literal scans are the primary evidence.

### Verdicts

| Item | Verdict | Note |
|---|---|---|
| api-001 | confirmed | Base `routes/time.rs` takes only RFC3339 or relative input; its error text lists only those two. Target `0x6a08c68` has `%Y-%m-%d` beside `routes\time.rs`, `0x6a08d13` has the new 3-family error text, `0x6a3fc6e` the export doc. |
| api-002 | confirmed | `routes\response_format.rs` is a new module (not in base). `text/csv`, `text/tab-separated-values` and the unknown-format error are absent from base. Offsets check out. |
| api-003 | confirmed | Base `SearchQuery` lacks `starred_session_id, starred_only, input_context_only, frame_id, actor_id, include_related, fields, format`. Target serde list `0x52ca6b8`, `SearchContentType` (`…memory parsed`) `0x6a303a0`, `ParsedContent` `0x6a3d348`, `ParsedItem` `0x6a3d150` all present. Overlaps api-002 (format/fields) and api-038 (pipe rules) by cross-reference only. |
| api-004 | confirmed | Correction: base `/elements` already has `limit` (default 50) and `offset`. They are new only for `/frames/:id/elements`, whose base `FrameElementsQuery` takes only `source`. State flags (`0x6a1f4fb`), `is_password…` vocabulary (`0x6a1ea0a`) and outline headers (`0x6a1f760`) are verified and absent from base `ElementResponse`. |
| api-005 | confirmed | Base `ActivitySummaryQuery` has only `include_recording/memories/snippets/guidance` + caps. Base response has no `starred_sessions*`, `parsed_context_count`, or attribution. Target `0x6a0c5b8`, `0x6a0c748`, and `frame_fallback` CTE `0x6a1a2c0` verified. |
| api-006 | confirmed | Handler `routes::activity_ledger::get_activity_ledger` `0x6a3ad98`; 31-day error `0x6208434`; field list `0x6208570`. Called from `(main)/home` and `layout` chunks. Absent from base. |
| api-007 | confirmed | `routes::starred::{save,list}` `0x6a7aff0`/`0x6a7b0c8`, `SaveSession` `0x5183990`, conflict text `0x5183bb8`, range rule `0x6a7b298`. No `starred` identifiers in base. |
| api-008 | confirmed | oasgen blob `0x5257ca8` begins with the six `/semantic/actors/*` paths; request structs `0x522dab0`. Absent from base. |
| api-009 | confirmed | `content::autocomplete_tags` `0x6a3dfbc`. Minor citation slip: `chunks/2037-*.js` renders the `frame_count/audio_count/memory_count` fields but does not call `/tags/autocomplete`; the call sites are `3696-*.js`, `layout-*.js`, `(main)/home` and `(main)/settings`. |
| api-010 | confirmed | Thumbnail headers `0x6a1cfd8`, preview-media doc `0x6a1dde0`, limit validation `0x6a1e951`. Frontend `3308-*.js` (`/thumbnail?…&fallback=false`, `/frames/preview-media/`). The base `/thumbnail` hit is a false positive (`./thumbnail-highlight-overlay` import). |
| api-011 | confirmed | Upgraded from *inferred* to *observed*. Base `server.rs:1069` has `path.starts_with("/frames/")`. In the target, the auth-exemption closure (code `0x286c800`–`0x286c990`, SSE fragment refs to `0x6a91c00`–`0x6a91c40`) inlines tests for `/health`, `/notify`, `/ws/health`, the `/mcp-servers/` prefix with `/oauth/callback` suffix, `/pipes/s…`, `/connections/oauth/callback`, and `/connections/browser/pair/{start,status}`. No `/fra…` or `…ames/` bytes occur anywhere in it. |
| api-012 | confirmed | New: `routes\request_origin.rs` (`x-screenpipe-client/agent` `0x5a5eb30`), `routes\internal_telemetry.rs` (`0x5a51b20`), `/internal/telemetry/*` literals `0x52585a8`/`0x52585e0`, and `mcp_call.rs` `McpCall` with client-id vocabulary `0x6a9aca8`. Correction: `x-screenpipe-session` is not new. Base `connections_api.rs:2343/2561` already reads it for browser-navigate ownership, and base `bash_env.rs` sets it. |
| api-013 | confirmed | Base `SearchSpeakersRequest{name}` and `ReassignSpeakerRequest{audio_chunk_id,new_speaker_name,propagate_similar}`. Target adds `include_samples` and `scope` (`0x5153508`; "ReassignSpeakerRequest with 4 elements" `0x5155c2f`), response fields `0x51535a8`, `/speakers/sample/:audio_chunk_id` `0x5258848`, wav/cache `0x5153810`. |
| api-014 | confirmed | Base `StartMeetingRequest{id,app,title,attendees}` (no `calendar_event_id`) and `ExportRequest` (no `include_audio`). `resolve_room_change_handler` `0x6a5c370`; request/summary type names `0x6a431c8`; summary-status path in blob `0x5257fbe`. |
| api-015 | confirmed | `/ws/meeting-overlay` `0x5259250` (axum literal block), `MeetingOverlayMessage` `0x6189a20`, `meeting_transcript_delta/final` `0x512e970`, `mic_capture_active` `0x512e850`. Frontend `shortcut-reminder/page-a5341a3196435d05.js`. Absent from base. |
| api-016 | confirmed | `routes\vision.rs` is new (`0x59fb855`); `VisionDeviceStatusEntry` `0x6a7236d`. Base `user_disabled` is audio-only (`audio_manager`). |
| api-017 | confirmed | Docs `0x6a7c9c8`, `.saf.json` validation `0x6a7cc8b`, list fields `0x6189418`. Absent from base. |
| api-018 | confirmed | `routes\ai_feedback.rs` is new; `FeedbackTarget` fields `0x5183483`; skill §14 `0x511d81d`. Absent from base. |
| api-019 | confirmed | `routes\structured_outputs.rs` `0x522d0ff`; store `structured-outputs`/`targets.json` `0x6a23bb8`; four literals at `0x5258618`–`0x52586c0`. Absent from base. |
| api-020 | confirmed | Literals `0x5258750`–`0x5258818`; `live-view-template.v1` / `live-view-kit.v1`; store `templates.json` `0x6a84e8b`. Absent from base. |
| api-021 | confirmed | Literals `0x52584e0`–`0x5258570`; `ManageAgentSkillRequest` `0x6221d58`; `ManageProfileRequest` `0x6222288`. Absent from base. |
| api-022 | confirmed | `routes\connect_broker.rs` new; `acp_connect_request/response` `0x5255a61`/`0x6a6aba0`. Absent from base. |
| api-023 | confirmed | `local_chat.rs` new (`0x5253b30`); preset errors `0x5253bdd`; Ollama/OpenAI defaults `0x6a64508`. Absent from base. |
| api-024 | already_in_base | Not a v2.7.104 delta. Base `routes/cloud_proxy.rs` already has `CLOUD_BASE_URL = "https://api.screenpipe.com"`, the same route and the same 50 MB cap. Target is unchanged (`0x5183758`). Decloud is a pre-existing base obligation; keep it on the decloud list, not the delta list. |
| api-025 | confirmed | The engine route is new (`routes\web_search.rs`, literal `0x5258ae8`, `cloud_token_missing` `0x6188f7b`). Note: base already ships `crates/screenpipe-core/assets/extensions/web-search.ts`, which calls `https://api.screenpipe.com/v1/web-search` directly. Freenpipe's decloud must cover that base extension too. |
| api-026 | confirmed | `routes\data_sync_proxy.rs` new; `/api/user/data-sync/` `0x6189042`; `SCREENPIPE_WEB_URL` `0x6a9e4a8`. Base `screenpipe-sync` "data-sync" is an unrelated generic upload library. |
| api-027 | confirmed | `routes\cloud_agents.rs` new; `api.cursor.com` `0x522a0c0`; `cloud_agents.cursor.api_key` `0x522a1de`. Absent from base. |
| api-028 | confirmed | Five `routes\workflow*.rs` modules new; `workflow_catalog::correct` `0x6a6aeb8`; 117 `/workflows/*` hits. Base has only `workflow_classifier.rs` (no routes). |
| api-029 | confirmed | Base `ListPipesQuery{include_executions}`, `ExecutionsQuery{limit}`, `RunPipeBody{notification_context}`; no `error_code`. Literals `0x5258c20`–`0x525904e`; error codes `0x6a6d648`. |
| api-030 | confirmed | `/data/compact`, `/sync/reset-account`, `/archive/download(/status)` are absent from base. Base `vault_lock` locks online with no 409; the target refusal text is at `0x522d6fa`. |
| api-031 | confirmed | The literal table `0x51826a8`–`0x5182e00` matches §2.4 exactly, with no gmail. Base has `/connections/gmail/*` (`connections_api.rs:811+`). The gmail removal is restated in api-036. |
| api-032 | confirmed | `/registry` `0x6188a00`, registry URL `0x6188550`. Absent from base. |
| api-033 | confirmed | Correction: `pool_stats`, `vision_db_write_stalled`, `audio_db_write_stalled`, `drm_content_paused`, `schedule_paused`, `hostname`, `version` are already in base `HealthCheckResponse`. Genuinely new: `dependencies, runtime, startup, unhealthy_reasons, vision_reason, loop_stage(_age_secs), audio_capture_mode, capture_status, recording_coverage, write_queue_*, write_pool_reopens, persistent_failure_signals`, the extra pipeline counters, and the cached-snapshot budget (`0x52565b0`). |
| api-034 | confirmed | Strengthened and corrected. The error code `search_only` is observed: an 11-byte String is built at code `0x1a236d9` (`search_o` + `only`), next to the message assembly at `0x1a2382a`–`0x1a23860`, near the auth middleware. Correction: the 16-byte table at `0x6a84320` is not shown to be the search-only allow-list. Its only users are compares at `0x282e1d5`–`0x282e459`, inside a function that also compares inlined `/vault/lock`, `/vault/unlock`, `/raw_sql`, `/openapi.json`, `/openapi.yaml`, `/vision/status`, `/audio/metrics`, `/capture/hd`, `/artifacts`, `/elements`, `/search/keyword`, `/vision/metrics`, `/speakers…`, `/meetings…`, `/memories…`. Nearby refs to the `search/retrieved/meeting/artifact/…/direct_api` vocabulary (`0x6a8dd8a`) suggest a request-origin / qualified-value route classifier (api-012). The gated route set is still unresolved, so treat the read/write split as a design decision. |
| api-035 | confirmed | `history_access_limited` and the 24-hour texts are present (`0x6a1cbd8`, `0x6a5dd2e`, `0x620a240`, `0x59fbb50`); `history_access.rs` is a new module. Absent from base. ee-stub: do not implement. |
| api-036 | unsupported | Refuted for openapi. The "0 literals" evidence cannot see code-inlined strings. At code `0x10a2909`/`0x10a2a01` the target allocates 13 bytes and stores `/openapi` at +0 and `api.yaml` / `api.json` at +5, which builds `/openapi.yaml` and `/openapi.json` (the oasgen spec-route pattern; base `server.rs:731-732`). Both paths are compared again at `0x282e21e`–`0x282e248`. The routes are retained; they are base-present and unchanged, not removed. §2.1 row "R" and the path counts ("2 removed") are wrong. The gmail half holds (the extension source `0x52a6a06` says Gmail has no `/connections` entry) but duplicates api-031. |
| api-037 | confirmed | Base app-server routes are `/notify, /notifications, /notifications/:id, /inbox, /log, /auth, /app-icon, /installed-apps, /window-size, /focus`. `/notifications/:id/read` (`0x59f6268`) is new. Base `NotifyPayload` = `{title, body, id, type, autoDismissMs, timeout, actions}`; target `0x52cadd8` adds the listed fields. |
| api-038 | confirmed | Base middleware already has endpoint gating, the audio/speaker denial and the 30 s 3-permit semaphore. New: the `workflow operation` lane (`0x6188e32`), the explicit-`content_type` error (`0x6a3ba0a`+), engine-side `access to app/window "…"` 403s (`0x6a3bab2`/`0x6a3bae5`) and the `include_related` refusal (`0x6a3bb1a`). Base has app denial only client-side, in `assets/extensions/screenpipe-permissions.ts`. |
| api-039 | partially_in_base | Base `retranscribe.rs:489/511` already refuses to retranscribe a meeting that is still running and already caps one pass at 2 h. Base CLI already supports `whisper-large-v3-turbo-quantized`, `openai-compatible` and `qwen3-asr` (`cli/mod.rs:50-88`). Genuinely new: `MeetingRetranscribeResponse.transcript_updated_at` and `.summary_refresh_event_emitted` (base struct ends at `replaced_segments`), plus the post-retranscribe summary-refresh request event. |
| api-040 | already_in_base | The item says so itself. Base has `privacy_filter.rs` with `SCREENPIPE_PRIVACY_FILTER_ENCLAVE` and the `screenpipe-redact` tinfoil adapters, and `routes/pipe_store.rs` with `/api/pipes/store`. Not a v2.7.104 delta; it is a pre-existing decloud obligation. |

### Corrections to apply in the body of this spec
- §2.1: `/openapi.yaml` and `/openapi.json` are **B** (retained), not **R**. Engine oasgen counts become target 119 vs base 80 → 39 new, 0 removed.
- §1.3 / api-012: `x-screenpipe-session` is base-present. Only `X-Screenpipe-Client` / `X-Screenpipe-Agent` are new.
- api-004: `limit`/`offset` are new only on `/frames/:id/elements`.
- api-033: drop `pool_stats`/stall/DRM/schedule flags from the "new" list.
- api-034: code `search_only` is observed; drop the `0x6a84320` allow-list hypothesis.
- api-039: the 2 h cap, the must-be-stopped rule and the engine list are base behaviour.

### Independent sweep — results
- All 20 new `screenpipe-engine\src\routes\*.rs` modules in the target map to an existing item (`vapi_srcmods.py`). So do the non-route `search_only.rs`, `history_access.rs`, `local_chat.rs`, `pipe_stream.rs`, `mcp_call.rs`, `live_views.rs`, `agent_profile.rs` and `agent_skills.rs`.
- The axum literal block (`0x52584e0`–`0x5259328`), the `/connections` block (`0x51826a8`–`0x5182e00`), the `/mcp-servers` block (`0x6188a00`+) and the app-server block (`0x59f61b0`–`0x59f6370`) match §2.2–§2.6 exactly.
- Frontend: no API call uses an unknown first segment. The only hit was `/envelope`, which is the Sentry SDK. Known-segment paths not literally in this spec are all base routes written here in brace or `:id` form.
- One out-of-dimension pointer: `(main)/home/page-c18385b042b3382e.js` holds a dev mock (`?cloudWorkflowState=`) of a cloud team-workflow API (`…/procedure` GET/PUT with `expected_revision`, `…/chat`, `…/chat/turn`) plus `/api/team/billing/{add-seat,seat-invite}`. These are screenpipe.com team features for the cloud/ee dimension, not engine API.

### Missed items (API dimension)
- **api-M01 — MCP OAuth relayed through screenpi.pe (decloud, S).** `POST /mcp-servers/:id/oauth/start` now takes `OAuthStartBody {app_scheme}` (`0x6188960`). `app_scheme` must be `screenpipe` or `screenpipe-enterprise` (`0x6a8dd01`). The redirect URI can be the cloud broker `https://screenpi.pe/api/oauth/mcp/<id>/callback?app_scheme=<scheme>` (`0x6a8dd37`/`0x6a8dd5c`), which bounces back to the app deep link, instead of `http://localhost:3030/mcp-servers/<id>/oauth/callback` (`0x6a8dc90`). Base has neither `app_scheme` nor `api/oauth/mcp`. Freenpipe: accept and ignore `app_scheme` and always use the localhost callback.
- **api-M02 — Per-pipe-session MCP server gate (new, S).** MCP server use is refused with an error naming the server id and stating that the current pipe session may not use it (`0x6a8dc5b`). The session is identified by the `x-screenpipe-session` header (§1.3 mentions this gate but no item specifies it). This applies to `/mcp-servers/:id/{tools,call}` for pipe or agent sessions whose allow-list excludes that server. Absent from base. Use 403.
- **api-M03 — Composio-managed connections via a screenpipe-hosted MCP (decloud, S; connections/agent dimension).** The agent extension treats an MCP server whose URL matches `https://(www.)screenpipe.com|screenpi.pe/api/composio/mcp` as the provider for Gmail, Zoom and Google Drive/Docs/Sheets, which have no `/connections` row (`0x52a6da7`, `0x52a6a06`, `0xa0cccad`). This is how the removed `/connections/gmail/*` is replaced in the target. Base has no `composio` identifiers. Freenpipe: do not ship that URL. Gmail stays on the base routes or IMAP.

