# Freenpipe RE spec — Connections, integrations, secrets, browser automation, wearables (dimension `connections`)

Target: screenpipe v2.7.104 Windows build (`screenpipe-app.exe`, static evidence only, never executed).
Base: Freenpipe @ upstream 892199f (`crates/screenpipe-connect`, `crates/screenpipe-secrets`,
`crates/screenpipe-vault`, `crates/screenpipe-engine/src/connections_api.rs`, app `src-tauri/src/oauth.rs`,
`owned_browser*.rs`, `components/settings/connections-section.tsx`).

Clean-room note: everything below paraphrases behaviour. Interop identifiers (routes, JSON keys,
SQL, settings keys, env vars, file names, Tauri command names, integration ids) are quoted exactly.
Agent-facing integration descriptions, UI copy and embedded JS/TS bodies in the target are NOT
reproduced; write new text in Freenpipe. Provider token-endpoint URLs given under "implementation
notes" come from the providers' public documentation, not from the target, and are marked as such.

## 0. Evidence and method

* `app_strings.tsv` offsets are hex file offsets into the exe. The connect crate's static data
  (integration defs, OAuth configs, descriptions) lives in one contiguous region
  `0x9f78000..0x9fc0000` (lines 687043..689521 of the TSV). Engine-side connection strings live
  around `0x517c750..0x5182e00` (connections_api.rs, browser snapshot/act scripts, route table) and a
  second engine copy around `0x6a30ca3..0x6a33b60`. The secrets crate appears in many copies
  (`0x49af000`, `0x5130234`, `0x518ae31`, `0x522f529`, `0x9f7a7f0`, `0xa04b369`...).
* Helper scripts written for this dimension (`<re-workspace>/tools/`):
  `conn_paths.py` (route-like token extractor), `conn_tauri_cmds.py` (Tauri command + arg names from
  the bindings chunk), `conn_jsctx.py` (context windows in minified JS). Shared helpers
  `mm_ctx.py`/`mm_grep.py` were also used.
* Frontend (`<re-workspace>/frontend/`):
  * `_next/static/chunks/5977-dd591e0cc8c27cfa.js` — Settings > Connections page, connection
    pickers for workflow triggers, Composio client, MCP-OAuth tiles, Bee/IMAP/Obsidian panels.
  * `_next/static/chunks/3696-4fbfea4ced3de899.js` — chat: owned-browser sidebar with tabs,
    inline `connection_action` card.
  * `_next/static/chunks/8001-00ed120106af2449.js` — generated Tauri bindings (`tauri.ts`).
  * `_next/static/chunks/app/onboarding/page-960f31d90328bcc8.js` — onboarding Composio use.
  * `images/` — new tiles (`plaud.png`, `krisp.svg`, `excalidraw.svg`, `hermes.png`, `bee.png`,
    `google-drive.svg`, `voice-memos.svg` ...).
  * `Users/you/Documents/MyVault` and `Users/you/Documents/logseq-graph` are NOT real assets: the
    asset-map extractor misread IntegrationDef placeholder/help-url pairs (manifest offsets
    167243293 ≈ `0x9f7ef1d`, obsidian-memories def; 167320921 ≈ `0x9f91d59`, logseq def). Their
    "contents" are the Obsidian and Logseq help URLs.
* REA: parent JS-application evidence `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`;
  module view of the connections chunk `ev_a4b18fd6c3edc1aac34710e74cd84c7e4466909a718787804cf036d4ec13236a`
  (identity only — the graph holds 2 observations for this asset); module trace (over budget)
  `ev_ffde12017a03a7037d3c0739fd622edfc9525e9a7d0f4d4413a541c5fdd75fa8`; string/api traces for
  `mcp.plaud.ai` and `/connections` returned no-match (`ev_d8e5414c1a487329fe9c4cdd04991c1c7d41e7f02c44ba88233c9034d86753d6`,
  `ev_03bc3421c2f71ed999300206b1749913e163c0d27aa417fa6ad2f95800255def`), so frontend facts are
  cited by file path + character offset.
* Target connect-crate source files (panic paths): `connections/{mod,supabase,odoo,google_docs,
  microsoft365,obsidian_memories,hermes,notion,stripe,clickup,granola,telegram,pipedrive,
  outlook_email,financialsense,ntfy,codex,mochi,teams,toggl,logseq,sentry,todoist,openclaw,
  workflowy,perplexity,claude_code,github_issues,bee,zoom,slack,discord,imap,email,calcom,monday,
  zapier,google_calendar,loops,hubspot,posthog,airtable,pushover,salesforce,jira,make,asana,glean,
  pocket,resend,trello,quickbooks,lexi,linear,vercel,calendly,n8n,brex,otter,zendesk,bitrix24,
  intercom,obsidian,readwise,fireflies,limitless,confluence}.rs`, `connections/browser/{mod,owned,
  user,bridge}.rs`, `oauth.rs`, `oauth_refresh_scheduler.rs`, `mcp_servers.rs`, `remote_sync.rs`,
  `sync_scheduler.rs`, `mdns.rs`, `ics_calendar.rs`, `calendar_windows.rs`, `whatsapp/mod.rs`.
  New vs base: `imap.rs`, `hermes.rs`, `outlook_email.rs`, `obsidian_memories.rs`.
  Absent vs base: `gmail.rs`, `google_sheets.rs` (no Gmail/Sheets client ids or API hosts remain).
  Core: `screenpipe-core/src/pipes/connection_triggers.rs` + `connection_triggers/voice.rs` (new).
* New crates (`crates.txt`): `async-imap 0.11.3`, `imap-proto 0.16.7`, `mail-parser 0.11.5`
  (all absent from base `Cargo.lock`), `rsa 0.9.10` + `aes` (present for Bee pairing, inferred).

## 1. Base vs target inventory

### 1.1 Native integrations (`all_integrations()` registry)

`B` = base, `T` = target. Auth: `key` = user-entered fields, `oauth` = authorization-code flow via the
screenpipe broker, `hyb` = OAuth with field fallback, `local` = filesystem/local process, `wh` = webhook URL.

| id | B | T | Auth (T) | Proxy / control surface (T) | Delta |
|---|---|---|---|---|---|
| telegram | key | key | bot_token, chat_id | NEW `POST /connections/telegram/send`; raw creds no longer surfaced | connections-telegram-send |
| slack | oauth (incoming-webhook) | oauth user token, variants `send`/`read_write` | broker + relay | `/slack/send`, NEW `/slack/search`, `/slack/conversations`, `/slack/history` | connections-slack-user-token |
| discord | wh | wh | webhook_url | NEW `POST /connections/discord/proxy` | connections-webhook-proxy |
| email (SMTP) | key | key | smtp_host/port, username, password, from, to | unchanged | — |
| imap | — | key | imap_host, imap_port, username, password (app password) | NEW `/imap/messages`, `/imap/messages/:uid`, `/imap/mailboxes` | connections-imap |
| todoist | key | key | api_token | proxy | — |
| teams | oauth + wh | oauth + wh | MS Graph; prompt now `select_account` | proxy + NEW webhook `POST /connections/teams/proxy` | connections-webhook-proxy, connections-oauth-scope-tweaks |
| notion | oauth | oauth (+ MCP alt) | broker | proxy; MCP tile `https://mcp.notion.com/mcp` | connections-mcp-oauth-tiles |
| linear | key | key (+ MCP alt) | api_key | proxy; MCP tile | connections-mcp-oauth-tiles |
| perplexity | key | key | api_key | proxy | — |
| obsidian | local | local (multi-vault) | vault_path, daily_note_folder | filesystem | connections-ui-catalog |
| obsidian-memories | — | local | vault_path, memories_folder | writes `<vault>/<folder>/screenpipe-memories.md` | connections-obsidian-memories |
| n8n / make / zapier | wh | wh | webhook_url | NEW `POST /connections/<id>/proxy` | connections-webhook-proxy |
| github | oauth (scope `repo`) | oauth (`repo read:org`) | broker | proxy | connections-oauth-scope-tweaks |
| jira | oauth | oauth (+ MCP alt) | broker | proxy `{cloud_id}` | connections-mcp-oauth-tiles |
| granola, bitrix24, limitless, bee, pocket, airtable, logseq, pushover, ntfy, toggl, brex, posthog, clickup, confluence, salesforce, trello, stripe, financialsense, fireflies, otter, lexi, sentry, pipedrive, odoo, intercom, monday, asana, glean, readwise, mochi, loops, resend, workflowy, openclaw | key/local | same | same | same; several gain MCP alternatives (stripe, sentry, intercom, asana, monday, clickup, airtable, confluence) | connections-mcp-oauth-tiles |
| bee | key | key + one-click pairing | developer token or paired token | proxy (private CA) + NEW `/bee/pair/start`, `/bee/pair/poll` | connections-bee-pairing |
| hubspot | key (api_token) | hyb | OAuth (broker) + Private App token fallback | proxy | connections-hubspot-oauth |
| zendesk | key (basic) | hyb | OAuth with `{subdomain}` auth URL (broker + relay) + email/API-token fallback | proxy | connections-zendesk-oauth |
| microsoft365 | oauth | oauth | prompt `select_account` | proxy | connections-oauth-scope-tweaks |
| outlook-email | — | oauth | MS Graph Mail scopes | proxy; `/outlook-email/instances` | connections-outlook-email |
| calcom, calendly, vercel, quickbooks, supabase, zoom, google-calendar, google-docs | oauth | oauth | broker (+ relay for calendly, quickbooks, zoom) | proxy (unchanged) | connections-oauth-byo-broker |
| gmail | oauth (native, `/gmail/*`) | REMOVED native; Composio MCP via screenpipe.com | vendor cloud | — | connections-gmail-sheets-composio |
| google-sheets | oauth (native) | REMOVED native; Composio | vendor cloud | — | connections-gmail-sheets-composio |
| claude-code, codex | local (memory digest) | local (MCP recall guidance only) | home_path | marker block | connections-agent-memory-guidance |
| hermes | — (UI tile only) | local | endpoint, token | agent describes its OpenAI-compatible API | connections-hermes |

### 1.2 Non-registry "connections" in `GET /connections`

| id | B | T | Delta |
|---|---|---|---|
| `ics-calendar` | yes (`feed_count`, `enabled_feed_count`, `error`) | same (`enabled_feed_count` key observed `0x517e220`) | — |
| `apple-calendar` | macOS only | same | — |
| `whatsapp` | QR gateway | same + sign-out recovery | connections-whatsapp-signout |
| browsers (`owned-default`, `user-browser`) | registry entries, Control block | Control block adds `/act`; snapshot v2 | connections-browser-snapshot-v2-act |
| MCP-OAuth / Composio providers | — | annotated `mcp`, `mcp_server_id` (pi extension; engine reads MCP servers for status `0x5182620`) | connections-mcp-oauth-tiles |

### 1.3 `/connections` route table (engine, nested under `/connections`)

Target table observed at `0x51826a8..0x5182e00` (also `0x5257ca8` blob). `+` new, `-` removed.

| Route | B | T |
|---|---|---|
| `GET /` | yes | yes |
| `GET /browsers`, `GET /browsers/:id/status`, `POST /browsers/:id/navigate`, `GET /browsers/:id/snapshot`, `POST /browsers/:id/eval` | yes | yes (snapshot v2) |
| `POST /browsers/:id/act` | — | `+` |
| `/browser/pair/{start,status,pending,approve}`, `/browser/{ws,eval,cookies,status}` | yes | yes |
| `GET /oauth/callback` | yes | yes (richer error handling) |
| `/calendar/{events,status}`, `/ics-calendar/{events,status}` | yes | yes |
| `/google-calendar/{events,status,disconnect}` | yes | yes (multi-calendar, `account_snapshots`, `require_complete`) |
| `/gmail/instances`, `/gmail/messages`, `/gmail/messages/:id`, `/gmail/send` | yes | `-` |
| `GET /imap/messages`, `GET /imap/messages/:uid`, `GET /imap/mailboxes` | — | `+` |
| `POST /slack/send` | yes | yes (user token) |
| `GET /slack/search`, `GET /slack/conversations`, `GET /slack/history` | — | `+` |
| `POST /telegram/send` | — | `+` |
| `/whatsapp/{pair,status,disconnect}` | yes | yes |
| `POST /bee/pair/start`, `POST /bee/pair/poll` | — | `+` |
| `ANY /:id/proxy` (no path: webhook proxy) | — | `+` |
| `ANY /:id/proxy/*path` | yes | yes |
| `GET /:id/config` | yes | yes |
| `GET /:id/instances`, `PUT/DELETE /:id/instances/:instance` | yes | yes (OAuth instances listed; secrets redacted) |
| `GET/PUT/DELETE /:id`, `POST /:id/test` | yes | yes (GET redacts secrets) |

Auth-exempt paths (engine middleware) unchanged in substance: `/connections/oauth/callback`,
`/connections/browser/pair/start`, `/connections/browser/pair/status`, `/mcp-servers/*/oauth/callback`.

### 1.4 Tauri commands (connections/browser/secrets)

From `8001-00ed120106af2449.js` (args listed). `+` new vs base, `~` changed signature.

| Command | Args (T) | Note |
|---|---|---|
| `oauth_connect` ~ | `integrationId, instance, variant` | base has no `variant` |
| `oauth_cancel`, `oauth_disconnect`, `oauth_list_instances`, `oauth_status` | unchanged | |
| `activate_app_after_oauth` + | — | bring app to foreground after browser consent |
| `open_google_calendar_auth_window` | `authUrl` | unchanged |
| `owned_browser_navigate` ~ | `url, owner, reveal` | base: `url` only |
| `owned_browser_history` + | `tabId, direction` | |
| `owned_browser_tab_navigate` + | `tabId, url, owner` | |
| `owned_browser_tab_set_bounds` + | `tabId, parent, x, y, width, height` | |
| `owned_browser_tab_hide`, `owned_browser_tab_close`, `owned_browser_tab_clear_browsing_data` + | `tabId` | |
| `owned_browser_set_bounds`, `owned_browser_hide`, `owned_browser_clear_browsing_data`, `owned_browser_resolve_session_access(requestId, allow)` | unchanged | |
| `set_browser_cookie_access_granted`, `set_browser_cookie_access_state`, `confirm_browser_cookie_access_for_session`, `get_browser_cookie_access_granted` | unchanged | |
| `get_keychain_status`, `enable_keychain_encryption`, `disable_keychain_encryption`, `reencrypt_store`, `vault_status`, `vault_unlock` | unchanged | |
| `ics_calendar_*`, `calendar_*` | unchanged | |

### 1.5 Stored state

| Store | B | T | Delta |
|---|---|---|---|
| `secrets` table | in main `db.sqlite` | in separate `<data>/secrets.sqlite` (+ `secrets.sqlite-journal`) | connections-secret-db-split (schema owned by db-026) |
| `credential_refresh_leases` | — | `secrets.sqlite` | connections-credential-refresh-leases |
| `secret_store_metadata` | — | `secrets.sqlite` (`legacy_main_db_migrated_v1`) | connections-secret-db-split |
| secret keys | `cred:<id>[:<inst>]`, `oauth:<id>[:<inst>]`, `mcp-oauth:<id>`, `mcp-oauth-pending:<state>` | same | — |
| OAuth token JSON fields | `access_token, refresh_token, expires_at, expires_in, last_refreshed_at, screenpipe_refresh_gen, email, workspace_name, ...` | same + Slack `authed_user`, `slack_user_id`, `team_id`; Zendesk `subdomain` | connections-slack-user-token, connections-zendesk-oauth |
| keychain item | service `com.screenpipe.app`, account `store-encryption-key` | same (`0xa2d0e1c`) | — |
| `connections.json`, `<id>-oauth.json` legacy files | migrated to SecretStore | same | — |
| `connection-triggers.json` | — | trigger cursor file | connections-connection-triggers |
| `upload_bindings(destination TEXT PRIMARY KEY, checkpoint TEXT NOT NULL)` | — | hybrid-storage upload checkpoint table (`0x49e787f`, `storage/bindings.rs`) — NOT a connections table; belongs to the storage/sync dimension | (out of scope) |
| `friend_wearable_requests` | created 2024, dropped by `20260311000000_drop_unused_tables` | same drop (`0xa2982af`) | none (no wearable API) |
| `vault.meta` / `.vault_locked` | encrypts `db.sqlite(+wal/shm)` | also `secrets.sqlite(+journal)`; refuses hybrid storage | connections-vault-secrets-db |

### 1.6 Vendor-cloud dependencies in this dimension

| Dependency | Where | Used for | Freenpipe replacement |
|---|---|---|---|
| `https://screenpi.pe/api/oauth/exchange` | connect `oauth.rs` (15 copies, e.g. `0x9f78c8e`, `0x5186da1`) | code exchange AND refresh for every native OAuth integration (client secrets held server-side) | BYO client id/secret, direct token endpoint (connections-oauth-byo-broker) |
| `https://screenpi.pe/api/oauth/callback` | redirect override (`0x9f96e2e` zoom, `0x9fb13d8` quickbooks, `0x9fb584a` calendly, `0x9fb95a0` zendesk; slack inferred) | HTTPS redirect relay back to localhost | loopback redirect or BYO HTTPS relay |
| `https://screenpi.pe/api/oauth/mcp/<id>/callback?app_scheme=` | `0x6a8dd01` | MCP OAuth redirect relay | loopback `http://localhost:3030/mcp-servers/<id>/oauth/callback` (already in base) |
| `https://screenpipe.com/api/composio` (+`/status`, `/authorize`, `/mcp`) | `5977…js` @~392000, onboarding page | Gmail, Google Drive/Docs/Sheets, Zoom via Composio managed auth | keep native Gmail/Sheets, add native Drive (BYO OAuth) or user's own Composio key |
| `https://auth.beeai-services.com/apps/pairing/request` + app id `ph9fssu1kv1b0hns69fxf7rx` | `0x5185bec` | Bee one-click pairing (third-party, but app id registered by screenpipe) | user-supplied Bee app id; default to developer-token path |
| OAuth Pro-subscription gate | base app `oauth.rs` (`cloud_subscribed`) | blocks OAuth for non-paying users | remove (target also no longer contains the gate string) |

---

## 2. Delta items

### connections-secret-db-split — Secret store opens its own `secrets.sqlite`

**Kind:** changed. **Confidence:** observed (schema, migration, file names), inferred (pool API names). **Size:** L.
**Evidence:** DDL `0x49af280..0x49af8ed`; migration `0x49af8ed..0x49afa1c`; file list `0x49873c0`, `0x522af47`, `0xa23c560`; pool messages `0x49aff48`, `0x49b001b`, `0x49b0109`; full copy `0x5130234..0x5131784`; encrypted count `0x51309e8`; decrypt/reencrypt list `0x5130a80`, `0x5131108`. Schema already recorded in `specs/db.md` db-026.

DDL (verbatim, `secrets.sqlite`):

```sql
CREATE TABLE IF NOT EXISTS secrets (
    key TEXT PRIMARY KEY,
    value BLOB NOT NULL,
    nonce BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    expires_at TEXT
);
CREATE TABLE IF NOT EXISTS credential_refresh_leases (
    key TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS secret_store_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

Behaviour:
1. Location `<screenpipe_data_dir>/secrets.sqlite`, rollback-journal mode (only `secrets.sqlite-journal`
   appears in file lists, never `-wal`/`-shm`). Value encoding unchanged: AES-256-GCM with 12-byte
   nonce when a keychain key is present, else base64 with an all-zero nonce.
2. Shared, cached pool per path (a log line reports how many credential pools are cached). Opening waits for older
   connections to close; integrity verification + quarantine reuse the SQLite coordinator (db-019);
   on verified damage, refuse to open and require protected repair.
3. One-shot legacy migration: if `secret_store_metadata` has no row `legacy_main_db_migrated_v1`,
   open `db.sqlite` read-only (`query_only=ON`, `mmap_size=0`), skip if it has no `secrets` table,
   abort with an error telling the user to repair the capture DB first if the main DB is quarantined, else copy all rows
   (`key, value, nonce, created_at, updated_at, expires_at`) with an upsert that only overwrites when the
   incoming `updated_at` is newer, then insert `(legacy_main_db_migrated_v1, 'complete')` in the same
   transaction. A separate "credential recovery" path re-imports from a legacy/migration source after
   storage migration (`recovering_migration_secrets`, see db spec).
4. New helper: count encrypted rows (`SELECT COUNT(*) FROM secrets WHERE hex(nonce) != '000000000000000000000000'`),
   exposed as `encrypted_count` (likely fed to keychain status / disable-encryption warnings).
5. `decrypt_encrypted_secrets` / `reencrypt_unencrypted_secrets` exist in base; unchanged semantics.

Base files to touch: `crates/screenpipe-secrets/src/store.rs` (add `SecretStore::open(data_dir, key)`
that owns its pool on `secrets.sqlite`; keep `new(pool, key)` for tests), new `src/legacy.rs` for the
migration; every construction site currently passing the main DB pool:
`crates/screenpipe-engine/src/bin/screenpipe-engine.rs:1312`, `crates/screenpipe-engine/src/auth_key.rs:176`,
`crates/screenpipe-engine/src/cli/connection.rs:22`, `crates/screenpipe-core/src/pipes/mod.rs:787`,
app `src-tauri/src/oauth.rs::open_secret_store` (opens `db.sqlite` today), app `secrets.rs`;
`crates/screenpipe-vault/src/manager.rs` (file list, see connections-vault-secrets-db); app
`disk_usage.rs` (count `secrets.sqlite*`).

Implementation notes: keep the `secrets` DDL byte-compatible so screenpipe-written stores open in
Freenpipe and vice versa. Never delete the legacy `secrets` table in `db.sqlite` (older builds may
still read it). Make the migration idempotent and safe if two processes (app + CLI engine) race:
do it inside `BEGIN IMMEDIATE`.

### connections-credential-refresh-leases — Cross-process refresh lease

**Kind:** new. **Confidence:** observed (SQL), inferred (callers other than ChatGPT, TTL). **Size:** S.
**Evidence:** `0x49af5b0..0x49af868` (`try_acquire_refresh_lease`, `release_refresh_lease`); ChatGPT
usage `0x49893f0` (an error for timing out while another process refreshes the ChatGPT token, and a log for a failed lease release),
`0xa0441d0` (`oauth:chatgpt`, `reauth_required`).

SQL (verbatim semantics):
* acquire: `INSERT INTO credential_refresh_leases (key, owner, expires_at) VALUES (?, ?, unixepoch() + ?) ON CONFLICT(key) DO UPDATE SET owner = excluded.owner, expires_at = excluded.expires_at WHERE credential_refresh_leases.expires_at <= unixepoch() OR credential_refresh_leases.owner = excluded.owner` — acquired iff one row affected.
* release: `DELETE FROM credential_refresh_leases WHERE key = ? AND owner = ?`.

Behaviour: a refresher (OAuth scheduler, lazy `get_valid_token`, ChatGPT login) acquires a lease
keyed by the secret key it is about to rotate (inferred: same key string as the token, e.g.
`oauth:google-calendar:alice@x.com`, `oauth:chatgpt`) with owner = per-process random id and a short
TTL. A loser waits/polls (re-reading the token, which the winner will have rotated) until a deadline,
then fails with a timeout error saying another refresh was already in progress. Prevents two processes
from burning a rotating refresh token concurrently.

Base files: `screenpipe-secrets/src/store.rs` (two methods), `screenpipe-connect/src/oauth.rs::refresh_token_instance`
and `oauth_refresh_scheduler.rs` (wrap refresh in lease). Defaults (inferred): TTL 60 s, poll 500 ms, wait cap 30 s.

### connections-oauth-byo-broker — Replace the screenpipe OAuth broker (BYO credentials)

**Kind:** decloud. **Confidence:** observed (broker URLs, client ids), inferred (replacement design). **Size:** XL.
**Evidence:** exchange/refresh URL `https://screenpi.pe/api/oauth/exchange` (`0x9f78c8e`, `0x512fa9a`, `0x5186da1`, 15 copies;
request field `subdomain` stored adjacent, i.e. the body now may carry a subdomain); relay
`https://screenpi.pe/api/oauth/callback` (`0x9f96e2e`, `0x9fb13d8`, `0x9fb584a`, `0x9fb95a0`); OAuth configs:

| id | auth URL (T) | client id (T, screenpipe-owned) | scope / extra params (T) | relay | offset |
|---|---|---|---|---|---|
| google-calendar | `https://accounts.google.com/o/oauth2/v2/auth` | `7048263620-kash2fri…` | calendar.readonly userinfo.email; `access_type=offline`, `prompt=consent select_account` | no | `0x9f9c2f5` |
| google-docs | same | `7048263620-v7f7ogpe…` | documents drive.file userinfo.email; offline | no | `0x9f7e0c7` |
| microsoft365 | `https://login.microsoftonline.com/common/oauth2/v2.0/authorize` | `be765a6d-…` | offline_access openid profile Mail.* Calendars.* Files.* Chat.ReadWrite Team.ReadBasic.All ChannelMessage.Read.All; `prompt=select_account` | no | `0x9f7ebf8` |
| outlook-email | same | same | offline_access openid profile Mail.Read Mail.ReadWrite Mail.Send; `prompt=select_account` | no | `0x9f8588d` |
| teams | same | same | Chat.ReadWrite Channel.ReadBasic.All ChannelMessage.Read.All Team.ReadBasic.All TeamMember.Read.All; `prompt=select_account` | no | `0x9f91a50` |
| notion | `https://api.notion.com/v1/oauth/authorize` | `334d872b-…` | `owner=user` | no | `0x9f86029` |
| github | `https://github.com/login/oauth/authorize` | `Ov23li9IT9uV0S9ql2ne` | `repo read:org` | no | `0x9f8ea80` |
| slack | `https://slack.com/oauth/v2/authorize` | `11089811693862.11135517223459` | `user_scope` per variant | yes (inferred) | `0x9f97159` |
| zoom | `https://zoom.us/oauth/authorize` | `EqdPSw5wS76kO3HyRzDb7Q` | granular (base) | yes | `0x9f96e2e` |
| jira | `https://auth.atlassian.com/authorize` | `FqySuu0Acb95…` | read/write:jira-work read:jira-user offline_access; `audience=api.atlassian.com`, `prompt=consent` | no | `0x9fa7f07` |
| calcom | `https://app.cal.com/auth/oauth2/authorize` | `e9e31920…` | BOOKING_READ PROFILE_READ EVENT_TYPE_READ SCHEDULE_READ | no | `0x9f9ce1b` |
| hubspot | `https://app.hubspot.com/oauth/authorize` | `a0587eeb-…` | crm.objects.{contacts,companies,deals}.{read,write} crm.schemas.{contacts,companies,deals}.read oauth | unknown | `0x9fa4308` |
| quickbooks | `https://appcenter.intuit.com/connect/oauth2` | `ABICNYWw…` | com.intuit.quickbooks.accounting | yes | `0x9fb131e` |
| vercel | `https://vercel.com/oauth/authorize` | `oac_RJfAj2…` | — | no | `0x9fb56b0` |
| calendly | `https://auth.calendly.com/oauth/authorize` | `ouME2bVF…` | — | yes | `0x9fb584a` |
| supabase | `https://api.supabase.com/v1/oauth/authorize` | `9ca2e439-…` | — | no | `0x9f78803` |
| zendesk | `https://{subdomain}.zendesk.com/oauth/authorizations/new` | `zdg-screenpipe` | `read write` | yes | `0x9fb9448` |

Every row above depends on the vendor broker (the broker holds the client secret and performs both
`authorization_code` and `refresh_token` grants). Without it Freenpipe cannot connect or refresh any
native OAuth integration. Replacement:

1. Add per-integration BYO client credentials: settings UI "Use your own OAuth app" with `client_id`,
   `client_secret` (secret), optional `redirect_uri`; persist in SecretStore under a new key family
   `oauth-client:<integration_id>` (Freenpipe-only key; screenpipe never reads it). Do not ship
   screenpipe's client ids (useless without the broker's secret and tied to their brand).
2. Extend `OAuthConfig` with `token_url` and `token_auth` (`body` vs HTTP Basic), `pkce: bool`,
   `refresh_supported: bool`. Exchange/refresh POST directly to the provider (form-encoded
   `grant_type`, `code`/`refresh_token`, `redirect_uri`, `client_id`, `client_secret`, `code_verifier`).
   Keep the response normalisation the base already has (`merge_refresh_response`, `expires_at`,
   `screenpipe_refresh_gen`, `last_refreshed_at`) so stored JSON stays screenpipe-compatible.
   Token endpoints (from provider public docs, not target evidence): Google
   `https://oauth2.googleapis.com/token`; Microsoft `https://login.microsoftonline.com/common/oauth2/v2.0/token`;
   Notion `https://api.notion.com/v1/oauth/token` (Basic); GitHub `https://github.com/login/oauth/access_token`
   (send `Accept: application/json`); Slack `https://slack.com/api/oauth.v2.access`; Zoom `https://zoom.us/oauth/token`
   (Basic); Atlassian `https://auth.atlassian.com/oauth/token`; Intuit
   `https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer` (Basic); Vercel
   `https://api.vercel.com/v2/oauth/access_token`; Calendly `https://auth.calendly.com/oauth/token`;
   Supabase `https://api.supabase.com/v1/oauth/token` (Basic); HubSpot `https://api.hubapi.com/oauth/v1/token`;
   Zendesk `https://{subdomain}.zendesk.com/oauth/tokens`; Cal.com — verify against current Cal.com docs.
3. Redirect: default loopback `http://localhost:3030/connections/oauth/callback` (`OAUTH_REDIRECT_URI`).
   For providers the target routes through the HTTPS relay (zoom, quickbooks, calendly, zendesk, slack),
   accept a user-provided HTTPS relay URL (`redirect_uri` field) that 302s to the loopback with the
   same query; otherwise show "this provider needs an HTTPS redirect" and steer to the token fallback
   where one exists (hubspot, zendesk).
4. When no BYO credentials are configured, OAuth tiles show "Configure OAuth app" instead of
   "Connect"; API-key fallbacks keep working. Remove the base Pro gate (`cloud_subscribed`) in
   `apps/screenpipe-app-tauri/src-tauri/src/oauth.rs`; the target build no longer contains that string.

Base files: `crates/screenpipe-connect/src/oauth.rs` (`EXCHANGE_PROXY_URL`, `exchange_code`,
`refresh_token_instance`), every `connections/*.rs` holding an `OAuthConfig`, app `oauth.rs`,
`connections-section.tsx` OAuth panels, `oauth_refresh_scheduler.rs`.

### connections-oauth-connect-v2 — OAuth flow: variants, subdomain, longer window, better callback

**Kind:** changed. **Confidence:** observed. **Size:** M.
**Evidence:** app `oauth.rs` strings `0x5112140..0x51129ef` (`{subdomain}`, invalid-subdomain error, `SCREENPIPE_OAUTH_TIMEOUT_SECS`,
missing-subdomain error, port-3030 check, provider error, unknown-scope-variant error, reconcile default slot,
identity URLs `0x5112ae8`); callback pages `0x517d3d8..0x517d74c`; `OAuthCallbackQuery` gains `error_description`
(`0x517e7b8`); bindings `oauth_connect(integrationId, instance, variant)`, `activate_app_after_oauth`; UI `5977…js` @308808 (`sK` variants), @309081 (Zendesk hints).

Behaviour changes vs base `oauth_connect`:
1. New `variant: Option<String>` argument. Integrations declare named scope variants; each variant
   overrides the scope parameter. Unknown variant → error naming both the rejected variant and the integration id.
   Only Slack declares variants in the target (`send`, `read_write`; see slack item). The UI deep-link
   event `open-settings` carries `{section:"connections", connectionId, scopeVariant}` and pre-selects it.
2. `{subdomain}` placeholder in `auth_url`: for integrations whose auth URL contains it (Zendesk), the
   `instance` argument is the subdomain. Validate `^[A-Za-z0-9-]+$`; empty → error saying the
   integration requires a subdomain; store it as `subdomain` in the token JSON and pass it to the exchange.
3. Waiting window: default 10 minutes (UI copy says a fresh sign-in stays valid for 10 minutes),
   overridable by env `SCREENPIPE_OAUTH_TIMEOUT_SECS`. Base: 120 s.
4. Refuse to start if the local API is not on port 3030 (callback URL is fixed); error tells the user
   to reset the port.
5. Callback (`GET /connections/oauth/callback`): read `error` + `error_description`; if a pending flow
   matches `state`, resolve it with the provider error so the awaiting command fails immediately
   (with an error saying the user refused or abandoned consent in the browser). Distinct HTML outcomes (write new copy):
   success; provider-reported failure; stale or unknown `state` (link reused);
   flow no longer awaited (state known but the app stopped waiting); malformed callback (missing code/state).
   Pending entries carry a deadline so "expired" vs "unknown" can be told apart.
6. Identity lookup for instance naming extended: zoom `https://api.zoom.us/v2/users/me`, calendly
   `https://api.calendly.com/users/me` (`resource.uri`/name), cal.com `https://api.cal.com/v2/me`
   (`cal-api-version: 2024-08-13`), plus base github/notion/vercel.
7. After save, "reconcile" (not delete) the default slot — see connections-oauth-default-slot-promotion.
8. `activate_app_after_oauth` Tauri command: frontend calls it when the flow resolves to refocus
   the window (macOS mostly; no-op acceptable on Windows).
9. UI shows a provider-specific hint after 8 s of pending (Zendesk: suggest token fallback).

Base files: `apps/screenpipe-app-tauri/src-tauri/src/oauth.rs`, `crates/screenpipe-connect/src/oauth.rs`
(`PendingOAuth` gets `deadline`; `OAuthConfig` gets `variants: &[(id, scope_param, scope)]`),
`crates/screenpipe-engine/src/connections_api.rs::oauth_callback`, `lib/utils/tauri.ts` (regenerate bindings).

### connections-oauth-default-slot-promotion — Promote instead of sweep

**Kind:** changed. **Confidence:** observed (messages), inferred (exact algorithm). **Size:** S.
**Evidence:** `0x512fe18..0x5130223` (a log for moving a shadowed default-slot account into a named instance,
an internal invariant check that `distinct_recoverable` carries an identity, and an error for a failed
default-slot reconcile after an instanced save).

Behaviour: base `sweep_shadowed_default_slots` deletes `oauth:<id>` whenever a named `oauth:<id>:<x>`
exists. Target: if the default slot is recoverable and its identity (email/workspace/team id) differs
from every named instance, move it to `oauth:<id>:<identity>` (then delete the default slot); only
delete it outright when it is unrecoverable or duplicates a named identity. On failure to promote,
leave it intact. Same reconcile runs after an instanced save in `oauth_connect`.
Base files: `crates/screenpipe-connect/src/oauth.rs` (sweep), app `oauth.rs` (post-save).

### connections-secret-redaction — Connection reads never return secrets

**Kind:** changed. **Confidence:** observed. **Size:** M.
**Evidence:** skill text `0x511a9aa` (reads return status + declared non-secret settings only; no
`/connections/<id>/token`); render_context lines `0x49ae0f4..0x49ae17a` / `0x5185884..0x518590a`
(a `config` line labelled as non-secret settings with a note that credentials remain on the server, and a
`webhook: POST …/proxy` line); per-integration
context lines (Telegram `0x9f8d850`, IMAP `0x9f8ca59`); frontend helper filtering credentials to fields with
`secret:false` (`5977…js` @~15119) and reads of `credentials.vault_path` / `credentials.username`.

Behaviour:
1. `GET /connections/:id` → `{"credentials": {<only fields whose FieldDef.secret == false>}}` (shape
   preserved, secret keys omitted, no `X-Deprecation-Warning`). OAuth-only integrations return `{}`.
2. `GET /connections/:id/instances` → each item `{instance, enabled, credentials:{non-secret}}`;
   for OAuth integrations list connected OAuth instances too (`instance`, `display_name`) — required by
   the Outlook description, which sends the agent here to enumerate connected accounts.
3. `render_context` (agent system context): proxy integrations show proxy + config lines; webhook
   integrations show one `webhook: POST <base>/<id>/proxy` line; Telegram and IMAP show a line pointing
   at their dedicated endpoints; no integration prints secret field values. Non-proxy local integrations
   (obsidian, logseq, hermes, openclaw, ntfy) may print non-secret fields only (inferred).
4. Update the bundled `screenpipe-api` skill "Connections" section: list/status/non-secret config,
   Telegram send, webhook proxy, Discord/Teams shapes, generic proxy, calendar, browser control incl.
   `/act` (write new text).

Base files: `crates/screenpipe-engine/src/connections_api.rs` (`get_connection`, `list_instances`),
`crates/screenpipe-connect/src/connections/mod.rs` (`render_context`; helper `non_secret_view(def, creds)`),
`crates/screenpipe-core/assets/skills/screenpipe-api/SKILL.md` (and the app copy).

### connections-webhook-proxy — `POST /connections/:id/proxy` for webhook integrations

**Kind:** new. **Confidence:** observed (route, error codes, descriptions), inferred (exact status codes). **Size:** M.
**Evidence:** route `/:id/proxy` `0x5182cb0`; error identifiers `0x517d028..0x517d238`
(`unsupported_query`, `webhook_proxy_unavailable`, `https`, `invalid_webhook_target`,
`connection_not_configured`, `upstream_rejected`, `transport_failed`, a completion log line);
descriptions discord `0x9f97410`, zapier `0x9f9d198`, make `0x9fb14a8`, n8n `0x9fb8f50`, teams `0x9f9195d`.

Behaviour: for integrations whose credential is a webhook URL (`discord`, `zapier`, `make`, `n8n`,
`teams` webhook mode), `POST /connections/<id>/proxy[?instance=]` forwards the request body (JSON) to
the stored `webhook_url` and returns the upstream status/body. Rules: only POST; reject any other query
param (`unsupported_query`); integration without webhook support → `webhook_proxy_unavailable`; no saved
URL → `connection_not_configured`; stored URL must parse and be `https` (`invalid_webhook_target`);
non-2xx upstream → `upstream_rejected` (502); network error → `transport_failed` (502). Error body
`{"error": <message>, "code": <identifier>}` (inferred). Body shapes the agent uses: Discord
`{"content":...}`, Teams `{"text":...}`, Zapier/Make/n8n arbitrary JSON.
Base files: `connections_api.rs` (new handler + route before `/:id/proxy/*path`), `connections/{discord,zapier,make,n8n,teams}.rs`
(add `webhook_field()` trait hook), descriptions.

### connections-telegram-send — `POST /connections/telegram/send`

**Kind:** new. **Confidence:** observed. **Size:** S.
**Evidence:** route `0x5182bb8`; struct `TelegramSendRequest` `0x517e810`; errors `0x517ceb0..0x517d000`
(empty text, not connected, missing bot token/chat id, `message_id`); connect strings `0x5186330`.

Behaviour: body `{"text": "..."}` (+ optional `instance` inferred). Load `bot_token`/`chat_id`, POST
`https://api.telegram.org/bot<token>/sendMessage` `{chat_id, text}`; map Telegram `ok:false` →
an error saying Telegram refused the message, carrying Telegram's `description`; success → `{"ok": true, "message_id": <n>}`.
Errors: 400 empty text; 404/400 not connected; 400 missing fields; 502 upstream.
Description of the `telegram` integration changes to point here (no raw credentials).
Base files: `connections_api.rs`, `connections/telegram.rs`.

### connections-slack-user-token — Slack acts as the user, with read access option

**Kind:** changed. **Confidence:** observed (scopes, routes, messages), inferred (response shapes). **Size:** L.
**Evidence:** def/description `0x9f97130`; variants `0x9f96e2e..0x9f96fa0` (`send` → `user_scope=chat:write`;
`read_write` → `chat:write,search:read,channels:history,groups:history,im:history,mpim:history,channels:read,groups:read,im:read,mpim:read,users:read`);
engine `0x517c818..0x517cb80`, `0x517ccf8`, `0x517ceb0`, `0x517d894`, `0x517e370`, `0x517e658`;
app slack fields `0x511267c` (`team`, `team_id`, `incoming_webhook`, `slack_channel`, `channel_id`,
`slack_channel_id`, `authed_user`, `slack_user_id`, "direct message"); test `0x9f962c0..0x9f96484`;
UI `5977…js` @308808 (variant labels), @510436 (`/slack/conversations?limit=200` → `channels[]`).

Behaviour:
1. OAuth requests a **user** token via `user_scope` (scope set chosen by variant; default `send`).
   Token response's `authed_user` holds the user token; store `slack_user_id`, `team_id`,
   `workspace_name` at top level; instance key = `team_id` (multi-workspace).
2. `POST /connections/slack/send` body `{text?, channel?, blocks?, attachments?, …, instance?}`: requires
   text or another payload field; default channel = the user's own DM (open via the user id); legacy
   tokens that only have `incoming_webhook` still post to the webhook; Slack `not_in_channel` → error
   explaining the channel-membership requirement and pointing at the `read_write` variant. Response `{ok, channel, ts}` (inferred from `ts`, `team` keys).
3. `GET /connections/slack/search?q=…[&instance=]` → `search.messages` (`query`). Requires `read_write`;
   otherwise 403 with an error saying the token lacks read access and must be reconnected with the `read_write` variant.
4. `GET /connections/slack/conversations?limit=…[&instance=]` → `conversations.list`
   `types=public_channel,private_channel,im,mpim`; response `{"channels":[{id,name,…}]}`.
5. `GET /connections/slack/history?channel=…&limit=…` → `conversations.history`.
6. `instance` may come from the JSON body or the query; omitting it is only valid with one workspace.
7. `test()`: with a user token, sends a short test DM to the user's own account; with webhook, posts.
Base files: `connections/slack.rs`, `connections_api.rs` (`slack_send` rewrite + 3 handlers), app
`oauth.rs` (Slack post-processing), `connections-section.tsx` (variant picker).

### connections-imap — Read-only IMAP inbox integration

**Kind:** new. **Confidence:** observed (fields, routes, crates, IMAP verbs), inferred (JSON shapes). **Size:** L.
**Evidence:** def `0x9f9c69e..0x9f9ca39`; connect strings `0x9f9b7d8..0x9f9b86b`, `0x9f9c500..0x9f9c69e`;
engine `0x5185d11..0x5185f90` (`LIST`, `EXAMINE`, search fetch `(UID INTERNALDATE RFC822.HEADER)`, message-not-found-in-mailbox
and message-parse-failure errors); routes `0x5182a90..0x5182ae8`; crates async-imap 0.11.3,
mail-parser 0.11.5; UI `5977…js` @35937 (form), @36452 (`/connections/imap/test` then PUT), @515456
(`/connections/imap/mailboxes` → `{"mailboxes":[string]}`).

Definition: id `imap`, name "Email Inbox (IMAP)", fields `imap_host` (placeholder `imap.gmail.com`),
`imap_port` (default `993`), `username` (email), `password` (secret; app password). No proxy.
Behaviour:
* Connect: TCP + TLS (rustls, SNI = host) to `imap_host:imap_port`, `LOGIN`, `EXAMINE` (read-only) the
  mailbox, `LOGOUT`. All operations bounded by a timeout (the timeout error reports the elapsed seconds; use 20 s).
* `test()`: open INBOX, report success together with the inbox message count. Login failure mapping:
  if the server text mentions app-specific password/2FA or host is Gmail, explain that Gmail needs an
  app password (myaccount.google.com/apppasswords) and Workspace admins may need to enable IMAP.
* `GET /connections/imap/messages?limit=20&mailbox=INBOX&query=<text>[&instance=]`: EXAMINE mailbox;
  `UID SEARCH` (`ALL` or `TEXT "<query>"`); take newest `limit` UIDs; `UID FETCH (UID INTERNALDATE RFC822.HEADER)`;
  parse headers with mail-parser. Proposed item shape `{uid, subject, from, to, date, mailbox}` + response
  `{messages:[…], mailbox}`.
* `GET /connections/imap/messages/{uid}?mailbox=` → full RFC822 fetch, parsed `{uid, subject, from, to, cc, date,
  text, html?}`; 404 when the UID is absent from the mailbox; 422 unparsable/no body.
* `GET /connections/imap/mailboxes` → `LIST "" "*"` → `{"mailboxes": [names]}`.
* render_context line says credentials are stored locally and to use these endpoints.
* UI: email field auto-derives host (`imap.<domain>`, Gmail detection), port default 993, test-then-save.
* Feeds connection triggers `email_received` (kind `message`) / `email_sent` (kind `sent_message`).
Base files: new `crates/screenpipe-connect/src/connections/imap.rs`, registry in `connections/mod.rs`,
`Cargo.toml` (`async-imap` with `runtime-tokio`, `mail-parser`, `tokio-rustls`), `connections_api.rs`
(3 handlers), settings UI panel.

### connections-outlook-email — Outlook mail via Microsoft Graph

**Kind:** new. **Confidence:** observed. **Size:** M.
**Evidence:** def `0x9f85388..0x9f8588d`; test `0x9f84ee8..0x9f85062`; trigger kinds in UI catalog @488554.

Definition: id `outlook-email`, name "Outlook", OAuth (MS client, scopes `offline_access openid profile
Mail.Read Mail.ReadWrite Mail.Send`, `prompt=select_account`), proxy base `https://graph.microsoft.com/v1.0`
(Bearer, path given without version), `supports_oauth_instances` true (multi-account, instance = email).
`test()`: `GET /me/mailFolders/inbox?$select=displayName,totalItemCount,unreadItemCount` → success message with the unread count.
Description (rewrite): use `/connections/outlook-email/proxy/me/messages`, `/me/mailFolders`, `/me/messages/{id}`,
`POST me/sendMail`; list accounts via `GET /connections/outlook-email/instances`; personal-account
`/me` quirk. Triggers: `outlook_received` (kind `message`), `outlook_sent` (kind `sent_message`).
Base files: new `connections/outlook_email.rs` (clone microsoft365 with narrower scopes), registry.
Needs BYO Microsoft app (connections-oauth-byo-broker).

### connections-hermes — Hermes agent integration

**Kind:** new. **Confidence:** observed. **Size:** S.
**Evidence:** `0x9f845b0..0x9f84648` (`endpoint`, `/v1/models`, `token`, Hermes test success message),
def `0x9f85b00..0x9f85f78`; UI hides `hermes` and `openclaw` from the main grid (`5977…js` @349519).

Definition: id `hermes`, name "Hermes", fields `endpoint` (label "API Server URL", default/placeholder
`http://127.0.0.1:8642`) and `token` (secret, optional, "API Server Key"). `test()`: `GET {endpoint}/v1/models`
with `Authorization: Bearer {token}` when set. Description (rewrite) tells the agent it can chat with the
agent via the OpenAI-compatible `POST {endpoint}/v1/chat/completions` (model `hermes-agent`), inject into a
session via `POST {endpoint}/api/sessions/{id}/chat`, and schedule via `POST {endpoint}/api/jobs`; server is
started with `hermes gateway` and `API_SERVER_ENABLED=true`. No proxy (local service).
Base files: new `connections/hermes.rs` modelled on `openclaw.rs`.

### connections-hubspot-oauth — HubSpot OAuth with token fallback

**Kind:** changed. **Confidence:** observed. **Size:** S.
**Evidence:** `0x9fa3587..0x9fa36fb` (test via `https://api.hubapi.com/account-info/v3/details`, `portalId`),
def `0x9fa4210..0x9fa4538`.
Behaviour: `oauth_config()` (see table) AND keep field `api_token`, relabelled to mark it as an optional fallback
(its placeholder tells the user to leave it empty when using OAuth). Proxy Bearer: OAuth token wins, else `api_token` (base
`resolve_auth` already does this). `test()`: account-info details → success message including the portal id.
UI: OAuth primary; an Advanced disclosure for token-based connection shows the field form.
Base files: `connections/hubspot.rs`, `connections-section.tsx`.

### connections-zendesk-oauth — Zendesk OAuth per subdomain

**Kind:** changed. **Confidence:** observed (config), inferred (exchange body). **Size:** M.
**Evidence:** `0x9fb8068..0x9fb81a4` (`/token`, "(OAuth)"), def `0x9fb9448`; app subdomain validation `0x5112140`;
UI hints `5977…js` @309081 (instance input "yourcompany").
Behaviour: OAuth auth URL `https://{subdomain}.zendesk.com/oauth/authorizations/new`, client id
`zdg-screenpipe` (vendor; replace with BYO), scope `read write`, relay redirect. The user types the subdomain
in the OAuth panel (passed as `instance`); exchange body carries `subdomain`; token JSON stores `subdomain`
so the proxy base `https://{subdomain}.zendesk.com/api/v2` resolves from OAuth extras. Fallback fields
`subdomain`, `email`, `api_token` → Basic `email/token:api_token` (inferred from `/token` literal; base used
`email:api_token`, check Zendesk docs: API-token auth is `{email}/token:{api_token}`). `test()` reports
the connected user, with an OAuth suffix when OAuth was used. UI: on failure/timeouts suggest the token fallback.
Base files: `connections/zendesk.rs`, `oauth.rs` (subdomain templating), `connections_api.rs::resolve_auth` (Basic `/token`).

### connections-oauth-scope-tweaks — Small OAuth config changes

**Kind:** changed. **Confidence:** observed. **Size:** S.
* `github`: scope `repo read:org` (base `repo`) `0x9f8ea80`. Description now documents issue + comment
  creation via proxy `repos/{owner}/{repo}/issues[/{n}/comments]` `0x9f8e8d3`.
* `microsoft365`, `teams`, `outlook-email`: `prompt=select_account` (base `consent select_account`)
  `0x9f7ebf8`, `0x9f91a50`, `0x9f8588d`. Google keeps `consent select_account`.
Base files: `connections/github_issues.rs`, `microsoft365.rs`, `teams.rs`.

### connections-gmail-sheets-composio — Gmail/Drive/Docs/Sheets/Zoom moved to Composio (vendor cloud)

**Kind:** decloud (target: removed native + vendor MCP). **Confidence:** observed. **Size:** L.
**Evidence:** no `gmail.rs`/`google_sheets.rs` panic paths, no Gmail/Sheets client ids or `gmail.googleapis` host;
route table lacks `/gmail/*`; Composio client `5977…js` @391938 (`/api/composio` on `https://screenpipe.com`,
`status`, `authorize {toolkit, alias?}` → `redirect_url`, MCP server id `composio` with URL
`<base>/mcp` and `Authorization: Bearer <screenpipe user token>`, toolkits `gmail, zoom, googledrive, googledocs, googlesheets`);
pi extension `0x52a6925..0x52a75d8` (synthesises Composio connections, server URL regex for screenpipe.com/screenpi.pe);
onboarding page uses the same.

Target behaviour (for reference only): tiles Gmail, Google Drive, Google Docs, Google Sheets, Zoom
connect through screenpipe.com's Composio passthrough using the user's screenpipe login; connected status
comes from `GET /api/composio/status`; agent calls Composio tools via the `composio` MCP server.
Native `google-docs` stays and wins when connected; native `zoom` moves under an Advanced section marked legacy.

Freenpipe: do NOT port. Keep the base native `gmail` (routes `/gmail/instances`, `/gmail/messages`,
`/gmail/messages/:id`, `/gmail/send`) and `google-sheets` integrations, wire them to BYO Google OAuth,
and add a native `google-drive` integration (proxy `https://www.googleapis.com`, scope `drive.readonly` or
`drive.file`) so the five tiles stay functional locally. Optional: "Composio (bring your own API key)"
as a plain custom MCP server pointing at Composio directly. UI tile ids to keep for interop:
`gmail`, `google-drive`, `google-docs`, `google-sheets`, `zoom`.
Base files: keep `connections/gmail.rs`, `google_sheets.rs`; new `google_drive.rs`; `connections-section.tsx`.

### connections-mcp-oauth-tiles — MCP-first connections for vendor-hosted MCP servers

**Kind:** changed. **Confidence:** observed. **Size:** M.
**Evidence:** provider map in engine `0x6a33590` and pi extension `0x52a5dae`; UI list `s6` (`5977…js` @~349519,
ids/urls below), status loop calling `/mcp-servers` + `/mcp-servers/{id}/oauth/status`; `s9()` merge rule;
Excalidraw MCP URL `https://api.excalidraw.com/api/v1/mcp` (minified const `te`) @363632 with API-key header; engine
warning log, emitted when the MCP server list cannot be read during the status pass, `0x5182620`.

Provider map (connection id → MCP URL): `linear` https://mcp.linear.app/mcp, `stripe` https://mcp.stripe.com,
`sentry` https://mcp.sentry.dev/mcp, `intercom` https://mcp.intercom.com/mcp, `asana` https://mcp.asana.com/mcp,
`monday` https://mcp.monday.com/mcp, `clickup` https://mcp.clickup.com/mcp, `airtable` https://mcp.airtable.com/mcp,
`confluence` and `jira` https://mcp.atlassian.com/v1/mcp, `notion` https://mcp.notion.com/mcp.
Base already has `krisp` https://mcp.krisp.ai/mcp and `plaud` https://mcp.plaud.ai/mcp OAuth panels.

Behaviour:
1. Settings tile for each provider opens an MCP-OAuth panel first (register/enable an MCP server with that
   URL via `PUT /mcp-servers/{id}`, start `/mcp-servers/{id}/oauth/start`, poll status). The native
   integration form moves under an Advanced disclosure offering API-key connection instead (worded as
   managing the existing connection when native is connected).
2. Tile connected = native connected OR (enabled MCP server whose URL equals the provider URL, trailing
   slashes ignored, AND its OAuth status `connected`).
3. `GET /connections`: for provider ids, mark `connected:true, mcp:true, mcp_server_id` when (2) holds via
   MCP (engine reads `mcp_servers.json` + `mcp-oauth:<id>` presence). The agent is told to use
   `sp_mcp_list_tools`/`sp_mcp_call` for MCP-connected ones and the proxy otherwise.
4. Excalidraw (`excalidraw`): API-key MCP — test `POST /mcp-servers/test {url, headers:[{name:"Authorization",value:"Bearer <key>"}]}`,
   then `PUT /mcp-servers/{random id}` `{name, url, headers, enabled:true}`.
5. MCP OAuth redirect must stay on the loopback (`http://localhost:3030/mcp-servers/<id>/oauth/callback`,
   base behaviour); do not use the target's `https://screenpi.pe/api/oauth/mcp/<id>/callback?app_scheme=`
   relay (`0x6a8dd01`). Providers that only accept HTTPS redirect URIs are an open question.
Base files: `connections-section.tsx` (generalise `OAuthMcpPanel` to a provider list), `connections_api.rs::list_connections`
(+MCP enrichment), `crates/screenpipe-connect/src/mcp_servers.rs` (helper `find_server_by_url`).

### connections-pi-connect-tools — Agent tools `screenpipe_list_connections` / `screenpipe_connect_app`

**Kind:** new. **Confidence:** observed. **Size:** M.
**Evidence:** embedded pi extension TS `0x52a59e8..0x52a9c79` (copy at `0xa0cc…`); chat handling in
`3696-4fbfea4ced3de899.js` (listens for pi `extension_ui_request` with `method:"confirm"` and title prefix
`screenpipe:connect:`, renders a `connection_action` content block, answers via the existing
`pi_extension_ui_response` command). Base bundles only `mcp-bridge.ts` and `web-search.ts`.

Behaviour (paraphrase, write new TS):
* Extension reads `SCREENPIPE_LOCAL_API_URL` / `SCREENPIPE_LOCAL_API_PORT` / `SCREENPIPE_PORT` (default 3030) and
  `SCREENPIPE_LOCAL_API_KEY` / `SCREENPIPE_API_AUTH_KEY` for Bearer auth.
* Tool `screenpipe_list_connections` (param `connected_only: boolean`): `GET /connections`, enrich with
  MCP-OAuth provider status (connections-mcp-oauth-tiles), drop `owned-default`, return JSON
  `{connections:[{id, name, connected, connected_via: "mcp"|"connection_proxy", mcp, mcp_server_id, category, description, action_hint}]}`
  where `action_hint` tells the model which mechanism to use.
* Tool `screenpipe_connect_app` (params `connectionId` required, `reason`, `requiredFor`; sequential): if already
  connected → `{status:"connected", …payload}`; if no UI → `declined` with a short message; else emit
  progress `{status:"waiting"}`, call `ctx.ui.confirm` with title `screenpipe:connect:<id>:<name>` and a message
  built from `reason`, optionally followed by a sentence naming `requiredFor` as the work that resumes once
  connected; on decline → `declined`; on accept re-fetch and
  return `connected` or `failed` (isError). Aborts → `declined`. Tool guidance (in our words): check status before
  relying on an app, wait for the connect tool to finish, and stop if the app is still not connected.
* Chat UI: a `connection_action` block shows the app icon/description with Connect/Decline; Connect opens
  Settings > Connections at that id (`open-settings` event), and the block resolves the confirm request
  with the user's choice once the connection flips.
Base files: new `crates/screenpipe-core/assets/extensions/connections.ts` (+ app copy), extension
installer in `screenpipe-core/src/agents/pi.rs`, chat components (content block type `connection_action`).

### connections-bee-pairing — One-click Bee pairing

**Kind:** new (third-party pairing; app id is vendor-registered → BYO). **Confidence:** observed (routes, URLs, flow), inferred (crypto). **Size:** M.
**Evidence:** routes `0x5182c60`, `0x5182c88`; `BeePairPollBody` `0x517e740`; engine `0x5185bec..0x5185ce4`
(`https://auth.beeai-services.com/apps/pairing/request`, app id `ph9fssu1kv1b0hns69fxf7rx`); connect `0x9f96741..0x9f96a2c`
(`https://bee.computer/connect#`, `encryptedToken`, `requestId`, `expiresAt`, token version/length/base64/utf-8 errors,
pairing-key generation error); `rsa 0.9.10` + `aes` in crates; UI `5977…js` @327708.

Flow:
1. `POST /connections/bee/pair/start` → server generates an ephemeral key pair (RSA inferred), POSTs to the
   Bee pairing service with the app id and public key, receives `requestId` + `expiresAt`, keeps the private
   key in memory keyed by request id, returns `{request_id, pairing_url, expires_at}` where `pairing_url` is
   `https://bee.computer/connect#<fragment carrying request id / key>` (fragment format inferred).
2. UI opens `pairing_url`, then every 2 s calls `POST /connections/bee/pair/poll {"request_id"}`; gives up after 300 s.
3. Poll → server queries the pairing service; returns `{status:"pending"|"completed"|"expired"|"unknown"}`. On
   `completed` it decrypts `encryptedToken` (versioned, base64; hybrid RSA+AES inferred), validates UTF-8,
   saves it as the `bee` connection's `api_key`, clears tombstone, returns `completed`.
4. Errors: non-JSON, unexpected status, missing fields, decryption failures → 502 with message.
Developer-token entry (base) remains under "Advanced". Description updated to mention both paths.
Freenpipe: make the Bee app id a setting (`bee_app_id`, empty by default → hide one-click button); never ship
screenpipe's id. Base files: `connections/bee.rs`, `connections_api.rs`, `connections-section.tsx`.

### connections-obsidian-memories — Memory digest into an Obsidian vault

**Kind:** new. **Confidence:** observed (def, file name, frontmatter keys), inferred (digest format). **Size:** M.
**Evidence:** def `0x9f7ed88..0x9f7f065`; test `0x9f7dbb1..0x9f7dd17` (`.obsidian` check, `.screenpipe-write-probe`);
external sync `0xa0ba74f..0xa0bae59` (`@privacy:local-on…`, `obsidian-memorie…`, frontmatter `title: screenpipe memories`,
tags `screenpipe`, `memory`, empty-state line); UI `5977…js` @284101 (`POST /memories/sync-external` result
`{results:[{destination_id, outcome:{ok, result:{wrote:{entries}}|{unchanged:{entries}}|{skipped:{reason}}, error}}]}`),
@290364 (`sU="obsidian-memories"`, default folder `sF="screenpipe"`).

Definition: id `obsidian-memories`, name "Obsidian Memories", fields `vault_path` (placeholder
`/Users/you/Documents/MyVault`), `memories_folder` (optional, default `screenpipe`). `test()`: path exists,
is a directory, contains `.obsidian/`, writable (create+delete `.screenpipe-write-probe`), → ready status naming the path.
Sync: new destination in `external_memory_sync` writing `<vault>/<memories_folder>/screenpipe-memories.md`,
rewritten whole each run, YAML frontmatter (title, tags list) + heading + one bullet per durable memory
(+ `#tags`), empty-state line when none. Folder sanitised: split on `/`/`\`, drop empty/`.`/`..`, join with `/`.
Memories flagged local-only (tag `@privacy:local-only`, inferred) are excluded from all external sinks.
UI: shown inside the Obsidian panel as a beta memory-sync section with a manual sync button; pre-fills vault from `obsidian`.
Base files: new `connections/obsidian_memories.rs`, `crates/screenpipe-engine/src/external_memory_sync.rs`
(add destination), `crates/screenpipe-core/src/memories/external_sync.rs` (renderer).

### connections-agent-memory-guidance — Claude Code / Codex sync static MCP guidance

**Kind:** changed. **Confidence:** observed. **Size:** S.
**Evidence:** descriptions `0x9f8e620`, `0x9f90f10`; renderer text `0xa0babd3..0xa0bae71`; disconnect cleanup
`0x5185988` (an error for a failed memory-copy removal in which restoring the connection also failed).
Behaviour: the `claude-code` sidecar `screenpipe-memories.md` and the `codex` marker block (between
`<!-- screenpipe-memories:start v… -->` and `<!-- screenpipe-memories:end -->`, base markers) now contain static
guidance telling the CLI agent to query memories on demand through the Screenpipe MCP `search-content`
tool (`content_type` = `memory`) and to treat results as untrusted data — raw memory text is no longer
written. Disconnecting removes both screenpipe-owned copies; if removal fails the connection is restored
and an error returned. Base files: `crates/screenpipe-core/src/memories/external_sync.rs`,
`external_memory_sync.rs`, `connections/{claude_code,codex}.rs`, `connections_api.rs::disconnect_integration`.

### connections-browser-snapshot-v2-act — Ref-stamped snapshot and `POST /browsers/:id/act`

**Kind:** new (act) + changed (snapshot). **Confidence:** observed. **Size:** M.
**Evidence:** snapshot script `0x517eb7f..0x518257c`; act script `0x6a320ee..0x6a33382`; route `0x5182760`;
`BrowserActBody{ref,action}` `0x517e6e8`; actions literal `0x517d918`; errors `0x517d9d9..0x517da79`;
Control block `0x6a3384c..0x6a33b5f`; skill text `0x511b0dc`.

Snapshot v2 (replace base `SNAPSHOT_SCRIPT`; write new JS):
* Wait for DOM ready ≤ 5000 ms; `MAX_LINES = 250`, `MAX_DEPTH = 12` (base 8).
* Before walking, remove every existing `data-sp-ref` attribute across light DOM, open shadow roots and
  same-origin iframes.
* Walk `document.body`, piercing open shadow roots and same-origin iframes; cross-origin iframes become a
  single placeholder `iframe` line marking the frame as cross-origin and unreadable.
* Skip script/style/noscript/template, `aria-hidden=true`, display:none/visibility:hidden, role
  presentation/none, hidden inputs. Unrendered nodes (< 2 px, opacity 0, fully off top/left) are not emitted
  but their children are walked at the same depth.
* Interactive if: interactive ARIA role (button, link, checkbox, menuitem*, option, radio, switch, tab, textbox,
  combobox, searchbox, slider, spinbutton); button/select/textarea/summary; non-hidden input; contenteditable;
  onclick handler; tabindex ≥ 0; `<a>` with navigable href (not empty, `#`, `javascript:`); div/span/li/td with
  `cursor:pointer`, visible text and no interactive descendant. `pointer-events:none` disqualifies unless disabled.
* Structural tags (h1–h6, nav, main, article, section, form, fieldset, legend, dialog, header, footer, aside) are
  emitted only when they have an accessible name.
* Accessible name: aria-label → aria-labelledby → input rules (button-like inputs use value; others placeholder →
  associated label → name → type) → form tags → text (≤120 chars) → child `img[alt]` → title/value.
* Each interactive element gets `ref = "e" + n` stored in `data-sp-ref` and rendered `#eN`; line format
  `<indent><kind> "<name≤100>" #eN (flags) → <href≤80> = <value≤60>`; kind = role, or `link`/`button` for anchors,
  `button` for pointer-cursor containers, else tag. Flags: disabled, checked, expanded/collapsed, selected,
  required, covered (center point hit-test via `elementFromPoint` on its root returns an unrelated element).
  Password input values are never emitted (the field itself is now listed so login flows work).
* Return `{title, url, tree, count, truncated}` (`count` new).

`POST /connections/browsers/:id/act` body `{"ref":"e7","action":"click"|"fill"|"type"|"clear"|"check"|"uncheck"|"select"|"hover"|"focus","value"?}`:
* 400 if `ref` missing (error points the caller at `/snapshot`) or action unknown (list valid actions); 404 unknown browser.
* The handler builds a script by prepending three JSON-encoded constants (`REF`, `ACTION`, `VALUE`) to a fixed
  act body (no string interpolation of page data) and runs it through the browser's eval transport with the
  caller's `x-screenpipe-session` owner.
* In-page: resolve by `[data-sp-ref="<ref>"]` across document, open shadow roots and same-origin iframes;
  not found → `{ok:false, error}` where the error tells the caller to take a fresh `/snapshot`; scroll into view (center); click → `el.click()`;
  fill/type/select/clear → type-aware setter (input/textarea via native prototype `value` setter + bubbling
  `input` and `change`; select by option value or trimmed label; contenteditable via textContent + `input`);
  unsupported element → error; check/uncheck toggles only when state differs; hover dispatches
  mouseover/mouseenter/mousemove; focus. Success `{ok:true, ref, action, tag, url}`.
* Status mapping like `/eval` (200/422/503/502/504). Timeout 15 s (inferred).
`format_browser_description` Control block adds the `/act` line and describes snapshot refs.
Base files: `crates/screenpipe-engine/src/connections_api.rs` (script consts, handler, route, tests),
skill doc.

### connections-owned-browser-tabs — Multi-tab owned browser

**Kind:** changed. **Confidence:** observed (commands/args, UI), inferred (Rust internals). **Size:** M.
**Evidence:** bindings (`owned_browser_history(tabId,direction)`, `owned_browser_tab_{navigate,set_bounds,hide,close,clear_browsing_data}`,
`owned_browser_navigate(url,owner,reveal)`); chat sidebar `3696-4fbfea4ced3de899.js` @268089..281774 (primary
tab vs extra tabs, per-tab `{id,url,title,loading,owner,navigationId}`, tabs closed when chat changes,
back/forward, reload, cookie-access retry); events `owned-browser:navigate|ready|state|session-access-request` (unchanged names).
Behaviour: the app keeps one primary webview (`owned-default`, managed by the existing commands) plus
additional tab webviews addressed by `tabId`. `owned_browser_navigate` gains `owner` (chat/session id) and
`reveal` (show the sidebar) — base hard-codes neither. History navigates back/forward (`direction`:
`"back"`/`"forward"`, inferred). State events include the tab id. Tabs never outlive the conversation; the
conversation file persists `browserState {url, updatedAt, width, collapsed}`.
Base files: `apps/screenpipe-app-tauri/src-tauri/src/owned_browser.rs` (tab map), `owned_browser_cookies.rs`
(per-tab clear), chat sidebar components, `lib/utils/tauri.ts`.

### connections-google-calendar-multi — All calendars, account snapshots

**Kind:** changed. **Confidence:** observed (keys, URLs), inferred (exact merge). **Size:** M.
**Evidence:** `0x525493b..0x5254c21` (`users/me/calendarList`, `minAccessRole`, `showHidden`, `maxResults` 250,
`nextPageToken`/`pageToken`, `deleted`, `hidden`, `accessRole`, `timeMin`, `timeMax`, `singleEvents`, `orderBy`,
`conferenceDataVersion`); query keys `require_complete`, `account_snapshots`, struct `GoogleAccountSnapshot{…, events}`
`0x517e8f8..0x517e950`; `iCalUID`, `excludeFromMeetingDetection`, `self`/`responseStatus`, `transparency`
`0x6a31fe0`, `0x6a33c00`; publisher `0x498cde8` (`?hours_back=1&hours_ahead=2&account_snapshots=true`,
`calendar_snapshots_retained`, `account_snapshot_retained_other_accounts_continue` `0x5287878`).
Behaviour: events are gathered from every visible, non-deleted calendar in `calendarList` (paginated,
`maxResults=250`; `owner` literal adjacent to `minAccessRole`), deduped by `iCalUID`, sorted by start; declined
events (self attendee `responseStatus=declined`) and `transparency=transparent` items are flagged/excluded from
meeting detection. `account_snapshots=true` returns one snapshot per connected account
(`[{account/instance, email, events, complete, error?}]`, inferred) instead of failing on ambiguity; with
`require_complete=true` a partial failure returns an incomplete-snapshot error. The app
publisher keeps the last good snapshot per account when one account fails. `calendarName` becomes the real
calendar summary (base hard-codes `"primary"`).
Base files: `connections_api.rs::gcal_events_inner`, app `google_calendar.rs` publisher.

### connections-connection-triggers — Pipes triggered by connection events

**Kind:** new. **Confidence:** observed (catalog, file/field names), inferred (polling internals). **Size:** L.
Overlaps the pipes/scheduler dimension; specified here because sources are connections.
**Evidence:** `screenpipe-core/src/pipes/connection_triggers.rs` + `connection_triggers/voice.rs` (`0xa0440f1`, `0xa116622`);
`connection-triggers.json` `0xa11617e`; kinds `0xa1162ba` (`notion github linear item task issue_created issue event_started page note phrase`);
subscription fields `0xa1166a8` (`kind instance path filter items`); event struct `ConnectionTriggerEvent{pipe, app, path, count, timestamp, delivery_id, subscription_key, context}`
`0xa11a1c8`; field extraction strings `0xa116a82..0xa116bbe`; logs (watch initialisation, a fire with its item count,
giving up and advancing the cursor, abandoning a delivery that timed out); UI catalog `5977…js` @487793..490206 and
pickers @504738..523383; pipe config `trigger.sources` read at @~654000.

Catalog (trigger id → app/kind/filter):
`voice_phrase` (local audio, `filter.phrases` newline list, match mode: the user's own speech vs any audio), `meeting_started`,
`meeting_ended` (built-in events), `email_received` (`imap`/`message`, `filter.mailbox`), `email_sent` (`imap`/`sent_message`),
`outlook_received`/`outlook_sent` (`outlook-email`, `message`/`sent_message`), `google_calendar_started`
(`google-calendar`/`event_started`), `slack` (`filter.channel`, `channel_name`), `notion` (page created/edited,
`filter.database_id`/`database_name`), `github_issue`/`github_pull_request` (`github`, `issue`/`pull_request`,
`filter.repository`), `linear_issue_created`/`linear_issue_assigned`/`linear_issue_status_changed` (`linear`,
`filter.team_id`/`team_name`), `todoist_task` (`todoist`/`task`), `obsidian` (new note, `path` = watched folder),
`pipe` (after another pipe completes, event `pipe_completed:<name>`).
Behaviour: a pipe's frontmatter `trigger.sources: [{app, kind, instance?, path?, filter?}]` subscribes it.
A per-subscription poller (credentials via the local connection proxy/endpoints, never exposed) keeps a cursor
(last seen id/timestamp) in `<data>/connection-triggers.json`; first run initialises the cursor without firing;
new items fire a `connection_trigger` workflow event `{pipe, app, path, count, timestamp, delivery_id, subscription_key, context}`
(context = extracted fields: title/html_url/created_at for GitHub, identifier/url/state changes/assignee for
Linear, subject/from/to/bodyPreview for mail, summary/start/location/hangoutLink for calendar, title/last_edited_time
for Notion, text/ts for Slack, path for Obsidian, phrase/device/is_input for voice). Delivery is retried; after
the max attempts the cursor advances. UI pickers: GitHub repos via `/connections/github/proxy/user/repos?per_page=100&sort=updated&direction=desc`,
Slack channels via `/connections/slack/conversations`, Linear teams via `/connections/linear/proxy/graphql`, IMAP
mailboxes via `/connections/imap/mailboxes`, Obsidian folder picker.
Base files: new `crates/screenpipe-core/src/pipes/connection_triggers.rs`, `pipes/mod.rs` (`TriggerConfig.sources`),
scheduler event bus, pipe editor UI. Defaults (inferred): poll every 60 s per source, 5 delivery attempts.

### connections-vault-secrets-db — Vault covers the secrets DB

**Kind:** changed. **Confidence:** observed. **Size:** S.
**Evidence:** file list `0xa23c560` (`vault.meta .vault_locked db.sqlite db.sqlite-wal db.sqlite-shm secrets.sqlite secrets.sqlite-journal`),
`0x522ae21` (the refusal error for hybrid storage).
Behaviour: lock/unlock/setup encrypt and decrypt `secrets.sqlite` and `secrets.sqlite-journal` alongside the main
DB files; setup refuses when the storage format is hybrid. Close the secret-store pool before locking.
Base files: `crates/screenpipe-vault/src/manager.rs` (file list at lines ~172–190 and ~266–279), `crates/screenpipe-engine/src/routes/vault.rs`.

### connections-whatsapp-signout — WhatsApp remote sign-out recovery

**Kind:** changed. **Confidence:** observed. **Size:** S.
**Evidence:** `0x49aee08..0x49aeee4`.
Behaviour: on a gateway logged-out disconnect, wipe `whatsapp-session` and report a pair-again state (error if
the old session cannot be reset); on transient disconnects report a reconnecting status. Listing description points to the
Connections page. Base files: `crates/screenpipe-connect/src/whatsapp/mod.rs`, `gateway.mjs`.

### connections-supports-oauth-instances — Integration metadata flag

**Kind:** changed. **Confidence:** observed (key), inferred (which ids). **Size:** S.
**Evidence:** serialized keys `0x52b7810` (`supports_oauth_instances`, `category`, `fields`, `FieldDef`, `secret`, `help_url`);
UI passes `supportsOAuthInstances` to the OAuth panel (`5977…js` @360151, @362497, @364722).
Behaviour: `IntegrationDef`/`ConnectionInfo` gains `supports_oauth_instances: bool`; true lets the OAuth panel
add further accounts or workspaces. Set true for google-calendar, google-docs, microsoft365, outlook-email, teams,
slack, github, notion (inferred). Base files: `connections/mod.rs`, each def, `connections-section.tsx`.

### connections-ui-catalog — Settings > Connections catalogue changes

**Kind:** changed. **Confidence:** observed. **Size:** M.
**Evidence:** tile list `5977…js` @349519 (`eH`), switch @360151..364722, icons @248379, try-in-chat prompts @264161,
Obsidian vault detection @320024, multi-vault @321783..323237.
Changes vs base `connections-section.tsx` (which already has detected flags, Try in Chat, Krisp/Plaud MCP, Obsidian vaults):
* New tiles: `grokbot` (Grok bot) and `grok` (Grok CLI) (AI-tool dimension), `excalidraw`, `google-drive`, `google-sheets`
  (and Composio-backed `gmail`), `remote-agent` (always-on agent), `pi-extensions` (agent tools). Renames: `custom-mcp`
  is relabelled as the advanced-connections entry, `ics-calendar` as a generic other-calendars entry, `claude` gets a
  plain Claude label, and `browser-url` is relabelled around browser activity.
  Removed from the grid: `apple-intelligence`, `input-monitoring`, `openclaw`, `hermes`, `obsidian-memories`,
  `owned-default` (still reachable via registry list where relevant).
* OAuth tiles with fields add an Advanced token-based connect option; MCP-provider tiles show the native form under
  Advanced; Zoom shows the legacy native OAuth under Advanced.
* `scopeVariant` deep link support; variant radio for Slack.
* Obsidian: vault auto-detection reads Obsidian's `obsidian.json` (`~/Library/Application Support/obsidian/`,
  `%USERPROFILE%\AppData\Roaming\Obsidian\`, `~/.config/obsidian/`), lists `vaults{id:{path}}`; adding a second vault
  stores it as instance `<slugified folder name>[-N]` via `PUT /connections/obsidian/instances/<name>` after
  `POST /connections/obsidian/test`; deleting the default promotes the first named vault to default.
* Bee: one-click pairing button + developer token under Advanced. IMAP form. Outlook tile.
Freenpipe: drop the Composio/screenpipe-login gating; replace with BYO prompts.
Base files: `apps/screenpipe-app-tauri/components/settings/connections-section.tsx` and new panels.

---

## 3. Items checked and found unchanged (no delta)

* Credential proxy core (`/:id/proxy/*path`, `{field|default}` URL templating, DELETE blocked, `?instance=` split,
  `extra_root_pem` for Bee's private CA — same PEM at `0x9f8c560`/`0x9f95fa1`), `/:id/config`, path routes for Google Docs.
* OAuth token storage semantics (`oauth:` keys, plaintext migration, instance fallback, `screenpipe_refresh_gen`,
  invalid_grant retry, `merge_refresh_response`), refresh scheduler constants (cooldown/consecutive failures).
* Keychain identifiers (`com.screenpipe.app` / `store-encryption-key`), `store.bin` encryption (`SPSTORE1`, `.encrypt-store`).
* ICS calendar (`IcsCalendarSettings`, `ics_calendar_*` commands), native calendar (EventKit/WinRT), WhatsApp gateway routes,
  browser extension pairing/bridge routes, `user-browser`/`owned-default` descriptions, cookie-access commands.
* Wearables: Limitless (`X-API-Key`, `https://api.limitless.ai/v1`, test `/v1/lifelogs?limit=1`), Pocket, Plaud and Krisp
  (MCP OAuth), Voice Memos tile (macOS only) — unchanged apart from Bee pairing. No lifelog import or wearable DB table exists;
  `friend_wearable_requests` is dropped in both versions.
* Remote sync over SSH/Tailscale (`remote_sync.rs`), mDNS, cross-device connection sync manifests
  (`/sync/connections/push|pull`, `oauth_rev`, tombstones) — present in base; decloud of the cloud sync transport belongs to the sync dimension.

## 4. Open questions

* Exact JSON shapes of `/imap/messages`, `/slack/search|history`, `/bee/pair/*` responses and of `GoogleAccountSnapshot` (only key names observed).
* Whether Slack/HubSpot still use the `https://screenpi.pe/api/oauth/callback` relay (string dedup hides adjacency).
* Bee pairing crypto (RSA-OAEP + AES-GCM assumed) and the `#fragment` format of `pairing_url`.
* Lease TTL/key naming for OAuth refreshes (only ChatGPT usage is explicit).
* Which providers accept `http://localhost` redirect URIs for BYO apps (Slack, Zoom, Intuit production, Calendly, Zendesk may require HTTPS).
* Connection-trigger poll intervals and retry counts; exact frontmatter nesting of `trigger.sources`.

## Verification

Adversarial pass (2026-10-14). Static evidence only. Every cited `app_strings.tsv` offset below was re-read with
`tools/verify_offs.py` and range dumps, the frontend offsets with `tools/slice.py`/`ctx.py`, and the base was grepped for
each identifier. New verifier helpers: `tools/vconn_cmd_diff.py` (diffs Tauri commands between the target bindings chunk
and base `lib/utils/tauri.ts`) and `tools/vconn_paths_diff.py` (lists `/connections`, `/mcp-servers`, `/memories` and
`/v1/connect` path tokens from the target that never appear in the base sources).
REA: trace attempts on the JS application graph `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`
for seeds `screenpipe:connect:` (`ev_f918359769cc990421b75acce2ae9d0b86e4de492cc1d8caddd6a9e6d6797733`) and `oauth_connect`
(`ev_def6ab1d4e5ea6a4a45261f40441f387d0614778084f9b69a7911ab660b6929b`) both returned no-match on the partial graph,
so the frontend facts are confirmed by direct file reads.

### Verdicts

| Item | Verdict | Notes |
|---|---|---|
| connections-secret-db-split | confirmed | DDL, the metadata gate (`legacy_main_db_migrated_v1`), the legacy copy, the pool and quarantine messages, and the `secrets.sqlite(-journal)` file lists are all present at the cited offsets. The base has none of `secrets.sqlite`, `secret_store_metadata` or the migration. **Correction:** the encrypted-row count helper is already in the base as `SecretStore::encrypted_secret_count` (`crates/screenpipe-secrets/src/store.rs:240`, same SQL), used by `disable_keychain_encryption` (`commands.rs:2097`). Only the `encrypted_count` label is new, so do not re-implement it. The schema overlaps db-026 (specs/db.md); this item owns the implementation. |
| connections-credential-refresh-leases | confirmed | Acquire/release SQL is at `0x49af5b0..0x49af7bf` (copy at `0x5130950`). The base has no lease table or calls. The only caller seen is the ChatGPT token refresh (app `0x49893f0`, core `0xa044210`; base `chatgpt_oauth.rs` and `pipes/mod.rs:790` refresh without a lease). Wrapping the integration OAuth refreshes in a lease is a recommendation, not observed. Overlaps db-026. |
| connections-oauth-byo-broker | confirmed | This is a decloud task, but the broker dependency is **not new in the target**. The base already exchanges and refreshes through `https://screenpi.pe/api/oauth/exchange` (`crates/screenpipe-connect/src/oauth.rs:971` `EXCHANGE_PROXY_URL`, used at :819 and :1720) and relays slack, zoom, calendly and quickbooks via `redirect_uri_override`. The target widens the surface: hubspot, zendesk (`{subdomain}`, with `subdomain` sent alongside the exchange URL `0x5186da1`) and outlook-email now use OAuth. The base Pro gate is at app `oauth.rs:81-90`; the target has no Pro-subscription requirement string (0 hits). All 17 cited config offsets were re-read and match. |
| connections-oauth-connect-v2 | confirmed | `0x5112140..0x51129ef` holds the `{subdomain}` validation, `SCREENPIPE_OAUTH_TIMEOUT_SECS`, the port-3030 refusal, the provider-error and denied messages, the expired-sign-in outcome, and the unknown-scope-variant error. The callback outcomes are at `0x517d3d8..0x517d74c`. Bindings: `oauth_connect{integrationId,instance,variant}` and `activate_app_after_oauth`. The base has a 120 s timeout, no variant, no `error_description` and no activate command. |
| connections-oauth-default-slot-promotion | confirmed | `0x512fe18..0x5130223` has the promote, leave-intact and "(multi-account)" messages. The base `sweep_shadowed_default_slots` (`oauth.rs:725`) only deletes; it has no "promoted" string. The recoverable-fallback lookups (`oauth.rs:223`, `:237`) are already in the base. |
| connections-secret-redaction | partially_in_base | **Already in base:** `render_context` prints the `proxy:` and `config: … (non-secret settings)` lines for proxy integrations (`connections/mod.rs:707-716`, `:731-740`), and `GET /:id/config` already filters out `FieldDef.secret` fields (`connections_api.rs:1938-1956`). **New in target:** `GET /:id` and `GET /:id/instances` return only non-secret fields (the base returns raw credentials plus `X-Deprecation-Warning`, `connections_api.rs:382-435`, `:492-516`). The non-proxy branch no longer prints raw credential values (the base dumps every string credential, `mod.rs:717-723`, including the Telegram bot token and webhook URLs). Also new: the server-side-credentials note on the config line, the `webhook: POST …/proxy` line, the Telegram and IMAP pointer lines, OAuth instances in `/instances`, and the skill rewrite (`0x511a9aa`). |
| connections-webhook-proxy | confirmed | The route `/:id/proxy` (`0x5182cb0`) is absent from the base route table (`connections_api.rs:2634-2697`). The error codes at `0x517d028..0x517d238` and the five descriptions all match. |
| connections-telegram-send | confirmed | Route `0x5182bb8`, `TelegramSendRequest` (`0x517e810`), and error and `message_id` strings at `0x517cf20..0x517d000` and `0x5186330`. The base has no telegram route. |
| connections-slack-user-token | confirmed | Variants `send`/`read_write` with `user_scope` are at `0x9f96e2e`/`0x9f96fa0`. Engine strings (`search.messages`, `conversations.list`/`history`, the no-read-access 403 text) and the routes at `0x5182b10..0x5182b90` match. The base Slack client is incoming-webhook only (`connections/slack.rs`, scope `incoming-webhook`). |
| connections-imap | confirmed | `connections/imap.rs` panic paths, the def at `0x9f9c69e`, three routes, and the crates `async-imap 0.11.3`, `imap-proto 0.16.7` and `mail-parser 0.11.5`. All of these are absent from the base `Cargo.lock` and sources. |
| connections-outlook-email | confirmed | `outlook_email.rs` panic path, the def/description at `0x9f85388`, scopes plus `select_account` at `0x9f8588d`, Graph base at `0x9f84ee8`, and the trigger catalog at 5977 @488554. No base file. |
| connections-hermes | confirmed | The base has only a UI card (`components/settings/hermes-card.tsx`: MCP snippet plus skills install). There is no `hermes.rs` and no integration def. The target def is at `0x9f85b00` (`API Server URL`, `http://127.0.0.1:8642`) and the test at `0x9f845b0`. |
| connections-hubspot-oauth | confirmed | Base `hubspot.rs` is token-only and tests `crm/v3/objects/contacts`. The target has an OAuth config (`0x9fa4308`), an optional-fallback `api_token` label (`0x9fa4210`) and an `account-info/v3/details` + `portalId` test (`0x9fa3650`). |
| connections-zendesk-oauth | confirmed | Base `zendesk.rs` uses token-only Basic auth with `email`/`api_token`. The target adds `https://{subdomain}.zendesk.com/oauth/authorizations/new`, `zdg-screenpipe`, `read write` (`0x9fb9448`), a `/token` literal and an "(OAuth)" test suffix (`0x9fb8140..0x9fb81a4`). |
| connections-oauth-scope-tweaks | confirmed | GitHub `repo read:org` (`0x9f8ea80`; base `github_issues.rs:17` is `repo`). microsoft365 and teams use `select_account` (base `consent select_account`, `microsoft365.rs:42`, `teams.rs:31`). Google keeps the consent prompt (`0x9f9c2f5`, `0x9f7e0c7`). outlook-email is new, so its prompt belongs to the outlook item, not this tweak. |
| connections-gmail-sheets-composio | confirmed | The base `gmail.rs`/`google_sheets.rs` client ids (`…ft2rcpk2…`, `…6k41um04…`) and the gmail/sheets API hosts occur 0 times in the target. There are no `gmail.rs` or `google_sheets.rs` panic paths, and the route table has no `/gmail/*`. The Composio client (`/api/composio` on screenpipe.com, toolkits gmail/zoom/googledrive/googledocs/googlesheets) is at 5977 @391898 and the status fetch at @345915. The base has no `composio` anywhere. |
| connections-mcp-oauth-tiles | confirmed | The provider map at `0x6a33590` (also in the pi-extension TS at `0x52a5dae`) and the relay at `0x6a8dd01` are verified. The base has only the Krisp and Plaud MCP panels (`connections-section.tsx:2925-2926`). One bullet of agents_pipes-032 covers the same provider list and relay decloud. Keep the UI tiles and `/connections` enrichment here and cross-link. |
| connections-pi-connect-tools | duplicate | Same feature as **agents_pipes-043** (connect broker and connection-gate tools) and the `connection-gate` row of agents_pipes-014. The evidence is valid (`screenpipe_list_connections`, `screenpipe_connect_app`, `connected_only` and `screenpipe:connect:` in the TS at `0x52a59e8..`; `connection_action` ×14 and `extension_ui_request` in 3696). Merge the richer tool and UI semantics from this spec into agents_pipes-043. Minor error: the base bundles six extensions (`context-pruning`, `mcp-bridge`, `orphan-guard`, `screenpipe-permissions`, `sub-agent`, `web-search`), not two. |
| connections-bee-pairing | confirmed | Routes at `0x5182c60`/`0x5182c88`; pairing service, app id and error/token strings at `0x5185bec..0x5185ce4` and `0x9f96741..0x9f96a2c`. Base `bee.rs` is developer-token only. **Correction:** `rsa 0.9.10` is already in the base `Cargo.lock` as a transitive dependency, so `crates.txt` is no evidence of the pairing crypto. RSA+AES stays inferred. |
| connections-obsidian-memories | confirmed | The def at `0x9f7ed88` and the sync renderer at `0xa0ba74f` are verified, and no base file contains `obsidian-memories`. **Note:** `POST /memories/sync-external` (`server.rs:678`) and its per-destination result shape (`destination_id`, `SyncOutcome` Wrote/Unchanged/Skipped in `external_memory_sync.rs`) are already in the base for claude-code and codex. Only the new destination and its renderer are the delta. The `.screenpipe-write-probe` pattern already exists in the base `claude_code.rs:48` and `codex.rs:45`. |
| connections-agent-memory-guidance | confirmed | The target descriptions (`0x9f8e620`, `0x9f90f10`) say raw memory is never embedded. The base descriptions say the sidecar holds the full digest. The disconnect cleanup and restore message is at `0x5185988`, with no base equivalent. |
| connections-browser-snapshot-v2-act | confirmed | Target `MAX_DEPTH = 12` (`0x517f1d7`) against base 8 (`connections_api.rs:2412`). `data-sp-ref` stamping, the `/browsers/:id/act` route (`0x5182760`), `BrowserActBody` and the action list are all verified. The base has none of `act` or `data-sp-ref`. |
| connections-owned-browser-tabs | confirmed | `vconn_cmd_diff.py`: `+owned_browser_history`, `+owned_browser_tab_{navigate,set_bounds,hide,close,clear_browsing_data}`, `~owned_browser_navigate` (`url` → `url,owner,reveal`). The chat chunk @268089 routes primary vs extra tabs. Side note: the target drops the base test hook `e2e_owned_browser_visible`. |
| connections-google-calendar-multi | confirmed | `users/me/calendarList`, pagination, `iCalUID`, `GoogleAccountSnapshot`, `require_complete`/`account_snapshots`, the publisher URL and the retained-snapshot log are all verified. The base hard-codes `"calendarName": "primary"` (`connections_api.rs:1264`). **Caveat:** the literal right after `minAccessRole` is `owner` (`0x5254a7e`), so the list query may be limited to calendars the user owns rather than "every visible calendar". Leave this open. |
| connections-connection-triggers | duplicate | Same feature as **agents_pipes-008** (connection triggers watcher), which also covers polling endpoints and delivery. Evidence verified (`connection_triggers.rs` panic paths, `ConnectionTriggerEvent` at `0xa11a1c8`, kinds at `0xa1162ba`). There is no base `connection_trigger`. Merge the UI trigger catalog (ids and filters, 5977 @487793..490206) and pickers into agents_pipes-008. **Conflict to resolve:** the cursor file is `<data>/connection-triggers.json` here and `<pipes_dir>/.connection-triggers.json` in agents_pipes-008. The string `slack.connection-triggers.json` (`0xa11617e`) is ambiguous because of literal packing. |
| connections-vault-secrets-db | confirmed | The file list at `0xa23c560` adds `secrets.sqlite(-journal)`; the base vault manager encrypts only `db.sqlite*` (`manager.rs:172-190`, `:266-279`). The hybrid-storage refusal (`0x522ae21`) is new. db-026 has a one-line overlap. |
| connections-whatsapp-signout | confirmed | The sign-out reset messages and the "Reconnecting" status are at `0x49aee08..0x49aeee4`. On `logged_out` the base only stops auto-restart and sets `Disconnected` (`whatsapp/mod.rs:219-223`, `:419-422`). It does not wipe the session and has no user-facing message. |
| connections-supports-oauth-instances | confirmed | Key `supports_oauth_instances` (`0x52b7810`) and `supportsOAuthInstances` ×4 in 5977. 0 hits in the base. |
| connections-ui-catalog | partially_in_base | **Already in base:** Obsidian vault auto-detection from `obsidian.json` on all three OS paths, multi-vault instances via `PUT /connections/obsidian/instances/<slug>` after `POST /connections/obsidian/test`, and promote-on-delete (`connections-section.tsx:2516-2652`, onboarding `connect-apps.tsx:215-258`). Icons `plaud.png`, `krisp.svg`, `hermes.png`, `bee.png`, `voice-memos.svg` and `google-sheets.svg` are already in base `public/images`. **New:** the `excalidraw` and `google-drive` tiles and icons; the `grokbot`/`grok`/`remote-agent`/`pi-extensions` tiles; the `custom-mcp`, `ics-calendar` and `browser-url` relabels; Advanced token/API-key fallbacks on OAuth and MCP tiles; the `scopeVariant` deep link; and the Bee, IMAP and Outlook panels. |

### Missed items

* **connections-chat-context-per-turn** (changed, S). The target chat runtime (pi sidecar) wraps the current
  connected-integrations context in a `<connections_context>` … `</connections_context>` block and refreshes it on every
  turn. The block opens with a header line identifying it as the current connected-integrations context, refreshed per turn.
  Evidence is in four copies: app `0x5176d20..0x5176d86`, plus `0x4992db8..0x4992e1e`, `0x49f4d58..0x49f4dbe` and
  `0x5226dc0..0x5226e26`, each next to the pi provider/extension strings. The base builds the connections context once in
  the frontend (`standalone-chat.tsx:4702` `buildConnectionsContext`) and appends it to the system prompt at session
  start. Pipes get it once through `PipeManager::set_connections_context` (`pipes/mod.rs:1363`). Mid-chat connects
  (`screenpipe_connect_app`) and disconnects are therefore invisible to the model until a new session starts.
  Freenpipe: before each prompt, render the redacted `render_context` output (see connections-secret-redaction) into a
  tagged block, and stop baking it into the system prompt. No other spec covers this.

Other new paths found by `vconn_paths_diff.py` are already owned elsewhere: `/mcp-servers/registry` → agents_pipes-032;
`/v1/connect-request` and `/v1/connect-response` → agents_pipes-043; `grokbot_connection` → frontend.md and
agents_pipes.md. Wearables (Limitless, `friend_wearable_requests` drop) show no new delta, which agrees with §3.

## Verification (independent pass 2, 2026-10-15)

Static evidence only; the exe was never run. Method:
* Every cited `app_strings.tsv` offset was re-read with `tools/verify_offs.py` / `tools/ver_range.py`, and frontend
  offsets with `tools/slice.py`.
* The base was grepped for every identifier, route, table and function name.
* New helper `tools/vconn2_srcdiff.py` diffs the connect/secrets/vault panic-path source files against the base tree
  (`tools/out/vconn2_srcdiff.txt`). It found new `hermes.rs`, `imap.rs`, `obsidian_memories.rs`, `outlook_email.rs` and
  `screenpipe-secrets/telemetry.rs`. `gmail.rs` and `google_sheets.rs` are absent from the target.
* `tools/vv_newmsg.py` swept the engine connections region (TSV lines 177260..178000,
  `out/vconn2_newmsg_engine.txt`) and the connect-crate static region (lines 687040..689525,
  `out/vconn2_newmsg_connect.txt`) for strings with no base match.
* `tools/vconn_paths_diff.py` was re-run (`out/vconn2_paths.txt`).
* REA: a `connections_context` string trace on `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`
  returned no-match on the partial graph (`ev_45876760a889ee69d2ae8fe072929b7f84efc7aa469f0c097353e11a96d4c30b`), so
  frontend facts are cited by file offset.

This pass also checked the other dimension specs for duplicates. Several items here describe the same delta as an item
that another spec already owns and that its verifier has confirmed. Those items are marked `duplicate` and name the
canonical item. Their extra detail should be merged into that item, not dropped.

### Verdicts (pass 2)

| Item | Verdict | Notes |
|---|---|---|
| connections-secret-db-split | duplicate | **Canonical: db-026** (confirmed in db.md). It already records the same DDL, the `legacy_main_db_migrated_v1` one-shot copy with newer-`updated_at` wins, and the vault coverage.<br>**Evidence holds:** DDL `0x49af280`, gate `0x49af8ed`, legacy source `0x49afa1c`, pool/quarantine `0x49aff48`/`0x49b001b`/`0x49b0109`, file lists `0x49873c0`/`0x522af47`/`0xa23c560`. The base opens the store on `db.sqlite` (app `oauth.rs:43-55`) and has no `secrets.sqlite` or `secret_store_metadata`.<br>**Merge into db-026:** this item's construction-site list, the pool/quarantine behaviour and the `BEGIN IMMEDIATE` note.<br>**Correction:** the encrypted-row count is already in the base as `SecretStore::encrypted_secret_count` (`crates/screenpipe-secrets/src/store.rs:240`, identical SQL). Only the `encrypted_count` label at `0x51309e8` is new. |
| connections-credential-refresh-leases | duplicate | **Canonical: db-026**, which already specifies the `credential_refresh_leases` table and its acquire/release semantics.<br>**Evidence holds:** SQL at `0x49af5b0`. The base has no lease table.<br>**Merge into db-026:** the only caller observed, the ChatGPT token refresh (`0x49893f0`, `0xa0441d0`).<br>**Not observed:** wrapping the integration OAuth refresh in a lease is a recommendation only. |
| connections-oauth-byo-broker | duplicate | **Canonical: cloud-018** (OAuth broker replaced by BYO client credentials: `token_url`/PKCE, HTTPS-only providers; the Pro gate is cloud-015).<br>**Not a target delta:** the base already exchanges and refreshes through `https://screenpi.pe/api/oauth/exchange` (`crates/screenpipe-connect/src/oauth.rs:971`). It also relays slack, zoom, calendly and quickbooks via `redirect_uri_override`. The target only widens the surface to hubspot, zendesk and outlook-email.<br>**Evidence holds:** exchange URL `0x9f78c8e`/`0x512fa9a`/`0x5186da1` (preceded by `subdomain`); relays `0x9f96e2e` (zoom), `0x9fb13d8`, `0x9fb95a0`. The Pro-gate string has 0 target hits.<br>**Merge into cloud-018:** the per-provider config table and the public token endpoints.<br>**Conflict to resolve:** the key family is `oauth-client:<id>` here and `oauth_client:<id>` in cloud-018. |
| connections-oauth-connect-v2 | confirmed | **Target, app `oauth.rs` strings (`0x5112140..0x51129ef`):** `{subdomain}` validation, `SCREENPIPE_OAUTH_TIMEOUT_SECS`, port-3030 refusal, provider-error and denied messages, the expired-sign-in outcome, and the unknown-scope-variant error.<br>**Target, other:** callback outcomes at `0x517d3d8..0x517d74c`; `error_description` at `0x517e7b8`; 8001 bindings `oauth_connect{integrationId,instance,variant}` and `activate_app_after_oauth`.<br>**Base:** app `oauth.rs:64-148` has no variant and a 120 s timeout. None of these identifiers is in the base. |
| connections-oauth-default-slot-promotion | confirmed | **Target:** `0x512fe18..0x5130223` holds the promote, leave-intact and `distinct_recoverable` messages. The reconcile-after-instanced-save string is at `0x5112890`.<br>**Base:** `sweep_shadowed_default_slots` (`oauth.rs:725`) only deletes. |
| connections-secret-redaction | partially_in_base | **Already in base:**<ul><li>`render_context` prints `proxy:` and `config: … (non-secret settings)` lines (`connections/mod.rs:707-716`, `:731-740`).</li><li>OAuth integrations without a proxy already hide the token (`mod.rs:743`).</li><li>`GET /:id/config` filters out secret fields.</li></ul>**New in target:**<ul><li>`GET /:id` and `GET /:id/instances` drop secret fields. The base returns raw credentials (`connections_api.rs:382-435`, `:492-516`).</li><li>The non-proxy branch stops dumping raw values (base `mod.rs:717-723`) and prints webhook, Telegram and IMAP pointer lines instead (`0x49ae0f4..0x49ae17a`, `0x9f8d850`, `0x9f8ca59`).</li><li>Skill rule at `0x511a9aa`; frontend filter at 5977 @15119.</li></ul> |
| connections-webhook-proxy | confirmed | Bare `/:id/proxy` (`0x5182cb0`) is absent from the base router (`connections_api.rs:2634-2697`). Error codes are at `0x517d028..0x517d238`. api-031 lists the route but defers its semantics to this item. |
| connections-telegram-send | confirmed | **Target:** route `0x5182bb8`, `TelegramSendRequest` `0x517e810`, errors and `message_id` at `0x517ceb0..0x517d000`. The base has no route.<br>**Conflict:** api-031 gives the body as `{text, chat_id?}`, but only `text` is observed, so a `chat_id` override is unverified. |
| connections-slack-user-token | confirmed | **Target:**<ul><li>Variants and `user_scope` at `0x9f96e2e`/`0x9f96fa0`; description at `0x9f97159`.</li><li>Engine strings `search.messages`, `conversations.list`/`history` and the read-access 403 (`0x517ccf8`, `0x517ceb0`, `0x517e370`, `0x517e658`).</li><li>Routes at `0x5182b10..0x5182b90`.</li></ul>**Base:** incoming-webhook only, via relay (`slack.rs:16`). |
| connections-imap | confirmed | **Target:** `imap.rs` (`0x5185d11`), def `0x9f9c69e..0x9f9ca39`, routes `0x5182a90..0x5182ae8`.<br>**Base:** `async-imap`, `imap-proto` and `mail-parser` are absent from `Cargo.lock`. |
| connections-outlook-email | confirmed | **Target:** `outlook_email.rs` (`0x9f84f28`), inbox test `0x9f84fb8`, def at `0x9f85388`/`0x9f8588d`.<br>**Base:** only a stray category constant `"outlook-email": "Communication"` (`apps/screenpipe-app-tauri/lib/constants/connections.ts:68`), with no integration behind it. |
| connections-hermes | confirmed | **Target:** `hermes.rs` (`0x9f845c5`) and def `0x9f85b00..0x9f85f78`.<br>**Base:** only `hermes-card.tsx` (MCP snippet) and a UI tile (`connections-section.tsx:3371`). No other spec owns this. |
| connections-hubspot-oauth | confirmed | **Target:** OAuth config `0x9fa4308`, optional-fallback label `0x9fa4210`, `account-info/v3/details` test `0x9fa3650`.<br>**Base:** `hubspot.rs` is token-only and tests `crm/v3/objects/contacts`. |
| connections-zendesk-oauth | confirmed | **Target:** OAuth config `0x9fb9448`; ` (OAuth)` test suffix `0x9fb819a`.<br>**Correction:** the `/token` literal (`0x9fb8178`) sits beside the `users/me.json` test URL, and the base test already authenticates as `{email}/token` (`zendesk.rs:76`). So it says nothing about the proxy. The base proxy sends plain `email:api_token` (`ProxyAuth::BasicAuth`). Fixing that is a Freenpipe bug fix, not an observed delta. |
| connections-oauth-scope-tweaks | confirmed | **GitHub:** `repo read:org` (`0x9f8ea80`); base `github_issues.rs:17` is `repo`.<br>**microsoft365 and teams:** `select_account` (`0x9f7ebf8`, `0x9f91a50`); base is `consent select_account` (`microsoft365.rs:42`, `teams.rs:31`).<br>The outlook-email prompt belongs to the outlook item. |
| connections-gmail-sheets-composio | duplicate | **Canonical: cloud-038** (confirmed): Composio-brokered Gmail/Zoom/Drive/Docs/Sheets. Freenpipe keeps the base integrations on BYO OAuth.<br>**Evidence holds:**<ul><li>No `gmail.rs`/`google_sheets.rs` panic paths (`vconn2_srcdiff`).</li><li>No `/gmail/*` in route table `0x51826a8..0x5182e00`.</li><li>Composio client at 5977 @391902 and status call @345919.</li><li>The base has 0 hits for `composio` or `google-drive`.</li></ul>**Merge into cloud-038:** the target dropped the native Gmail routes, and this item proposes a native `google-drive` integration. |
| connections-mcp-oauth-tiles | confirmed | **Target:** provider map `0x6a33590`/`0x52a5dae`, relay `0x6a8dd01`, engine log `0x5182620`, Excalidraw API-key MCP at 5977 @363600.<br>**Base:** only Krisp/Plaud MCP panels (`connections-section.tsx:2925-2926`), plus loopback MCP OAuth (`mcp_servers_api.rs:397`).<br>**Overlap:** agents_pipes-032 already covers the provider list and the relay decloud. This item keeps the Settings tiles, the connected-state rule and the `/connections` `mcp`/`mcp_server_id` enrichment. |
| connections-pi-connect-tools | duplicate | **Canonical: agents_pipes-043**, plus the connection-gate row of agents_pipes-014 (both confirmed).<br>**Evidence holds:** tool TS `0x52a851b`; `screenpipe:connect:` and `connection_action` in 3696.<br>**Merge note:** the target has two transports. The app pi extension uses `ctx.ui.confirm` and the `connection_action` card (`0x52a59e8..`). The core/ACP copy posts `/v1/connect-request` (`0xa0d302b`). Keep both. |
| connections-bee-pairing | confirmed | **Target:**<ul><li>Routes `0x5182c60`/`0x5182c88` and `BeePairPollBody` `0x517e740`.</li><li>Pairing service URL `0x5185bec` and app id `0x5185c7e`.</li><li>`bee.computer/connect#` `0x9f96758`; token errors `0x9f968ce..0x9f96a09`.</li></ul>**Base:** `bee.rs` has no pairing.<br>**Correction:** `rsa 0.9.10` is already in the base `Cargo.lock`, so the crypto stays inferred. |
| connections-obsidian-memories | duplicate | **Canonical: meetings_memory-035** (confirmed): same `obsidian-memories` id and fields, the `.obsidian` + `.screenpipe-write-probe` test, and `screenpipe-memories.md` output.<br>**Evidence holds:** `obsidian_memories.rs` `0x9f7dc79`, def `0x9f7ed88`, renderer `0xa0ba74f`.<br>`POST /memories/sync-external` and its per-destination outcomes already exist in the base (`external_memory_sync.rs:192-240`). |
| connections-agent-memory-guidance | duplicate | **Canonical: meetings_memory-035** (Claude Code sidecar switches to MCP-recall guidance) and the memory-sync bullet in agents_pipes.<br>**Merge into meetings_memory-035:** it has one fact wrong and one missing:<ul><li>**Codex also switched.** The `0x9f90f10` description says raw memory is never embedded in `AGENTS.md`, but meetings_memory-035 says Codex is unchanged.</li><li>**Disconnect cleanup.** Disconnecting removes the screenpipe-owned copies and restores the connection if removal fails (`0x5185988..0x51859b1`).</li></ul>**Base:** descriptions still promise the full digest (`claude_code.rs:16`, `codex.rs:16`). |
| connections-browser-snapshot-v2-act | confirmed | **Target:** `MAX_DEPTH = 12` (`0x517f1d7`) vs base 8 (`connections_api.rs:2412`). Also the `data-sp-ref` stamping script (`0x517eb7f..`), the act script `0x6a320ee`, route `0x5182760`, `BrowserActBody` `0x517e6e8`, actions `0x517d918`, and the ref errors at `0x517d9fe`.<br>**Base:** no `act` route and no `data-sp-ref`. api-031 defers to this item. |
| connections-owned-browser-tabs | duplicate | **Canonical: frontend-013** (same commands, args and events; its touchpoints include `owned_browser.rs`).<br>**Evidence holds:** 8001 bindings `owned_browser_history{tabId,direction}`, `owned_browser_tab_{navigate,set_bounds,hide,close,clear_browsing_data}`, `owned_browser_navigate{url,owner,reveal}`. Base `tauri.ts` has only the single-view commands. |
| connections-google-calendar-multi | confirmed | **Target:**<ul><li>`users/me/calendarList` `0x5254a7e`; pagination `0x52549e8`.</li><li>`GoogleAccountSnapshot` `0x517e8f8`; `require_complete`/`account_snapshots` `0x517e950`; incomplete-snapshot error `0x6a333f8`.</li><li>`iCalUID` `0x6a31fe0`; responseStatus/transparency `0x6a33c00`.</li><li>Publisher `0x498cde8`; retained-snapshot event `0x5287878`.</li></ul>**Base:** hard-codes `"calendarName": "primary"` (`connections_api.rs:1264`).<br>**Caveat:** `owner` directly follows `minAccessRole`, so the list call may be `minAccessRole=owner` (owned calendars only). |
| connections-connection-triggers | duplicate | **Canonical: agents_pipes-008** (confirmed).<br>**Evidence holds:** UI catalog at 5977 @487700..490300: `voice_phrase`, `email_received`/`email_sent`, `outlook_*`, `google_calendar_started`, `github_issue`/`github_pull_request`, `linear_issue_{created,assigned,status_changed}`, `todoist_task`, `obsidian`; kinds `message`, `sent_message`, `event_started`, `issue`, `pull_request`, `issue_created`, `issue_assigned`, `issue_status_changed`, `task`.<br>**Merge into agents_pipes-008:** the catalog and the pickers.<br>**Fix here:** the literal at `0xa11617e` reads `slack` followed by `.connection-triggers.json`, so the cursor file has a leading dot (as agents_pipes-008 says), not `connection-triggers.json`. |
| connections-vault-secrets-db | duplicate | **Canonical: meetings_memory-044** (confirmed), which cites the same hybrid refusal `0x522ae21` and `secrets.sqlite(+journal)` `0x522af47`. db-026 and api-030 restate it.<br>**Base:** the vault manager only handles `db.sqlite*` (`manager.rs:172-190`, `:266-279`).<br>**Merge:** the "close the secret-store pool before locking" note. |
| connections-whatsapp-signout | confirmed | **Target:** sign-out and reconnecting messages at `0x49aee08..0x49aeee4`.<br>**Base:** on `logged_out` it only stops auto-restart and sets Disconnected (`whatsapp/mod.rs:219-223`, `:419-422`). |
| connections-supports-oauth-instances | confirmed | **Target:** `supports_oauth_instances` (`0x52b7810`); `supportsOAuthInstances` ×4 in 5977 (@309699, @360123, @362469, @364694).<br>**Base:** 0 hits. |
| connections-ui-catalog | partially_in_base | **Already in base:** Obsidian `obsidian.json` detection on all three OS paths, multi-vault instances and promote-on-delete (`connections-section.tsx:2516-2652`). The icons `plaud.png`, `krisp.svg`, `hermes.png`, `bee.png`, `voice-memos.svg` and `google-sheets.svg` already ship in base `public/images`.<br>**New:** `excalidraw.svg` and `google-drive.svg`; the grokbot/grok/remote-agent/pi-extensions tiles; the `custom-mcp`, `ics-calendar` and `browser-url` relabels (5977 @351635, @349875); Advanced token fallbacks; the `scopeVariant` deep link (@345186); and the Bee and IMAP panels.<br>**Overlap:** frontend-050 owns the section. Note that it also wrongly lists Obsidian multi-vault as new. |

**Tally (pass 2):**

| Verdict | Count |
|---|---|
| confirmed | 17 |
| partially_in_base | 2 |
| duplicate | 10 |
| already_in_base | 0 |
| unsupported | 0 |

### Missed items (pass 2)

* **connections-chat-context-per-turn** (changed, S). This re-confirms the earlier pass and adds the frontend evidence.
  * **Target:** the pi chat runtime prepends a `<connections_context>` … `</connections_context>` block to each user turn. The block's header says it is refreshed for this turn. Copies are at `0x4992db8`, `0x49f4d58`, `0x5176d20` and `0x5226dc0`.
  * **Target UI:** the chat strips this prefix before showing the user's message (`3258-9bdba2535706ba5a.js` @12751, `2037-06384433950ad22c.js` @1832, plus copies in the settings, search and starred-sessions chunks).
  * **Base:** builds the context once into the system prompt (`standalone-chat.tsx:4702` → `lib/chat/system-prompt.ts:119`). Pipes get it once via `PipeManager::set_connections_context` (`pipes_api.rs:218`). Connects or disconnects made mid-chat therefore stay invisible to the model.
  * **Freenpipe:** render the redacted `render_context` output per turn into that tag, and strip it in the transcript view.
  * No other spec owns this.
* **connections-pipe-mcp-connection-refs** (changed, S).
  * **Target:** a pipe's `connections:` list may reference an MCP server as `mcp:<server_id>`, alongside the base `<id>` and `<id>:<instance>` forms. Helpers are at 5977 @470314..470700: prefix `mcp:`, parse and join helpers, and the picker that lists MCP servers as `mcp:<id>` entries.
  * **Target UI:** the pipe connections dialog (5977 @575200..577400) resolves each ref to one of three kinds:
    * native `connection` (instance-aware);
    * `composio`;
    * `mcp`, with missing reasons `disabled_mcp`, `unknown_mcp` and `deleted_mcp`.
  * **Target chat:** the chat connection list also exposes enabled MCP servers as `mcp:<id>` entries with an `@<slug>` mention (3696 @329538, @544524).
  * **Base:** the gate looks every ref up as `cred:<ref>` (`screenpipe-engine.rs:1428-1447` → `load_connection`). A screenpipe-written pipe declaring `mcp:<id>` would therefore stay in "setup mode" forever in Freenpipe.
  * **Freenpipe:**
    * Teach the gate and `pipes-section.tsx` / `post-install-connections-modal.tsx` that an `mcp:` ref is satisfied by an enabled entry in `mcp_servers.json`. If that server uses OAuth, it also needs a stored `mcp-oauth:<id>` token.
    * Treat Composio-only ids as unavailable with BYO guidance.
  * frontend-049 mentions per-pipe connections only generically.

Checked during the sweep and found **not** to be connections deltas:

| Finding | Why it is not a delta here |
|---|---|
| `screenpipe-secrets/src/telemetry.rs` (`0xa2d2040`, Sentry tags for secret-store failures) | Owned by cloud.md telemetry |
| `cloud.auth_token` secret key | Owned by shell-003 |
| `owned-browser:v20-cookie-blocked` | Already in base `owned_browser_cookies.rs` |
| `/browser/cookies` `host`/`timeout_secs` | Already in base `routes/browser.rs:64` |
| Windows calendar meeting-link hosts (`0x9f9d6b0`) | Already in base `calendar_windows.rs:252` |
| Multi-instance and refresh-failure wording (`0x5186a43..`) | Already in base `oauth.rs:459-468`; only the wording changed |
| Home "connected share" to Slack/Linear | Owned by frontend.md (@593) |
