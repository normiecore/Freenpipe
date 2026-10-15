//! Product analytics: removed.
//!
//! Upstream screenpipe posted usage events from the engine to a hosted
//! analytics service. Freenpipe never sends telemetry, so this module keeps
//! only the call surface the rest of the engine still uses:
//!
//! - every capture function is a no-op, so existing call sites compile and
//!   nothing is built, queued or sent;
//! - [`get_distinct_id`] is a local identifier. It never leaves the machine;
//!   it only salts hashes that are computed and logged locally (for example
//!   meeting event keys in `meeting_telemetry`).

use once_cell::sync::Lazy;
use serde_json::Value;
use tracing::debug;

#[cfg(target_os = "macos")]
use sysinfo::{System, SystemExt};
#[cfg(target_os = "macos")]
use tracing::warn;

use crate::telemetry_context::TelemetryContext;

/// Local install id: the app passes its persisted id through the environment;
/// a bare CLI run gets a random id for the lifetime of the process.
static DISTINCT_ID: Lazy<String> = Lazy::new(|| {
    TelemetryContext::distinct_id_from_env().unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
});

/// Whether telemetry is turned off by the environment.
///
/// screenpipe v2.7 lets `SCREENPIPE_DISABLE_TELEMETRY` (or a CI environment)
/// switch telemetry off. Freenpipe has no telemetry to switch, so the answer
/// is always `true` whatever the environment says.
pub fn is_telemetry_disabled_by_env() -> bool {
    true
}

/// Kept for call-site compatibility. The legacy opt-in flag is accepted and
/// ignored: no usage data is ever sent.
pub fn init(telemetry_enabled: bool) {
    Lazy::force(&DISTINCT_ID);
    if telemetry_enabled {
        debug!("analytics opt-in setting ignored: this build never sends usage data");
    }
}

/// Local install id (see the module docs). Never transmitted.
pub fn get_distinct_id() -> &'static str {
    &DISTINCT_ID
}

/// No-op. Upstream fired a usage event here; Freenpipe drops it unread.
pub fn capture_event_nonblocking(event: &'static str, properties: Value) {
    let _ = (event, properties);
}

/// Parse a macOS version string (e.g. "14.5" or "10.15.7") into its major number.
#[cfg(target_os = "macos")]
fn parse_macos_major_version(version_str: &str) -> Option<u32> {
    version_str.split('.').next()?.parse().ok()
}

/// Warn locally when the macOS version is too old for reliable screen capture.
///
/// - Below 12 (Monterey): ScreenCaptureKit is not available at all.
/// - Below 14 (Sonoma): capture works but upgrading is recommended.
#[cfg(target_os = "macos")]
pub fn check_macos_version() {
    let sys = System::new();
    let os_version = sys.os_version().unwrap_or_default();
    let os_name = sys.name().unwrap_or_default();

    if !os_name.to_lowercase().contains("mac") {
        return;
    }

    let major_version = match parse_macos_major_version(&os_version) {
        Some(v) => v,
        None => {
            debug!("Could not parse macOS version: {}", os_version);
            return;
        }
    };

    if major_version < 12 {
        warn!(
            "macOS {} detected. Screen recording requires macOS 12.3+ (Monterey). \
            Please upgrade your macOS for screen capture to work.",
            os_version
        );
    } else if major_version < 14 {
        warn!(
            "macOS {} detected. For best screen capture performance, \
            macOS 14+ (Sonoma) is recommended.",
            os_version
        );
    } else {
        debug!("macOS version {} is supported", os_version);
    }
}

/// No-op on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn check_macos_version() {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn telemetry_is_always_disabled() {
        assert!(is_telemetry_disabled_by_env());
    }

    #[test]
    fn distinct_id_is_stable_and_non_empty() {
        init(true);
        let first = get_distinct_id();
        assert!(!first.is_empty());
        assert_eq!(first, get_distinct_id());
    }

    #[test]
    fn capture_is_a_noop_outside_a_runtime() {
        // Upstream spawned a tokio task here; the no-op must not need a runtime.
        capture_event_nonblocking("anything", json!({ "k": "v" }));
    }
}
