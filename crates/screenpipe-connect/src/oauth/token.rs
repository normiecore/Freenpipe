// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Direct calls to a provider's OAuth token endpoint (RFC 6749 §4.1.3, §6)
//! with the user's own client credentials, plus mapping of the many shapes
//! providers use to report failure into one error type.

use std::fmt;

use serde_json::{Map, Value};

use super::client::OAuthClientCredentials;
use super::{TokenAuth, TokenBody};

/// Where and how to send token requests. Built from an
/// [`super::OAuthConfig`] in production and pointed at a mock server in tests.
#[derive(Debug, Clone, Copy)]
pub struct TokenEndpoint<'a> {
    pub url: &'a str,
    pub auth: TokenAuth,
    pub body: TokenBody,
}

/// Why a token request failed. Display strings are safe to log and to show
/// the user: they never include tokens, codes or the client secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OAuthTokenError {
    #[error(
        "no OAuth app is configured for {integration_id}: add the client ID (and secret) of an \
         OAuth app you registered with the provider in Settings > Connections"
    )]
    ClientNotConfigured { integration_id: String },
    #[error("{integration_id} does not use OAuth")]
    UnknownIntegration { integration_id: String },
    /// The code or refresh token is expired, revoked or already used.
    /// Retrying with the same grant cannot succeed; the user has to reconnect.
    #[error("the provider rejected the grant ({code}){}", describe(.description))]
    InvalidGrant {
        status: u16,
        code: String,
        description: Option<String>,
    },
    /// The client ID or secret is wrong, or the app is not allowed to use
    /// this grant. Fixing the OAuth app settings is the only remedy.
    #[error("the provider rejected the OAuth app credentials ({code}){}", describe(.description))]
    InvalidClient {
        status: u16,
        code: String,
        description: Option<String>,
    },
    #[error("the token endpoint returned HTTP {status} ({code}){}", describe(.description))]
    Provider {
        status: u16,
        code: String,
        description: Option<String>,
    },
    #[error("could not reach the token endpoint: {0}")]
    Transport(String),
    #[error("the token endpoint returned HTTP {status} with a body that is neither JSON nor form-encoded")]
    MalformedResponse { status: u16 },
    #[error("the token endpoint answered without an access_token")]
    MissingAccessToken,
}

fn describe(description: &Option<String>) -> String {
    match description.as_deref() {
        Some(d) if !d.is_empty() => format!(": {d}"),
        _ => String::new(),
    }
}

impl OAuthTokenError {
    pub fn is_invalid_grant(&self) -> bool {
        matches!(self, Self::InvalidGrant { .. })
    }
}

/// Error codes that mean "this code / refresh token is dead". The standard
/// one is `invalid_grant`; the rest are provider dialects (GitHub, Slack).
const INVALID_GRANT_CODES: &[&str] = &[
    "invalid_grant",
    "bad_verification_code",
    "bad_refresh_token",
    "invalid_code",
    "code_already_used",
    "code_expired",
    "invalid_refresh_token",
    "expired_refresh_token",
    "token_revoked",
];

/// Error codes that mean "the client credentials are wrong".
const INVALID_CLIENT_CODES: &[&str] = &[
    "invalid_client",
    "unauthorized_client",
    "incorrect_client_credentials",
    "invalid_client_id",
    "bad_client_secret",
    "invalid_client_secret",
];

const DESCRIPTION_LIMIT: usize = 300;

fn short(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.chars().count() <= DESCRIPTION_LIMIT {
        cleaned.to_string()
    } else {
        let cut: String = cleaned.chars().take(DESCRIPTION_LIMIT).collect();
        format!("{cut}…")
    }
}

/// Parse a token endpoint body. JSON is the norm; GitHub (without an Accept
/// header) and a few older servers answer `application/x-www-form-urlencoded`.
fn parse_body(body: &str) -> Option<Map<String, Value>> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(trimmed) {
        return Some(map);
    }
    if trimmed.starts_with('<') || trimmed.starts_with('{') || !trimmed.contains('=') {
        return None;
    }
    let mut map = Map::new();
    for (k, v) in form::parse(trimmed.as_bytes()) {
        // Real form keys are identifiers; anything else is prose that
        // happens to contain '=' and is better treated as plain text.
        if k.is_empty()
            || !k
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        {
            return None;
        }
        let value = match (k.as_ref(), v.parse::<u64>()) {
            ("expires_in" | "refresh_token_expires_in", Ok(n)) => Value::from(n),
            _ => Value::String(v.into_owned()),
        };
        map.insert(k.into_owned(), value);
    }
    (!map.is_empty()).then_some(map)
}

/// Pull `(code, description)` out of an error body. Handles RFC 6749
/// (`error` + `error_description`), Slack (`ok:false`, `error`), nested
/// `error: {code, message}` objects and `message`/`reason` fallbacks.
fn error_fields(map: &Map<String, Value>) -> (Option<String>, Option<String>) {
    let mut code = None;
    let mut description = None;
    match map.get("error") {
        Some(Value::String(s)) => code = Some(s.clone()),
        Some(Value::Object(inner)) => {
            code = inner
                .get("code")
                .or_else(|| inner.get("type"))
                .and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                });
            description = inner
                .get("message")
                .and_then(Value::as_str)
                .map(String::from);
        }
        _ => {}
    }
    if code.is_none() {
        code = ["errorCode", "error_code", "code"]
            .iter()
            .find_map(|k| map.get(*k).and_then(Value::as_str).map(String::from));
    }
    if description.is_none() {
        description = ["error_description", "message", "reason", "errorMessage"]
            .iter()
            .find_map(|k| map.get(*k).and_then(Value::as_str).map(String::from));
    }
    (code, description.map(|d| short(&d)))
}

fn classify(status: u16, code: String, description: Option<String>) -> OAuthTokenError {
    let lower = code.to_ascii_lowercase();
    if INVALID_GRANT_CODES.contains(&lower.as_str()) {
        OAuthTokenError::InvalidGrant {
            status,
            code,
            description,
        }
    } else if INVALID_CLIENT_CODES.contains(&lower.as_str()) {
        OAuthTokenError::InvalidClient {
            status,
            code,
            description,
        }
    } else {
        OAuthTokenError::Provider {
            status,
            code,
            description,
        }
    }
}

/// Turn a token endpoint response into the token JSON or a typed error.
/// Pure, so every provider dialect is covered by unit tests.
pub fn interpret_token_response(status: u16, body: &str) -> Result<Value, OAuthTokenError> {
    let success = (200..300).contains(&status);
    let parsed = parse_body(body);

    let Some(map) = parsed else {
        if success {
            // Do not echo the body: on a 2xx it may well hold a token.
            return Err(OAuthTokenError::MalformedResponse { status });
        }
        // Error pages (HTML, plain text) carry no secrets; keep a snippet.
        // Only a 400/401 that names a standard code is trusted to mean it:
        // a 5xx page mentioning `invalid_grant` is a server problem.
        let lower = body.to_ascii_lowercase();
        let client_error = status == 400 || status == 401;
        let code = if client_error && lower.contains("invalid_grant") {
            "invalid_grant".to_string()
        } else if client_error && lower.contains("invalid_client") {
            "invalid_client".to_string()
        } else {
            format!("http_{status}")
        };
        let snippet = short(body);
        return Err(classify(
            status,
            code,
            (!snippet.is_empty()).then_some(snippet),
        ));
    };

    // Slack answers 200 with `ok:false`; GitHub answers 200 with `error`.
    let reported_failure = map.get("ok") == Some(&Value::Bool(false))
        || (map.get("error").is_some() && map.get("access_token").is_none());

    if success && !reported_failure {
        return match map.get("access_token") {
            Some(Value::String(t)) if !t.is_empty() => Ok(Value::Object(map)),
            _ => Err(OAuthTokenError::MissingAccessToken),
        };
    }

    let (code, description) = error_fields(&map);
    Err(classify(
        status,
        code.unwrap_or_else(|| format!("http_{status}")),
        description,
    ))
}

/// The grant being redeemed. Kept private so only the two public entry
/// points below can build a request.
enum Grant<'a> {
    AuthorizationCode {
        code: &'a str,
        redirect_uri: &'a str,
        code_verifier: Option<&'a str>,
    },
    RefreshToken {
        refresh_token: &'a str,
    },
}

impl fmt::Debug for Grant<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Grant::AuthorizationCode { .. } => "authorization_code",
            Grant::RefreshToken { .. } => "refresh_token",
        })
    }
}

async fn post_token_request(
    client: &reqwest::Client,
    endpoint: TokenEndpoint<'_>,
    creds: &OAuthClientCredentials,
    grant: Grant<'_>,
) -> Result<Value, OAuthTokenError> {
    let mut params: Vec<(&str, &str)> = Vec::with_capacity(6);
    match &grant {
        Grant::AuthorizationCode {
            code,
            redirect_uri,
            code_verifier,
        } => {
            params.push(("grant_type", "authorization_code"));
            params.push(("code", code));
            params.push(("redirect_uri", redirect_uri));
            if let Some(v) = code_verifier {
                params.push(("code_verifier", v));
            }
        }
        Grant::RefreshToken { refresh_token } => {
            params.push(("grant_type", "refresh_token"));
            params.push(("refresh_token", refresh_token));
        }
    }

    let secret = creds.client_secret.as_deref().filter(|s| !s.is_empty());
    let use_basic = matches!(endpoint.auth, TokenAuth::ClientSecretBasic) && secret.is_some();
    if !use_basic {
        // Public clients (no secret) always identify themselves in the body.
        params.push(("client_id", &creds.client_id));
        if let Some(s) = secret {
            params.push(("client_secret", s));
        }
    }

    let mut req = client
        .post(endpoint.url)
        .header(reqwest::header::ACCEPT, "application/json");
    if use_basic {
        req = req.basic_auth(&creds.client_id, secret);
    }
    req = match endpoint.body {
        TokenBody::Form => req.form(&params),
        TokenBody::Json => {
            let obj: Map<String, Value> = params
                .iter()
                .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
                .collect();
            req.json(&obj)
        }
    };

    let resp = req
        .send()
        .await
        // reqwest's message names the URL (no credentials: they are in the
        // body or a header) and the transport failure.
        .map_err(|e| OAuthTokenError::Transport(e.without_url().to_string()))?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| OAuthTokenError::Transport(e.without_url().to_string()))?;
    let result = interpret_token_response(status, &body);
    if let Err(e) = &result {
        tracing::debug!(grant = ?grant, status, "oauth token request failed: {e}");
    }
    result
}

/// Redeem an authorization code at the provider's token endpoint.
pub async fn exchange_code(
    client: &reqwest::Client,
    endpoint: TokenEndpoint<'_>,
    creds: &OAuthClientCredentials,
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
) -> Result<Value, OAuthTokenError> {
    post_token_request(
        client,
        endpoint,
        creds,
        Grant::AuthorizationCode {
            code,
            redirect_uri,
            code_verifier,
        },
    )
    .await
}

/// Trade a refresh token for a new access token. The raw response is
/// returned; callers merge it over the stored token so fields the provider
/// does not echo (refresh_token, identity metadata) survive.
pub async fn request_refresh(
    client: &reqwest::Client,
    endpoint: TokenEndpoint<'_>,
    creds: &OAuthClientCredentials,
    refresh_token: &str,
) -> Result<Value, OAuthTokenError> {
    post_token_request(
        client,
        endpoint,
        creds,
        Grant::RefreshToken { refresh_token },
    )
    .await
}

/// Minimal `application/x-www-form-urlencoded` decoder, so this crate does
/// not need another dependency for one call.
mod form {
    use std::borrow::Cow;

    pub fn parse(input: &[u8]) -> Vec<(Cow<'_, str>, Cow<'_, str>)> {
        input
            .split(|b| *b == b'&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let mut it = pair.splitn(2, |b| *b == b'=');
                let k = it.next().unwrap_or_default();
                let v = it.next().unwrap_or_default();
                (decode(k), decode(v))
            })
            .collect()
    }

    fn decode(raw: &[u8]) -> Cow<'_, str> {
        let mut out = Vec::with_capacity(raw.len());
        let mut i = 0;
        while i < raw.len() {
            match raw[i] {
                b'+' => out.push(b' '),
                b'%' if i + 2 < raw.len() => {
                    let hex = std::str::from_utf8(&raw[i + 1..i + 3]).ok();
                    match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                        Some(b) => {
                            out.push(b);
                            i += 2;
                        }
                        None => out.push(b'%'),
                    }
                }
                b => out.push(b),
            }
            i += 1;
        }
        Cow::Owned(String::from_utf8_lossy(&out).into_owned())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::json;
    use std::collections::HashMap;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

    fn creds(secret: Option<&str>) -> OAuthClientCredentials {
        OAuthClientCredentials {
            client_id: "client-123".into(),
            client_secret: secret.map(String::from),
            redirect_uri: None,
        }
    }

    fn endpoint(
        server: &MockServer,
        auth: TokenAuth,
        body: TokenBody,
    ) -> (String, TokenAuth, TokenBody) {
        (format!("{}/oauth/token", server.uri()), auth, body)
    }

    fn form_of(req: &Request) -> HashMap<String, String> {
        form::parse(&req.body)
            .into_iter()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }

    /// Matches a form body that contains every `(key, value)` pair and none
    /// of the `absent` keys.
    struct FormBody {
        present: Vec<(&'static str, &'static str)>,
        absent: Vec<&'static str>,
    }

    impl Match for FormBody {
        fn matches(&self, req: &Request) -> bool {
            let form = form_of(req);
            self.present
                .iter()
                .all(|(k, v)| form.get(*k).map(String::as_str) == Some(*v))
                && self.absent.iter().all(|k| !form.contains_key(*k))
        }
    }

    fn basic(id: &str, secret: &str) -> String {
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"))
        )
    }

    // -- interpret_token_response (pure) -------------------------------------

    #[test]
    fn success_json_is_returned_as_is() {
        let v = interpret_token_response(
            200,
            r#"{"access_token":"at","refresh_token":"rt","expires_in":3599}"#,
        )
        .unwrap();
        assert_eq!(v["access_token"], "at");
        assert_eq!(v["expires_in"], 3599);
    }

    #[test]
    fn success_form_body_is_parsed_with_numeric_expiry() {
        let v = interpret_token_response(
            200,
            "access_token=gho_abc&scope=repo%2Cread%3Aorg&token_type=bearer&expires_in=28800",
        )
        .unwrap();
        assert_eq!(v["access_token"], "gho_abc");
        assert_eq!(v["scope"], "repo,read:org");
        assert_eq!(v["expires_in"], 28800);
    }

    #[test]
    fn rfc6749_invalid_grant_maps_to_invalid_grant() {
        let e = interpret_token_response(
            400,
            r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#,
        )
        .unwrap_err();
        assert_eq!(
            e,
            OAuthTokenError::InvalidGrant {
                status: 400,
                code: "invalid_grant".into(),
                description: Some("Token has been expired or revoked.".into()),
            }
        );
        assert!(e.is_invalid_grant());
        assert!(e.to_string().contains("expired or revoked"));
    }

    #[test]
    fn invalid_client_maps_to_invalid_client() {
        let e = interpret_token_response(401, r#"{"error":"invalid_client"}"#).unwrap_err();
        assert!(matches!(
            e,
            OAuthTokenError::InvalidClient { status: 401, .. }
        ));
    }

    #[test]
    fn slack_ok_false_on_http_200_is_an_error() {
        let e =
            interpret_token_response(200, r#"{"ok":false,"error":"invalid_code"}"#).unwrap_err();
        assert!(e.is_invalid_grant(), "{e:?}");
        let e = interpret_token_response(200, r#"{"ok":false,"error":"bad_client_secret"}"#)
            .unwrap_err();
        assert!(matches!(e, OAuthTokenError::InvalidClient { .. }), "{e:?}");
    }

    #[test]
    fn github_error_on_http_200_is_an_error() {
        let e = interpret_token_response(
            200,
            "error=bad_verification_code&error_description=The+code+passed+is+incorrect+or+expired.",
        )
        .unwrap_err();
        assert_eq!(
            e,
            OAuthTokenError::InvalidGrant {
                status: 200,
                code: "bad_verification_code".into(),
                description: Some("The code passed is incorrect or expired.".into()),
            }
        );
    }

    #[test]
    fn nested_error_objects_and_unknown_codes_map_to_provider() {
        let e = interpret_token_response(
            403,
            r#"{"error":{"code":"forbidden","message":"app not approved"}}"#,
        )
        .unwrap_err();
        assert_eq!(
            e,
            OAuthTokenError::Provider {
                status: 403,
                code: "forbidden".into(),
                description: Some("app not approved".into()),
            }
        );
        let e = interpret_token_response(
            400,
            r#"{"reason":"Invalid Token!","error":"invalid_request"}"#,
        )
        .unwrap_err();
        assert!(
            matches!(e, OAuthTokenError::Provider { ref code, .. } if code == "invalid_request")
        );
    }

    #[test]
    fn html_error_pages_keep_a_short_snippet() {
        let page = format!("<html><body>{}</body></html>", "x".repeat(1000));
        let e = interpret_token_response(502, &page).unwrap_err();
        match e {
            OAuthTokenError::Provider {
                status,
                code,
                description,
            } => {
                assert_eq!(status, 502);
                assert_eq!(code, "http_502");
                assert!(description.unwrap().chars().count() <= DESCRIPTION_LIMIT + 1);
            }
            other => panic!("unexpected {other:?}"),
        }
        // Prose containing '=' is not mistaken for a form body.
        match interpret_token_response(400, "Bad request: redirect_uri mismatch (a=b)") {
            Err(OAuthTokenError::Provider { description, .. }) => {
                assert!(description.unwrap().contains("redirect_uri mismatch"));
            }
            other => panic!("unexpected {other:?}"),
        }
        // A plain-text body that names invalid_grant is still a dead grant.
        assert!(interpret_token_response(400, "upstream said INVALID_GRANT")
            .unwrap_err()
            .is_invalid_grant());
    }

    #[test]
    fn unreadable_success_body_is_not_echoed() {
        let e = interpret_token_response(200, "<html>sekret-token</html>").unwrap_err();
        assert_eq!(e, OAuthTokenError::MalformedResponse { status: 200 });
        assert!(!e.to_string().contains("sekret"));
    }

    #[test]
    fn success_without_access_token_is_rejected() {
        let e = interpret_token_response(200, r#"{"token_type":"bearer"}"#).unwrap_err();
        assert_eq!(e, OAuthTokenError::MissingAccessToken);
    }

    #[test]
    fn descriptions_are_trimmed_and_stripped_of_control_characters() {
        let long = format!("line1\nline2{}", "y".repeat(500));
        let body = json!({ "error": "invalid_request", "error_description": long }).to_string();
        match interpret_token_response(400, &body).unwrap_err() {
            OAuthTokenError::Provider { description, .. } => {
                let d = description.unwrap();
                assert!(!d.contains('\n'));
                assert!(d.ends_with('…'));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    // -- wiremock: real HTTP round trips --------------------------------------

    #[tokio::test]
    async fn code_exchange_with_pkce_posts_a_standard_form() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(header("accept", "application/json"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(FormBody {
                present: vec![
                    ("grant_type", "authorization_code"),
                    ("code", "auth-code"),
                    (
                        "redirect_uri",
                        "http://localhost:3030/connections/oauth/callback",
                    ),
                    ("code_verifier", "verifier-xyz"),
                    ("client_id", "client-123"),
                    ("client_secret", "shh"),
                ],
                absent: vec![],
            })
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at-1",
                "refresh_token": "rt-1",
                "expires_in": 3600,
                "token_type": "Bearer",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (url, auth, body) = endpoint(&server, TokenAuth::ClientSecretPost, TokenBody::Form);
        let ep = TokenEndpoint {
            url: &url,
            auth,
            body,
        };
        let v = exchange_code(
            &reqwest::Client::new(),
            ep,
            &creds(Some("shh")),
            "auth-code",
            "http://localhost:3030/connections/oauth/callback",
            Some("verifier-xyz"),
        )
        .await
        .unwrap();
        assert_eq!(v["access_token"], "at-1");
        assert_eq!(v["refresh_token"], "rt-1");
    }

    #[tokio::test]
    async fn public_client_sends_client_id_without_secret() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(FormBody {
                present: vec![("client_id", "client-123"), ("code_verifier", "v")],
                absent: vec!["client_secret"],
            })
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "access_token": "at" })))
            .expect(1)
            .mount(&server)
            .await;

        // Basic auth is configured, but with no secret the client id goes in
        // the body and no Authorization header is sent.
        let (url, auth, body) = endpoint(&server, TokenAuth::ClientSecretBasic, TokenBody::Form);
        let ep = TokenEndpoint {
            url: &url,
            auth,
            body,
        };
        exchange_code(
            &reqwest::Client::new(),
            ep,
            &creds(None),
            "c",
            "http://localhost/cb",
            Some("v"),
        )
        .await
        .unwrap();
        let reqs = server.received_requests().await.unwrap();
        assert!(reqs[0].headers.get("authorization").is_none());
    }

    #[tokio::test]
    async fn basic_auth_keeps_credentials_out_of_the_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(header("authorization", basic("client-123", "shh").as_str()))
            .and(FormBody {
                present: vec![("grant_type", "refresh_token"), ("refresh_token", "rt-1")],
                absent: vec!["client_id", "client_secret"],
            })
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "access_token": "at-2" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (url, auth, body) = endpoint(&server, TokenAuth::ClientSecretBasic, TokenBody::Form);
        let ep = TokenEndpoint {
            url: &url,
            auth,
            body,
        };
        let v = request_refresh(&reqwest::Client::new(), ep, &creds(Some("shh")), "rt-1")
            .await
            .unwrap();
        assert_eq!(v["access_token"], "at-2");
    }

    #[tokio::test]
    async fn json_body_providers_get_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(header("content-type", "application/json"))
            .and(wiremock::matchers::body_json(json!({
                "grant_type": "authorization_code",
                "code": "c",
                "redirect_uri": "http://localhost:3030/connections/oauth/callback",
            })))
            .and(header("authorization", basic("client-123", "shh").as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "notion-at",
                "workspace_name": "Acme",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (url, auth, body) = endpoint(&server, TokenAuth::ClientSecretBasic, TokenBody::Json);
        let ep = TokenEndpoint {
            url: &url,
            auth,
            body,
        };
        let v = exchange_code(
            &reqwest::Client::new(),
            ep,
            &creds(Some("shh")),
            "c",
            "http://localhost:3030/connections/oauth/callback",
            None,
        )
        .await
        .unwrap();
        assert_eq!(v["workspace_name"], "Acme");
    }

    #[tokio::test]
    async fn refresh_rejection_maps_to_invalid_grant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_grant",
                "error_description": "refresh token revoked",
            })))
            .mount(&server)
            .await;

        let (url, auth, body) = endpoint(&server, TokenAuth::ClientSecretPost, TokenBody::Form);
        let ep = TokenEndpoint {
            url: &url,
            auth,
            body,
        };
        let e = request_refresh(&reqwest::Client::new(), ep, &creds(Some("shh")), "dead")
            .await
            .unwrap_err();
        assert!(e.is_invalid_grant(), "{e:?}");
        assert!(!e.to_string().contains("shh"));
        assert!(!e.to_string().contains("dead"));
    }

    #[tokio::test]
    async fn wrong_client_secret_maps_to_invalid_client() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": "invalid_client",
                "error_description": "Unauthorized",
            })))
            .mount(&server)
            .await;

        let (url, auth, body) = endpoint(&server, TokenAuth::ClientSecretPost, TokenBody::Form);
        let ep = TokenEndpoint {
            url: &url,
            auth,
            body,
        };
        let e = exchange_code(
            &reqwest::Client::new(),
            ep,
            &creds(Some("wrong")),
            "c",
            "http://localhost/cb",
            None,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(e, OAuthTokenError::InvalidClient { status: 401, .. }),
            "{e:?}"
        );
    }

    #[tokio::test]
    async fn unreachable_endpoint_is_a_transport_error() {
        // Reserve a free port, then close it so nothing is listening there.
        // (A dropped wiremock server goes back to its pool and keeps answering.)
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let url = format!("http://127.0.0.1:{port}/oauth/token");
        let ep = TokenEndpoint {
            url: &url,
            auth: TokenAuth::ClientSecretPost,
            body: TokenBody::Form,
        };
        let e = request_refresh(&reqwest::Client::new(), ep, &creds(None), "rt")
            .await
            .unwrap_err();
        assert!(matches!(e, OAuthTokenError::Transport(_)), "{e:?}");
    }

    #[test]
    fn form_decoder_handles_escapes_and_edge_cases() {
        let parsed: HashMap<String, String> = form::parse(b"a=1+2&b=%3D%26&c=&d&e=%zz&&f=%4")
            .into_iter()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(parsed["a"], "1 2");
        assert_eq!(parsed["b"], "=&");
        assert_eq!(parsed["c"], "");
        assert_eq!(parsed["d"], "");
        assert_eq!(parsed["e"], "%zz");
        assert_eq!(parsed["f"], "%4");
    }
}
