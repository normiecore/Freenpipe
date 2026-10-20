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
//! - Capture that is started again during the incident (from the tray, a
//!   settings save, an engine restart, ...) is stopped again at the next
//!   check, with a notice that says so. Recording through a low-disk
//!   incident is what switching the guard off is for.
//! - On recovery it starts capture again, but only if the guard was what
//!   stopped it. A recording the user paused stays paused.
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

    let status = monitor::latest_status();
    let threshold_bytes = match threshold_from_status(status.as_ref(), &data_dir) {
        Some(threshold_bytes) => threshold_bytes,
        None => read_threshold_or_floor(&data_dir).await,
    };
    Ok(LowDiskGuardConfig {
        threshold_bytes,
        check_interval_seconds,
    })
}

/// The threshold for `data_dir` from a read of our own; the minimum when the
/// volume cannot be read in time.
async fn read_threshold_or_floor(data_dir: &Path) -> u64 {
    let read = |dir: PathBuf| monitor::thresholds_for_dir(&dir);
    match read_thresholds(&PENDING_CONFIG_READ, data_dir, CONFIG_READ_TIMEOUT, read).await {
        Ok(limits) => limits.threshold_bytes,
        Err(e) => {
            warn!(
                "low-disk guard: could not read the volume of {}: {e}; showing the minimum threshold",
                data_dir.display()
            );
            monitor::Thresholds::floor().threshold_bytes
        }
    }
}

/// The threshold the running monitor applies to `data_dir`, when its latest
/// status is about that directory. A status from a failed read counts too:
/// the monitor keeps applying its last known limits (the minimum before its
/// first good read), and a read of our own would only wait on the same
/// volume.
fn threshold_from_status(
    status: Option<&monitor::DiskPressureStatus>,
    data_dir: &Path,
) -> Option<u64> {
    status
        .filter(|s| Path::new(&s.data_dir) == data_dir)
        .map(|s| s.threshold_bytes)
}

/// A free-space read of the command's own that outlived its caller's
/// timeout. The next call for the same directory waits on it instead of
/// starting another, so a hung volume holds at most one blocked thread.
struct PendingRead {
    dir: PathBuf,
    handle: tokio::task::JoinHandle<std::io::Result<monitor::Thresholds>>,
}

static PENDING_CONFIG_READ: tokio::sync::Mutex<Option<PendingRead>> =
    tokio::sync::Mutex::const_new(None);

/// Read the thresholds of `dir` on a blocking thread, within `timeout`
/// (waiting for another caller's read counts against it). A read that is
/// still running for `dir` is awaited rather than started again; a finished
/// one, or one for another directory, is dropped and a fresh read started.
async fn read_thresholds<F>(
    pending: &tokio::sync::Mutex<Option<PendingRead>>,
    dir: &Path,
    timeout: Duration,
    read: F,
) -> Result<monitor::Thresholds, String>
where
    F: FnOnce(PathBuf) -> std::io::Result<monitor::Thresholds> + Send + 'static,
{
    let deadline = tokio::time::Instant::now() + timeout;
    let mut slot = tokio::time::timeout_at(deadline, pending.lock())
        .await
        .map_err(|_| "another free-space read of the data dir is still running".to_string())?;
    let mut handle = match slot.take() {
        Some(earlier) if earlier.dir == dir && !earlier.handle.is_finished() => earlier.handle,
        _ => {
            let owned = dir.to_path_buf();
            tokio::task::spawn_blocking(move || read(owned))
        }
    };
    match tokio::time::timeout_at(deadline, &mut handle).await {
        Ok(Ok(result)) => result.map_err(|e| e.to_string()),
        Ok(Err(e)) => Err(format!("the free-space read failed: {e}")),
        Err(_) => {
            *slot = Some(PendingRead {
                dir: dir.to_path_buf(),
                handle,
            });
            Err(format!(
                "the free-space read did not finish within {}s",
                timeout.as_secs()
            ))
        }
    }
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
    /// While the guard was on during this incident, it saw recording off
    /// (stopped by the guard or paused by the user). Recording that is on
    /// again after that was started during the incident.
    pub saw_recording_off: bool,
    /// "Guard is off" was logged for this incident.
    pub guard_off_logged: bool,
}

/// Which low-space notice to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LowNotice {
    /// The guard stopped capture; it starts again once there is room.
    Paused,
    /// Recording was started during the incident and the guard stopped it
    /// again.
    PausedAgain,
    /// Recording was already off; the guard did not stop anything and will
    /// not start anything on recovery.
    RecordingOff,
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
    /// The capture being stopped was started during the incident.
    pub stop_again: bool,
    pub resume_capture: bool,
    pub notify_low: Option<LowNotice>,
    pub notify_resumed: bool,
    pub log_guard_off: bool,
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
        // Switched off mid-incident: give back what the guard took. The
        // switch is how a user records on a nearly full disk.
        p.resume_capture = memory.paused_by_guard && !facts.recording_intended;
        p.memory.paused_by_guard = false;
        p.memory.saw_recording_off = false;
        if !memory.guard_off_logged {
            p.log_guard_off = true;
            p.memory.guard_off_logged = true;
        }
        return p;
    }
    p.memory.guard_off_logged = false;
    if facts.recording_intended {
        // Whatever started it (the tray, a settings save, an engine restart,
        // an update rollback), recording does not run on a nearly full disk
        // while the guard is on.
        p.stop_capture = true;
        p.memory.paused_by_guard = true;
        p.memory.saw_recording_off = true;
        if memory.saw_recording_off {
            // Started during the incident: say why it stopped again.
            p.stop_again = true;
            p.notify_low = Some(LowNotice::PausedAgain);
        } else if !facts.incident_notified {
            p.notify_low = Some(LowNotice::Paused);
        }
    } else {
        p.memory.saw_recording_off = true;
        if !facts.incident_notified {
            p.notify_low = Some(if memory.paused_by_guard {
                LowNotice::Paused
            } else {
                LowNotice::RecordingOff
            });
        }
    }
    if p.notify_low.is_some() {
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
    notice: LowNotice,
    available_bytes: Option<u64>,
    threshold_bytes: Option<u64>,
) -> (String, String) {
    let available = size_or_unknown(available_bytes);
    let threshold = size_or_unknown(threshold_bytes);
    match notice {
        LowNotice::Paused => (
            "Recording paused: the disk is nearly full".to_string(),
            format!(
                "Only {available} is left on the drive that holds your recordings, and the \
                 low-disk guard stops recording at {threshold}. Free some space or change where \
                 recordings are stored; recording starts again by itself once there is room."
            ),
        ),
        LowNotice::PausedAgain => (
            "Recording paused again: the disk is still nearly full".to_string(),
            format!(
                "Recording was started, but only {available} is left on the drive that holds \
                 your recordings (the low-disk guard stops recording at {threshold}). It starts \
                 again by itself once there is room. To record anyway, turn off the low-disk \
                 guard in the storage settings."
            ),
        ),
        LowNotice::RecordingOff => (
            "The disk is nearly full".to_string(),
            format!(
                "Only {available} is left on the drive that holds your recordings. Recording is \
                 already off, and while free space stays at or below {threshold} the low-disk \
                 guard keeps it paused. Free some space or change where recordings are stored, \
                 then start recording again."
            ),
        ),
    }
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
        if plan.stop_capture {
            if plan.stop_again {
                warn!(
                    "low-disk guard: recording was started during the low-disk incident; stopping capture again, {} free (threshold {})",
                    size_or_unknown(reading.available_bytes),
                    size_or_unknown(reading.threshold_bytes)
                );
            } else {
                warn!(
                    "low-disk guard: stopping capture, {} free (threshold {}); the API, search and pipes keep running",
                    size_or_unknown(reading.available_bytes),
                    size_or_unknown(reading.threshold_bytes)
                );
            }
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
        if let Some(notice) = plan.notify_low {
            let (title, body) =
                low_notification_text(notice, reading.available_bytes, reading.threshold_bytes);
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
        assert!(p.stop_capture && !p.stop_again);
        assert_eq!(p.notify_low, Some(LowNotice::Paused));
        assert!(m.paused_by_guard);
        assert!(f.incident_active && f.incident_notified);

        // Repeated low readings (status ticks) change nothing.
        for _ in 0..3 {
            let p = step(Signal::Low, &mut f, &mut m);
            assert_eq!(
                (p.stop_capture, p.notify_low, p.resume_capture),
                (false, None, false)
            );
        }

        let p = step(Signal::Recovered, &mut f, &mut m);
        assert!(p.resume_capture && p.notify_resumed);
        assert!(f.recording_intended);
        assert_eq!(m, GuardMemory::default());
        assert!(!f.incident_active && !f.incident_notified);

        // The next incident is announced again.
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(p.stop_capture);
        assert_eq!(p.notify_low, Some(LowNotice::Paused));
    }

    #[test]
    fn guard_off_only_logs() {
        let mut f = facts(false, true);
        let mut m = GuardMemory::default();
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(p.log_guard_off);
        assert!(!p.stop_capture && p.notify_low.is_none());
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
        // The user still hears about the full disk, without a promise that
        // recording comes back by itself.
        assert_eq!(p.notify_low, Some(LowNotice::RecordingOff));
        let p = step(Signal::Recovered, &mut f, &mut m);
        assert!(!p.resume_capture);
        assert!(!f.recording_intended);
    }

    #[test]
    fn capture_started_during_the_incident_is_stopped_again_with_a_notice() {
        let mut f = facts(true, true);
        let mut m = GuardMemory::default();
        step(Signal::Low, &mut f, &mut m);
        // Something starts capture again: the tray, a settings save that
        // restarts capture, a mic-grant re-init, an engine restart.
        for _ in 0..2 {
            f.recording_intended = true;
            let p = step(Signal::Low, &mut f, &mut m);
            assert!(p.stop_capture && p.stop_again);
            assert_eq!(p.notify_low, Some(LowNotice::PausedAgain));
            assert!(!f.recording_intended && m.paused_by_guard);
            // Until the next start, the ticks are quiet again.
            let p = step(Signal::Low, &mut f, &mut m);
            assert_eq!((p.stop_capture, p.notify_low), (false, None));
        }
        // Recovery starts capture again: the guard stopped it last.
        let p = step(Signal::Recovered, &mut f, &mut m);
        assert!(p.resume_capture && p.notify_resumed);
    }

    #[test]
    fn starting_a_recording_the_user_had_paused_is_stopped_too() {
        let mut f = facts(true, false);
        let mut m = GuardMemory::default();
        let p = step(Signal::Low, &mut f, &mut m);
        assert_eq!(p.notify_low, Some(LowNotice::RecordingOff));
        f.recording_intended = true;
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(p.stop_capture && p.stop_again);
        assert_eq!(p.notify_low, Some(LowNotice::PausedAgain));
        // Now the guard stopped it, so recovery brings it back.
        let p = step(Signal::Recovered, &mut f, &mut m);
        assert!(p.resume_capture);
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
        // With the guard off, recording keeps running.
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(!p.stop_capture && p.notify_low.is_none());
        // Switching it back on stops capture again, without a second notice.
        f.guard_enabled = true;
        let p = step(Signal::Low, &mut f, &mut m);
        assert!(p.stop_capture && !p.stop_again);
        assert!(p.notify_low.is_none());
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
        assert!(p.stop_capture && !p.stop_again);
        assert!(p.notify_low.is_none());
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
        let notices = [
            LowNotice::Paused,
            LowNotice::PausedAgain,
            LowNotice::RecordingOff,
        ];
        let mut titles = std::collections::BTreeSet::new();
        for notice in notices {
            let (title, body) = low_notification_text(notice, Some(gib), Some(2 * gib));
            assert!(!title.is_empty());
            assert!(
                body.contains("1.0 GiB") && body.contains("2.0 GiB"),
                "{notice:?}: {body}"
            );
            let (_, body) = low_notification_text(notice, None, None);
            assert!(body.contains("unknown"), "{notice:?}: {body}");
            titles.insert(title);
        }
        assert_eq!(titles.len(), notices.len(), "each notice reads differently");
        let (_, body) = resumed_notification_text(Some(3 * gib));
        assert!(body.contains("3.0 GiB"), "{body}");
    }

    #[test]
    fn only_a_notice_for_a_stop_by_the_guard_promises_an_automatic_restart() {
        let promise = "by itself";
        for (notice, promises) in [
            (LowNotice::Paused, true),
            (LowNotice::PausedAgain, true),
            (LowNotice::RecordingOff, false),
        ] {
            let (_, body) = low_notification_text(notice, Some(1), Some(2));
            assert_eq!(body.contains(promise), promises, "{notice:?}: {body}");
        }
        let (_, body) = low_notification_text(LowNotice::PausedAgain, Some(1), Some(2));
        assert!(body.contains("turn off the low-disk guard"), "{body}");
    }

    fn status_for(
        dir: &str,
        threshold_bytes: u64,
        last_error: Option<&str>,
    ) -> monitor::DiskPressureStatus {
        monitor::DiskPressureStatus {
            low: false,
            available_bytes: last_error.is_none().then_some(50),
            total_bytes: last_error.is_none().then_some(100),
            threshold_bytes,
            recovery_threshold_bytes: threshold_bytes + 1,
            data_dir: dir.to_string(),
            checked_at: chrono::Utc::now(),
            last_error: last_error.map(str::to_string),
        }
    }

    #[test]
    fn the_config_uses_the_monitor_threshold_for_its_own_dir_even_after_a_failed_read() {
        let dir = std::env::temp_dir().join("fp-w3-30-config");
        let dir_str = dir.display().to_string();
        let ok = status_for(&dir_str, 7, None);
        assert_eq!(threshold_from_status(Some(&ok), &dir), Some(7));
        // A failing volume: no read of our own, the monitor's limits stand.
        let failing = status_for(&dir_str, 3, Some("the free-space read did not finish"));
        assert_eq!(threshold_from_status(Some(&failing), &dir), Some(3));
        // Another directory, or no monitor: the command reads for itself.
        let other = status_for("/somewhere/else", 7, None);
        assert_eq!(threshold_from_status(Some(&other), &dir), None);
        assert_eq!(threshold_from_status(None, &dir), None);
    }

    fn limits(threshold_bytes: u64) -> monitor::Thresholds {
        monitor::Thresholds::with_headroom(threshold_bytes)
    }

    #[tokio::test]
    async fn a_hung_config_read_is_shared_not_repeated() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let pending = tokio::sync::Mutex::new(None);
        let dir = PathBuf::from("/hung/share");
        let reads = Arc::new(AtomicUsize::new(0));
        let (release, wait) = std::sync::mpsc::channel::<()>();

        let counted = Arc::clone(&reads);
        let first = read_thresholds(&pending, &dir, Duration::from_millis(50), move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = wait.recv();
            Ok(limits(11))
        })
        .await;
        assert!(first.unwrap_err().contains("did not finish"));

        // While the first read hangs, later calls start no read of their own.
        for _ in 0..3 {
            let counted = Arc::clone(&reads);
            let again = read_thresholds(&pending, &dir, Duration::from_millis(50), move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok(limits(99))
            })
            .await;
            assert!(again.is_err());
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);

        // Once it finishes, the waiting caller gets its result.
        release.send(()).unwrap();
        let counted = Arc::clone(&reads);
        let done = read_thresholds(&pending, &dir, Duration::from_secs(5), move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(limits(99))
        })
        .await;
        assert_eq!(done.unwrap().threshold_bytes, 11);
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert!(pending.lock().await.is_none());

        // The next call reads afresh.
        let fresh =
            read_thresholds(&pending, &dir, Duration::from_secs(5), |_| Ok(limits(5))).await;
        assert_eq!(fresh.unwrap().threshold_bytes, 5);
    }

    #[tokio::test]
    async fn a_config_read_for_another_dir_is_not_held_up() {
        let pending = tokio::sync::Mutex::new(None);
        let (_release, wait) = std::sync::mpsc::channel::<()>();
        let hung = read_thresholds(
            &pending,
            Path::new("/hung/share"),
            Duration::from_millis(50),
            move |_| {
                let _ = wait.recv();
                Ok(limits(11))
            },
        )
        .await;
        assert!(hung.is_err());
        let other = read_thresholds(
            &pending,
            Path::new("/local/disk"),
            Duration::from_secs(5),
            |dir| {
                assert_eq!(dir, Path::new("/local/disk"));
                Ok(limits(4))
            },
        )
        .await;
        assert_eq!(other.unwrap().threshold_bytes, 4);
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
