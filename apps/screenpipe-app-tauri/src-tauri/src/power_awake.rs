// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Keep computer awake (`keepComputerAwake`, default off).
//!
//! The hold itself lives in the engine
//! (`screenpipe_engine::power::keep_awake`, thread `screenpipe-keep-awake`).
//! This module applies the saved setting at startup, exposes
//! `set_keep_awake` for the settings switch, and lets the quit teardown
//! release the hold (see `relaunch.rs`).
//!
//! `set_keep_awake` only applies the value; the settings page saves
//! `keepComputerAwake` itself, the same way it saves every other setting.

use std::time::Duration;

use screenpipe_engine::power::keep_awake::{self, StopOutcome};
use tauri::AppHandle;
use tracing::{debug, info, warn};

use crate::relaunch::StepOutcome;
use crate::store::SettingsStore;

/// How long the quit teardown waits for the hold to be released.
pub const RELEASE_LIMIT: Duration = Duration::from_secs(2);

/// Turn keep-awake on or off now.
#[tauri::command]
#[specta::specta]
pub async fn set_keep_awake(enabled: bool) -> Result<(), String> {
    match tauri::async_runtime::spawn_blocking(move || keep_awake::set_keep_awake(enabled)).await {
        Ok(Ok(())) => {
            info!("keep computer awake set to {enabled}");
            Ok(())
        }
        Ok(Err(e)) => {
            warn!("failed to apply the keep computer awake setting ({enabled}): {e}");
            Err(e.to_string())
        }
        Err(e) => {
            warn!("failed to apply the keep computer awake setting ({enabled}): {e}");
            Err(format!("keep-awake task failed: {e}"))
        }
    }
}

/// Apply the saved `keepComputerAwake` at startup. Does nothing when it is
/// off, so the keep-awake thread only exists for users who turned it on.
pub fn apply_saved_setting(app: &AppHandle) {
    let enabled = SettingsStore::get(app)
        .ok()
        .flatten()
        .map(|s| s.recording.keep_computer_awake)
        .unwrap_or(false);
    if !enabled {
        debug!("keep computer awake is off");
        return;
    }
    tauri::async_runtime::spawn_blocking(|| match keep_awake::set_keep_awake(true) {
        Ok(()) => info!("keep computer awake applied from settings"),
        Err(e) => warn!("failed to apply the saved keep computer awake setting: {e}"),
    });
}

/// The quit teardown's `keep_awake` step: give the hold back, waiting up to
/// [`RELEASE_LIMIT`] for the keep-awake thread.
pub async fn release_for_teardown() -> StepOutcome {
    let release =
        tauri::async_runtime::spawn_blocking(|| keep_awake::release_for_exit(RELEASE_LIMIT));
    teardown_outcome(release.await.ok())
}

/// How a release reads in the teardown report (`None`: the task failed).
fn teardown_outcome(release: Option<StopOutcome>) -> StepOutcome {
    match release {
        Some(StopOutcome::Stopped) => StepOutcome::Done,
        Some(StopOutcome::NotStarted) => StepOutcome::Skipped,
        Some(StopOutcome::TimedOut) | None => StepOutcome::TimedOut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teardown_reports_the_release() {
        assert_eq!(teardown_outcome(Some(StopOutcome::Stopped)), StepOutcome::Done);
        assert_eq!(
            teardown_outcome(Some(StopOutcome::NotStarted)),
            StepOutcome::Skipped
        );
        assert_eq!(
            teardown_outcome(Some(StopOutcome::TimedOut)),
            StepOutcome::TimedOut
        );
        assert_eq!(teardown_outcome(None), StepOutcome::TimedOut);
    }

    #[test]
    fn the_teardown_budget_has_room_for_the_release() {
        let budget = crate::relaunch::TeardownBudget::default();
        assert!(budget.keep_awake > RELEASE_LIMIT);
        assert!(budget.keep_awake < budget.overall);
    }
}
