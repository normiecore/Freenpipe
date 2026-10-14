// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Enterprise UI policy for Freenpipe: there is none.
//!
//! The base app compiled this module from a proprietary directory that is
//! not part of the fork. Freenpipe has no managed deployment mode, so every
//! answer here is the consumer one: the app UI is visible and no tray item is
//! hidden. The two commands remain registered so the generated TypeScript
//! bindings and any frontend code that still calls them keep working; they
//! accept their arguments and do nothing.

use tracing::debug;

/// True when a managed deployment wants the whole app UI hidden (tray-only,
/// headless recording). Freenpipe never hides its UI.
pub fn is_app_ui_hidden() -> bool {
    false
}

/// True when a managed deployment hides the tray item with this id
/// (`tray_chat`, `tray_search`, `tray_timeline`, `tray_plan`,
/// `tray_recording_controls`, `tray_settings`). Freenpipe shows them all.
pub fn is_tray_item_hidden(_item_id: &str) -> bool {
    false
}

/// Accepts a list of settings sections a managed policy would hide.
/// Freenpipe has no remote policy source, so the list is ignored.
#[tauri::command]
#[specta::specta]
pub fn set_enterprise_policy(hidden_sections: Vec<String>) {
    debug!(
        "set_enterprise_policy ignored ({} hidden sections): no managed mode in Freenpipe",
        hidden_sections.len()
    );
}

/// Accepts per-stream switches for a managed upload channel. Freenpipe
/// uploads nothing, so the switches are ignored.
#[tauri::command]
#[specta::specta]
pub fn set_sync_streams(
    frames: bool,
    audio: bool,
    ui_events: bool,
    memories: bool,
    snapshots: bool,
) {
    debug!(
        "set_sync_streams ignored (frames={frames} audio={audio} ui_events={ui_events} \
         memories={memories} snapshots={snapshots}): no managed upload in Freenpipe"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumer_policy_never_hides_anything() {
        assert!(!is_app_ui_hidden());
        for id in [
            "tray_chat",
            "tray_search",
            "tray_timeline",
            "tray_plan",
            "tray_recording_controls",
            "tray_settings",
        ] {
            assert!(!is_tray_item_hidden(id), "{id} must stay visible");
        }
    }

    #[test]
    fn commands_are_harmless_no_ops() {
        set_enterprise_policy(vec!["tray_chat".into(), "settings".into()]);
        set_sync_streams(true, true, true, true, true);
        assert!(!is_tray_item_hidden("tray_chat"));
        assert!(!is_app_ui_hidden());
    }
}
