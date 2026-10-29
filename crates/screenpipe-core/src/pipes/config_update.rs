// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Checking and applying config changes (`POST /pipes/:id/config`, CLI).
//!
//! A change is a JSON object whose keys are pipe.md frontmatter fields at the
//! top level, plus `prompt_body` (the markdown below the frontmatter) and
//! `raw_content` (the whole file). Rules:
//!
//! - a body wrapped in a `config` object is refused with a hint to send the
//!   fields directly, since older docs showed that shape;
//! - a key that is not a known field is refused, so a typo cannot quietly
//!   land in the frontmatter;
//! - each field given is checked on its own (type, allowed values, and the
//!   deeper checks for `schedule_config`, `trigger`, `run_in`, `effort`,
//!   `cloud_agent` and `artifacts`), and fields that were not sent are left
//!   alone even if they would not pass today's checks;
//! - a value equal to what the pipe already has (as `GET /pipes` shows it) is
//!   left alone unchecked, so a client that posts back what it read, such as
//!   the trigger editor or a full-object save, is not refused over a value
//!   this build loaded leniently or a free-form extra already in the file;
//!   inside `trigger` and `artifacts` this holds per key and per entry;
//! - `null` clears an optional field;
//! - `schedule` sent without `schedule_config` replaces a structured
//!   schedule: the settings UI edits only the string.
//!
//! Nothing is applied unless the whole change is valid. A whole-file save
//! (`raw_content`) goes through the same checks for the fields it changes
//! ([`check_file_changes`]).

use super::config_fields::{
    ArtifactDeclaration, CloudAgentConfig, PipeEffort, PipeRunIn, SourceTrigger,
};
use super::pipe_id::InvalidPipeId;
use super::schedule_config::ScheduleConfig;
use super::{
    deserialize_permissions_field, parse_schedule, preset_fallback, PipeConfig, TriggerConfig,
};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt;

/// Frontmatter fields a config change may set.
pub const CONFIG_FIELDS: &[&str] = &[
    "name",
    "schedule",
    "enabled",
    "agent",
    "model",
    "provider",
    "cloud_agent",
    "effort",
    "preset",
    "connections",
    "permissions",
    "timeout",
    "trigger",
    "source_slug",
    "installed_version",
    "source_hash",
    "subagent",
    "history",
    "run_in",
    "privacy_filter",
    "artifacts",
    "schedule_config",
    "event_read_only",
];

/// Display fields kept as free-form frontmatter extras: text values.
pub const UI_TEXT_FIELDS: &[&str] = &["title", "description", "icon"];
/// Display fields kept as free-form frontmatter extras: yes/no values.
pub const UI_FLAG_FIELDS: &[&str] = &["featured", "template"];
/// Keys that address the file rather than one frontmatter field.
pub const CONTENT_FIELDS: &[&str] = &["prompt_body", "raw_content"];

/// Keys applied even when the value matches the current one: `enabled` is
/// also this device's override (the file may say otherwise), and `schedule`
/// sent alone also replaces a structured schedule.
const ALWAYS_APPLIED: &[&str] = &["enabled", "schedule", "name", "prompt_body", "raw_content"];

/// Fields a whole-file save (`raw_content`) is checked on, when it changes
/// them. Other keys in a file are free-form or were never checked on save.
const FILE_CHECKED_FIELDS: &[&str] = &[
    "timeout",
    "trigger",
    "run_in",
    "cloud_agent",
    "artifacts",
    "schedule_config",
    "effort",
    "history",
    "event_read_only",
];

/// Why a config change (or a pipe operation that edits pipe.md) was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeConfigUpdateError {
    /// The pipe id is not a single portable folder name.
    InvalidId(InvalidPipeId),
    /// No pipe with this id is installed.
    NotFound(String),
    /// The pipe is locked by an organisation (never the case in Handvel).
    Managed(String),
    /// The fields were wrapped in a `config` object.
    Envelope,
    /// A key that is not a pipe config field.
    UnknownField(String),
    /// A known field with a value that does not pass its checks.
    Invalid { field: String, message: String },
}

impl PipeConfigUpdateError {
    fn invalid(field: &str, message: impl Into<String>) -> Self {
        PipeConfigUpdateError::Invalid {
            field: field.to_string(),
            message: message.into(),
        }
    }
}

impl fmt::Display for PipeConfigUpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipeConfigUpdateError::InvalidId(e) => write!(f, "{e}"),
            PipeConfigUpdateError::NotFound(name) => write!(f, "pipe '{name}' not found"),
            PipeConfigUpdateError::Managed(name) => write!(
                f,
                "pipe '{name}' is managed by an organisation and cannot be changed on this device"
            ),
            PipeConfigUpdateError::Envelope => f.write_str(
                "config fields go at the top level of the request body, e.g. \
                 {\"schedule\": \"every 1h\"}; do not wrap them in a \"config\" object",
            ),
            PipeConfigUpdateError::UnknownField(field) => write!(
                f,
                "unknown pipe config field '{field}' (fields: {}, {}, {}, {})",
                CONTENT_FIELDS.join(", "),
                CONFIG_FIELDS.join(", "),
                UI_TEXT_FIELDS.join(", "),
                UI_FLAG_FIELDS.join(", ")
            ),
            PipeConfigUpdateError::Invalid { field, message } => {
                write!(f, "invalid {field}: {message}")
            }
        }
    }
}

impl std::error::Error for PipeConfigUpdateError {}

impl From<InvalidPipeId> for PipeConfigUpdateError {
    fn from(e: InvalidPipeId) -> Self {
        PipeConfigUpdateError::InvalidId(e)
    }
}

/// What the caller still has to do after [`apply_config_updates`] besides
/// writing pipe.md.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AppliedConfigUpdate {
    /// `enabled` was set; record it as this device's override too.
    pub enabled: Option<bool>,
}

fn is_known_key(key: &str) -> bool {
    CONFIG_FIELDS.contains(&key)
        || UI_TEXT_FIELDS.contains(&key)
        || UI_FLAG_FIELDS.contains(&key)
        || CONTENT_FIELDS.contains(&key)
}

/// The config as clients read it (`config` in `GET /pipes`): typed fields
/// and free-form extras in one object.
fn client_view(config: &PipeConfig) -> Map<String, Value> {
    match serde_json::to_value(config) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

/// Refuse an envelope body or unknown keys before anything is touched. A
/// key that repeats a free-form extra the pipe already has, with the same
/// value, is an echo and passes.
fn check_update_keys(
    updates: &HashMap<String, Value>,
    current: &Map<String, Value>,
) -> Result<(), PipeConfigUpdateError> {
    let echoed = |key: &str| current.get(key) == updates.get(key);
    if updates.contains_key("config") && !echoed("config") {
        return Err(PipeConfigUpdateError::Envelope);
    }
    let mut unknown: Vec<&String> = updates
        .keys()
        .filter(|k| k.as_str() != "config" && !is_known_key(k) && !echoed(k))
        .collect();
    unknown.sort();
    match unknown.first() {
        Some(key) => Err(PipeConfigUpdateError::UnknownField((*key).clone())),
        None => Ok(()),
    }
}

/// Check every field in `updates` and apply them to `config` and `body`.
/// `raw_content` is the caller's business and is skipped here. On error
/// neither `config` nor `body` is changed.
pub fn apply_config_updates(
    pipe_name: &str,
    config: &mut PipeConfig,
    body: &mut String,
    updates: &HashMap<String, Value>,
) -> Result<AppliedConfigUpdate, PipeConfigUpdateError> {
    let current = client_view(config);
    check_update_keys(updates, &current)?;

    let mut next = config.clone();
    let mut next_body = body.clone();
    let mut applied = AppliedConfigUpdate::default();

    // Sorted so the first reported error does not depend on hash order.
    let mut keys: Vec<&String> = updates.keys().collect();
    keys.sort();
    for key in keys {
        let value = &updates[key];
        let field = key.as_str();
        // What the pipe already has stays as it is, unchecked.
        if !ALWAYS_APPLIED.contains(&field) && current.get(field) == Some(value) {
            continue;
        }
        if CONFIG_FIELDS.contains(&field) {
            // A value set aside at load (kept as written because it could not
            // be read) gives way to the new one.
            next.config.remove(field);
        }
        match field {
            "raw_content" => {}
            "prompt_body" => next_body = text(field, value)?,
            "name" => {
                if !value.is_null() && text(field, value)? != pipe_name {
                    return Err(PipeConfigUpdateError::invalid(
                        field,
                        "a pipe is named after its folder; rename is not supported here",
                    ));
                }
            }
            "schedule" => {
                let schedule = text(field, value)?.trim().to_string();
                // The schedule the pipe already has is not re-checked: older
                // files may carry one this build cannot read.
                if schedule != config.schedule.trim() {
                    check_schedule_string(&schedule)?;
                }
                next.schedule = schedule;
                // A schedule string sent on its own takes over from a
                // structured schedule, which would otherwise keep driving the
                // pipe while the settings UI (which edits only the string)
                // shows the new choice.
                if !updates.contains_key("schedule_config") {
                    next.schedule_config = None;
                    next.config.remove("schedule_config");
                }
            }
            "enabled" => {
                let enabled = strict_bool(field, value)?;
                next.enabled = enabled;
                applied.enabled = Some(enabled);
            }
            "agent" => {
                let agent = text(field, value)?.trim().to_string();
                if agent.is_empty() {
                    return Err(PipeConfigUpdateError::invalid(field, "must not be empty"));
                }
                next.agent = agent;
            }
            "model" => {
                let model = text(field, value)?.trim().to_string();
                if model.is_empty() {
                    return Err(PipeConfigUpdateError::invalid(
                        field,
                        "must not be empty (use \"auto\" for the default)",
                    ));
                }
                next.model = model;
            }
            "provider" => next.provider = optional_text(field, value)?,
            "preset" => {
                if !(value.is_null() || value.is_string() || value.is_array()) {
                    return Err(PipeConfigUpdateError::invalid(
                        field,
                        "must be a preset id, a list of preset ids, or null",
                    ));
                }
                if let Some(items) = value.as_array() {
                    if items.iter().any(|v| !v.is_string()) {
                        return Err(PipeConfigUpdateError::invalid(
                            field,
                            "every entry must be a preset id",
                        ));
                    }
                }
                next.preset = preset_fallback::parse_preset_list(value);
            }
            "connections" => next.connections = string_list(field, value)?,
            "permissions" => {
                next.permissions = deserialize_permissions_field(value.clone())
                    .map_err(|e| PipeConfigUpdateError::invalid(field, e.to_string()))?;
            }
            "timeout" => {
                next.timeout = match value {
                    Value::Null => None,
                    v => match v.as_u64() {
                        Some(secs) if secs > 0 => Some(secs),
                        _ => {
                            return Err(PipeConfigUpdateError::invalid(
                                field,
                                "must be a positive number of seconds, or null for the default",
                            ))
                        }
                    },
                }
            }
            "trigger" => {
                next.trigger = if value.is_null() {
                    None
                } else {
                    Some(merge_trigger(current.get(field), value)?)
                }
            }
            "source_slug" => next.source_slug = optional_text(field, value)?,
            "source_hash" => next.source_hash = optional_text(field, value)?,
            "installed_version" => {
                next.installed_version = match value {
                    Value::Null => None,
                    v => Some(v.as_i64().ok_or_else(|| {
                        PipeConfigUpdateError::invalid(field, "must be a whole number or null")
                    })?),
                }
            }
            "subagent" => next.subagent = flag(field, value)?,
            "privacy_filter" => next.privacy_filter = flag(field, value)?,
            "history" => next.history = flag(field, value)?,
            "event_read_only" => next.event_read_only = flag(field, value)?,
            "effort" => {
                next.effort = match value {
                    Value::Null => None,
                    Value::String(s) => Some(PipeEffort::parse(s).ok_or_else(|| {
                        PipeConfigUpdateError::invalid(
                            field,
                            format!("'{s}' is not supported: use {}", PipeEffort::ALLOWED),
                        )
                    })?),
                    _ => {
                        return Err(PipeConfigUpdateError::invalid(
                            field,
                            format!("must be {} or null", PipeEffort::ALLOWED),
                        ))
                    }
                }
            }
            "run_in" => {
                next.run_in = if value.is_null() {
                    None
                } else {
                    let run_in: PipeRunIn = typed(field, value)?;
                    run_in
                        .validate()
                        .map_err(|e| PipeConfigUpdateError::invalid(field, e))?;
                    Some(run_in)
                }
            }
            "cloud_agent" => {
                next.cloud_agent = if value.is_null() {
                    None
                } else {
                    let cloud: CloudAgentConfig = typed(field, value)?;
                    cloud
                        .validate()
                        .map_err(|e| PipeConfigUpdateError::invalid(field, e))?;
                    Some(cloud)
                }
            }
            "artifacts" => {
                next.artifacts = if value.is_null() {
                    Vec::new()
                } else {
                    let artifacts: Vec<ArtifactDeclaration> = typed(field, value)?;
                    let kept = current.get(field).and_then(Value::as_array);
                    for (artifact, sent) in artifacts.iter().zip(entries(value)) {
                        if kept.is_some_and(|kept| kept.contains(sent)) {
                            continue;
                        }
                        artifact
                            .validate()
                            .map_err(|e| PipeConfigUpdateError::invalid(field, e))?;
                    }
                    artifacts
                }
            }
            "schedule_config" => {
                next.schedule_config = if value.is_null() {
                    None
                } else {
                    let schedule: ScheduleConfig = typed(field, value)?;
                    schedule
                        .validate()
                        .map_err(|e| PipeConfigUpdateError::invalid(field, e.message()))?;
                    Some(schedule)
                }
            }
            f if UI_TEXT_FIELDS.contains(&f) => match value {
                Value::Null => {
                    next.config.remove(f);
                }
                Value::String(_) => {
                    next.config.insert(f.to_string(), value.clone());
                }
                _ => {
                    return Err(PipeConfigUpdateError::invalid(
                        field,
                        "must be text or null",
                    ))
                }
            },
            f if UI_FLAG_FIELDS.contains(&f) => match value {
                Value::Null => {
                    next.config.remove(f);
                }
                Value::Bool(_) => {
                    next.config.insert(f.to_string(), value.clone());
                }
                _ => {
                    return Err(PipeConfigUpdateError::invalid(
                        field,
                        "must be true, false or null",
                    ))
                }
            },
            other => return Err(PipeConfigUpdateError::UnknownField(other.to_string())),
        }
    }

    *config = next;
    *body = next_body;
    Ok(applied)
}

/// Check a whole-file save (`raw_content`): every field of
/// [`FILE_CHECKED_FIELDS`] whose value differs between `before` (the file on
/// disk) and `after` (the new file) goes through the same checks as when it
/// is sent on its own, so an unchanged value already on disk never blocks
/// the save. The `schedule` string is not checked: pipe files have always
/// been allowed to carry one this build cannot read (the pipe then just never
/// runs on a timer), and a full-file save should not start failing over it.
pub fn check_file_changes(
    pipe_name: &str,
    before: &PipeConfig,
    after: &PipeConfig,
) -> Result<(), PipeConfigUpdateError> {
    let old = client_view(before);
    let new = client_view(after);
    let changed: HashMap<String, Value> = FILE_CHECKED_FIELDS
        .iter()
        .filter_map(|key| {
            let value = new.get(*key)?;
            (old.get(*key) != Some(value)).then(|| (key.to_string(), value.clone()))
        })
        .collect();
    let mut scratch = before.clone();
    let mut scratch_body = String::new();
    apply_config_updates(pipe_name, &mut scratch, &mut scratch_body, &changed).map(|_| ())
}

/// Check every typed field of a whole config, as for a new file.
pub fn validate_config(config: &PipeConfig) -> Result<(), PipeConfigUpdateError> {
    check_file_changes(&config.name, &PipeConfig::default(), config)
}

fn check_schedule_string(schedule: &str) -> Result<(), PipeConfigUpdateError> {
    if schedule.is_empty() {
        return Err(PipeConfigUpdateError::invalid(
            "schedule",
            "must not be empty (use \"manual\" for no schedule)",
        ));
    }
    if schedule.eq_ignore_ascii_case("manual") || parse_schedule(schedule).is_some() {
        return Ok(());
    }
    Err(PipeConfigUpdateError::invalid(
        "schedule",
        format!(
            "'{schedule}' is not a schedule; use \"manual\", an interval such as \"every 30m\", \
             \"every day at 9am\", a cron expression, or \"at <RFC 3339 time>\""
        ),
    ))
}

/// Build the trigger a client sent. `current_view` is the pipe's trigger as
/// clients read it. Keys and source entries the client sent back unchanged
/// are kept without checks; the rest is checked.
fn merge_trigger(
    current_view: Option<&Value>,
    value: &Value,
) -> Result<TriggerConfig, PipeConfigUpdateError> {
    const FIELD: &str = "trigger";
    let Value::Object(sent) = value else {
        return Err(PipeConfigUpdateError::invalid(
            FIELD,
            "must be an object with events, custom or sources, or null",
        ));
    };
    let before = current_view.and_then(Value::as_object);
    let unchanged = |key: &str, v: &Value| before.and_then(|b| b.get(key)) == Some(v);

    let mut out = TriggerConfig::default();
    for (key, v) in sent {
        match key.as_str() {
            "events" => {
                out.events = typed::<Option<Vec<String>>>(FIELD, v)?.unwrap_or_default();
                if !unchanged(key, v) && out.events.iter().any(|e| e.trim().is_empty()) {
                    return Err(PipeConfigUpdateError::invalid(
                        FIELD,
                        "event names must not be empty",
                    ));
                }
            }
            "custom" => {
                out.custom = typed::<Option<Vec<String>>>(FIELD, v)?.unwrap_or_default();
            }
            "sources" if unchanged(key, v) => {
                // A list that could not be read at load stays as written.
                match typed::<Option<Vec<SourceTrigger>>>(FIELD, v) {
                    Ok(sources) => out.sources = sources.unwrap_or_default(),
                    Err(_) => {
                        out.extra.insert(key.clone(), v.clone());
                    }
                }
            }
            "sources" => {
                let sources: Vec<SourceTrigger> =
                    typed::<Option<Vec<SourceTrigger>>>(FIELD, v)?.unwrap_or_default();
                let kept = before.and_then(|b| b.get(key)).and_then(Value::as_array);
                for (source, entry) in sources.iter().zip(entries(v)) {
                    if kept.is_some_and(|kept| kept.contains(entry)) {
                        continue;
                    }
                    source
                        .validate()
                        .map_err(|e| PipeConfigUpdateError::invalid(FIELD, e))?;
                }
                out.sources = sources;
            }
            other if unchanged(other, v) => {
                out.extra.insert(other.to_string(), v.clone());
            }
            other => {
                return Err(PipeConfigUpdateError::invalid(
                    FIELD,
                    format!("unknown key '{other}': use events, custom or sources"),
                ))
            }
        }
    }
    Ok(out)
}

/// The entries of a JSON list (nothing for any other value).
fn entries(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

fn typed<T: serde::de::DeserializeOwned>(
    field: &str,
    value: &Value,
) -> Result<T, PipeConfigUpdateError> {
    serde_json::from_value(value.clone())
        .map_err(|e| PipeConfigUpdateError::invalid(field, e.to_string()))
}

fn text(field: &str, value: &Value) -> Result<String, PipeConfigUpdateError> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| PipeConfigUpdateError::invalid(field, "must be text"))
}

fn optional_text(field: &str, value: &Value) -> Result<Option<String>, PipeConfigUpdateError> {
    match value {
        Value::Null => Ok(None),
        Value::String(s) if s.trim().is_empty() => Ok(None),
        Value::String(s) => Ok(Some(s.clone())),
        _ => Err(PipeConfigUpdateError::invalid(
            field,
            "must be text or null",
        )),
    }
}

fn strict_bool(field: &str, value: &Value) -> Result<bool, PipeConfigUpdateError> {
    value
        .as_bool()
        .ok_or_else(|| PipeConfigUpdateError::invalid(field, "must be true or false"))
}

/// A yes/no field where null means "off".
fn flag(field: &str, value: &Value) -> Result<bool, PipeConfigUpdateError> {
    if value.is_null() {
        return Ok(false);
    }
    strict_bool(field, value)
}

fn string_list(field: &str, value: &Value) -> Result<Vec<String>, PipeConfigUpdateError> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        PipeConfigUpdateError::invalid(field, "every entry must be non-empty text")
                    })
            })
            .collect(),
        _ => Err(PipeConfigUpdateError::invalid(
            field,
            "must be a list of text",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipes::parse_frontmatter;
    use serde_json::json;

    fn base() -> (PipeConfig, String) {
        parse_frontmatter("---\nschedule: every 1h\nenabled: true\ntitle: Demo\n---\n\nBody")
            .unwrap()
    }

    fn apply(
        updates: Value,
    ) -> Result<(PipeConfig, String, AppliedConfigUpdate), PipeConfigUpdateError> {
        let (mut config, mut body) = base();
        let map: HashMap<String, Value> = serde_json::from_value(updates).unwrap();
        let applied = apply_config_updates("demo", &mut config, &mut body, &map)?;
        Ok((config, body, applied))
    }

    #[test]
    fn envelope_is_refused_with_a_hint() {
        let err = apply(json!({"config": {"schedule": "every 2h"}})).unwrap_err();
        assert_eq!(err, PipeConfigUpdateError::Envelope);
        assert!(err.to_string().contains("top level"));
    }

    #[test]
    fn unknown_field_is_refused() {
        let err = apply(json!({"schedule": "every 2h", "shedule": "x"})).unwrap_err();
        assert_eq!(err, PipeConfigUpdateError::UnknownField("shedule".into()));
    }

    #[test]
    fn nothing_changes_when_one_field_is_bad() {
        let (mut config, mut body) = base();
        let before = (config.clone(), body.clone());
        let map: HashMap<String, Value> = serde_json::from_value(
            json!({"enabled": false, "effort": "extreme", "prompt_body": "new"}),
        )
        .unwrap();
        let err = apply_config_updates("demo", &mut config, &mut body, &map).unwrap_err();
        assert!(err.to_string().contains("low, medium or high"), "{err}");
        assert_eq!(config.enabled, before.0.enabled);
        assert_eq!(body, before.1);
    }

    #[test]
    fn typed_fields_apply() {
        let (config, body, applied) = apply(json!({
            "prompt_body": "New body",
            "enabled": false,
            "effort": "High",
            "history": true,
            "event_read_only": true,
            "timeout": 900,
            "run_in": {"mode": "existing_chat", "chat_id": "chat-123"},
            "artifacts": [{"path": "output/report.md", "title": "Report", "kind": "markdown"}],
            "schedule_config": {"frequency": "days", "interval": 1, "at_hour": 9, "at_minute": 0},
            "cloud_agent": {"provider": "cursor", "repository": "octo/repo"},
            "trigger": {"events": ["meeting_ended"], "sources": [{"app": "slack", "path": "#general"}]},
            "permissions": "reader",
            "preset": ["primary", "backup"],
            "title": "Renamed",
            "icon": null,
            "featured": true
        }))
        .unwrap();
        assert_eq!(body, "New body");
        assert_eq!(applied.enabled, Some(false));
        assert!(!config.enabled);
        assert_eq!(config.effort, Some(PipeEffort::High));
        assert!(config.history && config.event_read_only);
        assert_eq!(config.timeout, Some(900));
        assert_eq!(config.run_in, Some(PipeRunIn::existing_chat("chat-123")));
        assert_eq!(config.artifacts[0].title.as_deref(), Some("Report"));
        assert!(config.schedule_config.is_some());
        assert_eq!(config.cloud_agent.as_ref().unwrap().provider, "cursor");
        assert_eq!(config.trigger.as_ref().unwrap().sources[0].app, "slack");
        assert_eq!(config.preset, vec!["primary", "backup"]);
        assert_eq!(config.config.get("title"), Some(&json!("Renamed")));
        assert_eq!(config.config.get("featured"), Some(&json!(true)));
        assert!(!config.config.contains_key("icon"));
    }

    #[test]
    fn null_clears_optional_fields() {
        let (config, _, _) = apply(json!({
            "effort": null, "run_in": null, "schedule_config": null, "cloud_agent": null,
            "artifacts": null, "trigger": null, "timeout": null, "provider": null, "title": null
        }))
        .unwrap();
        assert!(config.effort.is_none() && config.run_in.is_none());
        assert!(config.schedule_config.is_none() && config.cloud_agent.is_none());
        assert!(config.artifacts.is_empty() && config.trigger.is_none());
        assert!(!config.config.contains_key("title"));
    }

    #[test]
    fn each_field_is_checked() {
        let cases = [
            (json!({"schedule": "whenever"}), "schedule"),
            (json!({"schedule": ""}), "schedule"),
            (json!({"enabled": "yes"}), "enabled"),
            (json!({"timeout": 0}), "timeout"),
            (json!({"timeout": "10"}), "timeout"),
            (json!({"effort": 3}), "effort"),
            (
                json!({"run_in": {"mode": "existing_chat", "chat_id": "../x"}}),
                "run_in",
            ),
            (json!({"run_in": {"chat_id": "abc"}}), "run_in"),
            (
                json!({"cloud_agent": {"provider": "skynet"}}),
                "cloud_agent",
            ),
            (
                json!({"artifacts": [{"path": "../../etc/passwd"}]}),
                "artifacts",
            ),
            (
                json!({"schedule_config": {"frequency": "weeks"}}),
                "schedule_config",
            ),
            (
                json!({"schedule_config": {"frequency": "days", "at_hour": 30}}),
                "schedule_config",
            ),
            (json!({"trigger": {"sources": [{"app": "fax"}]}}), "trigger"),
            (json!({"trigger": {"events": [""]}}), "trigger"),
            (json!({"connections": "slack"}), "connections"),
            (json!({"preset": [1, 2]}), "preset"),
            (json!({"name": "other"}), "name"),
            (json!({"title": 5}), "title"),
            (json!({"featured": "yes"}), "featured"),
        ];
        for (updates, field) in cases {
            match apply(updates.clone()) {
                Err(PipeConfigUpdateError::Invalid { field: f, .. }) => {
                    assert_eq!(f, field, "{updates}")
                }
                other => panic!("{updates}: expected an invalid {field}, got {other:?}"),
            }
        }
    }

    #[test]
    fn echoed_values_pass_unchecked_and_changed_ones_are_checked() {
        let file = "---\nschedule: every 1h\ntrigger:\n  events: [a]\n  sources:\n  - app: myspace\ncolour: red\nconfig:\n  enabled: true\n---\n\nB";
        let (config, body) = parse_frontmatter(file).unwrap();
        let run = |updates: Value| {
            let (mut config, mut body) = (config.clone(), body.clone());
            let map: HashMap<String, Value> = serde_json::from_value(updates).unwrap();
            apply_config_updates("demo", &mut config, &mut body, &map).map(|_| config)
        };
        // The trigger editor's post: one key changed, the rest sent back.
        let updated = run(json!({
            "trigger": {"events": ["a", "b"], "sources": [{"app": "myspace"}]},
            "colour": "red",
            "config": {"enabled": true}
        }))
        .unwrap();
        let trigger = updated.trigger.unwrap();
        assert_eq!(trigger.events, vec!["a", "b"]);
        assert_eq!(trigger.sources[0].app, "myspace");

        let cases = [
            json!({"trigger": {"events": ["a"], "sources": [{"app": "myspace"}, {"app": "fax"}]}}),
            json!({"trigger": {"events": ["a"], "bogus": 1}}),
            json!({"trigger": {"events": [""], "sources": [{"app": "myspace"}]}}),
        ];
        for updates in cases {
            assert!(
                matches!(run(updates.clone()), Err(PipeConfigUpdateError::Invalid { ref field, .. }) if field == "trigger"),
                "{updates}"
            );
        }
        assert_eq!(
            run(json!({"colour": "blue"})).unwrap_err(),
            PipeConfigUpdateError::UnknownField("colour".into())
        );
        assert_eq!(
            run(json!({"config": {"enabled": false}})).unwrap_err(),
            PipeConfigUpdateError::Envelope
        );
    }

    #[test]
    fn schedule_alone_replaces_a_structured_schedule() {
        let (mut config, mut body) = parse_frontmatter(
            "---\nschedule: every 1h\nschedule_config:\n  frequency: days\n---\n\nB",
        )
        .unwrap();
        let map: HashMap<String, Value> =
            serde_json::from_value(json!({"schedule": "every 1h"})).unwrap();
        // Even the unchanged string takes over: it is the client's choice.
        apply_config_updates("demo", &mut config, &mut body, &map).unwrap();
        assert!(config.schedule_config.is_none());
        assert_eq!(config.schedule, "every 1h");
    }

    #[test]
    fn a_schedule_already_on_disk_is_not_rechecked() {
        let (mut config, mut body) =
            parse_frontmatter("---\nschedule: whenever I like\n---\n\nB").unwrap();
        let map: HashMap<String, Value> =
            serde_json::from_value(json!({"schedule": "whenever I like", "timeout": 60})).unwrap();
        apply_config_updates("demo", &mut config, &mut body, &map).unwrap();
        assert_eq!(config.timeout, Some(60));
    }

    #[test]
    fn file_saves_check_only_what_changed() {
        let (before, _) = parse_frontmatter(
            "---\nschedule: every 1h\neffort: max\nrun_in:\n  mode: new_chat\n---\n\nB",
        )
        .unwrap();
        let (same, _) = parse_frontmatter(
            "---\nschedule: every 2h\neffort: max\nrun_in:\n  mode: new_chat\n---\n\nNew",
        )
        .unwrap();
        assert!(check_file_changes("demo", &before, &same).is_ok());
        let (worse, _) = parse_frontmatter(
            "---\nschedule: every 1h\neffort: maximal\nrun_in:\n  mode: new_chat\n---\n\nB",
        )
        .unwrap();
        assert!(matches!(
            check_file_changes("demo", &before, &worse),
            Err(PipeConfigUpdateError::Invalid { ref field, .. }) if field == "effort"
        ));
    }

    #[test]
    fn same_name_is_accepted() {
        assert!(apply(json!({"name": "demo"})).is_ok());
    }

    #[test]
    fn whole_config_validation_catches_bad_typed_fields() {
        let (mut config, _) = base();
        assert!(validate_config(&config).is_ok());
        config.artifacts = vec![ArtifactDeclaration {
            path: "/abs/x.md".into(),
            title: None,
            kind: None,
        }];
        assert!(matches!(
            validate_config(&config),
            Err(PipeConfigUpdateError::Invalid { ref field, .. }) if field == "artifacts"
        ));
    }
}
