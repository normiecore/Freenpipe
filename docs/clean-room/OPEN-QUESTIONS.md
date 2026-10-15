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
    Decision: it redacts on the device with the deterministic `screenpipe-redact` regex pass (emails,
    phone numbers, cards, national IDs, connection strings, API-key and private-key shapes, every class
    it detects). The local AI model is not loaded at search time; with "AI PII removal" on, the
    background worker has already rewritten stored text with it. A configured enclave
    (`SCREENPIPE_PRIVACY_FILTER_ENCLAVE` or `_URL` plus `_REPO`) is used instead and still fails closed
    with 503 `privacy_filter_unavailable` on enclave errors.
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
