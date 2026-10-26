// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Shared rules for the notifications raised from audio engine events.
//!
//! Audio devices can flap (a Bluetooth headset dropping in and out, a USB
//! hub resetting), and each flap produces engine events. The user should
//! hear about a problem once, not once per flap, so every notice passes a
//! [`NoticeThrottle`]: one notice per key (event kind plus device) within
//! [`PER_KEY_INTERVAL`], and at most [`BURST_LIMIT`] notices of any key
//! within [`BURST_WINDOW`]. Devices the user switched off never produce a
//! notice ([`UserDisabledDevices`]), and the notification preferences
//! (`notificationPrefs`) are honoured.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use serde_json::Value;

/// Minimum time between two notices with the same key.
pub(super) const PER_KEY_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// Window and size of the cap on notices of any key.
pub(super) const BURST_WINDOW: Duration = Duration::from_secs(5 * 60);
pub(super) const BURST_LIMIT: usize = 3;

/// Decides whether a notice may be shown now.
#[derive(Debug, Default)]
pub(super) struct NoticeThrottle {
    last_by_key: HashMap<String, Instant>,
    recent: VecDeque<Instant>,
}

impl NoticeThrottle {
    /// Returns true (and records the notice) when `key` may notify at `now`.
    pub(super) fn allow(&mut self, key: &str, now: Instant) -> bool {
        while self
            .recent
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= BURST_WINDOW)
        {
            self.recent.pop_front();
        }
        if let Some(last) = self.last_by_key.get(key) {
            if now.saturating_duration_since(*last) < PER_KEY_INTERVAL {
                return false;
            }
        }
        if self.recent.len() >= BURST_LIMIT {
            return false;
        }
        self.last_by_key.insert(key.to_string(), now);
        self.recent.push_back(now);
        // Keep the map from growing without bound on long runs.
        self.last_by_key
            .retain(|_, t| now.saturating_duration_since(*t) < PER_KEY_INTERVAL);
        true
    }
}

static THROTTLE: Lazy<Mutex<NoticeThrottle>> = Lazy::new(|| Mutex::new(NoticeThrottle::default()));

/// [`NoticeThrottle::allow`] on the process-wide throttle.
pub(super) fn throttle_allows(key: &str) -> bool {
    THROTTLE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .allow(key, Instant::now())
}

/// Device names the user paused, learnt from `audio_device_status_changed`.
#[derive(Debug, Default)]
pub(super) struct UserDisabledDevices {
    names: HashSet<String>,
}

impl UserDisabledDevices {
    /// Update from one `audio_device_status_changed` payload.
    pub(super) fn observe_status(&mut self, data: &Value) {
        let Some(name) = data.get("device_name").and_then(Value::as_str) else {
            return;
        };
        match data.get("is_user_disabled").and_then(Value::as_bool) {
            Some(true) => {
                self.names.insert(name.to_string());
            }
            Some(false) => {
                self.names.remove(name);
            }
            None => {}
        }
    }

    pub(super) fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }
}

static USER_DISABLED: Lazy<Mutex<UserDisabledDevices>> =
    Lazy::new(|| Mutex::new(UserDisabledDevices::default()));

pub(super) fn observe_device_status(data: &Value) {
    USER_DISABLED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .observe_status(data);
}

/// True when the user switched `device` off.
pub(super) fn is_user_disabled(device: &str) -> bool {
    USER_DISABLED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(device)
}

/// Whether `notificationPrefs` lets a capture notice through at `now_ms`
/// (epoch milliseconds): the master switch is on, notifications are not
/// snoozed, and `captureStalls` is on. Missing keys mean "allowed".
pub(super) fn prefs_allow_capture_notice(prefs: Option<&Value>, now_ms: i64) -> bool {
    let Some(prefs) = prefs else {
        return true;
    };
    let flag = |key: &str| prefs.get(key).and_then(Value::as_bool).unwrap_or(true);
    if !flag("notificationsEnabled") || !flag("captureStalls") {
        return false;
    }
    let snoozed_until = prefs.get("snoozeUntil").and_then(Value::as_i64).unwrap_or(0);
    snoozed_until <= now_ms
}

/// Read the stored preferences and apply [`prefs_allow_capture_notice`].
pub(super) fn capture_notices_enabled(app: &tauri::AppHandle) -> bool {
    let settings = match crate::store::SettingsStore::get(app) {
        Ok(Some(s)) => s,
        _ => return true,
    };
    prefs_allow_capture_notice(
        settings.extra.get("notificationPrefs"),
        chrono::Utc::now().timestamp_millis(),
    )
}

/// Show an operating-system notification. Runs on its own thread: the
/// Linux backend blocks on an executor and panics inside a tokio runtime.
pub(super) fn show_os_notification(app: &tauri::AppHandle, title: String, body: String) {
    use tauri_plugin_notification::NotificationExt;
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = app.notification().builder().title(&title).body(&body).show() {
            tracing::warn!("audio notice: failed to show OS notification: {}", e);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn same_key_is_held_back_for_the_interval() {
        let mut t = NoticeThrottle::default();
        let t0 = Instant::now();
        assert!(t.allow("engaged:AirPods", t0));
        assert!(!t.allow("engaged:AirPods", t0 + Duration::from_secs(60)));
        assert!(!t.allow(
            "engaged:AirPods",
            t0 + PER_KEY_INTERVAL - Duration::from_secs(1)
        ));
        assert!(t.allow("engaged:AirPods", t0 + PER_KEY_INTERVAL));
    }

    #[test]
    fn burst_cap_limits_a_flapping_set_of_devices() {
        let mut t = NoticeThrottle::default();
        let t0 = Instant::now();
        assert!(t.allow("a", t0));
        assert!(t.allow("b", t0 + Duration::from_secs(1)));
        assert!(t.allow("c", t0 + Duration::from_secs(2)));
        assert!(!t.allow("d", t0 + Duration::from_secs(3)));
        // Once the window slides past the first notices, new keys pass.
        assert!(t.allow("d", t0 + BURST_WINDOW + Duration::from_secs(1)));
    }

    #[test]
    fn user_disabled_devices_follow_status_events() {
        let mut d = UserDisabledDevices::default();
        d.observe_status(&json!({"device_name": "Mic (input)", "is_running": false, "is_user_disabled": true}));
        assert!(d.contains("Mic (input)"));
        // A status without the flag changes nothing.
        d.observe_status(&json!({"device_name": "Mic (input)"}));
        assert!(d.contains("Mic (input)"));
        d.observe_status(&json!({"device_name": "Mic (input)", "is_running": true, "is_user_disabled": false}));
        assert!(!d.contains("Mic (input)"));
        d.observe_status(&json!({"is_user_disabled": true}));
        assert!(!d.contains(""));
    }

    #[test]
    fn prefs_gate_capture_notices() {
        let now = 1_700_000_000_000i64;
        assert!(prefs_allow_capture_notice(None, now));
        assert!(prefs_allow_capture_notice(Some(&json!({})), now));
        assert!(!prefs_allow_capture_notice(
            Some(&json!({"captureStalls": false})),
            now
        ));
        assert!(!prefs_allow_capture_notice(
            Some(&json!({"notificationsEnabled": false, "captureStalls": true})),
            now
        ));
        assert!(!prefs_allow_capture_notice(
            Some(&json!({"snoozeUntil": now + 1})),
            now
        ));
        assert!(prefs_allow_capture_notice(
            Some(&json!({"snoozeUntil": now - 1, "captureStalls": true})),
            now
        ));
    }
}
