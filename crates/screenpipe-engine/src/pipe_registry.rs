// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Read-only client for a user-configured pipe registry.
//!
//! Freenpipe ships without a registry address. The registry is whatever
//! [`Endpoint::PipeRegistry`] resolves to (a settings override or the
//! `SCREENPIPE_API_BASE_URL` environment variable), and it is unset by
//! default. Callers decide what "unset" means for them: the store listing
//! shows an empty list, everything else reports `registry_not_configured`.
//!
//! Two kinds of registry are understood, chosen by the configured URL:
//!
//! * **Static index**: a URL whose path ends in `.json`. That one document
//!   lists every pipe, either as a bare array or as an object holding a
//!   `data` (or `pipes`) array. Search, category filter, sort and paging run
//!   here, so any static file host works, including a plain directory server
//!   on the local machine. An entry carries its pipe source inline
//!   (`source_md`) or points at it with `source_url` / `source_path`,
//!   resolved against the index URL. When an entry has a `sha256`, the source
//!   must match it.
//! * **Registry API**: any other URL is the base of a server that implements
//!   the pipe store API: `GET {base}/api/pipes/store` to search and
//!   `GET {base}/api/pipes/store/{slug}` for one pipe.
//!
//! Nothing here writes to a registry. Publishing, unpublishing and reviews
//! are refused by the callers with [`RegistryError::read_only`], and no local
//! credential is ever forwarded to the registry.
//!
//! Every failure is a [`RegistryError`] with a stable `error_code`
//! (`registry_not_configured`, `registry_http_error`, `registry_network`,
//! `registry_timeout`, `invalid_registry_response`, `pipe_not_found`,
//! `registry_read_only`) and a matching HTTP status for the engine routes.

use std::fmt;
use std::time::Duration;

use axum::http::StatusCode;
use once_cell::sync::Lazy;
use screenpipe_core::endpoints::{self, Endpoint};
use screenpipe_core::pipes::parse_frontmatter;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;
use tracing::{debug, warn};
use url::Url;

/// Per-request timeout for registry traffic.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Largest registry document (index, detail or pipe source) that is read.
const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;

/// Longest slug accepted. Slugs become directory names under the pipes dir.
const MAX_SLUG_LEN: usize = 128;

/// One HTTP client for all registry requests (connection pooling).
static HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .pool_max_idle_per_host(4)
        .timeout(REQUEST_TIMEOUT)
        .user_agent(endpoints::user_agent(
            "freenpipe-engine",
            env!("CARGO_PKG_VERSION"),
        ))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
});

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// What went wrong while talking to (or not having) a registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryErrorKind {
    /// No registry URL is configured.
    NotConfigured,
    /// The registry answered with a non-success HTTP status.
    HttpError,
    /// The registry could not be reached.
    Network,
    /// The registry did not answer in time.
    Timeout,
    /// The registry answered with something that is not a usable document.
    InvalidResponse,
    /// The requested pipe is not listed (or the slug cannot exist).
    PipeNotFound,
    /// A write operation (publish, unpublish, review) was requested.
    ReadOnly,
}

impl RegistryErrorKind {
    /// Stable machine-readable code, sent as `error_code`.
    pub fn code(self) -> &'static str {
        match self {
            RegistryErrorKind::NotConfigured => "registry_not_configured",
            RegistryErrorKind::HttpError => "registry_http_error",
            RegistryErrorKind::Network => "registry_network",
            RegistryErrorKind::Timeout => "registry_timeout",
            RegistryErrorKind::InvalidResponse => "invalid_registry_response",
            RegistryErrorKind::PipeNotFound => "pipe_not_found",
            RegistryErrorKind::ReadOnly => "registry_read_only",
        }
    }

    /// HTTP status the engine answers with for this kind of failure.
    pub fn http_status(self) -> StatusCode {
        match self {
            RegistryErrorKind::NotConfigured => StatusCode::SERVICE_UNAVAILABLE,
            RegistryErrorKind::HttpError
            | RegistryErrorKind::Network
            | RegistryErrorKind::InvalidResponse => StatusCode::BAD_GATEWAY,
            RegistryErrorKind::Timeout => StatusCode::GATEWAY_TIMEOUT,
            RegistryErrorKind::PipeNotFound => StatusCode::NOT_FOUND,
            RegistryErrorKind::ReadOnly => StatusCode::NOT_IMPLEMENTED,
        }
    }
}

/// A registry failure with a human-readable message.
#[derive(Debug, Clone)]
pub struct RegistryError {
    pub kind: RegistryErrorKind,
    pub message: String,
    /// The registry's own HTTP status, for [`RegistryErrorKind::HttpError`].
    pub upstream_status: Option<u16>,
}

impl RegistryError {
    fn new(kind: RegistryErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            upstream_status: None,
        }
    }

    /// No registry is configured.
    pub fn not_configured() -> Self {
        Self::new(
            RegistryErrorKind::NotConfigured,
            "no pipe registry is configured: set SCREENPIPE_API_BASE_URL to a registry \
             server or to a static index (.json) to browse and install pipes",
        )
    }

    /// A write operation was requested; `action` names it ("publishing").
    pub fn read_only(action: &str) -> Self {
        Self::new(
            RegistryErrorKind::ReadOnly,
            format!(
                "{action} is not supported: pipe registries are read-only here. \
                 To share a pipe, export its pipe.md file instead"
            ),
        )
    }

    /// The pipe is not in the registry, or `slug` is not a usable pipe id.
    pub fn pipe_not_found(slug: &str) -> Self {
        Self::new(
            RegistryErrorKind::PipeNotFound,
            format!("pipe '{slug}' was not found in the registry"),
        )
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new(RegistryErrorKind::InvalidResponse, message)
    }

    fn http(status: reqwest::StatusCode, url: &str) -> Self {
        Self {
            kind: RegistryErrorKind::HttpError,
            message: format!("registry answered HTTP {} for {}", status.as_u16(), url),
            upstream_status: Some(status.as_u16()),
        }
    }

    fn from_reqwest(err: reqwest::Error, url: &str) -> Self {
        if err.is_timeout() {
            Self::new(
                RegistryErrorKind::Timeout,
                format!("registry did not answer in time ({url})"),
            )
        } else if err.is_decode() || err.is_body() {
            Self::invalid(format!(
                "could not read the registry response from {url}: {err}"
            ))
        } else {
            Self::new(
                RegistryErrorKind::Network,
                format!("could not reach the registry at {url}: {err}"),
            )
        }
    }

    /// The `{error, error_code}` envelope the engine routes answer with.
    pub fn body(&self) -> Value {
        let mut body = json!({
            "error": self.message,
            "error_code": self.kind.code(),
        });
        if let Some(status) = self.upstream_status {
            body["upstream_status"] = json!(status);
        }
        body
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.kind.code())
    }
}

impl std::error::Error for RegistryError {}

// ---------------------------------------------------------------------------
// Slugs
// ---------------------------------------------------------------------------

/// Whether `slug` can name a registry pipe. Slugs end up in registry URLs and
/// as directory names under the pipes dir, so only a conservative character
/// set is allowed: ASCII letters, digits, `-`, `_` and `.`, starting and
/// ending with a letter or digit, and never a Windows reserved device name.
pub fn is_valid_slug(slug: &str) -> bool {
    let bytes = slug.as_bytes();
    let (Some(first), Some(last)) = (bytes.first(), bytes.last()) else {
        return false;
    };
    if slug.len() > MAX_SLUG_LEN
        || !first.is_ascii_alphanumeric()
        || !last.is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return false;
    }
    let stem = slug.split('.').next().unwrap_or(slug).to_ascii_lowercase();
    let reserved = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    !reserved
}

// ---------------------------------------------------------------------------
// Location
// ---------------------------------------------------------------------------

/// How the configured registry is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryKind {
    /// One JSON document listing every pipe.
    StaticIndex,
    /// A server implementing the pipe store API.
    Api,
}

/// A parsed registry address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryLocation {
    kind: RegistryKind,
    url: Url,
}

impl RegistryLocation {
    /// Parse a registry URL. Only absolute `http(s)` URLs are accepted; a path
    /// ending in `.json` selects the static index reader.
    pub fn parse(raw: &str) -> Option<Self> {
        let url = Url::parse(raw.trim()).ok()?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return None;
        }
        let kind = if url.path().to_ascii_lowercase().ends_with(".json") {
            RegistryKind::StaticIndex
        } else {
            RegistryKind::Api
        };
        Some(Self { kind, url })
    }

    pub fn kind(&self) -> RegistryKind {
        self.kind
    }

    /// The configured URL, as given (minus trailing slashes for API bases).
    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    /// Host name of the registry, used to trust manifests it serves.
    pub fn host(&self) -> Option<&str> {
        self.url.host_str()
    }

    fn api_url(&self, path: &str) -> String {
        endpoints::join(self.url.as_str(), path)
    }
}

/// The registry configured through settings or the environment, if any.
pub fn configured_location() -> Option<RegistryLocation> {
    let raw = endpoints::resolve(Endpoint::PipeRegistry)?;
    let location = RegistryLocation::parse(&raw);
    if location.is_none() {
        warn!("pipe registry: ignoring unusable registry URL {:?}", raw);
    }
    location
}

// ---------------------------------------------------------------------------
// Queries and results
// ---------------------------------------------------------------------------

/// Store search parameters (`GET /pipes/store?q=&category=&sort=&limit=&offset=`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StoreQuery {
    pub q: Option<String>,
    pub category: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl StoreQuery {
    fn as_params(&self) -> Vec<(&'static str, String)> {
        let mut params = Vec::new();
        if let Some(q) = &self.q {
            params.push(("q", q.clone()));
        }
        if let Some(category) = &self.category {
            params.push(("category", category.clone()));
        }
        if let Some(sort) = &self.sort {
            params.push(("sort", sort.clone()));
        }
        if let Some(limit) = self.limit {
            params.push(("limit", limit.to_string()));
        }
        if let Some(offset) = self.offset {
            params.push(("offset", offset.to_string()));
        }
        params
    }
}

/// A pipe fetched from the registry, ready to install.
#[derive(Debug, Clone)]
pub struct RegistryPipe {
    pub slug: String,
    pub version: i64,
    pub source_md: String,
    /// Connections the pipe's frontmatter asks for.
    pub connections: Vec<String>,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Reads one registry. Cheap to create; create one per request or command.
pub struct RegistryClient {
    location: RegistryLocation,
    http: reqwest::Client,
    /// Static index entries, fetched at most once per client.
    index: OnceCell<Vec<Value>>,
}

impl RegistryClient {
    pub fn new(location: RegistryLocation) -> Self {
        Self {
            location,
            http: HTTP.clone(),
            index: OnceCell::new(),
        }
    }

    /// A client for the configured registry, or `registry_not_configured`.
    pub fn from_config() -> Result<Self, RegistryError> {
        configured_location()
            .map(Self::new)
            .ok_or_else(RegistryError::not_configured)
    }

    pub fn location(&self) -> &RegistryLocation {
        &self.location
    }

    /// Search the registry. The result is always an object whose `data` field
    /// is the array of matching pipes; other fields from an API registry
    /// (paging totals and the like) are passed through.
    pub async fn search(&self, query: &StoreQuery) -> Result<Value, RegistryError> {
        match self.location.kind {
            RegistryKind::Api => {
                let url = self.location.api_url("api/pipes/store");
                let body = self.get_json(&url, &query.as_params(), None).await?;
                list_envelope(body)
            }
            RegistryKind::StaticIndex => {
                let entries = self.index().await?;
                Ok(search_static(entries, query))
            }
        }
    }

    /// Full detail of one pipe, including its source as `source_md`.
    pub async fn detail(&self, slug: &str) -> Result<Value, RegistryError> {
        if !is_valid_slug(slug) {
            return Err(RegistryError::pipe_not_found(slug));
        }
        match self.location.kind {
            RegistryKind::Api => {
                let url = self.location.api_url(&format!("api/pipes/store/{slug}"));
                let body = self.get_json(&url, &[], Some(slug)).await?;
                if body.is_object() {
                    Ok(body)
                } else {
                    Err(RegistryError::invalid(format!(
                        "registry detail for '{slug}' is not a JSON object"
                    )))
                }
            }
            RegistryKind::StaticIndex => {
                let mut entry = self.static_entry(slug).await?;
                let source = self.static_source(&entry).await?;
                if let Some(obj) = entry.as_object_mut() {
                    obj.insert("source_md".to_string(), Value::String(source));
                }
                Ok(entry)
            }
        }
    }

    /// Fetch a pipe's source and version for installation. The source's
    /// frontmatter must parse, so a broken entry is reported as
    /// `invalid_registry_response` before anything is written locally.
    pub async fn fetch_pipe(&self, slug: &str) -> Result<RegistryPipe, RegistryError> {
        let detail = self.detail(slug).await?;
        let source_md = field(&detail, "source_md")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                RegistryError::invalid(format!("registry entry '{slug}' has no pipe source"))
            })?
            .to_string();
        let (config, _) = parse_frontmatter(&source_md).map_err(|e| {
            RegistryError::invalid(format!(
                "registry pipe '{slug}' has an invalid pipe.md: {e}"
            ))
        })?;
        let version = field(&detail, "version").and_then(version_of).unwrap_or(1);
        Ok(RegistryPipe {
            slug: slug.to_string(),
            version,
            source_md,
            connections: config.connections,
        })
    }

    /// Latest published version of a pipe, when the registry states one.
    /// A static index answers from the index alone, without fetching sources.
    pub async fn latest_version(&self, slug: &str) -> Result<Option<i64>, RegistryError> {
        match self.location.kind {
            RegistryKind::StaticIndex => {
                let entry = self.static_entry(slug).await?;
                Ok(entry.get("version").and_then(version_of))
            }
            RegistryKind::Api => {
                let detail = self.detail(slug).await?;
                Ok(field(&detail, "version").and_then(version_of))
            }
        }
    }

    /// Tell an API registry that a pipe was installed, so its install counter
    /// stays meaningful. Best effort; static indexes have no counter.
    pub async fn record_install(&self, slug: &str) {
        if self.location.kind != RegistryKind::Api || !is_valid_slug(slug) {
            return;
        }
        let url = self
            .location
            .api_url(&format!("api/pipes/store/{slug}/install"));
        if let Err(e) = self.http.post(&url).send().await {
            debug!("pipe registry: install count update failed: {}", e);
        }
    }

    // -- static index -------------------------------------------------------

    async fn index(&self) -> Result<&Vec<Value>, RegistryError> {
        self.index
            .get_or_try_init(|| async {
                let url = self.location.as_str().to_string();
                let doc = self.get_json(&url, &[], None).await?;
                index_entries(doc)
            })
            .await
    }

    async fn static_entry(&self, slug: &str) -> Result<Value, RegistryError> {
        if !is_valid_slug(slug) {
            return Err(RegistryError::pipe_not_found(slug));
        }
        self.index()
            .await?
            .iter()
            .find(|entry| entry.get("slug").and_then(Value::as_str) == Some(slug))
            .cloned()
            .ok_or_else(|| RegistryError::pipe_not_found(slug))
    }

    async fn static_source(&self, entry: &Value) -> Result<String, RegistryError> {
        let slug = entry.get("slug").and_then(Value::as_str).unwrap_or("?");
        let source = if let Some(inline) = entry.get("source_md").and_then(Value::as_str) {
            inline.to_string()
        } else if let Some(reference) = ["source_url", "source_path"]
            .iter()
            .find_map(|key| entry.get(*key).and_then(Value::as_str))
        {
            let url = self.resolve_reference(reference)?;
            self.get_text(&url).await?
        } else {
            return Err(RegistryError::invalid(format!(
                "registry entry '{slug}' has neither source_md nor source_url/source_path"
            )));
        };
        if let Some(expected) = entry.get("sha256").and_then(Value::as_str) {
            verify_sha256(slug, &source, expected)?;
        }
        Ok(source)
    }

    /// Resolve a `source_url` / `source_path` against the index URL.
    fn resolve_reference(&self, reference: &str) -> Result<String, RegistryError> {
        let url = self.location.url.join(reference.trim()).map_err(|e| {
            RegistryError::invalid(format!("bad pipe source reference {reference:?}: {e}"))
        })?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(RegistryError::invalid(format!(
                "pipe source {reference:?} is not an http(s) URL"
            )));
        }
        Ok(url.into())
    }

    // -- HTTP ---------------------------------------------------------------

    async fn get_bytes(
        &self,
        url: &str,
        params: &[(&'static str, String)],
        not_found_slug: Option<&str>,
    ) -> Result<Vec<u8>, RegistryError> {
        let response = self
            .http
            .get(url)
            .query(params)
            .send()
            .await
            .map_err(|e| RegistryError::from_reqwest(e, url))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            if let Some(slug) = not_found_slug {
                return Err(RegistryError::pipe_not_found(slug));
            }
        }
        if !status.is_success() {
            return Err(RegistryError::http(status, url));
        }
        if response
            .content_length()
            .is_some_and(|len| len > MAX_DOCUMENT_BYTES as u64)
        {
            return Err(RegistryError::invalid(format!(
                "registry document at {url} is larger than {MAX_DOCUMENT_BYTES} bytes"
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| RegistryError::from_reqwest(e, url))?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(RegistryError::invalid(format!(
                "registry document at {url} is larger than {MAX_DOCUMENT_BYTES} bytes"
            )));
        }
        Ok(bytes.to_vec())
    }

    async fn get_json(
        &self,
        url: &str,
        params: &[(&'static str, String)],
        not_found_slug: Option<&str>,
    ) -> Result<Value, RegistryError> {
        let bytes = self.get_bytes(url, params, not_found_slug).await?;
        serde_json::from_slice(&bytes).map_err(|e| {
            RegistryError::invalid(format!("registry response from {url} is not JSON: {e}"))
        })
    }

    async fn get_text(&self, url: &str) -> Result<String, RegistryError> {
        let bytes = self.get_bytes(url, &[], None).await?;
        String::from_utf8(bytes)
            .map_err(|_| RegistryError::invalid(format!("pipe source at {url} is not UTF-8 text")))
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (unit tested)
// ---------------------------------------------------------------------------

/// Look a field up at the top level or under a `data` wrapper.
fn field<'a>(doc: &'a Value, key: &str) -> Option<&'a Value> {
    doc.get(key)
        .or_else(|| doc.get("data").and_then(|data| data.get(key)))
}

/// Registry versions are integers; integer strings are accepted too.
fn version_of(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn verify_sha256(slug: &str, source: &str, expected: &str) -> Result<(), RegistryError> {
    let expected = expected.trim();
    let expected = expected
        .strip_prefix("sha256:")
        .unwrap_or(expected)
        .to_ascii_lowercase();
    let actual: String = Sha256::digest(source.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual == expected {
        Ok(())
    } else {
        Err(RegistryError::invalid(format!(
            "pipe source for '{slug}' does not match the sha256 listed in the registry"
        )))
    }
}

/// Bring an API registry's list answer into the `{ "data": [...] }` shape.
fn list_envelope(body: Value) -> Result<Value, RegistryError> {
    match body {
        Value::Array(items) => Ok(json!({ "data": items })),
        Value::Object(mut obj) => {
            if obj.get("data").is_some_and(Value::is_array) {
                return Ok(Value::Object(obj));
            }
            if let Some(items) = obj.get("pipes").filter(|v| v.is_array()).cloned() {
                obj.insert("data".to_string(), items);
                return Ok(Value::Object(obj));
            }
            let reason = obj
                .get("error")
                .and_then(Value::as_str)
                .map(|e| format!("registry reported an error: {e}"))
                .unwrap_or_else(|| "registry list has no data array".to_string());
            Err(RegistryError::invalid(reason))
        }
        _ => Err(RegistryError::invalid("registry list is not a JSON object")),
    }
}

/// Validate a static index document and normalise its entries: each one gets
/// a string `slug` (from `slug` or `id`) and a `title` (from `title`, `name`
/// or the slug). Entries without a usable slug are skipped with a warning;
/// duplicates keep the first occurrence.
fn index_entries(doc: Value) -> Result<Vec<Value>, RegistryError> {
    let items = match doc {
        Value::Array(items) => items,
        Value::Object(mut obj) => match obj.remove("data").or_else(|| obj.remove("pipes")) {
            Some(Value::Array(items)) => items,
            _ => {
                return Err(RegistryError::invalid(
                    "registry index has no data or pipes array",
                ))
            }
        },
        _ => {
            return Err(RegistryError::invalid(
                "registry index is not a JSON document",
            ))
        }
    };
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        let Value::Object(mut obj) = item else {
            warn!("pipe registry: skipping an index entry that is not an object");
            continue;
        };
        let slug = obj
            .get("slug")
            .or_else(|| obj.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let Some(slug) = slug.filter(|s| is_valid_slug(s)) else {
            warn!("pipe registry: skipping an index entry without a usable slug");
            continue;
        };
        if !seen.insert(slug.clone()) {
            warn!("pipe registry: skipping duplicate index entry '{}'", slug);
            continue;
        }
        let has_title = obj
            .get("title")
            .and_then(Value::as_str)
            .is_some_and(|t| !t.trim().is_empty());
        if !has_title {
            let title = obj
                .get("name")
                .and_then(Value::as_str)
                .filter(|n| !n.trim().is_empty())
                .unwrap_or(&slug)
                .to_string();
            obj.insert("title".to_string(), Value::String(title));
        }
        obj.insert("slug".to_string(), Value::String(slug));
        entries.push(Value::Object(obj));
    }
    Ok(entries)
}

fn text_of<'a>(entry: &'a Value, key: &str) -> &'a str {
    entry.get(key).and_then(Value::as_str).unwrap_or("")
}

fn number_of(entry: &Value, keys: &[&str]) -> f64 {
    keys.iter()
        .find_map(|key| entry.get(*key).and_then(Value::as_f64))
        .unwrap_or(0.0)
}

fn matches_terms(entry: &Value, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let mut haystack = String::new();
    for key in ["slug", "title", "name", "description", "category", "author"] {
        haystack.push_str(text_of(entry, key));
        haystack.push(' ');
    }
    if let Some(tags) = entry.get("tags").and_then(Value::as_array) {
        for tag in tags.iter().filter_map(Value::as_str) {
            haystack.push_str(tag);
            haystack.push(' ');
        }
    }
    let haystack = haystack.to_lowercase();
    terms.iter().all(|term| haystack.contains(term.as_str()))
}

/// Filter, sort and page static index entries like an API registry would.
/// Listing results leave out the pipe source to keep the answer small.
fn search_static(entries: &[Value], query: &StoreQuery) -> Value {
    let terms: Vec<String> = query
        .q
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let category = query
        .category
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty() && !c.eq_ignore_ascii_case("all"));

    let mut hits: Vec<&Value> = entries
        .iter()
        .filter(|entry| matches_terms(entry, &terms))
        .filter(|entry| category.is_none_or(|c| text_of(entry, "category").eq_ignore_ascii_case(c)))
        .collect();

    match query
        .sort
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("popular") => hits.sort_by(|a, b| {
            number_of(b, &["install_count", "installs"])
                .total_cmp(&number_of(a, &["install_count", "installs"]))
        }),
        Some("newest") | Some("recent") => hits.sort_by(|a, b| {
            let date = |e: &Value| {
                let updated = text_of(e, "updated_at");
                if updated.is_empty() {
                    text_of(e, "created_at").to_string()
                } else {
                    updated.to_string()
                }
            };
            date(b).cmp(&date(a))
        }),
        Some("rating") | Some("top_rated") => hits.sort_by(|a, b| {
            let keys = ["average_rating", "avg_rating", "rating"];
            number_of(b, &keys).total_cmp(&number_of(a, &keys))
        }),
        Some("name") | Some("alphabetical") => hits.sort_by(|a, b| {
            text_of(a, "title")
                .to_lowercase()
                .cmp(&text_of(b, "title").to_lowercase())
        }),
        _ => {}
    }

    let total = hits.len();
    let offset = query.offset.unwrap_or(0) as usize;
    let limit = query.limit.map(|l| l as usize).unwrap_or(usize::MAX);
    let page: Vec<Value> = hits
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|entry| {
            let mut entry = entry.clone();
            if let Some(obj) = entry.as_object_mut() {
                obj.remove("source_md");
            }
            entry
        })
        .collect();
    json!({ "data": page, "total": total })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use serde_json::Map;

    /// Build an index entry fixture from key/value pairs.
    fn entry(fields: &[(&str, Value)]) -> Value {
        let mut map = Map::new();
        for (key, value) in fields {
            map.insert((*key).to_string(), value.clone());
        }
        Value::Object(map)
    }

    // -- pure helpers -------------------------------------------------------

    #[test]
    fn slugs_are_restricted_to_safe_directory_names() {
        for good in ["daily-summary", "crm_v2", "a", "notes.obsidian", "Pipe9"] {
            assert!(is_valid_slug(good), "{good} should be accepted");
        }
        for bad in [
            "",
            "../etc",
            "a/b",
            "a\\b",
            ".hidden",
            "trailing.",
            "-dash",
            "with space",
            "con",
            "NUL",
            "com1",
            "lpt9.md",
            "ünicode",
        ] {
            assert!(!is_valid_slug(bad), "{bad:?} should be rejected");
        }
        assert!(!is_valid_slug(&"a".repeat(MAX_SLUG_LEN + 1)));
        assert!(is_valid_slug("console"));
    }

    #[test]
    fn locations_pick_the_reader_from_the_url() {
        let index = RegistryLocation::parse("http://127.0.0.1:8000/registry/index.json").unwrap();
        assert_eq!(index.kind(), RegistryKind::StaticIndex);
        assert_eq!(index.host(), Some("127.0.0.1"));
        let upper = RegistryLocation::parse("https://example.test/PIPES.JSON").unwrap();
        assert_eq!(upper.kind(), RegistryKind::StaticIndex);
        let api = RegistryLocation::parse("https://registry.example.test").unwrap();
        assert_eq!(api.kind(), RegistryKind::Api);
        assert_eq!(
            api.api_url("api/pipes/store/x"),
            "https://registry.example.test/api/pipes/store/x"
        );
        for bad in [
            "",
            "registry.example.test",
            "file:///tmp/index.json",
            "ftp://x/y.json",
        ] {
            assert!(RegistryLocation::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn error_codes_and_statuses_are_stable() {
        let cases = [
            (
                RegistryErrorKind::NotConfigured,
                "registry_not_configured",
                503,
            ),
            (RegistryErrorKind::HttpError, "registry_http_error", 502),
            (RegistryErrorKind::Network, "registry_network", 502),
            (RegistryErrorKind::Timeout, "registry_timeout", 504),
            (
                RegistryErrorKind::InvalidResponse,
                "invalid_registry_response",
                502,
            ),
            (RegistryErrorKind::PipeNotFound, "pipe_not_found", 404),
            (RegistryErrorKind::ReadOnly, "registry_read_only", 501),
        ];
        for (kind, code, status) in cases {
            assert_eq!(kind.code(), code);
            assert_eq!(kind.http_status().as_u16(), status);
        }
        let body = RegistryError::not_configured().body();
        assert_eq!(body["error_code"], "registry_not_configured");
        assert!(body["error"]
            .as_str()
            .unwrap()
            .contains("SCREENPIPE_API_BASE_URL"));
    }

    #[test]
    fn index_documents_are_validated_and_normalised() {
        let doc = json!({ "pipes": [
            { "id": "first", "name": "First pipe" },
            { "slug": "second", "title": "Second" },
            { "slug": "../escape" },
            { "title": "no slug" },
            "not an object",
            { "slug": "first", "title": "duplicate" },
        ]});
        let entries = index_entries(doc).unwrap();
        let slugs: Vec<&str> = entries.iter().map(|e| text_of(e, "slug")).collect();
        assert_eq!(slugs, ["first", "second"]);
        assert_eq!(entries[0]["title"], "First pipe");

        assert_eq!(index_entries(json!([{ "slug": "x" }])).unwrap().len(), 1);
        assert_eq!(
            index_entries(json!({ "data": [] })).unwrap().len(),
            0,
            "an empty registry is valid"
        );
        for bad in [json!({ "items": [] }), json!("text"), json!(42)] {
            let err = index_entries(bad).unwrap_err();
            assert_eq!(err.kind, RegistryErrorKind::InvalidResponse);
        }
    }

    #[test]
    fn api_list_answers_are_normalised() {
        let wrapped = list_envelope(json!({ "data": [1], "total": 1 })).unwrap();
        assert_eq!(wrapped, json!({ "data": [1], "total": 1 }));
        assert_eq!(
            list_envelope(json!([1, 2])).unwrap(),
            json!({ "data": [1, 2] })
        );
        assert_eq!(
            list_envelope(json!({ "pipes": [3] })).unwrap()["data"],
            json!([3])
        );
        let err = list_envelope(json!({ "error": "boom" })).unwrap_err();
        assert_eq!(err.kind, RegistryErrorKind::InvalidResponse);
        assert!(err.message.contains("boom"));
    }

    fn catalogue() -> Vec<Value> {
        index_entries(json!([
            entry(&[
                ("slug", json!("time-tracker")),
                ("title", json!("Time tracker")),
                ("description", json!("Logs where your hours go")),
                ("category", json!("productivity")),
                ("install_count", json!(40)),
                ("updated_at", json!("2026-03-01T00:00:00Z")),
                ("source_md", json!("---\nschedule: every 1h\n---\nbody")),
            ]),
            entry(&[
                ("slug", json!("standup-notes")),
                ("title", json!("Standup notes")),
                ("description", json!("Drafts a daily standup")),
                ("category", json!("Dev")),
                ("install_count", json!(90)),
                ("updated_at", json!("2026-01-01T00:00:00Z")),
                ("tags", json!(["meetings"])),
            ]),
            entry(&[
                ("slug", json!("reading-list")),
                ("title", json!("Reading list")),
                ("category", json!("productivity")),
                ("install_count", json!(5)),
                ("created_at", json!("2026-06-01T00:00:00Z")),
            ]),
        ]))
        .unwrap()
    }

    fn slugs(result: &Value) -> Vec<String> {
        result["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| text_of(e, "slug").to_string())
            .collect()
    }

    #[test]
    fn static_search_filters_sorts_and_pages() {
        let entries = catalogue();
        let all = search_static(&entries, &StoreQuery::default());
        assert_eq!(
            slugs(&all),
            ["time-tracker", "standup-notes", "reading-list"]
        );
        assert_eq!(all["total"], 3);
        assert!(
            all["data"][0].get("source_md").is_none(),
            "listings leave the source out"
        );

        let q = |q: &str| StoreQuery {
            q: Some(q.to_string()),
            ..Default::default()
        };
        assert_eq!(
            slugs(&search_static(&entries, &q("STANDUP"))),
            ["standup-notes"]
        );
        assert_eq!(
            slugs(&search_static(&entries, &q("meetings"))),
            ["standup-notes"]
        );
        assert_eq!(
            slugs(&search_static(&entries, &q("daily standup"))),
            ["standup-notes"]
        );
        assert!(slugs(&search_static(&entries, &q("nothing-matches"))).is_empty());

        let by_category = StoreQuery {
            category: Some("Productivity".into()),
            ..Default::default()
        };
        assert_eq!(
            slugs(&search_static(&entries, &by_category)),
            ["time-tracker", "reading-list"]
        );
        let all_category = StoreQuery {
            category: Some("All".into()),
            ..Default::default()
        };
        assert_eq!(search_static(&entries, &all_category)["total"], 3);

        let sorted = |sort: &str| StoreQuery {
            sort: Some(sort.to_string()),
            ..Default::default()
        };
        assert_eq!(
            slugs(&search_static(&entries, &sorted("popular"))),
            ["standup-notes", "time-tracker", "reading-list"]
        );
        assert_eq!(
            slugs(&search_static(&entries, &sorted("newest"))),
            ["reading-list", "time-tracker", "standup-notes"]
        );
        assert_eq!(
            slugs(&search_static(&entries, &sorted("name"))),
            ["reading-list", "standup-notes", "time-tracker"]
        );

        let paged = StoreQuery {
            sort: Some("popular".into()),
            offset: Some(1),
            limit: Some(1),
            ..Default::default()
        };
        let page = search_static(&entries, &paged);
        assert_eq!(slugs(&page), ["time-tracker"]);
        assert_eq!(page["total"], 3, "total counts every match, not the page");
    }

    #[test]
    fn versions_and_wrapped_fields_are_read_leniently() {
        assert_eq!(version_of(&json!(3)), Some(3));
        assert_eq!(version_of(&json!(" 4 ")), Some(4));
        assert_eq!(version_of(&json!("1.2.0")), None);
        assert_eq!(version_of(&json!(null)), None);
        let wrapped = json!({ "data": { "version": 7 } });
        assert_eq!(field(&wrapped, "version").and_then(version_of), Some(7));
    }

    #[test]
    fn sha256_must_match_when_listed() {
        let source = "---\nschedule: manual\n---\nhello";
        let digest: String = Sha256::digest(source.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert!(verify_sha256("p", source, &digest).is_ok());
        assert!(verify_sha256("p", source, &format!("sha256:{}", digest.to_uppercase())).is_ok());
        let err = verify_sha256("p", source, &"0".repeat(64)).unwrap_err();
        assert_eq!(err.kind, RegistryErrorKind::InvalidResponse);
    }

    // -- against a local server -------------------------------------------

    pub(crate) const HELLO_PIPE: &str =
        "---\nschedule: manual\nenabled: false\n---\n\nSay hello.\n";

    pub(crate) fn sha256_hex(text: &str) -> String {
        Sha256::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Serve `router` on an ephemeral localhost port; returns its base URL.
    pub(crate) async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}")
    }

    /// A static registry the way a plain file server would expose it.
    pub(crate) fn static_registry(index: Value) -> Router {
        Router::new()
            .route(
                "/registry/index.json",
                get(move || {
                    let index = index.clone();
                    async move { axum::Json(index) }
                }),
            )
            .route(
                "/registry/pipes/hello/pipe.md",
                get(|| async { HELLO_PIPE }),
            )
    }

    pub(crate) fn sample_index() -> Value {
        json!({ "data": [
            {
                "id": "hello",
                "name": "Hello",
                "description": "Says hello",
                "category": "demo",
                "version": 2,
                "source_path": "pipes/hello/pipe.md",
                "sha256": sha256_hex(HELLO_PIPE),
            },
            {
                "slug": "tampered",
                "title": "Tampered",
                "version": 1,
                "source_path": "pipes/hello/pipe.md",
                "sha256": "0".repeat(64),
            },
            {
                "slug": "missing-source",
                "title": "Missing",
                "source_path": "pipes/missing/pipe.md",
            },
            {
                "slug": "inline",
                "title": "Inline",
                "version": "3",
                "source_md": HELLO_PIPE,
            },
            {
                "slug": "broken",
                "title": "Broken frontmatter",
                "source_md": "---\nschedule: [unclosed\n---\nbody",
            },
        ]})
    }

    #[tokio::test]
    async fn static_index_lists_and_installs_from_a_local_file_server() {
        let base = serve(static_registry(sample_index())).await;
        let location = RegistryLocation::parse(&format!("{base}/registry/index.json")).unwrap();
        let client = RegistryClient::new(location);

        let listed = client.search(&StoreQuery::default()).await.unwrap();
        assert_eq!(
            slugs(&listed),
            ["hello", "tampered", "missing-source", "inline", "broken"]
        );

        let hello = client.fetch_pipe("hello").await.unwrap();
        assert_eq!(hello.version, 2);
        assert_eq!(hello.source_md, HELLO_PIPE);
        assert_eq!(client.latest_version("hello").await.unwrap(), Some(2));

        let inline = client.fetch_pipe("inline").await.unwrap();
        assert_eq!(inline.version, 3);

        let broken = client.fetch_pipe("broken").await.unwrap_err();
        assert_eq!(broken.kind, RegistryErrorKind::InvalidResponse);
        assert!(broken.message.contains("invalid pipe.md"), "{broken}");

        let detail = client.detail("hello").await.unwrap();
        assert_eq!(detail["source_md"], HELLO_PIPE);
        assert_eq!(detail["title"], "Hello");

        let tampered = client.fetch_pipe("tampered").await.unwrap_err();
        assert_eq!(tampered.kind, RegistryErrorKind::InvalidResponse);

        let missing = client.fetch_pipe("missing-source").await.unwrap_err();
        assert_eq!(missing.kind, RegistryErrorKind::HttpError);
        assert_eq!(missing.upstream_status, Some(404));

        let unknown = client.fetch_pipe("not-listed").await.unwrap_err();
        assert_eq!(unknown.kind, RegistryErrorKind::PipeNotFound);
        let traversal = client.fetch_pipe("../../etc").await.unwrap_err();
        assert_eq!(traversal.kind, RegistryErrorKind::PipeNotFound);
    }

    #[tokio::test]
    async fn api_registry_is_read_through_the_store_routes() {
        let router = Router::new()
            .route(
                "/api/pipes/store",
                get(|axum::extract::Query(q): axum::extract::Query<StoreQuery>| async move {
                    // Echo the query so the test can check it was forwarded.
                    axum::Json(json!({
                        "data": [{ "slug": "remote", "title": "Remote" }],
                        "echo_q": q.q,
                    }))
                }),
            )
            .route(
                "/api/pipes/store/remote",
                get(|| async {
                    axum::Json(json!({ "data": { "slug": "remote", "version": 5, "source_md": HELLO_PIPE } }))
                }),
            );
        let base = serve(router).await;
        let client = RegistryClient::new(RegistryLocation::parse(&base).unwrap());
        assert_eq!(client.location().kind(), RegistryKind::Api);

        let listed = client
            .search(&StoreQuery {
                q: Some("rem".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(slugs(&listed), ["remote"]);
        assert_eq!(listed["echo_q"], "rem");

        let pipe = client.fetch_pipe("remote").await.unwrap();
        assert_eq!(pipe.version, 5);
        assert_eq!(pipe.source_md, HELLO_PIPE);

        let gone = client.fetch_pipe("gone").await.unwrap_err();
        assert_eq!(gone.kind, RegistryErrorKind::PipeNotFound);
    }

    #[tokio::test]
    async fn unreachable_and_broken_registries_get_typed_errors() {
        // Bind and drop a listener so the port is very likely closed.
        let closed = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let client = RegistryClient::new(
            RegistryLocation::parse(&format!("http://{closed}/index.json")).unwrap(),
        );
        let err = client.search(&StoreQuery::default()).await.unwrap_err();
        assert_eq!(err.kind, RegistryErrorKind::Network);

        let base = serve(
            Router::new()
                .route("/index.json", get(|| async { "this is not json" }))
                .route(
                    "/down.json",
                    get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "down") }),
                ),
        )
        .await;
        let garbled =
            RegistryClient::new(RegistryLocation::parse(&format!("{base}/index.json")).unwrap());
        let err = garbled.search(&StoreQuery::default()).await.unwrap_err();
        assert_eq!(err.kind, RegistryErrorKind::InvalidResponse);

        let down =
            RegistryClient::new(RegistryLocation::parse(&format!("{base}/down.json")).unwrap());
        let err = down.search(&StoreQuery::default()).await.unwrap_err();
        assert_eq!(err.kind, RegistryErrorKind::HttpError);
        assert_eq!(err.body()["upstream_status"], 500);
    }
}
