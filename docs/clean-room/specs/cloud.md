# Freenpipe spec: cloud services, telemetry, licensing gates, enterprise hooks (dimension key: `cloud`)

Status: RE spec for clean-room implementation. Target = screenpipe v2.7.104 Windows build (static evidence only). Base = Freenpipe @ upstream 892199f with `ee/` removed.

Conventions used below:

- `0x<hex>` = offset of a row in `<re-workspace>/app_strings.tsv` (Rust packs literals together, so one row often holds several identifiers).
- `fe:<path>` = file under `<re-workspace>/frontend/` (Next.js static export).
- REA evidence: parent JS analysis `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`; module views used here: `ev_6d42880a2ed8c4b4a0d313d6f149cba208a5e0b3a84d9779d114bda2d7facb4f` (chunk 378: web-url helper, entitlement model, remote-control flags, settings defaults), `ev_419bce5b892daa19cd6f8244047b05c61837c6e32a7aefc2c0bd436443804f74` (chunk 9092: entitlement/login gate), `ev_73b6811f6b678fb495d52522f0aa9f088e1f77e911bc4445e7f5fee1b6a347ee` (chunk 9668: Workflows sharing), `ev_4ecb9db554c5111dd7ddd7ecf094a7d2ab860bb2c7570ea8d74c6ea13eae8eb7` (app/layout: remote-control applier, trial unlock), `ev_0ecd50e9ac156656f7f2af260fc931aef60f2278dc7815d2c2cf82d5744b8003` (chunk 961: enterprise policy/heartbeat), `ev_46bc6f414f9baaca64f92d089aee37621290580055d14430bae159403207e138` (settings page). REA's literal-seed traces (`ev_d510c5a5...`, `ev_f430af7c...`, `ev_8d65fb28...`) returned no-match because the graph did not retain string literals for these chunks; all URL/endpoint facts were therefore read directly from the chunk text and app_strings.
- Confidence: **observed** = literally present in target artifacts; **inferred** = reconstructed from surrounding identifiers/flow.
- Clean-room: behaviour is paraphrased. Identifiers (routes, JSON keys, settings keys, env vars, event names, command names, file names) are quoted exactly for interoperability. No code bodies, prose or prompt text were copied. No `ee/` source was read (it is absent).

Helper scripts written for this dimension (re-runnable): `<re-workspace>/tools/url_inventory.py`, `vendor_grep.py`, `cloud_sgrep.py`, `cloud_region.py`, `cloud_fegrep.py`, `cloud_reaview.py`. Intermediate outputs: `<re-workspace>/work/cloud/` (target_hosts.txt, target_urls.tsv, fe_urls.tsv, base_urls.tsv, base_vendor_hits.txt, t_*.txt).

---

## 1. Headline findings

1. The BASE is already heavily cloud-coupled: it embeds a PostHog project key (`phc_…`, 3 places) and two Sentry DSNs, has telemetry on by default, gates recording behind a paid "app entitlement" in release builds, gates OAuth integrations behind a Pro subscription, proxies OAuth token exchange through `screenpi.pe`, defaults AI to the vendor gateway `api.screenpipe.com/v1`, and polls `api.screenpipe.com/v1/usage` every 30 s even for anonymous users. All of this must be removed regardless of v2.7.104 parity.
2. The TARGET moves the vendor web origin from `screenpi.pe` to `screenpipe.com` (gateway stays `api.screenpipe.com`; a few legacy calls such as the OAuth broker and `syncstorage` still use `screenpi.pe`).
3. The TARGET adds several new phone-home channels that must NOT be reimplemented: remote support log collection (enabled by default), Workflows chat-sharing for model training, PostHog feature-flag "remote controls" that silently rewrite user settings and restart the recorder, a trial/paywall onboarding experiment, an account-refresh startup gate, hosted-AI allowance/quota gates, a free-plan pipe limit, Composio-brokered Gmail/Zoom/Google Drive/Docs/Sheets, cloud "data sync", and MCP-tool-call telemetry routes.
4. Every `ee/` reference in the base can be satisfied with tiny original stubs (section 5); none of the callers need real enterprise behaviour.

---

## 2. Remote host inventory (base vs target)

Classes: **V** = vendor-operated (screenpipe / Mediar), **VS** = SaaS used under the vendor's account (telemetry, enclave, broker), **3P** = third-party API the *user* talks to with their own account/key, **DL** = model/asset download, **L** = links only (opened in browser), **I** = local.

| Host / URL root | Class | Base use (files) | Target use (evidence) | Freenpipe disposition |
|---|---|---|---|---|
| `https://screenpi.pe` (+`/api/*`) | V | everywhere: `/api/oauth/exchange`, `/api/oauth/callback`, `/api/sync`, `/api/cloud-sync/{checkout,subscription}`, `/api/subscription/checkout`, `/api/referral[/invite]`, `/api/changelog`, `/api/logs[/confirm]`, `/api/attribution`, `/api/team/*`, `/api/enterprise/{policy,heartbeat,v1/*}`, `/api/pipes/{share,store}`, `/api/app-update/*`, `/survey`, `/download`, `/team`, `/enterprise` | still used for `/api/oauth/exchange` (0x5186da1), `/api/oauth/callback` (0x9fb584a), `/api/oauth/mcp/` (0x6a8dd01), `/api/syncstorage` (0xa0e5958), `SCREENPIPE_API_BASE_URL` default (0x6a92f80), onboarding fallback (fe chunk 3696) | remove all calls (cloud-009..024) |
| `https://screenpipe.com` (+`/api/*`) | V | login (`/login`), `/account`, `/billing`, `/onboarding`, `/changelog`, `/ideas`, `/security`, `/join/…`, versions page | primary web origin: `/login`, `/api/user`, `/api/app-update/{channel}/…`, `/api/app-update/rollback/`, `/api/attribution`, `/api/download`, `/api/composio[/status,/mcp]`, `/api/trajectories`, `/api/sops[/access]`, `/api/user/data-sync[/ingest,/mcp,/…]`, `/api/user/log-requests`, `/api/subscription/{checkout,offer,change-plan}`, `/api/billing`, `/api/cloud-sync/subscription`, `/api/referral/invite`, `/api/team[/billing/add-seat,/billing/seat-invite]`, `/api/enterprise/{policy,heartbeat,cloud-runner,member-workflows,v1/workflows/generated}`, `/api/pipes/share`, `/api/changelog?limit=50`, pages `/onboarding/checkout`, `/account/billing`, `/account/workspace`, `/team-dashboard`, `/enterprise?tab=builds|tokens`, `/survey`, `/ideas`, `/privacy`, `/integrations/grokbot` | remove; replace user-facing links with GitHub (cloud-023) |
| `https://api.screenpipe.com/v1` (`api.screenpi.pe`) | V (Cloudflare worker = base `packages/ai-gateway`) | `/v1/chat/completions`, `/v1/models`, `/v1/usage`, `/v1/listen`, `wss://…/v1/realtime`, `/v1/web-search`, event classifier | same plus `/workflow-voice`, `/listen?language=multi`; base URL now returned by Tauri command `get_screenpipe_ai_gateway_url` | BYO OpenAI-compatible / Ollama (cloud-016, cloud-017, cloud-042) |
| `us.i.posthog.com` (`/capture/`, SDK also `app.posthog.com`, `us.posthog.com`) | VS | posthog-js init (`app/providers.tsx:128`), Rust `analytics.rs` (app + engine), `resource_monitor.rs:335` | same + feature flags used as remote controls; key at 0x517714c, 0x6a78218, 0x6ab8c68 | remove (cloud-010, cloud-033) |
| `*.ingest.us.sentry.io` (org o4505591122886656) | VS | `main.rs:509` (app DSN), `screenpipe-engine.rs:482` (CLI DSN), `tauri-plugin-sentry`, `@sentry/react` | app DSN at 0x49f5158/0x52d98a0/0x5a66078; Sentry browser SDK 10.8.0 bundle at 0x618c7e0 (its built-in `o447951` URL at 0x61d06d4 is the SDK's own diagnostic, not a vendor DSN) | remove (cloud-011) |
| `pii.screenpipe.containers.tinfoil.dev` (+`kds-proxy.tinfoil.sh`, `github-proxy.tinfoil.sh`, sigstore `fulcio/rekor/ctfe/timestamp`) | VS | `screenpipe-redact/src/adapters/tinfoil.rs:76`, `tinfoil_image.rs` | same (0x498c790, 0x527a1a0, 0x5278cc8); attestation repo `screenpipe/privacy-filter` | local backend default; Tinfoil only with user-supplied enclave+key (cloud-020) |
| Composio (via `screenpipe.com/api/composio`) | VS | absent | Gmail, Zoom, Google Drive/Docs/Sheets (fe chunk 5977 @391921; 0x52a6925) | BYO Google/Zoom OAuth (cloud-038) |
| Clerk / Stripe (behind `screenpipe.com`) | VS | `user.clerk_id`, `stripe_*` fields; checkout via web | same + `destination_type: stripe_checkout` in checkout body | keep fields for schema compat only; no calls (cloud-044) |
| `github.com/screenpipe/screenpipe/raw/refs/heads/main/crates/screenpipe-audio/models/pyannote/{segmentation-3.0.onnx,wespeaker_en_voxceleb_CAM++.onnx}` | DL (vendor repo main) | `screenpipe-audio/src/speaker/models.rs:95` | 0x6b1fe98 | point to Freenpipe repo (cloud-024) |
| `raw.githubusercontent.com/screenpipe/screenpipe/main/apps/screenpipe-app-tauri/src-tauri/skills-registry.json` | DL | absent | 0x4995d13 (`fetch_skills_registry`) | point to Freenpipe repo (cloud-024) |
| `huggingface.co/screenpipe/{pii-redactor,pii-text-redactor,pii-image-redactor}` | DL (vendor HF org) | `server_core.rs:660/814`, `screenpipe-engine.rs:1875` | 0x49ad288, 0x49ace18, 0x49ad42c (`rfdetr_v38.onnx`, `v50_distilled6l`, `/resolve/v6`) | keep opt-in, make base URL configurable (cloud-024) |
| `huggingface.co` (whisper `ggerganov/whisper.cpp`, `istupakov/parakeet-tdt-0.6b-v3-onnx`, `Qwen/Qwen3-ASR-0.6B`), `hf-mirror.com` (opt-in `useChineseMirror` → `HF_ENDPOINT`) | DL (3P) | `main.rs:1151`, `server_core.rs:94` | 0x517714c | keep |
| `github.com/snakers4/silero-vad/raw/master/...silero_vad.onnx` | DL (3P) | `vad/silero.rs:128` | 0x49a65b0 | keep |
| `github.com/git-for-windows/...PortableGit-2.47.1-64-bit.7z.exe`, `github.com/oven-sh/bun/.../bun-v1.3.10/bun-windows-x64-baseline.zip` | DL (3P) | pi.rs PortableGit, pre_build.js bun | 0xa0b837f, 0xa0b629b | keep (third-party binaries) |
| npm `@screenpipe/cli-win32-x64` | DL (vendor npm) | absent | 0xa0c6a50 (`cli_runtime.rs`, `screenpipe-cli-refresh`) | use bundled sidecar (cloud-046) |
| `registry.modelcontextprotocol.io` | 3P | absent | 0x6a8dc90 (`io.modelcontextprotocol.registry/official`) | keep (MCP dimension) |
| `api.openai.com`, `auth.openai.com` (ChatGPT OAuth), `chatgpt.com/backend-api`, `api.anthropic.com`, `generativelanguage.googleapis.com`, `integrate.api.nvidia.com`, `api.perplexity.ai`, `localhost:11434` (Ollama) | 3P | presets, `chatgpt_oauth.rs` | 0x4992a28 (`anthropic-byok`, `openai-byok`, `$CUSTOM_API_KEY`, `$ANTHROPIC_API_KEY`, `$OPENAI_CHATGPT_TOKEN`, `$OPENAI_API_KEY`) | keep: these are the BYO replacements |
| `api.deepgram.com/v1/listen` (+`wss://`) | 3P | `deepgram/mod.rs:9`, `meeting_streaming/config.rs:11` | same | keep as BYO key option |
| Connections (slack, notion, linear, atlassian, asana, monday, clickup, airtable, intercom, stripe, sentry, posthog, hubspot, calendly, zoom, quickbooks, todoist, toggl, trello, zendesk, glean, pocket, granola, limitless, fireflies, otter, readwise, telegram, pushover, ntfy, resend, loops, vercel, brex, bitrix24, pipedrive, workflowy, mochi, financialsense, n8n, make, zapier, microsoft graph, google APIs, Bee `auth.beeai-services.com`, Grok Bot…) | 3P | `crates/screenpipe-connect/src/connections/*` | 0x9f8xxxx–0x9fbxxxx | keep; only the OAuth broker part is de-clouded (cloud-018) |
| `api.gtx.dev`, `cdn.gtx.dev` (General Translation i18n SDK) | 3P | absent | fe chunk 3188 (SDK constants; runtime translate client takes an API key) | inferred dev-only; ensure locales are bundled and no runtime fetch (open question) |
| `discord.com/invite/screenpipe`, `cal.com/team/screenpipe/chat`, `docs.screenpi.pe`, `support@screenpi.pe` | L | tray, overlay, onboarding, home | fe home/onboarding/overlay chunks | replace with Freenpipe GitHub links (cloud-023) |
| `ssl.com`, `crls/ocsps.ssl.com` | n/a | Authenticode signing chain inside the PE | 0xa8acc54… | irrelevant (signature metadata) |
| `localhost:3030`, `127.0.0.1:11435` | I | engine API, notification server | same | keep |

---

## 3. Base-vs-target inventory by subsystem

| Subsystem | Base (Freenpipe today) | Target v2.7.104 | Delta ids |
|---|---|---|---|
| ee/ path includes | `src-tauri/src/enterprise_{policy,install_metadata,sync}.rs` `#[path]` into missing `ee/desktop-rust/*`; `tests/enterprise_sync_test.rs`; `@ee/lib/app-update-policy` in 3 TS files; `@ee` alias in `tsconfig.json`/`vitest.config.ts`; `ee/sdk` workspace exclude; CI `sdk.yml`, `sdk-release.yml`, `release-enterprise.yml` | n/a (proprietary) | cloud-001..005 |
| Central vendor URL config | scattered constants + `lib/web-url.ts` (`NEXT_PUBLIC_SCREENPIPE_WEB_URL`) + `SCREENPIPE_API_BASE_URL` (pipe store) | `_p(path, base)` helper with base `https://screenpipe.com` (fe chunk 378 module 19686), engine `SCREENPIPE_WEB_URL` (0x6a9e4a8), `SCREENPIPE_API_BASE_URL` (0x6a92f80), command `get_screenpipe_ai_gateway_url` | cloud-009, cloud-042 |
| Telemetry (PostHog) | posthog-js + Rust capture, key embedded, `analyticsEnabled` default true, attribution fetch | same + `settings_changed`, `is_telemetry_disabled_by_env`, `SCREENPIPE_DISABLE_TELEMETRY`/`CI`/`GITHUB_ACTIONS`, `/internal/telemetry/mcp-*`, piggyback/migration/first-run telemetry, `screenpipe-telemetry-wire` crate | cloud-010, cloud-040 |
| Crash reporting (Sentry) | app + CLI DSNs, tauri-plugin-sentry | same DSN, Sentry browser SDK 10.8.0 | cloud-011 |
| Updater | Tauri updater → `screenpi.pe/api/app-update/{stable,beta,enterprise}/…`, rollback endpoint, auth header, source-build dialog | → `screenpipe.com/api/app-update/{stable,pre-release}/…`, `restart_for_update`, update-attempt marker, `update-install.log`/`update-recovery.log`, enterprise persistent updater | cloud-012, cloud-041 |
| Account / login | `open_login_window`, deep link `screenpipe://login?api_key=`, POST `/api/user`, auth interceptor, `set_cloud_token`/`get_cloud_token`, `~/.screenpipe/auth.json` | same + `startup_auth.rs` account-refresh worker and `SCREENPIPE_STARTUP_AUTHENTICATION_STATUS` | cloud-013, cloud-030 |
| License / entitlement gate | `app_entitled_or_dev` blocks auto-start and `spawn_screenpipe` in release builds; `app-entitlement.ts`, `app-entitlement-gate.tsx` | stricter model (plan set, verified-paid/free, `has_payment_method`, `entitlement_source`, `enterprise_account`), account access needed even for the free plan, trial read-only server | cloud-014, cloud-031, cloud-032 |
| Pro/Business gates | OAuth Pro gate, `screenpipe-cloud` STT Pro gate, calendar connect `isPro`, recording-settings "(pro)" | plus `workflow_business_required`, `free_pipe_limit_reached`, hosted-AI allowances | cloud-015, cloud-039 |
| Cloud AI | `screenpipe-cloud` provider default, `/v1/models`, `/v1/usage` poll, cloud proxy route `/v1/chat/completions`, web-search extension, classifier | same + gateway-URL command, `/v1/web-search` local proxy route, `/workflow-voice`, model auto-upgrades for Pro | cloud-016, cloud-042 |
| Cloud STT | `screenpipe-cloud` engine (`/v1/listen`), realtime `wss://…/v1/realtime`, BYO Deepgram | same + quota codes `daily_cost_limit_exceeded`, `transcription_capacity_paused`, `account_not_in_good_standing` | cloud-017 |
| OAuth broker | `screenpi.pe/api/oauth/exchange` (secrets server-side), HTTPS relay callback for slack/zoom/calendly/quickbooks | same + MCP OAuth relay `screenpi.pe/api/oauth/mcp/{id}/callback?app_scheme=` | cloud-018 |
| Cloud sync / archive | `screenpi.pe/api/sync/{init,upload,download,search,status,devices,data}`, cloud-sync checkout, archive | + `/api/syncstorage` presigned uploads, new "data sync" (`/api/user/data-sync/*`, engine proxy `/data-sync/{devices,search}`) | cloud-019, cloud-037 |
| PII enclave | Tinfoil adapters, `piiBackend` `local`/`tinfoil` | same; Workflows sharing forces `tinfoil` | cloud-020 |
| Pipe store / sharing | engine `pipe_store.rs` → `SCREENPIPE_API_BASE_URL` (default `https://screenpi.pe`); `/api/pipes/share` | same | cloud-021 |
| Team / enterprise cloud | `use-team.ts`, `screenpipe-team-memory`, CLI `team`, MCP `TEAM_API`, `use-enterprise-policy.ts` (policy+heartbeat) | + account-auth enterprise, host identity hashes, seat/recording-disabled responses, cloud-runner, member-workflows, SOPs, managed skills | cloud-022, cloud-043 |
| Feedback / support | `share-logs-button.tsx` uploads logs, screenshot, video to `/api/logs` presigned URLs | moved to Rust `feedback_upload.rs` (`start_feedback_upload`, `redact_pii_for_feedback`); new remote support log collection | cloud-036, cloud-034 |
| Training-data sharing | absent | Workflows sharing `/api/trajectories` | cloud-035 |
| Remote controls | `isFeatureEnabled("cloud-sync")` only | 8 PostHog flags rewrite settings | cloud-033 |
| Billing UI | links to web checkout | embedded checkout window, offer experiment, change-plan, seats, `purchase-successful` deep link | cloud-044 |
| Misc vendor links/endpoints | changelog API, referral, survey, usage chip, share pipe, discord, cal.com, docs | same on `screenpipe.com` | cloud-023 |
| Model/asset hosting | pyannote from vendor repo main; HF vendor org | + skills registry, npm CLI refresh | cloud-024, cloud-046 |

---

## 4. Freenpipe de-cloud architecture (applies to every item)

1. **One switchboard, empty by default.** Add a small module on each side:
   - Rust (new crate-free module, e.g. `crates/screenpipe-core/src/endpoints.rs`, re-exported to the app and engine): functions returning `Option<String>` for `web_base()`, `ai_gateway_base()`, `pipe_registry_base()`, `updater_manifest_url(channel)`, `model_mirror_base()`. Each reads a settings value or env var and returns `None` when unset. No vendor default anywhere.
   - TS (`apps/screenpipe-app-tauri/lib/web-url.ts`): keep the exported names `screenpipeWebBase` / `screenpipeWebUrl` so the ~20 call sites compile, but make them return `null` unless `NEXT_PUBLIC_SCREENPIPE_WEB_URL` is set at build time; callers must hide the related UI when the result is null. Add `freenpipeLink(kind)` for GitHub links (releases, issues, discussions, docs).
   - Env vars honoured for compatibility with scripts/pipes: `SCREENPIPE_WEB_URL`, `SCREENPIPE_API_BASE_URL`, `SCREENPIPE_EVENT_CLASSIFIER_URL`, `SCREENPIPE_MEETING_REALTIME_URL`, `SCREENPIPE_MEETING_DEEPGRAM_LIVE_URL`, `SCREENPIPE_PRIVACY_FILTER_{ENCLAVE,URL,REPO,API_KEY}`, `HF_ENDPOINT`. Each defaults to "feature disabled", never to a vendor host.
2. **Keep the data shapes.** Settings keys and `User`/entitlement JSON fields stay in `store.bin` schemas (`user.{id,name,email,image,token,clerk_id,api_key,credits,stripe_connected,stripe_account_status,github_username,bio,website,contact,cloud_subscribed,credits_balance,app_entitled,subscription_plan,entitlement,enterprise_account}`), so a screenpipe `store.bin` loads unchanged. Freenpipe simply never populates or trusts them.
3. **Gates collapse to "allowed".** Every license/plan/quota check becomes a constant-true helper with the same name and signature, so call sites need no restructuring.
4. **Network allowlist.** Remove `https://api.screenpipe.com/*` and `https://api.screenpi.pe/*` from `src-tauri/capabilities/main.json` `remote.urls`; keep `https://*` (needed for BYO endpoints) and localhost. Change outbound user agent `screenpipe-app (+https://screenpipe.com)` (0x5226f10) to a Freenpipe UA.
5. **CI guard (recommended).** Add a test (Rust `#[test]` or a `bun test`) that greps built sources for `screenpi.pe`, `screenpipe.com`, `posthog.com`, `sentry.io`, `phc_`, `tinfoil.dev` outside comments/tests and fails if any request-building code still references them.

---

## 5. ee/ reference inventory (complete) and stub interfaces

Grep of the base for `ee/`, `@ee`, `enterprise_policy`, `enterprise_install_metadata`, `enterprise_sync`, `enterprise-build`:

| Reference | Location | Caller needs |
|---|---|---|
| `#[path = "../../../../ee/desktop-rust/enterprise_policy.rs"]` | `apps/screenpipe-app-tauri/src-tauri/src/enterprise_policy.rs:9` | see cloud-001 |
| `#[path = ".../enterprise_install_metadata.rs"]` | `src-tauri/src/enterprise_install_metadata.rs:9` | see cloud-002 |
| `#[cfg(feature="enterprise-build")] #[path = ".../enterprise_sync.rs"]` + `imp` module | `src-tauri/src/enterprise_sync.rs:19-555` | see cloud-003 |
| `#![cfg(feature="enterprise-build")] #[path = ".../enterprise_sync.rs"]` | `src-tauri/tests/enterprise_sync_test.rs` | delete |
| `mod enterprise_install_metadata; mod enterprise_policy; mod enterprise_sync;` + `pub use …::{get_enterprise_install_metadata, set_enterprise_policy, set_sync_streams}` + `.typ::<enterprise_install_metadata::EnterpriseInstallMetadata>()` | `src-tauri/src/main.rs:45-47, 110-112, 361` | keep; stubs provide the symbols |
| `enterprise_policy::is_app_ui_hidden()` | `main.rs:882, 934, 1307, 1858, 1977`; `window/show.rs:407`; `window/panel.rs:84`; `commands.rs:938`; `tray.rs:6,117,913,944` | `fn() -> bool` |
| `enterprise_policy::is_tray_item_hidden(id)` with ids `tray_chat`, `tray_search`, `tray_timeline`, `tray_plan`, `tray_recording_controls`, `tray_settings` | `tray.rs:612-845` | `fn(&str) -> bool` |
| `enterprise_install_metadata::get_enterprise_install_metadata()` | `updates.rs:142` (reads `.managed`) | plain sync fn |
| `enterprise_sync::spawn(&app_handle)` | `main.rs:1817` | `fn(&AppHandle) -> Option<watch::Sender<bool>>` |
| `cfg!(feature = "enterprise-build")` | `updates.rs:115,120`, `tray.rs:582,750` | feature removal → always false |
| Cargo feature `enterprise-build = ["dep:thiserror", "dep:screenpipe-sync"]`, optional deps `thiserror`, `screenpipe-sync`, dev-deps comment for `tempfile`/`wiremock`/`serial_test`, cargo-machete note | `src-tauri/Cargo.toml:153-165, 260-266, 292-305` | remove feature + optional deps (keep dev-deps only if other tests use them) |
| `import … from "@ee/lib/app-update-policy"` | `components/settings/general-settings.tsx:35-39`; `lib/hooks/use-enterprise-policy.ts:23-28`; `lib/hooks/use-settings.tsx:19-22` | see cloud-004 |
| `"@ee/*": ["../../ee/desktop/*"]` | `apps/screenpipe-app-tauri/tsconfig.json:29-30` | remove |
| `"@ee": path.resolve(__dirname, "../../ee/desktop")` | `apps/screenpipe-app-tauri/vitest.config.ts:61` | remove |
| `exclude = [..., "ee/sdk"]` | root `Cargo.toml:8` | remove entry |
| `ee/sdk` | `.github/workflows/sdk.yml`, `sdk-release.yml`; `.gitignore:44`; `README.md:118` | delete workflows; drop gitignore line; rewrite README link |
| `--features enterprise-build` | `.github/workflows/release-enterprise.yml:211,482` | delete workflow |
| comments only | `crates/screenpipe-sync/src/destination/http_put.rs:31`, `crates/screenpipe-sync/README.md:28`, `crates/screenpipe-team-memory/src/lib.rs:69` | optional comment cleanup |
| generated bindings | `lib/utils/tauri.ts:547-548` (`getEnterpriseInstallMetadata`), `:1786` (`setEnterprisePolicy`), `:1821` (`setSyncStreams`), `:2219` (`EnterpriseInstallMetadata` type) | regenerate with `bun run bindings:generate` after stubbing |

---

## 6. Delta items

### cloud-001 (ee-stub) `enterprise_policy.rs` original stub

- **Base touchpoints:** `src-tauri/src/enterprise_policy.rs` (replace the `#[path]` re-export with original code), callers listed in section 5.
- **Interface required by callers (derived from base call sites and `lib/utils/tauri.ts`):**
  - `pub fn is_app_ui_hidden() -> bool` — headless/hidden-UI enterprise mode. Freenpipe: always `false`.
  - `pub fn is_tray_item_hidden(item_id: &str) -> bool` — Freenpipe: always `false`.
  - `#[tauri::command] #[specta::specta] pub fn set_enterprise_policy(hidden_sections: Vec<String>)` — returns unit (TS binding is `Promise<void>`, non-Result). Freenpipe: no-op (may store the list in a `Mutex<Vec<String>>` and make `is_tray_item_hidden` consult it, which keeps the stub honest if anyone calls it).
  - `#[tauri::command] #[specta::specta] pub fn set_sync_streams(frames: bool, audio: bool, ui_events: bool, memories: bool, snapshots: bool)` — unit return; TS arg names `frames, audio, uiEvents, memories, snapshots`. Freenpipe: no-op.
  - Both commands must carry `#[tauri::command]` **and** `#[specta::specta]` and stay re-exported from `main.rs` (`pub use enterprise_policy::{set_enterprise_policy, set_sync_streams}`) because `tauri_helper::generate_command_file` (build.rs:322) and `specta_collect_commands!` resolve them at crate root.
- **Target additions in the same family (observed in command list 0x5177a68):** `apply_enterprise_ui_visibility` (no args, unit), `set_enterprise_recording_authorized`, `get_enterprise_host_identity` (returns `{machine_id_hash, os_user_id_hash}` used in heartbeat, fe chunk 961), `sync_managed_team_skills`. Implement as no-op/default-returning commands only if the reimplemented frontend calls them (see cloud-043).
- **Confidence:** observed (callers), inferred (exact return types beyond what bindings show).
- **Notes:** write the file from scratch with the Freenpipe header; do not keep the upstream comment that cites the EE license.

### cloud-002 (ee-stub) `enterprise_install_metadata.rs` original stub

- **Interface:**
  - `#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)] pub struct EnterpriseInstallMetadata { pub install_source: String, pub update_manager: String, pub managed: bool, pub detected_by: Vec<String> }` — field names fixed by `tauri.ts:2219` and the heartbeat body in base `use-enterprise-policy.ts:249-252`.
  - `#[tauri::command] #[specta::specta] pub fn get_enterprise_install_metadata() -> EnterpriseInstallMetadata` — non-Result (binding returns the struct directly) and must also be callable as a plain sync function (`updates.rs:142` reads `.managed`).
- **Freenpipe value:** `{ install_source: "unknown", update_manager: "unknown", managed: false, detected_by: [] }` (matches the frontend's own fallback in `use-enterprise-policy.ts:136-141`). Optional local enhancement: detect a winget/MSI install and report `update_manager: "winget"`; never sent anywhere.
- **Keep** `.typ::<enterprise_install_metadata::EnterpriseInstallMetadata>()` in `main.rs:361`.
- **Confidence:** observed.

### cloud-003 (ee-stub) `enterprise_sync.rs`, test target and Cargo feature

- **Change:** delete the `#[cfg(feature = "enterprise-build")]` `#[path]` module and the entire `imp` module (lines 19-555 of the base file; that code is MIT but only exists to feed the proprietary uploader and to POST telemetry to the vendor ingest). Keep only `pub fn spawn(_app: &tauri::AppHandle) -> Option<tokio::sync::watch::Sender<bool>>` returning `None` (unconditionally). `main.rs:1817` keeps compiling.
- Delete `src-tauri/tests/enterprise_sync_test.rs`.
- In `src-tauri/Cargo.toml`: remove the `enterprise-build` feature, the optional `thiserror` and `screenpipe-sync` deps (unless another feature uses them; `screenpipe-sync` is otherwise unused by the app), and the related cargo-machete comments. Replace `cfg!(feature = "enterprise-build")` in `updates.rs:115,120` and `tray.rs:582,750` with `false` (or keep `is_enterprise_build()` returning `false`).
- **Target context (not to implement):** v2.7.104 ships `screenpipe-telemetry-wire` (records for frames/audio/ui/memory/feedback/snapshot with `device_id`, `device_label`, `image_b64`, `mime`, `width`, `height`; 0x69fc328-0x69fca0a) and an enterprise ingest; Freenpipe drops this channel.
- **Confidence:** observed.

### cloud-004 (ee-stub) `@ee/lib/app-update-policy` → local `lib/app-update-policy.ts`

- **New file:** `apps/screenpipe-app-tauri/lib/app-update-policy.ts` (original code, Freenpipe header).
- **Exports required by the three importers:**
  - `type EnterpriseAppUpdatePolicy = { mode: "screenpipe" | "auto_detect" | "mdm" | "manual"; default_auto_update: boolean; allow_employee_override: boolean; channel: string }` — modes taken from the base Rust consumer `updates.rs:143-151`; fields from `use-enterprise-policy.ts:153-156, 253-258` and `general-settings.tsx:259-267`.
  - `type EnterpriseInstallMetadata` — re-export the generated type from `@/lib/utils/tauri` (same 4 fields as cloud-002) so there is one source of truth.
  - `const DEFAULT_ENTERPRISE_APP_UPDATE_POLICY: EnterpriseAppUpdatePolicy` — Freenpipe default `{ mode: "screenpipe", default_auto_update: true, allow_employee_override: true, channel: "stable" }` (in a non-enterprise build none of this is read; the value just needs to be valid).
  - `function normalizeEnterpriseAppUpdatePolicy(raw: unknown): EnterpriseAppUpdatePolicy` — accept any object, coerce each field with type checks, fall back per field to the default; unknown `mode` → `"screenpipe"`; lower-case the mode (the Rust side lower-cases too).
  - `function describeEnterpriseUpdateMode(policy: EnterpriseAppUpdatePolicy): string` — short human label per mode (write your own wording).
- **Rewire imports:** change `"@ee/lib/app-update-policy"` → `"@/lib/app-update-policy"` in `general-settings.tsx`, `use-enterprise-policy.ts`, `use-settings.tsx`; remove the `@ee/*` path from `tsconfig.json` and the `@ee` alias from `vitest.config.ts`.
- **Confidence:** observed for field usage; inferred for exact default values.

### cloud-005 (ee-stub) workspace and CI references to `ee/`

- Root `Cargo.toml`: drop `"ee/sdk"` from `[workspace].exclude`.
- Delete `.github/workflows/sdk.yml`, `sdk-release.yml`, `release-enterprise.yml`, `sentry-cleanup.yml` (vendor infra). Review `release-app.yml` separately under cloud-012.
- `.gitignore:44` comment, `README.md:118` link to upstream `ee/sdk`: remove/replace.
- **Confidence:** observed.

### cloud-009 (decloud) central endpoint switchboard

- See section 4. Concretely: introduce `endpoints` helpers (Rust + TS), migrate every vendor constant listed below to them, and make the "unset" path disable the feature with a clear UI state ("not configured") instead of an error loop.
- Vendor constants to migrate (base): `crates/screenpipe-connect/src/oauth.rs:971` `EXCHANGE_PROXY_URL`; `crates/screenpipe-engine/src/workflow_classifier.rs:66` `DEFAULT_CLASSIFIER_URL`; `packages/screenpipe-mcp/src/index.ts:227` `TEAM_API`; `crates/screenpipe-engine/src/routes/cloud_proxy.rs:23` `CLOUD_BASE_URL`; `crates/screenpipe-core/src/sync/client.rs:20` `DEFAULT_API_URL`; `crates/screenpipe-engine/src/cli/team.rs:35`; `cli/survey.rs:5`; `cli_reminder.rs:17-18`; `crates/screenpipe-core/src/agents/pi.rs:21` `SCREENPIPE_API_URL`; `src-tauri/src/suggestions.rs:1432`; `src-tauri/src/pi.rs:298`; `src-tauri/src/commands.rs:1506` `LOGIN_URL`; `crates/screenpipe-audio/src/meeting_streaming/config.rs:10`; `crates/screenpipe-audio/src/transcription/deepgram/mod.rs:37`; `crates/screenpipe-engine/src/routes/pipe_store.rs:30`; `crates/screenpipe-redact/src/adapters/tinfoil.rs:76`; `src-tauri/src/updates.rs:37,827,831,920,932`; `src-tauri/src/analytics.rs:68,86`; `crates/screenpipe-engine/src/analytics.rs:14-15`; `resource_monitor.rs:335`; frontend `overlay/page.tsx:260`, `share-logs-button.tsx:167`, `onboarding/engine-startup.tsx:486,499,741`, `rewind/region-ocr-overlay.tsx:154`, `rewind/ai-presets-selector.tsx:209`, `settings/ai-presets.tsx:909-910,1077`, `changelog-dialog.tsx:23`, `team-section.tsx:40`, `use-team.ts:16`, `use-usage-status.tsx:27`, `app-entitlement.ts:56`, `auth-guard.tsx:22`, `general-settings.tsx:152`, `update-banner.tsx` (hard-coded `screenpi.pe/api/app-update/stable/windows-${cpuArch}`), `account-section.tsx`, `sync-settings.tsx`, `archive-settings.tsx`, `recording-settings.tsx`, `connect-apps.tsx`, `referral-card.tsx`, `pipes-section.tsx`, `feedback-section.tsx`, `post-install-connections-modal.tsx`, `gmail-card.tsx`, `google-*-card.tsx`, `connections-section.tsx`, `calendar-connect-dialog.tsx`, `privacy-section.tsx` (enterprise tokens link), `app/settings/page.tsx` (referral).
- Target equivalents (for parity awareness): engine `SCREENPIPE_WEB_URL` default `https://screenpipe.com` (0x6a9e4a8), `SCREENPIPE_API_BASE_URL` default `https://screenpi.pe` (0x6a92f80), TS `_p(path, base)` with a base constant of `https://screenpipe.com`, honouring `NEXT_PUBLIC_SCREENPIPE_WEB_URL` (fe chunk 378 module 19686; REA `ev_6d42880a…`).
- **Confidence:** observed.

### cloud-010 (decloud) PostHog telemetry removal

- **Base behaviour (observed):** posthog-js initialised with an embedded project key and `api_host` `https://us.i.posthog.com` in `app/providers.tsx:128-152`; ~40 components call `posthog.capture(...)`; identity bridging (`alias` by `clerk_id`, `identify` with plan fields) in `use-settings.tsx:1091-1125, 1271`; Rust `src-tauri/src/analytics.rs` posts `/capture/` with the key passed from `main.rs:1092` and started at `main.rs:1672` (periodic feature/health events, `fetch_attribution` GET `screenpi.pe/api/attribution`); engine `crates/screenpipe-engine/src/analytics.rs` (`POSTHOG_API_KEY`, `POSTHOG_HOST`, `capture_event`, `track_api_usage`), `resource_monitor.rs:335` (resource usage to PostHog), `telemetry_context.rs` (env `SCREENPIPE_ANALYTICS_ID`, `SCREENPIPE_SUPPORT_ID`, `SCREENPIPE_TELEMETRY_ID`, `SCREENPIPE_CUSTOMER_ID`, `SCREENPIPE_ORG_ID`, `SCREENPIPE_TELEMETRY_CUSTOMER_ID`, `SCREENPIPE_DEPLOYMENT_ID`, `SCREENPIPE_TELEMETRY_DEPLOYMENT_ID`, `SCREENPIPE_EMBEDDER`, `SCREENPIPE_HOST_APP`, `SCREENPIPE_TELEMETRY_HOST_APP`, `SCREENPIPE_EMBEDDER_VERSION`, `SCREENPIPE_HOST_VERSION`, `SCREENPIPE_TELEMETRY_HOST_VERSION`, `SCREENPIPE_DISTRIBUTION`, `SCREENPIPE_DIST`). Settings default `analyticsEnabled: true` (`use-settings.tsx:536`). `storage-section.tsx:60` gates the cloud-sync UI on PostHog flag `cloud-sync`.
- **Target (observed):** same plus a `settings_changed` event that reports which settings keys changed and their value types/booleans (fe chunk 378 @32166), `telemetry` opt-in/out events, `desktop_upgrade_offer_*`, `app_entitlement_*`, `trial_activation_force_unlocked`, `desktop_remote_control_applied`; env kill switch `SCREENPIPE_DISABLE_TELEMETRY` (also auto-disabled when `GITHUB_ACTIONS` or `CI` is set; 0x6a7f678); Tauri command `is_telemetry_disabled_by_env` (0x5177a68; frontend helper `fy({analyticsEnabled, envDisabled})` in chunk 378 @36155). Settings UI copy: usage data linked to the account when signed in. Default `analyticsEnabled: !0` (true) in target (chunk 378 @16293).
- **Freenpipe implementation:**
  1. Delete posthog-js from `package.json`; add `lib/analytics.ts` exporting a no-op object with `capture`, `identify`, `alias`, `opt_in_capturing`, `opt_out_capturing`, `has_opted_out_capturing` (returns true), `get_distinct_id` (returns undefined), `isFeatureEnabled` (false), `getFeatureFlag` (undefined), `onFeatureFlags` (returns an unsubscribe no-op), `reloadFeatureFlags`; replace `import posthog from "posthog-js"` across components with this module, and `PostHogProvider`/`usePostHog` with trivial equivalents. This keeps call sites intact.
  2. Rust: replace `analytics.rs` (app) with a stub `start_analytics(...)` that returns immediately; remove the embedded key at `main.rs:1092`; engine `analytics.rs` → `init()` no-op, `capture_event*` no-op, `get_distinct_id()` returns a random per-install id that never leaves the machine; delete the PostHog post in `resource_monitor.rs` (keep local logging); keep `telemetry_context.rs` only if other modules read it (it is harmless env parsing).
  3. Add command `is_telemetry_disabled_by_env() -> bool` returning `true` (frontend parity).
  4. Settings: keep key `analyticsEnabled` but default `false` and remove the toggle (or show it disabled with "Freenpipe never sends telemetry"). `analyticsId` keeps working as a local id for `/notify` etc.
  5. `storage-section.tsx`: drop the `cloud-sync` flag gate.
- **Confidence:** observed.

### cloud-011 (decloud) Sentry removal

- **Base:** `sentry::init` with app DSN in `main.rs:509-540` (90-day self-expiring reports, `release screenpipe-app@x`), `tauri_plugin_sentry::init` at `main.rs:905`; CLI DSN in `crates/screenpipe-engine/src/bin/screenpipe-engine.rs:475-495` (`SENTRY_RELEASE_NAME_APPEND`, sample 0.1, traces 0.01); `@sentry/react`, `tauri-plugin-sentry-api`, `@sentry/cli` in `package.json`; `sentry` workspace dep.
- **Target:** app DSN still present (0x49f5158 etc.), Sentry browser SDK v10.8.0 injected (0x618c7e0); `screenpipe-secrets/src/telemetry.rs` tags secret-store failures for Sentry (0xa2d2040).
- **Freenpipe:** remove the init calls, the plugin, the npm packages and the `sentry` crate dependency; keep `tracing` file logs. If a crash hook is wanted, write panics to `~/.screenpipe/crash-*.log` locally only.
- **Confidence:** observed.

### cloud-012 (decloud) updater → GitHub releases of normiecore/Freenpipe

- **Base:** Tauri updater endpoints in `tauri.prod.conf.json:57-60` (`https://screenpi.pe/api/app-update/stable/{{target}}-{{arch}}/{{current_version}}`), `tauri.beta.conf.json` (`…/beta/…`), `tauri.enterprise.conf.json` (`…/enterprise/…`), shared vendor minisign `pubkey`; `updates.rs` adds `Authorization: Bearer <user.token>` (or `X-License-Key`) because the vendor R2 bucket is paid-only, maps 401/403 to `update-auth-required` and a prompt to sign in before updating; rollback via `https://screenpi.pe/api/app-update/rollback/{target_arch}/{version}` (fake-high-version manifest); source builds show a dialog linking `screenpi.pe/download` and the upstream releases page; what's-new notification links `screenpi.pe/changelog` and posts to local `127.0.0.1:11435/notify` with `type: "app-update"`; `update-banner.tsx` hard-codes the Windows endpoint.
- **Target:** endpoint moved to `https://screenpipe.com/api/app-update/{channel}/{{target}}-{{arch}}/{{current_version}}` (0x5a0e0c8 embedded config, 0x6203b1d runtime builder, packed next to a download-failure status message, + `/{{target}}-{{arch}}/{{current_version}}`), channels `stable` and `pre-release` (0x6203ada; settings select "Standard"/"Pre-release" in settings page), rollback `https://screenpipe.com/api/app-update/rollback/` (0x512841e), same pubkey string, events `update-available`, `update-auth-required`, `update-downloading`, `update-download-progress`, `update-failed`, `update-restarting`; tray item `tray-upgrade` → `https://screenpipe.com/changelog`.
- **Freenpipe implementation:**
  - Generate a new minisign key pair; put only the public key in `tauri.conf.json` `plugins.updater.pubkey`; keep the private key in GitHub Actions secrets (`TAURI_SIGNING_PRIVATE_KEY`).
  - Stable endpoint: `https://github.com/normiecore/Freenpipe/releases/latest/download/latest.json` (static Tauri manifest generated by the release workflow; `platforms["windows-x86_64"]` with `url` + `signature`). Pre-release endpoint: a fixed-tag asset, e.g. `https://github.com/normiecore/Freenpipe/releases/download/prerelease/latest.json`, re-uploaded by CI on each pre-release. Build the endpoint at runtime from settings key `updateChannel` (`"stable"` default; accept `"pre-release"`; map legacy `"beta"` → `"pre-release"`) via `updater_builder().endpoints(...)` the way `updates.rs` already does for rollback.
  - Remove every auth header and the `auth_required` / `update-auth-required` path (GitHub assets are public). Keep the event names (`update-available`, `update-downloading`, `update-download-progress`, `update-failed`, `update-restarting`) and `PendingUpdateSnapshot` (keep `auth_required` field, always false, for binding stability).
  - Rollback (`rollback_to_version`, target command name observed): fetch `https://github.com/normiecore/Freenpipe/releases/download/v{version}/latest.json`; because Tauri refuses downgrades, set `allowDowngrades`/version comparator on the builder (the target binary references `allowDowngrades`, 0x513198b) instead of the fake-version trick.
  - Release CI enables a renamed feature (`official-build` can be kept as the flag name) so `is_source_build()` is false for official Freenpipe releases; the source-build dialog links to the Freenpipe releases page only.
  - What's-new: link `https://github.com/normiecore/Freenpipe/releases/tag/v{version}`; keep the local `/notify` call (`type: "app-update"`).
  - Opt-out: `autoUpdate` setting stays; add a hard-off path when no endpoint is configured (forks).
- **Confidence:** observed (endpoints, events, channels), inferred (GitHub layout is a Freenpipe design choice).

### cloud-013 (decloud) vendor account login and cloud token

- **Base (observed):** `open_login_window` (`commands.rs:1477`) opens `https://screenpipe.com/login` (`commands.rs:1506`) in a webview and intercepts `screenpipe://…?api_key=` deep links; `deeplink-handler.tsx:49`; frontend POSTs `{token}` to `screenpipeWebUrl("/api/user")` and stores the returned user (normalised by `normalizeAppUser` in `app-entitlement.ts:211-244`); `auth-guard.tsx` installs a fetch interceptor that signs the user out on 401 from `screenpi.pe`/`screenpipe.com` hosts (excluding `api.*`) and emits `screenpipe-auth-signout`; `get_cloud_token`/`set_cloud_token` (`commands.rs:769,799`) mirror the token into `~/.screenpipe/auth.json` and into the engine (`AppState.cloud_token`, `server.rs:198-337`); CLI `auth login/logout/status`.
- **Target (observed):** identical shape; `/api/user` body `{token, verify: true}` (chunk 378 @33954); `openLoginWindow(null, "sign-up")` now takes two optional args (chunk 4656 @38315); store keys `auth.json`, `store.bin`, `/settings/user/token` (0x6a7f008); keychain-encrypted token (#3943 note, 0x51b2cf8).
- **Freenpipe:** remove the login entry points from onboarding, settings (`account-section.tsx`), tray, chat, `ai-presets.tsx`, `archive-settings.tsx`, `connect-apps.tsx`; keep the commands compiled for binding stability but make `open_login_window` return `Err("not available in Freenpipe")`; keep `set_cloud_token`/`get_cloud_token` (they also carry the engine's local `SCREENPIPE_API_KEY` plumbing for self-hosted gateways, see cloud-042) but never call `/api/user`. Remove `installAuthInterceptor` (it is a no-op once no vendor host is contacted). Keep `User` struct fields untouched for store compatibility.
- **Confidence:** observed.

### cloud-014 (decloud) app-entitlement license gate removal (BASE gate)

- **Base (observed):** release builds refuse to start capture unless `SettingsStore::app_entitled_or_dev()` (`store.rs:1283-1312`) is true: debug builds bypass; `user.cloud_subscribed == true` bypasses; else requires `user.entitlement` with `features.app` or `user.app_entitled`, and either lifetime (`plan`/`source == "lifetime"`), a future `grace_until`, or `checked_at` within 72 h (`APP_ENTITLEMENT_MAX_STALE_HOURS = 72`, skew 5 min, `store.rs:45-46`) and `active == true`. Enforcement points: `main.rs:1431-1436` (skip auto-start, set recording status `Paused`, emit `app-entitlement-required`), `recording.rs:74-81` (`subscription_required:` error) used at `recording.rs:399, 587, 965`. Frontend twin `lib/app-entitlement.ts` (`hasAppEntitlement`, `hasCloudEntitlement`, `needsAppEntitlementRefresh`, `planDisplayName`, constants `APP_ENTITLEMENT_MAX_STALE_MS`, `APP_ENTITLEMENT_CLOCK_SKEW_MS`, `PRICING_URL`, `E2E_FORCE_BILLING_GATE_KEY = "screenpipe_e2e_force_billing_gate"`, env `NEXT_PUBLIC_SCREENPIPE_FORCE_BILLING_GATE`, `NEXT_PUBLIC_SCREENPIPE_DEV_BILLING_BYPASS`, `NEXT_PUBLIC_SCREENPIPE_E2E`) and `components/app-entitlement-gate.tsx`, `components/onboarding/login-gate.tsx`.
- **Freenpipe:** `app_entitled_or_dev()` → always `true`; `require_app_entitlement` → always `Ok(())`; delete the auto-start guard branch (or leave it unreachable); never emit `app-entitlement-required`. TS: `hasAppEntitlement`/`hasCloudEntitlement`/`isDevBillingBypassEnabled` → `true`, `needsAppEntitlementRefresh` → `false`; `AppEntitlementGate` renders children directly; remove the login-gate onboarding step. Keep the type exports and `normalizeAppUser` for store compatibility. Update tests `app-entitlement*.test.ts(x)` and e2e `zz-app-entitlement-gate.spec.ts` (delete or invert).
- **Confidence:** observed.

### cloud-015 (decloud) Pro/subscription feature gates in BASE

- `src-tauri/src/oauth.rs:81-90`: OAuth connect refused unless `user.cloud_subscribed == true` → delete the check.
- `store.rs:1314+` `audio_engine_resolution()`: `screenpipe-cloud` engine falls back with `NotLoggedIn`/`NotSubscribed` → in Freenpipe the `screenpipe-cloud` engine id is unavailable unless a self-hosted gateway is configured (cloud-017); keep `MissingDeepgramKey` fallback; keep the `AudioEngineFallbackReason` enum and `notify_audio_engine_fallback` notifications but reword them (no "log in").
- `components/settings/recording-settings.tsx:214, 1876, 2199, 2554-2555` (`(pro)` label and disabled option), `meeting-notes/calendar-connect-dialog.tsx:311` (`isPro`), `sync-settings.tsx:986-1029`, `ai-presets.tsx` (cloud checkout link), `tray.rs:722-735` (`tray_plan`, "Upgrade to Business", `plan_display_name`) and `tray.rs:1229-1320` (upgrade/changelog/download/cal.com handlers): remove plan UI and upgrade actions.
- **Confidence:** observed.

### cloud-016 (decloud) vendor AI gateway (`screenpipe-cloud` provider) → BYO

- **Base:** default AI preset provider `screenpipe-cloud`, url `https://api.screenpipe.com/v1` (`store.rs:791, 1034-1053`; `use-settings.tsx:446-467`); model list from `/v1/models` (`ai-presets.tsx:1077`, `ai-presets-selector.tsx:209`, `crates/screenpipe-core/src/agents/pi.rs:71-102`); Pi agent provider `screenpipe` with `SCREENPIPE_API_URL` and the cloud JWT in `SCREENPIPE_API_KEY` (hidden from agent bash via `bash_env.rs`); pi web-search extension `crates/screenpipe-core/assets/extensions/web-search.ts` → `https://api.screenpipe.com/v1/web-search` installed only for `screenpipe-cloud`; engine route `POST /v1/chat/completions` (`server.rs:787-793`, `routes/cloud_proxy.rs`) forwards to the gateway with the cloud token, error `{"error":"cloud_token_missing",…}` 503; `suggestions.rs:1432` uses the gateway when "enhanced AI" is on; `region-ocr-overlay.tsx:154` and onboarding `engine-startup.tsx:486-499` call `/v1/chat/completions` directly; workflow classifier (`workflow_classifier.rs:66`, env `SCREENPIPE_EVENT_CLASSIFIER_URL`, model `screenpipe-event-classifier`); usage chip `use-usage-status.tsx` polls `https://api.screenpipe.com/v1/usage` every 30 s with or without a token (shape `{tier, used_today, limit_today, remaining, resets_at}`); one-shot migrations `_proCloudMigrationDone`, `_chatOpusAppliedForPro`, `_proCloudAudioDefaultsAppliedV2` (`use-settings.tsx:488-905, 1133-1161`).
- **Target:** same provider; default presets `{id:"screenpipe", provider:"screenpipe-cloud", url:"", model:"auto", maxContextChars:200000, defaultPreset:true}` and, for verified-paid users, `chat` + `pipes` presets (chunk 378 @14644); legacy migrations rename provider `pi` → `screenpipe-cloud` and id `pi-agent` → `screenpipe-cloud`; Pro users auto-switched to model `claude-opus-5` from `claude-sonnet-4-5`; pi providers `openai-byok`, `anthropic-byok` (`anthropic-messages`), `openai-chatgpt` (`openai-codex-responses`, `https://chatgpt.com/backend-api`), `ollama` (`http://localhost:11434/v1`), custom (`$CUSTOM_API_KEY`) with headers `x-screenpipe-latency`, `x-screenpipe-workload` sent to the gateway (0xa079420, 0xa07b340); engine adds local proxy `POST /v1/web-search` (route table 0x5258ae8, `routes/web_search.rs` 0x6a9e418, error `cloud_token_missing` with a message asking the user to sign in before using web search, 0x6188f7b) used by the extension through `localhost:3030/v1/web-search` (0x52aa43c).
- **Freenpipe:**
  - Default presets on fresh install: one preset `{id:"default", provider:"native-ollama" or "custom", url:"http://localhost:11434/v1", model:"" (user picks), maxContextChars:200000, defaultPreset:true}`; the onboarding AI step lets the user pick Ollama / OpenAI-compatible URL+key / OpenAI / Anthropic / ChatGPT OAuth (these BYO providers already exist in base `pi.rs:1063-1100` and `crates/screenpipe-core/src/agents/pi.rs:792-836`).
  - Provider `screenpipe-cloud` is kept as an accepted value when loading old stores, but resolves to the user-configured gateway base (cloud-042). With no gateway configured, presets with this provider are migrated once to the default BYO preset (record a flag such as `_freenpipeCloudPresetMigrated` so it runs once).
  - Engine `/v1/chat/completions` and `/v1/web-search` routes: forward to the configured gateway base with the configured key; if none, return 503 `{"error":"cloud_token_missing", …}` (keep the error code so pipes/skills that already handle it degrade gracefully).
  - Web search extension: install only when a gateway is configured; otherwise skip (the agent already handles absence).
  - Usage chip: delete the poll; render nothing (`UsageStatus` null is already the "BYOK" state).
  - Enhanced AI suggestions and classifier: use the default BYO preset; classifier only runs if `SCREENPIPE_EVENT_CLASSIFIER_URL` is set.
  - Delete the Pro migration flags logic (leave the flag keys tolerated in the store).
  - `packages/ai-gateway` (vendor Cloudflare worker, MIT in base): exclude from the default build and CI; optionally document it as a self-host option whose URL can be plugged into cloud-042.
- **Confidence:** observed.

### cloud-017 (decloud) cloud transcription → local or BYO Deepgram

- **Base:** engine id `screenpipe-cloud` uses `https://api.screenpipe.com/v1/listen` (`deepgram/mod.rs:37`) with the cloud token; live meeting streaming `wss://api.screenpipe.com/v1/realtime` (`meeting_streaming/config.rs:10-13`) with env `SCREENPIPE_MEETING_STREAMING_PROVIDER`, `SCREENPIPE_MEETING_REALTIME_URL`, `SCREENPIPE_MEETING_DEEPGRAM_LIVE_URL`, `SCREENPIPE_MEETING_CLOUD_TOKEN`, `SCREENPIPE_MEETING_TRANSCRIPTION_MODEL`, `SCREENPIPE_MEETING_TRANSCRIPTION_LANGUAGE`, `SCREENPIPE_MEETING_LOCAL_SPEAKER_NAME`, `SCREENPIPE_MEETING_DEEPGRAM_API_KEY`/`DEEPGRAM_API_KEY`; direct Deepgram `https://api.deepgram.com/v1/listen` (`deepgram/mod.rs:9`) with `deepgramApiKey`; local engines whisper/parakeet/qwen3-asr.
- **Target:** unchanged URLs (0x6b13f73 `/v1/listen` Bearer, 0x6af4498 realtime + `nova-3` model + the same env names); network-failure messages naming `api.screenpipe.com`; quota/account codes `daily_cost_limit_exceeded`, `transcription_capacity_paused`, `account_not_in_good_standing` (0x6b024a8); voice dictation via gateway `/listen?language=multi` (fe chunk 3696); `/workflow-voice` (chunk 4656). Live meetings require either a vendor-cloud sign-in or a Deepgram key (0x6b33600, 0x6b34335).
- **Freenpipe:** default engine stays `whisper-large-v3-turbo-quantized` (target default too); `screenpipe-cloud` not offered unless cloud-042 gateway is configured (then it calls `{gateway}/listen` and `{gateway}/realtime`); Deepgram keeps working with the user's key (batch and live); live meeting notes fall back to the selected local engine (the target already ships a hint that points users at picking a local transcription engine so live notes work without the vendor cloud, 0x6b337f0). Also keep base's OpenAI-compatible transcription settings (`openaiCompatible*` keys, command `test_openai_compatible_transcription`) as the generic BYO path. Dictation: route `/listen?language=multi` to Deepgram (BYO) or local engine.
- **Confidence:** observed.

### cloud-018 (decloud) OAuth broker → BYO client credentials (incl. MCP OAuth relay)

- **Base:** `crates/screenpipe-connect/src/oauth.rs` — `OAuthConfig { auth_url, client_id, extra_auth_params, redirect_uri_override }` (`:84-101`); default redirect `http://localhost:3030/connections/oauth/callback` (`OAUTH_REDIRECT_URI`, `:63`); providers needing HTTPS use `redirect_uri_override: Some("https://screenpi.pe/api/oauth/callback")` (slack.rs:16, zoom.rs:45, calendly.rs:19, quickbooks.rs:35) and the website relays to the local server; code exchange POST `EXCHANGE_PROXY_URL` (`https://screenpi.pe/api/oauth/exchange`) with JSON `{integration_id, code, redirect_uri}` (`:1721-1724`) and refresh with `{integration_id, grant_type:"refresh_token", refresh_token}` (`:820-823`); secrets live server-side. Tokens stored in `SecretStore` under `oauth:<integration_id>[:<instance>]`.
- **Target:** same broker (`screenpi.pe/api/oauth/exchange` appears at ~12 offsets, e.g. 0x9f78c8e, 0x9fba42e; callback relay 0x9fb584a, 0x9fb95a0 incl. Bitrix24, Make); new MCP OAuth for user-added MCP servers: engine routes under `/mcp-servers` (`/registry`, `/:id/test`, `/:id/tools`, `/:id/call`, `/:id/oauth/status`, `/:id/oauth/start` with body `OAuthStartBody{app_scheme}` — 0x6188960-0x6188af0), local callback `http://localhost:3030/mcp-servers/{id}/oauth/callback`, vendor relay `https://screenpi.pe/api/oauth/mcp/{id}/callback?app_scheme=` where `app_scheme` must be `screenpipe` or `screenpipe-enterprise` (0x6a8dd01-0x6a8dd5c); result page titled "screenpipe MCP OAuth".
- **Freenpipe:**
  - Extend `OAuthConfig` with `token_url: &'static str` and `uses_pkce: bool` (original additions). Add per-integration BYO credentials stored in `SecretStore` as `oauth_client:<integration_id>` = `{client_id, client_secret?, redirect_uri?}`; UI: an "Use your own OAuth app" panel in each connection card with the redirect URI to register (`http://localhost:3030/connections/oauth/callback`).
  - `exchange_code` / `refresh_token_instance`: POST directly to `token_url` with standard form fields (`grant_type`, `code`, `redirect_uri`, `client_id`, `client_secret` or `code_verifier`). Keep the merge-preserving-refresh-token behaviour already in base.
  - Shipped `client_id` constants are vendor-registered apps; do not use them (they only work with the vendor's secret). Without BYO credentials the OAuth button is disabled with guidance, and the API-key / webhook alternatives the connections already offer remain available.
  - HTTPS-only providers (Slack, Zoom, Calendly, QuickBooks): require the user to supply an HTTPS redirect they control (e.g. their own relay) or document the provider's localhost policy; no Freenpipe-hosted relay.
  - MCP OAuth: always use the local callback; delete the `app_scheme` relay path. Most MCP servers support dynamic client registration, so no secrets are needed.
  - Remove the Pro gate (cloud-015).
- **Confidence:** observed.

### cloud-019 (decloud) vendor cloud sync, cloud archive and storage

- **Base:** `crates/screenpipe-core/src/sync/client.rs` (`DEFAULT_API_URL = https://screenpi.pe/api/sync`, endpoints `/init`, `/upload`, `/download`, `/search`, `/status`, `/devices`, `/devices?device_id=`, `/data`), S3 presigned uploads, engine `sync_api.rs`, `sync_provider.rs`, `cloud_search.rs`, `archive.rs`, `external_memory_sync.rs`; app `sync.rs`, `components/settings/sync-settings.tsx` (subscription check `https://screenpi.pe/api/cloud-sync/subscription?userId=&email=`, checkout `https://screenpi.pe/api/cloud-sync/checkout`), `archive-settings.tsx`; Tauri commands `init_sync`, `lock_sync`, `trigger_sync`, `get_sync_status`, `get_sync_devices`, `remove_sync_device`, `get_sync_config`, `update_sync_config`, `set_sync_enabled`, `upload_file_to_s3`. Separate LAN/SSH sync (`remote_sync_commands.rs`, `remote_sync_*` commands) is local and stays.
- **Target:** same plus `https://screenpi.pe/api/syncstorage` presigned blob storage with headers `Authorization: Bearer`, `X-Device-Id`, blob metadata fields `blob_type, time_start, time_end, size_bytes, checksum, device_id, key_version, encryption_noncesearch_tokens` (0xa0e5922-0xa0e5a41); `cloudArchiveEnabled` default false, `cloudArchiveRetentionDays` 7 (chunk 378 defaults).
- **Freenpipe:** hide cloud sync and cloud archive UI; make the sync client require an explicit user-supplied endpoint (no default), or remove the feature; keep LAN remote sync. If a BYO target is desired later, `crates/screenpipe-sync` (base, MIT) already has an `http_put` destination with body encryption that can target user S3/WebDAV. Keep the Tauri commands compiled and returning "not configured" so bindings and other dimensions do not break. Delete subscription/checkout calls.
- **Cross-ref:** a dedicated sync dimension may own the local redesign; this item only guarantees no vendor calls.
- **Confidence:** observed.

### cloud-020 (decloud) Tinfoil PII enclave default

- **Base:** `piiBackend` `"local"` | `"tinfoil"`; Tinfoil text/image adapters default to enclave `pii.screenpipe.containers.tinfoil.dev`, repo `screenpipe/privacy-filter`, attestation through `kds-proxy.tinfoil.sh`, `github-proxy.tinfoil.sh`, sigstore; env `SCREENPIPE_PRIVACY_FILTER_{ENCLAVE,URL,REPO,API_KEY}`; auth falls back to `SCREENPIPE_LOCAL_API_KEY`/cloud key.
- **Target:** same (0x498c790, 0x627c8e8); Workflows sharing forces `piiBackend: "tinfoil"` and refuses changes while sharing is on (chunk 378 @26651, @31760); telemetry props `setting_pii_backend`, `setting_tinfoil_pii_active` (0x5179f38).
- **Freenpipe:** default and migrate to `piiBackend: "local"`; show the Tinfoil option only when `SCREENPIPE_PRIVACY_FILTER_ENCLAVE` (or a settings field) and an API key are set by the user; remove `DEFAULT_ENCLAVE`/`DEFAULT_REPO` vendor constants (make them `Option`); remove the sharing coupling (cloud-035).
- **Confidence:** observed.

### cloud-021 (decloud) pipe store, pipe sharing, publish

- **Base:** engine `routes/pipe_store.rs` proxies `/pipes/store`, `/pipes/store/{slug}`, `/pipes/store/install`, `/pipes/store/check-updates`, `/pipes/store/update`, `/pipes/store/auto-update`, `/pipes/store/publish`, `/pipes/store/{slug}/review` to `SCREENPIPE_API_BASE_URL` (default `https://screenpi.pe`) `/api/pipes/store…` using the cloud token; `pipes-section.tsx` shares via `https://screenpi.pe/api/pipes/share`; `update-all-pipes` event fired from the updater.
- **Target:** same paths (0x5182e2a-0x51833ba), share via `screenpipe.com/api/pipes/share` (chunk 5977 @625390 body `{raw_content, name, author_id, author_email, …}`), CLI help text describing a publish command that targets the vendor pipe store and needs auth from `SCREENPIPE_API_KEY` or `~/.screenpipe/auth.json` (0x6a5ab64).
- **Freenpipe:** `SCREENPIPE_API_BASE_URL` default → unset → `/pipes/store` returns an empty list (HTTP 200, `{"data":[]}` style matching the current response envelope) instead of erroring; optional registry: a static JSON index in the Freenpipe repo (`registry/pipes.json`) served from GitHub raw, with install = download pipe source from the listed git URL. Disable publish/review/share (or turn "share" into "export pipe file"). Keep bundled built-in pipes working offline.
- **Confidence:** observed.

### cloud-022 (decloud) team and enterprise cloud features in BASE

- **Base:** `lib/hooks/use-team.ts` (`https://screenpi.pe/api/team` + `/configs`, `/create`, `/invite`, `/join`, `/key-claim/`, `/members`; join link `https://screenpipe.com/join/{invite_token}#key=…`), `lib/team-crypto.ts`, `components/settings/team-section.tsx`, crate `screenpipe-team-memory`, CLI `team` (`cli/team.rs`, `https://screenpi.pe/api/enterprise/v1/*`, `team_api_token` from `screenpi.pe/enterprise?tab=tokens`), `packages/screenpipe-mcp/src/index.ts:227` `TEAM_API`, `use-enterprise-policy.ts` (GET `https://screenpi.pe/api/enterprise/policy` with `X-License-Key`, `X-Device-Id`, optional Bearer; POST `/api/enterprise/heartbeat`; 5-min poll; localStorage `enterprise-policy-cache`; applies AI preset policy, app-update policy, PII policy, managed pipes), `use-enterprise-pipes.ts`, `enterprise-ai-preset-policy.ts`, `enterprise-license-prompt.tsx`, `enterprise-locked-setting.tsx`, commands `get_enterprise_license_key`, `save_enterprise_license_key`, `save_enterprise_team_config`, `is_enterprise_build_cmd`.
- **Freenpipe:** `is_enterprise_build_cmd` → `false` (already true for non-enterprise builds once the feature is removed), so `useEnterprisePolicy()` returns its no-op branch; additionally delete the network code paths from `use-enterprise-policy.ts` (or reduce the hook to the consumer no-op) so no vendor URL remains in the bundle. Hide team section; remove `TEAM_API` and the CLI `team` subcommand (or make it error "not supported"); exclude `screenpipe-team-memory` from the default build if nothing else depends on it.
- **Confidence:** observed.

### cloud-023 (decloud) miscellaneous vendor endpoints and links

| Base / target endpoint | Where | Freenpipe |
|---|---|---|
| `GET {web}/api/changelog?limit=50` | base `changelog-dialog.tsx:23`; target overlay chunk | read GitHub Releases API `https://api.github.com/repos/normiecore/Freenpipe/releases?per_page=50` (unauthenticated, cache 1 h) or the bundled `CHANGELOG.md` |
| `/api/referral`, `/api/referral/invite`, `?ref=` links | base `referral-card.tsx`, `app/settings/page.tsx`; target settings | delete |
| `/survey`, `/ideas`, `docs.screenpi.pe/*`, `discord.com/invite/screenpipe`, `cal.com/team/screenpipe/chat`, `support@screenpi.pe`, `/security`, `/privacy` | tray, home, onboarding, overlay, feedback-section, CLI `survey`, `cli_reminder.rs` | GitHub Discussions / Issues / repo docs links; delete booking/survey; CLI `survey` prints the Discussions URL |
| `/api/download`, `/download`, `/account/versions`, `/enterprise?tab=builds` | base `general-settings.tsx:152`, tray; target chunk 378 | GitHub Releases page |
| `/api/user/log-requests` | target only | see cloud-034 |
| `/api/attribution` | base `analytics.rs:86`; target 0x499ac68; target command `get_onboarding_attribution` | delete; if the command is reimplemented for frontend parity return `null` |
| `/integrations/grokbot#${device}` source ref | target 0x5124de0 | use a local `sourceRef` like `freenpipe://integrations/grokbot` |
| `/api/logs`, `/api/logs/confirm` | base `share-logs-button.tsx`, overlay, onboarding | see cloud-036 |
- **Confidence:** observed.

### cloud-024 (decloud) model and asset hosting

- Pyannote models: change `crates/screenpipe-audio/src/speaker/models.rs:95` (and the wespeaker URL next to it) from the upstream `raw/refs/heads/main` URL to `https://github.com/normiecore/Freenpipe/raw/main/crates/screenpipe-audio/models/pyannote/<file>`; the files already exist in the fork at that path (`crates/screenpipe-audio/models/pyannote/segmentation-3.0.onnx`, `wespeaker_en_voxceleb_CAM++.onnx`). Better: embed or ship them as Tauri resources so no download is needed.
- PII models on `huggingface.co/screenpipe/*` (`pii-redactor` `v50_distilled6l`, `pii-text-redactor` rev `v6`, `pii-image-redactor` `rfdetr_v38.onnx`): vendor-hosted weights of unknown licence. Keep the download opt-in (only when the user enables local PII ML redaction), honour `HF_ENDPOINT`, and add a settings/env override for the repo id so users can mirror. Regex-only redaction (no download) stays the default fallback. Open question on licence.
- Skills registry (target `fetch_skills_registry`, URL 0x4995d13): if implemented, read `https://raw.githubusercontent.com/normiecore/Freenpipe/main/apps/screenpipe-app-tauri/src-tauri/skills-registry.json`.
- Third-party downloads (whisper/parakeet/qwen on HF, silero VAD, PortableGit, bun) unchanged.
- **Confidence:** observed.

### cloud-030 (new→stub) startup authentication worker

- **Target (observed, 0x52d7218-0x52d7d68, 0x5a0c35c):** module `startup_auth.rs` runs before the startup gate: if a saved token exists it refreshes the account by calling `{web}/api/user` (with timeouts and a lock that defers account start-up), replaces the cached user (`user`, `subscription_plan`, `entitlement` with `plan`, `checked_at`, and the full `User` field list incl. `enterprise_account`) on success, signs out when the server rejects the session, keeps cached access marked unverified when offline, then logs that startup authentication has finished. It exports env `SCREENPIPE_STARTUP_AUTHENTICATION_STATUS` with values `authenticated`, `logged_out`, `not_required` (consumed by the engine/child processes). Tracing target `screenpipe_app::startup_auth`, label `consumer-startup-auth`.
- **Freenpipe:** do not implement the refresh. Set `SCREENPIPE_STARTUP_AUTHENTICATION_STATUS=not_required` for the engine and pipe child processes so any code (or third-party pipe) that reads it sees a valid value. No network.
- **Base touchpoint:** `src-tauri/src/main.rs` (engine/pipe env setup near `server_core` start), `server_core.rs`.
- **Confidence:** observed (env name/values), inferred (consumers).

### cloud-031 (changed→decloud) target entitlement model

- **Target (observed, chunk 378 module 96345; REA `ev_6d42880a…`; Rust 0x5287c28, 0x52d73ea):** plan ids `basic, standard, business, business_max, business_ultra, pro, pro_max, pro_ultra, team, enterprise, lifetime`; classification `verified-paid` / `verified-free` / `unknown` (paid requires `subscription_plan` == `entitlement.plan`, both in the set, `app_entitled` or `features.app`, and lifetime/grace/fresh-and-active; freshness window 72 h (`2592e5` ms) with 5-min skew; legacy `cloud_subscribed` no longer bypasses); Pro-tier check excludes `none/standard/basic` and only counts `team/enterprise` for internal screenpipe staff (`enterprise_account.{is_screenpipe_user, team_id, org_name}`); new user fields `has_payment_method`, `entitlement_source`, `enterprise_account`; manual grants (`source:"manual"` with future `current_period_end`); entitlement expiry time helper for "plan expiration notice" banners (`sidebar-plan-expiration-notice`, `account-plan-expiration-notice` ids in chunk 9668). The startup log line now says server auto-start is skipped because account access is required, and a free plan now also requires sign-in (the gate copy asks the user to create or sign in to an account before the free plan starts, chunk 9092); during trial activation a local read-only server starts without capture (0x5177520). Gate states in chunk 9092 (our labels): loading; access check failed; enterprise access check in progress; enterprise license-key entry (`ENT-XXXX-XXXX-XXXX-XXXX`); account not authorised; enterprise build required; sign-in required; access refresh needed; subscription required; analytics `app_entitlement_{gate_shown,choose_plan_clicked,login_clicked,refresh_clicked,restored,autoverify_poll_started,enterprise_download_clicked}`, `enterprise_auth_recording_restored`.
- **Freenpipe:** do not port the plan logic. Add `enterprise_account: Option<serde_json::Value>` to the Rust `User` struct and the TS `User` type (schema compatibility with target `store.bin`; default `None`), plus tolerate unknown `has_payment_method`/`entitlement_source`. All classification helpers return the most permissive answer (cloud-014). Do not render plan expiration notices.
- **Base touchpoints:** `src-tauri/src/store.rs` (`User`), `lib/utils/tauri.ts` (regenerate), `lib/utils/validation.ts:37-40`.
- **Confidence:** observed.

### cloud-032 (new→removed) trial activation / paywall onboarding experiment

- **Target (observed, module 78952 in chunks 5977/layout/onboarding; Rust 0x5128878, 0x499a328, 0x514f7f8, 0x51506dd, 0x52ae1e2, 0x52b107d):** PostHog experiment flag `first-summary-card-trial-v1` with force-unlock flag `first-summary-card-trial-v1-force-unlock`; onboarding steps `trial-activation-v1-summary`, `trial-activation-v1-paywall`, `trial-activation-v1-unlocked` (mapped to `summary`/`paywall`/`unlocked`); localStorage keys `screenpipe:trial-activation-checkout-state:v1`, `screenpipe_trial_activation_assignment_v1`; env `NEXT_PUBLIC_SCREENPIPE_TRIAL_ACTIVATION_DEV`; Tauri events `trial-activation-state`, `trial-unlocked`; while in trial, global shortcuts, the shortcut-reminder overlay and some routing are suppressed; after unlock the capture is restarted; onboarding store fields `trialActivationFreshInstall`, migration flag `trialActivationFreshInstallV1`; eligibility excludes users with `has_payment_method` or entitlement source in `subscription|enterprise|lifetime|dev`.
- **Freenpipe:** never assign these steps. When loading an onboarding store whose `currentStep` is one of the three trial steps, treat onboarding as complete (or move to the next normal step). Do not suppress shortcuts/overlay. Do not emit the events.
- **Base touchpoints:** onboarding store in `src-tauri/src/store.rs` (onboarding struct), `app/onboarding/page.tsx`, `commands set_onboarding_step`.
- **Confidence:** observed.

### cloud-033 (new→removed) PostHog "desktop remote controls"

- **Target (observed, chunk 378 module 96281 @66517; applier in app/layout @86262; REA `ev_4ecb9db5…`):** every 60 s and on window focus/visibility the app reloads PostHog feature flags and reads payloads for flags `semantic-context-control` → `enableSemanticContext` (shipped default on), `coreaudio-system-audio-control` → `experimentalCoreaudioSystemAudio` (on), `smart-recording-control` → `experimentalMeetingPiggyback` (on), `music-filter-control` → `filterMusic` (on), `input-latency-control` → `prioritizeInputLatency` (off, Windows only), `overlay-hiding-control` → `allowHidingShortcutOverlay` (off), `aec-mode-control` → `aecMode` (`off|screenpipe|macos|windows`, default `off`), `auto-update-control` → can force `autoUpdate` on (not for enterprise builds). Payload schema version 1: `{schemaVersion, boolean:{<control>:{defaultEnabled, forceDisabled}}, aecMode:{defaultValue, forceDisabled}, autoUpdate:{forceEnabled}}`. User preferences are kept in settings `remoteControlPreferences` (initial `{semanticContext:true, others:null}`) and `remoteControlPolicy`; when a non-overlay/non-autoupdate control changes, the recorder is stopped and respawned; event `desktop_remote_control_applied`.
- **Freenpipe:** delete the applier. Use the shipped defaults above as plain local setting defaults. Keep `remoteControlPreferences`/`remoteControlPolicy` keys tolerated in the store (ignore them), or drop them on load.
- **Confidence:** observed.

### cloud-034 (new→removed) remote support log collection

- **Target (observed, `remote_support_logs.rs` 0x498fcc0-0x499015e, 0x49f2fe8-0x49f3064, 0x528a0c2-0x528a7c8; settings page @458228-463342; REA `ev_46bc6f41…`):** a background worker, enabled by default (`remoteLogCollectionEnabled: true`, `remoteLogCollectionUserId: null` in defaults), syncs a consent flag to `{web}/api/user/log-requests` and polls it for short-lived support requests, using a dedicated HTTP client that sends `X-Device-Id`, `X-Device-Label`, `X-Platform`, `X-OS-Version`, `X-App-Version`; when a request arrives it collects recent app logs, filters secrets/PII locally, uploads the redacted bundle, and posts a native notification saying the diagnostic logs were shared; consent revocation is synced immediately; states emitted on event `remote-support-log-status` with payload `{state}` ∈ `checking, syncing, ready, sync_error, request_error, signed_out, uploading, disabled`; request fields `request_id`, `requested_at`, state `requested`; settings toggle id `remote-log-collection-toggle`; managed deployments lock it.
- **Freenpipe:** do not implement. Force `remoteLogCollectionEnabled=false` on load and hide the toggle (or replace it with an "Export diagnostics" button that writes a local zip, shared with cloud-036). If the frontend listens for `remote-support-log-status`, emit nothing.
- **Confidence:** observed.

### cloud-035 (new→removed) Workflows sharing / training contributions

- **Target (observed, chunks 9668, 4656, settings page module 49249; REA `ev_73b6811f…`):** endpoint `{web}/api/trajectories?noticeVersion=2026-09-23` with `GET` (status `{available, sharing, training, noticeVersion, acceptedNoticeVersion, accountId, epoch, revision}`), `PUT {sharing, training, noticeVersion, revision}`, `DELETE` (delete shared chats), `POST {id, epoch, source:"workflows-assistant", redaction:"tinfoil-strict-v1", question, answer}`; Bearer = cloud token; 10 s request timeout, 60 s contribution timeout, at most 2 concurrent, question/answer each capped at 1800 bytes and redacted first by Tauri command `redact_workflow_contribution`; local settings `workflowSharing {accountId, epoch, enabledAt, priorBackend}` and `workflowSharingPromptSeen {<userId>: noticeVersion}`; enabling forces `piiBackend:"tinfoil"`; UI section id `workflow-sharing`, plus a settings-search entry about helping improve Workflows.
- **Freenpipe:** remove. Drop `workflowSharing` on load (restore `piiBackend` to its `priorBackend` or `local`), ignore `workflowSharingPromptSeen`. If `redact_workflow_contribution` is needed by shared code, implement it as a local redaction helper only.
- **Confidence:** observed.

### cloud-036 (changed→decloud) feedback / share-logs upload

- **Base (observed, `components/share-logs-button.tsx:161-290`):** POST `{web}/api/logs {identifier, type}` returns `{signedUrl, path, signedUrlScreenshot, signedUrlVideo, screenshotPath, videoPath}`; PUTs combined logs (last 100 KB of up to 5 files + console logs + optional PII-redacted chat history up to 200 KB), screenshot, and video (via `upload_file_to_s3`), then POST `/api/logs/confirm {path, identifier, type, os, os_version, app_version, feedback_text, screenshot_url, video_url, screenpipe_id}`.
- **Target (observed, `feedback_upload.rs` 0x498c937-0x498cca6, 0x5a63148-0x5a63d1a, 0x621d4e1; struct `SignedUpload` 0x52b9a58):** same wire shape moved into Rust as background job `start_feedback_upload` (returns a `jobId`; fields add `video_ext`, `captionsPath`, `supportId`, `screenshotUploaded`, `videoUploaded`), video must be mp4/mov, PII scrub via `redact_pii_for_feedback`, completion event `feedback-upload-completed`, native notification on success/failure.
- **Freenpipe:** implement `start_feedback_upload` as a local export: write a zip (logs tail, optional redacted chat, screenshot, video) under `<data_dir>/feedback/<timestamp>.zip`, emit `feedback-upload-completed` with `{jobId, sent:false, path}`, then open `https://github.com/normiecore/Freenpipe/issues/new` with a prefilled body (title + version + OS) and tell the user to attach the zip. Keep `redact_pii_for_feedback` local. Remove `/api/logs` calls and `upload_file_to_s3` usage for feedback.
- **Confidence:** observed.

### cloud-037 (new→decloud) cloud "data sync" and its engine proxy

- **Target (observed, `data_sync.rs` 0x5228758-0x52289c0; engine `routes/data_sync_proxy.rs` 0x6189041, 0x6a9e4a8; routes 0x5258b10/0x5258b40; skill text 0x51187ae):** app worker uploads local search results (frames, audio, ui events, memories, parsed) to `{web}/api/user/data-sync/ingest`, cursor file `data_sync_cursor.json` with `last_frame_ts`, `last_audio_ts`, `last_ui_ts`, `last_memory_ts`, `last_parsed_ts`, `boundary`, `enabled_at`; settings `dataSyncEnabled` (default false), `dataSyncAccountId`, `dataSyncDeviceName`, `dataSyncEnabledAt`; command `set_activity_sync_enabled`; engine local routes `GET /data-sync/devices` and `GET /data-sync/search?device_name=&since_hours_ago=&q=&limit=` proxy to `{SCREENPIPE_WEB_URL}/api/user/data-sync/…` with the user token (the error tells the user to sign in before querying synced devices); remote MCP at `{web}/api/user/data-sync/mcp` (settings page, chunk 9665).
- **Freenpipe:** no vendor upload. Implement the two engine routes so skills/pipes that call them get a well-formed answer: `/data-sync/devices` returns only the local device (or LAN peers known to `remote_sync`), `/data-sync/search` delegates to local `/search` (or to LAN peers if a later sync dimension adds that). `set_activity_sync_enabled` stores the flag locally and does nothing else. Hide the remote MCP option.
- **Confidence:** observed (endpoints, keys), inferred (exact response shape — mirror the local `/search` envelope).

### cloud-038 (new→decloud) Composio-brokered integrations

- **Target (observed, chunk 5977 @345945, @391921; onboarding chunk; pi extension text 0x52a6792-0x52a6ec4):** Gmail, Zoom, Google Drive, Google Docs, Google Sheets connect via the vendor's Composio account: `{web}/api/composio` (connect/disconnect), `GET {web}/api/composio/status` (Bearer cloud token; returns per-toolkit `{connected}`), and an MCP server `https://screenpipe.com/api/composio/mcp` registered as a shared MCP entry; toolkit ids `gmail`, `zoom`, `googledrive`, `googledocs`, `googlesheets`.
- **Base:** Gmail/Docs/Sheets/Calendar use screenpipe-connect OAuth through the vendor broker (cloud-018).
- **Freenpipe:** keep the base screenpipe-connect integrations with BYO Google/Zoom OAuth clients (cloud-018). Do not add Composio. Where the target UI lists these tiles, map them to the local OAuth integrations.
- **Confidence:** observed.

### cloud-039 (new→removed) hosted-AI allowance and plan quotas

- **Target (observed):** usage response (chunk 67 @8825) fields `used_today`, `limit_today`, `remaining`, `resets_at`, `upsell_banner`, `upgrade_eligible`, `cost_limit_reached`, `hosted_ai {allowance_managed_by ("cloudflare"), allowances[{lane: combined|frontier|auto|explicit, remaining_percent, resets_at}], upgrade}`, `background_pipe_advisory {should_notify, notification_id, cooldown_hours (default 48), message, reason ("background_pipe_allowance_low"), reset_at}` → local `/notify` warning that background pipes are consuming most of the AI allowance, with deep link `screenpipe://scheduled`, opt-out `notificationPrefs.pipeAllowanceWarnings`; error codes `free_chat_limit_exceeded` (with `upgrade_url` restricted to screenpi.pe/screenpipe.com `/onboarding` or `/account/billing`), `hosted_ai_allowance_exceeded`, `trial_cost_limit_exceeded`, `daily_cost_limit_exceeded`, `daily_limit_exceeded`, `credits_exhausted`, `model_not_allowed`, `hosted_ai_capacity_reserved` (0xa0b5251), `activity_history/quota.rs` with `activitiesQuotaPause` (0x498d8c8, 0x621d7c2); Workflows gates `workflow_rollout_disabled`, `workflow_business_required`, `workflow_allowance_paused`, `workflow_usage_unavailable`, `workflow_sign_in_required`, `workflow_dependency_paused` (0xa074b28); engine `pipes_api.rs` error code `free_pipe_limit_reached` (0x6a6d6b0) surfaced in onboarding as a free-plan task-limit message.
- **Freenpipe:** no quotas. Do not implement `free_pipe_limit_reached` or any allowance pause; Workflows/activity generation run whenever a BYO model is configured (the only gate is "no model configured"). Error-code parsing for provider errors (`rate_limit`, `429`, `invalid_api_key`, `model_not_found`, `prompt is too long`) stays because those come from BYO providers.
- **Confidence:** observed.

### cloud-040 (new→stub) expanded telemetry surfaces

- **Target (observed):** engine routes `POST /internal/telemetry/mcp-call` and `POST /internal/telemetry/mcp-value` (`routes/internal_telemetry.rs`, events `mcp_tool_call_completed`, `qualified_value_event`, value kinds `search, retrieved, meeting, artifact, completed`, cohort `repeat_value_d7_v1`, client ids `chatgpt, gemini, openclaw, hermes, runner, windsurf, vscode, grok, grokbot, unknown`, fields `action`, `success`, `result_non_empty`, `direct_api` — 0x52585a8, 0x5a51b20, 0x6a8dd8a); `piggyback_telemetry.rs`, meeting watcher telemetry, first-run summary telemetry (`firstRunSummaryTelemetryVersion`), migration telemetry, activity-generation telemetry, `telemetry_schema_version` on tray/pipe events, search telemetry sampling (`telemetry_sample_interval_seconds`, `telemetry_sampling` none/first_per_interval_per_result_kind).
- **Freenpipe:** register both `/internal/telemetry/*` routes and answer `204 No Content` without recording anything (so the MCP server / external agents that call them do not log errors). All other telemetry hooks compile to no-ops.
- **Confidence:** observed.

### cloud-041 (new) updater/restart commands added in target

- **Target (observed):** command `restart_for_update(timeoutSecs)` returning `"proceed" | "errored" | "pending"` (frontend chunk 8001 @38212; banner calls it with 60), still `await_safe_restart(timeoutSecs)`; update-attempt marker with fields `target`, `attempt_ts`, `home_visible`, `executable`, `restart_committed` (0x6203a18-0x6203a82); local diagnostics `update-install.log`, `update-recovery.log` with symlink refusal (`update_diagnostics.rs`, 0x51751a4-0x51752b0); `installer_handoff_failed`, a Windows update-restart label, and an error asking the user to retry the update once audio has finished initialising (0x4998f1e); enterprise-only persistent updater (`enterprise_persistence.rs`, `screenpipe-enterprise-persistent.exe/.pkg`, `staged.json`/`request.json` with `{version, package, signature}`, embedded pubkey verification — 0x4990558, 0x52d930c-0x52d95f6).
- **Freenpipe:** add `restart_for_update(timeout_secs: Option<u64>) -> Result<String, String>`: wait on the existing `await_restart_gate`, and on `proceed` stop recording, set `QUIT_REQUESTED`, write the attempt marker (JSON in app data dir) and restart; otherwise return the state string. Write `update-install.log`/`update-recovery.log` under the logs dir, refusing symlinked paths. Skip the enterprise persistent updater entirely.
- **Base touchpoints:** `src-tauri/src/updates.rs`, `commands.rs`.
- **Confidence:** observed (names/fields), inferred (exact sequencing).

### cloud-042 (new) configurable AI gateway URL command

- **Target (observed, chunk 961 module 23277, chunk 8001 @12775, 0x516b360, 0x52b4408):** Tauri command `get_screenpipe_ai_gateway_url() -> Result<String, String>` returns the hosted gateway base (vendor default, enterprise policy `gateway_url` may override; non-http values ignored); frontend helper appends paths `/chat/completions`, `/models`, `/usage`, `/listen?language=multi`, `/workflow-voice`, and uses the Tauri HTTP plugin for `http://` bases.
- **Freenpipe:** implement the command to return the user's configured base from a new settings key (proposal `aiGatewayUrl`, default empty) or env `SCREENPIPE_AI_GATEWAY_URL`; return `Err("not configured")` when empty so callers hide gateway-only features. This is the single knob that lets power users point `screenpipe-cloud`-provider features at a self-hosted `packages/ai-gateway`, LiteLLM, or any OpenAI-compatible proxy. Apply the same base in Rust (pi agent `SCREENPIPE_API_URL`, `cloud_proxy.rs`, `suggestions.rs`, transcription `screenpipe-cloud` engine).
- **Confidence:** observed (command, paths), inferred (settings key name is a Freenpipe choice).

### cloud-043 (new→ee-stub) target enterprise additions

- **Target (observed, chunk 961; REA `ev_0ecd50e9…`):** enterprise auth by account (Bearer) or license key (`X-License-Key`, plus `deploymentLicenseKey`), heartbeat body extends base with `machine_id_hash`, `os_user_id_hash` (from `get_enterprise_host_identity`); heartbeat responses 403 `recording_disabled_by_admin` vs seat limit, 401 invalid credential, 402 expired key; policy fields `managedSkills`, `pruneUnlisted`, `recordingAllowed`, `requireAccountLogin`, `applyLocalPolicy`; managed-deployment detection (`isManagedDeployment`, `enterpriseManagedSettings` merged into settings, locked keys incl. `disableKeyboardCapture`, `disableClickCapture`, `piiBackend`); commands `apply_enterprise_ui_visibility`, `get_enterprise_host_identity`, `set_enterprise_recording_authorized`, `sync_managed_team_skills`; `/api/enterprise/cloud-runner`, `/api/enterprise/member-workflows[/…]`, `/api/enterprise/v1/workflows/generated`, `/api/sops`, `/api/sops/access`; enterprise app scheme `screenpipe-enterprise`.
- **Freenpipe:** non-enterprise only. If the reimplemented frontend references these commands, provide no-op stubs: `apply_enterprise_ui_visibility()` → unit; `get_enterprise_host_identity()` → `{machine_id_hash: "", os_user_id_hash: ""}`; `set_enterprise_recording_authorized(authorized: bool)` → unit; `sync_managed_team_skills(...)` → `Ok(0)`/unit; `isManagedDeployment` resolves `false` immediately; `enterpriseManagedSettings` ignored. No network.
- **Confidence:** observed (names), inferred (stub return shapes — match whatever the parity frontend expects).

### cloud-044 (new→removed) in-app billing and checkout

- **Target (observed):** embedded secure checkout window loading `{web}/onboarding/checkout` (chunk 3696 @176099, main layout, onboarding), `GET {web}/api/subscription/offer?distinct_id=&current_plan=` (upgrade offer experiment: `offerVersion`, `experiment.variant`, events `desktop_upgrade_offer_viewed/_config_failed`), `POST {web}/api/subscription/checkout {plan, interval, token, returnUrl, origin, posthog_distinct_id, source_tracking_id, product_tier, internal_plan, billing_interval, seats, cta_location, cta_action:"start_checkout", destination_type:"stripe_checkout", pricing_model_version, …}`, `/api/subscription/change-plan`, `/api/billing`, `/api/team/billing/add-seat`, `/api/team/billing/seat-invite`, `/api/cloud-sync/subscription`; deep link path `/purchase-successful` (0x5a0e0c8); tray "Upgrade to Business" (0x5a3f5cd).
- **Freenpipe:** remove all billing UI and calls; ignore `screenpipe://purchase-successful` deep links.
- **Confidence:** observed.

### cloud-046 (new→decloud) CLI runtime refresh from npm

- **Target (observed, `crates/screenpipe-core/src/agents/cli_runtime.rs` 0xa0c6a50):** the agent runtime refreshes the CLI binary from the npm package `@screenpipe/cli-win32-x64` (`bin`, task name `screenpipe-cli-refresh`).
- **Freenpipe:** use the CLI/engine binary bundled with the app (Tauri sidecar / resource); never fetch vendor npm packages. If the agent needs a `screenpipe` CLI on PATH, write a shim pointing at the bundled binary.
- **Confidence:** observed (package name), inferred (refresh mechanics).

---

## 7. Implementation order (suggested)

1. cloud-001..005 (compile without `ee/`), then cloud-010/011 (strip keys and DSNs) — this is the minimum for a lawful, non-phoning build.
2. cloud-014/015 (gates), cloud-009 switchboard, cloud-012 updater, cloud-013 login removal.
3. cloud-016/017/042 (BYO AI/STT), cloud-018 (OAuth BYO), cloud-020/024.
4. Parity stubs for target-only surfaces: cloud-030, 031, 040, 041, 043, 037 (local routes), 036 (local export).
5. Confirm removals never regress: cloud-019, 021, 022, 023, 032, 033, 034, 035, 038, 039, 044, 046.

## 8. Open questions

- Licence of the vendor-hosted PII model weights on `huggingface.co/screenpipe/*`; if non-redistributable, Freenpipe must not mirror them and should default to regex-only redaction.
- Whether the General Translation SDK (`api.gtx.dev`/`cdn.gtx.dev`) performs any runtime fetch in production builds (looks dev-only; needs a runtime check in the reimplemented frontend).
- Exact JSON returned by the target's `/data-sync/devices` and `/data-sync/search` local routes (only paths and query params are observed).
- Which target-only Tauri commands the reimplemented frontend will actually call (determines how many cloud-043 stubs are needed).
- Whether to keep `packages/ai-gateway` in-tree as a documented self-host option or delete it.

---

## Verification

Adversarial verifier pass, 2026-10-14. Static evidence only: the target was never executed, and no upstream or web material was consulted.

**Method.**

- **Target offsets.** Every cited `0x…` offset was re-read with `tools/verify_offs.py` and `tools/sline.py`.
- **Frontend claims.** Re-read from the chunk text with the new `tools/cloud_verify_fe.py`.
- **REA module views.** Two cited IDs were re-derived from parent `ev_57f23857…`: chunk 961 resolves to `ev_0ecd50e9ac156656f7f2af260fc931aef60f2278dc7815d2c2cf82d5744b8003` and chunk 9092 to `ev_419bce5b892daa19cd6f8244047b05c61837c6e32a7aefc2c0bd436443804f74`. Both match the citations. As the inventory agent warned, these views hold no string literals, so every literal was checked in the chunk files.
- **Base presence.** Checked with `tools/base_has.py`, plus Grep over `<repo>`. The base is at `0e2397a` and its working tree is clean.
- **Independent sweep.**
  - `tools/cloud_verify_ppaths.py`: every `_p("…")` vendor path in the frontend, checked against this spec.
  - `tools/cloud_verify_rsapi.py`: every `/api/…` literal in the exe, against the spec and the base. Output is in `work/cloud/verify_rsapi.txt`.
  - `tools/cloud_verify_basefiles.py`: base files that build vendor requests but are not named here. Output is in `work/cloud/verify_basefiles.txt`.

### Verdicts

| id | verdict | note |
|---|---|---|
| cloud-001 | confirmed | Base `enterprise_policy.rs` is only a `#[path]` include into the missing `ee/`. All callers match (main.rs:882,934,1307,1858,1977; tray.rs:6,117,612-845,913,944; show.rs:407; panel.rs:84; commands.rs:938). Nit: `setEnterprisePolicy` is at tauri.ts:1785, not 1786. |
| cloud-002 | confirmed | `#[path]` include confirmed. `updates.rs:142` reads `.managed`; `main.rs:361` registers `.typ`. The tauri.ts:2219 type has exactly the 4 fields. |
| cloud-003 | confirmed | `enterprise_sync.rs` is 561 lines, gated by `enterprise-build` (`Cargo.toml:266`) with optional `thiserror`/`screenpipe-sync`; `tests/enterprise_sync_test.rs` exists. `screenpipe-telemetry-wire` record and identity strings at 0x69fc328-0x69fca0a. |
| cloud-004 | confirmed | The three importers use exactly `DEFAULT_ENTERPRISE_APP_UPDATE_POLICY`, `describeEnterpriseUpdateMode`, `normalizeEnterpriseAppUpdatePolicy`, `EnterpriseAppUpdatePolicy` and `EnterpriseInstallMetadata`. The `@ee` aliases are in tsconfig.json:29 and vitest.config.ts:61. The mode values come from `updates.rs:144-146`. |
| cloud-005 | confirmed | `Cargo.toml:8`, `.gitignore:44`, `README.md:118`; the four workflows exist. Gap: `release-cli.yml` (138-139, 357-358, 553-554) and `release-app.yml` also upload debug files to Sentry; see cloud-M03. |
| cloud-009 | partially_in_base | The base already has override hooks: TS `lib/web-url.ts` honours `NEXT_PUBLIC_SCREENPIPE_WEB_URL`, the engine honours `SCREENPIPE_API_BASE_URL` (`pipe_store.rs:30`) and `SCREENPIPE_EVENT_CLASSIFIER_URL` (`screenpipe-engine.rs:1369`). What differs: every hook falls back to a vendor host; the engine has no `SCREENPIPE_WEB_URL` (target 0x6a9e4a8); there is no Rust switchboard; `capabilities/main.json:13-14` still lists `api.screenpipe.com`/`api.screenpi.pe`. The UA `screenpipe-app (+https://screenpipe.com)` (0x5226f10) exists only in the target, so the base has no UA to change. |
| cloud-010 | confirmed | Evidence holds (target key at 0x517714c, 0x6a78218, 0x6ab8c68; kill-switch env at 0x6a7f678; `is_telemetry_disabled_by_env` in command list 0x5177a68; `settings_changed` at chunk 378 @32507; `analyticsEnabled:!0` at @16293). **The base count is wrong:** the key is embedded in 7 base places, not 3: `providers.tsx:128`, `main.rs:1092`, engine `analytics.rs:14`, `resource_monitor.rs:335`, `packages/cli/screenpipe/scripts/postinstall.js:70` and `postinstall.sh:167,177`. The npm postinstall ping is not covered; see cloud-M03. |
| cloud-011 | confirmed | `main.rs:509` `sentry::init`, `main.rs:905` plugin, `screenpipe-engine.rs:481`, `@sentry/react`, `tauri-plugin-sentry-api` and `@sentry/cli` in package.json. Target DSN at 0x49f5158; Sentry SDK 10.8.0 at 0x618c7e0. |
| cloud-012 | confirmed | Base endpoints at `tauri.prod.conf.json:58` and `tauri.beta.conf.json:53`; Bearer/`X-License-Key` at `updates.rs:55,395,399`; rollback URL at `updates.rs:37`. Target: 0x6203b1d, `pre-release`/`stable` at 0x6203ada, rollback at 0x512841e. Note 1: the base already has the settings key `updateChannel` (`"stable"\|"beta"`, `use-settings.tsx:83,545`), but nothing reads it at runtime. Note 2: 0x513198b is the tauri-plugin-updater 2.11 `check()` argument list. It shows the plugin supports `allowDowngrades`, not that the target uses it, so using it is a Freenpipe design choice. |
| cloud-013 | confirmed | `commands.rs:769,799,1477,1506` match. |
| cloud-014 | confirmed | `store.rs:45-46,1283+`, `recording.rs:74-81` and `main.rs:1431-1436` (emits `app-entitlement-required`) all match exactly. |
| cloud-015 | confirmed | `oauth.rs:81-90` Pro gate; `store.rs:905-914` `NotLoggedIn`/`NotSubscribed`/`MissingDeepgramKey`; `calendar-connect-dialog.tsx:311` `isPro`; `recording-settings.tsx:2555` "(pro)"; tray `upgrade` item at `tray.rs:735` and cal.com at `:1320`. Overlaps cloud-044 on the tray "Upgrade to Business" item, which is base code. |
| cloud-016 | confirmed | Base: `cloud_proxy.rs:23` `CLOUD_BASE_URL`; `use-usage-status.tsx:27` usage poll; presets with `maxContextChars: 200000` at `use-settings.tsx:449`. Target: web-search proxy 0x6188e97-0x6188f7b, cloud proxy 0x5183758-0x518379f. Note: base and target `cloud_proxy.rs` both exist for "cloud media analysis", which this item never mentions; see cloud-M01. |
| cloud-017 | confirmed | Base `deepgram/mod.rs:9,37` and `meeting_streaming/config.rs:10-13`. Target 0x6b13f73, 0x6af4498, quota codes at 0x6b024a8, 0x6b34335 (login required for live meetings), 0x6b337f0 (local-engine fallback text). |
| cloud-018 | partially_in_base | The MCP half is already done in base. `mcp_servers_api.rs:381-483` implements `/mcp-servers/:id/oauth/{status,start,callback,disconnect}` with PKCE and the local redirect `http://localhost:3030/mcp-servers/{id}/oauth/callback`, and has no `app_scheme` relay; the relay (0x6188960 `OAuthStartBody{app_scheme}`, 0x6a8dd01) is target-only, so "don't port the relay" is the whole MCP task. Not in base, and real work: connector broker BYO (`oauth.rs:971` `EXCHANGE_PROXY_URL`, used at :819 and :1720; `redirect_uri_override` relays in slack/zoom/calendly/quickbooks; no `token_url`/`uses_pkce` in `OAuthConfig` at :89). |
| cloud-019 | confirmed | Base `sync/client.rs:20`; `sync-settings.tsx:964,1053` subscription and checkout. Target syncstorage at 0xa0e5958 and blob fields at 0xa0e5a41. Note: base already defaults `cloudArchiveEnabled: false` and `cloudArchiveRetentionDays: 7` (`use-settings.tsx:593-594`), so that sub-point needs no work. |
| cloud-020 | partially_in_base | The headline behaviour is already in base: `piiBackend` defaults to `"local"` in Rust (`screenpipe-config/src/recording.rs:637`) and TS (`privacy-section.tsx:510`). Remaining work: migrate stored `"tinfoil"`, gate the option on a user-supplied enclave and key, and remove the vendor constants. The spec misses the second constant copy at `crates/screenpipe-engine/src/privacy_filter.rs:52`; `tinfoil.rs:79` holds `DEFAULT_REPO`. The Workflows-sharing coupling is target-only and duplicates cloud-035. |
| cloud-021 | confirmed | Base `pipe_store.rs:30` defaults to `https://screenpi.pe`; `pipes-section.tsx:775` share. Target 0x5182e2a, `/api/pipes/store/publish` at 0x5182fb4, CLI publish text at 0x6a5ab64, chunk 5977 `/api/pipes/share`. |
| cloud-022 | confirmed | `use-team.ts:16`, `cli/team.rs:35`, `screenpipe-mcp/src/index.ts:227`, `use-enterprise-policy.ts` (`/api/enterprise/heartbeat`). |
| cloud-023 | confirmed | `changelog-dialog.tsx:23`, `cli/survey.rs:5`, `cli_reminder.rs:17-18`; target attribution at 0x499ac68; `get_onboarding_attribution` in 0x5177a68; grokbot `sourceRef` at 0x5124de0. Add to scope: `lib/chat/system-prompt.ts:105` tells the chat model to fetch `docs.screenpi.pe/llms-full.txt` (base only; the target dropped it); `google-calendar-card.tsx:262` and `google-sheets-card.tsx:158` open `screenpi.pe/onboarding`; there is a target link `/workspace/my-workflows` in the home page chunk. |
| cloud-024 | confirmed | `models.rs:95` matches. The fork really does contain `crates/screenpipe-audio/models/pyannote/{segmentation-3.0.onnx (5.98 MB), wespeaker_en_voxceleb_CAM++.onnx (29.3 MB)}`. Target 0x6b1fe98, 0x4995d13, 0x49ad288, 0x49ace18, 0x49ad42c. |
| cloud-030 | confirmed | 0x52d7218 `screenpipe_app::startup_auth`; 0x52d7308 `authenticated`/`logged_out`/`not_required` plus `SCREENPIPE_STARTUP_AUTHENTICATION_STATUS`; 0x5a0c35c `/api/user` in `startup_auth.rs`. Absent from base. |
| cloud-031 | confirmed | Chunk 378 @75061 plan set to `pro_ultra`; 0x5287c28 `entitlement_source`; 0x52d73ea User field list; 0x5177520 account-access startup message plus trial read server; chunk 9092 `ENT-XXXX…` (REA `ev_419bce5b…`). `has_payment_method`, `entitlement_source` and `enterprise_account` are all absent from base. |
| cloud-032 | confirmed | 0x5128878 events; 0x52ae1e2 steps; 0x499a328 suppressed shortcut reminder; force-unlock flag in chunk 5977 @731750, main layout @44146 and onboarding page @120291. Absent from base. |
| cloud-033 | confirmed | Chunk 378 @66683 control table (flagKey, settingKey, shippedDefault); `remoteControlPreferences` in 3 files. Absent from base. |
| cloud-034 | confirmed | 0x498fcc0 `remote_support_logs.rs`; 0x528a0c2 device headers; 0x528a139 `/api/user/log-requests` (Rust only, not in the frontend); 0x49f3040; chunk 378 @16313 `remoteLogCollectionEnabled:!0`. Absent from base. |
| cloud-035 | confirmed | `/api/trajectories?noticeVersion=` is built in chunk 4656 @38315 and settings page @46114, not as a literal in chunk 9668 (9668 holds the UI logic, @10994). `redact_workflow_contribution` is in 0x5177a68. Absent from base. |
| cloud-036 | confirmed | Base `share-logs-button.tsx:192,270`. Target 0x498c937, 0x5a63148, 0x52b9a58 `SignedUpload`; `start_feedback_upload`/`redact_pii_for_feedback` in 0x5177a68. |
| cloud-037 | confirmed | 0x6a9e4a8 engine `SCREENPIPE_WEB_URL` plus `data_sync_proxy`; 0x6189041; `data_sync_cursor.json` at 0x499bb09 with `/api/user/data-sync/ingest`; `set_activity_sync_enabled` in 0x5177a68. Absent from base. |
| cloud-038 | confirmed | Chunk 5977 @345915 `/api/composio/status`; onboarding page `/api/composio`. Absent from base. |
| cloud-039 | confirmed | Chunk 67 @9457 `background_pipe_advisory`; 0x621d7c2, 0x498d8c8, 0xa074b28, 0xa0b5251; 0x6a6d6b0 `free_pipe_limit_reached` in `pipes_api.rs`. Absent from base. |
| cloud-040 | confirmed | 0x52585a8/0x52585e0 routes; 0x5a51b20 `internal_telemetry.rs`; 0x6a8dd8a value kinds and client ids; `telemetry_schema_version` at 0x4994eb8, 0x49ee130, 0x5174b53. Absent from base. |
| cloud-041 | confirmed | 0x6203a18-0x6203aac marker fields; 0x51751a4 log files; 0x4998f1e; 0x4990558 enterprise persistent updater; chunk 8001 binding `restartForUpdate({timeoutSecs})`. Base has `await_restart_gate` but no `restart_for_update`. |
| cloud-042 | confirmed | Chunk 8001 binding `getScreenpipeAiGatewayUrl` → `get_screenpipe_ai_gateway_url`; chunk 961 @3953 caller (REA `ev_0ecd50e9…`); 0x52b4408 (a log line about skipping a non-http `gateway_url`). The citation 0x516b360 is weak: it is a truncated command-name table, and the name only appears there as a fragment. Absent from base. Complements cloud-016 rather than duplicating it. |
| cloud-043 | confirmed | `apply_enterprise_ui_visibility`, `get_enterprise_host_identity`, `set_enterprise_recording_authorized` and `sync_managed_team_skills` are in 0x5177a68. Chunk 961 @16875 host-identity hashes and @18114 `recording_disabled_by_admin`; chunk 5977 @562363 cloud-runner; chunk 4656 @24383 `/api/sops/access`. Absent from base. |
| cloud-044 | partially_in_base | It is not all target-new. The base already has: web checkout POST `https://screenpi.pe/api/subscription/checkout` (`account-section.tsx:135`), the tray "⚡ Upgrade to Business" item (`tray.rs:735`, also listed in cloud-015), and the `/purchase-successful` deep link `pathPrefix` on host `screenpi.pe` (`tauri.conf.json:72-74`, also in the beta and enterprise configs). Target-only: the embedded `/onboarding/checkout` window (chunk 3696 @176069), `/api/subscription/offer`, `change-plan`, `/api/billing`, and `/api/team/billing/{add-seat,seat-invite}`. |
| cloud-046 | confirmed | 0xa0c6a50 `@screenpipe/cli-win32-x64` / `screenpipe-cli-refresh` in `cli_runtime.rs`; absent from base. **Scope too narrow:** the base already pulls vendor npm packages at runtime via `bun x screenpipe@latest` / `bun x screenpipe-mcp@latest` (cloud-M02). Also, the base does not bundle a standalone CLI: `tauri.prod.conf.json` `externalBin` is only `bun`, so the `screenpipe` sidecar scope in `capabilities/main.json:262` is stale. "Use the bundled CLI" therefore means building and shipping one. |

No item is a pure duplicate. Partial overlaps: cloud-015/cloud-044 (tray upgrade), cloud-020/cloud-035 (Workflows-sharing coupling), cloud-010/cloud-023 (attribution fetch), cloud-016/cloud-042 (gateway base).

### Missed items

- **cloud-M01 (decloud) Cloud media analysis skill is default-on.** The base Pi agent injects a "cloud media analysis" block into the `screenpipe-api` skill unless `~/.screenpipe/cloud_media_analysis.disabled` exists. The block (`crates/screenpipe-core/assets/skills/screenpipe-api/cloud_media_analysis_block.md`) is injected by `crates/screenpipe-core/src/agents/pi.rs:221-253`. It steers the agent to send media to the vendor gateway's confidential-enclave model through the engine's `cloud_proxy.rs` → `api.screenpipe.com`.
  - Default is on: `privacy-section.tsx:462-463` uses `cloudMediaAnalysisEnabled ?? true`.
  - Command `set_cloud_media_analysis_skill` (`commands.rs:579-596`) writes or removes the marker.
  - The target still ships it: `set_cloud_media_analysis_skill` is in 0x5177a68, the opt-out marker is at 0x52b4da0, the block text ends at 0xa099432 (`gemma4-e4b`), and the target `cloud_proxy.rs` error at 0x518379f asks the user to sign in before using cloud media analysis.
  - **Freenpipe:** default `cloudMediaAnalysisEnabled` to false and treat a missing marker as disabled. Inject the block only when a BYO gateway (cloud-042) is configured, and point `cloud_proxy` at that gateway.
- **cloud-M02 (decloud) Vendor npm packages fetched at runtime.** The base and target both tell agents and external tools to run the vendor's npm packages `screenpipe@latest` and `screenpipe-mcp@latest` through `bun x`/`npx`. Post-relicense versions of those packages are vendor code.
  - Base: `crates/screenpipe-core/assets/skills/screenpipe-cli/SKILL.md` (25 refs), `lib/generated/screenpipe-skills.ts`, `components/chat/schedule-prompt-dialog.tsx:74-77`, `components/settings/{hermes-card.tsx:17, openclaw-card.tsx:16, connections-section.tsx, pipes-section.tsx}`, `components/onboarding/connect-apps.tsx`, `src-tauri/src/pi.rs:2465` (MCP configs as `bun x screenpipe-mcp@latest`).
  - Target: 0x49f3121 `npx -y screenpipe@latest agent setup`, 0x528e688 `screenpipe-mcp@latest` prewarm, 0x6a4c059 skill text.
  - **Freenpipe:** build `packages/screenpipe-mcp` (MIT, in-tree) and the engine CLI locally. Reference them by absolute path from bundled resources, or publish Freenpipe-named packages. Never resolve the vendor packages. This broadens cloud-046.
- **cloud-M03 (decloud) npm CLI postinstall telemetry and CI symbol upload.**
  - `packages/cli/screenpipe/scripts/postinstall.js:70-76` POSTs `cli_install_npm` to `us.i.posthog.com/capture/` with the embedded `phc_` key. `postinstall.sh:167,177` does the same.
  - `release-cli.yml:138-139,357-358,553-554` and `release-app.yml` download `sentry-cli` and upload debug files to the vendor Sentry org.
  - None of these are in the cloud-010, cloud-011 or cloud-005 touchpoints. **Freenpipe:** strip the postinstall ping (or the whole vendor npm wrapper) and the Sentry steps from the release CI.

### Open question resolved

- **General Translation SDK (`api.gtx.dev`/`cdn.gtx.dev`).** The production config loads translations from bundled dictionaries, with a custom `loadTranslations` and `runtimeUrl:null` (chunk `6139-*.js` @273514). The gtx hosts are SDK constants only (chunk 3188), so no runtime fetch is expected and no item is needed.

---

## Verification (pass 2, 2026-10-15)

This is an independent second adversarial pass. It used static evidence only: the target was never executed, and no upstream, post-relicense or web material was consulted. Base is `0e2397a`; the only working-tree change is `apps/screenpipe-app-tauri/bun.lock`, so none of these items has been implemented yet.

**Method.**

- Every `0x…` offset in the claimed item list was re-read with `tools/verify_offs.py` and `tools/sline.py`.
- Frontend literals were re-checked with the new `tools/cv2_fe.py` and `tools/cv2_sum.py`.
- Base presence was checked with `tools/base_has.py` and Grep over `<repo>`.
- Independent sweeps used new helpers:
  - `tools/cv2_vendor_sweep.py`: every vendor/SaaS URL and `/api/` path in the exe and frontend, checked against this spec and the base. Output: `work/cv2_sweep.txt`.
  - `tools/cv2_envdiff.py`: target `SCREENPIPE_*` env names absent from the base. Output: `work/cv2_env.txt`.
  - `tools/cv2_flags.py`: every PostHog flag read by the frontend.
  - `tools/cv2_xdim.py`: overlap of each item with the other dimension specs.
- REA: the app/layout module view is `ev_4ecb9db554c5111dd7ddd7ecf094a7d2ab860bb2c7570ea8d74c6ea13eae8eb7` (parent `ev_57f23857…`). As before, it holds no string literals, so literals were read from the chunk text at the offsets cited.

**Verdict policy.** `duplicate` is used only for duplicates inside this dimension's list. Overlaps with other dimension specs are listed in a separate table so the consolidation step can merge them; the dispositions were compared and none conflict.

### Verdicts (pass 2)

| id | verdict | note |
|---|---|---|
| cloud-001 | confirmed | Base `enterprise_policy.rs:9` is only a `#[path]` include into the missing `ee/`. `mod` at main.rs:46; re-exports at main.rs:111-112. `is_tray_item_hidden` ids at tray.rs:612,619,626,722,763,845. Bindings are at tauri.ts:1785 and 1820 (cited as 1786 and 1821). |
| cloud-002 | confirmed | `#[path]` at :9. `updates.rs:142` reads `.managed`. `.typ` at main.rs:361. The tauri.ts:2219 type has exactly the 4 fields; the command binding is at tauri.ts:547. |
| cloud-003 | confirmed | Base file is 561 lines: `imp` module at 23-553, no-op `spawn` at 558-561. Feature at Cargo.toml:266; optional deps at :158 and :164. `cfg!` uses at updates.rs:115,120 and tray.rs:582,750. Target `screenpipe-telemetry-wire` paths at 0x69fc608-0x69fca0a (records.rs, identity.rs). |
| cloud-004 | confirmed | The three importers use exactly the five names: general-settings.tsx:36-39, use-enterprise-policy.ts:24-28 and use-settings.tsx:20-22. Aliases are at tsconfig.json:29-30 and vitest.config.ts:61. The mode set matches updates.rs:144-146. |
| cloud-005 | confirmed | Cargo.toml:8, .gitignore:44 and README.md:118 confirmed; the four workflows exist. release-enterprise.yml:211 and 482 confirmed. Scope gap (release CI): see cloud-V02. |
| cloud-009 | partially_in_base | The target helper (chunk 378 module 19686 @7346) behaves the same as base `lib/web-url.ts`; it only adds the constant default `https://screenpipe.com`. The engine already honours `SCREENPIPE_API_BASE_URL` (pipe_store.rs:30, cli/pipe.rs:157, cli/install.rs:99) and `SCREENPIPE_EVENT_CLASSIFIER_URL` (screenpipe-engine.rs:1369). What differs: every hook falls back to a vendor host; there is no Rust switchboard; the engine has no `SCREENPIPE_WEB_URL` (target-only, 0x6a9e4a8); and `capabilities/main.json:13-14` still lists the gateway hosts. The UA string at 0x5226f10 is target-only (the base has none), so "change the UA" is a parity choice, not a base fix. |
| cloud-010 | confirmed | Target keys at 0x517714c, 0x6a78218, 0x6ab8c68; kill-switch env at 0x6a7f678; `is_telemetry_disabled_by_env` in the binding chunk 8001 and chunk 378. **Count fix:** "3 keys" counts target offsets only. The base embeds the same `phc_` key 4 times in app/engine code (providers.tsx:128, main.rs:1092, engine analytics.rs:14, resource_monitor.rs:335) and 3 more times in the npm postinstall. A different PostHog key is in `docs/mintlify/docs-mintlify-mig-tmp/docs.json:115`. The Rust default `analytics_enabled: true` (`screenpipe-config/src/recording.rs:597`) also needs flipping, not just the TS default at use-settings.tsx:536. |
| cloud-011 | confirmed | `sentry::init` at main.rs:509 (DSN :510) and plugin at :905; CLI DSN at screenpipe-engine.rs:481-482. `sentry` deps in the root Cargo.toml:40, the app Cargo.toml:108-109 and the engine Cargo.toml:53. `@sentry/react` is also imported directly in recording-settings.tsx:133 and privacy-section.tsx:51. Target DSN at 0x49f5158/0x52d98a0/0x5a66078; SDK 10.8.0 at 0x618c7e0. |
| cloud-012 | confirmed | Endpoints at tauri.prod.conf.json:58 and tauri.beta.conf.json:53, plus tauri.enterprise.conf.json:66. Hard-coded banner URL at update-banner.tsx:116; rollback at updates.rs:37 with Bearer at :55 and :399. Target channel strings at 0x6203ada, builder at 0x6203b1d, rollback at 0x512841e. The base already has `updateChannel` (`"stable"`, use-settings.tsx:545), but only the TS side uses it. **Scope gap:** release-app.yml publishes bundles to the vendor Cloudflare R2 bucket `screenpipe-releases` (lines 1304-1494) and never emits a Tauri `latest.json`. The GitHub-releases design therefore needs a new manifest-generation and release-upload step (cloud-V02). |
| cloud-013 | confirmed | commands.rs:769,799,1477,1506; deeplink-handler.tsx:49; POST `/api/user` at use-settings.tsx:1244. The auth guard is `lib/auth-guard.tsx` (vendor host test at :50, 401/403 interceptor at :109-131). |
| cloud-014 | confirmed | store.rs:45-46 and 1283-1312, recording.rs:74-81 and main.rs:1431-1435 match exactly, including the `app-entitlement-required` emit. |
| cloud-015 | confirmed | oauth.rs:81-90; store.rs:905-926 and 1339/1344; calendar-connect-dialog.tsx:311; recording-settings.tsx:2555; tray.rs:735, 1244, 1273 and 1320. Overlaps cloud-044 on the tray upgrade item and checkout links. |
| cloud-016 | confirmed | cloud_proxy.rs:23 `CLOUD_BASE_URL` (503 `cloud_token_missing` at :32-33); route at server.rs:792; usage poll at use-usage-status.tsx:27; web-search extension at :47. Pro migrations at use-settings.tsx:488 and 814. Rust preset url at store.rs:791 and 1034 (`max_context_chars: 128000` at :1039; TS presets already use 200000). Target `/v1/web-search` route at 0x5258ae8 and 0x6188e97-0x6188f7b. |
| cloud-017 | confirmed | deepgram/mod.rs:9,37 and meeting_streaming/config.rs:10-13. Target 0x6b13f73 and 0x6af4498. Dictation (`language=multi`, `workflow-voice`) is target-only: chunks 3696, 4656, 3768 and 9783. |
| cloud-018 | partially_in_base | **The MCP half is already in base.** `mcp_servers_api.rs:389-569` has PKCE `oauth/start` and a local `oauth/callback` (`http://localhost:3030/mcp-servers/{id}/oauth/callback`, :397), with no `app_scheme`; the relay (0x6188960, 0x6a8dd01/0x6a8dd5c) is target-only. The connector broker is not done: `EXCHANGE_PROXY_URL` at oauth.rs:971, used at :819 and :1720; `OAuthConfig` at :88-101 has no `token_url` or `uses_pkce`. |
| cloud-019 | confirmed | **Correction: there is no `/api/syncstorage` endpoint.** The 0xa0e5958 row is two packed literals: `https://screenpi.pe/api/sync` (same as base `sync/client.rs:20`) followed by an error string about rejected storage auth (a possibly expired presigned URL), which is base `client.rs:1046`. The blob fields at 0xa0e5a41 already exist in base `sync/blob.rs` and `client.rs`. So the target sync wire is the base's, and the base vendor default is the whole problem. Also already in base: `cloudArchiveEnabled:false` and `cloudArchiveRetentionDays:7` (use-settings.tsx:593-594). Add `delete_cloud_data` (sync.rs:796) to the stubbed commands. |
| cloud-020 | partially_in_base | `piiBackend` already defaults to `"local"`: `screenpipe-config/src/recording.rs:636-638` and `privacy-section.tsx:510`. Vendor constants are in 3 places: `redact/adapters/tinfoil.rs:76,79`, `tinfoil_image.rs:58-59` and `engine/src/privacy_filter.rs:52,55`. The Workflows-sharing coupling is target-only (chunk 378 @19592) and overlaps cloud-035. |
| cloud-021 | confirmed | pipe_store.rs:30; pipes-section.tsx:775. Target store paths at 0x5182e2a/0x51833ba; share call in chunk 5977 @625363. |
| cloud-022 | confirmed | use-team.ts:16, team-section.tsx:40, cli/team.rs:35, screenpipe-mcp index.ts:227, use-enterprise-policy.ts:237 and 354. `screenpipe-team-memory` is a member only through the `crates/*` glob, and nothing depends on it. |
| cloud-023 | confirmed | changelog-dialog.tsx:23, cli/survey.rs:5 and cli_reminder.rs:17-18 confirmed. The referral card is at `components/settings/referral-card.tsx`. Target `/api/changelog` is in the overlay page; `get_onboarding_attribution` is in chunk 8001; grokbot at 0x5124de0. |
| cloud-024 | confirmed | models.rs:95; the fork holds both pyannote files (5,983,836 B and 29,292,684 B). Base HF vendor URLs: opf.rs:100, onnx.rs:112, rfdetr.rs:129, rfdetr_mlx.rs:113. The target bumped model revisions (`v50_distilled6l`, `rfdetr_v38`); the base still has `v45_phase5_pruned` and `rfdetr_v12`. |
| cloud-030 | confirmed | 0x52d7218, 0x52d7308, 0x5a0c35c and 0x52d7d68 hold. Absent from base. |
| cloud-031 | confirmed | `pro_ultra` in 6 chunks; `has_payment_method`, `entitlement_source` and `enterprise_account` in chunks 2097, 5977, (main) layout and onboarding; `verified-paid`/`verified-free` in chunk 378; free-plan sign-in copy in chunk 9092; 0x5177520 and 0x5287c28. All absent from base. |
| cloud-032 | confirmed | Flag in chunks 3696, 5977, (main) layout and onboarding; steps at 0x52ae1e2; events at 0x5128878; suppression at 0x499a328. Absent from base. |
| cloud-033 | confirmed | Exactly 8 `*-control` flags in chunk 378 module 96281 (@66683). The applier in app/layout emits `desktop_remote_control_applied` (@86665) and reloads every 60 s (`6e4` @88289). Absent from base. |
| cloud-034 | confirmed | 0x528a0c2 (device headers), 0x528a139 `/api/user/log-requests` (Rust only), 0x528a150 status event and states; default `remoteLogCollectionEnabled:!0` in chunk 378 @16313. Absent from base. |
| cloud-035 | confirmed | `/api/trajectories` is in chunk 4656, the settings page and dev/workflows-ux; `workflowSharingPromptSeen` is in chunk 9668 and the home page. The forced `piiBackend` is in chunk 378 @19592. Absent from base. |
| cloud-036 | confirmed | Base share-logs-button.tsx:192, 259 and 270. Target 0x498c937, 0x5a63148, 0x5a63d1a and 0x52b9a58; event listener in chunk 7737. |
| cloud-037 | confirmed | 0x5228758, 0x52289c0, 0x522883d (`/api/user/data-sync/ingest`), 0x499bb09 cursor file, 0x5258b10/0x5258b40 routes and 0x6a9e4a8; `dataSyncEnabled:!1` in chunk 378 @15586. Absent from base. |
| cloud-038 | confirmed | `/api/composio` is in chunk 5977 and the onboarding page; pi text at 0x52a6925. **Gap:** the base has `gmail`, `google_docs`, `google_sheets`, `google_calendar` and `zoom` connectors but no Google Drive, so the target's Drive tile has no base integration to map to. It needs a new BYO integration or should be dropped. |
| cloud-039 | partially_in_base | "Daily limits" are not target-new. The base already parses `daily_cost_limit_exceeded`, `daily_limit_exceeded`, `credits_exhausted` and `model_not_allowed` (`lib/chat/quota-errors.ts`, standalone-chat.tsx, pipes-section.tsx), consumes `used_today`/`limit_today` (use-usage-status.tsx), and shows an upgrade-to-Business upsell (quota-errors.ts:30); these also need neutralising. Target-only and confirmed absent: `free_chat_limit_exceeded`, `hosted_ai_allowance_exceeded`, `trial_cost_limit_exceeded` (0x621d7c2), `hosted_ai` and `background_pipe_advisory` (chunk 67), `activitiesQuotaPause` (0x498d8c8), the `workflow_*` gates (0xa074b28), and `free_pipe_limit_reached` (0x6a6d6b0, onboarding page). |
| cloud-040 | confirmed | Routes at 0x52585a8/0x52585e0; `internal_telemetry.rs` at 0x5a51b20; value kinds and client ids at 0x6a8dd8a. Absent from base. |
| cloud-041 | confirmed | The base already has the gate and the three states (`await_restart_gate` and `await_safe_restart` → `"proceed"\|"errored"\|"pending"`, updates.rs:183-261). The target keeps `awaitSafeRestart` and adds `restart_for_update` as a Result command (binding chunk 8001). Callers in chunk 1491 and app/layout pass 60 and treat a non-ok result as `errored`. Marker fields at 0x6203a18-0x6203aac, logs at 0x51751a4/0x51752b0. |
| cloud-042 | confirmed | Binding `getScreenpipeAiGatewayUrl` → `get_screenpipe_ai_gateway_url` in chunk 8001; callers in chunk 961, app/layout and dev/workflows-ux. 0x52b4408 confirmed. 0x516b360 is only a truncated command-table fragment (weak; prefer the chunk 8001 citation). Absent from base. |
| cloud-043 | confirmed | All four commands are target-only. `get_enterprise_host_identity` is in chunk 8001 and app/layout; `machine_id_hash` in chunk 961 and app/layout; cloud-runner in chunk 5977; `/api/sops` in chunk 4656. **Add** `list_managed_team_skills` (also target-only, chunk 8001) to the stub list. |
| cloud-044 | partially_in_base | Base already has: web checkout `screenpi.pe/api/subscription/checkout` (account-section.tsx:135); `screenpi.pe/api/cloud-sync/checkout` in 5 files (connect-apps.tsx:612, archive-settings.tsx:194, ai-presets.tsx:1520, recording-settings.tsx:2201, sync-settings.tsx:1053); the tray "Upgrade to Business" item (tray.rs:735); and the `/purchase-successful` deep link (tauri.conf.json:74, plus the prod, beta and enterprise confs). Target-only: the `/onboarding/checkout` window, `/api/subscription/offer`, `change-plan`, `/api/billing` and `add-seat` (home and settings pages). **Gap:** the Business-trial "card ask" modal in the (main) layout (@34231-42294) is not listed. It has an embedded card iframe (`screenpipe-business-trial:init`), triggers including `grant_expiry`/`at_login`, A/B arms, checkout with `business_trial_mode` and `source_tracking_id:"desktop-card-ask-v1"`, and env `NEXT_PUBLIC_SCREENPIPE_FORCE_BUSINESS_TRIAL_MODAL`. Remove it too. |
| cloud-046 | confirmed | 0xa0c6a50 confirmed; absent from base. Same scope and disposition as agents_pipes-040, which also documents the `bun install screenpipe@latest` cache (`sp-cli-resolve`). See the overlap table and cloud-M02. |

No pure duplicates inside this dimension. Intra-dimension overlaps to merge at implementation: 015/044 (tray upgrade, checkout links), 020/035 (sharing coupling), 010/023 (attribution), 016/042 (gateway base), 013/030 (`/api/user` refresh).

### Cross-dimension overlaps (same disposition, merge at consolidation)

| cloud item | also specified in |
|---|---|
| 001, 002, 043 | shell-035, frontend-052, agents_pipes-041 |
| 010, 011 | shell-031 |
| 012, 041 | shell-016, shell-013, shell-038 |
| 014, 030, 031 | shell-034 |
| 016, 042 | agents_pipes-036 (also owns the Tinfoil "Private (Beta)" GLM chat provider, model `glm-5.3-flash-reap50-iq3m`, which is missing from section 2 of this spec), api-024, agents_pipes-035 |
| 018 | connections-oauth-byo-broker |
| 020 | vision-tinfoil-decloud |
| 021 | agents_pipes-037 |
| 024 | audio-032 |
| 033 | audio-029 |
| 034 | shell-032 |
| 036 | shell-033, frontend-046, meetings_memory-009 |
| 037 | api-026 |
| 040 | agents_pipes-038, api-012 |
| 046 | agents_pipes-040 |

### Missed items (pass 2)

The pass-1 items cloud-M01 and cloud-M02 were re-verified independently and stand.

- **M01.** Base `privacy-section.tsx:462-463` defaults to `?? true`; `pi.rs:221-253` injects the block; `commands.rs:579-596` sets the marker. Target 0x52b4da0 and 0x518379f confirm the feature still ships.
- **M02.** `screenpipe@latest` and `screenpipe-mcp@latest` appear 83 times across 16 base files, outside docs and node_modules.

Pass-1 M03 is folded into cloud-V02 below. Two new items:

- **cloud-V01 (new→removed) PostHog-delivered remote announcements and surveys.**
  - **Target behaviour (observed, app/layout chunk @124456-127433; REA `ev_4ecb9db5…`):**
    - The root layout reads the PostHog flag payload `app-announcement`. It is the only flag outside the cloud-033 and trial sets; `tools/cv2_flags.py` confirms this.
    - It reloads flags every 60 s and on focus or visibility change.
    - It renders vendor-authored banner, card, bubble or sidebar announcements.
    - A CTA can open a URL, navigate a route, or emit `install-pipe` with `registry:<slug>`. A remote push can therefore prompt a pipe install.
    - Surveys submit their answers as the event `announcement_survey_submitted`. Further events: `announcement_shown`, `announcement_opened`, `announcement_cta_clicked`.
    - State lives in localStorage `screenpipe-announcements-dismissed-v1` and `screenpipe-announcement-preview`.
    - It is skipped in dev/e2e and when `NEXT_PUBLIC_SCREENPIPE_DISABLE_REMOTE_ANNOUNCEMENTS=true`.
  - **Local path:** the same UI also accepts a local `announcement` event pushed by `/notify` (0x514b13e).
  - **Base:** absent; there is no `getFeatureFlagPayload` or `onFeatureFlags` anywhere.
  - **Freenpipe:** drop the remote source entirely. If the frontend dimension ports the announcement UI, feed it only from the local `/notify` `announcement` event, and never auto-offer pipe installs from a remote payload.
- **cloud-V02 (decloud, base) Release/CI and packaging still feed vendor infrastructure.** This supersedes pass-1 M03 and extends it.
  - **R2 publishing:** `release-app.yml:1304-1494` uploads every bundle to the vendor Cloudflare R2 bucket `screenpipe-releases`, and no step emits a Tauri updater manifest. cloud-012's GitHub `latest.json` design needs a new CI step that writes `latest.json` from the `.sig` files and uploads it with the bundles to a GitHub release.
  - **Sentry symbols:** `release-app.yml:1509-1535` and `release-cli.yml:133-139, 351-358, 548-554` upload debug symbols to Sentry org `mediar`.
  - **npm postinstall:** `packages/cli/screenpipe/scripts/postinstall.js:70` and `postinstall.sh:167,177` POST `cli_install_npm` to PostHog with the embedded key.
  - **Docs analytics:** `docs/mintlify/docs-mintlify-mig-tmp/docs.json:113-116` embeds a second PostHog key (EU host).
  - **Freenpipe:**
    - Replace the R2 upload with a GitHub release plus a `latest.json` step signed with the new key.
    - Delete the Sentry steps.
    - Strip the postinstall ping, or drop the vendor npm wrapper.
    - Remove the docs analytics block.
    - Add these paths to the section 4 item 5 CI guard.

### Evidence corrections to apply to the body of this spec

- Section 2 table and cloud-019: remove `/api/syncstorage`; it is a literal-packing artifact (see the cloud-019 verdict).
- Section 2 Tinfoil row: add the GLM confidential chat transport (`{gateway}/tinfoil/glm`, API `screenpipe-tinfoil`, 0xa082e1e-0xa084025); its disposition is owned by agents_pipes-036.
- cloud-010: correct the key count (see the cloud-010 verdict).
- cloud-042: prefer the chunk 8001 binding over the 0x516b360 fragment.
