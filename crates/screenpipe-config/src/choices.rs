// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Value sets for string-typed recording settings.
//!
//! The settings store keeps these as plain strings so that a value written by
//! a newer app version never stops an older one from loading the store.
//! Code that acts on a setting parses it here; an unrecognised value falls
//! back to the documented default instead of failing.

/// `audioCaptureMode`: when audio devices are recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCaptureMode {
    /// Record the configured devices all the time (the historical behaviour).
    Always,
    /// Open audio devices only while a confirmed meeting is running.
    MeetingsOnly,
    /// Never start audio capture.
    Disabled,
}

impl AudioCaptureMode {
    /// Value written for new installs of the desktop app.
    pub const NEW_INSTALL: AudioCaptureMode = AudioCaptureMode::MeetingsOnly;
    /// Value assumed when a store has no `audioCaptureMode` at all.
    pub const UNSET: AudioCaptureMode = AudioCaptureMode::Always;

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "always" => Some(Self::Always),
            "meetings-only" | "meetings_only" => Some(Self::MeetingsOnly),
            "disabled" | "off" => Some(Self::Disabled),
            _ => None,
        }
    }

    /// Parse a stored value; unknown or empty values mean [`Self::UNSET`].
    pub fn from_setting(value: &str) -> Self {
        Self::parse(value).unwrap_or(Self::UNSET)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::MeetingsOnly => "meetings-only",
            Self::Disabled => "disabled",
        }
    }
}

/// Operating systems the AEC choice is validated against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPlatform {
    Windows,
    Macos,
    Linux,
    Other,
}

impl SettingsPlatform {
    /// The platform this binary was built for.
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }

    /// Parse the frontend's platform names (`windows`, `macos`, `linux`).
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "windows" | "win32" => Self::Windows,
            "macos" | "darwin" => Self::Macos,
            "linux" => Self::Linux,
            _ => Self::Other,
        }
    }
}

/// `aecMode`: which echo canceller cleans the microphone signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AecMode {
    Off,
    /// The cross-platform software canceller.
    Screenpipe,
    /// Apple's voice-processing input unit (macOS only).
    Macos,
    /// The WASAPI communications-mode canceller (Windows only).
    Windows,
}

/// The three stored booleans an [`AecMode`] stands for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AecFlags {
    pub screenpipe_aec_enabled: bool,
    pub macos_input_vpio_enabled: bool,
    pub windows_input_aec_enabled: bool,
}

impl AecMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "" => Some(Self::Off),
            "screenpipe" => Some(Self::Screenpipe),
            "macos" => Some(Self::Macos),
            "windows" => Some(Self::Windows),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Screenpipe => "screenpipe",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }

    /// The mode that can actually run on `platform`: the software canceller
    /// runs everywhere, the OS cancellers only on their own system, and
    /// anything else is off.
    pub fn for_platform(self, platform: SettingsPlatform) -> Self {
        match (self, platform) {
            (Self::Screenpipe, _) => Self::Screenpipe,
            (Self::Macos, SettingsPlatform::Macos) => Self::Macos,
            (Self::Windows, SettingsPlatform::Windows) => Self::Windows,
            _ => Self::Off,
        }
    }

    pub fn flags(self) -> AecFlags {
        AecFlags {
            screenpipe_aec_enabled: self == Self::Screenpipe,
            macos_input_vpio_enabled: self == Self::Macos,
            windows_input_aec_enabled: self == Self::Windows,
        }
    }

    /// The mode older stores expressed through the booleans alone (they
    /// predate `aecMode`). The OS canceller of the running platform wins over
    /// the software one; a flag for another OS means nothing here.
    pub fn from_flags(flags: AecFlags, platform: SettingsPlatform) -> Self {
        if flags.windows_input_aec_enabled && platform == SettingsPlatform::Windows {
            Self::Windows
        } else if flags.macos_input_vpio_enabled && platform == SettingsPlatform::Macos {
            Self::Macos
        } else if flags.screenpipe_aec_enabled {
            Self::Screenpipe
        } else {
            Self::Off
        }
    }
}

/// `recordingDetail`: how often snapshots are taken while scrolling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingDetail {
    Auto,
    LowImpact,
    Balanced,
    MoreDetail,
}

impl RecordingDetail {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "low_impact" | "low-impact" => Some(Self::LowImpact),
            "balanced" => Some(Self::Balanced),
            "more_detail" | "more-detail" => Some(Self::MoreDetail),
            _ => None,
        }
    }

    /// Parse a stored value; unknown values mean [`Self::Auto`].
    pub fn from_setting(value: &str) -> Self {
        Self::parse(value).unwrap_or(Self::Auto)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::LowImpact => "low_impact",
            Self::Balanced => "balanced",
            Self::MoreDetail => "more_detail",
        }
    }

    /// Fixed scroll-snapshot spacing in milliseconds, or `None` for `Auto`
    /// (which adapts to capture cost and the power profile).
    pub fn fixed_interval_ms(self) -> Option<u64> {
        match self {
            Self::Auto => None,
            Self::LowImpact => Some(5_000),
            Self::Balanced => Some(2_000),
            Self::MoreDetail => Some(1_000),
        }
    }
}

/// `semanticContextMode`: what semantic parsing is tuned for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticContextMode {
    Memory,
    ComputerUse,
    Both,
}

impl SemanticContextMode {
    /// Accepts the frontend spelling (`computerUse`) and the engine spelling
    /// (`computer_use`).
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "memory" => Some(Self::Memory),
            "computerUse" | "computer_use" | "computer-use" => Some(Self::ComputerUse),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    /// Parse a stored value; unknown values mean [`Self::Memory`].
    pub fn from_setting(value: &str) -> Self {
        Self::parse(value).unwrap_or(Self::Memory)
    }

    /// The value the settings store uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::ComputerUse => "computerUse",
            Self::Both => "both",
        }
    }

    /// The value the engine uses.
    pub fn engine_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::ComputerUse => "computer_use",
            Self::Both => "both",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_capture_mode_parses_and_falls_back() {
        assert_eq!(
            AudioCaptureMode::from_setting("meetings-only"),
            AudioCaptureMode::MeetingsOnly
        );
        assert_eq!(
            AudioCaptureMode::from_setting("disabled"),
            AudioCaptureMode::Disabled
        );
        assert_eq!(
            AudioCaptureMode::from_setting("always"),
            AudioCaptureMode::Always
        );
        assert_eq!(AudioCaptureMode::from_setting(""), AudioCaptureMode::Always);
        assert_eq!(
            AudioCaptureMode::from_setting("sometimes"),
            AudioCaptureMode::Always
        );
        for mode in [
            AudioCaptureMode::Always,
            AudioCaptureMode::MeetingsOnly,
            AudioCaptureMode::Disabled,
        ] {
            assert_eq!(AudioCaptureMode::parse(mode.as_str()), Some(mode));
        }
    }

    #[test]
    fn aec_mode_is_validated_per_platform() {
        use SettingsPlatform::*;
        assert_eq!(AecMode::Windows.for_platform(Windows), AecMode::Windows);
        assert_eq!(AecMode::Windows.for_platform(Macos), AecMode::Off);
        assert_eq!(AecMode::Macos.for_platform(Macos), AecMode::Macos);
        assert_eq!(AecMode::Macos.for_platform(Linux), AecMode::Off);
        assert_eq!(AecMode::Screenpipe.for_platform(Linux), AecMode::Screenpipe);
        assert_eq!(AecMode::Off.for_platform(Windows), AecMode::Off);
    }

    #[test]
    fn aec_flags_follow_the_mode() {
        let flags = AecMode::Windows.flags();
        assert!(flags.windows_input_aec_enabled);
        assert!(!flags.macos_input_vpio_enabled && !flags.screenpipe_aec_enabled);
        assert_eq!(AecMode::Off.flags(), AecFlags::default());
        assert!(AecMode::Screenpipe.flags().screenpipe_aec_enabled);
    }

    #[test]
    fn aec_mode_is_derived_from_legacy_flags() {
        let win = AecFlags {
            windows_input_aec_enabled: true,
            ..AecFlags::default()
        };
        assert_eq!(
            AecMode::from_flags(win, SettingsPlatform::Windows),
            AecMode::Windows
        );
        assert_eq!(
            AecMode::from_flags(win, SettingsPlatform::Macos),
            AecMode::Off
        );
        let mac = AecFlags {
            macos_input_vpio_enabled: true,
            screenpipe_aec_enabled: true,
            ..AecFlags::default()
        };
        assert_eq!(
            AecMode::from_flags(mac, SettingsPlatform::Macos),
            AecMode::Macos
        );
        assert_eq!(
            AecMode::from_flags(mac, SettingsPlatform::Linux),
            AecMode::Screenpipe
        );
    }

    #[test]
    fn recording_detail_intervals() {
        assert_eq!(
            RecordingDetail::from_setting("auto").fixed_interval_ms(),
            None
        );
        assert_eq!(
            RecordingDetail::from_setting("low_impact").fixed_interval_ms(),
            Some(5_000)
        );
        assert_eq!(
            RecordingDetail::from_setting("balanced").fixed_interval_ms(),
            Some(2_000)
        );
        assert_eq!(
            RecordingDetail::from_setting("more_detail").fixed_interval_ms(),
            Some(1_000)
        );
        assert_eq!(RecordingDetail::from_setting("max"), RecordingDetail::Auto);
    }

    #[test]
    fn semantic_mode_accepts_both_spellings() {
        assert_eq!(
            SemanticContextMode::from_setting("computerUse"),
            SemanticContextMode::ComputerUse
        );
        assert_eq!(
            SemanticContextMode::from_setting("computer_use"),
            SemanticContextMode::ComputerUse
        );
        assert_eq!(
            SemanticContextMode::ComputerUse.engine_str(),
            "computer_use"
        );
        assert_eq!(
            SemanticContextMode::from_setting("nope"),
            SemanticContextMode::Memory
        );
    }
}
