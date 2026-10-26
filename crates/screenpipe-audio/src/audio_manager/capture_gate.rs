// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! `audioCaptureMode` bookkeeping for the audio manager.
//!
//! In `always` mode this is inert: every device may open and every chunk is
//! kept. In `meetings-only` mode it decides three things:
//!
//! * whether a device may open now (only while a meeting is confirmed);
//! * whether a recorded chunk belongs to a meeting (chunks recorded while no
//!   meeting was confirmed are dropped before they are written anywhere);
//! * when the streams opened for a meeting ("session devices") must be closed
//!   again (a grace period after the meeting ends, so a meeting that ends and
//!   restarts at once, such as a room change, keeps its streams).
//!
//! The state machine is plain data driven by explicit timestamps, so it is
//! tested without devices, clocks or an async runtime. [`CaptureGate`] wraps
//! it in a mutex for the manager.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use screenpipe_config::AudioCaptureMode;

/// Seconds the streams stay open after a meeting ends before they are
/// released (the spec infers 30 s).
pub const SESSION_RELEASE_GRACE_SECS: u64 = 30;

/// How many ended meeting windows are remembered for chunk attribution. A
/// chunk is at most a few minutes long, so only the latest few matter.
const ENDED_WINDOWS_KEPT: usize = 4;

/// A meeting the gate treats as running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmedMeeting {
    /// Database id, when the announcement carried one.
    pub id: Option<i64>,
    /// Epoch seconds at which the gate learned about it.
    pub since: u64,
}

/// Whether a device may be opened right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartGate {
    /// The device may open. `session` is true when it is opened for the
    /// current meeting in `meetings-only` mode.
    Open { session: bool },
    /// `meetings-only` and no meeting is confirmed.
    WaitingForMeeting,
}

/// Outcome of a meeting announcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateTransition {
    /// Nothing changed (wrong mode, duplicate, or about another meeting).
    Unchanged,
    /// A meeting is now confirmed; the configured devices should open.
    MeetingStarted,
    /// The confirmed meeting ended; streams are released after the grace.
    MeetingEnded,
}

/// What to do with a recorded chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkVerdict {
    Keep,
    /// `meetings-only` and the chunk does not overlap any confirmed meeting.
    Drop,
}

/// The gate's state. All times are epoch seconds supplied by the caller.
#[derive(Debug, Clone)]
pub struct CaptureGateState {
    mode: AudioCaptureMode,
    meeting: Option<ConfirmedMeeting>,
    /// `(started, ended)` of recently ended meetings, newest last.
    ended: VecDeque<(u64, u64)>,
    /// Devices opened for the current meeting. Only the release path clears
    /// them; the stale-stream cleaner leaves them alone.
    session_devices: BTreeSet<String>,
    /// Session inputs that failed to open during the current meeting, so the
    /// failure and the recovery are each reported once.
    failed_session_inputs: BTreeSet<String>,
}

impl CaptureGateState {
    pub fn new(mode: AudioCaptureMode) -> Self {
        Self {
            mode,
            meeting: None,
            ended: VecDeque::new(),
            session_devices: BTreeSet::new(),
            failed_session_inputs: BTreeSet::new(),
        }
    }

    pub fn mode(&self) -> AudioCaptureMode {
        self.mode
    }

    fn meetings_only(&self) -> bool {
        self.mode == AudioCaptureMode::MeetingsOnly
    }

    pub fn confirmed_meeting(&self) -> Option<ConfirmedMeeting> {
        self.meeting
    }

    /// `meetings-only` with no confirmed meeting (including the release grace).
    pub fn waiting_for_meeting(&self) -> bool {
        self.meetings_only() && self.meeting.is_none()
    }

    /// A meeting was announced. `id` is the meeting's database id when known.
    pub fn meeting_started(&mut self, id: Option<i64>, now: u64) -> GateTransition {
        if !self.meetings_only() {
            return GateTransition::Unchanged;
        }
        match self.meeting {
            Some(current) if current.id == id || id.is_none() => GateTransition::Unchanged,
            Some(current) if current.id.is_none() => {
                // The same meeting, now announced with its id.
                self.meeting = Some(ConfirmedMeeting {
                    id,
                    since: current.since,
                });
                GateTransition::Unchanged
            }
            Some(current) => {
                // A different meeting replaced the confirmed one without an
                // end announcement: close the old window, keep the streams.
                self.push_ended(current.since, now);
                self.meeting = Some(ConfirmedMeeting { id, since: now });
                GateTransition::Unchanged
            }
            None => {
                self.meeting = Some(ConfirmedMeeting { id, since: now });
                GateTransition::MeetingStarted
            }
        }
    }

    /// A meeting ended. An announcement about a meeting other than the
    /// confirmed one is ignored; `None` ends whatever is confirmed.
    pub fn meeting_ended(&mut self, id: Option<i64>, now: u64) -> GateTransition {
        let Some(current) = self.meeting else {
            return GateTransition::Unchanged;
        };
        if let (Some(ended), Some(confirmed)) = (id, current.id) {
            if ended != confirmed {
                return GateTransition::Unchanged;
            }
        }
        self.push_ended(current.since, now);
        self.meeting = None;
        self.failed_session_inputs.clear();
        GateTransition::MeetingEnded
    }

    fn push_ended(&mut self, started: u64, ended: u64) {
        self.ended.push_back((started, ended.max(started)));
        while self.ended.len() > ENDED_WINDOWS_KEPT {
            self.ended.pop_front();
        }
    }

    /// Whether a device may open now.
    pub fn start_gate(&self) -> StartGate {
        if !self.meetings_only() {
            return StartGate::Open { session: false };
        }
        if self.meeting.is_some() {
            StartGate::Open { session: true }
        } else {
            StartGate::WaitingForMeeting
        }
    }

    /// Record a device that finished opening for a meeting. Returns false
    /// when no meeting is confirmed any more (it ended while the device was
    /// opening): the caller must close the device again. A meeting that
    /// replaced the one the open began for still wants the device.
    pub fn mark_session_device(&mut self, device: &str) -> bool {
        if self.meeting.is_none() {
            return false;
        }
        self.session_devices.insert(device.to_string());
        true
    }

    pub fn is_session_device(&self, device: &str) -> bool {
        self.session_devices.contains(device)
    }

    /// Forget every session device (they are being closed).
    pub fn take_session_devices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.session_devices).into_iter().collect()
    }

    /// True when the streams should be closed: `meetings-only`, no meeting,
    /// and the grace after the last meeting has passed.
    pub fn release_due(&self, now: u64) -> bool {
        if !self.waiting_for_meeting() {
            return false;
        }
        match self.ended.back() {
            Some(&(_, ended)) => now >= ended.saturating_add(SESSION_RELEASE_GRACE_SECS),
            None => true,
        }
    }

    /// Whether a chunk recorded over `[start, end]` (epoch seconds) is kept.
    pub fn chunk_verdict(&self, start: u64, end: u64) -> ChunkVerdict {
        if !self.meetings_only() {
            return ChunkVerdict::Keep;
        }
        let end = end.max(start);
        let overlaps = |from: u64, to: Option<u64>| -> bool {
            // A chunk overlaps when it ends after the meeting began and
            // starts before the meeting ended (an open meeting never ends).
            end >= from && to.is_none_or(|to| start <= to)
        };
        if let Some(m) = self.meeting {
            if overlaps(m.since, None) {
                return ChunkVerdict::Keep;
            }
        }
        if self
            .ended
            .iter()
            .any(|&(from, to)| overlaps(from, Some(to)))
        {
            return ChunkVerdict::Keep;
        }
        ChunkVerdict::Drop
    }

    /// A session input failed to open. True the first time per meeting, so
    /// one failure is reported once however often the monitor retries.
    pub fn note_session_input_failed(&mut self, device: &str) -> bool {
        self.meeting.is_some() && self.failed_session_inputs.insert(device.to_string())
    }

    /// A session input opened. True when it had failed earlier in this
    /// meeting, so the recovery is reported.
    pub fn note_session_input_opened(&mut self, device: &str) -> bool {
        self.failed_session_inputs.remove(device)
    }
}

/// Thread-safe wrapper used by the audio manager. Every method takes the
/// lock briefly and never across an await.
#[derive(Debug)]
pub struct CaptureGate {
    state: Mutex<CaptureGateState>,
}

pub(crate) fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl CaptureGate {
    pub fn new(mode: AudioCaptureMode) -> Self {
        Self {
            state: Mutex::new(CaptureGateState::new(mode)),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut CaptureGateState) -> R) -> R {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    /// Start over in `mode` (new options applied while capture was stopped).
    pub fn reset(&self, mode: AudioCaptureMode) {
        self.with(|s| *s = CaptureGateState::new(mode));
    }

    pub fn mode(&self) -> AudioCaptureMode {
        self.with(|s| s.mode())
    }

    pub fn waiting_for_meeting(&self) -> bool {
        self.with(|s| s.waiting_for_meeting())
    }

    pub fn confirmed_meeting(&self) -> Option<ConfirmedMeeting> {
        self.with(|s| s.confirmed_meeting())
    }

    pub fn meeting_started(&self, id: Option<i64>) -> GateTransition {
        let now = now_epoch_secs();
        self.with(|s| s.meeting_started(id, now))
    }

    pub fn meeting_ended(&self, id: Option<i64>) -> GateTransition {
        let now = now_epoch_secs();
        self.with(|s| s.meeting_ended(id, now))
    }

    pub fn start_gate(&self) -> StartGate {
        self.with(|s| s.start_gate())
    }

    pub fn mark_session_device(&self, device: &str) -> bool {
        self.with(|s| s.mark_session_device(device))
    }

    pub fn is_session_device(&self, device: &str) -> bool {
        self.with(|s| s.is_session_device(device))
    }

    pub fn take_session_devices(&self) -> Vec<String> {
        self.with(|s| s.take_session_devices())
    }

    pub fn release_due(&self) -> bool {
        let now = now_epoch_secs();
        self.with(|s| s.release_due(now))
    }

    pub fn chunk_verdict(&self, start: u64, end: u64) -> ChunkVerdict {
        self.with(|s| s.chunk_verdict(start, end))
    }

    pub fn note_session_input_failed(&self, device: &str) -> bool {
        self.with(|s| s.note_session_input_failed(device))
    }

    pub fn note_session_input_opened(&self, device: &str) -> bool {
        self.with(|s| s.note_session_input_opened(device))
    }
}

/// A meeting lifecycle announcement read from the event bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeetingSignal {
    Started(Option<i64>),
    Ended(Option<i64>),
}

fn meeting_id_of(data: &serde_json::Value) -> Option<i64> {
    data.get("meeting_id")
        .or_else(|| data.get("id"))
        .and_then(|v| v.as_i64())
}

/// Interpret a `meeting_started`, `meeting_ended` or `meeting_status_changed`
/// payload. Other names, and status payloads without an `active` flag, give
/// `None`.
pub fn meeting_signal(event_name: &str, data: &serde_json::Value) -> Option<MeetingSignal> {
    match event_name {
        "meeting_started" => Some(MeetingSignal::Started(meeting_id_of(data))),
        "meeting_ended" => Some(MeetingSignal::Ended(meeting_id_of(data))),
        "meeting_status_changed" => {
            let active = data.get("active")?.as_bool()?;
            let id = data.get("active_meeting_id").and_then(|v| v.as_i64());
            Some(if active {
                MeetingSignal::Started(id)
            } else {
                MeetingSignal::Ended(None)
            })
        }
        _ => None,
    }
}

impl MeetingSignal {
    /// Apply this signal to the gate.
    pub fn apply(self, gate: &CaptureGate) -> GateTransition {
        match self {
            MeetingSignal::Started(id) => gate.meeting_started(id),
            MeetingSignal::Ended(id) => gate.meeting_ended(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meetings_only() -> CaptureGateState {
        CaptureGateState::new(AudioCaptureMode::MeetingsOnly)
    }

    #[test]
    fn always_mode_is_inert() {
        let mut g = CaptureGateState::new(AudioCaptureMode::Always);
        assert!(!g.waiting_for_meeting());
        assert_eq!(g.start_gate(), StartGate::Open { session: false });
        assert_eq!(g.chunk_verdict(100, 130), ChunkVerdict::Keep);
        assert!(!g.release_due(10_000));
        assert_eq!(g.meeting_started(Some(1), 50), GateTransition::Unchanged);
        assert_eq!(g.meeting_ended(Some(1), 60), GateTransition::Unchanged);
        assert_eq!(g.chunk_verdict(0, 1), ChunkVerdict::Keep);
    }

    #[test]
    fn meetings_only_waits_and_drops_while_idle() {
        let g = meetings_only();
        assert!(g.waiting_for_meeting());
        assert_eq!(g.start_gate(), StartGate::WaitingForMeeting);
        assert_eq!(g.chunk_verdict(100, 130), ChunkVerdict::Drop);
        // Nothing ever ran: streams opened by accident are released at once.
        assert!(g.release_due(0));
    }

    #[test]
    fn meeting_start_opens_and_end_releases_after_grace() {
        let mut g = meetings_only();
        assert_eq!(g.meeting_started(Some(7), 1_000), GateTransition::MeetingStarted);
        assert!(!g.waiting_for_meeting());
        assert_eq!(g.start_gate(), StartGate::Open { session: true });
        assert!(g.mark_session_device("Mic (input)"));
        assert!(g.is_session_device("Mic (input)"));
        assert!(!g.release_due(1_010));

        assert_eq!(g.meeting_ended(Some(7), 2_000), GateTransition::MeetingEnded);
        assert!(g.waiting_for_meeting());
        assert_eq!(g.start_gate(), StartGate::WaitingForMeeting);
        assert!(!g.release_due(2_000 + SESSION_RELEASE_GRACE_SECS - 1));
        assert!(g.release_due(2_000 + SESSION_RELEASE_GRACE_SECS));
        // Session marks survive until the release path takes them.
        assert!(g.is_session_device("Mic (input)"));
        assert_eq!(g.take_session_devices(), vec!["Mic (input)".to_string()]);
        assert!(!g.is_session_device("Mic (input)"));
    }

    #[test]
    fn duplicate_and_foreign_announcements_are_ignored() {
        let mut g = meetings_only();
        assert_eq!(g.meeting_started(Some(1), 10), GateTransition::MeetingStarted);
        // The same meeting announced again (event plus status change).
        assert_eq!(g.meeting_started(Some(1), 11), GateTransition::Unchanged);
        assert_eq!(g.meeting_started(None, 12), GateTransition::Unchanged);
        // Another meeting's end does not end the confirmed one.
        assert_eq!(g.meeting_ended(Some(99), 13), GateTransition::Unchanged);
        assert!(g.confirmed_meeting().is_some());
        // An end without an id ends whatever runs.
        assert_eq!(g.meeting_ended(None, 14), GateTransition::MeetingEnded);
        assert_eq!(g.meeting_ended(None, 15), GateTransition::Unchanged);
    }

    #[test]
    fn meeting_replaced_without_end_keeps_streams() {
        let mut g = meetings_only();
        g.meeting_started(Some(1), 100);
        assert_eq!(g.meeting_started(Some(2), 200), GateTransition::Unchanged);
        assert_eq!(g.confirmed_meeting().unwrap().id, Some(2));
        // An open that began for meeting 1 is still wanted by meeting 2.
        assert!(g.mark_session_device("Mic (input)"));
        // Audio from either meeting is kept.
        assert_eq!(g.chunk_verdict(150, 180), ChunkVerdict::Keep);
        assert_eq!(g.chunk_verdict(210, 240), ChunkVerdict::Keep);
    }

    #[test]
    fn device_finishing_after_meeting_end_is_refused() {
        let mut g = meetings_only();
        g.meeting_started(Some(3), 100);
        assert_eq!(g.start_gate(), StartGate::Open { session: true });
        g.meeting_ended(Some(3), 105);
        assert!(!g.mark_session_device("Speakers (output)"));
        assert!(!g.is_session_device("Speakers (output)"));
    }

    #[test]
    fn id_arriving_later_names_the_same_meeting() {
        let mut g = meetings_only();
        assert_eq!(g.meeting_started(None, 100), GateTransition::MeetingStarted);
        assert_eq!(g.meeting_started(Some(8), 101), GateTransition::Unchanged);
        let m = g.confirmed_meeting().unwrap();
        assert_eq!((m.id, m.since), (Some(8), 100));
        // Its end is matched by id.
        assert_eq!(g.meeting_ended(Some(8), 200), GateTransition::MeetingEnded);
    }

    #[test]
    fn chunks_are_attributed_by_overlap() {
        let mut g = meetings_only();
        // Before any meeting.
        assert_eq!(g.chunk_verdict(0, 30), ChunkVerdict::Drop);
        g.meeting_started(Some(5), 1_000);
        // Recorded inside the meeting.
        assert_eq!(g.chunk_verdict(1_000, 1_030), ChunkVerdict::Keep);
        g.meeting_ended(Some(5), 1_100);
        // Arrives after the end but started before it: holds meeting audio.
        assert_eq!(g.chunk_verdict(1_090, 1_120), ChunkVerdict::Keep);
        // Recorded entirely during the release grace: no meeting, dropped.
        assert_eq!(g.chunk_verdict(1_101, 1_129), ChunkVerdict::Drop);
        // Recorded entirely before the meeting began.
        assert_eq!(g.chunk_verdict(900, 999), ChunkVerdict::Drop);
        // A chunk that reaches into the meeting's first second is kept.
        assert_eq!(g.chunk_verdict(970, 1_000), ChunkVerdict::Keep);
    }

    #[test]
    fn ended_windows_are_bounded() {
        let mut g = meetings_only();
        for i in 0..10u64 {
            g.meeting_started(Some(i as i64), i * 100);
            g.meeting_ended(Some(i as i64), i * 100 + 50);
        }
        assert_eq!(g.ended.len(), ENDED_WINDOWS_KEPT);
        // The newest windows are kept, the oldest forgotten.
        assert_eq!(g.chunk_verdict(910, 920), ChunkVerdict::Keep);
        assert_eq!(g.chunk_verdict(10, 20), ChunkVerdict::Drop);
    }

    #[test]
    fn session_input_failure_reported_once_per_meeting() {
        let mut g = meetings_only();
        // No meeting: nothing to report.
        assert!(!g.note_session_input_failed("Headset (input)"));
        g.meeting_started(Some(1), 10);
        assert!(g.note_session_input_failed("Headset (input)"));
        assert!(!g.note_session_input_failed("Headset (input)"));
        assert!(g.note_session_input_opened("Headset (input)"));
        assert!(!g.note_session_input_opened("Headset (input)"));
        // A failure in the next meeting is reported again.
        assert!(g.note_session_input_failed("Headset (input)"));
        g.meeting_ended(Some(1), 20);
        g.meeting_started(Some(2), 30);
        assert!(g.note_session_input_failed("Headset (input)"));
    }

    #[test]
    fn signals_parse_from_meeting_payloads() {
        assert_eq!(
            meeting_signal("meeting_started", &json!({"meeting_id": 4, "app": "zoom"})),
            Some(MeetingSignal::Started(Some(4)))
        );
        assert_eq!(
            meeting_signal("meeting_ended", &json!({"meeting_id": 4})),
            Some(MeetingSignal::Ended(Some(4)))
        );
        assert_eq!(
            meeting_signal("meeting_ended", &json!({})),
            Some(MeetingSignal::Ended(None))
        );
        assert_eq!(
            meeting_signal(
                "meeting_status_changed",
                &json!({"active": true, "active_meeting_id": 9})
            ),
            Some(MeetingSignal::Started(Some(9)))
        );
        assert_eq!(
            meeting_signal(
                "meeting_status_changed",
                &json!({"active": false, "active_meeting_id": null})
            ),
            Some(MeetingSignal::Ended(None))
        );
        assert_eq!(meeting_signal("meeting_status_changed", &json!({})), None);
        assert_eq!(meeting_signal("permission_lost", &json!({})), None);
    }

    #[test]
    fn wrapper_reset_switches_mode() {
        let gate = CaptureGate::new(AudioCaptureMode::Always);
        assert!(!gate.waiting_for_meeting());
        gate.reset(AudioCaptureMode::MeetingsOnly);
        assert!(gate.waiting_for_meeting());
        assert_eq!(
            MeetingSignal::Started(Some(1)).apply(&gate),
            GateTransition::MeetingStarted
        );
        assert!(!gate.waiting_for_meeting());
        assert_eq!(
            MeetingSignal::Ended(Some(1)).apply(&gate),
            GateTransition::MeetingEnded
        );
        assert!(gate.waiting_for_meeting());
        assert!(!gate.release_due(), "release waits for the grace period");
    }
}
