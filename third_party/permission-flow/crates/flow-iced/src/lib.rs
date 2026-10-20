//! Iced-friendly, headless button model for `permission-flow`.
//!
//! `PermissionFlowButton` owns its own controller so each button can manage an
//! independent permission-flow lifecycle without relying on a global controller.
//!
//! Important:
//! The status returned by [`Permission::authorization_state`] reflects what the
//! current host process or host app can determine about its own permissions.
//! It does not verify that the arbitrary `.app` bundle shown in the floating
//! permission flow already has that permission.

use std::time::Duration;
use std::{ffi::NulError, path::Path};

use iced::{Event, Subscription, event, time, window};
pub use permission_flow::{
    AppPath, NewControllerError, Permission, PermissionAuthorizationState,
    PermissionFlowController, PermissionStatusError, StartFlowOptions, StartPermissionFlowError,
    StopPermissionFlowError,
};

/// Headless iced-friendly model for a permission-flow button.
///
/// The model eagerly owns a `PermissionFlowController`, so each button can keep
/// its own permission-flow lifecycle without relying on global state.
///
/// Important:
/// The stored authorization state is host-app/process-centric. If the
/// configured `app_path` points at some other app bundle, the state is still
/// about the current host application, not that target bundle.
pub struct PermissionFlowButton {
    controller: PermissionFlowController,
    options: StartFlowOptions,
    authorization_state: PermissionAuthorizationState,
}

impl PermissionFlowButton {
    /// Creates a button model from a string app bundle path.
    pub fn new(permission: Permission, app_path: &str) -> Result<Self, PermissionFlowButtonError> {
        let app_path = AppPath::try_from(app_path)?;
        Self::with_options(StartFlowOptions::new(permission, app_path))
    }

    /// Creates a button model from a filesystem path.
    pub fn new_from_path(
        permission: Permission,
        app_path: &Path,
    ) -> Result<Self, PermissionFlowButtonError> {
        let app_path = AppPath::try_from(app_path)?;
        Self::with_options(StartFlowOptions::new(permission, app_path))
    }

    /// Creates a button model from prebuilt start-flow options.
    pub fn with_options(options: StartFlowOptions) -> Result<Self, PermissionFlowButtonError> {
        let controller = PermissionFlowController::new()?;
        let authorization_state = options.permission().authorization_state()?;

        Ok(Self {
            controller,
            options,
            authorization_state,
        })
    }

    /// Starts the permission flow for this button's configured permission.
    pub fn press(&self) -> Result<(), StartPermissionFlowError> {
        self.controller.start_flow(self.options.clone())
    }

    /// Alias for `press` when a flow-centric name reads better at the callsite.
    pub fn start_flow(&self) -> Result<(), StartPermissionFlowError> {
        self.press()
    }

    /// Stops the current permission flow, if one is active.
    pub fn stop_current_flow(&self) -> Result<(), StopPermissionFlowError> {
        self.controller.stop_current_flow()
    }

    /// Refreshes the current host application's authorization status.
    pub fn refresh(&mut self) -> Result<(), PermissionStatusError> {
        self.authorization_state = self.options.permission().authorization_state()?;
        Ok(())
    }

    /// Refreshes status when an iced window regains focus.
    ///
    /// Returns `true` when a refresh happened.
    pub fn refresh_on_window_event(
        &mut self,
        event: &window::Event,
    ) -> Result<bool, PermissionStatusError> {
        if matches!(event, window::Event::Focused) {
            self.refresh()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Refreshes status when an iced event indicates the window regained focus.
    ///
    /// Returns `true` when a refresh happened.
    pub fn refresh_on_event(&mut self, event: &Event) -> Result<bool, PermissionStatusError> {
        match event {
            Event::Window(event) => self.refresh_on_window_event(event),
            _ => Ok(false),
        }
    }

    /// Returns the built-in refresh subscription for this button model.
    ///
    /// This combines:
    /// - a lightweight periodic refresh
    /// - a refresh when the window regains focus
    pub fn subscription(&self) -> Subscription<PermissionFlowButtonMessage> {
        self.subscription_with_interval(Duration::from_secs(1))
    }

    /// Returns the built-in refresh subscription with a custom polling interval.
    pub fn subscription_with_interval(
        &self,
        interval: Duration,
    ) -> Subscription<PermissionFlowButtonMessage> {
        Subscription::batch([
            event::listen_with(|event, _status, _window| match event {
                Event::Window(_) => Some(PermissionFlowButtonMessage::RuntimeEvent(event)),
                _ => None,
            }),
            time::every(interval).map(|_| PermissionFlowButtonMessage::RefreshTick),
        ])
    }

    /// Applies an internal button message and returns whether a refresh happened.
    pub fn update(
        &mut self,
        message: &PermissionFlowButtonMessage,
    ) -> Result<bool, PermissionStatusError> {
        match message {
            PermissionFlowButtonMessage::RefreshTick => {
                self.refresh()?;
                Ok(true)
            }
            PermissionFlowButtonMessage::RuntimeEvent(event) => self.refresh_on_event(event),
        }
    }

    /// Returns the current button presentation state.
    pub fn button_state(&self) -> PermissionFlowButtonState {
        PermissionFlowButtonState::from(self.authorization_state)
    }

    /// Returns the last known host-application authorization state.
    pub fn authorization_state(&self) -> PermissionAuthorizationState {
        self.authorization_state
    }

    /// Returns the configured start-flow options.
    pub fn options(&self) -> &StartFlowOptions {
        &self.options
    }
}

/// Messages produced by the built-in `PermissionFlowButton` refresh logic.
#[derive(Debug, Clone)]
pub enum PermissionFlowButtonMessage {
    RefreshTick,
    RuntimeEvent(Event),
}

/// Simple button state for rendering a permission-flow button in iced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionFlowButtonState {
    Granted,
    GrantAccess,
    OpenSettings,
    Checking,
}

impl PermissionFlowButtonState {
    pub fn from_authorization_state(state: PermissionAuthorizationState) -> Self {
        match state {
            PermissionAuthorizationState::Granted => Self::Granted,
            PermissionAuthorizationState::NotGranted => Self::GrantAccess,
            PermissionAuthorizationState::Unknown => Self::OpenSettings,
            PermissionAuthorizationState::Checking => Self::Checking,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Granted => "Granted",
            Self::GrantAccess => "Grant Access",
            Self::OpenSettings => "Open Settings",
            Self::Checking => "Checking...",
        }
    }

    pub fn is_granted(self) -> bool {
        matches!(self, Self::Granted)
    }
}

impl From<PermissionAuthorizationState> for PermissionFlowButtonState {
    fn from(state: PermissionAuthorizationState) -> Self {
        Self::from_authorization_state(state)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PermissionFlowButtonError {
    #[error(transparent)]
    InvalidAppPath(#[from] NulError),
    #[error(transparent)]
    NewController(#[from] NewControllerError),
    #[error(transparent)]
    PermissionStatus(#[from] PermissionStatusError),
}

#[cfg(test)]
mod tests {
    use super::{PermissionAuthorizationState, PermissionFlowButtonState};

    #[test]
    fn granted_state_maps_to_granted_button_state() {
        let state = PermissionFlowButtonState::from_authorization_state(
            PermissionAuthorizationState::Granted,
        );

        assert_eq!(state, PermissionFlowButtonState::Granted);
        assert!(state.is_granted());
    }

    #[test]
    fn not_granted_state_maps_to_call_to_action() {
        let state = PermissionFlowButtonState::from_authorization_state(
            PermissionAuthorizationState::NotGranted,
        );

        assert_eq!(state, PermissionFlowButtonState::GrantAccess);
        assert!(!state.is_granted());
    }
}
