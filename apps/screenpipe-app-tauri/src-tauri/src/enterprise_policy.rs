// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Enterprise UI policy for Freenpipe: there is none.
//!
//! The base app compiled this module from a proprietary directory that is
//! not part of the fork. Freenpipe has no managed deployment mode, so every
//! answer here is the consumer one: the app UI is visible and no tray item is
//! hidden. The commands remain registered so the generated TypeScript
//! bindings and any frontend code that still calls them keep working; they
//! accept their arguments and do nothing.
//!
//! screenpipe v2.7 added more commands to this family (UI visibility, host
//! identity for the admin console, recording authorisation, managed team
//! skills, history-access limits, activity sync). Freenpipe registers them
//! with the same names and argument names so a frontend written against
//! screenpipe's bindings keeps compiling, and answers each one for a personal
//! install. None of them reads machine identifiers, stores credentials or
//! opens a network connection.

use serde::{Deserialize, Serialize};
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

/// Re-applies a managed policy's window and tray visibility, then reports
/// whether the app UI ended up hidden. Freenpipe has no policy, so nothing
/// changes and the answer is always false.
#[tauri::command]
#[specta::specta]
pub fn apply_enterprise_ui_visibility() -> bool {
    is_app_ui_hidden()
}

/// Hashed identifiers an admin console would use to tell seats apart.
///
/// Both fields are always `null` in Freenpipe: it never derives a stable
/// machine or OS-user identifier, hashed or not, because nothing it runs
/// needs to recognise this computer from the outside.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EnterpriseHostIdentity {
    pub machine_id_hash: Option<String>,
    pub os_user_id_hash: Option<String>,
}

/// Identity of this computer for a managed deployment. Always empty.
#[tauri::command]
#[specta::specta]
pub fn get_enterprise_host_identity() -> EnterpriseHostIdentity {
    EnterpriseHostIdentity::default()
}

/// Accepts an admin console's verdict on whether this seat may record,
/// together with the credential it checked. Recording in Freenpipe is
/// controlled only by the user, so the verdict changes nothing, and the
/// credential is dropped without being stored or logged.
#[tauri::command]
#[specta::specta]
pub fn set_enterprise_recording_authorized(
    authorized: bool,
    credential_type: Option<String>,
    credential: Option<String>,
) -> Result<(), String> {
    debug!(
        "set_enterprise_recording_authorized ignored (authorized={authorized}, \
         credential type {:?}, credential {}): recording is user-controlled in Freenpipe",
        credential_type,
        if credential.is_some() {
            "supplied and discarded"
        } else {
            "absent"
        }
    );
    Ok(())
}

/// Whether reads of older history are refused on this install. Answered by
/// the engine's history-access policy, which never limits history, so this
/// is always false.
#[tauri::command]
#[specta::specta]
pub fn is_history_access_restricted() -> bool {
    screenpipe_engine::history_access::is_restricted()
}

/// A skill an organisation installed on this machine and keeps in sync.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ManagedTeamSkill {
    /// Folder name of the skill, as the skills commands list it.
    pub name: String,
}

/// Skills an organisation manages on this machine. Freenpipe has no
/// organisations, so the list is always empty.
#[tauri::command]
#[specta::specta]
pub fn list_managed_team_skills() -> Vec<ManagedTeamSkill> {
    screenpipe_core::pipes::managed::managed_team_skill_names()
        .into_iter()
        .map(|name| ManagedTeamSkill { name })
        .collect()
}

/// Accepts an organisation's skill list and whether skills missing from it
/// should be removed. Freenpipe installs nothing from a policy and removes
/// nothing, so the request is ignored. The entries are taken as plain JSON
/// so a payload in any shape is accepted rather than failing to parse.
#[tauri::command]
#[specta::specta]
pub fn sync_managed_team_skills(
    skills: Vec<serde_json::Value>,
    prune_unlisted: bool,
) -> Result<(), String> {
    debug!(
        "sync_managed_team_skills ignored ({} skills, prune_unlisted={prune_unlisted}): \
         no managed skills in Freenpipe",
        skills.len()
    );
    Ok(())
}

/// Turns the managed activity upload on or off. Freenpipe has no upload
/// worker, so the switch is ignored and nothing leaves the machine.
#[tauri::command]
#[specta::specta]
pub fn set_activity_sync_enabled(enabled: bool) -> Result<(), String> {
    debug!(
        "set_activity_sync_enabled ignored (enabled={enabled}): no activity upload in Freenpipe"
    );
    Ok(())
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

    #[test]
    fn ui_visibility_stays_visible() {
        assert!(!apply_enterprise_ui_visibility());
        assert!(!is_app_ui_hidden());
    }

    #[test]
    fn host_identity_is_empty_and_serializes_as_nulls() {
        let identity = get_enterprise_host_identity();
        assert_eq!(identity.machine_id_hash, None);
        assert_eq!(identity.os_user_id_hash, None);
        assert_eq!(
            serde_json::to_value(&identity).unwrap(),
            serde_json::json!({ "machine_id_hash": null, "os_user_id_hash": null })
        );
    }

    #[test]
    fn recording_authorisation_is_accepted_and_ignored() {
        assert_eq!(
            set_enterprise_recording_authorized(
                false,
                Some("license_key".into()),
                Some("ENT-0000-0000-0000-0000".into())
            ),
            Ok(())
        );
        assert_eq!(
            set_enterprise_recording_authorized(true, None, None),
            Ok(())
        );
    }

    #[test]
    fn history_access_is_never_restricted() {
        assert!(!is_history_access_restricted());
    }

    #[test]
    fn no_managed_team_skills() {
        assert!(list_managed_team_skills().is_empty());
        let pushed = vec![
            serde_json::json!({ "name": "org-skill", "version": "1" }),
            serde_json::json!("not even an object"),
        ];
        assert_eq!(sync_managed_team_skills(pushed, true), Ok(()));
        assert!(list_managed_team_skills().is_empty());
    }

    #[test]
    fn activity_sync_switch_is_inert() {
        assert_eq!(set_activity_sync_enabled(true), Ok(()));
        assert_eq!(set_activity_sync_enabled(false), Ok(()));
    }
}
