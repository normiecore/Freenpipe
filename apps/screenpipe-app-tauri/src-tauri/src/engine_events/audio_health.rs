// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Audio capture health events: forwards the engine's
//! `audio_capture_health_*` events to kebab-case Tauri events and tells the
//! user when capture needs attention.
//!
//! * `speaker_silent` — sound is playing but the recorded system audio is
//!   silent. Notified with a restart button.
//! * `mic_capture_failed` — a microphone that should record (a meeting's
//!   input) could not be opened. Notified with a restart button.
//! * `recovered` — the problem cleared. Notified only when the user was told
//!   about a failure of the same device, so the pair reads as one story.
//!
//! The notices use the in-app notification panel because it can carry the
//! `restart_recording` action; they pass the same preference check and
//! throttle as the device notices ([`super::notice`]).

use std::collections::HashSet;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tracing::{debug, info, warn};

use super::notice;
use crate::notifications::client;

/// Engine event names this module handles.
pub(super) const EVENT_NAMES: [&str; 3] = [
    "audio_capture_health_speaker_silent",
    "audio_capture_health_recovered",
    "audio_capture_health_mic_capture_failed",
];

/// Map an engine event name to the kebab-case Tauri event.
pub(super) fn tauri_event_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "audio_capture_health_speaker_silent" => "audio-capture-health-speaker-silent",
        "audio_capture_health_recovered" => "audio-capture-health-recovered",
        "audio_capture_health_mic_capture_failed" => "audio-capture-health-mic-capture-failed",
        _ => return None,
    })
}

/// A notice for the in-app panel.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct HealthNotice {
    pub key: String,
    pub title: String,
    pub body: String,
    /// Offer the "restart recording" action.
    pub offer_restart: bool,
}

fn display_name(device: &str) -> &str {
    device
        .strip_suffix(" (input)")
        .or_else(|| device.strip_suffix(" (output)"))
        .unwrap_or(device)
        .trim()
}

fn string_list(data: &Value, key: &str) -> Vec<String> {
    data.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|s| display_name(s).to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The notice for one health event. `failure_was_shown` says whether a
/// failure notice for `device` was shown earlier (recoveries only pair with
/// one); `user_disabled` filters devices the user switched off.
pub(super) fn notice_for(
    name: &str,
    data: &Value,
    failure_was_shown: impl Fn(&str) -> bool,
    user_disabled: impl Fn(&str) -> bool,
) -> Option<HealthNotice> {
    let device = data
        .get("device")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty());
    if device.is_some_and(&user_disabled) {
        return None;
    }
    match name {
        "audio_capture_health_mic_capture_failed" => {
            let body = match device {
                Some(d) => format!(
                    "{} could not be opened, so your side of the meeting is not recorded. Freenpipe keeps retrying; restarting recording often helps.",
                    display_name(d)
                ),
                None => "The microphone could not be opened, so your side of the meeting is not recorded. Freenpipe keeps retrying; restarting recording often helps.".to_string(),
            };
            Some(HealthNotice {
                key: format!("mic_capture_failed:{}", device.unwrap_or("")),
                title: "Microphone not recording".to_string(),
                body,
                offer_restart: true,
            })
        }
        "audio_capture_health_speaker_silent" => {
            let playing = string_list(data, "rendering_endpoints");
            let body = if playing.is_empty() {
                "Sound is playing, but the recorded system audio is silent. Restarting recording usually fixes this.".to_string()
            } else {
                format!(
                    "Sound is playing on {}, but the recorded system audio is silent. Restarting recording usually fixes this.",
                    playing.join(", ")
                )
            };
            Some(HealthNotice {
                key: format!("speaker_silent:{}", device.unwrap_or("")),
                title: "System audio not recording".to_string(),
                body,
                offer_restart: true,
            })
        }
        "audio_capture_health_recovered" => {
            let d = device?;
            if !failure_was_shown(d) {
                return None;
            }
            Some(HealthNotice {
                key: format!("recovered:{d}"),
                title: "Audio recording again".to_string(),
                body: format!("{} is recording again.", display_name(d)),
                offer_restart: false,
            })
        }
        _ => None,
    }
}

/// Devices whose failure notice was shown and not yet followed by a recovery.
static FAILURES_SHOWN: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

fn failure_was_shown(device: &str) -> bool {
    FAILURES_SHOWN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(device)
}

/// Handle one frame of an audio health event. Called from [`super::dispatch`].
pub(super) fn handle(app: &AppHandle, name: &str, data: &Value) {
    let Some(tauri_event) = tauri_event_name(name) else {
        debug!("audio_health::handle called with unexpected name: {}", name);
        return;
    };
    info!(event = %data, "{} (from engine)", tauri_event);
    if let Err(e) = app.emit(tauri_event, data.clone()) {
        warn!("failed to emit {}: {}", tauri_event, e);
    }

    let Some(notice) = notice_for(name, data, failure_was_shown, notice::is_user_disabled) else {
        return;
    };
    let device = data
        .get("device")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !notice.offer_restart {
        // A recovery closes the story whether or not it is shown.
        FAILURES_SHOWN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&device);
    }
    if !notice::capture_notices_enabled(app) {
        debug!("audio health notice skipped by notification preferences");
        return;
    }
    if !notice::throttle_allows(&notice.key) {
        debug!("audio health notice '{}' throttled", notice.key);
        return;
    }
    if notice.offer_restart && !device.is_empty() {
        FAILURES_SHOWN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(device);
    }
    let actions = if notice.offer_restart {
        vec![json!({ "label": "restart recording", "action": "restart_recording", "primary": true })]
    } else {
        Vec::new()
    };
    client::send_typed_with_actions(
        notice.title,
        notice.body,
        "system",
        Some(if notice.offer_restart { 30_000 } else { 8_000 }),
        actions,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never(_: &str) -> bool {
        false
    }

    #[test]
    fn every_handled_event_has_a_kebab_name() {
        for name in EVENT_NAMES {
            assert_eq!(tauri_event_name(name).expect(name), name.replace('_', "-"));
        }
        assert_eq!(tauri_event_name("audio_device_status_changed"), None);
    }

    #[test]
    fn mic_failure_offers_a_restart() {
        let n = notice_for(
            "audio_capture_health_mic_capture_failed",
            &json!({"state": "mic_capture_failed", "device": "Headset (input)", "meeting_id": 3}),
            never,
            never,
        )
        .unwrap();
        assert!(n.offer_restart);
        assert_eq!(n.key, "mic_capture_failed:Headset (input)");
        assert!(n.body.starts_with("Headset could not be opened"));
    }

    #[test]
    fn speaker_silent_names_the_playing_endpoints() {
        let n = notice_for(
            "audio_capture_health_speaker_silent",
            &json!({"state": "speaker_silent", "rendering_endpoints": ["Speakers (output)"], "captured_outputs": []}),
            never,
            never,
        )
        .unwrap();
        assert!(n.body.contains("on Speakers,"), "{}", n.body);
        assert!(n.offer_restart);
    }

    #[test]
    fn recovery_only_follows_a_shown_failure() {
        let data = json!({"state": "recovered", "device": "Headset (input)"});
        assert_eq!(
            notice_for("audio_capture_health_recovered", &data, never, never),
            None
        );
        let shown = |d: &str| d == "Headset (input)";
        let n = notice_for("audio_capture_health_recovered", &data, shown, never).unwrap();
        assert!(!n.offer_restart);
        assert_eq!(n.body, "Headset is recording again.");
        // Without a device there is nothing to pair with.
        assert_eq!(
            notice_for(
                "audio_capture_health_recovered",
                &json!({"state": "recovered"}),
                shown,
                never
            ),
            None
        );
    }

    #[test]
    fn user_disabled_devices_never_notify() {
        let disabled = |d: &str| d == "Headset (input)";
        assert_eq!(
            notice_for(
                "audio_capture_health_mic_capture_failed",
                &json!({"device": "Headset (input)"}),
                never,
                disabled
            ),
            None
        );
    }
}
