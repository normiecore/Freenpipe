// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Recording coverage: how much of the time the user was at the computer was
//! actually recorded.
//!
//! A ticker wakes every 5 s, decides which of four buckets "now" falls into
//! and adds the seconds since its previous tick to that bucket:
//!
//! | bucket          | meaning                                                  |
//! |-----------------|----------------------------------------------------------|
//! | `recording`     | user active and vision frames are landing                |
//! | `active_stalled`| user active but nothing has landed for over two minutes  |
//! | `active_paused` | user active but recording is paused on purpose           |
//! | `idle`          | no input for five minutes, or activity unknown           |
//!
//! "User active" means the UI recorder flushed input events in the last five
//! minutes. The counters cover the process lifetime: they are never reset and
//! never written to disk. `/health` exposes them as `recording_coverage`.
//!
//! The data is local only. It holds second counts and one ratio, with no app
//! names, titles, timestamps or content, and it is never sent anywhere.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use oasgen::OaSchema;
use screenpipe_screen::PipelineMetrics;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::IntervalStream;
use tokio_stream::{Stream, StreamExt};
use tracing::debug;

/// How often the ticker classifies "now".
pub const TICK_INTERVAL: Duration = Duration::from_secs(5);
/// Longest gap between ticks that is still counted. Longer gaps (sleep,
/// hibernation, a debugger stop, a stalled runtime) are dropped entirely.
pub const MAX_COUNTED_GAP_SECS: u64 = 15;
/// Without a UI event for longer than this, the user counts as idle.
pub const IDLE_AFTER_SECS: u64 = 300;
/// A vision write (or dedup skip) at most this old means recording is landing.
pub const FRESH_WRITE_SECS: u64 = 120;
/// Before the first vision write, this much pipeline uptime still counts as
/// recording (startup grace).
pub const STARTUP_GRACE_SECS: f64 = 120.0;
/// How many ticks between the local DEBUG summary lines (60 s).
const SUMMARY_EVERY_TICKS: u64 = 12;

/// Which bucket a counted interval goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageBucket {
    Recording,
    ActiveStalled,
    ActivePaused,
    Idle,
}

impl CoverageBucket {
    fn index(self) -> usize {
        match self {
            Self::Recording => 0,
            Self::ActiveStalled => 1,
            Self::ActivePaused => 2,
            Self::Idle => 3,
        }
    }
}

/// A point-in-time snapshot of recording coverage since the engine started.
/// Counts only: no private data, just wall-clock seconds per state and the
/// share of active time that was recorded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, OaSchema)]
pub struct CoverageSnapshot {
    /// Seconds the user was active and vision recording was landing.
    pub recording_secs: u64,
    /// Seconds the user was active while vision recording was stalled.
    pub active_stalled_secs: u64,
    /// Seconds the user was active while recording was paused on purpose
    /// (work-hours schedule, DRM content on screen).
    pub active_paused_secs: u64,
    /// Seconds the user was idle, or activity was unknown.
    pub idle_secs: u64,
    /// `recording_secs + active_stalled_secs + active_paused_secs`.
    pub active_secs: u64,
    /// `recording_secs / active_secs`, or 0 when there was no active time.
    pub coverage_ratio: f64,
}

/// Everything the classifier looks at, as plain values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoverageInputs {
    /// Wall clock, Unix seconds.
    pub now_unix: u64,
    /// When the UI recorder last flushed events (Unix seconds); `None` or 0
    /// when it never has.
    pub last_ui_event_unix: Option<i64>,
    /// Outside the configured work hours.
    pub schedule_paused: bool,
    /// Capture paused because DRM content is on screen.
    pub drm_paused: bool,
    /// Last vision DB write or dedup skip (Unix seconds); 0 = none yet.
    pub last_vision_write_unix: u64,
    /// Seconds since the vision pipeline metrics were created.
    pub pipeline_uptime_secs: f64,
}

/// Decide which bucket "now" belongs to. First match wins: idle, paused,
/// then recording versus stalled by the age of the last vision write.
pub fn classify(inputs: &CoverageInputs) -> CoverageBucket {
    let last_event = match inputs.last_ui_event_unix {
        Some(ts) if ts > 0 => ts as u64,
        _ => return CoverageBucket::Idle,
    };
    if inputs.now_unix.saturating_sub(last_event) > IDLE_AFTER_SECS {
        return CoverageBucket::Idle;
    }
    if inputs.schedule_paused || inputs.drm_paused {
        return CoverageBucket::ActivePaused;
    }
    if inputs.last_vision_write_unix > 0 {
        if inputs
            .now_unix
            .saturating_sub(inputs.last_vision_write_unix)
            <= FRESH_WRITE_SECS
        {
            CoverageBucket::Recording
        } else {
            CoverageBucket::ActiveStalled
        }
    } else if inputs.pipeline_uptime_secs < STARTUP_GRACE_SECS {
        CoverageBucket::Recording
    } else {
        CoverageBucket::ActiveStalled
    }
}

/// Whole seconds to credit for a tick that came `elapsed` after the previous
/// one: `None` for zero-length bursts (tokio catching up after a stall) and
/// for gaps longer than [`MAX_COUNTED_GAP_SECS`].
pub fn counted_secs(elapsed: Duration) -> Option<u64> {
    let secs = elapsed.as_secs();
    (1..=MAX_COUNTED_GAP_SECS).contains(&secs).then_some(secs)
}

/// The four second counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoverageAccumulator {
    buckets: [u64; 4],
}

impl CoverageAccumulator {
    pub const fn new() -> Self {
        Self { buckets: [0; 4] }
    }

    pub fn add(&mut self, bucket: CoverageBucket, secs: u64) {
        let slot = &mut self.buckets[bucket.index()];
        *slot = slot.saturating_add(secs);
    }

    pub fn snapshot(&self) -> CoverageSnapshot {
        let [recording, stalled, paused, idle] = self.buckets;
        let active = recording.saturating_add(stalled).saturating_add(paused);
        CoverageSnapshot {
            recording_secs: recording,
            active_stalled_secs: stalled,
            active_paused_secs: paused,
            idle_secs: idle,
            active_secs: active,
            coverage_ratio: if active == 0 {
                0.0
            } else {
                recording as f64 / active as f64
            },
        }
    }
}

static COVERAGE: Mutex<CoverageAccumulator> = Mutex::new(CoverageAccumulator::new());

/// Credit `secs` to `bucket` in the process-wide accumulator.
pub fn record(bucket: CoverageBucket, secs: u64) {
    if let Ok(mut acc) = COVERAGE.lock() {
        acc.add(bucket, secs);
    }
}

/// Coverage since the engine started. All zeros if the lock is poisoned.
pub fn snapshot() -> CoverageSnapshot {
    COVERAGE
        .lock()
        .map(|acc| acc.snapshot())
        .unwrap_or_default()
}

/// Remembers when the previous tick happened and turns each new tick into
/// "credit N seconds" or "skip".
#[derive(Debug, Clone, Copy)]
pub struct TickClock {
    previous: Instant,
}

impl TickClock {
    pub fn new(start: Instant) -> Self {
        Self { previous: start }
    }

    /// Seconds to credit for a tick at `now`; the marker always moves on.
    pub fn tick(&mut self, now: Instant) -> Option<u64> {
        let elapsed = now.saturating_duration_since(self.previous);
        self.previous = now;
        counted_secs(elapsed)
    }
}

/// Gather the classifier inputs from the live engine state.
pub fn current_inputs(vision: &PipelineMetrics) -> CoverageInputs {
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    CoverageInputs {
        now_unix,
        last_ui_event_unix: crate::ui_recorder::ui_recorder_status_snapshot()
            .last_event_at
            .map(|t| t.timestamp()),
        schedule_paused: crate::schedule_monitor::schedule_paused(),
        drm_paused: crate::drm_detector::drm_content_paused(),
        last_vision_write_unix: vision
            .last_db_write_ts
            .load(std::sync::atomic::Ordering::Relaxed),
        pipeline_uptime_secs: vision.started_at.elapsed().as_secs_f64(),
    }
}

/// The vision metrics the ticker reads. Replaced when a new server starts in
/// the same process, so a restarted engine is measured against its own
/// pipeline rather than a dead one.
static VISION_SOURCE: LazyLock<Mutex<Option<Arc<PipelineMetrics>>>> =
    LazyLock::new(|| Mutex::new(None));
static TICKER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Clears [`TICKER_RUNNING`] when the ticker task ends (including when its
/// runtime shuts down), so the next server start spawns a fresh one.
struct TickerGuard;

impl Drop for TickerGuard {
    fn drop(&mut self) {
        TICKER_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Start the 5 s coverage ticker for `vision`. Called at HTTP server start,
/// whether or not vision capture is enabled. Only one ticker runs per
/// process; a second call just points it at the new metrics. Must be called
/// inside a tokio runtime.
pub fn start_coverage_ticker(vision: Arc<PipelineMetrics>) {
    start_ticker_with(vision, interval_ticks(TICK_INTERVAL));
}

/// The instants at which a tokio interval of `period` fires, read from
/// tokio's clock when each tick is delivered (so a late tick reports how late
/// it really was). The first item comes at once.
fn interval_ticks(period: Duration) -> impl Stream<Item = Instant> + Send + Unpin + 'static {
    IntervalStream::new(tokio::time::interval(period))
        .map(|_| tokio::time::Instant::now().into_std())
}

/// Point the ticker at `vision` and, when no ticker runs yet, spawn one that
/// credits time at each instant `ticks` yields. Returns the spawned task, or
/// `None` when a ticker was already running (`ticks` is then dropped unread).
fn start_ticker_with<S>(vision: Arc<PipelineMetrics>, ticks: S) -> Option<JoinHandle<()>>
where
    S: Stream<Item = Instant> + Send + Unpin + 'static,
{
    if let Ok(mut source) = VISION_SOURCE.lock() {
        *source = Some(vision);
    }
    if TICKER_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return None;
    }
    // Made before the spawn so the flag is cleared even if the task is
    // dropped before its first poll (a runtime shutting down).
    let guard = TickerGuard;
    Some(tokio::spawn(async move {
        let _guard = guard;
        run_ticker(ticks).await;
    }))
}

/// Credit coverage time at each instant `ticks` yields; the first instant
/// only starts the clock. Ends when the stream ends.
async fn run_ticker<S>(mut ticks: S)
where
    S: Stream<Item = Instant> + Unpin,
{
    let Some(start) = ticks.next().await else {
        return;
    };
    let mut clock = TickClock::new(start);
    let mut count: u64 = 0;
    while let Some(now) = ticks.next().await {
        count = count.wrapping_add(1);
        let Some(secs) = clock.tick(now) else {
            continue;
        };
        let vision = VISION_SOURCE.lock().ok().and_then(|s| s.clone());
        let Some(vision) = vision else {
            continue;
        };
        record(classify(&current_inputs(&vision)), secs);

        if count.is_multiple_of(SUMMARY_EVERY_TICKS) {
            let s = snapshot();
            debug!(
                "recording coverage: ratio={:.3}, recording={}s, stalled={}s, paused={}s, idle={}s",
                s.coverage_ratio,
                s.recording_secs,
                s.active_stalled_secs,
                s.active_paused_secs,
                s.idle_secs
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::wrappers::UnboundedReceiverStream;

    const NOW: u64 = 1_800_000_000;

    fn active_inputs() -> CoverageInputs {
        CoverageInputs {
            now_unix: NOW,
            last_ui_event_unix: Some((NOW - 10) as i64),
            schedule_paused: false,
            drm_paused: false,
            last_vision_write_unix: NOW - 5,
            pipeline_uptime_secs: 3600.0,
        }
    }

    #[test]
    fn no_ui_event_ever_is_idle() {
        let mut i = active_inputs();
        i.last_ui_event_unix = None;
        assert_eq!(classify(&i), CoverageBucket::Idle);
        i.last_ui_event_unix = Some(0);
        assert_eq!(classify(&i), CoverageBucket::Idle);
        i.last_ui_event_unix = Some(-5);
        assert_eq!(classify(&i), CoverageBucket::Idle);
    }

    #[test]
    fn idle_boundary_is_three_hundred_seconds() {
        let mut i = active_inputs();
        i.last_ui_event_unix = Some((NOW - 300) as i64);
        assert_eq!(classify(&i), CoverageBucket::Recording);
        i.last_ui_event_unix = Some((NOW - 301) as i64);
        assert_eq!(classify(&i), CoverageBucket::Idle);
    }

    #[test]
    fn idle_wins_over_pause() {
        let mut i = active_inputs();
        i.last_ui_event_unix = Some((NOW - 1000) as i64);
        i.schedule_paused = true;
        assert_eq!(classify(&i), CoverageBucket::Idle);
    }

    #[test]
    fn schedule_or_drm_pause_is_active_paused() {
        let mut i = active_inputs();
        i.schedule_paused = true;
        assert_eq!(classify(&i), CoverageBucket::ActivePaused);
        let mut i = active_inputs();
        i.drm_paused = true;
        i.last_vision_write_unix = NOW - 10_000;
        assert_eq!(classify(&i), CoverageBucket::ActivePaused);
    }

    #[test]
    fn fresh_write_is_recording_and_old_write_is_stalled() {
        let mut i = active_inputs();
        i.last_vision_write_unix = NOW - 120;
        assert_eq!(classify(&i), CoverageBucket::Recording);
        i.last_vision_write_unix = NOW - 121;
        assert_eq!(classify(&i), CoverageBucket::ActiveStalled);
        // A write stamped in the future (clock change) is still fresh.
        i.last_vision_write_unix = NOW + 30;
        assert_eq!(classify(&i), CoverageBucket::Recording);
    }

    #[test]
    fn no_write_yet_uses_the_startup_grace() {
        let mut i = active_inputs();
        i.last_vision_write_unix = 0;
        i.pipeline_uptime_secs = 119.9;
        assert_eq!(classify(&i), CoverageBucket::Recording);
        i.pipeline_uptime_secs = 120.0;
        assert_eq!(classify(&i), CoverageBucket::ActiveStalled);
    }

    #[test]
    fn tick_window_accepts_one_to_fifteen_seconds() {
        assert_eq!(counted_secs(Duration::from_millis(0)), None);
        assert_eq!(counted_secs(Duration::from_millis(999)), None);
        assert_eq!(counted_secs(Duration::from_secs(1)), Some(1));
        assert_eq!(counted_secs(Duration::from_millis(5_900)), Some(5));
        assert_eq!(counted_secs(Duration::from_secs(15)), Some(15));
        assert_eq!(counted_secs(Duration::from_millis(15_999)), Some(15));
        assert_eq!(counted_secs(Duration::from_secs(16)), None);
        assert_eq!(counted_secs(Duration::from_secs(3600)), None);
    }

    #[test]
    fn tick_clock_skips_gaps_but_always_moves_on() {
        let start = Instant::now();
        let mut clock = TickClock::new(start);
        assert_eq!(clock.tick(start + Duration::from_secs(5)), Some(5));
        // A sleep: the long gap is dropped...
        assert_eq!(clock.tick(start + Duration::from_secs(605)), None);
        // ...and the burst tick right after it is too.
        assert_eq!(clock.tick(start + Duration::from_secs(605)), None);
        // Normal ticks resume from the new marker.
        assert_eq!(clock.tick(start + Duration::from_secs(610)), Some(5));
    }

    #[test]
    fn accumulator_snapshot_sums_active_time_and_ratio() {
        let mut acc = CoverageAccumulator::new();
        assert_eq!(acc.snapshot(), CoverageSnapshot::default());
        acc.add(CoverageBucket::Recording, 30);
        acc.add(CoverageBucket::ActiveStalled, 5);
        acc.add(CoverageBucket::ActivePaused, 5);
        acc.add(CoverageBucket::Idle, 100);
        let s = acc.snapshot();
        assert_eq!(s.recording_secs, 30);
        assert_eq!(s.active_stalled_secs, 5);
        assert_eq!(s.active_paused_secs, 5);
        assert_eq!(s.idle_secs, 100);
        assert_eq!(s.active_secs, 40);
        assert!((s.coverage_ratio - 0.75).abs() < 1e-12);
    }

    #[test]
    fn only_idle_time_gives_zero_ratio() {
        let mut acc = CoverageAccumulator::new();
        acc.add(CoverageBucket::Idle, 50);
        let s = acc.snapshot();
        assert_eq!(s.active_secs, 0);
        assert_eq!(s.coverage_ratio, 0.0);
    }

    #[test]
    fn snapshot_fields_serialise_in_target_order() {
        let text = serde_json::to_string(&CoverageSnapshot::default()).unwrap();
        let order = [
            "recording_secs",
            "active_stalled_secs",
            "active_paused_secs",
            "idle_secs",
            "active_secs",
            "coverage_ratio",
        ];
        let positions: Vec<usize> = order
            .iter()
            .map(|k| text.find(&format!("\"{}\":", k)).expect(k))
            .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{}", text);
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value.as_object().unwrap().len(), order.len());
    }

    /// Ticks are fed as synthetic instants, so no real time passes and the
    /// credited seconds are exact on any runner.
    #[tokio::test]
    async fn ticker_credits_whole_seconds_per_tick_and_runs_once() {
        let before = snapshot();
        let start = Instant::now();
        let (ticks, rx) = tokio::sync::mpsc::unbounded_channel::<Instant>();
        let ticker = start_ticker_with(
            Arc::new(PipelineMetrics::new()),
            UnboundedReceiverStream::new(rx),
        )
        .expect("the first start spawns the ticker");

        // A second start while it runs only re-points it: its ticks are
        // dropped unread, so time is never counted twice.
        let (second_ticks, second_rx) = tokio::sync::mpsc::unbounded_channel::<Instant>();
        assert!(start_ticker_with(
            Arc::new(PipelineMetrics::new()),
            UnboundedReceiverStream::new(second_rx),
        )
        .is_none());
        assert!(second_ticks.is_closed());

        // Start, three ticks 1.05 s apart (1 s each), a 20 s gap (dropped),
        // then a tick 2.5 s later (2 s).
        for ms in [0, 1_050, 2_100, 3_150, 23_150, 25_650] {
            ticks.send(start + Duration::from_millis(ms)).unwrap();
        }
        drop(ticks);
        ticker.await.unwrap();
        let after = snapshot();

        // Another test adds to `active_paused` directly, so only the other
        // buckets give an exact figure; the ticker's own credits may land in
        // any bucket, so the total is a lower bound.
        let unpaused =
            |s: &CoverageSnapshot| s.recording_secs + s.active_stalled_secs + s.idle_secs;
        let gained =
            (after.active_secs + after.idle_secs) - (before.active_secs + before.idle_secs);
        assert!(gained >= 5, "gained {} ({:?})", gained, after);
        assert!(unpaused(&after) - unpaused(&before) <= 5, "{:?}", after);

        // The finished ticker released its slot, so a later server start in
        // the same process gets a fresh one.
        let (late_ticks, late_rx) = tokio::sync::mpsc::unbounded_channel::<Instant>();
        let late = start_ticker_with(
            Arc::new(PipelineMetrics::new()),
            UnboundedReceiverStream::new(late_rx),
        )
        .expect("a stopped ticker is replaced");
        drop(late_ticks);
        late.await.unwrap();
    }

    #[test]
    fn process_accumulator_records_through_the_public_api() {
        let before = snapshot();
        record(CoverageBucket::ActivePaused, 7);
        let after = snapshot();
        assert!(after.active_paused_secs >= before.active_paused_secs + 7);
    }
}
