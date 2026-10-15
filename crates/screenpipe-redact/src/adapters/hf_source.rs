// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Where the optional local PII models come from.
//!
//! The weights for the ML redactors live in Hugging Face repositories. They
//! are fetched only after the user turns on local AI PII removal; the regex
//! redactor, which is the default, needs no download at all. Two settings let
//! users reach the files through a mirror:
//!
//! * `HF_ENDPOINT` — a Hugging Face compatible host (a corporate proxy, a
//!   regional mirror), the same variable the hub's own tooling reads;
//! * one `SCREENPIPE_PII_*_REPO` variable per model family — an `owner/name`
//!   repository id to read instead of the default one.
//!
//! The adapters keep pinning every file to a SHA-256, so a mirror can make
//! the weights reachable but cannot swap them for different ones.

use tracing::warn;

/// Environment variable naming a Hugging Face compatible host.
pub const HF_ENDPOINT_ENV: &str = "HF_ENDPOINT";

/// Host used when `HF_ENDPOINT` is unset.
pub const DEFAULT_HF_ENDPOINT: &str = "https://huggingface.co";

/// One model repository and the revision the adapter is pinned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HfModelSource {
    /// Repository used unless the user overrides it.
    pub default_repo: &'static str,
    /// Environment variable holding an `owner/name` override.
    pub repo_env: &'static str,
    /// Branch, tag or commit the files are read from.
    pub revision: &'static str,
}

/// ONNX text redactor (xlm-roberta token classifier).
pub const TEXT_REDACTOR: HfModelSource = HfModelSource {
    default_repo: "screenpipe/pii-redactor",
    repo_env: "SCREENPIPE_PII_REDACTOR_REPO",
    revision: "main",
};

/// OPF text redactor checkpoint (candle).
pub const OPF_TEXT_REDACTOR: HfModelSource = HfModelSource {
    default_repo: "screenpipe/pii-text-redactor",
    repo_env: "SCREENPIPE_PII_TEXT_REDACTOR_REPO",
    revision: "v6",
};

/// RF-DETR image detector (ONNX and MLX weights).
pub const IMAGE_REDACTOR: HfModelSource = HfModelSource {
    default_repo: "screenpipe/pii-image-redactor",
    repo_env: "SCREENPIPE_PII_IMAGE_REDACTOR_REPO",
    revision: "main",
};

/// `owner/name` with the characters the hub allows in each half.
pub fn is_valid_repo_id(value: &str) -> bool {
    let mut parts = value.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let ok = |s: &str| {
        !s.is_empty()
            && !s.starts_with('.')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    ok(owner) && ok(name)
}

fn endpoint_or_default(raw: Option<&str>) -> String {
    match raw.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) if (v.starts_with("https://") || v.starts_with("http://")) && !v.contains(' ') => {
            v.trim_end_matches('/').to_string()
        }
        Some(v) => {
            warn!("{HF_ENDPOINT_ENV}={v:?} is not an http(s) URL; using {DEFAULT_HF_ENDPOINT}");
            DEFAULT_HF_ENDPOINT.to_string()
        }
        None => DEFAULT_HF_ENDPOINT.to_string(),
    }
}

impl HfModelSource {
    /// Repository after applying a (validated) override.
    pub fn repo_with(&self, repo_override: Option<&str>) -> String {
        match repo_override.map(str::trim).filter(|v| !v.is_empty()) {
            Some(v) if is_valid_repo_id(v) => v.to_string(),
            Some(v) => {
                warn!(
                    "{}={v:?} is not an owner/name repository id; using {}",
                    self.repo_env, self.default_repo
                );
                self.default_repo.to_string()
            }
            None => self.default_repo.to_string(),
        }
    }

    /// URL of `path` inside the repository, from explicit endpoint and
    /// repository overrides (pure; used by tests and by [`Self::file_url`]).
    pub fn file_url_with(
        &self,
        endpoint_override: Option<&str>,
        repo_override: Option<&str>,
        path: &str,
    ) -> String {
        format!(
            "{}/{}/resolve/{}/{}",
            endpoint_or_default(endpoint_override),
            self.repo_with(repo_override),
            self.revision,
            path.trim_start_matches('/')
        )
    }

    /// URL of `path` inside the repository, honouring `HF_ENDPOINT` and the
    /// family's repository override.
    pub fn file_url(&self, path: &str) -> String {
        let endpoint = std::env::var(HF_ENDPOINT_ENV).ok();
        let repo = std::env::var(self.repo_env).ok();
        self.file_url_with(endpoint.as_deref(), repo.as_deref(), path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_point_at_the_public_hub() {
        assert_eq!(
            OPF_TEXT_REDACTOR.file_url_with(None, None, "config.json"),
            "https://huggingface.co/screenpipe/pii-text-redactor/resolve/v6/config.json"
        );
        assert_eq!(
            TEXT_REDACTOR.file_url_with(None, None, "v45_phase5_pruned/tokenizer.json"),
            "https://huggingface.co/screenpipe/pii-redactor/resolve/main/v45_phase5_pruned/tokenizer.json"
        );
        assert_eq!(
            IMAGE_REDACTOR.file_url_with(None, None, "/rfdetr_v12.onnx"),
            "https://huggingface.co/screenpipe/pii-image-redactor/resolve/main/rfdetr_v12.onnx"
        );
    }

    #[test]
    fn endpoint_and_repo_can_be_mirrored() {
        assert_eq!(
            IMAGE_REDACTOR.file_url_with(
                Some("https://hf-mirror.example.test/"),
                Some("acme/pii-image"),
                "rfdetr_v12.onnx"
            ),
            "https://hf-mirror.example.test/acme/pii-image/resolve/main/rfdetr_v12.onnx"
        );
    }

    #[test]
    fn bad_overrides_fall_back_to_defaults() {
        for endpoint in ["ftp://x", "hf-mirror.example.test", "http://a b"] {
            assert!(IMAGE_REDACTOR
                .file_url_with(Some(endpoint), None, "f")
                .starts_with("https://huggingface.co/"));
        }
        for repo in ["noslash", "a/b/c", "../etc", "a/", "/b", "a b/c", "a/.hidden"] {
            assert_eq!(IMAGE_REDACTOR.repo_with(Some(repo)), IMAGE_REDACTOR.default_repo, "{repo:?}");
        }
    }

    #[test]
    fn repo_ids_are_validated() {
        assert!(is_valid_repo_id("screenpipe/pii-redactor"));
        assert!(is_valid_repo_id("Org_1/model.v2"));
        assert!(!is_valid_repo_id("screenpipe"));
        assert!(!is_valid_repo_id("a/b?c"));
    }

    #[test]
    fn override_variables_are_distinct() {
        let names = [
            TEXT_REDACTOR.repo_env,
            OPF_TEXT_REDACTOR.repo_env,
            IMAGE_REDACTOR.repo_env,
        ];
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
    }
}
