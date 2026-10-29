// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Start at login (shell-020).
//!
//! The login entry starts the app with [`AUTOSTART_ARG`], which tells it to
//! come up in the background: tray and engine start, Home stays closed
//! (unless onboarding is unfinished). A second `--autostart` launch while the
//! app already runs is ignored by the focus handoff instead of opening Home.
//!
//! - Windows: a value named after the product under
//!   `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, holding
//!   `"<exe>" --autostart` (quoted, so a path with spaces still starts). It is
//!   written here rather than through the autostart plugin, whose value is
//!   unquoted and which also re-enables the entry in Task Manager on every
//!   call. A user who turned the entry off under Task Manager > Startup apps
//!   (`Explorer\StartupApproved\Run`) keeps that choice at every start;
//!   only switching autostart on in Handvel's settings turns it back on.
//! - macOS and Linux: the autostart plugin (login LaunchAgent, XDG autostart
//!   entry), configured with the same argument.
//! - Debug builds never touch the login entry, so a development build cannot
//!   take over the entry of the installed app.

use tracing::info;

/// Argument the login entry passes.
pub const AUTOSTART_ARG: &str = "--autostart";

/// True when this process was started by the login entry. `args` is the full
/// command line; the program name in `args[0]` is skipped.
pub fn is_autostart_launch<S: AsRef<str>>(args: &[S]) -> bool {
    args.iter().skip(1).any(|a| a.as_ref() == AUTOSTART_ARG)
}

/// Debug builds keep away from the OS login entry (dev isolation).
pub fn dev_isolation() -> bool {
    cfg!(debug_assertions) || std::env::var("TAURI_ENV_DEBUG").as_deref() == Ok("true")
}

/// Command line the Windows Run value holds for `exe`.
#[cfg_attr(not(windows), allow(dead_code))] // used by the Windows login entry
pub fn run_command(exe: &str) -> String {
    format!("\"{exe}\" {AUTOSTART_ARG}")
}

/// How an existing login entry compares with the one this build wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryState {
    Missing,
    /// Same command (compared without regard to case, as Windows paths are).
    Current,
    /// Another command: an older build without the argument, an unquoted
    /// path, or another install location.
    Outdated,
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn entry_state(existing: Option<&str>, expected: &str) -> EntryState {
    match existing.map(str::trim) {
        None | Some("") => EntryState::Missing,
        Some(value) if value.eq_ignore_ascii_case(expected.trim()) => EntryState::Current,
        Some(_) => EntryState::Outdated,
    }
}

/// What the start-up sync does with the login entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupAction {
    /// Debug build: leave the entry alone.
    SkipDevIsolation,
    Register,
    /// Rewrite an outdated entry, keeping the Task Manager state.
    Refresh,
    Keep,
    Remove,
    Nothing,
}

pub fn startup_action(enabled: bool, dev_isolation: bool, state: EntryState) -> StartupAction {
    if dev_isolation {
        return StartupAction::SkipDevIsolation;
    }
    match (enabled, state) {
        (true, EntryState::Missing) => StartupAction::Register,
        (true, EntryState::Outdated) => StartupAction::Refresh,
        (true, EntryState::Current) => StartupAction::Keep,
        (false, EntryState::Missing) => StartupAction::Nothing,
        (false, _) => StartupAction::Remove,
    }
}

/// `StartupApproved\Run` data: an odd first byte means the user turned the
/// entry off in Task Manager (even means on).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn approved_value_disabled(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(|b| b & 1 == 1)
}

/// Bring the login entry in line with the `autoStartEnabled` setting at
/// start-up. Never re-enables an entry the user turned off in Task Manager.
pub fn sync_at_startup(app: &tauri::AppHandle, enabled: bool) {
    platform::sync_at_startup(app, enabled, dev_isolation());
}

/// The user switched autostart in settings.
pub fn set_enabled(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    if dev_isolation() {
        info!(
            "autostart {} requested; skipping login entry registration under dev isolation (debug build)",
            if enabled { "on" } else { "off" }
        );
        return Ok(());
    }
    platform::set_enabled(app, enabled)
}

#[cfg(windows)]
mod platform {
    use super::{entry_state, run_command, startup_action, StartupAction};
    use tracing::{debug, info, warn};
    use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    use winreg::{RegKey, RegValue};

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    /// "Enabled" in Task Manager's format: flag 2, no disable time.
    const APPROVED_ENABLED: [u8; 12] = [0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

    fn entry_name(app: &tauri::AppHandle) -> String {
        app.package_info().name.clone()
    }

    fn expected_command() -> Result<String, String> {
        let exe =
            std::env::current_exe().map_err(|e| format!("cannot resolve the executable: {e}"))?;
        Ok(run_command(&exe.display().to_string()))
    }

    fn read_entry(name: &str) -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_KEY, KEY_READ)
            .ok()?
            .get_value::<String, _>(name)
            .ok()
    }

    fn write_entry(name: &str, command: &str) -> Result<(), String> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(RUN_KEY)
            .map_err(|e| format!("cannot open the Run key: {e}"))?;
        key.set_value(name, &command.to_string())
            .map_err(|e| format!("cannot write the Run value: {e}"))
    }

    fn remove_entry(name: &str) -> Result<(), String> {
        let key = match RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)
        {
            Ok(key) => key,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("cannot open the Run key: {e}")),
        };
        match key.delete_value(name) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("cannot remove the Run value: {e}")),
        }
    }

    fn turned_off_in_task_manager(name: &str) -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(APPROVED_KEY, KEY_READ)
            .and_then(|key| key.get_raw_value(name))
            .map(|value| super::approved_value_disabled(&value.bytes))
            .unwrap_or(false)
    }

    /// Mark the entry enabled in Task Manager. Only for an explicit "on"
    /// from the user.
    fn approve(name: &str) {
        let Ok(key) =
            RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(APPROVED_KEY, KEY_SET_VALUE)
        else {
            return;
        };
        let value = RegValue {
            vtype: RegType::REG_BINARY,
            bytes: APPROVED_ENABLED.to_vec(),
        };
        if let Err(e) = key.set_raw_value(name, &value) {
            warn!("autostart: could not mark the entry enabled in Task Manager: {e}");
        }
    }

    pub(super) fn sync_at_startup(app: &tauri::AppHandle, enabled: bool, dev_isolation: bool) {
        let name = entry_name(app);
        let expected = match expected_command() {
            Ok(command) => command,
            Err(e) => {
                warn!("autostart: {e}");
                return;
            }
        };
        let existing = read_entry(&name);
        let action = startup_action(
            enabled,
            dev_isolation,
            entry_state(existing.as_deref(), &expected),
        );
        let result = match action {
            StartupAction::SkipDevIsolation => {
                info!("autostart: skipping login entry registration under dev isolation (debug build)");
                Ok(())
            }
            StartupAction::Register | StartupAction::Refresh => write_entry(&name, &expected),
            StartupAction::Keep | StartupAction::Nothing => Ok(()),
            StartupAction::Remove => remove_entry(&name),
        };
        match result {
            Ok(()) => debug!("autostart: {action:?} for {name:?}"),
            Err(e) => warn!("autostart: {action:?} failed: {e}"),
        }
        if enabled && !dev_isolation && turned_off_in_task_manager(&name) {
            info!(
                "autostart is turned off in Task Manager's startup apps; keeping the user's choice"
            );
        }
    }

    pub(super) fn set_enabled(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
        let name = entry_name(app);
        if enabled {
            write_entry(&name, &expected_command()?)?;
            approve(&name);
        } else {
            remove_entry(&name)?;
        }
        info!(
            "autostart {}: login entry {}",
            if enabled { "enabled" } else { "disabled" },
            if read_entry(&name).is_some() {
                "present"
            } else {
                "absent"
            }
        );
        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{startup_action, EntryState, StartupAction};
    use tauri_plugin_autostart::ManagerExt as _;
    use tracing::{debug, info, warn};

    pub(super) fn sync_at_startup(app: &tauri::AppHandle, enabled: bool, dev_isolation: bool) {
        let manager = app.autolaunch();
        // The plugin cannot read the entry's arguments back, so an existing
        // entry is rewritten every start: that is how one from an older
        // build gains `--autostart`.
        let state = match manager.is_enabled() {
            Ok(true) if enabled => EntryState::Outdated,
            Ok(true) => EntryState::Current,
            _ => EntryState::Missing,
        };
        let action = startup_action(enabled, dev_isolation, state);
        let result = match action {
            StartupAction::SkipDevIsolation => {
                info!("autostart: skipping login entry registration under dev isolation (debug build)");
                Ok(())
            }
            StartupAction::Register | StartupAction::Refresh => manager.enable(),
            StartupAction::Remove => manager.disable(),
            StartupAction::Keep | StartupAction::Nothing => Ok(()),
        };
        match result {
            Ok(()) => debug!("autostart: {action:?}"),
            Err(e) => warn!("autostart: {action:?} failed: {e}"),
        }
    }

    pub(super) fn set_enabled(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
        let manager = app.autolaunch();
        if enabled {
            manager.enable().map_err(|e| e.to_string())?;
        } else if manager.is_enabled().unwrap_or(true) {
            manager.disable().map_err(|e| e.to_string())?;
        }
        info!(
            "autostart {}: is_enabled={}",
            if enabled { "enabled" } else { "disabled" },
            manager.is_enabled().unwrap_or(false)
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_launch_is_recognised_after_the_program_name() {
        assert!(is_autostart_launch(&["C:\\Handvel.exe", "--autostart"]));
        assert!(is_autostart_launch(&[
            "/usr/bin/handvel",
            "--foo",
            "--autostart"
        ]));
        assert!(!is_autostart_launch(&["--autostart"]));
        assert!(!is_autostart_launch(&["handvel", "--autostart=1"]));
        assert!(!is_autostart_launch::<&str>(&[]));
        assert!(!is_autostart_launch(&["handvel", "handvel://settings"]));
    }

    #[test]
    fn run_value_quotes_the_executable() {
        assert_eq!(
            run_command(r"C:\Users\Ada Lovelace\AppData\Local\Handvel\Handvel.exe"),
            r#""C:\Users\Ada Lovelace\AppData\Local\Handvel\Handvel.exe" --autostart"#
        );
    }

    #[test]
    fn entry_state_compares_like_windows_paths() {
        let expected = run_command(r"C:\Apps\Handvel\Handvel.exe");
        assert_eq!(entry_state(None, &expected), EntryState::Missing);
        assert_eq!(entry_state(Some("  "), &expected), EntryState::Missing);
        assert_eq!(
            entry_state(
                Some(r#""c:\apps\handvel\HANDVEL.EXE" --autostart "#),
                &expected
            ),
            EntryState::Current
        );
        // What older builds wrote: unquoted, no argument.
        assert_eq!(
            entry_state(Some(r"C:\Apps\Handvel\Handvel.exe "), &expected),
            EntryState::Outdated
        );
        assert_eq!(
            entry_state(Some(r#""D:\Other\Handvel.exe" --autostart"#), &expected),
            EntryState::Outdated
        );
    }

    #[test]
    fn startup_sync_follows_the_setting_and_respects_dev_isolation() {
        use EntryState::*;
        use StartupAction::*;
        for state in [Missing, Current, Outdated] {
            assert_eq!(startup_action(true, true, state), SkipDevIsolation);
            assert_eq!(startup_action(false, true, state), SkipDevIsolation);
        }
        assert_eq!(startup_action(true, false, Missing), Register);
        assert_eq!(startup_action(true, false, Outdated), Refresh);
        assert_eq!(startup_action(true, false, Current), Keep);
        assert_eq!(startup_action(false, false, Missing), Nothing);
        assert_eq!(startup_action(false, false, Current), Remove);
        assert_eq!(startup_action(false, false, Outdated), Remove);
    }

    #[test]
    fn task_manager_state_is_read_from_the_first_byte() {
        assert!(!approved_value_disabled(&[
            0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]));
        assert!(!approved_value_disabled(&[
            0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]));
        assert!(approved_value_disabled(&[
            0x03, 0x10, 0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]));
        assert!(approved_value_disabled(&[0x07]));
        assert!(!approved_value_disabled(&[]));
    }

    #[test]
    fn debug_test_builds_are_dev_isolated() {
        if cfg!(debug_assertions) {
            assert!(dev_isolation());
        }
    }
}
