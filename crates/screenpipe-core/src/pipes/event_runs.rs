// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Event-triggered runs: instance keys, claims and run slots.
//!
//! The same event instance can reach the scheduler more than once. A meeting
//! that is rejoined within the rejoin window reopens the same row and ends a
//! second time, so `meeting_ended` fires twice for one meeting id. Before the
//! scheduler starts an event-triggered run it claims `(pipe, event, key)` in
//! `pipe_event_runs` with `INSERT OR IGNORE`; the primary key makes the claim
//! atomic, so only one claimer runs, however the two events race.
//!
//! A claim is given back when the run is cancelled (before or while it runs)
//! or when it could not start (the agent never launched, or the execution
//! was finished by someone else before it began). Claims are otherwise kept
//! and pruned once they are [`CLAIM_RETENTION_DAYS`] old, far beyond the
//! two-minute rejoin window, so the second `meeting_ended` of a rejoined
//! meeting is always recognised.
//!
//! Events without an instance key (for example `pipe_completed:*`) are not
//! claimed and run as before.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::Value;

/// Claims older than this many days are pruned.
pub const CLAIM_RETENTION_DAYS: i64 = 30;

/// Longest event payload shown to the agent, in characters.
pub const TRIGGER_CONTEXT_MAX_CHARS: usize = 2_000;

/// How long a refresh request that cannot start yet (agent still installing,
/// pipe busy) is kept and tried again.
pub const DEFERRED_REQUEST_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// One `pipe_event_runs` row: the pipe, the event and the event instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventClaim {
    pub pipe: String,
    pub event: String,
    pub key: String,
}

/// A run the scheduler means to start because of an event.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRun {
    pub pipe: String,
    /// Name recorded as `trigger_event`.
    pub event: String,
    /// Instance key recorded as `trigger_key` and claimed; `None` when the
    /// event has no instance identity.
    pub key: Option<String>,
    /// Payload shown to the agent.
    pub data: Value,
    /// Asked for by pipe name (a refresh request) instead of matched by the
    /// pipe's trigger list.
    pub directed: bool,
}

impl EventRun {
    pub fn claim(&self) -> Option<EventClaim> {
        self.key.as_ref().map(|key| EventClaim {
            pipe: self.pipe.clone(),
            event: self.event.clone(),
            key: key.clone(),
        })
    }
}

/// Instance key of an event, from its payload:
///
/// - `meeting_ended`: the meeting id;
/// - connection triggers: `source_delivery_id`;
/// - other events (workflow events): the payload's `id` or `event_id`.
///
/// `None` means the event is not claimed.
pub fn event_instance_key(event_name: &str, data: &Value) -> Option<String> {
    if event_name.starts_with("pipe_completed:") {
        return None;
    }
    if event_name == "meeting_ended" {
        return scalar_key(data.get("meeting_id").or_else(|| data.get("id")));
    }
    if let Some(key) = scalar_key(data.get("source_delivery_id")) {
        return Some(key);
    }
    if event_name.starts_with("meeting_") {
        return None;
    }
    scalar_key(data.get("id").or_else(|| data.get("event_id")))
}

fn scalar_key(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => {
            let s = s.trim();
            (!s.is_empty()).then(|| s.to_string())
        }
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Text added to an event-triggered run's prompt so the agent knows which
/// event instance it is handling.
pub fn render_trigger_context(run: &EventRun) -> String {
    let mut payload = serde_json::to_string(&run.data).unwrap_or_default();
    if payload.chars().count() > TRIGGER_CONTEXT_MAX_CHARS {
        payload = payload.chars().take(TRIGGER_CONTEXT_MAX_CHARS).collect();
        payload.push_str("…");
    }
    let mut out = format!("\nThis run was started by the `{}` event", run.event);
    if let Some(key) = &run.key {
        out.push_str(&format!(" (instance `{key}`)"));
    }
    out.push_str(". Handle that event instance only.\n");
    if !payload.is_empty() && payload != "null" {
        out.push_str("Event payload: ");
        out.push_str(&payload);
        out.push('\n');
    }
    out
}

/// What the scheduler knows about a queued or running run, so that a stop
/// can cancel it before it starts and give its claim back.
#[derive(Debug, Clone, Default)]
pub struct RunSlot {
    pub execution_id: Option<i64>,
    pub claim: Option<EventClaim>,
    /// The agent has been launched.
    pub started: bool,
    pub cancelled: Arc<AtomicBool>,
}

impl RunSlot {
    pub fn new(execution_id: Option<i64>, claim: Option<EventClaim>) -> Self {
        Self {
            execution_id,
            claim,
            started: false,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn meeting_ended_is_keyed_by_meeting_id() {
        assert_eq!(
            event_instance_key("meeting_ended", &json!({"meeting_id": 42})),
            Some("42".to_string())
        );
        assert_eq!(
            event_instance_key("meeting_ended", &json!({"meeting_id": " 7 "})),
            Some("7".to_string())
        );
        assert_eq!(event_instance_key("meeting_ended", &json!({})), None);
        assert_eq!(
            event_instance_key("meeting_ended", &json!({"meeting_id": null})),
            None
        );
    }

    #[test]
    fn other_events_use_delivery_or_payload_ids() {
        assert_eq!(
            event_instance_key(
                "connection_trigger",
                &json!({"source_delivery_id": "d-1", "id": 9})
            ),
            Some("d-1".to_string())
        );
        assert_eq!(
            event_instance_key("crm_update_from_social", &json!({"id": "w-3"})),
            Some("w-3".to_string())
        );
        assert_eq!(
            event_instance_key(
                "debugging_session",
                &json!({"event_type": "debugging_session"})
            ),
            None
        );
        // Pipe chains and other meeting events are not claimed.
        assert_eq!(
            event_instance_key("pipe_completed:digest", &json!({"id": 1})),
            None
        );
        assert_eq!(
            event_instance_key("meeting_started", &json!({"meeting_id": 1, "id": 1})),
            None
        );
    }

    #[test]
    fn trigger_context_names_the_event_and_trims_the_payload() {
        let run = EventRun {
            pipe: "meeting-summary".into(),
            event: "meeting_ended".into(),
            key: Some("12".into()),
            data: json!({"meeting_id": 12}),
            directed: false,
        };
        let text = render_trigger_context(&run);
        assert!(text.contains("`meeting_ended` event (instance `12`)"));
        assert!(text.contains(r#"Event payload: {"meeting_id":12}"#));
        assert_eq!(
            run.claim(),
            Some(EventClaim {
                pipe: "meeting-summary".into(),
                event: "meeting_ended".into(),
                key: "12".into()
            })
        );

        let big = EventRun {
            data: json!({"text": "x".repeat(10_000)}),
            key: None,
            ..run
        };
        let text = render_trigger_context(&big);
        assert!(text.chars().count() < TRIGGER_CONTEXT_MAX_CHARS + 200);
        assert!(!text.contains("(instance"));
        assert_eq!(big.claim(), None);
    }

    #[test]
    fn slots_cancel() {
        let slot = RunSlot::new(Some(3), None);
        let shared = slot.clone();
        assert!(!shared.is_cancelled());
        slot.cancel();
        assert!(shared.is_cancelled());
    }
}
