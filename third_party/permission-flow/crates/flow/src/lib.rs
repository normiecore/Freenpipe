//! Rust bindings for the macOS permission guidance flow shipped in this
//! workspace's Swift/AppKit implementation.
//!
//! On non-macOS targets, this crate intentionally compiles as a no-op shim so
//! cross-platform workspaces can still build. In that mode, controller methods
//! quietly succeed and permission status resolves to `Unknown`.

use std::ffi::{CStr, CString, NulError};
use std::fmt;
use std::marker::PhantomData;
use std::path::Path;
use std::rc::Rc;

#[cfg(target_os = "macos")]
use std::ffi::OsString;
#[cfg(target_os = "macos")]
use std::ffi::c_void;
#[cfg(target_os = "macos")]
use std::mem::{MaybeUninit, size_of};
#[cfg(target_os = "macos")]
use std::os::raw::{c_char, c_int};
#[cfg(target_os = "macos")]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(target_os = "macos")]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::ptr::{NonNull, null_mut};

/// Main controller for driving the Swift permission flow from Rust.
pub struct PermissionFlowController {
    #[cfg(target_os = "macos")]
    pointer: NonNull<c_void>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl PermissionFlowController {
    /// Creates a controller for starting permission flows.
    ///
    /// This must be called on the macOS main thread.
    pub fn new() -> Result<Self, NewControllerError> {
        #[cfg(target_os = "macos")]
        {
            let mut controller = null_mut();
            let status = unsafe { permission_flow_controller_new(&mut controller) };
            if status != OK_STATUS {
                assert_eq!(
                    status, NOT_MAIN_THREAD_ERROR_STATUS,
                    "The shim should only report a non-main-thread error in this version"
                );
                return Err(NewControllerError(()));
            }

            // If this ever happens, the Rust and Swift sides have drifted out of contract.
            let pointer =
                NonNull::new(controller).expect("Shim returned ok for an invalid pointer");

            Ok(Self {
                pointer,
                not_send_or_sync: PhantomData,
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok(Self {
                not_send_or_sync: PhantomData,
            })
        }
    }

    /// Starts a permission flow.
    ///
    /// The underlying library only keeps one panel open at a time, so starting
    /// a new flow closes the previous one.
    pub fn start_flow(&self, options: StartFlowOptions) -> Result<(), StartPermissionFlowError> {
        #[cfg(target_os = "macos")]
        {
            let status = unsafe {
                permission_flow_controller_start_flow(
                    self.pointer.as_ptr(),
                    options.permission.as_ffi(),
                    options.app_path.path.as_ptr(),
                    if options.use_click_source_frame { 1 } else { 0 },
                )
            };

            if status != OK_STATUS {
                Err(StartPermissionFlowError(status))
            } else {
                Ok(())
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = options;
            Ok(())
        }
    }

    /// Stops the current permission flow, if one is active.
    pub fn stop_current_flow(&self) -> Result<(), StopPermissionFlowError> {
        #[cfg(target_os = "macos")]
        {
            let status = unsafe { permission_flow_controller_close_panel(self.pointer.as_ptr()) };

            if status != OK_STATUS {
                Err(StopPermissionFlowError(status))
            } else {
                Ok(())
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok(())
        }
    }
}

// ================================================================ MODELS ================================================================ //

/// A macOS privacy permission that can be opened through the Swift shim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Permission(u8);

impl Permission {
    /// Privacy & Security > Accessibility.
    pub const ACCESSIBILITY: Permission = Permission(1);
    /// Privacy & Security > Input Monitoring.
    pub const INPUT_MONITORING: Permission = Permission(2);
    /// Privacy & Security > Screen & System Audio Recording.
    pub const SCREEN_RECORDING: Permission = Permission(3);
    /// Privacy & Security > App Management.
    pub const APP_MANAGEMENT: Permission = Permission(4);
    /// Privacy & Security > Bluetooth.
    pub const BLUETOOTH: Permission = Permission(5);
    /// Privacy & Security > Developer Tools.
    pub const DEVELOPER_TOOLS: Permission = Permission(6);
    /// Privacy & Security > Full Disk Access.
    pub const FULL_DISK_ACCESS: Permission = Permission(7);
    /// Privacy & Security > Media & Apple Music.
    pub const MEDIA_APPLE_MUSIC: Permission = Permission(8);

    #[cfg(target_os = "macos")]
    fn as_ffi(self) -> i8 {
        self.0 as i8
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::ACCESSIBILITY => "Accessibility",
            Self::INPUT_MONITORING => "Input Monitoring",
            Self::SCREEN_RECORDING => "Screen Recording",
            Self::APP_MANAGEMENT => "App Management",
            Self::BLUETOOTH => "Bluetooth",
            Self::DEVELOPER_TOOLS => "Developer Tools",
            Self::FULL_DISK_ACCESS => "Full Disk Access",
            Self::MEDIA_APPLE_MUSIC => "Media & Apple Music",
            _ => "Unknown Permission",
        }
    }

    /// Returns the host application's current authorization state for this permission.
    ///
    /// This does not report whether an arbitrary target app already has the
    /// permission. It only reports what the current host process or host app
    /// can determine about its own authorization state.
    pub fn authorization_state(
        self,
    ) -> Result<PermissionAuthorizationState, PermissionStatusError> {
        #[cfg(target_os = "macos")]
        {
            let mut state = 0;
            let status = unsafe { permission_flow_authorization_state(self.as_ffi(), &mut state) };

            if status != OK_STATUS {
                return Err(PermissionStatusError(status));
            }

            let state = match state {
                AUTHORIZATION_GRANTED_STATE => PermissionAuthorizationState::Granted,
                AUTHORIZATION_NOT_GRANTED_STATE => PermissionAuthorizationState::NotGranted,
                AUTHORIZATION_UNKNOWN_STATE => PermissionAuthorizationState::Unknown,
                AUTHORIZATION_CHECKING_STATE => PermissionAuthorizationState::Checking,
                _ => panic!("Shim returned an invalid authorization state"),
            };

            Ok(state)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok(PermissionAuthorizationState::Unknown)
        }
    }
}

/// A validated C-compatible app path passed through the FFI boundary.
#[derive(Clone)]
pub struct AppPath {
    path: CString,
}

impl AppPath {
    pub fn as_c_str(&self) -> &CStr {
        &self.path
    }

    /// Returns a best-effort guess for the app bundle users are most likely
    /// trying to grant permission to in the current launch context.
    ///
    /// If the current executable lives inside an `.app` bundle, that bundle is
    /// returned. Otherwise, this walks up the parent process chain and returns
    /// the first enclosing `.app` bundle it finds, which covers common cases
    /// like Terminal, iTerm, or an IDE-integrated terminal.
    pub fn suggested_host_app() -> Option<Self> {
        #[cfg(target_os = "macos")]
        {
            suggested_host_app_path().and_then(|path| Self::try_from(path.as_path()).ok())
        }

        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }
}

/// Options for starting a permission flow.
#[derive(Clone)]
pub struct StartFlowOptions {
    permission: Permission,
    app_path: AppPath,
    use_click_source_frame: bool,
}

impl StartFlowOptions {
    /// Creates a new set of start-flow options.
    pub fn new<P: Into<AppPath>>(permission: Permission, app_path: P) -> Self {
        Self {
            permission,
            app_path: app_path.into(),
            use_click_source_frame: true,
        }
    }

    /// Sets whether the current mouse location should be used as the source
    /// frame for the launch animation.
    pub fn use_click_source_frame(mut self, use_click_source_frame: bool) -> Self {
        self.use_click_source_frame = use_click_source_frame;
        self
    }

    /// Disables the click-source-frame launch animation.
    pub fn without_click_source_frame(mut self) -> Self {
        self.use_click_source_frame = false;
        self
    }

    pub fn permission(&self) -> Permission {
        self.permission
    }

    pub fn app_path(&self) -> &AppPath {
        &self.app_path
    }

    pub fn uses_click_source_frame(&self) -> bool {
        self.use_click_source_frame
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionAuthorizationState {
    Granted,
    NotGranted,
    Unknown,
    Checking,
}

// ================================================================ ERRORS ================================================================ //

#[derive(PartialEq, Debug)]
pub struct NewControllerError(());

#[derive(PartialEq, Debug)]
pub struct StartPermissionFlowError(i8);

#[derive(PartialEq, Debug)]
pub struct StopPermissionFlowError(i8);

#[derive(PartialEq, Debug)]
pub struct PermissionStatusError(i8);

// ================================================================ TRAIT_IMPLS ================================================================ //

impl Drop for PermissionFlowController {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: PermissionFlowController is created on the main thread. Since
            // it is neither Send nor Sync, it cannot be soundly moved away from the
            // main thread. That means Drop, and practically every other method, also
            // runs on the main thread.
            let status = unsafe { permission_flow_controller_free(self.pointer.as_ptr()) };

            debug_assert_eq!(status, OK_STATUS);
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

impl From<CString> for AppPath {
    fn from(value: CString) -> Self {
        Self { path: value }
    }
}

impl From<&CStr> for AppPath {
    fn from(value: &CStr) -> Self {
        Self {
            path: value.to_owned(),
        }
    }
}

impl TryFrom<&Path> for AppPath {
    type Error = NulError;

    fn try_from(path: &Path) -> Result<Self, NulError> {
        #[cfg(unix)]
        let bytes = path.as_os_str().as_bytes();
        #[cfg(not(unix))]
        let owned = path.to_string_lossy().into_owned();
        #[cfg(not(unix))]
        let bytes = owned.as_bytes();

        Ok(Self {
            path: CString::new(bytes)?,
        })
    }
}

impl TryFrom<&str> for AppPath {
    type Error = NulError;

    fn try_from(path: &str) -> Result<Self, NulError> {
        Ok(Self {
            path: CString::new(path.as_bytes())?,
        })
    }
}

impl fmt::Display for NewControllerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PermissionFlowController::new must be called on the main thread")
    }
}

impl fmt::Display for StartPermissionFlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PermissionFlowController::start_flow: {}",
            format_error(self.0)
        )
    }
}

impl fmt::Display for StopPermissionFlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PermissionFlowController::stop_current_flow: {}",
            format_error(self.0)
        )
    }
}

impl fmt::Display for PermissionStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Permission::authorization_state: {}",
            format_error(self.0)
        )
    }
}

impl std::error::Error for NewControllerError {}
impl std::error::Error for StartPermissionFlowError {}
impl std::error::Error for StopPermissionFlowError {}
impl std::error::Error for PermissionStatusError {}

#[cfg(target_os = "macos")]
const OK_STATUS: i8 = 0;
const INVALID_PERMISSION_ERROR_STATUS: i8 = 1;
const NULL_CONTROLLER_ERROR_STATUS: i8 = 2;
const NOT_MAIN_THREAD_ERROR_STATUS: i8 = 3;
#[cfg(target_os = "macos")]
const AUTHORIZATION_GRANTED_STATE: i8 = 0;
#[cfg(target_os = "macos")]
const AUTHORIZATION_NOT_GRANTED_STATE: i8 = 1;
#[cfg(target_os = "macos")]
const AUTHORIZATION_UNKNOWN_STATE: i8 = 2;
#[cfg(target_os = "macos")]
const AUTHORIZATION_CHECKING_STATE: i8 = 3;
#[cfg(target_os = "macos")]
const PROC_PIDTBSDINFO: c_int = 3;
#[cfg(target_os = "macos")]
const PROC_PIDPATHINFO_MAXSIZE: usize = 4096;
#[cfg(target_os = "macos")]
const MAXCOMLEN: usize = 16;

fn format_error(err: i8) -> &'static str {
    match err {
        INVALID_PERMISSION_ERROR_STATUS => "invalid permission pane",
        NULL_CONTROLLER_ERROR_STATUS => "controller pointer was null",
        NOT_MAIN_THREAD_ERROR_STATUS => "permission-flow UI APIs must run on the main thread",
        _ => "unknown error",
    }
}

#[cfg(target_os = "macos")]
fn suggested_host_app_path() -> Option<PathBuf> {
    let current_executable = std::env::current_exe().ok()?;
    if let Some(app_bundle) = enclosing_app_bundle(&current_executable) {
        return Some(app_bundle.to_path_buf());
    }

    let mut pid = std::process::id() as c_int;
    for _ in 0..32 {
        let parent = parent_pid(pid)?;
        if parent <= 1 || parent == pid {
            break;
        }

        pid = parent;
        let path = process_path(pid)?;
        if let Some(app_bundle) = enclosing_app_bundle(&path) {
            return Some(app_bundle.to_path_buf());
        }
    }

    None
}

#[cfg(any(target_os = "macos", test))]
fn enclosing_app_bundle(path: &Path) -> Option<&Path> {
    path.ancestors().find(|ancestor| {
        ancestor
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    })
}

#[cfg(target_os = "macos")]
fn parent_pid(pid: c_int) -> Option<c_int> {
    let mut info = MaybeUninit::<ProcBsdInfo>::zeroed();
    let size = size_of::<ProcBsdInfo>() as c_int;
    let result = unsafe { proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
    if result != size {
        return None;
    }

    Some(unsafe { info.assume_init() }.pbi_ppid as c_int)
}

#[cfg(target_os = "macos")]
fn process_path(pid: c_int) -> Option<PathBuf> {
    let mut buffer = [0_u8; PROC_PIDPATHINFO_MAXSIZE];
    let result = unsafe { proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if result <= 0 {
        return None;
    }

    let bytes = unsafe { CStr::from_ptr(buffer.as_ptr().cast()) }.to_bytes();
    if bytes.is_empty() {
        return None;
    }

    Some(PathBuf::from(OsString::from_vec(bytes.to_vec())))
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn permission_flow_controller_new(controller_out: *mut *mut c_void) -> i8;
    fn permission_flow_controller_free(controller: *mut c_void) -> i8;
    fn permission_flow_controller_start_flow(
        controller: *mut c_void,
        permission: i8,
        app_path: *const c_char,
        use_click_source_frame: i8,
    ) -> i8;
    fn permission_flow_controller_close_panel(controller: *mut c_void) -> i8;
    fn permission_flow_authorization_state(permission: i8, state_out: *mut i8) -> i8;
    fn proc_pidinfo(
        pid: c_int,
        flavor: c_int,
        arg: u64,
        buffer: *mut c_void,
        buffersize: c_int,
    ) -> c_int;
    fn proc_pidpath(pid: c_int, buffer: *mut c_void, buffersize: u32) -> c_int;
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct ProcBsdInfo {
    pbi_flags: u32,
    pbi_status: u32,
    pbi_xstatus: u32,
    pbi_pid: u32,
    pbi_ppid: u32,
    pbi_uid: u32,
    pbi_gid: u32,
    pbi_ruid: u32,
    pbi_rgid: u32,
    pbi_svuid: u32,
    pbi_svgid: u32,
    rfu_1: u32,
    pbi_comm: [u8; MAXCOMLEN],
    pbi_name: [u8; 2 * MAXCOMLEN],
    pbi_nfiles: u32,
    pbi_pgid: u32,
    pbi_pjobc: u32,
    e_tdev: u32,
    e_tpgid: u32,
    pbi_nice: i32,
    pbi_start_tvsec: u64,
    pbi_start_tvusec: u64,
}

#[cfg(test)]
mod tests {
    use super::{
        INVALID_PERMISSION_ERROR_STATUS, Permission, PermissionFlowController, StartFlowOptions,
        StartPermissionFlowError, enclosing_app_bundle,
    };
    use std::path::Path;

    #[test]
    #[cfg(target_os = "macos")]
    fn new_controller_returns_not_main_thread_on_worker_thread() {
        let handle = std::thread::spawn(|| PermissionFlowController::new().err());
        let result = handle.join().expect("worker thread panicked");
        assert!(result.is_some());
    }

    #[test]
    fn permission_authorization_state_is_available_on_worker_thread() {
        let handle = std::thread::spawn(|| Permission::ACCESSIBILITY.authorization_state());
        let result = handle.join().expect("worker thread panicked");

        assert!(result.is_ok());
    }

    #[test]
    fn enclosing_app_bundle_finds_nested_bundle() {
        let path = Path::new("/Applications/RustRover.app/Contents/MacOS/rustrover");
        let bundle = enclosing_app_bundle(path);

        assert_eq!(bundle, Some(Path::new("/Applications/RustRover.app")));
    }

    #[test]
    fn enclosing_app_bundle_returns_none_for_non_bundle_paths() {
        let path = Path::new("/Users/example/project/target/debug/app");
        assert_eq!(enclosing_app_bundle(path), None);
    }

    #[test]
    fn start_flow_options_use_click_source_frame_by_default() {
        let options = StartFlowOptions::new(Permission::ACCESSIBILITY, c"/Applications/Test.app");

        assert!(options.uses_click_source_frame());
    }

    #[test]
    fn start_flow_options_can_disable_click_source_frame() {
        let options = StartFlowOptions::new(Permission::ACCESSIBILITY, c"/Applications/Test.app")
            .without_click_source_frame();

        assert!(!options.uses_click_source_frame());
    }

    #[test]
    fn permission_display_names_are_human_readable() {
        assert_eq!(
            Permission::MEDIA_APPLE_MUSIC.to_string(),
            "Media & Apple Music"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "requires the macOS main thread, which the Rust test harness does not guarantee"]
    fn start_controller_does_not_panic_on_invalid_permission() {
        let controller = PermissionFlowController::new().unwrap();
        let err = controller.start_flow(
            StartFlowOptions::new(Permission(15), c"This App").without_click_source_frame(),
        );

        assert_eq!(
            err,
            Err(StartPermissionFlowError(INVALID_PERMISSION_ERROR_STATUS))
        );
    }
}
