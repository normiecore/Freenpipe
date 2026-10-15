// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Bring-your-own OAuth client credentials.
//!
//! Every native OAuth integration needs an OAuth app that the user registers
//! with the provider themselves. Its `client_id`, optional `client_secret` and
//! optional redirect override live in the SecretStore under
//! `oauth_client:<integration_id>`, never in settings JSON, and the secret is
//! never logged or handed back to the UI.
//!
//! This module also holds the pure pieces of the authorization request:
//! redirect selection and validation, PKCE, and the authorization URL. They
//! are kept free of I/O so they can be unit-tested on every platform.

use std::fmt;

use base64::Engine;
use reqwest::Url;
use screenpipe_secrets::SecretStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::OAuthConfig;

/// Key family for BYO client credentials in the SecretStore. The character
/// after `oauth` is an underscore, so these keys never fall under the token
/// prefix `oauth:` that the refresh scheduler enumerates.
pub const CLIENT_STORE_KEY_PREFIX: &str = "oauth_client:";

/// Path the engine serves the connector OAuth callback on.
pub const OAUTH_CALLBACK_PATH: &str = "/connections/oauth/callback";

/// Longest value accepted for any credential field. Real client ids and
/// secrets are far shorter; the cap only stops pathological pastes.
const MAX_FIELD_LEN: usize = 2048;

/// SecretStore key holding the BYO client for `integration_id`.
pub fn client_store_key(integration_id: &str) -> String {
    format!("{}{}", CLIENT_STORE_KEY_PREFIX, integration_id)
}

/// Loopback redirect URI for an engine listening on `port`. With the default
/// port this equals [`super::OAUTH_REDIRECT_URI`].
pub fn loopback_redirect_uri(port: u16) -> String {
    format!("http://localhost:{}{}", port, OAUTH_CALLBACK_PATH)
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// The user's own OAuth app for one integration, as stored in the SecretStore.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthClientCredentials {
    pub client_id: String,
    /// Absent for public clients that rely on PKCE alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// Redirect registered with the provider when it is not the loopback
    /// default, e.g. an HTTPS relay the user runs that forwards the query
    /// string to the loopback callback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
}

impl OAuthClientCredentials {
    pub fn has_secret(&self) -> bool {
        self.client_secret.as_deref().is_some_and(|s| !s.is_empty())
    }
}

// Hand-written so a stray `{:?}` in a log line can never print the secret.
impl fmt::Debug for OAuthClientCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthClientCredentials")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &if self.has_secret() { "<redacted>" } else { "<none>" },
            )
            .field("redirect_uri", &self.redirect_uri)
            .finish()
    }
}

/// An edit submitted from the settings UI.
#[derive(Clone, Default)]
pub struct OAuthClientUpdate {
    pub client_id: String,
    /// `None` keeps the stored secret, `Some("")` removes it and any other
    /// value replaces it. The UI never receives the stored secret, so "leave
    /// the field empty" has to mean "keep".
    pub client_secret: Option<String>,
    /// Empty or `None` means "use the loopback default".
    pub redirect_uri: Option<String>,
}

impl fmt::Debug for OAuthClientUpdate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secret = match self.client_secret.as_deref() {
            None => "<keep>",
            Some("") => "<clear>",
            Some(_) => "<redacted>",
        };
        f.debug_struct("OAuthClientUpdate")
            .field("client_id", &self.client_id)
            .field("client_secret", &secret)
            .field("redirect_uri", &self.redirect_uri)
            .finish()
    }
}

/// Validation failures for client credentials and authorization requests.
/// Messages are shown to the user as-is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OAuthClientError {
    #[error("enter the client ID of the OAuth app you registered with the provider")]
    MissingClientId,
    #[error("the {field} contains spaces or control characters; paste it again without them")]
    InvalidCharacters { field: &'static str },
    #[error("the {field} is too long")]
    TooLong { field: &'static str },
    #[error("the redirect URI is not a valid absolute URL ({0})")]
    InvalidRedirect(String),
    #[error(
        "the redirect URI must use https, or plain http on localhost, 127.0.0.1 or [::1]"
    )]
    InsecureRedirect,
    #[error(
        "this provider does not accept loopback redirects: enter the HTTPS address of a relay \
         you control that forwards the provider's query string to {loopback}"
    )]
    RedirectRequired { loopback: String },
    #[error("the provider authorization URL is invalid ({0})")]
    InvalidAuthUrl(String),
}

fn clean_field(
    value: &str,
    field: &'static str,
    allow_inner_spaces: bool,
) -> Result<String, OAuthClientError> {
    let trimmed = value.trim();
    if trimmed.len() > MAX_FIELD_LEN {
        return Err(OAuthClientError::TooLong { field });
    }
    let bad = trimmed
        .chars()
        .any(|c| c.is_control() || (!allow_inner_spaces && c.is_whitespace()));
    if bad {
        return Err(OAuthClientError::InvalidCharacters { field });
    }
    Ok(trimmed.to_string())
}

/// Merge a settings-UI edit into the stored credentials (if any), validating
/// every field. Pure; the caller persists the result.
pub fn apply_client_update(
    existing: Option<&OAuthClientCredentials>,
    update: OAuthClientUpdate,
    config: &OAuthConfig,
    loopback: &str,
) -> Result<OAuthClientCredentials, OAuthClientError> {
    let client_id = clean_field(&update.client_id, "client ID", false)?;
    if client_id.is_empty() {
        return Err(OAuthClientError::MissingClientId);
    }

    let client_secret = match update.client_secret {
        None => existing.and_then(|e| e.client_secret.clone()),
        Some(raw) => {
            let cleaned = clean_field(&raw, "client secret", false)?;
            (!cleaned.is_empty()).then_some(cleaned)
        }
    };

    let redirect_uri = match update.redirect_uri.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) => {
            validate_redirect_uri(raw)?;
            Some(raw.to_string())
        }
    };

    let creds = OAuthClientCredentials {
        client_id,
        client_secret,
        redirect_uri,
    };
    // Surface the "needs an HTTPS relay" problem at save time rather than
    // after the browser round trip.
    resolve_redirect_uri(config, &creds, loopback)?;
    Ok(creds)
}

// ---------------------------------------------------------------------------
// Redirects
// ---------------------------------------------------------------------------

/// True for `http://localhost`, `http://127.0.0.1` and `http://[::1]` URLs.
pub fn is_loopback_http(url: &Url) -> bool {
    if url.scheme() != "http" {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    // IPv6 hosts come back bracketed ("[::1]").
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Accept `https://` URLs with a host, and plain `http://` only on loopback.
/// Fragments are refused because OAuth redirect URIs must not carry one
/// (RFC 6749 §3.1.2).
pub fn validate_redirect_uri(uri: &str) -> Result<Url, OAuthClientError> {
    let url = Url::parse(uri).map_err(|e| OAuthClientError::InvalidRedirect(e.to_string()))?;
    if url.fragment().is_some() {
        return Err(OAuthClientError::InvalidRedirect(
            "fragments are not allowed".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(OAuthClientError::InvalidRedirect(
            "credentials in the URL are not allowed".into(),
        ));
    }
    let https_with_host = url.scheme() == "https" && url.host_str().is_some_and(|h| !h.is_empty());
    if https_with_host || is_loopback_http(&url) {
        Ok(url)
    } else {
        Err(OAuthClientError::InsecureRedirect)
    }
}

/// Pick the redirect URI for an authorization request: the user's override
/// when present, otherwise the loopback callback. Providers that refuse
/// loopback redirects have no default and need the override.
pub fn resolve_redirect_uri(
    config: &OAuthConfig,
    creds: &OAuthClientCredentials,
    loopback: &str,
) -> Result<String, OAuthClientError> {
    match creds
        .redirect_uri
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(custom) => {
            validate_redirect_uri(custom)?;
            Ok(custom.to_string())
        }
        None if config.requires_https_redirect => Err(OAuthClientError::RedirectRequired {
            loopback: loopback.to_string(),
        }),
        None => Ok(loopback.to_string()),
    }
}

// ---------------------------------------------------------------------------
// PKCE (RFC 7636, S256)
// ---------------------------------------------------------------------------

/// A PKCE verifier and its S256 challenge.
#[derive(Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    /// Fresh verifier from 32 random bytes (two v4 UUIDs), base64url without
    /// padding: 43 characters from the unreserved set.
    pub fn generate() -> Self {
        let mut raw = [0u8; 32];
        raw[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        raw[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        Self::from_verifier(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw))
    }

    pub fn from_verifier(verifier: String) -> Self {
        let digest = Sha256::digest(verifier.as_bytes());
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
        Self {
            verifier,
            challenge,
        }
    }
}

impl fmt::Debug for Pkce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pkce")
            .field("challenge", &self.challenge)
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Authorization URL
// ---------------------------------------------------------------------------

/// Build the provider authorization URL for the user's client.
///
/// `login_hint` is only forwarded to Google, the one provider whose account
/// picker honours it for the multi-account flow.
pub fn build_authorization_url(
    config: &OAuthConfig,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    pkce: Option<&Pkce>,
    login_hint: Option<&str>,
) -> Result<Url, OAuthClientError> {
    let mut url =
        Url::parse(config.auth_url).map_err(|e| OAuthClientError::InvalidAuthUrl(e.to_string()))?;
    let is_google = url.host_str() == Some("accounts.google.com");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", state);
        for (k, v) in config.extra_auth_params {
            q.append_pair(k, v);
        }
        if let Some(p) = pkce {
            q.append_pair("code_challenge", &p.challenge)
                .append_pair("code_challenge_method", "S256");
        }
        if let (true, Some(hint)) = (is_google, login_hint) {
            q.append_pair("login_hint", hint);
        }
    }
    Ok(url)
}

// ---------------------------------------------------------------------------
// SecretStore I/O
// ---------------------------------------------------------------------------

/// Load the BYO client for `integration_id`. `None` when no store is
/// available (CLI), nothing is saved, or the row cannot be read.
pub async fn load_oauth_client(
    store: Option<&SecretStore>,
    integration_id: &str,
) -> Option<OAuthClientCredentials> {
    let store = store?;
    match store
        .get_json::<OAuthClientCredentials>(&client_store_key(integration_id))
        .await
    {
        Ok(Some(c)) if !c.client_id.trim().is_empty() => Some(c),
        Ok(_) => None,
        Err(e) => {
            // The error comes from the store layer (decrypt/deserialize) and
            // does not contain the stored value.
            tracing::warn!(
                "oauth: could not read the OAuth client for {}: {e:#}",
                integration_id
            );
            None
        }
    }
}

/// Persist the BYO client for `integration_id`.
pub async fn save_oauth_client(
    store: &SecretStore,
    integration_id: &str,
    creds: &OAuthClientCredentials,
) -> anyhow::Result<()> {
    store
        .set_json(&client_store_key(integration_id), creds)
        .await
        .map_err(|e| anyhow::anyhow!("failed to save the OAuth client for {integration_id}: {e:#}"))
}

/// Remove the BYO client for `integration_id`. Missing rows are not an error.
pub async fn delete_oauth_client(store: &SecretStore, integration_id: &str) -> anyhow::Result<()> {
    store
        .delete(&client_store_key(integration_id))
        .await
        .map_err(|e| {
            anyhow::anyhow!("failed to remove the OAuth client for {integration_id}: {e:#}")
        })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::{TokenAuth, TokenBody, OAUTH_REDIRECT_URI};
    use std::collections::HashMap;

    const LOOPBACK: &str = OAUTH_REDIRECT_URI;

    static PLAIN: OAuthConfig = OAuthConfig {
        auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
        token_url: "https://oauth2.googleapis.com/token",
        uses_pkce: true,
        token_auth: TokenAuth::ClientSecretPost,
        token_body: TokenBody::Form,
        extra_auth_params: &[("scope", "a b"), ("access_type", "offline")],
        requires_https_redirect: false,
    };

    static HTTPS_ONLY: OAuthConfig = OAuthConfig {
        auth_url: "https://provider.example/oauth/authorize",
        token_url: "https://provider.example/oauth/token",
        uses_pkce: false,
        token_auth: TokenAuth::ClientSecretBasic,
        token_body: TokenBody::Form,
        extra_auth_params: &[],
        requires_https_redirect: true,
    };

    fn creds(id: &str, secret: Option<&str>, redirect: Option<&str>) -> OAuthClientCredentials {
        OAuthClientCredentials {
            client_id: id.into(),
            client_secret: secret.map(String::from),
            redirect_uri: redirect.map(String::from),
        }
    }

    async fn mem_store() -> SecretStore {
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        SecretStore::new(pool, None).await.unwrap()
    }

    #[test]
    fn store_key_is_outside_the_token_prefix() {
        let key = client_store_key("google-calendar");
        assert_eq!(key, "oauth_client:google-calendar");
        assert!(!key.starts_with(crate::oauth::STORE_KEY_PREFIX));
    }

    #[test]
    fn default_port_loopback_matches_the_interop_constant() {
        assert_eq!(loopback_redirect_uri(3030), OAUTH_REDIRECT_URI);
        assert_eq!(
            loopback_redirect_uri(3035),
            "http://localhost:3035/connections/oauth/callback"
        );
    }

    #[test]
    fn debug_output_never_contains_the_secret() {
        let c = creds("id-1", Some("s3cr3t-value"), None);
        let printed = format!("{c:?}");
        assert!(printed.contains("id-1"));
        assert!(!printed.contains("s3cr3t-value"));
        let u = OAuthClientUpdate {
            client_id: "id-1".into(),
            client_secret: Some("s3cr3t-value".into()),
            redirect_uri: None,
        };
        assert!(!format!("{u:?}").contains("s3cr3t-value"));
    }

    #[test]
    fn redirect_validation_accepts_https_and_loopback_only() {
        for ok in [
            "https://relay.example.com/oauth",
            "http://localhost:3030/connections/oauth/callback",
            "http://127.0.0.1:3035/connections/oauth/callback",
            "http://[::1]:3030/connections/oauth/callback",
        ] {
            assert!(validate_redirect_uri(ok).is_ok(), "{ok} should be accepted");
        }
        assert_eq!(
            validate_redirect_uri("http://relay.example.com/cb").unwrap_err(),
            OAuthClientError::InsecureRedirect
        );
        assert_eq!(
            validate_redirect_uri("myapp://callback").unwrap_err(),
            OAuthClientError::InsecureRedirect
        );
        assert!(matches!(
            validate_redirect_uri("https://relay.example.com/cb#frag"),
            Err(OAuthClientError::InvalidRedirect(_))
        ));
        assert!(matches!(
            validate_redirect_uri("not a url"),
            Err(OAuthClientError::InvalidRedirect(_))
        ));
        assert!(matches!(
            validate_redirect_uri("https://user:pw@relay.example.com/cb"),
            Err(OAuthClientError::InvalidRedirect(_))
        ));
    }

    #[test]
    fn redirect_defaults_to_loopback_unless_the_provider_needs_https() {
        let c = creds("id", None, None);
        assert_eq!(resolve_redirect_uri(&PLAIN, &c, LOOPBACK).unwrap(), LOOPBACK);
        assert_eq!(
            resolve_redirect_uri(&HTTPS_ONLY, &c, LOOPBACK).unwrap_err(),
            OAuthClientError::RedirectRequired {
                loopback: LOOPBACK.into()
            }
        );
        let relayed = creds("id", None, Some("https://relay.example.com/cb"));
        assert_eq!(
            resolve_redirect_uri(&HTTPS_ONLY, &relayed, LOOPBACK).unwrap(),
            "https://relay.example.com/cb"
        );
    }

    #[test]
    fn update_keeps_clears_or_replaces_the_secret() {
        let existing = creds("old-id", Some("old-secret"), None);
        let keep = apply_client_update(
            Some(&existing),
            OAuthClientUpdate {
                client_id: "  new-id  ".into(),
                client_secret: None,
                redirect_uri: None,
            },
            &PLAIN,
            LOOPBACK,
        )
        .unwrap();
        assert_eq!(keep.client_id, "new-id");
        assert_eq!(keep.client_secret.as_deref(), Some("old-secret"));

        let cleared = apply_client_update(
            Some(&existing),
            OAuthClientUpdate {
                client_id: "new-id".into(),
                client_secret: Some("   ".into()),
                redirect_uri: Some("".into()),
            },
            &PLAIN,
            LOOPBACK,
        )
        .unwrap();
        assert_eq!(cleared.client_secret, None);
        assert_eq!(cleared.redirect_uri, None);

        let replaced = apply_client_update(
            Some(&existing),
            OAuthClientUpdate {
                client_id: "new-id".into(),
                client_secret: Some("new-secret".into()),
                redirect_uri: Some(" http://127.0.0.1:3035/connections/oauth/callback ".into()),
            },
            &PLAIN,
            LOOPBACK,
        )
        .unwrap();
        assert_eq!(replaced.client_secret.as_deref(), Some("new-secret"));
        assert_eq!(
            replaced.redirect_uri.as_deref(),
            Some("http://127.0.0.1:3035/connections/oauth/callback")
        );
    }

    #[test]
    fn update_rejects_bad_input() {
        let upd = |id: &str, secret: Option<&str>, redirect: Option<&str>| OAuthClientUpdate {
            client_id: id.into(),
            client_secret: secret.map(String::from),
            redirect_uri: redirect.map(String::from),
        };
        assert_eq!(
            apply_client_update(None, upd("  ", None, None), &PLAIN, LOOPBACK).unwrap_err(),
            OAuthClientError::MissingClientId
        );
        assert_eq!(
            apply_client_update(None, upd("a b", None, None), &PLAIN, LOOPBACK).unwrap_err(),
            OAuthClientError::InvalidCharacters { field: "client ID" }
        );
        assert_eq!(
            apply_client_update(None, upd("id", Some("x\ny"), None), &PLAIN, LOOPBACK)
                .unwrap_err(),
            OAuthClientError::InvalidCharacters {
                field: "client secret"
            }
        );
        assert_eq!(
            apply_client_update(None, upd("id", None, Some("http://evil.example/cb")), &PLAIN, LOOPBACK)
                .unwrap_err(),
            OAuthClientError::InsecureRedirect
        );
        assert!(matches!(
            apply_client_update(None, upd("id", None, None), &HTTPS_ONLY, LOOPBACK).unwrap_err(),
            OAuthClientError::RedirectRequired { .. }
        ));
        assert_eq!(
            apply_client_update(None, upd(&"x".repeat(MAX_FIELD_LEN + 1), None, None), &PLAIN, LOOPBACK)
                .unwrap_err(),
            OAuthClientError::TooLong { field: "client ID" }
        );
    }

    #[test]
    fn pkce_matches_the_rfc_7636_appendix_b_vector() {
        let p = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());
        assert_eq!(p.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn generated_pkce_verifier_is_well_formed_and_unique() {
        let a = Pkce::generate();
        let b = Pkce::generate();
        assert_eq!(a.verifier.len(), 43);
        assert!(a
            .verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~')));
        assert_ne!(a.verifier, b.verifier);
        assert!(!format!("{a:?}").contains(&a.verifier));
    }

    #[test]
    fn authorization_url_carries_client_redirect_state_and_pkce() {
        let pkce = Pkce::from_verifier("v".repeat(43));
        let url = build_authorization_url(
            &PLAIN,
            "my-client.apps.example",
            LOOPBACK,
            "state-123",
            Some(&pkce),
            Some("me@example.com"),
        )
        .unwrap();
        let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "my-client.apps.example");
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["redirect_uri"], LOOPBACK);
        assert_eq!(q["state"], "state-123");
        assert_eq!(q["scope"], "a b");
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["code_challenge"], pkce.challenge);
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["login_hint"], "me@example.com");
        assert!(!url.as_str().contains(&pkce.verifier));
    }

    #[test]
    fn authorization_url_without_pkce_or_google_has_no_extras() {
        let url = build_authorization_url(
            &HTTPS_ONLY,
            "cid",
            "https://relay.example.com/cb",
            "s",
            None,
            Some("me@example.com"),
        )
        .unwrap();
        let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert!(!q.contains_key("code_challenge"));
        assert!(!q.contains_key("login_hint"));
        assert_eq!(q["redirect_uri"], "https://relay.example.com/cb");
    }

    #[tokio::test]
    async fn store_round_trip_and_delete() {
        let store = mem_store().await;
        assert!(load_oauth_client(Some(&store), "_t_byo").await.is_none());
        assert!(load_oauth_client(None, "_t_byo").await.is_none());

        let c = creds("cid", Some("csecret"), Some("https://relay.example.com/cb"));
        save_oauth_client(&store, "_t_byo", &c).await.unwrap();
        assert_eq!(load_oauth_client(Some(&store), "_t_byo").await, Some(c));

        // Tokens and clients live in separate key families.
        let token_keys = store.list(crate::oauth::STORE_KEY_PREFIX).await.unwrap();
        assert!(token_keys.is_empty(), "client row leaked into oauth: keys: {token_keys:?}");

        delete_oauth_client(&store, "_t_byo").await.unwrap();
        assert!(load_oauth_client(Some(&store), "_t_byo").await.is_none());
        // Deleting again is fine.
        delete_oauth_client(&store, "_t_byo").await.unwrap();
    }

    #[tokio::test]
    async fn stored_json_uses_the_documented_field_names() {
        let store = mem_store().await;
        save_oauth_client(&store, "_t_shape", &creds("cid", None, None))
            .await
            .unwrap();
        let raw: serde_json::Value = store
            .get_json("oauth_client:_t_shape")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(raw, serde_json::json!({ "client_id": "cid" }));
    }
}
