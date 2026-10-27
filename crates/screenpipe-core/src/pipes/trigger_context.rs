// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! `.trigger-context.json`: the event behind an event-triggered run, written
//! into the pipe's own folder before the agent starts, so the pipe's prompt
//! (and any script it runs) can read which event instance it is handling.
//!
//! Shape:
//!
//! ```json
//! {
//!   "event": "meeting_ended",
//!   "key": "42",
//!   "pipe": "meeting-summary",
//!   "trigger_type": "event",
//!   "triggered_at": "2026-10-27T09:30:00.000Z",
//!   "data": { "meeting_id": 42 }
//! }
//! ```
//!
//! The payload goes through [`redact`](super::redact) first, so connection
//! tokens, API keys and other credentials never reach the file. A payload
//! larger than [`MAX_PAYLOAD_BYTES`] is replaced by a cut-down text preview
//! (`data_truncated: true`).
//!
//! Runs that were not started by an event remove the file, so a scheduled
//! or manual run never mistakes an old event for its own.

use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};

use super::event_runs::EventRun;
use super::redact::redact_value;
use crate::paths::PIPE_TRIGGER_CONTEXT_FILE;

/// Largest payload written as JSON; bigger ones become a preview.
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
/// Length of the preview that replaces an oversized payload.
const PREVIEW_CHARS: usize = 8 * 1024;

/// Path of the trigger context file in `pipe_dir`.
pub fn path(pipe_dir: &Path) -> PathBuf {
    pipe_dir.join(PIPE_TRIGGER_CONTEXT_FILE)
}

/// The JSON document written for `run`.
pub fn render(run: &EventRun, triggered_at: DateTime<Utc>) -> Value {
    let data = redact_value(&run.data);
    let mut doc = json!({
        "event": run.event,
        "key": run.key,
        "pipe": run.pipe,
        "trigger_type": run.trigger_type(),
        "triggered_at": triggered_at.to_rfc3339_opts(SecondsFormat::Millis, true),
    });
    let text = serde_json::to_string(&data).unwrap_or_default();
    if text.len() > MAX_PAYLOAD_BYTES {
        let preview: String = text.chars().take(PREVIEW_CHARS).collect();
        doc["data"] = Value::Null;
        doc["data_truncated"] = Value::Bool(true);
        doc["data_preview"] = Value::String(preview);
    } else {
        doc["data"] = data;
    }
    doc
}

/// Write the trigger context of `run` into `pipe_dir`, replacing any older
/// one. The file is written next to its final name and renamed into place,
/// so a reader never sees half a document. Readable by the owner only on
/// Unix.
pub fn write(pipe_dir: &Path, run: &EventRun, triggered_at: DateTime<Utc>) -> std::io::Result<PathBuf> {
    let doc = render(run, triggered_at);
    let body = serde_json::to_vec_pretty(&doc)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::create_dir_all(pipe_dir)?;
    let target = path(pipe_dir);
    let tmp = pipe_dir.join(format!(
        "{}.{}.tmp",
        PIPE_TRIGGER_CONTEXT_FILE,
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(&body)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map(|()| target)
}

/// Remove the trigger context of `pipe_dir`, if any (best effort).
pub fn clear(pipe_dir: &Path) {
    let target = path(pipe_dir);
    match std::fs::remove_file(&target) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::debug!("could not remove {}: {}", target.display(), e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(data: Value) -> EventRun {
        EventRun {
            pipe: "digest".into(),
            event: "connection_trigger".into(),
            key: Some("slack:C1:1".into()),
            data,
            directed: true,
        }
    }

    #[test]
    fn written_context_names_the_event_and_hides_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let at = DateTime::parse_from_rfc3339("2026-10-27T09:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let r = run(json!({
            "pipe_name": "digest",
            "source_delivery_id": "slack:C1:1",
            "item": {"text": "hello", "url": "https://h.io/x?token=abc123"},
            "connection": {"access_token": "xoxb-1234567890-abcdefghij", "workspace": "acme"}
        }));
        let written = write(dir.path(), &r, at).unwrap();
        assert_eq!(written, dir.path().join(".trigger-context.json"));
        let text = std::fs::read_to_string(&written).unwrap();
        assert!(!text.contains("xoxb-1234567890"), "{text}");
        assert!(!text.contains("abc123"), "{text}");
        let doc: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["event"], "connection_trigger");
        assert_eq!(doc["key"], "slack:C1:1");
        assert_eq!(doc["pipe"], "digest");
        assert_eq!(doc["trigger_type"], "connection");
        assert_eq!(doc["triggered_at"], "2026-10-27T09:30:00.000Z");
        assert_eq!(doc["data"]["item"]["text"], "hello");
        assert_eq!(doc["data"]["connection"]["workspace"], "acme");
        // No temporary files are left behind.
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&written).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "context file is private");
        }

        clear(dir.path());
        assert!(!written.exists());
        clear(dir.path()); // nothing to remove is fine
    }

    #[test]
    fn oversized_payloads_become_a_preview() {
        let r = run(json!({"text": "x".repeat(MAX_PAYLOAD_BYTES + 10)}));
        let doc = render(&r, Utc::now());
        assert!(doc["data"].is_null());
        assert_eq!(doc["data_truncated"], true);
        assert!(doc["data_preview"].as_str().unwrap().len() <= PREVIEW_CHARS);
    }
}
