// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Bring-your-own AI gateway.
//!
//! screenpipe shipped a hosted, OpenAI-compatible gateway and a provider id
//! (`screenpipe-cloud`) that pointed at it. Freenpipe keeps the provider id so
//! existing stores, pipes and skills still load, but the id now resolves to a
//! gateway the *user* runs or chose (LiteLLM, a self-hosted worker, any
//! OpenAI-compatible proxy). With no gateway configured every gateway-only
//! feature is off: nothing falls back to a hosted service.
//!
//! The base URL comes from the endpoint switchboard
//! ([`crate::endpoints::Endpoint::AiGateway`]): the app pushes the
//! `aiGatewayUrl` setting in as an override, and the CLI reads
//! `SCREENPIPE_AI_GATEWAY_URL`. The base is the OpenAI-style root including
//! its version segment (for example `http://127.0.0.1:4000/v1`), exactly like
//! the URL of a custom AI preset; feature paths are appended to it.
//!
//! The credential sent to the gateway is never the local API key. It is the
//! value the app pushes into the engine (the `aiGatewayApiKey` setting) or,
//! for headless runs, `SCREENPIPE_AI_GATEWAY_KEY`.
//!
//! This module also owns the opt-in for "cloud media analysis", the agent
//! skill section that tells agents they may send audio, video frames and
//! images to the gateway. It is off unless the user opted in *and* a gateway
//! is configured.

use std::io;
use std::path::Path;

use crate::endpoints::{self, Endpoint};

/// Settings provider id that resolves to the user's gateway. Kept from
/// screenpipe so stored presets and pipe frontmatter keep working.
pub const PROVIDER_ID: &str = "screenpipe-cloud";

/// Environment variable carrying the gateway credential for headless runs.
pub const KEY_ENV: &str = "SCREENPIPE_AI_GATEWAY_KEY";

/// Environment variable the agent runtime reads to find the gateway base.
/// Same name as the switchboard's environment source, so a child process
/// resolves the same base its parent did.
pub const URL_ENV: &str = "SCREENPIPE_AI_GATEWAY_URL";

/// Error string returned when no gateway is configured. Callers that expose
/// it (the `get_screenpipe_ai_gateway_url` command) hide gateway features.
pub const NOT_CONFIGURED: &str = "not configured";

/// Error code of the local proxy routes when no gateway is configured. The
/// code is kept from screenpipe so pipes and skills that already handle it
/// degrade the same way.
pub const NOT_CONFIGURED_CODE: &str = "cloud_token_missing";

/// The configured gateway base, or `None`.
pub fn base() -> Option<String> {
    endpoints::ai_gateway_base()
}

/// Whether a gateway base is configured.
pub fn is_configured() -> bool {
    base().is_some()
}

/// `{base}/chat/completions`.
pub fn chat_completions_url(base: &str) -> String {
    endpoints::join(base, "chat/completions")
}

/// `{base}/models`.
pub fn models_url(base: &str) -> String {
    endpoints::join(base, "models")
}

/// `{base}/listen` (batch transcription, Deepgram wire format).
pub fn listen_url(base: &str) -> String {
    endpoints::join(base, "listen")
}

/// `{base}/web-search`.
pub fn web_search_url(base: &str) -> String {
    endpoints::join(base, "web-search")
}

/// `{base}/realtime` as a websocket URL (`http` becomes `ws`, `https`
/// becomes `wss`). `None` when the base has an unexpected scheme.
pub fn realtime_url(base: &str) -> Option<String> {
    let base = base.trim().trim_end_matches('/');
    let (scheme, rest) = base.split_once("://")?;
    let ws_scheme = match scheme.to_ascii_lowercase().as_str() {
        "https" | "wss" => "wss",
        "http" | "ws" => "ws",
        _ => return None,
    };
    if rest.is_empty() {
        return None;
    }
    Some(endpoints::join(
        &format!("{ws_scheme}://{rest}"),
        "realtime",
    ))
}

/// The gateway credential: an explicit value when non-empty, else
/// [`KEY_ENV`].
pub fn credential(explicit: Option<&str>) -> Option<String> {
    credential_with(explicit, |name| std::env::var(name).ok())
}

/// Pure form of [`credential`] with the environment supplied by the caller.
pub fn credential_with(
    explicit: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    explicit
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| {
            env(KEY_ENV)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
}

/// Marker file that records the user's opt-in to cloud media analysis.
pub const MEDIA_ANALYSIS_ENABLED_MARKER: &str = "cloud_media_analysis.enabled";

/// Marker file screenpipe writes for an explicit opt-out. Honoured so an
/// opt-out made in screenpipe survives the switch.
pub const MEDIA_ANALYSIS_DISABLED_MARKER: &str = "cloud_media_analysis.disabled";

/// Whether the user opted in to cloud media analysis in `data_dir`. A
/// missing marker means "no": the feature defaults off.
pub fn media_analysis_opted_in(data_dir: &Path) -> bool {
    data_dir.join(MEDIA_ANALYSIS_ENABLED_MARKER).exists()
        && !data_dir.join(MEDIA_ANALYSIS_DISABLED_MARKER).exists()
}

/// Record the user's choice in `data_dir`. Writes one marker and removes
/// the other, so the state is unambiguous for both Freenpipe and screenpipe.
pub fn set_media_analysis_opt_in(data_dir: &Path, enabled: bool) -> io::Result<()> {
    let (write, remove) = if enabled {
        (
            MEDIA_ANALYSIS_ENABLED_MARKER,
            MEDIA_ANALYSIS_DISABLED_MARKER,
        )
    } else {
        (
            MEDIA_ANALYSIS_DISABLED_MARKER,
            MEDIA_ANALYSIS_ENABLED_MARKER,
        )
    };
    std::fs::create_dir_all(data_dir)?;
    let note = if enabled {
        "Freenpipe: agents may send media to your configured AI gateway for analysis.\n"
    } else {
        "Freenpipe: cloud media analysis is off. Agents are not told about it.\n"
    };
    std::fs::write(data_dir.join(write), note)?;
    match std::fs::remove_file(data_dir.join(remove)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Whether agents should be told about cloud media analysis: the user opted
/// in and a gateway is configured.
pub fn media_analysis_active(data_dir: &Path) -> bool {
    media_analysis_active_with(data_dir, is_configured())
}

/// Pure form of [`media_analysis_active`].
pub fn media_analysis_active_with(data_dir: &Path, gateway_configured: bool) -> bool {
    gateway_configured && media_analysis_opted_in(data_dir)
}

/// Push a settings value for the gateway base into the switchboard. An empty
/// or missing value clears the override (the environment still applies).
/// Returns `false` when the value is present but not a usable URL.
pub fn apply_setting(value: Option<&str>) -> bool {
    endpoints::set_override(Endpoint::AiGateway, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_paths_hang_off_the_base() {
        let base = "http://127.0.0.1:4000/v1";
        assert_eq!(
            chat_completions_url(base),
            "http://127.0.0.1:4000/v1/chat/completions"
        );
        assert_eq!(models_url(base), "http://127.0.0.1:4000/v1/models");
        assert_eq!(listen_url(base), "http://127.0.0.1:4000/v1/listen");
        assert_eq!(web_search_url(base), "http://127.0.0.1:4000/v1/web-search");
        assert_eq!(
            chat_completions_url("https://gw.example.test/v1/"),
            "https://gw.example.test/v1/chat/completions"
        );
    }

    #[test]
    fn realtime_url_switches_to_websocket_schemes() {
        assert_eq!(
            realtime_url("https://gw.example.test/v1").as_deref(),
            Some("wss://gw.example.test/v1/realtime")
        );
        assert_eq!(
            realtime_url("http://127.0.0.1:4000/v1/").as_deref(),
            Some("ws://127.0.0.1:4000/v1/realtime")
        );
        assert_eq!(
            realtime_url("wss://relay.example.test").as_deref(),
            Some("wss://relay.example.test/realtime")
        );
        assert_eq!(realtime_url("ftp://x.example.test"), None);
        assert_eq!(realtime_url("https://"), None);
        assert_eq!(realtime_url("not a url"), None);
    }

    #[test]
    fn credential_prefers_explicit_then_env_and_ignores_blanks() {
        let env = |name: &str| (name == KEY_ENV).then(|| "env-key".to_string());
        assert_eq!(
            credential_with(Some(" explicit "), env).as_deref(),
            Some("explicit")
        );
        assert_eq!(credential_with(Some("  "), env).as_deref(), Some("env-key"));
        assert_eq!(credential_with(None, env).as_deref(), Some("env-key"));
        assert_eq!(credential_with(None, |_| Some("   ".to_string())), None);
        assert_eq!(credential_with(None, |_| None), None);
    }

    #[test]
    fn credential_never_reads_the_local_api_key() {
        let env = |name: &str| match name {
            "SCREENPIPE_API_KEY" | "SCREENPIPE_LOCAL_API_KEY" => Some("local".to_string()),
            _ => None,
        };
        assert_eq!(credential_with(None, env), None);
    }

    #[test]
    fn media_analysis_is_off_until_opted_in_and_configured() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert!(!media_analysis_opted_in(d), "missing marker means off");
        assert!(!media_analysis_active_with(d, true));

        set_media_analysis_opt_in(d, true).unwrap();
        assert!(media_analysis_opted_in(d));
        assert!(d.join(MEDIA_ANALYSIS_ENABLED_MARKER).exists());
        assert!(!d.join(MEDIA_ANALYSIS_DISABLED_MARKER).exists());
        assert!(
            !media_analysis_active_with(d, false),
            "no gateway, no block"
        );
        assert!(media_analysis_active_with(d, true));

        set_media_analysis_opt_in(d, false).unwrap();
        assert!(!media_analysis_opted_in(d));
        assert!(d.join(MEDIA_ANALYSIS_DISABLED_MARKER).exists());
        assert!(!d.join(MEDIA_ANALYSIS_ENABLED_MARKER).exists());
    }

    #[test]
    fn a_screenpipe_opt_out_marker_wins() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join(MEDIA_ANALYSIS_ENABLED_MARKER), "").unwrap();
        std::fs::write(d.join(MEDIA_ANALYSIS_DISABLED_MARKER), "").unwrap();
        assert!(!media_analysis_opted_in(d));
    }
}
