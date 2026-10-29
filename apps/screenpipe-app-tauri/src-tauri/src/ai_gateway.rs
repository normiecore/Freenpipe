// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Bring-your-own remote AI endpoints, as seen by the desktop app.
//!
//! The `screenpipe-cloud` provider, the cloud transcription engine, cloud
//! media analysis and the agent web-search tool all talk to one
//! OpenAI-compatible gateway that the user chose (`aiGatewayUrl` and
//! `aiGatewayApiKey` settings, or `SCREENPIPE_AI_GATEWAY_URL` /
//! `SCREENPIPE_AI_GATEWAY_KEY`). Nothing here has a hosted default.
//!
//! The engine runs inside this process, so the app hands the configured base
//! to the shared endpoint switchboard (`screenpipe_core::endpoints`) and the
//! engine's routes, transcription and agents read it from there. The
//! settings are re-applied on every engine start and whenever the frontend
//! asks for the gateway URL, so a change takes effect without a restart.
//!
//! The PII-redaction enclave is the other bring-your-own remote endpoint;
//! its status command lives here too.

use tauri::AppHandle;
use tracing::warn;

use crate::store::SettingsStore;

/// Push the stored gateway URL into the endpoint switchboard. An empty or
/// invalid value clears the override (environment variables still apply).
/// Returns the base the engine will use, if any.
pub fn apply_settings(store: &SettingsStore) -> Option<String> {
    let value = store.ai_gateway_url.trim();
    if !screenpipe_core::ai_gateway::apply_setting(Some(value)) {
        warn!(
            "ignoring aiGatewayUrl setting {:?}: expected an http(s) URL",
            value
        );
        screenpipe_core::ai_gateway::apply_setting(None);
    }
    screenpipe_core::ai_gateway::base()
}

/// Gateway base and credential for callers inside the app (the chat agent,
/// suggestions). `None` when no gateway is configured.
pub struct GatewayAccess {
    pub base: String,
    pub key: Option<String>,
}

/// Resolve the gateway from the current settings (re-applying them first).
pub fn gateway_access(app: &AppHandle) -> Option<GatewayAccess> {
    let store = SettingsStore::get(app).ok().flatten();
    if let Some(ref store) = store {
        apply_settings(store);
    }
    let base = screenpipe_core::ai_gateway::base()?;
    let key = screenpipe_core::ai_gateway::credential(
        store.as_ref().map(|s| s.ai_gateway_api_key.as_str()),
    );
    Some(GatewayAccess { base, key })
}

/// Base URL of the user's AI gateway (the `aiGatewayUrl` setting, else
/// `SCREENPIPE_AI_GATEWAY_URL`). Errors with "not configured" when there is
/// none, so callers hide gateway-only features. Calling it also re-applies
/// the stored setting to the running engine.
#[tauri::command]
#[specta::specta]
pub fn get_screenpipe_ai_gateway_url(app: AppHandle) -> Result<String, String> {
    if let Ok(Some(store)) = SettingsStore::get(&app) {
        apply_settings(&store);
    }
    screenpipe_core::ai_gateway::base()
        .ok_or_else(|| screenpipe_core::ai_gateway::NOT_CONFIGURED.to_string())
}

/// Whether an AI gateway credential is available to the engine: the
/// `aiGatewayApiKey` setting, else `SCREENPIPE_AI_GATEWAY_KEY`. The key itself
/// is never returned. Gateway transcription needs this and a gateway URL.
#[tauri::command]
#[specta::specta]
pub fn get_ai_gateway_key_configured(app: AppHandle) -> bool {
    let store = SettingsStore::get(&app).ok().flatten();
    screenpipe_core::ai_gateway::credential(store.as_ref().map(|s| s.ai_gateway_api_key.as_str()))
        .is_some()
}

/// Whether a remote PII-redaction enclave is fully configured: an enclave
/// host from the endpoint switchboard (`SCREENPIPE_PRIVACY_FILTER_ENCLAVE` or
/// `_URL`), plus `SCREENPIPE_PRIVACY_FILTER_REPO` and `_API_KEY`. The
/// settings page offers the enclave backend only when this is true.
#[tauri::command]
#[specta::specta]
pub fn get_privacy_filter_enclave_configured() -> bool {
    // Also false in a build without the opt-in `tinfoil-enclave` feature
    // (the enclave client is AGPL-3.0 and left out of default and release
    // builds), so such a build never offers the enclave backend.
    screenpipe_redact::adapters::tinfoil::is_available_for(
        screenpipe_core::endpoints::privacy_filter_base(),
    )
}
