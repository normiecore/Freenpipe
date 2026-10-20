// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Low-disk recording guard (`stopRecordingOnLowDisk`, default on).
//!
//! The engine's monitor (`screenpipe_engine::disk_pressure`) watches the
//! volume of the data directory and publishes `disk_space_low` and, once
//! there is room again (with headroom, so it does not flap),
//! `disk_space_recovered`. This module acts on them:
//!
//! - On low space with the guard on, it stops capture (the API, search and
//!   pipes keep running) and posts one notification with a button that
//!   opens the storage settings. With the guard off it only logs.
//! - On recovery it starts capture again, but only if the guard was what
//!   stopped it. A recording the user paused stays paused, and a recording
//!   the user resumed during the incident is left alone.
//! - Whether the current incident was already announced is kept in
//!   `disk-pressure-notification-state.json` in the data dir, so restarting
//!   the app does not repeat the notification. Recovery re-arms it.
//!
//! Besides the events it re-reads the monitor's latest status every check
//! interval. That catches an event published before this task subscribed,
//! a recovery that happened while the app was closed, and the guard being
//! switched off in the middle of an incident.
//!
//! `get_low_disk_guard_config` gives the settings page and the recording
//! status the threshold that applies to the active data dir.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use screenpipe_engine::disk_pressure::{
    self as monitor, DiskSpaceLowEvent, DiskSpaceRecoveredEvent,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tracing::{debug, info, warn};

use crate::recording::RecordingState;
use crate::store::SettingsStore;

/// Notification type of the low-space notice.
pub const LOW_NOTIFICATION_TYPE: &str = "disk_space_low";
/// Notification type of the "recording resumed" notice.
pub const RECOVERED_NOTIFICATION_TYPE: &str = "disk_space_recovered";
/// Route the notification's button opens.
pub const STORAGE_SETTINGS_ROUTE: &str = "settings?section=storage";

// ---------------------------------------------------------------------------
// get_low_disk_guard_config
// ---------------------------------------------------------------------------

/// What the settings page shows next to the low-disk switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LowDiskGuardConfig {
    /// Capture stops when free space on the data-dir volume is at or below this.
    pub threshold_bytes: u64,
    /// How often the engine reads the free space.
    pub check_interval_seconds: u64,
}

/// How long the command waits for a free-space read of its own.
const CONFIG_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Threshold and check interval of the low-disk guard for the active data dir.
#[tauri::command]
#[specta::specta]
pub async fn get_low_disk_guard_config(app: AppHandle) -> Result<LowDiskGuardConfig, String> {
    let data_dir = PathBuf::from(crate::commands::get_active_data_dir(app)?);
    let check_interval_seconds = monitor::DEFAULT_CHECK_INTERVAL.as_secs();

    // The running monitor already knows the answer for its own directory.
    if let Some(status) = monitor::latest_status() {
        if status.total_bytes.is_some() && Path::new(&status.data_dir) == data_dir {
            return Ok(LowDiskGuardConfig {
                threshold_bytes: status.threshold_bytes,
                check_interval_seconds,
            });
        }
    }

    let dir = data_dir.clone();
    let read = tauri::async_runtime::spawn_blocking(move || monitor::thresholds_for_dir(&dir));
    let threshold_bytes = match tokio::time::timeout(CONFIG_READ_TIMEOUT, read).await {
        Ok(Ok(Ok(limits))) => limits.threshold_bytes,
        Ok(Ok(Err(e))) => {
            warn!(
                "low-disk guard: could not read the volume of {}: {e}; showing the minimum threshold",
                data_dir.display()
            );
            monitor::Thresholds::floor().threshold_bytes
        }
        Ok(Err(e)) => {
            warn!("low-disk guard: the free-space read failed: {e}");
            monitor::Thresholds::floor().threshold_bytes
        }
        Err(_) => {
            warn!(
                "low-disk guard: reading the volume of {} took too long; showing the minimum threshold",
                data_dir.display()
            );
            monitor::Thresholds::floor().threshold_bytes
        }
    };
    Ok(LowDiskGuardConfig {
        threshold_bytes,
        check_interval_seconds,
    })
}

// ---------------------------------------------------------------------------
// Persisted notification state
// ---------------------------------------------------------------------------

/// Contents of `disk-pressure-notification-state.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationState {
    /// A low-space incident is in progress.
    #[serde(default)]
    pub incident_active: bool,
    /// The user was told about the current incident.
    #[serde(default)]
    pub notified: bool,
    /// When the incident started (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Free bytes when the incident started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_bytes: Option<u64>,
    /// Threshold in force when the incident started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_bytes: Option<u64>,
}

impl NotificationState {
    /// Read the file. A missing file means no incident; an unreadable one is
    /// logged and treated the same way.
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                warn!(
                    "low-disk guard: ignoring unreadable {}: {e}",
                    path.display()
                );
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                warn!("low-disk guard: could not read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// Write the file atomically (temp file, then rename).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }
}

/// Where the state file lives for a data dir.
pub fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join(screenpipe_core::paths::DISK_PRESSURE_STATE_FILE)
}

// ---------------------------------------------------------------------------
// Decision
// ---------------------------------------------------------------------------

/// What the monitor says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Free space is at or below the threshold (event or status).
    Low,
    /// `disk_space_recovered` arrived.
    Recovered,
    /// The latest status shows no incident.
    Normal,
}

/// What the guard remembers while the app runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GuardMemory {
    /// The guard stopped capture and owes a restart on recovery.
    pub paused_by_guard: bool,
    /// The user started capture again during this incident; the guard
    /// leaves capture alone until the incident ends.
    pub user_override: bool,
    /// "Guard is off" was logged for this incident.
    pub guard_off_logged: bool,
}

/// Everything else the decision depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    /// `stopRecordingOnLowDisk`.
    pub guard_enabled: bool,
    /// The user wants recording on (not paused, not quitting).
    pub recording_intended: bool,
    /// Persisted: an incident is in progress.
    pub incident_active: bool,
    /// Persisted: the incident was announced.
    pub incident_notified: bool,
}

/// What to do, and the state that follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Plan {
    pub stop_capture: bool,
    pub resume_capture: bool,
    pub notify_low: bool,
    pub notify_resumed: bool,
    pub log_guard_off: bool,
    pub log_user_override: bool,
    pub memory: GuardMemory,
    pub incident_active: bool,
    pub incident_notified: bool,
}

/// The guard's rules (see the module docs).
pub fn plan(signal: Signal, facts: Facts, memory: GuardMemory) -> Plan {
    match signal {
        Signal::Low => plan_low(facts, memory),
        Signal::Recovered | Signal::Normal => {
            let resume = memory.paused_by_guard && !facts.recording_intended;
            Plan {
                resume_capture: resume,
                notify_resumed: resume,
                memory: GuardMemory::default(),
                incident_active: false,
                incident_notified: false,
                ..Plan::default()
            }
        }
    }
}

fn plan_low(facts: Facts, memory: GuardMemory) -> Plan {
    let mut p = Plan {
        memory,
        incident_active: true,
        incident_notified: facts.incident_notified,
        ..Plan::default()
    };
    if !facts.guard_enabled {
        // Switched off mid-incident: give back what the guard took.
        p.resume_capture = memory.paused_by_guard && !facts.recording_intended;
        p.memory.paused_by_guard = false;
        p.memory.user_override = false;
        if !memory.guard_off_logged {
            p.log_guard_off = true;
            p.memory.guard_off_logged = true;
        }
        return p;
    }
    p.memory.guard_off_logged = false;
    if memory.user_override {
        // The user chose to record anyway.
    } else if memory.paused_by_guard && facts.recording_intended {
        p.memory.paused_by_guard = false;
        p.memory.user_override = true;
        p.log_user_override = true;
    } else if !memory.paused_by_guard && facts.recording_intended {
        p.stop_capture = true;
        p.memory.paused_by_guard = true;
    }
    if !facts.incident_notified {
        p.notify_low = true;
        p.incident_notified = true;
    }
    p
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// Figures quoted in logs and notifications.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Reading {
    available_bytes: Option<u64>,
    threshold_bytes: Option<u64>,
}

impl From<&DiskSpaceLowEvent> for Reading {
    fn from(e: &DiskSpaceLowEvent) -> Self {
        Self {
            available_bytes: Some(e.available_bytes),
            threshold_bytes: Some(e.threshold_bytes),
        }
    }
}

impl From<&DiskSpaceRecoveredEvent> for Reading {
    fn from(e: &DiskSpaceRecoveredEvent) -> Self {
        Self {
            available_bytes: Some(e.available_bytes),
            threshold_bytes: Some(e.threshold_bytes),
        }
    }
}

fn size_or_unknown(bytes: Option<u64>) -> String {
    bytes
        .map(monitor::human_bytes)
        .unwrap_or_else(|| "an unknown amount".to_string())
}

/// Text of the low-space notification: (title, body).
pub fn low_notification_text(
    available_bytes: Option<u64>,
    threshold_bytes: Option<u64>,
) -> (String, String) {
    (
        "Recording paused: the disk is nearly full".to_string(),
        format!(
            "Only {} is left on the drive that holds your recordings, and the low-disk guard stops \
             recording at {}. Free some space or change where recordings are stored; recording \
             starts again by itself once there is room.",
            size_or_unknown(available_bytes),
            size_or_unknown(threshold_bytes)
        ),
    )
}

/// Text of the notification sent when the guard restarts capture.
pub fn resumed_notification_text(available_bytes: Option<u64>) -> (String, String) {
    (
        "Recording resumed".to_string(),
        format!(
            "There is room on the disk again ({} free), so recording has started again.",
            size_or_unknown(available_bytes)
        ),
    )
}

struct Guard {
    app: AppHandle,
    state_path: PathBuf,
    state: NotificationState,
    memory: GuardMemory,
}

impl Guard {
    fn guard_enabled(&self) -> bool {
        match SettingsStore::get(&self.app) {
            Ok(Some(settings)) => settings.stop_recording_on_low_disk,
            // Missing settings mean the default, which is on.
            _ => true,
        }
    }

    async fn handle(&mut self, signal: Signal, reading: Reading) {
        let facts = Facts {
            // Only a low reading consults the setting; skip the store read
            // on the routine "all fine" checks.
            guard_enabled: signal != Signal::Low || self.guard_enabled(),
            recording_intended: crate::health::recording_intended(),
            incident_active: self.state.incident_active,
            incident_notified: self.state.notified,
        };
        let mut plan = plan(signal, facts, self.memory);
        debug!("low-disk guard: {signal:?} with {facts:?} -> {plan:?}");

        if plan.log_guard_off {
            info!(
                "low-disk guard: {} free (threshold {}), but stopRecordingOnLowDisk is off, so capture keeps running",
                size_or_unknown(reading.available_bytes),
                size_or_unknown(reading.threshold_bytes)
            );
        }
        if plan.log_user_override {
            info!("low-disk guard: recording was started again during the low-disk incident; leaving it running until space recovers");
        }
        if plan.stop_capture {
            warn!(
                "low-disk guard: stopping capture, {} free (threshold {}); the API, search and pipes keep running",
                size_or_unknown(reading.available_bytes),
                size_or_unknown(reading.threshold_bytes)
            );
            self.stop_capture().await;
        }
        if plan.resume_capture {
            if let Err(e) = self.start_capture().await {
                warn!("low-disk guard: could not start capture again: {e}");
                plan.notify_resumed = false;
                // A start that got as far as setting the intent is now the
                // regular start/respawn machinery's job; otherwise retry on
                // the next check.
                plan.memory.paused_by_guard = !crate::health::recording_intended();
            } else {
                info!("low-disk guard: capture started again");
            }
        }
        if plan.notify_low {
            let (title, body) =
                low_notification_text(reading.available_bytes, reading.threshold_bytes);
            let action = serde_json::json!({
                "id": "review_storage",
                "label": "Review storage",
                "action": "review_storage",
                "type": "deeplink",
                "url": crate::brand::app_deep_link(STORAGE_SETTINGS_ROUTE),
                "primary": true,
            });
            crate::notifications::client::send_typed_with_actions(
                title,
                body,
                LOW_NOTIFICATION_TYPE,
                Some(60_000),
                vec![action],
            );
        }
        if plan.notify_resumed {
            let (title, body) = resumed_notification_text(reading.available_bytes);
            crate::notifications::client::send_typed_with_actions(
                title,
                body,
                RECOVERED_NOTIFICATION_TYPE,
                Some(15_000),
                Vec::new(),
            );
        }

        self.memory = plan.memory;
        self.persist(&plan, reading);
    }

    fn persist(&mut self, plan: &Plan, reading: Reading) {
        let next = next_state(&self.state, plan, reading, chrono::Utc::now().to_rfc3339());
        if next == self.state {
            return;
        }
        if !next.incident_active {
            info!("low-disk guard: incident over; the next low-disk notice is armed again");
        }
        if let Err(e) = next.save(&self.state_path) {
            warn!(
                "low-disk guard: could not write {}: {e}",
                self.state_path.display()
            );
        }
        self.state = next;
    }

    async fn stop_capture(&self) {
        let Some(state) = self.app.try_state::<RecordingState>() else {
            debug!("low-disk guard: recording state not ready; nothing to stop");
            crate::health::set_recording_intended(false);
            return;
        };
        if let Err(e) = crate::recording::stop_capture(state, self.app.clone()).await {
            warn!("low-disk guard: stopping capture failed: {e}");
        }
    }

    async fn start_capture(&self) -> Result<(), String> {
        if crate::relaunch::is_shutting_down() {
            return Err("the app is quitting".into());
        }
        let state = self
            .app
            .try_state::<RecordingState>()
            .ok_or_else(|| "recording state not ready".to_string())?;
        crate::recording::start_capture(state, self.app.clone()).await
    }
}

/// The state file contents after `plan`. The start time and figures of an
/// incident are those of its first reading.
fn next_state(
    current: &NotificationState,
    plan: &Plan,
    reading: Reading,
    now: String,
) -> NotificationState {
    if !plan.incident_active {
        return NotificationState::default();
    }
    if current.incident_active {
        return NotificationState {
            notified: plan.incident_notified,
            ..current.clone()
        };
    }
    NotificationState {
        incident_active: true,
        notified: plan.incident_notified,
        since: Some(now),
        available_bytes: reading.available_bytes,
        threshold_bytes: reading.threshold_bytes,
    }
}

/// Start the guard. Call once from setup.
pub fn start(app: AppHandle) {
    let data_dir = match crate::commands::get_active_data_dir(app.clone()) {
        Ok(dir) => PathBuf::from(dir),
        Err(e) => {
            warn!("low-disk guard not started: {e}");
            return;
        }
    };
    tauri::async_runtime::spawn(run(app, data_dir));
}

async fn run(app: AppHandle, data_dir: PathBuf) {
    // Subscribe before the first status read so nothing slips between them.
    let mut lows =
        screenpipe_events::subscribe_to_event::<DiskSpaceLowEvent>(monitor::DISK_SPACE_LOW_EVENT);
    let mut recoveries = screenpipe_events::subscribe_to_event::<DiskSpaceRecoveredEvent>(
        monitor::DISK_SPACE_RECOVERED_EVENT,
    );
    let path = state_path(&data_dir);
    let mut guard = Guard {
        app,
        state: NotificationState::load(&path),
        state_path: path,
        memory: GuardMemory::default(),
    };
    info!(
        "low-disk guard started (state file {}, incident in progress: {})",
        guard.state_path.display(),
        guard.state.incident_active
    );
    let mut ticks = tokio::time::interval(monitor::DEFAULT_CHECK_INTERVAL);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            Some(event) = lows.next() => {
                guard.handle(Signal::Low, Reading::from(&event.data)).await;
            }
            Some(event) = recoveries.next() => {
                guard.handle(Signal::Recovered, Reading::from(&event.data)).await;
            }
            _ = ticks.tick() => {
                // A failed read says nothing new: the monitor keeps its level
                // and only a successful read can start or end an incident.
                if let Some(status) = monitor::latest_status().filter(|s| s.last_error.is_none()) {
                    let signal = if status.low { Signal::Low } else { Signal::Normal };
                    let reading = Reading {
                        available_bytes: status.available_bytes,
                        threshold_bytes: Some(status.threshold_bytes),
                    };
                    guard.handle(signal, reading).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(guard_enabled: bool, recording_intended: bool) -> Facts {
        Facts {
            guard_enabled,
            recording_intended,
            incident_active: false,
            incident_notified: false,
        }
    }

    /// Apply a plan the way the runtime does (assuming actions succeed).
    fn step(signal: Signal, f: &mut Facts, m: &mut GuardMemory) -> Plan {
        let p = plan(signal, *f, *m);
        if p.stop_capture {
            f.recording_intended = false;
        }
        if p.resume_capture {
            f.recording_intended = true;
        }
        *m = p.memory;
        f.incident_active = p.incident_active;
        f.incident_notified = p.incident_notified;
        p
    }

    #[test]
    fn low_space_stops_capture_and_notifies_once_then_recovery_resumes() {
        let mut f = facts(true, true);
        let mut m = GuardMemory::default();

        let p = step(Signal::Low, &mut f, &mut m);
        assert!(p.stop_capture && p.notify_low);
        assert!(m.paused_by_guard);
        assert!(f.incident_active && f.incident_notified);

        // Repeated low readings (status ticks) change nothing.
        for _ in 0..3 {
            let p = step(Signal::Low, &mut f, &mut m);
            assert_eq!(
                (p.stop_capture, p.notify_low, p.resume_capture),
                (false, false, false)
            );
        }

        let p = step(Signal::Recovered, &mut f, &mut m);
        assert!(p.resume_capture && p.notify_resumed);
        assert!(f.recording_intended);
        assert_eq!(m, GuardMemory::default());
        assert!(!f.incident_active && !f.incident_notified);

        // The next incident is announced again.
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(p.stop_capture && p.notify_low);
    }

    #[test]
    fn guard_off_only_logs() {
        let mut f = facts(false, true);
        let mut m = GuardMemory::default();
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(p.log_guard_off);
        assert!(!p.stop_capture && !p.notify_low);
        // Logged once per incident.
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(!p.log_guard_off);
        let p = step(Signal::Recovered, &mut f, &mut m);
        assert!(!p.resume_capture && !p.notify_resumed);
    }

    #[test]
    fn a_recording_the_user_paused_is_not_resumed() {
        let mut f = facts(true, false);
        let mut m = GuardMemory::default();
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(!p.stop_capture);
        // The user still hears about the full disk.
        assert!(p.notify_low);
        let p = step(Signal::Recovered, &mut f, &mut m);
        assert!(!p.resume_capture);
        assert!(!f.recording_intended);
    }

    #[test]
    fn a_manual_resume_during_the_incident_is_respected() {
        let mut f = facts(true, true);
        let mut m = GuardMemory::default();
        step(Signal::Low, &mut f, &mut m);
        // The user starts recording again from the tray.
        f.recording_intended = true;
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(p.log_user_override && !p.stop_capture);
        // Even a later pause and resume is the user's business.
        f.recording_intended = false;
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(!p.stop_capture && !p.resume_capture);
        f.recording_intended = true;
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(!p.stop_capture);
        // Recovery does not start anything (and does not need to).
        f.recording_intended = false;
        let p = step(Signal::Recovered, &mut f, &mut m);
        assert!(!p.resume_capture);
    }

    #[test]
    fn switching_the_guard_off_mid_incident_resumes_capture() {
        let mut f = facts(true, true);
        let mut m = GuardMemory::default();
        step(Signal::Low, &mut f, &mut m);
        assert!(!f.recording_intended);
        f.guard_enabled = false;
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(p.resume_capture && p.log_guard_off);
        assert!(f.recording_intended);
        assert!(!m.paused_by_guard);
        // Switching it back on stops capture again, without a second notice.
        f.guard_enabled = true;
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(p.stop_capture && !p.notify_low);
    }

    #[test]
    fn a_restart_mid_incident_stops_capture_without_a_second_notice() {
        // Persisted: the incident was announced by the previous run.
        let mut f = Facts {
            guard_enabled: true,
            recording_intended: true,
            incident_active: true,
            incident_notified: true,
        };
        let mut m = GuardMemory::default();
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(p.stop_capture && !p.notify_low);
    }

    #[test]
    fn a_normal_status_clears_a_stale_incident_quietly() {
        let mut f = Facts {
            guard_enabled: true,
            recording_intended: true,
            incident_active: true,
            incident_notified: true,
        };
        let mut m = GuardMemory::default();
        let p = step(Signal::Normal, &mut f, &mut m);
        assert!(!p.resume_capture && !p.notify_resumed);
        assert!(!f.incident_active && !f.incident_notified);
    }

    #[test]
    fn the_state_file_keeps_the_first_reading_of_an_incident() {
        let first = Reading {
            available_bytes: Some(10),
            threshold_bytes: Some(20),
        };
        let later = Reading {
            available_bytes: Some(5),
            threshold_bytes: Some(20),
        };
        let mut f = facts(true, true);
        let mut m = GuardMemory::default();
        let p = step(Signal::Low, &mut f, &mut m);
        let s1 = next_state(&NotificationState::default(), &p, first, "t1".into());
        assert_eq!(
            s1,
            NotificationState {
                incident_active: true,
                notified: true,
                since: Some("t1".into()),
                available_bytes: Some(10),
                threshold_bytes: Some(20),
            }
        );
        let p = step(Signal::Low, &mut f, &mut m);
        assert_eq!(next_state(&s1, &p, later, "t2".into()), s1);
        let p = step(Signal::Recovered, &mut f, &mut m);
        assert_eq!(
            next_state(&s1, &p, later, "t3".into()),
            NotificationState::default()
        );
    }

    #[test]
    fn state_file_round_trips_and_survives_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = state_path(dir.path());
        assert_eq!(
            path.file_name().unwrap(),
            "disk-pressure-notification-state.json"
        );
        assert_eq!(NotificationState::load(&path), NotificationState::default());
        let state = NotificationState {
            incident_active: true,
            notified: true,
            since: Some("2026-10-20T10:00:00+00:00".into()),
            available_bytes: Some(1),
            threshold_bytes: Some(2),
        };
        state.save(&path).unwrap();
        assert_eq!(NotificationState::load(&path), state);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(NotificationState::load(&path), NotificationState::default());
        // Unknown and missing fields are tolerated.
        std::fs::write(&path, br#"{"notified":true,"extra":1}"#).unwrap();
        assert!(NotificationState::load(&path).notified);
    }

    #[test]
    fn notification_text_quotes_the_figures() {
        let gib = 1024 * 1024 * 1024;
        let (title, body) = low_notification_text(Some(gib), Some(2 * gib));
        assert!(!title.is_empty());
        assert!(
            body.contains("1.0 GiB") && body.contains("2.0 GiB"),
            "{body}"
        );
        let (_, body) = low_notification_text(None, None);
        assert!(body.contains("unknown"), "{body}");
        let (_, body) = resumed_notification_text(Some(3 * gib));
        assert!(body.contains("3.0 GiB"), "{body}");
    }

    #[test]
    fn the_button_opens_storage_settings() {
        let link = crate::brand::app_deep_link(STORAGE_SETTINGS_ROUTE);
        assert!(link.ends_with("://settings?section=storage"), "{link}");
        assert!(crate::brand::is_app_deep_link(&link));
    }

    #[test]
    fn config_serialises_with_the_ui_field_names() {
        let v = serde_json::to_value(LowDiskGuardConfig {
            threshold_bytes: 5,
            check_interval_seconds: 30,
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({"thresholdBytes": 5, "checkIntervalSeconds": 30})
        );
    }
}
