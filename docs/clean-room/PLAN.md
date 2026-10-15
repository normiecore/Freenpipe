# Freenpipe implementation plan: BASE 892199f to parity with screenpipe v2.7.104

First generated 2026-10-15 from the verified delta in `docs/clean-room/specs/*.md` by `python3 tools/plan/plan_build.py` (data in `tools/plan/plan_data.py`, `plan_head.md` and `plan_tail.md`). Edit those files and regenerate rather than editing this file by hand. Spec paths written `specs/<name>.md` below refer to `docs/clean-room/specs/<name>.md`.

## 1. Inputs and scope

- **BASE:** `<repo>` at upstream 892199f (about v2.5.27, last MIT commit, `ee/` removed). It is untouched: the three `#[path]` shims still point into the missing `ee/`, and branding, telemetry, updater and gates are still vendor.
- **TARGET:** screenpipe v2.7.104 Windows build, known only through static evidence in `<re-workspace>` (strings, crates list, extracted frontend, REA summaries). Never execute it.
- **Delta:** 13 dimension specs. The verifiers kept every item marked confirmed, partially_in_base, gap-unverified or verifier-found as live; items marked already_in_base, unsupported or duplicate are dropped, and section 9 records where their remaining notes land.
- **Shape of the plan:** wave 0 is the foundation (build, `ee/` stubs, de-cloud, rebrand). Wave 1 is cross-cutting foundations: the full 117-migration schema, the settings schema, and integration seams. Waves 2 to 8 are features ordered schema, then engine API, then app backend, then UI. WPs in one wave run in parallel. Wave 0 has an internal order: W0-01 first, W0-02..W0-11 in parallel, W0-12 last. Every WP lists its depends_on explicitly.

## 2. Ground rules for every WP

1. **Clean room (binding, see `Freenpipe/CLAUDE.md`).** Work only from `specs/*.md`, the local RE artifacts and BASE source. Do not consult post-relicense upstream material, browse vendor sites, or execute the target. Write original code. Do not paste decompiled code, extracted frontend JS, UI copy paragraphs or LLM prompt text; paraphrase behaviour. Interoperability identifiers (routes, JSON fields, SQL DDL and names, CLI flags, settings keys, env vars, file names) may match exactly. Built-in pipes, starter skills, the meeting-summary prompt, activity-history prompts and workflow agent prompts must be written fresh.
2. **No phoning home.** No vendor host, PostHog key, Sentry DSN or vendor OAuth client id may appear in request-building code. All new network features are user-configured (BYO) or third-party and user-initiated. The vendor guard test (W0-01 scaffolds it, W0-12 makes it fail the build) enforces this.
3. **New file header.** Every new .rs/.ts/.tsx/.js/.py file starts with the two-line Freenpipe header from CLAUDE.md. Upstream headers on edited files stay.
4. **Dev isolation.** screenpipe v2.7.104 is installed on the dev machine and may be running. It owns port 3030, the :11435 control server, `~/.screenpipe` and the `screenpipe://` scheme. Rules:
   - Never point a Freenpipe build at `~/.screenpipe`. Use `FP_DD` (section 4).
   - Never use port 3030 for dev engines. Use 3035.
   - Set `SCREENPIPE_API_KEY`, so the engine does not read or delete `~/.screenpipe/auth.json` (base `auth_key.rs` falls back to it).
   - Before any `bun run tauri dev`, quit the installed screenpipe app, because the base app control server binds :11435 unconditionally. W0-01 also removes the base blind `taskkill` of other port holders, which would otherwise kill the installed screenpipe engine.
   - Never use the owner's real recordings or databases as test fixtures. Build synthetic fixtures.
5. **Parallel agents share the tree.** Commit only your own files. Never `git reset --hard`, `git clean`, `git checkout -- .` or `git stash`. Edit only your owner paths and regions (section 6). For files outside them, coordinate through the WP that owns them.
6. **Generated files.** Regenerate `apps/screenpipe-app-tauri/lib/utils/tauri.ts` with `bun run bindings:generate`, then merge-resolve by regenerating again; never hand-merge it. The same applies to `Cargo.lock` (`cargo update -w`) and `bun.lock` (`bun install`). Tauri commands need both `#[tauri::command]` and `#[specta::specta]`; tauri-helper auto-collects them at build time, so no central list needs editing.
7. **Settings keys are owned by W1-02.** Feature WPs read the keys W1-02 adds and do not add their own. If a WP truly needs a missing key, add it in a dedicated small commit touching only the settings files and record it in the WP notes.
8. **Path moves.** After W4-22, `app/home` and `app/settings` live under `apps/screenpipe-app-tauri/app/(main)/`. Owner paths in later waves already use the new location.

## 3. Build environment status (as of 2026-10-15)

- Toolchain: Rust 1.93.1 (`rust-toolchain.toml`), MSVC 2022 BuildTools 14.44 + Windows SDK 10.0.26100, LLVM (for `LIBCLANG_PATH`), CMake, bun. `source <re-workspace>/tools/devenv.sh` exports the vcvars environment for Git Bash.
- `cargo build -p screenpipe-engine --profile release-dev` **succeeds** (`build-logs/engine-build-6.log`, about 7 minutes).
- Prerequisites:
  - User env `CMAKE_GENERATOR=Visual Studio 17 2022` is required. An empty "VS 18" BuildTools shell otherwise makes CMake pick the wrong generator. If CMake reports "Does not match the generator used previously", delete `target/*/build/<crate>-*/out/build`.
  - Run `bun scripts/pre_build.js` in `apps/screenpipe-app-tauri` first. It fetches the ffmpeg, OpenBLAS and bun sidecars. Its ffmpeg URL already points at the GyanD GitHub mirror.
  - `OPENBLAS_PATH=src-tauri/openblas` is needed by antirez-asr-sys.
- Known blockers already hit:
  - `libsamplerate-sys`, `antirez-asr-sys` and `whisper-rs-sys` fail without the vcvars environment. bindgen reports `stdio.h` not found unless INCLUDE and LIBCLANG are set.
  - The **debug** profile fails to link: knf-rs-sys is built with the debug CRT (`MDd`/`_ITERATOR_DEBUG_LEVEL=2`) while whisper-rs-sys uses the release CRT (`LNK2038`, `LNK1120`). Use `--profile release-dev`, or have W0-01 force a single MSVC runtime (`CMAKE_MSVC_RUNTIME_LIBRARY` / `CFLAGS`) for the C++ sys crates.
- `bun install` in the app succeeded (1394 packages). The Tauri app crate has **not** been built yet, and it cannot build until W0-01 stubs `ee/`.
- Sidecars present in `src-tauri/`: bun, ffmpeg, onnxruntime 1.22 (CPU), openblas. Vulkan SDK and DirectML are not yet installed or bundled; W2-19 needs them.

## 4. Acceptance harness

Every WP's acceptance checks assume this shell setup (Git Bash). Keep commands short: Git Bash on this machine fails under heavy pipelines.

```bash
source <re-workspace>/tools/devenv.sh
export FP_ROOT=<repo>
export FP_DD="$LOCALAPPDATA/freenpipe-dev"     # never ~/.screenpipe
export FP_PORT=3035                              # never 3030 on this machine
export SCREENPIPE_API_KEY=fp-dev-key             # keeps the engine off ~/.screenpipe/auth.json
cd "$FP_ROOT" && cargo build -p screenpipe-engine --profile release-dev
"$FP_ROOT/target/release-dev/screenpipe.exe" record --port $FP_PORT --data-dir "$FP_DD" --disable-telemetry > "$FP_DD/engine.log" 2>&1 &
api()  { local p="$1"; shift; curl -s -H "Authorization: Bearer $SCREENPIPE_API_KEY" "$@" "http://127.0.0.1:$FP_PORT$p"; }
sqlq() { python -I -c "import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);[print(r) for r in c.execute(sys.argv[2])]" "$FP_DD/db.sqlite" "$1"; }
```

- Rust: `cargo test -p <crate> [--test <name>] --profile release-dev` (until the debug CRT issue is fixed).
- App: `cd apps/screenpipe-app-tauri && bun test && bun run bindings:check && bun run build`. UI smoke is `bun run tauri dev` with the installed screenpipe app quit, the same isolated data dir (`SCREENPIPE_DATA_DIR="$FP_DD"`) and the app's port setting at 3035.
- SQL schema checks use `sqlq` or a `cargo test` in `crates/screenpipe-db/tests`.
- A WP is done when its acceptance boxes pass, the engine and app still build, `cargo test` for the crates it touched passes, and the vendor guard passes.

## 5. Integration seams (built by W1-03) and conventions

These seams keep parallel WPs in their own files:

- **Engine routes:** `crates/screenpipe-engine/src/routes/ext.rs` mounts one stub module per planned feature. Each stub exposes an oasgen registration function and an axum router that both start empty. The stubs are starred, semantic, artifacts, structured_outputs, live_views, ai_feedback, activity_ledger, vision, workflows, agent, connect_broker, local_chat, web_search, data_sync_proxy, cloud_agents, internal_telemetry, request_origin, response_format, search_only, pipe_stream and meeting_overlay. Middleware for request origin and search-only is pre-wired as pass-through. A feature WP fills its stub and never edits `server.rs`, except for the small regions listed in section 6.
- **DB:** `crates/screenpipe-db/src/ext/<feature>.rs` holds an empty `impl DatabaseManager {}` for each of outputs, feedback, starred, semantic, activity, display_layout, pipe_events, frames_preview, meetings_v2, tags and retention_strip. New DB functions go there. Existing functions are modified in place in `db.rs` only inside the WP's declared region.
- **Connections:** `crates/screenpipe-engine/src/connections_routes/` gets one file per new integration route group (webhook_proxy, telegram, slack, imap, bee, browser_act), merged into the `/connections` router at one point.
- **Pipes:** `crates/screenpipe-core/src/pipes/hooks.rs` provides run-lifecycle hooks (before run, after run, on event), so artifacts, finalizers and trajectories do not edit the 6k-line `pipes/mod.rs`.
- **Pi extensions:** data-driven bundled-extension manifests, `crates/screenpipe-core/src/agents/extensions_manifest.rs` and `src-tauri/src/pi_extensions_manifest.rs`. Adding an extension means one new asset file plus one manifest entry.
- **App modules:** W1-03 pre-declares `mod` lines and empty files in `src-tauri/src/` for every planned app module, for example activity_history, first_run_summary, brain_views, workflow_video, workflows_media, coding_workspace, chat_control, storage_migration, db_self_heal, overlay_health, port_conflict, disk_pressure, power_awake, search_only and calendar/reminders.
- **Background workers:** a registry in `crates/screenpipe-engine/src/core.rs`. Workers register there instead of editing the startup sequence.
- **Shared constants:** data-dir file names in `crates/screenpipe-core/src/paths.rs` and `SCREENPIPE_*` env names in `crates/screenpipe-core/src/env.rs`.
- **Migrations:** W1-01 adds all 28 target migrations at once under the exact target versions, with DDL as recorded in db.md. Later WPs never add migrations. A WP that finds a schema gap stops and raises it, because a Freenpipe-only migration would break checksum interop.

## 6. Shared hotspot files and region ownership

Several large files are edited by more than one WP in the same wave. Each WP edits only its region, and merges resolve hunk by hunk.

| File | Lines | Same-wave sharers (region) |
|---|---|---|
| `crates/screenpipe-db/src/db.rs` | 11.4k | W2-01 (DatabaseManager::new, checkpoint, heavy reads), W2-08 (meetings rejoin), W2-16 (speaker functions), W2-17 (meeting transcript + timeline audio); W3-02 (search), W3-03 (elements), W3-09 (retention/evict/delete), W3-16 (speaker embedding + dedup), W3-27 (fault hook) |
| `apps/.../components/standalone-chat.tsx` | 9.7k | serialised: W4-05 (one call), W5-15, W6-01, W7-05/W7-06/W7-07 (composer region vs one mount line each), W8-01 |
| `crates/screenpipe-core/src/pipes/mod.rs` | 6.2k | one WP per wave: W1-03 (hook call sites), W2-22, W3-06, W4-02, W5-01, W6-02/W6-03 (run_in region vs one subscription line), W7-08 |
| `crates/screenpipe-engine/src/meeting_detector.rs` | 6.1k | W3-08 only; W4-12 works in new `meeting_watcher/audio_process/` |
| `apps/.../src-tauri/src/pi.rs` | 3.6k | W2-23 (runtime), W4-03/W4-05 (new file vs pool region), W5-12 (hook), W6-02/W6-09 (stream-to-chat region vs env line) |
| `crates/screenpipe-engine/src/connections_api.rs` | 3.5k | W2-20 (GET handlers), W4-17 (gcal handler) vs W4-18 (browser snapshot handler); new routes go in `connections_routes/` |
| `crates/screenpipe-engine/src/event_driven_capture.rs` | 2.8k | one WP per wave: W3-11, W4-14, W5-05 |
| `apps/.../lib/hooks/use-settings.tsx` | 1.3k | W0-01 (imports), W0-04 (user/login block), W0-06 (defaults/migrations); then W1-02 only; W6-01 (AIPreset type) |
| `apps/.../src-tauri/src/store.rs` | 1.9k | W0-04 (entitlement/User schema) vs W0-06 (preset/STT defaults); W1-02; W6-01 (AIPreset) |
| `apps/.../src-tauri/src/main.rs` | 2.0k | W0-02 (Sentry/PostHog init) vs W0-04 (auto-start guard); W1-02 (store init) vs W1-03 (mod lines); W2-26 (exit) vs W2-27 (argv/startup); later only argv-table lines |
| `apps/.../src-tauri/src/server.rs` (:11435) | - | W2-26 (port kill) vs W2-27 (focus, Origin/Host); W3-18 (route table) |
| `crates/screenpipe-engine/src/server.rs` | 1.2k | W1-03; W2-10 (auth exemption list) vs W2-13 (health ticker) |
| `crates/screenpipe-connect/src/connections/mod.rs` | - | W2-20 (render_context, flags) vs W2-21 (registry lines); W3-20 vs W3-22 (registry lines) |
| `apps/.../components/settings/connections-section.tsx` | 3.8k | W0-07 (BYO client hook-up) vs W0-09 (MCP snippet); W3-19 (Slack panel) vs W3-21 (MCP tiles); W5-23 |
| `apps/.../app/(main)/home/page.tsx` | 1.5k | W5-16; W6-07, W6-13, W6-14 (one mount region each) |
| `crates/screenpipe-audio/Cargo.toml` | - | W2-18 (windows features) vs W2-19 (vulkan/directml features) |
| `crates/screenpipe-capture/src/paired_capture.rs` | 1.1k | W1-01; W3-12 (OCR call); W4-14 (walker call); W5-06 (OCR gate) |

## 7. Wave overview

| Wave | Theme | WPs | Sizes |
|---|---|---|---|
| 0 | Foundation: build, ee/ stubs, de-cloud, rebrand | 13 | 6 L, 7 M |
| 1 | Cross-cutting foundations: schema v2.7, settings schema, integration seams | 4 | 2 L, 1 M, 1 S |
| 2 | Engine data layer and core backends | 27 | 21 L, 5 M, 1 XL |
| 3 | Feature backends (search, pipes, connections, capture, audio) | 31 | 25 L, 6 M |
| 4 | Advanced backends, agents runtime, app shell | 22 | 14 L, 3 M, 5 XL |
| 5 | Feature UIs and second-tier backends | 24 | 19 L, 4 M, 1 XL |
| 6 | Agent/workflow UIs, piggyback, hybrid bulk | 21 | 12 L, 3 M, 6 XL |
| 7 | Hybrid lifecycle, overlays, chat follow-ups | 9 | 6 L, 1 M, 2 XL |
| 8 | Final UI pieces and surface audit | 3 | 2 L, 1 XL |
| total | | 154 | 107 L, 30 M, 1 S, 16 XL |

Coverage: 412 live delta items, each in exactly one WP (checked by `python3 tools/plan/plan_build.py`); 38 verifier-dropped items are mapped in section 9.

## 8. Work packages

### Wave 0: Foundation: build, ee/ stubs, de-cloud, rebrand

| WP | Size | Title | Depends on | Items |
|---|---|---|---|---|
| W0-01 | L | Build bootstrap, ee/ stubs, endpoint switchboard and dev isolation | - | cloud-001, cloud-002, cloud-003, cloud-004, cloud-005, cloud-009 |
| W0-02 | M | Remove telemetry and crash reporting (PostHog, Sentry) | W0-01 | cloud-010, cloud-011, gap_health-lifecycle-internals-010 |
| W0-03 | M | Updater on GitHub releases, release pipeline de-vendoring, restart_for_update | W0-01 | cloud-012, cloud-041, cloud-V02 |
| W0-04 | L | Remove entitlement, login and plan gates | W0-01 | cloud-013, cloud-014, cloud-015, cloud-030, cloud-031, cloud-032 |
| W0-05 | L | Remove commerce, team and vendor UI surfaces | W0-01 | cloud-022, cloud-023, cloud-039, cloud-044, frontend-051 |
| W0-06 | L | AI gateway, transcription, PII backend and media analysis to bring-your-own | W0-01 | cloud-016, cloud-017, cloud-020, cloud-042, cloud-M01, agents_pipes-036 |
| W0-07 | L | OAuth broker replaced by BYO client credentials | W0-01 | cloud-018 |
| W0-08 | M | Cloud sync, archive and pipe store off by default | W0-01 | cloud-019, cloud-021, agents_pipes-037 |
| W0-09 | M | Bundled CLI/MCP runtime and self-hosted model sources | W0-01 | cloud-024, cloud-046, cloud-M02, agents_pipes-040 |
| W0-10 | M | Logs and feedback upload replaced by a local diagnostics bundle | W0-01 | cloud-036, frontend-046, meetings_memory-009 |
| W0-11 | L | SQLite toolchain upgrade (sqlx 0.9, libsqlite3-sys 0.37, rusqlite vtab) | W0-01 | db-029 |
| W0-12 | M | Rebrand user-visible identity to Freenpipe | W0-02, W0-03, W0-04, W0-05, W0-06, W0-07, W0-08, W0-09, W0-10, W0-11 | (no delta id) rebrand: product name, identifier, deep-link scheme, tray, icons, window titles, installer |
| W0-13 | M | Vendor git dependencies into third_party/ | W0-11 | (owner decision 2) vendor git deps |

#### W0-01 Build bootstrap, ee/ stubs, endpoint switchboard and dev isolation (L)

- **Items:** cloud-001, cloud-002, cloud-003, cloud-004, cloud-005, cloud-009
- **Depends on:** none
- **Specs:** specs/cloud.md sections 4-5, cloud-001..005, cloud-009

Make the untouched BASE compile on Windows with ee/ absent: write original stubs for the three #[path] shims, drop the enterprise-build feature, add a local app-update-policy.ts and remove the @ee aliases and ee CI jobs. Document the MSVC/LLVM/CMake environment (<re-workspace>/tools/devenv.sh) and the debug-profile CRT mismatch (knf-rs-sys vs whisper-rs-sys; use the release-dev profile or force one MSVC runtime). Add the empty-by-default endpoint switchboard (Rust endpoints.rs + TS web-url.ts returning null) that every later de-cloud WP routes through, strip the vendor gateway hosts from capabilities and the UA, and scaffold a vendor-host guard test in report mode. Dev isolation before any app run: the owner machine runs screenpipe v2.7.104, so neutralise the base blind taskkill of foreign port holders (log and refuse instead), make dev runs use a separate data dir, port and SCREENPIPE_API_KEY, and audit hard-coded ~/.screenpipe paths (auth.json) that bypass --data-dir.

Owner paths:
- `Cargo.toml (workspace exclude list)`
- `apps/screenpipe-app-tauri/src-tauri/Cargo.toml ([features], optional deps)`
- `apps/screenpipe-app-tauri/src-tauri/src/enterprise_policy.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/enterprise_install_metadata.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/enterprise_sync.rs`
- `apps/screenpipe-app-tauri/src-tauri/tests/enterprise_sync_test.rs (delete)`
- `apps/screenpipe-app-tauri/lib/app-update-policy.ts (new)`
- `apps/screenpipe-app-tauri/tsconfig.json`
- `apps/screenpipe-app-tauri/vitest.config.ts`
- `apps/screenpipe-app-tauri/components/settings/general-settings.tsx (import lines only)`
- `apps/screenpipe-app-tauri/lib/hooks/use-enterprise-policy.ts (import lines only)`
- `apps/screenpipe-app-tauri/lib/hooks/use-settings.tsx (import lines only)`
- `.github/workflows/sdk.yml`
- `.github/workflows/sdk-release.yml`
- `.github/workflows/release-enterprise.yml`
- `.github/workflows/sentry-cleanup.yml`
- `.gitignore`
- `README.md (ee/sdk link only)`
- `crates/screenpipe-core/src/endpoints.rs (new)`
- `crates/screenpipe-core/src/lib.rs`
- `apps/screenpipe-app-tauri/lib/web-url.ts`
- `apps/screenpipe-app-tauri/src-tauri/capabilities/main.json (remote.urls)`
- `apps/screenpipe-app-tauri/src-tauri/src/recording.rs (blind port-kill block only)`
- `apps/screenpipe-app-tauri/src-tauri/src/server.rs (blind port-kill block only)`
- `docs/dev/windows-build.md (new)`
- `scripts/dev/ (new)`
- `scripts/ci/vendor-guard.test.ts (new)`

Acceptance:
- [ ] source <re-workspace>/tools/devenv.sh && cargo build -p screenpipe-engine --profile release-dev exits 0
- [ ] cargo build --manifest-path apps/screenpipe-app-tauri/src-tauri/Cargo.toml --profile release-dev exits 0 with no ee/ directory; grep -rn 'ee/desktop' apps Cargo.toml returns nothing
- [ ] cd apps/screenpipe-app-tauri && bun install && bun run build && bun run bindings:check all pass
- [ ] Harness smoke (PLAN.md section 4): engine started on port 3035 with FP_DD answers `api /health` with JSON; ~/.screenpipe is not touched (mtime of ~/.screenpipe/auth.json unchanged)
- [ ] bun test scripts/ci/vendor-guard.test.ts runs and prints the remaining vendor-host hits (report mode)

#### W0-02 Remove telemetry and crash reporting (PostHog, Sentry) (M)

- **Items:** cloud-010, cloud-011, gap_health-lifecycle-internals-010
- **Depends on:** W0-01
- **Specs:** specs/cloud.md cloud-010, cloud-011; specs/gap_health-lifecycle-internals.md item 010

Delete every PostHog key and /capture/ post (app, engine analytics, resource monitor, vision_pipeline_health) and every Sentry DSN, plugin and crate. Replace posthog-js with a no-op shim exposing the same API so call sites compile; keep local tracing and the panic log. Add is_telemetry_disabled_by_env() returning true for parity, default analytics_enabled to false in the Rust config (the TS default flip is done by W0-06, owner of use-settings.tsx defaults).

Owner paths:
- `apps/screenpipe-app-tauri/app/providers.tsx`
- `apps/screenpipe-app-tauri/lib/analytics/posthog-shim.ts (new)`
- `apps/screenpipe-app-tauri/package.json (posthog-js, @sentry/* entries only)`
- `apps/screenpipe-app-tauri/src-tauri/src/analytics.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/main.rs (sentry::init, tauri-plugin-sentry, PostHog key regions)`
- `crates/screenpipe-engine/src/analytics.rs`
- `crates/screenpipe-engine/src/resource_monitor.rs (send_to_posthog + PostHog client/key)`
- `crates/screenpipe-engine/src/bin/screenpipe-engine.rs (sentry init region)`
- `crates/screenpipe-engine/src/server.rs (vision_pipeline_health PostHog emitter)`
- `crates/screenpipe-engine/src/telemetry_context.rs`
- `crates/screenpipe-engine/src/meeting_telemetry.rs`
- `crates/screenpipe-config/src/recording.rs (analytics_enabled default)`
- `Cargo.toml / crates/screenpipe-engine/Cargo.toml / apps/screenpipe-app-tauri/src-tauri/Cargo.toml (sentry deps)`
- `apps/screenpipe-app-tauri/components/settings/recording-settings.tsx (@sentry import only)`
- `apps/screenpipe-app-tauri/components/settings/privacy-section.tsx (@sentry import only)`

Acceptance:
- [ ] grep -rnE 'phc_|sentry\.io|ingest\.us\.sentry' crates apps/screenpipe-app-tauri --include=*.rs --include=*.ts --include=*.tsx (excluding node_modules) returns nothing
- [ ] cargo tree -p screenpipe-engine -i sentry and the same for the app crate report the package is not in the graph
- [ ] Engine and app build (release-dev); bun run build passes; engine log over a 2-minute run contains no 'posthog' or 'sentry' lines

#### W0-03 Updater on GitHub releases, release pipeline de-vendoring, restart_for_update (M)

- **Items:** cloud-012, cloud-041, cloud-V02
- **Depends on:** W0-01
- **Specs:** specs/cloud.md cloud-012, cloud-041, cloud-V02; specs/shell.md shell-016 (merged)

Point the Tauri updater at github.com/normiecore/Freenpipe releases with a new minisign public key; stable reads releases/latest/download/latest.json, pre-release (beta maps to it) reads a fixed-tag manifest; drop bearer auth and the auth_required flow; rollback uses per-tag manifests with allowDowngrades. Add restart_for_update(timeout_secs) wrapping the base restart gate, writing the local update-attempt marker and update-install/update-recovery logs (refuse symlinks). In CI replace the R2 upload with a GitHub release plus a generated latest.json, delete Sentry symbol uploads, strip the npm postinstall PostHog ping, the docs analytics block and any job that pulls from the upstream repo.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/updates.rs`
- `apps/screenpipe-app-tauri/src-tauri/tauri.prod.conf.json (plugins.updater only)`
- `apps/screenpipe-app-tauri/src-tauri/tauri.beta.conf.json (plugins.updater only)`
- `apps/screenpipe-app-tauri/src-tauri/tauri.enterprise.conf.json (delete)`
- `apps/screenpipe-app-tauri/components/update-banner.tsx`
- `.github/workflows/release-app.yml`
- `.github/workflows/release-cli.yml`
- `.github/workflows/sync-skills.yml`
- `packages/cli (npm postinstall scripts)`
- `docs/mintlify/docs-mintlify-mig-tmp/docs.json (analytics block)`
- `scripts/release/make-latest-json.ts (new)`

Acceptance:
- [ ] grep -rn 'api/app-update' apps returns nothing; the updater endpoint in tauri.prod.conf.json is the GitHub latest.json URL
- [ ] cargo test (app crate) updates:: covers channel->manifest URL mapping and marker writing; restart_for_update appears in lib/utils/tauri.ts
- [ ] bun scripts/release/make-latest-json.ts on fixture .sig files emits a valid Tauri latest.json
- [ ] grep -nE 'r2|sentry|posthog' .github/workflows/release-app.yml .github/workflows/release-cli.yml returns nothing

#### W0-04 Remove entitlement, login and plan gates (L)

- **Items:** cloud-013, cloud-014, cloud-015, cloud-030, cloud-031, cloud-032
- **Depends on:** W0-01
- **Specs:** specs/cloud.md cloud-013..015, cloud-030..032; specs/shell.md shell-034 (merged)

Collapse every license/plan check to a constant-allow helper with the same name and signature (app_entitled_or_dev, require_app_entitlement, hasAppEntitlement, hasCloudEntitlement, needsAppEntitlementRefresh), remove the login UI entry points, the POST {web}/api/user call, the auth interceptor and api_key deep-link login. Keep set/get_cloud_token for local key plumbing and keep every User/entitlement field (plus the target's enterprise_account and plan fields) in the store schema so a screenpipe store.bin loads unchanged. Export SCREENPIPE_STARTUP_AUTHENTICATION_STATUS=not_required to engine and pipes. Never assign trial-activation onboarding steps; a stored trial step counts as onboarding complete.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/store.rs (entitlement helpers, User schema, onboarding trial-step region)`
- `apps/screenpipe-app-tauri/src-tauri/src/recording.rs (require_app_entitlement)`
- `apps/screenpipe-app-tauri/src-tauri/src/main.rs (auto-start guard, app-entitlement-required emit)`
- `apps/screenpipe-app-tauri/src-tauri/src/oauth.rs (Pro gate lines only)`
- `apps/screenpipe-app-tauri/src-tauri/src/commands.rs (open_login_window / cloud token region)`
- `apps/screenpipe-app-tauri/src-tauri/src/server_core.rs (startup auth env)`
- `apps/screenpipe-app-tauri/lib/auth-guard.tsx`
- `apps/screenpipe-app-tauri/lib/app-entitlement.ts`
- `apps/screenpipe-app-tauri/components/app-entitlement-gate.tsx`
- `apps/screenpipe-app-tauri/components/login-dialog.tsx`
- `apps/screenpipe-app-tauri/components/deeplink-handler.tsx (auth/api_key, purchase, subscription hosts)`
- `apps/screenpipe-app-tauri/app/onboarding/page.tsx (login step)`
- `apps/screenpipe-app-tauri/lib/hooks/use-settings.tsx (user/login fetch block only)`
- `apps/screenpipe-app-tauri/lib/utils/validation.ts`
- `apps/screenpipe-app-tauri/components/meeting-notes/calendar-connect-dialog.tsx (isPro)`
- `apps/screenpipe-app-tauri/components/settings/recording-settings.tsx ((pro) labels only)`

Acceptance:
- [ ] cargo test (app crate): app_entitled_or_dev() is true; a fixture store.bin carrying user.enterprise_account, subscription_plan=pro_ultra and has_payment_method round-trips
- [ ] bun test: hasAppEntitlement/hasCloudEntitlement return true and AppEntitlementGate renders children
- [ ] grep -rn 'app-entitlement-required\|/api/user"' apps/screenpipe-app-tauri --include=*.rs --include=*.ts* returns nothing outside tests
- [ ] UI smoke: fresh store, `bun run tauri dev` (screenpipe app quit), recording auto-starts with no sign-in prompt

#### W0-05 Remove commerce, team and vendor UI surfaces (L)

- **Items:** cloud-022, cloud-023, cloud-039, cloud-044, frontend-051
- **Depends on:** W0-01
- **Specs:** specs/cloud.md cloud-022, cloud-023, cloud-039, cloud-044; specs/frontend.md frontend-051

Delete the vendor commerce and account surfaces present in the base: referral card, team API and seats, usage/credit chip, upgrade links and /api/cloud-sync/checkout, tray plan/Upgrade rows, purchase-successful deep link, enterprise policy/heartbeat network calls, CLI team/survey, and the base daily-limit/upsell error copy. Changelog reads GitHub Releases or the bundled CHANGELOG; docs/discord/download/ideas links go to GitHub. Target-only surfaces (trial paywall, card ask, annual banner, announcements, remote control policy) are simply never ported; record that in the vendor guard allow-list.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/tray.rs (plan/upgrade rows, cal.com and download handlers)`
- `apps/screenpipe-app-tauri/components/settings/account-section.tsx`
- `apps/screenpipe-app-tauri/components/settings/usage-section.tsx`
- `apps/screenpipe-app-tauri/components/settings/team-section.tsx`
- `apps/screenpipe-app-tauri/components/settings/referral-card.tsx`
- `apps/screenpipe-app-tauri/components/changelog-dialog.tsx`
- `apps/screenpipe-app-tauri/components/settings/feedback-section.tsx (vendor links)`
- `apps/screenpipe-app-tauri/lib/hooks/use-team.ts`
- `apps/screenpipe-app-tauri/lib/hooks/use-enterprise-policy.ts (network paths)`
- `apps/screenpipe-app-tauri/lib/hooks/use-usage-status.tsx`
- `apps/screenpipe-app-tauri/lib/chat/quota-errors.ts`
- `apps/screenpipe-app-tauri/components/settings/sync-settings.tsx (checkout links)`
- `apps/screenpipe-app-tauri/components/settings/archive-settings.tsx (checkout links)`
- `apps/screenpipe-app-tauri/components/onboarding/connect-apps.tsx (checkout links)`
- `apps/screenpipe-app-tauri/components/settings/ai-presets.tsx (checkout link only)`
- `crates/screenpipe-engine/src/cli/team.rs`
- `crates/screenpipe-engine/src/cli/survey.rs`
- `crates/screenpipe-engine/src/cli_reminder.rs`
- `packages/screenpipe-mcp/src/index.ts (TEAM_API)`
- `apps/screenpipe-app-tauri/src-tauri/tauri.conf.json (purchase-successful deep link)`
- `Cargo.toml (exclude screenpipe-team-memory from default members)`

Acceptance:
- [ ] grep -rnE 'subscription/checkout|cloud-sync/checkout|api/referral|api/team|cal\.com/team' apps crates packages (non-docs) returns nothing
- [ ] bun run build and bun test pass; Settings renders without Account/Usage/Team/Referral sections (UI smoke)
- [ ] Tray menu shows no plan/upgrade row (UI smoke); `cargo build -p screenpipe-engine` succeeds without the team CLI

#### W0-06 AI gateway, transcription, PII backend and media analysis to bring-your-own (L)

- **Items:** cloud-016, cloud-017, cloud-020, cloud-042, cloud-M01, agents_pipes-036
- **Depends on:** W0-01
- **Specs:** specs/cloud.md cloud-016, 017, 020, 042, M01; specs/agents_pipes.md agents_pipes-036

Fresh installs default to an Ollama/OpenAI-compatible preset; the screenpipe-cloud provider resolves to a user-configured gateway (new get_screenpipe_ai_gateway_url returns the aiGatewayUrl setting or SCREENPIPE_AI_GATEWAY_URL, else 'not configured') and is migrated once to the BYO preset when none is set. /v1/chat/completions forwards to that gateway or returns 503 cloud_token_missing. Remove the /v1/usage poll, Pro model upgrades, /v1/models gateway fetch and the base web-search vendor call. screenpipe-cloud STT is offered only with a gateway; default to local whisper, Deepgram uses the user's key, live notes fall back to the local engine. piiBackend defaults/migrates to local and vendor Tinfoil constants go; Tinfoil only with an explicit endpoint. The cloud media analysis skill block defaults off and is injected only with a BYO gateway.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/store.rs (AI preset defaults, STT fallback region)`
- `apps/screenpipe-app-tauri/lib/hooks/use-settings.tsx (defaults and migrations, incl. analyticsEnabled=false)`
- `apps/screenpipe-app-tauri/components/settings/ai-presets.tsx`
- `apps/screenpipe-app-tauri/components/rewind/ai-presets-selector.tsx`
- `apps/screenpipe-app-tauri/src-tauri/src/ai_gateway.rs (new: get_screenpipe_ai_gateway_url)`
- `crates/screenpipe-engine/src/routes/cloud_proxy.rs`
- `crates/screenpipe-core/src/agents/pi.rs (cloud media analysis block, gateway base)`
- `crates/screenpipe-audio/src/transcription/deepgram/mod.rs`
- `crates/screenpipe-audio/src/meeting_streaming/config.rs`
- `crates/screenpipe-redact/src/adapters/tinfoil.rs`
- `crates/screenpipe-redact/src/adapters/tinfoil_image.rs`
- `crates/screenpipe-engine/src/privacy_filter.rs`
- `apps/screenpipe-app-tauri/components/settings/privacy-section.tsx (piiBackend option)`
- `apps/screenpipe-app-tauri/components/settings/recording-settings.tsx (STT engine options)`
- `crates/screenpipe-core/assets/extensions/web-search.ts`
- `apps/screenpipe-app-tauri/src-tauri/assets/extensions/web-search.ts`
- `packages/ai-gateway (exclude from build)`

Acceptance:
- [ ] bun test: fresh-settings defaults select an Ollama/OpenAI-compatible preset and analyticsEnabled=false; a stored piiBackend 'tinfoil' migrates to 'local'
- [ ] Harness: `api /v1/chat/completions -X POST -d '{"model":"x","messages":[]}'` with no gateway returns 503 with error cloud_token_missing
- [ ] With aiGatewayUrl pointing at a local mock OpenAI server, the same call is forwarded (mock logs the request)
- [ ] grep -rn 'api.screenpipe.com\|pii.screenpipe.containers' crates apps (non-test) returns nothing

#### W0-07 OAuth broker replaced by BYO client credentials (L)

- **Items:** cloud-018
- **Depends on:** W0-01
- **Specs:** specs/cloud.md cloud-018; specs/connections.md connections-oauth-byo-broker (merged)

Add token_url and uses_pkce to OAuthConfig and per-integration BYO {client_id, client_secret, redirect_uri} stored as oauth_client:<id> in the SecretStore. Exchange and refresh directly at each provider token endpoint; delete EXCHANGE_PROXY_URL and the screenpi.pe HTTPS callback relay. Providers that refuse loopback redirects show a user-supplied HTTPS relay field. MCP OAuth keeps the base loopback callback only.

Owner paths:
- `crates/screenpipe-connect/src/oauth.rs`
- `crates/screenpipe-connect/src/oauth_refresh_scheduler.rs`
- `crates/screenpipe-connect/src/connections/*.rs (OAuthConfig token_url/uses_pkce fields only)`
- `apps/screenpipe-app-tauri/src-tauri/src/oauth.rs (exchange/callback region)`
- `apps/screenpipe-app-tauri/components/settings/oauth-client-dialog.tsx (new)`
- `apps/screenpipe-app-tauri/components/settings/connections-section.tsx (BYO client form hook-up only)`
- `crates/screenpipe-engine/src/mcp_servers_api.rs (assert loopback only)`

Acceptance:
- [ ] cargo test -p screenpipe-connect oauth:: (wiremock token endpoint: code exchange with PKCE, refresh, error mapping)
- [ ] grep -rn 'EXCHANGE_PROXY_URL\|screenpi.pe/api/oauth' crates apps returns nothing
- [ ] Manual: with a BYO Google client, connect Google Calendar via the loopback callback; tokens land in the secret store

#### W0-08 Cloud sync, archive and pipe store off by default (M)

- **Items:** cloud-019, cloud-021, agents_pipes-037
- **Depends on:** W0-01
- **Specs:** specs/cloud.md cloud-019, cloud-021; specs/agents_pipes.md agents_pipes-037

Remove the screenpi.pe sync/archive defaults; cloud sync and archive commands return 'not configured' and the UI hides them; keep LAN remote_sync and the screenpipe-sync http_put BYO target. /pipes/store returns an empty list (not an error) when SCREENPIPE_API_BASE_URL is unset, works against a user-configured static registry, keeps the typed registry_* error codes, and publish/review are disabled; pipe share becomes 'export pipe file'.

Owner paths:
- `crates/screenpipe-core/src/sync/client.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/sync.rs (cloud sync, delete_cloud_data region)`
- `apps/screenpipe-app-tauri/components/settings/sync-settings.tsx`
- `apps/screenpipe-app-tauri/components/settings/archive-settings.tsx`
- `crates/screenpipe-engine/src/routes/pipe_store.rs`
- `crates/screenpipe-engine/src/cli/pipe.rs`
- `crates/screenpipe-engine/src/cli/install.rs`
- `apps/screenpipe-app-tauri/components/pipe-store.tsx`
- `apps/screenpipe-app-tauri/components/settings/pipes-section.tsx (share -> export file)`

Acceptance:
- [ ] Harness: `api /pipes/store` returns 200 with an empty data array when SCREENPIPE_API_BASE_URL is unset
- [ ] With SCREENPIPE_API_BASE_URL pointing at a local static JSON server the store lists its pipes
- [ ] grep -rn 'screenpi.pe/api/sync\|screenpi.pe"' crates apps (non-test) returns nothing

#### W0-09 Bundled CLI/MCP runtime and self-hosted model sources (M)

- **Items:** cloud-024, cloud-046, cloud-M02, agents_pipes-040
- **Depends on:** W0-01
- **Specs:** specs/cloud.md cloud-024, cloud-046, cloud-M02; specs/agents_pipes.md agents_pipes-040

Stop resolving vendor npm packages at runtime: build packages/screenpipe-mcp and the engine CLI locally, ship them as app resources and reference them by absolute path (SCREENPIPE_CLI / PATH shim) in skills, MCP configs and agent cards. Pyannote models are bundled or fetched from a Freenpipe-controlled mirror; HuggingFace screenpipe/* PII weights stay opt-in and repo-overridable.

Owner paths:
- `crates/screenpipe-audio/src/speaker/models.rs (download source)`
- `crates/screenpipe-redact/src/adapters/opf.rs`
- `crates/screenpipe-redact/src/adapters/onnx.rs`
- `crates/screenpipe-redact/src/adapters/rfdetr.rs`
- `crates/screenpipe-redact/src/adapters/rfdetr_mlx.rs`
- `crates/screenpipe-core/src/agents/pi.rs (screenpipe@latest / screenpipe-mcp@latest references)`
- `crates/screenpipe-core/src/agents/bash_env.rs`
- `apps/screenpipe-app-tauri/components/onboarding/connect-apps.tsx (MCP config snippet)`
- `apps/screenpipe-app-tauri/components/settings/connections-section.tsx (MCP snippet only)`
- `apps/screenpipe-app-tauri/components/settings/agent-card.tsx`
- `crates/screenpipe-core/assets/skills/**/SKILL.md (CLI invocation text)`
- `packages/screenpipe-mcp (local build)`
- `apps/screenpipe-app-tauri/src-tauri/tauri.windows.conf.json (bundle resources for MCP and pyannote models)`

Acceptance:
- [ ] grep -rnE 'screenpipe@latest|screenpipe-mcp@latest|raw/refs/heads/main' crates apps packages (excluding docs, node_modules) returns nothing
- [ ] Generated Claude Desktop MCP config points at the bundled MCP entry by absolute path (bun test on the snippet generator)
- [ ] cargo test -p screenpipe-audio speaker::models resolves the bundled model path before any download

#### W0-10 Logs and feedback upload replaced by a local diagnostics bundle (M)

- **Items:** cloud-036, frontend-046, meetings_memory-009
- **Depends on:** W0-01
- **Specs:** specs/cloud.md cloud-036; specs/frontend.md frontend-046; specs/meetings_memory.md meetings_memory-009; specs/shell.md shell-033 (merged)

Implement start_feedback_upload, redact_pii_for_feedback and read_log_tail(path, maxBytes) locally: build a redacted zip under <data_dir>/feedback (logs, panic log, crash dumps, update/recovery logs, health snapshot, settings with sensitive keys masked), emit feedback-upload-completed and offer a prefilled GitHub issue link. Remove every /api/logs upload path from the base JS.

Owner paths:
- `apps/screenpipe-app-tauri/components/share-logs-button.tsx`
- `apps/screenpipe-app-tauri/components/feedback-dialog.tsx`
- `apps/screenpipe-app-tauri/app/overlay/page.tsx (log upload code only)`
- `apps/screenpipe-app-tauri/components/onboarding/engine-startup.tsx (log upload code only)`
- `apps/screenpipe-app-tauri/src-tauri/src/feedback_upload.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/log_files.rs`

Acceptance:
- [ ] UI smoke: Help > share logs produces <FP_DD>/feedback/<ts>.zip and shows the completion state
- [ ] bun test: the redaction helper masks keys matching the sensitive-key rule
- [ ] grep -rn '/api/logs' apps/screenpipe-app-tauri (non-test) returns nothing

#### W0-11 SQLite toolchain upgrade (sqlx 0.9, libsqlite3-sys 0.37, rusqlite vtab) (L)

- **Items:** db-029
- **Depends on:** W0-01
- **Specs:** specs/db.md db-029

Move to sqlx 0.9 and a bundled libsqlite3-sys with SQLite >= 3.51.3 (recover extension and FTS5 contentless_delete compiled in), rusqlite with vtab, and add parquet + zstd as dependencies of screenpipe-db (unused until W5-07). Mechanical API migration only; re-verify that the migrator still applies out-of-order pending versions and still honours set_ignore_missing.

Owner paths:
- `Cargo.toml`
- `Cargo.lock`
- `crates/*/Cargo.toml (sqlx / rusqlite / libsqlite3-sys lines)`
- `crates/screenpipe-db/src/** (API migration only)`
- `crates/screenpipe-secrets/src/store.rs (API migration only)`
- `crates/screenpipe-engine/src/** sqlx call sites (API migration only)`
- `apps/screenpipe-app-tauri/src-tauri/src/ sqlx call sites (API migration only)`

Acceptance:
- [ ] cargo test -p screenpipe-db passes on the new toolchain
- [ ] A test asserts `select sqlite_version()` >= 3.51.3 and creates an FTS5 table with contentless_delete=1
- [ ] A migrator test applies a pending version older than the newest applied one (out-of-order) without error
- [ ] Engine and app build (release-dev)

#### W0-12 Rebrand user-visible identity to Freenpipe (M)

- **Items:** (no delta id) rebrand: product name, identifier, deep-link scheme, tray, icons, window titles, installer
- **Depends on:** W0-02, W0-03, W0-04, W0-05, W0-06, W0-07, W0-08, W0-09, W0-10, W0-11
- **Specs:** CLAUDE.md (interop identifiers); specs/shell.md (identifier, scheme)

Last WP of wave 0, after all de-cloud work. Rename user-visible identity to Freenpipe: productName, bundle identifier (com.normiecore.freenpipe and a .dev variant), installer/NSIS names, tray tooltip and menu title, window titles, icons and logos, About text, outbound user agent and user-facing copy that names the product. Register the freenpipe:// deep-link scheme (see open question on a screenpipe:// alias). Keep crate names, route paths, JSON fields, settings keys, env var names, CLI flags and data file names unchanged. Flip the vendor guard to failing mode.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/tauri*.conf.json (productName, identifier, deep-link schemes, window titles, bundle names)`
- `apps/screenpipe-app-tauri/src-tauri/icons/**`
- `apps/screenpipe-app-tauri/src-tauri/windows/hooks.nsh`
- `apps/screenpipe-app-tauri/src-tauri/src/tray.rs (tooltip/labels)`
- `apps/screenpipe-app-tauri/src-tauri/src/window/** (titles)`
- `apps/screenpipe-app-tauri/app/layout.tsx (metadata)`
- `apps/screenpipe-app-tauri/public/** (logos)`
- `apps/screenpipe-app-tauri/components/** (user-visible product-name strings only)`
- `README.md`
- `crates/screenpipe-core/src/endpoints.rs (user agent)`
- `scripts/ci/vendor-guard.test.ts (switch to failing mode)`

Acceptance:
- [ ] Release-dev bundle builds an installer named Freenpipe; installed app shows Freenpipe in tray tooltip and window titles (UI smoke)
- [ ] reg query HKCU\Software\Classes\freenpipe exists after install and HKCU\Software\Classes\screenpipe is untouched
- [ ] bun test scripts/ci/vendor-guard.test.ts passes in failing mode
- [ ] git diff of this WP changes no route string, settings key or SQL identifier (reviewer check with grep on '/\w' route literals)

#### W0-13 Vendor git dependencies into third_party/ (M)

- **Items:** (owner decision 2) vendor git deps
- **Depends on:** W0-11
- **Specs:** docs/clean-room/DECISIONS.md decision 2

Copy every git dependency hosted under the vendor's org or its contributors into third_party/<name>/ at its pinned revision (licence-checked; screenpipe-org revisions must predate 2026-06-10), switch to path deps / [patch], keep Cargo.lock consistent, and document name, upstream URL, revision, licence, date and local changes in third_party/README.md. Established third-party git deps may stay but are listed.

Owner paths:
- `third_party/ (new)`
- `third_party/README.md (new)`
- `Cargo.toml ([patch] / path deps)`
- `Cargo.lock`
- `crates/*/Cargo.toml (git dep lines only)`
- `apps/screenpipe-app-tauri/src-tauri/Cargo.toml (git dep lines only)`

Acceptance:
- [ ] No git dependency under the vendor orgs remains in any Cargo.toml
- [ ] cargo build --profile release-dev -p screenpipe-engine and the app crate build succeed
- [ ] third_party/README.md lists every vendored and remaining git dependency with licence and revision

### Wave 1: Cross-cutting foundations: schema v2.7, settings schema, integration seams

| WP | Size | Title | Depends on | Items |
|---|---|---|---|---|
| W1-01 | L | Schema v2.7.104: 28 migrations and ocr_text retirement | W0-12 | db-001, meetings_memory-001, db-005, db-006, db-027 |
| W1-02 | L | Settings schema v2 and store compatibility | W0-12 | shell-001, shell-002, shell-003, shell-037, shell-M02, frontend-053, audio-028, cloud-033, cloud-034, cloud-035 |
| W1-03 | M | Integration seams, stub modules and shared constants | W0-12 | shell-025, shell-038 |
| W1-04 | S | Enterprise and plan-gate no-op surface | W0-12 | frontend-052, cloud-043, agents_pipes-041, agents_pipes-045, api-035, meetings_memory-042 |

#### W1-01 Schema v2.7.104: 28 migrations and ocr_text retirement (L)

- **Items:** db-001, meetings_memory-001, db-005, db-006, db-027
- **Depends on:** W0-12
- **Specs:** specs/db.md db-001, db-005, db-006, db-027; specs/meetings_memory.md meetings_memory-001

Add the 28 target migrations under their exact versions (20260601000000..20261002190000; one sorts before the base's last two) with original file text and the DDL recorded in db.md; keep set_ignore_missing and the checksum rewrite path, which is how screenpipe-written DBs are adopted. Ship 20260613130000 together with the code change it forces: all OCR writes go to frames.text_json/full_text, ocr_text, its FTS and triggers disappear, search returns an empty ocr_engine, MigrationWorker is deleted, and the pre-migrate notice is logged. Add the performance indexes (no query rewrite needed for julianday).

Owner paths:
- `crates/screenpipe-db/src/migrations/ (28 new files)`
- `crates/screenpipe-db/src/db.rs (run_migrations notice + ocr_text query sites)`
- `crates/screenpipe-db/src/write_queue.rs`
- `crates/screenpipe-db/src/types.rs (OCR types)`
- `crates/screenpipe-db/src/migration_worker.rs (delete)`
- `crates/screenpipe-db/src/lib.rs (migration_worker export)`
- `crates/screenpipe-capture/src/paired_capture.rs (OCR write path)`
- `crates/screenpipe-redact/src/worker/tables.rs (ocr_text surface)`
- `crates/screenpipe-engine/src/routes/frames.rs, routes/search.rs, routes/streaming.rs (ocr_text refs)`
- `crates/screenpipe-engine/src/hot_frame_cache.rs`
- `crates/screenpipe-engine/src/video_cache.rs`
- `crates/screenpipe-engine/src/sync_provider.rs`
- `crates/screenpipe-db/tests/**`
- `crates/screenpipe-engine/tests/endpoint_test.rs`
- `crates/screenpipe-screen/tests/** (ocr_text refs)`

Acceptance:
- [ ] cargo test -p screenpipe-db passes, including a new migrations test asserting 117 applied versions identical to the target list in db.md
- [ ] On a fresh DB: `sqlq "select name from sqlite_master where name like 'ocr_text%'"` is empty and frames has text_json; starred_sessions, outputs, semantic_items_fts, activity_tasks exist
- [ ] A base-schema fixture DB with ocr_text rows migrates and its text appears in frames.full_text/text_json
- [ ] Harness: record 2 minutes, `api '/search?content_type=ocr&limit=5'` returns items with empty ocr_engine

#### W1-02 Settings schema v2 and store compatibility (L)

- **Items:** shell-001, shell-002, shell-003, shell-037, shell-M02, frontend-053, audio-028, cloud-033, cloud-034, cloud-035
- **Depends on:** W0-12
- **Specs:** specs/shell.md shell-001..003, 037, M02; specs/frontend.md frontend-053; specs/audio.md audio-028; specs/cloud.md cloud-033..035

Add every new settings key and type once, in target field order, across RecordingSettings, SettingsStore, TS defaults and bindings: ~45 shell keys, audio keys (audioCaptureMode, experimentalMeetingPiggyback, alwaysRecordBluetoothMic, screenpipeAecEnabled, aecMode), vision keys (disableScreenshots, captureMaxWidth, recordingDetail, includedUrls with the UrlRule/DomainRule types, piiRedactionColumns, piiRedactionPseudonyms), notification prefs, activities*, sidebarNavLayout, uiLocale, etc. Apply the new defaults (usePiiRemoval, autoUpdate, filterMusic true), new one-time migrations and the OnboardingStore firstRunSummary* fields; keep hideThinkingBlocks for compatibility. Move the cloud token into the secret store and fail closed when the store cannot initialise. Remote-control flags, remote log collection and workflow-sharing keys are accepted for round-trip but forced off and never acted on. Feature WPs consume these keys and must not add keys of their own.

Owner paths:
- `crates/screenpipe-config/src/recording.rs`
- `crates/screenpipe-config/src/defaults.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/store.rs (settings + OnboardingStore)`
- `apps/screenpipe-app-tauri/src-tauri/src/commands.rs (get_cloud_token, get_screenpipe_base_dir, get_active_data_dir)`
- `apps/screenpipe-app-tauri/src-tauri/src/main.rs (store init region)`
- `apps/screenpipe-app-tauri/lib/hooks/use-settings.tsx`
- `apps/screenpipe-app-tauri/lib/utils/tauri.ts (generated)`
- `crates/screenpipe-engine/src/recording_config.rs`
- `crates/screenpipe-engine/src/cli/mod.rs (new audio/vision flags only)`

Acceptance:
- [ ] cargo test -p screenpipe-config: a target-shaped settings JSON fixture with every new key round-trips and unknown keys are preserved
- [ ] cargo test (app crate): a screenpipe-shaped store.bin with firstRunSummary* and trialActivationFreshInstall loads; cloud token is migrated to cloud.auth_token
- [ ] bun test: defaults usePiiRemoval/autoUpdate/filterMusic=true, remoteLogCollectionEnabled=false, workflowSharing absent after load
- [ ] bun run bindings:check passes

#### W1-03 Integration seams, stub modules and shared constants (M)

- **Items:** shell-025, shell-038
- **Depends on:** W0-12
- **Specs:** specs/shell.md shell-025, shell-038; PLAN.md section 5

Pre-build the seams that let later WPs add code in their own files. Engine: each planned route module exists as a stub exposing an empty oasgen registration and axum router, mounted once from server.rs; middleware layers (request origin, search-only) are pre-wired as pass-through. DB: an ext/ module tree of empty `impl DatabaseManager` files. Connections: a router merge point for per-integration route files. Pipes: a run-lifecycle hook module. Pi: data-driven bundled-extension manifests on core and app side. App: mod declarations plus empty stub files for every planned app module. Centralise data-dir file names (crash-dumps, recording-recovery.log, update-*.log, health-startup.json, secrets.sqlite, ...) and SCREENPIPE_* env names. No behaviour change.

Owner paths:
- `crates/screenpipe-engine/src/server.rs (router composition)`
- `crates/screenpipe-engine/src/routes/mod.rs`
- `crates/screenpipe-engine/src/routes/ext.rs (new)`
- `crates/screenpipe-engine/src/routes/<feature>.rs stubs (starred, semantic, artifacts, structured_outputs, live_views, ai_feedback, activity_ledger, vision, workflows, agent, connect_broker, local_chat, web_search, data_sync_proxy, cloud_agents, internal_telemetry, request_origin, response_format, search_only, pipe_stream, meeting_overlay)`
- `crates/screenpipe-engine/src/lib.rs`
- `crates/screenpipe-engine/src/core.rs (background worker registry)`
- `crates/screenpipe-engine/src/connections_api.rs (router merge point only)`
- `crates/screenpipe-engine/src/connections_routes/ (new: mod.rs + stubs)`
- `crates/screenpipe-db/src/lib.rs (pub mod ext)`
- `crates/screenpipe-db/src/ext/ (new: outputs, feedback, starred, semantic, activity, display_layout, pipe_events, frames_preview, meetings_v2, tags, retention_strip stubs)`
- `crates/screenpipe-core/src/pipes/hooks.rs (new) + call sites in crates/screenpipe-core/src/pipes/mod.rs`
- `crates/screenpipe-core/src/agents/extensions_manifest.rs (new)`
- `crates/screenpipe-core/src/agents/pi.rs (install loop reads manifest)`
- `apps/screenpipe-app-tauri/src-tauri/src/pi_extensions_manifest.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/pi.rs (install loop reads manifest)`
- `apps/screenpipe-app-tauri/src-tauri/src/main.rs (mod declarations for planned modules)`
- `crates/screenpipe-core/src/paths.rs (data-dir file-name constants)`
- `crates/screenpipe-core/src/env.rs (new: SCREENPIPE_* names)`

Acceptance:
- [ ] Engine and app build; cargo test -p screenpipe-engine --test endpoint_test passes unchanged
- [ ] `api /openapi.json | python -I -c "import json,sys;print(len(json.load(sys.stdin)['paths']))"` prints the same count as before the change
- [ ] Unit test: the bundled extension manifests list exactly the extensions the base installed
- [ ] PLAN.md section 5 stub list matches the files on disk (reviewer check)

#### W1-04 Enterprise and plan-gate no-op surface (S)

- **Items:** frontend-052, cloud-043, agents_pipes-041, agents_pipes-045, api-035, meetings_memory-042
- **Depends on:** W0-12
- **Specs:** specs/frontend.md frontend-052; specs/cloud.md cloud-043; specs/api.md api-035; specs/meetings_memory.md meetings_memory-042; specs/agents_pipes.md agents_pipes-041, 045

Register the target-only enterprise commands the parity frontend calls (apply_enterprise_ui_visibility, get_enterprise_host_identity, set_enterprise_recording_authorized, is_history_access_restricted, list/sync_managed_team_skills, set_activity_sync_enabled) as inert, network-free stubs. Provide history_access as always-unrestricted (the history_access_limited code exists only as an enum value, never returned), managed pipes/team skills as 'not managed', and no pipe-count limit.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/enterprise_policy.rs (new no-op commands)`
- `apps/screenpipe-app-tauri/lib/hooks/use-enterprise-policy.ts`
- `crates/screenpipe-engine/src/history_access.rs (new)`
- `crates/screenpipe-core/src/pipes/managed.rs (new)`

Acceptance:
- [ ] tauri.ts exposes isHistoryAccessRestricted and it resolves false (bun test against bindings)
- [ ] cargo test -p screenpipe-engine history_access: every caller gets Unrestricted
- [ ] grep -rn 'history_access_limited' crates shows only the enum definition

### Wave 2: Engine data layer and core backends

| WP | Size | Title | Depends on | Items |
|---|---|---|---|---|
| W2-01 | L | DB runtime hardening: pragmas, WAL maintenance, preflight, planner stats, cancellable reads | W1-01, W1-03 | db-016, db-017, db-018, db-v2-01 |
| W2-02 | L | Outputs and artifacts registry | W1-01, W1-03 | db-002, meetings_memory-002, meetings_memory-003, meetings_memory-004, api-017, agents_pipes-010 |
| W2-03 | M | Unified AI feedback table and /feedback | W1-01, W1-03 | db-010, meetings_memory-007, api-018, agents_pipes-044 |
| W2-04 | M | Starred work sessions backend and HD binding | W1-01, W1-03 | db-015, api-007, meetings_memory-010, vision-starred-hd |
| W2-05 | M | Semantic store: runs, items, FTS, actors | W1-01, W1-03 | db-008, meetings_memory-019, meetings_memory-020 |
| W2-06 | XL | screenpipe-semantic parser crate | W0-12 | meetings_memory-021 |
| W2-07 | L | Activity ledger store and deterministic reconciler | W1-01, W1-03 | db-013, meetings_memory-013, meetings_memory-014 |
| W2-08 | L | Meetings API v2: calendar binding, summary endpoints, room-change route | W1-01, W1-03 | db-012, meetings_memory-025, meetings_memory-032, meetings_memory-047, api-014, meetings_memory-026 |
| W2-09 | M | Response formats, calendar time literals, tag autocomplete | W1-01, W1-03 | api-001, api-002, api-009 |
| W2-10 | L | Frame media routes: thumbnail, preview samples, preview media, typed errors, auth | W1-01, W1-03 | api-010, vision-thumbnail-endpoint, vision-preview-samples, vision-preview-media, vision-frame-media-errors, api-011 |
| W2-11 | L | Per-monitor vision control and display layout persistence | W1-01, W1-03 | vision-device-control, api-016, vision-display-layout, db-007 |
| W2-12 | L | Native WinRT/D3D11 Windows Graphics Capture and frame-diff hardening | W0-12 | vision-wgc-native-capture, vision-frame-comparison-hardening |
| W2-13 | L | Health v2, vision metrics and recording coverage | W1-03 | api-033, vision-pipeline-metrics, gap_health-lifecycle-internals-001, gap_health-lifecycle-internals-002, gap_health-lifecycle-internals-003, audio-027 |
| W2-14 | L | Resource monitor v2: process tree, breakdown, leak trend | W1-03 | gap_health-lifecycle-internals-004, gap_health-lifecycle-internals-005, gap_health-lifecycle-internals-006, gap_health-lifecycle-internals-007, gap_health-lifecycle-internals-008, gap_health-lifecycle-internals-009 |
| W2-15 | L | Snapshot compaction v2 and suspend/lock awareness | W1-01, W1-03 | vision-compaction-encoder, gap_health-lifecycle-internals-011, gap_health-lifecycle-internals-012 |
| W2-16 | L | Speakers v2: samples, search paging, merge integrity, reassign v2 | W1-01 | audio-002, audio-003, audio-004, audio-005, audio-006, audio-009, api-013, db-011 |
| W2-17 | L | Stream-scoped live speakers and strict meeting-segment mapping | W1-01 | audio-001, db-014, audio-008, audio-M01, audio-M05 |
| W2-18 | L | Windows WASAPI loopback engine and exe-based audio exclusions | W1-02 | audio-013, audio-014 |
| W2-19 | L | GPU runtimes and bundled models: Vulkan whisper, DirectML, install layout | W0-12 | audio-019, audio-020, audio-022, shell-029 |
| W2-20 | L | Connections core: secret-safe reads, webhook proxy, Telegram send, router deltas, MCP refs | W1-03 | connections-secret-redaction, connections-webhook-proxy, connections-telegram-send, api-031, connections-supports-oauth-instances, connections-pipe-mcp-connection-refs, cloud-038, api-M03 |
| W2-21 | L | OAuth v2 flow and new OAuth integrations | W1-03 | connections-oauth-connect-v2, connections-oauth-default-slot-promotion, connections-oauth-scope-tweaks, connections-hubspot-oauth, connections-zendesk-oauth, connections-outlook-email |
| W2-22 | L | Pipe config v2: new frontmatter, schedule_config and preview, ids, preamble | W1-03 | agents_pipes-003, agents_pipes-004, agents_pipes-m03, agents_pipes-m05 |
| W2-23 | L | Pi runtime 0.84.1, isolated config, baseline bun, compaction patch | W1-03 | agents_pipes-015 |
| W2-24 | M | Engine local stubs: attribution headers, telemetry no-op routes, local data-sync | W1-03 | api-012, agents_pipes-038, cloud-040, api-026, cloud-037 |
| W2-25 | L | Frontend stack refresh and browser-dev mock runtime | W1-02 | frontend-001, frontend-004 |
| W2-26 | L | App engine supervision: auto-respawn, port-conflict classification, safe relaunch | W1-02 | shell-012, shell-013 |
| W2-27 | L | App startup hardening: argv helper modes, autostart, focus handoff, env, AVX2, WER, opener | W1-03 | shell-017, shell-019, shell-020, shell-021, shell-022, shell-024, shell-028 |

#### W2-01 DB runtime hardening: pragmas, WAL maintenance, preflight, planner stats, cancellable reads (L)

- **Items:** db-016, db-017, db-018, db-v2-01
- **Depends on:** W1-01, W1-03
- **Specs:** specs/db.md db-016..018, db-v2-01

wal_autocheckpoint 0 with a PASSIVE maintenance loop, RESTART at boot, query_only read pool, an offline DELETE/FULL/EXCLUSIVE mode, bounded SQLite header preflight, refusal of SQLite < 3.51.3, sqlite-vec registration check and a one-manager-per-db .owner file. Periodic ANALYZE + optimize with the sqlite-planner-maintenance.json report. Interrupt reads when requests drop and roll back abandoned read transactions. On-demand /data/checkpoint stops using TRUNCATE.

Owner paths:
- `crates/screenpipe-db/src/db.rs (DatabaseManager::new, WAL checkpoint task, wal_checkpoint(), heavy-read helpers)`
- `crates/screenpipe-db/src/write_queue.rs (writer pragmas)`
- `crates/screenpipe-db/src/maintenance.rs (new)`
- `crates/screenpipe-db/src/planner_stats.rs (new)`
- `crates/screenpipe-db/src/cancellable_query.rs (new)`

Acceptance:
- [ ] cargo test -p screenpipe-db: pragma test (wal_autocheckpoint=0 on writer, query_only=1 on reader); preflight rejects a non-SQLite file; version guard
- [ ] Harness: `api /data/checkpoint -X POST` returns busy/log/checkpointed and the engine log shows RESTART or FULL, never TRUNCATE
- [ ] With the maintenance interval shortened by a test env, <FP_DD>/sqlite-planner-maintenance.json appears with latest/last_success
- [ ] Test: dropping a long-running query future interrupts it within 1 s and the pool connection is reusable

#### W2-02 Outputs and artifacts registry (L)

- **Items:** db-002, meetings_memory-002, meetings_memory-003, meetings_memory-004, api-017, agents_pipes-010
- **Depends on:** W1-01, W1-03
- **Specs:** specs/db.md db-002; specs/meetings_memory.md 002..004; specs/api.md api-017; specs/agents_pipes.md agents_pipes-010

outputs CRUD/list/facets (excluding structured-output rows), the output_search_index sidecar and output_search_fts with hash-skip reindex and startup backfill. POST /artifacts/register copies into <data>/outputs (100 MB cap, SAF envelope detection), GET /artifacts merges registered and pipe-dir artifacts with q/limit/offset/per_pipe_limit/include_internal, DELETE /artifacts/:id. Declared pipe artifacts are auto-registered after runs (source_type pipe-run) via the pipe hook; add the register_artifact pipe tool and the save_artifact chat tool.

Owner paths:
- `crates/screenpipe-db/src/ext/outputs.rs`
- `crates/screenpipe-engine/src/routes/artifacts.rs`
- `crates/screenpipe-engine/src/artifacts_index.rs (new: startup backfill)`
- `crates/screenpipe-core/src/pipes/artifacts.rs (new)`
- `crates/screenpipe-core/src/pipes/hooks.rs (artifact registration hook)`
- `crates/screenpipe-core/assets/extensions/register-artifact.ts (new)`
- `apps/screenpipe-app-tauri/src-tauri/assets/extensions/save-artifact.ts (new)`
- `crates/screenpipe-core/src/agents/extensions_manifest.rs (entry)`
- `apps/screenpipe-app-tauri/src-tauri/src/pi_extensions_manifest.rs (entry)`

Acceptance:
- [ ] Harness: `api /artifacts/register -X POST -H 'content-type: application/json' -d '{"path":"C:/tmp/a.md","title":"A"}'` returns an id and the file exists under <FP_DD>/outputs
- [ ] `api '/artifacts?q=<word in a.md>'` returns it; `api /artifacts/<id> -X DELETE` removes it
- [ ] A 101 MB file is rejected; an invalid .saf.json envelope returns 400
- [ ] cargo test -p screenpipe-db outputs: FTS hash-skip and bm25 ordering

#### W2-03 Unified AI feedback table and /feedback (M)

- **Items:** db-010, meetings_memory-007, api-018, agents_pipes-044
- **Depends on:** W1-01, W1-03
- **Specs:** specs/db.md db-010; specs/meetings_memory.md meetings_memory-007; specs/api.md api-018; specs/agents_pipes.md agents_pipes-044

Upsert/list over the feedback table: natural-key uniqueness per target+actor+device, comment only with a down rating, pipes cannot rate their own outputs, pipe-scoped visibility, filters kind/target_id/producer/rating/q/since/after_id/order/limit.

Owner paths:
- `crates/screenpipe-db/src/ext/feedback.rs`
- `crates/screenpipe-engine/src/routes/ai_feedback.rs`

Acceptance:
- [ ] Harness: POST /feedback {target:{kind:'notification',id:'n1'},rating:'up'} returns 200; repeating it updates the same row (sqlq count = 1)
- [ ] A comment with rating up returns 400; a request carrying a pipe session header rating that pipe's output returns 403
- [ ] `api '/feedback?kind=notification&rating=up'` lists the row

#### W2-04 Starred work sessions backend and HD binding (M)

- **Items:** db-015, api-007, meetings_memory-010, vision-starred-hd
- **Depends on:** W1-01, W1-03
- **Specs:** specs/db.md db-015; specs/api.md api-007; specs/meetings_memory.md meetings_memory-010; specs/vision.md vision-starred-hd

POST /starred-sessions with optimistic revision (0 creates, mismatch 409), <24 h ranges and start rules, user-only marking; GET lists overlapping sessions with computed has_audio; membership and coverage-island queries. HighFpsController gains a Starred session kind for hd_requested sessions, ending at the session end and honouring pauses and exclusions.

Owner paths:
- `crates/screenpipe-db/src/ext/starred.rs`
- `crates/screenpipe-engine/src/routes/starred.rs`
- `crates/screenpipe-engine/src/high_fps_controller.rs`

Acceptance:
- [ ] Harness: POST /starred-sessions {id:<uuid>,start,end,hd_requested:false,revision:0} returns revision 1; a stale revision returns 409
- [ ] A 25 h range returns 400; `api '/starred-sessions?start_time=..&end_time=..'` lists it with has_audio
- [ ] cargo test -p screenpipe-engine high_fps_controller: an active hd_requested starred session reports HD active until its end

#### W2-05 Semantic store: runs, items, FTS, actors (M)

- **Items:** db-008, meetings_memory-019, meetings_memory-020
- **Depends on:** W1-01, W1-03
- **Specs:** specs/db.md db-008; specs/meetings_memory.md meetings_memory-019, 020

DB layer for the semantic projection: fingerprinted runs, content-addressed items, run-item hierarchy, external-content FTS, frames.semantic_run_id attachment in batches, actors/aliases/item-actors with store-time alias lookup and heuristic assignment, merge/rename/reassign SQL and GC of unreferenced runs and items.

Owner paths:
- `crates/screenpipe-db/src/ext/semantic.rs`
- `crates/screenpipe-db/tests/semantic_store_test.rs (new)`

Acceptance:
- [ ] cargo test -p screenpipe-db --test semantic_store_test: store, FTS match, GC, alias uniqueness, merge and reassign

#### W2-06 screenpipe-semantic parser crate (XL)

- **Items:** meetings_memory-021
- **Depends on:** W0-12
- **Specs:** specs/meetings_memory.md meetings_memory-021

New pure crate: budgeted compact accessibility tree, domain-separated 32-byte fingerprints, a parser registry of ~50 app/URL profiles (chat, mail, docs, notes, AI chat, code hosts, editors, terminals) with family parsers, message time parsing and a validation step that abstains on low confidence. Profiles and parsers are written from the observed catalogue, not copied.

Owner paths:
- `crates/screenpipe-semantic/** (new crate)`

Acceptance:
- [ ] cargo test -p screenpipe-semantic: synthetic trees for Slack, Teams, Outlook, VS Code and a browser page yield typed items
- [ ] Fingerprints are stable across runs and differ across domains; node-budget overflow returns the budget error; ambiguous input abstains

#### W2-07 Activity ledger store and deterministic reconciler (L)

- **Items:** db-013, meetings_memory-013, meetings_memory-014
- **Depends on:** W1-01, W1-03
- **Specs:** specs/db.md db-013; specs/meetings_memory.md meetings_memory-013, 014

DB helpers for activity_tasks/intervals/actions/evidence/ledger_state and the deterministic-v1 reconciler worker: 10 s sampled frames, UI events and audio (>= 8 chars) become app/window/document/semantic/audio/unobserved tasks, with cut-point reconcile, upserts, task GC and a 7-day backfill. Registered through the core worker registry.

Owner paths:
- `crates/screenpipe-db/src/ext/activity.rs`
- `crates/screenpipe-engine/src/activity_ledger.rs (new reconciler)`

Acceptance:
- [ ] cargo test: synthetic frames/ui_events/audio produce the expected tasks and intervals; rerunning the reconciler is idempotent
- [ ] Deleting frames removes their evidence and empty unlocked tasks through the triggers (sqlq counts before/after)

#### W2-08 Meetings API v2: calendar binding, summary endpoints, room-change route (L)

- **Items:** db-012, meetings_memory-025, meetings_memory-032, meetings_memory-047, api-014, meetings_memory-026
- **Depends on:** W1-01, W1-03
- **Specs:** specs/db.md db-012; specs/meetings_memory.md 025, 026, 032, 047; specs/api.md api-014

calendar_event_id on start and PUT with conditional binding (unique violation means 'already bound', keep the title); rejoin lookup excludes room_changed; transcript segments expose audioStartTimeSecs/sessionSpeakerId; list q covers meeting_app; export include_audio. New POST /meetings/:id/summary (merge or replace the '## Summary' section, 400 on empty or failure text), GET /meetings/:id/summary-status and POST /meetings/room-change {meetingId, token, decision}. The built-in meeting-summary pipe saves through the summary endpoint (prompt text rewritten in our own words).

Owner paths:
- `crates/screenpipe-engine/src/routes/meetings.rs`
- `crates/screenpipe-engine/src/meeting_export.rs`
- `crates/screenpipe-db/src/ext/meetings_v2.rs`
- `crates/screenpipe-db/src/db.rs (meetings rejoin lookup region)`
- `crates/screenpipe-db/src/types.rs (meeting request/response types)`
- `crates/screenpipe-core/assets/pipes/meeting-summary/pipe.md`

Acceptance:
- [ ] Harness: POST /meetings/start {calendar_event_id:'evt1'} binds; PUT another meeting with the same id keeps its title and does not error
- [ ] POST /meetings/<id>/summary {summary:'x'} appends a '## Summary' section; an empty summary returns 400; GET /meetings/<id>/summary-status returns state idle
- [ ] POST /meetings/room-change {decision:'switch'} ends the meeting with end_reason room_changed and starts a new one
- [ ] cargo test -p screenpipe-db: rejoin lookup ignores room_changed meetings

#### W2-09 Response formats, calendar time literals, tag autocomplete (M)

- **Items:** api-001, api-002, api-009
- **Depends on:** W1-01, W1-03
- **Specs:** specs/api.md api-001, 002, 009

Flexible time params accept today, yesterday and YYYY-MM-DD as local midnights. A shared response formatter gives format=json|csv|tsv and dotted-path fields= projection (flat JSON keys when fields are used, 400 on unknown format) for /search, /elements and /frames/:id/elements. GET /tags/autocomplete?q&limit returns name plus frame/audio/memory counts.

Owner paths:
- `crates/screenpipe-engine/src/routes/time.rs`
- `crates/screenpipe-engine/src/routes/response_format.rs`
- `crates/screenpipe-engine/src/routes/content.rs (autocomplete handler)`
- `crates/screenpipe-db/src/ext/tags.rs`
- `crates/screenpipe-engine/src/routes/search.rs (format/fields plumbing only)`
- `crates/screenpipe-engine/src/routes/elements.rs (format/fields plumbing only)`

Acceptance:
- [ ] Harness: `api '/search?start_time=today&limit=1'` returns 200; start_time=nonsense returns 400 naming the three accepted families
- [ ] `api '/search?format=csv&fields=content.text&limit=3' -i` shows text/csv; format=xml returns 400
- [ ] `api '/tags/autocomplete?q=wo'` returns [{name,frame_count,audio_count,memory_count}]

#### W2-10 Frame media routes: thumbnail, preview samples, preview media, typed errors, auth (L)

- **Items:** api-010, vision-thumbnail-endpoint, vision-preview-samples, vision-preview-media, vision-frame-media-errors, api-011
- **Depends on:** W1-01, W1-03
- **Specs:** specs/api.md api-010, 011; specs/vision.md thumbnail, preview-samples, preview-media, frame-media-errors

GET /frames/:id/thumbnail (width/quality/fallback/retry, in-memory cache, ETag frame-thumbnail-*, x-screenpipe-thumbnail-* headers, private no-cache), GET /frames/preview-samples (evenly spread refs for an app/domain in <= 24 h, never extracts media), GET /frames/preview-media/:video_chunk_id (byte ranges over existing local MP4s). /frames/:id adds frame_media_unavailable and snapshot_read_failed and rechecks the exact frame before snapshot_missing. /frames/* stop being auth-exempt (cookie covers img src).

Owner paths:
- `crates/screenpipe-engine/src/routes/frames.rs`
- `crates/screenpipe-engine/src/video_utils.rs`
- `crates/screenpipe-engine/src/frame_thumbnail_cache.rs (new)`
- `crates/screenpipe-db/src/ext/frames_preview.rs`
- `crates/screenpipe-engine/src/server.rs (auth exemption list only)`

Acceptance:
- [ ] Harness: `api '/frames/<id>/thumbnail?width=320' -i` returns image/jpeg with x-screenpipe-thumbnail-width and an ETag; If-None-Match returns 304
- [ ] `api '/frames/preview-samples?app_name=Code&start_time=..&end_time=..&limit=6'` returns frames; a 25 h range returns 400
- [ ] `api /frames/preview-media/<chunk> -H 'Range: bytes=0-99' -i` returns 206
- [ ] Without the bearer header, /frames/<id> returns 403 while /health returns 200

#### W2-11 Per-monitor vision control and display layout persistence (L)

- **Items:** vision-device-control, api-016, vision-display-layout, db-007
- **Depends on:** W1-01, W1-03
- **Specs:** specs/vision.md device-control, display-layout; specs/api.md api-016; specs/db.md db-007

POST /vision/device/start|stop {monitor_id} and GET /vision/device/status [{id,name,active,user_disabled}] (key is id, per the frontend) backed by an in-memory user-paused set; start_monitor and the watcher skip paused monitors. The watcher writes canonical sorted display_layout JSON at startup and on change (timeout, retry next pass). ui_events.element_ancestors gets root-first [{role,name}]. The delete-range keep-newest rule for display_layout is implemented by W3-09.

Owner paths:
- `crates/screenpipe-engine/src/routes/vision.rs`
- `crates/screenpipe-engine/src/vision_manager/manager.rs`
- `crates/screenpipe-engine/src/vision_manager/monitor_watcher.rs`
- `crates/screenpipe-db/src/ext/display_layout.rs`
- `crates/screenpipe-engine/src/ui_recorder.rs (element_ancestors)`
- `crates/screenpipe-a11y/src/events.rs (ancestor capture)`

Acceptance:
- [ ] Harness: `api /vision/device/status` returns [{id,name,active,user_disabled}]
- [ ] POST /vision/device/stop {monitor_id} sets user_disabled and no frames arrive from that monitor for 30 s (sqlq count by device); start resumes
- [ ] sqlq 'select count(*) from display_layout' is >= 1 after startup; ui_events rows from clicks carry element_ancestors JSON

#### W2-12 Native WinRT/D3D11 Windows Graphics Capture and frame-diff hardening (L)

- **Items:** vision-wgc-native-capture, vision-frame-comparison-hardening
- **Depends on:** W0-12
- **Specs:** specs/vision.md wgc-native-capture, frame-comparison-hardening

Replace the xcap VideoRecorder wrapper with direct GraphicsCaptureItem/free-threaded frame pool sessions: border and cursor off, SetMinUpdateInterval when available, staging-texture readback, pool recreate on resize, device-removed recovery and WARP fallback, EnumDisplayMonitors enumeration; keep the 3-failure per-frame fallback and the stable_id format. FrameComparer uses fallible thumbnail allocation (failure = no change, rate-limited structured log).

Owner paths:
- `crates/screenpipe-screen/src/wgc_capture.rs`
- `crates/screenpipe-screen/src/monitor.rs`
- `crates/screenpipe-screen/src/monitor/windows.rs (new)`
- `crates/screenpipe-screen/src/frame_comparison.rs`
- `crates/screenpipe-screen/Cargo.toml`
- `crates/screenpipe-screen/examples/wgc_smoke.rs (new)`

Acceptance:
- [ ] cargo test -p screenpipe-screen (frame_comparison allocation-failure path)
- [ ] cargo run -p screenpipe-screen --example wgc_smoke --profile release-dev writes 10 non-black JPEGs per monitor
- [ ] Manual: change display scaling while capturing; the pool is recreated and capture continues; stable_id matches the base value

#### W2-13 Health v2, vision metrics and recording coverage (L)

- **Items:** api-033, vision-pipeline-metrics, gap_health-lifecycle-internals-001, gap_health-lifecycle-internals-002, gap_health-lifecycle-internals-003, audio-027
- **Depends on:** W1-03
- **Specs:** specs/api.md api-033; specs/vision.md pipeline-metrics; specs/gap_health-lifecycle-internals.md 001-003; specs/audio.md audio-027

HealthCheckResponse gains dependencies/runtime/startup, unhealthy_reasons, vision_reason, loop_stage fields, audio_capture_mode, capture_status, recording_coverage (between ui_recorder and pool_stats, field order per gap_health-002), write-queue and pool signals and extra counters, with a cached snapshot on budget overrun. MetricsSnapshot gains the 13 vision fields; audio metrics gain last_stream_timeout_at. The 5 s coverage ticker classifies idle/paused/recording/stalled. Coverage keys are never sent anywhere.

Owner paths:
- `crates/screenpipe-engine/src/routes/health.rs`
- `crates/screenpipe-engine/src/recording_coverage.rs`
- `crates/screenpipe-screen/src/metrics.rs`
- `crates/screenpipe-audio/src/metrics.rs`
- `crates/screenpipe-engine/src/server.rs (health ticker spawn region)`

Acceptance:
- [ ] Harness: `api /health` contains recording_coverage with recording_secs, active_stalled_secs, active_paused_secs, idle_secs, active_secs, coverage_ratio in that order, plus unhealthy_reasons, vision_reason and audio_capture_mode
- [ ] cargo test -p screenpipe-engine recording_coverage: classifier cases (idle, schedule pause, fresh write, stalled) and the 1..15 s tick window
- [ ] `api /vision/metrics` includes silent_loss_rate and capture_loop_heartbeats

#### W2-14 Resource monitor v2: process tree, breakdown, leak trend (L)

- **Items:** gap_health-lifecycle-internals-004, gap_health-lifecycle-internals-005, gap_health-lifecycle-internals-006, gap_health-lifecycle-internals-007, gap_health-lifecycle-internals-008, gap_health-lifecycle-internals-009
- **Depends on:** W1-03
- **Specs:** specs/gap_health-lifecycle-internals.md 004-009

Descendant-tree accounting with swap, loads and footprint; process breakdown groups (app, app_child, mcp_child, mcp_external, other) with top-12 lists; pressure WARN and opt-in INFO logging; least-squares leak detector over a 6 h window after 30 min warm-up; 18-key JSONL file; INFO 'host:' startup line. Local only.

Owner paths:
- `crates/screenpipe-engine/src/resource_monitor.rs`
- `crates/screenpipe-engine/src/resource_breakdown.rs (new)`

Acceptance:
- [ ] cargo test: breakdown classification over a synthetic process table; leak detector fires on synthetic linear growth and not on noise
- [ ] SAVE_RESOURCE_USAGE=1 engine run writes resource_usage_*.json whose lines carry the 18 keys in order
- [ ] Engine log contains one INFO line starting 'host: os='

#### W2-15 Snapshot compaction v2 and suspend/lock awareness (L)

- **Items:** vision-compaction-encoder, gap_health-lifecycle-internals-011, gap_health-lifecycle-internals-012
- **Depends on:** W1-01, W1-03
- **Specs:** specs/vision.md compaction-encoder; specs/gap_health-lifecycle-internals.md 011, 012

Probe hevc_nvenc/qsv/amf (and videotoolbox on macOS) then libx264 once with a tiny lavfi test, overridable by SCREENPIPE_COMPACTION_ENCODER; defer unreadable snapshots, clear missing pointers, keep JPEGs on ffmpeg failure. Register PowerRegisterSuspendResumeNotification with a system-suspended flag (base polling kept as fallback); compaction computes ordered deferral reasons and pauses mid-cycle on suspend or lock.

Owner paths:
- `crates/screenpipe-engine/src/snapshot_compaction.rs`
- `crates/screenpipe-engine/src/sleep_monitor.rs`
- `crates/screenpipe-engine/src/power/monitor.rs`
- `crates/screenpipe-engine/src/power/profile.rs`
- `crates/screenpipe-core/src/video.rs`
- `crates/screenpipe-engine/Cargo.toml (Win32_System_Power feature)`

Acceptance:
- [ ] Engine log shows the probed encoder; SCREENPIPE_COMPACTION_ENCODER=software forces libx264
- [ ] cargo test: deferral reason ordering (suspended > locked > battery > low-power > power-paused > battery-saver)
- [ ] Manual: sleep and resume the laptop; the log shows suspend registration and the flag set/cleared; compaction resumes

#### W2-16 Speakers v2: samples, search paging, merge integrity, reassign v2 (L)

- **Items:** audio-002, audio-003, audio-004, audio-005, audio-006, audio-009, api-013, db-011
- **Depends on:** W1-01
- **Specs:** specs/audio.md audio-002..006, 009; specs/api.md api-013; specs/db.md db-011

GET /speakers/sample/:audio_chunk_id?start&end returns a WAV clip; /speakers/search adds limit/offset/include_samples with a light NOCASE prefix path; audio_samples gain audio_chunk_id and skip unplayable paths; merge/absorb repoint all 5 referencing tables and delete only unreferenced speakers; reassign gains scope (auto renames an unnamed voice in place, moves matching live segments) with undo; stricter self-speaker naming; startup self-heal for speaker tables.

Owner paths:
- `crates/screenpipe-engine/src/routes/speakers.rs`
- `crates/screenpipe-db/src/db.rs (speaker functions region and speaker self-heal)`
- `crates/screenpipe-engine/src/calendar_speaker_id.rs`
- `apps/screenpipe-app-tauri/components/settings/speakers-section.tsx (sample playback)`

Acceptance:
- [ ] Harness: `api '/speakers/sample/<chunk>?start=0&end=2' -i` returns audio/wav; end<=start returns 400
- [ ] `api '/speakers/search?name=a&limit=5&include_samples=false'` returns at most 5 names
- [ ] cargo test -p screenpipe-db speaker_reassignment_test plus new tests: 5-table merge, referenced-speaker delete kept, scope=auto rename and undo

#### W2-17 Stream-scoped live speakers and strict meeting-segment mapping (L)

- **Items:** audio-001, db-014, audio-008, audio-M01, audio-M05
- **Depends on:** W1-01
- **Specs:** specs/audio.md audio-001, 008, M01, M05; specs/db.md db-014

Live deltas/finals carry stream_id and session_speaker_id, persisted on meeting_transcript_segments (legacy rows read 'legacy'); grouping uses COALESCE(session_speaker_id, speaker_name). Per-chunk speaker backfill becomes one guarded UPDATE; meeting segments map to a global speaker only on exact instant/device/direction/text with a single distinct speaker. Timeline audio rows expose is_live and unique negative audio_chunk_id.

Owner paths:
- `crates/screenpipe-audio/src/meeting_streaming/events.rs`
- `crates/screenpipe-audio/src/meeting_streaming/deepgram_live.rs`
- `crates/screenpipe-audio/src/meeting_streaming/controller.rs`
- `crates/screenpipe-db/src/db.rs (meeting_transcript_segments insert/backfill and timeline audio region)`
- `crates/screenpipe-audio/src/audio_manager/reconciliation.rs (backfill call site only)`
- `crates/screenpipe-engine/src/routes/streaming.rs (timeline audio rows)`

Acceptance:
- [ ] cargo test -p screenpipe-db: insert with stream_id/session_speaker_id; legacy default; guarded backfill skips ambiguous chunks; strict mapping skips identical-text twins
- [ ] Harness: timeline stream rows for a live meeting have is_live=1 and distinct negative audio_chunk_id values

#### W2-18 Windows WASAPI loopback engine and exe-based audio exclusions (L)

- **Items:** audio-013, audio-014
- **Depends on:** W1-02
- **Specs:** specs/audio.md audio-013, 014

Own event-driven WASAPI loopback client for the default render endpoint, plus per-process include/exclude taps through ActivateAudioInterfaceAsync on build >= 20348, with rebuild on default-endpoint change, target pid watch, activation/start timeouts and multi-root fallback. audio-exclusions.json applies on Windows through exclude-process-tree for the first running excluded exe; .exe metadata/icon reader; tap availability reports true on supported builds.

Owner paths:
- `crates/screenpipe-audio/src/core/process_tap.rs`
- `crates/screenpipe-audio/src/core/wasapi_loopback.rs (new, cfg windows)`
- `crates/screenpipe-audio/src/core/stream.rs (backend selection region)`
- `crates/screenpipe-audio/Cargo.toml (windows crate features)`
- `apps/screenpipe-app-tauri/src-tauri/src/audio_exclusions.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/permissions.rs (check_coreaudio_process_tap_available)`
- `apps/screenpipe-app-tauri/components/settings/recording-settings.tsx (.exe picker filter line)`

Acceptance:
- [ ] Manual: play audio in a browser; output chunks appear (sqlq on audio_chunks with the output device)
- [ ] Manual: exclude chrome.exe in audio-exclusions.json; its audio is absent while another app's audio is captured
- [ ] Switching the default output device rebuilds the stream (log line); cargo build for non-Windows cfg still type-checks (cargo check --target x86_64-unknown-linux-gnu -p screenpipe-audio if toolchain allows)

#### W2-19 GPU runtimes and bundled models: Vulkan whisper, DirectML, install layout (L)

- **Items:** audio-019, audio-020, audio-022, shell-029
- **Depends on:** W0-12
- **Specs:** specs/audio.md audio-019, 020, 022; specs/shell.md shell-029, shell-030 (merged)

Compile whisper with ggml-vulkan on Windows and ship vulkan-1.dll; a failed GPU state or inference rebuilds a shared CPU context and retries once. Parakeet/Qwen3 pick a non-software, non-remote D3D12 adapter, honour SCREENPIPE_DIRECTML, recycle DML sessions and retry on CPU (implemented in Freenpipe code unless the newer audiopipe revision's licence is cleared). Bundle segmentation, embedding and VAD models plus DirectML/ORT/OpenBLAS/Vulkan DLLs; resolve models from the exe dir, then cache, then download.

Owner paths:
- `crates/screenpipe-audio/Cargo.toml (features region)`
- `crates/screenpipe-audio/src/transcription/engine.rs`
- `crates/screenpipe-audio/src/transcription/whisper/model.rs`
- `crates/screenpipe-audio/src/transcription/whisper/batch.rs`
- `crates/screenpipe-audio/src/transcription/parakeet_windows.rs (new)`
- `crates/screenpipe-audio/src/speaker/models.rs (resolution order)`
- `crates/screenpipe-audio/src/vad/silero.rs (path resolution only)`
- `crates/screenpipe-audio/src/models/download.rs`
- `apps/screenpipe-app-tauri/src-tauri/tauri.windows.conf.json`
- `apps/screenpipe-app-tauri/src-tauri/windows/hooks.nsh`
- `apps/screenpipe-app-tauri/src-tauri/Cargo.toml (feature flags)`
- `.github/workflows/release-app.yml (Vulkan SDK step)`

Acceptance:
- [ ] Release-dev build with the vulkan and directml features succeeds (Vulkan SDK installed per docs/dev/windows-build.md)
- [ ] Transcribing a fixture wav logs Vulkan; forcing a GPU failure via test env falls back to CPU and still returns text
- [ ] SCREENPIPE_DIRECTML=0 forces CPU for parakeet (log); unit test: model resolution prefers the exe-dir copy

#### W2-20 Connections core: secret-safe reads, webhook proxy, Telegram send, router deltas, MCP refs (L)

- **Items:** connections-secret-redaction, connections-webhook-proxy, connections-telegram-send, api-031, connections-supports-oauth-instances, connections-pipe-mcp-connection-refs, cloud-038, api-M03
- **Depends on:** W1-03
- **Specs:** specs/connections.md secret-redaction, webhook-proxy, telegram-send, supports-oauth-instances, pipe-mcp-connection-refs; specs/api.md api-031, api-M03; specs/cloud.md cloud-038

Connection reads return only non-secret fields (OAuth instances listed); render_context prints proxy/config/webhook/endpoint pointers instead of raw secrets. POST /connections/:id/proxy forwards JSON to stored https webhooks (discord, zapier, make, n8n, teams) with the typed error codes; POST /connections/telegram/send {text}. Expose supports_oauth_instances. Pipe connections may reference mcp:<server_id>, satisfied by an enabled MCP server (plus token when OAuth). Keep base Gmail routes (target dropped them for a vendor broker); never ship Composio URLs.

Owner paths:
- `crates/screenpipe-engine/src/connections_api.rs (GET /:id and /:id/instances handlers)`
- `crates/screenpipe-engine/src/connections_routes/webhook_proxy.rs`
- `crates/screenpipe-engine/src/connections_routes/telegram.rs`
- `crates/screenpipe-connect/src/connections/mod.rs (render_context, IntegrationDef flags)`
- `crates/screenpipe-connect/src/connections/discord.rs`
- `crates/screenpipe-connect/src/connections/zapier.rs`
- `crates/screenpipe-connect/src/connections/make.rs`
- `crates/screenpipe-connect/src/connections/teams.rs`
- `crates/screenpipe-connect/src/connections/telegram.rs`
- `crates/screenpipe-core/assets/skills/screenpipe-api/SKILL.md (Connections section)`
- `crates/screenpipe-engine/src/bin/screenpipe-engine.rs (pipe connection gate)`
- `apps/screenpipe-app-tauri/components/post-install-connections-modal.tsx`
- `apps/screenpipe-app-tauri/components/settings/pipes-section.tsx (connection refs only)`

Acceptance:
- [ ] Harness: `api /connections/telegram` shows no bot_token field after connecting Telegram
- [ ] POST /connections/discord/proxy with a stored http:// webhook returns 400 invalid_webhook_target; ?x=1 returns unsupported_query
- [ ] POST /connections/telegram/send {text:''} returns 400; base /connections/gmail/* routes still respond
- [ ] cargo test: a pipe declaring connections [mcp:foo] is runnable when mcp server foo is enabled and stays in setup mode when it is disabled

#### W2-21 OAuth v2 flow and new OAuth integrations (L)

- **Items:** connections-oauth-connect-v2, connections-oauth-default-slot-promotion, connections-oauth-scope-tweaks, connections-hubspot-oauth, connections-zendesk-oauth, connections-outlook-email
- **Depends on:** W1-03
- **Specs:** specs/connections.md oauth-connect-v2, default-slot-promotion, scope-tweaks, hubspot-oauth, zendesk-oauth, outlook-email

oauth_connect(integrationId, instance, variant) with named scope variants, {subdomain} templating, a 10-minute window overridable by SCREENPIPE_OAUTH_TIMEOUT_SECS, provider error handling and distinct expired/used/failed pages, and activate_app_after_oauth. Promote shadowed default-slot accounts with a distinct identity instead of deleting them. GitHub scope 'repo read:org', Microsoft prompt=select_account. HubSpot and Zendesk (per-subdomain) gain OAuth with token fallbacks; new outlook-email integration via Graph.

Owner paths:
- `crates/screenpipe-connect/src/oauth.rs (flow, variants, slots)`
- `apps/screenpipe-app-tauri/src-tauri/src/oauth.rs`
- `crates/screenpipe-connect/src/connections/hubspot.rs`
- `crates/screenpipe-connect/src/connections/zendesk.rs`
- `crates/screenpipe-connect/src/connections/github_issues.rs`
- `crates/screenpipe-connect/src/connections/microsoft365.rs`
- `crates/screenpipe-connect/src/connections/teams.rs`
- `crates/screenpipe-connect/src/connections/outlook_email.rs (new)`
- `crates/screenpipe-connect/src/connections/mod.rs (registry lines only)`

Acceptance:
- [ ] cargo test -p screenpipe-connect: unknown variant errors; timeout env respected; default-slot promotion to oauth:<id>:<identity>
- [ ] Harness: `api /connections` lists outlook-email and hubspot with an OAuth flag
- [ ] Manual with BYO credentials: connect Outlook mail and read the unread count via the test endpoint

#### W2-22 Pipe config v2: new frontmatter, schedule_config and preview, ids, preamble (L)

- **Items:** agents_pipes-003, agents_pipes-004, agents_pipes-m03, agents_pipes-m05
- **Depends on:** W1-03
- **Specs:** specs/agents_pipes.md agents_pipes-003, 004, m03, m05

Typed pipe.md fields effort, history, run_in, artifacts (with title), schedule_config, cloud_agent, trigger.sources, event_read_only; PipeStatus has_saved_context/is_bundled_builtin/next_run/execution_count; per-field config validation, config-envelope rejection, managed-pipe locks. Structured ScheduleConfig with POST /pipes/schedule/preview {summary, next[]}. Pipe ids must be one portable path component. Add the four preamble directives in our own words.

Owner paths:
- `crates/screenpipe-core/src/pipes/mod.rs (PipeConfig, PipeStatus, parse_schedule/should_run, preamble regions)`
- `crates/screenpipe-core/src/pipes/schedule_config.rs (new)`
- `crates/screenpipe-engine/src/pipes_api.rs (config update validation, /pipes/schedule/preview)`

Acceptance:
- [ ] cargo test -p screenpipe-core pipes::: frontmatter with every new field round-trips; '../x' is rejected as an invalid pipe identifier
- [ ] Harness: POST /pipes/schedule/preview {frequency:'days',interval:1,at_hour:9,at_minute:0} returns a summary and the next run times
- [ ] POST /pipes/<id>/config with an unknown field returns 400

#### W2-23 Pi runtime 0.84.1, isolated config, baseline bun, compaction patch (L)

- **Items:** agents_pipes-015
- **Depends on:** W1-03
- **Specs:** specs/agents_pipes.md agents_pipes-015

Pin pi-coding-agent/pi-ai 0.84.1 with overrides, seed an isolated pi-config from the global one (with the migrated marker), keep the models.json merge rules, select a baseline bun on CPUs without AVX2, add the bash probe and a proactive compaction patch with summary validation (behaviour paraphrased, code original), PI_*/SCREENPIPE_PI_AGENT_DIR env.

Owner paths:
- `crates/screenpipe-core/src/agents/pi.rs (runtime install and seeding region)`
- `apps/screenpipe-app-tauri/src-tauri/src/pi.rs (runtime/env region)`
- `apps/screenpipe-app-tauri/src-tauri/assets/pi-patches/ (new)`

Acceptance:
- [ ] Fresh data dir: the installed package.json pins 0.84.1 and pi-config contains the .migrated-from-global marker
- [ ] UI smoke: a chat turn with a local Ollama preset completes; forcing the non-AVX2 path selects the baseline bun (log)

#### W2-24 Engine local stubs: attribution headers, telemetry no-op routes, local data-sync (M)

- **Items:** api-012, agents_pipes-038, cloud-040, api-026, cloud-037
- **Depends on:** W1-03
- **Specs:** specs/api.md api-012, api-026; specs/agents_pipes.md agents_pipes-038; specs/cloud.md cloud-037, cloud-040

Accept X-Screenpipe-Client/X-Screenpipe-Agent for tracing only; POST /internal/telemetry/mcp-call and mcp-value return 204 and record nothing; /data-sync/devices and /data-sync/search answer from the local device and local search; set_activity_sync_enabled only stores the flag. No network.

Owner paths:
- `crates/screenpipe-engine/src/routes/request_origin.rs`
- `crates/screenpipe-engine/src/routes/internal_telemetry.rs`
- `crates/screenpipe-engine/src/routes/data_sync_proxy.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/data_sync.rs (new)`

Acceptance:
- [ ] Harness: `api /internal/telemetry/mcp-call -X POST -d '{}' -i` returns 204
- [ ] `api /data-sync/devices` returns the local device; `api '/data-sync/search?q=test&limit=5'` returns local results
- [ ] A request with X-Screenpipe-Client: claude-code shows the client in the trace log

#### W2-25 Frontend stack refresh and browser-dev mock runtime (L)

- **Items:** frontend-001, frontend-004
- **Depends on:** W1-02
- **Specs:** specs/frontend.md frontend-001, frontend-004

Next 15.5, React 19 (stable release preferred over the target's canary), @xyflow/react, Space Grotesk heading font as a real @font-face, mermaid as a static asset, an i18n runtime for W4-20; PostHog flag gates become local defaults. Optional dev-only Tauri mock (NEXT_PUBLIC_SCREENPIPE_WEB_DEV=mock|live) so pages render in a browser.

Owner paths:
- `apps/screenpipe-app-tauri/package.json`
- `apps/screenpipe-app-tauri/bun.lock`
- `apps/screenpipe-app-tauri/next.config.mjs`
- `apps/screenpipe-app-tauri/app/layout.tsx`
- `apps/screenpipe-app-tauri/app/globals.css`
- `apps/screenpipe-app-tauri/components/rewind/mermaid-diagram.tsx`
- `apps/screenpipe-app-tauri/public/vendor/mermaid.min.js (new)`
- `apps/screenpipe-app-tauri/lib/dev/tauri-mock/** (new)`

Acceptance:
- [ ] cd apps/screenpipe-app-tauri && bun install && bun run build && bun test && bunx tsc --noEmit all pass
- [ ] NEXT_PUBLIC_SCREENPIPE_WEB_DEV=mock bun run dev renders /home in a browser without Tauri
- [ ] UI smoke under `bun run tauri dev`: home, settings, chat and timeline render without console errors

#### W2-26 App engine supervision: auto-respawn, port-conflict classification, safe relaunch (L)

- **Items:** shell-012, shell-013
- **Depends on:** W1-02
- **Specs:** specs/shell.md shell-012, shell-013

Respawn the embedded engine after consecutive unreachable checks while recording is intended (rate-limited). Classify API-port holders via netstat (self, healthy screenpipe-family, unhealthy, foreign) and only ever kill an unhealthy instance of our own executable; never a foreign or healthy process. One bounded teardown (capture stop, HTTP drain, audio shutdown, pool close, Pi stop) with force-exit; safe relaunch with SCREENPIPE_RELAUNCH_HOME_VISIBLE/SCREENPIPE_DATA_DIR; restart_after_screen_recording_permission.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/health.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/recording.rs (port conflict and restart region)`
- `apps/screenpipe-app-tauri/src-tauri/src/server.rs (port-kill region)`
- `apps/screenpipe-app-tauri/src-tauri/src/port_conflict.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/relaunch.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/main.rs (RunEvent::Exit region)`

Acceptance:
- [ ] cargo test (app crate) port_conflict: netstat fixtures classify self/healthy/unhealthy/foreign; foreign is never selected for kill
- [ ] UI smoke: kill the engine thread via a debug command; it is respawned within the configured checks
- [ ] Quit completes and the process exits within the teardown budget (manual timing)

#### W2-27 App startup hardening: argv helper modes, autostart, focus handoff, env, AVX2, WER, opener (L)

- **Items:** shell-017, shell-019, shell-020, shell-021, shell-022, shell-024, shell-028
- **Depends on:** W1-03
- **Specs:** specs/shell.md shell-017, 019..022, 024, 028

Early argv dispatch table for --screenpipe-acp-runtime, --screenpipe-acp-process-guard, --screenpipe-portable-mcp (and later --render-workflow-video) before single-instance work. /focus payload adds launch_exe/launchd_job_label and rejects other executables or schemes; duplicate --autostart handoffs ignored; :11435 rejects non-local Origin/Host. Autostart registers --autostart and starts in the background. SCREENPIPE_SKIP_ONBOARDING, runtime fs/asset scope for custom data dirs, AVX2 compatibility mode, WER runtime-exception helper with local crash-dumps harvest (never uploaded), opener allows claude://, cursor://, codex://.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/main.rs (early argv dispatch and startup region)`
- `apps/screenpipe-app-tauri/src-tauri/src/server.rs (/focus, Origin/Host checks)`
- `apps/screenpipe-app-tauri/src-tauri/src/commands.rs (autostart)`
- `apps/screenpipe-app-tauri/src-tauri/src/config.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/cpu_compat.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/wer.rs (new)`
- `crates/screenpipe-wer-helper/ (new cdylib, cfg windows)`
- `apps/screenpipe-app-tauri/src-tauri/capabilities/main.json (opener schemes)`

Acceptance:
- [ ] Launching the built app with --autostart does not show the Home window (UI smoke)
- [ ] curl -X POST http://127.0.0.1:11435/focus with a different launch_exe returns 403; a request with Origin: http://evil.example returns 403
- [ ] App started with --screenpipe-portable-mcp exits through the dispatch path before single-instance (stub usage message)
- [ ] After a forced crash in a test build, a .dmp appears under <data dir>/crash-dumps on next launch

### Wave 3: Feature backends (search, pipes, connections, capture, audio)

| WP | Size | Title | Depends on | Items |
|---|---|---|---|---|
| W3-01 | L | Semantic projection worker and actors API | W2-05, W2-06 | meetings_memory-022, meetings_memory-023, api-008 |
| W3-02 | L | Search v2: parsed content, starred, actor and related filters | W2-04, W2-05, W2-09 | api-003, meetings_memory-011, meetings_memory-024 |
| W3-03 | L | Element API formats and state | W2-09 | api-004, vision-elements-formats |
| W3-04 | L | Activity APIs: activity-summary v2 and activity-ledger route | W2-04, W2-07 | api-005, meetings_memory-018, meetings_memory-015, api-006 |
| W3-05 | L | Structured output targets | W2-02, W2-03 | api-019, meetings_memory-005, agents_pipes-011 |
| W3-06 | L | Event-trigger idempotency, run outcome verification, meeting-summary finalizer | W2-08, W2-22 | db-009, meetings_memory-028, agents_pipes-001, agents_pipes-002, meetings_memory-027, api-039, agents_pipes-m01 |
| W3-07 | M | Live meeting overlay websocket | W2-17, W2-13 | api-015, meetings_memory-033 |
| W3-08 | L | UI-scan meeting detector: calendar binding and lock awareness | W2-08 | meetings_memory-031, gap_health-lifecycle-internals-013 |
| W3-09 | L | Retention v2: lean mode, strip, NULL-safe deletes, coverage | W2-05, W2-11 | gap_retention-v2-engine-001, gap_retention-v2-engine-002, gap_retention-v2-engine-003, gap_retention-v2-engine-004, gap_retention-v2-engine-005, gap_retention-v2-engine-006, gap_retention-v2-engine-007, gap_retention-v2-engine-009, gap_retention-v2-engine-010, gap_retention-v2-engine-011 |
| W3-10 | L | PII redaction surfaces v2 | W1-01, W1-02 | db-003, db-004, vision-redaction-surfaces |
| W3-11 | L | Capture loop hardening A: stages and wedge watchdog, probe timeouts, fail-closed exclusions, corrupt frames | W2-11, W2-13 | vision-capture-wedge-watchdog, vision-probe-timeouts, vision-exclusion-unknown-fail-closed, vision-corrupt-frame-guard |
| W3-12 | M | Dedicated Windows OCR worker and OCR availability reporting | W1-01, W2-10 | vision-windows-ocr-worker |
| W3-13 | L | URL domain rules, Windows window identity, input-hook watchdog | W1-02, W2-11 | vision-url-domain-rules, vision-windows-window-resolution, a11y-input-hook-watchdog |
| W3-14 | L | Audio capture modes and device status events | W1-02, W2-13 | audio-010, audio-026, audio-018 |
| W3-15 | L | Software AEC3 + NS + AGC2 (aecMode=screenpipe) | W2-18 | audio-012 |
| W3-16 | L | Reconciliation durability, identification gating, STT retries, dedup | W2-17 | audio-007, audio-023, audio-024, audio-M04 |
| W3-17 | L | Startup health diagnostics (health-startup.json) and ORT init watchdog | W2-13, W2-19 | shell-M01, audio-M06, audio-021 |
| W3-18 | L | Notifications v2 backend on :11435 | W1-02 | api-037, meetings_memory-037, meetings_memory-038 |
| W3-19 | L | Slack user-token integration with read routes | W2-21 | connections-slack-user-token |
| W3-20 | L | IMAP read-only inbox integration | W2-20 | connections-imap |
| W3-21 | L | BYO MCP: registry search, MCP-OAuth-first tiles, session allow-gate, relay-free OAuth | W2-20 | connections-mcp-oauth-tiles, api-032, api-M01, agents_pipes-032, api-M02 |
| W3-22 | M | Local integrations: memory-sync guidance, Obsidian memories, Hermes, WhatsApp sign-out | W2-20 | meetings_memory-035, connections-hermes, connections-whatsapp-signout |
| W3-23 | L | Pi extension set expansion and extension packages | W2-23 | agents_pipes-014, frontend-016 |
| W3-24 | L | Agent self-improvement: skills store, user profile, learning | W1-03 | agents_pipes-021, agents_pipes-022, agents_pipes-023, api-021 |
| W3-25 | L | Local OpenAI-compatible proxy and BYO web search | W1-03 | agents_pipes-033, api-023, api-025, agents_pipes-035 |
| W3-26 | M | Connect broker and connection-gate tools | W2-20 | agents_pipes-043, api-022 |
| W3-27 | L | SQLite hard-fault quarantine coordinator | W2-01 | db-019 |
| W3-28 | L | Headless mode and search-only mode | W2-26 | shell-004, shell-005, api-034 |
| W3-29 | M | Recording health incident state | W2-26 | shell-011 |
| W3-30 | M | Low-disk recording guard and keep-awake | W1-02, W1-03 | shell-006, shell-007 |
| W3-31 | L | SOP narration providers: OpenAI-compatible, local OS TTS, captions-only | W1-02 | gap_workflow-sop-video-and-media-narration-decloud |

#### W3-01 Semantic projection worker and actors API (L)

- **Items:** meetings_memory-022, meetings_memory-023, api-008
- **Depends on:** W2-05, W2-06
- **Specs:** specs/meetings_memory.md meetings_memory-022, 023; specs/api.md api-008

Worker parses frames when enableSemanticContext is on (default off; modes memory/computer_use/both), reuses runs by input fingerprint, stores items/run items, assigns actors and batches frame attachments; failures never affect capture. Six /semantic/actors/* routes (search, create, update, merge, reassign, aliases/reassign) returning item_count and aliases.

Owner paths:
- `crates/screenpipe-engine/src/semantic_worker.rs (new)`
- `crates/screenpipe-engine/src/routes/semantic.rs`

Acceptance:
- [ ] With enableSemanticContext=false no semantic_runs rows appear; with it on and a VS Code window focused, rows appear and frames.semantic_run_id is set
- [ ] Harness: `api '/semantic/actors/search?q=a'`; POST create, merge and reassign change rows as specified (sqlq)

#### W3-02 Search v2: parsed content, starred, actor and related filters (L)

- **Items:** api-003, meetings_memory-011, meetings_memory-024
- **Depends on:** W2-04, W2-05, W2-09
- **Specs:** specs/api.md api-003; specs/meetings_memory.md meetings_memory-011, 024

New params starred_session_id, starred_only, input_context_only, frame_id, actor_id, include_related, fields, format; content_type=parsed returns Parsed items (run/parser metadata, compact text, typed items, actors) and is excluded from all; hits carry starred; related map; starred filters apply before pagination; 30 s timeout and busy errors.

Owner paths:
- `crates/screenpipe-engine/src/routes/search.rs`
- `crates/screenpipe-db/src/db.rs (search query region)`
- `crates/screenpipe-db/src/types.rs (ContentType Parsed/Memory, SearchQuery)`
- `crates/screenpipe-db/src/ext/search_parsed.rs (new)`

Acceptance:
- [ ] Harness: `api '/search?content_type=parsed&q=x'` returns Parsed items; content_type=all returns none of them
- [ ] starred_only=true returns only hits inside starred sessions and each hit has starred:true
- [ ] include_related=true adds the related map; actor_id filters parsed items

#### W3-03 Element API formats and state (L)

- **Items:** api-004, vision-elements-formats
- **Depends on:** W2-09
- **Specs:** specs/api.md api-004; specs/vision.md elements-formats

ElementResponse.state {focused, selected, expanded}; format=outline (text/plain, x-total-count, no-store), format=automation (revision r_, response refs eN, stable keys k_ with confidence, bounds, allowed actions by role class), format=preferred following semanticContextMode; limit/offset/fields on /frames/:id/elements.

Owner paths:
- `crates/screenpipe-engine/src/routes/elements.rs`
- `crates/screenpipe-engine/src/elements_automation.rs (new)`
- `crates/screenpipe-db/src/db.rs (elements query region)`

Acceptance:
- [ ] Harness: `api '/elements?format=outline&limit=20' -i` returns text/plain with x-total-count
- [ ] `api '/frames/<id>/elements?format=automation'` returns refs eN and keys k_; `?limit=10&offset=10` pages
- [ ] cargo test: automation key stability across two identical snapshots

#### W3-04 Activity APIs: activity-summary v2 and activity-ledger route (L)

- **Items:** api-005, meetings_memory-018, meetings_memory-015, api-006
- **Depends on:** W2-04, W2-07
- **Specs:** specs/api.md api-005, api-006; specs/meetings_memory.md meetings_memory-015, 018

/activity-summary adds include_apps/windows/key_texts/parsed_count/starred, starred_sessions (<= 10, has_more), parsed_context_count and app attribution from ui_events (native/recovered/unresolved). GET /activity-ledger start/end (<= 31 days), depth task|action, include_artifacts, refresh returns intervals with evidence.

Owner paths:
- `crates/screenpipe-engine/src/routes/activity_summary.rs`
- `crates/screenpipe-engine/src/routes/activity_ledger.rs`

Acceptance:
- [ ] Harness: `api '/activity-summary?start_time=today&include_apps=false'` omits apps and reports attribution counts
- [ ] `api '/activity-ledger?start_time=today&depth=task'` returns intervals; a 32-day range returns 400

#### W3-05 Structured output targets (L)

- **Items:** api-019, meetings_memory-005, agents_pipes-011
- **Depends on:** W2-02, W2-03
- **Specs:** specs/api.md api-019; specs/meetings_memory.md meetings_memory-005; specs/agents_pipes.md agents_pipes-011

Typed targets in structured-outputs/targets.json bound to pipes: GET /outputs/targets?pipe=, POST create, POST /:target_id/submit (revision check, JSON-schema subset validation, evidence <= 50), /feedback (into the feedback table) and /items/:item_id/actions (list.v1 dispositions); values stored as structured-output SAF artifacts; structured_output pi tool.

Owner paths:
- `crates/screenpipe-engine/src/routes/structured_outputs.rs`
- `crates/screenpipe-engine/src/structured_outputs/ (new: store, schema validator)`
- `crates/screenpipe-core/assets/extensions/structured-output.ts (new)`
- `crates/screenpipe-core/src/agents/extensions_manifest.rs (entry)`

Acceptance:
- [ ] Harness: create a target, submit with the current revision (200) and a stale one (409); a schema-violating payload returns 400; 51 evidence refs return 400
- [ ] POST /outputs/targets/<t>/items/<i>/actions {action:'resolve'} updates the disposition; feedback appears in GET /feedback

#### W3-06 Event-trigger idempotency, run outcome verification, meeting-summary finalizer (L)

- **Items:** db-009, meetings_memory-028, agents_pipes-001, agents_pipes-002, meetings_memory-027, api-039, agents_pipes-m01
- **Depends on:** W2-08, W2-22
- **Specs:** specs/db.md db-009; specs/meetings_memory.md 027, 028; specs/agents_pipes.md 001, 002, m01; specs/api.md api-039

Claim (pipe, event, key) in pipe_event_runs with INSERT OR IGNORE before event runs, release on cancel, prune by age; meeting_ended keyed by meeting id survives the rejoin window. pipe_executions records trigger_event/trigger_key/conversation_id with a guarded running transition and keyset paging. A clean exit without a verified final result fails; refusal and compaction failures are classified. The finalizer recovers an unsaved '## Summary' from output or marks summary_not_saved/nothing_to_summarize, requeues interrupted runs and skips trivial meetings. Retranscribe returns transcript_updated_at and summary_refresh_event_emitted.

Owner paths:
- `crates/screenpipe-core/src/pipes/mod.rs (event dispatch and execution recording regions)`
- `crates/screenpipe-core/src/pipes/hooks.rs`
- `crates/screenpipe-core/src/pipes/meeting_summary/ (new: extract, finalizer)`
- `crates/screenpipe-engine/src/pipe_store.rs`
- `crates/screenpipe-db/src/ext/pipe_events.rs`
- `crates/screenpipe-engine/src/routes/retranscribe.rs`

Acceptance:
- [ ] cargo test -p screenpipe-core: two meeting_ended events for one meeting id run the pipe once; cancelling a queued run releases the claim
- [ ] sqlq: pipe_executions rows carry trigger_event and trigger_key
- [ ] Unit: finalizer extracts a '## Summary' block from fixture stdout; a run with no final result is marked failed
- [ ] Harness: POST /meetings/<id>/retranscribe response contains transcript_updated_at and summary_refresh_event_emitted

#### W3-07 Live meeting overlay websocket (M)

- **Items:** api-015, meetings_memory-033
- **Depends on:** W2-17, W2-13
- **Specs:** specs/api.md api-015; specs/meetings_memory.md meetings_memory-033

/ws/meeting-overlay emits {type: status|snapshot|item, data} for the active meeting with live transcript items {meetingId,itemId,deviceName,deviceType,speakerName,capturedAt,text,isFinal}; /ws/metrics adds audio.mic_capture_active. The overlay UI is W6-19.

Owner paths:
- `crates/screenpipe-engine/src/routes/websocket.rs`
- `crates/screenpipe-engine/src/routes/meeting_overlay.rs`

Acceptance:
- [ ] websocat ws://127.0.0.1:3035/ws/meeting-overlay receives a status then a snapshot message
- [ ] During a manual meeting with live transcription, item messages arrive with isFinal flags; /ws/metrics payload has audio.mic_capture_active

#### W3-08 UI-scan meeting detector: calendar binding and lock awareness (L)

- **Items:** meetings_memory-031, gap_health-lifecycle-internals-013
- **Depends on:** W2-08
- **Specs:** specs/meetings_memory.md meetings_memory-031; specs/gap_health-lifecycle-internals.md 013

Calendar matches bind only on exact conference URL identity through the conditional calendar_event_id update (time-only matches do not bind), with an ownership check; excludeFromMeetingDetection; meeting_ended carries transcript_segments. Meeting scan loops skip iterations while the screen is locked.

Owner paths:
- `crates/screenpipe-engine/src/meeting_detector.rs`
- `crates/screenpipe-engine/src/meeting_watcher.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/google_calendar.rs (binding region)`
- `apps/screenpipe-app-tauri/src-tauri/src/calendar.rs`

Acceptance:
- [ ] cargo test -p screenpipe-engine meeting_detector: exact-URL match binds, time-only match does not, excluded app is ignored
- [ ] Unit: with SCREEN_IS_LOCKED set the scan iteration is skipped without state change

#### W3-09 Retention v2: lean mode, strip, NULL-safe deletes, coverage (L)

- **Items:** gap_retention-v2-engine-001, gap_retention-v2-engine-002, gap_retention-v2-engine-003, gap_retention-v2-engine-004, gap_retention-v2-engine-005, gap_retention-v2-engine-006, gap_retention-v2-engine-007, gap_retention-v2-engine-009, gap_retention-v2-engine-010, gap_retention-v2-engine-011
- **Depends on:** W2-05, W2-11
- **Specs:** specs/gap_retention-v2-engine.md 001-007, 009-011

RetentionMode Media/Lean/All; lean batches evict media then strip heavy text in one transaction (delete elements with anchor re-homing, NULL accessibility_tree_json/text_json, delete ui_events; keep full_text and memories). Checked cutoff arithmetic, 1..36500 day validation, oldest-in-loop, shared unlink helper, post-loop policy. Fix the base NULL-unsafe chunk subqueries. delete-range drops ocr_deleted and keeps the newest display_layout row. Semantic GC after frame deletion. App auto-start accepts lean; fresh-store guard keeps retention off when recordings exist. CLI accepts lean.

Owner paths:
- `crates/screenpipe-engine/src/retention.rs`
- `crates/screenpipe-db/src/db.rs (evict/delete_time_range/orphan cleanup region)`
- `crates/screenpipe-db/src/ext/retention_strip.rs`
- `crates/screenpipe-engine/src/routes/data.rs (delete-range response)`
- `apps/screenpipe-app-tauri/src-tauri/src/sync.rs (auto_start_retention region)`
- `crates/screenpipe-engine/src/cli/mod.rs (retention flags)`
- `crates/screenpipe-engine/src/bin/screenpipe-engine.rs (banner)`

Acceptance:
- [ ] Harness: POST /retention/configure {retention_days:36501} returns 400 with a message containing `at most 36500` (exact string, client-visible); {mode:'lean'} is echoed by /retention/status
- [ ] cargo test -p screenpipe-db: lean strip removes elements and ui_events, NULLs tree/text_json, keeps full_text; a chunk referenced with NULL ids is now evicted and marked
- [ ] POST /data/delete-range response has no ocr_deleted; display_layout keeps the newest row in range

#### W3-10 PII redaction surfaces v2 (L)

- **Items:** db-003, db-004, vision-redaction-surfaces
- **Depends on:** W1-01, W1-02
- **Specs:** specs/db.md db-003, db-004; specs/vision.md redaction-surfaces

Redaction worker covers elements text and properties, frames.full_text with propagation of detections to derived copies (accessibility tree, window_name, browser_url, text_json) through a redaction map, and multi-column ui_events; watermarks use the partial indexes; piiRedactionColumns selects surfaces.

Owner paths:
- `crates/screenpipe-redact/src/worker/mod.rs`
- `crates/screenpipe-redact/src/worker/tables.rs`
- `crates/screenpipe-redact/src/redaction_map.rs (new)`
- `crates/screenpipe-redact/tests/**`

Acceptance:
- [ ] cargo test -p screenpipe-redact: elements text redacted and redacted_at set; a full_text detection propagates to window_name/browser_url/text_json
- [ ] piiRedactionColumns subset is respected; EXPLAIN QUERY PLAN for the watermark scan uses the partial index

#### W3-11 Capture loop hardening A: stages and wedge watchdog, probe timeouts, fail-closed exclusions, corrupt frames (L)

- **Items:** vision-capture-wedge-watchdog, vision-probe-timeouts, vision-exclusion-unknown-fail-closed, vision-corrupt-frame-guard
- **Depends on:** W2-11, W2-13
- **Specs:** specs/vision.md capture-wedge-watchdog, probe-timeouts, exclusion-unknown-fail-closed, corrupt-frame-guard

Loops publish named stages and heartbeats; the watcher detects still-attempting wedges and silent stalls and restarts the VisionManager (local log only, no analytics). Visual-change probe and excluded-window enumeration run under timeouts; when exclusion state is unknown, frame pixels are skipped until a refresh succeeds while a11y capture continues. Reject green decode-garbage bands and count corrupt black/green frames.

Owner paths:
- `crates/screenpipe-engine/src/event_driven_capture.rs`
- `crates/screenpipe-engine/src/visual_probe.rs (new)`
- `crates/screenpipe-engine/src/capture_exclusions.rs (new)`
- `crates/screenpipe-engine/src/vision_manager/monitor_watcher.rs (wedge restart)`
- `crates/screenpipe-screen/src/utils.rs`

Acceptance:
- [ ] cargo test -p screenpipe-engine: a simulated hang in the capture stage triggers a VisionManager restart via a test hook
- [ ] Unit: exclusion refresh timeout leads to a11y-only frames (no snapshot written); green-band fixture is rejected and frames_corrupt_green increments
- [ ] Harness: `api /health` shows loop_stage and loop_stage_age_secs

#### W3-12 Dedicated Windows OCR worker and OCR availability reporting (M)

- **Items:** vision-windows-ocr-worker
- **Depends on:** W1-01, W2-10
- **Specs:** specs/vision.md windows-ocr-worker

Run WinRT OcrEngine on a long-lived 'screenpipe-windows-ocr' thread with a bounded queue shared by live and on-demand OCR; validate image dimensions and buffers; probe availability (recognizer languages, bundled tesseract) and surface vision_reason=ocr_unavailable.

Owner paths:
- `crates/screenpipe-screen/src/microsoft.rs`
- `crates/screenpipe-screen/src/tesseract.rs`
- `crates/screenpipe-capture/src/paired_capture.rs (OCR call region)`
- `crates/screenpipe-engine/src/routes/frames.rs (on-demand OCR handler only)`

Acceptance:
- [ ] Engine log names the screenpipe-windows-ocr thread; cargo test: invalid dimensions are rejected without panic
- [ ] With no OCR language installed (test override), `api /health` reports vision_reason ocr_unavailable

#### W3-13 URL domain rules, Windows window identity, input-hook watchdog (L)

- **Items:** vision-url-domain-rules, vision-windows-window-resolution, a11y-input-hook-watchdog
- **Depends on:** W1-02, W2-11
- **Specs:** specs/vision.md url-domain-rules, windows-window-resolution, a11y-input-hook-watchdog

ignoredUrls accepts legacy strings or DomainRule{domain, includeSubdomains, excludedSubdomains}; includedUrls restricts browser capture (unverifiable URL is not recorded); block beats allow; validation messages. Resolve UWP apps behind ApplicationFrameHost; add skip reasons for a frame blocked by the browser URL policy and for a URL whose validation is still pending. Dedicated input worker with a soft-capped queue, HIGHEST hook thread priority and a watchdog that reinstalls silently removed low-level hooks.

Owner paths:
- `crates/screenpipe-a11y/src/url_filter.rs`
- `crates/screenpipe-a11y/src/platform/windows.rs`
- `crates/screenpipe-a11y/src/tree/windows.rs (identity region)`
- `crates/screenpipe-a11y/src/events.rs (SkipReason)`
- `crates/screenpipe-screen/src/capture_screenshot_by_window.rs`
- `crates/screenpipe-engine/src/vision_manager/manager.rs (URL policy plumbing)`

Acceptance:
- [ ] cargo test -p screenpipe-a11y url_filter: subdomain include/exclude rules, allowlist rejection, block-beats-allow
- [ ] Manual: Windows Calculator frames carry the real app name rather than ApplicationFrameHost
- [ ] Unit: simulated hook silence with active input triggers reinstall (test hook)

#### W3-14 Audio capture modes and device status events (L)

- **Items:** audio-010, audio-026, audio-018
- **Depends on:** W1-02, W2-13
- **Specs:** specs/audio.md audio-010, 018, 026

audioCaptureMode always | meetings-only (streams open only in confirmed meetings as session devices, released afterwards, unattributed chunks dropped, tray waiting state) | disabled (audio never starts; routes return `Audio capture is disabled in settings` (exact string, client-visible)); restart required. Publish audio_device_status_changed on every start/stop, plus fallback_unavailable and capture_health events, forwarded by the app as kebab-case events with native notifications.

Owner paths:
- `crates/screenpipe-audio/src/audio_manager/manager.rs`
- `crates/screenpipe-audio/src/audio_manager/device_monitor.rs (session-device region)`
- `crates/screenpipe-engine/src/routes/audio.rs`
- `crates/screenpipe-events/src/custom_events/audio_devices.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/engine_events/audio_device.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/engine_events/mod.rs`
- `crates/screenpipe-engine/src/recording_config.rs (mode plumbing)`

Acceptance:
- [ ] Harness with audioCaptureMode=disabled: POST /audio/start returns the disabled error
- [ ] meetings-only: no audio chunks while idle; POST /meetings/start then speak; chunks appear and stop after /meetings/stop (sqlq)
- [ ] POST /audio/device/stop emits audio_device_status_changed on /ws/events

#### W3-15 Software AEC3 + NS + AGC2 (aecMode=screenpipe) (L)

- **Items:** audio-012
- **Depends on:** W2-18
- **Specs:** specs/audio.md audio-012

Clean mic audio at 16 kHz with the system-output stream as far-end reference, with drift handling, bypass and diagnostics; aecMode off|screenpipe|macos|windows is validated per platform and drives the three legacy toggles. Use a licence-cleared AEC3 port.

Owner paths:
- `crates/screenpipe-audio/src/core/aec.rs (new)`
- `crates/screenpipe-audio/src/core/run_record_and_transcribe.rs`
- `crates/screenpipe-audio/src/core/stream.rs (far-end tap region)`
- `crates/screenpipe-audio/src/audio_manager/builder.rs`
- `crates/screenpipe-audio/Cargo.toml (AEC dependency)`

Acceptance:
- [ ] cargo test -p screenpipe-audio aec: synthetic echo is attenuated by at least 20 dB; no far-end means bypass
- [ ] Settings validation rejects aecMode=macos on Windows

#### W3-16 Reconciliation durability, identification gating, STT retries, dedup (L)

- **Items:** audio-007, audio-023, audio-024, audio-M04
- **Depends on:** W2-17
- **Specs:** specs/audio.md audio-007, 023, 024, M04

Per-segment skip/match_only/match_and_learn by duration, top-2 lookup with ambiguity abstain, 400-sample minimum. Pending-chunk marker journal with startup recovery, PendingTranscription v2 with sha256-staged merged audio, refusal without a durable data dir. OpenAI-compatible 3 attempts with 429/transport backoff; Deepgram retries 429 with Retry-After then leaves the chunk pending. Dedup anchored to capture time, excludes the same device and fails open.

Owner paths:
- `crates/screenpipe-audio/src/audio_manager/reconciliation.rs`
- `crates/screenpipe-audio/src/transcription/transcription_result.rs`
- `crates/screenpipe-audio/src/speaker/prepare_segments.rs`
- `crates/screenpipe-audio/src/transcription/openai_compatible/batch.rs`
- `crates/screenpipe-audio/src/transcription/deepgram/batch.rs`
- `crates/screenpipe-db/src/db.rs (get_speaker_from_embedding, has_similar_recent_transcription region)`

Acceptance:
- [ ] cargo test -p screenpipe-audio: abstain when best and runner-up are within margin; pending marker replays after a simulated restart
- [ ] wiremock: a 429 with Retry-After is retried and then succeeds
- [ ] cargo test -p screenpipe-db: dedup ignores the same device and uses the capture-time window

#### W3-17 Startup health diagnostics (health-startup.json) and ORT init watchdog (L)

- **Items:** shell-M01, audio-M06, audio-021
- **Depends on:** W2-13, W2-19
- **Specs:** specs/shell.md shell-M01; specs/audio.md audio-021, M06

Write health-startup.json: phases, preflight checks (data_dir_writable, path_has_ffmpeg, env_overrides) with remedies, last_failed_start, ffmpeg/ffprobe probes, per-component states, degraded features, crash-dump helper status and a process snapshot. ORT and model sessions initialise on a watchdog thread with ok/init_failed/init_timed_out.

Owner paths:
- `crates/screenpipe-core/src/health_diagnostics.rs (new)`
- `crates/screenpipe-audio/src/speaker/mod.rs`
- `crates/screenpipe-audio/src/segmentation/segmentation_manager.rs`
- `crates/screenpipe-audio/src/vad/silero.rs (init region)`
- `crates/screenpipe-audio/src/ort_watchdog.rs (new)`
- `crates/screenpipe-engine/src/bin/screenpipe-engine.rs (phase markers)`

Acceptance:
- [ ] After engine start, <FP_DD>/health-startup.json exists with components transcription, vad, speaker_segmentation, speaker_embedding
- [ ] A corrupt VAD model fixture yields vad: init_failed; a test hook that hangs init yields init_timed_out

#### W3-18 Notifications v2 backend on :11435 (L)

- **Items:** api-037, meetings_memory-037, meetings_memory-038
- **Depends on:** W1-02
- **Specs:** specs/api.md api-037; specs/meetings_memory.md meetings_memory-037, 038

NotifyPayload v2 (priority, transient, pipe_name, workflow_ids, source ids/url, surface/position/anchor, cta, survey, dismissible, expires_at; action types link/deeplink/pipe/chat/api/dismiss/copy/source/mute/manage); delivery gates (master, snooze, quiet hours, allowDuringPause, categories, mutedPipes, pipe notify permission, duplicate suppression); x-screenpipe-session; history entries keep source, actions and priority; POST /notifications/:id/read.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/notifications/routes.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/notifications/store.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/notifications/rewrite.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/notifications/mod.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/server.rs (route table region)`

Acceptance:
- [ ] curl -X POST http://127.0.0.1:11435/notify with priority, transient and a chat action returns 200 and GET /notifications shows the new fields
- [ ] POST /notifications/<id>/read marks it read; an identical notify within the dedupe window is suppressed
- [ ] cargo test (app crate) notifications: quiet hours and snooze suppress delivery

#### W3-19 Slack user-token integration with read routes (L)

- **Items:** connections-slack-user-token
- **Depends on:** W2-21
- **Specs:** specs/connections.md slack-user-token

Slack acts as the user (user_scope): variant send = chat:write, read_write adds search/history/read scopes; /slack/send defaults to self-DM and accepts channel/blocks; GET /slack/search, /slack/conversations, /slack/history; instance = team_id; read routes return 403 without read_write.

Owner paths:
- `crates/screenpipe-connect/src/connections/slack.rs`
- `crates/screenpipe-engine/src/connections_routes/slack.rs`
- `apps/screenpipe-app-tauri/components/settings/connections-section.tsx (Slack panel region)`

Acceptance:
- [ ] cargo test -p screenpipe-connect slack: the read_write variant requests the read scopes
- [ ] wiremock: GET /connections/slack/search?q=x proxies search.messages; a send-only instance gets 403 on read routes

#### W3-20 IMAP read-only inbox integration (L)

- **Items:** connections-imap
- **Depends on:** W2-20
- **Specs:** specs/connections.md imap

New imap integration (host, port 993, username, app password) over rustls using EXAMINE only; GET /imap/messages?limit&mailbox&query, /imap/messages/:uid (parsed), /imap/mailboxes; app-password guidance on login failure.

Owner paths:
- `crates/screenpipe-connect/src/connections/imap.rs (new)`
- `crates/screenpipe-connect/Cargo.toml (async-imap, mail-parser)`
- `crates/screenpipe-engine/src/connections_routes/imap.rs`
- `crates/screenpipe-connect/src/connections/mod.rs (registry line)`

Acceptance:
- [ ] cargo test -p screenpipe-connect imap: message parsing and query building
- [ ] Manual with an app password: `api /connections/imap/mailboxes` returns mailboxes and `api '/connections/imap/messages?limit=5'` returns messages without marking them read

#### W3-21 BYO MCP: registry search, MCP-OAuth-first tiles, session allow-gate, relay-free OAuth (L)

- **Items:** connections-mcp-oauth-tiles, api-032, api-M01, agents_pipes-032, api-M02
- **Depends on:** W2-20
- **Specs:** specs/connections.md mcp-oauth-tiles; specs/api.md api-032, M01, M02; specs/agents_pipes.md agents_pipes-032

GET /mcp-servers/registry proxies registry.modelcontextprotocol.io (user-initiated). Vendor MCP servers become the primary connect path for linear/notion/jira/stripe/... tiles with native forms under Advanced; connected = native or matching enabled MCP server; /connections marks mcp/mcp_server_id. OAuth start accepts and ignores app_scheme and always uses the loopback callback. tools/call is refused (403 naming the server) when the server is not allowed for the calling pipe session (x-screenpipe-session, SCREENPIPE_MCP_SERVER_ALLOWLIST).

Owner paths:
- `crates/screenpipe-engine/src/mcp_servers_api.rs`
- `crates/screenpipe-connect/src/mcp_servers.rs`
- `crates/screenpipe-engine/src/mcp_access.rs (new)`
- `apps/screenpipe-app-tauri/components/settings/connections-section.tsx (MCP tiles region)`
- `apps/screenpipe-app-tauri/components/settings/custom-mcp-card.tsx`

Acceptance:
- [ ] Harness: `api '/mcp-servers/registry?search=github&limit=5'` returns {data:{servers,nextCursor}}
- [ ] POST /mcp-servers/<id>/oauth/start {app_scheme:'screenpipe'} returns an authorize URL whose redirect_uri is the localhost callback
- [ ] A tools/call carrying a pipe session that does not allow the server returns 403 naming it

#### W3-22 Local integrations: memory-sync guidance, Obsidian memories, Hermes, WhatsApp sign-out (M)

- **Items:** meetings_memory-035, connections-hermes, connections-whatsapp-signout
- **Depends on:** W2-20
- **Specs:** specs/meetings_memory.md meetings_memory-035; specs/connections.md hermes, whatsapp-signout

Claude Code and Codex receive static MCP recall guidance instead of raw memories (disconnect restores on failed cleanup); new obsidian-memories target writes a digest note with frontmatter after a .obsidian check and write probe; new hermes integration (endpoint default 127.0.0.1:8642, optional token, /v1/models test); WhatsApp logged-out disconnects wipe the session and ask to re-pair.

Owner paths:
- `crates/screenpipe-core/src/memories/external_sync.rs`
- `crates/screenpipe-engine/src/external_memory_sync.rs`
- `crates/screenpipe-connect/src/connections/obsidian_memories.rs (new)`
- `crates/screenpipe-connect/src/connections/hermes.rs (new)`
- `crates/screenpipe-connect/src/connections/claude_code.rs`
- `crates/screenpipe-connect/src/connections/codex.rs`
- `crates/screenpipe-connect/src/whatsapp/mod.rs`
- `crates/screenpipe-connect/src/whatsapp/gateway.mjs`
- `crates/screenpipe-connect/src/connections/mod.rs (registry lines)`

Acceptance:
- [ ] Harness: POST /memories/sync-external with obsidian-memories configured writes <vault>/screenpipe/screenpipe-memories.md with frontmatter
- [ ] The Claude Code sidecar file contains recall guidance and no memory text
- [ ] cargo test: hermes test call hits {endpoint}/v1/models (mock); WhatsApp logged_out path resets the session

#### W3-23 Pi extension set expansion and extension packages (L)

- **Items:** agents_pipes-014, frontend-016
- **Depends on:** W2-23
- **Specs:** specs/agents_pipes.md agents_pipes-014; specs/frontend.md frontend-016

Rename the agent tools to sp_* (with aliases), add the remaining extensions (fill_work_context, sp_mcp_read, workflow workspace scaffolding hooks) and pi_*_extension_package commands for npm:/GitHub sources with a safe mode; pi-subagents mandatory; answer extension UI requests via pi_extension_ui_response.

Owner paths:
- `crates/screenpipe-core/assets/extensions/** (extensions not owned by other WPs)`
- `apps/screenpipe-app-tauri/src-tauri/assets/extensions/**`
- `apps/screenpipe-app-tauri/src-tauri/src/pi_extension_packages.rs (new)`
- `apps/screenpipe-app-tauri/components/settings/skills-card.tsx (packages UI)`
- `crates/screenpipe-core/src/agents/extensions_manifest.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/pi_extensions_manifest.rs`

Acceptance:
- [ ] bun test: manifest lists the expected extensions and tool ids are sp_*
- [ ] UI smoke: list, install (from a local tarball or npm, user-initiated) and remove an extension package; safe mode disables it

#### W3-24 Agent self-improvement: skills store, user profile, learning (L)

- **Items:** agents_pipes-021, agents_pipes-022, agents_pipes-023, api-021
- **Depends on:** W1-03
- **Specs:** specs/agents_pipes.md agents_pipes-021..023; specs/api.md api-021

POST /agent/skills/manage (list/read/create/patch with sha256 optimistic patching, user/agent/bundled provenance manifest, unsafe-pattern scan, create requires confirmed), POST /agent/profile/manage (list/save/delete user-profile facts with threat scan), GET /agent/self-improvement/context {system_prompt} with a frozen profile snapshot, POST /agent/learning/chats {query}, and the opt-in skill-learning task.

Owner paths:
- `crates/screenpipe-engine/src/agent_skills.rs (new)`
- `crates/screenpipe-engine/src/agent_profile.rs (new)`
- `crates/screenpipe-engine/src/routes/agent.rs`
- `crates/screenpipe-core/assets/extensions/self-improvement.ts (new)`
- `crates/screenpipe-core/assets/extensions/skill-learning.ts (new)`
- `crates/screenpipe-core/assets/pipes/skill-learning/pipe.md (new, own text)`

Acceptance:
- [ ] Harness: create without confirmed is refused; with confirmed the manifest .screenpipe-agent-skills.json gains the skill; patch with a wrong sha256 returns 409
- [ ] POST /agent/profile/manage save/list/delete round-trips; GET /agent/self-improvement/context returns system_prompt

#### W3-25 Local OpenAI-compatible proxy and BYO web search (L)

- **Items:** agents_pipes-033, api-023, api-025, agents_pipes-035
- **Depends on:** W1-03
- **Specs:** specs/agents_pipes.md agents_pipes-033, 035; specs/api.md api-023, 025

POST /v1/local/chat/completions forwards (streaming) to the user's default AI preset without exposing keys, with clear errors for a missing preset/base URL or unreachable provider. POST /v1/web-search {query} -> {content, sources[]} backed by a user-configured provider (SearXNG or API key), disabled with a clear error when unconfigured; sp_web_search returns {text, sources, query}.

Owner paths:
- `crates/screenpipe-engine/src/local_chat.rs (new)`
- `crates/screenpipe-engine/src/routes/local_chat.rs`
- `crates/screenpipe-engine/src/routes/web_search.rs`
- `crates/screenpipe-core/assets/extensions/web-search.ts`
- `apps/screenpipe-app-tauri/src-tauri/assets/extensions/web-search.ts`

Acceptance:
- [ ] Harness with a local mock OpenAI server as default preset: `api /v1/local/chat/completions -X POST -d '{"messages":[...],"stream":true}'` streams SSE
- [ ] No default preset returns 400 with the missing-preset message; POST /v1/web-search unconfigured returns the disabled error, configured against a mock SearXNG returns sources

#### W3-26 Connect broker and connection-gate tools (M)

- **Items:** agents_pipes-043, api-022
- **Depends on:** W2-20
- **Specs:** specs/agents_pipes.md agents_pipes-043; specs/api.md api-022

POST /v1/connect-request blocks until the UI answers via POST /v1/connect-response (event acp_connect_request) and returns connected|declined|timeout; tools screenpipe_list_connections and screenpipe_connect_app with an inline connect card (app extension uses the UI confirm, core/ACP copy uses the broker).

Owner paths:
- `crates/screenpipe-engine/src/routes/connect_broker.rs`
- `crates/screenpipe-core/assets/extensions/connection-gate.ts (new)`
- `apps/screenpipe-app-tauri/src-tauri/assets/extensions/connection-gate.ts (new)`
- `apps/screenpipe-app-tauri/components/chat/connect-card.tsx (new)`

Acceptance:
- [ ] Harness: a connect-request answered by a connect-response {status:'connected'} returns connected; unanswered returns timeout
- [ ] UI smoke: asking the agent to use an unconnected app shows the connect card

#### W3-27 SQLite hard-fault quarantine coordinator (L)

- **Items:** db-019
- **Depends on:** W2-01
- **Specs:** specs/db.md db-019

On corrupt/notadb/ioerr/full, close writer admission for the database generation and write <db>.quarantine.json (SqliteQuarantineMarker fields) with reserve/verification-pending sidecars; fresh-process verification confirms or clears; recording-recovery.log. The app UX is W5-08.

Owner paths:
- `crates/screenpipe-sqlite-coordinator/ (new crate)`
- `crates/screenpipe-db/src/write_queue.rs (admission gate)`
- `crates/screenpipe-db/src/db.rs (fault classification hook)`
- `crates/screenpipe-db/src/sqlite_error.rs`

Acceptance:
- [ ] cargo test: an injected SQLITE_CORRUPT on write closes admission and writes the marker with schema_version, file_identity and sqlite_code
- [ ] A verification run on a healthy copy clears the marker; recording-recovery.log receives entries

#### W3-28 Headless mode and search-only mode (L)

- **Items:** shell-004, shell-005, api-034
- **Depends on:** W2-26
- **Specs:** specs/shell.md shell-004, 005; specs/api.md api-034

headless/headlessRecordOnly destroy webviews (keeping incomplete onboarding), unregister shortcuts and suppress shows until tray Open wakes the UI. keepSearchAvailableAfterQuit (default true): Quit stops capture and closes UI but keeps the read API; live-capture routes return 503 search_only with a message explaining that only saved history is being served; failure falls back to full exit. The gated route set is inferred (open question).

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/window/show.rs (suppression)`
- `apps/screenpipe-app-tauri/src-tauri/src/shortcuts.rs (unregister/restore)`
- `apps/screenpipe-app-tauri/src-tauri/src/tray.rs (Open wakes UI)`
- `apps/screenpipe-app-tauri/src-tauri/src/main.rs (quit path)`
- `apps/screenpipe-app-tauri/src-tauri/src/search_only.rs`
- `crates/screenpipe-engine/src/search_only.rs`
- `crates/screenpipe-engine/src/server.rs (gate layer via seam)`

Acceptance:
- [ ] UI smoke: headless=true leaves no WebView2 child processes; tray Open restores windows and shortcuts
- [ ] Quit with keepSearchAvailableAfterQuit: `api '/search?limit=1'` still returns 200 and POST /audio/start returns 503 with code search_only

#### W3-29 Recording health incident state (M)

- **Items:** shell-011
- **Depends on:** W2-26
- **Specs:** specs/shell.md shell-011

State machine emitting recording-health-state as 'state|detail|kind|extra' (normal, failure, recovering, fixing, recovered, manual-reopen) with commands overlay_restart_recording, overlay_dismiss_incident, get_recording_health_state, retry_screenpipe, is_capture_paused; BootPhaseSnapshot gains cpuCompatMode. The overlay UI is W6-19.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/overlay_health.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/health.rs (state emit region)`

Acceptance:
- [ ] UI smoke: stopping capture produces a failure state readable via get_recording_health_state; overlay_restart_recording recovers it
- [ ] cargo test (app crate): state transitions for failure -> recovering -> recovered

#### W3-30 Low-disk recording guard and keep-awake (M)

- **Items:** shell-006, shell-007
- **Depends on:** W1-02, W1-03
- **Specs:** specs/shell.md shell-006, 007

Engine monitor publishes disk_space_low/disk_space_recovered with hysteresis; the app stops capture when stopRecordingOnLowDisk, notifies with a settings?section=storage deep link, persists disk-pressure-notification-state.json; get_low_disk_guard_config returns {thresholdBytes, checkIntervalSeconds}. keepComputerAwake and set_keep_awake hold SetThreadExecutionState on a dedicated thread.

Owner paths:
- `crates/screenpipe-engine/src/disk_pressure.rs (new)`
- `crates/screenpipe-engine/src/power/keep_awake.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/disk_pressure.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/power_awake.rs (new)`

Acceptance:
- [ ] With a test threshold above free space, disk_space_low fires, capture stops and a notification appears; lowering it resumes capture
- [ ] set_keep_awake(true) then `powercfg /requests` lists a SYSTEM request from the app; false clears it

#### W3-31 SOP narration providers: OpenAI-compatible, local OS TTS, captions-only (L)

- **Items:** gap_workflow-sop-video-and-media-narration-decloud
- **Depends on:** W1-02
- **Specs:** specs/gap_workflow-sop-video-and-media.md narration-decloud

SpeechProvider trait replacing the hosted /tts: OpenAI-compatible POST {base}/audio/speech (chunked, size-capped, retries), local WinRT/SAPI voice, and a default captions-only provider emitting estimated-duration silence. Settings key workflowVideoNarration; API key in the secret store; no account or plan gate.

Owner paths:
- `crates/screenpipe-core/src/workflow_speech.rs (new)`
- `crates/screenpipe-core/src/workflow_speech/openai.rs (new)`
- `crates/screenpipe-core/src/workflow_speech/sapi_windows.rs (new)`
- `crates/screenpipe-core/src/workflow_speech/captions_only.rs (new)`

Acceptance:
- [ ] cargo test -p screenpipe-core workflow_speech: captions-only returns silence of the estimated duration; OpenAI provider against a mock returns PCM
- [ ] Manual (Windows): the SAPI provider synthesises a sentence to a wav

### Wave 4: Advanced backends, agents runtime, app shell

| WP | Size | Title | Depends on | Items |
|---|---|---|---|---|
| W4-01 | XL | Workflows core and /workflows API | W2-22, W3-23, W2-02 | agents_pipes-026, api-028, agents_pipes-027 |
| W4-02 | L | Pipe scheduler v2: defer/coalesce, topics, preset '*', trajectories, log archive | W3-06 | agents_pipes-007, agents_pipes-042, agents_pipes-m06 |
| W4-03 | XL | ACP agent runtime and screenpipe-tools MCP | W2-23, W2-27 | agents_pipes-016, agents_pipes-039 |
| W4-04 | L | Cloud coding agents (BYO Cursor/Codex/Claude) | W2-22 | agents_pipes-028, api-027 |
| W4-05 | M | Chat runtime commands and per-turn connections context | W2-23 | agents_pipes-018, connections-chat-context-per-turn |
| W4-06 | M | Bundled skills update and starter skills | W2-23, W3-23 | agents_pipes-024 |
| W4-07 | L | CLI v2: service, diagnose, richer status, agent setup | W3-17 | shell-026, agents_pipes-031 |
| W4-08 | M | Pipe permissions v2 | W3-02, W3-21 | api-038, agents_pipes-013, agents_pipes-m04, meetings_memory-045 |
| W4-09 | L | Database recovery v2: embedded page-level recovery | W3-27 | db-020 |
| W4-10 | L | Data maintenance, vault and secrets DB split | W3-27 | db-025, db-026, api-030, meetings_memory-044 |
| W4-11 | L | Live Views backend | W3-05 | api-020 |
| W4-12 | XL | Audio-process (mic-holder) meeting detector | W3-08, W2-18 | meetings_memory-030 |
| W4-13 | XL | Event-driven retained UIA tree on Windows | W3-13 | vision-uia-retained-capture |
| W4-14 | L | Capture loop B: tree-walker worker and foreground identity gate | W3-11 | vision-tree-walker-worker, vision-capture-identity-gate |
| W4-15 | L | Redaction pseudonyms, a11y-aware secret detection, configurable models | W3-10 | vision-redaction-pseudonyms-a11y, vision-redaction-models |
| W4-16 | L | Bluetooth meeting gate, output follow and speaker watchdog | W3-14 | audio-011, audio-017 |
| W4-17 | L | Calendar v2: all-calendar Google events and reminder scheduler | W2-21, W3-18 | connections-google-calendar-multi, meetings_memory-046 |
| W4-18 | L | Bee pairing and browser snapshot v2 with /act | W2-20 | connections-bee-pairing, connections-browser-snapshot-v2-act |
| W4-19 | L | Window inventory, screen-share protection, renderer heartbeat | W3-28 | shell-008, shell-018, shell-039, frontend-005 |
| W4-20 | L | UI localization (en + ja) and native string tables | W2-25, W1-02 | shell-036, frontend-003 |
| W4-21 | XL | Narrated SOP video renderer (core) | W3-31 | gap_workflow-sop-video-and-media-core-renderer |
| W4-22 | L | App shell: (main) route group, navigation v2, command palette | W2-25 | frontend-002, frontend-006, frontend-008 |

#### W4-01 Workflows core and /workflows API (XL)

- **Items:** agents_pipes-026, api-028, agents_pipes-027
- **Depends on:** W2-22, W3-23, W2-02
- **Specs:** specs/agents_pipes.md agents_pipes-026, 027; specs/api.md api-028

Workflow record contract, revisioned file-backed catalog.json/pipeline.json/profile.json with backups and locks, multi-agent workspace (start/propose/checkpoint/handoff/reject/publish/finish with revisions), enrichment pipeline stages, edits and corrections, list/get with automation evidence and context. Gating is a local toggle and optional local budget only.

Owner paths:
- `crates/screenpipe-core/src/workflows/** (new)`
- `crates/screenpipe-engine/src/routes/workflows.rs`
- `crates/screenpipe-engine/src/routes/workflow_catalog.rs (new)`
- `crates/screenpipe-engine/src/routes/workflow_pipeline.rs (new)`
- `crates/screenpipe-engine/src/routes/workflow_workspace.rs (new)`
- `crates/screenpipe-engine/src/routes/workflow_edits.rs (new)`

Acceptance:
- [ ] Harness: `api /workflows` returns []; POST /workflows/catalog creates revision 1; a stale revision returns 409
- [ ] A workspace start -> propose -> checkpoint -> publish sequence via curl updates catalog.json; POST /workflows/corrections is recorded
- [ ] grep shows no workflow_business_required / hosted_ai/plan checks in the code path

#### W4-02 Pipe scheduler v2: defer/coalesce, topics, preset '*', trajectories, log archive (L)

- **Items:** agents_pipes-007, agents_pipes-042, agents_pipes-m06
- **Depends on:** W3-06
- **Specs:** specs/agents_pipes.md agents_pipes-007, 042, m06

Consume connection_trigger, workflow_ready:<pipe> and meeting_summary_refresh_requested; defer busy events with a backlog cap and coalesce; write .trigger-context.json; skip trivial meetings; '*' preset expands to user presets and a missing named preset fails without provider fallback; persist skipped runs. Optional JSONL trajectories (SCREENPIPE_TRAJECTORIES_DIR/MAX_RUNS) and a periodic pipe-log archive sweep.

Owner paths:
- `crates/screenpipe-core/src/pipes/mod.rs (start_scheduler region)`
- `crates/screenpipe-core/src/pipes/preset_fallback.rs`
- `crates/screenpipe-core/src/pipes/trajectory.rs (new)`
- `crates/screenpipe-core/src/pipes/log_archive.rs (new)`

Acceptance:
- [ ] cargo test -p screenpipe-core scheduler: busy pipe defers then runs once (coalesced); backlog cap enforced; '*' expands; missing named preset fails
- [ ] SCREENPIPE_TRAJECTORIES_DIR set: a run writes a JSONL file; the archive sweep reports scanned/archived counts in the log

#### W4-03 ACP agent runtime and screenpipe-tools MCP (XL)

- **Items:** agents_pipes-016, agents_pipes-039
- **Depends on:** W2-23, W2-27
- **Specs:** specs/agents_pipes.md agents_pipes-016, 039

Agent catalog (pi, codex, claude, cursor, copilot, custom command) with pinned launches, a --screenpipe-acp-runtime child speaking agent-client-protocol, the SCREENPIPE_ACP_* env contract, session config/modes/approval ask|allow-all, install/login commands, ACP events, a dependency-free screenpipe-tools MCP server (stdio or HTTP) and ACP-backed pipes. Feature flags become local settings; the vendor /anthropic cloud routing is not offered.

Owner paths:
- `crates/screenpipe-core/src/agents/acp/** (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/pi_acp.rs (new)`
- `crates/screenpipe-tools-mcp/ (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/main.rs (argv table entries only)`

Acceptance:
- [ ] Freenpipe.exe --screenpipe-acp-runtime starts and answers an ACP initialize over stdio (test harness)
- [ ] screenpipe-tools MCP lists its tools over stdio; an ACP session with the pi agent answers a prompt (UI smoke)
- [ ] Approval mode ask raises a permission request event

#### W4-04 Cloud coding agents (BYO Cursor/Codex/Claude) (L)

- **Items:** agents_pipes-028, api-027
- **Depends on:** W2-22
- **Specs:** specs/agents_pipes.md agents_pipes-028; specs/api.md api-027

Pipes with agent: cloud-agent and CloudAgentConfig; /cloud-agents/status, /:provider/connect, /:provider/codebases, /cursor-agents, PUT /cursor-key (secret cloud_agents.cursor.api_key, never returned); opt-in bounded context capsule; snapshot commits use a Freenpipe identity. Third-party, user-initiated only.

Owner paths:
- `crates/screenpipe-core/src/agents/cloud.rs (new)`
- `crates/screenpipe-core/src/agents/cloud_context.rs (new)`
- `crates/screenpipe-engine/src/routes/cloud_agents.rs`

Acceptance:
- [ ] Harness: PUT /cloud-agents/cursor-key {key:'x'} stores it; GET /cloud-agents/status shows connected without the key
- [ ] wiremock api.cursor.com: a cloud-agent pipe run launches an agent with the bounded context capsule

#### W4-05 Chat runtime commands and per-turn connections context (M)

- **Items:** agents_pipes-018, connections-chat-context-per-turn
- **Depends on:** W2-23
- **Specs:** specs/agents_pipes.md agents_pipes-018; specs/connections.md chat-context-per-turn

Commands pi_start_and_prompt, pi_stop_if_idle, pi_request_state, thinking level, pi_extension_ui_response, list_chat_entries_by_mtime; pool start stages and busy refusal; agent_event types thinking_level_changed/extension_ui_request/compaction_end. Each user turn is prefixed with a fresh <connections_context> block (redacted render_context) that the transcript view strips.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/pi.rs (pool/command region)`
- `apps/screenpipe-app-tauri/src-tauri/src/pi_command_queue.rs`
- `apps/screenpipe-app-tauri/lib/chat/strip-connections-context.ts (new)`
- `apps/screenpipe-app-tauri/components/standalone-chat.tsx (one render call)`

Acceptance:
- [ ] bun run bindings:check; tauri.ts exposes the new commands
- [ ] bun test: the strip helper removes the block; chat log shows the block on each turn (UI smoke)

#### W4-06 Bundled skills update and starter skills (M)

- **Items:** agents_pipes-024
- **Depends on:** W2-23, W3-23
- **Specs:** specs/agents_pipes.md agents_pipes-024

Add screenpipe-chats, screenpipe-workflow-maintenance and video-sop skills and 20 starter skills written in our own words with .screenpipe-starter-* markers; never overwrite user-edited starter skills.

Owner paths:
- `crates/screenpipe-core/assets/skills/** (new skills, own text)`
- `crates/screenpipe-core/src/agents/pi.rs (skill install region)`

Acceptance:
- [ ] Fresh data dir installs the three skills and 20 starter skills with markers
- [ ] cargo test: a user-modified starter skill is left untouched on upgrade

#### W4-07 CLI v2: service, diagnose, richer status, agent setup (L)

- **Items:** shell-026, agents_pipes-031
- **Depends on:** W3-17
- **Specs:** specs/shell.md shell-026; specs/agents_pipes.md agents_pipes-031

service install [--mode recorder|server]/uninstall/status (Windows: per-user scheduled task or Run key), diagnose [--dry-run] (local bundle only), status verdicts and JSON (health.*, last_audio_capture, storage_size_bytes, database_path), and agent setup --target {openclaw, hermes, claude-code, claude-desktop, codex, cursor, gemini, runner, windsurf, vscode} writing an mcpServers entry plus skills with backups and locks, pointing at the bundled binary.

Owner paths:
- `crates/screenpipe-engine/src/cli/mod.rs (subcommand enum region)`
- `crates/screenpipe-engine/src/cli/service.rs (new)`
- `crates/screenpipe-engine/src/cli/diagnose.rs (new)`
- `crates/screenpipe-engine/src/cli/status.rs`
- `crates/screenpipe-engine/src/cli/agent.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/ai_tools.rs (new)`
- `apps/screenpipe-app-tauri/components/onboarding/connect-apps.tsx (use agent setup)`

Acceptance:
- [ ] screenpipe service install --mode recorder then service status reports installed; uninstall removes it
- [ ] screenpipe diagnose --dry-run lists bundle contents and writes nothing; screenpipe status --json has health, last_audio_capture, storage_size_bytes, database_path
- [ ] screenpipe agent setup --target claude-desktop writes mcpServers.screenpipe and a backup of the previous config

#### W4-08 Pipe permissions v2 (M)

- **Items:** api-038, agents_pipes-013, agents_pipes-m04, meetings_memory-045
- **Depends on:** W3-02, W3-21
- **Specs:** specs/api.md api-038; specs/agents_pipes.md agents_pipes-013, m04; specs/meetings_memory.md meetings_memory-045

Separate workflow-operation concurrency lane for /workflows/workspace and /workflows/catalog; explicit content_type, server-side app/window 403s and include_related refusal on /search for restricted pipes; reader allowlist adds starred-sessions, workflows, feedback and the structured-output set; Content() accepts memory and parsed. MCP session filtering itself is W3-21.

Owner paths:
- `crates/screenpipe-engine/src/pipe_permissions_middleware.rs`
- `crates/screenpipe-core/src/pipes/permissions.rs`
- `crates/screenpipe-core/assets/extensions/screenpipe-permissions.ts`

Acceptance:
- [ ] cargo test: a restricted pipe calling /search without content_type gets 403; a denied app returns 403 server-side; include_related is refused
- [ ] A pipe with Content(parsed) can query content_type=parsed; GET /starred-sessions is allowed by default

#### W4-09 Database recovery v2: embedded page-level recovery (L)

- **Items:** db-020
- **Depends on:** W3-27
- **Specs:** specs/db.md db-020

Replace the sqlite3 .recover shell-out with in-process recovery into db-recovery-<ts>/ (working copy, candidate.sqlite, recovery-manifest.json), then FTS rebuild, DELETE journal, fresh identity, integrity and foreign_key checks, write-canary readback and a manifest phase machine; resolved-quarantine.json; lock payload gains host_id.

Owner paths:
- `crates/screenpipe-engine/src/cli/db.rs`
- `crates/screenpipe-sqlite-recovery/ (new crate)`
- `crates/screenpipe-db/src/recovery.rs (new)`

Acceptance:
- [ ] screenpipe db recover --data-dir <copy of a corrupted fixture> produces candidate.sqlite and a manifest in phase complete
- [ ] The canary readback check passes; the lock file contains host_id; recovery refuses while the API port is live

#### W4-10 Data maintenance, vault and secrets DB split (L)

- **Items:** db-025, db-026, api-030, meetings_memory-044
- **Depends on:** W3-27
- **Specs:** specs/db.md db-025, db-026; specs/api.md api-030; specs/meetings_memory.md meetings_memory-044; specs/connections.md secret-db-split, credential-refresh-leases, vault-secrets-db (merged)

POST /data/compact (free-space precheck, busy_timeout dance, VACUUM, refused for hybrid). Secrets move to secrets.sqlite with credential_refresh_leases and secret_store_metadata, a one-time legacy copy gated by legacy_main_db_migrated_v1 (newer updated_at wins), quarantine-aware open; all construction sites switch pools. Vault covers secrets.sqlite(+journal), refuses online lock with 409 and refuses hybrid setup. /sync/reset-account and /archive/download(+status) return 501.

Owner paths:
- `crates/screenpipe-engine/src/routes/data.rs (compact route)`
- `crates/screenpipe-engine/src/routes/vault.rs`
- `crates/screenpipe-engine/src/sync_api.rs (reset-account)`
- `crates/screenpipe-engine/src/archive.rs (download)`
- `crates/screenpipe-secrets/src/store.rs`
- `crates/screenpipe-vault/src/manager.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/disk_usage.rs`
- `crates/screenpipe-engine/src/auth_key.rs`
- `crates/screenpipe-engine/src/cli/connection.rs`

Acceptance:
- [ ] Harness: POST /data/compact returns bytes_before/bytes_after/bytes_reclaimed
- [ ] <FP_DD>/secrets.sqlite exists and legacy secrets were copied once (sqlq on secret_store_metadata); lease acquire/release cargo test
- [ ] POST /vault/lock while the server runs returns 409; POST /sync/reset-account returns 501

#### W4-11 Live Views backend (L)

- **Items:** api-020
- **Depends on:** W3-05
- **Specs:** specs/api.md api-020

GET /live-views, /live-views/schema, /live-views/catalog, /live-views/:id/template; PUT (and DELETE) /live-views/:id with expectedRevision; live-view-template.v1 and live-view-kit.v1 schemas; templates.json store; bundled kits written by us.

Owner paths:
- `crates/screenpipe-engine/src/live_views.rs (new)`
- `crates/screenpipe-engine/src/routes/live_views.rs`
- `crates/screenpipe-core/assets/live-views/ (new: bundled kits, own content)`

Acceptance:
- [ ] Harness: PUT /live-views/v1 with expectedRevision 0 creates it; a stale expectedRevision returns 409
- [ ] `api /live-views/schema` returns both schema ids; `api /live-views/catalog` lists the bundled kits

#### W4-12 Audio-process (mic-holder) meeting detector (XL)

- **Items:** meetings_memory-030
- **Depends on:** W3-08, W2-18
- **Specs:** specs/meetings_memory.md meetings_memory-030; specs/audio.md audio-015 (merged)

Enumerate capture/render audio sessions (IAudioSessionManager2) to find the mic-holding process, confirm via recent frames, an active-tab URL probe and a bounded UIA call-control scan; start/reopen/merge/end rules (single-open, manual precedence, 120 s rejoin, room/calendar guards), orphan close, auto-end, shutdown end, RDP/VDI warning. Reuse the base lifecycle primitives.

Owner paths:
- `crates/screenpipe-engine/src/meeting_watcher/audio_process/** (new)`
- `crates/screenpipe-engine/src/meeting_watcher.rs (integration)`
- `crates/screenpipe-audio/src/meeting_processes.rs (new, WASAPI session snapshot)`

Acceptance:
- [ ] cargo test: session snapshot classification with a mocked enumerator; rejoin within 120 s reopens the same meeting
- [ ] Manual: a Teams or browser call holding the mic starts a meeting; releasing the mic ends it; a manual meeting takes precedence

#### W4-13 Event-driven retained UIA tree on Windows (XL)

- **Items:** vision-uia-retained-capture
- **Depends on:** W3-13
- **Specs:** specs/vision.md uia-retained-capture

Keep a retained baseline tree for the focused window, update it from UIA structure/property/text/layout events within a time slice, resync on identity mismatch or overflow, periodic resync for weak providers, drop state under privacy rules, fall back to pixel_only with diagnostics, skip fragile providers; TruncationReason gains Pending.

Owner paths:
- `crates/screenpipe-a11y/src/tree/windows.rs`
- `crates/screenpipe-a11y/src/tree/mod.rs`
- `crates/screenpipe-a11y/src/tree/retained_tree.rs (new)`
- `crates/screenpipe-a11y/src/tree/capture_diagnostics.rs (new)`
- `crates/screenpipe-a11y/src/platform/windows_uia.rs`
- `crates/screenpipe-a11y/src/budget.rs`

Acceptance:
- [ ] cargo test -p screenpipe-a11y retained_tree: event application, overflow resync and identity-mismatch resync
- [ ] Manual: typing in Notepad updates the retained tree without a full re-walk (debug counter)

#### W4-14 Capture loop B: tree-walker worker and foreground identity gate (L)

- **Items:** vision-tree-walker-worker, vision-capture-identity-gate
- **Depends on:** W3-11
- **Specs:** specs/vision.md tree-walker-worker, capture-identity-gate

One walker thread per capture loop with deadline, restart-and-retry-once on failure, detach-and-restart on timeout and suspend/resume for Cold or paused monitors. Compare foreground identity before and after the screenshot (reject on change or unknown), keep AX text marked not pixel-aligned when only the walked process differs, time-box the focused-metadata lookup and re-apply privacy filters and URL policy after resolving the real app/window/URL.

Owner paths:
- `crates/screenpipe-capture/src/lib.rs`
- `crates/screenpipe-capture/src/tree_walker_worker.rs (new)`
- `crates/screenpipe-capture/src/paired_capture.rs (walker call region)`
- `crates/screenpipe-engine/src/event_driven_capture.rs`

Acceptance:
- [ ] cargo test: walker timeout detaches and restarts; a failure retries once
- [ ] Unit: identity change between pre and post screenshot rejects the frame; a post-resolution blocked URL drops the frame

#### W4-15 Redaction pseudonyms, a11y-aware secret detection, configurable models (L)

- **Items:** vision-redaction-pseudonyms-a11y, vision-redaction-models
- **Depends on:** W3-10
- **Specs:** specs/vision.md redaction-pseudonyms-a11y, redaction-models

Consistent pseudonyms keyed by .redaction/pseudonym.key (never silently regenerated); accessibility-context secret detection (password/secure fields, label hints, token prefixes, PEM) feeding text and image redaction; secret continuation across frames; CPU-budgeted batches; AI adapter failure circuit falling back to regex. Model id/URL/hash configurable with base defaults (rfdetr_v12, v45) kept.

Owner paths:
- `crates/screenpipe-redact/src/pseudonym.rs (new)`
- `crates/screenpipe-redact/src/a11y_context.rs (new)`
- `crates/screenpipe-redact/src/ocr_json.rs (new)`
- `crates/screenpipe-redact/src/tree_json.rs (new)`
- `crates/screenpipe-redact/src/pipeline.rs`
- `crates/screenpipe-redact/src/image/worker.rs`
- `crates/screenpipe-redact/src/image/secret_continuation.rs (new)`
- `crates/screenpipe-redact/src/adapters/rfdetr.rs`
- `crates/screenpipe-redact/src/adapters/onnx.rs`

Acceptance:
- [ ] cargo test -p screenpipe-redact: same input and key give the same pseudonym; a missing key is an error state, not a new key
- [ ] A password-field fixture and a PEM block are redacted; model config overrides are honoured

#### W4-16 Bluetooth meeting gate, output follow and speaker watchdog (L)

- **Items:** audio-011, audio-017
- **Depends on:** W3-14
- **Specs:** specs/audio.md audio-011, 017

Bluetooth combo-headset mics stay closed outside meetings unless alwaysRecordBluetoothMic or explicitly selected (PnP container walk with name fallback, fail-safe to combo). [AUDIO_FOLLOW] captures any rendering-but-uncaptured endpoint and stops when idle; [SPEAKER_WATCHDOG] cycles output capture when a meeting renders audio but yields no chunks and raises speaker_silent.

Owner paths:
- `crates/screenpipe-audio/src/core/device.rs`
- `crates/screenpipe-audio/src/core/device_detection.rs`
- `crates/screenpipe-audio/src/audio_manager/device_monitor.rs (follow, watchdog, BT regions)`
- `crates/screenpipe-audio/src/audio_manager/manager.rs (BT gate call)`

Acceptance:
- [ ] cargo test: name heuristics and a mocked PnP walk classify combo headsets
- [ ] Manual: a BT headset mic is not opened while idle; [AUDIO_FOLLOW] log appears when audio plays on a second output

#### W4-17 Calendar v2: all-calendar Google events and reminder scheduler (L)

- **Items:** connections-google-calendar-multi, meetings_memory-046
- **Depends on:** W2-21, W3-18
- **Specs:** specs/connections.md google-calendar-multi; specs/meetings_memory.md meetings_memory-046

Read every visible calendar (paginated calendarList), dedupe by iCalUID, honour declined/transparent, real calendarName, account_snapshots and require_complete; the app publisher keeps the last good per-account snapshot. App-side reminder scheduler with notificationPrefs.meetingReminderLeadSeconds, a durable receipt ledger and withholding when the meeting is already active.

Owner paths:
- `crates/screenpipe-engine/src/connections_api.rs (Google Calendar events handler)`
- `apps/screenpipe-app-tauri/src-tauri/src/google_calendar.rs (publisher region)`
- `apps/screenpipe-app-tauri/src-tauri/src/calendar/reminders.rs (new)`
- `crates/screenpipe-events/src/custom_events/meetings.rs (prewarm lead)`
- `apps/screenpipe-app-tauri/src-tauri/src/calendar.rs`

Acceptance:
- [ ] Harness (BYO Google OAuth): `api '/connections/google-calendar/events?account_snapshots=true'` returns events from several calendars with calendarName
- [ ] cargo test (app crate) reminders: fires at the lead time, does not refire after restart, withheld when the meeting is active

#### W4-18 Bee pairing and browser snapshot v2 with /act (L)

- **Items:** connections-bee-pairing, connections-browser-snapshot-v2-act
- **Depends on:** W2-20
- **Specs:** specs/connections.md bee-pairing, browser-snapshot-v2-act

POST /connections/bee/pair/start and /pair/poll with an ephemeral keypair and versioned token decryption; the pairing app id is a user setting (no vendor default). Snapshot v2 (depth 12, shadow roots, same-origin iframes, names, state flags, data-sp-ref eN refs, count) and POST /connections/browsers/:id/act {ref, action, value} through a fixed action script.

Owner paths:
- `crates/screenpipe-connect/src/connections/bee.rs`
- `crates/screenpipe-engine/src/connections_routes/bee.rs`
- `crates/screenpipe-engine/src/connections_routes/browser_act.rs`
- `crates/screenpipe-engine/src/connections_api.rs (browser snapshot handler)`
- `crates/screenpipe-core/assets/skills/screenpipe-api/SKILL.md (browser control block)`

Acceptance:
- [ ] Harness: pair/start without a configured Bee app id returns 400; with one it returns request_id and pairing_url
- [ ] On the owned browser: snapshot returns refs; POST /connections/browsers/<id>/act {ref:'e3',action:'click'} succeeds

#### W4-19 Window inventory, screen-share protection, renderer heartbeat (L)

- **Items:** shell-008, shell-018, shell-039, frontend-005
- **Depends on:** W3-28
- **Specs:** specs/shell.md shell-008, 018, 039; specs/frontend.md frontend-005

notification-inbox and starred-sessions windows (show_notification_inbox, toggle/hide_starred_sessions), central headless and policy suppression, live set_chat_always_on_top; hideAppInScreenShare applies WDA_EXCLUDEFROMCAPTURE to all windows with get/set_app_screen_capture_protection; every webview sends webview_renderer_heartbeat each second and stale renderers reuse the ProcessFailed recovery; swipe navigation disabled; context menu suppressed.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/window/show.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/window/mod.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/window/content_process.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/windows_overlay.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/screen_capture_protection.rs (new)`
- `apps/screenpipe-app-tauri/app/layout.tsx (heartbeat, contextmenu, swipe)`

Acceptance:
- [ ] UI smoke: show_notification_inbox opens a window labelled notification-inbox; toggle_starred_sessions opens starred-sessions
- [ ] With hideAppInScreenShare on, a screen capture of the desktop does not show app windows
- [ ] Debug log shows heartbeats every second per window

#### W4-20 UI localization (en + ja) and native string tables (L)

- **Items:** shell-036, frontend-003
- **Depends on:** W2-25, W1-02
- **Specs:** specs/shell.md shell-036; specs/frontend.md frontend-003

uiLocale (system or code) and uiLocalizationEnabled; resolve locale from OS/navigator, set html lang/dir, mirror to localStorage; own message catalogues with English fallback; the native layer rebuilds tray/menu/notification strings on store changes. A visible toggle replaces the vendor rollout flag.

Owner paths:
- `apps/screenpipe-app-tauri/app/providers.tsx`
- `apps/screenpipe-app-tauri/lib/i18n/** (new, own message ids)`
- `apps/screenpipe-app-tauri/components/settings/general-settings.tsx (language selector)`
- `apps/screenpipe-app-tauri/src-tauri/src/localization.rs (new)`

Acceptance:
- [ ] bun test: catalogue keys are complete for en and fall back for missing ja strings
- [ ] UI smoke: switching to ja updates html lang, visible strings and the tray menu

#### W4-21 Narrated SOP video renderer (core) (XL)

- **Items:** gap_workflow-sop-video-and-media-core-renderer
- **Depends on:** W3-31
- **Specs:** specs/gap_workflow-sop-video-and-media.md core-renderer

Scene/Focus types and validation; ffmpeg preflight; screenshot normalisation; per-scene parts on a 1280x720@24 canvas with header, focus zoompan or scale/pad, or a text card; narration retimed into PCM via the speech provider; WebVTT captions; concat and mux with AAC and a default mov_text 'Narration' track, libx264 +faststart; duration/size/time limits; progress and cancel callbacks.

Owner paths:
- `crates/screenpipe-core/src/workflow_video.rs (new)`
- `crates/screenpipe-core/src/workflow_video/ (new)`
- `crates/screenpipe-core/src/ffmpeg.rs (preflight helper only)`
- `crates/screenpipe-core/tests/workflow_video_test.rs (new)`

Acceptance:
- [ ] cargo test -p screenpipe-core --test workflow_video_test renders a 2-scene project; ffprobe shows 1280x720, 24 fps and a mov_text track titled Narration
- [ ] 51 sections or pace 2.0 are rejected; the cancel callback stops rendering within 2 s

#### W4-22 App shell: (main) route group, navigation v2, command palette (L)

- **Items:** frontend-002, frontend-006, frontend-008
- **Depends on:** W2-25
- **Specs:** specs/frontend.md frontend-002, 006, 008

Shared (main) layout for /home and /settings with the resizable sidebar and drag surface, ?from= back navigation and section aliases; nav sections Chat/Meetings/Timeline/Activity/Library/Automations/Connections with reorder/hide (sidebarNavLayout, legacy migration) and badges; cmdk palette and the fixed in-app keybindings with a shortcut guide. After this WP, home/settings pages live under app/(main)/ and later WPs edit them there.

Owner paths:
- `apps/screenpipe-app-tauri/app/(main)/** (new route group; app/home and app/settings move here)`
- `apps/screenpipe-app-tauri/components/app-sidebar.tsx`
- `apps/screenpipe-app-tauri/components/command-palette.tsx (new)`
- `apps/screenpipe-app-tauri/lib/keybindings.ts (new)`
- `apps/screenpipe-app-tauri/components/shortcut-guide.tsx (new)`

Acceptance:
- [ ] bun run build still exports /home and /settings; bun test keybindings registry
- [ ] UI smoke: Ctrl+K opens the palette; reordering and hiding nav items persists across restart

### Wave 5: Feature UIs and second-tier backends

| WP | Size | Title | Depends on | Items |
|---|---|---|---|---|
| W5-01 | L | Pipes API v2 and SSE output stream | W4-02 | agents_pipes-005, agents_pipes-006, api-029 |
| W5-02 | L | Skills registry, external-agent skill export, provider automations | W4-06, W4-03 | agents_pipes-025, agents_pipes-029, frontend-048 |
| W5-03 | M | Meeting room-change offers | W4-12 | meetings_memory-029 |
| W5-04 | L | Audio device lifecycle hardening and shared engine reuse | W4-16 | audio-M02, audio-M03 |
| W5-05 | L | Capture loop C: screenshot policy, scroll cadence, width cap, frozen streams | W4-14 | vision-capture-loop-refinements, vision-disable-screenshots, vision-recording-detail, vision-capture-max-width |
| W5-06 | L | OCR text-region gate and crop OCR | W3-12, W4-14 | vision-ocr-gate |
| W5-07 | XL | Hybrid storage format hybrid-parquet-v1 | W2-01 | db-021 |
| W5-08 | L | Database self-heal user flow | W4-09 | shell-014 |
| W5-09 | M | Tray menu v2 | W4-20, W2-11 | shell-023 |
| W5-10 | L | Workflow video jobs in the app | W4-21 | gap_workflow-sop-video-and-media-app-video-jobs |
| W5-11 | L | Captured-moment replay server and open command | W2-10 | gap_workflow-sop-video-and-media-recording-replay, gap_workflow-sop-video-and-media-captured-moment-open |
| W5-12 | L | Activity history generator (app) | W3-04, W3-25 | meetings_memory-016 |
| W5-13 | L | Notifications UI v2: inbox window, bell, panel, preferences | W3-18, W4-19 | meetings_memory-039, frontend-030, frontend-031, meetings_memory-040, frontend-040, meetings_memory-034 |
| W5-14 | L | Starred sessions UI, star shortcut, overlay snooze/anchor, CLI | W2-04, W4-19 | meetings_memory-012, frontend-029, shell-009 |
| W5-15 | L | Chat tabs, split pane, side chat, groups and landing cards | W4-05, W4-22 | frontend-009, frontend-020 |
| W5-16 | L | Library: memories and artifacts | W2-02, W4-22 | meetings_memory-006, frontend-021 |
| W5-17 | L | Meetings workspace UI | W2-08, W4-22 | frontend-024 |
| W5-18 | L | Timeline and search window v2 | W3-02, W4-22 | frontend-025, frontend-026, frontend-027 |
| W5-19 | L | Settings navigation v2, general, AI features, permissions | W4-22, W4-20 | frontend-035, frontend-041, frontend-042, frontend-043 |
| W5-20 | L | Screen and audio settings | W3-14, W3-15, W4-22 | frontend-036, frontend-037 |
| W5-21 | M | Deep links v2 | W4-22 | frontend-045, frontend-M01, cloud-V01 |
| W5-22 | M | Viewer, permission recovery and help refresh | W2-26, W4-22 | frontend-028, frontend-033, frontend-055 |
| W5-23 | L | Connections UI catalogue | W3-19, W3-20, W3-21, W4-18 | connections-ui-catalog, frontend-050 |
| WR-02 | L | Sync and archive to bring-your-own storage | W0-08, W4-10 | (restore) cloud-019, (restore) cloud-037 |

#### W5-01 Pipes API v2 and SSE output stream (L)

- **Items:** agents_pipes-005, agents_pipes-006, api-029
- **Depends on:** W4-02
- **Specs:** specs/agents_pipes.md agents_pipes-005, 006; specs/api.md api-029

GET /pipes with pagination and include_* flags, /pipes/activity, /pipes/:id/executions/:exec_id, POST /pipes/bundled/:id/install, run body run_context/trigger_type/meeting_summary returning execution_id, error_code envelope and 404 for missing pipes; /pipes/:id/stream SSE with progress/keep-alive, a concurrent-stream cap and pipe_done.

Owner paths:
- `crates/screenpipe-engine/src/pipes_api.rs`
- `crates/screenpipe-engine/src/routes/pipe_stream.rs`
- `crates/screenpipe-core/src/pipes/mod.rs (run body and execution query region)`
- `crates/screenpipe-engine/src/pipe_store.rs (paging queries)`

Acceptance:
- [ ] Harness: `api '/pipes?limit=5&offset=0'` returns pagination; `api /pipes/activity` returns an aggregate; missing pipe returns 404 with error_code
- [ ] POST /pipes/<id>/run {trigger_type:'manual'} returns execution_id; `curl -N` on /pipes/<id>/stream receives progress events and pipe_done

#### W5-02 Skills registry, external-agent skill export, provider automations (L)

- **Items:** agents_pipes-025, agents_pipes-029, frontend-048
- **Depends on:** W4-06, W4-03
- **Specs:** specs/agents_pipes.md agents_pipes-025, 029; specs/frontend.md frontend-048

fetch_skills_registry reads a bundled registry (optional user-configured URL), install_registry_skill from user-chosen GitHub folders with limits, install/remove_external_agent_skills for Claude Code, Codex, Cursor, Gemini, OpenClaw and Hermes skill dirs, auto-connect opt-out; list/manage_provider_automation reads Claude scheduled tasks and Codex automation manifests.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/skills.rs (registry and export region)`
- `apps/screenpipe-app-tauri/src-tauri/src/provider_automations.rs (new)`
- `apps/screenpipe-app-tauri/components/settings/skills-card.tsx (registry UI)`
- `apps/screenpipe-app-tauri/components/settings/provider-automations.tsx (new)`
- `apps/screenpipe-app-tauri/src-tauri/assets/skills-registry.json (new, bundled)`

Acceptance:
- [ ] cargo test (app crate): registry loads from the bundled file with no network; provider automations parse fixture files
- [ ] UI smoke: install a registry skill into ~/.claude/skills and remove it

#### W5-03 Meeting room-change offers (M)

- **Items:** meetings_memory-029
- **Depends on:** W4-12
- **Specs:** specs/meetings_memory.md meetings_memory-029

The audio-process detector emits meeting_room_change_offer when the same audio session moves to another call; the app asks the user whether to keep the meeting together or start a new note, and answers through POST /meetings/room-change (route from W2-08).

Owner paths:
- `crates/screenpipe-events/src/custom_events/meetings.rs (offer/response events)`
- `crates/screenpipe-engine/src/meeting_watcher/audio_process/room_change.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/meeting_live_notes.rs (room-change prompt region)`

Acceptance:
- [ ] cargo test: a simulated session move emits one offer with a token
- [ ] UI smoke: choosing the start-a-new-note option ends the old meeting with end_reason room_changed and opens a new one

#### W5-04 Audio device lifecycle hardening and shared engine reuse (L)

- **Items:** audio-M02, audio-M03
- **Depends on:** W4-16
- **Specs:** specs/audio.md audio-M02, M03

Idempotent teardown even when a device is already marked stopped, per-device serialised start/stop with timeouts, worker-exit tracking, refusal of 0-channel/0-Hz streams, 0x80070005 classified as access denied, monitor backoff and user notice on lost devices. Reuse the shared transcription engine across restarts, build engines in a panic-isolated task and keep the receiver alive when no session can be created.

Owner paths:
- `crates/screenpipe-audio/src/device/device_manager.rs`
- `crates/screenpipe-audio/src/audio_manager/manager.rs (start/stop/receiver)`
- `crates/screenpipe-audio/src/audio_manager/device_monitor.rs (backoff region)`

Acceptance:
- [ ] cargo test -p screenpipe-audio: double stop is a no-op; concurrent start/stop are serialised; a zero-channel config is refused
- [ ] Unit: an engine factory panic leaves the receiver running and audio pending

#### W5-05 Capture loop C: screenshot policy, scroll cadence, width cap, frozen streams (L)

- **Items:** vision-capture-loop-refinements, vision-disable-screenshots, vision-recording-detail, vision-capture-max-width
- **Depends on:** W4-14
- **Specs:** specs/vision.md capture-loop-refinements, disable-screenshots, recording-detail, capture-max-width

disableScreenshots keeps triggers and a11y walks but writes frames without JPEG/OCR/visual probes; recordingDetail controls scroll snapshot cadence (auto adapts 1-5 s); captureMaxWidth caps capture width on Windows after readback; frozen persistent streams are released, drop accounting splits timeout vs error, soft checkpoints are deferred, the pause gate runs before focus probes and the a11y worker suspends on pause/Cold.

Owner paths:
- `crates/screenpipe-engine/src/event_driven_capture.rs`
- `crates/screenpipe-engine/src/vision_manager/manager.rs (screenshot policy)`
- `crates/screenpipe-engine/src/ui_recorder.rs (scroll snapshot cadence)`
- `crates/screenpipe-engine/src/power/profile.rs`
- `crates/screenpipe-screen/src/monitor.rs (Windows downscale)`
- `crates/screenpipe-engine/src/routes/health.rs (screenshots_disabled_by_config reason)`

Acceptance:
- [ ] disableScreenshots=true: new frames have no snapshot path and `api /health` reports screenshots_disabled_by_config
- [ ] captureMaxWidth=1280: new frame images are at most 1280 px wide; recordingDetail=more_detail yields about 1 s scroll snapshots (log)
- [ ] cargo test: a frozen stream test hook triggers rebuild

#### W5-06 OCR text-region gate and crop OCR (L)

- **Items:** vision-ocr-gate
- **Depends on:** W3-12, W4-14
- **Specs:** specs/vision.md ocr-gate

Detect text regions on a downscaled grayscale (optionally window-cropped) image; no regions skips OCR; otherwise OCR the padded union and map boxes back to full-frame normalised coordinates; fail open; ocr_gate_* metrics. The detector design is ours.

Owner paths:
- `crates/screenpipe-screen/src/text_regions.rs (new)`
- `crates/screenpipe-screen/src/lib.rs`
- `crates/screenpipe-capture/src/paired_capture.rs (OCR gate region)`
- `crates/screenpipe-screen/src/metrics.rs (gate counters)`

Acceptance:
- [ ] cargo test -p screenpipe-screen text_regions: a blank image yields no regions and OCR is skipped; a text fixture's boxes map back within 1%
- [ ] `api /vision/metrics` shows ocr_gate_skips and crop_ocr counters increasing during capture

#### W5-07 Hybrid storage format hybrid-parquet-v1 (XL)

- **Items:** db-021
- **Depends on:** W2-01
- **Specs:** specs/db.md db-021; specs/vision.md hybrid-payload-storage (merged)

Opt-in format: SQLite index plus checksummed Parquet payloads (screenpipe_search_v1 / screenpipe_detail_v1) and storage.json; _hybrid_migrations, storage_metadata, upload_bindings, payload_files, frame_payloads, payload_* frame columns, contentless frames_fts, budget/revision/revocation triggers, sealing worker, revision-checked reader and raw_sql restrictions. Nothing changes for non-hybrid databases.

Owner paths:
- `crates/screenpipe-db/src/storage/** (new: descriptor, hybrid migrations, payload tables, sealing worker, reader)`
- `crates/screenpipe-db/Cargo.toml`
- `crates/screenpipe-db/src/db.rs (read-path dispatch region)`

Acceptance:
- [ ] cargo test -p screenpipe-db storage: create a hybrid index, insert frames, seal a day into search/detail parquet with checksums
- [ ] The reader returns the same text as the SQLite path; a revoked payload is not readable; storage.json has the documented fields

#### W5-08 Database self-heal user flow (L)

- **Items:** shell-014
- **Depends on:** W4-09
- **Specs:** specs/shell.md shell-014

Classify DB open errors, keep the UI up and retry transient failures, notify with recover-database/dismiss actions and the database-recovery deep link, commands start_database_recovery/restart_database_verification, DB-wedge restart and recording retry events. Recovery internals come from W3-27/W4-09.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/db_self_heal.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/db_relaunch.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/db_wedge.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/notifications/db_recovery.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/recording.rs (retry events region)`

Acceptance:
- [ ] UI smoke: starting with a quarantine marker shows the recovery notification; start_database_recovery runs recovery and relaunches
- [ ] A simulated transient open error is retried without tearing down the UI (log)

#### W5-09 Tray menu v2 (M)

- **Items:** shell-023
- **Depends on:** W4-20, W2-11
- **Specs:** specs/shell.md shell-023

Per-monitor toggles through /vision/device/start|stop, compound status lines (audio waiting for meeting, meeting detection unavailable, outside work hours), pause/resume notifications, Open first summary, Help submenu, localized templates, Windows tray recreation and menu-install retries. No plan/upgrade rows.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/tray.rs`

Acceptance:
- [ ] UI smoke: toggling a monitor in the tray changes /vision/device/status; with meetings-only and no meeting the tray shows the waiting status
- [ ] Restarting explorer.exe recreates the tray icon

#### W5-10 Workflow video jobs in the app (L)

- **Items:** gap_workflow-sop-video-and-media-app-video-jobs
- **Depends on:** W4-21
- **Specs:** specs/gap_workflow-sop-video-and-media.md app-video-jobs

create_workflow_video(id, scenes) -> {path, captionsPath}, cancel and discard; storage under <appData>/workflow-videos/<uuid> with a temp .render-<id>; data-URL screenshot validation; single-flight; 700 MiB quota and 24 h pruning; progress on workflow-video-<id>.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/workflow_video.rs (new)`
- `apps/screenpipe-app-tauri/lib/workflows/video-jobs.ts (new)`

Acceptance:
- [ ] UI or test harness: create_workflow_video writes video.mp4 and captions.vtt and emits progress events starting with `Preparing video` (exact string, client-visible)
- [ ] A second concurrent call is refused; quota pruning test removes the oldest previews

#### W5-11 Captured-moment replay server and open command (L)

- **Items:** gap_workflow-sop-video-and-media-recording-replay, gap_workflow-sop-video-and-media-captured-moment-open
- **Depends on:** W2-10
- **Specs:** specs/gap_workflow-sop-video-and-media.md recording-replay, captured-moment-open

load_workflow_recording(timestamp, appName) finds the nearest frame through preview-samples and returns a video clip served by a loopback replay server (GET /media/:token with Range, Host/Origin checks, no-store, nosniff) or an inline snapshot via the thumbnail route; release_workflow_recording stops the server when idle; open_workflow_captured_moment validates and navigates the timeline.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/workflows_media.rs (new)`

Acceptance:
- [ ] Test: load_workflow_recording returns kind video with an http://127.0.0.1:<port>/media/<token> URL; a Range request returns 206; a wrong Host returns 403
- [ ] open_workflow_captured_moment with frameId 0 returns `invalid captured moment` (exact string, client-visible)

#### W5-12 Activity history generator (app) (L)

- **Items:** meetings_memory-016
- **Depends on:** W3-04, W3-25
- **Specs:** specs/meetings_memory.md meetings_memory-016

get_activity_history/generate_activity_history/set_activity_sync_enabled run a pi agent over meetings, activity-summary, activity-ledger and search to produce validated JSON entries with evidence, a repair pass, coverage tracking and an auto schedule (activitiesEnabled, activitiesAiPresetId, activitiesIntervalMinutes); a notification when activities have been updated. Uses the user's preset; no vendor quota.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/activity_history.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/pi.rs (session API hook only)`

Acceptance:
- [ ] With a mock local preset, generate_activity_history returns entries that pass validation; with no capture it returns activity_no_data
- [ ] The auto schedule runs at the configured interval (shortened in test)

#### W5-13 Notifications UI v2: inbox window, bell, panel, preferences (L)

- **Items:** meetings_memory-039, frontend-030, frontend-031, meetings_memory-040, frontend-040, meetings_memory-034
- **Depends on:** W3-18, W4-19
- **Specs:** specs/meetings_memory.md 034, 039, 040; specs/frontend.md frontend-030, 031, 040

Bell opens the notification-inbox window with Priority/All views, a high-priority heuristic, unread-high badge, per-item read, copy, source, actions and settings link (5 s polling). Toast gains open source chat and meeting toasts can render in the overlay. Preferences: master switch, snooze presets, quiet hours, grouped categories, per-pipe mute/VIP, meeting reminders (migrated from meetingLiveNotes) and reminder lead time.

Owner paths:
- `apps/screenpipe-app-tauri/app/notification-inbox/page.tsx (new)`
- `apps/screenpipe-app-tauri/components/notification-bell.tsx`
- `apps/screenpipe-app-tauri/app/notification-panel/page.tsx`
- `apps/screenpipe-app-tauri/components/settings/notifications-settings.tsx`
- `apps/screenpipe-app-tauri/src-tauri/src/meeting_live_notes.rs (live-note prompt gate region)`

Acceptance:
- [ ] UI smoke: the bell opens the inbox; expanding an item calls POST /notifications/<id>/read
- [ ] bun test: meetingReminders migrates from meetingLiveNotes; quiet hours persist in notificationPrefs.quietHours

#### W5-14 Starred sessions UI, star shortcut, overlay snooze/anchor, CLI (L)

- **Items:** meetings_memory-012, frontend-029, shell-009
- **Depends on:** W2-04, W4-19
- **Specs:** specs/meetings_memory.md meetings_memory-012; specs/frontend.md frontend-029; specs/shell.md shell-009

starSessionShortcut (Alt+Shift+B on Windows) toggles the starred-sessions window: star 5/15/30/60 min, +15, end, edit times, HD flag, ask in chat, export video; revision conflicts reload; events starred-sessions-changed/-visibility and starred-session-toggle/-state. Overlay one-hour snooze, edge anchors (legacy corners mapped), display choice. CLI star list/start/end/edit.

Owner paths:
- `apps/screenpipe-app-tauri/app/starred-sessions/page.tsx (new)`
- `apps/screenpipe-app-tauri/components/shortcut-reminder.tsx (star button region)`
- `apps/screenpipe-app-tauri/app/shortcut-reminder/page.tsx (snooze/anchor region)`
- `apps/screenpipe-app-tauri/src-tauri/src/shortcuts.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/starred_sessions.rs (new)`
- `crates/screenpipe-engine/src/cli/star.rs (new)`
- `crates/screenpipe-engine/src/cli/mod.rs (subcommand line only)`

Acceptance:
- [ ] UI smoke: Alt+Shift+B opens the window; the 15-minute star action creates a session visible in GET /starred-sessions; +15 bumps the revision
- [ ] screenpipe star list prints the session; snoozing the overlay sets shortcutOverlaySnoozedUntil one hour ahead

#### W5-15 Chat tabs, split pane, side chat, groups and landing cards (L)

- **Items:** frontend-009, frontend-020
- **Depends on:** W4-05, W4-22
- **Specs:** specs/frontend.md frontend-009, 020

Tab strip with status glyphs, split pane, temporary side chat, branch/archive/pin/rename/copy, user sidebar groups, Pinned/Recents/Automations with filters, sort and bulk actions; persisted lastContentAt; fast listing through list_chat_entries_by_mtime. Landing cards gain run-in-agent actions and contextual suggestions gated by showChatSuggestions.

Owner paths:
- `apps/screenpipe-app-tauri/components/chat-sidebar.tsx`
- `apps/screenpipe-app-tauri/components/standalone-chat.tsx (tabs/split/landing regions)`
- `apps/screenpipe-app-tauri/lib/stores/chat-store.ts`
- `apps/screenpipe-app-tauri/lib/chat-storage.ts`
- `apps/screenpipe-app-tauri/components/chat/tab-strip.tsx (new)`
- `apps/screenpipe-app-tauri/components/chat/split-pane.tsx (new)`
- `apps/screenpipe-app-tauri/components/chat/summary-cards.tsx`

Acceptance:
- [ ] bun test chat-store: tabs, groups and lastContentAt persistence
- [ ] UI smoke: open three tabs, split two, branch one, archive one; restart keeps the layout

#### W5-16 Library: memories and artifacts (L)

- **Items:** meetings_memory-006, frontend-021
- **Depends on:** W2-02, W4-22
- **Specs:** specs/meetings_memory.md meetings_memory-006; specs/frontend.md frontend-021

Library section replacing Memories: view switcher, filters (labels, sources, people, dates), memory CRUD via /memories, artifacts browser via /artifacts with SAF badges, open in viewer or source chat, ask AI, delete, 30 s counters, artifact deep links.

Owner paths:
- `apps/screenpipe-app-tauri/components/library/** (new)`
- `apps/screenpipe-app-tauri/components/settings/memories-section.tsx`
- `apps/screenpipe-app-tauri/app/(main)/home/page.tsx (library section mount)`

Acceptance:
- [ ] UI smoke: Library lists memories and artifacts; deleting an artifact calls DELETE /artifacts/<id>
- [ ] bun test: filter reducer and SAF badge mapping

#### W5-17 Meetings workspace UI (L)

- **Items:** frontend-024
- **Depends on:** W2-08, W4-22
- **Specs:** specs/frontend.md frontend-024

Replay player controls, streaming summary surface polling /meetings/:id/summary-status with model change, meeting chat panel, copy_rich_text_to_clipboard, async export with progress events, editor slash menu and format toolbar, join suggestions and calendar nudge.

Owner paths:
- `apps/screenpipe-app-tauri/components/meeting-notes/**`
- `apps/screenpipe-app-tauri/src-tauri/src/meeting_export.rs (async start_export_recording + export:event)`

Acceptance:
- [ ] UI smoke: a meeting note shows summary state from summary-status; export shows progress and produces an mp4
- [ ] bun test: slash menu inserts the expected nodes

#### W5-18 Timeline and search window v2 (L)

- **Items:** frontend-025, frontend-026, frontend-027
- **Depends on:** W3-02, W4-22
- **Specs:** specs/frontend.md frontend-025, 026, 027

Timeline selection export (POST /export), on-demand daily summary with the user's model, a placeholder for frames captured while screenshots were paused, timeline-* events from native menus, native timeline commands (unavailable on Windows), search window recent-activity chips, timelineOrigin, search-hidden/search-reset events.

Owner paths:
- `apps/screenpipe-app-tauri/components/rewind/**`
- `apps/screenpipe-app-tauri/app/search/page.tsx`
- `apps/screenpipe-app-tauri/src-tauri/src/native_timeline.rs (new, Windows returns unavailable)`

Acceptance:
- [ ] UI smoke: selecting a range and exporting produces a file; frames without snapshots show the placeholder
- [ ] native_timeline_is_available resolves false on Windows (bun test against bindings)

#### W5-19 Settings navigation v2, general, AI features, permissions (L)

- **Items:** frontend-035, frontend-041, frontend-042, frontend-043
- **Depends on:** W4-22, W4-20
- **Specs:** specs/frontend.md frontend-035, 041, 042, 043

Grouped navigation (four groups: capture and data, AI, account, app) with audio, activities, ai-settings and permissions sections, default 'recording', last section remembered, fuse search with field highlight; general/appearance deltas (keepSearchAvailableAfterQuit, release channel, live chat-on-top, hide overlay from capture, star shortcut row); split AI features vs models with a mandatory connection test and Gemini via OpenAI-compatible URL; macOS permissions section hidden on Windows.

Owner paths:
- `apps/screenpipe-app-tauri/app/(main)/settings/page.tsx`
- `apps/screenpipe-app-tauri/components/settings/settings-search.tsx`
- `apps/screenpipe-app-tauri/components/settings/general-settings.tsx`
- `apps/screenpipe-app-tauri/components/settings/display-section.tsx`
- `apps/screenpipe-app-tauri/components/settings/shortcut-section.tsx`
- `apps/screenpipe-app-tauri/components/settings/ai-presets.tsx (connection test region)`
- `apps/screenpipe-app-tauri/components/settings/ai-features-section.tsx (new)`
- `apps/screenpipe-app-tauri/components/settings/permissions-section.tsx (new)`
- `apps/screenpipe-app-tauri/components/settings/input-monitoring-card.tsx`

Acceptance:
- [ ] UI smoke: grouped nav renders; search jumps to a field; reopening settings returns to the last section
- [ ] bun test: preset save is blocked until the mocked connection test passes

#### W5-20 Screen and audio settings (L)

- **Items:** frontend-036, frontend-037
- **Depends on:** W3-14, W3-15, W4-22
- **Specs:** specs/frontend.md frontend-036, 037

Separate screen-context vs screenshots toggles, quality presets, capture-frequency slider, scroll capture and recordingDetail, keep awake; audioCaptureMode, engine list with test_openai_compatible_transcription before enabling, unified aecMode, single-app Windows audio exclusion, Bluetooth mic toggle, live captions provider limited to local/BYO.

Owner paths:
- `apps/screenpipe-app-tauri/components/settings/recording-settings.tsx`
- `apps/screenpipe-app-tauri/components/settings/battery-saver-section.tsx`
- `apps/screenpipe-app-tauri/components/settings/meeting-apps-picker.tsx`
- `apps/screenpipe-app-tauri/components/settings/audio-settings-section.tsx (new)`

Acceptance:
- [ ] UI smoke: audioCaptureMode and aecMode selections persist and only platform-valid aecMode options appear
- [ ] bun test: enabling OpenAI-compatible transcription is blocked when the mocked test fails

#### W5-21 Deep links v2 (M)

- **Items:** frontend-045, frontend-M01, cloud-V01
- **Depends on:** W4-22
- **Specs:** specs/frontend.md frontend-045, M01; specs/cloud.md cloud-V01; specs/shell.md shell-027 (merged)

Hosts activity, artifact, chat, database-recovery, database-restart-verification, first-run-agent, first-run-summary, home, live-view, pipe, scheduled, settings, workflows (reviewRequest dedupe), calendar-connections; versioned external links chat/new?v=1&prompt= and pipe/<name>?v=1 with strict validation and no auto-send. Announcements are never fetched remotely; only a local /notify announcement event may feed that UI and never offers pipe installs.

Owner paths:
- `apps/screenpipe-app-tauri/components/deeplink-handler.tsx`
- `apps/screenpipe-app-tauri/lib/deeplinks/** (new validator)`

Acceptance:
- [ ] bun test: chat/new?v=1&prompt=hi accepted; v=2, unknown params, credentials, a port, a fragment or a >4096-byte prompt rejected; pipe/../x rejected
- [ ] grep -rn 'app-announcement' apps/screenpipe-app-tauri returns nothing

#### W5-22 Viewer, permission recovery and help refresh (M)

- **Items:** frontend-028, frontend-033, frontend-055
- **Depends on:** W2-26, W4-22
- **Specs:** specs/frontend.md frontend-028, 033, 055

Viewer renders HTML in a no-network sandbox with a source toggle, refuses unsafe files and shows diagnostics listing the locations it searched; permission recovery adds the restart prompt and retry_screenpipe; Help gains local log bundle, getting started, shortcuts and rerun-guide entries with GitHub links.

Owner paths:
- `apps/screenpipe-app-tauri/app/viewer/page.tsx`
- `apps/screenpipe-app-tauri/src-tauri/src/viewer.rs`
- `apps/screenpipe-app-tauri/app/permission-recovery/page.tsx`
- `apps/screenpipe-app-tauri/src-tauri/src/permissions.rs (restart/retry commands)`
- `apps/screenpipe-app-tauri/components/settings/feedback-section.tsx`

Acceptance:
- [ ] UI smoke: an HTML file opens in the sandbox and cannot load remote resources; an .exe is refused
- [ ] Help > share logs uses the local bundle from W0-10

#### W5-23 Connections UI catalogue (L)

- **Items:** connections-ui-catalog, frontend-050
- **Depends on:** W3-19, W3-20, W3-21, W4-18
- **Specs:** specs/connections.md ui-catalog; specs/frontend.md frontend-050

New tiles and renames (an advanced-connections group, an other-calendars tile and a browser-activity tile), Advanced token/API-key fallbacks for OAuth and MCP tiles, scopeVariant deep links, Bee and IMAP panels with test/mailboxes, MCP registry browser, oauth_connect variant; Composio/login gating replaced by BYO prompts.

Owner paths:
- `apps/screenpipe-app-tauri/components/settings/connections-section.tsx`
- `apps/screenpipe-app-tauri/components/settings/imap-card.tsx (new)`
- `apps/screenpipe-app-tauri/components/settings/bee-card.tsx (new)`
- `apps/screenpipe-app-tauri/public/images/excalidraw.svg (own)`
- `apps/screenpipe-app-tauri/public/images/google-drive.svg (own)`

Acceptance:
- [ ] UI smoke: Bee pairing and IMAP connect end to end; a scopeVariant deep link opens the Slack panel with read_write selected
- [ ] No Composio tile or link is present (grep composio in components returns nothing)

#### WR-02 Sync and archive to bring-your-own storage (L)

- **Items:** (restore) cloud-019, (restore) cloud-037
- **Depends on:** W0-08, W4-10
- **Specs:** specs/cloud.md cloud-019, cloud-037

Restore cross-device sync and cloud archive using storage the user controls: S3-compatible (AWS/R2/B2/MinIO), WebDAV (Nextcloud etc.) and a plain folder (e.g. inside OneDrive/Dropbox). Keep the base client-side encryption model (user passphrase -> key, encrypted chunks and manifests; the storage provider never sees plaintext), the existing sync manifest/merge semantics, LAN remote_sync, and the http_put target. Credentials live in the secret store. Archive moves aged media to the chosen backend per retention settings and can restore on demand. The UI exposes backend choice, test connection, passphrase setup and status.

Owner paths:
- `crates/screenpipe-core/src/sync/ (backends: s3.rs, webdav.rs, folder.rs new)`
- `crates/screenpipe-sync/src/`
- `apps/screenpipe-app-tauri/src-tauri/src/sync.rs`
- `apps/screenpipe-app-tauri/components/settings/sync-settings.tsx`
- `apps/screenpipe-app-tauri/components/settings/archive-settings.tsx`
- `crates/screenpipe-engine/src/routes/data_sync_proxy.rs`

Acceptance:
- [ ] cargo test for the sync crate against a local MinIO-compatible mock (or s3s fake) and a temp-folder backend: round-trip sync of two data dirs
- [ ] Ciphertext-only check: objects written to the backend contain no plaintext OCR text or audio
- [ ] UI smoke: configure a folder backend, sync, then archive and restore a chunk

### Wave 6: Agent/workflow UIs, piggyback, hybrid bulk

| WP | Size | Title | Depends on | Items |
|---|---|---|---|---|
| W6-01 | XL | ACP presets and coding-agent chat UI | W4-03, W5-15 | agents_pipes-017, frontend-015 |
| W6-02 | M | Run destinations and conversation continuity | W5-01 | agents_pipes-009, agents_pipes-m02 |
| W6-03 | L | Connection-source pipe triggers | W4-02, W3-19, W3-20, W2-21 | agents_pipes-008 |
| W6-04 | L | External chat import and chat-control broker | W5-15 | agents_pipes-019, frontend-010 |
| W6-05 | XL | Meeting piggyback (smart recording) | W5-04, W4-12, W4-16 | audio-016 |
| W6-06 | XL | Hybrid bulk column archival and screenpipe_elements virtual table | W5-07 | db-022 |
| W6-07 | XL | Workflows workspace UI | W4-01, W4-22 | frontend-047 |
| W6-08 | L | Workflow ready-to-review notifications | W4-01, W3-18 | gap_workflow-sop-video-and-media-workflow-ready-notification |
| W6-09 | L | Video render CLI mode and Pi video-sop extension | W5-10, W2-27 | gap_workflow-sop-video-and-media-render-cli, gap_workflow-sop-video-and-media-pi-video-sop-extension |
| W6-10 | M | Workflow-to-skill drafting | W4-01, W3-24, W3-25 | agents_pipes-046 |
| W6-11 | L | Built-in MCP server v0.20 and portable MCP launcher | W4-01, W2-03, W5-01 | agents_pipes-030 |
| W6-12 | XL | Live Views dashboards UI | W4-11, W5-16 | frontend-022 |
| W6-13 | L | Activity view UI | W5-12, W3-04 | meetings_memory-017, frontend-023 |
| W6-14 | M | Recording status popover and first-run guide | W3-30, W2-11, W4-22 | frontend-007, frontend-019 |
| W6-15 | L | Privacy settings v2 | W3-13, W3-10, W4-15 | frontend-038 |
| W6-16 | L | Onboarding v2 | W5-01, W4-22 | frontend-034 |
| W6-17 | L | Automations (pipes) UI v2 | W5-01 | frontend-049 |
| W6-18 | L | Right sidebar: multi-tab owned browser, files, connections | W5-15 | frontend-013 |
| W6-19 | L | Shortcut overlay v2 (webview) | W5-14, W3-07, W3-29 | frontend-032 |
| WR-01 | L | Pipe store on a GitHub-hosted registry | W0-08, W2-22, W5-01 | (restore) cloud-021, (restore) agents_pipes-037 |
| WR-03 | XL | Freenpipe Hub: self-hostable team server and team client | W0-05, W3-02, W4-10, W5-16 | (restore) cloud-022, (restore) team memory |

#### W6-01 ACP presets and coding-agent chat UI (XL)

- **Items:** agents_pipes-017, frontend-015
- **Depends on:** W4-03, W5-15
- **Specs:** specs/agents_pipes.md agents_pipes-017; specs/frontend.md frontend-015

AIPreset gains provider 'acp' and acpAgent {id, command, args, env, authMethod, config, modeId, approvalMode}; PiProviderConfig adds backend/allowedTools/resumeSessionId/unattended; per-chat presetId. UI: agent catalogue, probe/install/login, in-chat mode and config options, approval modes, thinking level, reauth. No cloud routing option.

Owner paths:
- `apps/screenpipe-app-tauri/components/settings/ai-presets.tsx (ACP region)`
- `apps/screenpipe-app-tauri/components/standalone-chat.tsx (ACP mode/approval/thinking region)`
- `apps/screenpipe-app-tauri/components/chat/acp/** (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/store.rs (AIPreset fields)`
- `crates/screenpipe-engine/src/cli/presets.rs`
- `apps/screenpipe-app-tauri/lib/hooks/use-settings.tsx (AIPreset type only)`

Acceptance:
- [ ] UI smoke: create a codex-acp preset (user-initiated install), chat with it, switch approval mode and thinking level
- [ ] bun test: preset schema accepts provider acp and rejects unknown approval modes

#### W6-02 Run destinations and conversation continuity (M)

- **Items:** agents_pipes-009, agents_pipes-m02
- **Depends on:** W5-01
- **Specs:** specs/agents_pipes.md agents_pipes-009, m02

Scheduled runs can stream into an existing chat; history:true resumes with conversation_id persisted; ACP session map; scheduled-task preamble line; per-device .local-run-destinations.json so a synced pipe never targets another device's chat (corrupt file ignored with a warning).

Owner paths:
- `crates/screenpipe-core/src/pipes/mod.rs (run_in region)`
- `crates/screenpipe-core/src/pipes/run_destinations.rs (new)`
- `crates/screenpipe-core/src/agents/pi.rs (resume session region)`
- `apps/screenpipe-app-tauri/src-tauri/src/pi.rs (stream-to-chat region)`
- `apps/screenpipe-app-tauri/lib/chat-storage.ts (existing_chat append)`

Acceptance:
- [ ] Test: a pipe with run_in existing_chat appends its output to the chosen chat file
- [ ] .local-run-destinations.json stores the mapping; a corrupt file is ignored with a warning; history:true resumes the same conversation_id

#### W6-03 Connection-source pipe triggers (L)

- **Items:** agents_pipes-008
- **Depends on:** W4-02, W3-19, W3-20, W2-21
- **Specs:** specs/agents_pipes.md agents_pipes-008; specs/connections.md connection-triggers (merged)

Poll connected apps through the local /connections proxies (Slack, Notion, IMAP, Google Calendar, Outlook, GitHub, Linear, Todoist, Obsidian, spoken phrases) with cursors in .connection-triggers.json and fire pipes through connection_trigger events with retry then cursor advance.

Owner paths:
- `crates/screenpipe-core/src/pipes/connection_triggers.rs (new)`
- `crates/screenpipe-core/src/pipes/mod.rs (subscription registration line)`

Acceptance:
- [ ] cargo test with a mocked IMAP proxy: a new message fires one connection_trigger for a subscribed pipe; the cursor advances
- [ ] A voice-phrase subscription fires on a matching transcript

#### W6-04 External chat import and chat-control broker (L)

- **Items:** agents_pipes-019, frontend-010
- **Depends on:** W5-15
- **Specs:** specs/agents_pipes.md agents_pipes-019; specs/frontend.md frontend-010

Import ~/.claude/projects and ~/.codex/sessions JSONL (7-day lookback, 100 sessions, skip > 32 MiB) into chats with importedFrom and deterministic ids; local TCP broker (SCREENPIPE_CHAT_CONTROL_ADDR/TOKEN) with search/send/worktree actions across local AI chats.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/chat_control.rs (new)`
- `crates/screenpipe-core/src/agents/chat_control.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/external_chat_import.rs (new)`
- `apps/screenpipe-app-tauri/components/chat-sidebar.tsx (import region)`
- `apps/screenpipe-app-tauri/lib/chat-storage.ts (importedFrom)`

Acceptance:
- [ ] Test with fixture JSONL files: import creates chats with importedFrom; re-import updates instead of duplicating
- [ ] The broker rejects a wrong token and answers a search action with the right one

#### W6-05 Meeting piggyback (smart recording) (XL)

- **Items:** audio-016
- **Depends on:** W5-04, W4-12, W4-16
- **Specs:** specs/audio.md audio-016, audio-015 (merged)

During a meeting capture the meeting app's audio as 'Meeting Tap (output)' with strikes and give-up, follow the meeting app's mic (process/configured/system_default), suspend other mics, restore configured devices afterwards and classify the outcome (full/partial/stable_fallback/no_pid/unavailable). Absorbs the per-pid input resolution from audio-015. Telemetry stays local.

Owner paths:
- `crates/screenpipe-audio/src/audio_manager/manager.rs (piggyback region)`
- `crates/screenpipe-audio/src/audio_manager/device_monitor.rs`
- `crates/screenpipe-audio/src/meeting_audio/windows.rs (new)`
- `crates/screenpipe-audio/src/piggyback.rs (new)`

Acceptance:
- [ ] Manual Teams call: /audio/device/status lists 'Meeting Tap (output)'; afterwards the configured devices are restored
- [ ] cargo test: outcome classification for each case with mocked device state

#### W6-06 Hybrid bulk column archival and screenpipe_elements virtual table (XL)

- **Items:** db-022
- **Depends on:** W5-07
- **Specs:** specs/db.md db-022

_bulk_files/_bulk_legacy_sql catalogue, _archive_* columns, contentless <t>_fts and hybrid_bulk_* triggers for audio_transcriptions, ui_events, semantic_items, pipe_executions, meeting_transcript_segments and outputs; elements becomes a vtab over _bulk_element_rows plus Parquet ranges with element sealing.

Owner paths:
- `crates/screenpipe-db/src/storage/bulk/** (new)`
- `crates/screenpipe-db/src/storage/elements_vtab.rs (new)`
- `crates/screenpipe-db/src/db.rs (element insert region: explicit ids)`

Acceptance:
- [ ] cargo test -p screenpipe-db storage::bulk: archived columns read back transparently; elements vtab returns the same rows as the table for a frame
- [ ] Contentless FTS search on archived audio text returns the expected rows

#### W6-07 Workflows workspace UI (XL)

- **Items:** frontend-047
- **Depends on:** W4-01, W4-22
- **Specs:** specs/frontend.md frontend-047

Chat/Workflows workspace switcher; catalogue via analyze_workflows and agent pipes; editor (steps, timing, captured moments, voice notes, corrections); SOP documents with HTML export; skill generation entry; assistant. No Tinfoil verification, contribution sharing or web SOP editor.

Owner paths:
- `apps/screenpipe-app-tauri/components/workflows/** (new)`
- `apps/screenpipe-app-tauri/app/(main)/home/page.tsx (workspace switcher mount)`
- `apps/screenpipe-app-tauri/src-tauri/src/workflows_runtime.rs (new)`

Acceptance:
- [ ] UI smoke: the Workflows workspace lists /workflows entries; editing a step posts /workflows/edits; SOP export writes an HTML file
- [ ] grep in components/workflows for trajectories/sops/tinfoil returns nothing

#### W6-08 Workflow ready-to-review notifications (L)

- **Items:** gap_workflow-sop-video-and-media-workflow-ready-notification
- **Depends on:** W4-01, W3-18
- **Specs:** specs/gap_workflow-sop-video-and-media.md workflow-ready-notification

Read catalog.json (backup fallback), select saved supported workflows, keep notification-ledger.json with fingerprints and suppression reasons (no_meaningful_change, daily_limit, ledger_full, paused, already_viewing, duplicate, transport_failed, display_timeout), coalesce hourly with workflow-review:<key> ids and a workflows deep link.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/notifications/workflow.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/notifications/mod.rs (dispatch line)`
- `apps/screenpipe-app-tauri/src-tauri/src/notifications/routes.rs (workflow branch)`
- `apps/screenpipe-app-tauri/src-tauri/src/notifications/client.rs`

Acceptance:
- [ ] Test: a new saved workflow yields one notification with a workflows?workflow=...&reviewRequest=... deep link; the same fingerprint is suppressed
- [ ] An unreadable ledger fails closed (no toast, logged)

#### W6-09 Video render CLI mode and Pi video-sop extension (L)

- **Items:** gap_workflow-sop-video-and-media-render-cli, gap_workflow-sop-video-and-media-pi-video-sop-extension
- **Depends on:** W5-10, W2-27
- **Specs:** specs/gap_workflow-sop-video-and-media.md render-cli, pi-video-sop-extension

--render-workflow-video <dir> before single-instance: exclusive render.lock, reads render-scenes.json with caps, renders into rendered/, NDJSON progress on stdout, stdin EOF cancels, exit 0 only on success. Pi extension read/edit/render_video_sop over video-project.json with VideoDraft/VideoEdit validation and tool guards.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/workflow_video_cli.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/main.rs (argv table entry only)`
- `apps/screenpipe-app-tauri/src-tauri/src/pi.rs (SCREENPIPE_VIDEO_CLI env line)`
- `apps/screenpipe-app-tauri/src-tauri/assets/extensions/workflow-video.ts (new)`
- `apps/screenpipe-app-tauri/src-tauri/assets/skills/video-sop/SKILL.md (new, own text)`
- `apps/screenpipe-app-tauri/src-tauri/src/pi_extensions_manifest.rs (entry)`

Acceptance:
- [ ] Freenpipe.exe --render-workflow-video <fixture dir> writes rendered/video.mp4 and NDJSON progress, exit 0; closing stdin cancels with non-zero exit
- [ ] bun test: VideoEdit validation limits (50 changes, pace range, focus requires image)

#### W6-10 Workflow-to-skill drafting (M)

- **Items:** agents_pipes-046
- **Depends on:** W4-01, W3-24, W3-25
- **Specs:** specs/agents_pipes.md agents_pipes-046

generate_workflow_skill/save_workflow_skill draft {name, description, instructions} from a reviewed workflow (>= 2 stages) with the user's preset (own prompt) and save it as an owned local skill linking to get-workflow. No sign-in gate.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/workflow_skill.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/skills.rs (workflow skill save region)`

Acceptance:
- [ ] Test with a mock preset: a 2-stage workflow yields a draft; a 1-stage workflow returns an error; saving writes the skill with the ownership marker

#### W6-11 Built-in MCP server v0.20 and portable MCP launcher (L)

- **Items:** agents_pipes-030
- **Depends on:** W4-01, W2-03, W5-01
- **Specs:** specs/agents_pipes.md agents_pipes-030

Add tools synced-devices, search-synced-content, get-feedback, list-pipes, create-pipe, run-pipe, pipe-logs, team-frame (returns unavailable), list-workflows, get-workflow and screenpipe-skills; --screenpipe-portable-mcp runs the MCP server through bundled bun with SCREENPIPE_MCP_TRANSPORT/HOST/PORT/PATH.

Owner paths:
- `packages/screenpipe-mcp/src/**`
- `apps/screenpipe-app-tauri/src-tauri/src/portable_mcp.rs (new)`

Acceptance:
- [ ] cd packages/screenpipe-mcp && bun test; tools/list over stdio includes the new tool names
- [ ] Freenpipe.exe --screenpipe-portable-mcp with SCREENPIPE_MCP_TRANSPORT=http listens on SCREENPIPE_MCP_PORT

#### W6-12 Live Views dashboards UI (XL)

- **Items:** frontend-022
- **Depends on:** W4-11, W5-16
- **Specs:** specs/frontend.md frontend-022

BrainView/Slot/Canvas/TemplateKit model (live-view-canvas.v1, period policies, markdown/list/bar/line/timeline components) with revisioned saves, pipe-bound blocks, editor and React Flow canvas, AI composer with diff review (user's preset), card feedback and item actions, templates. Vendor share targets are omitted.

Owner paths:
- `apps/screenpipe-app-tauri/components/library/live-views/** (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/brain_views.rs (new)`

Acceptance:
- [ ] UI smoke: create a view from a bundled template, edit the canvas, save; a concurrent save prompts reload on 409
- [ ] bun test: canvas schema validation and period policy resolution

#### W6-13 Activity view UI (L)

- **Items:** meetings_memory-017, frontend-023
- **Depends on:** W5-12, W3-04
- **Specs:** specs/meetings_memory.md meetings_memory-017; specs/frontend.md frontend-023

Range today/24h/7d/custom, AI preset picker and refresh; combines /activity-ledger, a slim /activity-summary and /meetings into day-grouped cards with evidence links to Meetings or Timeline; generated history with coverage and no-data reasons; settings for activitiesEnabled and interval.

Owner paths:
- `apps/screenpipe-app-tauri/components/activity/** (new)`
- `apps/screenpipe-app-tauri/app/(main)/home/page.tsx (activity section mount)`
- `apps/screenpipe-app-tauri/components/settings/activities-section.tsx (new)`

Acceptance:
- [ ] UI smoke: the Activity section renders day-grouped cards for today; evidence icons open the timeline
- [ ] bun test: empty states for no_capture_in_range and empty_but_recording

#### W6-14 Recording status popover and first-run guide (M)

- **Items:** frontend-007, frontend-019
- **Depends on:** W3-30, W2-11, W4-22
- **Specs:** specs/frontend.md frontend-007, 019

Sidebar status popover with per-device pause (vision and audio device routes), pause/resume all and a low-storage warning from get_active_data_dir + get_disk_usage vs get_low_disk_guard_config; one-time three-step coach-mark guide (firstRunGuideDone).

Owner paths:
- `apps/screenpipe-app-tauri/components/recording-status-popover.tsx (new)`
- `apps/screenpipe-app-tauri/components/app-sidebar.tsx (status trigger region)`
- `apps/screenpipe-app-tauri/components/first-run-guide.tsx (new)`
- `apps/screenpipe-app-tauri/app/(main)/home/page.tsx (guide mount)`

Acceptance:
- [ ] UI smoke: pausing a monitor in the popover updates /vision/device/status; a mocked low threshold shows the warning
- [ ] The guide shows once and sets firstRunGuideDone

#### W6-15 Privacy settings v2 (L)

- **Items:** frontend-038
- **Depends on:** W3-13, W3-10, W4-15
- **Specs:** specs/frontend.md frontend-038

Apps/Websites tabs with category switches (categoryOwnedFilters), DomainRule editors for ignoredUrls/includedUrls, PII Off/Basic/Smart with labels, columns, pseudonyms and agent-log redaction, click capture, enhanced incognito, API key regenerate; piiBackend stays local unless an explicit endpoint is set; no remote-log toggle.

Owner paths:
- `apps/screenpipe-app-tauri/components/settings/privacy-section.tsx`
- `apps/screenpipe-app-tauri/components/settings/window-picker.tsx`
- `apps/screenpipe-app-tauri/components/settings/website-rules-editor.tsx (new)`

Acceptance:
- [ ] bun test: the zod schema accepts DomainRule objects and legacy strings and rejects invalid domains
- [ ] UI smoke: adding includedUrls example.com stops capture of other sites (frames show the skip reason)

#### W6-16 Onboarding v2 (L)

- **Items:** frontend-034
- **Depends on:** W5-01, W4-22
- **Specs:** specs/frontend.md frontend-034

Step flow permissions/timeline/engine/recommended-setup/first-task (vendor login, acquisition and plan steps dropped) with the legacy step map, trust disclosure with the data dir, low-tier screenshot choice, recommended pipes install via /pipes/bundled/:id/install with typed errors; completion opens the Home library.

Owner paths:
- `apps/screenpipe-app-tauri/app/onboarding/page.tsx`
- `apps/screenpipe-app-tauri/components/onboarding/**`

Acceptance:
- [ ] UI smoke on a fresh store: onboarding completes without network access and installs a recommended pipe
- [ ] bun test: legacy currentStep values map to the new steps

#### W6-17 Automations (pipes) UI v2 (L)

- **Items:** frontend-049
- **Depends on:** W5-01
- **Specs:** specs/frontend.md frontend-049

Schedule editor with /pipes/schedule/preview, preset chain, effort, run and chat destinations, per-pipe connections, history reset, one-chat mode and store auto-update; cloud-agent runner configuration is BYO; sharing exports a file.

Owner paths:
- `apps/screenpipe-app-tauri/components/settings/pipes-section.tsx`
- `apps/screenpipe-app-tauri/components/pipe-store.tsx`
- `apps/screenpipe-app-tauri/lib/hooks/use-pipes.ts`
- `apps/screenpipe-app-tauri/components/pipes/** (new)`

Acceptance:
- [ ] UI smoke: editing a schedule shows next runs from the preview route and saves schedule_config to pipe.md
- [ ] bun test: use-pipes paging over /pipes pagination

#### W6-18 Right sidebar: multi-tab owned browser, files, connections (L)

- **Items:** frontend-013
- **Depends on:** W5-15
- **Specs:** specs/frontend.md frontend-013; specs/connections.md owned-browser-tabs (merged)

Right panel with owned-browser tabs (owned_browser_tab_navigate/set_bounds/hide/close/clear_browsing_data, owned_browser_history, navigate owner/reveal), Files & outputs list from /artifacts and a Connections tab; cookie-access prompts kept.

Owner paths:
- `apps/screenpipe-app-tauri/components/browser-sidebar.tsx`
- `apps/screenpipe-app-tauri/components/right-sidebar/** (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/owned_browser.rs`
- `apps/screenpipe-app-tauri/src-tauri/src/owned_browser_cookies.rs`

Acceptance:
- [ ] UI smoke: open two browser tabs, go back/forward, close one; Files & outputs lists artifacts
- [ ] bun run bindings:check includes the owned_browser_tab_* commands

#### W6-19 Shortcut overlay v2 (webview) (L)

- **Items:** frontend-032
- **Depends on:** W5-14, W3-07, W3-29
- **Specs:** specs/frontend.md frontend-032

Recording-health incidents with restart/dismiss/quit-reopen, live meeting transcript from /ws/meeting-overlay with echo dedupe and stop, drag-to-anchor, one-hour snooze, star button and size scale.

Owner paths:
- `apps/screenpipe-app-tauri/app/shortcut-reminder/**`
- `apps/screenpipe-app-tauri/components/shortcut-reminder.tsx`
- `apps/screenpipe-app-tauri/src-tauri/src/native_shortcut_reminder.rs (set_shortcut_overlay_anchor)`

Acceptance:
- [ ] UI smoke: during a manual meeting the overlay shows live transcript lines; stopping capture shows an incident with restart
- [ ] Dragging the overlay to the right edge persists the anchor

#### WR-01 Pipe store on a GitHub-hosted registry (L)

- **Items:** (restore) cloud-021, (restore) agents_pipes-037
- **Depends on:** W0-08, W2-22, W5-01
- **Specs:** specs/cloud.md cloud-021; specs/agents_pipes.md agents_pipes-037

Make the pipe store work out of the box without vendor servers. Define a static registry format (index.json listing id, name, description, author, version, permissions, sha256, source path) served from raw.githubusercontent.com of normiecore/Freenpipe (registry/ directory on main) as the DEFAULT registry URL, overridable by SCREENPIPE_API_BASE_URL or a settings key. Implement list/search/detail/install/update against it with sha256 verification and the existing typed registry_* errors. 'Publish' produces a ready-to-PR pipe folder plus instructions (opens the GitHub new-PR page); a CI workflow validates registry PRs (schema, sha256, frontmatter, no vendor hosts). Seed the registry with the built-in pipe catalogue once W7-08 lands (until then, with 3 sample pipes written fresh).

Owner paths:
- `registry/ (new: index.json, pipes/<id>/pipe.md, schema.json, README.md)`
- `crates/screenpipe-engine/src/routes/pipe_store.rs`
- `crates/screenpipe-engine/src/cli/pipe.rs`
- `crates/screenpipe-engine/src/cli/install.rs`
- `apps/screenpipe-app-tauri/components/pipe-store.tsx`
- `apps/screenpipe-app-tauri/components/settings/pipes-section.tsx`
- `.github/workflows/registry-validate.yml (new)`

Acceptance:
- [ ] Harness: /pipes/store lists the registry's pipes with no env set; install of a sample pipe verifies sha256 and the pipe runs
- [ ] A tampered sha256 yields the typed registry error; SCREENPIPE_API_BASE_URL override points the store at a local static server
- [ ] bun run build + UI smoke: store tab lists, installs and updates a pipe; publish produces a valid registry folder

#### WR-03 Freenpipe Hub: self-hostable team server and team client (XL)

- **Items:** (restore) cloud-022, (restore) team memory
- **Depends on:** W0-05, W3-02, W4-10, W5-16
- **Specs:** specs/cloud.md cloud-022; specs/meetings_memory.md (team memory, if present)

Restore Team as a self-hostable product. New crate freenpipe-hub: a small axum server (single binary + Dockerfile, SQLite or Postgres) implementing the team API surface the client expects (recorded in specs/cloud.md cloud-022 and the base client code): org/team creation, invite links, seats and admin roles, per-member API tokens, opt-in sharing policies, shared team memory (reuse crates/screenpipe-team-memory), and admin query of teammates' shared history (search over data members explicitly publish; never raw unshared data). Members' engines push only what their sharing policy allows, end-to-end over TLS with per-member tokens. Client side: restore the team settings UI, the `screenpipe team` CLI and MCP TEAM_API from the MIT base history (W0-05 removed their vendor wiring) and point them at a configurable hub URL (settings + SCREENPIPE_TEAM_HUB_URL). No hub configured = Team hidden. Document deployment (Docker, Windows service) in docs/hub.md.

Owner paths:
- `crates/freenpipe-hub/ (new server crate + Dockerfile)`
- `crates/screenpipe-team-memory/`
- `crates/screenpipe-engine/src/cli/team.rs (restore + rewire)`
- `apps/screenpipe-app-tauri/components/settings/team-section.tsx (restore + rewire)`
- `apps/screenpipe-app-tauri/lib/hooks/use-team.ts (restore + rewire)`
- `packages/screenpipe-mcp/src/index.ts (TEAM_API -> hub URL)`
- `docs/hub.md (new)`

Acceptance:
- [ ] cargo test -p freenpipe-hub: invite/join/roles/token auth; a member's search does not return another member's unshared data
- [ ] Harness: hub on :4040 + two engines on isolated data dirs; member A shares memories, admin queries them via `screenpipe team search`
- [ ] UI smoke: team section joins a hub via invite link and lists members; with no hub URL the section is hidden

### Wave 7: Hybrid lifecycle, overlays, chat follow-ups

| WP | Size | Title | Depends on | Items |
|---|---|---|---|---|
| W7-01 | XL | Storage migration lifecycle (SQLite to hybrid) | W6-06 | db-023 |
| W7-02 | XL | Hybrid compact, sparse reclaim, backup bundle and retention strip | W6-06, W3-09 | db-024, gap_retention-v2-engine-008 |
| W7-03 | L | SOP video project workspace and gate removal | W6-07, W6-09 | gap_workflow-sop-video-and-media-video-project-workspace, gap_workflow-sop-video-and-media-entitlement-and-telemetry |
| W7-04 | L | Native Win32 shortcut overlay pill | W6-19 | shell-010 |
| W7-05 | M | Chat composer upgrades | W6-01 | frontend-011 |
| W7-06 | L | First-run summary | W5-12, W6-01 | meetings_memory-041, frontend-018 |
| W7-07 | L | Agent worktrees and per-chat coding workspace | W6-01, W6-04 | agents_pipes-020, frontend-017 |
| W7-08 | L | Built-in pipe catalogue and builtin text migrations | W6-03, W4-01, W3-06 | agents_pipes-012 |
| WR-04 | L | Composio integrations (BYO key) and workflow sharing | W2-20, W6-07, WR-01 | (restore) cloud-038, (restore) cloud-035 |

#### W7-01 Storage migration lifecycle (SQLite to hybrid) (XL)

- **Items:** db-023
- **Depends on:** W6-06
- **Specs:** specs/db.md db-023; specs/shell.md shell-015 (merged)

User-initiated offline-copy or in-place conversion with journals (storage-migration.json, -complete.json, diagnostics, storage-migration-error.txt, pendingStorageMigration) and _storage_conversion_* tables; SHA-256 table parity, FTS parity, identity checks, activation, explicit delete-original and secrets recovery; Tauri commands get_storage_migration_status/activity, start/cancel_storage_migration, delete_original_storage_database, get_active_data_dir and the storage-migration-activity event.

Owner paths:
- `crates/screenpipe-db/src/storage/lifecycle.rs (new)`
- `crates/screenpipe-db/src/storage/in_place.rs (new)`
- `crates/screenpipe-db/src/storage/parity.rs (new)`
- `crates/screenpipe-db/src/storage/diagnostics.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/storage_migration.rs (new)`

Acceptance:
- [ ] cargo test: offline-copy migration of a fixture DB passes table and FTS parity and activates
- [ ] Cancelling midway then restarting resumes; delete_original_storage_database is refused before activation

#### W7-02 Hybrid compact, sparse reclaim, backup bundle and retention strip (XL)

- **Items:** db-024, gap_retention-v2-engine-008
- **Depends on:** W6-06, W3-09
- **Specs:** specs/db.md db-024; specs/gap_retention-v2-engine.md 008

Offline compact via compacting.sqlite and storage-maintenance.json; in-place freelist reclaim by punching sparse holes under exclusive ownership after a FULL checkpoint; hybrid backup = VACUUM INTO + payload/bulk copies + storage.json + manifest.json. Retention on hybrid strips sealed detail by coordinated staged payload replacement with retry on generation change.

Owner paths:
- `crates/screenpipe-db/src/storage/maintenance.rs (new)`
- `crates/screenpipe-db/src/storage/reclaim.rs (new)`
- `crates/screenpipe-db/src/storage/backup.rs (new)`
- `crates/screenpipe-db/src/storage/sealing.rs (retention routine)`
- `crates/screenpipe-engine/src/retention.rs (hybrid dispatch line)`

Acceptance:
- [ ] cargo test: compact shrinks a fixture hybrid DB; backup bundle contains manifest.json, storage.json and payload files
- [ ] Reclaim on NTFS leaves sparse ranges (fsutil sparse queryrange); lean retention on hybrid restages payloads (test)

#### W7-03 SOP video project workspace and gate removal (L)

- **Items:** gap_workflow-sop-video-and-media-video-project-workspace, gap_workflow-sop-video-and-media-entitlement-and-telemetry
- **Depends on:** W6-07, W6-09
- **Specs:** specs/gap_workflow-sop-video-and-media.md video-project-workspace, entitlement-and-telemetry

Build video drafts from SOP sections (<= 50 sections, FNV-1a source hash invalidation), agent video project dirs under pi-workflows-guide/video-projects with caps and pruning, Pi session for edits, copy rendered output into workflow-videos. Narration depends only on the configured provider; no sign-in/Business checks, web SOP links or analytics.

Owner paths:
- `apps/screenpipe-app-tauri/components/workflows/video/** (new)`
- `apps/screenpipe-app-tauri/lib/workflows/video-draft.ts (new)`

Acceptance:
- [ ] bun test: draft builder respects caps and the source hash changes when the SOP changes
- [ ] UI smoke: generate a video from a workflow with the captions-only provider

#### W7-04 Native Win32 shortcut overlay pill (L)

- **Items:** shell-010
- **Depends on:** W6-19
- **Specs:** specs/shell.md shell-010

Layered Direct2D/DirectWrite pill fed by /ws/metrics and /ws/meeting-overlay with overlay/chat/star/search chips; falls back to the webview overlay on failure.

Owner paths:
- `crates/screenpipe-overlay-win/ (new crate)`
- `apps/screenpipe-app-tauri/src-tauri/src/native_shortcut_reminder.rs (Windows branch)`

Acceptance:
- [ ] UI smoke (Windows): the native pill renders and its chips open chat/search/star
- [ ] Forcing D2D init failure (env) falls back to the webview overlay

#### W7-05 Chat composer upgrades (M)

- **Items:** frontend-011
- **Depends on:** W6-01
- **Specs:** specs/frontend.md frontend-011

Slash commands (/new, /stop, /inspector, /pipes, installed skills), extended ~ time tokens, filters popover (time, content, apps, tags, speakers, connections), dictation through local or BYO STT, large paste saved as a chunk file.

Owner paths:
- `apps/screenpipe-app-tauri/components/standalone-chat.tsx (composer region)`
- `apps/screenpipe-app-tauri/components/chat/composer/** (new)`
- `apps/screenpipe-app-tauri/lib/chat-utils.ts`

Acceptance:
- [ ] bun test: ~lastweek and ~(date) tokens expand; slash registry lists installed skills
- [ ] UI smoke: dictation transcribes with the local engine (no vendor /listen request)

#### W7-06 First-run summary (L)

- **Items:** meetings_memory-041, frontend-018
- **Depends on:** W5-12, W6-01
- **Specs:** specs/meetings_memory.md meetings_memory-041; specs/frontend.md frontend-018

A few minutes after setup, generate a short summary from /activity-summary with the user's preset (own prompt), notify with the first-run-summary deep link, update the OnboardingStore firstRunSummary* fields and show the learning banner with an ask-agent hand-off. Other producers (db recovery, disk pressure, workflow review) already route through /notify.

Owner paths:
- `apps/screenpipe-app-tauri/src-tauri/src/first_run_summary.rs (new)`
- `apps/screenpipe-app-tauri/components/chat/first-run-summary-banner.tsx (new)`
- `apps/screenpipe-app-tauri/components/standalone-chat.tsx (banner mount line)`
- `apps/screenpipe-app-tauri/lib/hooks/use-onboarding.tsx`

Acceptance:
- [ ] UI smoke on a fresh install with a mock preset: the summary notification arrives and opens the summary chat
- [ ] store.bin onboarding fields firstRunSummaryPhase/ChatId are set; no telemetry is emitted

#### W7-07 Agent worktrees and per-chat coding workspace (L)

- **Items:** agents_pipes-020, frontend-017
- **Depends on:** W6-01, W6-04
- **Specs:** specs/agents_pipes.md agents_pipes-020; specs/frontend.md frontend-017

Managed git worktrees for coding chats (start_worktree tool, coding_workspace_prepare/create/get) from HEAD with an agent-worktree-owner.json record, hooks and fsmonitor disabled, safe branch claiming, never auto-removed; header badge with the path.

Owner paths:
- `crates/screenpipe-core/src/agents/worktree.rs (new)`
- `apps/screenpipe-app-tauri/src-tauri/src/coding_workspace.rs (new)`
- `apps/screenpipe-app-tauri/components/chat/coding-workspace-badge.tsx (new)`
- `apps/screenpipe-app-tauri/components/standalone-chat.tsx (badge mount line)`

Acceptance:
- [ ] Test in a temp git repo: coding_workspace_create makes a worktree with the owner record and core.hooksPath disabled
- [ ] UI smoke: the chat header shows the workspace badge

#### W7-08 Built-in pipe catalogue and builtin text migrations (L)

- **Items:** agents_pipes-012
- **Depends on:** W6-03, W4-01, W3-06
- **Specs:** specs/agents_pipes.md agents_pipes-012

Ship the 21 built-ins (workflow discovery/enrichment agents, skill-learning, automate-my-work, missed-todos, speaker reconciliation, commitments, accounting follow-through, meeting summary, ...) with the observed frontmatter and freshly written prompts; drop ai-prompt-journal; idempotent builtin text migrations for installed copies; presets resolve to the user's presets.

Owner paths:
- `crates/screenpipe-core/assets/pipes/** (new built-ins, own prose)`
- `crates/screenpipe-core/src/pipes/builtin_migrations.rs (new)`
- `crates/screenpipe-core/src/pipes/mod.rs (install_builtin_pipes region)`

Acceptance:
- [ ] Fresh data dir: `api /pipes` lists 21 built-ins and no ai-prompt-journal
- [ ] cargo test: running builtin migrations twice leaves installed copies unchanged after the first pass

#### WR-04 Composio integrations (BYO key) and workflow sharing (L)

- **Items:** (restore) cloud-038, (restore) cloud-035
- **Depends on:** W2-20, W6-07, WR-01
- **Specs:** specs/cloud.md cloud-035, cloud-038

Composio: restore the Composio-brokered integrations using the user's own Composio API key (stored in the secret store), calling Composio's public API directly; list available toolkits, connect accounts via Composio's hosted auth links, and expose connected tools to the agent like other connections. Workflow sharing: export a workflow (definition, steps, optional SOP video, redacted by default) as a signed .fpworkflow bundle, import it with a preview and permission review, and browse/publish to a gallery index under registry/workflows/ on normiecore/Freenpipe (publish = generated PR folder, like WR-01). Training-data contribution remains removed.

Owner paths:
- `crates/screenpipe-connect/src/connections/composio.rs (new)`
- `apps/screenpipe-app-tauri/components/settings/connections-section.tsx (Composio tile)`
- `crates/screenpipe-core/src/workflows/share.rs (new)`
- `apps/screenpipe-app-tauri/components/workflows/share-dialog.tsx (new)`
- `registry/workflows/ (new gallery index)`

Acceptance:
- [ ] cargo test: Composio client against a wiremock server (toolkit list, connect link, tool call) with key from the secret store
- [ ] Round-trip: export a workflow, import on a fresh data dir, it runs; the bundle contains no unredacted PII by default
- [ ] UI smoke: gallery lists entries from the registry index; import shows the permission review

### Wave 8: Final UI pieces and surface audit

| WP | Size | Title | Depends on | Items |
|---|---|---|---|---|
| W8-01 | XL | Rich chat blocks and local feedback UI | W7-05, W2-03 | frontend-012, frontend-014, meetings_memory-008 |
| W8-02 | L | Storage and retention settings UI with migration overlays | W7-01, W7-02, W3-09, W4-10 | frontend-039, frontend-044 |
| W8-03 | L | Tauri command/event surface audit, ACL and release readiness | W8-01, W8-02, W7-01, W7-02, W7-03, W7-04, W7-05, W7-06, W7-07, W7-08 | frontend-056 |

#### W8-01 Rich chat blocks and local feedback UI (XL)

- **Items:** frontend-012, frontend-014, meetings_memory-008
- **Depends on:** W7-05, W2-03
- **Specs:** specs/frontend.md frontend-012, 014; specs/meetings_memory.md meetings_memory-008

Typed blocks: 13 chart kinds, plans, structured output, rich results, tool activity, ask-user, agent-action (ACP permission), context usage, turn status, sandboxed HTML, citation and selected-text actions. Thumbs up/down with optional comment on chat replies, pipe notifications and Live View cards posting to /feedback.

Owner paths:
- `apps/screenpipe-app-tauri/components/chat/blocks/** (new)`
- `apps/screenpipe-app-tauri/components/markdown.tsx`
- `apps/screenpipe-app-tauri/lib/chat/tool-presentation.ts`
- `apps/screenpipe-app-tauri/components/standalone-chat.tsx (block renderer region)`
- `apps/screenpipe-app-tauri/components/notification-bell.tsx (rating controls)`
- `apps/screenpipe-app-tauri/app/notification-panel/page.tsx (rating controls)`

Acceptance:
- [ ] bun test renders each chart kind from fixture JSON and the ask-user card round-trip
- [ ] UI smoke: thumbs-down with a comment on a reply creates a feedback row with target.kind chat (GET /feedback)

#### W8-02 Storage and retention settings UI with migration overlays (L)

- **Items:** frontend-039, frontend-044
- **Depends on:** W7-01, W7-02, W3-09, W4-10
- **Specs:** specs/frontend.md frontend-039, 044

Retention modes off/media/lean/all, erase recent, Compact DB (gated on 2x DB + 512 MiB free, calls /data/compact), low-disk guard toggle, storage migration card (start/cancel/finish/delete original) and the root-layout prompt and non-dismissable progress modal driven by storage-migration-activity, plus database-recovery deep links.

Owner paths:
- `apps/screenpipe-app-tauri/components/settings/storage-section.tsx`
- `apps/screenpipe-app-tauri/components/settings/retention-settings.tsx`
- `apps/screenpipe-app-tauri/components/settings/storage-migration-card.tsx (new)`
- `apps/screenpipe-app-tauri/components/storage-migration-overlay.tsx (new)`
- `apps/screenpipe-app-tauri/app/layout.tsx (overlay mount line)`

Acceptance:
- [ ] UI smoke: switching retention to lean configures the engine (/retention/status shows lean); Compact DB reports reclaimed bytes
- [ ] Starting a migration on a fixture data dir shows the progress modal until completion

#### W8-03 Tauri command/event surface audit, ACL and release readiness (L)

- **Items:** frontend-056
- **Depends on:** W8-01, W8-02, W7-01, W7-02, W7-03, W7-04, W7-05, W7-06, W7-07, W7-08
- **Specs:** specs/frontend.md frontend-056; specs/frontend.md frontend-054 (merged)

Audit the 112 target-new commands, 7 argument extensions and new events against the bindings: each is implemented or explicitly recorded as dropped (vendor-only); add ACL entries and the typed channels job:event, export:event, engine:event, notification:action; regenerate bindings; run the full test matrix and build the installer.

Owner paths:
- `apps/screenpipe-app-tauri/lib/utils/tauri.ts (generated)`
- `apps/screenpipe-app-tauri/lib/events/types.ts`
- `apps/screenpipe-app-tauri/src-tauri/capabilities/*.json (ACL)`
- `docs/parity/command-audit.md (new)`
- `scripts/ci/command-audit.ts (new)`

Acceptance:
- [ ] bun scripts/ci/command-audit.ts reports 0 unaccounted commands against specs/frontend.md
- [ ] bun run bindings:check, cargo test --workspace (release-dev profile) and bun test all pass; vendor guard passes
- [ ] Release-dev installer builds and a clean install records screen and audio for 10 minutes with /health healthy

## 9. Verifier-dropped items and where their residue lands

| Item | Verdict | Handled by |
|---|---|---|
| api-024 | already_in_base | cloud-016 in W0-06 (BYO gateway or 503) |
| api-036 | unsupported | no work: /openapi.* stays (base-present); Gmail retention handled with api-031 in W2-20 |
| api-040 | already_in_base | cloud-020 in W0-06 and cloud-021 in W0-08 |
| db-028 | duplicate | paths owned by their WPs; secrets.sqlite counted in DB size by db-026 (W4-10) |
| vision-ocr-unify-frames | duplicate | db-005 and db-006 in W1-01 |
| vision-hybrid-payload-storage | duplicate | db-021 (W5-07) and db-022 (W6-06); vision must read/write through them |
| vision-tinfoil-decloud | duplicate | cloud-020 in W0-06 |
| vision-telemetry-decloud | duplicate | cloud-010 in W0-02; wedge restart stays a local log in W3-11 |
| vision-history-access-stub | duplicate | api-035 in W1-04 |
| audio-015 | duplicate | session snapshot in meetings_memory-030 (W4-12); per-pid input resolution in audio-016 (W6-05) |
| audio-025 | duplicate | api-039 in W3-06 |
| audio-029 | duplicate | cloud-033 in W1-02 (remote-control flags ignored, shipped defaults kept) |
| audio-030 | duplicate | cloud-017 in W0-06 |
| audio-031 | duplicate | cloud-017 in W0-06 |
| audio-032 | duplicate | cloud-024 in W0-09 |
| audio-033 | duplicate | cloud-010 in W0-02 |
| meetings_memory-043 | already_in_base | no work; screenpipe-team-memory excluded from default members by W0-05 |
| agents_pipes-034 | already_in_base | cloud-016 (W0-06) and agents_pipes-033 (W3-25) |
| connections-secret-db-split | duplicate | db-026 in W4-10 (construction-site list, pool/quarantine notes merged) |
| connections-credential-refresh-leases | duplicate | db-026 in W4-10 (ChatGPT refresh caller merged) |
| connections-oauth-byo-broker | duplicate | cloud-018 in W0-07 (secret key name fixed as oauth_client:<id>) |
| connections-gmail-sheets-composio | duplicate | cloud-038 in W2-20 |
| connections-pi-connect-tools | duplicate | agents_pipes-043 (W3-26) and agents_pipes-014 (W3-23) |
| connections-obsidian-memories | duplicate | meetings_memory-035 in W3-22 |
| connections-agent-memory-guidance | duplicate | meetings_memory-035 in W3-22 (Codex switch + restore-on-failed-cleanup merged) |
| connections-owned-browser-tabs | duplicate | frontend-013 in W6-18 |
| connections-connection-triggers | duplicate | agents_pipes-008 in W6-03 (file is .connection-triggers.json) |
| connections-vault-secrets-db | duplicate | meetings_memory-044 in W4-10 |
| frontend-054 | unsupported | command removals recorded by frontend-056 (W8-03); screenpipe-ui-theme and console_logs keys stay |
| shell-015 | duplicate | db-023 in W7-01 (activity event, error file, pendingStorageMigration merged) |
| shell-016 | duplicate | cloud-012 and cloud-041 in W0-03 |
| shell-027 | duplicate | frontend-045 in W5-21 |
| shell-030 | duplicate | audio-020 in W2-19 |
| shell-031 | duplicate | cloud-010 and cloud-011 in W0-02 |
| shell-032 | duplicate | cloud-034 in W1-02 |
| shell-033 | duplicate | cloud-036 in W0-10 |
| shell-034 | duplicate | cloud-014 and cloud-030 in W0-04 |
| shell-035 | duplicate | cloud-001/002 (W0-01) and cloud-043 (W1-04) |

## 10. Item to WP index

| Item | WP |
|---|---|
| api-001 | W2-09 |
| api-002 | W2-09 |
| api-003 | W3-02 |
| api-004 | W3-03 |
| api-005 | W3-04 |
| api-006 | W3-04 |
| api-007 | W2-04 |
| api-008 | W3-01 |
| api-009 | W2-09 |
| api-010 | W2-10 |
| api-011 | W2-10 |
| api-012 | W2-24 |
| api-013 | W2-16 |
| api-014 | W2-08 |
| api-015 | W3-07 |
| api-016 | W2-11 |
| api-017 | W2-02 |
| api-018 | W2-03 |
| api-019 | W3-05 |
| api-020 | W4-11 |
| api-021 | W3-24 |
| api-022 | W3-26 |
| api-023 | W3-25 |
| api-025 | W3-25 |
| api-026 | W2-24 |
| api-027 | W4-04 |
| api-028 | W4-01 |
| api-029 | W5-01 |
| api-030 | W4-10 |
| api-031 | W2-20 |
| api-032 | W3-21 |
| api-033 | W2-13 |
| api-034 | W3-28 |
| api-035 | W1-04 |
| api-037 | W3-18 |
| api-038 | W4-08 |
| api-039 | W3-06 |
| api-M01 | W3-21 |
| api-M02 | W3-21 |
| api-M03 | W2-20 |
| db-001 | W1-01 |
| db-002 | W2-02 |
| db-003 | W3-10 |
| db-004 | W3-10 |
| db-005 | W1-01 |
| db-006 | W1-01 |
| db-007 | W2-11 |
| db-008 | W2-05 |
| db-009 | W3-06 |
| db-010 | W2-03 |
| db-011 | W2-16 |
| db-012 | W2-08 |
| db-013 | W2-07 |
| db-014 | W2-17 |
| db-015 | W2-04 |
| db-016 | W2-01 |
| db-017 | W2-01 |
| db-018 | W2-01 |
| db-019 | W3-27 |
| db-020 | W4-09 |
| db-021 | W5-07 |
| db-022 | W6-06 |
| db-023 | W7-01 |
| db-024 | W7-02 |
| db-025 | W4-10 |
| db-026 | W4-10 |
| db-027 | W1-01 |
| db-029 | W0-11 |
| db-v2-01 | W2-01 |
| vision-device-control | W2-11 |
| vision-display-layout | W2-11 |
| vision-capture-wedge-watchdog | W3-11 |
| vision-probe-timeouts | W3-11 |
| vision-capture-loop-refinements | W5-05 |
| vision-wgc-native-capture | W2-12 |
| vision-corrupt-frame-guard | W3-11 |
| vision-pipeline-metrics | W2-13 |
| vision-ocr-gate | W5-06 |
| vision-windows-ocr-worker | W3-12 |
| vision-tree-walker-worker | W4-14 |
| vision-uia-retained-capture | W4-13 |
| vision-windows-window-resolution | W3-13 |
| vision-url-domain-rules | W3-13 |
| vision-disable-screenshots | W5-05 |
| vision-capture-max-width | W5-05 |
| vision-recording-detail | W5-05 |
| vision-thumbnail-endpoint | W2-10 |
| vision-preview-samples | W2-10 |
| vision-preview-media | W2-10 |
| vision-elements-formats | W3-03 |
| vision-compaction-encoder | W2-15 |
| vision-redaction-surfaces | W3-10 |
| vision-redaction-pseudonyms-a11y | W4-15 |
| vision-redaction-models | W4-15 |
| vision-starred-hd | W2-04 |
| vision-frame-comparison-hardening | W2-12 |
| vision-capture-identity-gate | W4-14 |
| vision-exclusion-unknown-fail-closed | W3-11 |
| vision-frame-media-errors | W2-10 |
| a11y-input-hook-watchdog | W3-13 |
| audio-001 | W2-17 |
| audio-002 | W2-16 |
| audio-003 | W2-16 |
| audio-004 | W2-16 |
| audio-005 | W2-16 |
| audio-006 | W2-16 |
| audio-007 | W3-16 |
| audio-008 | W2-17 |
| audio-009 | W2-16 |
| audio-010 | W3-14 |
| audio-011 | W4-16 |
| audio-012 | W3-15 |
| audio-013 | W2-18 |
| audio-014 | W2-18 |
| audio-016 | W6-05 |
| audio-017 | W4-16 |
| audio-018 | W3-14 |
| audio-019 | W2-19 |
| audio-020 | W2-19 |
| audio-021 | W3-17 |
| audio-022 | W2-19 |
| audio-023 | W3-16 |
| audio-024 | W3-16 |
| audio-026 | W3-14 |
| audio-027 | W2-13 |
| audio-028 | W1-02 |
| audio-M01 | W2-17 |
| audio-M02 | W5-04 |
| audio-M03 | W5-04 |
| audio-M04 | W3-16 |
| audio-M05 | W2-17 |
| audio-M06 | W3-17 |
| meetings_memory-001 | W1-01 |
| meetings_memory-002 | W2-02 |
| meetings_memory-003 | W2-02 |
| meetings_memory-004 | W2-02 |
| meetings_memory-005 | W3-05 |
| meetings_memory-006 | W5-16 |
| meetings_memory-007 | W2-03 |
| meetings_memory-008 | W8-01 |
| meetings_memory-009 | W0-10 |
| meetings_memory-010 | W2-04 |
| meetings_memory-011 | W3-02 |
| meetings_memory-012 | W5-14 |
| meetings_memory-013 | W2-07 |
| meetings_memory-014 | W2-07 |
| meetings_memory-015 | W3-04 |
| meetings_memory-016 | W5-12 |
| meetings_memory-017 | W6-13 |
| meetings_memory-018 | W3-04 |
| meetings_memory-019 | W2-05 |
| meetings_memory-020 | W2-05 |
| meetings_memory-021 | W2-06 |
| meetings_memory-022 | W3-01 |
| meetings_memory-023 | W3-01 |
| meetings_memory-024 | W3-02 |
| meetings_memory-025 | W2-08 |
| meetings_memory-026 | W2-08 |
| meetings_memory-027 | W3-06 |
| meetings_memory-028 | W3-06 |
| meetings_memory-029 | W5-03 |
| meetings_memory-030 | W4-12 |
| meetings_memory-031 | W3-08 |
| meetings_memory-032 | W2-08 |
| meetings_memory-033 | W3-07 |
| meetings_memory-034 | W5-13 |
| meetings_memory-035 | W3-22 |
| meetings_memory-037 | W3-18 |
| meetings_memory-038 | W3-18 |
| meetings_memory-039 | W5-13 |
| meetings_memory-040 | W5-13 |
| meetings_memory-041 | W7-06 |
| meetings_memory-042 | W1-04 |
| meetings_memory-044 | W4-10 |
| meetings_memory-045 | W4-08 |
| meetings_memory-046 | W4-17 |
| meetings_memory-047 | W2-08 |
| agents_pipes-001 | W3-06 |
| agents_pipes-002 | W3-06 |
| agents_pipes-003 | W2-22 |
| agents_pipes-004 | W2-22 |
| agents_pipes-005 | W5-01 |
| agents_pipes-006 | W5-01 |
| agents_pipes-007 | W4-02 |
| agents_pipes-008 | W6-03 |
| agents_pipes-009 | W6-02 |
| agents_pipes-010 | W2-02 |
| agents_pipes-011 | W3-05 |
| agents_pipes-012 | W7-08 |
| agents_pipes-013 | W4-08 |
| agents_pipes-014 | W3-23 |
| agents_pipes-015 | W2-23 |
| agents_pipes-016 | W4-03 |
| agents_pipes-017 | W6-01 |
| agents_pipes-018 | W4-05 |
| agents_pipes-019 | W6-04 |
| agents_pipes-020 | W7-07 |
| agents_pipes-021 | W3-24 |
| agents_pipes-022 | W3-24 |
| agents_pipes-023 | W3-24 |
| agents_pipes-024 | W4-06 |
| agents_pipes-025 | W5-02 |
| agents_pipes-026 | W4-01 |
| agents_pipes-027 | W4-01 |
| agents_pipes-028 | W4-04 |
| agents_pipes-029 | W5-02 |
| agents_pipes-030 | W6-11 |
| agents_pipes-031 | W4-07 |
| agents_pipes-032 | W3-21 |
| agents_pipes-033 | W3-25 |
| agents_pipes-035 | W3-25 |
| agents_pipes-036 | W0-06 |
| agents_pipes-037 | W0-08 |
| agents_pipes-038 | W2-24 |
| agents_pipes-039 | W4-03 |
| agents_pipes-040 | W0-09 |
| agents_pipes-041 | W1-04 |
| agents_pipes-042 | W4-02 |
| agents_pipes-043 | W3-26 |
| agents_pipes-044 | W2-03 |
| agents_pipes-045 | W1-04 |
| agents_pipes-046 | W6-10 |
| agents_pipes-m01 | W3-06 |
| agents_pipes-m02 | W6-02 |
| agents_pipes-m03 | W2-22 |
| agents_pipes-m04 | W4-08 |
| agents_pipes-m05 | W2-22 |
| agents_pipes-m06 | W4-02 |
| connections-oauth-connect-v2 | W2-21 |
| connections-oauth-default-slot-promotion | W2-21 |
| connections-secret-redaction | W2-20 |
| connections-webhook-proxy | W2-20 |
| connections-telegram-send | W2-20 |
| connections-slack-user-token | W3-19 |
| connections-imap | W3-20 |
| connections-outlook-email | W2-21 |
| connections-hermes | W3-22 |
| connections-hubspot-oauth | W2-21 |
| connections-zendesk-oauth | W2-21 |
| connections-oauth-scope-tweaks | W2-21 |
| connections-mcp-oauth-tiles | W3-21 |
| connections-bee-pairing | W4-18 |
| connections-browser-snapshot-v2-act | W4-18 |
| connections-google-calendar-multi | W4-17 |
| connections-whatsapp-signout | W3-22 |
| connections-supports-oauth-instances | W2-20 |
| connections-ui-catalog | W5-23 |
| connections-chat-context-per-turn | W4-05 |
| connections-pipe-mcp-connection-refs | W2-20 |
| frontend-001 | W2-25 |
| frontend-002 | W4-22 |
| frontend-003 | W4-20 |
| frontend-004 | W2-25 |
| frontend-005 | W4-19 |
| frontend-006 | W4-22 |
| frontend-007 | W6-14 |
| frontend-008 | W4-22 |
| frontend-009 | W5-15 |
| frontend-010 | W6-04 |
| frontend-011 | W7-05 |
| frontend-012 | W8-01 |
| frontend-013 | W6-18 |
| frontend-014 | W8-01 |
| frontend-015 | W6-01 |
| frontend-016 | W3-23 |
| frontend-017 | W7-07 |
| frontend-018 | W7-06 |
| frontend-019 | W6-14 |
| frontend-020 | W5-15 |
| frontend-021 | W5-16 |
| frontend-022 | W6-12 |
| frontend-023 | W6-13 |
| frontend-024 | W5-17 |
| frontend-025 | W5-18 |
| frontend-026 | W5-18 |
| frontend-027 | W5-18 |
| frontend-028 | W5-22 |
| frontend-029 | W5-14 |
| frontend-030 | W5-13 |
| frontend-031 | W5-13 |
| frontend-032 | W6-19 |
| frontend-033 | W5-22 |
| frontend-034 | W6-16 |
| frontend-035 | W5-19 |
| frontend-036 | W5-20 |
| frontend-037 | W5-20 |
| frontend-038 | W6-15 |
| frontend-039 | W8-02 |
| frontend-040 | W5-13 |
| frontend-041 | W5-19 |
| frontend-042 | W5-19 |
| frontend-043 | W5-19 |
| frontend-044 | W8-02 |
| frontend-045 | W5-21 |
| frontend-046 | W0-10 |
| frontend-047 | W6-07 |
| frontend-048 | W5-02 |
| frontend-049 | W6-17 |
| frontend-050 | W5-23 |
| frontend-051 | W0-05 |
| frontend-052 | W1-04 |
| frontend-053 | W1-02 |
| frontend-055 | W5-22 |
| frontend-056 | W8-03 |
| frontend-M01 | W5-21 |
| shell-001 | W1-02 |
| shell-002 | W1-02 |
| shell-003 | W1-02 |
| shell-004 | W3-28 |
| shell-005 | W3-28 |
| shell-006 | W3-30 |
| shell-007 | W3-30 |
| shell-008 | W4-19 |
| shell-009 | W5-14 |
| shell-010 | W7-04 |
| shell-011 | W3-29 |
| shell-012 | W2-26 |
| shell-013 | W2-26 |
| shell-014 | W5-08 |
| shell-017 | W2-27 |
| shell-018 | W4-19 |
| shell-019 | W2-27 |
| shell-020 | W2-27 |
| shell-021 | W2-27 |
| shell-022 | W2-27 |
| shell-023 | W5-09 |
| shell-024 | W2-27 |
| shell-025 | W1-03 |
| shell-026 | W4-07 |
| shell-028 | W2-27 |
| shell-029 | W2-19 |
| shell-036 | W4-20 |
| shell-037 | W1-02 |
| shell-038 | W1-03 |
| shell-039 | W4-19 |
| shell-M01 | W3-17 |
| shell-M02 | W1-02 |
| cloud-001 | W0-01 |
| cloud-002 | W0-01 |
| cloud-003 | W0-01 |
| cloud-004 | W0-01 |
| cloud-005 | W0-01 |
| cloud-009 | W0-01 |
| cloud-010 | W0-02 |
| cloud-011 | W0-02 |
| cloud-012 | W0-03 |
| cloud-013 | W0-04 |
| cloud-014 | W0-04 |
| cloud-015 | W0-04 |
| cloud-016 | W0-06 |
| cloud-017 | W0-06 |
| cloud-018 | W0-07 |
| cloud-019 | W0-08 |
| cloud-020 | W0-06 |
| cloud-021 | W0-08 |
| cloud-022 | W0-05 |
| cloud-023 | W0-05 |
| cloud-024 | W0-09 |
| cloud-030 | W0-04 |
| cloud-031 | W0-04 |
| cloud-032 | W0-04 |
| cloud-033 | W1-02 |
| cloud-034 | W1-02 |
| cloud-035 | W1-02 |
| cloud-036 | W0-10 |
| cloud-037 | W2-24 |
| cloud-038 | W2-20 |
| cloud-039 | W0-05 |
| cloud-040 | W2-24 |
| cloud-041 | W0-03 |
| cloud-042 | W0-06 |
| cloud-043 | W1-04 |
| cloud-044 | W0-05 |
| cloud-046 | W0-09 |
| cloud-V01 | W5-21 |
| cloud-V02 | W0-03 |
| cloud-M01 | W0-06 |
| cloud-M02 | W0-09 |
| gap_retention-v2-engine-001 | W3-09 |
| gap_retention-v2-engine-002 | W3-09 |
| gap_retention-v2-engine-003 | W3-09 |
| gap_retention-v2-engine-004 | W3-09 |
| gap_retention-v2-engine-005 | W3-09 |
| gap_retention-v2-engine-006 | W3-09 |
| gap_retention-v2-engine-007 | W3-09 |
| gap_retention-v2-engine-008 | W7-02 |
| gap_retention-v2-engine-009 | W3-09 |
| gap_retention-v2-engine-010 | W3-09 |
| gap_retention-v2-engine-011 | W3-09 |
| gap_workflow-sop-video-and-media-core-renderer | W4-21 |
| gap_workflow-sop-video-and-media-narration-decloud | W3-31 |
| gap_workflow-sop-video-and-media-app-video-jobs | W5-10 |
| gap_workflow-sop-video-and-media-render-cli | W6-09 |
| gap_workflow-sop-video-and-media-pi-video-sop-extension | W6-09 |
| gap_workflow-sop-video-and-media-video-project-workspace | W7-03 |
| gap_workflow-sop-video-and-media-recording-replay | W5-11 |
| gap_workflow-sop-video-and-media-captured-moment-open | W5-11 |
| gap_workflow-sop-video-and-media-workflow-ready-notification | W6-08 |
| gap_workflow-sop-video-and-media-entitlement-and-telemetry | W7-03 |
| gap_health-lifecycle-internals-001 | W2-13 |
| gap_health-lifecycle-internals-002 | W2-13 |
| gap_health-lifecycle-internals-003 | W2-13 |
| gap_health-lifecycle-internals-004 | W2-14 |
| gap_health-lifecycle-internals-005 | W2-14 |
| gap_health-lifecycle-internals-006 | W2-14 |
| gap_health-lifecycle-internals-007 | W2-14 |
| gap_health-lifecycle-internals-008 | W2-14 |
| gap_health-lifecycle-internals-009 | W2-14 |
| gap_health-lifecycle-internals-010 | W0-02 |
| gap_health-lifecycle-internals-011 | W2-15 |
| gap_health-lifecycle-internals-012 | W2-15 |
| gap_health-lifecycle-internals-013 | W3-08 |

## 11. Risks

**Build and toolchain**
- The base only builds inside the vcvars environment, and the debug profile does not link (CRT mismatch between knf-rs-sys and whisper-rs-sys). Until W0-01 fixes it, every acceptance check uses `--profile release-dev`, and each release-dev build takes about 7 minutes.
- The Tauri app has never been built on this machine. Expect sidecar, NSIS and WebView2 surprises in W0-01.
- W0-11 (sqlx 0.7.2 to 0.9, libsqlite3-sys 0.26 to 0.37) is a breaking API migration across db, engine, secrets and app. It is the prerequisite for the 3.51.3 WAL-reset guard, `sqlite3_recover`, FTS5 `contentless_delete` and the rusqlite vtab. sqlx's out-of-order and ignore-missing behaviour must be re-verified, because target migration 20260601000000 sorts before the base's last two.
- W2-19 needs the Vulkan SDK (glslc) to compile ggml-vulkan, plus DirectML / ORT rc.12. The target's audiopipe revision (b099a2e) postdates the relicense, so implement the DML recycle and CPU retry ourselves unless its licence is cleared.
- W2-25 moves to React 19 and Next 15.5 underneath about 200 components. Expect breakage in Radix, tiptap and the test setup. Pin stable releases at least two weeks old, not the target's canary.
- Several third-party crates carry unknown or vendor-adjacent licences: the AEC3 port (sonora), the forks under github.com/screenpipe/* (whisper-rs, rusty-tesseract) and divanshu-go/* (audiopipe, tauri-helper). W0-01 must confirm that pinned revisions predate 2026-06-10, and mirror them.

**Coexistence and data safety (owner machine)**
- The installed screenpipe v2.7.104 shares port 3030, the :11435 control server, `~/.screenpipe`, `~/.screenpipe/auth.json` and the `screenpipe://` scheme. The base app's blind port-kill would terminate it, and base auth code can delete its auth.json. W0-01 and the section-2 rules must land before any app run.
- W1-01 runs an irreversible migration that drops ocr_text. Opening a real screenpipe DB with a Freenpipe build is one-way:
  - A v2.7.104 DB is already migrated, so versions match.
  - A W0-state build (89 migrations) opening a v2.7 DB fails at runtime, because ocr_text is gone.
  Neither build may touch real data during development.
- Freenpipe's migration files differ in text from the target's: upstream header comments are inside the checksummed bodies. Interop relies on the checksum-rewrite path on both sides. If a DB alternates between the two apps, each one rewrites checksums on open.

**Scale and overlap**
- Huge single items: the semantic parser crate (W2-06), workflows core (W4-01), the ACP runtime (W4-03), the audio-process detector (W4-12), the retained UIA tree (W4-13), the SOP renderer (W4-21), hybrid storage (W5-07, W6-06, W7-01, W7-02), meeting piggyback (W6-05), and the Live Views, Workflows and chat-blocks UIs. Each is XL and may need internal phasing within one agent's session.
- Hotspot files (section 6): `db.rs`, `standalone-chat.tsx`, `pipes/mod.rs`, `meeting_detector.rs`, app `pi.rs`, `connections_api.rs` and `event_driven_capture.rs`. Region discipline and the W1-03 seams are what keep the waves mergeable.

**Testing gaps**
- Windows-only behaviour needs real hardware and manual smoke tests: WASAPI process loopback (build 20348 or later), native WGC, UIA retained trees, the LL-hook watchdog, WER, PowerRegisterSuspendResumeNotification, SAPI, DirectML and Vulkan. CI cannot cover these, so manual checks are listed explicitly.

**Spec gaps (inferred behaviour)**
- Wedge-watchdog thresholds, Bluetooth-gate and identification-gate numbers, the text-region detector algorithm, compaction thresholds, and the app reaction to a stale renderer heartbeat are all inferred.
- The search-only gated route set (api-034) is unresolved.
- The retention ui_events strip is inferred-strong.
- The hybrid payload byte layout is only partially observed.
Implementers must choose sensible values and document them in code comments.

**Clean-room hazard**
- Several WPs are prompt- and prose-heavy: 21 built-in pipes, 20 starter skills, meeting summary, first-run summary, activity history, workflow agents and the video-sop skill. Reviewers must check that all of this text is newly written.

**Scope creep from vendor features**
- Do not port vendor-only features, even as local stubs, unless the parity frontend calls them: Composio, data-sync upload, team, trials, quotas, remote controls, announcements and workflow sharing.

## 12. Open questions for the owner

1. **Default data dir and coexistence.** Option A: keep `~/.screenpipe` and port 3030/11435 for drop-in interop with existing data, pipes and MCP clients, at the cost of never running beside screenpipe. Option B: default to a Freenpipe dir and port, with an "import from screenpipe" path. Which one?
2. **Deep-link scheme.** Register only `freenpipe://`, or also `screenpipe://` (and `screenpipe-enterprise:`) as an opt-in alias for external links (frontend-M01)? An alias would collide with an installed screenpipe.
3. **CLI binary name.** Keep `screenpipe` (pipes, skills and MCP configs invoke it), rename to `freenpipe`, or ship both?
4. **Hybrid Parquet storage** (db-021..024, retention-008, parts of frontend-039/044). It is opt-in in the target, and it is five XL/L WPs with real data-loss risk. Is it required for parity, or should it be deferred and the waves after W5 trimmed?
5. **SQLite toolchain jump** (W0-11). Is the move to sqlx 0.9 / libsqlite3-sys 0.37 acceptable? The alternative is to stay on 0.7 and give up the 3.51.3 guard, embedded recovery and the vtab.
6. **Git dependencies under the vendor's orgs.** Should they be mirrored under normiecore after the date and licence check? May the target's newer audiopipe revision be used if its licence is clearly MIT or Apache?
7. **Updater and releases.** Who creates the minisign key and the GitHub release secrets for normiecore/Freenpipe? Is `autoUpdate` defaulting to true (as in the target) acceptable for Freenpipe?
8. **Localization.** Ship English only with the ja catalogue scaffolded, or commission a Japanese translation?
9. **Third-party, user-initiated network features.** Are these acceptable to keep: the MCP registry search (registry.modelcontextprotocol.io), ACP agent npm installs, Cursor Cloud (api.cursor.com), the GitHub skill install and the Bee pairing service? The Bee pairing app id would have to be supplied by the user.
10. **Default narration provider** for SOP videos. Captions-only (silent) unless the user configures TTS, or local Windows SAPI?
11. **React version.** Follow the target to React 19 now (W2-25), or stay on React 18 until the feature UIs land?
12. **Built-in pipe and skill content.** 21 pipes and 20 starter skills need fresh prose. Should all of them be written for parity, or a curated subset?
13. **CI.** Does normiecore/Freenpipe have Windows GitHub Actions runners for the build, test and installer pipeline that W0-03 produces?

## 13. Regenerating this plan

`python3 tools/plan/plan_build.py` validates the following and rewrites this file and `docs/clean-room/plan_wps.json`:
- every live item is in exactly one WP;
- every dropped item is mapped in section 9;
- dependencies point to known WPs in the same or an earlier wave.

`python3 tools/plan/plan_build.py --check` only validates and fails when either output is out of date. Work-package status (merged, in review, claimed, ready, blocked) is not part of this plan; `python3 tools/plan/status.py` derives it from git and GitHub and the merger publishes it as `docs/clean-room/STATUS.md`.
