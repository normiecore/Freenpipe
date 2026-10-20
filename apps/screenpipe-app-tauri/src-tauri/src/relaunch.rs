// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! One bounded teardown for every way the app ends, and a safe relaunch.
//!
//! Quit (tray), the `RunEvent::Exit` path and a safe relaunch all run the
//! same [`teardown`]: stop capture, shut the engine down (pipe scheduler and
//! audio manager, 15 s for audio), wait for the engine's HTTP runtime to drain,
//! close the SQLite pools, and stop the Pi agent sessions. Every step has its
//! own time limit, clipped to an overall budget. When the budget runs out the
//! process is ended with [`hard_exit`] instead of hanging in a half-closed
//! state.
//!
//! A safe relaunch records whether the Home window was open, runs the
//! teardown, stops the control server (so the replacement does not hand its
//! start-up off to us and exit), starts the same executable with
//! `SCREENPIPE_RELAUNCH_HOME_VISIBLE` and `SCREENPIPE_DATA_DIR` set, and then
//! ends this process. The replacement reads the flag once the event loop is
//! ready and keeps Home closed when the old instance had it closed.
//!
//! Quit requests that arrive while a teardown is already running, or while a
//! protected database recovery holds the gate, are ignored.

use crate::recording::RecordingState;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::Manager;
use tracing::{debug, error, info, warn};

// ---------------------------------------------------------------------------
// Gate
// ---------------------------------------------------------------------------

static TEARDOWN_STARTED: AtomicBool = AtomicBool::new(false);
static TEARDOWN_FINISHED: AtomicBool = AtomicBool::new(false);
static PROTECTED_DB_RECOVERY: AtomicBool = AtomicBool::new(false);
static FINISHED_SIGNAL: (Mutex<()>, std::sync::Condvar) = (Mutex::new(()), std::sync::Condvar::new());

/// True from the moment a quit, exit or relaunch teardown starts. The engine
/// watchdog stays idle while this is set.
pub fn is_shutting_down() -> bool {
    TEARDOWN_STARTED.load(Ordering::SeqCst)
}

/// Marks a database recovery that must not be interrupted by a quit. While
/// set, quit and relaunch requests are ignored.
#[allow(dead_code)] // used by the database recovery flow (shell-014)
pub fn set_protected_db_recovery(active: bool) {
    PROTECTED_DB_RECOVERY.store(active, Ordering::SeqCst);
    info!("protected database recovery {}", if active { "started" } else { "ended" });
}

/// Answer to a quit or relaunch request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitGate {
    Proceed,
    /// A teardown is already running; this request adds nothing.
    TeardownRunning,
    /// A protected database recovery is running; quitting now could leave
    /// the database half-swapped.
    DbRecoveryActive,
}

/// Pure decision behind [`try_begin_teardown`].
pub fn quit_gate(teardown_started: bool, db_recovery_active: bool) -> QuitGate {
    if teardown_started {
        QuitGate::TeardownRunning
    } else if db_recovery_active {
        QuitGate::DbRecoveryActive
    } else {
        QuitGate::Proceed
    }
}

/// Claim the teardown. Only the first caller gets `Proceed`.
fn try_begin_teardown() -> QuitGate {
    let gate = quit_gate(
        TEARDOWN_STARTED.load(Ordering::SeqCst),
        PROTECTED_DB_RECOVERY.load(Ordering::SeqCst),
    );
    if gate != QuitGate::Proceed {
        return gate;
    }
    if TEARDOWN_STARTED.swap(true, Ordering::SeqCst) {
        return QuitGate::TeardownRunning;
    }
    gate
}

fn mark_finished() {
    TEARDOWN_FINISHED.store(true, Ordering::SeqCst);
    let _guard = FINISHED_SIGNAL.0.lock().unwrap_or_else(|e| e.into_inner());
    FINISHED_SIGNAL.1.notify_all();
}

/// Give the gate back after a relaunch that could not start its replacement.
fn reopen_gate() {
    TEARDOWN_FINISHED.store(false, Ordering::SeqCst);
    TEARDOWN_STARTED.store(false, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// Budget
// ---------------------------------------------------------------------------

/// Time limits for the teardown steps. Each step gets its own limit, clipped
/// to whatever is left of `overall`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeardownBudget {
    pub overall: Duration,
    pub capture: Duration,
    /// Pipe scheduler, background workers and the audio manager (whose own
    /// shutdown is capped at 15 s inside `ServerCore::shutdown`).
    pub engine: Duration,
    /// Engine HTTP runtime and remaining tasks letting go of the database.
    pub http_drain: Duration,
    pub pools: Duration,
    pub pi: Duration,
}

impl Default for TeardownBudget {
    fn default() -> Self {
        Self {
            overall: Duration::from_secs(25),
            capture: Duration::from_secs(6),
            engine: Duration::from_secs(17),
            http_drain: Duration::from_secs(3),
            pools: Duration::from_secs(3),
            pi: Duration::from_secs(3),
        }
    }
}

/// Extra time after the budget before the outer guard ends the process even
/// if the teardown task itself is stuck.
const GUARD_MARGIN: Duration = Duration::from_secs(3);
/// How long `app.exit` may take on Windows after a finished teardown.
#[cfg(not(unix))]
const EXIT_GRACE: Duration = Duration::from_secs(5);

/// Time a step may use: its own limit or what is left of the overall
/// budget, whichever is smaller.
pub fn step_limit(step: Duration, deadline: Instant, now: Instant) -> Duration {
    step.min(deadline.saturating_duration_since(now))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    Done,
    /// Nothing to do (already stopped, no Pi state, ...).
    Skipped,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepReport {
    pub name: &'static str,
    pub outcome: StepOutcome,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TeardownReport {
    pub steps: Vec<StepReport>,
    pub elapsed: Duration,
    /// Some step timed out or the overall budget ran out.
    pub overran: bool,
}

impl TeardownReport {
    fn summary(&self) -> String {
        self.steps
            .iter()
            .map(|s| format!("{}={:?}/{}ms", s.name, s.outcome, s.elapsed.as_millis()))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Run `fut` under the step limit and record the result.
async fn run_step<F>(
    report: &mut TeardownReport,
    name: &'static str,
    limit: Duration,
    deadline: Instant,
    fut: F,
) where
    F: std::future::Future<Output = StepOutcome>,
{
    let started = Instant::now();
    let limit = step_limit(limit, deadline, started);
    let outcome = if limit.is_zero() {
        StepOutcome::TimedOut
    } else {
        tokio::time::timeout(limit, fut)
            .await
            .unwrap_or(StepOutcome::TimedOut)
    };
    let elapsed = started.elapsed();
    if outcome == StepOutcome::TimedOut {
        report.overran = true;
        warn!("teardown step {name} did not finish within {}ms", limit.as_millis());
    } else {
        debug!("teardown step {name}: {outcome:?} in {}ms", elapsed.as_millis());
    }
    report.steps.push(StepReport {
        name,
        outcome,
        elapsed,
    });
}

// ---------------------------------------------------------------------------
// Teardown
// ---------------------------------------------------------------------------

/// The shared teardown. The caller must hold the gate (see
/// [`try_begin_teardown`]). Never panics; always returns within about
/// `budget.overall`.
async fn teardown(app: &tauri::AppHandle, reason: &str, budget: TeardownBudget) -> TeardownReport {
    let started = Instant::now();
    let deadline = started + budget.overall;
    let mut report = TeardownReport::default();
    info!("pre-exit teardown started ({reason})");

    // The watchdog must not bring the engine back while it is being stopped.
    crate::health::set_recording_intended(false);

    let state = app.try_state::<RecordingState>();
    let (capture_arc, server_arc) = match &state {
        Some(s) => (Some(s.capture.clone()), Some(s.server.clone())),
        None => (None, None),
    };

    run_step(&mut report, "capture", budget.capture, deadline, async {
        let Some(capture) = capture_arc else {
            return StepOutcome::Skipped;
        };
        let session = capture.lock().await.take();
        match session {
            Some(session) => {
                session.stop().await;
                StepOutcome::Done
            }
            None => StepOutcome::Skipped,
        }
    })
    .await;

    // Keep the read pool so it can be closed explicitly once the engine is gone.
    let mut read_pool = None;
    run_step(&mut report, "engine", budget.engine, deadline, async {
        let Some(server) = server_arc else {
            return StepOutcome::Skipped;
        };
        let core = server.lock().await.take();
        match core {
            Some(core) => {
                read_pool = Some(core.db.pool.clone());
                crate::recording::shutdown_server_core(core).await;
                StepOutcome::Done
            }
            None => StepOutcome::Skipped,
        }
    })
    .await;

    run_step(&mut report, "http_drain", budget.http_drain, deadline, async {
        if crate::recording::wait_for_retired_generations(budget.http_drain).await {
            StepOutcome::Done
        } else {
            StepOutcome::TimedOut
        }
    })
    .await;

    run_step(&mut report, "sqlite_pools", budget.pools, deadline, async {
        match read_pool.take() {
            Some(pool) => {
                pool.close().await;
                info!("all SQLite pools of the engine are closed");
                StepOutcome::Done
            }
            None => StepOutcome::Skipped,
        }
    })
    .await;

    run_step(&mut report, "pi", budget.pi, deadline, async {
        match app.try_state::<crate::pi::PiState>() {
            Some(pi_state) => {
                crate::pi::cleanup_pi(&pi_state).await;
                StepOutcome::Done
            }
            None => StepOutcome::Skipped,
        }
    })
    .await;

    report.elapsed = started.elapsed();
    if report.elapsed > budget.overall {
        report.overran = true;
    }
    if report.overran {
        warn!(
            "pre-exit teardown overran its {}s budget after {}ms: {}",
            budget.overall.as_secs(),
            report.elapsed.as_millis(),
            report.summary()
        );
    } else {
        info!(
            "pre-exit teardown finished in {}ms: {}",
            report.elapsed.as_millis(),
            report.summary()
        );
    }
    report
}

/// Starts a thread that ends the process if the teardown has not finished by
/// `limit`. This is the guarantee behind the budget: it holds even when the
/// async runtime running the teardown is itself wedged.
fn arm_exit_guard(limit: Duration, code: i32) {
    let _ = std::thread::Builder::new()
        .name("teardown-guard".to_string())
        .spawn(move || {
            let guard = FINISHED_SIGNAL.0.lock().unwrap_or_else(|e| e.into_inner());
            let (_guard, timeout) = FINISHED_SIGNAL
                .1
                .wait_timeout_while(guard, limit, |_| !TEARDOWN_FINISHED.load(Ordering::SeqCst))
                .unwrap_or_else(|e| e.into_inner());
            if timeout.timed_out() && !TEARDOWN_FINISHED.load(Ordering::SeqCst) {
                error!(
                    "teardown still running after {}s; forcing exit",
                    limit.as_secs()
                );
                hard_exit(code);
            }
        });
}

/// End the process now, skipping atexit handlers and C++ static destructors
/// (the whisper/ggml Metal context asserts in its destructor on macOS).
pub fn hard_exit(code: i32) -> ! {
    info!("forcing process exit with status {code}");
    #[cfg(unix)]
    {
        extern "C" {
            fn _exit(status: i32) -> !;
        }
        // SAFETY: `_exit` ends the process immediately; nothing runs after it.
        unsafe { _exit(code) }
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        // SAFETY: terminating our own process through its pseudo-handle.
        unsafe {
            let _ = TerminateProcess(GetCurrentProcess(), code as u32);
        }
        std::process::exit(code)
    }
    #[cfg(not(any(unix, windows)))]
    std::process::exit(code)
}

/// Leave after a finished teardown. On Unix `_exit` (see [`hard_exit`]). On
/// Windows `app.exit` lets Tauri run its own cleanup, with a guard that ends
/// the process if that takes longer than a few seconds.
fn exit_after_teardown(app: &tauri::AppHandle, code: i32) {
    #[cfg(unix)]
    {
        let _ = app;
        hard_exit(code);
    }
    #[cfg(not(unix))]
    {
        let _ = std::thread::Builder::new()
            .name("exit-guard".to_string())
            .spawn(move || {
                std::thread::sleep(EXIT_GRACE);
                warn!("app exit did not complete within {}s", EXIT_GRACE.as_secs());
                hard_exit(code);
            });
        app.exit(code);
    }
}

/// Quit the app: the tray's Quit item and anything else that means "close
/// Freenpipe". Ignored while a teardown or a protected database recovery runs.
pub fn request_quit(app: tauri::AppHandle, source: &'static str) {
    match try_begin_teardown() {
        QuitGate::Proceed => {}
        gate => {
            info!("quit request from {source} ignored ({gate:?})");
            return;
        }
    }
    crate::tray::QUIT_REQUESTED.store(true, Ordering::SeqCst);
    let budget = TeardownBudget::default();
    arm_exit_guard(budget.overall + GUARD_MARGIN, 1);
    tauri::async_runtime::spawn(async move {
        let report = teardown(&app, source, budget).await;
        mark_finished();
        exit_after_teardown(&app, if report.overran { 1 } else { 0 });
    });
}

/// `RunEvent::Exit`: run the teardown synchronously (the process ends right
/// after this returns), unless a quit or relaunch already ran it.
pub fn on_run_exit(app: &tauri::AppHandle) {
    let budget = TeardownBudget::default();
    // The event loop is ending whatever the gate says, so the recovery gate
    // does not apply here (it only holds back user quit requests, which keep
    // QUIT_REQUESTED false and so keep ExitRequested from getting this far).
    if TEARDOWN_STARTED.swap(true, Ordering::SeqCst) {
        if !TEARDOWN_FINISHED.load(Ordering::SeqCst) {
            // Another teardown is in flight (an OS session end during a
            // quit, say). Give it the rest of its budget.
            let guard = FINISHED_SIGNAL.0.lock().unwrap_or_else(|e| e.into_inner());
            let _ = FINISHED_SIGNAL.1.wait_timeout_while(
                guard,
                budget.overall + GUARD_MARGIN,
                |_| !TEARDOWN_FINISHED.load(Ordering::SeqCst),
            );
        }
        debug!("exit: teardown already done");
        return;
    }

    info!("app exiting, running the shared teardown");
    crate::tray::QUIT_REQUESTED.store(true, Ordering::SeqCst);
    arm_exit_guard(budget.overall + GUARD_MARGIN, 1);
    // A dedicated thread: Exit can fire from inside an async context, where
    // blocking on another future would panic.
    let app = app.clone();
    let worker = std::thread::Builder::new()
        .name("exit-teardown".to_string())
        .spawn(move || {
            let report =
                tauri::async_runtime::block_on(async move { teardown(&app, "exit", budget).await });
            mark_finished();
            report
        });
    match worker {
        Ok(handle) => match handle.join() {
            Ok(report) if report.overran => {
                hard_exit(1);
            }
            Ok(_) => {}
            Err(_) => {
                error!("exit teardown thread panicked");
                hard_exit(1);
            }
        },
        Err(e) => {
            error!("could not start the exit teardown thread: {e}");
            mark_finished();
        }
    }
}

// ---------------------------------------------------------------------------
// Safe relaunch
// ---------------------------------------------------------------------------

/// Environment for the replacement process.
pub fn relaunch_env(home_visible: bool, base_dir: &Path) -> Vec<(&'static str, String)> {
    vec![
        (
            screenpipe_core::env::SCREENPIPE_RELAUNCH_HOME_VISIBLE,
            if home_visible { "true" } else { "false" }.to_string(),
        ),
        (
            screenpipe_core::env::SCREENPIPE_DATA_DIR,
            base_dir.to_string_lossy().into_owned(),
        ),
    ]
}

/// `SCREENPIPE_RELAUNCH_HOME_VISIBLE` as set by [`relaunch_env`]. Anything
/// other than `true`/`false` (or `1`/`0`) means "not a relaunch".
pub fn parse_home_visible(value: Option<&str>) -> Option<bool> {
    match value.map(|v| v.trim().to_ascii_lowercase()) {
        Some(v) if v == "true" || v == "1" => Some(true),
        Some(v) if v == "false" || v == "0" => Some(false),
        _ => None,
    }
}

fn home_window_visible(app: &tauri::AppHandle) -> bool {
    app.get_webview_window("home")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// Start-up half of the relaunch: called once the event loop is ready. When
/// the previous instance had Home closed, close it here too.
pub fn on_ready(app: &tauri::AppHandle) {
    let flag = parse_home_visible(
        std::env::var(screenpipe_core::env::SCREENPIPE_RELAUNCH_HOME_VISIBLE)
            .ok()
            .as_deref(),
    );
    match flag {
        None => {}
        Some(true) => info!("startup_window_restore outcome=home_restored"),
        Some(false) => {
            if let Some(home) = app.get_webview_window("home") {
                let _ = home.hide();
            }
            info!("startup_window_restore outcome=home_kept_closed");
        }
    }
}

/// Managed sender of a control server started again after a failed
/// relaunch. Dropping it would stop that server.
static RESTARTED_CONTROL_SERVER: Mutex<Option<tokio::sync::mpsc::Sender<()>>> = Mutex::new(None);

fn focus_port() -> u16 {
    std::env::var(screenpipe_core::env::SCREENPIPE_FOCUS_PORT)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(11435)
}

/// Stop the :11435 control server. A replacement that finds it still
/// answering `/focus` would hand off to us and exit.
async fn stop_control_server(app: &tauri::AppHandle) {
    if let Some(tx) = app.try_state::<tokio::sync::mpsc::Sender<()>>() {
        let _ = tx.try_send(());
    }
    let restarted = RESTARTED_CONTROL_SERVER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(tx) = restarted {
        let _ = tx.try_send(());
    }
    let port = focus_port();
    for _ in 0..20 {
        if !crate::port_conflict::port_in_use(port).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    warn!("control server port {port} still busy before relaunch");
}

#[cfg(windows)]
fn spawn_detached(exe: &Path, env: &[(&'static str, String)]) -> std::io::Result<u32> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let build = |flags: u32| {
        let mut cmd = std::process::Command::new(exe);
        cmd.envs(env.iter().map(|(k, v)| (*k, v.as_str())))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(flags);
        cmd
    };
    // Leave our job object if we can, so a job that kills its members when
    // we exit does not take the replacement with us.
    match build(CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB).spawn() {
        Ok(child) => Ok(child.id()),
        Err(_) => build(CREATE_NEW_PROCESS_GROUP).spawn().map(|c| c.id()),
    }
}

#[cfg(not(windows))]
fn spawn_detached(exe: &Path, env: &[(&'static str, String)]) -> std::io::Result<u32> {
    std::process::Command::new(exe)
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|c| c.id())
}

fn resolve_current_binary(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let exe = tauri::process::current_binary(&app.env())
        .map_err(|e| format!("could not resolve the current binary: {e}"))?;
    if !exe.is_file() {
        return Err(format!(
            "the current binary is missing on disk: {}",
            exe.display()
        ));
    }
    Ok(exe)
}

/// Replace this process with a fresh copy of the same executable. On success
/// this does not return. `home_visible` overrides the Home window's current
/// visibility. On failure recording is resumed and the error returned.
pub async fn safe_relaunch(
    app: tauri::AppHandle,
    reason: &'static str,
    home_visible: Option<bool>,
) -> Result<(), String> {
    info!("app_relaunch_requested reason={reason}");
    let exe = resolve_current_binary(&app).map_err(|e| {
        error!("safe relaunch: {e}");
        e
    })?;
    let home_visible = home_visible.unwrap_or_else(|| home_window_visible(&app));
    info!("relaunch_window_state home_visible={home_visible}");

    match try_begin_teardown() {
        QuitGate::Proceed => {}
        gate => {
            info!("relaunch ({reason}) ignored ({gate:?})");
            return Err(format!("relaunch ignored: {gate:?}"));
        }
    }
    crate::tray::QUIT_REQUESTED.store(true, Ordering::SeqCst);
    let budget = TeardownBudget::default();
    let report = teardown(&app, reason, budget).await;
    stop_control_server(&app).await;

    let base_dir = screenpipe_core::paths::default_screenpipe_data_dir();
    let env = relaunch_env(home_visible, &base_dir);
    match spawn_detached(&exe, &env) {
        Ok(pid) => {
            info!(
                "safe relaunch: replacement pid {pid} started from {} (teardown {}ms)",
                exe.display(),
                report.elapsed.as_millis()
            );
            mark_finished();
            hard_exit(0);
        }
        Err(e) => {
            error!("safe relaunch: could not start {}: {e}", exe.display());
            // Undo: this instance keeps running and records again.
            reopen_gate();
            crate::tray::QUIT_REQUESTED.store(false, Ordering::SeqCst);
            let tx = crate::server::spawn_server(app.clone(), focus_port());
            *RESTARTED_CONTROL_SERVER
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(tx);
            crate::health::set_recording_intended(true);
            let state = app.state::<RecordingState>();
            if let Err(err) = crate::recording::spawn_screenpipe(state, app.clone(), None).await {
                error!("safe relaunch: could not resume recording: {err}");
            }
            Err(format!("could not start a new Freenpipe process: {e}"))
        }
    }
}

/// Debug commands (engine kill, relaunch) run only in debug builds or with
/// developer mode on.
pub(crate) fn debug_commands_allowed(app: &tauri::AppHandle) -> bool {
    cfg!(debug_assertions)
        || crate::store::SettingsStore::get(app)
            .ok()
            .flatten()
            .map_or(false, |s| s.dev_mode)
}

/// Relaunch after the user granted screen-recording permission. macOS only
/// reads that permission at process start, so a new process is needed there;
/// on Windows and Linux nothing needs to restart and this returns at once.
#[tauri::command]
#[specta::specta]
pub async fn restart_after_screen_recording_permission(
    app: tauri::AppHandle,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        safe_relaunch(app, "screen_recording_permission", Some(true)).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        debug!("restart_after_screen_recording_permission: not needed on this platform");
        Ok(())
    }
}

/// Developer command: safe-relaunch the app now, keeping Home as it is.
#[tauri::command]
#[specta::specta]
pub async fn debug_relaunch_app(app: tauri::AppHandle) -> Result<(), String> {
    if !debug_commands_allowed(&app) {
        return Err("debug commands need developer mode".to_string());
    }
    safe_relaunch(app, "debug", None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_gate_ignores_requests_during_teardown_and_recovery() {
        assert_eq!(quit_gate(false, false), QuitGate::Proceed);
        assert_eq!(quit_gate(true, false), QuitGate::TeardownRunning);
        assert_eq!(quit_gate(false, true), QuitGate::DbRecoveryActive);
        assert_eq!(quit_gate(true, true), QuitGate::TeardownRunning);
    }

    #[test]
    fn step_limits_are_clipped_to_the_overall_budget() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(5);
        assert_eq!(
            step_limit(Duration::from_secs(3), deadline, now),
            Duration::from_secs(3)
        );
        assert_eq!(
            step_limit(Duration::from_secs(17), deadline, now),
            Duration::from_secs(5)
        );
        assert_eq!(
            step_limit(Duration::from_secs(3), now, now + Duration::from_secs(1)),
            Duration::ZERO
        );
    }

    #[test]
    fn default_budget_covers_the_audio_shutdown_and_fits_overall() {
        let b = TeardownBudget::default();
        // ServerCore::shutdown caps the audio manager at 15 s.
        assert!(b.engine > Duration::from_secs(15));
        assert_eq!(b.http_drain, Duration::from_secs(3));
        assert!(b.overall >= b.engine + b.http_drain);
        assert!(b.overall + GUARD_MARGIN <= Duration::from_secs(30));
    }

    #[tokio::test]
    async fn steps_that_overrun_are_cut_off_and_reported() {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(300);
        let mut report = TeardownReport::default();
        run_step(&mut report, "fast", Duration::from_secs(1), deadline, async {
            StepOutcome::Done
        })
        .await;
        run_step(&mut report, "stuck", Duration::from_secs(10), deadline, async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            StepOutcome::Done
        })
        .await;
        run_step(&mut report, "late", Duration::from_secs(1), deadline, async {
            StepOutcome::Done
        })
        .await;
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(report.overran);
        let outcomes: Vec<_> = report.steps.iter().map(|s| (s.name, s.outcome)).collect();
        assert_eq!(
            outcomes,
            vec![
                ("fast", StepOutcome::Done),
                ("stuck", StepOutcome::TimedOut),
                ("late", StepOutcome::TimedOut),
            ]
        );
    }

    #[test]
    fn relaunch_env_carries_home_visibility_and_data_dir() {
        let dir = Path::new("/tmp/fp-data");
        let env = relaunch_env(false, dir);
        assert_eq!(
            env,
            vec![
                ("SCREENPIPE_RELAUNCH_HOME_VISIBLE", "false".to_string()),
                ("SCREENPIPE_DATA_DIR", "/tmp/fp-data".to_string()),
            ]
        );
        assert_eq!(relaunch_env(true, dir)[0].1, "true");
        // The replacement reads back exactly what we wrote.
        assert_eq!(parse_home_visible(Some(&env[0].1)), Some(false));
        assert_eq!(parse_home_visible(Some("true")), Some(true));
        assert_eq!(parse_home_visible(Some(" TRUE ")), Some(true));
        assert_eq!(parse_home_visible(Some("0")), Some(false));
        assert_eq!(parse_home_visible(Some("maybe")), None);
        assert_eq!(parse_home_visible(None), None);
    }
}
