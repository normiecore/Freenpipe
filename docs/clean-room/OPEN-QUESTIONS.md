# Open questions

Spec ambiguities that a work package had to decide. Each entry gives the question, the decision
taken and the reason. Later work packages may revisit a decision; say so under their own heading.

## W0-05

- **Question:** the base Settings "Usage" section only counted local chat and pipe requests per
  model; it never called the vendor. Keep it, given that the acceptance check asks for Settings
  without a Usage section and frontend-035 allows "hide `usage` or replace it with local usage
  stats"?
  **Decision:** removed the section and `usage-section.tsx`; links to `?section=usage` land on the
  default section.
  **Reason:** the acceptance check is explicit, the page labelled hosted usage as the vendor's
  cloud, and the slot is the one the target fills with hosted AI credits. frontend-035 (settings
  nav v2) can bring back a local usage page if it is wanted.

- **Question:** `lib/hooks/use-team.ts` and `lib/hooks/use-usage-status.tsx` should go, but
  `components/settings/ai-presets.tsx` (owned by W0-06 this wave, "checkout link only" for W0-05)
  still imports them. Delete and edit ai-presets, or keep them?
  **Decision:** kept both files as inert local stubs with the same exports: `useTeam()` reports no
  team and its `pushConfig` rejects; `useUsageStatus()` returns `null`. Neither makes a request.
  **Reason:** keeps W0-05 out of W0-06's regions so the wave merges cleanly. W0-06 or WR-03 can
  delete the stubs once the preset share button and quota chip are gone from ai-presets.

- **Question:** the bundled `screenpipe-team` agent skill and the Privacy "Admin Team API Token"
  card are clients of the same hosted team console as `cli/team.rs` and `TEAM_API`, but are not in
  the WP's owner paths. Leave them?
  **Decision:** the skill is never installed and stale copies are deleted from agent sessions
  (`PiExecutor::ensure_screenpipe_team_skill`); the skill asset and the admin token card are
  removed. The `screenpipe-team` name stays reserved in the baseline skill list.
  **Reason:** the data dir is shared with screenpipe (DECISIONS 1), so an `enterprise.json` written
  by screenpipe would otherwise make the Freenpipe agent query the vendor's console.

- **Question:** the CLI checked `registry.npmjs.org/screenpipe/latest` at every engine start and
  every few minutes. Repoint it at GitHub Releases or drop it?
  **Decision:** dropped. CLI tips are now printed from local text only and link to the Freenpipe
  repository.
  **Reason:** it compared against the vendor's npm package, the target no longer has
  `SCREENPIPE_NO_UPDATE_CHECK` (shell-025), and DECISIONS 3 keeps update checks off by default.

- **Question:** remove `screenpipe survey` or keep it?
  **Decision:** kept; it prints the GitHub Discussions and new-issue links and opens nothing.
  **Reason:** cloud-023 asks for exactly that, and scripts that call it keep working.

- **Question:** where does the changelog dialog read from, and when?
  **Decision:** the unauthenticated GitHub Releases API of normiecore/Freenpipe, only when the
  dialog opens, cached for one hour in localStorage (`freenpipe:changelog-cache`); a stale cache is
  shown when GitHub is unreachable, and the bundled `CHANGELOG.md` when there is nothing cached.
  **Reason:** cloud-023 names both sources; reading only on open keeps the app from making
  background requests.

- **Question:** besides `/purchase-successful`, the Tauri configs registered a `/stripe-connect`
  app link. Keep it?
  **Decision:** removed from `tauri.conf.json`, `tauri.prod.conf.json` and `tauri.beta.conf.json`.
  **Reason:** it is a payments flow (cloud-044) and its only handler lived in the deleted Account
  section.

- **Question:** should the vendor guard's new commerce rules wait for strict mode like the host
  rules?
  **Decision:** `COMMERCE_RULES` fail in every mode, with a reasoned allow-list
  (`COMMERCE_ALLOWED`) for code that names a vendor identifier in order to ignore it.
  **Reason:** W0-05 leaves no such surface behind, and the target-only ones (trial, card ask,
  annual banner, announcements, remote control policy, hosted quotas) are never ported, so any new
  hit is a regression worth stopping at once.

- **Question:** the URL-detection benchmark fixture contains OCR text `/api/cloud-sync/checkout`,
  which the acceptance grep flags.
  **Decision:** changed those two OCR samples to `/api/cloud-sync/status` (and its OCR misread).
  **Reason:** both are non-URL path strings with no expected URLs, so the benchmark measures the
  same thing; its results are unchanged.

- **Question:** cloud-044 lists `/api/cloud-sync/subscription` among the billing calls, and the
  base Sync panel still asks it whether the user has a sync plan. W0-05 lists `sync-settings.tsx`
  for its checkout links only, while W0-08 owns the rest of the file. Remove the lookup here too?
  **Decision:** no. W0-05 removes the checkout, the plan card and the subscribe flow; the plan
  lookup is left to W0-08, which removes it and hides the Sync and Archive tabs.
  **Reason:** both packages would otherwise rewrite the same hunk of the same function in the
  same wave. Once both merge, nothing in the panel names a vendor endpoint.

- **Question:** the crash screen (`app/global-error.tsx`) sent "get help" to the vendor's Discord
  invite. The file is in no work package's owner paths.
  **Decision:** W0-05 points it at a new issue on the Freenpipe repository, and the vendor guard's
  commerce rules now include the vendor's Discord invite (`community-invite`).
  **Reason:** the WP text sends every Discord link to GitHub, and an issue is where a crash report
  belongs. The help panel already lists Discussions for questions.

## W0-07

- **Key family for BYO client credentials.** connections.md suggests `oauth-client:<id>`, cloud-018 and
  PLAN.md say `oauth_client:<id>`. Decision: `oauth_client:<integration_id>`, holding
  `{client_id, client_secret?, redirect_uri?}`. Reason: PLAN.md names cloud-018 as canonical; the
  underscore also keeps the rows outside the `oauth:` token prefix the refresh scheduler scans.
- **Extra `OAuthConfig` fields.** The plan names only `token_url` and `uses_pkce`. Decision: also add
  `token_auth` (client_secret_post or client_secret_basic), `token_body` (form or JSON) and
  `requires_https_redirect`, and drop `client_id` and `redirect_uri_override`. Reason: Notion, Zoom,
  Intuit and Supabase want HTTP Basic client authentication, Notion and Atlassian document JSON bodies,
  the vendor client ids are useless without the vendor's secret, and the override only ever held the
  vendor relay.
- **HTTPS-only providers (Slack, Zoom, Calendly, QuickBooks).** Decision: there is no default redirect
  for them; the OAuth app dialog requires a redirect URI, and saving without one fails with an
  explanation. Any `https://` URL or a loopback `http://` URL is accepted. Reason: the spec asks for a
  user-supplied relay with no Freenpipe-hosted one; accepting an explicit loopback URL keeps Intuit
  development keys (which allow localhost) usable, and the provider itself rejects what it does not allow.
- **Which providers get PKCE.** Decision: on for Google, Microsoft (365 and Teams), Zoom, Calendly and
  Supabase; off for Notion, GitHub, Slack, Atlassian, Cal.com, Intuit and Vercel. Reason: enabled where
  the provider documents S256 support; RFC 6749 servers ignore unknown parameters, but staying
  conservative avoids surprises with strict endpoints.
- **Cal.com token endpoint.** The spec asks to verify it against current Cal.com docs; the cloud VM has
  no route to cal.com. Decision: `https://app.cal.com/api/auth/oauth/token`, form body, client secret in
  the body. Reason: best available knowledge; a smoke test with a real Cal.com app should confirm it and
  whether refreshes need a different path.
- **Default loopback port.** `OAUTH_REDIRECT_URI` hard-codes port 3030. Decision: the app builds the
  default redirect from the port the embedded engine was started on; before the engine exists it falls
  back to a valid `SCREENPIPE_PORT` (the override `to_recording_config` applies), then the stored
  `recording.port`, then 3030. The dialog shows the exact URI to register, and connect sends the same
  one. Reason: dev builds run on 3035 through `SCREENPIPE_PORT` while settings keep 3030, and a 3030
  redirect would land where no Freenpipe engine listens (or on an installed screenpipe).
- **MCP OAuth redirect.** Decision: `POST /mcp-servers/:id/oauth/start` accepts a `redirect_uri` only when
  it is the loopback `/mcp-servers/<id>/oauth/callback` of this engine; the default takes its port from
  a loopback `Host` header (else 3030); an `app_scheme` field is ignored. Reason: the plan keeps MCP OAuth
  on the local callback, and the old code passed any caller-supplied redirect through.
- **Refresh without an OAuth app.** Decision: the background scheduler skips such tokens (counted as
  `refreshes_skipped`, no failure cooldown), and on-demand refresh fails with a "no OAuth app is
  configured" error. Removing an OAuth app keeps existing tokens. Reason: the fix is in settings, not in
  waiting; deleting tokens on app removal would lose identity metadata the user may want back.
- **Google's dedicated cards.** Decision: the OAuth app block is rendered above the Google Calendar, Docs,
  Sheets and Gmail cards from `connections-section.tsx`, which also passes each card a
  `connectBlockedReason` prop (from `connectBlocker`). Inside the cards the changes are that prop, which
  disables the connect / add-account button with the reason as its tooltip, and an error toast on the
  Gmail, Docs and Sheets connect buttons, as the Calendar card already had. Reason: cloud-018 asks for a
  disabled OAuth button with guidance; the cards are outside this WP's owner paths, but no other WP owns
  them, and with user-entered credentials a missing or rejected OAuth app is the common failure.
- **Other OAuth connect surfaces.** The onboarding "connect apps" tiles and the pipe post-install
  connections dialog call `oauth_connect` too and dropped an error result silently. Decision: both now
  show the backend message (the onboarding tile stays in its error state until clicked again; the dialog
  prints it under the connect button), which names Settings > Connections when no OAuth app is set up.
  They do not embed the OAuth app form. Reason: the smallest change that gives the guidance cloud-018
  asks for in files other WPs own in other regions (W0-05, W0-09, W4-07 for `connect-apps.tsx`; W2-20
  for `post-install-connections-modal.tsx`).
- **`oauth:` rows outside the registry.** `oauth:chatgpt` (and any future non-registry row) sits under the
  token prefix the refresh scheduler scans. Decision: the scheduler treats "not a registry OAuth
  integration" like "no OAuth app": skipped, not counted as a failure, no cooldown. Reason: those rows
  are refreshed by their own flows, and failing them every tick only produces warnings.
- **Tokens carried over from screenpipe.** Drop-in data dirs can hold `oauth:<id>` tokens that the
  vendor's OAuth apps issued, and a refresh token only works for the client that obtained it. Decision:
  no conversion. Until an OAuth app is entered the scheduler skips them; afterwards the first refresh
  fails with the provider's grant or client error, the normal failure cooldown applies, and the user
  reconnects. Reason: nothing local can re-bind a grant to another client, and the vendor's secrets are
  not available.
- **Errors when no OAuth app is stored.** Decision: `describe_oauth_error` (used by the Gmail, Google
  Calendar and proxy routes) adds a sentence saying no OAuth app is configured, except in the
  multiple-accounts case, and only when a SecretStore is available to check. Reason: "reconnect it" alone
  sends the user round a loop that cannot succeed without an app; the CLI has no store, so it cannot tell.

## W0-08

- **How is a "static registry" told apart from a registry API when both come from `SCREENPIPE_API_BASE_URL`?**
  Decision: a URL whose path ends in `.json` is read as a static index; any other URL is the base of a server speaking the pipe store API (`GET {base}/api/pipes/store`, `GET {base}/api/pipes/store/{slug}`).
  Reason: no new env var or settings key (keys belong to W1-02), existing API-base users keep working, and it matches WR-01's planned `index.json` default.
- **What does a static index entry look like?**
  Decision: the document is an array, or an object with a `data` (or `pipes`) array. Entries need `slug` (or `id`); `title` falls back to `name`, then the slug. The source is inline `source_md`, or `source_url` / `source_path` resolved against the index URL (http/https only). An optional `sha256` (hex, optional `sha256:` prefix) must match the source. `version` is an integer (integer strings accepted, default 1). Listings drop `source_md`; search/category/sort (`popular`, `newest`, `rating`, `name`) and offset/limit run locally.
  Reason: the smallest format that works from any static host; WR-01 can extend it.
- **Which HTTP statuses go with the typed registry error codes, and what is returned when no registry is set?**
  Decision: `{error, error_code}` envelopes. `registry_http_error` 502 (plus `upstream_status`), `registry_network` 502, `invalid_registry_response` 502, `registry_timeout` 504, `pipe_not_found` 404. Two codes are new: `registry_not_configured` 503 (detail/install/update without a registry) and `registry_read_only` 501 (publish, unpublish, review). Local write failures use `installation_failed` 500, a code the target frontend already knows. The listing without a registry is `200 {"data":[],"registry_configured":false}`; update checks return `{"data":[]}`.
  Reason: the target strings give the codes but not statuses, and say nothing about an unset registry.
- **Should publish/review work against a user-configured registry API?**
  Decision: no, they are always refused for now.
  Reason: the base handlers forwarded the request's bearer token, which in Freenpipe is the local API key; sending it to a remote host would leak it. WR-01 turns publishing into a ready-to-PR pipe folder.
- **How is a sync server configured for cloud sync/archive?**
  Decision: not at all in W0-08. `SyncClientConfig::new` has no server, every sync API call fails with `SyncError::NotConfigured` before any network traffic, the app's `sync_server_url()` returns `None`, and the engine's `/sync/init` and `/archive/init` answer 501. The settings UI hides both tabs (`lib/cloud-storage.ts`).
  Reason: settings keys belong to W1-02 and bring-your-own storage is WR-02; 501 matches what api-030 asks for vendor-only sync routes.
- **Which registry slugs are accepted?**
  Decision: ASCII letters, digits, `-`, `_`, `.`; first and last character alphanumeric; at most 128 characters; no Windows device names (`con`, `nul`, `com1`...). Anything else is `pipe_not_found` without a network call.
  Reason: slugs become directory names under the pipes dir, and `/pipes/store/install` is reachable without the API key, so traversal must be impossible.
- **What happens to `screenpipe install <manifest>` and its trusted hosts?**
  Decision: the manifest URL is now required (the vendor starter manifest default is gone), there is no built-in trusted host list, and the configured registry's host plus `SCREENPIPE_TRUSTED_HOSTS` are trusted. Without a registry the command stops before fetching anything.
  Reason: no vendor defaults; a manifest only names slugs, so it is useless without a registry.
- **Should `screenpipe pipe install <slug>` look the slug up in the registry?**
  Decision: yes, when the source is not an existing path or URL and has the slug shape. `pipe info` already printed that hint, but base `pipe install` rejected slugs.
  Reason: makes the CLI consistent with the store route at no extra surface.
- **Does "export pipe file" mask secrets?**
  Decision: yes, credential-shaped strings (API keys, tokens, bearer values, `*_KEY=`/`PASSWORD:` style assignments) are masked and the toast reports how many.
  Reason: the export replaces "share", so the file is meant to leave the machine; the pipe on disk is untouched.
- **Should installs still bump a registry's install counter?**
  Decision: only for a registry API (best-effort `POST {base}/api/pipes/store/{slug}/install` to the user's own server); static indexes have none.
  Reason: there is no vendor default any more, so the counter only reaches a server the user chose.

## W0-11

1. **Which sqlx 0.9 release, and which SQLite crates go with it?**
   Decision: sqlx 0.9.0 (stable, published 2026-05-21), libsqlite3-sys 0.37.0, rusqlite 0.39.0. The
   Tauri app crate uses the same three versions in its own lockfile.
   Reason: 0.9.0 is the only stable 0.9 release and is older than two weeks. sqlx-sqlite 0.9.0
   accepts libsqlite3-sys `>=0.30.1, <0.38.0`, so 0.37.0 is the newest it can share, and rusqlite
   0.39.0 is the last rusqlite on 0.37 (0.40 moves to libsqlite3-sys 0.38, which would put a second
   `links = "sqlite3"` crate in the graph). libsqlite3-sys 0.37.0 bundles SQLite 3.51.3, source id
   `2026-03-13 10:38:09 737ae4a3...`, the same date the spec records for the target. These are
   also the versions the spec lists for the target (db-029).

2. **sqlx 0.9.0 declares `rust-version = 1.94.0`; the repository pinned Rust 1.93.1.**
   Decision: move `rust-toolchain.toml` to 1.94.1 (the last 1.94 patch, 2026-03-26) and the app
   crate's `rust-version` to 1.94. bootstrap.sh's fallback channel, the generated cloud setup script
   and windows-build.md follow.
   Reason: cargo refuses to build a package whose `rust-version` is newer than the active compiler.
   `--ignore-rust-version` would have to be passed on every cargo invocation (it has no config key),
   which is not workable for CI, tauri builds or contributors.

3. **What does "recover extension compiled in" require from this WP?**
   Decision: the single bundled SQLite is built with `SQLITE_ENABLE_DBPAGE_VTAB` (through
   `LIBSQLITE3_FLAGS` in `.cargo/config.toml`), which is the core feature the recover extension
   reads pages through; FTS5 is already enabled by libsqlite3-sys's bundled build. The extension's
   own C sources (`sqlite3recover.c`, `dbdata.c`) are not part of the amalgamation and are left to
   W4-09's `screenpipe-sqlite-recovery` crate, which can compile them against the same headers via
   the `DEP_SQLITE3_INCLUDE` variable that libsqlite3-sys exports.
   Reason: the target ships page-level recovery as its own crate (db-020); compiling the extension
   here as well would give W4-09 duplicate C symbols to untangle. A test pins the prerequisite
   (`sqlite_dbpage` must be queryable and `PRAGMA compile_options` must list `ENABLE_DBPAGE_VTAB`).
   A build started from outside the repository tree does not read `.cargo/config.toml`; that test
   is what catches it.
   Note for W4-09: libsqlite3-sys 0.37.0 ships only `sqlite3.c`, `sqlite3.h` and `sqlite3ext.h`.
   Cargo hands `DEP_SQLITE3_INCLUDE` only to build scripts of crates that depend on libsqlite3-sys
   directly, so the recovery crate must list it itself (`{ workspace = true }`). The matching
   public-domain sources are `ext/recover/sqlite3recover.{c,h}` and `ext/misc/dbdata.c` at tag
   `version-3.51.3` of the official GitHub mirror `sqlite/sqlite`; www.sqlite.org is not reachable
   through the cloud VM's proxy, the GitHub mirror is.

4. **What replaces sqlx's removed `runtime-tokio-rustls` feature?**
   Decision: `runtime-tokio` with no TLS backend, declared once in `[workspace.dependencies]`.
   Reason: only the SQLite driver is used and it never opens a network connection. Dropping the TLS
   feature also removes rustls 0.21 and its webpki stack from the lockfile.

5. **parquet, zstd, sha2 and rusqlite are added to screenpipe-db before any code uses them.**
   Decision: parquet 59.3.0 with default features off and only `zstd` on (no arrow, no other
   codecs), zstd 0.13 (the major parquet 59 uses, so one zstd-sys build), sha2 0.10.9 (already in
   the lockfile), rusqlite with `vtab`. They are listed in `[package.metadata.cargo-machete]
   ignored` until the storage module (W5-07) and the virtual tables (db-022) use them.
   Reason: the plan asks for them in this WP so the SQLite toolchain is settled in one change. The
   lean parquet feature set keeps build time down; W5-07 can enable `arrow` if its writer needs it.
   parquet 59.3.1 and the newest twox-hash and zstd-sys releases were younger than two weeks, so the
   lockfiles pin parquet 59.3.0, twox-hash 2.1.4 and zstd-sys 2.1.0+zstd.1.5.7.

6. **The bundled SQLite makes `PRAGMA quick_check` run FTS5 integrity checks.**
   Decision: keep the startup `quick_check(1)` for file databases (db-016 keeps it in the target,
   which links the same SQLite), run it on its own short-lived read-only connection
   (`DatabaseManager::run_startup_integrity_check`) instead of a pooled one, and skip it for
   in-memory databases.
   Reason: since SQLite 3.44, integrity_check and quick_check call each virtual table's xIntegrity,
   and FTS5 ignores the quick flag. With 3.51.3 that causes two problems.
   - False corruption on healthy file databases. FTS5's integrity method reads the segment layout
     the connection cached during earlier FTS queries, and unlike an ordinary FTS read
     (`fts5NewTransaction` from xOpen/xBegin) it does not first check whether another connection
     has rewritten the index. A read-pool connection that served a `frames_fts` search before the
     writer merged segments therefore reported `fts5: corruption found reading blob ... from table
     "frames_fts"`, and the engine logged "DATABASE CORRUPTION DETECTED" for a sound database (found
     in review; 10 of 10 pooled checks in the reviewer's probe). A connection opened only for the
     check has nothing cached; its verdict matches FTS5's own `integrity-check` command. db-019
     builds quarantine and fail-closed writer admission on this check, so it must not raise false
     alarms. `sqlite_toolchain_test::startup_integrity_check_passes_after_searches_and_fts_merges`
     pins this (searches through the pool, more frames so segments merge, then the check says
     "ok"); a pooled quick_check in the same state reports the corruption. The engine CLI's
     `db check` runs the sqlite3 shell, a fresh connection, so it is not affected. The upstream
     `repair_database` still verifies on a pooled connection but has no callers; whoever revives it
     should switch it to a fresh connection too.
   - A hang on in-memory databases. On the shared-cache connections sqlx opens for
     `sqlite::memory:` pools, the check never returns (reproduced with a one-row FTS5 table).
     In-memory databases have no file that could be damaged, and the hang froze screenpipe-db's
     timeline performance test ten seconds after startup.
   On large file databases the check now also walks the FTS5 indexes, so it takes longer; it still
   runs in the background.
