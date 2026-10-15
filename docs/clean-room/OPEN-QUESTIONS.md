# Open questions

Spec ambiguities resolved during implementation: the question, the decision taken and why. Grouped by
work package.

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

5. **Should agents fall back to a `screenpipe` found on `PATH`?**
   - Decision: no. Agents get `SCREENPIPE_CLI`, a launcher directory (`<data dir>/pi-agent/bin`)
     first on `PATH`, and a bash function in the `BASH_ENV` shim, all pointing at the bundled binary.
     Only the MCP server's API-key discovery tries `screenpipe` on `PATH`, as its last CLI option.
   - Reason: a `screenpipe` on `PATH` may belong to another install of a different product; agents
     acting on the user's data should use the binary that came with this app.

6. **How is the MCP server shipped?**
   - Decision: `packages/screenpipe-mcp` is bundled by `bun build --target=node --format=cjs` into one
     file, `mcp/screenpipe-mcp.js`, and every generated MCP config runs `<bundled bun> <that file>`
     by absolute path (`lib/mcp-config.ts`). The config also carries `SCREENPIPE_LOCAL_API_KEY` and a
     `--port` when the API does not use 3030.
   - Reason: a single CommonJS file needs no install step and runs under bun or node, and the server
     always matches the installed app version.

7. **What does the standalone CLI's update notice compare against?**
   - Decision: the latest GitHub release of `normiecore/Freenpipe` (`tag_name`), instead of the npm
     registry entry of the upstream `screenpipe` package.
   - Reason: the upstream package version says nothing about Freenpipe releases, and querying it
     resolves a vendor package (cloud-M02). W0-05 owns `cli_reminder.rs` and removes the update check
     altogether; once both are merged, W0-05's version of the file applies.

8. **Which SKILL.md do the agent cards serve?**
   - Decision: `scripts/gen-skill-content.js` now reads the product copies under
     `crates/screenpipe-core/assets/skills/`, and the Hermes card installs skills from that folder of
     this repository. `.claude/skills/` (the repository's developer mirror, still mentioning the npm
     CLI) is no longer read by the app.
   - Reason: the product copies are what the app installs into its own agent sessions, and the
     mirror is edited by other tooling.
