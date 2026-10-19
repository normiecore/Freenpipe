use tauri::{AppHandle, Manager, ResourceId, Runtime, command};

use crate::PermissionFlowExt;
use crate::Result;
#[cfg(desktop)]
use crate::desktop::PermissionFlowHandle;
use crate::models::*;

/// Creates a new controller handle and returns its Tauri resource id.
#[command]
pub(crate) async fn create<R: Runtime>(app: AppHandle<R>) -> Result<ResourceId> {
    app.permission_flow().create()
}

/// Returns the current host-app status for a permission.
#[command]
pub(crate) async fn authorization_state<R: Runtime>(
    app: AppHandle<R>,
    permission: Permission,
) -> Result<PermissionAuthorizationState> {
    app.permission_flow().authorization_state(permission)
}

/// Returns a best-effort guess for the host app bundle path in the current
/// launch context.
#[command]
pub(crate) async fn suggested_host_app_path<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<String>> {
    app.permission_flow().suggested_host_app_path()
}

/// Starts the floating guidance flow using an existing controller handle.
#[command]
pub(crate) async fn start_flow<R: Runtime>(
    app: AppHandle<R>,
    rid: ResourceId,
    payload: StartFlowRequest,
) -> Result<()> {
    #[cfg(desktop)]
    {
        let handle = app.resources_table().get::<PermissionFlowHandle<R>>(rid)?;
        app.permission_flow()
            .start_flow(handle.controller_id(), payload)
    }

    #[cfg(not(desktop))]
    {
        let _ = rid;
        app.permission_flow().start_flow(0, payload)
    }
}

/// Stops the active floating guidance flow for an existing controller handle.
#[command]
pub(crate) async fn stop_current_flow<R: Runtime>(
    app: AppHandle<R>,
    rid: ResourceId,
) -> Result<()> {
    #[cfg(desktop)]
    {
        let handle = app.resources_table().get::<PermissionFlowHandle<R>>(rid)?;
        app.permission_flow()
            .stop_current_flow(handle.controller_id())
    }

    #[cfg(not(desktop))]
    {
        let _ = rid;
        app.permission_flow().stop_current_flow(0)
    }
}
