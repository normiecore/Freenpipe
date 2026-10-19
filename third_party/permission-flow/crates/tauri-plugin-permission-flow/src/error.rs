use std::ffi::NulError;

use serde::{Serialize, ser::Serializer};

/// Result type used by the Tauri plugin surface.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("permission-flow is only available on macOS")]
    UnsupportedPlatform,
    #[error("appPath contains an embedded NUL byte")]
    InvalidAppPath(#[from] NulError),
    #[error("failed to receive a result from the main thread")]
    MainThreadChannelClosed,
    #[error("permission flow handle has already been closed")]
    DriverClosed,
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
    // The native `permission_flow` crate is a macOS-only dependency, so its
    // error types only exist there. Gating the variants keeps `Error` buildable
    // on non-macOS targets (Windows/Linux) where the plugin compiles as a stub.
    #[cfg(target_os = "macos")]
    #[error(transparent)]
    NewController(#[from] permission_flow::NewControllerError),
    #[cfg(target_os = "macos")]
    #[error(transparent)]
    StartFlow(#[from] permission_flow::StartPermissionFlowError),
    #[cfg(target_os = "macos")]
    #[error(transparent)]
    StopFlow(#[from] permission_flow::StopPermissionFlowError),
    #[cfg(target_os = "macos")]
    #[error(transparent)]
    PermissionStatus(#[from] permission_flow::PermissionStatusError),
    #[cfg(mobile)]
    #[error(transparent)]
    PluginInvoke(#[from] tauri::plugin::mobile::PluginInvokeError),
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_ref())
    }
}
