// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Typed pipe.md fields added with the v2 pipe format: `effort`, `run_in`,
//! `artifacts`, `cloud_agent` and `trigger.sources`, plus their checks.
//!
//! Loading stays lenient so a pipe.md written by screenpipe (or by hand)
//! loads even when a value is one this build does not act on; the checks in
//! this module run when a client changes the config over the API, and give an
//! error that names the field and the accepted values.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use tracing::warn;

// ---------------------------------------------------------------------------
// effort
// ---------------------------------------------------------------------------

/// How hard the agent should think on a run (`effort:`). Passed to the
/// agent as its reasoning or thinking level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipeEffort {
    Low,
    Medium,
    High,
}

impl PipeEffort {
    /// The accepted values, for error messages.
    pub const ALLOWED: &'static str = "low, medium or high";

    pub fn as_str(self) -> &'static str {
        match self {
            PipeEffort::Low => "low",
            PipeEffort::Medium => "medium",
            PipeEffort::High => "high",
        }
    }

    /// Read an effort level; case and surrounding spaces do not matter.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "low" => Some(PipeEffort::Low),
            "medium" => Some(PipeEffort::Medium),
            "high" => Some(PipeEffort::High),
            _ => None,
        }
    }
}

impl fmt::Display for PipeEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for PipeEffort {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PipeEffort {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        PipeEffort::parse(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "effort '{raw}' is not supported: use {}",
                PipeEffort::ALLOWED
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// run_in
// ---------------------------------------------------------------------------

/// `run_in.mode` that delivers runs into an existing chat.
pub const RUN_IN_EXISTING_CHAT: &str = "existing_chat";

/// Longest chat id accepted in `run_in.chat_id`.
pub const MAX_CHAT_ID_LEN: usize = 200;

/// Where a pipe's runs are delivered (`run_in:`). Without it a run shows up
/// as a standalone pipe execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipeRunIn {
    /// Only `existing_chat` is defined. Kept as text so a pipe naming a mode
    /// this build does not know still loads.
    pub mode: String,
    /// The chat conversation that receives the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
}

impl PipeRunIn {
    /// A destination that posts runs into chat `chat_id`.
    pub fn existing_chat(chat_id: impl Into<String>) -> Self {
        Self {
            mode: RUN_IN_EXISTING_CHAT.to_string(),
            chat_id: Some(chat_id.into()),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.mode != RUN_IN_EXISTING_CHAT {
            return Err(format!(
                "mode '{}' is not supported: use \"{RUN_IN_EXISTING_CHAT}\"",
                self.mode
            ));
        }
        let chat_id = self
            .chat_id
            .as_deref()
            .ok_or_else(|| "chat_id is required for an existing_chat destination".to_string())?;
        validate_chat_id(chat_id)
    }
}

/// A chat id names a conversation file, so it must be one plain file-name
/// component.
pub fn validate_chat_id(chat_id: &str) -> Result<(), String> {
    let problem = if chat_id.trim().is_empty() {
        Some("is empty")
    } else if chat_id != chat_id.trim() {
        Some("has leading or trailing spaces")
    } else if chat_id.chars().count() > MAX_CHAT_ID_LEN {
        Some("is longer than 200 characters")
    } else if chat_id == "." || chat_id == ".." || chat_id.contains(['/', '\\']) {
        Some("looks like a path")
    } else if chat_id.chars().any(char::is_control) {
        Some("contains a control character")
    } else {
        None
    };
    match problem {
        Some(why) => Err(format!("chat_id {chat_id:?} {why}")),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// artifacts
// ---------------------------------------------------------------------------

/// Artifact kinds a declaration may name.
pub const ARTIFACT_KINDS: &[&str] = &["html", "json", "markdown"];

/// An output file a pipe promises to produce (`artifacts:` entries), with a
/// path relative to the pipe folder, normally under `output/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArtifactDeclaration {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `html`, `json` or `markdown`; inferred from the extension when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

impl<'de> Deserialize<'de> for ArtifactDeclaration {
    /// An entry is either a map `{path, title?, kind?}` or just the path.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Full {
            path: String,
            #[serde(default)]
            title: Option<String>,
            #[serde(default)]
            kind: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Entry {
            Path(String),
            Full(Full),
        }
        Ok(match Entry::deserialize(deserializer)? {
            Entry::Path(path) => ArtifactDeclaration {
                path,
                title: None,
                kind: None,
            },
            Entry::Full(f) => ArtifactDeclaration {
                path: f.path,
                title: f.title,
                kind: f.kind,
            },
        })
    }
}

impl ArtifactDeclaration {
    /// The declared file inside `pipe_dir`, or `None` when the path is empty,
    /// absolute, or climbs out of the folder. Purely lexical: nothing is read
    /// from disk.
    pub fn resolve_in(&self, pipe_dir: &Path) -> Option<PathBuf> {
        let relative = relative_inside(&self.path)?;
        Some(pipe_dir.join(relative))
    }

    /// The kind to register the file under: the declared one, or a guess
    /// from the extension (`.html`/`.htm`, `.json`, `.md`/`.markdown`).
    pub fn effective_kind(&self) -> Option<String> {
        if let Some(kind) = self
            .kind
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            return Some(kind.to_ascii_lowercase());
        }
        let ext = Path::new(&self.path)
            .extension()?
            .to_string_lossy()
            .to_ascii_lowercase();
        match ext.as_str() {
            "html" | "htm" => Some("html".into()),
            "json" => Some("json".into()),
            "md" | "markdown" => Some("markdown".into()),
            _ => None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if relative_inside(&self.path).is_none() {
            return Err(format!(
                "artifact path {:?} must be a relative path inside the pipe folder, such as output/report.md",
                self.path
            ));
        }
        if let Some(kind) = self.kind.as_deref() {
            if !ARTIFACT_KINDS.contains(&kind.trim().to_ascii_lowercase().as_str()) {
                return Err(format!(
                    "artifact kind '{kind}' is not supported: use {}",
                    ARTIFACT_KINDS.join(", ")
                ));
            }
        }
        if self.title.as_deref().is_some_and(|t| t.trim().is_empty()) {
            return Err(format!("artifact {:?} has an empty title", self.path));
        }
        Ok(())
    }
}

/// `raw` as a path below the pipe folder, with `.` segments dropped, or
/// `None` if it is empty, rooted, carries a drive prefix or uses `..`.
fn relative_inside(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    // Treat both separators alike so a Windows-style path is judged the same
    // way on every platform.
    let unified = trimmed.replace('\\', "/");
    if unified.is_empty() || unified.starts_with('/') || unified.contains(':') {
        return None;
    }
    let mut out = PathBuf::new();
    for component in Path::new(&unified).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// The declared artifacts of a pipe that resolve inside `pipe_dir`, paired
/// with their absolute paths. Declarations that would escape the folder are
/// skipped with a warning.
pub fn declared_artifacts(
    pipe_name: &str,
    artifacts: &[ArtifactDeclaration],
    pipe_dir: &Path,
) -> Vec<(ArtifactDeclaration, PathBuf)> {
    artifacts
        .iter()
        .filter_map(|decl| match decl.resolve_in(pipe_dir) {
            Some(path) => Some((decl.clone(), path)),
            None => {
                warn!(
                    "pipe '{}': skipping artifact {:?}, its path leaves the pipe folder",
                    pipe_name, decl.path
                );
                None
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// trigger.sources
// ---------------------------------------------------------------------------

/// Apps a connection trigger can watch.
pub const SOURCE_TRIGGER_APPS: &[&str] = &[
    "slack",
    "notion",
    "imap",
    "google-calendar",
    "outlook-email",
    "github",
    "linear",
    "todoist",
    "obsidian",
    "voice",
];

/// Item kinds a connection trigger can match.
pub const SOURCE_TRIGGER_KINDS: &[&str] = &[
    "item",
    "task",
    "issue_created",
    "issue",
    "event_started",
    "page",
    "note",
    "phrase",
];

/// One connection the pipe watches for new items (`trigger.sources:`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceTrigger {
    /// Connection id, e.g. `slack`, `github`, `voice`.
    pub app: String,
    /// Which connected instance (account, workspace) when there are several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// What an item must match: free text, or a structure such as a phrase
    /// list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<serde_json::Value>,
    /// Only items of this kind (see [`SOURCE_TRIGGER_KINDS`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// App-specific location: a channel, repository, database or vault path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl SourceTrigger {
    pub fn validate(&self) -> Result<(), String> {
        let app = self.app.trim();
        if app.is_empty() {
            return Err("a source trigger needs an app".into());
        }
        if !SOURCE_TRIGGER_APPS.contains(&app) {
            return Err(format!(
                "source trigger app '{app}' is not supported: use one of {}",
                SOURCE_TRIGGER_APPS.join(", ")
            ));
        }
        if let Some(kind) = self.kind.as_deref() {
            if !SOURCE_TRIGGER_KINDS.contains(&kind.trim()) {
                return Err(format!(
                    "source trigger kind '{kind}' is not supported: use one of {}",
                    SOURCE_TRIGGER_KINDS.join(", ")
                ));
            }
        }
        if self
            .instance
            .as_deref()
            .is_some_and(|i| i.trim().is_empty())
        {
            return Err(format!("source trigger for '{app}' has an empty instance"));
        }
        if self.path.as_deref().is_some_and(|p| p.trim().is_empty()) {
            return Err(format!("source trigger for '{app}' has an empty path"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// cloud_agent
// ---------------------------------------------------------------------------

/// Providers `cloud_agent.provider` may name.
pub const CLOUD_AGENT_PROVIDERS: &[&str] = &["cursor", "codex", "claude"];
/// Upper bound for `cloud_agent.context_lookback_hours` (30 days).
pub const MAX_CONTEXT_LOOKBACK_HOURS: i64 = 720;
/// Upper bound for `cloud_agent.context_max_items`.
pub const MAX_CONTEXT_ITEMS: i64 = 1000;

/// Settings for `agent: cloud-agent`, which hands the task to a third-party
/// coding agent the user has connected with their own account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudAgentConfig {
    /// `cursor`, `codex` or `claude`.
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starting_ref: Option<String>,
    /// Attach a bounded summary of recent screen activity to the task. Off
    /// unless the pipe opts in, because it leaves the machine.
    #[serde(
        default,
        deserialize_with = "deserialize_flag",
        skip_serializing_if = "is_false"
    )]
    pub send_screenpipe_context: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_lookback_hours: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_max_items: Option<i64>,
}

impl CloudAgentConfig {
    pub fn validate(&self) -> Result<(), String> {
        let provider = self.provider.trim();
        if !CLOUD_AGENT_PROVIDERS.contains(&provider) {
            return Err(format!(
                "cloud agent provider '{provider}' is not supported: use {}",
                CLOUD_AGENT_PROVIDERS.join(", ")
            ));
        }
        if let Some(h) = self.context_lookback_hours {
            if !(1..=MAX_CONTEXT_LOOKBACK_HOURS).contains(&h) {
                return Err(format!(
                    "context_lookback_hours must be from 1 to {MAX_CONTEXT_LOOKBACK_HOURS}, got {h}"
                ));
            }
        }
        if let Some(n) = self.context_max_items {
            if !(1..=MAX_CONTEXT_ITEMS).contains(&n) {
                return Err(format!(
                    "context_max_items must be from 1 to {MAX_CONTEXT_ITEMS}, got {n}"
                ));
            }
        }
        for (field, value) in [
            ("environment_id", &self.environment_id),
            ("branch", &self.branch),
            ("session_id", &self.session_id),
            ("agent_id", &self.agent_id),
            ("repository", &self.repository),
            ("starting_ref", &self.starting_ref),
        ] {
            if let Some(v) = value {
                if v.trim().is_empty() {
                    return Err(format!("{field} is empty; leave it out instead"));
                }
                if v.chars().any(char::is_control) {
                    return Err(format!("{field} contains a control character"));
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Flags
// ---------------------------------------------------------------------------

fn is_false(b: &bool) -> bool {
    !*b
}

/// `artifacts:` where null or a missing key means no artifacts.
pub fn deserialize_artifacts<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<ArtifactDeclaration>, D::Error> {
    Ok(Option::<Vec<ArtifactDeclaration>>::deserialize(deserializer)?.unwrap_or_default())
}

/// `trigger.sources:` where null or a missing key means no sources.
pub fn deserialize_sources<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<SourceTrigger>, D::Error> {
    Ok(Option::<Vec<SourceTrigger>>::deserialize(deserializer)?.unwrap_or_default())
}

/// Read a yes/no frontmatter flag. Besides YAML booleans it takes the
/// spellings people type by hand (`yes`/`no`, `on`/`off`, `1`/`0`, any case)
/// and treats null as false.
pub fn deserialize_flag<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Flag {
        Bool(bool),
        Int(i64),
        Text(String),
    }
    match Option::<Flag>::deserialize(deserializer)? {
        None => Ok(false),
        Some(Flag::Bool(b)) => Ok(b),
        Some(Flag::Int(0)) => Ok(false),
        Some(Flag::Int(1)) => Ok(true),
        Some(Flag::Int(n)) => Err(serde::de::Error::custom(format!(
            "expected true or false, got {n}"
        ))),
        Some(Flag::Text(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Ok(true),
            "false" | "no" | "off" | "0" | "" => Ok(false),
            _ => Err(serde::de::Error::custom(format!(
                "expected true or false, got '{s}'"
            ))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn effort_parses_case_insensitively_and_names_allowed_values() {
        assert_eq!(PipeEffort::parse(" High "), Some(PipeEffort::High));
        let err = serde_json::from_value::<PipeEffort>(json!("max"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("low, medium or high"), "{err}");
        assert_eq!(
            serde_json::to_value(PipeEffort::Medium).unwrap(),
            json!("medium")
        );
    }

    #[test]
    fn run_in_requires_existing_chat_and_a_safe_chat_id() {
        assert!(
            PipeRunIn::existing_chat("4f1c2d9e-0b7a-4b44-9d8e-5f0c1a2b3c4d")
                .validate()
                .is_ok()
        );
        let wrong_mode = PipeRunIn {
            mode: "new_chat".into(),
            chat_id: Some("abc".into()),
        };
        assert!(wrong_mode.validate().unwrap_err().contains("existing_chat"));
        let missing = PipeRunIn {
            mode: RUN_IN_EXISTING_CHAT.into(),
            chat_id: None,
        };
        assert!(missing.validate().unwrap_err().contains("chat_id"));
        for bad in [
            "",
            "  ",
            "../chats/x",
            "a/b",
            "a\\b",
            " pad",
            "nl\n",
            &"c".repeat(201),
        ] {
            assert!(
                PipeRunIn::existing_chat(bad).validate().is_err(),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn artifacts_accept_maps_or_bare_paths() {
        let parsed: Vec<ArtifactDeclaration> = serde_yaml::from_str(
            "- path: output/latest.md\n  title: Latest change\n  kind: markdown\n- output/data.json\n",
        )
        .unwrap();
        assert_eq!(parsed[0].title.as_deref(), Some("Latest change"));
        assert_eq!(parsed[1].path, "output/data.json");
        assert_eq!(parsed[1].effective_kind().as_deref(), Some("json"));
    }

    #[test]
    fn artifact_paths_cannot_leave_the_pipe_folder() {
        let dir = Path::new("/data/pipes/demo");
        let decl = |p: &str| ArtifactDeclaration {
            path: p.into(),
            title: None,
            kind: None,
        };
        assert_eq!(
            decl("./output/report.md").resolve_in(dir),
            Some(dir.join("output").join("report.md"))
        );
        for bad in [
            "",
            "../other/x.md",
            "output/../../x.md",
            "/etc/passwd",
            "C:\\Windows\\x",
            "..\\x.md",
            "\\\\server\\share\\x",
            ".",
        ] {
            assert!(
                decl(bad).resolve_in(dir).is_none(),
                "{bad:?} should be refused"
            );
            assert!(
                decl(bad).validate().is_err(),
                "{bad:?} should fail validation"
            );
        }
        let kept = declared_artifacts(
            "demo",
            &[
                decl("output/a.md"),
                decl("../escape.md"),
                decl("output/b.html"),
            ],
            dir,
        );
        let paths: Vec<_> = kept.iter().map(|(d, _)| d.path.as_str()).collect();
        assert_eq!(paths, ["output/a.md", "output/b.html"]);
    }

    #[test]
    fn artifact_kind_must_be_known() {
        let decl = ArtifactDeclaration {
            path: "output/x.txt".into(),
            title: Some("X".into()),
            kind: Some("pdf".into()),
        };
        assert!(decl
            .validate()
            .unwrap_err()
            .contains("html, json, markdown"));
    }

    #[test]
    fn source_triggers_check_app_and_kind() {
        let ok: SourceTrigger = serde_json::from_value(json!({
            "app": "github", "path": "octo/repo", "kind": "issue_created", "filter": "bug"
        }))
        .unwrap();
        assert!(ok.validate().is_ok());
        let voice: SourceTrigger = serde_json::from_value(json!({
            "app": "voice", "filter": {"phrases": ["remind me"]}, "kind": "phrase"
        }))
        .unwrap();
        assert!(voice.validate().is_ok());
        let bad_app: SourceTrigger = serde_json::from_value(json!({"app": "myspace"})).unwrap();
        assert!(bad_app.validate().unwrap_err().contains("myspace"));
        let bad_kind: SourceTrigger =
            serde_json::from_value(json!({"app": "slack", "kind": "emoji"})).unwrap();
        assert!(bad_kind.validate().unwrap_err().contains("emoji"));
    }

    #[test]
    fn cloud_agent_checks_provider_and_bounds() {
        let ok: CloudAgentConfig = serde_json::from_value(json!({
            "provider": "cursor", "repository": "octo/repo", "send_screenpipe_context": true,
            "context_lookback_hours": 24, "context_max_items": 50
        }))
        .unwrap();
        assert!(ok.validate().is_ok());
        assert!(ok.send_screenpipe_context);
        let off: CloudAgentConfig = serde_json::from_value(json!({"provider": "codex"})).unwrap();
        assert!(!off.send_screenpipe_context, "context sharing is opt-in");
        for bad in [
            json!({"provider": "devin"}),
            json!({"provider": "claude", "context_lookback_hours": 0}),
            json!({"provider": "claude", "context_max_items": 5000}),
            json!({"provider": "claude", "branch": "  "}),
        ] {
            let parsed: CloudAgentConfig = serde_json::from_value(bad.clone()).unwrap();
            assert!(parsed.validate().is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn flags_accept_hand_written_spellings() {
        #[derive(Deserialize)]
        struct Holder {
            #[serde(default, deserialize_with = "deserialize_flag")]
            flag: bool,
        }
        let read = |yaml: &str| serde_yaml::from_str::<Holder>(yaml).map(|h| h.flag);
        assert!(read("flag: true").unwrap());
        assert!(read("flag: yes").unwrap());
        assert!(read("flag: On").unwrap());
        assert!(read("flag: 1").unwrap());
        assert!(!read("flag: no").unwrap());
        assert!(!read("flag: 0").unwrap());
        assert!(!read("flag: null").unwrap());
        assert!(!read("other: 1").unwrap());
        assert!(read("flag: maybe").is_err());
    }
}
