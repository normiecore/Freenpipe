// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Endpoint switchboard: the single place that decides which remote base URL
//! a feature may talk to.
//!
//! Every endpoint starts out **unset**. A lookup returns `None` unless the
//! user configured it, either through a settings value that the app pushes in
//! with [`set_override`] or through the process environment. There is no
//! built-in hosted default anywhere in this module, and there must never be
//! one: a `None` means "the user has not pointed this feature anywhere".
//!
//! What a caller does with `None` depends on the endpoint (see the variant
//! docs). For services that only ever existed as a vendor-hosted backend
//! (website, AI gateway, pipe registry, classifier, realtime relay, privacy
//! enclave, updater feed) the feature is disabled and the UI should say "not
//! configured" instead of retrying. For third-party services that the user
//! reaches with their own account (the public model hub), `None` simply means
//! "no mirror": the caller keeps using the provider's public host.
//!
//! Resolution order: settings override, then the endpoint's environment
//! variables in the listed order, then `None`. Values are trimmed, trailing
//! slashes are removed, and anything that is not an `http(s)://` or
//! `ws(s)://` URL is rejected (with a warning) so a typo cannot silently
//! send traffic somewhere unexpected.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use tracing::warn;

/// Canonical home of the project, used for user-facing links only.
pub const FREENPIPE_REPO_URL: &str = "https://github.com/normiecore/Freenpipe";

/// A remote service Freenpipe can be pointed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Endpoint {
    /// Companion website (account pages, web links). Unset: hide the links.
    Web,
    /// Pipe registry / store API. Unset: registry browsing and publishing are
    /// unavailable; locally installed pipes keep working.
    PipeRegistry,
    /// OpenAI-compatible gateway used by the hosted-AI provider, cloud media
    /// analysis and the web-search proxy. Unset: those features are off.
    AiGateway,
    /// Workflow event classifier service. Unset: classification is skipped.
    EventClassifier,
    /// Realtime meeting-transcription relay (websocket). Unset: the relay
    /// provider is unavailable; local and bring-your-own providers still work.
    MeetingRealtime,
    /// Remote privacy-filter (PII) enclave. Accepts a bare host or a URL.
    /// Unset: only the local redactor is used.
    PrivacyFilter,
    /// Hugging Face compatible model mirror. Unset: the public hub is used.
    ModelMirror,
    /// App-updater manifest URL. May contain a `{channel}` placeholder.
    /// Unset: update checks are off.
    UpdaterManifest,
}

impl Endpoint {
    /// Every endpoint, for diagnostics and tests.
    pub const ALL: [Endpoint; 8] = [
        Endpoint::Web,
        Endpoint::PipeRegistry,
        Endpoint::AiGateway,
        Endpoint::EventClassifier,
        Endpoint::MeetingRealtime,
        Endpoint::PrivacyFilter,
        Endpoint::ModelMirror,
        Endpoint::UpdaterManifest,
    ];

    /// Environment variables consulted for this endpoint, highest priority
    /// first. The names match the ones existing scripts and pipes already use.
    pub fn env_vars(self) -> &'static [&'static str] {
        match self {
            Endpoint::Web => &["SCREENPIPE_WEB_URL"],
            Endpoint::PipeRegistry => &["SCREENPIPE_API_BASE_URL"],
            Endpoint::AiGateway => &["SCREENPIPE_AI_GATEWAY_URL"],
            Endpoint::EventClassifier => &["SCREENPIPE_EVENT_CLASSIFIER_URL"],
            Endpoint::MeetingRealtime => &["SCREENPIPE_MEETING_REALTIME_URL"],
            Endpoint::PrivacyFilter => &[
                "SCREENPIPE_PRIVACY_FILTER_ENCLAVE",
                "SCREENPIPE_PRIVACY_FILTER_URL",
            ],
            Endpoint::ModelMirror => &["HF_ENDPOINT"],
            Endpoint::UpdaterManifest => &["SCREENPIPE_UPDATER_MANIFEST_URL"],
        }
    }

    /// Short stable name, used in logs.
    pub fn name(self) -> &'static str {
        match self {
            Endpoint::Web => "web",
            Endpoint::PipeRegistry => "pipe_registry",
            Endpoint::AiGateway => "ai_gateway",
            Endpoint::EventClassifier => "event_classifier",
            Endpoint::MeetingRealtime => "meeting_realtime",
            Endpoint::PrivacyFilter => "privacy_filter",
            Endpoint::ModelMirror => "model_mirror",
            Endpoint::UpdaterManifest => "updater_manifest",
        }
    }

    /// Whether a bare host name (no scheme) is an acceptable value.
    fn accepts_bare_host(self) -> bool {
        matches!(self, Endpoint::PrivacyFilter)
    }
}

fn overrides() -> &'static RwLock<HashMap<Endpoint, String>> {
    static OVERRIDES: OnceLock<RwLock<HashMap<Endpoint, String>>> = OnceLock::new();
    OVERRIDES.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Install (or clear, with `None` or an empty string) the settings-provided
/// value for an endpoint. Settings win over environment variables. Returns
/// `false`, and leaves the previous value in place, when the value is
/// present but not a usable URL.
pub fn set_override(endpoint: Endpoint, value: Option<&str>) -> bool {
    let normalized = match value.map(str::trim).filter(|v| !v.is_empty()) {
        None => None,
        Some(raw) => match normalize(endpoint, raw) {
            Some(v) => Some(v),
            None => return false,
        },
    };
    let mut map = overrides().write().unwrap_or_else(|e| e.into_inner());
    match normalized {
        Some(v) => {
            map.insert(endpoint, v);
        }
        None => {
            map.remove(&endpoint);
        }
    }
    true
}

/// Resolve an endpoint from the settings override and the environment.
pub fn resolve(endpoint: Endpoint) -> Option<String> {
    let current = overrides()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&endpoint)
        .cloned();
    resolve_with(endpoint, current.as_deref(), |name| {
        std::env::var(name).ok()
    })
}

/// Pure resolution step behind [`resolve`], with the override and the
/// environment supplied by the caller (used by tests and diagnostics).
pub fn resolve_with(
    endpoint: Endpoint,
    override_value: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    if let Some(v) = override_value.and_then(|raw| normalize(endpoint, raw)) {
        return Some(v);
    }
    endpoint
        .env_vars()
        .iter()
        .filter_map(|name| env(name))
        .filter(|raw| !raw.trim().is_empty())
        .find_map(|raw| normalize(endpoint, &raw))
}

/// Trim, strip trailing slashes and check the scheme. `None` when unusable.
fn normalize(endpoint: Endpoint, raw: &str) -> Option<String> {
    let value = raw.trim().trim_end_matches('/');
    if value.is_empty() {
        return None;
    }
    if value.chars().any(char::is_whitespace) {
        warn!(
            "endpoint {}: ignoring value with whitespace: {:?}",
            endpoint.name(),
            raw
        );
        return None;
    }
    match value.split_once("://") {
        Some((scheme, rest)) => {
            let scheme_ok = matches!(
                scheme.to_ascii_lowercase().as_str(),
                "http" | "https" | "ws" | "wss"
            );
            if scheme_ok && !rest.is_empty() {
                Some(value.to_string())
            } else {
                warn!(
                    "endpoint {}: ignoring unsupported URL {:?} (need http, https, ws or wss)",
                    endpoint.name(),
                    raw
                );
                None
            }
        }
        None if endpoint.accepts_bare_host() => Some(value.to_string()),
        None => {
            warn!(
                "endpoint {}: ignoring {:?}, expected an absolute URL",
                endpoint.name(),
                raw
            );
            None
        }
    }
}

/// Append `path` to `base` with exactly one slash between them.
pub fn join(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        base.to_string()
    } else {
        format!("{base}/{path}")
    }
}

/// Resolve an endpoint and append a path to it. `None` when unset.
pub fn url(endpoint: Endpoint, path: &str) -> Option<String> {
    resolve(endpoint).map(|base| join(&base, path))
}

/// Companion website base. `None` unless configured.
pub fn web_base() -> Option<String> {
    resolve(Endpoint::Web)
}

/// OpenAI-compatible gateway base. `None` unless configured.
pub fn ai_gateway_base() -> Option<String> {
    resolve(Endpoint::AiGateway)
}

/// Pipe registry base. `None` unless configured.
pub fn pipe_registry_base() -> Option<String> {
    resolve(Endpoint::PipeRegistry)
}

/// Workflow event classifier URL. `None` unless configured.
pub fn event_classifier_url() -> Option<String> {
    resolve(Endpoint::EventClassifier)
}

/// Realtime meeting-transcription relay URL. `None` unless configured.
pub fn meeting_realtime_url() -> Option<String> {
    resolve(Endpoint::MeetingRealtime)
}

/// Remote privacy-filter enclave (host or URL). `None` unless configured.
pub fn privacy_filter_base() -> Option<String> {
    resolve(Endpoint::PrivacyFilter)
}

/// Model-hub mirror base. `None` means "use the public hub".
pub fn model_mirror_base() -> Option<String> {
    resolve(Endpoint::ModelMirror)
}

/// Updater manifest URL for a release channel. `None` (updates off) unless
/// configured. A `{channel}` placeholder in the configured value is replaced
/// with `channel`.
pub fn updater_manifest_url(channel: &str) -> Option<String> {
    resolve(Endpoint::UpdaterManifest).map(|template| template.replace("{channel}", channel))
}

/// User-facing project links. These open in the browser; nothing is fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreenpipeLink {
    Repo,
    Releases,
    LatestRelease,
    Issues,
    NewIssue,
    Discussions,
    Docs,
}

/// Build a user-facing link into the Freenpipe repository.
pub fn freenpipe_link(kind: FreenpipeLink) -> String {
    let suffix = match kind {
        FreenpipeLink::Repo => "",
        FreenpipeLink::Releases => "/releases",
        FreenpipeLink::LatestRelease => "/releases/latest",
        FreenpipeLink::Issues => "/issues",
        FreenpipeLink::NewIssue => "/issues/new",
        FreenpipeLink::Discussions => "/discussions",
        FreenpipeLink::Docs => "/tree/main/docs",
    };
    format!("{FREENPIPE_REPO_URL}{suffix}")
}

/// Outbound User-Agent for HTTP clients that want one, e.g.
/// `user_agent("freenpipe-app", env!("CARGO_PKG_VERSION"))`.
pub fn user_agent(product: &str, version: &str) -> String {
    format!("{product}/{version} (+{FREENPIPE_REPO_URL})")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn every_endpoint_is_unset_by_default() {
        for endpoint in Endpoint::ALL {
            assert_eq!(resolve_with(endpoint, None, no_env), None, "{endpoint:?}");
        }
    }

    #[test]
    fn env_value_is_trimmed_and_loses_trailing_slashes() {
        let env = |name: &str| {
            (name == "SCREENPIPE_AI_GATEWAY_URL").then(|| "  http://127.0.0.1:4000/v1// ".into())
        };
        assert_eq!(
            resolve_with(Endpoint::AiGateway, None, env).as_deref(),
            Some("http://127.0.0.1:4000/v1")
        );
    }

    #[test]
    fn override_beats_env() {
        let env = |_: &str| Some("https://env.example.test".to_string());
        assert_eq!(
            resolve_with(Endpoint::Web, Some("https://settings.example.test/"), env).as_deref(),
            Some("https://settings.example.test")
        );
    }

    #[test]
    fn invalid_override_falls_through_to_env() {
        let env = |_: &str| Some("https://env.example.test".to_string());
        assert_eq!(
            resolve_with(Endpoint::Web, Some("ftp://nope"), env).as_deref(),
            Some("https://env.example.test")
        );
    }

    #[test]
    fn blank_and_unsupported_values_are_rejected() {
        for bad in [
            "",
            "   ",
            "localhost:3000",
            "file:///etc/passwd",
            "https://",
            "http://a b",
        ] {
            let env = move |_: &str| Some(bad.to_string());
            assert_eq!(
                resolve_with(Endpoint::PipeRegistry, None, env),
                None,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn websocket_schemes_are_accepted() {
        let env = |_: &str| Some("wss://relay.example.test/v1/realtime".to_string());
        assert_eq!(
            resolve_with(Endpoint::MeetingRealtime, None, env).as_deref(),
            Some("wss://relay.example.test/v1/realtime")
        );
    }

    #[test]
    fn privacy_filter_takes_a_bare_host_and_prefers_the_enclave_var() {
        let env = |name: &str| match name {
            "SCREENPIPE_PRIVACY_FILTER_ENCLAVE" => Some("enclave.example.test".to_string()),
            "SCREENPIPE_PRIVACY_FILTER_URL" => Some("https://legacy.example.test".to_string()),
            _ => None,
        };
        assert_eq!(
            resolve_with(Endpoint::PrivacyFilter, None, env).as_deref(),
            Some("enclave.example.test")
        );
        let legacy_only = |name: &str| {
            (name == "SCREENPIPE_PRIVACY_FILTER_URL").then(|| "https://legacy.example.test".into())
        };
        assert_eq!(
            resolve_with(Endpoint::PrivacyFilter, None, legacy_only).as_deref(),
            Some("https://legacy.example.test")
        );
    }

    #[test]
    fn an_unusable_first_env_var_does_not_hide_a_valid_second_one() {
        let env = |name: &str| match name {
            "SCREENPIPE_PRIVACY_FILTER_ENCLAVE" => Some("ftp://bad.example.test".to_string()),
            "SCREENPIPE_PRIVACY_FILTER_URL" => Some("https://ok.example.test".to_string()),
            _ => None,
        };
        assert_eq!(
            resolve_with(Endpoint::PrivacyFilter, None, env).as_deref(),
            Some("https://ok.example.test")
        );
    }

    #[test]
    fn join_uses_exactly_one_slash() {
        assert_eq!(join("http://h/v1/", "/models"), "http://h/v1/models");
        assert_eq!(join("http://h/v1", "models"), "http://h/v1/models");
        assert_eq!(join("http://h", ""), "http://h");
    }

    #[test]
    fn settings_override_round_trip() {
        // The only test that touches the process-wide table; it uses an
        // endpoint no other test reads through `resolve`.
        assert!(!set_override(Endpoint::UpdaterManifest, Some("not a url")));
        assert!(set_override(
            Endpoint::UpdaterManifest,
            Some("https://updates.example.test/{channel}/latest.json")
        ));
        assert_eq!(
            updater_manifest_url("stable").as_deref(),
            Some("https://updates.example.test/stable/latest.json")
        );
        assert!(set_override(Endpoint::UpdaterManifest, None));
        assert!(
            !overrides()
                .read()
                .unwrap()
                .contains_key(&Endpoint::UpdaterManifest),
            "clearing the override must leave nothing behind"
        );
    }

    #[test]
    fn links_and_user_agent_point_at_the_project() {
        assert_eq!(freenpipe_link(FreenpipeLink::Repo), FREENPIPE_REPO_URL);
        assert!(freenpipe_link(FreenpipeLink::Releases).ends_with("/releases"));
        let ua = user_agent("freenpipe-app", "1.2.3");
        assert!(ua.starts_with("freenpipe-app/1.2.3 "));
        assert!(ua.contains(FREENPIPE_REPO_URL));
    }

    #[test]
    fn env_var_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for endpoint in Endpoint::ALL {
            for name in endpoint.env_vars() {
                assert!(seen.insert(*name), "{name} is mapped twice");
            }
        }
    }
}
