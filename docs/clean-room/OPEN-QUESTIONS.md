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

## W0-09

1. **Where is the "Freenpipe-controlled mirror" for the pyannote models, and what are the file names?**
   - Decision: this repository's own Git LFS storage, read through GitHub's media endpoint at the
     import commit:
     `https://media.githubusercontent.com/media/normiecore/Freenpipe/bcef0bcde5e6ea12db811a27047ef63869b7374a/crates/screenpipe-audio/models/pyannote/`
     followed by the repository file name, percent-encoded (`segmentation-3.0.onnx`,
     `wespeaker_en_voxceleb_CAM%2B%2B.onnx`). `DEFAULT_MODEL_BASE_URL` in
     `crates/screenpipe-audio/src/speaker/models.rs` and `scripts/stage_bundled_runtime.js` use the same
     base. Downloads are checked against the size and SHA-256 in the Git LFS pointers. An HTTP 404 or
     410 is treated as permanent: the downloader does not retry it, and callers waiting for the model
     get the error at once instead of after the two-minute timeout.
   - Reason: those LFS objects were imported with the fork, the endpoint serves them byte-identical to
     the pointers, and a commit id cannot move, so no release has to be created. An earlier draft of
     this WP pointed at a release (`pyannote-models-v1`) that did not exist; the engine then spent about
     four minutes retrying 404s before it started its server. The local file names are unchanged, so
     existing caches keep working.
   - No owner action is needed.

2. **How do users override the model sources?**
   - Decision: environment variables only, for now: `SCREENPIPE_PYANNOTE_BASE_URL` (mirror base URL),
     `SCREENPIPE_PYANNOTE_DIR` (directory with pre-installed pyannote files), `HF_ENDPOINT`
     (Hugging Face compatible host for the PII models) and one repository override per PII model
     family: `SCREENPIPE_PII_REDACTOR_REPO`, `SCREENPIPE_PII_TEXT_REDACTOR_REPO`,
     `SCREENPIPE_PII_IMAGE_REDACTOR_REPO` (`owner/name`).
   - Reason: the target has no evidence of such settings, and settings keys belong to W1-02. The
     names follow the `SCREENPIPE_*` convention; `HF_ENDPOINT` is the hub's own variable and is already
     listed in shell.md. SHA-256 pins stay in force for every source, so a mirror cannot change the
     weights.

3. **Do the PII weights keep downloading from the vendor's Hugging Face organisation by default?**
   - Decision: yes, as cloud.md cloud-024 says (opt-in only, when local AI PII removal is enabled;
     regex-only redaction needs no download). The licence of those weights is still unknown and stays
     an open question for the owner.

4. **How is the CLI bundled, given that the base ships no standalone CLI?**
   - Decision: the engine's `screenpipe` binary is staged by `scripts/stage_bundled_runtime.js`
     (`FREENPIPE_CLI_BINARY=<path>` or `FREENPIPE_BUILD_CLI=1`) into `src-tauri/bundled/cli/` and
     mapped as a resource into the install root next to the app, bun and the runtime DLLs. It is not an
     `externalBin`, so development builds without a staged CLI still build. Only the Windows bundle
     maps it (`tauri.windows.conf.json`); macOS and Linux keep working without it (agents use the
     REST API, MCP snippets show placeholders, models download on first use).
   - Reason: the CLI must sit beside the DLLs it loads; an `externalBin` would make every Windows dev
     build depend on a release build of the engine.
   - Hand-off to the release pipeline (`.github/workflows/release-app.yml`, W0-03, already merged):
     the Windows "Run pre_build.js" step should set `FREENPIPE_BUILD_CLI=1` (or build the CLI in an
     earlier step and pass `FREENPIPE_CLI_BINARY`) and `FREENPIPE_REQUIRE_BUNDLED_RUNTIME=1`. Until it
     does, a Windows installer built the default way ships without the CLI. Agents then fall back to
     the REST API: the screenpipe-cli skill and the pipe-creation prompts say so. In required mode the
     staging step refuses a CLI left over from an earlier run.

5. **Should agents fall back to a `screenpipe` found on `PATH`?**
   - Decision: no. Agents get `SCREENPIPE_CLI`, a launcher directory (`<data dir>/pi-agent/bin`)
     first on `PATH`, and a bash function in the `BASH_ENV` shim, all pointing at the bundled binary.
     Only the MCP server's API-key discovery tries `screenpipe` on `PATH`, as its last CLI option.
   - Reason: a `screenpipe` on `PATH` may belong to another install of a different product; agents
     acting on the user's data should use the binary that came with this app.

6. **How is the MCP server shipped?**
   - Decision: `packages/screenpipe-mcp` is installed with `bun install --frozen-lockfile` and
     bundled by `bun build --target=node --format=cjs` into one file, `mcp/screenpipe-mcp.js`. Every
     generated MCP config runs `<bundled bun> <that file>` by absolute path (`lib/mcp-config.ts`),
     with `--port` when the API does not use 3030. The staging step fails if the bundle mentions a
     registry package (`screenpipe@latest`, `screenpipe-mcp@latest`) or a vendor host
     (`screenpi.pe`, `screenpipe.com`, `mediar.ai`). To pass that check this branch drops the
     enterprise `team-*` tools (`TEAM_API`, `fetchTeam`), with deletions identical to W0-05's, so
     the two branches merge cleanly there.
   - Reason: a single CommonJS file needs no install step and runs under bun or node, and the server
     always matches the installed app version. The check turns a vendor call that slips back into
     the package into a failed build instead of a shipped one.

7. **Does the standalone CLI show an update notice?**
   - Decision: no. None of W0-09's items (cloud-024, cloud-046, cloud-M02, agents_pipes-040) asks
     for one, so the merged `crates/screenpipe-engine/src/cli_reminder.rs` is W0-05's network-free
     tip rotation unchanged: no version check, no registry query, no `npx` lines. Its MCP tip already
     says `screenpipe mcp setup`, which is what W0-09 needed from that file. The CLI starts no update
     fetch.
   - Reason: an earlier draft of this branch kept an opt-in check against a user-configured updater
     manifest; once W0-05 removed the check altogether there was nothing left for W0-09 to require,
     and a startup fetch that no spec item asks for is not worth carrying. Desktop update checks stay
     with the app updater (W0-03).

8. **Which SKILL.md do the agent cards serve?**
   - Decision: `scripts/gen-skill-content.js` now reads the product copies under
     `crates/screenpipe-core/assets/skills/`, and the Hermes card installs skills from that folder of
     this repository. `.claude/skills/` (the repository's developer mirror, still mentioning the npm
     CLI) is no longer read by the app.
   - Reason: the product copies are what the app installs into its own agent sessions, and the
     mirror is edited by other tooling.

9. **Where does the local API key go in MCP configs written by onboarding?**
   - Decision: when the app ships the CLI (`mcp_runtime_check` reports `cliPath`), onboarding writes
     no key; the MCP server runs `<install>/screenpipe auth token` at start and reads it from the
     secret store. Only builds without the CLI write `SCREENPIPE_LOCAL_API_KEY` into the Claude
     Desktop, Cursor and Codex configs. Settings > Connections keeps writing the key, as the base app
     did there.
   - Reason: a plain-text copy of the key in third-party config files is avoidable once the CLI can
     hand it over; the fallback keeps the tools working on builds without the CLI.

10. **Does the PII model download honour the switchboard's model mirror setting?**
    - Decision: not yet. `screenpipe-redact` reads `HF_ENDPOINT` directly (`adapters/hf_source.rs`),
      which is the same variable the switchboard's `Endpoint::ModelMirror` reads, so environment
      configuration behaves the same. A settings override of `ModelMirror` (W1-02) would not reach
      it: `screenpipe-redact` does not depend on `screenpipe-core`.
    - Hand-off to W1-02: when the `ModelMirror` settings key lands, pass
      `screenpipe_core::endpoints::model_mirror_base()` into `screenpipe-redact` (a small setter or
      resolver hook in `hf_source.rs`, installed by the engine at start).

11. **What licences cover the bundled speaker models?**
    - Decision: pyannote segmentation-3.0 is MIT; the WeSpeaker CAM++ VoxCeleb embedding model is
      CC BY 4.0 (WeSpeaker ties VoxCeleb-trained models to the dataset licence). Attribution is in
      `crates/screenpipe-audio/models/pyannote/NOTICE.md`, staged next to the models in the installer,
      and `NOTICE.md` points to it.
    - Open for the owner: the build VM could not reach the Hugging Face model card, so the MIT
      declaration for segmentation-3.0 is from memory of that card and should be confirmed before a
      release.
## W0-10

1. **Where does the bundle go?** cloud-036 says `<data_dir>/feedback/`, shell-033 says Downloads.
   **Decision:** `<data dir>/feedback/<YYYYMMDD-HHMMSS>.zip` (local time, `-2`, `-3` suffixes on a
   clash), where the data dir is the default one (`SCREENPIPE_DATA_DIR` or `~/.screenpipe`) that holds
   the logs, not a custom recording `dataDir`. Logs from the custom dir are still collected when it
   differs. **Reason:** the WP acceptance check names `<FP_DD>/feedback/`, and keeping bundles beside
   the logs needs no extra permission or folder picker.
2. **`read_log_tail` arguments and limits.** **Decision:** `read_log_tail(path, maxBytes)` as the
   shell-033 correction says; default 100 KB, clamped to 1 KB..8 MB; the file must resolve (symlinks
   followed first) inside the default or configured data dir and have `.log` in its name; a cut tail
   starts on a whole line and carries a one-line marker with the shown and total KB. **Reason:** a
   renderer-callable file reader must not become a general file read.
3. **Sensitive-key rule for settings.** **Decision:** the target's fragments plus `passphrase`,
   `cookie`, `privatekey`, `devicename` and any key ending in `key`; exceptions are the target's list
   plus `maxTokens`. The rule lives in `apps/screenpipe-app-tauri/lib/feedback/sensitive-keys.json`,
   read by both the TypeScript helper and the Rust writer, with shared test vectors. **Reason:**
   Freenpipe settings hold keys such as `deepgramApiKey`, `openaiKey` and `licenseKey` the target list
   misses, and `maxTokens` matches `token` while being harmless and useful for debugging.
4. **What does a masked value look like?** **Decision:** strings and numbers become `[redacted]`,
   non-empty arrays and objects become `[redacted: N items]` / `[redacted: N keys]`; booleans, nulls,
   empty strings and empty containers are kept. **Reason:** "is it set / is it on" stays answerable
   without exposing the value.
5. **How is free text scrubbed (`redact_pii_for_feedback`)?** **Decision:** the local deterministic
   detector from `screenpipe-redact` with labels secret, email, phone, url and id always on, plus the
   user's `piiRedactionLabels`; then user names in home-folder paths, IPv4 addresses other than
   127.x and 0.0.0.0, bearer tokens and `password=`-style values, and the signed-in user's name,
   e-mail, the computer's host name and the OS login name. Bare IPv4 addresses, which the shared
   detector labels `url`, are left to the IPv4 rule so loopback stays readable; URLs with
   credentials are still removed whole. Host and login names are scrubbed only when they have four
   or more characters and are not generic (`admin`, `user`, `owner`, `desktop`, ...). `settingsJson`
   is optional and bad JSON is ignored. No remote or model-based redaction. **Reason:** decloud (no
   Tinfoil enclave), and a bundle may end up in a public issue; generic names would blank out
   ordinary words in the logs.
6. **Does the app open the GitHub issue by itself?** **Decision:** no. The completion event carries
   `issueUrl` and the UI offers "open github issue". The body holds a scrubbed excerpt of the note
   (shortened so the URL stays under 7,000 characters), the version, OS, report source and the bundle
   file name, never the full local path. **Reason:** the WP says "offer"; opening a browser
   unasked is surprising, and the full path contains the user name.
7. **`feedback-upload-completed` payload.** **Decision:** `{ jobId, sent: false, path, fileName,
   sizeBytes, issueUrl, error }`; failures send the same event with `error` set and `path` null.
   **Reason:** superset of the spec's `{jobId, sent:false, path}` so the UI needs no second call.
8. **Native notification on completion?** **Decision:** none. The dialog shows the completion
   state, and a toast appears when the dialog was closed while the bundle was being written.
   **Reason:** the bundle is user-initiated and usually finishes in seconds.
9. **Retention.** **Decision:** keep the newest 10 bundles in `feedback/` and delete leftover
   `.zip.partial` files; only files with the bundle naming pattern are touched. **Reason:** a bundle
   can carry a 50 MB recording; unbounded growth in the data dir is a disk-pressure risk.
10. **Defaults for optional content.** **Decision:** chat history is off by default (base had it on);
    screenshot and recording (mp4/mov, 50 MB at most) only when the user adds them; up to three
    newest crash dumps (64 MB each at most) from `<data dir>/crash-dumps/` are always included. The
    README states that screenshots, recordings and dumps are not scrubbed. **Reason:** privacy by
    default for content the user may post publicly; crash dumps are what shell-033 asks for.
11. **More than one bundle at once?** **Decision:** one job at a time; a second
    `start_feedback_upload` while one runs returns an error. **Reason:** avoids two jobs pruning or
    naming bundles in the same second.
