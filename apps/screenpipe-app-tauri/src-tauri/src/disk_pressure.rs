// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Low-disk recording guard (`stopRecordingOnLowDisk`, default on).
//!
//! The engine's monitor (`screenpipe_engine::disk_pressure`) watches the
//! volume of the data directory and publishes `disk_space_low` and, once
//! there is room again (with headroom, so it does not flap),
//! `disk_space_recovered`. This module acts on them:
//!
//! - On a fresh low reading with the guard on, it stops capture (the API,
//!   search and pipes keep running) and posts one notification with a
//!   button that opens the storage settings. With the guard off it only
//!   logs.
//! - "Fresh" means a reading the guard has not acted on yet: the first low
//!   reading of a monitor run (each engine start or restart has its own
//!   generation), or of a new incident after a recovery. Recording started
//!   after the guard acted (from the tray, a deep link, a settings save that
//!   only restarts capture) keeps running: that is how a user records on a
//!   nearly full disk. A later engine restart ("Apply & Restart", an update
//!   rollback, a failed relaunch, crash recovery) starts a new run, whose
//!   low reading stops capture again, with a notice that says how to keep
//!   recording.
//! - On recovery it starts capture again, but only if the guard's stop is
//!   still the last change to recording. A recording the user paused, or
//!   started and paused again, stays paused. Recovery means free space past
//!   the recovery threshold. A new monitor run whose first reading falls
//!   between the two thresholds reports no incident; while the guard still
//!   owes a restart, such a reading does not end the incident.
//! - Whether the current incident was already announced is kept in
//!   `disk-pressure-notification-state.json` in the data dir, so restarting
//!   the app does not repeat the notification. Recovery re-arms it.
//!
//! Besides the events it re-reads the monitor's latest status every check
//! interval. That catches an event published before this task subscribed,
//! a recovery that happened while the app was closed, and the guard being
//! switched off or on in the middle of an incident.
//!
//! `get_low_disk_guard_config` gives the settings page the threshold that
//! applies to the active data dir.

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
/// Label of the `stopRecordingOnLowDisk` switch in the storage settings
/// (`LOW_DISK_GUARD_LABEL` in `lib/low-disk-guard.ts`). The notices name the
/// switch, so the two must read the same.
pub const GUARD_SWITCH_LABEL: &str = "Pause recording on low disk";

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
/// (waiting for another caller's read counts against it). A read kept for
/// `dir` is awaited rather than started again, whether it is still running
/// or has finished since (its answer is then used as it is); one for another
/// directory is dropped and a fresh read started.
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
        Some(earlier) if earlier.dir == dir => earlier.handle,
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
    /// Free space is at or below the threshold, as read by the monitor run
    /// with this generation (`disk_space_low`, or a low status).
    Low(u64),
    /// `disk_space_recovered` arrived.
    Recovered,
    /// The latest status shows no incident, but free space is not past the
    /// recovery threshold. A monitor run that starts with the disk in that
    /// band (an engine restart mid-incident) reports no incident, because it
    /// never saw the drop.
    Band,
    /// The latest status shows no incident, with free space past the
    /// recovery threshold.
    Normal,
}

/// What the guard remembers while the app runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GuardMemory {
    /// The monitor run whose low reading the guard has acted on. Further
    /// low readings of that run change nothing, so a recording started
    /// after the guard acted keeps running until a fresh `disk_space_low`
    /// (a new monitor run, which an engine restart starts, or a new
    /// incident after a recovery).
    pub handled_generation: Option<u64>,
    /// The guard stopped capture: the recording-intent epoch right after
    /// its stop. Any later change of intent (someone started or paused
    /// recording) means the pause is no longer the guard's to undo.
    pub guard_stop_epoch: Option<u64>,
    /// The guard has acted on an earlier monitor run of this incident in
    /// this app run, so a recording it finds on at a fresh run was started
    /// during the incident.
    pub acted_this_incident: bool,
    /// "Guard is off" was logged for this incident.
    pub guard_off_logged: bool,
}

/// Which low-space notice to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LowNotice {
    /// The guard stopped capture; it starts again once there is room.
    Paused,
    /// Recording was started during the incident, the recording engine
    /// restarted, and the guard stopped capture again.
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
    /// How often the recording intent has changed so far
    /// (`health::recording_intent_epoch`).
    pub intent_epoch: u64,
    /// Persisted: the current incident was announced.
    pub incident_notified: bool,
}

/// What to do, and the state that follows. After a stop the runtime
/// records the intent epoch in `memory.guard_stop_epoch`.
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

/// Whether the guard's own stop is still the last word on recording: it
/// stopped capture, and nobody has started or paused recording since.
fn guard_pause_holds(facts: Facts, memory: GuardMemory) -> bool {
    !facts.recording_intended && memory.guard_stop_epoch == Some(facts.intent_epoch)
}

/// The guard's rules (see the module docs).
pub fn plan(signal: Signal, facts: Facts, memory: GuardMemory) -> Plan {
    match signal {
        Signal::Low(generation) => plan_low(Some(generation), facts, memory),
        // The guard's pause stands until there is room past the recovery
        // threshold, also when a new monitor run starts inside the band. A
        // reading there is not a fresh low one either, so it stops nothing.
        Signal::Band if guard_pause_holds(facts, memory) => plan_low(None, facts, memory),
        // With nothing to give back, the band ends the incident as before:
        // a later drop to the threshold is a new incident, announced afresh.
        Signal::Band | Signal::Recovered | Signal::Normal => {
            let resume = guard_pause_holds(facts, memory);
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

/// An incident goes on: `generation` is the run behind a low reading, `None`
/// for a reading inside the band, which never counts as fresh.
fn plan_low(generation: Option<u64>, facts: Facts, memory: GuardMemory) -> Plan {
    let holds = guard_pause_holds(facts, memory);
    let mut p = Plan {
        memory,
        incident_active: true,
        incident_notified: facts.incident_notified,
        ..Plan::default()
    };
    if !holds {
        // Someone started or paused recording since the guard's stop; a
        // recovery must not override that.
        p.memory.guard_stop_epoch = None;
    }
    if !facts.guard_enabled {
        // Switched off mid-incident: give back what the guard took, and
        // forget what it did, so switching it on again acts afresh.
        p.resume_capture = holds;
        p.memory = GuardMemory {
            guard_off_logged: true,
            ..GuardMemory::default()
        };
        p.log_guard_off = !memory.guard_off_logged;
        return p;
    }
    p.memory.guard_off_logged = false;
    let Some(generation) = generation else {
        return p;
    };
    if memory.handled_generation == Some(generation) {
        // Already acted on this run's reading. A recording started since
        // then (from the tray, a deep link, a settings save that only
        // restarts capture) is the user's choice and keeps running.
        return p;
    }
    p.memory.handled_generation = Some(generation);
    p.memory.acted_this_incident = true;
    if facts.recording_intended {
        p.stop_capture = true;
        if memory.acted_this_incident {
            // Started during the incident, then the engine restarted.
            p.stop_again = true;
            p.notify_low = Some(LowNotice::PausedAgain);
        } else if !facts.incident_notified {
            p.notify_low = Some(LowNotice::Paused);
        }
    } else if !facts.incident_notified {
        p.notify_low = Some(if holds {
            LowNotice::Paused
        } else {
            LowNotice::RecordingOff
        });
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
        // The event has no threshold, and nothing on recovery quotes one.
        Self {
            available_bytes: Some(e.available_bytes),
            threshold_bytes: None,
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
    let anyway = format!(
        "To record anyway, start recording again, or turn off \"{GUARD_SWITCH_LABEL}\" in the \
         storage settings."
    );
    match notice {
        LowNotice::Paused => (
            "Recording paused: the disk is nearly full".to_string(),
            format!(
                "Only {available} is left on the drive that holds your recordings (recording \
                 pauses at {threshold}). Free some space or change where recordings are stored; \
                 recording starts again by itself once there is room. {anyway}"
            ),
        ),
        LowNotice::PausedAgain => (
            "Recording paused again: the disk is still nearly full".to_string(),
            format!(
                "The recording engine restarted and found only {available} left on the drive \
                 that holds your recordings (recording pauses at {threshold}), so recording is \
                 paused again. It starts again by itself once there is room. {anyway}"
            ),
        ),
        LowNotice::RecordingOff => (
            "The disk is nearly full".to_string(),
            format!(
                "Only {available} is left on the drive that holds your recordings (recording \
                 pauses at {threshold}). Recording is already off. Free some space or change \
                 where recordings are stored before you start it again."
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
        // Only a reading that can keep an incident going consults the
        // setting; skip the store read on the routine "all fine" checks.
        let guard_enabled =
            !matches!(signal, Signal::Low(_) | Signal::Band) || self.guard_enabled();
        // The epoch before the intent: a change between the two reads then
        // shows as a newer epoch, never as a missed one.
        let intent_epoch = crate::health::recording_intent_epoch();
        let facts = Facts {
            guard_enabled,
            recording_intended: crate::health::recording_intended(),
            intent_epoch,
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
                    "low-disk guard: the engine restarted during the low-disk incident with recording on; stopping capture again, {} free (threshold {})",
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
            // The stop set the intent off; any later change is someone else's.
            plan.memory.guard_stop_epoch = Some(crate::health::recording_intent_epoch());
        }
        if plan.resume_capture {
            if let Err(e) = self.start_capture().await {
                warn!("low-disk guard: could not start capture again: {e}");
                plan.notify_resumed = false;
                // A start that got as far as setting the intent is now the
                // regular start/respawn machinery's job; otherwise the pause
                // is still the guard's, and the next check retries.
                let epoch = crate::health::recording_intent_epoch();
                if !crate::health::recording_intended() {
                    plan.memory.guard_stop_epoch = Some(epoch);
                }
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
                match low_event_generation(&event.data, monitor::latest_status().as_ref()) {
                    Some(generation) => {
                        guard.handle(Signal::Low(generation), Reading::from(&event.data)).await;
                    }
                    // Overtaken by a newer reading that is not low; the next
                    // check acts on that one.
                    None => debug!("low-disk guard: {} superseded", monitor::DISK_SPACE_LOW_EVENT),
                }
            }
            Some(event) = recoveries.next() => {
                guard.handle(Signal::Recovered, Reading::from(&event.data)).await;
            }
            _ = ticks.tick() => {
                if let Some((signal, reading)) = status_signal(monitor::latest_status().as_ref()) {
                    guard.handle(signal, reading).await;
                }
            }
        }
    }
}

/// The monitor run behind a `disk_space_low` event. The monitor stores a
/// reading's status before it publishes the event, so the latest status is
/// that reading or a newer one; `None` when the newest reading for that
/// directory is no longer low.
fn low_event_generation(
    event: &DiskSpaceLowEvent,
    latest: Option<&monitor::DiskPressureStatus>,
) -> Option<u64> {
    latest
        .filter(|s| s.low && s.data_dir == event.data_dir)
        .map(|s| s.generation)
}

/// What a periodic check reads from the monitor's latest status. A failed
/// read says nothing new: the monitor keeps its level, and only a
/// successful read can start or end an incident.
fn status_signal(status: Option<&monitor::DiskPressureStatus>) -> Option<(Signal, Reading)> {
    let status = status.filter(|s| s.last_error.is_none())?;
    let in_band = status
        .available_bytes
        .is_some_and(|available| available <= status.recovery_threshold_bytes);
    let signal = if status.low {
        Signal::Low(status.generation)
    } else if in_band {
        Signal::Band
    } else {
        Signal::Normal
    };
    let reading = Reading {
        available_bytes: status.available_bytes,
        threshold_bytes: Some(status.threshold_bytes),
    };
    Some((signal, reading))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run of the decision rules against a simulated recording intent,
    /// applying each plan the way the runtime does (assuming every action
    /// succeeds).
    struct Sim {
        f: Facts,
        m: GuardMemory,
        /// The persisted `incident_active`.
        incident_active: bool,
    }

    impl Sim {
        fn new(guard_enabled: bool, recording_intended: bool) -> Self {
            Self {
                f: Facts {
                    guard_enabled,
                    recording_intended,
                    intent_epoch: 0,
                    incident_notified: false,
                },
                m: GuardMemory::default(),
                incident_active: false,
            }
        }

        fn set_intent(&mut self, on: bool) {
            if self.f.recording_intended != on {
                self.f.recording_intended = on;
                self.f.intent_epoch += 1;
            }
        }

        /// The user (tray, deep link, settings save) starts recording.
        fn user_start(&mut self) {
            self.set_intent(true);
        }

        /// The user pauses recording.
        fn user_pause(&mut self) {
            self.set_intent(false);
        }

        fn step(&mut self, signal: Signal) -> Plan {
            let mut p = plan(signal, self.f, self.m);
            if p.stop_capture {
                self.set_intent(false);
                p.memory.guard_stop_epoch = Some(self.f.intent_epoch);
            }
            if p.resume_capture {
                self.set_intent(true);
            }
            self.m = p.memory;
            self.incident_active = p.incident_active;
            self.f.incident_notified = p.incident_notified;
            p
        }

        fn quiet(&mut self, signal: Signal) {
            let p = self.step(signal);
            assert_eq!(
                (
                    p.stop_capture,
                    p.resume_capture,
                    p.notify_low,
                    p.notify_resumed
                ),
                (false, false, None, false),
                "{signal:?} should change nothing"
            );
        }
    }

    #[test]
    fn low_space_stops_capture_and_notifies_once_then_recovery_resumes() {
        let mut s = Sim::new(true, true);

        let p = s.step(Signal::Low(1));
        assert!(p.stop_capture && !p.stop_again);
        assert_eq!(p.notify_low, Some(LowNotice::Paused));
        assert!(!s.f.recording_intended);
        assert!(s.incident_active && s.f.incident_notified);

        // Repeated low readings of the same run (status ticks) change nothing.
        for _ in 0..3 {
            s.quiet(Signal::Low(1));
        }

        let p = s.step(Signal::Recovered);
        assert!(p.resume_capture && p.notify_resumed);
        assert!(s.f.recording_intended);
        assert_eq!(s.m, GuardMemory::default());
        assert!(!s.incident_active && !s.f.incident_notified);

        // The next incident, even in the same monitor run, is announced again.
        let p = s.step(Signal::Low(1));
        assert!(p.stop_capture);
        assert_eq!(p.notify_low, Some(LowNotice::Paused));
    }

    #[test]
    fn guard_off_only_logs() {
        let mut s = Sim::new(false, true);
        let p = s.step(Signal::Low(1));
        assert!(p.log_guard_off);
        assert!(!p.stop_capture && p.notify_low.is_none());
        // Logged once per incident.
        let p = s.step(Signal::Low(1));
        assert!(!p.log_guard_off);
        let p = s.step(Signal::Recovered);
        assert!(!p.resume_capture && !p.notify_resumed);
        assert!(s.f.recording_intended);
    }

    #[test]
    fn a_recording_the_user_paused_is_not_resumed() {
        let mut s = Sim::new(true, false);
        let p = s.step(Signal::Low(1));
        assert!(!p.stop_capture);
        // The user still hears about the full disk, without a promise that
        // recording comes back by itself.
        assert_eq!(p.notify_low, Some(LowNotice::RecordingOff));
        s.quiet(Signal::Recovered);
        assert!(!s.f.recording_intended);
    }

    #[test]
    fn a_start_after_the_guard_stopped_capture_stands_until_a_fresh_low() {
        let mut s = Sim::new(true, true);
        s.step(Signal::Low(1));
        assert!(!s.f.recording_intended);

        // The user starts recording from the tray, a deep link or a settings
        // save that only restarts capture: the same monitor run, so the
        // checks leave it running.
        s.user_start();
        for _ in 0..3 {
            s.quiet(Signal::Low(1));
            assert!(s.f.recording_intended);
        }
        // The guard no longer owes a restart: the recording is the user's.
        assert_eq!(s.m.guard_stop_epoch, None);

        // An engine restart starts a new monitor run, whose first low reading
        // is fresh: capture is stopped again, with a notice saying how to keep
        // recording.
        let p = s.step(Signal::Low(2));
        assert!(p.stop_capture && p.stop_again);
        assert_eq!(p.notify_low, Some(LowNotice::PausedAgain));
        assert!(!s.f.recording_intended);
        s.quiet(Signal::Low(2));

        // That stop is the guard's, so recovery brings recording back.
        let p = s.step(Signal::Recovered);
        assert!(p.resume_capture && p.notify_resumed);
    }

    #[test]
    fn a_start_while_the_user_had_paused_stands_too() {
        let mut s = Sim::new(true, false);
        let p = s.step(Signal::Low(1));
        assert_eq!(p.notify_low, Some(LowNotice::RecordingOff));
        s.user_start();
        s.quiet(Signal::Low(1));
        assert!(s.f.recording_intended);
        // Recovery changes nothing: recording is already on.
        s.quiet(Signal::Recovered);
        assert!(s.f.recording_intended);
    }

    #[test]
    fn a_start_and_pause_after_the_guards_stop_is_not_undone_on_recovery() {
        let mut s = Sim::new(true, true);
        s.step(Signal::Low(1));
        // Both changes land between two checks: the guard never sees
        // recording on, but the epoch shows that someone took over.
        s.user_start();
        s.user_pause();
        s.quiet(Signal::Low(1));
        let p = s.step(Signal::Recovered);
        assert!(!p.resume_capture && !p.notify_resumed);
        assert!(!s.f.recording_intended);
    }

    #[test]
    fn a_new_run_that_finds_recording_off_does_not_stop_or_notify() {
        let mut s = Sim::new(true, true);
        s.step(Signal::Low(1));
        // The engine restarts while recording is still paused by the guard.
        s.quiet(Signal::Low(2));
        // The pause is still the guard's: recovery resumes.
        let p = s.step(Signal::Normal);
        assert!(p.resume_capture);
    }

    #[test]
    fn a_new_run_inside_the_band_keeps_the_guards_pause_until_there_is_room() {
        let mut s = Sim::new(true, true);
        s.step(Signal::Low(1));
        assert!(!s.f.recording_intended);
        // The engine restarts without touching the recording intent (a crash
        // respawn) after some space was freed: the new run's first reading is
        // above the threshold but not past the recovery threshold, so it
        // reports no incident. The guard's pause stands, and nothing else
        // happens either.
        for _ in 0..3 {
            s.quiet(Signal::Band);
        }
        assert!(s.incident_active && s.f.incident_notified);
        assert!(!s.f.recording_intended);
        // Only room past the recovery threshold brings recording back.
        let p = s.step(Signal::Normal);
        assert!(p.resume_capture && p.notify_resumed);
        assert!(!s.incident_active && !s.f.incident_notified);
    }

    #[test]
    fn switching_the_guard_off_inside_the_band_gives_capture_back() {
        let mut s = Sim::new(true, true);
        s.step(Signal::Low(1));
        s.quiet(Signal::Band);
        s.f.guard_enabled = false;
        let p = s.step(Signal::Band);
        assert!(p.resume_capture && p.log_guard_off);
        assert!(s.f.recording_intended);
    }

    #[test]
    fn the_band_ends_an_incident_when_the_guard_owes_no_restart() {
        // The user started recording after the guard's stop, then the engine
        // restarted inside the band: the incident ends quietly, and a later
        // drop to the threshold is a new one, announced as such.
        let mut s = Sim::new(true, true);
        s.step(Signal::Low(1));
        s.user_start();
        s.quiet(Signal::Band);
        assert!(!s.incident_active && !s.f.incident_notified);
        assert!(s.f.recording_intended);
        let p = s.step(Signal::Low(2));
        assert!(p.stop_capture && !p.stop_again);
        assert_eq!(p.notify_low, Some(LowNotice::Paused));

        // An app restart inside the band: the app started capture itself, so
        // the guard owes nothing, and the persisted incident is cleared.
        let mut s = Sim::new(true, true);
        s.incident_active = true;
        s.f.incident_notified = true;
        s.quiet(Signal::Band);
        assert!(!s.incident_active && !s.f.incident_notified);
        assert!(s.f.recording_intended);
    }

    #[test]
    fn switching_the_guard_off_mid_incident_resumes_capture_and_on_again_stops_it() {
        let mut s = Sim::new(true, true);
        s.step(Signal::Low(1));
        assert!(!s.f.recording_intended);
        s.f.guard_enabled = false;
        let p = s.step(Signal::Low(1));
        assert!(p.resume_capture && p.log_guard_off);
        assert!(s.f.recording_intended);
        assert_eq!(s.m.guard_stop_epoch, None);
        // With the guard off, recording keeps running.
        let p = s.step(Signal::Low(1));
        assert!(!p.stop_capture && p.notify_low.is_none() && !p.log_guard_off);
        // Switching it back on is a choice made for this incident: capture
        // stops again, without a second notice.
        s.f.guard_enabled = true;
        let p = s.step(Signal::Low(1));
        assert!(p.stop_capture && !p.stop_again);
        assert!(p.notify_low.is_none());
    }

    #[test]
    fn switching_the_guard_on_mid_incident_announces_it_when_nobody_was_told() {
        let mut s = Sim::new(false, true);
        s.step(Signal::Low(1));
        s.f.guard_enabled = true;
        let p = s.step(Signal::Low(1));
        assert!(p.stop_capture);
        assert_eq!(p.notify_low, Some(LowNotice::Paused));
    }

    #[test]
    fn switching_the_guard_off_leaves_a_users_pause_alone() {
        let mut s = Sim::new(true, true);
        s.step(Signal::Low(1));
        s.user_start();
        s.user_pause();
        s.f.guard_enabled = false;
        let p = s.step(Signal::Low(1));
        assert!(!p.resume_capture);
        assert!(!s.f.recording_intended);
    }

    #[test]
    fn an_app_restart_mid_incident_stops_capture_without_a_second_notice() {
        // Persisted: the incident was announced by the previous app run.
        let mut s = Sim::new(true, true);
        s.incident_active = true;
        s.f.incident_notified = true;
        let p = s.step(Signal::Low(1));
        assert!(p.stop_capture && !p.stop_again);
        assert!(p.notify_low.is_none());
    }

    #[test]
    fn a_normal_status_clears_a_stale_incident_quietly() {
        let mut s = Sim::new(true, true);
        s.incident_active = true;
        s.f.incident_notified = true;
        s.quiet(Signal::Normal);
        assert!(!s.incident_active && !s.f.incident_notified);
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
        let mut s = Sim::new(true, true);
        let p = s.step(Signal::Low(1));
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
        let p = s.step(Signal::Low(1));
        assert_eq!(next_state(&s1, &p, later, "t2".into()), s1);
        let p = s.step(Signal::Recovered);
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
        for (notice, stopped_by_guard) in [
            (LowNotice::Paused, true),
            (LowNotice::PausedAgain, true),
            (LowNotice::RecordingOff, false),
        ] {
            let (_, body) = low_notification_text(notice, Some(1), Some(2));
            assert_eq!(
                body.contains(promise),
                stopped_by_guard,
                "{notice:?}: {body}"
            );
            // A stop by the guard says how to keep recording, naming the
            // switch exactly as the storage settings label it.
            let names_switch = body.contains(&format!("\"{GUARD_SWITCH_LABEL}\""));
            assert_eq!(names_switch, stopped_by_guard, "{notice:?}: {body}");
            assert_eq!(
                body.contains("start recording again"),
                stopped_by_guard,
                "{notice:?}: {body}"
            );
        }
    }

    #[test]
    fn the_switch_label_matches_the_settings_page() {
        let ts = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("lib")
            .join("low-disk-guard.ts");
        let source = std::fs::read_to_string(&ts).unwrap();
        assert!(
            source.contains(&format!("LOW_DISK_GUARD_LABEL = \"{GUARD_SWITCH_LABEL}\"")),
            "{} must define LOW_DISK_GUARD_LABEL as {GUARD_SWITCH_LABEL:?}",
            ts.display()
        );
    }

    #[test]
    fn a_low_event_takes_the_generation_of_the_latest_low_status() {
        let event = DiskSpaceLowEvent {
            available_bytes: 1,
            threshold_bytes: 2,
            data_dir: "/data".into(),
        };
        let mut status = status_for("/data", 2, None);
        status.low = true;
        status.generation = 7;
        assert_eq!(low_event_generation(&event, Some(&status)), Some(7));
        // A newer reading that is no longer low, or one for another dir, or
        // none at all: the event is left to the next check.
        status.low = false;
        assert_eq!(low_event_generation(&event, Some(&status)), None);
        let mut other = status_for("/elsewhere", 2, None);
        other.low = true;
        assert_eq!(low_event_generation(&event, Some(&other)), None);
        assert_eq!(low_event_generation(&event, None), None);
    }

    #[test]
    fn a_check_reads_the_level_and_run_from_the_status() {
        let mut status = status_for("/data", 2, None);
        status.generation = 3;
        assert_eq!(status_signal(Some(&status)).unwrap().0, Signal::Normal);
        // Above the threshold but not past the recovery threshold (50 free,
        // recovery at 50): the band, unless the run itself reports low.
        let mut band = status_for("/data", 49, None);
        band.generation = 3;
        assert_eq!(status_signal(Some(&band)).unwrap().0, Signal::Band);
        band.low = true;
        assert_eq!(status_signal(Some(&band)).unwrap().0, Signal::Low(3));
        status.low = true;
        let (signal, reading) = status_signal(Some(&status)).unwrap();
        assert_eq!(signal, Signal::Low(3));
        assert_eq!(reading.available_bytes, Some(50));
        assert_eq!(reading.threshold_bytes, Some(2));
        // A failed read says nothing new.
        let failed = status_for("/data", 2, Some("unreadable"));
        assert_eq!(status_signal(Some(&failed)), None);
        assert_eq!(status_signal(None), None);
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
            generation: 0,
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

        // Once it finishes, the next caller gets its result, also when the
        // read ended before that call came in (the path that used to start a
        // second read).
        release.send(()).unwrap();
        loop {
            let finished = pending
                .lock()
                .await
                .as_ref()
                .is_some_and(|p| p.handle.is_finished());
            if finished {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
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
