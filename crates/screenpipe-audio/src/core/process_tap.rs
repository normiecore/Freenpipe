// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! System-audio taps: one facade over the per-platform backends.
//!
//! * macOS 14.4+: the CoreAudio Process Tap in [`macos`] (moved here
//!   unchanged from the former single-file module).
//! * Windows: the WASAPI loopback engine in `core/wasapi_loopback.rs`, with
//!   per-process include/exclude taps on build 20348 and later.
//! * Elsewhere: no tap; [`is_process_tap_available`] is `false`.
//!
//! The submodules [`exclusions`], [`targets`] and [`pcm`] hold the
//! platform-neutral rules the Windows engine runs on (exclusion file, target
//! choice, process-tree roots, sample decoding) so they are unit-tested on
//! every platform.

pub mod exclusions;
pub mod pcm;
pub mod targets;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::{is_process_tap_available, spawn_process_tap_capture};

/// Whether this Windows build supports per-process audio taps (include or
/// exclude a process tree, build 20348 or later). The answer is cached.
#[cfg(target_os = "windows")]
pub fn is_process_tap_available() -> bool {
    super::wasapi_loopback::process_loopback_support().is_available()
}

/// No per-process tap exists on this platform.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn is_process_tap_available() -> bool {
    false
}

#[cfg(all(test, not(any(target_os = "macos", target_os = "windows"))))]
mod tests {
    #[test]
    fn no_tap_on_this_platform() {
        assert!(!super::is_process_tap_available());
    }
}
