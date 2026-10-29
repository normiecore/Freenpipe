// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Bring-your-own web search: which provider the engine's `/v1/web-search`
//! route talks to.
//!
//! screenpipe sent agent web searches to its hosted API. Handvel has no
//! hosted search and no default provider. A search provider exists only when
//! the user configured one, and the engine route, the `sp_web_search` agent
//! tool and its Pi extension are all off otherwise.
//!
//! Sources, first match wins:
//!
//! 1. [`PROVIDER_ENV`] names the provider explicitly: `searxng`, `brave`,
//!    `tavily`, `gateway`, or `off` to disable web search even when something
//!    else is configured.
//! 2. With no provider named, [`URL_ENV`] alone means a SearXNG instance.
//! 3. Otherwise web search is off.
//!
//! The user's AI gateway ([`crate::ai_gateway`]) serves searches at
//! `{gateway}/web-search` only when [`PROVIDER_ENV`] is `gateway`. A
//! configured gateway alone does not turn search on: most self-hosted
//! gateways (LiteLLM, OpenRouter and the like) have no such route, and a
//! run on a local model must not start sending agent-written queries to a
//! remote host without an explicit opt-in.
//!
//! `brave` and `tavily` need [`API_KEY_ENV`]; their public API hosts are used
//! unless [`URL_ENV`] points somewhere else (a proxy, a test server). The
//! SearXNG instance gets the key as a bearer token when one is set, for
//! instances behind an authenticating proxy.
//!
//! The settings schema has no web-search keys yet, so configuration is
//! environment-only (see `docs/clean-room/open-questions/W3-25.md`).

use std::fmt;

use crate::env;

/// Names the provider: `searxng`, `brave`, `tavily`, `gateway` or `off`.
pub const PROVIDER_ENV: &str = env::SCREENPIPE_WEB_SEARCH_PROVIDER;
/// Base URL of the SearXNG instance, or another base for an API provider.
pub const URL_ENV: &str = env::SCREENPIPE_WEB_SEARCH_URL;
/// API key for `brave` or `tavily` (optional bearer token for SearXNG).
pub const API_KEY_ENV: &str = env::SCREENPIPE_WEB_SEARCH_API_KEY;

/// Error code of `/v1/web-search` when no provider is configured.
pub const NOT_CONFIGURED_CODE: &str = "web_search_not_configured";

/// Public endpoint of the Brave Search web API, used only for `brave`.
pub const BRAVE_DEFAULT_URL: &str = "https://api.search.brave.com/res/v1/web/search";
/// Public endpoint of the Tavily search API, used only for `tavily`.
pub const TAVILY_DEFAULT_URL: &str = "https://api.tavily.com/search";

/// A search backend the engine knows how to call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    /// A SearXNG instance's JSON API (`{url}/search?format=json`).
    SearXng,
    /// The Brave Search web API, with the user's subscription token.
    Brave,
    /// The Tavily search API, with the user's key.
    Tavily,
    /// The user's AI gateway (`{gateway}/web-search`), which already answers
    /// in the route's own `{content, sources}` shape.
    Gateway,
}

impl ProviderKind {
    /// Every provider, for diagnostics and tests.
    pub const ALL: [ProviderKind; 4] = [
        ProviderKind::SearXng,
        ProviderKind::Brave,
        ProviderKind::Tavily,
        ProviderKind::Gateway,
    ];

    /// The value [`PROVIDER_ENV`] uses for this provider.
    pub fn id(self) -> &'static str {
        match self {
            ProviderKind::SearXng => "searxng",
            ProviderKind::Brave => "brave",
            ProviderKind::Tavily => "tavily",
            ProviderKind::Gateway => "gateway",
        }
    }

    /// Parse a [`PROVIDER_ENV`] value (case and surrounding space ignored).
    pub fn parse(value: &str) -> Option<ProviderKind> {
        let value = value.trim().to_ascii_lowercase();
        ProviderKind::ALL.into_iter().find(|p| p.id() == value)
    }

    /// Whether the provider refuses to work without an API key.
    pub fn needs_key(self) -> bool {
        matches!(self, ProviderKind::Brave | ProviderKind::Tavily)
    }

    /// The provider's own public API endpoint, for the providers that have
    /// one. SearXNG and the gateway are always user-supplied.
    pub fn public_url(self) -> Option<&'static str> {
        match self {
            ProviderKind::Brave => Some(BRAVE_DEFAULT_URL),
            ProviderKind::Tavily => Some(TAVILY_DEFAULT_URL),
            ProviderKind::SearXng | ProviderKind::Gateway => None,
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// A usable web-search configuration.
#[derive(Clone, PartialEq, Eq)]
pub struct WebSearchConfig {
    pub provider: ProviderKind,
    /// Base URL, without a trailing slash. For SearXNG it is the instance
    /// root; for Brave and Tavily the full search endpoint; for the gateway
    /// its OpenAI-style root.
    pub url: String,
    /// The provider key. Never set for [`ProviderKind::Gateway`]: the engine
    /// attaches the gateway credential itself.
    pub api_key: Option<String>,
}

impl fmt::Debug for WebSearchConfig {
    // Keeps the key out of logs and panic messages.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebSearchConfig")
            .field("provider", &self.provider)
            .field("url", &self.url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Why there is no usable configuration. [`ConfigProblem::message`] is
/// written for the person reading the route's error or the agent's tool
/// result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigProblem {
    /// Nothing is configured.
    NotConfigured,
    /// The user turned web search off.
    Disabled,
    /// [`PROVIDER_ENV`] holds a name this build does not know.
    UnknownProvider(String),
    /// The provider needs a URL and none (or no valid one) is set.
    MissingUrl(ProviderKind),
    /// The provider needs a key and none is set.
    MissingKey(ProviderKind),
    /// [`URL_ENV`] is set but is not an http(s) URL.
    InvalidUrl,
    /// A key is set but nothing says which API it belongs to.
    KeyWithoutProvider,
}

impl ConfigProblem {
    /// A short explanation with the fix.
    pub fn message(&self) -> String {
        match self {
            ConfigProblem::NotConfigured => format!(
                "web search is not configured. Point {URL_ENV} at a SearXNG instance, or set \
                 {PROVIDER_ENV} to brave or tavily together with {API_KEY_ENV}, then restart \
                 Handvel. {PROVIDER_ENV}=gateway uses an AI gateway that serves /web-search."
            ),
            ConfigProblem::Disabled => {
                format!("web search is turned off ({PROVIDER_ENV}=off).")
            }
            ConfigProblem::UnknownProvider(name) => format!(
                "web search provider {name:?} in {PROVIDER_ENV} is not supported; use searxng, \
                 brave, tavily, gateway or off."
            ),
            ConfigProblem::MissingUrl(ProviderKind::Gateway) => format!(
                "{PROVIDER_ENV}=gateway needs an AI gateway URL (the aiGatewayUrl setting or \
                 SCREENPIPE_AI_GATEWAY_URL)."
            ),
            ConfigProblem::MissingUrl(provider) => {
                format!("{PROVIDER_ENV}={provider} needs {URL_ENV} set to the instance URL.")
            }
            ConfigProblem::MissingKey(provider) => {
                format!("{PROVIDER_ENV}={provider} needs {API_KEY_ENV} set to your {provider} key.")
            }
            ConfigProblem::InvalidUrl => {
                format!("{URL_ENV} must be an http:// or https:// URL.")
            }
            ConfigProblem::KeyWithoutProvider => format!(
                "{API_KEY_ENV} is set but {PROVIDER_ENV} does not say which service it is for; \
                 set it to brave or tavily."
            ),
        }
    }
}

impl fmt::Display for ConfigProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for ConfigProblem {}

/// Resolve the configuration from the process environment and the AI
/// gateway switchboard.
pub fn resolve() -> Result<WebSearchConfig, ConfigProblem> {
    resolve_with(|name| std::env::var(name).ok(), crate::ai_gateway::base())
}

/// Whether the engine has a usable web-search provider. Gates the agent
/// tool: with `false` it is not offered at all.
pub fn is_configured() -> bool {
    resolve().is_ok()
}

/// Pure form of [`resolve`]: the environment and the gateway base are
/// supplied by the caller. The gateway base is used only when
/// [`PROVIDER_ENV`] is `gateway`.
pub fn resolve_with(
    env: impl Fn(&str) -> Option<String>,
    gateway_base: Option<String>,
) -> Result<WebSearchConfig, ConfigProblem> {
    let read = |name: &str| {
        env(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let provider_name = read(PROVIDER_ENV);
    let raw_url = read(URL_ENV);
    let api_key = read(API_KEY_ENV);
    let url = match raw_url.as_deref() {
        Some(raw) => Some(normalize_url(raw).ok_or(ConfigProblem::InvalidUrl)?),
        None => None,
    };
    let gateway_base = gateway_base
        .map(|b| b.trim().trim_end_matches('/').to_string())
        .filter(|b| !b.is_empty());

    let provider = match provider_name.as_deref() {
        Some(name) if is_off(name) => return Err(ConfigProblem::Disabled),
        Some(name) => {
            ProviderKind::parse(name).ok_or_else(|| ConfigProblem::UnknownProvider(name.into()))?
        }
        None if url.is_some() => ProviderKind::SearXng,
        None if api_key.is_some() => return Err(ConfigProblem::KeyWithoutProvider),
        // A configured AI gateway alone is not a search opt-in.
        None => return Err(ConfigProblem::NotConfigured),
    };

    match provider {
        ProviderKind::SearXng => Ok(WebSearchConfig {
            provider,
            url: url
                .map(|u| strip_searxng_search_path(&u))
                .ok_or(ConfigProblem::MissingUrl(provider))?,
            api_key,
        }),
        ProviderKind::Brave | ProviderKind::Tavily => {
            let api_key = api_key.ok_or(ConfigProblem::MissingKey(provider))?;
            let url = url
                .or_else(|| provider.public_url().map(str::to_string))
                .ok_or(ConfigProblem::MissingUrl(provider))?;
            Ok(WebSearchConfig {
                provider,
                url,
                api_key: Some(api_key),
            })
        }
        ProviderKind::Gateway => Ok(WebSearchConfig {
            provider,
            url: gateway_base.ok_or(ConfigProblem::MissingUrl(provider))?,
            api_key: None,
        }),
    }
}

fn is_off(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "off" | "none" | "disabled" | "false" | "0"
    )
}

/// Trim, drop trailing slashes and accept only `http(s)://host...` without
/// whitespace. `None` when unusable.
pub fn normalize_url(raw: &str) -> Option<String> {
    let value = raw.trim().trim_end_matches('/');
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return None;
    }
    let (scheme, rest) = value.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.starts_with('@') || host.ends_with('@') {
        return None;
    }
    Some(value.to_string())
}

/// Users often paste the instance's search page (`https://searx.example/search`)
/// rather than its root. The JSON API lives at `{root}/search`, so drop it.
fn strip_searxng_search_path(url: &str) -> String {
    url.strip_suffix("/search")
        .filter(|root| root.contains("://") && !root.ends_with(':') && !root.ends_with('/'))
        .unwrap_or(url)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn nothing_configured_is_off() {
        assert_eq!(
            resolve_with(env_of(&[]), None),
            Err(ConfigProblem::NotConfigured)
        );
        // Blank values count as unset.
        assert_eq!(
            resolve_with(env_of(&[(URL_ENV, "  "), (PROVIDER_ENV, "")]), None),
            Err(ConfigProblem::NotConfigured)
        );
    }

    #[test]
    fn a_url_alone_means_searxng() {
        let cfg = resolve_with(env_of(&[(URL_ENV, " http://127.0.0.1:8888/ ")]), None).unwrap();
        assert_eq!(cfg.provider, ProviderKind::SearXng);
        assert_eq!(cfg.url, "http://127.0.0.1:8888");
        assert_eq!(cfg.api_key, None);
    }

    #[test]
    fn a_pasted_searxng_search_page_resolves_to_its_root() {
        let cfg = resolve_with(
            env_of(&[(URL_ENV, "https://searx.example.test/search")]),
            None,
        )
        .unwrap();
        assert_eq!(cfg.url, "https://searx.example.test");
        let cfg = resolve_with(
            env_of(&[(URL_ENV, "https://searx.example.test/sub/search/")]),
            None,
        )
        .unwrap();
        assert_eq!(cfg.url, "https://searx.example.test/sub");
    }

    #[test]
    fn searxng_takes_an_optional_bearer_key() {
        let cfg = resolve_with(
            env_of(&[
                (PROVIDER_ENV, "SearXNG"),
                (URL_ENV, "https://searx.example.test"),
                (API_KEY_ENV, "proxy-token"),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(cfg.provider, ProviderKind::SearXng);
        assert_eq!(cfg.api_key.as_deref(), Some("proxy-token"));
        assert_eq!(
            resolve_with(env_of(&[(PROVIDER_ENV, "searxng")]), None),
            Err(ConfigProblem::MissingUrl(ProviderKind::SearXng))
        );
    }

    #[test]
    fn api_providers_need_a_key_and_default_to_their_public_endpoint() {
        for (name, kind, public) in [
            ("brave", ProviderKind::Brave, BRAVE_DEFAULT_URL),
            ("tavily", ProviderKind::Tavily, TAVILY_DEFAULT_URL),
        ] {
            assert_eq!(
                resolve_with(env_of(&[(PROVIDER_ENV, name)]), None),
                Err(ConfigProblem::MissingKey(kind))
            );
            let cfg =
                resolve_with(env_of(&[(PROVIDER_ENV, name), (API_KEY_ENV, "k")]), None).unwrap();
            assert_eq!(cfg.provider, kind);
            assert_eq!(cfg.url, public);
            assert_eq!(cfg.api_key.as_deref(), Some("k"));
            let cfg = resolve_with(
                env_of(&[
                    (PROVIDER_ENV, name),
                    (API_KEY_ENV, "k"),
                    (URL_ENV, "http://127.0.0.1:9100/search-proxy"),
                ]),
                None,
            )
            .unwrap();
            assert_eq!(cfg.url, "http://127.0.0.1:9100/search-proxy");
        }
    }

    #[test]
    fn a_key_without_a_provider_is_reported() {
        assert_eq!(
            resolve_with(env_of(&[(API_KEY_ENV, "k")]), None),
            Err(ConfigProblem::KeyWithoutProvider)
        );
        // Even with a gateway: the key shows intent that must not be guessed.
        assert_eq!(
            resolve_with(
                env_of(&[(API_KEY_ENV, "k")]),
                Some("http://gw.test/v1".into())
            ),
            Err(ConfigProblem::KeyWithoutProvider)
        );
    }

    #[test]
    fn the_gateway_searches_only_when_named() {
        // A configured gateway alone does not turn web search on.
        assert_eq!(
            resolve_with(env_of(&[]), Some("http://127.0.0.1:4000/v1/".into())),
            Err(ConfigProblem::NotConfigured)
        );
        let cfg = resolve_with(
            env_of(&[(PROVIDER_ENV, "gateway")]),
            Some("http://127.0.0.1:4000/v1/".into()),
        )
        .unwrap();
        assert_eq!(cfg.provider, ProviderKind::Gateway);
        assert_eq!(cfg.url, "http://127.0.0.1:4000/v1");
        assert_eq!(cfg.api_key, None);
        assert_eq!(
            resolve_with(env_of(&[(PROVIDER_ENV, "gateway")]), None),
            Err(ConfigProblem::MissingUrl(ProviderKind::Gateway))
        );
        // An explicit SearXNG URL wins over the gateway.
        let cfg = resolve_with(
            env_of(&[(URL_ENV, "http://127.0.0.1:8888")]),
            Some("http://127.0.0.1:4000/v1".into()),
        )
        .unwrap();
        assert_eq!(cfg.provider, ProviderKind::SearXng);
    }

    #[test]
    fn off_disables_everything() {
        for off in ["off", "OFF", "none", "disabled", "0", "false"] {
            assert_eq!(
                resolve_with(
                    env_of(&[(PROVIDER_ENV, off), (URL_ENV, "http://127.0.0.1:8888")]),
                    Some("http://127.0.0.1:4000/v1".into())
                ),
                Err(ConfigProblem::Disabled),
                "{off}"
            );
        }
    }

    #[test]
    fn bad_values_are_rejected_not_guessed() {
        assert_eq!(
            resolve_with(env_of(&[(PROVIDER_ENV, "bing")]), None),
            Err(ConfigProblem::UnknownProvider("bing".into()))
        );
        for bad in [
            "searx.example.test",
            "ftp://searx.example.test",
            "http://",
            "http:// spaced.example.test",
            "javascript:alert(1)",
        ] {
            assert_eq!(
                resolve_with(env_of(&[(URL_ENV, bad)]), None),
                Err(ConfigProblem::InvalidUrl),
                "{bad}"
            );
        }
    }

    #[test]
    fn provider_ids_round_trip() {
        for kind in ProviderKind::ALL {
            assert_eq!(ProviderKind::parse(kind.id()), Some(kind));
            assert_eq!(kind.to_string(), kind.id());
        }
        assert_eq!(ProviderKind::parse(" Tavily "), Some(ProviderKind::Tavily));
        assert!(ProviderKind::Brave.needs_key() && !ProviderKind::SearXng.needs_key());
    }

    #[test]
    fn debug_output_hides_the_key() {
        let cfg = WebSearchConfig {
            provider: ProviderKind::Brave,
            url: BRAVE_DEFAULT_URL.into(),
            api_key: Some("super-secret-token".into()),
        };
        let shown = format!("{cfg:?}");
        assert!(!shown.contains("super-secret-token"), "{shown}");
        assert!(shown.contains("<redacted>"));
    }

    #[test]
    fn every_problem_explains_itself() {
        for problem in [
            ConfigProblem::NotConfigured,
            ConfigProblem::Disabled,
            ConfigProblem::UnknownProvider("x".into()),
            ConfigProblem::MissingUrl(ProviderKind::SearXng),
            ConfigProblem::MissingUrl(ProviderKind::Gateway),
            ConfigProblem::MissingKey(ProviderKind::Brave),
            ConfigProblem::InvalidUrl,
            ConfigProblem::KeyWithoutProvider,
        ] {
            let message = problem.message();
            assert!(message.starts_with("web search") || message.contains("SCREENPIPE_"));
            assert!(!message.is_empty());
        }
        assert!(ConfigProblem::NotConfigured.message().contains(URL_ENV));
    }
}
