// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Pure decision logic behind `/health` v2: `vision_reason`, the audio
//! `capture_status`, the startup phase, the audio `runtime` component view
//! and the structured `unhealthy_reasons` list.
//!
//! Everything here works on plain values so it is unit-tested without a
//! running engine; `routes/health.rs` gathers the live inputs.

use oasgen::OaSchema;
use screenpipe_config::AudioCaptureMode;
use screenpipe_screen::MonitorListOutcome;
use serde::{Deserialize, Serialize};

/// Uptime during which "nothing captured yet" is not reported as a failure.
pub const STARTUP_GRACE_SECS: u64 = 120;
/// No transcript for this long, while devices deliver audio, reads as
/// "waiting for someone to speak" rather than "recording speech".
pub const VOICE_QUIET_SECS: u64 = 300;
/// Thresholds kept from base: drop rate over 50 %, DB latency over 10 s.
pub const FRAME_DROP_RATE_LIMIT: f64 = 0.5;
pub const DB_LATENCY_LIMIT_MS: f64 = 10_000.0;

pub const SEVERITY_ERROR: &str = "error";
pub const SEVERITY_WARNING: &str = "warning";

// --- vision_reason --------------------------------------------------------

/// Inputs for [`vision_reason`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisionReasonInputs {
    /// Screen capture turned off (`disableVision` / `--disable-vision`).
    pub vision_disabled: bool,
    /// How the latest monitor enumeration ended.
    pub monitor_outcome: MonitorListOutcome,
    /// `disableScreenshots` is set.
    pub screenshots_disabled_by_config: bool,
    /// The active power profile stores no screenshots (or pauses capture).
    pub screenshots_disabled_by_power: bool,
    /// The legacy `frame_status` value computed for this response.
    pub frame_status: FrameStatus,
    /// Capture attempts continue but DB writes stopped.
    pub vision_write_stalled: bool,
    /// The OCR engine could not be created on its last use.
    pub ocr_unavailable: bool,
}

/// The legacy `frame_status` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameStatus {
    Ok,
    Disabled,
    NotStarted,
    Stale,
}

impl FrameStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Disabled => "disabled",
            Self::NotStarted => "not_started",
            Self::Stale => "stale",
        }
    }

    pub fn is_healthy(self) -> bool {
        matches!(self, Self::Ok | Self::Disabled)
    }
}

/// Decide the legacy `frame_status`. A deliberate pause (locked screen,
/// power-profile pause, DRM content, outside work hours) is `ok`: no frames
/// are expected, so their absence is not a stall.
pub fn frame_status(
    vision_disabled: bool,
    paused: bool,
    last_activity_ts: u64,
    now_ts: u64,
    stale_after_secs: u64,
) -> FrameStatus {
    if vision_disabled {
        FrameStatus::Disabled
    } else if paused {
        FrameStatus::Ok
    } else if last_activity_ts == 0 {
        FrameStatus::NotStarted
    } else if now_ts.saturating_sub(last_activity_ts) < stale_after_secs {
        FrameStatus::Ok
    } else {
        FrameStatus::Stale
    }
}

/// Why screen recording is in the state it is in. First match wins.
pub fn vision_reason(i: &VisionReasonInputs) -> &'static str {
    if i.vision_disabled {
        "disabled_by_setting"
    } else if i.monitor_outcome == MonitorListOutcome::NoMonitors {
        "no_displays_expected"
    } else if i.monitor_outcome == MonitorListOutcome::PermissionDenied {
        "permission_denied"
    } else if i.screenshots_disabled_by_config {
        "screenshots_disabled_by_config"
    } else if i.screenshots_disabled_by_power {
        "screenshots_disabled_by_power_profile"
    } else if i.frame_status == FrameStatus::NotStarted {
        "not_started"
    } else if i.frame_status == FrameStatus::Stale || i.vision_write_stalled {
        "capture_stalled"
    } else if i.ocr_unavailable {
        "ocr_unavailable"
    } else {
        "ok"
    }
}

// --- capture_status (audio) ----------------------------------------------

/// Audio side of what is being captured right now.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, OaSchema)]
pub struct CaptureStatus {
    /// One of `recording`, `waiting_for_voice`, `transcript_pending`,
    /// `transcript_paused`, `audio_stalled`, `audio_not_started`, `mic_paused`,
    /// `no_input_device`, `active_no_data`, `waiting_for_meeting`,
    /// `meeting_detector_unavailable`, `screen_locked`, `disabled`.
    pub status: String,
    /// A short human-readable explanation of `status`.
    pub reason: Option<String>,
    /// Audio devices with a running recording stream.
    pub active_audio_devices: u64,
    /// Audio devices the user paused.
    pub paused_audio_devices: u64,
    /// Audio segments still waiting for background transcription.
    pub pending_transcription_segments: u64,
}

/// Inputs for [`capture_status`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioCaptureInputs {
    /// Audio is off (`disableAudio`, `audioCaptureMode=disabled`).
    pub audio_disabled: bool,
    pub capture_mode: AudioCaptureMode,
    pub screen_locked: bool,
    pub uptime_secs: u64,
    /// Devices with a running recording stream.
    pub active_devices: u64,
    /// Running devices that delivered real audio in the last few seconds.
    pub delivering_devices: u64,
    pub paused_devices: u64,
    /// `Some(in_meeting)` when a meeting detector exists, `None` otherwise.
    pub meeting_state: Option<bool>,
    /// The legacy `audio_status` value computed for this response.
    pub legacy_status: LegacyAudioStatus,
    pub transcription_paused: bool,
    pub pending_transcription_segments: u64,
    /// Seconds since the last transcript was written; `None` if never.
    pub last_transcript_age_secs: Option<u64>,
}

/// The legacy `audio_status` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyAudioStatus {
    Ok,
    Disabled,
    NotStarted,
    ActiveNoData,
    Stale,
}

impl LegacyAudioStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Disabled => "disabled",
            Self::NotStarted => "not_started",
            Self::ActiveNoData => "active_no_data",
            Self::Stale => "stale",
        }
    }

    pub fn is_healthy(self) -> bool {
        matches!(self, Self::Ok | Self::Disabled)
    }
}

/// Classify the audio side. The order puts deliberate states (off, locked,
/// waiting for a meeting, paused by the user) before failures, and failures
/// before the transcript-level states of a working recorder.
pub fn capture_status(i: &AudioCaptureInputs) -> CaptureStatus {
    let (status, reason): (&str, &str) = if i.audio_disabled {
        ("disabled", "audio recording is turned off")
    } else if i.screen_locked && i.delivering_devices == 0 {
        (
            "screen_locked",
            "audio recording is paused while the screen is locked",
        )
    } else if i.capture_mode == AudioCaptureMode::MeetingsOnly
        && i.active_devices == 0
        && i.meeting_state.is_none()
    {
        (
            "meeting_detector_unavailable",
            "audio is set to record meetings only, but meeting detection is not running",
        )
    } else if i.capture_mode == AudioCaptureMode::MeetingsOnly
        && i.active_devices == 0
        && i.meeting_state == Some(false)
    {
        (
            "waiting_for_meeting",
            "audio is set to record meetings only and no meeting is in progress",
        )
    } else if i.active_devices == 0 && i.paused_devices > 0 {
        ("mic_paused", "every audio device is paused")
    } else if i.active_devices == 0 {
        if i.uptime_secs < STARTUP_GRACE_SECS {
            ("audio_not_started", "audio devices are still starting")
        } else {
            ("no_input_device", "no audio device is recording")
        }
    } else {
        match i.legacy_status {
            LegacyAudioStatus::NotStarted => (
                "audio_not_started",
                "audio devices are open but no audio has been captured yet",
            ),
            LegacyAudioStatus::ActiveNoData => (
                "active_no_data",
                "a device is open but delivers only silence; it is being reconnected",
            ),
            LegacyAudioStatus::Stale => (
                "audio_stalled",
                "no audio has arrived from any device recently",
            ),
            LegacyAudioStatus::Ok | LegacyAudioStatus::Disabled => {
                if i.transcription_paused {
                    (
                        "transcript_paused",
                        "audio is recorded; transcription is paused and will catch up later",
                    )
                } else if i.pending_transcription_segments > 0 {
                    (
                        "transcript_pending",
                        "audio is recorded; some segments are waiting for transcription",
                    )
                } else if i.uptime_secs >= VOICE_QUIET_SECS
                    && i.last_transcript_age_secs
                        .is_none_or(|age| age >= VOICE_QUIET_SECS)
                {
                    (
                        "waiting_for_voice",
                        "audio is recorded but no speech has been transcribed recently",
                    )
                } else {
                    ("recording", "audio is being recorded and transcribed")
                }
            }
        }
    };
    CaptureStatus {
        status: status.to_string(),
        reason: Some(reason.to_string()),
        active_audio_devices: i.active_devices,
        paused_audio_devices: i.paused_devices,
        pending_transcription_segments: i.pending_transcription_segments,
    }
}

/// The value reported as `audio_capture_mode`.
pub fn audio_capture_mode_value(
    audio_disabled: bool,
    configured: Option<AudioCaptureMode>,
) -> &'static str {
    if audio_disabled {
        AudioCaptureMode::Disabled.as_str()
    } else {
        configured.unwrap_or(AudioCaptureMode::UNSET).as_str()
    }
}

// --- startup ----------------------------------------------------------------

/// `starting` until every enabled pipeline produced output or the grace
/// period ran out, `running` afterwards.
pub fn startup_phase(
    uptime_secs: u64,
    vision_enabled: bool,
    vision_has_output: bool,
    audio_enabled: bool,
    audio_has_output: bool,
) -> &'static str {
    let produced = (!vision_enabled || vision_has_output) && (!audio_enabled || audio_has_output);
    if produced || uptime_secs >= STARTUP_GRACE_SECS {
        "running"
    } else {
        "starting"
    }
}

// --- runtime (audio model components) -------------------------------------

/// Map a component state to the feature a user would notice missing.
pub fn degraded_feature(component: &str) -> &'static str {
    match component {
        "transcription" => "transcription",
        "vad" => "voice_activity_detection",
        _ => "speaker_identification",
    }
}

/// Fill in a component state nobody reported, from what the metrics show.
/// `worked` = the component has produced output; `errors` = it has failed.
pub fn inferred_component_state(worked: bool, errors: bool) -> &'static str {
    if worked {
        "ok"
    } else if errors {
        "degraded"
    } else {
        "unknown"
    }
}

// --- unhealthy_reasons ------------------------------------------------------

/// One machine-readable problem.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, OaSchema)]
pub struct UnhealthyReason {
    /// Stable snake_case identifier, e.g. `vision_write_stalled`.
    pub code: String,
    /// `vision`, `audio`, `database` or `health`.
    pub component: String,
    /// `error` (data is not being recorded) or `warning` (degraded).
    pub severity: String,
    /// What is wrong, in one sentence.
    pub message: String,
    /// What the user can do about it, when there is something to do.
    pub remediation: Option<String>,
}

impl UnhealthyReason {
    pub fn new(
        code: &str,
        component: &str,
        severity: &str,
        message: impl Into<String>,
        remediation: Option<&str>,
    ) -> Self {
        Self {
            code: code.to_string(),
            component: component.to_string(),
            severity: severity.to_string(),
            message: message.into(),
            remediation: remediation.map(str::to_string),
        }
    }

    /// Served when the health computation overran its budget.
    pub fn snapshot_timeout() -> Self {
        Self::new(
            "health_snapshot_timeout",
            "health",
            SEVERITY_WARNING,
            "the health check took too long; this is the last complete snapshot",
            Some("if this persists, the engine is overloaded or stuck; restart recording"),
        )
    }
}

/// Every signal the reason list is derived from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReasonInputs<'a> {
    pub frame_status: FrameStatus,
    pub vision_reason: &'a str,
    pub vision_write_stalled: bool,
    /// Vision checks are past their warm-up (uptime > 120 s).
    pub vision_warmed_up: bool,
    pub frame_drop_rate: f64,
    pub avg_db_latency_ms: f64,
    pub legacy_audio_status: LegacyAudioStatus,
    pub audio_write_stalled: bool,
    pub audio_warmed_up: bool,
    pub chunks_channel_full: u64,
    /// Transcription backlog that is not explained by a live session.
    pub transcription_backlog: Option<u64>,
    pub write_queue_degraded: bool,
    pub write_queue_consecutive_fatal: u64,
    pub write_queue_consecutive_contention: u64,
}

const REMEDY_SCREEN_PERMISSION: &str =
    "allow screen recording for the app in the operating system's privacy settings";
const REMEDY_RESTART: &str =
    "restart recording; if it keeps happening, export a diagnostics bundle";
const REMEDY_DISK: &str =
    "check free disk space and that the data folder is reachable, then restart recording";

/// Build the structured problem list, most important first within each
/// component (vision, audio, database).
pub fn unhealthy_reasons(i: &ReasonInputs<'_>) -> Vec<UnhealthyReason> {
    let mut out = Vec::new();

    if !i.frame_status.is_healthy() {
        let (code, message, remedy) = match i.vision_reason {
            "permission_denied" => (
                "vision_permission_denied",
                "screen recording permission was not granted",
                Some(REMEDY_SCREEN_PERMISSION),
            ),
            "no_displays_expected" => (
                "vision_no_displays",
                "no display is available to record",
                None,
            ),
            "not_started" => (
                "vision_not_started",
                "screen capture has not produced a frame yet",
                Some(REMEDY_RESTART),
            ),
            _ => (
                "vision_capture_stalled",
                "screen capture stopped producing frames",
                Some(REMEDY_RESTART),
            ),
        };
        out.push(UnhealthyReason::new(
            code,
            "vision",
            SEVERITY_ERROR,
            message,
            remedy,
        ));
    }
    if i.vision_write_stalled {
        out.push(UnhealthyReason::new(
            "vision_write_stalled",
            "vision",
            SEVERITY_ERROR,
            "screen capture is running but its frames are not being saved",
            Some(REMEDY_DISK),
        ));
    }
    if i.vision_warmed_up && i.frame_drop_rate > FRAME_DROP_RATE_LIMIT {
        out.push(UnhealthyReason::new(
            "frame_drop_rate_high",
            "vision",
            SEVERITY_WARNING,
            format!(
                "{:.0}% of captured frames were dropped before being saved",
                i.frame_drop_rate * 100.0
            ),
            None,
        ));
    }
    if i.vision_warmed_up && i.avg_db_latency_ms > DB_LATENCY_LIMIT_MS {
        out.push(UnhealthyReason::new(
            "write_latency_high",
            "vision",
            SEVERITY_WARNING,
            format!(
                "saving a frame takes {:.0} ms on average",
                i.avg_db_latency_ms
            ),
            Some(REMEDY_DISK),
        ));
    }
    if i.vision_reason == "ocr_unavailable" {
        out.push(UnhealthyReason::new(
            "vision_ocr_unavailable",
            "vision",
            SEVERITY_WARNING,
            "text recognition (OCR) is unavailable; apps without accessibility text are recorded without their text",
            Some("install an OCR language pack for your display language"),
        ));
    }

    match i.legacy_audio_status {
        LegacyAudioStatus::NotStarted => out.push(UnhealthyReason::new(
            "audio_not_started",
            "audio",
            SEVERITY_ERROR,
            "audio capture has not delivered any audio yet",
            Some("check that a microphone is connected and allowed for the app"),
        )),
        LegacyAudioStatus::Stale => out.push(UnhealthyReason::new(
            "audio_stalled",
            "audio",
            SEVERITY_ERROR,
            "no audio has arrived from any device recently",
            Some(REMEDY_RESTART),
        )),
        LegacyAudioStatus::ActiveNoData => out.push(UnhealthyReason::new(
            "audio_active_no_data",
            "audio",
            SEVERITY_WARNING,
            "an audio device delivers only silence (another app may hold it); reconnecting",
            None,
        )),
        LegacyAudioStatus::Ok | LegacyAudioStatus::Disabled => {}
    }
    if i.audio_write_stalled {
        out.push(UnhealthyReason::new(
            "transcription_write_stalled",
            "audio",
            SEVERITY_ERROR,
            "audio is recorded but transcripts are not being saved",
            Some(REMEDY_DISK),
        ));
    }
    if i.audio_warmed_up && i.chunks_channel_full > 0 {
        out.push(UnhealthyReason::new(
            "transcription_queue_full",
            "audio",
            SEVERITY_WARNING,
            format!(
                "{} audio chunk(s) were dropped because transcription could not keep up",
                i.chunks_channel_full
            ),
            Some("choose a smaller or faster transcription engine"),
        ));
    }
    if let Some(count) = i.transcription_backlog {
        out.push(UnhealthyReason::new(
            "audio_transcript_pending",
            "audio",
            SEVERITY_WARNING,
            format!(
                "{} audio segment(s) are waiting for background transcription",
                count
            ),
            None,
        ));
    }

    if i.write_queue_degraded {
        out.push(UnhealthyReason::new(
            "database",
            "database",
            SEVERITY_ERROR,
            format!(
                "database writes are failing ({} broken and {} contended batch(es) in a row)",
                i.write_queue_consecutive_fatal, i.write_queue_consecutive_contention
            ),
            Some(REMEDY_DISK),
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vision_ok() -> VisionReasonInputs {
        VisionReasonInputs {
            vision_disabled: false,
            monitor_outcome: MonitorListOutcome::Listed,
            screenshots_disabled_by_config: false,
            screenshots_disabled_by_power: false,
            frame_status: FrameStatus::Ok,
            vision_write_stalled: false,
            ocr_unavailable: false,
        }
    }

    #[test]
    fn vision_reason_follows_the_precedence_order() {
        let mut i = vision_ok();
        assert_eq!(vision_reason(&i), "ok");
        i.ocr_unavailable = true;
        assert_eq!(vision_reason(&i), "ocr_unavailable");
        i.frame_status = FrameStatus::Stale;
        assert_eq!(vision_reason(&i), "capture_stalled");
        i.frame_status = FrameStatus::NotStarted;
        assert_eq!(vision_reason(&i), "not_started");
        i.screenshots_disabled_by_power = true;
        assert_eq!(vision_reason(&i), "screenshots_disabled_by_power_profile");
        i.screenshots_disabled_by_config = true;
        assert_eq!(vision_reason(&i), "screenshots_disabled_by_config");
        i.monitor_outcome = MonitorListOutcome::PermissionDenied;
        assert_eq!(vision_reason(&i), "permission_denied");
        i.monitor_outcome = MonitorListOutcome::NoMonitors;
        assert_eq!(vision_reason(&i), "no_displays_expected");
        i.vision_disabled = true;
        assert_eq!(vision_reason(&i), "disabled_by_setting");
    }

    #[test]
    fn write_stall_alone_is_a_capture_stall() {
        let mut i = vision_ok();
        i.vision_write_stalled = true;
        assert_eq!(vision_reason(&i), "capture_stalled");
    }

    #[test]
    fn frame_status_treats_pauses_as_ok() {
        assert_eq!(frame_status(true, false, 0, 100, 60), FrameStatus::Disabled);
        assert_eq!(frame_status(false, true, 1, 10_000, 60), FrameStatus::Ok);
        assert_eq!(
            frame_status(false, false, 0, 100, 60),
            FrameStatus::NotStarted
        );
        assert_eq!(frame_status(false, false, 100, 159, 60), FrameStatus::Ok);
        assert_eq!(frame_status(false, false, 100, 160, 60), FrameStatus::Stale);
        // A timestamp ahead of the clock is fresh, not a panic.
        assert_eq!(frame_status(false, false, 500, 100, 60), FrameStatus::Ok);
    }

    fn audio_recording() -> AudioCaptureInputs {
        AudioCaptureInputs {
            audio_disabled: false,
            capture_mode: AudioCaptureMode::Always,
            screen_locked: false,
            uptime_secs: 3600,
            active_devices: 2,
            delivering_devices: 2,
            paused_devices: 0,
            meeting_state: Some(false),
            legacy_status: LegacyAudioStatus::Ok,
            transcription_paused: false,
            pending_transcription_segments: 0,
            last_transcript_age_secs: Some(20),
        }
    }

    fn status_of(i: &AudioCaptureInputs) -> String {
        capture_status(i).status
    }

    #[test]
    fn capture_status_reports_a_working_recorder() {
        let i = audio_recording();
        let s = capture_status(&i);
        assert_eq!(s.status, "recording");
        assert_eq!(s.active_audio_devices, 2);
        assert!(s.reason.is_some());
    }

    #[test]
    fn capture_status_deliberate_states() {
        let mut i = audio_recording();
        i.audio_disabled = true;
        assert_eq!(status_of(&i), "disabled");

        let mut i = audio_recording();
        i.screen_locked = true;
        assert_eq!(status_of(&i), "recording", "still delivering while locked");
        i.delivering_devices = 0;
        assert_eq!(status_of(&i), "screen_locked");

        let mut i = audio_recording();
        i.capture_mode = AudioCaptureMode::MeetingsOnly;
        i.active_devices = 0;
        i.delivering_devices = 0;
        assert_eq!(status_of(&i), "waiting_for_meeting");
        i.meeting_state = None;
        assert_eq!(status_of(&i), "meeting_detector_unavailable");

        let mut i = audio_recording();
        i.active_devices = 0;
        i.paused_devices = 1;
        assert_eq!(status_of(&i), "mic_paused");
    }

    #[test]
    fn capture_status_failures() {
        let mut i = audio_recording();
        i.active_devices = 0;
        i.delivering_devices = 0;
        assert_eq!(status_of(&i), "no_input_device");
        i.uptime_secs = 30;
        assert_eq!(status_of(&i), "audio_not_started");

        let mut i = audio_recording();
        i.legacy_status = LegacyAudioStatus::NotStarted;
        assert_eq!(status_of(&i), "audio_not_started");
        i.legacy_status = LegacyAudioStatus::ActiveNoData;
        assert_eq!(status_of(&i), "active_no_data");
        i.legacy_status = LegacyAudioStatus::Stale;
        assert_eq!(status_of(&i), "audio_stalled");
    }

    #[test]
    fn capture_status_transcript_states() {
        let mut i = audio_recording();
        i.pending_transcription_segments = 4;
        let s = capture_status(&i);
        assert_eq!(s.status, "transcript_pending");
        assert_eq!(s.pending_transcription_segments, 4);
        i.transcription_paused = true;
        assert_eq!(status_of(&i), "transcript_paused");

        let mut i = audio_recording();
        i.last_transcript_age_secs = Some(VOICE_QUIET_SECS);
        assert_eq!(status_of(&i), "waiting_for_voice");
        i.last_transcript_age_secs = None;
        assert_eq!(status_of(&i), "waiting_for_voice");
        i.uptime_secs = 60;
        assert_eq!(status_of(&i), "recording", "too early to call it quiet");
    }

    #[test]
    fn capture_mode_value_reports_disabled_when_audio_is_off() {
        assert_eq!(
            audio_capture_mode_value(true, Some(AudioCaptureMode::Always)),
            "disabled"
        );
        assert_eq!(
            audio_capture_mode_value(false, Some(AudioCaptureMode::MeetingsOnly)),
            "meetings-only"
        );
        assert_eq!(audio_capture_mode_value(false, None), "always");
    }

    #[test]
    fn startup_phase_ends_on_output_or_grace() {
        assert_eq!(startup_phase(10, true, false, true, false), "starting");
        assert_eq!(startup_phase(10, true, true, true, false), "starting");
        assert_eq!(startup_phase(10, true, true, true, true), "running");
        assert_eq!(startup_phase(10, false, false, true, true), "running");
        assert_eq!(startup_phase(10, true, true, false, false), "running");
        assert_eq!(startup_phase(120, true, false, true, false), "running");
    }

    #[test]
    fn inferred_component_states() {
        assert_eq!(inferred_component_state(true, true), "ok");
        assert_eq!(inferred_component_state(false, true), "degraded");
        assert_eq!(inferred_component_state(false, false), "unknown");
        assert_eq!(
            degraded_feature("speaker_embedding"),
            "speaker_identification"
        );
        assert_eq!(degraded_feature("vad"), "voice_activity_detection");
    }

    fn healthy_reasons() -> ReasonInputs<'static> {
        ReasonInputs {
            frame_status: FrameStatus::Ok,
            vision_reason: "ok",
            vision_write_stalled: false,
            vision_warmed_up: true,
            frame_drop_rate: 0.0,
            avg_db_latency_ms: 20.0,
            legacy_audio_status: LegacyAudioStatus::Ok,
            audio_write_stalled: false,
            audio_warmed_up: true,
            chunks_channel_full: 0,
            transcription_backlog: None,
            write_queue_degraded: false,
            write_queue_consecutive_fatal: 0,
            write_queue_consecutive_contention: 0,
        }
    }

    fn codes(i: &ReasonInputs<'_>) -> Vec<String> {
        unhealthy_reasons(i).into_iter().map(|r| r.code).collect()
    }

    #[test]
    fn healthy_engine_has_no_reasons() {
        assert!(unhealthy_reasons(&healthy_reasons()).is_empty());
    }

    #[test]
    fn vision_problems_map_to_codes() {
        let mut i = healthy_reasons();
        i.frame_status = FrameStatus::Stale;
        i.vision_reason = "capture_stalled";
        i.vision_write_stalled = true;
        i.frame_drop_rate = 0.8;
        i.avg_db_latency_ms = 12_000.0;
        assert_eq!(
            codes(&i),
            vec![
                "vision_capture_stalled",
                "vision_write_stalled",
                "frame_drop_rate_high",
                "write_latency_high"
            ]
        );
        let mut i = healthy_reasons();
        i.frame_status = FrameStatus::NotStarted;
        i.vision_reason = "permission_denied";
        let reasons = unhealthy_reasons(&i);
        assert_eq!(reasons[0].code, "vision_permission_denied");
        assert_eq!(reasons[0].severity, "error");
        assert!(reasons[0].remediation.is_some());
    }

    #[test]
    fn warm_up_suppresses_rate_based_reasons() {
        let mut i = healthy_reasons();
        i.vision_warmed_up = false;
        i.audio_warmed_up = false;
        i.frame_drop_rate = 0.9;
        i.chunks_channel_full = 3;
        assert!(codes(&i).is_empty());
    }

    #[test]
    fn audio_and_database_problems_map_to_codes() {
        let mut i = healthy_reasons();
        i.legacy_audio_status = LegacyAudioStatus::Stale;
        i.audio_write_stalled = true;
        i.chunks_channel_full = 2;
        i.transcription_backlog = Some(30);
        i.write_queue_degraded = true;
        i.write_queue_consecutive_fatal = 2;
        assert_eq!(
            codes(&i),
            vec![
                "audio_stalled",
                "transcription_write_stalled",
                "transcription_queue_full",
                "audio_transcript_pending",
                "database"
            ]
        );
        let db = unhealthy_reasons(&i).pop().unwrap();
        assert_eq!(db.component, "database");
        assert!(db.message.contains('2'));
    }

    #[test]
    fn ocr_unavailable_is_a_warning_only() {
        let mut i = healthy_reasons();
        i.vision_reason = "ocr_unavailable";
        let reasons = unhealthy_reasons(&i);
        assert_eq!(reasons.len(), 1);
        assert_eq!(reasons[0].code, "vision_ocr_unavailable");
        assert_eq!(reasons[0].severity, "warning");
    }

    #[test]
    fn reason_serialises_the_five_fields_in_order() {
        let text = serde_json::to_string(&UnhealthyReason::snapshot_timeout()).unwrap();
        let order = ["code", "component", "severity", "message", "remediation"];
        let at: Vec<usize> = order
            .iter()
            .map(|k| text.find(&format!("\"{}\":", k)).unwrap())
            .collect();
        assert!(at.windows(2).all(|w| w[0] < w[1]));
        assert!(text.contains("\"health_snapshot_timeout\""));
    }

    #[test]
    fn capture_status_serialises_its_fields_in_order() {
        let text = serde_json::to_string(&capture_status(&audio_recording())).unwrap();
        let order = [
            "status",
            "reason",
            "active_audio_devices",
            "paused_audio_devices",
            "pending_transcription_segments",
        ];
        let at: Vec<usize> = order
            .iter()
            .map(|k| text.find(&format!("\"{}\":", k)).unwrap())
            .collect();
        assert!(at.windows(2).all(|w| w[0] < w[1]));
    }
}
