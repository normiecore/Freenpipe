// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Audio capture health events.
//!
//! These report problems the device-level events cannot express: the
//! system is playing sound but the output capture hears silence, or a
//! meeting's microphone could not be opened, and the matching recovery.
//! Published on the event bus as `audio_capture_health_<state>`; the
//! desktop app forwards them to the webview and may notify the user.

use serde::{Deserialize, Serialize};

/// What the health event reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCaptureHealthState {
    /// Sound is being rendered but every captured output stream is silent.
    SpeakerSilent,
    /// A previously reported problem cleared.
    Recovered,
    /// A microphone that should record (for example a meeting's input) could
    /// not be opened.
    MicCaptureFailed,
}

impl AudioCaptureHealthState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SpeakerSilent => "speaker_silent",
            Self::Recovered => "recovered",
            Self::MicCaptureFailed => "mic_capture_failed",
        }
    }
}

/// Payload of the `audio_capture_health_*` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioCaptureHealthEvent {
    pub state: AudioCaptureHealthState,
    /// Output endpoints the system is currently playing sound on.
    #[serde(default)]
    pub rendering_endpoints: Vec<String>,
    /// Output devices with an open capture stream.
    #[serde(default)]
    pub captured_outputs: Vec<String>,
    /// Input devices with an open capture stream.
    #[serde(default)]
    pub capturing_inputs: Vec<String>,
    /// The device the event is about, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Short technical reason (an error message, or what recovered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The meeting the capture belonged to, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting_id: Option<i64>,
}

impl AudioCaptureHealthEvent {
    /// An event with empty device lists; fill them in with the setters.
    pub fn new(state: AudioCaptureHealthState) -> Self {
        Self {
            state,
            rendering_endpoints: Vec::new(),
            captured_outputs: Vec::new(),
            capturing_inputs: Vec::new(),
            device: None,
            reason: None,
            meeting_id: None,
        }
    }

    pub fn with_device(mut self, device: impl Into<String>) -> Self {
        self.device = Some(device.into());
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_meeting(mut self, meeting_id: Option<i64>) -> Self {
        self.meeting_id = meeting_id;
        self
    }

    pub fn with_streams(mut self, captured_outputs: Vec<String>, capturing_inputs: Vec<String>) -> Self {
        self.captured_outputs = captured_outputs;
        self.capturing_inputs = capturing_inputs;
        self
    }

    pub fn with_rendering_endpoints(mut self, endpoints: Vec<String>) -> Self {
        self.rendering_endpoints = endpoints;
        self
    }

    /// Event name to publish on the bus.
    pub fn event_name(&self) -> &'static str {
        match self.state {
            AudioCaptureHealthState::SpeakerSilent => "audio_capture_health_speaker_silent",
            AudioCaptureHealthState::Recovered => "audio_capture_health_recovered",
            AudioCaptureHealthState::MicCaptureFailed => "audio_capture_health_mic_capture_failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_per_state() {
        assert_eq!(
            AudioCaptureHealthEvent::new(AudioCaptureHealthState::SpeakerSilent).event_name(),
            "audio_capture_health_speaker_silent"
        );
        assert_eq!(
            AudioCaptureHealthEvent::new(AudioCaptureHealthState::Recovered).event_name(),
            "audio_capture_health_recovered"
        );
        assert_eq!(
            AudioCaptureHealthEvent::new(AudioCaptureHealthState::MicCaptureFailed).event_name(),
            "audio_capture_health_mic_capture_failed"
        );
    }

    #[test]
    fn payload_omits_unset_optionals() {
        let e = AudioCaptureHealthEvent::new(AudioCaptureHealthState::MicCaptureFailed)
            .with_device("Headset (input)")
            .with_streams(vec!["Speakers (output)".into()], Vec::new())
            .with_meeting(Some(7));
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "state": "mic_capture_failed",
                "rendering_endpoints": [],
                "captured_outputs": ["Speakers (output)"],
                "capturing_inputs": [],
                "device": "Headset (input)",
                "meeting_id": 7
            })
        );
        let back: AudioCaptureHealthEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn minimal_payload_parses() {
        let e: AudioCaptureHealthEvent =
            serde_json::from_value(serde_json::json!({"state": "speaker_silent"})).unwrap();
        assert_eq!(e.state, AudioCaptureHealthState::SpeakerSilent);
        assert!(e.rendering_endpoints.is_empty());
        assert_eq!(e.state.as_str(), "speaker_silent");
    }
}
