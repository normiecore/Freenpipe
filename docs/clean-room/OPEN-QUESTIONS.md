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

## W0-06

1. **Where does the AI gateway credential come from?**
   Decision: a new `aiGatewayApiKey` setting next to `aiGatewayUrl` (both typed fields on the app's
   `SettingsStore`, optional in TypeScript), pushed into the running engine with the existing
   `set_cloud_token` command; headless runs use `SCREENPIPE_AI_GATEWAY_KEY`. The engine's
   `SCREENPIPE_API_KEY` (the local API key in Freenpipe) and any cached screenpipe sign-in token are
   never sent to a gateway.
   Reason: cloud-042 names only the URL setting, and cloud-013 says the cloud-token plumbing carries
   the gateway key; reusing `SCREENPIPE_API_KEY` or `user.token` would leak the local key or a vendor
   token to a third-party host. Two settings keys were added in this WP (PLAN.md section 2 rule 7).

2. **What exactly is the gateway base URL?**
   Decision: the OpenAI-style base including its version path (for example `http://127.0.0.1:4000/v1`),
   like a custom preset URL. Feature paths are appended to it: `chat/completions`, `models`, `listen`,
   `realtime` (websocket scheme) and `web-search`.
   Reason: cloud-042 says the helper "appends paths `/chat/completions`, `/models`, `/usage`, `/listen…`"
   to the base, which only works when the base carries the version segment.

3. **How are stored `screenpipe-cloud` presets migrated?**
   Decision: once (flag `_freenpipeCloudPresetMigrated`), when no gateway is configured, each preset on
   `screenpipe-cloud` or legacy `pi` becomes a local Ollama preset in place, keeping its id, prompt and
   default flag, with the model cleared. With a gateway configured they are kept (legacy `pi` renamed).
   The Rust store and the frontend run the same step under the same flag.
   Reason: keeping ids lets pipes that name a preset still resolve it; the vendor model names mean
   nothing to Ollama. Running it in Rust too means pipes see migrated presets before the UI loads.

4. **Fresh preset has an empty model.**
   Decision: as the spec says (`model: ""`, the user picks). Consequence: pipes and chat on the default
   preset fail until a model is picked. A clearer "pick a model" error in pipe preset resolution
   (`crates/screenpipe-core/src/pipes/mod.rs`, not an owner path here) is left to the pipes WPs.

5. **Should a stored `piiBackend: "tinfoil"` always become `local`?**
   Decision: yes, once (flag `_freenpipePrivacyDefaultsApplied`, which also turns off the legacy
   `analyticsEnabled` and `cloudMediaAnalysisEnabled`). The enclave option is shown only when
   `SCREENPIPE_PRIVACY_FILTER_ENCLAVE` (or `_URL`), `SCREENPIPE_PRIVACY_FILTER_REPO` and
   `SCREENPIPE_PRIVACY_FILTER_API_KEY` are all set; no settings field was added for it. The attestation
   repo has no default any more, and the local API key is no longer an enclave credential fallback.
   Reason: a stored `tinfoil` in a screenpipe store points at the vendor enclave; re-selecting it after
   configuring your own enclave is one click. Environment-only configuration avoids more settings keys
   for a power-user feature. The enclave host is read through the W0-01 switchboard
   (`endpoints::privacy_filter_base()`, same variable names) by the engine and the app, so a later
   settings override applies everywhere.

6. **How is the cloud media analysis opt-in stored?**
   Decision: a `cloud_media_analysis.enabled` marker in the data dir records the opt-in; the screenpipe
   opt-out marker `cloud_media_analysis.disabled` is honoured and written on opt-out. The skill block is
   injected only with the opt-in and a configured gateway. The block was rewritten for a user gateway and
   names no model (the gateway decides which multimodal models exist).
   Reason: cloud-M01 asks for "missing marker means disabled", which needs a positive marker.

7. **Gateway transcription and live notes readiness.**
   Decision: the `screenpipe-cloud` STT engine needs a gateway URL *and* key, otherwise it falls back to
   Whisper with the new `gatewayNotConfigured` reason (the `notLoggedIn`/`notSubscribed` reasons are
   gone). The live relay is an explicit `SCREENPIPE_MEETING_REALTIME_URL` or `{gateway}/realtime` and
   also needs the key; a `screenpipe-cloud` live choice without them falls back to the selected local
   engine. `selected-engine` stays on the service the batch engine already uses: with the gateway engine
   it may use the gateway relay, with the user's own Deepgram engine it may use direct Deepgram live, and
   otherwise (or when that live path is not ready) it keeps the selected engine. It is never moved to a
   different remote service.
   Reason: the existing live and batch clients always send a Bearer credential, and a chat-only gateway
   (for example LiteLLM) has no Deepgram-format `/realtime`.

8. **Web search without a vendor route.**
   Decision: the agent web-search extension calls `{gateway}/web-search` directly (base exported to the
   agent as `SCREENPIPE_AI_GATEWAY_URL`) and is installed only for the gateway provider with a gateway
   configured. No engine `/v1/web-search` route was added.
   Reason: PLAN.md section 5 gives the `web_search` route to a W1-03 stub filled by a later WP;
   registering the path here would collide with it.

9. **Enhanced AI suggestions and the workflow classifier.**
   Decision: enhanced suggestions call the default AI preset (OpenAI-compatible chat; gateway presets go
   through the local proxy; ChatGPT OAuth presets are not used). The classifier starts only when
   `SCREENPIPE_EVENT_CLASSIFIER_URL` is set. Its bearer token is `SCREENPIPE_EVENT_CLASSIFIER_KEY` (new
   variable), else the gateway key only when the classifier URL has the gateway's origin; a gateway key
   is never sent to another host.
   Reason: cloud-016 says both "use the default BYO preset" and "classifier only runs if … is set".

10. **`use-usage-status.tsx` overlaps with W0-05.**
    Decision: this WP removed the hook's only caller (the quota badge in `ai-presets.tsx`) and deleted the
    file, because W0-06 must remove the `/v1/usage` poll. If W0-05 edited the file, resolve the conflict
    by deleting it.

11. **`packages/ai-gateway`.**
    Decision: left in place, documented as not built or shipped (`packages/ai-gateway/FREENPIPE.md`); no
    workspace or workflow referenced it. The vendor guard still reports its hosts; W0-12 should skip the
    directory or the owner can delete it.

12. **What does `/search?filter_pii=true` do without an enclave?**
    Decision: it redacts on the device. The deterministic `screenpipe-redact` regex pass (emails,
    phone numbers, cards, national IDs, connection strings, API-key and private-key shapes, every class
    it detects) runs on every text. When the "AI PII removal" worker has loaded the local ONNX text
    model, the engine registers it with the search filter and it also runs on the regex output, every
    class it detects, cached per text and limited to a 1 s budget per request (texts it does not reach,
    or fails on, keep the regex result). No model is downloaded or loaded at search time, and the OPF
    fallback model is never used here (too slow and too large to load inside a request). This path
    cannot fail. A configured enclave (`SCREENPIPE_PRIVACY_FILTER_ENCLAVE` or `_URL` plus `_REPO`) is
    used instead and still fails closed with 503 `privacy_filter_unavailable` on enclave errors.
    Reason: specs/api.md says "implement locally (`screenpipe-redact` regex/NER)". Failing closed when
    nothing is configured would break the onboarding live feed, every pipe whose manifest sets
    `privacy_filter: true` (DECISIONS #1) and the e2e search contract (no 5xx for `filter_pii=true`).

13. **Where is the gateway key stored?**
    Decision: for now `aiGatewayApiKey` sits in `store.bin` like the existing preset `apiKey`,
    `deepgramApiKey` and `openaiCompatibleApiKey` (encrypted at rest only when `encryptStore` is on;
    the `.last-good` recovery snapshot is plain JSON). It is visible to the webview through the
    settings object and is written into the agent's `~/.pi/agent/models.json`/`auth.json` like other
    provider keys. Follow-up (owner: the W1-02 settings
    schema or a dedicated secrets WP): move every AI provider key, this one included, into
    `screenpipe_secrets::SecretStore` and push it into the engine with `set_cloud_token`.
    Reason: moving only the new key would leave the same exposure for the older keys and split one
    settings page across two stores; the move is a cross-cutting change outside this WP's paths.

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

## W0-13

1. **Do any `screenpipe` organisation pins postdate the 2026-06-10 relicensing?**
   Decision: no. Committer dates of the pinned commits in cargo's git cache: whisper-rs 2026-03-24,
   rusty-tesseract 2024-07-23, cpal 2026-06-05, sck-rs 2026-05-30, opf-rs 2026-05-08. All five
   are vendored; no screenpipe-org git dependency is left.
   Reason: DECISIONS.md decision 2 only excludes revisions dated after the cut-off.

2. **opf-rs (screenpipe/opf-rs) is licensed CC BY-NC 4.0, not under an open-source licence. Vendor
   it?**
   Decision: vendored with its `LICENSE`, `NOTICE` and `LICENSE.upstream-apache2.txt` untouched and
   the licence called out in `third_party/README.md` and in the comment above the dependency in
   `crates/screenpipe-redact/Cargo.toml`. Feature wiring is unchanged: `screenpipe-engine` and the
   app crate still enable `screenpipe-redact/opf-text`, so default builds link it. **Owner action:**
   decide whether `opf-text` should become opt-in or be replaced by a permissively licensed
   redactor, since CC BY-NC grants no commercial use (including, arguably, use inside a company).
   Reason: CC BY-NC allows non-commercial redistribution with attribution, so the copy is lawful;
   vendoring does not change what the binaries already linked through the git dependency; keeping
   the git dependency would leave the build depending on the vendor's repository, which is what
   decision 2 removes. Changing default features is a product decision outside this WP.

3. **sck-rs, vad-rs, audiopipe and the hf-hub fork have no licence file at the pinned revision
   (qwen3-asr.cpp declares MIT only in its README). Is that enough to vendor them?**
   Decision: yes. The `license` field of each `Cargo.toml` (MIT, MIT, MIT, Apache-2.0) and the
   README statement are recorded as the grant; no licence files were written for them.
   Reason: those declarations are the authors' own licence grants, and a generated licence file
   would have to invent a copyright holder.

4. **How lean should the trees be?**
   Decision: keep all Rust sources, examples and tests that are plain text and small; drop VCS and
   CI metadata, standalone `Cargo.lock` files, other projects' agent instruction files
   (`CLAUDE.md`, `AGENTS.md`), Python tooling, media and binary fixtures, demo apps, and from the
   two ggml copies the backends that no Cargo feature of whisper-rs or qwen3-asr-sys can enable.
   whisper.cpp keeps the file set of whisper-rs-sys's own `include` list. Every removal is listed
   in `third_party/README.md` and encoded in `third_party/vendor.sh`, which rebuilds the trees
   byte for byte (`--check`). Result: 17.5 MB of source, about 4 MB compressed in git.
   Reason: the build must keep working for every feature our crates expose, on every platform;
   beyond that, smaller is better. Agent instruction files would be loaded by agents working in
   this repository, so they must not be imported.

5. **qwen3-asr-sys's build script rewrites two ggml-metal sources in place on every macOS Metal
   build. What should happen with a vendored copy?**
   Decision: ship the rewritten files (`third_party/patches/audiopipe/0002-ggml-metal-prepatch.patch`,
   generated by running the build script's own two patch functions). The build script finds its
   markers and leaves the files alone, so macOS builds no longer modify the working tree.
   Reason: identical compiled result, and a build should never dirty the repository.

6. **examples/secret_probe.rs in opf-rs contains fake API keys that GitHub push protection
   blocks.**
   Decision: removed from the copy. Its `[[example]]` entry stays in `Cargo.toml`; cargo does not
   look for example files of a dependency.
   Reason: the push cannot go through otherwise, and nothing in Freenpipe builds that example.

7. **tinfoil-rs (tinfoilsh/tinfoil-rs) is AGPL-3.0 and stays a git dependency. Anything to do?**
   Decision: nothing in W0-13 (it is an established third-party repository, outside decision 2);
   its licence is recorded in `third_party/README.md`. **Owner action:** AGPL-3.0 applies to any
   binary that links it; it is compiled into screenpipe-engine and screenpipe-redact even though
   W0-06 only uses it when the user configures an endpoint. Consider moving it behind an opt-in
   Cargo feature.
   Reason: licence observations from the W0-13 licence check, recorded so they are not lost.
