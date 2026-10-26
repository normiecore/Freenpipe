// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Local OpenAI-compatible chat proxy (`POST /v1/local/chat/completions`).
//!
//! Pipes, agents and scripts call the local API with the local API key and
//! an OpenAI chat-completions body. The engine looks up the user's default
//! AI preset in the settings store (`store.bin` → `settings.aiPresets`, the
//! entry with `defaultPreset: true`), sends the body to
//! `{preset url}/chat/completions` with the preset's key and model, and
//! passes the answer back, streamed chunk by chunk when the provider streams.
//! Callers use "the user's model" without ever holding its key.
//!
//! Rules this module keeps:
//!
//! - The preset key goes only into the upstream `Authorization` header. It
//!   is never logged, and every error text that leaves the engine has it
//!   (and any `user:password@` part of a URL) blanked out first.
//! - Nothing goes to a host the user did not configure. A preset without a
//!   base URL is an error, except a local Ollama preset, which defaults to
//!   Ollama's loopback address. `screenpipe-cloud` presets map to the user's
//!   own AI gateway ([`screenpipe_core::ai_gateway`]) or are refused.
//! - Waits are bounded: connecting, the wait for the provider's answer to
//!   start, and every gap between streamed chunks have their own limit.
//! - When the caller hangs up, the response body is dropped, which drops the
//!   upstream response and closes that connection, so the provider stops
//!   generating for nobody.
//!
//! Errors use the OpenAI error shape `{"error": {"message", "type", "code"}}`
//! so OpenAI client libraries surface the message as-is.

use std::error::Error as StdError;
use std::fmt;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{Stream, StreamExt};
use serde_json::{json, Map, Value};
use tracing::{debug, warn};

/// Route path, kept from screenpipe.
pub const LOCAL_CHAT_PATH: &str = "/v1/local/chat/completions";

/// Largest request body the route accepts. Requests may carry base64
/// images, so this matches the other chat-completions route.
pub const MAX_REQUEST_BYTES: usize = 50 * 1024 * 1024;

/// Loopback address of a local Ollama server's OpenAI-compatible API, used
/// for `native-ollama` presets that leave the URL empty.
pub const OLLAMA_DEFAULT_URL: &str = "http://localhost:11434/v1";

/// Most bytes of an upstream error body passed back to the caller.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

/// Preset providers that resolve to the user's AI gateway.
const GATEWAY_PROVIDERS: &[&str] = &["screenpipe-cloud", "pi"];

/// Preset providers that cannot serve OpenAI chat completions.
const UNSUPPORTED_PROVIDERS: &[&str] = &["openai-chatgpt"];

/// How long each stage of an upstream call may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// Establishing the TCP/TLS connection.
    pub connect: Duration,
    /// From sending the request until the provider's response headers
    /// arrive. Local models can take a while to load and, without
    /// streaming, to finish the whole answer.
    pub response: Duration,
    /// Longest silence between two chunks of the response body.
    pub idle: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            response: Duration::from_secs(300),
            idle: Duration::from_secs(120),
        }
    }
}

/// The user's AI gateway as the engine sees it, for `screenpipe-cloud`
/// presets.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct GatewayAccess {
    pub base: Option<String>,
    pub key: Option<String>,
}

impl GatewayAccess {
    /// The configured gateway and the credential the engine holds for it.
    pub fn current(pushed_token: Option<&str>) -> Self {
        Self {
            base: screenpipe_core::ai_gateway::base(),
            key: screenpipe_core::ai_gateway::credential(pushed_token),
        }
    }
}

impl fmt::Debug for GatewayAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GatewayAccess")
            .field("base", &self.base)
            .field("key", &self.key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Where one chat request goes.
#[derive(Clone, PartialEq, Eq)]
pub struct ChatTarget {
    /// Id of the preset the target came from.
    pub preset_id: String,
    /// The preset's provider type, as stored.
    pub provider: String,
    /// Full `.../chat/completions` URL.
    pub url: String,
    pub api_key: Option<String>,
    /// The preset's model; `None` when the preset names none.
    pub model: Option<String>,
}

impl fmt::Debug for ChatTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatTarget")
            .field("preset_id", &self.preset_id)
            .field("provider", &self.provider)
            .field("url", &redact_url_credentials(&self.url))
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("model", &self.model)
            .finish()
    }
}

impl ChatTarget {
    /// Strings that must never appear in a response or a log line.
    fn secrets(&self) -> Vec<String> {
        self.api_key.iter().cloned().collect()
    }
}

/// Why a request was answered by the engine itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalChatError {
    /// The request body is not a JSON object.
    InvalidBody(String),
    /// The settings store could not be read.
    SettingsUnreadable(String),
    /// No preset is marked as the default.
    NoDefaultPreset,
    /// The default preset has no base URL.
    MissingBaseUrl { preset: String },
    /// The default preset's base URL is not an http(s) URL.
    InvalidBaseUrl { preset: String },
    /// The default preset uses the hosted provider and no AI gateway is
    /// configured to stand in for it.
    GatewayNotConfigured { preset: String },
    /// The default preset's provider does not speak OpenAI chat completions.
    UnsupportedProvider { preset: String, provider: String },
    /// Neither the preset nor the request names a model.
    MissingModel { preset: String },
    /// The provider could not be reached.
    Unreachable(String),
    /// The provider did not answer in time.
    Timeout(String),
}

impl LocalChatError {
    pub fn status(&self) -> StatusCode {
        match self {
            LocalChatError::SettingsUnreadable(_) => StatusCode::INTERNAL_SERVER_ERROR,
            LocalChatError::Unreachable(_) => StatusCode::BAD_GATEWAY,
            LocalChatError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
            _ => StatusCode::BAD_REQUEST,
        }
    }

    /// OpenAI error `type`.
    pub fn error_type(&self) -> &'static str {
        match self.status() {
            StatusCode::BAD_REQUEST => "invalid_request_error",
            _ => "api_error",
        }
    }

    /// Machine-readable `code`.
    pub fn code(&self) -> &'static str {
        match self {
            LocalChatError::InvalidBody(_) => "invalid_body",
            LocalChatError::SettingsUnreadable(_) => "settings_unreadable",
            LocalChatError::NoDefaultPreset => "no_default_preset",
            LocalChatError::MissingBaseUrl { .. } => "preset_missing_base_url",
            LocalChatError::InvalidBaseUrl { .. } => "preset_invalid_base_url",
            LocalChatError::GatewayNotConfigured { .. } => "ai_gateway_not_configured",
            LocalChatError::UnsupportedProvider { .. } => "unsupported_preset_provider",
            LocalChatError::MissingModel { .. } => "model_missing",
            LocalChatError::Unreachable(_) => "upstream_unreachable",
            LocalChatError::Timeout(_) => "upstream_timeout",
        }
    }

    /// The message shown to the caller. Contains no secret: callers build
    /// the variants from already-redacted text.
    pub fn message(&self) -> String {
        match self {
            LocalChatError::InvalidBody(why) => {
                format!("the request body must be an OpenAI chat-completions JSON object ({why})")
            }
            LocalChatError::SettingsUnreadable(why) => {
                format!("could not read the AI presets from the settings store: {why}")
            }
            LocalChatError::NoDefaultPreset => "no default AI preset is set up. Create an AI \
                 preset in Freenpipe's AI settings and make it the default, then try again."
                .to_string(),
            LocalChatError::MissingBaseUrl { preset } => format!(
                "the default AI preset \"{preset}\" has no base URL. Add the provider's URL to \
                 it in Freenpipe's AI settings, then try again."
            ),
            LocalChatError::InvalidBaseUrl { preset } => format!(
                "the base URL of the default AI preset \"{preset}\" is not an http(s) URL. Fix \
                 it in Freenpipe's AI settings, then try again."
            ),
            LocalChatError::GatewayNotConfigured { preset } => format!(
                "the default AI preset \"{preset}\" uses the screenpipe-cloud provider, which \
                 needs your own AI gateway. Set one in Freenpipe's AI settings or make another \
                 preset the default."
            ),
            LocalChatError::UnsupportedProvider { preset, provider } => format!(
                "the default AI preset \"{preset}\" uses the {provider} provider, which this \
                 OpenAI-compatible proxy cannot call. Make an OpenAI-compatible preset the \
                 default."
            ),
            LocalChatError::MissingModel { preset } => format!(
                "the default AI preset \"{preset}\" has no model and the request names none. \
                 Pick a model for the preset in Freenpipe's AI settings."
            ),
            LocalChatError::Unreachable(detail) => {
                format!("could not reach the AI provider: {detail}")
            }
            LocalChatError::Timeout(detail) => {
                format!("the AI provider did not respond in time ({detail})")
            }
        }
    }

    /// The OpenAI-shaped JSON body.
    pub fn body(&self) -> Value {
        json!({
            "error": {
                "message": self.message(),
                "type": self.error_type(),
                "code": self.code(),
            }
        })
    }
}

impl fmt::Display for LocalChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl StdError for LocalChatError {}

impl IntoResponse for LocalChatError {
    fn into_response(self) -> Response {
        (self.status(), axum::Json(self.body())).into_response()
    }
}

// ---------------------------------------------------------------------------
// Preset resolution
// ---------------------------------------------------------------------------

fn non_empty_str<'a>(preset: &'a Value, key: &str) -> Option<&'a str> {
    preset
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Find the default preset in a parsed `store.bin` and work out where its
/// requests go.
pub fn resolve_default_preset(
    store: &Value,
    gateway: &GatewayAccess,
) -> Result<ChatTarget, LocalChatError> {
    let presets = store
        .pointer("/settings/aiPresets")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let preset = presets
        .iter()
        .find(|p| p.get("defaultPreset").and_then(Value::as_bool) == Some(true))
        .ok_or(LocalChatError::NoDefaultPreset)?;
    target_for_preset(preset, gateway)
}

/// Where requests for one preset go.
pub fn target_for_preset(
    preset: &Value,
    gateway: &GatewayAccess,
) -> Result<ChatTarget, LocalChatError> {
    let preset_id = non_empty_str(preset, "id").unwrap_or("default").to_string();
    let provider = non_empty_str(preset, "provider")
        .unwrap_or("custom")
        .to_string();
    let model = non_empty_str(preset, "model").map(str::to_string);
    let preset_key = non_empty_str(preset, "apiKey").map(str::to_string);

    if UNSUPPORTED_PROVIDERS.contains(&provider.as_str()) {
        return Err(LocalChatError::UnsupportedProvider {
            preset: preset_id,
            provider,
        });
    }

    let (base, api_key) = if GATEWAY_PROVIDERS.contains(&provider.as_str()) {
        let base = gateway
            .base
            .clone()
            .ok_or_else(|| LocalChatError::GatewayNotConfigured {
                preset: preset_id.clone(),
            })?;
        (base, gateway.key.clone())
    } else {
        let base = match non_empty_str(preset, "url") {
            Some(url) => url.to_string(),
            None if provider == "native-ollama" => OLLAMA_DEFAULT_URL.to_string(),
            None => {
                return Err(LocalChatError::MissingBaseUrl { preset: preset_id });
            }
        };
        (base, preset_key)
    };

    let url = chat_completions_url(&base).ok_or_else(|| LocalChatError::InvalidBaseUrl {
        preset: preset_id.clone(),
    })?;
    Ok(ChatTarget {
        preset_id,
        provider,
        url,
        api_key,
        model,
    })
}

/// `{base}/chat/completions`, accepting a base that already ends with it.
/// `None` unless the base is an http(s) URL.
pub fn chat_completions_url(base: &str) -> Option<String> {
    let base = base.trim().trim_end_matches('/');
    if base.chars().any(char::is_whitespace) {
        return None;
    }
    let parsed = url::Url::parse(base).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return None;
    }
    if base.ends_with("/chat/completions") {
        Some(base.to_string())
    } else {
        Some(format!("{base}/chat/completions"))
    }
}

/// Check the request body and put the preset's model in it. The preset
/// decides the model: callers ask for "the user's model", and a pipe cannot
/// switch the user's key to another model. Everything else is passed on
/// unchanged.
pub fn prepare_body(raw: &[u8], target: &ChatTarget) -> Result<(Vec<u8>, bool), LocalChatError> {
    let mut body: Map<String, Value> = match serde_json::from_slice::<Value>(raw) {
        Ok(Value::Object(map)) => map,
        Ok(_) => return Err(LocalChatError::InvalidBody("not a JSON object".into())),
        Err(e) => return Err(LocalChatError::InvalidBody(format!("invalid JSON: {e}"))),
    };
    match &target.model {
        Some(model) => {
            body.insert("model".into(), Value::String(model.clone()));
        }
        None => {
            let named = body
                .get("model")
                .and_then(Value::as_str)
                .is_some_and(|m| !m.trim().is_empty());
            if !named {
                return Err(LocalChatError::MissingModel {
                    preset: target.preset_id.clone(),
                });
            }
        }
    }
    let streaming = body.get("stream").and_then(Value::as_bool) == Some(true);
    let bytes = serde_json::to_vec(&Value::Object(body))
        .map_err(|e| LocalChatError::InvalidBody(e.to_string()))?;
    Ok((bytes, streaming))
}

// ---------------------------------------------------------------------------
// Redaction
// ---------------------------------------------------------------------------

/// Replace every occurrence of each secret in `text`, and the credentials
/// part of any URL in it.
pub fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = redact_url_credentials(text);
    for secret in secrets {
        let secret = secret.trim();
        // Very short values would blank out ordinary words.
        if secret.len() >= 4 {
            out = out.replace(secret, "[redacted]");
        }
    }
    out
}

/// Blank the `user:password@` part of every `scheme://` URL in `text`.
pub fn redact_url_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("://") {
        let (head, tail) = rest.split_at(pos + 3);
        out.push_str(head);
        let authority_end = tail
            .find(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace() || c == ')')
            .unwrap_or(tail.len());
        let authority = &tail[..authority_end];
        match authority.rfind('@') {
            Some(at) => {
                out.push_str("[redacted]@");
                out.push_str(&authority[at + 1..]);
            }
            None => out.push_str(authority),
        }
        rest = &tail[authority_end..];
    }
    out.push_str(rest);
    out
}

/// An error with its chain of causes, which is where reqwest keeps the
/// useful part ("connection refused").
pub fn error_chain(err: &(dyn StdError + 'static)) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !out.contains(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        source = cause.source();
    }
    out
}

// ---------------------------------------------------------------------------
// Forwarding
// ---------------------------------------------------------------------------

/// HTTP client with the default connect timeout, built once.
fn default_client() -> Result<reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| build_client(Timeouts::default().connect))
        .clone()
}

fn build_client(connect: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(connect)
        // Never follow a redirect with the preset key to another host.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

fn client_for(timeouts: &Timeouts) -> Result<reqwest::Client, String> {
    if timeouts.connect == Timeouts::default().connect {
        default_client()
    } else {
        build_client(timeouts.connect)
    }
}

/// Handle one proxy request end to end: resolve the default preset from
/// `store`, check the body, forward it and pass the answer back.
pub async fn proxy(
    store: Result<Value, String>,
    gateway: &GatewayAccess,
    raw_body: &[u8],
    timeouts: Timeouts,
) -> Response {
    let store = match store {
        Ok(store) => store,
        Err(why) => return LocalChatError::SettingsUnreadable(why).into_response(),
    };
    let target = match resolve_default_preset(&store, gateway) {
        Ok(target) => target,
        Err(e) => return e.into_response(),
    };
    let (body, streaming) = match prepare_body(raw_body, &target) {
        Ok(prepared) => prepared,
        Err(e) => return e.into_response(),
    };
    forward(&target, body, streaming, timeouts).await
}

/// Send a prepared body to `target` and turn the provider's answer into the
/// route's response.
pub async fn forward(
    target: &ChatTarget,
    body: Vec<u8>,
    streaming: bool,
    timeouts: Timeouts,
) -> Response {
    let secrets = target.secrets();
    let host = url::Url::parse(&target.url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default();
    let client = match client_for(&timeouts) {
        Ok(c) => c,
        Err(e) => return LocalChatError::Unreachable(redact(&e, &secrets)).into_response(),
    };

    let mut request = client
        .post(&target.url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body);
    if streaming {
        request = request.header(reqwest::header::ACCEPT, "text/event-stream");
    }
    if let Some(key) = &target.api_key {
        request = request.bearer_auth(key);
    }

    debug!(
        preset = %target.preset_id,
        provider = %target.provider,
        host = %host,
        streaming,
        "local chat: forwarding to the default preset"
    );

    let upstream = match tokio::time::timeout(timeouts.response, request.send()).await {
        Err(_) => {
            warn!(host = %host, "local chat: the AI provider did not answer in time");
            return LocalChatError::Timeout(format!(
                "no answer within {} s",
                timeouts.response.as_secs()
            ))
            .into_response();
        }
        Ok(Err(e)) => {
            let detail = redact(&error_chain(&e), &secrets);
            warn!(host = %host, "local chat: AI provider unreachable: {detail}");
            return if e.is_timeout() {
                LocalChatError::Timeout(detail).into_response()
            } else {
                LocalChatError::Unreachable(detail).into_response()
            };
        }
        Ok(Ok(resp)) => resp,
    };

    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| HeaderValue::from_bytes(v.as_bytes()).ok())
        .unwrap_or_else(|| HeaderValue::from_static("application/json"));

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, content_type.clone());
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));

    if !status.is_success() {
        // Provider errors are small; read them whole so a key the provider
        // echoed back can be blanked before the caller sees it.
        let text = read_capped(upstream, MAX_ERROR_BODY_BYTES, timeouts.idle).await;
        warn!(host = %host, status = %status, "local chat: the AI provider returned an error");
        return (status, headers, redact(&text, &secrets)).into_response();
    }

    let is_sse = content_type
        .to_str()
        .map(|ct| ct.to_ascii_lowercase().starts_with("text/event-stream"))
        .unwrap_or(false);
    let stream = relay_stream(upstream.bytes_stream(), timeouts.idle, is_sse, secrets);
    (status, headers, Body::from_stream(stream)).into_response()
}

/// Read at most `cap` bytes of a response body as text, giving up after
/// `idle` without data.
async fn read_capped(resp: reqwest::Response, cap: usize, idle: Duration) -> String {
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while buf.len() < cap {
        match tokio::time::timeout(idle, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                let room = cap - buf.len();
                buf.extend_from_slice(&chunk[..chunk.len().min(room)]);
            }
            _ => break,
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

type ByteStream = Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>;

enum RelayState {
    Live(ByteStream),
    Done,
}

/// Pass the provider's body through chunk by chunk. A failure or a silence
/// longer than `idle` ends the stream: an SSE stream gets a final `error`
/// event in OpenAI's shape so clients can show it, any other body is cut
/// off with an error so the caller sees a truncated response, not a short
/// valid one.
pub fn relay_stream<S>(
    upstream: S,
    idle: Duration,
    is_sse: bool,
    secrets: Vec<String>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static
where
    S: Stream<Item = reqwest::Result<Bytes>> + Send + 'static,
{
    let boxed: ByteStream = Box::pin(upstream);
    futures::stream::unfold(RelayState::Live(boxed), move |state| {
        let secrets = secrets.clone();
        async move {
            let RelayState::Live(mut upstream) = state else {
                return None;
            };
            let failure = match tokio::time::timeout(idle, upstream.next()).await {
                Ok(Some(Ok(chunk))) => return Some((Ok(chunk), RelayState::Live(upstream))),
                Ok(None) => return None,
                Ok(Some(Err(e))) => LocalChatError::Unreachable(format!(
                    "the stream broke off: {}",
                    redact(&error_chain(&e), &secrets)
                )),
                Err(_) => LocalChatError::Timeout(format!(
                    "no data for {} s while streaming",
                    idle.as_secs()
                )),
            };
            warn!("local chat: {}", failure.message());
            // Dropping `upstream` here closes the provider connection.
            drop(upstream);
            let item = if is_sse {
                Ok(Bytes::from(format!("data: {}\n\n", failure.body())))
            } else {
                Err(std::io::Error::other(failure.message()))
            };
            Some((item, RelayState::Done))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::extract::State as MockState;
    use axum::routing::post;
    use axum::Router;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn store_with(presets: Value) -> Value {
        json!({ "settings": { "aiPresets": presets } })
    }

    fn no_gateway() -> GatewayAccess {
        GatewayAccess::default()
    }

    async fn error_of(resp: Response) -> (StatusCode, Value) {
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    // -- preset resolution --------------------------------------------------

    #[test]
    fn the_flagged_default_preset_is_used() {
        let store = store_with(json!([
            {"id": "other", "provider": "openai", "url": "https://api.example.test/v1",
             "model": "m1", "apiKey": "k1", "defaultPreset": false},
            {"id": "main", "provider": "custom", "url": "http://127.0.0.1:8000/v1/",
             "model": "m2", "apiKey": "k2", "defaultPreset": true},
        ]));
        let target = resolve_default_preset(&store, &no_gateway()).unwrap();
        assert_eq!(target.preset_id, "main");
        assert_eq!(target.url, "http://127.0.0.1:8000/v1/chat/completions");
        assert_eq!(target.api_key.as_deref(), Some("k2"));
        assert_eq!(target.model.as_deref(), Some("m2"));
    }

    #[test]
    fn no_presets_or_no_flag_is_the_missing_preset_error() {
        for store in [
            json!({}),
            json!({"settings": {}}),
            store_with(json!([])),
            store_with(json!([{"id": "a", "url": "http://x.test/v1", "model": "m"}])),
        ] {
            assert_eq!(
                resolve_default_preset(&store, &no_gateway()),
                Err(LocalChatError::NoDefaultPreset)
            );
        }
        let e = LocalChatError::NoDefaultPreset;
        assert_eq!(e.status(), StatusCode::BAD_REQUEST);
        assert_eq!(e.body()["error"]["type"], "invalid_request_error");
        assert!(e.message().contains("default AI preset"));
    }

    #[test]
    fn a_preset_without_url_is_refused_except_local_ollama() {
        let custom = json!({"id": "c", "provider": "custom", "model": "m", "url": " "});
        assert_eq!(
            target_for_preset(&custom, &no_gateway()),
            Err(LocalChatError::MissingBaseUrl { preset: "c".into() })
        );
        // No hosted default either, even for a well-known provider.
        let openai = json!({"id": "o", "provider": "openai", "model": "gpt", "apiKey": "k"});
        assert_eq!(
            target_for_preset(&openai, &no_gateway()),
            Err(LocalChatError::MissingBaseUrl { preset: "o".into() })
        );
        let ollama = json!({"id": "l", "provider": "native-ollama", "model": "llama3"});
        let target = target_for_preset(&ollama, &no_gateway()).unwrap();
        assert_eq!(target.url, "http://localhost:11434/v1/chat/completions");
        assert_eq!(target.api_key, None);
    }

    #[test]
    fn bad_urls_are_refused() {
        for url in ["localhost:11434/v1", "ftp://x.test/v1", "http://", "http://a b/v1"] {
            let preset = json!({"id": "p", "provider": "custom", "model": "m", "url": url});
            assert_eq!(
                target_for_preset(&preset, &no_gateway()),
                Err(LocalChatError::InvalidBaseUrl { preset: "p".into() }),
                "{url}"
            );
        }
        assert_eq!(
            chat_completions_url("http://h.test/v1/chat/completions/").as_deref(),
            Some("http://h.test/v1/chat/completions")
        );
    }

    #[test]
    fn hosted_presets_map_to_the_users_gateway_or_are_refused() {
        let preset = json!({"id": "screenpipe", "provider": "screenpipe-cloud", "model": "auto",
                            "url": "", "apiKey": "ignored-preset-key", "defaultPreset": true});
        assert_eq!(
            target_for_preset(&preset, &no_gateway()),
            Err(LocalChatError::GatewayNotConfigured {
                preset: "screenpipe".into()
            })
        );
        let gateway = GatewayAccess {
            base: Some("http://127.0.0.1:4000/v1".into()),
            key: Some("gw-key".into()),
        };
        let target = target_for_preset(&preset, &gateway).unwrap();
        assert_eq!(target.url, "http://127.0.0.1:4000/v1/chat/completions");
        assert_eq!(target.api_key.as_deref(), Some("gw-key"));
    }

    #[test]
    fn chatgpt_oauth_presets_are_unsupported() {
        let preset = json!({"id": "gpt", "provider": "openai-chatgpt", "model": "gpt-5",
                            "url": "https://chat.example.test"});
        let err = target_for_preset(&preset, &no_gateway()).unwrap_err();
        assert_eq!(err.code(), "unsupported_preset_provider");
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }

    // -- body ----------------------------------------------------------------

    fn target(model: Option<&str>) -> ChatTarget {
        ChatTarget {
            preset_id: "p".into(),
            provider: "custom".into(),
            url: "http://127.0.0.1:1/v1/chat/completions".into(),
            api_key: Some("sk-secret-value".into()),
            model: model.map(str::to_string),
        }
    }

    #[test]
    fn the_preset_model_wins_and_the_rest_passes_through() {
        let raw = br#"{"model":"caller-pick","messages":[{"role":"user","content":"hi"}],"stream":true,"temperature":0.2}"#;
        let (bytes, streaming) = prepare_body(raw, &target(Some("preset-model"))).unwrap();
        assert!(streaming);
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["model"], "preset-model");
        assert_eq!(v["temperature"], 0.2);
        assert_eq!(v["messages"][0]["content"], "hi");

        let (bytes, streaming) = prepare_body(br#"{"model":"mine","messages":[]}"#, &target(None)).unwrap();
        assert!(!streaming);
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["model"], "mine");

        assert_eq!(
            prepare_body(br#"{"messages":[]}"#, &target(None)).unwrap_err().code(),
            "model_missing"
        );
    }

    #[test]
    fn non_object_bodies_are_rejected() {
        for raw in [&b"not json"[..], b"[1,2]", b"\"x\"", b""] {
            let err = prepare_body(raw, &target(Some("m"))).unwrap_err();
            assert_eq!(err.code(), "invalid_body");
            assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        }
    }

    // -- redaction -------------------------------------------------------------

    #[test]
    fn redaction_blanks_keys_and_url_credentials() {
        let text = "error sending request for url (https://user:pa55@host.test/v1?x=1): key sk-secret-value rejected";
        let out = redact(text, &["sk-secret-value".to_string()]);
        assert!(!out.contains("pa55") && !out.contains("user:"), "{out}");
        assert!(!out.contains("sk-secret-value"), "{out}");
        assert!(out.contains("https://[redacted]@host.test/v1?x=1"), "{out}");
        assert_eq!(redact_url_credentials("no urls here"), "no urls here");
        assert_eq!(
            redact_url_credentials("http://a.test/x and http://b@c.test"),
            "http://a.test/x and http://[redacted]@c.test"
        );
        // Tiny secrets are not applied (they would mangle ordinary text).
        assert_eq!(redact("abc def", &["ab".into()]), "abc def");
    }

    #[test]
    fn debug_output_hides_keys() {
        let shown = format!("{:?}", target(Some("m")));
        assert!(!shown.contains("sk-secret-value"), "{shown}");
        let gw = GatewayAccess {
            base: Some("http://g.test".into()),
            key: Some("gw-secret".into()),
        };
        assert!(!format!("{gw:?}").contains("gw-secret"));
    }

    // -- forwarding against mock providers -----------------------------------

    type Seen = Arc<Mutex<Vec<(Option<String>, Value)>>>;

    async fn serve(router: Router) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        addr
    }

    /// A mock OpenAI server: JSON for plain requests, three SSE chunks for
    /// `stream: true`.
    async fn mock_openai() -> (std::net::SocketAddr, Seen) {
        let seen: Seen = Arc::default();
        let router = Router::new()
            .route(
                "/v1/chat/completions",
                post(
                    |MockState(seen): MockState<Seen>, headers: axum::http::HeaderMap, body: String| async move {
                        let auth = headers
                            .get(header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string);
                        let body: Value = serde_json::from_str(&body).unwrap();
                        let streaming = body["stream"] == true;
                        seen.lock().unwrap().push((auth, body));
                        if streaming {
                            let chunks = ["hel", "lo", "!"].map(|t| {
                                Ok::<_, std::io::Error>(Bytes::from(format!(
                                    "data: {}\n\n",
                                    json!({"choices": [{"delta": {"content": t}}]})
                                )))
                            });
                            let done = Ok(Bytes::from_static(b"data: [DONE]\n\n"));
                            let stream = futures::stream::iter(chunks.into_iter().chain([done]));
                            (
                                [(header::CONTENT_TYPE, "text/event-stream")],
                                Body::from_stream(stream),
                            )
                                .into_response()
                        } else {
                            axum::Json(json!({
                                "id": "chatcmpl-mock",
                                "choices": [{"message": {"role": "assistant", "content": "hello"}}]
                            }))
                            .into_response()
                        }
                    },
                ),
            )
            .with_state(seen.clone());
        (serve(router).await, seen)
    }

    fn store_for(addr: std::net::SocketAddr) -> Value {
        store_with(json!([{
            "id": "mock", "provider": "custom", "url": format!("http://{addr}/v1"),
            "model": "mock-model", "apiKey": "sk-preset-secret", "defaultPreset": true
        }]))
    }

    #[tokio::test]
    async fn plain_requests_reach_the_preset_with_its_key_and_model() {
        let (addr, seen) = mock_openai().await;
        let resp = proxy(
            Ok(store_for(addr)),
            &no_gateway(),
            br#"{"messages":[{"role":"user","content":"hi"}]}"#,
            Timeouts::default(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-cache");
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["id"], "chatcmpl-mock");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0.as_deref(), Some("Bearer sk-preset-secret"));
        assert_eq!(seen[0].1["model"], "mock-model");
    }

    #[tokio::test]
    async fn streaming_requests_come_back_as_sse() {
        let (addr, _) = mock_openai().await;
        let resp = proxy(
            Ok(store_for(addr)),
            &no_gateway(),
            br#"{"messages":[{"role":"user","content":"hi"}],"stream":true}"#,
            Timeouts::default(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::CONTENT_TYPE], "text/event-stream");
        assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-cache");
        let text = String::from_utf8(to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec())
            .unwrap();
        let events: Vec<&str> = text.split("\n\n").filter(|e| !e.is_empty()).collect();
        assert_eq!(events.len(), 4, "{text}");
        assert!(events[0].starts_with("data: ") && events[0].contains("\"hel\""));
        assert_eq!(events[3], "data: [DONE]");
    }

    #[tokio::test]
    async fn an_unreachable_provider_is_a_502_without_the_key() {
        let store = store_with(json!([{
            "id": "down", "provider": "custom",
            "url": "http://user:hunter22@127.0.0.1:9/v1",
            "model": "m", "apiKey": "sk-never-shown", "defaultPreset": true
        }]));
        let resp = proxy(Ok(store), &no_gateway(), br#"{"messages":[]}"#, Timeouts::default()).await;
        let (status, body) = error_of(resp).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body["error"]["code"], "upstream_unreachable");
        let message = body["error"]["message"].as_str().unwrap();
        assert!(message.starts_with("could not reach the AI provider: "), "{message}");
        assert!(!message.contains("sk-never-shown") && !message.contains("hunter22"), "{message}");
    }

    #[tokio::test]
    async fn a_slow_provider_is_a_504() {
        let router = Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                "late"
            }),
        );
        let addr = serve(router).await;
        let timeouts = Timeouts {
            response: Duration::from_millis(300),
            ..Timeouts::default()
        };
        let resp = proxy(Ok(store_for(addr)), &no_gateway(), br#"{"messages":[]}"#, timeouts).await;
        let (status, body) = error_of(resp).await;
        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(body["error"]["code"], "upstream_timeout");
    }

    #[tokio::test]
    async fn provider_errors_pass_through_with_the_key_blanked() {
        let router = Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                (
                    StatusCode::UNAUTHORIZED,
                    axum::Json(json!({"error": {"message": "Incorrect API key provided: sk-preset-secret"}})),
                )
            }),
        );
        let addr = serve(router).await;
        let resp = proxy(Ok(store_for(addr)), &no_gateway(), br#"{"messages":[]}"#, Timeouts::default()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let text = String::from_utf8(to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec())
            .unwrap();
        assert!(!text.contains("sk-preset-secret"), "{text}");
        assert!(text.contains("[redacted]"), "{text}");
    }

    #[tokio::test]
    async fn settings_problems_are_reported_before_any_request() {
        let resp = proxy(Err("store.bin is encrypted".into()), &no_gateway(), b"{}", Timeouts::default()).await;
        let (status, body) = error_of(resp).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"]["code"], "settings_unreadable");

        let resp = proxy(Ok(json!({})), &no_gateway(), b"{}", Timeouts::default()).await;
        let (status, body) = error_of(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "no_default_preset");
    }

    /// A provider that streams one chunk, then holds the connection open and
    /// records when its side of the stream is dropped.
    async fn mock_endless_stream(
        dropped: Arc<AtomicBool>,
        sent: Arc<AtomicUsize>,
        pause: Duration,
    ) -> std::net::SocketAddr {
        struct DropFlag(Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let router = Router::new().route(
            "/v1/chat/completions",
            post(move || {
                let dropped = dropped.clone();
                let sent = sent.clone();
                async move {
                    let flag = DropFlag(dropped);
                    let stream = futures::stream::unfold((flag, 0u32), move |(flag, n)| {
                        let sent = sent.clone();
                        async move {
                            if n > 0 {
                                tokio::time::sleep(pause).await;
                            }
                            sent.fetch_add(1, Ordering::SeqCst);
                            Some((
                                Ok::<_, std::io::Error>(Bytes::from(format!("data: {{\"n\":{n}}}\n\n"))),
                                (flag, n + 1),
                            ))
                        }
                    });
                    ([(header::CONTENT_TYPE, "text/event-stream")], Body::from_stream(stream))
                }
            }),
        );
        serve(router).await
    }

    #[tokio::test]
    async fn a_silent_stream_ends_with_an_sse_error_event() {
        let dropped = Arc::new(AtomicBool::new(false));
        let sent = Arc::new(AtomicUsize::new(0));
        let addr = mock_endless_stream(dropped.clone(), sent, Duration::from_secs(30)).await;
        let timeouts = Timeouts {
            idle: Duration::from_millis(300),
            ..Timeouts::default()
        };
        let resp = proxy(
            Ok(store_for(addr)),
            &no_gateway(),
            br#"{"messages":[],"stream":true}"#,
            timeouts,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let text = String::from_utf8(to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec())
            .unwrap();
        assert!(text.starts_with("data: {\"n\":0}"), "{text}");
        assert!(text.contains("\"code\":\"upstream_timeout\""), "{text}");
        assert!(text.ends_with("\n\n"), "the error is a complete SSE event: {text}");
    }

    #[tokio::test]
    async fn a_silent_plain_body_is_cut_off_with_an_error() {
        let upstream = futures::stream::iter(vec![Ok::<Bytes, reqwest::Error>(
            Bytes::from_static(b"{\"partial\":"),
        )])
        .chain(futures::stream::pending());
        let relayed: Vec<_> = relay_stream(upstream, Duration::from_millis(200), false, Vec::new())
            .collect()
            .await;
        assert_eq!(relayed.len(), 2);
        assert!(relayed[0].is_ok());
        assert!(
            relayed[1].is_err(),
            "a cut-off plain body must surface as an error"
        );
    }

    /// The caller hangs up mid-stream: the engine must close the provider
    /// connection so the model stops generating.
    #[tokio::test]
    async fn a_client_disconnect_stops_the_upstream_request() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let dropped = Arc::new(AtomicBool::new(false));
        let sent = Arc::new(AtomicUsize::new(0));
        let upstream =
            mock_endless_stream(dropped.clone(), sent.clone(), Duration::from_millis(50)).await;

        // The proxy under test, served over real TCP like the engine.
        let store = store_for(upstream);
        let proxy_router = Router::new().route(
            LOCAL_CHAT_PATH,
            post(move |body: Bytes| {
                let store = store.clone();
                async move { proxy(Ok(store), &GatewayAccess::default(), &body, Timeouts::default()).await }
            }),
        );
        let proxy_addr = serve(proxy_router).await;

        let mut socket = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
        let body = r#"{"messages":[],"stream":true}"#;
        let request = format!(
            "POST {LOCAL_CHAT_PATH} HTTP/1.1\r\nhost: localhost\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut buf = vec![0u8; 4096];
        let mut received = String::new();
        while !received.contains("data: {\"n\":1}") {
            let n = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buf))
                .await
                .expect("no streamed data")
                .unwrap();
            assert!(n > 0, "proxy closed early: {received}");
            received.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        assert!(received.contains("text/event-stream"), "{received}");
        drop(socket);

        tokio::time::timeout(Duration::from_secs(5), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the provider kept streaming after the caller left");
        let after = sent.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(sent.load(Ordering::SeqCst), after, "provider still producing");
    }
}
