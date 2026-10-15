// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Scrubbing for everything that goes into a diagnostics bundle.
//!
//! Two independent tools live here:
//!
//! - [`mask_sensitive_settings`] replaces the values of settings keys that
//!   name secrets or personal data. The key rule is shared with the
//!   frontend through `lib/feedback/sensitive-keys.json`.
//! - [`FeedbackRedactor`] scrubs free text (logs, feedback, chat history).
//!   It runs the deterministic detector from `screenpipe-redact` with a
//!   policy that is stricter than the capture-time default, then removes
//!   home-directory user names, non-loopback IPv4 addresses, bearer tokens
//!   and the user's own name, e-mail and host name.
//!
//! Everything is local. No model download, no network.

use std::collections::BTreeSet;

use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use serde::Deserialize;
use serde_json::{Map, Value};

use screenpipe_redact::adapters::regex::redact_one;
use screenpipe_redact::{RedactedSpan, SpanLabel, TextRedactionPolicy};

/// Replacement for a masked settings value.
pub const MASKED_VALUE: &str = "[redacted]";

/// Span labels always scrubbed from diagnostics text, on top of whatever the
/// user selected in `piiRedactionLabels`. Wider than the capture-time
/// default (secrets only) because a bundle may end up attached to a public
/// issue.
const BASELINE_LABELS: &[&str] = &["secret", "email", "phone", "url", "id"];

#[derive(Debug, Deserialize)]
struct KeyRule {
    fragments: Vec<String>,
    suffixes: Vec<String>,
    exceptions: Vec<String>,
}

struct NormalizedRule {
    fragments: Vec<String>,
    suffixes: Vec<String>,
    exceptions: BTreeSet<String>,
}

static RULE: Lazy<NormalizedRule> = Lazy::new(|| {
    let raw: KeyRule = serde_json::from_str(include_str!(
        "../../../lib/feedback/sensitive-keys.json"
    ))
    .expect("lib/feedback/sensitive-keys.json is valid");
    NormalizedRule {
        fragments: raw.fragments.iter().map(|f| normalize_key(f)).collect(),
        suffixes: raw.suffixes.iter().map(|s| normalize_key(s)).collect(),
        exceptions: raw.exceptions.iter().map(|e| normalize_key(e)).collect(),
    }
});

/// Lower-case a key and keep only ASCII letters and digits.
pub fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// True when values stored under `key` must be masked.
pub fn is_sensitive_key(key: &str) -> bool {
    let normalized = normalize_key(key);
    if normalized.is_empty() || RULE.exceptions.contains(&normalized) {
        return false;
    }
    RULE.fragments.iter().any(|f| normalized.contains(f.as_str()))
        || RULE.suffixes.iter().any(|s| normalized.ends_with(s.as_str()))
}

/// What a value under a sensitive key turns into. Booleans and nulls are
/// kept (they say whether something is on, not what it is); empty strings
/// and containers are kept (they say whether something is set).
fn mask_value(value: &Value) -> Value {
    match value {
        Value::Null | Value::Bool(_) => value.clone(),
        Value::String(s) if s.is_empty() => Value::String(String::new()),
        Value::String(_) | Value::Number(_) => Value::String(MASKED_VALUE.to_string()),
        Value::Array(items) if items.is_empty() => Value::Array(Vec::new()),
        Value::Array(items) => Value::String(format!("[redacted: {} items]", items.len())),
        Value::Object(map) if map.is_empty() => Value::Object(Map::new()),
        Value::Object(map) => Value::String(format!("[redacted: {} keys]", map.len())),
    }
}

/// Deep copy of `value` with every sensitive key masked.
pub fn mask_sensitive_settings(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(mask_sensitive_settings).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child)| {
                    let masked = if is_sensitive_key(key) {
                        mask_value(child)
                    } else {
                        mask_sensitive_settings(child)
                    };
                    (key.clone(), masked)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Free-text redaction
// ---------------------------------------------------------------------------

/// `C:\Users\<name>`, `/Users/<name>` and `/home/<name>`, including the
/// doubled backslashes that `Debug` output and JSON produce.
static HOME_PATH: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)((?:^|[^A-Za-z0-9_])(?:[A-Z]:)?(?:\\{1,2}|/)(?:Users|home)(?:\\{1,2}|/))([^\\/\s"'`:;,|<>]+)"#)
        .expect("home path regex")
});

static IPV4: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})\b").expect("ipv4 regex")
});

/// The whole of `s` is a dotted-quad address, nothing more.
static IPV4_ONLY: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}$").expect("ipv4-only regex"));

/// True when `span` covers exactly a bare IPv4 address in `line` (as opposed
/// to a URL with credentials, which the detector also labels `Url`).
fn is_bare_ipv4(line: &str, span: &RedactedSpan) -> bool {
    line.get(span.start..span.end)
        .is_some_and(|text| IPV4_ONLY.is_match(text))
}

/// `Authorization: Bearer <token>` / `Bearer <token>` with a long token.
static BEARER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(bearer)(\s+)[A-Za-z0-9._~+/=-]{16,}").expect("bearer regex")
});

/// `key=value` / `"key": "value"` pairs whose key names a credential.
static CREDENTIAL_ASSIGNMENT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?i)\b((?:api[_-]?key|access[_-]?token|refresh[_-]?token|auth[_-]?token|client[_-]?secret|password|passwd|secret)["']?\s*[:=]\s*["']?)([^\s"',;&]{6,})"#,
    )
    .expect("credential assignment regex")
});

/// A personal value the user configured, scrubbed wherever it appears.
struct Literal {
    pattern: Regex,
    placeholder: &'static str,
}

/// Local text scrubber for diagnostics. Cheap to build; build one per job.
pub struct FeedbackRedactor {
    policy: TextRedactionPolicy,
    literals: Vec<Literal>,
}

impl Default for FeedbackRedactor {
    fn default() -> Self {
        Self::new(None, &[])
    }
}

impl FeedbackRedactor {
    /// Build a redactor from the user's settings (for `piiRedactionLabels`
    /// and the signed-in user's name/e-mail) plus extra machine values such
    /// as the host name. Values shorter than three characters are ignored,
    /// they would match too much.
    pub fn new(settings: Option<&Value>, machine_values: &[(String, &'static str)]) -> Self {
        let mut labels: Vec<String> = BASELINE_LABELS.iter().map(|l| l.to_string()).collect();
        if let Some(chosen) = settings
            .and_then(|s| s.get("piiRedactionLabels"))
            .and_then(Value::as_array)
        {
            labels.extend(chosen.iter().filter_map(Value::as_str).map(str::to_string));
        }
        let policy = TextRedactionPolicy::from_labels(&labels);

        let mut values: Vec<(String, &'static str)> = machine_values.to_vec();
        if let Some(user) = settings.and_then(|s| s.get("user")) {
            for (field, placeholder) in [
                ("email", "[EMAIL]"),
                ("name", "[USER]"),
                ("github_username", "[USER]"),
                ("contact", "[USER]"),
            ] {
                if let Some(v) = user.get(field).and_then(Value::as_str) {
                    values.push((v.to_string(), placeholder));
                    if field == "name" {
                        // "Ada Lovelace" also shows up as "Ada" or "Lovelace".
                        for part in v.split_whitespace().filter(|p| p.chars().count() >= 4) {
                            values.push((part.to_string(), placeholder));
                        }
                    }
                }
            }
        }

        let mut seen = BTreeSet::new();
        let mut literals = Vec::new();
        // Longest first, so "Ada Lovelace" wins over "Lovelace".
        values.sort_by_key(|(v, _)| std::cmp::Reverse(v.trim().chars().count()));
        for (value, placeholder) in values {
            let value = value.trim();
            if value.chars().count() < 3 || !seen.insert(value.to_lowercase()) {
                continue;
            }
            let escaped = regex::escape(value);
            let starts_word = value.chars().next().is_some_and(|c| c.is_alphanumeric());
            let ends_word = value.chars().last().is_some_and(|c| c.is_alphanumeric());
            let pattern = format!(
                "(?i){}{}{}",
                if starts_word { r"\b" } else { "" },
                escaped,
                if ends_word { r"\b" } else { "" }
            );
            if let Ok(pattern) = Regex::new(&pattern) {
                literals.push(Literal {
                    pattern,
                    placeholder,
                });
            }
        }

        Self { policy, literals }
    }

    /// Scrub `text`. Works line by line so one huge log does not build one
    /// huge span list, and so placeholders never span lines.
    pub fn redact(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            out.push_str(&self.redact_line(line));
        }
        out
    }

    fn redact_line(&self, line: &str) -> String {
        if line.trim().is_empty() {
            return line.to_string();
        }
        let detected = redact_one(line);
        let mut spans: Vec<&RedactedSpan> = detected
            .spans
            .iter()
            .filter(|s| self.policy.allows(s.label, s.subtype.as_deref()))
            // The shared detector files bare IPv4 addresses under `Url`. Leave
            // those to the IPv4 pass below, which keeps loopback and 0.0.0.0
            // (they only say which interface something bound to) and turns
            // every other address into `[IP]`.
            .filter(|s| !(s.label == SpanLabel::Url && is_bare_ipv4(line, s)))
            .collect();
        spans.sort_by_key(|s| (s.start, std::cmp::Reverse(s.end)));

        let mut text = String::with_capacity(line.len());
        let mut cursor = 0;
        for span in spans {
            if span.start < cursor
                || span.end > line.len()
                || !line.is_char_boundary(span.start)
                || !line.is_char_boundary(span.end)
            {
                continue;
            }
            text.push_str(&line[cursor..span.start]);
            text.push_str(span.label.placeholder());
            cursor = span.end;
        }
        text.push_str(&line[cursor..]);

        let text = CREDENTIAL_ASSIGNMENT.replace_all(&text, "${1}[SECRET]");
        let text = BEARER.replace_all(&text, "${1}${2}[SECRET]");
        let text = HOME_PATH.replace_all(&text, "${1}[USER]");
        let text = IPV4.replace_all(&text, |caps: &Captures| {
            let octets: Vec<u32> = (1..=4)
                .map(|i| caps[i].parse::<u32>().unwrap_or(999))
                .collect();
            let valid = octets.iter().all(|o| *o <= 255);
            let keep = !valid || octets[0] == 127 || octets.iter().all(|o| *o == 0);
            if keep {
                caps[0].to_string()
            } else {
                "[IP]".to_string()
            }
        });

        let mut text = text.into_owned();
        for literal in &self.literals {
            if literal.pattern.is_match(&text) {
                text = literal
                    .pattern
                    .replace_all(&text, literal.placeholder)
                    .into_owned();
            }
        }
        text
    }
}

/// Account and machine names so common that scrubbing them would blank out
/// ordinary words in logs without protecting anyone.
const GENERIC_NAMES: &[&str] = &[
    "localhost",
    "admin",
    "administrator",
    "user",
    "users",
    "root",
    "owner",
    "guest",
    "default",
    "public",
    "home",
    "desktop",
    "laptop",
    "computer",
];

/// True when a host or login name is specific enough to scrub wherever it
/// appears: at least four characters and not a generic word.
pub fn is_distinctive_value(value: &str) -> bool {
    let value = value.trim();
    value.chars().count() >= 4
        && !GENERIC_NAMES
            .iter()
            .any(|generic| value.eq_ignore_ascii_case(generic))
}

/// Parse the `settingsJson` argument of `redact_pii_for_feedback`. Bad or
/// missing JSON just means "no user preferences".
pub fn parse_settings_json(settings_json: Option<&str>) -> Option<Value> {
    settings_json
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .filter(Value::is_object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Vectors {
        sensitive_keys: Vec<String>,
        plain_keys: Vec<String>,
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        name: String,
        input: Value,
        expected: Value,
    }

    fn vectors() -> Vectors {
        serde_json::from_str(include_str!(
            "../../../lib/feedback/sensitive-keys.test-vectors.json"
        ))
        .expect("test vectors parse")
    }

    #[test]
    fn key_rule_matches_shared_vectors() {
        let v = vectors();
        for key in &v.sensitive_keys {
            assert!(is_sensitive_key(key), "{key} should be sensitive");
        }
        for key in &v.plain_keys {
            assert!(!is_sensitive_key(key), "{key} should be plain");
        }
    }

    #[test]
    fn masking_matches_shared_vectors() {
        for case in vectors().cases {
            assert_eq!(
                mask_sensitive_settings(&case.input),
                case.expected,
                "case: {}",
                case.name
            );
        }
    }

    #[test]
    fn masked_settings_hold_no_secret() {
        let secret = "very-secret-value-0123";
        let settings = json!({
            "aiPresets": [{ "apiKey": secret, "provider": "openai" }],
            "deepgramApiKey": secret,
            "user": { "token": secret },
            "pipes": { "obsidian": { "vaultPath": secret } }
        });
        let out = serde_json::to_string(&mask_sensitive_settings(&settings)).unwrap();
        assert!(!out.contains(secret), "{out}");
    }

    #[test]
    fn scrubs_contact_details_and_secrets() {
        let r = FeedbackRedactor::default();
        let out = r.redact("mail ada@example.org or call +1 415-555-0132\n");
        assert!(!out.contains("ada@example.org"), "{out}");
        assert!(!out.contains("555-0132"), "{out}");
        assert!(out.ends_with('\n'));

        let out = r.redact("Authorization: Bearer abcdefghijklmnopqrstuvwxyz012345");
        assert!(!out.contains("abcdefghijklmnopqrstuvwxyz012345"), "{out}");

        let out = r.redact(r#"{"api_key": "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH"}"#);
        assert!(!out.contains("abcdefghijklmnopqrstuvwxyz0123456789"), "{out}");

        let out = r.redact("password=hunter2hunter2 next");
        assert!(!out.contains("hunter2hunter2"), "{out}");
        assert!(out.contains("next"), "{out}");
    }

    #[test]
    fn scrubs_home_directory_user_names() {
        let r = FeedbackRedactor::default();
        for (input, leaked) in [
            (r"opening C:\Users\darren\AppData\Local\x.db", "darren"),
            (r#"path: "C:\\Users\\darren\\.screenpipe\\db.sqlite""#, "darren"),
            ("/Users/ada/.screenpipe/data", "ada"),
            ("cwd=/home/grace/projects", "grace"),
            ("C:/Users/linus/Documents", "linus"),
        ] {
            let out = r.redact(input);
            assert!(!out.contains(leaked), "{input} -> {out}");
            assert!(out.contains("[USER]"), "{input} -> {out}");
        }
        // Idempotent: scrubbing twice changes nothing more.
        let once = r.redact("/home/grace/x");
        assert_eq!(r.redact(&once), once);
    }

    #[test]
    fn scrubs_public_ips_but_keeps_loopback() {
        let r = FeedbackRedactor::default();
        let out = r.redact("bound 127.0.0.1:3030 and 0.0.0.0, peer 192.168.1.20, remote 8.8.4.4");
        assert!(out.contains("127.0.0.1:3030"), "{out}");
        assert!(out.contains("0.0.0.0"), "{out}");
        assert!(!out.contains("192.168.1.20"), "{out}");
        assert!(!out.contains("8.8.4.4"), "{out}");
        assert_eq!(out.matches("[IP]").count(), 2, "{out}");
        // Not an address: an octet above 255.
        assert!(r.redact("value 300.1.1.1").contains("300.1.1.1"));
        // URLs with credentials are still removed whole, loopback or not.
        let out = r.redact("db postgres://admin:pa55word@127.0.0.1:5432/app");
        assert!(!out.contains("pa55word"), "{out}");
        assert!(out.contains("[URL]"), "{out}");
    }

    #[test]
    fn keeps_ordinary_log_lines_readable() {
        let r = FeedbackRedactor::default();
        for line in [
            "2026-10-15T14:05:01.123456Z  INFO screenpipe_engine::core: frame 549130407 written in 12ms",
            "monitor 605818409 resolution 2880x1800 fps=0.5",
            "meeting started (id=68), pid: 25616, elapsed=100.4287155s",
            "audio device \"Microphone (Realtek(R) Audio)\" (input) started",
            "GET http://localhost:3030/health 200",
            "version 2.5.28 on windows x86_64",
        ] {
            assert_eq!(r.redact(line), line);
        }
    }

    #[test]
    fn scrubs_the_users_own_identity() {
        let settings = json!({
            "user": { "name": "Ada Lovelace", "email": "ada@analytical.engine", "id": "u_1" }
        });
        let r = FeedbackRedactor::new(Some(&settings), &[("ADA-DESKTOP".to_string(), "[HOST]")]);
        let out = r.redact("Ada Lovelace signed in on ada-desktop; Lovelace again; ada@analytical.engine");
        assert!(!out.to_lowercase().contains("lovelace"), "{out}");
        assert!(!out.to_lowercase().contains("ada-desktop"), "{out}");
        assert!(!out.contains("ada@analytical.engine"), "{out}");
        assert!(out.contains("[HOST]"), "{out}");
    }

    #[test]
    fn honours_extra_labels_from_settings() {
        use screenpipe_redact::SpanLabel;
        let settings = json!({ "piiRedactionLabels": ["date", "person", "bogus"] });
        let custom = FeedbackRedactor::new(Some(&settings), &[]);
        let base = FeedbackRedactor::default();
        for label in [SpanLabel::Secret, SpanLabel::Email, SpanLabel::Phone, SpanLabel::Id] {
            assert!(base.policy.allows(label, None), "{label:?}");
            assert!(custom.policy.allows(label, None), "{label:?}");
        }
        assert!(!base.policy.allows(SpanLabel::Date, None));
        assert!(custom.policy.allows(SpanLabel::Date, None));
        assert!(custom.policy.allows(SpanLabel::Person, None));
    }

    #[test]
    fn only_distinctive_machine_names_are_scrubbed() {
        for name in ["DARREN-PC", "darren", "ws-0142", "Ada.Lovelace"] {
            assert!(is_distinctive_value(name), "{name}");
        }
        for name in ["", "abc", "  ab  ", "localhost", "Admin", "USER", "root", "Owner", "DESKTOP"] {
            assert!(!is_distinctive_value(name), "{name}");
        }
        // A login name given as a machine value is removed outside paths too.
        let r = FeedbackRedactor::new(None, &[("darren".to_string(), "[USER]")]);
        assert_eq!(r.redact(r"signed in as WORKGROUP\darren"), r"signed in as WORKGROUP\[USER]");
        assert_eq!(r.redact("darrenx stays"), "darrenx stays");
    }

    #[test]
    fn parse_settings_json_tolerates_garbage() {
        assert!(parse_settings_json(None).is_none());
        assert!(parse_settings_json(Some("")).is_none());
        assert!(parse_settings_json(Some("not json")).is_none());
        assert!(parse_settings_json(Some("[1,2]")).is_none());
        assert!(parse_settings_json(Some(r#"{"a":1}"#)).is_some());
    }
}
