// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Window identity rules for Windows that need no OS calls, kept outside the
//! `cfg(windows)` module so every platform's test run covers them. The Win32
//! glue that feeds them lives in `platform::windows`.
//!
//! UWP apps (Calculator, Settings, Photos, ...) do not own their top-level
//! window. Their frame belongs to `ApplicationFrameHost.exe`, and the app's
//! own process owns a core window that is a child of that frame. Asking for
//! the foreground window's process therefore names the frame host for every
//! UWP app, so frames, app switches and ignore rules would all see
//! "ApplicationFrameHost.exe". The helpers here pick the hosted app from the
//! frame's child windows instead, and fall back to the host's name (never to
//! an empty one) while the app has not attached its window yet.
//!
//! Older versions reported every UWP app under the host's name, so a user may
//! have excluded them with a rule that names `ApplicationFrameHost`. A
//! resolved app keeps the host's name in [`WindowIdentity::frame_host`], and
//! exclusions (never inclusions) are also checked against it, so such a rule
//! keeps excluding what it excluded before.

use crate::config::UiCaptureConfig;
use screenpipe_core::window_pattern::{self, WindowPattern};

/// Executable of the process that draws UWP app frames.
pub const UWP_FRAME_HOST_PROCESS: &str = "ApplicationFrameHost.exe";

/// Window class of a UWP app frame (owned by the frame host).
pub const UWP_FRAME_WINDOW_CLASS: &str = "ApplicationFrameWindow";

/// Window class of a UWP app's own core window (a child of its frame).
pub const UWP_CORE_WINDOW_CLASS: &str = "Windows.UI.Core.CoreWindow";

/// Name used when a window's process cannot be identified at all.
pub const UNKNOWN_APP_NAME: &str = "Unknown";

/// Most child windows looked at when searching a frame for its hosted app.
/// A UWP frame has a handful; the cap only bounds a pathological window.
pub const MAX_FRAME_CHILDREN: usize = 64;

/// True when the window is a UWP frame drawn by the frame host, so the app
/// it shows has to be looked up among its children.
pub fn is_uwp_frame(raw_process: &str, window_class: &str) -> bool {
    raw_process.eq_ignore_ascii_case(UWP_FRAME_HOST_PROCESS)
        && window_class == UWP_FRAME_WINDOW_CLASS
}

/// A child window of a UWP frame: the process that owns it and its class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameChild {
    pub pid: u32,
    pub class: String,
}

/// The process of the app hosted in a frame owned by `host_pid`: a child
/// window owned by another process, preferring the app's core window over
/// any other foreign child. `None` while no such child exists (the app is
/// starting, suspended or closing).
pub fn hosted_app_pid(host_pid: u32, children: &[FrameChild]) -> Option<u32> {
    let foreign = |c: &&FrameChild| c.pid != 0 && c.pid != host_pid;
    children
        .iter()
        .filter(foreign)
        .find(|c| c.class == UWP_CORE_WINDOW_CLASS)
        .or_else(|| children.iter().find(foreign))
        .map(|c| c.pid)
}

/// Who a window belongs to, as frames, app switches and input report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowIdentity {
    /// The reported app name. Never empty.
    pub app_name: String,
    /// For a UWP app resolved behind its frame: the frame host's name, the
    /// name older versions reported the app under. Exclusion rules are also
    /// checked against it ([`frame_host_excluded`]). `None` otherwise.
    pub frame_host: Option<String>,
}

impl WindowIdentity {
    /// An identity with no frame host (every window that is not a resolved
    /// UWP frame).
    pub fn plain(app_name: impl Into<String>) -> Self {
        Self {
            app_name: app_name.into(),
            frame_host: None,
        }
    }
}

/// The identity of a UWP frame: the hosted app's process name when it is
/// known (keeping the host's name as [`WindowIdentity::frame_host`]), else
/// the frame host's own name. The app name is never empty.
pub fn frame_identity(host_process: &str, hosted_process: Option<&str>) -> WindowIdentity {
    match hosted_process
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        Some(hosted) => WindowIdentity {
            app_name: hosted.to_string(),
            frame_host: Some(non_empty_app_name(host_process)),
        },
        None => WindowIdentity::plain(non_empty_app_name(host_process)),
    }
}

/// True when an exclusion names the frame host of a resolved UWP app: an
/// `excluded_apps` entry contained in the host's name, or an ignore pattern
/// (unscoped, or scoped to the host and this title) that matches it.
/// Inclusions are not checked against the host: an include rule written
/// for the host no longer lets a resolved app through, which records less,
/// never more.
pub fn frame_host_excluded(
    identity: &WindowIdentity,
    window_title: Option<&str>,
    excluded_apps: &[String],
    ignored: &[WindowPattern],
) -> bool {
    let Some(host) = identity.frame_host.as_deref() else {
        return false;
    };
    let host_lower = host.to_lowercase();
    if excluded_apps
        .iter()
        .any(|excluded| host_lower.contains(&excluded.to_lowercase()))
    {
        return true;
    }
    let title_lower = window_title.unwrap_or_default().to_lowercase();
    window_pattern::matches_any(ignored, &host_lower, &title_lower)
}

/// [`UiCaptureConfig::should_capture_target`] for a window identity: the app
/// name must pass every filter, and no exclusion may name the frame host of
/// a resolved UWP app.
pub fn should_capture_identity(
    config: &UiCaptureConfig,
    identity: &WindowIdentity,
    window_title: Option<&str>,
) -> bool {
    if !config.should_capture_target(&identity.app_name, window_title) {
        return false;
    }
    if identity.frame_host.is_none() {
        return true;
    }
    let ignored = WindowPattern::parse_list(&config.ignored_windows);
    !frame_host_excluded(identity, window_title, &config.excluded_apps, &ignored)
}

/// `name` trimmed, or [`UNKNOWN_APP_NAME`] when nothing is left.
pub fn non_empty_app_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        UNKNOWN_APP_NAME.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child(pid: u32, class: &str) -> FrameChild {
        FrameChild {
            pid,
            class: class.to_string(),
        }
    }

    #[test]
    fn only_frame_host_frames_are_resolved() {
        assert!(is_uwp_frame(
            "ApplicationFrameHost.exe",
            "ApplicationFrameWindow"
        ));
        assert!(is_uwp_frame(
            "applicationframehost.exe",
            "ApplicationFrameWindow"
        ));
        // Chromium content re-parented under the host is Edge, not UWP.
        assert!(!is_uwp_frame(
            "ApplicationFrameHost.exe",
            "Chrome_WidgetWin_1"
        ));
        assert!(!is_uwp_frame("CalculatorApp.exe", "ApplicationFrameWindow"));
        assert!(!is_uwp_frame("explorer.exe", "CabinetWClass"));
    }

    #[test]
    fn hosted_app_is_the_foreign_core_window() {
        let host = 100;
        let children = vec![
            child(host, "ApplicationFrameTitleBarWindow"),
            child(host, "ApplicationFrameInputSinkWindow"),
            child(4242, UWP_CORE_WINDOW_CLASS),
        ];
        assert_eq!(hosted_app_pid(host, &children), Some(4242));
    }

    #[test]
    fn core_window_wins_over_other_foreign_children() {
        let host = 100;
        let children = vec![
            child(777, "SomeOverlay"),
            child(4242, UWP_CORE_WINDOW_CLASS),
        ];
        assert_eq!(hosted_app_pid(host, &children), Some(4242));
        // Without a core window, any foreign child identifies the app.
        assert_eq!(hosted_app_pid(host, &children[..1]), Some(777));
    }

    #[test]
    fn no_hosted_app_while_only_host_children_exist() {
        let host = 100;
        assert_eq!(hosted_app_pid(host, &[]), None);
        assert_eq!(
            hosted_app_pid(host, &[child(host, UWP_CORE_WINDOW_CLASS), child(0, "x")]),
            None
        );
    }

    #[test]
    fn frame_name_falls_back_to_host_and_is_never_empty() {
        assert_eq!(
            frame_identity("ApplicationFrameHost.exe", Some("CalculatorApp.exe")),
            WindowIdentity {
                app_name: "CalculatorApp.exe".to_string(),
                frame_host: Some("ApplicationFrameHost.exe".to_string()),
            }
        );
        assert_eq!(
            frame_identity("ApplicationFrameHost.exe", None),
            WindowIdentity::plain("ApplicationFrameHost.exe")
        );
        assert_eq!(
            frame_identity("ApplicationFrameHost.exe", Some("  ")),
            WindowIdentity::plain("ApplicationFrameHost.exe")
        );
        assert_eq!(
            frame_identity("", None),
            WindowIdentity::plain(UNKNOWN_APP_NAME)
        );
        assert_eq!(non_empty_app_name(" notepad.exe "), "notepad.exe");
    }

    fn config(ignored: &[&str], included: &[&str], excluded_apps: &[&str]) -> UiCaptureConfig {
        let mut config = UiCaptureConfig::new();
        config.ignored_windows = ignored.iter().map(|s| s.to_string()).collect();
        config.included_windows = included.iter().map(|s| s.to_string()).collect();
        config.excluded_apps = excluded_apps.iter().map(|s| s.to_string()).collect();
        config.compile_patterns();
        config
    }

    fn calculator() -> WindowIdentity {
        frame_identity("ApplicationFrameHost.exe", Some("CalculatorApp.exe"))
    }

    #[test]
    fn rules_written_for_the_frame_host_still_exclude_resolved_apps() {
        let title = Some("Calculator");
        // Unscoped and scoped ignore rules, and excluded apps, that name the
        // host keep excluding the app they excluded before.
        for config in [
            config(&["ApplicationFrameHost"], &[], &[]),
            config(&["applicationframehost::calc"], &[], &[]),
            config(&[], &[], &["ApplicationFrameHost"]),
        ] {
            assert!(!should_capture_identity(&config, &calculator(), title));
        }
        // A rule scoped to the host and another title leaves it alone.
        let other_title = config(&["ApplicationFrameHost::Settings"], &[], &[]);
        assert!(should_capture_identity(&other_title, &calculator(), title));
        // Rules naming the app itself work as for any other app.
        let by_app = config(&["CalculatorApp"], &[], &[]);
        assert!(!should_capture_identity(&by_app, &calculator(), title));
        // Without rules it is recorded.
        assert!(should_capture_identity(
            &config(&[], &[], &[]),
            &calculator(),
            title
        ));
    }

    #[test]
    fn the_host_name_only_ever_excludes() {
        let title = Some("Calculator");
        // An include rule for the host does not let the resolved app in:
        // that records less than before, never more.
        let include_host = config(&[], &["ApplicationFrameHost"], &[]);
        assert!(!should_capture_identity(
            &include_host,
            &calculator(),
            Some("x")
        ));
        let include_app = config(&[], &["CalculatorApp"], &[]);
        assert!(should_capture_identity(&include_app, &calculator(), title));
        // Windows without a frame host are judged on their own name only.
        let plain = WindowIdentity::plain("notepad.exe");
        let ignore_host = config(&["ApplicationFrameHost"], &[], &[]);
        assert!(should_capture_identity(&ignore_host, &plain, Some("notes")));
        assert!(!frame_host_excluded(&plain, None, &[], &[]));
        // A frame host that shows no app yet is reported under its own
        // name, which the rule matches directly.
        let unresolved = frame_identity("ApplicationFrameHost.exe", None);
        assert!(!should_capture_identity(&ignore_host, &unresolved, None));
    }
}
