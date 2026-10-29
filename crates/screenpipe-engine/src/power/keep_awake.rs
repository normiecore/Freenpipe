// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Keep the computer awake (`keepComputerAwake`, `set_keep_awake`).
//!
//! One dedicated thread, `screenpipe-keep-awake`, owns the operating-system
//! hold. It takes the hold when asked to enable, gives it back when asked to
//! disable or to stop, and gives it back too if its owner goes away. Every
//! call into the OS happens on that thread, which matters on Windows: the
//! `SetThreadExecutionState` fallback belongs to the thread that set it.
//! Callers wait for the thread to confirm, up to [`CONFIRM_TIMEOUT`]. An
//! enable that is not confirmed in time is undone: a release is queued
//! behind it, so a caller told "it failed" never leaves the computer held.
//!
//! "Awake" means the system does not sleep because the user is idle. The
//! display may still turn off.
//! - Windows: a power request (`PowerCreateRequest`, then `PowerSetRequest`
//!   with `PowerRequestSystemRequired`). `powercfg /requests` lists it under
//!   SYSTEM with Handvel's reason text. If no request can be made, the
//!   thread sets `ES_CONTINUOUS | ES_SYSTEM_REQUIRED` with
//!   `SetThreadExecutionState` instead.
//! - macOS: a `caffeinate -i -w <pid>` child, which also ends if this
//!   process dies.
//! - Linux: `systemd-inhibit --what=idle:sleep` around a `cat` that reads a
//!   pipe from this process; the inhibitor ends as soon as the pipe closes,
//!   including when this process dies.
//!
//! The OS releases every one of these when the process ends, so a crash
//! never leaves the machine unable to sleep.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use once_cell::sync::Lazy;
use tracing::{debug, info, warn};

/// Name of the thread that holds the assertion.
pub const THREAD_NAME: &str = "screenpipe-keep-awake";
/// How long a caller waits for the thread to confirm a change.
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(3);
/// Reason shown by the OS next to the hold, where it shows one.
pub const HOLD_REASON: &str = "Handvel keeps this computer awake (Keep computer awake is on)";

/// The OS side of the hold. Created and used only on the keep-awake thread.
pub trait AwakeBackend {
    /// Take the hold. Called only while it is not held.
    fn hold(&mut self) -> Result<(), String>;
    /// Give the hold back. Called only while it is held.
    fn release(&mut self) -> Result<(), String>;
}

/// Builds the backend on the keep-awake thread.
pub type BackendFactory = dyn Fn() -> Box<dyn AwakeBackend> + Send + Sync;

/// Why a change did not take effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeepAwakeError {
    /// The thread did not answer in time. A disable may still take effect
    /// later; an enable is undone (see [`KeepAwake::set_enabled`]).
    Timeout(Duration),
    /// The OS refused.
    Backend(String),
    /// The thread could not be started or has gone away.
    Thread(String),
}

impl std::fmt::Display for KeepAwakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout(limit) => write!(
                f,
                "the {THREAD_NAME} thread did not confirm within {}s",
                limit.as_secs_f32()
            ),
            Self::Backend(e) => write!(f, "{e}"),
            Self::Thread(e) => write!(f, "keep-awake thread unavailable: {e}"),
        }
    }
}

impl std::error::Error for KeepAwakeError {}

/// What [`KeepAwake::stop`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// The thread was never started (nothing was ever held).
    NotStarted,
    /// The thread let go of any hold and ended.
    Stopped,
    /// The thread did not answer in time.
    TimedOut,
}

enum Request {
    Set {
        enabled: bool,
        reply: mpsc::SyncSender<Result<(), String>>,
    },
    Stop {
        reply: mpsc::SyncSender<()>,
    },
}

struct Worker {
    tx: mpsc::Sender<Request>,
    thread: thread::JoinHandle<()>,
}

/// Handle on the keep-awake thread. Most callers use [`global`].
pub struct KeepAwake {
    factory: Arc<BackendFactory>,
    confirm_timeout: Duration,
    worker: Mutex<Option<Worker>>,
    enabled: AtomicBool,
}

impl KeepAwake {
    /// A handle that uses this platform's backend.
    pub fn new() -> Self {
        Self::with_backend(platform_backend)
    }

    /// A handle with a custom backend (tests).
    pub fn with_backend<F>(factory: F) -> Self
    where
        F: Fn() -> Box<dyn AwakeBackend> + Send + Sync + 'static,
    {
        Self {
            factory: Arc::new(factory),
            confirm_timeout: CONFIRM_TIMEOUT,
            worker: Mutex::new(None),
            enabled: AtomicBool::new(false),
        }
    }

    /// Change how long callers wait for the thread.
    pub fn with_confirm_timeout(mut self, timeout: Duration) -> Self {
        self.confirm_timeout = timeout;
        self
    }

    /// Whether the last confirmed change turned the hold on.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Turn the hold on or off and wait for the thread to confirm. Blocks
    /// for up to the confirm timeout, so async callers run it on a blocking
    /// thread.
    ///
    /// When an enable times out, the thread may still take the hold once it
    /// gets to the request. A release is then queued right behind it, so the
    /// hold ends up off, matching both the error and [`is_enabled`](Self::is_enabled).
    pub fn set_enabled(&self, enabled: bool) -> Result<(), KeepAwakeError> {
        let mut worker = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        if !enabled && worker.is_none() {
            // Nothing was ever held.
            self.enabled.store(false, Ordering::SeqCst);
            return Ok(());
        }
        if worker.is_none() {
            *worker = Some(self.spawn()?);
        }
        let (reply, answer) = mpsc::sync_channel(1);
        let sent = worker
            .as_ref()
            .map(|w| w.tx.send(Request::Set { enabled, reply }).is_ok())
            .unwrap_or(false);
        if !sent {
            *worker = None;
            return Err(KeepAwakeError::Thread("it has stopped".into()));
        }
        match answer.recv_timeout(self.confirm_timeout) {
            Ok(Ok(())) => {
                self.enabled.store(enabled, Ordering::SeqCst);
                Ok(())
            }
            Ok(Err(e)) => Err(KeepAwakeError::Backend(e)),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                warn!(
                    "keep-awake: timed out after {}s waiting for the {THREAD_NAME} thread to {}",
                    self.confirm_timeout.as_secs_f32(),
                    if enabled { "enable" } else { "disable" }
                );
                if enabled {
                    // Nobody waits for this answer: the receiver is dropped
                    // at once and the thread's reply goes nowhere.
                    let (undo, _) = mpsc::sync_channel(1);
                    let queued = worker.as_ref().is_some_and(|w| {
                        w.tx.send(Request::Set {
                            enabled: false,
                            reply: undo,
                        })
                        .is_ok()
                    });
                    if queued {
                        debug!("keep-awake: queued a release behind the unconfirmed enable");
                    }
                }
                Err(KeepAwakeError::Timeout(self.confirm_timeout))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                *worker = None;
                Err(KeepAwakeError::Thread("it ended unexpectedly".into()))
            }
        }
    }

    /// Give back any hold and end the thread (app quit). Waits up to
    /// `timeout`.
    pub fn stop(&self, timeout: Duration) -> StopOutcome {
        let worker = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(worker) = worker else {
            return StopOutcome::NotStarted;
        };
        self.enabled.store(false, Ordering::SeqCst);
        let (reply, answer) = mpsc::sync_channel(1);
        if worker.tx.send(Request::Stop { reply }).is_err() {
            // The thread already ended, and released on its way out.
            let _ = worker.thread.join();
            return StopOutcome::Stopped;
        }
        match answer.recv_timeout(timeout) {
            Ok(()) => {
                let _ = worker.thread.join();
                StopOutcome::Stopped
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = worker.thread.join();
                StopOutcome::Stopped
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                warn!(
                    "keep-awake: the {THREAD_NAME} thread did not stop within {}s",
                    timeout.as_secs_f32()
                );
                StopOutcome::TimedOut
            }
        }
    }

    fn spawn(&self) -> Result<Worker, KeepAwakeError> {
        let (tx, rx) = mpsc::channel();
        let factory = Arc::clone(&self.factory);
        let thread = thread::Builder::new()
            .name(THREAD_NAME.to_string())
            .spawn(move || run_thread(factory(), rx))
            .map_err(|e| KeepAwakeError::Thread(e.to_string()))?;
        Ok(Worker { tx, thread })
    }
}

impl Default for KeepAwake {
    fn default() -> Self {
        Self::new()
    }
}

fn run_thread(mut backend: Box<dyn AwakeBackend>, rx: mpsc::Receiver<Request>) {
    let mut held = false;
    while let Ok(request) = rx.recv() {
        match request {
            Request::Set { enabled, reply } => {
                let result = match (enabled, held) {
                    (true, false) => backend.hold().map(|()| {
                        held = true;
                        info!("keep-awake enabled: the computer will not sleep while idle");
                    }),
                    (false, true) => backend.release().map(|()| {
                        held = false;
                        info!("keep-awake disabled: idle sleep is allowed again");
                    }),
                    _ => Ok(()),
                };
                if let Err(e) = &result {
                    warn!(
                        "keep-awake: could not {} the hold: {e}",
                        if enabled { "take" } else { "release" }
                    );
                }
                let _ = reply.send(result);
            }
            Request::Stop { reply } => {
                if held {
                    release_on_exit(backend.as_mut());
                }
                let _ = reply.send(());
                return;
            }
        }
    }
    // The owner went away without a stop.
    if held {
        release_on_exit(backend.as_mut());
    }
}

fn release_on_exit(backend: &mut dyn AwakeBackend) {
    match backend.release() {
        Ok(()) => info!("keep-awake released"),
        Err(e) => warn!("keep-awake: could not release the hold: {e}"),
    }
}

static GLOBAL: Lazy<KeepAwake> = Lazy::new(KeepAwake::new);

/// The process-wide handle.
pub fn global() -> &'static KeepAwake {
    &GLOBAL
}

/// Turn the process-wide hold on or off (blocking, see
/// [`KeepAwake::set_enabled`]).
pub fn set_keep_awake(enabled: bool) -> Result<(), KeepAwakeError> {
    global().set_enabled(enabled)
}

/// Give back the process-wide hold for a quit (blocking, up to `timeout`).
/// Cheap when keep-awake was never turned on.
pub fn release_for_exit(timeout: Duration) -> StopOutcome {
    if Lazy::get(&GLOBAL).is_none() {
        return StopOutcome::NotStarted;
    }
    let outcome = global().stop(timeout);
    debug!("keep-awake release for exit: {outcome:?}");
    outcome
}

// ---------------------------------------------------------------------------
// Platform backends
// ---------------------------------------------------------------------------

fn platform_backend() -> Box<dyn AwakeBackend> {
    Box::new(os::Backend::default())
}

#[cfg(windows)]
mod os {
    use super::{AwakeBackend, HOLD_REASON};
    use tracing::warn;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Power::{
        PowerClearRequest, PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest,
        SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
    };
    use windows::Win32::System::Threading::{
        POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
    };

    /// `POWER_REQUEST_CONTEXT_VERSION`; the named constant sits behind a
    /// `windows` feature the engine does not otherwise need.
    const REASON_CONTEXT_VERSION: u32 = 0;

    pub struct Backend {
        request: Option<HANDLE>,
        execution_state: bool,
        reason: Vec<u16>,
    }

    impl Default for Backend {
        fn default() -> Self {
            Self {
                request: None,
                execution_state: false,
                reason: HOLD_REASON.encode_utf16().chain(Some(0)).collect(),
            }
        }
    }

    impl Backend {
        fn power_request(&mut self) -> Result<HANDLE, String> {
            let context = REASON_CONTEXT {
                Version: REASON_CONTEXT_VERSION,
                Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
                Reason: REASON_CONTEXT_0 {
                    SimpleReasonString: PWSTR(self.reason.as_mut_ptr()),
                },
            };
            // SAFETY: `context` and the reason string it points to are alive
            // for the call; the OS copies the string.
            let handle = unsafe { PowerCreateRequest(&context) }
                .map_err(|e| format!("PowerCreateRequest failed: {e}"))?;
            // SAFETY: `handle` is the request just created.
            if let Err(e) = unsafe { PowerSetRequest(handle, PowerRequestSystemRequired) } {
                // SAFETY: the handle is ours and not used afterwards.
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(format!("PowerSetRequest failed: {e}"));
            }
            Ok(handle)
        }
    }

    impl AwakeBackend for Backend {
        fn hold(&mut self) -> Result<(), String> {
            match self.power_request() {
                Ok(handle) => {
                    self.request = Some(handle);
                    return Ok(());
                }
                Err(e) => warn!("keep-awake: {e}; falling back to SetThreadExecutionState"),
            }
            // SAFETY: plain flag call; it applies to the calling thread.
            let previous = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
            if previous.0 == 0 {
                return Err("SetThreadExecutionState failed to set ES_SYSTEM_REQUIRED".into());
            }
            self.execution_state = true;
            Ok(())
        }

        fn release(&mut self) -> Result<(), String> {
            let mut problems = Vec::new();
            if let Some(handle) = self.request.take() {
                // SAFETY: `handle` is our live power request.
                if let Err(e) = unsafe { PowerClearRequest(handle, PowerRequestSystemRequired) } {
                    problems.push(format!("PowerClearRequest failed: {e}"));
                }
                // SAFETY: as above; it is not used after this.
                if let Err(e) = unsafe { CloseHandle(handle) } {
                    problems.push(format!("closing the power request failed: {e}"));
                }
            }
            if self.execution_state {
                self.execution_state = false;
                // SAFETY: clears the flags this thread set.
                if unsafe { SetThreadExecutionState(ES_CONTINUOUS) }.0 == 0 {
                    problems.push("SetThreadExecutionState failed to clear the flags".into());
                }
            }
            if problems.is_empty() {
                Ok(())
            } else {
                Err(problems.join("; "))
            }
        }
    }

    impl Drop for Backend {
        fn drop(&mut self) {
            if self.request.is_some() || self.execution_state {
                let _ = self.release();
            }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod os {
    use super::AwakeBackend;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    /// How long a just-started helper must stay up to count as holding.
    const SETTLE: Duration = Duration::from_millis(200);
    /// How long a released helper may take to exit before it is killed.
    const EXIT_WAIT: Duration = Duration::from_secs(2);

    #[derive(Default)]
    pub struct Backend {
        child: Option<Child>,
    }

    #[cfg(target_os = "macos")]
    fn helper() -> Command {
        let mut cmd = Command::new("/usr/bin/caffeinate");
        // -i: no idle sleep; -w: end when this process ends.
        cmd.arg("-i")
            .arg("-w")
            .arg(std::process::id().to_string())
            .stdin(Stdio::null());
        cmd
    }

    #[cfg(target_os = "linux")]
    fn helper() -> Command {
        let mut cmd = Command::new("systemd-inhibit");
        cmd.arg("--what=idle:sleep")
            .arg(format!("--who={}", screenpipe_core::brand::PRODUCT_NAME))
            .arg(format!("--why={}", super::HOLD_REASON))
            .arg("--mode=block")
            // `cat` keeps the inhibitor alive until our end of its stdin closes.
            .arg("cat")
            .stdin(Stdio::piped());
        cmd
    }

    fn wait_for_exit(child: &mut Child, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    impl AwakeBackend for Backend {
        fn hold(&mut self) -> Result<(), String> {
            let mut cmd = helper();
            let program = cmd.get_program().to_string_lossy().into_owned();
            let mut child = cmd
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("could not start {program}: {e}"))?;
            std::thread::sleep(SETTLE);
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!("{program} exited at once ({status})"));
            }
            self.child = Some(child);
            Ok(())
        }

        fn release(&mut self) -> Result<(), String> {
            let Some(mut child) = self.child.take() else {
                return Ok(());
            };
            // Closing stdin ends the Linux helper; caffeinate needs a signal.
            drop(child.stdin.take());
            if cfg!(target_os = "linux") && wait_for_exit(&mut child, EXIT_WAIT) {
                return Ok(());
            }
            child
                .kill()
                .map_err(|e| format!("could not stop the keep-awake helper: {e}"))?;
            let _ = child.wait();
            Ok(())
        }
    }

    impl Drop for Backend {
        fn drop(&mut self) {
            let _ = self.release();
        }
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod os {
    use super::AwakeBackend;

    #[derive(Default)]
    pub struct Backend;

    impl AwakeBackend for Backend {
        fn hold(&mut self) -> Result<(), String> {
            Err("keep-awake is not supported on this platform".into())
        }

        fn release(&mut self) -> Result<(), String> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Op {
        Hold(String),
        Release(String),
    }

    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<Op>>>);

    impl Log {
        fn ops(&self) -> Vec<Op> {
            self.0.lock().unwrap().clone()
        }
    }

    struct FakeBackend {
        log: Log,
        fail_hold: bool,
        delay: Duration,
    }

    fn current_thread() -> String {
        thread::current().name().unwrap_or("?").to_string()
    }

    impl AwakeBackend for FakeBackend {
        fn hold(&mut self) -> Result<(), String> {
            thread::sleep(self.delay);
            if self.fail_hold {
                return Err("SetThreadExecutionState failed (fake)".into());
            }
            self.log.0.lock().unwrap().push(Op::Hold(current_thread()));
            Ok(())
        }
        fn release(&mut self) -> Result<(), String> {
            self.log
                .0
                .lock()
                .unwrap()
                .push(Op::Release(current_thread()));
            Ok(())
        }
    }

    fn fake(log: &Log, fail_hold: bool, delay: Duration) -> KeepAwake {
        let log = log.clone();
        KeepAwake::with_backend(move || {
            Box::new(FakeBackend {
                log: log.clone(),
                fail_hold,
                delay,
            })
        })
    }

    fn hold() -> Op {
        Op::Hold(THREAD_NAME.to_string())
    }
    fn release() -> Op {
        Op::Release(THREAD_NAME.to_string())
    }

    #[test]
    fn enable_and_disable_run_on_the_dedicated_thread() {
        let log = Log::default();
        let k = fake(&log, false, Duration::ZERO);
        k.set_enabled(true).unwrap();
        assert!(k.is_enabled());
        // Asking again changes nothing.
        k.set_enabled(true).unwrap();
        k.set_enabled(false).unwrap();
        assert!(!k.is_enabled());
        k.set_enabled(false).unwrap();
        assert_eq!(log.ops(), vec![hold(), release()]);
        assert_eq!(k.stop(Duration::from_secs(2)), StopOutcome::Stopped);
        assert_eq!(log.ops(), vec![hold(), release()]);
    }

    #[test]
    fn disabling_before_any_enable_starts_no_thread() {
        let log = Log::default();
        let k = fake(&log, false, Duration::ZERO);
        k.set_enabled(false).unwrap();
        assert_eq!(k.stop(Duration::from_secs(1)), StopOutcome::NotStarted);
        assert!(log.ops().is_empty());
    }

    #[test]
    fn stop_releases_a_held_assertion() {
        let log = Log::default();
        let k = fake(&log, false, Duration::ZERO);
        k.set_enabled(true).unwrap();
        assert_eq!(k.stop(Duration::from_secs(2)), StopOutcome::Stopped);
        assert!(!k.is_enabled());
        assert_eq!(log.ops(), vec![hold(), release()]);
        // A later enable starts a new thread.
        k.set_enabled(true).unwrap();
        assert_eq!(log.ops(), vec![hold(), release(), hold()]);
        k.stop(Duration::from_secs(2));
    }

    #[test]
    fn dropping_the_handle_releases_the_hold() {
        let log = Log::default();
        let k = fake(&log, false, Duration::ZERO);
        k.set_enabled(true).unwrap();
        drop(k);
        wait_for_ops(&log, 2);
        assert_eq!(log.ops(), vec![hold(), release()]);
    }

    #[test]
    fn an_os_failure_is_reported_and_leaves_it_off() {
        let log = Log::default();
        let k = fake(&log, true, Duration::ZERO);
        let err = k.set_enabled(true).unwrap_err();
        assert!(
            matches!(&err, KeepAwakeError::Backend(m) if m.contains("SetThreadExecutionState"))
        );
        assert!(!k.is_enabled());
        // Nothing is held, so there is nothing to release.
        k.stop(Duration::from_secs(2));
        assert!(log.ops().is_empty());
    }

    fn wait_for_ops(log: &Log, count: usize) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while log.ops().len() < count && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn an_enable_that_times_out_is_undone_without_a_stop() {
        let log = Log::default();
        let k = fake(&log, false, Duration::from_millis(300))
            .with_confirm_timeout(Duration::from_millis(20));
        assert_eq!(
            k.set_enabled(true),
            Err(KeepAwakeError::Timeout(Duration::from_millis(20)))
        );
        assert!(!k.is_enabled());
        // The thread takes the late hold and then the queued release, while
        // the handle stays alive (the app's switch shows off all along).
        wait_for_ops(&log, 2);
        assert_eq!(log.ops(), vec![hold(), release()]);
        // Nothing is left to release.
        assert_eq!(k.stop(Duration::from_secs(3)), StopOutcome::Stopped);
        assert_eq!(log.ops(), vec![hold(), release()]);
    }

    #[test]
    fn an_enable_after_a_timed_out_one_holds_again() {
        let log = Log::default();
        let k = fake(&log, false, Duration::from_millis(200))
            .with_confirm_timeout(Duration::from_millis(20));
        assert!(matches!(
            k.set_enabled(true),
            Err(KeepAwakeError::Timeout(_))
        ));
        // Queued behind the undo, a later enable that is given time to
        // confirm ends with the hold taken.
        let k = k.with_confirm_timeout(Duration::from_secs(3));
        k.set_enabled(true).unwrap();
        assert!(k.is_enabled());
        assert_eq!(log.ops(), vec![hold(), release(), hold()]);
        assert_eq!(k.stop(Duration::from_secs(3)), StopOutcome::Stopped);
        assert_eq!(log.ops(), vec![hold(), release(), hold(), release()]);
    }

    #[test]
    fn release_for_exit_is_cheap_when_never_used() {
        // The global may already be in use by another test binary path, but
        // in this crate's tests nothing touches it before this point.
        if Lazy::get(&GLOBAL).is_none() {
            assert_eq!(
                release_for_exit(Duration::from_secs(1)),
                StopOutcome::NotStarted
            );
            assert!(Lazy::get(&GLOBAL).is_none());
        }
    }
}
