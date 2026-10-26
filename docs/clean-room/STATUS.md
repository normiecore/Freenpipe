# Freenpipe work-package status

> Generated file: the merger routine regenerates it with `python3 tools/plan/status.py --write`. Do not edit it by hand. The plan is [PLAN.md](PLAN.md); the process is [docs/dev/cloud-runbook.md](../dev/cloud-runbook.md).

Generated 2026-10-26T22:37:30Z from `origin/main` (latest code commit `4274991`). GitHub data: available (via gh).

## Progress

**40 of 155 work packages merged (25.8%)**: 0 in review, 4 claimed, 31 ready, 80 blocked.

| Wave | Theme | Merged | In review | Claimed | Ready | Blocked | Progress |
|---|---|---|---|---|---|---|---|
| 0 | Foundation: build, ee/ stubs, de-cloud, rebrand | 13/13 | 0 | 0 | 0 | 0 | `##########` 100% |
| 1 | Cross-cutting foundations: schema v2.7, settings schema, integration seams | 5/5 | 0 | 0 | 0 | 0 | `##########` 100% |
| 2 | Engine data layer and core backends | 17/27 | 0 | 0 | 10 | 0 | `######----` 63% |
| 3 | Feature backends (search, pipes, connections, capture, audio) | 5/31 | 0 | 4 | 14 | 8 | `##--------` 16% |
| 4 | Advanced backends, agents runtime, app shell | 0/22 | 0 | 0 | 5 | 17 | `----------` 0% |
| 5 | Feature UIs and second-tier backends | 0/24 | 0 | 0 | 2 | 22 | `----------` 0% |
| 6 | Agent/workflow UIs, piggyback, hybrid bulk | 0/21 | 0 | 0 | 0 | 21 | `----------` 0% |
| 7 | Hybrid lifecycle, overlays, chat follow-ups | 0/9 | 0 | 0 | 0 | 9 | `----------` 0% |
| 8 | Final UI pieces and surface audit | 0/3 | 0 | 0 | 0 | 3 | `----------` 0% |
| all | | 40/155 | 0 | 4 | 31 | 80 | `###-------` 25.8% |

CI on main (merge gate on the latest non-status commit `4274991`): **pending** (test-windows: in_progress; trufflehog: in_progress; waiting for windows, linux, gates)

## In review

None.

## Claimed

| WP | Worker | Last commit (UTC) | Work commits | State |
|---|---|---|---|---|
| W3-02 | unknown | 2026-10-26T22:36:01Z | 5 | active |
| W3-06 | unknown | 2026-10-26T20:45:45Z | 4 | active |
| W3-08 | unknown | 2026-10-26T20:39:53Z | 3 | active |
| W3-13 | unknown | 2026-10-26T22:37:18Z | 6 | active |

## Ready, in plan order

| WP | Wave | Size | Title |
|---|---|---|---|
| W2-02 | 2 | L | Outputs and artifacts registry |
| W2-03 | 2 | M | Unified AI feedback table and /feedback |
| W2-06 | 2 | XL | screenpipe-semantic parser crate |
| W2-07 | 2 | L | Activity ledger store and deterministic reconciler |
| W2-14 | 2 | L | Resource monitor v2: process tree, breakdown, leak trend |
| W2-15 | 2 | L | Snapshot compaction v2 and suspend/lock awareness |
| W2-20 | 2 | L | Connections core: secret-safe reads, webhook proxy, Telegram send, router deltas, MCP refs |
| W2-21 | 2 | L | OAuth v2 flow and new OAuth integrations |
| W2-24 | 2 | M | Engine local stubs: attribution headers, telemetry no-op routes, local data-sync |
| W2-25 | 2 | L | Frontend stack refresh and browser-dev mock runtime |
| W3-03 | 3 | L | Element API formats and state |
| W3-07 | 3 | M | Live meeting overlay websocket |
| W3-09 | 3 | L | Retention v2: lean mode, strip, NULL-safe deletes, coverage |
| W3-10 | 3 | L | PII redaction surfaces v2 |
| W3-15 | 3 | L | Software AEC3 + NS + AGC2 (aecMode=screenpipe) |
| W3-16 | 3 | L | Reconciliation durability, identification gating, STT retries, dedup |
| W3-17 | 3 | L | Startup health diagnostics (health-startup.json) and ORT init watchdog |
| W3-18 | 3 | L | Notifications v2 backend on :11435 |
| W3-23 | 3 | L | Pi extension set expansion and extension packages |
| W3-24 | 3 | L | Agent self-improvement: skills store, user profile, learning |
| W3-27 | 3 | L | SQLite hard-fault quarantine coordinator |
| W3-28 | 3 | L | Headless mode and search-only mode |
| W3-29 | 3 | M | Recording health incident state |
| W3-31 | 3 | L | SOP narration providers: OpenAI-compatible, local OS TTS, captions-only |
| W4-03 | 4 | XL | ACP agent runtime and screenpipe-tools MCP |
| W4-04 | 4 | L | Cloud coding agents (BYO Cursor/Codex/Claude) |
| W4-05 | 4 | M | Chat runtime commands and per-turn connections context |
| W4-14 | 4 | L | Capture loop B: tree-walker worker and foreground identity gate |
| W4-16 | 4 | L | Bluetooth meeting gate, output follow and speaker watchdog |
| W5-07 | 5 | XL | Hybrid storage format hybrid-parquet-v1 |
| W5-11 | 5 | L | Captured-moment replay server and open command |

## Wave integration log

### Wave 0: partial (recorded 2026-10-20T05:34:35Z, main `4d052c5`)

# Wave 0 integration

Recorded 2026-10-20 on main `4d052c5`, which carries all 13 wave 0 work packages. (The unreviewed W1-01 checkpoint merged as PR #11 was reverted in `61f0faf`, so the code under `crates/` and `apps/` is the reviewed wave 0 state.)

## CI on main

- Freenpipe CI (windows, linux, gates): success, https://github.com/normiecore/Freenpipe/actions/runs/39369534010
- Windows CLI Integration Test: success, https://github.com/normiecore/Freenpipe/actions/runs/39369534021
- Secret scan: success, https://github.com/normiecore/Freenpipe/actions/runs/39369534077
- Release build of all five installers (Windows x64/ARM, macOS Intel/Apple Silicon, Linux) from `c5489be`, the same code: success, published as pre-release `v2.5.28-alpha.1`, https://github.com/normiecore/Freenpipe/actions/runs/39361490499

## Acceptance checks

| WP | Check | Result |
|---|---|---|
| W0-01 | release-dev engine and app builds; frontend build and bindings check | pass (CI windows/linux jobs and the release build) |
| W0-01 | `ee/desktop` grep | pass: no build reference; two doc comments in screenpipe-team-memory and screenpipe-sync name upstream file paths |
| W0-01 | harness `/health` on port 3035 with FP_DD | not run (needs a local engine build; disk is reserved for wave 1 builds) |
| W0-01, W0-12 | vendor guard | pass (strict mode, CI gates) |
| W0-02 | telemetry key and host grep | pass: remaining hits are the user's own Sentry connector (`screenpipe-connect`) and a PII regex that detects PostHog keys |
| W0-02 | 2-minute engine log without posthog/sentry | not run (needs a local engine run) |
| W0-03 | `api/app-update` grep, release workflow grep | pass |
| W0-03, W0-04, W0-07, W0-09, W0-11 | cargo tests (updates::, entitlement, oauth::, speaker::models, screenpipe-db) | pass (CI engine and app test steps) |
| W0-03 | latest.json generator on fixtures | pass (app crate test `release_script_fixture_parses_as_a_tauri_manifest`) |
| W0-04, W0-05, W0-06, W0-09, W0-10 | bun tests | pass (CI frontend tests) |
| W0-04, W0-05, W0-06, W0-07, W0-09, W0-10 | vendor endpoint greps | pass |
| W0-06, W0-08 | harness checks on port 3035 (gateway 503 and forwarding, empty pipe store, static registry) | covered by the WPs' own integration tests in CI (`cloud_proxy_test`, pipe store tests); live harness not run |
| W0-08 | `screenpi.pe` endpoint grep | pass: remaining hits are a vendor-name filter list in pi.rs and a test email address in calendar_speaker_id.rs |
| W0-11 | SQLite version, FTS5 contentless_delete, out-of-order migration | pass (CI `sqlite_toolchain_test`, `migrator_compat_test`) |
| W0-12 | route, settings key and SQL identifiers unchanged | pass (W0-12 review) |
| W0-13 | no vendor-org git deps; third_party/README.md complete | pass (grep; CI third-party guard) |
| W0-04, W0-05, W0-07, W0-10, W0-12 | UI, tray, OAuth, installer and registry checks | Windows smoke checklist |

## Windows smoke checklist

46 wave 0 entries in `docs/dev/windows-smoke-checklist.md`; 0 ticked by the owner so far. The pre-release installer `v2.5.28-alpha.1` is the build to run them on.

## Follow-ups

- Run the local harness checks (W0-01 `/health`, W0-02 log scan, W0-06 and W0-08 live calls) when disk allows a main-checkout engine build.
- Review-time follow-ups are listed in `docs/dev/coordinator-handoff.md`.

Complete waves awaiting the integration checklist: 1.

## Board

### Wave 0: Foundation: build, ee/ stubs, de-cloud, rebrand (13/13 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W0-01 | L | merged | Build bootstrap, ee/ stubs, endpoint switchboard and dev isolation | merge `be94bb3` |
| W0-02 | M | merged | Remove telemetry and crash reporting (PostHog, Sentry) | merge `56b1812` |
| W0-03 | M | merged | Updater on GitHub releases, release pipeline de-vendoring, restart_for_update | merge `6c659c4` |
| W0-04 | L | merged | Remove entitlement, login and plan gates | merge `61d5b1a` |
| W0-05 | L | merged | Remove commerce, team and vendor UI surfaces | merge `63b3c83` |
| W0-06 | L | merged | AI gateway, transcription, PII backend and media analysis to bring-your-own | merge `aa8ec6a` |
| W0-07 | L | merged | OAuth broker replaced by BYO client credentials | merge `55174d9` |
| W0-08 | M | merged | Cloud sync, archive and pipe store off by default | merge `4c31752` |
| W0-09 | M | merged | Bundled CLI/MCP runtime and self-hosted model sources | merge `c83b674` |
| W0-10 | M | merged | Logs and feedback upload replaced by a local diagnostics bundle | merge `7869d46` |
| W0-11 | L | merged | SQLite toolchain upgrade (sqlx 0.9, libsqlite3-sys 0.37, rusqlite vtab) | merge `7b684de` |
| W0-12 | M | merged | Rebrand user-visible identity to Freenpipe | merge `06383cc` |
| W0-13 | M | merged | Vendor git dependencies into third_party/ | merge `d482528` |

### Wave 1: Cross-cutting foundations: schema v2.7, settings schema, integration seams (5/5 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W1-01 | L | merged | Schema v2.7.104: 28 migrations and ocr_text retirement | merge `0ca3a2b` |
| W1-02 | L | merged | Settings schema v2 and store compatibility | merge `5d279d8` |
| W1-03 | M | merged | Integration seams, stub modules and shared constants | merge `b3d3c31` |
| W1-04 | S | merged | Enterprise and plan-gate no-op surface | merge `1bdc390` |
| W1-05 | M | merged | Licence gating for commercial builds | merge `40c94a6` |

### Wave 2: Engine data layer and core backends (17/27 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W2-01 | L | merged | DB runtime hardening: pragmas, WAL maintenance, preflight, planner stats, cancellable reads | merge `789156e` |
| W2-02 | L | ready | Outputs and artifacts registry |  |
| W2-03 | M | ready | Unified AI feedback table and /feedback |  |
| W2-04 | M | merged | Starred work sessions backend and HD binding | merge `bc96012` |
| W2-05 | M | merged | Semantic store: runs, items, FTS, actors | merge `94ea731` |
| W2-06 | XL | ready | screenpipe-semantic parser crate |  |
| W2-07 | L | ready | Activity ledger store and deterministic reconciler |  |
| W2-08 | L | merged | Meetings API v2: calendar binding, summary endpoints, room-change route | merge `3b53e0d` |
| W2-09 | M | merged | Response formats, calendar time literals, tag autocomplete | merge `8f0ad8b` |
| W2-10 | L | merged | Frame media routes: thumbnail, preview samples, preview media, typed errors, auth | merge `7cea019` |
| W2-11 | L | merged | Per-monitor vision control and display layout persistence | merge `257bea6` |
| W2-12 | L | merged | Native WinRT/D3D11 Windows Graphics Capture and frame-diff hardening | merge `ca237c2` |
| W2-13 | L | merged | Health v2, vision metrics and recording coverage | merge `fdcfc51` |
| W2-14 | L | ready | Resource monitor v2: process tree, breakdown, leak trend |  |
| W2-15 | L | ready | Snapshot compaction v2 and suspend/lock awareness |  |
| W2-16 | L | merged | Speakers v2: samples, search paging, merge integrity, reassign v2 | merge `49311c0` |
| W2-17 | L | merged | Stream-scoped live speakers and strict meeting-segment mapping | merge `6b3a4ab` |
| W2-18 | L | merged | Windows WASAPI loopback engine and exe-based audio exclusions | merge `584aac4` |
| W2-19 | L | merged | GPU runtimes and bundled models: Vulkan whisper, DirectML, install layout | merge `ee8632a` |
| W2-20 | L | ready | Connections core: secret-safe reads, webhook proxy, Telegram send, router deltas, MCP refs |  |
| W2-21 | L | ready | OAuth v2 flow and new OAuth integrations |  |
| W2-22 | L | merged | Pipe config v2: new frontmatter, schedule_config and preview, ids, preamble | merge `8d2ceb1` |
| W2-23 | L | merged | Pi runtime 0.84.1, isolated config, baseline bun, compaction patch | merge `1d22b9e` |
| W2-24 | M | ready | Engine local stubs: attribution headers, telemetry no-op routes, local data-sync |  |
| W2-25 | L | ready | Frontend stack refresh and browser-dev mock runtime |  |
| W2-26 | L | merged | App engine supervision: auto-respawn, port-conflict classification, safe relaunch | merge `b215b2f` |
| W2-27 | L | merged | App startup hardening: argv helper modes, autostart, focus handoff, env, AVX2, WER, opener | merge `db1b49f` |

### Wave 3: Feature backends (search, pipes, connections, capture, audio) (5/31 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W3-01 | L | blocked | Semantic projection worker and actors API | waits for W2-06 |
| W3-02 | L | claimed | Search v2: parsed content, starred, actor and related filters | unknown, last commit 2026-10-26T22:36:01Z |
| W3-03 | L | ready | Element API formats and state |  |
| W3-04 | L | blocked | Activity APIs: activity-summary v2 and activity-ledger route | waits for W2-07 |
| W3-05 | L | blocked | Structured output targets | waits for W2-02, W2-03 |
| W3-06 | L | claimed | Event-trigger idempotency, run outcome verification, meeting-summary finalizer | unknown, last commit 2026-10-26T20:45:45Z |
| W3-07 | M | ready | Live meeting overlay websocket |  |
| W3-08 | L | claimed | UI-scan meeting detector: calendar binding and lock awareness | unknown, last commit 2026-10-26T20:39:53Z |
| W3-09 | L | ready | Retention v2: lean mode, strip, NULL-safe deletes, coverage |  |
| W3-10 | L | ready | PII redaction surfaces v2 |  |
| W3-11 | L | merged | Capture loop hardening A: stages and wedge watchdog, probe timeouts, fail-closed exclusions, corrupt frames | merge `171689d` |
| W3-12 | M | merged | Dedicated Windows OCR worker and OCR availability reporting | merge `fb8c43d` |
| W3-13 | L | claimed | URL domain rules, Windows window identity, input-hook watchdog | unknown, last commit 2026-10-26T22:37:18Z |
| W3-14 | L | merged | Audio capture modes and device status events | merge `0418c0a` |
| W3-15 | L | ready | Software AEC3 + NS + AGC2 (aecMode=screenpipe) |  |
| W3-16 | L | ready | Reconciliation durability, identification gating, STT retries, dedup |  |
| W3-17 | L | ready | Startup health diagnostics (health-startup.json) and ORT init watchdog |  |
| W3-18 | L | ready | Notifications v2 backend on :11435 |  |
| W3-19 | L | blocked | Slack user-token integration with read routes | waits for W2-21 |
| W3-20 | L | blocked | IMAP read-only inbox integration | waits for W2-20 |
| W3-21 | L | blocked | BYO MCP: registry search, MCP-OAuth-first tiles, session allow-gate, relay-free OAuth | waits for W2-20 |
| W3-22 | M | blocked | Local integrations: memory-sync guidance, Obsidian memories, Hermes, WhatsApp sign-out | waits for W2-20 |
| W3-23 | L | ready | Pi extension set expansion and extension packages |  |
| W3-24 | L | ready | Agent self-improvement: skills store, user profile, learning |  |
| W3-25 | L | merged | Local OpenAI-compatible proxy and BYO web search | merge `6fb2c91` |
| W3-26 | M | blocked | Connect broker and connection-gate tools | waits for W2-20 |
| W3-27 | L | ready | SQLite hard-fault quarantine coordinator |  |
| W3-28 | L | ready | Headless mode and search-only mode |  |
| W3-29 | M | ready | Recording health incident state |  |
| W3-30 | M | merged | Low-disk recording guard and keep-awake | merge `72be431` |
| W3-31 | L | ready | SOP narration providers: OpenAI-compatible, local OS TTS, captions-only |  |

### Wave 4: Advanced backends, agents runtime, app shell (0/22 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W4-01 | XL | blocked | Workflows core and /workflows API | waits for W3-23, W2-02 |
| W4-02 | L | blocked | Pipe scheduler v2: defer/coalesce, topics, preset '*', trajectories, log archive | waits for W3-06 |
| W4-03 | XL | ready | ACP agent runtime and screenpipe-tools MCP |  |
| W4-04 | L | ready | Cloud coding agents (BYO Cursor/Codex/Claude) |  |
| W4-05 | M | ready | Chat runtime commands and per-turn connections context |  |
| W4-06 | M | blocked | Bundled skills update and starter skills | waits for W3-23 |
| W4-07 | L | blocked | CLI v2: service, diagnose, richer status, agent setup | waits for W3-17 |
| W4-08 | M | blocked | Pipe permissions v2 | waits for W3-02, W3-21 |
| W4-09 | L | blocked | Database recovery v2: embedded page-level recovery | waits for W3-27 |
| W4-10 | L | blocked | Data maintenance, vault and secrets DB split | waits for W3-27 |
| W4-11 | L | blocked | Live Views backend | waits for W3-05 |
| W4-12 | XL | blocked | Audio-process (mic-holder) meeting detector | waits for W3-08 |
| W4-13 | XL | blocked | Event-driven retained UIA tree on Windows | waits for W3-13 |
| W4-14 | L | ready | Capture loop B: tree-walker worker and foreground identity gate |  |
| W4-15 | L | blocked | Redaction pseudonyms, a11y-aware secret detection, configurable models | waits for W3-10 |
| W4-16 | L | ready | Bluetooth meeting gate, output follow and speaker watchdog |  |
| W4-17 | L | blocked | Calendar v2: all-calendar Google events and reminder scheduler | waits for W2-21, W3-18 |
| W4-18 | L | blocked | Bee pairing and browser snapshot v2 with /act | waits for W2-20 |
| W4-19 | L | blocked | Window inventory, screen-share protection, renderer heartbeat | waits for W3-28 |
| W4-20 | L | blocked | UI localization (en + ja) and native string tables | waits for W2-25 |
| W4-21 | XL | blocked | Narrated SOP video renderer (core) | waits for W3-31 |
| W4-22 | L | blocked | App shell: (main) route group, navigation v2, command palette | waits for W2-25 |

### Wave 5: Feature UIs and second-tier backends (0/24 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W5-01 | L | blocked | Pipes API v2 and SSE output stream | waits for W4-02 |
| W5-02 | L | blocked | Skills registry, external-agent skill export, provider automations | waits for W4-06, W4-03 |
| W5-03 | M | blocked | Meeting room-change offers | waits for W4-12 |
| W5-04 | L | blocked | Audio device lifecycle hardening and shared engine reuse | waits for W4-16 |
| W5-05 | L | blocked | Capture loop C: screenshot policy, scroll cadence, width cap, frozen streams | waits for W4-14 |
| W5-06 | L | blocked | OCR text-region gate and crop OCR | waits for W4-14 |
| W5-07 | XL | ready | Hybrid storage format hybrid-parquet-v1 |  |
| W5-08 | L | blocked | Database self-heal user flow | waits for W4-09 |
| W5-09 | M | blocked | Tray menu v2 | waits for W4-20 |
| W5-10 | L | blocked | Workflow video jobs in the app | waits for W4-21 |
| W5-11 | L | ready | Captured-moment replay server and open command |  |
| W5-12 | L | blocked | Activity history generator (app) | waits for W3-04 |
| W5-13 | L | blocked | Notifications UI v2: inbox window, bell, panel, preferences | waits for W3-18, W4-19 |
| W5-14 | L | blocked | Starred sessions UI, star shortcut, overlay snooze/anchor, CLI | waits for W4-19 |
| W5-15 | L | blocked | Chat tabs, split pane, side chat, groups and landing cards | waits for W4-05, W4-22 |
| W5-16 | L | blocked | Library: memories and artifacts | waits for W2-02, W4-22 |
| W5-17 | L | blocked | Meetings workspace UI | waits for W4-22 |
| W5-18 | L | blocked | Timeline and search window v2 | waits for W3-02, W4-22 |
| W5-19 | L | blocked | Settings navigation v2, general, AI features, permissions | waits for W4-22, W4-20 |
| W5-20 | L | blocked | Screen and audio settings | waits for W3-15, W4-22 |
| W5-21 | M | blocked | Deep links v2 | waits for W4-22 |
| W5-22 | M | blocked | Viewer, permission recovery and help refresh | waits for W4-22 |
| W5-23 | L | blocked | Connections UI catalogue | waits for W3-19, W3-20, W3-21, W4-18 |
| WR-02 | L | blocked | Sync and archive to bring-your-own storage | waits for W4-10 |

### Wave 6: Agent/workflow UIs, piggyback, hybrid bulk (0/21 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W6-01 | XL | blocked | ACP presets and coding-agent chat UI | waits for W4-03, W5-15 |
| W6-02 | M | blocked | Run destinations and conversation continuity | waits for W5-01 |
| W6-03 | L | blocked | Connection-source pipe triggers | waits for W4-02, W3-19, W3-20, W2-21 |
| W6-04 | L | blocked | External chat import and chat-control broker | waits for W5-15 |
| W6-05 | XL | blocked | Meeting piggyback (smart recording) | waits for W5-04, W4-12, W4-16 |
| W6-06 | XL | blocked | Hybrid bulk column archival and screenpipe_elements virtual table | waits for W5-07 |
| W6-07 | XL | blocked | Workflows workspace UI | waits for W4-01, W4-22 |
| W6-08 | L | blocked | Workflow ready-to-review notifications | waits for W4-01, W3-18 |
| W6-09 | L | blocked | Video render CLI mode and Pi video-sop extension | waits for W5-10 |
| W6-10 | M | blocked | Workflow-to-skill drafting | waits for W4-01, W3-24 |
| W6-11 | L | blocked | Built-in MCP server v0.20 and portable MCP launcher | waits for W4-01, W2-03, W5-01 |
| W6-12 | XL | blocked | Live Views dashboards UI | waits for W4-11, W5-16 |
| W6-13 | L | blocked | Activity view UI | waits for W5-12, W3-04 |
| W6-14 | M | blocked | Recording status popover and first-run guide | waits for W4-22 |
| W6-15 | L | blocked | Privacy settings v2 | waits for W3-13, W3-10, W4-15 |
| W6-16 | L | blocked | Onboarding v2 | waits for W5-01, W4-22 |
| W6-17 | L | blocked | Automations (pipes) UI v2 | waits for W5-01 |
| W6-18 | L | blocked | Right sidebar: multi-tab owned browser, files, connections | waits for W5-15 |
| W6-19 | L | blocked | Shortcut overlay v2 (webview) | waits for W5-14, W3-07, W3-29 |
| WR-01 | L | blocked | Pipe store on a GitHub-hosted registry | waits for W5-01 |
| WR-03 | XL | blocked | Freenpipe Hub: self-hostable team server and team client | waits for W3-02, W4-10, W5-16 |

### Wave 7: Hybrid lifecycle, overlays, chat follow-ups (0/9 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W7-01 | XL | blocked | Storage migration lifecycle (SQLite to hybrid) | waits for W6-06 |
| W7-02 | XL | blocked | Hybrid compact, sparse reclaim, backup bundle and retention strip | waits for W6-06, W3-09 |
| W7-03 | L | blocked | SOP video project workspace and gate removal | waits for W6-07, W6-09 |
| W7-04 | L | blocked | Native Win32 shortcut overlay pill | waits for W6-19 |
| W7-05 | M | blocked | Chat composer upgrades | waits for W6-01 |
| W7-06 | L | blocked | First-run summary | waits for W5-12, W6-01 |
| W7-07 | L | blocked | Agent worktrees and per-chat coding workspace | waits for W6-01, W6-04 |
| W7-08 | L | blocked | Built-in pipe catalogue and builtin text migrations | waits for W6-03, W4-01, W3-06 |
| WR-04 | L | blocked | Composio integrations (BYO key) and workflow sharing | waits for W2-20, W6-07, WR-01 |

### Wave 8: Final UI pieces and surface audit (0/3 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W8-01 | XL | blocked | Rich chat blocks and local feedback UI | waits for W7-05, W2-03 |
| W8-02 | L | blocked | Storage and retention settings UI with migration overlays | waits for W7-01, W7-02, W3-09, W4-10 |
| W8-03 | L | blocked | Tauri command/event surface audit, ACL and release readiness | waits for W8-01, W8-02, W7-01, W7-02, W7-03, W7-04 and 4 more |
