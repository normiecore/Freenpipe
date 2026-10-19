# Freenpipe work-package status

> Generated file: the merger routine regenerates it with `python3 tools/plan/status.py --write`. Do not edit it by hand. The plan is [PLAN.md](PLAN.md); the process is [docs/dev/cloud-runbook.md](../dev/cloud-runbook.md).

Generated 2026-10-16T05:01:31Z from `origin/main` (latest code commit `63b3c83`). GitHub data: available (via GitHub connector snapshot).

## Progress

**7 of 154 work packages merged (4.5%)**: 4 in review, 0 claimed, 0 ready, 143 blocked.

| Wave | Theme | Merged | In review | Claimed | Ready | Blocked | Progress |
|---|---|---|---|---|---|---|---|
| 0 | Foundation: build, ee/ stubs, de-cloud, rebrand | 7/13 | 4 | 0 | 0 | 2 | `#####-----` 54% |
| 1 | Cross-cutting foundations: schema v2.7, settings schema, integration seams | 0/4 | 0 | 0 | 0 | 4 | `----------` 0% |
| 2 | Engine data layer and core backends | 0/27 | 0 | 0 | 0 | 27 | `----------` 0% |
| 3 | Feature backends (search, pipes, connections, capture, audio) | 0/31 | 0 | 0 | 0 | 31 | `----------` 0% |
| 4 | Advanced backends, agents runtime, app shell | 0/22 | 0 | 0 | 0 | 22 | `----------` 0% |
| 5 | Feature UIs and second-tier backends | 0/24 | 0 | 0 | 0 | 24 | `----------` 0% |
| 6 | Agent/workflow UIs, piggyback, hybrid bulk | 0/21 | 0 | 0 | 0 | 21 | `----------` 0% |
| 7 | Hybrid lifecycle, overlays, chat follow-ups | 0/9 | 0 | 0 | 0 | 9 | `----------` 0% |
| 8 | Final UI pieces and surface audit | 0/3 | 0 | 0 | 0 | 3 | `----------` 0% |
| all | | 7/154 | 4 | 0 | 0 | 143 | `----------` 4.5% |

CI on main (merge gate on the latest non-status commit `63b3c83`): **pending** (linux: in_progress; test-windows: in_progress; windows: in_progress)

## In review

| WP | PR | Head | CI | Review | Mergeable | Worker |
|---|---|---|---|---|---|---|
| W0-06 | [#5](https://github.com/normiecore/Freenpipe/pull/5) | `c433f80` | success | pending | conflict | - |
| W0-09 | [#2](https://github.com/normiecore/Freenpipe/pull/2) | `ed4f0ac` | success | pending | conflict | - |
| W0-10 | [#3](https://github.com/normiecore/Freenpipe/pull/3) | `4c6fc1e` | none | pending | conflict | - |
| W0-11 | [#4](https://github.com/normiecore/Freenpipe/pull/4) | `4134aa3` | success | approved | conflict | - |

## Claimed

None.

## Ready, in plan order

None.

## Wave integration log

No wave integration has been recorded by the merger yet.

## Board

### Wave 0: Foundation: build, ee/ stubs, de-cloud, rebrand (7/13 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W0-01 | L | merged | Build bootstrap, ee/ stubs, endpoint switchboard and dev isolation | merge `be94bb3` |
| W0-02 | M | merged | Remove telemetry and crash reporting (PostHog, Sentry) | merge `56b1812` |
| W0-03 | M | merged | Updater on GitHub releases, release pipeline de-vendoring, restart_for_update | merge `6c659c4` |
| W0-04 | L | merged | Remove entitlement, login and plan gates | merge `61d5b1a` |
| W0-05 | L | merged | Remove commerce, team and vendor UI surfaces | merge `63b3c83` |
| W0-06 | L | in_review | AI gateway, transcription, PII backend and media analysis to bring-your-own | PR #5: CI success, review pending |
| W0-07 | L | merged | OAuth broker replaced by BYO client credentials | merge `55174d9` |
| W0-08 | M | merged | Cloud sync, archive and pipe store off by default | merge `4c31752` |
| W0-09 | M | in_review | Bundled CLI/MCP runtime and self-hosted model sources | PR #2: CI success, review pending |
| W0-10 | M | in_review | Logs and feedback upload replaced by a local diagnostics bundle | PR #3: CI none, review pending |
| W0-11 | L | in_review | SQLite toolchain upgrade (sqlx 0.9, libsqlite3-sys 0.37, rusqlite vtab) | PR #4: CI success, review approved |
| W0-12 | M | blocked | Rebrand user-visible identity to Freenpipe | waits for W0-06, W0-09, W0-10, W0-11 |
| W0-13 | M | blocked | Vendor git dependencies into third_party/ | waits for W0-11 |

### Wave 1: Cross-cutting foundations: schema v2.7, settings schema, integration seams (0/4 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W1-01 | L | blocked | Schema v2.7.104: 28 migrations and ocr_text retirement | waits for W0-12 |
| W1-02 | L | blocked | Settings schema v2 and store compatibility | waits for W0-12 |
| W1-03 | M | blocked | Integration seams, stub modules and shared constants | waits for W0-12 |
| W1-04 | S | blocked | Enterprise and plan-gate no-op surface | waits for W0-12 |

### Wave 2: Engine data layer and core backends (0/27 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W2-01 | L | blocked | DB runtime hardening: pragmas, WAL maintenance, preflight, planner stats, cancellable reads | waits for W1-01, W1-03 |
| W2-02 | L | blocked | Outputs and artifacts registry | waits for W1-01, W1-03 |
| W2-03 | M | blocked | Unified AI feedback table and /feedback | waits for W1-01, W1-03 |
| W2-04 | M | blocked | Starred work sessions backend and HD binding | waits for W1-01, W1-03 |
| W2-05 | M | blocked | Semantic store: runs, items, FTS, actors | waits for W1-01, W1-03 |
| W2-06 | XL | blocked | screenpipe-semantic parser crate | waits for W0-12 |
| W2-07 | L | blocked | Activity ledger store and deterministic reconciler | waits for W1-01, W1-03 |
| W2-08 | L | blocked | Meetings API v2: calendar binding, summary endpoints, room-change route | waits for W1-01, W1-03 |
| W2-09 | M | blocked | Response formats, calendar time literals, tag autocomplete | waits for W1-01, W1-03 |
| W2-10 | L | blocked | Frame media routes: thumbnail, preview samples, preview media, typed errors, auth | waits for W1-01, W1-03 |
| W2-11 | L | blocked | Per-monitor vision control and display layout persistence | waits for W1-01, W1-03 |
| W2-12 | L | blocked | Native WinRT/D3D11 Windows Graphics Capture and frame-diff hardening | waits for W0-12 |
| W2-13 | L | blocked | Health v2, vision metrics and recording coverage | waits for W1-03 |
| W2-14 | L | blocked | Resource monitor v2: process tree, breakdown, leak trend | waits for W1-03 |
| W2-15 | L | blocked | Snapshot compaction v2 and suspend/lock awareness | waits for W1-01, W1-03 |
| W2-16 | L | blocked | Speakers v2: samples, search paging, merge integrity, reassign v2 | waits for W1-01 |
| W2-17 | L | blocked | Stream-scoped live speakers and strict meeting-segment mapping | waits for W1-01 |
| W2-18 | L | blocked | Windows WASAPI loopback engine and exe-based audio exclusions | waits for W1-02 |
| W2-19 | L | blocked | GPU runtimes and bundled models: Vulkan whisper, DirectML, install layout | waits for W0-12 |
| W2-20 | L | blocked | Connections core: secret-safe reads, webhook proxy, Telegram send, router deltas, MCP refs | waits for W1-03 |
| W2-21 | L | blocked | OAuth v2 flow and new OAuth integrations | waits for W1-03 |
| W2-22 | L | blocked | Pipe config v2: new frontmatter, schedule_config and preview, ids, preamble | waits for W1-03 |
| W2-23 | L | blocked | Pi runtime 0.84.1, isolated config, baseline bun, compaction patch | waits for W1-03 |
| W2-24 | M | blocked | Engine local stubs: attribution headers, telemetry no-op routes, local data-sync | waits for W1-03 |
| W2-25 | L | blocked | Frontend stack refresh and browser-dev mock runtime | waits for W1-02 |
| W2-26 | L | blocked | App engine supervision: auto-respawn, port-conflict classification, safe relaunch | waits for W1-02 |
| W2-27 | L | blocked | App startup hardening: argv helper modes, autostart, focus handoff, env, AVX2, WER, opener | waits for W1-03 |

### Wave 3: Feature backends (search, pipes, connections, capture, audio) (0/31 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W3-01 | L | blocked | Semantic projection worker and actors API | waits for W2-05, W2-06 |
| W3-02 | L | blocked | Search v2: parsed content, starred, actor and related filters | waits for W2-04, W2-05, W2-09 |
| W3-03 | L | blocked | Element API formats and state | waits for W2-09 |
| W3-04 | L | blocked | Activity APIs: activity-summary v2 and activity-ledger route | waits for W2-04, W2-07 |
| W3-05 | L | blocked | Structured output targets | waits for W2-02, W2-03 |
| W3-06 | L | blocked | Event-trigger idempotency, run outcome verification, meeting-summary finalizer | waits for W2-08, W2-22 |
| W3-07 | M | blocked | Live meeting overlay websocket | waits for W2-17, W2-13 |
| W3-08 | L | blocked | UI-scan meeting detector: calendar binding and lock awareness | waits for W2-08 |
| W3-09 | L | blocked | Retention v2: lean mode, strip, NULL-safe deletes, coverage | waits for W2-05, W2-11 |
| W3-10 | L | blocked | PII redaction surfaces v2 | waits for W1-01, W1-02 |
| W3-11 | L | blocked | Capture loop hardening A: stages and wedge watchdog, probe timeouts, fail-closed exclusions, corrupt frames | waits for W2-11, W2-13 |
| W3-12 | M | blocked | Dedicated Windows OCR worker and OCR availability reporting | waits for W1-01, W2-10 |
| W3-13 | L | blocked | URL domain rules, Windows window identity, input-hook watchdog | waits for W1-02, W2-11 |
| W3-14 | L | blocked | Audio capture modes and device status events | waits for W1-02, W2-13 |
| W3-15 | L | blocked | Software AEC3 + NS + AGC2 (aecMode=screenpipe) | waits for W2-18 |
| W3-16 | L | blocked | Reconciliation durability, identification gating, STT retries, dedup | waits for W2-17 |
| W3-17 | L | blocked | Startup health diagnostics (health-startup.json) and ORT init watchdog | waits for W2-13, W2-19 |
| W3-18 | L | blocked | Notifications v2 backend on :11435 | waits for W1-02 |
| W3-19 | L | blocked | Slack user-token integration with read routes | waits for W2-21 |
| W3-20 | L | blocked | IMAP read-only inbox integration | waits for W2-20 |
| W3-21 | L | blocked | BYO MCP: registry search, MCP-OAuth-first tiles, session allow-gate, relay-free OAuth | waits for W2-20 |
| W3-22 | M | blocked | Local integrations: memory-sync guidance, Obsidian memories, Hermes, WhatsApp sign-out | waits for W2-20 |
| W3-23 | L | blocked | Pi extension set expansion and extension packages | waits for W2-23 |
| W3-24 | L | blocked | Agent self-improvement: skills store, user profile, learning | waits for W1-03 |
| W3-25 | L | blocked | Local OpenAI-compatible proxy and BYO web search | waits for W1-03 |
| W3-26 | M | blocked | Connect broker and connection-gate tools | waits for W2-20 |
| W3-27 | L | blocked | SQLite hard-fault quarantine coordinator | waits for W2-01 |
| W3-28 | L | blocked | Headless mode and search-only mode | waits for W2-26 |
| W3-29 | M | blocked | Recording health incident state | waits for W2-26 |
| W3-30 | M | blocked | Low-disk recording guard and keep-awake | waits for W1-02, W1-03 |
| W3-31 | L | blocked | SOP narration providers: OpenAI-compatible, local OS TTS, captions-only | waits for W1-02 |

### Wave 4: Advanced backends, agents runtime, app shell (0/22 merged)

| WP | Size | Status | Title | Detail |
|---|---|---|---|---|
| W4-01 | XL | blocked | Workflows core and /workflows API | waits for W2-22, W3-23, W2-02 |
| W4-02 | L | blocked | Pipe scheduler v2: defer/coalesce, topics, preset '*', trajectories, log archive | waits for W3-06 |
| W4-03 | XL | blocked | ACP agent runtime and screenpipe-tools MCP | waits for W2-23, W2-27 |
| W4-04 | L | blocked | Cloud coding agents (BYO Cursor/Codex/Claude) | waits for W2-22 |
| W4-05 | M | blocked | Chat runtime commands and per-turn connections context | waits for W2-23 |
| W4-06 | M | blocked | Bundled skills update and starter skills | waits for W2-23, W3-23 |
| W4-07 | L | blocked | CLI v2: service, diagnose, richer status, agent setup | waits for W3-17 |
| W4-08 | M | blocked | Pipe permissions v2 | waits for W3-02, W3-21 |
| W4-09 | L | blocked | Database recovery v2: embedded page-level recovery | waits for W3-27 |
| W4-10 | L | blocked | Data maintenance, vault and secrets DB split | waits for W3-27 |
| W4-11 | L | blocked | Live Views backend | waits for W3-05 |
| W4-12 | XL | blocked | Audio-process (mic-holder) meeting detector | waits for W3-08, W2-18 |
| W4-13 | XL | blocked | Event-driven retained UIA tree on Windows | waits for W3-13 |
| W4-14 | L | blocked | Capture loop B: tree-walker worker and foreground identity gate | waits for W3-11 |
| W4-15 | L | blocked | Redaction pseudonyms, a11y-aware secret detection, configurable models | waits for W3-10 |
| W4-16 | L | blocked | Bluetooth meeting gate, output follow and speaker watchdog | waits for W3-14 |
| W4-17 | L | blocked | Calendar v2: all-calendar Google events and reminder scheduler | waits for W2-21, W3-18 |
| W4-18 | L | blocked | Bee pairing and browser snapshot v2 with /act | waits for W2-20 |
| W4-19 | L | blocked | Window inventory, screen-share protection, renderer heartbeat | waits for W3-28 |
| W4-20 | L | blocked | UI localization (en + ja) and native string tables | waits for W2-25, W1-02 |
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
| W5-06 | L | blocked | OCR text-region gate and crop OCR | waits for W3-12, W4-14 |
| W5-07 | XL | blocked | Hybrid storage format hybrid-parquet-v1 | waits for W2-01 |
| W5-08 | L | blocked | Database self-heal user flow | waits for W4-09 |
| W5-09 | M | blocked | Tray menu v2 | waits for W4-20, W2-11 |
| W5-10 | L | blocked | Workflow video jobs in the app | waits for W4-21 |
| W5-11 | L | blocked | Captured-moment replay server and open command | waits for W2-10 |
| W5-12 | L | blocked | Activity history generator (app) | waits for W3-04, W3-25 |
| W5-13 | L | blocked | Notifications UI v2: inbox window, bell, panel, preferences | waits for W3-18, W4-19 |
| W5-14 | L | blocked | Starred sessions UI, star shortcut, overlay snooze/anchor, CLI | waits for W2-04, W4-19 |
| W5-15 | L | blocked | Chat tabs, split pane, side chat, groups and landing cards | waits for W4-05, W4-22 |
| W5-16 | L | blocked | Library: memories and artifacts | waits for W2-02, W4-22 |
| W5-17 | L | blocked | Meetings workspace UI | waits for W2-08, W4-22 |
| W5-18 | L | blocked | Timeline and search window v2 | waits for W3-02, W4-22 |
| W5-19 | L | blocked | Settings navigation v2, general, AI features, permissions | waits for W4-22, W4-20 |
| W5-20 | L | blocked | Screen and audio settings | waits for W3-14, W3-15, W4-22 |
| W5-21 | M | blocked | Deep links v2 | waits for W4-22 |
| W5-22 | M | blocked | Viewer, permission recovery and help refresh | waits for W2-26, W4-22 |
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
| W6-09 | L | blocked | Video render CLI mode and Pi video-sop extension | waits for W5-10, W2-27 |
| W6-10 | M | blocked | Workflow-to-skill drafting | waits for W4-01, W3-24, W3-25 |
| W6-11 | L | blocked | Built-in MCP server v0.20 and portable MCP launcher | waits for W4-01, W2-03, W5-01 |
| W6-12 | XL | blocked | Live Views dashboards UI | waits for W4-11, W5-16 |
| W6-13 | L | blocked | Activity view UI | waits for W5-12, W3-04 |
| W6-14 | M | blocked | Recording status popover and first-run guide | waits for W3-30, W2-11, W4-22 |
| W6-15 | L | blocked | Privacy settings v2 | waits for W3-13, W3-10, W4-15 |
| W6-16 | L | blocked | Onboarding v2 | waits for W5-01, W4-22 |
| W6-17 | L | blocked | Automations (pipes) UI v2 | waits for W5-01 |
| W6-18 | L | blocked | Right sidebar: multi-tab owned browser, files, connections | waits for W5-15 |
| W6-19 | L | blocked | Shortcut overlay v2 (webview) | waits for W5-14, W3-07, W3-29 |
| WR-01 | L | blocked | Pipe store on a GitHub-hosted registry | waits for W2-22, W5-01 |
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
