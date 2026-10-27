// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Scrubbing of credentials from event payloads and agent events before they
//! are written to disk (`.trigger-context.json`, trajectories) or shown to an
//! agent.
//!
//! Two passes:
//!
//! - **By field name.** A JSON field whose name says it holds a credential
//!   (`token`, `api_key`, `authorization`, `client_secret`, `password`,
//!   `cookie`, ...) has its whole value replaced by [`REDACTED`]. Names are
//!   compared case-insensitively with `_`, `-`, `.` and spaces removed, so
//!   `apiKey`, `api-key` and `X-Api-Key` are the same name. Counters such as
//!   `max_tokens` are left alone.
//! - **By shape.** Every remaining string is scanned for credentials that
//!   can sit inside ordinary text: `Bearer <token>` and `Basic <token>`
//!   headers, user:password in a URL, credential-named query parameters
//!   (`?access_token=...`) and keys with a well-known vendor prefix
//!   (`sk-`, `ghp_`, `xoxb-`, `AKIA`, ...).
//!
//! The scrub is conservative about what it keeps: it prefers losing a
//! harmless value to writing a live credential to disk.

use serde_json::{Map, Value};

/// What a removed credential is replaced with.
pub const REDACTED: &str = "[redacted]";

/// Field names (normalised) whose value is always a credential.
const SECRET_NAMES: &[&str] = &[
    "auth",
    "authorization",
    "proxyauthorization",
    "cookie",
    "cookies",
    "setcookie",
    "password",
    "passwd",
    "pwd",
    "passphrase",
    "secret",
    "secrets",
    "token",
    "apikey",
    "credential",
    "credentials",
    "otp",
    "totp",
    "mfacode",
    "pincode",
    "dsn",
    "connectionstring",
];

/// Endings (normalised) that mark a credential field: `access_token`,
/// `client_secret`, `x-api-key`, `aws_secret_access_key`, ...
const SECRET_SUFFIXES: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "passphrase",
    "apikey",
    "privatekey",
    "secretkey",
    "accesskey",
    "signingkey",
    "encryptionkey",
    "sessionkey",
    "licensekey",
    "masterkey",
    "credential",
    "credentials",
    "cookie",
];

/// Query parameter names (lower case) whose value is a credential.
const SECRET_QUERY_PARAMS: &[&str] = &[
    "token",
    "access_token",
    "refresh_token",
    "id_token",
    "api_key",
    "apikey",
    "key",
    "secret",
    "client_secret",
    "password",
    "pass",
    "pwd",
    "auth",
    "sig",
    "signature",
    "x-amz-signature",
    "x-amz-credential",
    "x-amz-security-token",
    "code",
];

/// Vendor key prefixes and the shortest run of key characters after the
/// prefix that counts as a key.
const KEY_PREFIXES: &[(&str, usize)] = &[
    ("sk-", 16),
    ("pk_live_", 12),
    ("sk_live_", 12),
    ("rk_live_", 12),
    ("ghp_", 20),
    ("gho_", 20),
    ("ghu_", 20),
    ("ghs_", 20),
    ("ghr_", 20),
    ("github_pat_", 20),
    ("glpat-", 16),
    ("xoxb-", 10),
    ("xoxp-", 10),
    ("xoxa-", 10),
    ("xoxr-", 10),
    ("xoxs-", 10),
    ("xapp-", 10),
    ("AKIA", 16),
    ("ASIA", 16),
    ("AIza", 30),
    ("ya29.", 20),
    ("hf_", 20),
    ("lin_api_", 20),
    ("ntn_", 20),
    ("secret_", 30),
];

/// Whether a JSON field name says its value is a credential.
pub fn is_secret_field(name: &str) -> bool {
    let norm: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if norm.is_empty() {
        return false;
    }
    // Counters and limits ("max_tokens", "token_count", "tokens_used").
    if norm.ends_with("tokens") || norm.ends_with("count") || norm.starts_with("max") {
        return false;
    }
    SECRET_NAMES.contains(&norm.as_str()) || SECRET_SUFFIXES.iter().any(|s| norm.ends_with(s))
}

/// A copy of `value` with credentials removed (see the module docs).
pub fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                if is_secret_field(k) && !v.is_null() {
                    out.insert(k.clone(), Value::String(REDACTED.to_string()));
                } else {
                    out.insert(k.clone(), redact_value(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        Value::String(s) => Value::String(redact_text(s)),
        other => other.clone(),
    }
}

/// `text` with credentials that can appear inside free text removed.
pub fn redact_text(text: &str) -> String {
    // Cheap exit for the common case: nothing that could start a credential.
    if !could_hold_secret(text) {
        return text.to_string();
    }
    let text = redact_auth_schemes(text);
    let text = redact_url_userinfo(&text);
    let text = redact_query_params(&text);
    redact_prefixed_keys(&text)
}

fn could_hold_secret(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("bearer")
        || lower.contains("basic ")
        || lower.contains("://")
        || lower.contains('=')
        || KEY_PREFIXES
            .iter()
            .any(|(p, _)| text.contains(p) || lower.contains(&p.to_ascii_lowercase()))
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '+' | '/' | '=')
}

/// `Bearer xyz` / `Basic xyz` (any case) keep the scheme, lose the value.
fn redact_auth_schemes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let lower = text.to_ascii_lowercase();
    let mut i = 0;
    while i < text.len() {
        let rest = &lower[i..];
        let scheme_len = if rest.starts_with("bearer ") {
            Some(7)
        } else if rest.starts_with("basic ") {
            Some(6)
        } else {
            None
        };
        let at_word_start = i == 0
            || !text[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric());
        if let (Some(len), true) = (scheme_len, at_word_start) {
            let after = i + len;
            let value_len: usize = text[after..]
                .chars()
                .take_while(|c| is_token_char(*c))
                .map(char::len_utf8)
                .sum();
            // A credential, not an English word ("bearer instruments").
            let value = &text[after..after + value_len];
            if value_len >= 12 && value.chars().any(|c| c.is_ascii_digit()) {
                out.push_str(&text[i..after]);
                out.push_str(REDACTED);
                i = after + value_len;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// `scheme://user:password@host` becomes `scheme://[redacted]@host`.
fn redact_url_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("://") {
        let (head, tail) = rest.split_at(pos + 3);
        out.push_str(head);
        // The authority runs to the first character that ends it.
        let authority_len = tail
            .find(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace() || c == '"')
            .unwrap_or(tail.len());
        let authority = &tail[..authority_len];
        match authority.rfind('@') {
            Some(at) if at > 0 => {
                out.push_str(REDACTED);
                out.push_str(&authority[at..]);
            }
            _ => out.push_str(authority),
        }
        rest = &tail[authority_len..];
    }
    out.push_str(rest);
    out
}

/// `?access_token=abc&x=1` becomes `?access_token=[redacted]&x=1`.
fn redact_query_params(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < text.len() {
        let c = bytes[i];
        if c == b'?' || c == b'&' {
            out.push(c as char);
            i += 1;
            let name_len = text[i..]
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
                .unwrap_or(text.len() - i);
            let name = &text[i..i + name_len];
            let after_name = i + name_len;
            if name_len > 0
                && text[after_name..].starts_with('=')
                && SECRET_QUERY_PARAMS.contains(&name.to_ascii_lowercase().as_str())
            {
                out.push_str(name);
                out.push('=');
                let value_start = after_name + 1;
                let value_len = text[value_start..]
                    .find(|ch: char| {
                        matches!(ch, '&' | '#' | '"' | '\'' | '<' | '>') || ch.is_whitespace()
                    })
                    .unwrap_or(text.len() - value_start);
                if value_len > 0 {
                    out.push_str(REDACTED);
                }
                i = value_start + value_len;
            }
            continue;
        }
        let ch = text[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Keys with a well-known vendor prefix, when the prefix starts a word and
/// enough key characters follow it.
fn redact_prefixed_keys(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let at_word_start = i == 0
            || !text[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if at_word_start {
            let rest = &text[i..];
            let hit = KEY_PREFIXES.iter().find_map(|(prefix, min)| {
                if !rest.starts_with(prefix) {
                    return None;
                }
                let body: usize = rest[prefix.len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                    .count();
                (body >= *min).then_some(prefix.len() + body)
            });
            if let Some(len) = hit {
                out.push_str(REDACTED);
                i += len;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn credential_fields_are_blanked_whatever_their_spelling() {
        let v = json!({
            "source_delivery_id": "slack:C1:171.2",
            "pipe_name": "digest",
            "access_token": "xyz-very-secret",
            "apiKey": "abc",
            "X-Api-Key": "abc",
            "headers": {"Authorization": "Bearer abcdefghijkl", "Accept": "json"},
            "client_secret": {"nested": "value"},
            "password": 1234,
            "max_tokens": 4000,
            "token_count": 12,
            "key": "42",
            "note": null,
            "token": null
        });
        let r = redact_value(&v);
        assert_eq!(r["source_delivery_id"], "slack:C1:171.2");
        assert_eq!(r["pipe_name"], "digest");
        assert_eq!(r["access_token"], REDACTED);
        assert_eq!(r["apiKey"], REDACTED);
        assert_eq!(r["X-Api-Key"], REDACTED);
        assert_eq!(r["headers"]["Authorization"], REDACTED);
        assert_eq!(r["headers"]["Accept"], "json");
        assert_eq!(r["client_secret"], REDACTED);
        assert_eq!(r["password"], REDACTED);
        assert_eq!(r["max_tokens"], 4000);
        assert_eq!(r["token_count"], 12);
        // The event instance key is not a credential.
        assert_eq!(r["key"], "42");
        assert!(r["token"].is_null());
    }

    #[test]
    fn credentials_inside_text_are_removed() {
        let t = redact_text("curl -H 'Authorization: Bearer abc.def-123456' https://x.dev");
        assert!(!t.contains("abc.def-123456"), "{t}");
        assert!(t.contains("Bearer [redacted]"));

        let t = redact_text("see https://alice:hunter2@example.com/path?x=1");
        assert_eq!(t, "see https://[redacted]@example.com/path?x=1");

        let t = redact_text("https://api.example.com/cb?code=abc&state=ok&access_token=zzz#frag");
        assert_eq!(
            t,
            "https://api.example.com/cb?code=[redacted]&state=ok&access_token=[redacted]#frag"
        );

        let t = redact_text("my key is sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUV and ghp_0123456789abcdefghijABCD");
        assert!(!t.contains("ABCDEFGHIJKLMNOP"), "{t}");
        assert!(!t.contains("0123456789abcdefghij"), "{t}");
        assert_eq!(t.matches(REDACTED).count(), 2);

        // Ordinary text survives untouched.
        let plain = "Meeting with Sam about the sk-learn upgrade; tokens: 120, basic plan";
        assert_eq!(redact_text(plain), plain);
        let plain = "Basic understanding of bearer instruments";
        assert_eq!(redact_text(plain), plain);
        assert_eq!(redact_text("ask-me later"), "ask-me later");
    }

    #[test]
    fn strings_nested_in_payloads_are_scanned_too() {
        let v = json!({"items": [{"url": "https://u:p@h.io/x"}, "Bearer 0123456789abcdef"]});
        let r = redact_value(&v);
        assert_eq!(r["items"][0]["url"], "https://[redacted]@h.io/x");
        assert_eq!(r["items"][1], "Bearer [redacted]");
    }
}
