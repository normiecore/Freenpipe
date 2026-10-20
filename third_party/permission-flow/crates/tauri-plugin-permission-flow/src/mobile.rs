use serde::de::DeserializeOwned;
use tauri::{
    AppHandle, ResourceId, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::Error;
use crate::models::*;

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_permission_flow);

/// Initializes the mobile plugin binding.
///
/// The public crate currently targets the macOS desktop experience, so the
/// mobile backend remains a thin placeholder until a mobile flow exists.
pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> crate::Result<PermissionFlowPlugin<R>> {
    #[cfg(target_os = "android")]
    let handle = api.register_android_plugin("", "PermissionFlowPlugin")?;
    #[cfg(target_os = "ios")]
    let handle = api.register_ios_plugin(init_plugin_permission_flow)?;
    Ok(PermissionFlowPlugin(handle))
}

/// Access to the mobile permission-flow APIs.
pub struct PermissionFlowPlugin<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> PermissionFlowPlugin<R> {
    /// Mobile does not currently expose a controller-handle story.
    pub fn create(&self) -> crate::Result<ResourceId> {
        Err(Error::UnsupportedPlatform)
    }

    /// Reserved for a future mobile implementation.
    pub fn start_flow(&self, controller_id: u64, payload: StartFlowRequest) -> crate::Result<()> {
        let _ = controller_id;
        self.0
            .run_mobile_plugin("start_flow", payload)
            .map_err(Into::into)
    }

    /// Reserved for a future mobile implementation.
    pub fn stop_current_flow(&self, controller_id: u64) -> crate::Result<()> {
        let _ = controller_id;
        self.0
            .run_mobile_plugin("stop_current_flow", ())
            .map_err(Into::into)
    }

    /// Reserved for a future mobile implementation.
    pub fn authorization_state(
        &self,
        permission: Permission,
    ) -> crate::Result<PermissionAuthorizationState> {
        let _ = permission;
        Err(Error::UnsupportedPlatform)
    }

    /// Returns `None` because mobile platforms do not currently expose a host
    /// app bundle path in this plugin.
    pub fn suggested_host_app_path(&self) -> crate::Result<Option<String>> {
        Ok(None)
    }
}
