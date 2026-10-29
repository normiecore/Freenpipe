# Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
#
# Work-package data for docs/clean-room/PLAN.md and docs/clean-room/plan_wps.json.
# Imported by plan_build.py; regenerate with `python3 tools/plan/plan_build.py` after editing.

E = "crates/screenpipe-engine/src/"
D = "crates/screenpipe-db/src/"
C = "crates/screenpipe-core/src/"
CA = "crates/screenpipe-core/assets/"
AU = "crates/screenpipe-audio/src/"
CN = "crates/screenpipe-connect/src/"
SC = "crates/screenpipe-screen/src/"
AX = "crates/screenpipe-a11y/src/"
RD = "crates/screenpipe-redact/src/"
A = "apps/screenpipe-app-tauri/"
AT = "apps/screenpipe-app-tauri/src-tauri/"
AR = "apps/screenpipe-app-tauri/src-tauri/src/"

WAVE_NAMES = {
    0: "Foundation: build, ee/ stubs, de-cloud, rebrand",
    1: "Cross-cutting foundations: schema v2.7, settings schema, integration seams",
    2: "Engine data layer and core backends",
    3: "Feature backends (search, pipes, connections, capture, audio)",
    4: "Advanced backends, agents runtime, app shell",
    5: "Feature UIs and second-tier backends",
    6: "Agent/workflow UIs, piggyback, hybrid bulk",
    7: "Hybrid lifecycle, overlays, chat follow-ups",
    8: "Final UI pieces and surface audit",
}

WPS = []


def wp(id, wave, size, title, items, owner_paths, depends_on, description, acceptance, spec_refs):
    WPS.append(dict(id=id, wave=wave, size=size, title=title, items=items,
                    owner_paths=owner_paths, depends_on=depends_on,
                    description=description, acceptance=acceptance, spec_refs=spec_refs))


W0_MID = ["W0-02", "W0-03", "W0-04", "W0-05", "W0-06", "W0-07", "W0-08", "W0-09", "W0-10", "W0-11"]

# ---------------------------------------------------------------- wave 0
wp("W0-01", 0, "L", "Build bootstrap, ee/ stubs, endpoint switchboard and dev isolation",
   ["cloud-001", "cloud-002", "cloud-003", "cloud-004", "cloud-005", "cloud-009"],
   ["Cargo.toml (workspace exclude list)", AT + "Cargo.toml ([features], optional deps)",
    AR + "enterprise_policy.rs", AR + "enterprise_install_metadata.rs", AR + "enterprise_sync.rs",
    AT + "tests/enterprise_sync_test.rs (delete)", A + "lib/app-update-policy.ts (new)", A + "tsconfig.json",
    A + "vitest.config.ts", A + "components/settings/general-settings.tsx (import lines only)",
    A + "lib/hooks/use-enterprise-policy.ts (import lines only)", A + "lib/hooks/use-settings.tsx (import lines only)",
    ".github/workflows/sdk.yml", ".github/workflows/sdk-release.yml", ".github/workflows/release-enterprise.yml",
    ".github/workflows/sentry-cleanup.yml", ".gitignore", "README.md (ee/sdk link only)",
    C + "endpoints.rs (new)", C + "lib.rs", A + "lib/web-url.ts", AT + "capabilities/main.json (remote.urls)",
    AR + "recording.rs (blind port-kill block only)", AR + "server.rs (blind port-kill block only)",
    "docs/dev/windows-build.md (new)", "scripts/dev/ (new)", "scripts/ci/vendor-guard.test.ts (new)"],
   [],
   "Make the untouched BASE compile on Windows with ee/ absent: write original stubs for the three #[path] shims, drop the "
   "enterprise-build feature, add a local app-update-policy.ts and remove the @ee aliases and ee CI jobs. Document the MSVC/LLVM/"
   "CMake environment (scripts/dev/msvc-env.sh) and the debug-profile CRT mismatch (knf-rs-sys vs whisper-rs-sys; use the "
   "release-dev profile or force one MSVC runtime). Add the empty-by-default endpoint switchboard (Rust endpoints.rs + TS web-url.ts "
   "returning null) that every later de-cloud WP routes through, strip the vendor gateway hosts from capabilities and the UA, and "
   "scaffold a vendor-host guard test in report mode. Dev isolation before any app run: the owner machine runs screenpipe v2.7.104, "
   "so neutralise the base blind taskkill of foreign port holders (log and refuse instead), make dev runs use a separate data dir, "
   "port and SCREENPIPE_API_KEY, and audit hard-coded ~/.screenpipe paths (auth.json) that bypass --data-dir.",
   ["source scripts/dev/msvc-env.sh && cargo build -p screenpipe-engine --profile release-dev exits 0",
    "cargo build --manifest-path apps/screenpipe-app-tauri/src-tauri/Cargo.toml --profile release-dev exits 0 with no ee/ directory; grep -rn 'ee/desktop' apps Cargo.toml returns nothing",
    "cd apps/screenpipe-app-tauri && bun install && bun run build && bun run bindings:check all pass",
    "Harness smoke (PLAN.md section 4): engine started on port 3035 with FP_DD answers `api /health` with JSON; ~/.screenpipe is not touched (mtime of ~/.screenpipe/auth.json unchanged)",
    "bun test scripts/ci/vendor-guard.test.ts runs and prints the remaining vendor-host hits (report mode)"],
   ["specs/cloud.md sections 4-5, cloud-001..005, cloud-009"])

wp("W0-02", 0, "M", "Remove telemetry and crash reporting (PostHog, Sentry)",
   ["cloud-010", "cloud-011", "gap_health-lifecycle-internals-010"],
   [A + "app/providers.tsx", A + "lib/analytics/posthog-shim.ts (new)", A + "package.json (posthog-js, @sentry/* entries only)",
    AR + "analytics.rs", AR + "main.rs (sentry::init, tauri-plugin-sentry, PostHog key regions)", E + "analytics.rs",
    E + "resource_monitor.rs (send_to_posthog + PostHog client/key)", E + "bin/screenpipe-engine.rs (sentry init region)",
    E + "server.rs (vision_pipeline_health PostHog emitter)", E + "telemetry_context.rs", E + "meeting_telemetry.rs",
    "crates/screenpipe-config/src/recording.rs (analytics_enabled default)", "Cargo.toml / crates/screenpipe-engine/Cargo.toml / " + AT + "Cargo.toml (sentry deps)",
    A + "components/settings/recording-settings.tsx (@sentry import only)", A + "components/settings/privacy-section.tsx (@sentry import only)"],
   ["W0-01"],
   "Delete every PostHog key and /capture/ post (app, engine analytics, resource monitor, vision_pipeline_health) and every Sentry DSN, "
   "plugin and crate. Replace posthog-js with a no-op shim exposing the same API so call sites compile; keep local tracing and the panic "
   "log. Add is_telemetry_disabled_by_env() returning true for parity, default analytics_enabled to false in the Rust config "
   "(the TS default flip is done by W0-06, owner of use-settings.tsx defaults).",
   ["grep -rnE 'phc_|sentry\\.io|ingest\\.us\\.sentry' crates apps/screenpipe-app-tauri --include=*.rs --include=*.ts --include=*.tsx (excluding node_modules) returns nothing",
    "cargo tree -p screenpipe-engine -i sentry and the same for the app crate report the package is not in the graph",
    "Engine and app build (release-dev); bun run build passes; engine log over a 2-minute run contains no 'posthog' or 'sentry' lines"],
   ["specs/cloud.md cloud-010, cloud-011", "specs/gap_health-lifecycle-internals.md item 010"])

wp("W0-03", 0, "M", "Updater on GitHub releases, release pipeline de-vendoring, restart_for_update",
   ["cloud-012", "cloud-041", "cloud-V02"],
   [AR + "updates.rs", AT + "tauri.prod.conf.json (plugins.updater only)", AT + "tauri.beta.conf.json (plugins.updater only)",
    AT + "tauri.enterprise.conf.json (delete)", A + "components/update-banner.tsx", ".github/workflows/release-app.yml",
    ".github/workflows/release-cli.yml", ".github/workflows/sync-skills.yml", "packages/cli (npm postinstall scripts)",
    "docs/mintlify/docs-mintlify-mig-tmp/docs.json (analytics block)", "scripts/release/make-latest-json.ts (new)"],
   ["W0-01"],
   "Point the Tauri updater at github.com/normiecore/Freenpipe releases with a new minisign public key; stable reads "
   "releases/latest/download/latest.json, pre-release (beta maps to it) reads a fixed-tag manifest; drop bearer auth and the "
   "auth_required flow; rollback uses per-tag manifests with allowDowngrades. Add restart_for_update(timeout_secs) wrapping the base "
   "restart gate, writing the local update-attempt marker and update-install/update-recovery logs (refuse symlinks). In CI replace the "
   "R2 upload with a GitHub release plus a generated latest.json, delete Sentry symbol uploads, strip the npm postinstall PostHog ping, "
   "the docs analytics block and any job that pulls from the upstream repo.",
   ["grep -rn 'api/app-update' apps returns nothing; the updater endpoint in tauri.prod.conf.json is the GitHub latest.json URL",
    "cargo test (app crate) updates:: covers channel->manifest URL mapping and marker writing; restart_for_update appears in lib/utils/tauri.ts",
    "bun scripts/release/make-latest-json.ts on fixture .sig files emits a valid Tauri latest.json",
    "grep -nE 'r2|sentry|posthog' .github/workflows/release-app.yml .github/workflows/release-cli.yml returns nothing"],
   ["specs/cloud.md cloud-012, cloud-041, cloud-V02", "specs/shell.md shell-016 (merged)"])

wp("W0-04", 0, "L", "Remove entitlement, login and plan gates",
   ["cloud-013", "cloud-014", "cloud-015", "cloud-030", "cloud-031", "cloud-032"],
   [AR + "store.rs (entitlement helpers, User schema, onboarding trial-step region)", AR + "recording.rs (require_app_entitlement)",
    AR + "main.rs (auto-start guard, app-entitlement-required emit)", AR + "oauth.rs (Pro gate lines only)",
    AR + "commands.rs (open_login_window / cloud token region)", AR + "server_core.rs (startup auth env)",
    A + "lib/auth-guard.tsx", A + "lib/app-entitlement.ts", A + "components/app-entitlement-gate.tsx", A + "components/login-dialog.tsx",
    A + "components/deeplink-handler.tsx (auth/api_key, purchase, subscription hosts)", A + "app/onboarding/page.tsx (login step)",
    A + "lib/hooks/use-settings.tsx (user/login fetch block only)", A + "lib/utils/validation.ts",
    A + "components/meeting-notes/calendar-connect-dialog.tsx (isPro)", A + "components/settings/recording-settings.tsx ((pro) labels only)"],
   ["W0-01"],
   "Collapse every license/plan check to a constant-allow helper with the same name and signature (app_entitled_or_dev, "
   "require_app_entitlement, hasAppEntitlement, hasCloudEntitlement, needsAppEntitlementRefresh), remove the login UI entry points, the "
   "POST {web}/api/user call, the auth interceptor and api_key deep-link login. Keep set/get_cloud_token for local key plumbing and keep "
   "every User/entitlement field (plus the target's enterprise_account and plan fields) in the store schema so a screenpipe store.bin "
   "loads unchanged. Export SCREENPIPE_STARTUP_AUTHENTICATION_STATUS=not_required to engine and pipes. Never assign trial-activation "
   "onboarding steps; a stored trial step counts as onboarding complete.",
   ["cargo test (app crate): app_entitled_or_dev() is true; a fixture store.bin carrying user.enterprise_account, subscription_plan=pro_ultra and has_payment_method round-trips",
    "bun test: hasAppEntitlement/hasCloudEntitlement return true and AppEntitlementGate renders children",
    "grep -rn 'app-entitlement-required\\|/api/user\"' apps/screenpipe-app-tauri --include=*.rs --include=*.ts* returns nothing outside tests",
    "UI smoke: fresh store, `bun run tauri dev` (screenpipe app quit), recording auto-starts with no sign-in prompt"],
   ["specs/cloud.md cloud-013..015, cloud-030..032", "specs/shell.md shell-034 (merged)"])

wp("W0-05", 0, "L", "Remove commerce, team and vendor UI surfaces",
   ["cloud-022", "cloud-023", "cloud-039", "cloud-044", "frontend-051"],
   [AR + "tray.rs (plan/upgrade rows, cal.com and download handlers)", A + "components/settings/account-section.tsx",
    A + "components/settings/usage-section.tsx", A + "components/settings/team-section.tsx", A + "components/settings/referral-card.tsx",
    A + "components/changelog-dialog.tsx", A + "components/settings/feedback-section.tsx (vendor links)", A + "lib/hooks/use-team.ts",
    A + "lib/hooks/use-enterprise-policy.ts (network paths)", A + "lib/hooks/use-usage-status.tsx", A + "lib/chat/quota-errors.ts",
    A + "components/settings/sync-settings.tsx (checkout links)", A + "components/settings/archive-settings.tsx (checkout links)",
    A + "components/onboarding/connect-apps.tsx (checkout links)", A + "components/settings/ai-presets.tsx (checkout link only)",
    E + "cli/team.rs", E + "cli/survey.rs", E + "cli_reminder.rs", "packages/screenpipe-mcp/src/index.ts (TEAM_API)",
    AT + "tauri.conf.json (purchase-successful deep link)", "Cargo.toml (exclude screenpipe-team-memory from default members)"],
   ["W0-01"],
   "Delete the vendor commerce and account surfaces present in the base: referral card, team API and seats, usage/credit chip, "
   "upgrade links and /api/cloud-sync/checkout, tray plan/Upgrade rows, purchase-successful deep link, enterprise policy/heartbeat "
   "network calls, CLI team/survey, and the base daily-limit/upsell error copy. Changelog reads GitHub Releases or the bundled "
   "CHANGELOG; docs/discord/download/ideas links go to GitHub. Target-only surfaces (trial paywall, card ask, annual banner, "
   "announcements, remote control policy) are simply never ported; record that in the vendor guard allow-list.",
   ["grep -rnE 'subscription/checkout|cloud-sync/checkout|api/referral|api/team|cal\\.com/team' apps crates packages (non-docs) returns nothing",
    "bun run build and bun test pass; Settings renders without Account/Usage/Team/Referral sections (UI smoke)",
    "Tray menu shows no plan/upgrade row (UI smoke); `cargo build -p screenpipe-engine` succeeds without the team CLI"],
   ["specs/cloud.md cloud-022, cloud-023, cloud-039, cloud-044", "specs/frontend.md frontend-051"])

wp("W0-06", 0, "L", "AI gateway, transcription, PII backend and media analysis to bring-your-own",
   ["cloud-016", "cloud-017", "cloud-020", "cloud-042", "cloud-M01", "agents_pipes-036"],
   [AR + "store.rs (AI preset defaults, STT fallback region)", A + "lib/hooks/use-settings.tsx (defaults and migrations, incl. analyticsEnabled=false)",
    A + "components/settings/ai-presets.tsx", A + "components/rewind/ai-presets-selector.tsx", AR + "ai_gateway.rs (new: get_screenpipe_ai_gateway_url)",
    E + "routes/cloud_proxy.rs", C + "agents/pi.rs (cloud media analysis block, gateway base)",
    AU + "transcription/deepgram/mod.rs", AU + "meeting_streaming/config.rs", RD + "adapters/tinfoil.rs", RD + "adapters/tinfoil_image.rs",
    E + "privacy_filter.rs", A + "components/settings/privacy-section.tsx (piiBackend option)", A + "components/settings/recording-settings.tsx (STT engine options)",
    CA + "extensions/web-search.ts", AT + "assets/extensions/web-search.ts", "packages/ai-gateway (exclude from build)"],
   ["W0-01"],
   "Fresh installs default to an Ollama/OpenAI-compatible preset; the screenpipe-cloud provider resolves to a user-configured "
   "gateway (new get_screenpipe_ai_gateway_url returns the aiGatewayUrl setting or SCREENPIPE_AI_GATEWAY_URL, else 'not configured') "
   "and is migrated once to the BYO preset when none is set. /v1/chat/completions forwards to that gateway or returns 503 "
   "cloud_token_missing. Remove the /v1/usage poll, Pro model upgrades, /v1/models gateway fetch and the base web-search vendor call. "
   "screenpipe-cloud STT is offered only with a gateway; default to local whisper, Deepgram uses the user's key, live notes fall back "
   "to the local engine. piiBackend defaults/migrates to local and vendor Tinfoil constants go; Tinfoil only with an explicit "
   "endpoint. The cloud media analysis skill block defaults off and is injected only with a BYO gateway.",
   ["bun test: fresh-settings defaults select an Ollama/OpenAI-compatible preset and analyticsEnabled=false; a stored piiBackend 'tinfoil' migrates to 'local'",
    "Harness: `api /v1/chat/completions -X POST -d '{\"model\":\"x\",\"messages\":[]}'` with no gateway returns 503 with error cloud_token_missing",
    "With aiGatewayUrl pointing at a local mock OpenAI server, the same call is forwarded (mock logs the request)",
    "grep -rn 'api.screenpipe.com\\|pii.screenpipe.containers' crates apps (non-test) returns nothing"],
   ["specs/cloud.md cloud-016, 017, 020, 042, M01", "specs/agents_pipes.md agents_pipes-036"])

wp("W0-07", 0, "L", "OAuth broker replaced by BYO client credentials",
   ["cloud-018"],
   [CN + "oauth.rs", CN + "oauth_refresh_scheduler.rs", CN + "connections/*.rs (OAuthConfig token_url/uses_pkce fields only)",
    AR + "oauth.rs (exchange/callback region)", A + "components/settings/oauth-client-dialog.tsx (new)",
    A + "components/settings/connections-section.tsx (BYO client form hook-up only)", E + "mcp_servers_api.rs (assert loopback only)"],
   ["W0-01"],
   "Add token_url and uses_pkce to OAuthConfig and per-integration BYO {client_id, client_secret, redirect_uri} stored as "
   "oauth_client:<id> in the SecretStore. Exchange and refresh directly at each provider token endpoint; delete EXCHANGE_PROXY_URL and "
   "the screenpi.pe HTTPS callback relay. Providers that refuse loopback redirects show a user-supplied HTTPS relay field. MCP OAuth "
   "keeps the base loopback callback only.",
   ["cargo test -p screenpipe-connect oauth:: (wiremock token endpoint: code exchange with PKCE, refresh, error mapping)",
    "grep -rn 'EXCHANGE_PROXY_URL\\|screenpi.pe/api/oauth' crates apps returns nothing",
    "Manual: with a BYO Google client, connect Google Calendar via the loopback callback; tokens land in the secret store"],
   ["specs/cloud.md cloud-018", "specs/connections.md connections-oauth-byo-broker (merged)"])

wp("W0-08", 0, "M", "Cloud sync, archive and pipe store off by default",
   ["cloud-019", "cloud-021", "agents_pipes-037"],
   [C + "sync/client.rs", AR + "sync.rs (cloud sync, delete_cloud_data region)", A + "components/settings/sync-settings.tsx",
    A + "components/settings/archive-settings.tsx", E + "routes/pipe_store.rs", E + "cli/pipe.rs", E + "cli/install.rs",
    A + "components/pipe-store.tsx", A + "components/settings/pipes-section.tsx (share -> export file)"],
   ["W0-01"],
   "Remove the screenpi.pe sync/archive defaults; cloud sync and archive commands return 'not configured' and the UI hides them; "
   "keep LAN remote_sync and the screenpipe-sync http_put BYO target. /pipes/store returns an empty list (not an error) when "
   "SCREENPIPE_API_BASE_URL is unset, works against a user-configured static registry, keeps the typed registry_* error codes, "
   "and publish/review are disabled; pipe share becomes 'export pipe file'.",
   ["Harness: `api /pipes/store` returns 200 with an empty data array when SCREENPIPE_API_BASE_URL is unset",
    "With SCREENPIPE_API_BASE_URL pointing at a local static JSON server the store lists its pipes",
    "grep -rn 'screenpi.pe/api/sync\\|screenpi.pe\"' crates apps (non-test) returns nothing"],
   ["specs/cloud.md cloud-019, cloud-021", "specs/agents_pipes.md agents_pipes-037"])

wp("W0-09", 0, "M", "Bundled CLI/MCP runtime and self-hosted model sources",
   ["cloud-024", "cloud-046", "cloud-M02", "agents_pipes-040"],
   [AU + "speaker/models.rs (download source)", RD + "adapters/opf.rs", RD + "adapters/onnx.rs", RD + "adapters/rfdetr.rs", RD + "adapters/rfdetr_mlx.rs",
    C + "agents/pi.rs (screenpipe@latest / screenpipe-mcp@latest references)", C + "agents/bash_env.rs",
    A + "components/onboarding/connect-apps.tsx (MCP config snippet)", A + "components/settings/connections-section.tsx (MCP snippet only)",
    A + "components/settings/agent-card.tsx", CA + "skills/**/SKILL.md (CLI invocation text)", "packages/screenpipe-mcp (local build)",
    AT + "tauri.windows.conf.json (bundle resources for MCP and pyannote models)"],
   ["W0-01"],
   "Stop resolving vendor npm packages at runtime: build packages/screenpipe-mcp and the engine CLI locally, ship them as app "
   "resources and reference them by absolute path (SCREENPIPE_CLI / PATH shim) in skills, MCP configs and agent cards. Pyannote "
   "models are bundled or fetched from a Handvel-controlled mirror; HuggingFace screenpipe/* PII weights stay opt-in and "
   "repo-overridable.",
   ["grep -rnE 'screenpipe@latest|screenpipe-mcp@latest|raw/refs/heads/main' crates apps packages (excluding docs, node_modules) returns nothing",
    "Generated Claude Desktop MCP config points at the bundled MCP entry by absolute path (bun test on the snippet generator)",
    "cargo test -p screenpipe-audio speaker::models resolves the bundled model path before any download"],
   ["specs/cloud.md cloud-024, cloud-046, cloud-M02", "specs/agents_pipes.md agents_pipes-040"])

wp("W0-10", 0, "M", "Logs and feedback upload replaced by a local diagnostics bundle",
   ["cloud-036", "frontend-046", "meetings_memory-009"],
   [A + "components/share-logs-button.tsx", A + "components/feedback-dialog.tsx", A + "app/overlay/page.tsx (log upload code only)",
    A + "components/onboarding/engine-startup.tsx (log upload code only)", AR + "feedback_upload.rs (new)", AR + "log_files.rs"],
   ["W0-01"],
   "Implement start_feedback_upload, redact_pii_for_feedback and read_log_tail(path, maxBytes) locally: build a redacted zip under "
   "<data_dir>/feedback (logs, panic log, crash dumps, update/recovery logs, health snapshot, settings with sensitive keys masked), "
   "emit feedback-upload-completed and offer a prefilled GitHub issue link. Remove every /api/logs upload path from the base JS.",
   ["UI smoke: Help > share logs produces <FP_DD>/feedback/<ts>.zip and shows the completion state",
    "bun test: the redaction helper masks keys matching the sensitive-key rule",
    "grep -rn '/api/logs' apps/screenpipe-app-tauri (non-test) returns nothing"],
   ["specs/cloud.md cloud-036", "specs/frontend.md frontend-046", "specs/meetings_memory.md meetings_memory-009", "specs/shell.md shell-033 (merged)"])

wp("W0-11", 0, "L", "SQLite toolchain upgrade (sqlx 0.9, libsqlite3-sys 0.37, rusqlite vtab)",
   ["db-029"],
   ["Cargo.toml", "Cargo.lock", "crates/*/Cargo.toml (sqlx / rusqlite / libsqlite3-sys lines)", D + "** (API migration only)",
    "crates/screenpipe-secrets/src/store.rs (API migration only)", E + "** sqlx call sites (API migration only)", AR + " sqlx call sites (API migration only)"],
   ["W0-01"],
   "Move to sqlx 0.9 and a bundled libsqlite3-sys with SQLite >= 3.51.3 (recover extension and FTS5 contentless_delete compiled in), "
   "rusqlite with vtab, and add parquet + zstd as dependencies of screenpipe-db (unused until W5-07). Mechanical API migration only; "
   "re-verify that the migrator still applies out-of-order pending versions and still honours set_ignore_missing.",
   ["cargo test -p screenpipe-db passes on the new toolchain",
    "A test asserts `select sqlite_version()` >= 3.51.3 and creates an FTS5 table with contentless_delete=1",
    "A migrator test applies a pending version older than the newest applied one (out-of-order) without error",
    "Engine and app build (release-dev)"],
   ["specs/db.md db-029"])

wp("W0-12", 0, "M", "Rebrand user-visible identity to Handvel",
   ["(no delta id) rebrand: product name, identifier, deep-link scheme, tray, icons, window titles, installer"],
   [AT + "tauri*.conf.json (productName, identifier, deep-link schemes, window titles, bundle names)", AT + "icons/**",
    AT + "windows/hooks.nsh", AR + "tray.rs (tooltip/labels)", AR + "window/** (titles)", A + "app/layout.tsx (metadata)",
    A + "public/** (logos)", A + "components/** (user-visible product-name strings only)", "README.md", C + "endpoints.rs (user agent)",
    "scripts/ci/vendor-guard.test.ts (switch to failing mode)"],
   W0_MID,
   "Last WP of wave 0, after all de-cloud work. Rename user-visible identity to Handvel: productName, bundle identifier "
   "(com.normiecore.handvel and a .dev variant), installer/NSIS names, tray tooltip and menu title, window titles, icons and logos, "
   "About text, outbound user agent and user-facing copy that names the product. Register the handvel:// deep-link scheme (see open "
   "question on a screenpipe:// alias). Keep crate names, route paths, JSON fields, settings keys, env var names, CLI flags and data "
   "file names unchanged. Flip the vendor guard to failing mode.",
   ["Release-dev bundle builds an installer named Handvel; installed app shows Handvel in tray tooltip and window titles (UI smoke)",
    "reg query HKCU\\Software\\Classes\\handvel exists after install and HKCU\\Software\\Classes\\screenpipe is untouched",
    "bun test scripts/ci/vendor-guard.test.ts passes in failing mode",
    "git diff of this WP changes no route string, settings key or SQL identifier (reviewer check with grep on '/\\w' route literals)"],
   ["CLAUDE.md (interop identifiers)", "specs/shell.md (identifier, scheme)"])

# ---------------------------------------------------------------- wave 1
wp("W1-01", 1, "L", "Schema v2.7.104: 28 migrations and ocr_text retirement",
   ["db-001", "meetings_memory-001", "db-005", "db-006", "db-027"],
   [D + "migrations/ (28 new files)", D + "db.rs (run_migrations notice + ocr_text query sites)", D + "write_queue.rs", D + "types.rs (OCR types)",
    D + "migration_worker.rs (delete)", D + "lib.rs (migration_worker export)", "crates/screenpipe-capture/src/paired_capture.rs (OCR write path)",
    RD + "worker/tables.rs (ocr_text surface)", E + "routes/frames.rs, routes/search.rs, routes/streaming.rs (ocr_text refs)",
    E + "hot_frame_cache.rs", E + "video_cache.rs", E + "sync_provider.rs", "crates/screenpipe-db/tests/**", "crates/screenpipe-engine/tests/endpoint_test.rs",
    "crates/screenpipe-screen/tests/** (ocr_text refs)"],
   ["W0-12"],
   "Add the 28 target migrations under their exact versions (20260601000000..20261002190000; one sorts before the base's last two) "
   "with original file text and the DDL recorded in db.md; keep set_ignore_missing and the checksum rewrite path, which is how "
   "screenpipe-written DBs are adopted. Ship 20260613130000 together with the code change it forces: all OCR writes go to "
   "frames.text_json/full_text, ocr_text, its FTS and triggers disappear, search returns an empty ocr_engine, MigrationWorker is "
   "deleted, and the pre-migrate notice is logged. Add the performance indexes (no query rewrite needed for julianday).",
   ["cargo test -p screenpipe-db passes, including a new migrations test asserting 117 applied versions identical to the target list in db.md",
    "On a fresh DB: `sqlq \"select name from sqlite_master where name like 'ocr_text%'\"` is empty and frames has text_json; starred_sessions, outputs, semantic_items_fts, activity_tasks exist",
    "A base-schema fixture DB with ocr_text rows migrates and its text appears in frames.full_text/text_json",
    "Harness: record 2 minutes, `api '/search?content_type=ocr&limit=5'` returns items with empty ocr_engine"],
   ["specs/db.md db-001, db-005, db-006, db-027", "specs/meetings_memory.md meetings_memory-001"])

wp("W1-02", 1, "L", "Settings schema v2 and store compatibility",
   ["shell-001", "shell-002", "shell-003", "shell-037", "shell-M02", "frontend-053", "audio-028", "cloud-033", "cloud-034", "cloud-035"],
   ["crates/screenpipe-config/src/recording.rs", "crates/screenpipe-config/src/defaults.rs", AR + "store.rs (settings + OnboardingStore)",
    AR + "commands.rs (get_cloud_token, get_screenpipe_base_dir, get_active_data_dir)", AR + "main.rs (store init region)",
    A + "lib/hooks/use-settings.tsx", A + "lib/utils/tauri.ts (generated)", E + "recording_config.rs", E + "cli/mod.rs (new audio/vision flags only)"],
   ["W0-12"],
   "Add every new settings key and type once, in target field order, across RecordingSettings, SettingsStore, TS defaults and bindings: "
   "~45 shell keys, audio keys (audioCaptureMode, experimentalMeetingPiggyback, alwaysRecordBluetoothMic, screenpipeAecEnabled, aecMode), "
   "vision keys (disableScreenshots, captureMaxWidth, recordingDetail, includedUrls with the UrlRule/DomainRule types, piiRedactionColumns, "
   "piiRedactionPseudonyms), notification prefs, activities*, sidebarNavLayout, uiLocale, etc. Apply the new defaults (usePiiRemoval, "
   "autoUpdate, filterMusic true), new one-time migrations and the OnboardingStore firstRunSummary* fields; keep hideThinkingBlocks for "
   "compatibility. Move the cloud token into the secret store and fail closed when the store cannot initialise. Remote-control flags, "
   "remote log collection and workflow-sharing keys are accepted for round-trip but forced off and never acted on. Feature WPs consume "
   "these keys and must not add keys of their own.",
   ["cargo test -p screenpipe-config: a target-shaped settings JSON fixture with every new key round-trips and unknown keys are preserved",
    "cargo test (app crate): a screenpipe-shaped store.bin with firstRunSummary* and trialActivationFreshInstall loads; cloud token is migrated to cloud.auth_token",
    "bun test: defaults usePiiRemoval/autoUpdate/filterMusic=true, remoteLogCollectionEnabled=false, workflowSharing absent after load",
    "bun run bindings:check passes"],
   ["specs/shell.md shell-001..003, 037, M02", "specs/frontend.md frontend-053", "specs/audio.md audio-028", "specs/cloud.md cloud-033..035"])

wp("W1-03", 1, "M", "Integration seams, stub modules and shared constants",
   ["shell-025", "shell-038"],
   [E + "server.rs (router composition)", E + "routes/mod.rs", E + "routes/ext.rs (new)", E + "routes/<feature>.rs stubs (starred, semantic, artifacts, structured_outputs, live_views, ai_feedback, activity_ledger, vision, workflows, agent, connect_broker, local_chat, web_search, data_sync_proxy, cloud_agents, internal_telemetry, request_origin, response_format, search_only, pipe_stream, meeting_overlay)",
    E + "lib.rs", E + "core.rs (background worker registry)", E + "connections_api.rs (router merge point only)", E + "connections_routes/ (new: mod.rs + stubs)",
    D + "lib.rs (pub mod ext)", D + "ext/ (new: outputs, feedback, starred, semantic, activity, display_layout, pipe_events, frames_preview, meetings_v2, tags, retention_strip stubs)",
    C + "pipes/hooks.rs (new) + call sites in " + C + "pipes/mod.rs", C + "agents/extensions_manifest.rs (new)", C + "agents/pi.rs (install loop reads manifest)",
    AR + "pi_extensions_manifest.rs (new)", AR + "pi.rs (install loop reads manifest)", AR + "main.rs (mod declarations for planned modules)",
    C + "paths.rs (data-dir file-name constants)", C + "env.rs (new: SCREENPIPE_* names)"],
   ["W0-12"],
   "Pre-build the seams that let later WPs add code in their own files. Engine: each planned route module exists as a stub exposing "
   "an empty oasgen registration and axum router, mounted once from server.rs; middleware layers (request origin, search-only) are "
   "pre-wired as pass-through. DB: an ext/ module tree of empty `impl DatabaseManager` files. Connections: a router merge point for "
   "per-integration route files. Pipes: a run-lifecycle hook module. Pi: data-driven bundled-extension manifests on core and app side. "
   "App: mod declarations plus empty stub files for every planned app module. Centralise data-dir file names (crash-dumps, "
   "recording-recovery.log, update-*.log, health-startup.json, secrets.sqlite, ...) and SCREENPIPE_* env names. No behaviour change.",
   ["Engine and app build; cargo test -p screenpipe-engine --test endpoint_test passes unchanged",
    "`api /openapi.json | python -I -c \"import json,sys;print(len(json.load(sys.stdin)['paths']))\"` prints the same count as before the change",
    "Unit test: the bundled extension manifests list exactly the extensions the base installed",
    "PLAN.md section 5 stub list matches the files on disk (reviewer check)"],
   ["specs/shell.md shell-025, shell-038", "PLAN.md section 5"])

wp("W1-04", 1, "S", "Enterprise and plan-gate no-op surface",
   ["frontend-052", "cloud-043", "agents_pipes-041", "agents_pipes-045", "api-035", "meetings_memory-042"],
   [AR + "enterprise_policy.rs (new no-op commands)", A + "lib/hooks/use-enterprise-policy.ts", E + "history_access.rs (new)",
    C + "pipes/managed.rs (new)"],
   ["W0-12"],
   "Register the target-only enterprise commands the parity frontend calls (apply_enterprise_ui_visibility, get_enterprise_host_identity, "
   "set_enterprise_recording_authorized, is_history_access_restricted, list/sync_managed_team_skills, set_activity_sync_enabled) as "
   "inert, network-free stubs. Provide history_access as always-unrestricted (the history_access_limited code exists only as an "
   "enum value, never returned), managed pipes/team skills as 'not managed', and no pipe-count limit.",
   ["tauri.ts exposes isHistoryAccessRestricted and it resolves false (bun test against bindings)",
    "cargo test -p screenpipe-engine history_access: every caller gets Unrestricted",
    "grep -rn 'history_access_limited' crates shows only the enum definition"],
   ["specs/frontend.md frontend-052", "specs/cloud.md cloud-043", "specs/api.md api-035", "specs/meetings_memory.md meetings_memory-042", "specs/agents_pipes.md agents_pipes-041, 045"])

# ---------------------------------------------------------------- wave 2
wp("W2-01", 2, "L", "DB runtime hardening: pragmas, WAL maintenance, preflight, planner stats, cancellable reads",
   ["db-016", "db-017", "db-018", "db-v2-01"],
   [D + "db.rs (DatabaseManager::new, WAL checkpoint task, wal_checkpoint(), heavy-read helpers)", D + "write_queue.rs (writer pragmas)",
    D + "maintenance.rs (new)", D + "planner_stats.rs (new)", D + "cancellable_query.rs (new)"],
   ["W1-01", "W1-03"],
   "wal_autocheckpoint 0 with a PASSIVE maintenance loop, RESTART at boot, query_only read pool, an offline DELETE/FULL/EXCLUSIVE "
   "mode, bounded SQLite header preflight, refusal of SQLite < 3.51.3, sqlite-vec registration check and a one-manager-per-db .owner "
   "file. Periodic ANALYZE + optimize with the sqlite-planner-maintenance.json report. Interrupt reads when requests drop and roll back "
   "abandoned read transactions. On-demand /data/checkpoint stops using TRUNCATE.",
   ["cargo test -p screenpipe-db: pragma test (wal_autocheckpoint=0 on writer, query_only=1 on reader); preflight rejects a non-SQLite file; version guard",
    "Harness: `api /data/checkpoint -X POST` returns busy/log/checkpointed and the engine log shows RESTART or FULL, never TRUNCATE",
    "With the maintenance interval shortened by a test env, <FP_DD>/sqlite-planner-maintenance.json appears with latest/last_success",
    "Test: dropping a long-running query future interrupts it within 1 s and the pool connection is reusable"],
   ["specs/db.md db-016..018, db-v2-01"])

wp("W2-02", 2, "L", "Outputs and artifacts registry",
   ["db-002", "meetings_memory-002", "meetings_memory-003", "meetings_memory-004", "api-017", "agents_pipes-010"],
   [D + "ext/outputs.rs", E + "routes/artifacts.rs", E + "artifacts_index.rs (new: startup backfill)", C + "pipes/artifacts.rs (new)",
    C + "pipes/hooks.rs (artifact registration hook)", CA + "extensions/register-artifact.ts (new)", AT + "assets/extensions/save-artifact.ts (new)",
    C + "agents/extensions_manifest.rs (entry)", AR + "pi_extensions_manifest.rs (entry)"],
   ["W1-01", "W1-03"],
   "outputs CRUD/list/facets (excluding structured-output rows), the output_search_index sidecar and output_search_fts with hash-skip "
   "reindex and startup backfill. POST /artifacts/register copies into <data>/outputs (100 MB cap, SAF envelope detection), GET /artifacts "
   "merges registered and pipe-dir artifacts with q/limit/offset/per_pipe_limit/include_internal, DELETE /artifacts/:id. Declared pipe "
   "artifacts are auto-registered after runs (source_type pipe-run) via the pipe hook; add the register_artifact pipe tool and the "
   "save_artifact chat tool.",
   ["Harness: `api /artifacts/register -X POST -H 'content-type: application/json' -d '{\"path\":\"C:/tmp/a.md\",\"title\":\"A\"}'` returns an id and the file exists under <FP_DD>/outputs",
    "`api '/artifacts?q=<word in a.md>'` returns it; `api /artifacts/<id> -X DELETE` removes it",
    "A 101 MB file is rejected; an invalid .saf.json envelope returns 400",
    "cargo test -p screenpipe-db outputs: FTS hash-skip and bm25 ordering"],
   ["specs/db.md db-002", "specs/meetings_memory.md 002..004", "specs/api.md api-017", "specs/agents_pipes.md agents_pipes-010"])

wp("W2-03", 2, "M", "Unified AI feedback table and /feedback",
   ["db-010", "meetings_memory-007", "api-018", "agents_pipes-044"],
   [D + "ext/feedback.rs", E + "routes/ai_feedback.rs"],
   ["W1-01", "W1-03"],
   "Upsert/list over the feedback table: natural-key uniqueness per target+actor+device, comment only with a down rating, pipes cannot "
   "rate their own outputs, pipe-scoped visibility, filters kind/target_id/producer/rating/q/since/after_id/order/limit.",
   ["Harness: POST /feedback {target:{kind:'notification',id:'n1'},rating:'up'} returns 200; repeating it updates the same row (sqlq count = 1)",
    "A comment with rating up returns 400; a request carrying a pipe session header rating that pipe's output returns 403",
    "`api '/feedback?kind=notification&rating=up'` lists the row"],
   ["specs/db.md db-010", "specs/meetings_memory.md meetings_memory-007", "specs/api.md api-018", "specs/agents_pipes.md agents_pipes-044"])

wp("W2-04", 2, "M", "Starred work sessions backend and HD binding",
   ["db-015", "api-007", "meetings_memory-010", "vision-starred-hd"],
   [D + "ext/starred.rs", E + "routes/starred.rs", E + "high_fps_controller.rs"],
   ["W1-01", "W1-03"],
   "POST /starred-sessions with optimistic revision (0 creates, mismatch 409), <24 h ranges and start rules, user-only marking; GET "
   "lists overlapping sessions with computed has_audio; membership and coverage-island queries. HighFpsController gains a Starred "
   "session kind for hd_requested sessions, ending at the session end and honouring pauses and exclusions.",
   ["Harness: POST /starred-sessions {id:<uuid>,start,end,hd_requested:false,revision:0} returns revision 1; a stale revision returns 409",
    "A 25 h range returns 400; `api '/starred-sessions?start_time=..&end_time=..'` lists it with has_audio",
    "cargo test -p screenpipe-engine high_fps_controller: an active hd_requested starred session reports HD active until its end"],
   ["specs/db.md db-015", "specs/api.md api-007", "specs/meetings_memory.md meetings_memory-010", "specs/vision.md vision-starred-hd"])

wp("W2-05", 2, "M", "Semantic store: runs, items, FTS, actors",
   ["db-008", "meetings_memory-019", "meetings_memory-020"],
   [D + "ext/semantic.rs", "crates/screenpipe-db/tests/semantic_store_test.rs (new)"],
   ["W1-01", "W1-03"],
   "DB layer for the semantic projection: fingerprinted runs, content-addressed items, run-item hierarchy, external-content FTS, "
   "frames.semantic_run_id attachment in batches, actors/aliases/item-actors with store-time alias lookup and heuristic assignment, "
   "merge/rename/reassign SQL and GC of unreferenced runs and items.",
   ["cargo test -p screenpipe-db --test semantic_store_test: store, FTS match, GC, alias uniqueness, merge and reassign"],
   ["specs/db.md db-008", "specs/meetings_memory.md meetings_memory-019, 020"])

wp("W2-06", 2, "XL", "screenpipe-semantic parser crate",
   ["meetings_memory-021"],
   ["crates/screenpipe-semantic/** (new crate)"],
   ["W0-12"],
   "New pure crate: budgeted compact accessibility tree, domain-separated 32-byte fingerprints, a parser registry of ~50 app/URL "
   "profiles (chat, mail, docs, notes, AI chat, code hosts, editors, terminals) with family parsers, message time parsing and a "
   "validation step that abstains on low confidence. Profiles and parsers are written from the observed catalogue, not copied.",
   ["cargo test -p screenpipe-semantic: synthetic trees for Slack, Teams, Outlook, VS Code and a browser page yield typed items",
    "Fingerprints are stable across runs and differ across domains; node-budget overflow returns the budget error; ambiguous input abstains"],
   ["specs/meetings_memory.md meetings_memory-021"])

wp("W2-07", 2, "L", "Activity ledger store and deterministic reconciler",
   ["db-013", "meetings_memory-013", "meetings_memory-014"],
   [D + "ext/activity.rs", E + "activity_ledger.rs (new reconciler)"],
   ["W1-01", "W1-03"],
   "DB helpers for activity_tasks/intervals/actions/evidence/ledger_state and the deterministic-v1 reconciler worker: 10 s sampled "
   "frames, UI events and audio (>= 8 chars) become app/window/document/semantic/audio/unobserved tasks, with cut-point reconcile, "
   "upserts, task GC and a 7-day backfill. Registered through the core worker registry.",
   ["cargo test: synthetic frames/ui_events/audio produce the expected tasks and intervals; rerunning the reconciler is idempotent",
    "Deleting frames removes their evidence and empty unlocked tasks through the triggers (sqlq counts before/after)"],
   ["specs/db.md db-013", "specs/meetings_memory.md meetings_memory-013, 014"])

wp("W2-08", 2, "L", "Meetings API v2: calendar binding, summary endpoints, room-change route",
   ["db-012", "meetings_memory-025", "meetings_memory-032", "meetings_memory-047", "api-014", "meetings_memory-026"],
   [E + "routes/meetings.rs", E + "meeting_export.rs", D + "ext/meetings_v2.rs", D + "db.rs (meetings rejoin lookup region)",
    D + "types.rs (meeting request/response types)", CA + "pipes/meeting-summary/pipe.md"],
   ["W1-01", "W1-03"],
   "calendar_event_id on start and PUT with conditional binding (unique violation means 'already bound', keep the title); rejoin "
   "lookup excludes room_changed; transcript segments expose audioStartTimeSecs/sessionSpeakerId; list q covers meeting_app; export "
   "include_audio. New POST /meetings/:id/summary (merge or replace the '## Summary' section, 400 on empty or failure text), GET "
   "/meetings/:id/summary-status and POST /meetings/room-change {meetingId, token, decision}. The built-in meeting-summary pipe saves "
   "through the summary endpoint (prompt text rewritten in our own words).",
   ["Harness: POST /meetings/start {calendar_event_id:'evt1'} binds; PUT another meeting with the same id keeps its title and does not error",
    "POST /meetings/<id>/summary {summary:'x'} appends a '## Summary' section; an empty summary returns 400; GET /meetings/<id>/summary-status returns state idle",
    "POST /meetings/room-change {decision:'switch'} ends the meeting with end_reason room_changed and starts a new one",
    "cargo test -p screenpipe-db: rejoin lookup ignores room_changed meetings"],
   ["specs/db.md db-012", "specs/meetings_memory.md 025, 026, 032, 047", "specs/api.md api-014"])

wp("W2-09", 2, "M", "Response formats, calendar time literals, tag autocomplete",
   ["api-001", "api-002", "api-009"],
   [E + "routes/time.rs", E + "routes/response_format.rs", E + "routes/content.rs (autocomplete handler)", D + "ext/tags.rs",
    E + "routes/search.rs (format/fields plumbing only)", E + "routes/elements.rs (format/fields plumbing only)"],
   ["W1-01", "W1-03"],
   "Flexible time params accept today, yesterday and YYYY-MM-DD as local midnights. A shared response formatter gives format=json|csv|tsv "
   "and dotted-path fields= projection (flat JSON keys when fields are used, 400 on unknown format) for /search, /elements and "
   "/frames/:id/elements. GET /tags/autocomplete?q&limit returns name plus frame/audio/memory counts.",
   ["Harness: `api '/search?start_time=today&limit=1'` returns 200; start_time=nonsense returns 400 naming the three accepted families",
    "`api '/search?format=csv&fields=content.text&limit=3' -i` shows text/csv; format=xml returns 400",
    "`api '/tags/autocomplete?q=wo'` returns [{name,frame_count,audio_count,memory_count}]"],
   ["specs/api.md api-001, 002, 009"])

wp("W2-10", 2, "L", "Frame media routes: thumbnail, preview samples, preview media, typed errors, auth",
   ["api-010", "vision-thumbnail-endpoint", "vision-preview-samples", "vision-preview-media", "vision-frame-media-errors", "api-011"],
   [E + "routes/frames.rs", E + "video_utils.rs", E + "frame_thumbnail_cache.rs (new)", D + "ext/frames_preview.rs",
    E + "server.rs (auth exemption list only)"],
   ["W1-01", "W1-03"],
   "GET /frames/:id/thumbnail (width/quality/fallback/retry, in-memory cache, ETag frame-thumbnail-*, x-screenpipe-thumbnail-* headers, "
   "private no-cache), GET /frames/preview-samples (evenly spread refs for an app/domain in <= 24 h, never extracts media), GET "
   "/frames/preview-media/:video_chunk_id (byte ranges over existing local MP4s). /frames/:id adds frame_media_unavailable and "
   "snapshot_read_failed and rechecks the exact frame before snapshot_missing. /frames/* stop being auth-exempt (cookie covers img src).",
   ["Harness: `api '/frames/<id>/thumbnail?width=320' -i` returns image/jpeg with x-screenpipe-thumbnail-width and an ETag; If-None-Match returns 304",
    "`api '/frames/preview-samples?app_name=Code&start_time=..&end_time=..&limit=6'` returns frames; a 25 h range returns 400",
    "`api /frames/preview-media/<chunk> -H 'Range: bytes=0-99' -i` returns 206",
    "Without the bearer header, /frames/<id> returns 403 while /health returns 200"],
   ["specs/api.md api-010, 011", "specs/vision.md thumbnail, preview-samples, preview-media, frame-media-errors"])

wp("W2-11", 2, "L", "Per-monitor vision control and display layout persistence",
   ["vision-device-control", "api-016", "vision-display-layout", "db-007"],
   [E + "routes/vision.rs", E + "vision_manager/manager.rs", E + "vision_manager/monitor_watcher.rs", D + "ext/display_layout.rs",
    E + "ui_recorder.rs (element_ancestors)", AX + "events.rs (ancestor capture)"],
   ["W1-01", "W1-03"],
   "POST /vision/device/start|stop {monitor_id} and GET /vision/device/status [{id,name,active,user_disabled}] (key is id, per the "
   "frontend) backed by an in-memory user-paused set; start_monitor and the watcher skip paused monitors. The watcher writes canonical "
   "sorted display_layout JSON at startup and on change (timeout, retry next pass). ui_events.element_ancestors gets root-first "
   "[{role,name}]. The delete-range keep-newest rule for display_layout is implemented by W3-09.",
   ["Harness: `api /vision/device/status` returns [{id,name,active,user_disabled}]",
    "POST /vision/device/stop {monitor_id} sets user_disabled and no frames arrive from that monitor for 30 s (sqlq count by device); start resumes",
    "sqlq 'select count(*) from display_layout' is >= 1 after startup; ui_events rows from clicks carry element_ancestors JSON"],
   ["specs/vision.md device-control, display-layout", "specs/api.md api-016", "specs/db.md db-007"])

wp("W2-12", 2, "L", "Native WinRT/D3D11 Windows Graphics Capture and frame-diff hardening",
   ["vision-wgc-native-capture", "vision-frame-comparison-hardening"],
   [SC + "wgc_capture.rs", SC + "monitor.rs", SC + "monitor/windows.rs (new)", SC + "frame_comparison.rs", "crates/screenpipe-screen/Cargo.toml",
    "crates/screenpipe-screen/examples/wgc_smoke.rs (new)"],
   ["W0-12"],
   "Replace the xcap VideoRecorder wrapper with direct GraphicsCaptureItem/free-threaded frame pool sessions: border and cursor off, "
   "SetMinUpdateInterval when available, staging-texture readback, pool recreate on resize, device-removed recovery and WARP fallback, "
   "EnumDisplayMonitors enumeration; keep the 3-failure per-frame fallback and the stable_id format. FrameComparer uses fallible "
   "thumbnail allocation (failure = no change, rate-limited structured log).",
   ["cargo test -p screenpipe-screen (frame_comparison allocation-failure path)",
    "cargo run -p screenpipe-screen --example wgc_smoke --profile release-dev writes 10 non-black JPEGs per monitor",
    "Manual: change display scaling while capturing; the pool is recreated and capture continues; stable_id matches the base value"],
   ["specs/vision.md wgc-native-capture, frame-comparison-hardening"])

wp("W2-13", 2, "L", "Health v2, vision metrics and recording coverage",
   ["api-033", "vision-pipeline-metrics", "gap_health-lifecycle-internals-001", "gap_health-lifecycle-internals-002",
    "gap_health-lifecycle-internals-003", "audio-027"],
   [E + "routes/health.rs", E + "recording_coverage.rs", SC + "metrics.rs", AU + "metrics.rs", E + "server.rs (health ticker spawn region)"],
   ["W1-03"],
   "HealthCheckResponse gains dependencies/runtime/startup, unhealthy_reasons, vision_reason, loop_stage fields, audio_capture_mode, "
   "capture_status, recording_coverage (between ui_recorder and pool_stats, field order per gap_health-002), write-queue and pool signals "
   "and extra counters, with a cached snapshot on budget overrun. MetricsSnapshot gains the 13 vision fields; audio metrics gain "
   "last_stream_timeout_at. The 5 s coverage ticker classifies idle/paused/recording/stalled. Coverage keys are never sent anywhere.",
   ["Harness: `api /health` contains recording_coverage with recording_secs, active_stalled_secs, active_paused_secs, idle_secs, active_secs, coverage_ratio in that order, plus unhealthy_reasons, vision_reason and audio_capture_mode",
    "cargo test -p screenpipe-engine recording_coverage: classifier cases (idle, schedule pause, fresh write, stalled) and the 1..15 s tick window",
    "`api /vision/metrics` includes silent_loss_rate and capture_loop_heartbeats"],
   ["specs/api.md api-033", "specs/vision.md pipeline-metrics", "specs/gap_health-lifecycle-internals.md 001-003", "specs/audio.md audio-027"])

wp("W2-14", 2, "L", "Resource monitor v2: process tree, breakdown, leak trend",
   ["gap_health-lifecycle-internals-004", "gap_health-lifecycle-internals-005", "gap_health-lifecycle-internals-006",
    "gap_health-lifecycle-internals-007", "gap_health-lifecycle-internals-008", "gap_health-lifecycle-internals-009"],
   [E + "resource_monitor.rs", E + "resource_breakdown.rs (new)"],
   ["W1-03"],
   "Descendant-tree accounting with swap, loads and footprint; process breakdown groups (app, app_child, mcp_child, mcp_external, other) "
   "with top-12 lists; pressure WARN and opt-in INFO logging; least-squares leak detector over a 6 h window after 30 min warm-up; "
   "18-key JSONL file; INFO 'host:' startup line. Local only.",
   ["cargo test: breakdown classification over a synthetic process table; leak detector fires on synthetic linear growth and not on noise",
    "SAVE_RESOURCE_USAGE=1 engine run writes resource_usage_*.json whose lines carry the 18 keys in order",
    "Engine log contains one INFO line starting 'host: os='"],
   ["specs/gap_health-lifecycle-internals.md 004-009"])

wp("W2-15", 2, "L", "Snapshot compaction v2 and suspend/lock awareness",
   ["vision-compaction-encoder", "gap_health-lifecycle-internals-011", "gap_health-lifecycle-internals-012"],
   [E + "snapshot_compaction.rs", E + "sleep_monitor.rs", E + "power/monitor.rs", E + "power/profile.rs", C + "video.rs",
    "crates/screenpipe-engine/Cargo.toml (Win32_System_Power feature)"],
   ["W1-01", "W1-03"],
   "Probe hevc_nvenc/qsv/amf (and videotoolbox on macOS) then libx264 once with a tiny lavfi test, overridable by "
   "SCREENPIPE_COMPACTION_ENCODER; defer unreadable snapshots, clear missing pointers, keep JPEGs on ffmpeg failure. Register "
   "PowerRegisterSuspendResumeNotification with a system-suspended flag (base polling kept as fallback); compaction computes ordered "
   "deferral reasons and pauses mid-cycle on suspend or lock.",
   ["Engine log shows the probed encoder; SCREENPIPE_COMPACTION_ENCODER=software forces libx264",
    "cargo test: deferral reason ordering (suspended > locked > battery > low-power > power-paused > battery-saver)",
    "Manual: sleep and resume the laptop; the log shows suspend registration and the flag set/cleared; compaction resumes"],
   ["specs/vision.md compaction-encoder", "specs/gap_health-lifecycle-internals.md 011, 012"])

wp("W2-16", 2, "L", "Speakers v2: samples, search paging, merge integrity, reassign v2",
   ["audio-002", "audio-003", "audio-004", "audio-005", "audio-006", "audio-009", "api-013", "db-011"],
   [E + "routes/speakers.rs", D + "db.rs (speaker functions region and speaker self-heal)", E + "calendar_speaker_id.rs",
    A + "components/settings/speakers-section.tsx (sample playback)"],
   ["W1-01"],
   "GET /speakers/sample/:audio_chunk_id?start&end returns a WAV clip; /speakers/search adds limit/offset/include_samples with a light "
   "NOCASE prefix path; audio_samples gain audio_chunk_id and skip unplayable paths; merge/absorb repoint all 5 referencing tables and "
   "delete only unreferenced speakers; reassign gains scope (auto renames an unnamed voice in place, moves matching live segments) with "
   "undo; stricter self-speaker naming; startup self-heal for speaker tables.",
   ["Harness: `api '/speakers/sample/<chunk>?start=0&end=2' -i` returns audio/wav; end<=start returns 400",
    "`api '/speakers/search?name=a&limit=5&include_samples=false'` returns at most 5 names",
    "cargo test -p screenpipe-db speaker_reassignment_test plus new tests: 5-table merge, referenced-speaker delete kept, scope=auto rename and undo"],
   ["specs/audio.md audio-002..006, 009", "specs/api.md api-013", "specs/db.md db-011"])

wp("W2-17", 2, "L", "Stream-scoped live speakers and strict meeting-segment mapping",
   ["audio-001", "db-014", "audio-008", "audio-M01", "audio-M05"],
   [AU + "meeting_streaming/events.rs", AU + "meeting_streaming/deepgram_live.rs", AU + "meeting_streaming/controller.rs",
    D + "db.rs (meeting_transcript_segments insert/backfill and timeline audio region)", AU + "audio_manager/reconciliation.rs (backfill call site only)",
    E + "routes/streaming.rs (timeline audio rows)"],
   ["W1-01"],
   "Live deltas/finals carry stream_id and session_speaker_id, persisted on meeting_transcript_segments (legacy rows read 'legacy'); "
   "grouping uses COALESCE(session_speaker_id, speaker_name). Per-chunk speaker backfill becomes one guarded UPDATE; meeting segments map "
   "to a global speaker only on exact instant/device/direction/text with a single distinct speaker. Timeline audio rows expose is_live "
   "and unique negative audio_chunk_id.",
   ["cargo test -p screenpipe-db: insert with stream_id/session_speaker_id; legacy default; guarded backfill skips ambiguous chunks; strict mapping skips identical-text twins",
    "Harness: timeline stream rows for a live meeting have is_live=1 and distinct negative audio_chunk_id values"],
   ["specs/audio.md audio-001, 008, M01, M05", "specs/db.md db-014"])

wp("W2-18", 2, "L", "Windows WASAPI loopback engine and exe-based audio exclusions",
   ["audio-013", "audio-014"],
   [AU + "core/process_tap.rs", AU + "core/wasapi_loopback.rs (new, cfg windows)", AU + "core/stream.rs (backend selection region)",
    "crates/screenpipe-audio/Cargo.toml (windows crate features)", AR + "audio_exclusions.rs", AR + "permissions.rs (check_coreaudio_process_tap_available)",
    A + "components/settings/recording-settings.tsx (.exe picker filter line)"],
   ["W1-02"],
   "Own event-driven WASAPI loopback client for the default render endpoint, plus per-process include/exclude taps through "
   "ActivateAudioInterfaceAsync on build >= 20348, with rebuild on default-endpoint change, target pid watch, activation/start timeouts "
   "and multi-root fallback. audio-exclusions.json applies on Windows through exclude-process-tree for the first running excluded exe; "
   ".exe metadata/icon reader; tap availability reports true on supported builds.",
   ["Manual: play audio in a browser; output chunks appear (sqlq on audio_chunks with the output device)",
    "Manual: exclude chrome.exe in audio-exclusions.json; its audio is absent while another app's audio is captured",
    "Switching the default output device rebuilds the stream (log line); cargo build for non-Windows cfg still type-checks (cargo check --target x86_64-unknown-linux-gnu -p screenpipe-audio if toolchain allows)"],
   ["specs/audio.md audio-013, 014"])

wp("W2-19", 2, "L", "GPU runtimes and bundled models: Vulkan whisper, DirectML, install layout",
   ["audio-019", "audio-020", "audio-022", "shell-029"],
   ["crates/screenpipe-audio/Cargo.toml (features region)", AU + "transcription/engine.rs", AU + "transcription/whisper/model.rs",
    AU + "transcription/whisper/batch.rs", AU + "transcription/parakeet_windows.rs (new)", AU + "speaker/models.rs (resolution order)",
    AU + "vad/silero.rs (path resolution only)", AU + "models/download.rs", AT + "tauri.windows.conf.json", AT + "windows/hooks.nsh",
    AT + "Cargo.toml (feature flags)", ".github/workflows/release-app.yml (Vulkan SDK step)"],
   ["W0-12"],
   "Compile whisper with ggml-vulkan on Windows and ship vulkan-1.dll; a failed GPU state or inference rebuilds a shared CPU context "
   "and retries once. Parakeet/Qwen3 pick a non-software, non-remote D3D12 adapter, honour SCREENPIPE_DIRECTML, recycle DML sessions "
   "and retry on CPU (implemented in Handvel code unless the newer audiopipe revision's licence is cleared). Bundle segmentation, "
   "embedding and VAD models plus DirectML/ORT/OpenBLAS/Vulkan DLLs; resolve models from the exe dir, then cache, then download.",
   ["Release-dev build with the vulkan and directml features succeeds (Vulkan SDK installed per docs/dev/windows-build.md)",
    "Transcribing a fixture wav logs Vulkan; forcing a GPU failure via test env falls back to CPU and still returns text",
    "SCREENPIPE_DIRECTML=0 forces CPU for parakeet (log); unit test: model resolution prefers the exe-dir copy"],
   ["specs/audio.md audio-019, 020, 022", "specs/shell.md shell-029, shell-030 (merged)"])

wp("W2-20", 2, "L", "Connections core: secret-safe reads, webhook proxy, Telegram send, router deltas, MCP refs",
   ["connections-secret-redaction", "connections-webhook-proxy", "connections-telegram-send", "api-031",
    "connections-supports-oauth-instances", "connections-pipe-mcp-connection-refs", "cloud-038", "api-M03"],
   [E + "connections_api.rs (GET /:id and /:id/instances handlers)", E + "connections_routes/webhook_proxy.rs", E + "connections_routes/telegram.rs",
    CN + "connections/mod.rs (render_context, IntegrationDef flags)", CN + "connections/discord.rs", CN + "connections/zapier.rs",
    CN + "connections/make.rs", CN + "connections/teams.rs", CN + "connections/telegram.rs", CA + "skills/screenpipe-api/SKILL.md (Connections section)",
    E + "bin/screenpipe-engine.rs (pipe connection gate)", A + "components/post-install-connections-modal.tsx", A + "components/settings/pipes-section.tsx (connection refs only)"],
   ["W1-03"],
   "Connection reads return only non-secret fields (OAuth instances listed); render_context prints proxy/config/webhook/endpoint "
   "pointers instead of raw secrets. POST /connections/:id/proxy forwards JSON to stored https webhooks (discord, zapier, make, n8n, teams) "
   "with the typed error codes; POST /connections/telegram/send {text}. Expose supports_oauth_instances. Pipe connections may reference "
   "mcp:<server_id>, satisfied by an enabled MCP server (plus token when OAuth). Keep base Gmail routes (target dropped them for a vendor "
   "broker); never ship Composio URLs.",
   ["Harness: `api /connections/telegram` shows no bot_token field after connecting Telegram",
    "POST /connections/discord/proxy with a stored http:// webhook returns 400 invalid_webhook_target; ?x=1 returns unsupported_query",
    "POST /connections/telegram/send {text:''} returns 400; base /connections/gmail/* routes still respond",
    "cargo test: a pipe declaring connections [mcp:foo] is runnable when mcp server foo is enabled and stays in setup mode when it is disabled"],
   ["specs/connections.md secret-redaction, webhook-proxy, telegram-send, supports-oauth-instances, pipe-mcp-connection-refs", "specs/api.md api-031, api-M03", "specs/cloud.md cloud-038"])

wp("W2-21", 2, "L", "OAuth v2 flow and new OAuth integrations",
   ["connections-oauth-connect-v2", "connections-oauth-default-slot-promotion", "connections-oauth-scope-tweaks",
    "connections-hubspot-oauth", "connections-zendesk-oauth", "connections-outlook-email"],
   [CN + "oauth.rs (flow, variants, slots)", AR + "oauth.rs", CN + "connections/hubspot.rs", CN + "connections/zendesk.rs",
    CN + "connections/github_issues.rs", CN + "connections/microsoft365.rs", CN + "connections/teams.rs", CN + "connections/outlook_email.rs (new)",
    CN + "connections/mod.rs (registry lines only)"],
   ["W1-03"],
   "oauth_connect(integrationId, instance, variant) with named scope variants, {subdomain} templating, a 10-minute window overridable "
   "by SCREENPIPE_OAUTH_TIMEOUT_SECS, provider error handling and distinct expired/used/failed pages, and activate_app_after_oauth. "
   "Promote shadowed default-slot accounts with a distinct identity instead of deleting them. GitHub scope 'repo read:org', Microsoft "
   "prompt=select_account. HubSpot and Zendesk (per-subdomain) gain OAuth with token fallbacks; new outlook-email integration via Graph.",
   ["cargo test -p screenpipe-connect: unknown variant errors; timeout env respected; default-slot promotion to oauth:<id>:<identity>",
    "Harness: `api /connections` lists outlook-email and hubspot with an OAuth flag",
    "Manual with BYO credentials: connect Outlook mail and read the unread count via the test endpoint"],
   ["specs/connections.md oauth-connect-v2, default-slot-promotion, scope-tweaks, hubspot-oauth, zendesk-oauth, outlook-email"])

wp("W2-22", 2, "L", "Pipe config v2: new frontmatter, schedule_config and preview, ids, preamble",
   ["agents_pipes-003", "agents_pipes-004", "agents_pipes-m03", "agents_pipes-m05"],
   [C + "pipes/mod.rs (PipeConfig, PipeStatus, parse_schedule/should_run, preamble regions)", C + "pipes/schedule_config.rs (new)",
    E + "pipes_api.rs (config update validation, /pipes/schedule/preview)"],
   ["W1-03"],
   "Typed pipe.md fields effort, history, run_in, artifacts (with title), schedule_config, cloud_agent, trigger.sources, event_read_only; "
   "PipeStatus has_saved_context/is_bundled_builtin/next_run/execution_count; per-field config validation, config-envelope rejection, "
   "managed-pipe locks. Structured ScheduleConfig with POST /pipes/schedule/preview {summary, next[]}. Pipe ids must be one portable path "
   "component. Add the four preamble directives in our own words.",
   ["cargo test -p screenpipe-core pipes::: frontmatter with every new field round-trips; '../x' is rejected as an invalid pipe identifier",
    "Harness: POST /pipes/schedule/preview {frequency:'days',interval:1,at_hour:9,at_minute:0} returns a summary and the next run times",
    "POST /pipes/<id>/config with an unknown field returns 400"],
   ["specs/agents_pipes.md agents_pipes-003, 004, m03, m05"])

wp("W2-23", 2, "L", "Pi runtime 0.84.1, isolated config, baseline bun, compaction patch",
   ["agents_pipes-015"],
   [C + "agents/pi.rs (runtime install and seeding region)", AR + "pi.rs (runtime/env region)", AT + "assets/pi-patches/ (new)"],
   ["W1-03"],
   "Pin pi-coding-agent/pi-ai 0.84.1 with overrides, seed an isolated pi-config from the global one (with the migrated marker), keep the "
   "models.json merge rules, select a baseline bun on CPUs without AVX2, add the bash probe and a proactive compaction patch with "
   "summary validation (behaviour paraphrased, code original), PI_*/SCREENPIPE_PI_AGENT_DIR env.",
   ["Fresh data dir: the installed package.json pins 0.84.1 and pi-config contains the .migrated-from-global marker",
    "UI smoke: a chat turn with a local Ollama preset completes; forcing the non-AVX2 path selects the baseline bun (log)"],
   ["specs/agents_pipes.md agents_pipes-015"])

wp("W2-24", 2, "M", "Engine local stubs: attribution headers, telemetry no-op routes, local data-sync",
   ["api-012", "agents_pipes-038", "cloud-040", "api-026", "cloud-037"],
   [E + "routes/request_origin.rs", E + "routes/internal_telemetry.rs", E + "routes/data_sync_proxy.rs", AR + "data_sync.rs (new)"],
   ["W1-03"],
   "Accept X-Screenpipe-Client/X-Screenpipe-Agent for tracing only; POST /internal/telemetry/mcp-call and mcp-value return 204 and "
   "record nothing; /data-sync/devices and /data-sync/search answer from the local device and local search; set_activity_sync_enabled "
   "only stores the flag. No network.",
   ["Harness: `api /internal/telemetry/mcp-call -X POST -d '{}' -i` returns 204",
    "`api /data-sync/devices` returns the local device; `api '/data-sync/search?q=test&limit=5'` returns local results",
    "A request with X-Screenpipe-Client: claude-code shows the client in the trace log"],
   ["specs/api.md api-012, api-026", "specs/agents_pipes.md agents_pipes-038", "specs/cloud.md cloud-037, cloud-040"])

wp("W2-25", 2, "L", "Frontend stack refresh and browser-dev mock runtime",
   ["frontend-001", "frontend-004"],
   [A + "package.json", A + "bun.lock", A + "next.config.mjs", A + "app/layout.tsx", A + "app/globals.css", A + "components/rewind/mermaid-diagram.tsx",
    A + "public/vendor/mermaid.min.js (new)", A + "lib/dev/tauri-mock/** (new)"],
   ["W1-02"],
   "Next 15.5, React 19 (stable release preferred over the target's canary), @xyflow/react, Space Grotesk heading font as a real "
   "@font-face, mermaid as a static asset, an i18n runtime for W4-20; PostHog flag gates become local defaults. Optional dev-only "
   "Tauri mock (NEXT_PUBLIC_SCREENPIPE_WEB_DEV=mock|live) so pages render in a browser.",
   ["cd apps/screenpipe-app-tauri && bun install && bun run build && bun test && bunx tsc --noEmit all pass",
    "NEXT_PUBLIC_SCREENPIPE_WEB_DEV=mock bun run dev renders /home in a browser without Tauri",
    "UI smoke under `bun run tauri dev`: home, settings, chat and timeline render without console errors"],
   ["specs/frontend.md frontend-001, frontend-004"])

wp("W2-26", 2, "L", "App engine supervision: auto-respawn, port-conflict classification, safe relaunch",
   ["shell-012", "shell-013"],
   [AR + "health.rs", AR + "recording.rs (port conflict and restart region)", AR + "server.rs (port-kill region)", AR + "port_conflict.rs (new)",
    AR + "relaunch.rs (new)", AR + "main.rs (RunEvent::Exit region)"],
   ["W1-02"],
   "Respawn the embedded engine after consecutive unreachable checks while recording is intended (rate-limited). Classify API-port "
   "holders via netstat (self, healthy screenpipe-family, unhealthy, foreign) and only ever kill an unhealthy instance of our own "
   "executable; never a foreign or healthy process. One bounded teardown (capture stop, HTTP drain, audio shutdown, pool close, Pi stop) "
   "with force-exit; safe relaunch with SCREENPIPE_RELAUNCH_HOME_VISIBLE/SCREENPIPE_DATA_DIR; restart_after_screen_recording_permission.",
   ["cargo test (app crate) port_conflict: netstat fixtures classify self/healthy/unhealthy/foreign; foreign is never selected for kill",
    "UI smoke: kill the engine thread via a debug command; it is respawned within the configured checks",
    "Quit completes and the process exits within the teardown budget (manual timing)"],
   ["specs/shell.md shell-012, shell-013"])

wp("W2-27", 2, "L", "App startup hardening: argv helper modes, autostart, focus handoff, env, AVX2, WER, opener",
   ["shell-017", "shell-019", "shell-020", "shell-021", "shell-022", "shell-024", "shell-028"],
   [AR + "main.rs (early argv dispatch and startup region)", AR + "server.rs (/focus, Origin/Host checks)", AR + "commands.rs (autostart)",
    AR + "config.rs", AR + "cpu_compat.rs (new)", AR + "wer.rs (new)", "crates/screenpipe-wer-helper/ (new cdylib, cfg windows)",
    AT + "capabilities/main.json (opener schemes)"],
   ["W1-03"],
   "Early argv dispatch table for --screenpipe-acp-runtime, --screenpipe-acp-process-guard, --screenpipe-portable-mcp (and later "
   "--render-workflow-video) before single-instance work. /focus payload adds launch_exe/launchd_job_label and rejects other executables "
   "or schemes; duplicate --autostart handoffs ignored; :11435 rejects non-local Origin/Host. Autostart registers --autostart and starts "
   "in the background. SCREENPIPE_SKIP_ONBOARDING, runtime fs/asset scope for custom data dirs, AVX2 compatibility mode, WER "
   "runtime-exception helper with local crash-dumps harvest (never uploaded), opener allows claude://, cursor://, codex://.",
   ["Launching the built app with --autostart does not show the Home window (UI smoke)",
    "curl -X POST http://127.0.0.1:11435/focus with a different launch_exe returns 403; a request with Origin: http://evil.example returns 403",
    "App started with --screenpipe-portable-mcp exits through the dispatch path before single-instance (stub usage message)",
    "After a forced crash in a test build, a .dmp appears under <data dir>/crash-dumps on next launch"],
   ["specs/shell.md shell-017, 019..022, 024, 028"])

# ---------------------------------------------------------------- wave 3
wp("W3-01", 3, "L", "Semantic projection worker and actors API",
   ["meetings_memory-022", "meetings_memory-023", "api-008"],
   [E + "semantic_worker.rs (new)", E + "routes/semantic.rs"],
   ["W2-05", "W2-06"],
   "Worker parses frames when enableSemanticContext is on (default off; modes memory/computer_use/both), reuses runs by input "
   "fingerprint, stores items/run items, assigns actors and batches frame attachments; failures never affect capture. Six "
   "/semantic/actors/* routes (search, create, update, merge, reassign, aliases/reassign) returning item_count and aliases.",
   ["With enableSemanticContext=false no semantic_runs rows appear; with it on and a VS Code window focused, rows appear and frames.semantic_run_id is set",
    "Harness: `api '/semantic/actors/search?q=a'`; POST create, merge and reassign change rows as specified (sqlq)"],
   ["specs/meetings_memory.md meetings_memory-022, 023", "specs/api.md api-008"])

wp("W3-02", 3, "L", "Search v2: parsed content, starred, actor and related filters",
   ["api-003", "meetings_memory-011", "meetings_memory-024"],
   [E + "routes/search.rs", D + "db.rs (search query region)", D + "types.rs (ContentType Parsed/Memory, SearchQuery)", D + "ext/search_parsed.rs (new)"],
   ["W2-04", "W2-05", "W2-09"],
   "New params starred_session_id, starred_only, input_context_only, frame_id, actor_id, include_related, fields, format; "
   "content_type=parsed returns Parsed items (run/parser metadata, compact text, typed items, actors) and is excluded from all; hits "
   "carry starred; related map; starred filters apply before pagination; 30 s timeout and busy errors.",
   ["Harness: `api '/search?content_type=parsed&q=x'` returns Parsed items; content_type=all returns none of them",
    "starred_only=true returns only hits inside starred sessions and each hit has starred:true",
    "include_related=true adds the related map; actor_id filters parsed items"],
   ["specs/api.md api-003", "specs/meetings_memory.md meetings_memory-011, 024"])

wp("W3-03", 3, "L", "Element API formats and state",
   ["api-004", "vision-elements-formats"],
   [E + "routes/elements.rs", E + "elements_automation.rs (new)", D + "db.rs (elements query region)"],
   ["W2-09"],
   "ElementResponse.state {focused, selected, expanded}; format=outline (text/plain, x-total-count, no-store), format=automation "
   "(revision r_, response refs eN, stable keys k_ with confidence, bounds, allowed actions by role class), format=preferred following "
   "semanticContextMode; limit/offset/fields on /frames/:id/elements.",
   ["Harness: `api '/elements?format=outline&limit=20' -i` returns text/plain with x-total-count",
    "`api '/frames/<id>/elements?format=automation'` returns refs eN and keys k_; `?limit=10&offset=10` pages",
    "cargo test: automation key stability across two identical snapshots"],
   ["specs/api.md api-004", "specs/vision.md elements-formats"])

wp("W3-04", 3, "L", "Activity APIs: activity-summary v2 and activity-ledger route",
   ["api-005", "meetings_memory-018", "meetings_memory-015", "api-006"],
   [E + "routes/activity_summary.rs", E + "routes/activity_ledger.rs"],
   ["W2-04", "W2-07"],
   "/activity-summary adds include_apps/windows/key_texts/parsed_count/starred, starred_sessions (<= 10, has_more), parsed_context_count "
   "and app attribution from ui_events (native/recovered/unresolved). GET /activity-ledger start/end (<= 31 days), depth task|action, "
   "include_artifacts, refresh returns intervals with evidence.",
   ["Harness: `api '/activity-summary?start_time=today&include_apps=false'` omits apps and reports attribution counts",
    "`api '/activity-ledger?start_time=today&depth=task'` returns intervals; a 32-day range returns 400"],
   ["specs/api.md api-005, api-006", "specs/meetings_memory.md meetings_memory-015, 018"])

wp("W3-05", 3, "L", "Structured output targets",
   ["api-019", "meetings_memory-005", "agents_pipes-011"],
   [E + "routes/structured_outputs.rs", E + "structured_outputs/ (new: store, schema validator)", CA + "extensions/structured-output.ts (new)",
    C + "agents/extensions_manifest.rs (entry)"],
   ["W2-02", "W2-03"],
   "Typed targets in structured-outputs/targets.json bound to pipes: GET /outputs/targets?pipe=, POST create, POST /:target_id/submit "
   "(revision check, JSON-schema subset validation, evidence <= 50), /feedback (into the feedback table) and /items/:item_id/actions "
   "(list.v1 dispositions); values stored as structured-output SAF artifacts; structured_output pi tool.",
   ["Harness: create a target, submit with the current revision (200) and a stale one (409); a schema-violating payload returns 400; 51 evidence refs return 400",
    "POST /outputs/targets/<t>/items/<i>/actions {action:'resolve'} updates the disposition; feedback appears in GET /feedback"],
   ["specs/api.md api-019", "specs/meetings_memory.md meetings_memory-005", "specs/agents_pipes.md agents_pipes-011"])

wp("W3-06", 3, "L", "Event-trigger idempotency, run outcome verification, meeting-summary finalizer",
   ["db-009", "meetings_memory-028", "agents_pipes-001", "agents_pipes-002", "meetings_memory-027", "api-039", "agents_pipes-m01"],
   [C + "pipes/mod.rs (event dispatch and execution recording regions)", C + "pipes/hooks.rs", C + "pipes/meeting_summary/ (new: extract, finalizer)",
    E + "pipe_store.rs", D + "ext/pipe_events.rs", E + "routes/retranscribe.rs"],
   ["W2-08", "W2-22"],
   "Claim (pipe, event, key) in pipe_event_runs with INSERT OR IGNORE before event runs, release on cancel, prune by age; meeting_ended "
   "keyed by meeting id survives the rejoin window. pipe_executions records trigger_event/trigger_key/conversation_id with a guarded "
   "running transition and keyset paging. A clean exit without a verified final result fails; refusal and compaction failures are "
   "classified. The finalizer recovers an unsaved '## Summary' from output or marks summary_not_saved/nothing_to_summarize, requeues "
   "interrupted runs and skips trivial meetings. Retranscribe returns transcript_updated_at and summary_refresh_event_emitted.",
   ["cargo test -p screenpipe-core: two meeting_ended events for one meeting id run the pipe once; cancelling a queued run releases the claim",
    "sqlq: pipe_executions rows carry trigger_event and trigger_key",
    "Unit: finalizer extracts a '## Summary' block from fixture stdout; a run with no final result is marked failed",
    "Harness: POST /meetings/<id>/retranscribe response contains transcript_updated_at and summary_refresh_event_emitted"],
   ["specs/db.md db-009", "specs/meetings_memory.md 027, 028", "specs/agents_pipes.md 001, 002, m01", "specs/api.md api-039"])

wp("W3-07", 3, "M", "Live meeting overlay websocket",
   ["api-015", "meetings_memory-033"],
   [E + "routes/websocket.rs", E + "routes/meeting_overlay.rs"],
   ["W2-17", "W2-13"],
   "/ws/meeting-overlay emits {type: status|snapshot|item, data} for the active meeting with live transcript items "
   "{meetingId,itemId,deviceName,deviceType,speakerName,capturedAt,text,isFinal}; /ws/metrics adds audio.mic_capture_active. The overlay "
   "UI is W6-19.",
   ["websocat ws://127.0.0.1:3035/ws/meeting-overlay receives a status then a snapshot message",
    "During a manual meeting with live transcription, item messages arrive with isFinal flags; /ws/metrics payload has audio.mic_capture_active"],
   ["specs/api.md api-015", "specs/meetings_memory.md meetings_memory-033"])

wp("W3-08", 3, "L", "UI-scan meeting detector: calendar binding and lock awareness",
   ["meetings_memory-031", "gap_health-lifecycle-internals-013"],
   [E + "meeting_detector.rs", E + "meeting_watcher.rs", AR + "google_calendar.rs (binding region)", AR + "calendar.rs"],
   ["W2-08"],
   "Calendar matches bind only on exact conference URL identity through the conditional calendar_event_id update (time-only matches do "
   "not bind), with an ownership check; excludeFromMeetingDetection; meeting_ended carries transcript_segments. Meeting scan loops skip "
   "iterations while the screen is locked.",
   ["cargo test -p screenpipe-engine meeting_detector: exact-URL match binds, time-only match does not, excluded app is ignored",
    "Unit: with SCREEN_IS_LOCKED set the scan iteration is skipped without state change"],
   ["specs/meetings_memory.md meetings_memory-031", "specs/gap_health-lifecycle-internals.md 013"])

wp("W3-09", 3, "L", "Retention v2: lean mode, strip, NULL-safe deletes, coverage",
   ["gap_retention-v2-engine-001", "gap_retention-v2-engine-002", "gap_retention-v2-engine-003", "gap_retention-v2-engine-004",
    "gap_retention-v2-engine-005", "gap_retention-v2-engine-006", "gap_retention-v2-engine-007", "gap_retention-v2-engine-009",
    "gap_retention-v2-engine-010", "gap_retention-v2-engine-011"],
   [E + "retention.rs", D + "db.rs (evict/delete_time_range/orphan cleanup region)", D + "ext/retention_strip.rs", E + "routes/data.rs (delete-range response)",
    AR + "sync.rs (auto_start_retention region)", E + "cli/mod.rs (retention flags)", E + "bin/screenpipe-engine.rs (banner)"],
   ["W2-05", "W2-11"],
   "RetentionMode Media/Lean/All; lean batches evict media then strip heavy text in one transaction (delete elements with anchor "
   "re-homing, NULL accessibility_tree_json/text_json, delete ui_events; keep full_text and memories). Checked cutoff arithmetic, 1..36500 "
   "day validation, oldest-in-loop, shared unlink helper, post-loop policy. Fix the base NULL-unsafe chunk subqueries. delete-range drops "
   "ocr_deleted and keeps the newest display_layout row. Semantic GC after frame deletion. App auto-start accepts lean; fresh-store "
   "guard keeps retention off when recordings exist. CLI accepts lean.",
   ["Harness: POST /retention/configure {retention_days:36501} returns 400 with a message containing `at most 36500` (exact string, client-visible); {mode:'lean'} is echoed by /retention/status",
    "cargo test -p screenpipe-db: lean strip removes elements and ui_events, NULLs tree/text_json, keeps full_text; a chunk referenced with NULL ids is now evicted and marked",
    "POST /data/delete-range response has no ocr_deleted; display_layout keeps the newest row in range"],
   ["specs/gap_retention-v2-engine.md 001-007, 009-011"])

wp("W3-10", 3, "L", "PII redaction surfaces v2",
   ["db-003", "db-004", "vision-redaction-surfaces"],
   [RD + "worker/mod.rs", RD + "worker/tables.rs", RD + "redaction_map.rs (new)", "crates/screenpipe-redact/tests/**"],
   ["W1-01", "W1-02"],
   "Redaction worker covers elements text and properties, frames.full_text with propagation of detections to derived copies "
   "(accessibility tree, window_name, browser_url, text_json) through a redaction map, and multi-column ui_events; watermarks use the "
   "partial indexes; piiRedactionColumns selects surfaces.",
   ["cargo test -p screenpipe-redact: elements text redacted and redacted_at set; a full_text detection propagates to window_name/browser_url/text_json",
    "piiRedactionColumns subset is respected; EXPLAIN QUERY PLAN for the watermark scan uses the partial index"],
   ["specs/db.md db-003, db-004", "specs/vision.md redaction-surfaces"])

wp("W3-11", 3, "L", "Capture loop hardening A: stages and wedge watchdog, probe timeouts, fail-closed exclusions, corrupt frames",
   ["vision-capture-wedge-watchdog", "vision-probe-timeouts", "vision-exclusion-unknown-fail-closed", "vision-corrupt-frame-guard"],
   [E + "event_driven_capture.rs", E + "visual_probe.rs (new)", E + "capture_exclusions.rs (new)", E + "vision_manager/monitor_watcher.rs (wedge restart)",
    SC + "utils.rs"],
   ["W2-11", "W2-13"],
   "Loops publish named stages and heartbeats; the watcher detects still-attempting wedges and silent stalls and restarts the "
   "VisionManager (local log only, no analytics). Visual-change probe and excluded-window enumeration run under timeouts; when exclusion "
   "state is unknown, frame pixels are skipped until a refresh succeeds while a11y capture continues. Reject green decode-garbage bands "
   "and count corrupt black/green frames.",
   ["cargo test -p screenpipe-engine: a simulated hang in the capture stage triggers a VisionManager restart via a test hook",
    "Unit: exclusion refresh timeout leads to a11y-only frames (no snapshot written); green-band fixture is rejected and frames_corrupt_green increments",
    "Harness: `api /health` shows loop_stage and loop_stage_age_secs"],
   ["specs/vision.md capture-wedge-watchdog, probe-timeouts, exclusion-unknown-fail-closed, corrupt-frame-guard"])

wp("W3-12", 3, "M", "Dedicated Windows OCR worker and OCR availability reporting",
   ["vision-windows-ocr-worker"],
   [SC + "microsoft.rs", SC + "tesseract.rs", "crates/screenpipe-capture/src/paired_capture.rs (OCR call region)", E + "routes/frames.rs (on-demand OCR handler only)"],
   ["W1-01", "W2-10"],
   "Run WinRT OcrEngine on a long-lived 'screenpipe-windows-ocr' thread with a bounded queue shared by live and on-demand OCR; validate "
   "image dimensions and buffers; probe availability (recognizer languages, bundled tesseract) and surface vision_reason=ocr_unavailable.",
   ["Engine log names the screenpipe-windows-ocr thread; cargo test: invalid dimensions are rejected without panic",
    "With no OCR language installed (test override), `api /health` reports vision_reason ocr_unavailable"],
   ["specs/vision.md windows-ocr-worker"])

wp("W3-13", 3, "L", "URL domain rules, Windows window identity, input-hook watchdog",
   ["vision-url-domain-rules", "vision-windows-window-resolution", "a11y-input-hook-watchdog"],
   [AX + "url_filter.rs", AX + "platform/windows.rs", AX + "tree/windows.rs (identity region)", AX + "events.rs (SkipReason)",
    SC + "capture_screenshot_by_window.rs", E + "vision_manager/manager.rs (URL policy plumbing)"],
   ["W1-02", "W2-11"],
   "ignoredUrls accepts legacy strings or DomainRule{domain, includeSubdomains, excludedSubdomains}; includedUrls restricts browser "
   "capture (unverifiable URL is not recorded); block beats allow; validation messages. Resolve UWP apps behind ApplicationFrameHost; "
   "add skip reasons for a frame blocked by the browser URL policy and for a URL whose validation is still pending. Dedicated input worker with a soft-capped queue, HIGHEST hook "
   "thread priority and a watchdog that reinstalls silently removed low-level hooks.",
   ["cargo test -p screenpipe-a11y url_filter: subdomain include/exclude rules, allowlist rejection, block-beats-allow",
    "Manual: Windows Calculator frames carry the real app name rather than ApplicationFrameHost",
    "Unit: simulated hook silence with active input triggers reinstall (test hook)"],
   ["specs/vision.md url-domain-rules, windows-window-resolution, a11y-input-hook-watchdog"])

wp("W3-14", 3, "L", "Audio capture modes and device status events",
   ["audio-010", "audio-026", "audio-018"],
   [AU + "audio_manager/manager.rs", AU + "audio_manager/device_monitor.rs (session-device region)", E + "routes/audio.rs",
    "crates/screenpipe-events/src/custom_events/audio_devices.rs", AR + "engine_events/audio_device.rs", AR + "engine_events/mod.rs", E + "recording_config.rs (mode plumbing)"],
   ["W1-02", "W2-13"],
   "audioCaptureMode always | meetings-only (streams open only in confirmed meetings as session devices, released afterwards, "
   "unattributed chunks dropped, tray waiting state) | disabled (audio never starts; routes return `Audio capture is disabled in "
   "settings` (exact string, client-visible)); restart required. Publish audio_device_status_changed on every start/stop, plus fallback_unavailable and "
   "capture_health events, forwarded by the app as kebab-case events with native notifications.",
   ["Harness with audioCaptureMode=disabled: POST /audio/start returns the disabled error",
    "meetings-only: no audio chunks while idle; POST /meetings/start then speak; chunks appear and stop after /meetings/stop (sqlq)",
    "POST /audio/device/stop emits audio_device_status_changed on /ws/events"],
   ["specs/audio.md audio-010, 018, 026"])

wp("W3-15", 3, "L", "Software AEC3 + NS + AGC2 (aecMode=screenpipe)",
   ["audio-012"],
   [AU + "core/aec.rs (new)", AU + "core/run_record_and_transcribe.rs", AU + "core/stream.rs (far-end tap region)", AU + "audio_manager/builder.rs",
    "crates/screenpipe-audio/Cargo.toml (AEC dependency)"],
   ["W2-18"],
   "Clean mic audio at 16 kHz with the system-output stream as far-end reference, with drift handling, bypass and diagnostics; aecMode "
   "off|screenpipe|macos|windows is validated per platform and drives the three legacy toggles. Use a licence-cleared AEC3 port.",
   ["cargo test -p screenpipe-audio aec: synthetic echo is attenuated by at least 20 dB; no far-end means bypass",
    "Settings validation rejects aecMode=macos on Windows"],
   ["specs/audio.md audio-012"])

wp("W3-16", 3, "L", "Reconciliation durability, identification gating, STT retries, dedup",
   ["audio-007", "audio-023", "audio-024", "audio-M04"],
   [AU + "audio_manager/reconciliation.rs", AU + "transcription/transcription_result.rs", AU + "speaker/prepare_segments.rs",
    AU + "transcription/openai_compatible/batch.rs", AU + "transcription/deepgram/batch.rs",
    D + "db.rs (get_speaker_from_embedding, has_similar_recent_transcription region)"],
   ["W2-17"],
   "Per-segment skip/match_only/match_and_learn by duration, top-2 lookup with ambiguity abstain, 400-sample minimum. Pending-chunk "
   "marker journal with startup recovery, PendingTranscription v2 with sha256-staged merged audio, refusal without a durable data dir. "
   "OpenAI-compatible 3 attempts with 429/transport backoff; Deepgram retries 429 with Retry-After then leaves the chunk pending. Dedup "
   "anchored to capture time, excludes the same device and fails open.",
   ["cargo test -p screenpipe-audio: abstain when best and runner-up are within margin; pending marker replays after a simulated restart",
    "wiremock: a 429 with Retry-After is retried and then succeeds",
    "cargo test -p screenpipe-db: dedup ignores the same device and uses the capture-time window"],
   ["specs/audio.md audio-007, 023, 024, M04"])

wp("W3-17", 3, "L", "Startup health diagnostics (health-startup.json) and ORT init watchdog",
   ["shell-M01", "audio-M06", "audio-021"],
   [C + "health_diagnostics.rs (new)", AU + "speaker/mod.rs", AU + "segmentation/segmentation_manager.rs", AU + "vad/silero.rs (init region)",
    AU + "ort_watchdog.rs (new)", E + "bin/screenpipe-engine.rs (phase markers)"],
   ["W2-13", "W2-19"],
   "Write health-startup.json: phases, preflight checks (data_dir_writable, path_has_ffmpeg, env_overrides) with remedies, "
   "last_failed_start, ffmpeg/ffprobe probes, per-component states, degraded features, crash-dump helper status and a process snapshot. "
   "ORT and model sessions initialise on a watchdog thread with ok/init_failed/init_timed_out.",
   ["After engine start, <FP_DD>/health-startup.json exists with components transcription, vad, speaker_segmentation, speaker_embedding",
    "A corrupt VAD model fixture yields vad: init_failed; a test hook that hangs init yields init_timed_out"],
   ["specs/shell.md shell-M01", "specs/audio.md audio-021, M06"])

wp("W3-18", 3, "L", "Notifications v2 backend on :11435",
   ["api-037", "meetings_memory-037", "meetings_memory-038"],
   [AR + "notifications/routes.rs", AR + "notifications/store.rs", AR + "notifications/rewrite.rs", AR + "notifications/mod.rs",
    AR + "server.rs (route table region)"],
   ["W1-02"],
   "NotifyPayload v2 (priority, transient, pipe_name, workflow_ids, source ids/url, surface/position/anchor, cta, survey, dismissible, "
   "expires_at; action types link/deeplink/pipe/chat/api/dismiss/copy/source/mute/manage); delivery gates (master, snooze, quiet hours, "
   "allowDuringPause, categories, mutedPipes, pipe notify permission, duplicate suppression); x-screenpipe-session; history entries keep "
   "source, actions and priority; POST /notifications/:id/read.",
   ["curl -X POST http://127.0.0.1:11435/notify with priority, transient and a chat action returns 200 and GET /notifications shows the new fields",
    "POST /notifications/<id>/read marks it read; an identical notify within the dedupe window is suppressed",
    "cargo test (app crate) notifications: quiet hours and snooze suppress delivery"],
   ["specs/api.md api-037", "specs/meetings_memory.md meetings_memory-037, 038"])

wp("W3-19", 3, "L", "Slack user-token integration with read routes",
   ["connections-slack-user-token"],
   [CN + "connections/slack.rs", E + "connections_routes/slack.rs", A + "components/settings/connections-section.tsx (Slack panel region)"],
   ["W2-21"],
   "Slack acts as the user (user_scope): variant send = chat:write, read_write adds search/history/read scopes; /slack/send defaults to "
   "self-DM and accepts channel/blocks; GET /slack/search, /slack/conversations, /slack/history; instance = team_id; read routes return 403 "
   "without read_write.",
   ["cargo test -p screenpipe-connect slack: the read_write variant requests the read scopes",
    "wiremock: GET /connections/slack/search?q=x proxies search.messages; a send-only instance gets 403 on read routes"],
   ["specs/connections.md slack-user-token"])

wp("W3-20", 3, "L", "IMAP read-only inbox integration",
   ["connections-imap"],
   [CN + "connections/imap.rs (new)", "crates/screenpipe-connect/Cargo.toml (async-imap, mail-parser)", E + "connections_routes/imap.rs",
    CN + "connections/mod.rs (registry line)"],
   ["W2-20"],
   "New imap integration (host, port 993, username, app password) over rustls using EXAMINE only; GET /imap/messages?limit&mailbox&query, "
   "/imap/messages/:uid (parsed), /imap/mailboxes; app-password guidance on login failure.",
   ["cargo test -p screenpipe-connect imap: message parsing and query building",
    "Manual with an app password: `api /connections/imap/mailboxes` returns mailboxes and `api '/connections/imap/messages?limit=5'` returns messages without marking them read"],
   ["specs/connections.md imap"])

wp("W3-21", 3, "L", "BYO MCP: registry search, MCP-OAuth-first tiles, session allow-gate, relay-free OAuth",
   ["connections-mcp-oauth-tiles", "api-032", "api-M01", "agents_pipes-032", "api-M02"],
   [E + "mcp_servers_api.rs", CN + "mcp_servers.rs", E + "mcp_access.rs (new)", A + "components/settings/connections-section.tsx (MCP tiles region)",
    A + "components/settings/custom-mcp-card.tsx"],
   ["W2-20"],
   "GET /mcp-servers/registry proxies registry.modelcontextprotocol.io (user-initiated). Vendor MCP servers become the primary connect "
   "path for linear/notion/jira/stripe/... tiles with native forms under Advanced; connected = native or matching enabled MCP server; "
   "/connections marks mcp/mcp_server_id. OAuth start accepts and ignores app_scheme and always uses the loopback callback. tools/call is "
   "refused (403 naming the server) when the server is not allowed for the calling pipe session (x-screenpipe-session, "
   "SCREENPIPE_MCP_SERVER_ALLOWLIST).",
   ["Harness: `api '/mcp-servers/registry?search=github&limit=5'` returns {data:{servers,nextCursor}}",
    "POST /mcp-servers/<id>/oauth/start {app_scheme:'screenpipe'} returns an authorize URL whose redirect_uri is the localhost callback",
    "A tools/call carrying a pipe session that does not allow the server returns 403 naming it"],
   ["specs/connections.md mcp-oauth-tiles", "specs/api.md api-032, M01, M02", "specs/agents_pipes.md agents_pipes-032"])

wp("W3-22", 3, "M", "Local integrations: memory-sync guidance, Obsidian memories, Hermes, WhatsApp sign-out",
   ["meetings_memory-035", "connections-hermes", "connections-whatsapp-signout"],
   [C + "memories/external_sync.rs", E + "external_memory_sync.rs", CN + "connections/obsidian_memories.rs (new)", CN + "connections/hermes.rs (new)",
    CN + "connections/claude_code.rs", CN + "connections/codex.rs", CN + "whatsapp/mod.rs", CN + "whatsapp/gateway.mjs",
    CN + "connections/mod.rs (registry lines)"],
   ["W2-20"],
   "Claude Code and Codex receive static MCP recall guidance instead of raw memories (disconnect restores on failed cleanup); new "
   "obsidian-memories target writes a digest note with frontmatter after a .obsidian check and write probe; new hermes integration "
   "(endpoint default 127.0.0.1:8642, optional token, /v1/models test); WhatsApp logged-out disconnects wipe the session and ask to re-pair.",
   ["Harness: POST /memories/sync-external with obsidian-memories configured writes <vault>/screenpipe/screenpipe-memories.md with frontmatter",
    "The Claude Code sidecar file contains recall guidance and no memory text",
    "cargo test: hermes test call hits {endpoint}/v1/models (mock); WhatsApp logged_out path resets the session"],
   ["specs/meetings_memory.md meetings_memory-035", "specs/connections.md hermes, whatsapp-signout"])

wp("W3-23", 3, "L", "Pi extension set expansion and extension packages",
   ["agents_pipes-014", "frontend-016"],
   [CA + "extensions/** (extensions not owned by other WPs)", AT + "assets/extensions/**", AR + "pi_extension_packages.rs (new)",
    A + "components/settings/skills-card.tsx (packages UI)", C + "agents/extensions_manifest.rs", AR + "pi_extensions_manifest.rs"],
   ["W2-23"],
   "Rename the agent tools to sp_* (with aliases), add the remaining extensions (fill_work_context, sp_mcp_read, workflow workspace "
   "scaffolding hooks) and pi_*_extension_package commands for npm:/GitHub sources with a safe mode; pi-subagents mandatory; answer "
   "extension UI requests via pi_extension_ui_response.",
   ["bun test: manifest lists the expected extensions and tool ids are sp_*",
    "UI smoke: list, install (from a local tarball or npm, user-initiated) and remove an extension package; safe mode disables it"],
   ["specs/agents_pipes.md agents_pipes-014", "specs/frontend.md frontend-016"])

wp("W3-24", 3, "L", "Agent self-improvement: skills store, user profile, learning",
   ["agents_pipes-021", "agents_pipes-022", "agents_pipes-023", "api-021"],
   [E + "agent_skills.rs (new)", E + "agent_profile.rs (new)", E + "routes/agent.rs", CA + "extensions/self-improvement.ts (new)",
    CA + "extensions/skill-learning.ts (new)", CA + "pipes/skill-learning/pipe.md (new, own text)"],
   ["W1-03"],
   "POST /agent/skills/manage (list/read/create/patch with sha256 optimistic patching, user/agent/bundled provenance manifest, "
   "unsafe-pattern scan, create requires confirmed), POST /agent/profile/manage (list/save/delete user-profile facts with threat scan), "
   "GET /agent/self-improvement/context {system_prompt} with a frozen profile snapshot, POST /agent/learning/chats {query}, and the opt-in "
   "skill-learning task.",
   ["Harness: create without confirmed is refused; with confirmed the manifest .screenpipe-agent-skills.json gains the skill; patch with a wrong sha256 returns 409",
    "POST /agent/profile/manage save/list/delete round-trips; GET /agent/self-improvement/context returns system_prompt"],
   ["specs/agents_pipes.md agents_pipes-021..023", "specs/api.md api-021"])

wp("W3-25", 3, "L", "Local OpenAI-compatible proxy and BYO web search",
   ["agents_pipes-033", "api-023", "api-025", "agents_pipes-035"],
   [E + "local_chat.rs (new)", E + "routes/local_chat.rs", E + "routes/web_search.rs", CA + "extensions/web-search.ts", AT + "assets/extensions/web-search.ts"],
   ["W1-03"],
   "POST /v1/local/chat/completions forwards (streaming) to the user's default AI preset without exposing keys, with clear errors for a "
   "missing preset/base URL or unreachable provider. POST /v1/web-search {query} -> {content, sources[]} backed by a user-configured "
   "provider (SearXNG or API key), disabled with a clear error when unconfigured; sp_web_search returns {text, sources, query}.",
   ["Harness with a local mock OpenAI server as default preset: `api /v1/local/chat/completions -X POST -d '{\"messages\":[...],\"stream\":true}'` streams SSE",
    "No default preset returns 400 with the missing-preset message; POST /v1/web-search unconfigured returns the disabled error, configured against a mock SearXNG returns sources"],
   ["specs/agents_pipes.md agents_pipes-033, 035", "specs/api.md api-023, 025"])

wp("W3-26", 3, "M", "Connect broker and connection-gate tools",
   ["agents_pipes-043", "api-022"],
   [E + "routes/connect_broker.rs", CA + "extensions/connection-gate.ts (new)", AT + "assets/extensions/connection-gate.ts (new)",
    A + "components/chat/connect-card.tsx (new)"],
   ["W2-20"],
   "POST /v1/connect-request blocks until the UI answers via POST /v1/connect-response (event acp_connect_request) and returns "
   "connected|declined|timeout; tools screenpipe_list_connections and screenpipe_connect_app with an inline connect card (app extension "
   "uses the UI confirm, core/ACP copy uses the broker).",
   ["Harness: a connect-request answered by a connect-response {status:'connected'} returns connected; unanswered returns timeout",
    "UI smoke: asking the agent to use an unconnected app shows the connect card"],
   ["specs/agents_pipes.md agents_pipes-043", "specs/api.md api-022"])

wp("W3-27", 3, "L", "SQLite hard-fault quarantine coordinator",
   ["db-019"],
   ["crates/screenpipe-sqlite-coordinator/ (new crate)", D + "write_queue.rs (admission gate)", D + "db.rs (fault classification hook)", D + "sqlite_error.rs"],
   ["W2-01"],
   "On corrupt/notadb/ioerr/full, close writer admission for the database generation and write <db>.quarantine.json "
   "(SqliteQuarantineMarker fields) with reserve/verification-pending sidecars; fresh-process verification confirms or clears; "
   "recording-recovery.log. The app UX is W5-08.",
   ["cargo test: an injected SQLITE_CORRUPT on write closes admission and writes the marker with schema_version, file_identity and sqlite_code",
    "A verification run on a healthy copy clears the marker; recording-recovery.log receives entries"],
   ["specs/db.md db-019"])

wp("W3-28", 3, "L", "Headless mode and search-only mode",
   ["shell-004", "shell-005", "api-034"],
   [AR + "window/show.rs (suppression)", AR + "shortcuts.rs (unregister/restore)", AR + "tray.rs (Open wakes UI)", AR + "main.rs (quit path)",
    AR + "search_only.rs", E + "search_only.rs", E + "server.rs (gate layer via seam)"],
   ["W2-26"],
   "headless/headlessRecordOnly destroy webviews (keeping incomplete onboarding), unregister shortcuts and suppress shows until tray Open "
   "wakes the UI. keepSearchAvailableAfterQuit (default true): Quit stops capture and closes UI but keeps the read API; live-capture "
   "routes return 503 search_only with a message explaining that only saved history is being served; failure falls back to full exit. The gated route set is "
   "inferred (open question).",
   ["UI smoke: headless=true leaves no WebView2 child processes; tray Open restores windows and shortcuts",
    "Quit with keepSearchAvailableAfterQuit: `api '/search?limit=1'` still returns 200 and POST /audio/start returns 503 with code search_only"],
   ["specs/shell.md shell-004, 005", "specs/api.md api-034"])

wp("W3-29", 3, "M", "Recording health incident state",
   ["shell-011"],
   [AR + "overlay_health.rs (new)", AR + "health.rs (state emit region)"],
   ["W2-26"],
   "State machine emitting recording-health-state as 'state|detail|kind|extra' (normal, failure, recovering, fixing, recovered, "
   "manual-reopen) with commands overlay_restart_recording, overlay_dismiss_incident, get_recording_health_state, retry_screenpipe, "
   "is_capture_paused; BootPhaseSnapshot gains cpuCompatMode. The overlay UI is W6-19.",
   ["UI smoke: stopping capture produces a failure state readable via get_recording_health_state; overlay_restart_recording recovers it",
    "cargo test (app crate): state transitions for failure -> recovering -> recovered"],
   ["specs/shell.md shell-011"])

wp("W3-30", 3, "M", "Low-disk recording guard and keep-awake",
   ["shell-006", "shell-007"],
   [E + "disk_pressure.rs (new)", E + "power/keep_awake.rs (new)", AR + "disk_pressure.rs (new)", AR + "power_awake.rs (new)"],
   ["W1-02", "W1-03"],
   "Engine monitor publishes disk_space_low/disk_space_recovered with hysteresis; the app stops capture when stopRecordingOnLowDisk, "
   "notifies with a settings?section=storage deep link, persists disk-pressure-notification-state.json; get_low_disk_guard_config "
   "returns {thresholdBytes, checkIntervalSeconds}. keepComputerAwake and set_keep_awake hold SetThreadExecutionState on a dedicated thread.",
   ["With a test threshold above free space, disk_space_low fires, capture stops and a notification appears; lowering it resumes capture",
    "set_keep_awake(true) then `powercfg /requests` lists a SYSTEM request from the app; false clears it"],
   ["specs/shell.md shell-006, 007"])

wp("W3-31", 3, "L", "SOP narration providers: OpenAI-compatible, local OS TTS, captions-only",
   ["gap_workflow-sop-video-and-media-narration-decloud"],
   [C + "workflow_speech.rs (new)", C + "workflow_speech/openai.rs (new)", C + "workflow_speech/sapi_windows.rs (new)", C + "workflow_speech/captions_only.rs (new)"],
   ["W1-02"],
   "SpeechProvider trait replacing the hosted /tts: OpenAI-compatible POST {base}/audio/speech (chunked, size-capped, retries), local "
   "WinRT/SAPI voice, and a default captions-only provider emitting estimated-duration silence. Settings key workflowVideoNarration; API "
   "key in the secret store; no account or plan gate.",
   ["cargo test -p screenpipe-core workflow_speech: captions-only returns silence of the estimated duration; OpenAI provider against a mock returns PCM",
    "Manual (Windows): the SAPI provider synthesises a sentence to a wav"],
   ["specs/gap_workflow-sop-video-and-media.md narration-decloud"])

# ---------------------------------------------------------------- wave 4
wp("W4-01", 4, "XL", "Workflows core and /workflows API",
   ["agents_pipes-026", "api-028", "agents_pipes-027"],
   [C + "workflows/** (new)", E + "routes/workflows.rs", E + "routes/workflow_catalog.rs (new)", E + "routes/workflow_pipeline.rs (new)",
    E + "routes/workflow_workspace.rs (new)", E + "routes/workflow_edits.rs (new)"],
   ["W2-22", "W3-23", "W2-02"],
   "Workflow record contract, revisioned file-backed catalog.json/pipeline.json/profile.json with backups and locks, multi-agent "
   "workspace (start/propose/checkpoint/handoff/reject/publish/finish with revisions), enrichment pipeline stages, edits and "
   "corrections, list/get with automation evidence and context. Gating is a local toggle and optional local budget only.",
   ["Harness: `api /workflows` returns []; POST /workflows/catalog creates revision 1; a stale revision returns 409",
    "A workspace start -> propose -> checkpoint -> publish sequence via curl updates catalog.json; POST /workflows/corrections is recorded",
    "grep shows no workflow_business_required / hosted_ai/plan checks in the code path"],
   ["specs/agents_pipes.md agents_pipes-026, 027", "specs/api.md api-028"])

wp("W4-02", 4, "L", "Pipe scheduler v2: defer/coalesce, topics, preset '*', trajectories, log archive",
   ["agents_pipes-007", "agents_pipes-042", "agents_pipes-m06"],
   [C + "pipes/mod.rs (start_scheduler region)", C + "pipes/preset_fallback.rs", C + "pipes/trajectory.rs (new)", C + "pipes/log_archive.rs (new)"],
   ["W3-06"],
   "Consume connection_trigger, workflow_ready:<pipe> and meeting_summary_refresh_requested; defer busy events with a backlog cap and "
   "coalesce; write .trigger-context.json; skip trivial meetings; '*' preset expands to user presets and a missing named preset fails "
   "without provider fallback; persist skipped runs. Optional JSONL trajectories (SCREENPIPE_TRAJECTORIES_DIR/MAX_RUNS) and a periodic "
   "pipe-log archive sweep.",
   ["cargo test -p screenpipe-core scheduler: busy pipe defers then runs once (coalesced); backlog cap enforced; '*' expands; missing named preset fails",
    "SCREENPIPE_TRAJECTORIES_DIR set: a run writes a JSONL file; the archive sweep reports scanned/archived counts in the log"],
   ["specs/agents_pipes.md agents_pipes-007, 042, m06"])

wp("W4-03", 4, "XL", "ACP agent runtime and screenpipe-tools MCP",
   ["agents_pipes-016", "agents_pipes-039"],
   [C + "agents/acp/** (new)", AR + "pi_acp.rs (new)", "crates/screenpipe-tools-mcp/ (new)", AR + "main.rs (argv table entries only)"],
   ["W2-23", "W2-27"],
   "Agent catalog (pi, codex, claude, cursor, copilot, custom command) with pinned launches, a --screenpipe-acp-runtime child speaking "
   "agent-client-protocol, the SCREENPIPE_ACP_* env contract, session config/modes/approval ask|allow-all, install/login commands, ACP "
   "events, a dependency-free screenpipe-tools MCP server (stdio or HTTP) and ACP-backed pipes. Feature flags become local settings; the "
   "vendor /anthropic cloud routing is not offered.",
   ["Handvel.exe --screenpipe-acp-runtime starts and answers an ACP initialize over stdio (test harness)",
    "screenpipe-tools MCP lists its tools over stdio; an ACP session with the pi agent answers a prompt (UI smoke)",
    "Approval mode ask raises a permission request event"],
   ["specs/agents_pipes.md agents_pipes-016, 039"])

wp("W4-04", 4, "L", "Cloud coding agents (BYO Cursor/Codex/Claude)",
   ["agents_pipes-028", "api-027"],
   [C + "agents/cloud.rs (new)", C + "agents/cloud_context.rs (new)", E + "routes/cloud_agents.rs"],
   ["W2-22"],
   "Pipes with agent: cloud-agent and CloudAgentConfig; /cloud-agents/status, /:provider/connect, /:provider/codebases, /cursor-agents, "
   "PUT /cursor-key (secret cloud_agents.cursor.api_key, never returned); opt-in bounded context capsule; snapshot commits use a Handvel "
   "identity. Third-party, user-initiated only.",
   ["Harness: PUT /cloud-agents/cursor-key {key:'x'} stores it; GET /cloud-agents/status shows connected without the key",
    "wiremock api.cursor.com: a cloud-agent pipe run launches an agent with the bounded context capsule"],
   ["specs/agents_pipes.md agents_pipes-028", "specs/api.md api-027"])

wp("W4-05", 4, "M", "Chat runtime commands and per-turn connections context",
   ["agents_pipes-018", "connections-chat-context-per-turn"],
   [AR + "pi.rs (pool/command region)", AR + "pi_command_queue.rs", A + "lib/chat/strip-connections-context.ts (new)",
    A + "components/standalone-chat.tsx (one render call)"],
   ["W2-23"],
   "Commands pi_start_and_prompt, pi_stop_if_idle, pi_request_state, thinking level, pi_extension_ui_response, list_chat_entries_by_mtime; "
   "pool start stages and busy refusal; agent_event types thinking_level_changed/extension_ui_request/compaction_end. Each user turn is "
   "prefixed with a fresh <connections_context> block (redacted render_context) that the transcript view strips.",
   ["bun run bindings:check; tauri.ts exposes the new commands",
    "bun test: the strip helper removes the block; chat log shows the block on each turn (UI smoke)"],
   ["specs/agents_pipes.md agents_pipes-018", "specs/connections.md chat-context-per-turn"])

wp("W4-06", 4, "M", "Bundled skills update and starter skills",
   ["agents_pipes-024"],
   [CA + "skills/** (new skills, own text)", C + "agents/pi.rs (skill install region)"],
   ["W2-23", "W3-23"],
   "Add screenpipe-chats, screenpipe-workflow-maintenance and video-sop skills and 20 starter skills written in our own words with "
   ".screenpipe-starter-* markers; never overwrite user-edited starter skills.",
   ["Fresh data dir installs the three skills and 20 starter skills with markers",
    "cargo test: a user-modified starter skill is left untouched on upgrade"],
   ["specs/agents_pipes.md agents_pipes-024"])

wp("W4-07", 4, "L", "CLI v2: service, diagnose, richer status, agent setup",
   ["shell-026", "agents_pipes-031"],
   [E + "cli/mod.rs (subcommand enum region)", E + "cli/service.rs (new)", E + "cli/diagnose.rs (new)", E + "cli/status.rs", E + "cli/agent.rs (new)",
    AR + "ai_tools.rs (new)", A + "components/onboarding/connect-apps.tsx (use agent setup)"],
   ["W3-17"],
   "service install [--mode recorder|server]/uninstall/status (Windows: per-user scheduled task or Run key), diagnose [--dry-run] "
   "(local bundle only), status verdicts and JSON (health.*, last_audio_capture, storage_size_bytes, database_path), and agent setup "
   "--target {openclaw, hermes, claude-code, claude-desktop, codex, cursor, gemini, runner, windsurf, vscode} writing an mcpServers "
   "entry plus skills with backups and locks, pointing at the bundled binary.",
   ["screenpipe service install --mode recorder then service status reports installed; uninstall removes it",
    "screenpipe diagnose --dry-run lists bundle contents and writes nothing; screenpipe status --json has health, last_audio_capture, storage_size_bytes, database_path",
    "screenpipe agent setup --target claude-desktop writes mcpServers.screenpipe and a backup of the previous config"],
   ["specs/shell.md shell-026", "specs/agents_pipes.md agents_pipes-031"])

wp("W4-08", 4, "M", "Pipe permissions v2",
   ["api-038", "agents_pipes-013", "agents_pipes-m04", "meetings_memory-045"],
   [E + "pipe_permissions_middleware.rs", C + "pipes/permissions.rs", CA + "extensions/screenpipe-permissions.ts"],
   ["W3-02", "W3-21"],
   "Separate workflow-operation concurrency lane for /workflows/workspace and /workflows/catalog; explicit content_type, server-side "
   "app/window 403s and include_related refusal on /search for restricted pipes; reader allowlist adds starred-sessions, workflows, "
   "feedback and the structured-output set; Content() accepts memory and parsed. MCP session filtering itself is W3-21.",
   ["cargo test: a restricted pipe calling /search without content_type gets 403; a denied app returns 403 server-side; include_related is refused",
    "A pipe with Content(parsed) can query content_type=parsed; GET /starred-sessions is allowed by default"],
   ["specs/api.md api-038", "specs/agents_pipes.md agents_pipes-013, m04", "specs/meetings_memory.md meetings_memory-045"])

wp("W4-09", 4, "L", "Database recovery v2: embedded page-level recovery",
   ["db-020"],
   [E + "cli/db.rs", "crates/screenpipe-sqlite-recovery/ (new crate)", D + "recovery.rs (new)"],
   ["W3-27"],
   "Replace the sqlite3 .recover shell-out with in-process recovery into db-recovery-<ts>/ (working copy, candidate.sqlite, "
   "recovery-manifest.json), then FTS rebuild, DELETE journal, fresh identity, integrity and foreign_key checks, write-canary readback and "
   "a manifest phase machine; resolved-quarantine.json; lock payload gains host_id.",
   ["screenpipe db recover --data-dir <copy of a corrupted fixture> produces candidate.sqlite and a manifest in phase complete",
    "The canary readback check passes; the lock file contains host_id; recovery refuses while the API port is live"],
   ["specs/db.md db-020"])

wp("W4-10", 4, "L", "Data maintenance, vault and secrets DB split",
   ["db-025", "db-026", "api-030", "meetings_memory-044"],
   [E + "routes/data.rs (compact route)", E + "routes/vault.rs", E + "sync_api.rs (reset-account)", E + "archive.rs (download)",
    "crates/screenpipe-secrets/src/store.rs", "crates/screenpipe-vault/src/manager.rs", AR + "disk_usage.rs", E + "auth_key.rs", E + "cli/connection.rs"],
   ["W3-27"],
   "POST /data/compact (free-space precheck, busy_timeout dance, VACUUM, refused for hybrid). Secrets move to secrets.sqlite with "
   "credential_refresh_leases and secret_store_metadata, a one-time legacy copy gated by legacy_main_db_migrated_v1 (newer updated_at "
   "wins), quarantine-aware open; all construction sites switch pools. Vault covers secrets.sqlite(+journal), refuses online lock with "
   "409 and refuses hybrid setup. /sync/reset-account and /archive/download(+status) return 501.",
   ["Harness: POST /data/compact returns bytes_before/bytes_after/bytes_reclaimed",
    "<FP_DD>/secrets.sqlite exists and legacy secrets were copied once (sqlq on secret_store_metadata); lease acquire/release cargo test",
    "POST /vault/lock while the server runs returns 409; POST /sync/reset-account returns 501"],
   ["specs/db.md db-025, db-026", "specs/api.md api-030", "specs/meetings_memory.md meetings_memory-044", "specs/connections.md secret-db-split, credential-refresh-leases, vault-secrets-db (merged)"])

wp("W4-11", 4, "L", "Live Views backend",
   ["api-020"],
   [E + "live_views.rs (new)", E + "routes/live_views.rs", CA + "live-views/ (new: bundled kits, own content)"],
   ["W3-05"],
   "GET /live-views, /live-views/schema, /live-views/catalog, /live-views/:id/template; PUT (and DELETE) /live-views/:id with "
   "expectedRevision; live-view-template.v1 and live-view-kit.v1 schemas; templates.json store; bundled kits written by us.",
   ["Harness: PUT /live-views/v1 with expectedRevision 0 creates it; a stale expectedRevision returns 409",
    "`api /live-views/schema` returns both schema ids; `api /live-views/catalog` lists the bundled kits"],
   ["specs/api.md api-020"])

wp("W4-12", 4, "XL", "Audio-process (mic-holder) meeting detector",
   ["meetings_memory-030"],
   [E + "meeting_watcher/audio_process/** (new)", E + "meeting_watcher.rs (integration)", AU + "meeting_processes.rs (new, WASAPI session snapshot)"],
   ["W3-08", "W2-18"],
   "Enumerate capture/render audio sessions (IAudioSessionManager2) to find the mic-holding process, confirm via recent frames, an "
   "active-tab URL probe and a bounded UIA call-control scan; start/reopen/merge/end rules (single-open, manual precedence, 120 s "
   "rejoin, room/calendar guards), orphan close, auto-end, shutdown end, RDP/VDI warning. Reuse the base lifecycle primitives.",
   ["cargo test: session snapshot classification with a mocked enumerator; rejoin within 120 s reopens the same meeting",
    "Manual: a Teams or browser call holding the mic starts a meeting; releasing the mic ends it; a manual meeting takes precedence"],
   ["specs/meetings_memory.md meetings_memory-030", "specs/audio.md audio-015 (merged)"])

wp("W4-13", 4, "XL", "Event-driven retained UIA tree on Windows",
   ["vision-uia-retained-capture"],
   [AX + "tree/windows.rs", AX + "tree/mod.rs", AX + "tree/retained_tree.rs (new)", AX + "tree/capture_diagnostics.rs (new)",
    AX + "platform/windows_uia.rs", AX + "budget.rs"],
   ["W3-13"],
   "Keep a retained baseline tree for the focused window, update it from UIA structure/property/text/layout events within a time slice, "
   "resync on identity mismatch or overflow, periodic resync for weak providers, drop state under privacy rules, fall back to pixel_only "
   "with diagnostics, skip fragile providers; TruncationReason gains Pending.",
   ["cargo test -p screenpipe-a11y retained_tree: event application, overflow resync and identity-mismatch resync",
    "Manual: typing in Notepad updates the retained tree without a full re-walk (debug counter)"],
   ["specs/vision.md uia-retained-capture"])

wp("W4-14", 4, "L", "Capture loop B: tree-walker worker and foreground identity gate",
   ["vision-tree-walker-worker", "vision-capture-identity-gate"],
   ["crates/screenpipe-capture/src/lib.rs", "crates/screenpipe-capture/src/tree_walker_worker.rs (new)",
    "crates/screenpipe-capture/src/paired_capture.rs (walker call region)", E + "event_driven_capture.rs"],
   ["W3-11"],
   "One walker thread per capture loop with deadline, restart-and-retry-once on failure, detach-and-restart on timeout and suspend/resume "
   "for Cold or paused monitors. Compare foreground identity before and after the screenshot (reject on change or unknown), keep AX text "
   "marked not pixel-aligned when only the walked process differs, time-box the focused-metadata lookup and re-apply privacy filters and "
   "URL policy after resolving the real app/window/URL.",
   ["cargo test: walker timeout detaches and restarts; a failure retries once",
    "Unit: identity change between pre and post screenshot rejects the frame; a post-resolution blocked URL drops the frame"],
   ["specs/vision.md tree-walker-worker, capture-identity-gate"])

wp("W4-15", 4, "L", "Redaction pseudonyms, a11y-aware secret detection, configurable models",
   ["vision-redaction-pseudonyms-a11y", "vision-redaction-models"],
   [RD + "pseudonym.rs (new)", RD + "a11y_context.rs (new)", RD + "ocr_json.rs (new)", RD + "tree_json.rs (new)", RD + "pipeline.rs",
    RD + "image/worker.rs", RD + "image/secret_continuation.rs (new)", RD + "adapters/rfdetr.rs", RD + "adapters/onnx.rs"],
   ["W3-10"],
   "Consistent pseudonyms keyed by .redaction/pseudonym.key (never silently regenerated); accessibility-context secret detection "
   "(password/secure fields, label hints, token prefixes, PEM) feeding text and image redaction; secret continuation across frames; "
   "CPU-budgeted batches; AI adapter failure circuit falling back to regex. Model id/URL/hash configurable with base defaults "
   "(rfdetr_v12, v45) kept.",
   ["cargo test -p screenpipe-redact: same input and key give the same pseudonym; a missing key is an error state, not a new key",
    "A password-field fixture and a PEM block are redacted; model config overrides are honoured"],
   ["specs/vision.md redaction-pseudonyms-a11y, redaction-models"])

wp("W4-16", 4, "L", "Bluetooth meeting gate, output follow and speaker watchdog",
   ["audio-011", "audio-017"],
   [AU + "core/device.rs", AU + "core/device_detection.rs", AU + "audio_manager/device_monitor.rs (follow, watchdog, BT regions)",
    AU + "audio_manager/manager.rs (BT gate call)"],
   ["W3-14"],
   "Bluetooth combo-headset mics stay closed outside meetings unless alwaysRecordBluetoothMic or explicitly selected (PnP container walk "
   "with name fallback, fail-safe to combo). [AUDIO_FOLLOW] captures any rendering-but-uncaptured endpoint and stops when idle; "
   "[SPEAKER_WATCHDOG] cycles output capture when a meeting renders audio but yields no chunks and raises speaker_silent.",
   ["cargo test: name heuristics and a mocked PnP walk classify combo headsets",
    "Manual: a BT headset mic is not opened while idle; [AUDIO_FOLLOW] log appears when audio plays on a second output"],
   ["specs/audio.md audio-011, 017"])

wp("W4-17", 4, "L", "Calendar v2: all-calendar Google events and reminder scheduler",
   ["connections-google-calendar-multi", "meetings_memory-046"],
   [E + "connections_api.rs (Google Calendar events handler)", AR + "google_calendar.rs (publisher region)", AR + "calendar/reminders.rs (new)",
    "crates/screenpipe-events/src/custom_events/meetings.rs (prewarm lead)", AR + "calendar.rs"],
   ["W2-21", "W3-18"],
   "Read every visible calendar (paginated calendarList), dedupe by iCalUID, honour declined/transparent, real calendarName, "
   "account_snapshots and require_complete; the app publisher keeps the last good per-account snapshot. App-side reminder scheduler with "
   "notificationPrefs.meetingReminderLeadSeconds, a durable receipt ledger and withholding when the meeting is already active.",
   ["Harness (BYO Google OAuth): `api '/connections/google-calendar/events?account_snapshots=true'` returns events from several calendars with calendarName",
    "cargo test (app crate) reminders: fires at the lead time, does not refire after restart, withheld when the meeting is active"],
   ["specs/connections.md google-calendar-multi", "specs/meetings_memory.md meetings_memory-046"])

wp("W4-18", 4, "L", "Bee pairing and browser snapshot v2 with /act",
   ["connections-bee-pairing", "connections-browser-snapshot-v2-act"],
   [CN + "connections/bee.rs", E + "connections_routes/bee.rs", E + "connections_routes/browser_act.rs", E + "connections_api.rs (browser snapshot handler)",
    CA + "skills/screenpipe-api/SKILL.md (browser control block)"],
   ["W2-20"],
   "POST /connections/bee/pair/start and /pair/poll with an ephemeral keypair and versioned token decryption; the pairing app id is a "
   "user setting (no vendor default). Snapshot v2 (depth 12, shadow roots, same-origin iframes, names, state flags, data-sp-ref eN refs, "
   "count) and POST /connections/browsers/:id/act {ref, action, value} through a fixed action script.",
   ["Harness: pair/start without a configured Bee app id returns 400; with one it returns request_id and pairing_url",
    "On the owned browser: snapshot returns refs; POST /connections/browsers/<id>/act {ref:'e3',action:'click'} succeeds"],
   ["specs/connections.md bee-pairing, browser-snapshot-v2-act"])

wp("W4-19", 4, "L", "Window inventory, screen-share protection, renderer heartbeat",
   ["shell-008", "shell-018", "shell-039", "frontend-005"],
   [AR + "window/show.rs", AR + "window/mod.rs", AR + "window/content_process.rs", AR + "windows_overlay.rs", AR + "screen_capture_protection.rs (new)",
    A + "app/layout.tsx (heartbeat, contextmenu, swipe)"],
   ["W3-28"],
   "notification-inbox and starred-sessions windows (show_notification_inbox, toggle/hide_starred_sessions), central headless and "
   "policy suppression, live set_chat_always_on_top; hideAppInScreenShare applies WDA_EXCLUDEFROMCAPTURE to all windows with "
   "get/set_app_screen_capture_protection; every webview sends webview_renderer_heartbeat each second and stale renderers reuse the "
   "ProcessFailed recovery; swipe navigation disabled; context menu suppressed.",
   ["UI smoke: show_notification_inbox opens a window labelled notification-inbox; toggle_starred_sessions opens starred-sessions",
    "With hideAppInScreenShare on, a screen capture of the desktop does not show app windows",
    "Debug log shows heartbeats every second per window"],
   ["specs/shell.md shell-008, 018, 039", "specs/frontend.md frontend-005"])

wp("W4-20", 4, "L", "UI localization (en + ja) and native string tables",
   ["shell-036", "frontend-003"],
   [A + "app/providers.tsx", A + "lib/i18n/** (new, own message ids)", A + "components/settings/general-settings.tsx (language selector)",
    AR + "localization.rs (new)"],
   ["W2-25", "W1-02"],
   "uiLocale (system or code) and uiLocalizationEnabled; resolve locale from OS/navigator, set html lang/dir, mirror to localStorage; "
   "own message catalogues with English fallback; the native layer rebuilds tray/menu/notification strings on store changes. A visible "
   "toggle replaces the vendor rollout flag.",
   ["bun test: catalogue keys are complete for en and fall back for missing ja strings",
    "UI smoke: switching to ja updates html lang, visible strings and the tray menu"],
   ["specs/shell.md shell-036", "specs/frontend.md frontend-003"])

wp("W4-21", 4, "XL", "Narrated SOP video renderer (core)",
   ["gap_workflow-sop-video-and-media-core-renderer"],
   [C + "workflow_video.rs (new)", C + "workflow_video/ (new)", C + "ffmpeg.rs (preflight helper only)", "crates/screenpipe-core/tests/workflow_video_test.rs (new)"],
   ["W3-31"],
   "Scene/Focus types and validation; ffmpeg preflight; screenshot normalisation; per-scene parts on a 1280x720@24 canvas with header, "
   "focus zoompan or scale/pad, or a text card; narration retimed into PCM via the speech provider; WebVTT captions; concat and mux with "
   "AAC and a default mov_text 'Narration' track, libx264 +faststart; duration/size/time limits; progress and cancel callbacks.",
   ["cargo test -p screenpipe-core --test workflow_video_test renders a 2-scene project; ffprobe shows 1280x720, 24 fps and a mov_text track titled Narration",
    "51 sections or pace 2.0 are rejected; the cancel callback stops rendering within 2 s"],
   ["specs/gap_workflow-sop-video-and-media.md core-renderer"])

wp("W4-22", 4, "L", "App shell: (main) route group, navigation v2, command palette",
   ["frontend-002", "frontend-006", "frontend-008"],
   [A + "app/(main)/** (new route group; app/home and app/settings move here)", A + "components/app-sidebar.tsx", A + "components/command-palette.tsx (new)",
    A + "lib/keybindings.ts (new)", A + "components/shortcut-guide.tsx (new)"],
   ["W2-25"],
   "Shared (main) layout for /home and /settings with the resizable sidebar and drag surface, ?from= back navigation and section aliases; "
   "nav sections Chat/Meetings/Timeline/Activity/Library/Automations/Connections with reorder/hide (sidebarNavLayout, legacy migration) "
   "and badges; cmdk palette and the fixed in-app keybindings with a shortcut guide. After this WP, home/settings pages live under "
   "app/(main)/ and later WPs edit them there.",
   ["bun run build still exports /home and /settings; bun test keybindings registry",
    "UI smoke: Ctrl+K opens the palette; reordering and hiding nav items persists across restart"],
   ["specs/frontend.md frontend-002, 006, 008"])

# ---------------------------------------------------------------- wave 5
wp("W5-01", 5, "L", "Pipes API v2 and SSE output stream",
   ["agents_pipes-005", "agents_pipes-006", "api-029"],
   [E + "pipes_api.rs", E + "routes/pipe_stream.rs", C + "pipes/mod.rs (run body and execution query region)", E + "pipe_store.rs (paging queries)"],
   ["W4-02"],
   "GET /pipes with pagination and include_* flags, /pipes/activity, /pipes/:id/executions/:exec_id, POST /pipes/bundled/:id/install, "
   "run body run_context/trigger_type/meeting_summary returning execution_id, error_code envelope and 404 for missing pipes; "
   "/pipes/:id/stream SSE with progress/keep-alive, a concurrent-stream cap and pipe_done.",
   ["Harness: `api '/pipes?limit=5&offset=0'` returns pagination; `api /pipes/activity` returns an aggregate; missing pipe returns 404 with error_code",
    "POST /pipes/<id>/run {trigger_type:'manual'} returns execution_id; `curl -N` on /pipes/<id>/stream receives progress events and pipe_done"],
   ["specs/agents_pipes.md agents_pipes-005, 006", "specs/api.md api-029"])

wp("W5-02", 5, "L", "Skills registry, external-agent skill export, provider automations",
   ["agents_pipes-025", "agents_pipes-029", "frontend-048"],
   [AR + "skills.rs (registry and export region)", AR + "provider_automations.rs (new)", A + "components/settings/skills-card.tsx (registry UI)",
    A + "components/settings/provider-automations.tsx (new)", AT + "assets/skills-registry.json (new, bundled)"],
   ["W4-06", "W4-03"],
   "fetch_skills_registry reads a bundled registry (optional user-configured URL), install_registry_skill from user-chosen GitHub folders "
   "with limits, install/remove_external_agent_skills for Claude Code, Codex, Cursor, Gemini, OpenClaw and Hermes skill dirs, "
   "auto-connect opt-out; list/manage_provider_automation reads Claude scheduled tasks and Codex automation manifests.",
   ["cargo test (app crate): registry loads from the bundled file with no network; provider automations parse fixture files",
    "UI smoke: install a registry skill into ~/.claude/skills and remove it"],
   ["specs/agents_pipes.md agents_pipes-025, 029", "specs/frontend.md frontend-048"])

wp("W5-03", 5, "M", "Meeting room-change offers",
   ["meetings_memory-029"],
   ["crates/screenpipe-events/src/custom_events/meetings.rs (offer/response events)", E + "meeting_watcher/audio_process/room_change.rs (new)",
    AR + "meeting_live_notes.rs (room-change prompt region)"],
   ["W4-12"],
   "The audio-process detector emits meeting_room_change_offer when the same audio session moves to another call; the app asks the user whether to keep "
   "the meeting together or start a new note, and answers through POST /meetings/room-change (route from W2-08).",
   ["cargo test: a simulated session move emits one offer with a token",
    "UI smoke: choosing the start-a-new-note option ends the old meeting with end_reason room_changed and opens a new one"],
   ["specs/meetings_memory.md meetings_memory-029"])

wp("W5-04", 5, "L", "Audio device lifecycle hardening and shared engine reuse",
   ["audio-M02", "audio-M03"],
   [AU + "device/device_manager.rs", AU + "audio_manager/manager.rs (start/stop/receiver)", AU + "audio_manager/device_monitor.rs (backoff region)"],
   ["W4-16"],
   "Idempotent teardown even when a device is already marked stopped, per-device serialised start/stop with timeouts, worker-exit "
   "tracking, refusal of 0-channel/0-Hz streams, 0x80070005 classified as access denied, monitor backoff and user notice on lost devices. "
   "Reuse the shared transcription engine across restarts, build engines in a panic-isolated task and keep the receiver alive when no "
   "session can be created.",
   ["cargo test -p screenpipe-audio: double stop is a no-op; concurrent start/stop are serialised; a zero-channel config is refused",
    "Unit: an engine factory panic leaves the receiver running and audio pending"],
   ["specs/audio.md audio-M02, M03"])

wp("W5-05", 5, "L", "Capture loop C: screenshot policy, scroll cadence, width cap, frozen streams",
   ["vision-capture-loop-refinements", "vision-disable-screenshots", "vision-recording-detail", "vision-capture-max-width"],
   [E + "event_driven_capture.rs", E + "vision_manager/manager.rs (screenshot policy)", E + "ui_recorder.rs (scroll snapshot cadence)",
    E + "power/profile.rs", SC + "monitor.rs (Windows downscale)", E + "routes/health.rs (screenshots_disabled_by_config reason)"],
   ["W4-14"],
   "disableScreenshots keeps triggers and a11y walks but writes frames without JPEG/OCR/visual probes; recordingDetail controls scroll "
   "snapshot cadence (auto adapts 1-5 s); captureMaxWidth caps capture width on Windows after readback; frozen persistent streams are "
   "released, drop accounting splits timeout vs error, soft checkpoints are deferred, the pause gate runs before focus probes and the "
   "a11y worker suspends on pause/Cold.",
   ["disableScreenshots=true: new frames have no snapshot path and `api /health` reports screenshots_disabled_by_config",
    "captureMaxWidth=1280: new frame images are at most 1280 px wide; recordingDetail=more_detail yields about 1 s scroll snapshots (log)",
    "cargo test: a frozen stream test hook triggers rebuild"],
   ["specs/vision.md capture-loop-refinements, disable-screenshots, recording-detail, capture-max-width"])

wp("W5-06", 5, "L", "OCR text-region gate and crop OCR",
   ["vision-ocr-gate"],
   [SC + "text_regions.rs (new)", SC + "lib.rs", "crates/screenpipe-capture/src/paired_capture.rs (OCR gate region)", SC + "metrics.rs (gate counters)"],
   ["W3-12", "W4-14"],
   "Detect text regions on a downscaled grayscale (optionally window-cropped) image; no regions skips OCR; otherwise OCR the padded union "
   "and map boxes back to full-frame normalised coordinates; fail open; ocr_gate_* metrics. The detector design is ours.",
   ["cargo test -p screenpipe-screen text_regions: a blank image yields no regions and OCR is skipped; a text fixture's boxes map back within 1%",
    "`api /vision/metrics` shows ocr_gate_skips and crop_ocr counters increasing during capture"],
   ["specs/vision.md ocr-gate"])

wp("W5-07", 5, "XL", "Hybrid storage format hybrid-parquet-v1",
   ["db-021"],
   [D + "storage/** (new: descriptor, hybrid migrations, payload tables, sealing worker, reader)", "crates/screenpipe-db/Cargo.toml",
    D + "db.rs (read-path dispatch region)"],
   ["W2-01"],
   "Opt-in format: SQLite index plus checksummed Parquet payloads (screenpipe_search_v1 / screenpipe_detail_v1) and storage.json; "
   "_hybrid_migrations, storage_metadata, upload_bindings, payload_files, frame_payloads, payload_* frame columns, contentless frames_fts, "
   "budget/revision/revocation triggers, sealing worker, revision-checked reader and raw_sql restrictions. Nothing changes for "
   "non-hybrid databases.",
   ["cargo test -p screenpipe-db storage: create a hybrid index, insert frames, seal a day into search/detail parquet with checksums",
    "The reader returns the same text as the SQLite path; a revoked payload is not readable; storage.json has the documented fields"],
   ["specs/db.md db-021", "specs/vision.md hybrid-payload-storage (merged)"])

wp("W5-08", 5, "L", "Database self-heal user flow",
   ["shell-014"],
   [AR + "db_self_heal.rs (new)", AR + "db_relaunch.rs (new)", AR + "db_wedge.rs (new)", AR + "notifications/db_recovery.rs (new)",
    AR + "recording.rs (retry events region)"],
   ["W4-09"],
   "Classify DB open errors, keep the UI up and retry transient failures, notify with recover-database/dismiss actions and the "
   "database-recovery deep link, commands start_database_recovery/restart_database_verification, DB-wedge restart and recording retry "
   "events. Recovery internals come from W3-27/W4-09.",
   ["UI smoke: starting with a quarantine marker shows the recovery notification; start_database_recovery runs recovery and relaunches",
    "A simulated transient open error is retried without tearing down the UI (log)"],
   ["specs/shell.md shell-014"])

wp("W5-09", 5, "M", "Tray menu v2",
   ["shell-023"],
   [AR + "tray.rs"],
   ["W4-20", "W2-11"],
   "Per-monitor toggles through /vision/device/start|stop, compound status lines (audio waiting for meeting, meeting detection "
   "unavailable, outside work hours), pause/resume notifications, Open first summary, Help submenu, localized templates, Windows tray "
   "recreation and menu-install retries. No plan/upgrade rows.",
   ["UI smoke: toggling a monitor in the tray changes /vision/device/status; with meetings-only and no meeting the tray shows the waiting status",
    "Restarting explorer.exe recreates the tray icon"],
   ["specs/shell.md shell-023"])

wp("W5-10", 5, "L", "Workflow video jobs in the app",
   ["gap_workflow-sop-video-and-media-app-video-jobs"],
   [AR + "workflow_video.rs (new)", A + "lib/workflows/video-jobs.ts (new)"],
   ["W4-21"],
   "create_workflow_video(id, scenes) -> {path, captionsPath}, cancel and discard; storage under <appData>/workflow-videos/<uuid> with a "
   "temp .render-<id>; data-URL screenshot validation; single-flight; 700 MiB quota and 24 h pruning; progress on workflow-video-<id>.",
   ["UI or test harness: create_workflow_video writes video.mp4 and captions.vtt and emits progress events starting with `Preparing video` (exact string, client-visible)",
    "A second concurrent call is refused; quota pruning test removes the oldest previews"],
   ["specs/gap_workflow-sop-video-and-media.md app-video-jobs"])

wp("W5-11", 5, "L", "Captured-moment replay server and open command",
   ["gap_workflow-sop-video-and-media-recording-replay", "gap_workflow-sop-video-and-media-captured-moment-open"],
   [AR + "workflows_media.rs (new)"],
   ["W2-10"],
   "load_workflow_recording(timestamp, appName) finds the nearest frame through preview-samples and returns a video clip served by a "
   "loopback replay server (GET /media/:token with Range, Host/Origin checks, no-store, nosniff) or an inline snapshot via the thumbnail "
   "route; release_workflow_recording stops the server when idle; open_workflow_captured_moment validates and navigates the timeline.",
   ["Test: load_workflow_recording returns kind video with an http://127.0.0.1:<port>/media/<token> URL; a Range request returns 206; a wrong Host returns 403",
    "open_workflow_captured_moment with frameId 0 returns `invalid captured moment` (exact string, client-visible)"],
   ["specs/gap_workflow-sop-video-and-media.md recording-replay, captured-moment-open"])

wp("W5-12", 5, "L", "Activity history generator (app)",
   ["meetings_memory-016"],
   [AR + "activity_history.rs (new)", AR + "pi.rs (session API hook only)"],
   ["W3-04", "W3-25"],
   "get_activity_history/generate_activity_history/set_activity_sync_enabled run a pi agent over meetings, activity-summary, "
   "activity-ledger and search to produce validated JSON entries with evidence, a repair pass, coverage tracking and an auto schedule "
   "(activitiesEnabled, activitiesAiPresetId, activitiesIntervalMinutes); a notification when activities have been updated. Uses the user's preset; no "
   "vendor quota.",
   ["With a mock local preset, generate_activity_history returns entries that pass validation; with no capture it returns activity_no_data",
    "The auto schedule runs at the configured interval (shortened in test)"],
   ["specs/meetings_memory.md meetings_memory-016"])

wp("W5-13", 5, "L", "Notifications UI v2: inbox window, bell, panel, preferences",
   ["meetings_memory-039", "frontend-030", "frontend-031", "meetings_memory-040", "frontend-040", "meetings_memory-034"],
   [A + "app/notification-inbox/page.tsx (new)", A + "components/notification-bell.tsx", A + "app/notification-panel/page.tsx",
    A + "components/settings/notifications-settings.tsx", AR + "meeting_live_notes.rs (live-note prompt gate region)"],
   ["W3-18", "W4-19"],
   "Bell opens the notification-inbox window with Priority/All views, a high-priority heuristic, unread-high badge, per-item read, copy, "
   "source, actions and settings link (5 s polling). Toast gains open source chat and meeting toasts can render in the overlay. "
   "Preferences: master switch, snooze presets, quiet hours, grouped categories, per-pipe mute/VIP, meeting reminders (migrated from "
   "meetingLiveNotes) and reminder lead time.",
   ["UI smoke: the bell opens the inbox; expanding an item calls POST /notifications/<id>/read",
    "bun test: meetingReminders migrates from meetingLiveNotes; quiet hours persist in notificationPrefs.quietHours"],
   ["specs/meetings_memory.md 034, 039, 040", "specs/frontend.md frontend-030, 031, 040"])

wp("W5-14", 5, "L", "Starred sessions UI, star shortcut, overlay snooze/anchor, CLI",
   ["meetings_memory-012", "frontend-029", "shell-009"],
   [A + "app/starred-sessions/page.tsx (new)", A + "components/shortcut-reminder.tsx (star button region)", A + "app/shortcut-reminder/page.tsx (snooze/anchor region)",
    AR + "shortcuts.rs", AR + "starred_sessions.rs (new)", E + "cli/star.rs (new)", E + "cli/mod.rs (subcommand line only)"],
   ["W2-04", "W4-19"],
   "starSessionShortcut (Alt+Shift+B on Windows) toggles the starred-sessions window: star 5/15/30/60 min, +15, end, edit times, HD flag, "
   "ask in chat, export video; revision conflicts reload; events starred-sessions-changed/-visibility and starred-session-toggle/-state. "
   "Overlay one-hour snooze, edge anchors (legacy corners mapped), display choice. CLI star list/start/end/edit.",
   ["UI smoke: Alt+Shift+B opens the window; the 15-minute star action creates a session visible in GET /starred-sessions; +15 bumps the revision",
    "screenpipe star list prints the session; snoozing the overlay sets shortcutOverlaySnoozedUntil one hour ahead"],
   ["specs/meetings_memory.md meetings_memory-012", "specs/frontend.md frontend-029", "specs/shell.md shell-009"])

wp("W5-15", 5, "L", "Chat tabs, split pane, side chat, groups and landing cards",
   ["frontend-009", "frontend-020"],
   [A + "components/chat-sidebar.tsx", A + "components/standalone-chat.tsx (tabs/split/landing regions)", A + "lib/stores/chat-store.ts",
    A + "lib/chat-storage.ts", A + "components/chat/tab-strip.tsx (new)", A + "components/chat/split-pane.tsx (new)", A + "components/chat/summary-cards.tsx"],
   ["W4-05", "W4-22"],
   "Tab strip with status glyphs, split pane, temporary side chat, branch/archive/pin/rename/copy, user sidebar groups, Pinned/Recents/"
   "Automations with filters, sort and bulk actions; persisted lastContentAt; fast listing through list_chat_entries_by_mtime. Landing "
   "cards gain run-in-agent actions and contextual suggestions gated by showChatSuggestions.",
   ["bun test chat-store: tabs, groups and lastContentAt persistence",
    "UI smoke: open three tabs, split two, branch one, archive one; restart keeps the layout"],
   ["specs/frontend.md frontend-009, 020"])

wp("W5-16", 5, "L", "Library: memories and artifacts",
   ["meetings_memory-006", "frontend-021"],
   [A + "components/library/** (new)", A + "components/settings/memories-section.tsx", A + "app/(main)/home/page.tsx (library section mount)"],
   ["W2-02", "W4-22"],
   "Library section replacing Memories: view switcher, filters (labels, sources, people, dates), memory CRUD via /memories, artifacts "
   "browser via /artifacts with SAF badges, open in viewer or source chat, ask AI, delete, 30 s counters, artifact deep links.",
   ["UI smoke: Library lists memories and artifacts; deleting an artifact calls DELETE /artifacts/<id>",
    "bun test: filter reducer and SAF badge mapping"],
   ["specs/meetings_memory.md meetings_memory-006", "specs/frontend.md frontend-021"])

wp("W5-17", 5, "L", "Meetings workspace UI",
   ["frontend-024"],
   [A + "components/meeting-notes/**", AR + "meeting_export.rs (async start_export_recording + export:event)"],
   ["W2-08", "W4-22"],
   "Replay player controls, streaming summary surface polling /meetings/:id/summary-status with model change, meeting chat panel, "
   "copy_rich_text_to_clipboard, async export with progress events, editor slash menu and format toolbar, join suggestions and calendar "
   "nudge.",
   ["UI smoke: a meeting note shows summary state from summary-status; export shows progress and produces an mp4",
    "bun test: slash menu inserts the expected nodes"],
   ["specs/frontend.md frontend-024"])

wp("W5-18", 5, "L", "Timeline and search window v2",
   ["frontend-025", "frontend-026", "frontend-027"],
   [A + "components/rewind/**", A + "app/search/page.tsx", AR + "native_timeline.rs (new, Windows returns unavailable)"],
   ["W3-02", "W4-22"],
   "Timeline selection export (POST /export), on-demand daily summary with the user's model, a placeholder for frames captured while screenshots were paused, timeline-* "
   "events from native menus, native timeline commands (unavailable on Windows), search window recent-activity chips, timelineOrigin, "
   "search-hidden/search-reset events.",
   ["UI smoke: selecting a range and exporting produces a file; frames without snapshots show the placeholder",
    "native_timeline_is_available resolves false on Windows (bun test against bindings)"],
   ["specs/frontend.md frontend-025, 026, 027"])

wp("W5-19", 5, "L", "Settings navigation v2, general, AI features, permissions",
   ["frontend-035", "frontend-041", "frontend-042", "frontend-043"],
   [A + "app/(main)/settings/page.tsx", A + "components/settings/settings-search.tsx", A + "components/settings/general-settings.tsx",
    A + "components/settings/display-section.tsx", A + "components/settings/shortcut-section.tsx", A + "components/settings/ai-presets.tsx (connection test region)",
    A + "components/settings/ai-features-section.tsx (new)", A + "components/settings/permissions-section.tsx (new)", A + "components/settings/input-monitoring-card.tsx"],
   ["W4-22", "W4-20"],
   "Grouped navigation (four groups: capture and data, AI, account, app) with audio, activities, ai-settings and permissions sections, default "
   "'recording', last section remembered, fuse search with field highlight; general/appearance deltas (keepSearchAvailableAfterQuit, "
   "release channel, live chat-on-top, hide overlay from capture, star shortcut row); split AI features vs models with a mandatory "
   "connection test and Gemini via OpenAI-compatible URL; macOS permissions section hidden on Windows.",
   ["UI smoke: grouped nav renders; search jumps to a field; reopening settings returns to the last section",
    "bun test: preset save is blocked until the mocked connection test passes"],
   ["specs/frontend.md frontend-035, 041, 042, 043"])

wp("W5-20", 5, "L", "Screen and audio settings",
   ["frontend-036", "frontend-037"],
   [A + "components/settings/recording-settings.tsx", A + "components/settings/battery-saver-section.tsx", A + "components/settings/meeting-apps-picker.tsx",
    A + "components/settings/audio-settings-section.tsx (new)"],
   ["W3-14", "W3-15", "W4-22"],
   "Separate screen-context vs screenshots toggles, quality presets, capture-frequency slider, scroll capture and recordingDetail, keep "
   "awake; audioCaptureMode, engine list with test_openai_compatible_transcription before enabling, unified aecMode, single-app Windows "
   "audio exclusion, Bluetooth mic toggle, live captions provider limited to local/BYO.",
   ["UI smoke: audioCaptureMode and aecMode selections persist and only platform-valid aecMode options appear",
    "bun test: enabling OpenAI-compatible transcription is blocked when the mocked test fails"],
   ["specs/frontend.md frontend-036, 037"])

wp("W5-21", 5, "M", "Deep links v2",
   ["frontend-045", "frontend-M01", "cloud-V01"],
   [A + "components/deeplink-handler.tsx", A + "lib/deeplinks/** (new validator)"],
   ["W4-22"],
   "Hosts activity, artifact, chat, database-recovery, database-restart-verification, first-run-agent, first-run-summary, home, live-view, "
   "pipe, scheduled, settings, workflows (reviewRequest dedupe), calendar-connections; versioned external links chat/new?v=1&prompt= and "
   "pipe/<name>?v=1 with strict validation and no auto-send. Announcements are never fetched remotely; only a local /notify announcement "
   "event may feed that UI and never offers pipe installs.",
   ["bun test: chat/new?v=1&prompt=hi accepted; v=2, unknown params, credentials, a port, a fragment or a >4096-byte prompt rejected; pipe/../x rejected",
    "grep -rn 'app-announcement' apps/screenpipe-app-tauri returns nothing"],
   ["specs/frontend.md frontend-045, M01", "specs/cloud.md cloud-V01", "specs/shell.md shell-027 (merged)"])

wp("W5-22", 5, "M", "Viewer, permission recovery and help refresh",
   ["frontend-028", "frontend-033", "frontend-055"],
   [A + "app/viewer/page.tsx", AR + "viewer.rs", A + "app/permission-recovery/page.tsx", AR + "permissions.rs (restart/retry commands)",
    A + "components/settings/feedback-section.tsx"],
   ["W2-26", "W4-22"],
   "Viewer renders HTML in a no-network sandbox with a source toggle, refuses unsafe files and shows diagnostics listing the locations it searched; permission "
   "recovery adds the restart prompt and retry_screenpipe; Help gains local log bundle, getting started, shortcuts and rerun-guide "
   "entries with GitHub links.",
   ["UI smoke: an HTML file opens in the sandbox and cannot load remote resources; an .exe is refused",
    "Help > share logs uses the local bundle from W0-10"],
   ["specs/frontend.md frontend-028, 033, 055"])

wp("W5-23", 5, "L", "Connections UI catalogue",
   ["connections-ui-catalog", "frontend-050"],
   [A + "components/settings/connections-section.tsx", A + "components/settings/imap-card.tsx (new)", A + "components/settings/bee-card.tsx (new)",
    A + "public/images/excalidraw.svg (own)", A + "public/images/google-drive.svg (own)"],
   ["W3-19", "W3-20", "W3-21", "W4-18"],
   "New tiles and renames (an advanced-connections group, an other-calendars tile and a browser-activity tile), Advanced token/API-key fallbacks for OAuth and MCP "
   "tiles, scopeVariant deep links, Bee and IMAP panels with test/mailboxes, MCP registry browser, oauth_connect variant; Composio/login "
   "gating replaced by BYO prompts.",
   ["UI smoke: Bee pairing and IMAP connect end to end; a scopeVariant deep link opens the Slack panel with read_write selected",
    "No Composio tile or link is present (grep composio in components returns nothing)"],
   ["specs/connections.md ui-catalog", "specs/frontend.md frontend-050"])

# ---------------------------------------------------------------- wave 6
wp("W6-01", 6, "XL", "ACP presets and coding-agent chat UI",
   ["agents_pipes-017", "frontend-015"],
   [A + "components/settings/ai-presets.tsx (ACP region)", A + "components/standalone-chat.tsx (ACP mode/approval/thinking region)", A + "components/chat/acp/** (new)",
    AR + "store.rs (AIPreset fields)", E + "cli/presets.rs", A + "lib/hooks/use-settings.tsx (AIPreset type only)"],
   ["W4-03", "W5-15"],
   "AIPreset gains provider 'acp' and acpAgent {id, command, args, env, authMethod, config, modeId, approvalMode}; PiProviderConfig adds "
   "backend/allowedTools/resumeSessionId/unattended; per-chat presetId. UI: agent catalogue, probe/install/login, in-chat mode and "
   "config options, approval modes, thinking level, reauth. No cloud routing option.",
   ["UI smoke: create a codex-acp preset (user-initiated install), chat with it, switch approval mode and thinking level",
    "bun test: preset schema accepts provider acp and rejects unknown approval modes"],
   ["specs/agents_pipes.md agents_pipes-017", "specs/frontend.md frontend-015"])

wp("W6-02", 6, "M", "Run destinations and conversation continuity",
   ["agents_pipes-009", "agents_pipes-m02"],
   [C + "pipes/mod.rs (run_in region)", C + "pipes/run_destinations.rs (new)", C + "agents/pi.rs (resume session region)",
    AR + "pi.rs (stream-to-chat region)", A + "lib/chat-storage.ts (existing_chat append)"],
   ["W5-01"],
   "Scheduled runs can stream into an existing chat; history:true resumes with conversation_id persisted; ACP session map; scheduled-task "
   "preamble line; per-device .local-run-destinations.json so a synced pipe never targets another device's chat (corrupt file ignored "
   "with a warning).",
   ["Test: a pipe with run_in existing_chat appends its output to the chosen chat file",
    ".local-run-destinations.json stores the mapping; a corrupt file is ignored with a warning; history:true resumes the same conversation_id"],
   ["specs/agents_pipes.md agents_pipes-009, m02"])

wp("W6-03", 6, "L", "Connection-source pipe triggers",
   ["agents_pipes-008"],
   [C + "pipes/connection_triggers.rs (new)", C + "pipes/mod.rs (subscription registration line)"],
   ["W4-02", "W3-19", "W3-20", "W2-21"],
   "Poll connected apps through the local /connections proxies (Slack, Notion, IMAP, Google Calendar, Outlook, GitHub, Linear, Todoist, "
   "Obsidian, spoken phrases) with cursors in .connection-triggers.json and fire pipes through connection_trigger events with retry then "
   "cursor advance.",
   ["cargo test with a mocked IMAP proxy: a new message fires one connection_trigger for a subscribed pipe; the cursor advances",
    "A voice-phrase subscription fires on a matching transcript"],
   ["specs/agents_pipes.md agents_pipes-008", "specs/connections.md connection-triggers (merged)"])

wp("W6-04", 6, "L", "External chat import and chat-control broker",
   ["agents_pipes-019", "frontend-010"],
   [AR + "chat_control.rs (new)", C + "agents/chat_control.rs (new)", AR + "external_chat_import.rs (new)", A + "components/chat-sidebar.tsx (import region)",
    A + "lib/chat-storage.ts (importedFrom)"],
   ["W5-15"],
   "Import ~/.claude/projects and ~/.codex/sessions JSONL (7-day lookback, 100 sessions, skip > 32 MiB) into chats with importedFrom and "
   "deterministic ids; local TCP broker (SCREENPIPE_CHAT_CONTROL_ADDR/TOKEN) with search/send/worktree actions across local AI chats.",
   ["Test with fixture JSONL files: import creates chats with importedFrom; re-import updates instead of duplicating",
    "The broker rejects a wrong token and answers a search action with the right one"],
   ["specs/agents_pipes.md agents_pipes-019", "specs/frontend.md frontend-010"])

wp("W6-05", 6, "XL", "Meeting piggyback (smart recording)",
   ["audio-016"],
   [AU + "audio_manager/manager.rs (piggyback region)", AU + "audio_manager/device_monitor.rs", AU + "meeting_audio/windows.rs (new)", AU + "piggyback.rs (new)"],
   ["W5-04", "W4-12", "W4-16"],
   "During a meeting capture the meeting app's audio as 'Meeting Tap (output)' with strikes and give-up, follow the meeting app's mic "
   "(process/configured/system_default), suspend other mics, restore configured devices afterwards and classify the outcome "
   "(full/partial/stable_fallback/no_pid/unavailable). Absorbs the per-pid input resolution from audio-015. Telemetry stays local.",
   ["Manual Teams call: /audio/device/status lists 'Meeting Tap (output)'; afterwards the configured devices are restored",
    "cargo test: outcome classification for each case with mocked device state"],
   ["specs/audio.md audio-016, audio-015 (merged)"])

wp("W6-06", 6, "XL", "Hybrid bulk column archival and screenpipe_elements virtual table",
   ["db-022"],
   [D + "storage/bulk/** (new)", D + "storage/elements_vtab.rs (new)", D + "db.rs (element insert region: explicit ids)"],
   ["W5-07"],
   "_bulk_files/_bulk_legacy_sql catalogue, _archive_* columns, contentless <t>_fts and hybrid_bulk_* triggers for audio_transcriptions, "
   "ui_events, semantic_items, pipe_executions, meeting_transcript_segments and outputs; elements becomes a vtab over _bulk_element_rows "
   "plus Parquet ranges with element sealing.",
   ["cargo test -p screenpipe-db storage::bulk: archived columns read back transparently; elements vtab returns the same rows as the table for a frame",
    "Contentless FTS search on archived audio text returns the expected rows"],
   ["specs/db.md db-022"])

wp("W6-07", 6, "XL", "Workflows workspace UI",
   ["frontend-047"],
   [A + "components/workflows/** (new)", A + "app/(main)/home/page.tsx (workspace switcher mount)", AR + "workflows_runtime.rs (new)"],
   ["W4-01", "W4-22"],
   "Chat/Workflows workspace switcher; catalogue via analyze_workflows and agent pipes; editor (steps, timing, captured moments, voice "
   "notes, corrections); SOP documents with HTML export; skill generation entry; assistant. No Tinfoil verification, contribution "
   "sharing or web SOP editor.",
   ["UI smoke: the Workflows workspace lists /workflows entries; editing a step posts /workflows/edits; SOP export writes an HTML file",
    "grep in components/workflows for trajectories/sops/tinfoil returns nothing"],
   ["specs/frontend.md frontend-047"])

wp("W6-08", 6, "L", "Workflow ready-to-review notifications",
   ["gap_workflow-sop-video-and-media-workflow-ready-notification"],
   [AR + "notifications/workflow.rs (new)", AR + "notifications/mod.rs (dispatch line)", AR + "notifications/routes.rs (workflow branch)", AR + "notifications/client.rs"],
   ["W4-01", "W3-18"],
   "Read catalog.json (backup fallback), select saved supported workflows, keep notification-ledger.json with fingerprints and "
   "suppression reasons (no_meaningful_change, daily_limit, ledger_full, paused, already_viewing, duplicate, transport_failed, "
   "display_timeout), coalesce hourly with workflow-review:<key> ids and a workflows deep link.",
   ["Test: a new saved workflow yields one notification with a workflows?workflow=...&reviewRequest=... deep link; the same fingerprint is suppressed",
    "An unreadable ledger fails closed (no toast, logged)"],
   ["specs/gap_workflow-sop-video-and-media.md workflow-ready-notification"])

wp("W6-09", 6, "L", "Video render CLI mode and Pi video-sop extension",
   ["gap_workflow-sop-video-and-media-render-cli", "gap_workflow-sop-video-and-media-pi-video-sop-extension"],
   [AR + "workflow_video_cli.rs (new)", AR + "main.rs (argv table entry only)", AR + "pi.rs (SCREENPIPE_VIDEO_CLI env line)",
    AT + "assets/extensions/workflow-video.ts (new)", AT + "assets/skills/video-sop/SKILL.md (new, own text)", AR + "pi_extensions_manifest.rs (entry)"],
   ["W5-10", "W2-27"],
   "--render-workflow-video <dir> before single-instance: exclusive render.lock, reads render-scenes.json with caps, renders into "
   "rendered/, NDJSON progress on stdout, stdin EOF cancels, exit 0 only on success. Pi extension read/edit/render_video_sop over "
   "video-project.json with VideoDraft/VideoEdit validation and tool guards.",
   ["Handvel.exe --render-workflow-video <fixture dir> writes rendered/video.mp4 and NDJSON progress, exit 0; closing stdin cancels with non-zero exit",
    "bun test: VideoEdit validation limits (50 changes, pace range, focus requires image)"],
   ["specs/gap_workflow-sop-video-and-media.md render-cli, pi-video-sop-extension"])

wp("W6-10", 6, "M", "Workflow-to-skill drafting",
   ["agents_pipes-046"],
   [AR + "workflow_skill.rs (new)", AR + "skills.rs (workflow skill save region)"],
   ["W4-01", "W3-24", "W3-25"],
   "generate_workflow_skill/save_workflow_skill draft {name, description, instructions} from a reviewed workflow (>= 2 stages) with the "
   "user's preset (own prompt) and save it as an owned local skill linking to get-workflow. No sign-in gate.",
   ["Test with a mock preset: a 2-stage workflow yields a draft; a 1-stage workflow returns an error; saving writes the skill with the ownership marker"],
   ["specs/agents_pipes.md agents_pipes-046"])

wp("W6-11", 6, "L", "Built-in MCP server v0.20 and portable MCP launcher",
   ["agents_pipes-030"],
   ["packages/screenpipe-mcp/src/**", AR + "portable_mcp.rs (new)"],
   ["W4-01", "W2-03", "W5-01"],
   "Add tools synced-devices, search-synced-content, get-feedback, list-pipes, create-pipe, run-pipe, pipe-logs, team-frame (returns "
   "unavailable), list-workflows, get-workflow and screenpipe-skills; --screenpipe-portable-mcp runs the MCP server through bundled bun "
   "with SCREENPIPE_MCP_TRANSPORT/HOST/PORT/PATH.",
   ["cd packages/screenpipe-mcp && bun test; tools/list over stdio includes the new tool names",
    "Handvel.exe --screenpipe-portable-mcp with SCREENPIPE_MCP_TRANSPORT=http listens on SCREENPIPE_MCP_PORT"],
   ["specs/agents_pipes.md agents_pipes-030"])

wp("W6-12", 6, "XL", "Live Views dashboards UI",
   ["frontend-022"],
   [A + "components/library/live-views/** (new)", AR + "brain_views.rs (new)"],
   ["W4-11", "W5-16"],
   "BrainView/Slot/Canvas/TemplateKit model (live-view-canvas.v1, period policies, markdown/list/bar/line/timeline components) with "
   "revisioned saves, pipe-bound blocks, editor and React Flow canvas, AI composer with diff review (user's preset), card feedback and item "
   "actions, templates. Vendor share targets are omitted.",
   ["UI smoke: create a view from a bundled template, edit the canvas, save; a concurrent save prompts reload on 409",
    "bun test: canvas schema validation and period policy resolution"],
   ["specs/frontend.md frontend-022"])

wp("W6-13", 6, "L", "Activity view UI",
   ["meetings_memory-017", "frontend-023"],
   [A + "components/activity/** (new)", A + "app/(main)/home/page.tsx (activity section mount)", A + "components/settings/activities-section.tsx (new)"],
   ["W5-12", "W3-04"],
   "Range today/24h/7d/custom, AI preset picker and refresh; combines /activity-ledger, a slim /activity-summary and /meetings into "
   "day-grouped cards with evidence links to Meetings or Timeline; generated history with coverage and no-data reasons; settings for "
   "activitiesEnabled and interval.",
   ["UI smoke: the Activity section renders day-grouped cards for today; evidence icons open the timeline",
    "bun test: empty states for no_capture_in_range and empty_but_recording"],
   ["specs/meetings_memory.md meetings_memory-017", "specs/frontend.md frontend-023"])

wp("W6-14", 6, "M", "Recording status popover and first-run guide",
   ["frontend-007", "frontend-019"],
   [A + "components/recording-status-popover.tsx (new)", A + "components/app-sidebar.tsx (status trigger region)", A + "components/first-run-guide.tsx (new)",
    A + "app/(main)/home/page.tsx (guide mount)"],
   ["W3-30", "W2-11", "W4-22"],
   "Sidebar status popover with per-device pause (vision and audio device routes), pause/resume all and a low-storage warning from "
   "get_active_data_dir + get_disk_usage vs get_low_disk_guard_config; one-time three-step coach-mark guide (firstRunGuideDone).",
   ["UI smoke: pausing a monitor in the popover updates /vision/device/status; a mocked low threshold shows the warning",
    "The guide shows once and sets firstRunGuideDone"],
   ["specs/frontend.md frontend-007, 019"])

wp("W6-15", 6, "L", "Privacy settings v2",
   ["frontend-038"],
   [A + "components/settings/privacy-section.tsx", A + "components/settings/window-picker.tsx", A + "components/settings/website-rules-editor.tsx (new)"],
   ["W3-13", "W3-10", "W4-15"],
   "Apps/Websites tabs with category switches (categoryOwnedFilters), DomainRule editors for ignoredUrls/includedUrls, PII Off/Basic/Smart "
   "with labels, columns, pseudonyms and agent-log redaction, click capture, enhanced incognito, API key regenerate; piiBackend stays local "
   "unless an explicit endpoint is set; no remote-log toggle.",
   ["bun test: the zod schema accepts DomainRule objects and legacy strings and rejects invalid domains",
    "UI smoke: adding includedUrls example.com stops capture of other sites (frames show the skip reason)"],
   ["specs/frontend.md frontend-038"])

wp("W6-16", 6, "L", "Onboarding v2",
   ["frontend-034"],
   [A + "app/onboarding/page.tsx", A + "components/onboarding/**"],
   ["W5-01", "W4-22"],
   "Step flow permissions/timeline/engine/recommended-setup/first-task (vendor login, acquisition and plan steps dropped) with the legacy "
   "step map, trust disclosure with the data dir, low-tier screenshot choice, recommended pipes install via /pipes/bundled/:id/install with "
   "typed errors; completion opens the Home library.",
   ["UI smoke on a fresh store: onboarding completes without network access and installs a recommended pipe",
    "bun test: legacy currentStep values map to the new steps"],
   ["specs/frontend.md frontend-034"])

wp("W6-17", 6, "L", "Automations (pipes) UI v2",
   ["frontend-049"],
   [A + "components/settings/pipes-section.tsx", A + "components/pipe-store.tsx", A + "lib/hooks/use-pipes.ts", A + "components/pipes/** (new)"],
   ["W5-01"],
   "Schedule editor with /pipes/schedule/preview, preset chain, effort, run and chat destinations, per-pipe connections, history reset, "
   "one-chat mode and store auto-update; cloud-agent runner configuration is BYO; sharing exports a file.",
   ["UI smoke: editing a schedule shows next runs from the preview route and saves schedule_config to pipe.md",
    "bun test: use-pipes paging over /pipes pagination"],
   ["specs/frontend.md frontend-049"])

wp("W6-18", 6, "L", "Right sidebar: multi-tab owned browser, files, connections",
   ["frontend-013"],
   [A + "components/browser-sidebar.tsx", A + "components/right-sidebar/** (new)", AR + "owned_browser.rs", AR + "owned_browser_cookies.rs"],
   ["W5-15"],
   "Right panel with owned-browser tabs (owned_browser_tab_navigate/set_bounds/hide/close/clear_browsing_data, owned_browser_history, "
   "navigate owner/reveal), Files & outputs list from /artifacts and a Connections tab; cookie-access prompts kept.",
   ["UI smoke: open two browser tabs, go back/forward, close one; Files & outputs lists artifacts",
    "bun run bindings:check includes the owned_browser_tab_* commands"],
   ["specs/frontend.md frontend-013", "specs/connections.md owned-browser-tabs (merged)"])

wp("W6-19", 6, "L", "Shortcut overlay v2 (webview)",
   ["frontend-032"],
   [A + "app/shortcut-reminder/**", A + "components/shortcut-reminder.tsx", AR + "native_shortcut_reminder.rs (set_shortcut_overlay_anchor)"],
   ["W5-14", "W3-07", "W3-29"],
   "Recording-health incidents with restart/dismiss/quit-reopen, live meeting transcript from /ws/meeting-overlay with echo dedupe and "
   "stop, drag-to-anchor, one-hour snooze, star button and size scale.",
   ["UI smoke: during a manual meeting the overlay shows live transcript lines; stopping capture shows an incident with restart",
    "Dragging the overlay to the right edge persists the anchor"],
   ["specs/frontend.md frontend-032"])

# ---------------------------------------------------------------- wave 7
wp("W7-01", 7, "XL", "Storage migration lifecycle (SQLite to hybrid)",
   ["db-023"],
   [D + "storage/lifecycle.rs (new)", D + "storage/in_place.rs (new)", D + "storage/parity.rs (new)", D + "storage/diagnostics.rs (new)",
    AR + "storage_migration.rs (new)"],
   ["W6-06"],
   "User-initiated offline-copy or in-place conversion with journals (storage-migration.json, -complete.json, diagnostics, "
   "storage-migration-error.txt, pendingStorageMigration) and _storage_conversion_* tables; SHA-256 table parity, FTS parity, identity "
   "checks, activation, explicit delete-original and secrets recovery; Tauri commands get_storage_migration_status/activity, "
   "start/cancel_storage_migration, delete_original_storage_database, get_active_data_dir and the storage-migration-activity event.",
   ["cargo test: offline-copy migration of a fixture DB passes table and FTS parity and activates",
    "Cancelling midway then restarting resumes; delete_original_storage_database is refused before activation"],
   ["specs/db.md db-023", "specs/shell.md shell-015 (merged)"])

wp("W7-02", 7, "XL", "Hybrid compact, sparse reclaim, backup bundle and retention strip",
   ["db-024", "gap_retention-v2-engine-008"],
   [D + "storage/maintenance.rs (new)", D + "storage/reclaim.rs (new)", D + "storage/backup.rs (new)", D + "storage/sealing.rs (retention routine)",
    E + "retention.rs (hybrid dispatch line)"],
   ["W6-06", "W3-09"],
   "Offline compact via compacting.sqlite and storage-maintenance.json; in-place freelist reclaim by punching sparse holes under "
   "exclusive ownership after a FULL checkpoint; hybrid backup = VACUUM INTO + payload/bulk copies + storage.json + manifest.json. "
   "Retention on hybrid strips sealed detail by coordinated staged payload replacement with retry on generation change.",
   ["cargo test: compact shrinks a fixture hybrid DB; backup bundle contains manifest.json, storage.json and payload files",
    "Reclaim on NTFS leaves sparse ranges (fsutil sparse queryrange); lean retention on hybrid restages payloads (test)"],
   ["specs/db.md db-024", "specs/gap_retention-v2-engine.md 008"])

wp("W7-03", 7, "L", "SOP video project workspace and gate removal",
   ["gap_workflow-sop-video-and-media-video-project-workspace", "gap_workflow-sop-video-and-media-entitlement-and-telemetry"],
   [A + "components/workflows/video/** (new)", A + "lib/workflows/video-draft.ts (new)"],
   ["W6-07", "W6-09"],
   "Build video drafts from SOP sections (<= 50 sections, FNV-1a source hash invalidation), agent video project dirs under "
   "pi-workflows-guide/video-projects with caps and pruning, Pi session for edits, copy rendered output into workflow-videos. Narration "
   "depends only on the configured provider; no sign-in/Business checks, web SOP links or analytics.",
   ["bun test: draft builder respects caps and the source hash changes when the SOP changes",
    "UI smoke: generate a video from a workflow with the captions-only provider"],
   ["specs/gap_workflow-sop-video-and-media.md video-project-workspace, entitlement-and-telemetry"])

wp("W7-04", 7, "L", "Native Win32 shortcut overlay pill",
   ["shell-010"],
   ["crates/screenpipe-overlay-win/ (new crate)", AR + "native_shortcut_reminder.rs (Windows branch)"],
   ["W6-19"],
   "Layered Direct2D/DirectWrite pill fed by /ws/metrics and /ws/meeting-overlay with overlay/chat/star/search chips; falls back to the "
   "webview overlay on failure.",
   ["UI smoke (Windows): the native pill renders and its chips open chat/search/star",
    "Forcing D2D init failure (env) falls back to the webview overlay"],
   ["specs/shell.md shell-010"])

wp("W7-05", 7, "M", "Chat composer upgrades",
   ["frontend-011"],
   [A + "components/standalone-chat.tsx (composer region)", A + "components/chat/composer/** (new)", A + "lib/chat-utils.ts"],
   ["W6-01"],
   "Slash commands (/new, /stop, /inspector, /pipes, installed skills), extended ~ time tokens, filters popover (time, content, apps, tags, "
   "speakers, connections), dictation through local or BYO STT, large paste saved as a chunk file.",
   ["bun test: ~lastweek and ~(date) tokens expand; slash registry lists installed skills",
    "UI smoke: dictation transcribes with the local engine (no vendor /listen request)"],
   ["specs/frontend.md frontend-011"])

wp("W7-06", 7, "L", "First-run summary",
   ["meetings_memory-041", "frontend-018"],
   [AR + "first_run_summary.rs (new)", A + "components/chat/first-run-summary-banner.tsx (new)", A + "components/standalone-chat.tsx (banner mount line)",
    A + "lib/hooks/use-onboarding.tsx"],
   ["W5-12", "W6-01"],
   "A few minutes after setup, generate a short summary from /activity-summary with the user's preset (own prompt), notify with the "
   "first-run-summary deep link, update the OnboardingStore firstRunSummary* fields and show the learning banner with an ask-agent hand-off. "
   "Other producers (db recovery, disk pressure, workflow review) already route through /notify.",
   ["UI smoke on a fresh install with a mock preset: the summary notification arrives and opens the summary chat",
    "store.bin onboarding fields firstRunSummaryPhase/ChatId are set; no telemetry is emitted"],
   ["specs/meetings_memory.md meetings_memory-041", "specs/frontend.md frontend-018"])

wp("W7-07", 7, "L", "Agent worktrees and per-chat coding workspace",
   ["agents_pipes-020", "frontend-017"],
   [C + "agents/worktree.rs (new)", AR + "coding_workspace.rs (new)", A + "components/chat/coding-workspace-badge.tsx (new)",
    A + "components/standalone-chat.tsx (badge mount line)"],
   ["W6-01", "W6-04"],
   "Managed git worktrees for coding chats (start_worktree tool, coding_workspace_prepare/create/get) from HEAD with an "
   "agent-worktree-owner.json record, hooks and fsmonitor disabled, safe branch claiming, never auto-removed; header badge with the path.",
   ["Test in a temp git repo: coding_workspace_create makes a worktree with the owner record and core.hooksPath disabled",
    "UI smoke: the chat header shows the workspace badge"],
   ["specs/agents_pipes.md agents_pipes-020", "specs/frontend.md frontend-017"])

wp("W7-08", 7, "L", "Built-in pipe catalogue and builtin text migrations",
   ["agents_pipes-012"],
   [CA + "pipes/** (new built-ins, own prose)", C + "pipes/builtin_migrations.rs (new)", C + "pipes/mod.rs (install_builtin_pipes region)"],
   ["W6-03", "W4-01", "W3-06"],
   "Ship the 21 built-ins (workflow discovery/enrichment agents, skill-learning, automate-my-work, missed-todos, speaker reconciliation, "
   "commitments, accounting follow-through, meeting summary, ...) with the observed frontmatter and freshly written prompts; drop "
   "ai-prompt-journal; idempotent builtin text migrations for installed copies; presets resolve to the user's presets.",
   ["Fresh data dir: `api /pipes` lists 21 built-ins and no ai-prompt-journal",
    "cargo test: running builtin migrations twice leaves installed copies unchanged after the first pass"],
   ["specs/agents_pipes.md agents_pipes-012"])

# ---------------------------------------------------------------- wave 8
wp("W8-01", 8, "XL", "Rich chat blocks and local feedback UI",
   ["frontend-012", "frontend-014", "meetings_memory-008"],
   [A + "components/chat/blocks/** (new)", A + "components/markdown.tsx", A + "lib/chat/tool-presentation.ts", A + "components/standalone-chat.tsx (block renderer region)",
    A + "components/notification-bell.tsx (rating controls)", A + "app/notification-panel/page.tsx (rating controls)"],
   ["W7-05", "W2-03"],
   "Typed blocks: 13 chart kinds, plans, structured output, rich results, tool activity, ask-user, agent-action (ACP permission), "
   "context usage, turn status, sandboxed HTML, citation and selected-text actions. Thumbs up/down with optional comment on chat replies, "
   "pipe notifications and Live View cards posting to /feedback.",
   ["bun test renders each chart kind from fixture JSON and the ask-user card round-trip",
    "UI smoke: thumbs-down with a comment on a reply creates a feedback row with target.kind chat (GET /feedback)"],
   ["specs/frontend.md frontend-012, 014", "specs/meetings_memory.md meetings_memory-008"])

wp("W8-02", 8, "L", "Storage and retention settings UI with migration overlays",
   ["frontend-039", "frontend-044"],
   [A + "components/settings/storage-section.tsx", A + "components/settings/retention-settings.tsx", A + "components/settings/storage-migration-card.tsx (new)",
    A + "components/storage-migration-overlay.tsx (new)", A + "app/layout.tsx (overlay mount line)"],
   ["W7-01", "W7-02", "W3-09", "W4-10"],
   "Retention modes off/media/lean/all, erase recent, Compact DB (gated on 2x DB + 512 MiB free, calls /data/compact), low-disk guard "
   "toggle, storage migration card (start/cancel/finish/delete original) and the root-layout prompt and non-dismissable progress modal "
   "driven by storage-migration-activity, plus database-recovery deep links.",
   ["UI smoke: switching retention to lean configures the engine (/retention/status shows lean); Compact DB reports reclaimed bytes",
    "Starting a migration on a fixture data dir shows the progress modal until completion"],
   ["specs/frontend.md frontend-039, 044"])

wp("W8-03", 8, "L", "Tauri command/event surface audit, ACL and release readiness",
   ["frontend-056"],
   [A + "lib/utils/tauri.ts (generated)", A + "lib/events/types.ts", AT + "capabilities/*.json (ACL)", "docs/parity/command-audit.md (new)",
    "scripts/ci/command-audit.ts (new)"],
   ["W8-01", "W8-02", "W7-01", "W7-02", "W7-03", "W7-04", "W7-05", "W7-06", "W7-07", "W7-08"],
   "Audit the 112 target-new commands, 7 argument extensions and new events against the bindings: each is implemented or explicitly "
   "recorded as dropped (vendor-only); add ACL entries and the typed channels job:event, export:event, engine:event, notification:action; "
   "regenerate bindings; run the full test matrix and build the installer.",
   ["bun scripts/ci/command-audit.ts reports 0 unaccounted commands against specs/frontend.md",
    "bun run bindings:check, cargo test --workspace (release-dev profile) and bun test all pass; vendor guard passes",
    "Release-dev installer builds and a clean install records screen and audio for 10 minutes with /health healthy"],
   ["specs/frontend.md frontend-056", "specs/frontend.md frontend-054 (merged)"])


# ---- Restore wave items (owner decision 5, 2026-10-15): self-hostable / BYO replacements for
# ---- vendor-server-backed features. Server sides are original designs that satisfy the client
# ---- API surface recorded in the specs; nothing is ported from vendor servers.

wp("WR-01", 6, "L", "Pipe store on a GitHub-hosted registry",
   ["(restore) cloud-021", "(restore) agents_pipes-037"],
   ["registry/ (new: index.json, pipes/<id>/pipe.md, schema.json, README.md)", E + "routes/pipe_store.rs",
    E + "cli/pipe.rs", E + "cli/install.rs", A + "components/pipe-store.tsx", A + "components/settings/pipes-section.tsx",
    ".github/workflows/registry-validate.yml (new)"],
   ["W0-08", "W2-22", "W5-01"],
   "Make the pipe store work out of the box without vendor servers. Define a static registry format (index.json listing id, name, "
   "description, author, version, permissions, sha256, source path) served from raw.githubusercontent.com of normiecore/Freenpipe "
   "(registry/ directory on main) as the DEFAULT registry URL, overridable by SCREENPIPE_API_BASE_URL or a settings key. Implement "
   "list/search/detail/install/update against it with sha256 verification and the existing typed registry_* errors. 'Publish' "
   "produces a ready-to-PR pipe folder plus instructions (opens the GitHub new-PR page); a CI workflow validates registry PRs "
   "(schema, sha256, frontmatter, no vendor hosts). Seed the registry with the built-in pipe catalogue once W7-08 lands (until then, "
   "with 3 sample pipes written fresh).",
   ["Harness: /pipes/store lists the registry's pipes with no env set; install of a sample pipe verifies sha256 and the pipe runs",
    "A tampered sha256 yields the typed registry error; SCREENPIPE_API_BASE_URL override points the store at a local static server",
    "bun run build + UI smoke: store tab lists, installs and updates a pipe; publish produces a valid registry folder"],
   ["specs/cloud.md cloud-021", "specs/agents_pipes.md agents_pipes-037"])

wp("WR-02", 5, "L", "Sync and archive to bring-your-own storage",
   ["(restore) cloud-019", "(restore) cloud-037"],
   [C + "sync/ (backends: s3.rs, webdav.rs, folder.rs new)", "crates/screenpipe-sync/src/", AR + "sync.rs",
    A + "components/settings/sync-settings.tsx", A + "components/settings/archive-settings.tsx", E + "routes/data_sync_proxy.rs"],
   ["W0-08", "W4-10"],
   "Restore cross-device sync and cloud archive using storage the user controls: S3-compatible (AWS/R2/B2/MinIO), WebDAV "
   "(Nextcloud etc.) and a plain folder (e.g. inside OneDrive/Dropbox). Keep the base client-side encryption model (user passphrase "
   "-> key, encrypted chunks and manifests; the storage provider never sees plaintext), the existing sync manifest/merge semantics, "
   "LAN remote_sync, and the http_put target. Credentials live in the secret store. Archive moves aged media to the chosen backend "
   "per retention settings and can restore on demand. The UI exposes backend choice, test connection, passphrase setup and status.",
   ["cargo test for the sync crate against a local MinIO-compatible mock (or s3s fake) and a temp-folder backend: round-trip sync of two data dirs",
    "Ciphertext-only check: objects written to the backend contain no plaintext OCR text or audio",
    "UI smoke: configure a folder backend, sync, then archive and restore a chunk"],
   ["specs/cloud.md cloud-019, cloud-037"])

wp("WR-03", 6, "XL", "Handvel Hub: self-hostable team server and team client",
   ["(restore) cloud-022", "(restore) team memory"],
   ["crates/handvel-hub/ (new server crate + Dockerfile)", "crates/screenpipe-team-memory/", E + "cli/team.rs (restore + rewire)",
    A + "components/settings/team-section.tsx (restore + rewire)", A + "lib/hooks/use-team.ts (restore + rewire)",
    "packages/screenpipe-mcp/src/index.ts (TEAM_API -> hub URL)", "docs/hub.md (new)"],
   ["W0-05", "W3-02", "W4-10", "W5-16"],
   "Restore Team as a self-hostable product. New crate handvel-hub: a small axum server (single binary + Dockerfile, SQLite or "
   "Postgres) implementing the team API surface the client expects (recorded in specs/cloud.md cloud-022 and the base client code): "
   "org/team creation, invite links, seats and admin roles, per-member API tokens, opt-in sharing policies, shared team memory "
   "(reuse crates/screenpipe-team-memory), and admin query of teammates' shared history (search over data members explicitly "
   "publish; never raw unshared data). Members' engines push only what their sharing policy allows, end-to-end over TLS with "
   "per-member tokens. Client side: restore the team settings UI, the `screenpipe team` CLI and MCP TEAM_API from the MIT base "
   "history (W0-05 removed their vendor wiring) and point them at a configurable hub URL (settings + SCREENPIPE_TEAM_HUB_URL). "
   "No hub configured = Team hidden. Document deployment (Docker, Windows service) in docs/hub.md.",
   ["cargo test -p handvel-hub: invite/join/roles/token auth; a member's search does not return another member's unshared data",
    "Harness: hub on :4040 + two engines on isolated data dirs; member A shares memories, admin queries them via `screenpipe team search`",
    "UI smoke: team section joins a hub via invite link and lists members; with no hub URL the section is hidden"],
   ["specs/cloud.md cloud-022", "specs/meetings_memory.md (team memory, if present)"])

wp("WR-04", 7, "L", "Composio integrations (BYO key) and workflow sharing",
   ["(restore) cloud-038", "(restore) cloud-035"],
   [CN + "connections/composio.rs (new)", A + "components/settings/connections-section.tsx (Composio tile)",
    "crates/screenpipe-core/src/workflows/share.rs (new)", A + "components/workflows/share-dialog.tsx (new)",
    "registry/workflows/ (new gallery index)"],
   ["W2-20", "W6-07", "WR-01"],
   "Composio: restore the Composio-brokered integrations using the user's own Composio API key (stored in the secret store), calling "
   "Composio's public API directly; list available toolkits, connect accounts via Composio's hosted auth links, and expose connected "
   "tools to the agent like other connections. Workflow sharing: export a workflow (definition, steps, optional SOP video, redacted "
   "by default) as a signed .fpworkflow bundle, import it with a preview and permission review, and browse/publish to a gallery "
   "index under registry/workflows/ on normiecore/Freenpipe (publish = generated PR folder, like WR-01). Training-data contribution "
   "remains removed.",
   ["cargo test: Composio client against a wiremock server (toolkit list, connect link, tool call) with key from the secret store",
    "Round-trip: export a workflow, import on a fresh data dir, it runs; the bundle contains no unredacted PII by default",
    "UI smoke: gallery lists entries from the registry index; import shows the permission review"],
   ["specs/cloud.md cloud-035, cloud-038"])

wp("W1-05", 1, "M", "Licence gating for commercial builds",
   ["(owner decision 7) commercial-ready default builds"],
   ["crates/screenpipe-redact/Cargo.toml (opf-text, tinfoil features)", RD + "adapters/ (tinfoil and opf adapters behind features)",
    "crates/screenpipe-engine/Cargo.toml (feature flags)", E + "privacy_filter.rs (enclave path behind a feature)",
    E + "bin/screenpipe-engine.rs (opf fallback behind a feature)", AT + "Cargo.toml (feature flags)",
    "scripts/ci/licence-guard.test.ts (new)", ".github/workflows/freenpipe-ci.yml (gates step)", "third_party/README.md"],
   ["W0-13"],
   "Make every build the project ships commercially usable by default. opf-rs (CC BY-NC 4.0) and tinfoil (AGPL-3.0) move behind "
   "opt-in Cargo features that no default build and no release-app.yml feature set enables: screenpipe-redact `opf-text` and a new "
   "`tinfoil-enclave` feature, forwarded by screenpipe-engine and the app crate as opt-in features of their own. With the features "
   "off, the text PII worker skips the opf fallback (v45 ONNX, then regex), and the BYO-enclave paths (search filter_pii enclave, "
   "tinfoil text and image adapters) report 'not built with enclave support' instead of compiling the client. Add a CI gate that "
   "resolves the default and release feature sets of the engine and app crates and fails if any package licensed CC BY-NC, AGPL, "
   "GPL (other than LGPL), SSPL or without any licence declaration enters them, with a short commented allow-list for "
   "declared exceptions. Record both crates' status in third_party/README.md.",
   ["cargo tree for screenpipe-engine and the app crate with default and release features contains neither opf nor tinfoil",
    "cargo check of screenpipe-redact and screenpipe-engine passes both with and without the opt-in features",
    "the licence gate passes on main and fails on a branch that re-enables opf-text by default (negative test in the gate's tests)"],
   ["docs/clean-room/DECISIONS.md decision 7", "third_party/README.md"])

wp("W0-13", 0, "M", "Vendor git dependencies into third_party/",
   ["(owner decision 2) vendor git deps"],
   ["third_party/ (new)", "third_party/README.md (new)", "Cargo.toml ([patch] / path deps)", "Cargo.lock",
    "crates/*/Cargo.toml (git dep lines only)", AT + "Cargo.toml (git dep lines only)"],
   ["W0-11"],
   "Copy every git dependency hosted under the vendor's org or its contributors into third_party/<name>/ at its pinned revision "
   "(licence-checked; screenpipe-org revisions must predate 2026-06-10), switch to path deps / [patch], keep Cargo.lock consistent, and "
   "document name, upstream URL, revision, licence, date and local changes in third_party/README.md. Established third-party git deps "
   "may stay but are listed.",
   ["No git dependency under the vendor orgs remains in any Cargo.toml",
    "cargo build --profile release-dev -p screenpipe-engine and the app crate build succeed",
    "third_party/README.md lists every vendored and remaining git dependency with licence and revision"],
   ["docs/clean-room/DECISIONS.md decision 2"])
