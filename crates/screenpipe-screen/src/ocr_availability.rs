// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Whether the platform OCR engine can run, probed once at engine start.
//!
//! - Windows: the WinRT recognizer languages, read on the OCR worker thread.
//!   A bundled or installed tesseract is reported alongside for diagnostics;
//!   live capture uses the WinRT engine.
//! - macOS: Apple Vision ships with the OS and is always available.
//! - Linux and others: a tesseract binary, installed system-wide or bundled
//!   beside the engine (see `tesseract.rs`).
//!
//! An unavailable engine is published through
//! [`crate::metrics::set_ocr_engine_available`], so `/health` reports
//! `vision_reason = ocr_unavailable` when nothing more serious is wrong.
//! Capture itself carries on: frames are stored, accessibility text is still
//! recorded, and the metrics keep counting.
//!
//! `FREENPIPE_OCR_FAULT=unavailable` (or `1`/`true`) makes every OCR backend
//! behave as if no engine or language were installed, for tests and for
//! checking the reporting path on a machine that does have OCR.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tracing::{info, warn};

/// Test hook: makes the OCR engines report themselves as missing.
pub const OCR_FAULT_ENV: &str = "FREENPIPE_OCR_FAULT";

static SIMULATED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

/// Turns the simulated "no OCR installed" state on or off in-process (the
/// programmatic twin of [`OCR_FAULT_ENV`]).
pub fn simulate_ocr_unavailable(on: bool) {
    SIMULATED_UNAVAILABLE.store(on, Ordering::SeqCst);
}

fn fault_value_means_unavailable(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "unavailable" | "no_languages"
    )
}

/// True when OCR should behave as if no engine were installed.
pub fn ocr_fault_injected() -> bool {
    SIMULATED_UNAVAILABLE.load(Ordering::SeqCst)
        || std::env::var(OCR_FAULT_ENV)
            .map(|v| fault_value_means_unavailable(&v))
            .unwrap_or(false)
}

/// Reason given when the fault hook is active.
pub const SIMULATED_REASON: &str =
    "OCR is turned off by the FREENPIPE_OCR_FAULT test hook (no engine or language is treated as installed)";

/// Result of an availability probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcrAvailability {
    /// Whether screen text can be recognised.
    pub available: bool,
    /// The engine live capture uses on this platform.
    pub backend: &'static str,
    /// Recognizer languages the engine offers (Windows), if known.
    pub languages: Vec<String>,
    /// Path of the tesseract binary that was found, if any.
    pub tesseract: Option<String>,
    /// Why OCR is unavailable (None when it is available).
    pub reason: Option<String>,
    /// Whether the fault hook produced this result.
    pub simulated: bool,
}

/// Availability from what the Windows OCR worker reported.
pub fn availability_from_windows_probe(
    recognizer_languages: Vec<String>,
    user_profile_engine: bool,
    tesseract: Option<String>,
    simulated: bool,
) -> OcrAvailability {
    let reason = if simulated {
        Some(SIMULATED_REASON.to_string())
    } else if recognizer_languages.is_empty() && !user_profile_engine {
        Some(
            "no Windows OCR recognizer language is installed; add a language with \
             optical character recognition under Settings > Time & language > \
             Language & region"
                .to_string(),
        )
    } else {
        None
    };
    OcrAvailability {
        available: reason.is_none(),
        backend: "windows-native",
        languages: recognizer_languages,
        tesseract,
        reason,
        simulated,
    }
}

/// Availability of a tesseract-based OCR path.
pub fn availability_from_tesseract(binary: Option<String>, simulated: bool) -> OcrAvailability {
    let reason = if simulated {
        Some(SIMULATED_REASON.to_string())
    } else if binary.is_none() {
        Some(
            "the tesseract program was not found, neither installed system-wide nor \
             bundled beside the engine; install tesseract-ocr to get searchable screen text"
                .to_string(),
        )
    } else {
        None
    };
    OcrAvailability {
        available: reason.is_none(),
        backend: "tesseract",
        languages: Vec::new(),
        tesseract: binary,
        reason,
        simulated,
    }
}

/// Probes the OCR engine live capture uses on this platform. Returns an
/// error when the probe itself could not finish (for example the Windows OCR
/// worker did not answer); the published state is then left unchanged.
pub async fn probe_ocr_availability() -> Result<OcrAvailability, String> {
    platform_probe(ocr_fault_injected()).await
}

#[cfg(target_os = "windows")]
async fn platform_probe(simulated: bool) -> Result<OcrAvailability, String> {
    let tesseract = tokio::task::spawn_blocking(|| {
        crate::tesseract::tesseract_install().map(|t| t.binary.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("tesseract discovery task failed: {}", e))?;
    if simulated {
        return Ok(availability_from_windows_probe(
            Vec::new(),
            false,
            tesseract,
            true,
        ));
    }
    let probe = crate::microsoft::probe_windows_ocr()
        .await
        .map_err(|e| e.to_string())?;
    Ok(availability_from_windows_probe(
        probe.recognizer_languages,
        probe.user_profile_engine,
        tesseract,
        false,
    ))
}

#[cfg(target_os = "macos")]
async fn platform_probe(simulated: bool) -> Result<OcrAvailability, String> {
    Ok(OcrAvailability {
        available: !simulated,
        backend: "apple-native",
        languages: Vec::new(),
        tesseract: None,
        reason: simulated.then(|| SIMULATED_REASON.to_string()),
        simulated,
    })
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
async fn platform_probe(simulated: bool) -> Result<OcrAvailability, String> {
    let binary = tokio::task::spawn_blocking(|| {
        crate::tesseract::tesseract_install().map(|t| t.binary.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("tesseract discovery task failed: {}", e))?;
    Ok(availability_from_tesseract(binary, simulated))
}

/// Probes once and publishes the result for `/health`. Logs what was found.
pub async fn probe_and_publish() -> Option<OcrAvailability> {
    match probe_ocr_availability().await {
        Ok(availability) => {
            publish(&availability);
            Some(availability)
        }
        Err(e) => {
            warn!("could not determine whether OCR is available: {}", e);
            None
        }
    }
}

/// Publishes a probe result for `/health` and logs it.
pub fn publish(availability: &OcrAvailability) {
    crate::metrics::set_ocr_engine_available(availability.available);
    if availability.available {
        info!(
            backend = availability.backend,
            languages = %availability.languages.join(","),
            "screen text recognition (OCR) is available"
        );
    } else {
        warn!(
            backend = availability.backend,
            "screen text recognition (OCR) is unavailable: {}. Frames are still recorded and \
             accessibility text is still captured, but screenshots carry no searchable OCR text; \
             /health reports vision_reason=ocr_unavailable",
            availability.reason.as_deref().unwrap_or("unknown reason")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fault_values() {
        for v in ["1", "true", "TRUE", " unavailable ", "no_languages", "on", "yes"] {
            assert!(fault_value_means_unavailable(v), "{v:?}");
        }
        for v in ["", "0", "false", "off", "inference"] {
            assert!(!fault_value_means_unavailable(v), "{v:?}");
        }
    }

    #[test]
    fn windows_without_languages_is_unavailable() {
        let a = availability_from_windows_probe(Vec::new(), false, None, false);
        assert!(!a.available);
        assert_eq!(a.backend, "windows-native");
        assert!(a.reason.unwrap().contains("recognizer language"));
    }

    #[test]
    fn windows_with_a_language_is_available() {
        let a = availability_from_windows_probe(
            vec!["en-US".to_string()],
            true,
            Some("C:/t/tesseract.exe".into()),
            false,
        );
        assert!(a.available);
        assert_eq!(a.reason, None);
        assert_eq!(a.languages, vec!["en-US"]);
        assert_eq!(a.tesseract.as_deref(), Some("C:/t/tesseract.exe"));
    }

    #[test]
    fn a_working_user_profile_engine_counts_as_available() {
        // Some builds create a profile engine even when the language list
        // comes back empty.
        assert!(availability_from_windows_probe(Vec::new(), true, None, false).available);
    }

    #[test]
    fn the_fault_hook_wins_over_installed_languages() {
        let a = availability_from_windows_probe(vec!["en-US".into()], true, None, true);
        assert!(!a.available);
        assert!(a.simulated);
        let t = availability_from_tesseract(Some("/usr/bin/tesseract".into()), true);
        assert!(!t.available);
        assert_eq!(t.reason.as_deref(), Some(SIMULATED_REASON));
    }

    #[test]
    fn tesseract_presence_decides_availability() {
        assert!(availability_from_tesseract(Some("/usr/bin/tesseract".into()), false).available);
        let missing = availability_from_tesseract(None, false);
        assert!(!missing.available);
        assert!(missing.reason.unwrap().contains("tesseract-ocr"));
    }
}
