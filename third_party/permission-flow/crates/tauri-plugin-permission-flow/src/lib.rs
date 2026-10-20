use tauri::{
    Manager, Runtime,
    plugin::{Builder, TauriPlugin},
};

// Platform-specific backends.
#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

// Shared public surface.
mod commands;
mod error;
mod models;

pub use error::{Error, Result};
pub use models::*;

#[cfg(desktop)]
use desktop::PermissionFlowPlugin;
#[cfg(mobile)]
use mobile::PermissionFlowPlugin;

/// Extensions to [`tauri::App`], [`tauri::AppHandle`] and [`tauri::Window`]
/// for accessing the plugin state from Rust.
pub trait PermissionFlowExt<R: Runtime> {
    fn permission_flow(&self) -> &PermissionFlowPlugin<R>;
}

impl<R: Runtime, T: Manager<R>> crate::PermissionFlowExt<R> for T {
    fn permission_flow(&self) -> &PermissionFlowPlugin<R> {
        self.state::<PermissionFlowPlugin<R>>().inner()
    }
}

/// Initializes the plugin and registers its command surface.
///
/// The command order mirrors the intended public story:
/// 1. create a handle when you want to keep a controller alive,
/// 2. query host-app status when needed,
/// 3. start or stop the floating guidance flow through that handle.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("permission-flow")
        .invoke_handler(tauri::generate_handler![
            commands::create,
            commands::suggested_host_app_path,
            commands::authorization_state,
            commands::start_flow,
            commands::stop_current_flow
        ])
        .setup(|app, api| {
            #[cfg(mobile)]
            let permission_flow = mobile::init(app, api)?;
            #[cfg(desktop)]
            let permission_flow = desktop::init(app, api)?;
            app.manage(permission_flow);
            Ok(())
        })
        .build()
}
