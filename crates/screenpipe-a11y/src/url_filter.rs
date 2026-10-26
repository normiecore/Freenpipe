//! Domain-level URL blocking shared by the vision capture path
//! (`screenpipe-screen`'s `WindowFilters`) and the a11y tree walker.
//! Lives here because the dependency direction is screen → a11y: an ignored
//! URL must produce neither frames nor accessibility snapshots.
//!
//! Two layers live here:
//!
//! - [`is_url_blocked`]: the original matcher for plain pattern strings
//!   (`ignoredUrls` entries written by older app versions). Its behaviour is
//!   unchanged so existing block lists keep meaning exactly what they meant.
//! - [`UrlPolicy`]: the full website policy built from the stored
//!   `ignoredUrls` (legacy strings and structured [`DomainRule`]s) and the
//!   `includedUrls` allow list. Structured rules match on the parsed host
//!   only, never on a substring of the URL; the block list wins over the
//!   allow list; and with an allow list in place, a browser tab whose URL
//!   cannot be read is not recorded ([`UrlDecision::Pending`]).

/// The stored rule shapes, re-exported for crates that build policies.
pub use screenpipe_config::{DomainRule, UrlRule};
use std::fmt;
use std::sync::Arc;
use url::Url;

/// Check if a URL should be filtered out for privacy.
///
/// Uses domain-level matching to avoid false positives (e.g. "chase" won't
/// match "purchase.com"). Patterns are matched case-insensitively against
/// the URL's host:
///
/// 1. Exact match: `host == pattern`
/// 2. Subdomain: host ends with `.{pattern}`
/// 3. No-TLD pattern: `chase` matches `chase.com` / `www.chase.net` / …
///
/// Returns `true` if the URL is blocked (should be skipped).
pub fn is_url_blocked(url: &str, blocked_patterns: &[String]) -> bool {
    if blocked_patterns.is_empty() {
        return false;
    }

    // Normalize so bare hosts ("wellsfargo.com") parse too.
    let url_to_parse = if !url.starts_with("http://") && !url.starts_with("https://") {
        format!("https://{}", url)
    } else {
        url.to_string()
    };

    if let Ok(parsed) = Url::parse(&url_to_parse) {
        if let Some(host) = parsed.host_str() {
            let host_lower = host.to_lowercase();
            return blocked_patterns
                .iter()
                .any(|blocked| host_matches_pattern(&host_lower, &blocked.to_lowercase()));
        }
    }

    // Fallback to simple contains check if URL parsing fails.
    // Less precise, but ensures we don't miss obvious matches.
    let url_lower = url.to_lowercase();
    blocked_patterns
        .iter()
        .any(|blocked| url_lower.contains(&blocked.to_lowercase()))
}

/// The URL as the legacy matcher sees it: the lowercased host when the URL
/// parses (with `https://` assumed when no http(s) scheme is given), else the
/// lowercased URL for the substring fallback. Computed once per URL so a
/// policy can test every legacy pattern against it.
enum LegacySubject {
    Host(String),
    Unparsed(String),
}

impl LegacySubject {
    fn of(url: &str) -> Self {
        let url_to_parse = if !url.starts_with("http://") && !url.starts_with("https://") {
            format!("https://{}", url)
        } else {
            url.to_string()
        };
        match Url::parse(&url_to_parse) {
            Ok(parsed) => match parsed.host_str() {
                Some(host) => LegacySubject::Host(host.to_lowercase()),
                None => LegacySubject::Unparsed(url.to_lowercase()),
            },
            Err(_) => LegacySubject::Unparsed(url.to_lowercase()),
        }
    }

    /// Same decision as [`is_url_blocked`] for one lowercased pattern.
    fn matches(&self, pattern_lower: &str) -> bool {
        match self {
            LegacySubject::Host(host) => host_matches_pattern(host, pattern_lower),
            LegacySubject::Unparsed(url) => url.contains(pattern_lower),
        }
    }
}

/// Domain-boundary match of one lowercased host against one lowercased pattern.
fn host_matches_pattern(host_lower: &str, blocked: &str) -> bool {
    // Exact match
    if host_lower == blocked {
        return true;
    }

    // Subdomain match: host ends with ".blocked"
    if host_lower.ends_with(&format!(".{}", blocked)) {
        return true;
    }

    // For patterns without a TLD (e.g. "chase" instead of "chase.com"),
    // match the pattern against any domain label. This covers every TLD
    // (chase.com, chase.co.uk, chase.io, …) without hardcoding a list,
    // while still respecting domain boundaries: "purchase.com" splits to
    // ["purchase", "com"], so "chase" never matches it.
    if !blocked.contains('.') {
        return host_lower.split('.').any(|label| label == blocked);
    }

    false
}

// ---------------------------------------------------------------------------
// Website policy: structured domain rules, allow list, validation
// ---------------------------------------------------------------------------

/// Browsers whose tabs the website allow list governs. An app counts as one
/// of them when a word of its name (split on anything that is not a letter or
/// digit, `.exe`/`.app` removed) is in this list, so `Google Chrome`,
/// `chrome.exe`, `brave-browser` and `Microsoft Edge` all count, while
/// `SearchHost.exe` or `ArcGIS Pro` do not. Other apps that happen to report a
/// URL (Electron apps) are only subject to the block list.
pub const URL_POLICY_BROWSERS: &[&str] = &[
    "chrome", "chromium", "firefox", "safari", "edge", "brave", "arc", "vivaldi", "opera", "zen",
    "comet", "epiphany", "helium",
];

/// True when `app_name` is one of the [`URL_POLICY_BROWSERS`].
pub fn is_policy_browser(app_name: &str) -> bool {
    let lower = app_name.trim().to_lowercase();
    let stem = lower
        .strip_suffix(".exe")
        .or_else(|| lower.strip_suffix(".app"))
        .unwrap_or(&lower);
    // Edge's Windows process name has no separator between the words.
    if stem == "msedge" {
        return true;
    }
    stem.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word == "msedge" || URL_POLICY_BROWSERS.contains(&word))
}

/// Why a structured website rule cannot be used as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlRuleError {
    /// The rule has no domain at all.
    EmptyDomain,
    /// A URL-form entry uses a scheme other than http or https.
    UnsupportedScheme(String),
    /// A URL-form entry has no host name.
    MissingHost(String),
    /// The text is not a usable host name.
    MalformedDomain(String),
    /// The host has no dot (`localhost`, `intranet`).
    NeedsDot(String),
    /// `excludedSubdomains` is set while `includeSubdomains` is off.
    ExclusionsWithoutSubdomains,
    /// An excluded subdomain is not below the rule's domain.
    ExclusionOutsideDomain { excluded: String, domain: String },
    /// An excluded subdomain is itself not a usable host name.
    InvalidExclusion {
        excluded: String,
        reason: Box<UrlRuleError>,
    },
}

impl fmt::Display for UrlRuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UrlRuleError::EmptyDomain => f.write_str("a website rule needs a domain"),
            UrlRuleError::UnsupportedScheme(scheme) => write!(
                f,
                "only http:// and https:// addresses can be used here, not {}://",
                scheme
            ),
            UrlRuleError::MissingHost(text) => {
                write!(f, "\"{}\" does not name a host", text)
            }
            UrlRuleError::MalformedDomain(text) => {
                write!(f, "\"{}\" is not a valid domain name", text)
            }
            UrlRuleError::NeedsDot(host) => write!(
                f,
                "\"{}\" is too short to be a website domain; use a full name like example.com",
                host
            ),
            UrlRuleError::ExclusionsWithoutSubdomains => {
                f.write_str("subdomain exceptions only make sense when the rule covers subdomains")
            }
            UrlRuleError::ExclusionOutsideDomain { excluded, domain } => {
                write!(f, "\"{}\" is not a subdomain of {}", excluded, domain)
            }
            UrlRuleError::InvalidExclusion { excluded, reason } => {
                write!(f, "subdomain exception \"{}\": {}", excluded, reason)
            }
        }
    }
}

impl std::error::Error for UrlRuleError {}

/// Lowercase a host as the URL parser reports it and drop one trailing dot.
fn canonical_host(host: &str) -> String {
    let lower = host.to_lowercase();
    match lower.strip_suffix('.') {
        Some(stripped) => stripped.to_string(),
        None => lower,
    }
}

/// Characters that cannot appear in a bare domain entry (they belong to
/// URLs, ports, credentials, wildcards or whitespace).
fn has_non_domain_char(text: &str) -> bool {
    text.chars().any(|c| {
        c.is_whitespace()
            || matches!(
                c,
                '/' | '?' | '#' | '@' | ':' | '*' | '\\' | '[' | ']' | '%'
            )
    })
}

/// The canonical host a rule's `domain` (or an excluded subdomain) names.
///
/// Accepts a bare host (`example.com`, `Example.COM.`, an internationalised
/// name) or an http(s) URL whose host is used. Does not apply the "needs a
/// dot" rule; [`validate_domain_rule`] does.
fn rule_host(text: &str) -> Result<String, UrlRuleError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(UrlRuleError::EmptyDomain);
    }

    if let Some(idx) = trimmed.find("://") {
        let scheme = trimmed[..idx].to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(UrlRuleError::UnsupportedScheme(scheme));
        }
        let parsed =
            Url::parse(trimmed).map_err(|_| UrlRuleError::MalformedDomain(trimmed.to_string()))?;
        return match parsed.host_str() {
            Some(host) if !host.is_empty() => Ok(canonical_host(host)),
            _ => Err(UrlRuleError::MissingHost(trimmed.to_string())),
        };
    }

    let without_dot = trimmed.strip_suffix('.').unwrap_or(trimmed);
    let labels_ok = !without_dot.is_empty()
        && without_dot.len() <= 253
        && without_dot.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
        });
    if has_non_domain_char(trimmed) || !labels_ok {
        return Err(UrlRuleError::MalformedDomain(trimmed.to_string()));
    }
    // The URL parser applies IDNA, so `bücher.example` and its punycode
    // spelling end up identical, as they are in a browser's address bar.
    match Url::parse(&format!("https://{}/", trimmed)) {
        Ok(parsed) => match parsed.host_str() {
            Some(host) if !host.is_empty() => Ok(canonical_host(host)),
            _ => Err(UrlRuleError::MalformedDomain(trimmed.to_string())),
        },
        Err(_) => Err(UrlRuleError::MalformedDomain(trimmed.to_string())),
    }
}

/// True when `host` is a proper subdomain of `parent` (`a.b.example` is
/// below `b.example`; `notexample` and `example` itself are not).
fn is_below(host: &str, parent: &str) -> bool {
    host.len() > parent.len() + 1
        && host.ends_with(parent)
        && host.as_bytes()[host.len() - parent.len() - 1] == b'.'
}

/// A validated domain rule, ready to match hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CompiledDomainRule {
    domain: String,
    include_subdomains: bool,
    excluded: Vec<String>,
}

impl CompiledDomainRule {
    fn compile(rule: &DomainRule) -> Result<Self, UrlRuleError> {
        let domain = rule_host(&rule.domain)?;
        if !domain.contains('.') {
            return Err(UrlRuleError::NeedsDot(domain));
        }
        let listed: Vec<&String> = rule
            .excluded_subdomains
            .iter()
            .filter(|e| !e.trim().is_empty())
            .collect();
        if !listed.is_empty() && !rule.include_subdomains {
            return Err(UrlRuleError::ExclusionsWithoutSubdomains);
        }
        let mut excluded = Vec::with_capacity(listed.len());
        for entry in listed {
            let host = rule_host(entry).map_err(|reason| UrlRuleError::InvalidExclusion {
                excluded: entry.trim().to_string(),
                reason: Box::new(reason),
            })?;
            if !is_below(&host, &domain) {
                return Err(UrlRuleError::ExclusionOutsideDomain {
                    excluded: host,
                    domain: domain.clone(),
                });
            }
            excluded.push(host);
        }
        Ok(Self {
            domain,
            include_subdomains: rule.include_subdomains,
            excluded,
        })
    }

    /// Host-only match: the exact domain, or (with subdomains on) any host
    /// below it that is not at or below an excluded subdomain.
    fn matches_host(&self, host: &str) -> bool {
        if host == self.domain {
            return true;
        }
        if !self.include_subdomains || !is_below(host, &self.domain) {
            return false;
        }
        !self
            .excluded
            .iter()
            .any(|ex| host == ex.as_str() || is_below(host, ex))
    }
}

/// Check one structured rule the way the settings UI should before saving
/// it. `Ok` means the engine will apply the rule exactly as written.
pub fn validate_domain_rule(rule: &DomainRule) -> Result<(), UrlRuleError> {
    CompiledDomainRule::compile(rule).map(|_| ())
}

/// Which list a rejected rule came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlRuleList {
    /// `ignoredUrls`
    Blocked,
    /// `includedUrls`
    Allowed,
}

/// A stored rule the policy could not apply as written, and what it did
/// instead (see [`UrlPolicy::new`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedUrlRule {
    pub list: UrlRuleList,
    /// Position of the rule in its stored list.
    pub index: usize,
    pub domain: String,
    pub error: UrlRuleError,
}

/// What a browser URL looks like to the allow list.
enum BrowserUrl {
    /// A parsed web address with this canonical host.
    Host(String),
    /// A readable address that has no host (`about:blank`, `data:`, `file:`).
    NoHost,
    /// Nothing usable: missing, empty or unparseable.
    Unknown,
}

/// Schemes that never carry a host; such URLs are known, just not websites.
const HOSTLESS_SCHEMES: &[&str] = &[
    "about:",
    "data:",
    "javascript:",
    "blob:",
    "view-source:",
    "mailto:",
    "file:",
];

impl BrowserUrl {
    fn of(url: Option<&str>) -> Self {
        let Some(text) = url.map(str::trim).filter(|t| !t.is_empty()) else {
            return BrowserUrl::Unknown;
        };
        let lower = text.to_ascii_lowercase();
        let has_scheme =
            lower.contains("://") || HOSTLESS_SCHEMES.iter().any(|s| lower.starts_with(s));
        let candidate = if has_scheme {
            text.to_string()
        } else {
            // Address bars often drop the scheme (`github.com/foo`).
            format!("https://{}", text)
        };
        match Url::parse(&candidate) {
            Ok(parsed) => match parsed.host_str() {
                Some(host) if !host.is_empty() => BrowserUrl::Host(canonical_host(host)),
                _ => BrowserUrl::NoHost,
            },
            Err(_) => BrowserUrl::Unknown,
        }
    }
}

/// One entry of the compiled block list.
#[derive(Debug, Clone)]
enum BlockRule {
    /// Legacy pattern, lowercased; matched like [`is_url_blocked`].
    Legacy(String),
    /// Structured rule; matched on the parsed host only.
    Domain(CompiledDomainRule),
}

#[derive(Debug, Default)]
struct PolicyRules {
    blocked: Vec<BlockRule>,
    allowed: Vec<CompiledDomainRule>,
    /// True when `includedUrls` has entries, even if none of them is usable:
    /// an allow list of broken rules allows nothing rather than everything.
    allow_list_active: bool,
    rejected: Vec<RejectedUrlRule>,
}

impl PolicyRules {
    fn blocks(&self, url: &str) -> bool {
        if self.blocked.is_empty() {
            return false;
        }
        let legacy = LegacySubject::of(url);
        let mut parsed: Option<BrowserUrl> = None;
        self.blocked.iter().any(|rule| match rule {
            BlockRule::Legacy(pattern) => legacy.matches(pattern),
            BlockRule::Domain(rule) => {
                match parsed.get_or_insert_with(|| BrowserUrl::of(Some(url))) {
                    BrowserUrl::Host(host) => rule.matches_host(host),
                    BrowserUrl::NoHost | BrowserUrl::Unknown => false,
                }
            }
        })
    }

    fn allows_host(&self, host: &str) -> bool {
        self.allowed.iter().any(|rule| rule.matches_host(host))
    }
}

/// Outcome of the website policy for one captured window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlDecision {
    /// Record the window.
    Allowed,
    /// The URL matches the block list.
    Blocked,
    /// An allow list is set and this browser URL is not on it.
    NotAllowed,
    /// An allow list is set and this browser's URL is not known (yet), so
    /// it cannot be checked. Nothing is recorded for the capture.
    Pending,
}

impl UrlDecision {
    pub fn is_allowed(self) -> bool {
        self == UrlDecision::Allowed
    }

    /// The skip reason a walker reports for this decision (`None` when the
    /// window is recorded).
    pub fn skip_reason(self) -> Option<crate::tree::SkipReason> {
        match self {
            UrlDecision::Allowed => None,
            UrlDecision::Blocked | UrlDecision::NotAllowed => {
                Some(crate::tree::SkipReason::BlockedUrl)
            }
            UrlDecision::Pending => Some(crate::tree::SkipReason::UrlValidationPending),
        }
    }

    /// Short stable label for logs.
    pub fn as_str(self) -> &'static str {
        match self {
            UrlDecision::Allowed => "allowed",
            UrlDecision::Blocked => "blocked",
            UrlDecision::NotAllowed => "not_allowed",
            UrlDecision::Pending => "pending",
        }
    }
}

/// The website block and allow lists, compiled once and cheap to clone.
///
/// Built from the stored settings with [`UrlPolicy::new`] or from plain
/// pattern strings with [`UrlPolicy::from_legacy_patterns`]. An empty policy
/// allows everything.
#[derive(Clone, Default)]
pub struct UrlPolicy {
    rules: Option<Arc<PolicyRules>>,
}

impl fmt::Debug for UrlPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.rules {
            None => f.write_str("UrlPolicy(empty)"),
            Some(rules) => f
                .debug_struct("UrlPolicy")
                .field("blocked", &rules.blocked.len())
                .field("allowed", &rules.allowed.len())
                .field("allow_list_active", &rules.allow_list_active)
                .field("rejected", &rules.rejected.len())
                .finish(),
        }
    }
}

impl UrlPolicy {
    /// Compile the stored `ignoredUrls` and `includedUrls` lists.
    ///
    /// A rule that fails [`validate_domain_rule`] never lets more through
    /// than the user asked for: a broken block rule falls back to the broad
    /// legacy match on its domain (that domain and everything below it), and
    /// a broken allow rule allows nothing. Each such rule is listed in
    /// [`UrlPolicy::rejected_rules`] and logged once here.
    pub fn new(ignored: &[UrlRule], included: &[DomainRule]) -> Self {
        let mut rules = PolicyRules {
            allow_list_active: !included.is_empty(),
            ..PolicyRules::default()
        };

        for (index, entry) in ignored.iter().enumerate() {
            match entry {
                UrlRule::Legacy(pattern) => {
                    let pattern = pattern.trim().to_lowercase();
                    if !pattern.is_empty() {
                        rules.blocked.push(BlockRule::Legacy(pattern));
                    }
                }
                UrlRule::Domain(rule) => match CompiledDomainRule::compile(rule) {
                    Ok(compiled) => rules.blocked.push(BlockRule::Domain(compiled)),
                    Err(error) => {
                        let fallback = entry.legacy_pattern().trim().to_lowercase();
                        if !fallback.is_empty() {
                            rules.blocked.push(BlockRule::Legacy(fallback));
                        }
                        rules.rejected.push(RejectedUrlRule {
                            list: UrlRuleList::Blocked,
                            index,
                            domain: rule.domain.clone(),
                            error,
                        });
                    }
                },
            }
        }

        for (index, rule) in included.iter().enumerate() {
            match CompiledDomainRule::compile(rule) {
                Ok(compiled) => rules.allowed.push(compiled),
                Err(error) => rules.rejected.push(RejectedUrlRule {
                    list: UrlRuleList::Allowed,
                    index,
                    domain: rule.domain.clone(),
                    error,
                }),
            }
        }

        for rejected in &rules.rejected {
            match rejected.list {
                UrlRuleList::Blocked => tracing::warn!(
                    index = rejected.index,
                    "ignoredUrls rule {:?} is invalid ({}); blocking that domain and all of its subdomains instead",
                    rejected.domain,
                    rejected.error
                ),
                UrlRuleList::Allowed => tracing::warn!(
                    index = rejected.index,
                    "includedUrls rule {:?} is invalid ({}); it allows nothing until it is fixed",
                    rejected.domain,
                    rejected.error
                ),
            }
        }

        Self::from_rules(rules)
    }

    /// A block-only policy from plain pattern strings (legacy semantics).
    pub fn from_legacy_patterns(patterns: &[String]) -> Self {
        let blocked = patterns
            .iter()
            .map(|p| p.trim().to_lowercase())
            .filter(|p| !p.is_empty())
            .map(BlockRule::Legacy)
            .collect();
        Self::from_rules(PolicyRules {
            blocked,
            ..PolicyRules::default()
        })
    }

    fn from_rules(rules: PolicyRules) -> Self {
        if rules.blocked.is_empty() && !rules.allow_list_active {
            return Self::default();
        }
        Self {
            rules: Some(Arc::new(rules)),
        }
    }

    /// True when the policy neither blocks nor restricts anything.
    pub fn is_empty(&self) -> bool {
        self.rules.is_none()
    }

    /// True when `includedUrls` has entries, so browser capture is limited
    /// to the allowed sites.
    pub fn has_allow_list(&self) -> bool {
        self.rules.as_ref().is_some_and(|r| r.allow_list_active)
    }

    /// True when the block list has entries.
    pub fn has_block_list(&self) -> bool {
        self.rules.as_ref().is_some_and(|r| !r.blocked.is_empty())
    }

    /// Stored rules that could not be applied as written.
    pub fn rejected_rules(&self) -> &[RejectedUrlRule] {
        self.rules.as_ref().map_or(&[], |r| r.rejected.as_slice())
    }

    /// True when `url` matches the block list.
    pub fn is_blocked(&self, url: &str) -> bool {
        self.rules.as_ref().is_some_and(|r| r.blocks(url))
    }

    /// True when the window of `app_name` cannot be recorded without first
    /// reading its URL: an allow list is set and the app is a browser.
    pub fn needs_verified_url(&self, app_name: &str) -> bool {
        self.has_allow_list() && is_policy_browser(app_name)
    }

    /// [`UrlPolicy::decide`] for a capture whose app may be unknown. With an
    /// allow list, an unknown app could be a browser showing a site that is
    /// not allowed, so it is treated like a browser whose URL is unknown.
    pub fn decide_for_capture(&self, app_name: Option<&str>, url: Option<&str>) -> UrlDecision {
        match app_name.map(str::trim).filter(|a| !a.is_empty()) {
            Some(app) => self.decide(app, url),
            None => {
                let url = url.map(str::trim).filter(|u| !u.is_empty());
                if url.is_some_and(|u| self.is_blocked(u)) {
                    UrlDecision::Blocked
                } else if self.has_allow_list() {
                    UrlDecision::Pending
                } else {
                    UrlDecision::Allowed
                }
            }
        }
    }

    /// Decide whether a window with this app and (browser) URL may be
    /// recorded. The block list applies to every window that reports a URL
    /// and wins over the allow list; the allow list applies to browsers only.
    pub fn decide(&self, app_name: &str, url: Option<&str>) -> UrlDecision {
        let Some(rules) = self.rules.as_ref() else {
            return UrlDecision::Allowed;
        };
        let url = url.map(str::trim).filter(|u| !u.is_empty());
        if let Some(url) = url {
            if rules.blocks(url) {
                return UrlDecision::Blocked;
            }
        }
        if !rules.allow_list_active || !is_policy_browser(app_name) {
            return UrlDecision::Allowed;
        }
        match BrowserUrl::of(url) {
            BrowserUrl::Unknown => UrlDecision::Pending,
            BrowserUrl::NoHost => UrlDecision::NotAllowed,
            BrowserUrl::Host(host) => {
                if rules.allows_host(&host) {
                    UrlDecision::Allowed
                } else {
                    UrlDecision::NotAllowed
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(patterns: &[&str]) -> Vec<String> {
        patterns.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_empty_list_blocks_nothing() {
        assert!(!is_url_blocked("https://wellsfargo.com", &[]));
    }

    #[test]
    fn test_exact_domain_match() {
        let b = blocked(&["wellsfargo.com"]);
        assert!(is_url_blocked("https://wellsfargo.com", &b));
        assert!(is_url_blocked("https://wellsfargo.com/login", &b));
        assert!(is_url_blocked("https://www.wellsfargo.com", &b));
        assert!(is_url_blocked("https://online.wellsfargo.com/account", &b));
    }

    #[test]
    fn test_case_insensitive() {
        let b = blocked(&["WellsFargo.com"]);
        assert!(is_url_blocked("https://WELLSFARGO.COM", &b));
        assert!(is_url_blocked("https://wellsfargo.com/Login", &b));
    }

    #[test]
    fn test_no_false_positive_on_substring() {
        let b = blocked(&["chase.com"]);
        assert!(is_url_blocked("https://chase.com", &b));
        assert!(is_url_blocked("https://www.chase.com/login", &b));
        assert!(!is_url_blocked("https://purchase.com", &b));
        assert!(!is_url_blocked("https://showcase.example.com", &b));
    }

    #[test]
    fn test_partial_domain_pattern_matches_any_tld() {
        let b = blocked(&["chase"]);
        assert!(is_url_blocked("https://chase.com", &b));
        assert!(is_url_blocked("https://www.chase.com", &b));
        assert!(is_url_blocked("https://chase.bank", &b));
        // TLDs not in the old hardcoded list still match (no TLD allowlist).
        assert!(is_url_blocked("https://chase.co.uk", &b));
        assert!(is_url_blocked("https://chase.io", &b));
        assert!(is_url_blocked("https://online.chase.de/account", &b));
        // Domain boundaries are still respected — no substring false positives.
        assert!(!is_url_blocked("https://purchase.com", &b));
        assert!(!is_url_blocked("https://purchase.co.uk", &b));
        assert!(!is_url_blocked("https://showcase.example.com", &b));
    }

    #[test]
    fn test_without_protocol() {
        let b = blocked(&["wellsfargo.com"]);
        assert!(is_url_blocked("wellsfargo.com", &b));
        assert!(is_url_blocked("www.wellsfargo.com/account", &b));
    }

    #[test]
    fn test_multiple_patterns() {
        let b = blocked(&["wellsfargo.com", "chase.com", "bankofamerica.com"]);
        assert!(is_url_blocked("https://chase.com/login", &b));
        assert!(is_url_blocked("https://www.bankofamerica.com", &b));
        assert!(!is_url_blocked("https://google.com", &b));
    }

    // ---- website policy (structured rules, allow list) ----

    fn rule(domain: &str, include_subdomains: bool, excluded: &[&str]) -> DomainRule {
        DomainRule {
            domain: domain.to_string(),
            include_subdomains,
            excluded_subdomains: excluded.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn block_policy(rules: Vec<UrlRule>) -> UrlPolicy {
        UrlPolicy::new(&rules, &[])
    }

    #[test]
    fn domain_rule_matches_exact_host_only_by_default() {
        let p = block_policy(vec![rule("bank.com", false, &[]).into()]);
        assert!(p.is_blocked("https://bank.com/login"));
        assert!(p.is_blocked("bank.com"));
        assert!(p.is_blocked("HTTPS://BANK.COM./x"));
        assert!(!p.is_blocked("https://www.bank.com"));
    }

    #[test]
    fn domain_rule_never_matches_url_substrings() {
        let p = block_policy(vec![rule("bank.com", true, &[]).into()]);
        assert!(!p.is_blocked("https://notbank.com"));
        assert!(!p.is_blocked("https://bank.com.evil.io/login"));
        assert!(!p.is_blocked("https://evil.io/?next=https://bank.com"));
        assert!(!p.is_blocked("https://evil.io/bank.com"));
        assert!(!p.is_blocked("https://bank.com@evil.io/"));
        assert!(!p.is_blocked("not a url at all bank.com"));
        assert!(p.is_blocked("https://online.bank.com"));
    }

    #[test]
    fn subdomain_include_and_exclude_rules() {
        let p = block_policy(vec![rule(
            "example.com",
            true,
            &["public.example.com", "Docs.Example.com."],
        )
        .into()]);
        assert!(p.is_blocked("https://example.com"));
        assert!(p.is_blocked("https://mail.example.com"));
        assert!(p.is_blocked("https://a.b.example.com"));
        // Excluded subdomains and everything below them are left alone.
        assert!(!p.is_blocked("https://public.example.com"));
        assert!(!p.is_blocked("https://cdn.public.example.com"));
        assert!(!p.is_blocked("https://docs.example.com/guide"));
        // A sibling whose name merely ends with the excluded text still matches.
        assert!(p.is_blocked("https://notpublic.example.com"));
    }

    #[test]
    fn legacy_strings_keep_their_old_meaning_inside_a_policy() {
        let patterns = ["chase", "wellsfargo.com", "localhost", "192.168.1.1"];
        let legacy: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
        let from_rules = block_policy(patterns.iter().map(|p| UrlRule::from(*p)).collect());
        let from_strings = UrlPolicy::from_legacy_patterns(&legacy);
        for url in [
            "https://chase.co.uk",
            "https://www.chase.com/login",
            "https://purchase.com",
            "https://online.wellsfargo.com",
            "wellsfargo.com",
            "http://localhost:3000",
            "http://192.168.1.1:8080",
            "http://192.168.1.2",
            "something-wellsfargo.com-something",
            "",
            "not-a-valid-url",
        ] {
            let expected = is_url_blocked(url, &legacy);
            assert_eq!(from_rules.is_blocked(url), expected, "{url}");
            assert_eq!(from_strings.is_blocked(url), expected, "{url}");
        }
    }

    #[test]
    fn allow_list_rejects_sites_that_are_not_listed() {
        let p = UrlPolicy::new(&[], &[rule("docs.example.org", false, &[])]);
        assert!(p.has_allow_list());
        assert_eq!(
            p.decide("chrome.exe", Some("https://docs.example.org/page")),
            UrlDecision::Allowed
        );
        assert_eq!(
            p.decide("chrome.exe", Some("https://news.example.org")),
            UrlDecision::NotAllowed
        );
        assert_eq!(
            p.decide("Google Chrome", Some("https://docs.example.org.evil.io")),
            UrlDecision::NotAllowed
        );
        // Known but host-less pages are not on any allow list.
        assert_eq!(
            p.decide("firefox.exe", Some("about:blank")),
            UrlDecision::NotAllowed
        );
        assert_eq!(
            p.decide("msedge.exe", Some("file:///C:/notes.txt")),
            UrlDecision::NotAllowed
        );
    }

    #[test]
    fn allow_list_with_subdomain_exceptions() {
        let p = UrlPolicy::new(&[], &[rule("example.org", true, &["private.example.org"])]);
        assert!(p
            .decide("Arc", Some("https://wiki.example.org"))
            .is_allowed());
        assert!(p.decide("Arc", Some("example.org/x")).is_allowed());
        assert_eq!(
            p.decide("Arc", Some("https://private.example.org")),
            UrlDecision::NotAllowed
        );
        assert_eq!(
            p.decide("Arc", Some("https://a.private.example.org")),
            UrlDecision::NotAllowed
        );
    }

    #[test]
    fn unverifiable_browser_url_is_pending_under_an_allow_list() {
        let p = UrlPolicy::new(&[], &[rule("example.org", true, &[])]);
        assert_eq!(p.decide("chrome.exe", None), UrlDecision::Pending);
        assert_eq!(p.decide("chrome.exe", Some("   ")), UrlDecision::Pending);
        assert_eq!(
            p.decide("chrome.exe", Some("https://exa mple.org")),
            UrlDecision::Pending
        );
        assert_eq!(
            UrlDecision::Pending.skip_reason(),
            Some(crate::tree::SkipReason::UrlValidationPending)
        );
        assert!(p.needs_verified_url("Microsoft Edge"));
        assert!(!p.needs_verified_url("Code.exe"));
    }

    #[test]
    fn unknown_app_under_an_allow_list_is_not_recorded() {
        let p = UrlPolicy::new(&[], &[rule("example.org", false, &[])]);
        assert_eq!(p.decide_for_capture(None, None), UrlDecision::Pending);
        assert_eq!(p.decide_for_capture(Some(" "), None), UrlDecision::Pending);
        assert!(p.decide_for_capture(Some("Code.exe"), None).is_allowed());
        assert_eq!(
            p.decide_for_capture(Some("chrome.exe"), None),
            UrlDecision::Pending
        );
        // Without an allow list an unknown app is recorded unless its URL
        // is blocked.
        let block = block_policy(vec![rule("bank.com", false, &[]).into()]);
        assert!(block.decide_for_capture(None, None).is_allowed());
        assert_eq!(
            block.decide_for_capture(None, Some("https://bank.com")),
            UrlDecision::Blocked
        );
    }

    #[test]
    fn allow_list_leaves_native_apps_alone() {
        let p = UrlPolicy::new(&[], &[rule("example.org", false, &[])]);
        assert!(p.decide("Code.exe", None).is_allowed());
        assert!(p
            .decide("slack.exe", Some("https://app.slack.com"))
            .is_allowed());
        assert!(p.decide("SearchHost.exe", None).is_allowed());
    }

    #[test]
    fn block_beats_allow() {
        let p = UrlPolicy::new(
            &[rule("secret.example.org", false, &[]).into()],
            &[rule("example.org", true, &[])],
        );
        assert_eq!(
            p.decide("chrome.exe", Some("https://secret.example.org")),
            UrlDecision::Blocked
        );
        assert!(p
            .decide("chrome.exe", Some("https://www.example.org"))
            .is_allowed());
        // The block list also covers non-browser apps that report a URL.
        assert_eq!(
            p.decide("slack.exe", Some("https://secret.example.org/x")),
            UrlDecision::Blocked
        );
        assert_eq!(
            UrlDecision::Blocked.skip_reason(),
            Some(crate::tree::SkipReason::BlockedUrl)
        );
    }

    #[test]
    fn allow_list_of_broken_rules_allows_nothing() {
        let p = UrlPolicy::new(&[], &[rule("localhost", false, &[]), rule("", false, &[])]);
        assert!(p.has_allow_list());
        assert_eq!(p.rejected_rules().len(), 2);
        assert_eq!(
            p.decide("chrome.exe", Some("http://localhost:3000")),
            UrlDecision::NotAllowed
        );
        assert_eq!(
            p.decide("chrome.exe", Some("https://example.com")),
            UrlDecision::NotAllowed
        );
    }

    #[test]
    fn broken_block_rule_falls_back_to_broad_legacy_match() {
        // Exceptions without includeSubdomains are invalid: the rule then
        // blocks the domain and everything below it.
        let p = block_policy(vec![rule("bank.com", false, &["help.bank.com"]).into()]);
        assert_eq!(p.rejected_rules().len(), 1);
        assert_eq!(p.rejected_rules()[0].list, UrlRuleList::Blocked);
        assert!(p.is_blocked("https://bank.com"));
        assert!(p.is_blocked("https://help.bank.com"));
        assert!(!p.is_blocked("https://notbank.com"));
    }

    #[test]
    fn empty_lists_make_an_empty_policy() {
        let p = UrlPolicy::new(&[UrlRule::from("  ")], &[]);
        assert!(p.is_empty());
        assert!(p.decide("chrome.exe", None).is_allowed());
        assert!(UrlPolicy::from_legacy_patterns(&[]).is_empty());
        assert!(UrlPolicy::default()
            .decide("chrome.exe", Some("x"))
            .is_allowed());
    }

    #[test]
    fn validation_messages() {
        use UrlRuleError::*;
        assert_eq!(
            validate_domain_rule(&rule("", false, &[])),
            Err(EmptyDomain)
        );
        assert_eq!(
            validate_domain_rule(&rule("ftp://files.example.com", false, &[])),
            Err(UnsupportedScheme("ftp".into()))
        );
        assert!(matches!(
            validate_domain_rule(&rule("https://", false, &[])),
            Err(MalformedDomain(_)) | Err(MissingHost(_))
        ));
        assert_eq!(
            validate_domain_rule(&rule("localhost", false, &[])),
            Err(NeedsDot("localhost".into()))
        );
        for bad in [
            "exa mple.com",
            "*.example.com",
            "example..com",
            "-a.example.com",
            "a.com:80",
        ] {
            assert!(
                matches!(
                    validate_domain_rule(&rule(bad, false, &[])),
                    Err(MalformedDomain(_))
                ),
                "{bad}"
            );
        }
        assert_eq!(
            validate_domain_rule(&rule("example.com", false, &["a.example.com"])),
            Err(ExclusionsWithoutSubdomains)
        );
        assert_eq!(
            validate_domain_rule(&rule("example.com", true, &["example.org"])),
            Err(ExclusionOutsideDomain {
                excluded: "example.org".into(),
                domain: "example.com".into()
            })
        );
        assert!(matches!(
            validate_domain_rule(&rule("example.com", true, &["example.com"])),
            Err(ExclusionOutsideDomain { .. })
        ));
        assert!(matches!(
            validate_domain_rule(&rule("example.com", true, &["bad host.example.com"])),
            Err(InvalidExclusion { .. })
        ));
        // Accepted forms: bare host, URL form, mixed case, trailing dot, IDN.
        for ok in [
            "example.com",
            "https://Example.com/some/path",
            "http://sub.example.com:8080",
            "Example.COM.",
            "bücher.example",
        ] {
            assert_eq!(validate_domain_rule(&rule(ok, false, &[])), Ok(()), "{ok}");
        }
        assert_eq!(
            validate_domain_rule(&rule("example.com", true, &["a.example.com", " "])),
            Ok(())
        );
        // Every message reads as a sentence fragment, never empty.
        assert!(!NeedsDot("x".into()).to_string().is_empty());
    }

    #[test]
    fn url_form_and_idn_rules_match_parsed_hosts() {
        let p = UrlPolicy::new(
            &[],
            &[
                rule("https://Docs.Example.com/path", false, &[]),
                rule("bücher.example", false, &[]),
            ],
        );
        assert!(p
            .decide("firefox", Some("https://docs.example.com"))
            .is_allowed());
        assert!(p
            .decide("firefox", Some("https://xn--bcher-kva.example/"))
            .is_allowed());
        assert!(p
            .decide("firefox", Some("https://bücher.example/"))
            .is_allowed());
    }

    #[test]
    fn browser_names_match_whole_words_only() {
        for name in [
            "chrome.exe",
            "Google Chrome",
            "msedge.exe",
            "Microsoft Edge",
            "firefox",
            "Firefox Developer Edition",
            "brave-browser",
            "Arc",
            "Safari",
            "Zen Browser",
            "opera.exe",
            "vivaldi",
            "Comet",
            "epiphany",
            "helium.exe",
            "Chromium",
        ] {
            assert!(is_policy_browser(name), "{name}");
        }
        for name in [
            "SearchHost.exe",
            "ArcGIS Pro",
            "Knowledge Base",
            "Code.exe",
            "",
            "explorer.exe",
        ] {
            assert!(!is_policy_browser(name), "{name}");
        }
    }
}
