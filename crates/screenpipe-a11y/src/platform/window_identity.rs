// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

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

/// The app name to report for a UWP frame: the hosted app's process name
/// when it is known, else the frame host's own name. Never empty.
pub fn frame_app_name(host_process: &str, hosted_process: Option<&str>) -> String {
    match hosted_process
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        Some(hosted) => hosted.to_string(),
        None => non_empty_app_name(host_process),
    }
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
            frame_app_name("ApplicationFrameHost.exe", Some("CalculatorApp.exe")),
            "CalculatorApp.exe"
        );
        assert_eq!(
            frame_app_name("ApplicationFrameHost.exe", None),
            "ApplicationFrameHost.exe"
        );
        assert_eq!(
            frame_app_name("ApplicationFrameHost.exe", Some("  ")),
            "ApplicationFrameHost.exe"
        );
        assert_eq!(frame_app_name("", None), UNKNOWN_APP_NAME);
        assert_eq!(non_empty_app_name(" notepad.exe "), "notepad.exe");
    }
}
