# Open questions

Spec ambiguities decided while implementing a work package. Each entry: the question, the decision
taken, and why. The owner can overturn any of them.

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
   for a power-user feature.

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
   engine, and `selected-engine` is never promoted to a remote provider.
   Reason: the existing live and batch clients always send a Bearer credential.

8. **Web search without a vendor route.**
   Decision: the agent web-search extension calls `{gateway}/web-search` directly (base exported to the
   agent as `SCREENPIPE_AI_GATEWAY_URL`) and is installed only for the gateway provider with a gateway
   configured. No engine `/v1/web-search` route was added.
   Reason: PLAN.md section 5 gives the `web_search` route to a W1-03 stub filled by a later WP;
   registering the path here would collide with it.

9. **Enhanced AI suggestions and the workflow classifier.**
   Decision: enhanced suggestions call the default AI preset (OpenAI-compatible chat; gateway presets go
   through the local proxy; ChatGPT OAuth presets are not used). The classifier starts only when
   `SCREENPIPE_EVENT_CLASSIFIER_URL` is set.
   Reason: cloud-016 says both "use the default BYO preset" and "classifier only runs if … is set".

10. **`use-usage-status.tsx` overlaps with W0-05.**
    Decision: this WP removed the hook's only caller (the quota badge in `ai-presets.tsx`) and deleted the
    file, because W0-06 must remove the `/v1/usage` poll. If W0-05 edited the file, resolve the conflict
    by deleting it.

11. **`packages/ai-gateway`.**
    Decision: left in place, documented as not built or shipped (`packages/ai-gateway/FREENPIPE.md`); no
    workspace or workflow referenced it. The vendor guard still reports its hosts; W0-12 should skip the
    directory or the owner can delete it.
