// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Prefilled "new issue" link for the Handvel tracker.
//!
//! Opening the link only shows GitHub's issue form in the browser; nothing
//! is sent until the user submits it, and the bundle is never attached
//! automatically.

/// Where new issues are filed.
pub const NEW_ISSUE_URL: &str = "https://github.com/normiecore/Freenpipe/issues/new";

/// Stay well under the URL length browsers and GitHub accept.
const MAX_URL_CHARS: usize = 7_000;
/// Longest excerpt of the user's text placed in the body.
const MAX_EXCERPT_CHARS: usize = 1_500;
const MAX_TITLE_CHARS: usize = 80;

/// Facts that go into the issue.
#[derive(Debug, Clone, Default)]
pub struct IssueDetails<'a> {
    /// The user's (already scrubbed) description.
    pub feedback: &'a str,
    pub app_version: &'a str,
    pub os_platform: &'a str,
    pub os_version: &'a str,
    pub os_arch: &'a str,
    pub source: &'a str,
    /// File name of the bundle, never its full path (that holds the user name).
    pub bundle_file_name: Option<&'a str>,
}

fn take_chars(text: &str, max: usize) -> (String, bool) {
    let mut out: String = text.chars().take(max).collect();
    let cut = out.len() < text.len();
    if cut {
        out.push('…');
    }
    (out, cut)
}

/// Issue title: first line of the feedback, or a generic one.
pub fn issue_title(details: &IssueDetails<'_>) -> String {
    let first_line = details
        .feedback
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first_line.is_empty() {
        return match details.source {
            "" | "feedback" => "Problem report".to_string(),
            other => format!("Problem report ({other})"),
        };
    }
    take_chars(first_line, MAX_TITLE_CHARS).0
}

fn issue_body(details: &IssueDetails<'_>, excerpt_chars: usize) -> String {
    let feedback = details.feedback.trim();
    let mut body = String::new();
    body.push_str("### What happened\n\n");
    if feedback.is_empty() || excerpt_chars == 0 {
        body.push_str("_Describe what you did, what you expected and what happened instead._\n\n");
    } else {
        body.push_str(&take_chars(feedback, excerpt_chars).0);
        body.push_str("\n\n");
    }
    body.push_str("### Environment\n\n");
    body.push_str(&format!("- Handvel: {}\n", details.app_version));
    body.push_str(&format!(
        "- OS: {} {} ({})\n",
        details.os_platform, details.os_version, details.os_arch
    ));
    if !details.source.is_empty() {
        body.push_str(&format!("- Reported from: {}\n", details.source));
    }
    body.push_str("\n### Diagnostics\n\n");
    match details.bundle_file_name {
        Some(name) => body.push_str(&format!(
            "A diagnostics bundle was saved on my computer as `{name}` (in the Handvel \
             data folder, under `feedback`). Drag it into this issue if you are happy to \
             share it; check its text files first.\n"
        )),
        None => body.push_str("No diagnostics bundle attached.\n"),
    }
    body
}

/// Full `issues/new` URL with title and body filled in. The excerpt is
/// shortened until the URL fits.
pub fn new_issue_url(details: &IssueDetails<'_>) -> String {
    let title = issue_title(details);
    let mut excerpt = MAX_EXCERPT_CHARS;
    loop {
        let body = issue_body(details, excerpt);
        let url = url::Url::parse_with_params(NEW_ISSUE_URL, [("title", &title), ("body", &body)])
            .map(String::from)
            .unwrap_or_else(|_| NEW_ISSUE_URL.to_string());
        if url.len() <= MAX_URL_CHARS || excerpt == 0 {
            return url;
        }
        excerpt /= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn details(feedback: &str) -> IssueDetails<'_> {
        IssueDetails {
            feedback,
            app_version: "2.5.28",
            os_platform: "windows",
            os_version: "10.0.22631",
            os_arch: "x86_64",
            source: "feedback",
            bundle_file_name: Some("20261015-143005.zip"),
        }
    }

    fn query(url: &str) -> std::collections::HashMap<String, String> {
        url::Url::parse(url).unwrap().query_pairs().into_owned().collect()
    }

    #[test]
    fn targets_the_freenpipe_tracker_with_a_prefilled_form() {
        let url = new_issue_url(&details("Timeline stays blank\nafter wake from sleep"));
        assert!(url.starts_with("https://github.com/normiecore/Freenpipe/issues/new?"), "{url}");
        let q = query(&url);
        assert_eq!(q["title"], "Timeline stays blank");
        let body = &q["body"];
        assert!(body.contains("after wake from sleep"));
        assert!(body.contains("- Handvel: 2.5.28"));
        assert!(body.contains("- OS: windows 10.0.22631 (x86_64)"));
        assert!(body.contains("`20261015-143005.zip`"));
    }

    #[test]
    fn empty_feedback_gets_a_generic_title_and_prompt() {
        let mut d = details("   \n");
        d.source = "server-not-running";
        let q = query(&new_issue_url(&d));
        assert_eq!(q["title"], "Problem report (server-not-running)");
        assert!(q["body"].contains("Describe what you did"));
    }

    #[test]
    fn long_feedback_is_shortened_until_the_url_fits() {
        let long = "ü".repeat(20_000);
        let url = new_issue_url(&details(&long));
        assert!(url.len() <= 7_000, "{}", url.len());
        let q = query(&url);
        assert!(q["title"].chars().count() <= MAX_TITLE_CHARS + 1);
        assert!(q["body"].contains("ü"));
    }

    #[test]
    fn never_mentions_a_full_path() {
        let mut d = details("x");
        d.bundle_file_name = None;
        let body = &query(&new_issue_url(&d))["body"];
        assert!(body.contains("No diagnostics bundle attached."));
    }
}
