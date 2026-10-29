// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Website rules for the `ignoredUrls` (block list) and `includedUrls`
//! (allow list) settings.
//!
//! Older stores hold plain strings in `ignoredUrls`; newer ones may also hold
//! structured domain rules. Both shapes load, and each entry is written back
//! in the shape it was read in, so a store keeps working with every app
//! version that shares it. Matching itself lives with the capture code; this
//! module only defines the stored shapes and a few pure helpers.

use serde::{Deserialize, Serialize};

/// A structured website rule: one registrable host, optionally with its
/// subdomains, minus any excluded subdomains.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DomainRule {
    /// Host name the rule is about, for example `example.com`.
    pub domain: String,
    /// When true the rule also covers hosts below `domain`.
    #[serde(default)]
    pub include_subdomains: bool,
    /// Hosts below `domain` that stay outside the rule even when
    /// `include_subdomains` is on.
    #[serde(default)]
    pub excluded_subdomains: Vec<String>,
}

impl DomainRule {
    /// The rule's host in canonical form: trimmed, lower case, without a
    /// trailing dot.
    pub fn normalized_domain(&self) -> String {
        normalize_host(&self.domain)
    }

    /// Build a rule from a short text form, as typed on a command line:
    /// `example.com` covers that host only, `*.example.com` covers it and
    /// every host below it. Returns `None` for an empty host.
    pub fn from_pattern(pattern: &str) -> Option<Self> {
        let trimmed = pattern.trim();
        let (host, include_subdomains) = match trimmed.strip_prefix("*.") {
            Some(rest) => (rest, true),
            None => (trimmed, false),
        };
        let domain = normalize_host(host);
        if domain.is_empty() {
            return None;
        }
        Some(Self {
            domain,
            include_subdomains,
            excluded_subdomains: Vec::new(),
        })
    }
}

/// One entry of the `ignoredUrls` list: a legacy pattern string or a
/// structured [`DomainRule`]. Serialised without a tag so either shape
/// round-trips exactly as stored.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(untagged)]
pub enum UrlRule {
    /// A free-form pattern from older stores (a host, a URL or a bare word).
    Legacy(String),
    /// A structured domain rule.
    Domain(DomainRule),
}

impl UrlRule {
    /// A plain pattern for consumers that only understand legacy strings.
    ///
    /// A structured rule maps to its domain. Legacy matching treats a domain
    /// as covering its subdomains too, so the mapped pattern can only block
    /// more than the structured rule would, never less. That is the safe
    /// direction for a block list until structured matching is wired in.
    pub fn legacy_pattern(&self) -> String {
        match self {
            UrlRule::Legacy(pattern) => pattern.clone(),
            UrlRule::Domain(rule) => rule.normalized_domain(),
        }
    }

    /// The structured rule, if this entry is one.
    pub fn as_domain_rule(&self) -> Option<&DomainRule> {
        match self {
            UrlRule::Legacy(_) => None,
            UrlRule::Domain(rule) => Some(rule),
        }
    }
}

impl From<String> for UrlRule {
    fn from(value: String) -> Self {
        UrlRule::Legacy(value)
    }
}

impl From<&str> for UrlRule {
    fn from(value: &str) -> Self {
        UrlRule::Legacy(value.to_string())
    }
}

impl From<DomainRule> for UrlRule {
    fn from(value: DomainRule) -> Self {
        UrlRule::Domain(value)
    }
}

/// Legacy patterns for a list of rules, empty entries dropped.
pub fn legacy_patterns(rules: &[UrlRule]) -> Vec<String> {
    rules
        .iter()
        .map(UrlRule::legacy_pattern)
        .filter(|p| !p.trim().is_empty())
        .collect()
}

/// Trim, lower-case and drop one trailing dot from a host name.
pub fn normalize_host(host: &str) -> String {
    let trimmed = host.trim().to_ascii_lowercase();
    trimmed.strip_suffix('.').unwrap_or(&trimmed).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strings_and_objects_load_side_by_side() {
        let rules: Vec<UrlRule> = serde_json::from_value(json!([
            "bank.example",
            {"domain": "mail.example.com", "includeSubdomains": true,
             "excludedSubdomains": ["public.mail.example.com"]},
            {"domain": "shop.example"}
        ]))
        .unwrap();

        assert_eq!(rules[0], UrlRule::Legacy("bank.example".into()));
        let rule = rules[1].as_domain_rule().unwrap();
        assert!(rule.include_subdomains);
        assert_eq!(rule.excluded_subdomains, vec!["public.mail.example.com"]);
        let bare = rules[2].as_domain_rule().unwrap();
        assert!(!bare.include_subdomains);
        assert!(bare.excluded_subdomains.is_empty());
    }

    #[test]
    fn each_entry_is_written_back_in_its_own_shape() {
        let stored = json!([
            "legacy.example",
            {"domain": "a.example", "includeSubdomains": false, "excludedSubdomains": []}
        ]);
        let rules: Vec<UrlRule> = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(serde_json::to_value(&rules).unwrap(), stored);
    }

    #[test]
    fn command_line_patterns_become_domain_rules() {
        let exact = DomainRule::from_pattern(" Docs.Example.org ").unwrap();
        assert_eq!(exact.domain, "docs.example.org");
        assert!(!exact.include_subdomains);
        let wide = DomainRule::from_pattern("*.example.com.").unwrap();
        assert_eq!(wide.domain, "example.com");
        assert!(wide.include_subdomains);
        assert!(DomainRule::from_pattern("  ").is_none());
        assert!(DomainRule::from_pattern("*.").is_none());
    }

    #[test]
    fn legacy_patterns_map_domains_conservatively() {
        let rules = vec![
            UrlRule::from("Bank.Example"),
            UrlRule::from(DomainRule {
                domain: " Mail.Example.COM. ".into(),
                include_subdomains: false,
                excluded_subdomains: vec![],
            }),
            UrlRule::from("   "),
        ];
        assert_eq!(
            legacy_patterns(&rules),
            vec!["Bank.Example".to_string(), "mail.example.com".to_string()]
        );
    }
}
