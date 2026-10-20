use serde::{Deserialize, Serialize};

/// A macOS permission pane that can be opened by the plugin.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    Accessibility,
    InputMonitoring,
    ScreenRecording,
    AppManagement,
    Bluetooth,
    DeveloperTools,
    FullDiskAccess,
    MediaAppleMusic,
}

/// The current host-app status reported by macOS for a permission.
///
/// This does not describe whether an arbitrary target `appPath` already has
/// permission. It only captures what the current host app or process can
/// preflight about itself.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionAuthorizationState {
    Granted,
    NotGranted,
    Unknown,
    Checking,
}

/// Payload used to launch the floating permission guidance flow.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartFlowRequest {
    pub permission: Permission,
    pub app_path: String,
    /// Whether the current click position should be used as the animation
    /// source frame for the panel launch.
    #[serde(default = "default_use_click_source_frame")]
    pub use_click_source_frame: bool,
}

const fn default_use_click_source_frame() -> bool {
    true
}
