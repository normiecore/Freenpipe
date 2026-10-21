// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! CPU compatibility mode for processors without AVX2 (shell-022).
//!
//! Freenpipe's Whisper engines (whisper.cpp/ggml) and its Qwen3-ASR engine
//! (the antirez C build, on every platform) run AVX2 code. On an x86 CPU
//! without AVX2 they die with an illegal-instruction fault instead of
//! transcribing. Such CPUs are rare but real: older Pentium/Celeron parts and
//! some virtual machines that hide the feature.
//!
//! The app checks once, at the top of `app_main`, and keeps the answer. In
//! compatibility mode it
//! - transcribes with Parakeet (ONNX Runtime, no AVX2 requirement) when a
//!   Whisper or Qwen3-ASR engine is selected, or turns local transcription
//!   off in a build without Parakeet ([`apply_to_resolution`]),
//! - skips the Whisper model pre-download ([`whisper_predownload_allowed`]),
//! - reports `cpuCompatMode: true` in the boot phase snapshot, so onboarding
//!   and settings can say why.
//!
//! The check uses the standard library's runtime feature detection (CPUID
//! plus the OS's XSAVE state), so nothing here repeats that logic. On other
//! architectures the question does not arise and compatibility mode is off.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use tracing::{debug, info, warn};

use crate::store::{AudioEngineFallbackReason, AudioEngineResolution};

/// Engine compatibility mode switches to.
pub const COMPAT_ENGINE: &str = "parakeet";
/// What compatibility mode leaves when Parakeet is not compiled in.
pub const NO_LOCAL_ENGINE: &str = "disabled";

/// What the processor supports, as far as this module cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuSupport {
    /// `Some(true)`/`Some(false)` on x86 and x86_64; `None` elsewhere, where
    /// AVX2 is not a thing.
    pub avx2: Option<bool>,
}

impl CpuSupport {
    /// True when engines that need AVX2 must not run.
    pub fn compat_mode(self) -> bool {
        self.avx2 == Some(false)
    }
}

/// Ask the processor (through the standard library's cached detection).
pub fn detect() -> CpuSupport {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        CpuSupport {
            avx2: Some(std::arch::is_x86_feature_detected!("avx2")),
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        CpuSupport { avx2: None }
    }
}

static SUPPORT: OnceLock<CpuSupport> = OnceLock::new();

/// Detect once and remember. `app_main` calls it first thing; later calls
/// return the stored answer.
pub fn init() -> CpuSupport {
    *SUPPORT.get_or_init(detect)
}

/// Whether the app runs in CPU compatibility mode.
pub fn compat_mode() -> bool {
    init().compat_mode()
}

/// Whether this build has the Parakeet engine.
pub fn parakeet_built() -> bool {
    cfg!(feature = "parakeet")
}

/// Engines whose Freenpipe build executes AVX2 instructions.
pub fn engine_needs_avx2(engine: &str) -> bool {
    engine.starts_with("whisper") || engine == "qwen3-asr"
}

/// The engine to run instead of `engine`, or `None` when `engine` can run.
pub fn replacement_engine(engine: &str, compat: bool, parakeet: bool) -> Option<&'static str> {
    if !compat || !engine_needs_avx2(engine) {
        return None;
    }
    Some(if parakeet {
        COMPAT_ENGINE
    } else {
        NO_LOCAL_ENGINE
    })
}

/// The fallback reason that explains switching to `engine`: Parakeet, or
/// no local engine at all in a build without Parakeet.
pub fn fallback_reason_for(engine: &str) -> AudioEngineFallbackReason {
    if engine == COMPAT_ENGINE {
        AudioEngineFallbackReason::CpuCompatMode
    } else {
        AudioEngineFallbackReason::CpuCompatModeNoLocalEngine
    }
}

/// Whether the engine switch has been logged at warn level. The resolution
/// is recomputed for every recording config (the frontend polls it), so
/// only the first switch in a process is a warning.
static SWITCH_LOGGED: AtomicBool = AtomicBool::new(false);

/// Apply compatibility mode to an engine resolution: an engine that needs
/// AVX2 (including a Whisper fallback chosen for a cloud engine without its
/// key) becomes Parakeet, or `disabled` when the build has no Parakeet, and
/// the reason says which.
pub fn apply_to_resolution(resolution: &mut AudioEngineResolution, compat: bool, parakeet: bool) {
    if let Some(engine) = replacement_engine(&resolution.active, compat, parakeet) {
        if !SWITCH_LOGGED.swap(true, Ordering::Relaxed) {
            warn!(
                "CPU compatibility mode: {} needs AVX2, transcribing with {engine} instead",
                resolution.active
            );
        } else {
            debug!(
                "CPU compatibility mode: {} replaced by {engine}",
                resolution.active
            );
        }
        resolution.active = engine.to_string();
        resolution.fallback_reason = Some(fallback_reason_for(engine));
    }
}

/// Whether the Whisper model may be fetched ahead of time.
pub fn whisper_predownload_allowed() -> bool {
    !compat_mode()
}

/// Log what the start-up check found. Call once logging works.
pub fn log_startup_state() {
    let support = init();
    if support.compat_mode() {
        warn!(
            "this CPU has no AVX2: running in compatibility mode (Whisper and Qwen3-ASR are off, Parakeet and cloud engines stay available)"
        );
    }
    info!(
        "cpu-smoke: native start-up passed (avx2={}, compat_mode={})",
        match support.avx2 {
            Some(true) => "yes",
            Some(false) => "no",
            None => "n/a",
        },
        support.compat_mode()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolution(active: &str) -> AudioEngineResolution {
        AudioEngineResolution {
            requested: active.to_string(),
            active: active.to_string(),
            fallback_reason: None,
        }
    }

    #[test]
    fn compat_mode_only_for_x86_without_avx2() {
        assert!(CpuSupport { avx2: Some(false) }.compat_mode());
        assert!(!CpuSupport { avx2: Some(true) }.compat_mode());
        assert!(!CpuSupport { avx2: None }.compat_mode());
    }

    #[test]
    fn detection_is_stable() {
        assert_eq!(init(), init());
        assert_eq!(init(), detect());
        if cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
            assert!(detect().avx2.is_some());
        } else {
            assert_eq!(detect().avx2, None);
        }
    }

    #[test]
    fn avx2_engines_are_whisper_and_qwen() {
        for engine in [
            "whisper-tiny",
            "whisper-tiny-quantized",
            "whisper-large",
            "whisper-large-v3-turbo",
            "whisper-large-v3-turbo-quantized",
            "qwen3-asr",
        ] {
            assert!(engine_needs_avx2(engine), "{engine}");
        }
        for engine in [
            "parakeet",
            "parakeet-mlx",
            "deepgram",
            "screenpipe-cloud",
            "openai-compatible",
            "disabled",
        ] {
            assert!(!engine_needs_avx2(engine), "{engine}");
        }
    }

    #[test]
    fn compat_mode_switches_to_parakeet() {
        assert_eq!(
            replacement_engine("whisper-large-v3-turbo", true, true),
            Some("parakeet")
        );
        assert_eq!(
            replacement_engine("qwen3-asr", true, true),
            Some("parakeet")
        );
        assert_eq!(
            replacement_engine("qwen3-asr", true, false),
            Some("disabled")
        );
        assert_eq!(replacement_engine("whisper-tiny", false, true), None);
        assert_eq!(replacement_engine("deepgram", true, true), None);
    }

    #[test]
    fn resolution_keeps_engines_that_run_and_explains_a_switch() {
        let mut fine = resolution("parakeet");
        apply_to_resolution(&mut fine, true, true);
        assert_eq!(fine, resolution("parakeet"));

        let mut off = resolution("whisper-large-v3-turbo-quantized");
        apply_to_resolution(&mut off, false, true);
        assert_eq!(off, resolution("whisper-large-v3-turbo-quantized"));

        // A cloud engine that already fell back to Whisper ends on Parakeet.
        let mut fallback = AudioEngineResolution {
            requested: "deepgram".to_string(),
            active: "whisper-large-v3-turbo-quantized".to_string(),
            fallback_reason: Some(AudioEngineFallbackReason::MissingDeepgramKey),
        };
        apply_to_resolution(&mut fallback, true, true);
        assert_eq!(fallback.requested, "deepgram");
        assert_eq!(fallback.active, "parakeet");
        assert_eq!(
            fallback.fallback_reason,
            Some(AudioEngineFallbackReason::CpuCompatMode)
        );

        // Recomputed again (every recording config does): same answer.
        let mut again = resolution("whisper-large-v3-turbo-quantized");
        apply_to_resolution(&mut again, true, true);
        assert_eq!(again.active, "parakeet");
        assert!(SWITCH_LOGGED.load(Ordering::Relaxed));
    }

    #[test]
    fn a_build_without_parakeet_says_local_transcription_is_off() {
        let mut none = resolution("qwen3-asr");
        apply_to_resolution(&mut none, true, false);
        assert_eq!(none.active, "disabled");
        assert_eq!(
            none.fallback_reason,
            Some(AudioEngineFallbackReason::CpuCompatModeNoLocalEngine)
        );
        assert!(!none
            .fallback_reason
            .as_ref()
            .unwrap()
            .notification_body()
            .contains("Parakeet instead"));
        assert_eq!(
            fallback_reason_for("parakeet"),
            AudioEngineFallbackReason::CpuCompatMode
        );
        assert!(AudioEngineFallbackReason::CpuCompatMode
            .notification_body()
            .contains("Parakeet"));
    }
}
