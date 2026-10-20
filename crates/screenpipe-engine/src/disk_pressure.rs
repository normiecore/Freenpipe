// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Low-disk monitor for the data directory.
//!
//! A background worker reads the free space of the volume that holds the
//! active data directory, once at start and then every
//! [`DEFAULT_CHECK_INTERVAL`]. When the free bytes fall to or below the
//! threshold it publishes `disk_space_low` on the event bus. It publishes
//! `disk_space_recovered` only after the free bytes climb above a higher
//! recovery threshold, so a volume that hovers around the threshold does not
//! make capture stop and start over and over.
//!
//! The engine only reports. The desktop app decides what happens: with
//! `stopRecordingOnLowDisk` on it stops capture and tells the user, and it
//! starts capture again on recovery. [`latest_status`] keeps the newest
//! reading for the app and for `/health`.
//!
//! Thresholds are Freenpipe's choice (the shipped numbers were not
//! observable): 2 % of the volume, never less than 2 GiB and never more than
//! 10 GiB, and recovery needs 1 GiB on top of that.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::core::{WorkerContext, WorkerFuture};

/// Event published when free space drops to or below the threshold.
pub const DISK_SPACE_LOW_EVENT: &str = "disk_space_low";
/// Event published when free space is back above the recovery threshold.
pub const DISK_SPACE_RECOVERED_EVENT: &str = "disk_space_recovered";
/// Name of the background worker in the engine's registry.
pub const WORKER_NAME: &str = "disk_pressure";

const GIB: u64 = 1024 * 1024 * 1024;
/// Smallest threshold, whatever the size of the volume.
pub const MIN_THRESHOLD_BYTES: u64 = 2 * GIB;
/// Largest threshold, so a big volume is not declared full with tens of
/// gigabytes still free.
pub const MAX_THRESHOLD_BYTES: u64 = 10 * GIB;
/// Share of the volume used as the threshold between the two bounds.
pub const THRESHOLD_VOLUME_PERCENT: u64 = 2;
/// Extra free space needed, above the threshold, before an incident ends.
pub const RECOVERY_HEADROOM_BYTES: u64 = GIB;
/// How often the volume is read.
pub const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_secs(30);
/// A read that takes longer than this (an unplugged drive, a stalled network
/// share) counts as a failed read. It is not started again until it ends.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Events and status
// ---------------------------------------------------------------------------

/// Payload of `disk_space_low`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskSpaceLowEvent {
    pub available_bytes: u64,
    pub threshold_bytes: u64,
    pub data_dir: String,
}

/// Payload of `disk_space_recovered`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskSpaceRecoveredEvent {
    pub available_bytes: u64,
    #[serde(default)]
    pub threshold_bytes: u64,
    pub recovery_threshold_bytes: u64,
    pub data_dir: String,
}

/// The monitor's latest reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskPressureStatus {
    /// An incident is in progress: free space reached the threshold and has
    /// not yet climbed past the recovery threshold.
    pub low: bool,
    /// Free bytes available to this process; `None` when the read failed.
    pub available_bytes: Option<u64>,
    /// Size of the volume; `None` when the read failed.
    pub total_bytes: Option<u64>,
    pub threshold_bytes: u64,
    pub recovery_threshold_bytes: u64,
    pub data_dir: String,
    /// When this reading was taken.
    pub checked_at: DateTime<Utc>,
    /// Why the read failed, when it did.
    pub last_error: Option<String>,
}

static LATEST: RwLock<Option<DiskPressureStatus>> = RwLock::new(None);

/// The newest reading of the running monitor, or `None` before its first one.
pub fn latest_status() -> Option<DiskPressureStatus> {
    LATEST.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn store_status(status: &DiskPressureStatus) {
    *LATEST.write().unwrap_or_else(|e| e.into_inner()) = Some(status.clone());
}

// ---------------------------------------------------------------------------
// Thresholds and hysteresis
// ---------------------------------------------------------------------------

/// Free space of one volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeSpace {
    /// Bytes this process may still write (quota and reserved blocks taken out).
    pub available_bytes: u64,
    pub total_bytes: u64,
}

/// The two limits of the hysteresis band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    /// At or below this, capture should stop.
    pub threshold_bytes: u64,
    /// Above this, an incident is over.
    pub recovery_threshold_bytes: u64,
}

impl Thresholds {
    /// `threshold_bytes` with the standard recovery headroom on top.
    pub fn with_headroom(threshold_bytes: u64) -> Self {
        Self {
            threshold_bytes,
            recovery_threshold_bytes: threshold_bytes.saturating_add(RECOVERY_HEADROOM_BYTES),
        }
    }

    /// The thresholds for a volume of `total_bytes`.
    pub fn for_volume(total_bytes: u64) -> Self {
        let scaled = total_bytes / 100 * THRESHOLD_VOLUME_PERCENT;
        Self::with_headroom(scaled.clamp(MIN_THRESHOLD_BYTES, MAX_THRESHOLD_BYTES))
    }

    /// The thresholds used while the volume size is unknown.
    pub fn floor() -> Self {
        Self::with_headroom(MIN_THRESHOLD_BYTES)
    }
}

/// How the monitor picks its thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThresholdRule {
    /// [`Thresholds::for_volume`] of the volume read each time.
    VolumeScaled,
    /// Fixed limits (tests and diagnostics).
    Fixed(Thresholds),
}

impl ThresholdRule {
    pub fn resolve(&self, space: Option<&VolumeSpace>) -> Thresholds {
        match (self, space) {
            (Self::Fixed(t), _) => *t,
            (Self::VolumeScaled, Some(space)) => Thresholds::for_volume(space.total_bytes),
            (Self::VolumeScaled, None) => Thresholds::floor(),
        }
    }
}

/// Where the free space currently stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PressureLevel {
    /// Nothing read yet.
    #[default]
    Unknown,
    Normal,
    Low,
}

/// A change of level worth an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    BecameLow,
    Recovered,
}

/// The hysteresis. Entering `Low` needs free space at or below the
/// threshold; leaving it needs free space above the recovery threshold.
/// Anything in between keeps the current level.
#[derive(Debug, Default)]
pub struct PressureTracker {
    level: PressureLevel,
}

impl PressureTracker {
    pub fn level(&self) -> PressureLevel {
        self.level
    }

    pub fn observe(&mut self, available_bytes: u64, limits: Thresholds) -> Option<Transition> {
        match self.level {
            PressureLevel::Unknown | PressureLevel::Normal => {
                if available_bytes <= limits.threshold_bytes {
                    self.level = PressureLevel::Low;
                    Some(Transition::BecameLow)
                } else {
                    self.level = PressureLevel::Normal;
                    None
                }
            }
            PressureLevel::Low => {
                if available_bytes > limits.recovery_threshold_bytes {
                    self.level = PressureLevel::Normal;
                    Some(Transition::Recovered)
                } else {
                    None
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Reading the volume
// ---------------------------------------------------------------------------

/// Reads the free space of the volume that holds a directory.
pub trait FreeSpaceProbe: Send + Sync + 'static {
    fn probe(&self, dir: &Path) -> io::Result<VolumeSpace>;
}

/// The operating system's view: `GetDiskFreeSpaceExW` on Windows, `statvfs`
/// elsewhere. A data directory that does not exist yet is measured through
/// its nearest existing parent.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProbe;

impl FreeSpaceProbe for SystemProbe {
    fn probe(&self, dir: &Path) -> io::Result<VolumeSpace> {
        let target = nearest_existing_dir(dir)?;
        os_volume_space(&target)
    }
}

fn nearest_existing_dir(dir: &Path) -> io::Result<PathBuf> {
    dir.ancestors()
        .find(|p| !p.as_os_str().is_empty() && p.is_dir())
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no existing directory at or above {}", dir.display()),
            )
        })
}

#[cfg(windows)]
fn os_volume_space(dir: &Path) -> io::Result<VolumeSpace> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    // A UNC directory must end with a backslash; a local one may.
    let mut wide: Vec<u16> = dir.as_os_str().encode_wide().collect();
    if wide.last() != Some(&(b'\\' as u16)) && wide.last() != Some(&(b'/' as u16)) {
        wide.push(b'\\' as u16);
    }
    wide.push(0);

    let mut available = 0u64;
    let mut total = 0u64;
    // SAFETY: `wide` is NUL-terminated and outlives the call; the out
    // pointers refer to live locals.
    unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(wide.as_ptr()),
            Some(std::ptr::addr_of_mut!(available)),
            Some(std::ptr::addr_of_mut!(total)),
            None,
        )
    }
    .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(VolumeSpace {
        available_bytes: available,
        total_bytes: total,
    })
}

#[cfg(unix)]
// The statvfs field types differ between platforms, hence the casts.
#[allow(clippy::unnecessary_cast)]
fn os_volume_space(dir: &Path) -> io::Result<VolumeSpace> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(dir.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    // SAFETY: `statvfs` is plain old data, so all-zero is a valid value.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is NUL-terminated and `st` is a valid out pointer.
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let block = if st.f_frsize > 0 {
        st.f_frsize as u64
    } else {
        st.f_bsize as u64
    };
    Ok(VolumeSpace {
        available_bytes: (st.f_bavail as u64).saturating_mul(block),
        total_bytes: (st.f_blocks as u64).saturating_mul(block),
    })
}

#[cfg(not(any(windows, unix)))]
fn os_volume_space(dir: &Path) -> io::Result<VolumeSpace> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "free space of {} cannot be read on this platform",
            dir.display()
        ),
    ))
}

/// The thresholds that apply to the volume holding `dir` (what the app shows
/// next to the low-disk switch). Blocking.
pub fn thresholds_for_dir(dir: &Path) -> io::Result<Thresholds> {
    let space = SystemProbe.probe(dir)?;
    Ok(ThresholdRule::VolumeScaled.resolve(Some(&space)))
}

// ---------------------------------------------------------------------------
// Monitor
// ---------------------------------------------------------------------------

/// Where the monitor's events and readings go.
pub trait PressureSink: Send + Sync {
    fn disk_space_low(&self, event: &DiskSpaceLowEvent);
    fn disk_space_recovered(&self, event: &DiskSpaceRecoveredEvent);
    fn status(&self, status: &DiskPressureStatus);
}

/// The engine's event bus plus [`latest_status`].
#[derive(Debug, Default, Clone, Copy)]
pub struct EventBusSink;

impl PressureSink for EventBusSink {
    fn disk_space_low(&self, event: &DiskSpaceLowEvent) {
        if let Err(e) = screenpipe_events::send_event(DISK_SPACE_LOW_EVENT, event.clone()) {
            warn!("disk pressure: could not publish {DISK_SPACE_LOW_EVENT}: {e}");
        }
    }

    fn disk_space_recovered(&self, event: &DiskSpaceRecoveredEvent) {
        if let Err(e) = screenpipe_events::send_event(DISK_SPACE_RECOVERED_EVENT, event.clone()) {
            warn!("disk pressure: could not publish {DISK_SPACE_RECOVERED_EVENT}: {e}");
        }
    }

    fn status(&self, status: &DiskPressureStatus) {
        store_status(status);
    }
}

/// Settings of one monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorConfig {
    pub check_interval: Duration,
    pub rule: ThresholdRule,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            check_interval: DEFAULT_CHECK_INTERVAL,
            rule: ThresholdRule::VolumeScaled,
        }
    }
}

/// The synchronous part of the monitor: turns one reading into a status and,
/// on a change of level, an event.
pub struct MonitorCore {
    data_dir: PathBuf,
    rule: ThresholdRule,
    tracker: PressureTracker,
    limits: Thresholds,
    total_bytes: Option<u64>,
    read_failing: bool,
}

impl MonitorCore {
    pub fn new(data_dir: PathBuf, rule: ThresholdRule) -> Self {
        Self {
            limits: rule.resolve(None),
            data_dir,
            rule,
            tracker: PressureTracker::default(),
            total_bytes: None,
            read_failing: false,
        }
    }

    pub fn level(&self) -> PressureLevel {
        self.tracker.level()
    }

    /// Replace the threshold rule; the next reading uses it.
    pub fn set_rule(&mut self, rule: ThresholdRule) {
        self.rule = rule;
    }

    /// Apply one reading (or the error that replaced it).
    pub fn apply(
        &mut self,
        reading: Result<VolumeSpace, String>,
        sink: &dyn PressureSink,
    ) -> Option<Transition> {
        let data_dir = self.data_dir.display().to_string();
        let (transition, available, error) = match reading {
            Ok(space) => {
                if self.read_failing {
                    info!("disk pressure: the volume behind {data_dir} can be read again");
                    self.read_failing = false;
                }
                self.limits = self.rule.resolve(Some(&space));
                self.total_bytes = Some(space.total_bytes);
                let transition = self.tracker.observe(space.available_bytes, self.limits);
                (transition, Some(space.available_bytes), None)
            }
            Err(e) => {
                if self.read_failing {
                    debug!("disk pressure: free space of {data_dir} still unreadable: {e}");
                } else {
                    warn!(
                        "disk pressure: could not resolve the volume behind the data dir {data_dir}: {e}; \
                         keeping the last known state"
                    );
                    self.read_failing = true;
                }
                (None, None, Some(e))
            }
        };

        match (transition, available) {
            (Some(Transition::BecameLow), Some(available_bytes)) => {
                warn!(
                    "disk pressure: {} free on the volume of {data_dir}, at or below the {} threshold",
                    human_bytes(available_bytes),
                    human_bytes(self.limits.threshold_bytes)
                );
                sink.disk_space_low(&DiskSpaceLowEvent {
                    available_bytes,
                    threshold_bytes: self.limits.threshold_bytes,
                    data_dir: data_dir.clone(),
                });
            }
            (Some(Transition::Recovered), Some(available_bytes)) => {
                info!(
                    "disk pressure: {} free on the volume of {data_dir}, past the {} needed to re-arm",
                    human_bytes(available_bytes),
                    human_bytes(self.limits.recovery_threshold_bytes)
                );
                sink.disk_space_recovered(&DiskSpaceRecoveredEvent {
                    available_bytes,
                    threshold_bytes: self.limits.threshold_bytes,
                    recovery_threshold_bytes: self.limits.recovery_threshold_bytes,
                    data_dir: data_dir.clone(),
                });
            }
            _ => {}
        }

        sink.status(&DiskPressureStatus {
            low: self.tracker.level() == PressureLevel::Low,
            available_bytes: available,
            total_bytes: if error.is_some() {
                None
            } else {
                self.total_bytes
            },
            threshold_bytes: self.limits.threshold_bytes,
            recovery_threshold_bytes: self.limits.recovery_threshold_bytes,
            data_dir,
            checked_at: Utc::now(),
            last_error: error,
        });
        transition
    }
}

/// One read on a blocking thread, bounded by [`PROBE_TIMEOUT`]. A read that
/// timed out is kept in `in_flight` and awaited again next time instead of
/// starting another, so a hung volume never piles up threads.
async fn read_volume(
    probe: &Arc<dyn FreeSpaceProbe>,
    dir: &Path,
    in_flight: &mut Option<JoinHandle<io::Result<VolumeSpace>>>,
) -> Result<VolumeSpace, String> {
    let mut handle = match in_flight.take() {
        Some(handle) => handle,
        None => {
            let probe = Arc::clone(probe);
            let dir = dir.to_path_buf();
            tokio::task::spawn_blocking(move || probe.probe(&dir))
        }
    };
    match tokio::time::timeout(PROBE_TIMEOUT, &mut handle).await {
        Ok(Ok(result)) => result.map_err(|e| e.to_string()),
        Ok(Err(join_error)) => Err(format!("the free-space read failed: {join_error}")),
        Err(_) => {
            *in_flight = Some(handle);
            Err(format!(
                "the free-space read did not finish within {}s",
                PROBE_TIMEOUT.as_secs()
            ))
        }
    }
}

/// Run a monitor until `shutdown` is cancelled.
pub async fn run_monitor(
    data_dir: PathBuf,
    config: MonitorConfig,
    probe: Arc<dyn FreeSpaceProbe>,
    sink: Arc<dyn PressureSink>,
    shutdown: CancellationToken,
) {
    info!(
        "disk pressure monitor started for {} (every {}s)",
        data_dir.display(),
        config.check_interval.as_secs()
    );
    let mut core = MonitorCore::new(data_dir.clone(), config.rule);
    let mut in_flight = None;
    loop {
        let reading = tokio::select! {
            _ = shutdown.cancelled() => break,
            reading = read_volume(&probe, &data_dir, &mut in_flight) => reading,
        };
        core.apply(reading, sink.as_ref());
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep(config.check_interval) => {}
        }
    }
    info!("disk pressure monitor stopped for {}", data_dir.display());
}

/// Background-worker entry (see [`crate::core::BACKGROUND_WORKERS`]).
pub fn start_worker(ctx: WorkerContext) -> WorkerFuture {
    Box::pin(run_monitor(
        ctx.data_dir,
        MonitorConfig::default(),
        Arc::new(SystemProbe),
        Arc::new(EventBusSink),
        ctx.shutdown,
    ))
}

/// Bytes as a short human-readable size (`"1.5 GiB"`).
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Seen {
        Low(DiskSpaceLowEvent),
        Recovered(DiskSpaceRecoveredEvent),
    }

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<Seen>>,
        statuses: Mutex<Vec<DiskPressureStatus>>,
    }

    impl RecordingSink {
        fn events(&self) -> Vec<Seen> {
            self.events.lock().unwrap().clone()
        }
        fn statuses(&self) -> Vec<DiskPressureStatus> {
            self.statuses.lock().unwrap().clone()
        }
    }

    impl PressureSink for RecordingSink {
        fn disk_space_low(&self, event: &DiskSpaceLowEvent) {
            self.events.lock().unwrap().push(Seen::Low(event.clone()));
        }
        fn disk_space_recovered(&self, event: &DiskSpaceRecoveredEvent) {
            self.events
                .lock()
                .unwrap()
                .push(Seen::Recovered(event.clone()));
        }
        fn status(&self, status: &DiskPressureStatus) {
            self.statuses.lock().unwrap().push(status.clone());
        }
    }

    /// Hands out scripted readings, then repeats the last one.
    struct ScriptedProbe {
        script: Mutex<VecDeque<io::Result<VolumeSpace>>>,
        last: Mutex<Option<VolumeSpace>>,
    }

    impl ScriptedProbe {
        fn new(available: &[u64]) -> Self {
            Self {
                script: Mutex::new(available.iter().map(|&a| Ok(space(a))).collect()),
                last: Mutex::new(None),
            }
        }
    }

    impl FreeSpaceProbe for ScriptedProbe {
        fn probe(&self, _dir: &Path) -> io::Result<VolumeSpace> {
            match self.script.lock().unwrap().pop_front() {
                Some(Ok(s)) => {
                    *self.last.lock().unwrap() = Some(s);
                    Ok(s)
                }
                Some(Err(e)) => Err(e),
                None => self
                    .last
                    .lock()
                    .unwrap()
                    .ok_or_else(|| io::Error::other("empty script")),
            }
        }
    }

    const TOTAL: u64 = 500 * GIB;

    fn space(available_bytes: u64) -> VolumeSpace {
        VolumeSpace {
            available_bytes,
            total_bytes: TOTAL,
        }
    }

    fn limits() -> Thresholds {
        Thresholds::with_headroom(5 * GIB)
    }

    #[test]
    fn thresholds_scale_with_the_volume_within_bounds() {
        // 2 % of 50 GiB is 1 GiB: the 2 GiB floor applies.
        assert_eq!(Thresholds::for_volume(50 * GIB).threshold_bytes, 2 * GIB);
        // 2 % of 250 GiB is 5 GiB.
        assert_eq!(Thresholds::for_volume(250 * GIB).threshold_bytes, 5 * GIB);
        // 2 % of 2 TiB is ~41 GiB: the 10 GiB cap applies.
        assert_eq!(Thresholds::for_volume(2048 * GIB).threshold_bytes, 10 * GIB);
        // A volume of unknown size uses the floor.
        assert_eq!(Thresholds::for_volume(0).threshold_bytes, 2 * GIB);
        let t = Thresholds::for_volume(250 * GIB);
        assert_eq!(t.recovery_threshold_bytes, t.threshold_bytes + GIB);
        assert_eq!(
            ThresholdRule::VolumeScaled.resolve(None),
            Thresholds::floor()
        );
        assert_eq!(
            Thresholds::with_headroom(u64::MAX).recovery_threshold_bytes,
            u64::MAX
        );
    }

    #[test]
    fn low_fires_at_the_threshold_inclusive() {
        let mut t = PressureTracker::default();
        assert_eq!(t.observe(5 * GIB + 1, limits()), None);
        assert_eq!(t.level(), PressureLevel::Normal);
        assert_eq!(t.observe(5 * GIB, limits()), Some(Transition::BecameLow));
        assert_eq!(t.level(), PressureLevel::Low);
    }

    #[test]
    fn hovering_around_the_threshold_does_not_flap() {
        let mut t = PressureTracker::default();
        assert_eq!(t.observe(4 * GIB, limits()), Some(Transition::BecameLow));
        // Back above the threshold but inside the headroom: still low.
        for available in [5 * GIB + 1, 4 * GIB, 6 * GIB - 1, 6 * GIB, 5 * GIB] {
            assert_eq!(t.observe(available, limits()), None, "{available}");
            assert_eq!(t.level(), PressureLevel::Low);
        }
        assert_eq!(
            t.observe(6 * GIB + 1, limits()),
            Some(Transition::Recovered)
        );
        // And back inside the band from above: still normal.
        assert_eq!(t.observe(5 * GIB + 1, limits()), None);
        assert_eq!(t.level(), PressureLevel::Normal);
    }

    #[test]
    fn a_volume_already_low_at_start_fires_once() {
        let mut t = PressureTracker::default();
        assert_eq!(t.observe(GIB, limits()), Some(Transition::BecameLow));
        assert_eq!(t.observe(GIB, limits()), None);
    }

    #[test]
    fn core_reports_each_incident_once_with_its_figures() {
        let sink = RecordingSink::default();
        let mut core = MonitorCore::new(PathBuf::from("/data"), ThresholdRule::Fixed(limits()));
        for available in [
            20 * GIB,
            5 * GIB,
            3 * GIB,
            5 * GIB + 10,
            7 * GIB,
            7 * GIB,
            2 * GIB,
        ] {
            core.apply(Ok(space(available)), &sink);
        }
        let data_dir = PathBuf::from("/data").display().to_string();
        assert_eq!(
            sink.events(),
            vec![
                Seen::Low(DiskSpaceLowEvent {
                    available_bytes: 5 * GIB,
                    threshold_bytes: 5 * GIB,
                    data_dir: data_dir.clone(),
                }),
                Seen::Recovered(DiskSpaceRecoveredEvent {
                    available_bytes: 7 * GIB,
                    threshold_bytes: 5 * GIB,
                    recovery_threshold_bytes: 6 * GIB,
                    data_dir: data_dir.clone(),
                }),
                Seen::Low(DiskSpaceLowEvent {
                    available_bytes: 2 * GIB,
                    threshold_bytes: 5 * GIB,
                    data_dir,
                }),
            ]
        );
        let lows: Vec<bool> = sink.statuses().iter().map(|s| s.low).collect();
        assert_eq!(lows, vec![false, true, true, true, false, false, true]);
        let last = sink.statuses().pop().unwrap();
        assert_eq!(last.available_bytes, Some(2 * GIB));
        assert_eq!(last.total_bytes, Some(TOTAL));
        assert_eq!(last.last_error, None);
    }

    #[test]
    fn a_failed_read_keeps_the_level_and_says_why() {
        let sink = RecordingSink::default();
        let mut core = MonitorCore::new(PathBuf::from("/data"), ThresholdRule::Fixed(limits()));
        core.apply(Ok(space(GIB)), &sink);
        core.apply(Err("drive unplugged".into()), &sink);
        core.apply(Err("drive unplugged".into()), &sink);
        assert_eq!(core.level(), PressureLevel::Low);
        let status = sink.statuses().pop().unwrap();
        assert!(status.low);
        assert_eq!(status.available_bytes, None);
        assert_eq!(status.last_error.as_deref(), Some("drive unplugged"));
        // Only the first reading changed the level.
        assert_eq!(sink.events().len(), 1);
        // A read that works again carries on from the same level.
        core.apply(Ok(space(10 * GIB)), &sink);
        assert!(matches!(sink.events().last(), Some(Seen::Recovered(_))));
    }

    #[test]
    fn volume_scaled_thresholds_follow_the_volume_read() {
        let sink = RecordingSink::default();
        let mut core = MonitorCore::new(PathBuf::from("/data"), ThresholdRule::VolumeScaled);
        core.apply(
            Ok(VolumeSpace {
                available_bytes: 4 * GIB,
                total_bytes: 250 * GIB,
            }),
            &sink,
        );
        let status = sink.statuses().pop().unwrap();
        assert_eq!(status.threshold_bytes, 5 * GIB);
        assert_eq!(status.recovery_threshold_bytes, 6 * GIB);
        assert!(status.low);
    }

    #[test]
    fn the_system_probe_reads_a_real_directory() {
        let dir = tempfile::tempdir().unwrap();
        let space = SystemProbe.probe(dir.path()).unwrap();
        assert!(space.total_bytes > 0);
        assert!(space.available_bytes <= space.total_bytes);
        // A data dir that does not exist yet is measured through its parent.
        let missing = dir.path().join("not").join("created").join("yet");
        let via_parent = SystemProbe.probe(&missing).unwrap();
        assert_eq!(via_parent.total_bytes, space.total_bytes);
        assert!(thresholds_for_dir(dir.path()).unwrap().threshold_bytes >= MIN_THRESHOLD_BYTES);
    }

    /// The acceptance flow on a real temp data dir: a threshold above the
    /// free space starts an incident, a threshold far below it ends it.
    #[test]
    fn a_test_threshold_above_free_space_fires_and_lowering_it_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let sink = RecordingSink::default();
        let free = SystemProbe.probe(dir.path()).unwrap().available_bytes;
        let mut core = MonitorCore::new(
            dir.path().to_path_buf(),
            ThresholdRule::Fixed(Thresholds::with_headroom(free.saturating_mul(2).max(GIB))),
        );
        assert_eq!(
            core.apply(
                SystemProbe.probe(dir.path()).map_err(|e| e.to_string()),
                &sink
            ),
            Some(Transition::BecameLow)
        );
        core.set_rule(ThresholdRule::Fixed(Thresholds {
            threshold_bytes: 0,
            recovery_threshold_bytes: 1,
        }));
        assert_eq!(
            core.apply(
                SystemProbe.probe(dir.path()).map_err(|e| e.to_string()),
                &sink
            ),
            Some(Transition::Recovered)
        );
        let events = sink.events();
        assert!(
            matches!(&events[0], Seen::Low(e) if e.data_dir == dir.path().display().to_string())
        );
        assert!(matches!(&events[1], Seen::Recovered(_)));
    }

    #[tokio::test]
    async fn the_monitor_loop_publishes_and_stops_on_shutdown() {
        let sink = Arc::new(RecordingSink::default());
        let probe: Arc<dyn FreeSpaceProbe> = Arc::new(ScriptedProbe::new(&[
            20 * GIB,
            4 * GIB,
            5 * GIB + 1,
            9 * GIB,
        ]));
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(run_monitor(
            PathBuf::from("/data"),
            MonitorConfig {
                check_interval: Duration::from_millis(5),
                rule: ThresholdRule::Fixed(limits()),
            },
            probe,
            sink.clone() as Arc<dyn PressureSink>,
            shutdown.clone(),
        ));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while sink.statuses().len() < 6 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("monitor stops on shutdown")
            .unwrap();
        let events = sink.events();
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(matches!(events[0], Seen::Low(_)));
        assert!(matches!(events[1], Seen::Recovered(_)));
    }

    /// Blocks until the test lets go of the sender.
    struct HungProbe(Mutex<std::sync::mpsc::Receiver<()>>);
    impl FreeSpaceProbe for HungProbe {
        fn probe(&self, _dir: &Path) -> io::Result<VolumeSpace> {
            let _ = self.0.lock().unwrap().recv();
            Err(io::Error::other("released"))
        }
    }

    #[tokio::test]
    async fn shutdown_does_not_wait_for_a_hung_read() {
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let sink = Arc::new(RecordingSink::default());
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(run_monitor(
            PathBuf::from("/data"),
            MonitorConfig::default(),
            Arc::new(HungProbe(Mutex::new(hold))),
            sink.clone() as Arc<dyn PressureSink>,
            shutdown.clone(),
        ));
        tokio::time::sleep(Duration::from_millis(50)).await;
        shutdown.cancel();
        let stopped = tokio::time::timeout(Duration::from_secs(2), task).await;
        drop(release);
        stopped.expect("monitor stops while a read hangs").unwrap();
        assert!(sink.events().is_empty());
        assert!(sink.statuses().is_empty());
    }

    #[test]
    fn events_keep_the_wire_names() {
        let low = serde_json::to_value(DiskSpaceLowEvent {
            available_bytes: 1,
            threshold_bytes: 2,
            data_dir: "d".into(),
        })
        .unwrap();
        assert_eq!(
            low,
            serde_json::json!({"available_bytes": 1, "threshold_bytes": 2, "data_dir": "d"})
        );
        let recovered: DiskSpaceRecoveredEvent = serde_json::from_value(serde_json::json!({
            "available_bytes": 9, "recovery_threshold_bytes": 3, "data_dir": "d"
        }))
        .unwrap();
        assert_eq!(recovered.recovery_threshold_bytes, 3);
        assert_eq!(recovered.threshold_bytes, 0);
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(3 * GIB / 2), "1.5 GiB");
        assert_eq!(human_bytes(2 * 1024 * GIB), "2.0 TiB");
    }

    #[test]
    fn the_worker_is_registered() {
        assert!(crate::core::BACKGROUND_WORKERS
            .iter()
            .any(|w| w.name == WORKER_NAME));
    }
}
