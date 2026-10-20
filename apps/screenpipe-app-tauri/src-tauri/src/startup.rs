// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Decisions the app makes while it starts (shell-019, shell-020,
//! shell-021, shell-024), kept apart from `main.rs` so they can be tested.
//!
//! - Helper modes: the app executable doubles as a few command-line helpers.
//!   `main` looks at `argv[1]` before anything else (no Tauri, no
//!   single-instance check, no store, no focus port) and runs the helper
//!   instead of the app ([`dispatch_helper_mode`]). The table itself lives in
//!   `main.rs`; the features behind the flags are built by other work
//!   packages, and until then each flag ends with a usage message.
//! - Which window opens first ([`startup_window`]): none for a login launch
//!   (`--autostart`) or for a safe relaunch of an instance whose Home was
//!   closed, onboarding while it is unfinished, Home otherwise.
//! - `SCREENPIPE_SKIP_ONBOARDING` ([`skip_onboarding_requested`]).
//! - Runtime file-system and asset-protocol access for a data directory
//!   outside the static scopes in `capabilities/main.json` and
//!   `tauri.conf.json` ([`dirs_needing_scope`], [`grant_data_dir_scopes`]).

use std::path::{Path, PathBuf};

use tracing::{info, warn};

/// Exit status of a helper flag whose feature this build does not have, or
/// that was called with the wrong arguments.
pub const HELPER_USAGE_EXIT: i32 = 2;

/// One helper mode of the executable.
pub struct HelperMode {
    /// Exact value of `argv[1]` that selects it.
    pub flag: &'static str,
    /// Runs the helper with the arguments after the flag; returns the exit
    /// status of the process.
    pub run: fn(&[String]) -> i32,
}

/// The helper selected by `args` (the full command line), if any.
pub fn helper_mode_for<'a>(args: &[String], table: &'a [HelperMode]) -> Option<&'a HelperMode> {
    let first = args.get(1)?;
    table.iter().find(|mode| mode.flag == first.as_str())
}

/// Run the helper `args` selects. `None` means "start the app"; `Some`
/// carries the exit status the process must end with.
pub fn dispatch_helper_mode(args: &[String], table: &[HelperMode]) -> Option<i32> {
    let mode = helper_mode_for(args, table)?;
    Some((mode.run)(&args[2..]))
}

/// Usage text for a helper flag this build cannot run yet.
fn unavailable(flag: &str, purpose: &str, contract: &str) -> i32 {
    eprintln!(
        "{product}: {flag} is not available in this build.\n\
         It {purpose}.\n\
         {contract}\n\
         The app itself starts when this flag is left out.",
        product = crate::brand::PRODUCT_NAME,
    );
    HELPER_USAGE_EXIT
}

/// `--screenpipe-acp-runtime`: an ACP agent runtime for the app's agents.
pub fn acp_runtime_unavailable(_args: &[String]) -> i32 {
    unavailable(
        "--screenpipe-acp-runtime",
        "runs one ACP coding agent on behalf of the app",
        "It is configured through SCREENPIPE_ACP_ID, SCREENPIPE_ACP_CWD, SCREENPIPE_BUN_PATH, \
         SCREENPIPE_ACP_ARGS_JSON, SCREENPIPE_ACP_ENV_JSON and SCREENPIPE_ACP_UNATTENDED.",
    )
}

/// `--screenpipe-acp-process-guard`: keeps an agent's child processes tied
/// to the app.
pub fn acp_process_guard_unavailable(_args: &[String]) -> i32 {
    unavailable(
        "--screenpipe-acp-process-guard",
        "watches an agent's child processes and ends them when the app goes away",
        "It is started by the app, not by hand.",
    )
}

/// `--screenpipe-portable-mcp`: the bundled MCP server over HTTP.
pub fn portable_mcp_unavailable(_args: &[String]) -> i32 {
    unavailable(
        "--screenpipe-portable-mcp",
        "serves the bundled MCP server for AI clients over HTTP",
        "It reads SCREENPIPE_MCP_TRANSPORT (http), SCREENPIPE_MCP_HOST (127.0.0.1), \
         SCREENPIPE_MCP_PORT and SCREENPIPE_MCP_PATH (/mcp).",
    )
}

/// `--render-workflow-video <project dir>`: errors go to stdout as one JSON
/// object, which is what the agent extension that calls it reads.
pub fn render_workflow_video_unavailable(args: &[String]) -> i32 {
    let message = if args.len() != 1 {
        "expected exactly one argument: the workflow project directory"
    } else {
        "rendering workflow videos is not available in this build"
    };
    println!("{}", serde_json::json!({ "error": message }));
    HELPER_USAGE_EXIT
}

/// Why no window opens at start-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoWindowReason {
    /// Managed deployment with the UI hidden.
    HiddenUi,
    /// Safe relaunch of an instance whose Home was closed.
    RelaunchHomeClosed,
    /// Started by the login entry.
    Autostart,
}

/// Window to open when the app starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupWindow {
    None(NoWindowReason),
    Onboarding,
    Home,
}

/// Inputs of [`startup_window`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StartupInputs {
    pub ui_hidden: bool,
    pub onboarding_completed: bool,
    /// Started with `--autostart`.
    pub autostart: bool,
    /// `relaunch::relaunch_home_visible()`: `Some` only for the replacement
    /// started by a safe relaunch.
    pub relaunch_home_visible: Option<bool>,
}

/// Which window the app opens first. Unfinished onboarding always shows, even
/// for a login launch: otherwise a user who never finished it would only ever
/// see a tray icon.
pub fn startup_window(inputs: StartupInputs) -> StartupWindow {
    if inputs.ui_hidden {
        return StartupWindow::None(NoWindowReason::HiddenUi);
    }
    if !inputs.onboarding_completed {
        return StartupWindow::Onboarding;
    }
    match inputs.relaunch_home_visible {
        Some(false) => StartupWindow::None(NoWindowReason::RelaunchHomeClosed),
        Some(true) => StartupWindow::Home,
        None if inputs.autostart => StartupWindow::None(NoWindowReason::Autostart),
        None => StartupWindow::Home,
    }
}

/// `SCREENPIPE_SKIP_ONBOARDING` set to a true value (`1`, `true`, `yes`,
/// `on`), or the `onboarding` end-to-end seed.
pub fn skip_onboarding_requested(env_value: Option<&str>, e2e_flags: &[String]) -> bool {
    let from_env = env_value.is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    });
    from_env || e2e_flags.iter().any(|f| f == "onboarding")
}

/// Directories among `dirs` that the static scopes do not cover: not inside
/// any of `static_roots`. Duplicates are dropped.
pub fn dirs_needing_scope(dirs: &[&Path], static_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        let covered = static_roots.iter().any(|root| dir.starts_with(root));
        if !covered && !out.iter().any(|d| d == dir) {
            out.push(dir.to_path_buf());
        }
    }
    out
}

/// `~/.screenpipe`, covered by the static fs and asset scopes.
pub fn static_scope_roots() -> Vec<PathBuf> {
    dirs::home_dir()
        .map(|home| vec![home.join(".screenpipe")])
        .unwrap_or_default()
}

/// Let the webviews read `dirs` (frames, media) through the fs plugin and
/// the asset protocol. Failures are logged; the app keeps starting.
pub fn grant_data_dir_scopes(app: &tauri::AppHandle, dirs: &[PathBuf]) {
    use tauri::Manager;
    use tauri_plugin_fs::FsExt;
    for dir in dirs {
        match app.try_fs_scope() {
            Some(scope) => {
                if let Err(e) = scope.allow_directory(dir, true) {
                    warn!("could not add {} to the fs scope: {e}", dir.display());
                }
            }
            None => warn!(
                "could not add {} to the fs scope: the fs plugin is not ready",
                dir.display()
            ),
        }
        if let Err(e) = app.asset_protocol_scope().allow_directory(dir, true) {
            warn!(
                "could not add {} to the asset protocol scope: {e}",
                dir.display()
            );
        }
        info!("data directory {} added to the webview scopes", dir.display());
    }
}

/// Tell the user that another install refused this launch, then the caller
/// exits. A login launch stays silent (it would pop up at every sign-in).
pub fn report_refused_handoff(message: &str, autostart: bool) {
    let text = format!(
        "{product} is already running from a different install, so this copy will not start.\n\n\
         Quit the other copy first, or start the one that is already running.\n\n({message})",
        product = crate::brand::PRODUCT_NAME,
    );
    eprintln!("{text}");
    if autostart {
        return;
    }
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MESSAGEBOX_STYLE,
        };
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let body = wide(&text);
        let title = wide(crate::brand::PRODUCT_NAME);
        // SAFETY: both strings are NUL-terminated and outlive the call.
        unsafe {
            MessageBoxW(
                HWND::default(),
                PCWSTR(body.as_ptr()),
                PCWSTR(title.as_ptr()),
                MESSAGEBOX_STYLE(MB_OK.0 | MB_ICONWARNING.0 | MB_SETFOREGROUND.0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    fn record(args: &[String]) -> i32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        10 + args.len() as i32
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    const TABLE: &[HelperMode] = &[
        HelperMode {
            flag: "--screenpipe-portable-mcp",
            run: record,
        },
        HelperMode {
            flag: "--render-workflow-video",
            run: render_workflow_video_unavailable,
        },
    ];

    #[test]
    fn helper_flag_must_be_the_first_argument() {
        assert!(helper_mode_for(&args(&["app", "--screenpipe-portable-mcp"]), TABLE).is_some());
        assert!(helper_mode_for(&args(&["app"]), TABLE).is_none());
        assert!(helper_mode_for(&args(&[]), TABLE).is_none());
        // Later positions are ordinary app arguments (a deep link, --autostart).
        assert!(helper_mode_for(&args(&["app", "--autostart", "--screenpipe-portable-mcp"]), TABLE).is_none());
        assert!(helper_mode_for(&args(&["app", "--screenpipe-portable-mcp=1"]), TABLE).is_none());
    }

    #[test]
    fn dispatch_runs_the_helper_with_the_remaining_arguments() {
        let before = CALLS.load(Ordering::SeqCst);
        assert_eq!(
            dispatch_helper_mode(&args(&["app", "--screenpipe-portable-mcp", "a", "b"]), TABLE),
            Some(12)
        );
        assert_eq!(CALLS.load(Ordering::SeqCst), before + 1);
        assert_eq!(dispatch_helper_mode(&args(&["app", "freenpipe://x"]), TABLE), None);
    }

    #[test]
    fn unavailable_helpers_exit_with_the_usage_status() {
        assert_eq!(acp_runtime_unavailable(&[]), HELPER_USAGE_EXIT);
        assert_eq!(acp_process_guard_unavailable(&[]), HELPER_USAGE_EXIT);
        assert_eq!(portable_mcp_unavailable(&args(&["x"])), HELPER_USAGE_EXIT);
        assert_eq!(render_workflow_video_unavailable(&[]), HELPER_USAGE_EXIT);
        assert_eq!(render_workflow_video_unavailable(&args(&["/p"])), HELPER_USAGE_EXIT);
        assert_eq!(render_workflow_video_unavailable(&args(&["/p", "/q"])), HELPER_USAGE_EXIT);
    }

    #[test]
    fn startup_window_rules() {
        use NoWindowReason::*;
        let done = StartupInputs {
            onboarding_completed: true,
            ..Default::default()
        };
        assert_eq!(startup_window(done), StartupWindow::Home);
        assert_eq!(
            startup_window(StartupInputs { autostart: true, ..done }),
            StartupWindow::None(Autostart)
        );
        assert_eq!(
            startup_window(StartupInputs { ui_hidden: true, ..done }),
            StartupWindow::None(HiddenUi)
        );
        assert_eq!(
            startup_window(StartupInputs {
                relaunch_home_visible: Some(false),
                ..done
            }),
            StartupWindow::None(RelaunchHomeClosed)
        );
        assert_eq!(
            startup_window(StartupInputs {
                relaunch_home_visible: Some(true),
                autostart: true,
                ..done
            }),
            StartupWindow::Home
        );
        // Unfinished onboarding wins over a background launch.
        assert_eq!(
            startup_window(StartupInputs {
                autostart: true,
                relaunch_home_visible: Some(false),
                ..Default::default()
            }),
            StartupWindow::Onboarding
        );
        assert_eq!(
            startup_window(StartupInputs {
                ui_hidden: true,
                ..Default::default()
            }),
            StartupWindow::None(HiddenUi)
        );
    }

    #[test]
    fn skip_onboarding_values() {
        for value in ["1", "true", "TRUE", " yes ", "on"] {
            assert!(skip_onboarding_requested(Some(value), &[]), "{value}");
        }
        for value in ["", "0", "false", "no", "off", "2"] {
            assert!(!skip_onboarding_requested(Some(value), &[]), "{value}");
        }
        assert!(!skip_onboarding_requested(None, &[]));
        assert!(skip_onboarding_requested(None, &args(&["no-audio", "onboarding"])));
        assert!(!skip_onboarding_requested(None, &args(&["no-audio"])));
    }

    #[test]
    fn only_directories_outside_the_static_scopes_are_granted() {
        let roots = vec![PathBuf::from("/home/ada/.screenpipe")];
        let base = Path::new("/home/ada/.screenpipe");
        let custom = Path::new("/mnt/recordings");
        let nested = Path::new("/home/ada/.screenpipe/data");
        let lookalike = Path::new("/home/ada/.screenpipe-old");
        assert_eq!(dirs_needing_scope(&[base, nested], &roots), Vec::<PathBuf>::new());
        assert_eq!(
            dirs_needing_scope(&[base, custom, custom, lookalike], &roots),
            vec![PathBuf::from("/mnt/recordings"), PathBuf::from("/home/ada/.screenpipe-old")]
        );
        assert_eq!(dirs_needing_scope(&[custom], &[]), vec![PathBuf::from("/mnt/recordings")]);
    }
}

#[cfg(test)]
mod capability_tests {
    /// URL patterns of `opener:allow-open-url` in capabilities/main.json.
    fn opener_patterns() -> Vec<glob::Pattern> {
        let caps: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/main.json")).unwrap();
        let entry = caps["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["identifier"] == "opener:allow-open-url")
            .expect("opener:allow-open-url entry");
        entry["allow"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| glob::Pattern::new(a["url"].as_str().unwrap()).unwrap())
            .collect()
    }

    fn allowed(url: &str) -> bool {
        opener_patterns().iter().any(|p| p.matches(url))
    }

    #[test]
    fn opener_hands_prompts_to_coding_agents() {
        assert!(allowed("claude://claude.ai/new?q=summarise%20my%20day"));
        assert!(allowed("cursor://anysphere.cursor-deeplink/prompt?text=fix%20the%20bug"));
        assert!(allowed("codex://threads/new?prompt=write%20tests"));
        assert!(allowed("obsidian://open?vault=notes&file=today"));
        assert!(allowed("https://github.com/normiecore/Freenpipe"));
        assert!(allowed("mailto:someone@example.test"));
    }

    #[test]
    fn opener_refuses_other_agent_links() {
        assert!(!allowed("claude://claude.ai/settings"));
        assert!(!allowed("claude://evil.example/new?q=x"));
        assert!(!allowed("cursor://anysphere.cursor-deeplink/mcp/install?name=x"));
        assert!(!allowed("codex://threads/42"));
        assert!(!allowed("file:///C:/Windows/System32/calc.exe"));
        assert!(!allowed("ms-settings:privacy"));
    }
}
