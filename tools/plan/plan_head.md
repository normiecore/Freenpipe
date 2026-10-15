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
