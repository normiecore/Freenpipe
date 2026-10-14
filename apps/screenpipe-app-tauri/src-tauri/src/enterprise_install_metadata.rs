// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! How this copy of the app was installed, as far as update management is
//! concerned.
//!
//! The base app compiled this module from a proprietary directory that is
//! not part of the fork. Freenpipe keeps the same type (the frontend and
//! `updates.rs` read it) but reports an unmanaged install with no detection
//! evidence. Nothing here leaves the machine.

use serde::{Deserialize, Serialize};

/// Install and update-management facts. The field names match what older
/// builds persisted in settings (`enterpriseInstallMetadata`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EnterpriseInstallMetadata {
    /// Where the install came from (installer, package manager, ...).
    pub install_source: String,
    /// Which tool is responsible for updating this install.
    pub update_manager: String,
    /// True when an external manager (MDM, package manager) owns updates.
    pub managed: bool,
    /// Signals that led to the `managed` verdict, for display only.
    pub detected_by: Vec<String>,
}

impl EnterpriseInstallMetadata {
    /// The answer for an ordinary, self-updating install.
    pub fn unmanaged() -> Self {
        Self {
            install_source: "unknown".to_string(),
            update_manager: "unknown".to_string(),
            managed: false,
            detected_by: Vec::new(),
        }
    }
}

impl Default for EnterpriseInstallMetadata {
    fn default() -> Self {
        Self::unmanaged()
    }
}

// Also called directly as a plain function: updates.rs reads `.managed`.
/// How this install is updated. Freenpipe always reports an unmanaged,
/// self-updating install.
#[tauri::command]
#[specta::specta]
pub fn get_enterprise_install_metadata() -> EnterpriseInstallMetadata {
    EnterpriseInstallMetadata::unmanaged()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_an_unmanaged_install() {
        let m = get_enterprise_install_metadata();
        assert!(!m.managed);
        assert_eq!(m.install_source, "unknown");
        assert_eq!(m.update_manager, "unknown");
        assert!(m.detected_by.is_empty());
        assert_eq!(m, EnterpriseInstallMetadata::default());
    }

    #[test]
    fn serializes_with_the_frontend_field_names() {
        let v = serde_json::to_value(get_enterprise_install_metadata()).unwrap();
        let obj = v.as_object().unwrap();
        let mut keys: Vec<_> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["detected_by", "install_source", "managed", "update_manager"]
        );
    }
}
