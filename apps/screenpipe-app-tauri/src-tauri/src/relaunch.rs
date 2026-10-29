// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! One bounded teardown for every way the app ends, and a safe relaunch.
//!
//! Quit (tray), the `RunEvent::Exit` path and a safe relaunch all run the
//! same [`teardown`]: stop capture, shut the engine down (background workers,
//! pipe scheduler and audio manager, 15 s for audio, and any engine another
//! task is starting or stopping at that moment), wait for the engine's HTTP
//! runtime to drain, close the engine's read pool (its write pool goes with the
//! database manager once that drains), stop the Pi agent sessions, and release
//! the keep-awake hold. Every step has its own time limit, clipped to an
//! overall budget. A late HTTP drain, pool close, Pi stop or keep-awake
//! release is only logged: capture and audio are already stopped, so
//! the app leaves normally (status 0, through Tauri where it can). Only a
//! capture or engine step that did not stop in time ends the process with
//! [`hard_exit`], because returning then would run native destructors under
//! live capture or audio work. A guard thread force-exits a teardown that is
//! itself wedged.
//!
//! A safe relaunch records whether the Home window was open, runs the
//! teardown, stops the control server (so the replacement does not hand its
//! start-up off to us and exit), starts the same executable with
//! `SCREENPIPE_RELAUNCH_HOME_VISIBLE` and `SCREENPIPE_DATA_DIR` set, and then
//! ends this process. The replacement reads (and removes) the flag at the top
//! of `main`, before any thread exists, and keeps Home closed once the event
//! loop is ready when the old instance had it closed.
//!
//! Quit requests that arrive while a teardown is already running, or while a
//! protected database recovery holds the gate, are ignored.
//!
//! Tauri's own `app.restart()` (update install, rollback) also ends in
//! `RunEvent::Exit`. The teardown runs there too, and Tauri is left to start
//! the replacement unless the capture or engine step overran; in that case
//! this module starts the replacement itself and force-exits.

use crate::recording::RecordingState;
use crate::server_core::{AUDIO_SHUTDOWN_LIMIT, BACKGROUND_WORKER_GRACE};
use std::ffi::OsString;
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
static FINISHED_SIGNAL: (Mutex<()>, std::sync::Condvar) =
    (Mutex::new(()), std::sync::Condvar::new());

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
    info!(
        "protected database recovery {}",
        if active { "started" } else { "ended" }
    );
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
// Pending Tauri restart (update install, rollback)
// ---------------------------------------------------------------------------

/// The process Tauri's `restart()` would start: the same binary with the
/// same arguments.
#[derive(Debug, Clone)]
struct RestartTarget {
    exe: PathBuf,
    args: Vec<OsString>,
}

static RESTART_PENDING: AtomicBool = AtomicBool::new(false);
static RESTART_TARGET: Mutex<Option<RestartTarget>> = Mutex::new(None);
/// Set once a replacement for a pending restart was started by this module,
/// so the exit path and the guard thread never start two.
static RESTART_REPLACEMENT_STARTED: AtomicBool = AtomicBool::new(false);

/// `RunEvent::ExitRequested`. Tauri's `app.restart()` asks to exit with
/// `RESTART_EXIT_CODE` (and ignores `prevent_exit` for it). Remember that a
/// restart is pending, and what it would start, so the exit path does not
/// end the process before the replacement exists. Returns true for a
/// restart request.
pub fn on_exit_requested(app: &tauri::AppHandle, code: Option<i32>) -> bool {
    if code != Some(tauri::RESTART_EXIT_CODE) {
        return false;
    }
    RESTART_PENDING.store(true, Ordering::SeqCst);
    let env = app.env();
    match tauri::process::current_binary(&env) {
        Ok(exe) => {
            let args = env.args_os.iter().skip(1).cloned().collect();
            *RESTART_TARGET.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(RestartTarget { exe, args });
        }
        Err(e) => warn!("app restart pending, but the current binary could not be resolved: {e}"),
    }
    info!("ExitRequested during app restart: letting it through");
    true
}

fn restart_pending() -> bool {
    RESTART_PENDING.load(Ordering::SeqCst)
}

/// Start the replacement a pending `app.restart()` would have started. Used
/// only when this process is about to end itself instead of returning to
/// Tauri. Runs at most once.
fn spawn_pending_restart(context: &str) {
    if RESTART_REPLACEMENT_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let target = RESTART_TARGET
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    match target {
        Some(t) => match spawn_detached(&t.exe, &t.args, &[]) {
            Ok(child) => info!(
                "pending app restart: replacement pid {} started from {} ({context})",
                child.id(),
                t.exe.display()
            ),
            Err(e) => error!(
                "pending app restart: could not start {} ({context}): {e}",
                t.exe.display()
            ),
        },
        None => error!("pending app restart: no binary to start ({context})"),
    }
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
    /// `ServerCore::shutdown`: background workers (their grace,
    /// [`BACKGROUND_WORKER_GRACE`]), the pipe scheduler (aborted, normally at
    /// once) and the audio manager (capped at [`AUDIO_SHUTDOWN_LIMIT`]), plus
    /// a respawn or start that is bringing an engine up or down meanwhile.
    pub engine: Duration,
    /// Engine HTTP runtime and remaining tasks letting go of the database.
    pub http_drain: Duration,
    /// Closing the engine's read pool.
    pub pools: Duration,
    pub pi: Duration,
    /// Giving back the keep-awake hold (`keepComputerAwake`). The last step
    /// and not a critical one: the OS releases the hold when the process
    /// ends anyway.
    pub keep_awake: Duration,
}

/// Room the engine step leaves for the pipe scheduler's stop on top of the
/// workers' grace and the audio cap. Its abort normally lands at once.
const SCHEDULER_STOP_ALLOWANCE: Duration = Duration::from_secs(1);

impl Default for TeardownBudget {
    fn default() -> Self {
        let engine = BACKGROUND_WORKER_GRACE + AUDIO_SHUTDOWN_LIMIT + SCHEDULER_STOP_ALLOWANCE;
        Self {
            // Capture and the engine at their limits still leave a little
            // room for the drain; the later steps are not critical.
            overall: Duration::from_secs(29),
            capture: Duration::from_secs(6),
            engine,
            http_drain: Duration::from_secs(3),
            pools: Duration::from_secs(3),
            pi: Duration::from_secs(3),
            keep_awake: crate::power_awake::RELEASE_LIMIT + Duration::from_secs(1),
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
    /// The step used up its own time limit.
    TimedOut,
    /// The overall budget ran out before the step's own limit did (or before
    /// the step could start).
    OutOfBudget,
}

impl StepOutcome {
    /// The step did not finish in the time it was given.
    pub fn is_overrun(self) -> bool {
        matches!(self, StepOutcome::TimedOut | StepOutcome::OutOfBudget)
    }
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
    /// Some step did not finish in time, for either reason.
    pub overran: bool,
    /// The overall budget ran out: a step was cut short by it, or the whole
    /// teardown took longer than the budget.
    pub budget_exhausted: bool,
}

impl TeardownReport {
    fn summary(&self) -> String {
        self.steps
            .iter()
            .map(|s| format!("{}={:?}/{}ms", s.name, s.outcome, s.elapsed.as_millis()))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Comma-separated names of the steps with `outcome`, or `none`.
    fn names_with(&self, outcome: StepOutcome) -> String {
        let names: Vec<&str> = self
            .steps
            .iter()
            .filter(|s| s.outcome == outcome)
            .map(|s| s.name)
            .collect();
        if names.is_empty() {
            "none".to_string()
        } else {
            names.join(",")
        }
    }

    /// Comma-separated names of every step that did not finish in time.
    fn overrun_names(&self) -> String {
        self.steps
            .iter()
            .filter(|s| s.outcome.is_overrun())
            .map(|s| s.name)
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Outcome of a step that did not return in time: `OutOfBudget` when what
/// was left of the overall budget, not the step's own limit, set the cut-off.
pub fn overrun_outcome(own_limit: Duration, budget_left: Duration) -> StepOutcome {
    if budget_left < own_limit {
        StepOutcome::OutOfBudget
    } else {
        StepOutcome::TimedOut
    }
}

/// The warning for a step that did not finish in time, saying which limit
/// ended it; `None` for a step that finished. `limit` is the time the step
/// was actually given (its own limit clipped to the budget left).
fn step_overrun_message(
    name: &str,
    outcome: StepOutcome,
    limit: Duration,
    own_limit: Duration,
) -> Option<String> {
    match outcome {
        StepOutcome::Done | StepOutcome::Skipped => None,
        StepOutcome::TimedOut => Some(format!(
            "teardown step {name} hit its own {}ms limit; moving on",
            own_limit.as_millis()
        )),
        StepOutcome::OutOfBudget if limit.is_zero() => Some(format!(
            "teardown step {name} not run: the overall budget was already used up (its own limit is {}ms)",
            own_limit.as_millis()
        )),
        StepOutcome::OutOfBudget => Some(format!(
            "teardown step {name} cut off by the overall budget after {}ms (its own limit is {}ms)",
            limit.as_millis(),
            own_limit.as_millis()
        )),
    }
}

/// Run `fut` under the step limit and record the result.
async fn run_step<F>(
    report: &mut TeardownReport,
    name: &'static str,
    own_limit: Duration,
    deadline: Instant,
    fut: F,
) where
    F: std::future::Future<Output = StepOutcome>,
{
    let started = Instant::now();
    let budget_left = deadline.saturating_duration_since(started);
    let limit = step_limit(own_limit, deadline, started);
    let finished = if limit.is_zero() {
        None
    } else {
        tokio::time::timeout(limit, fut).await.ok()
    };
    let elapsed = started.elapsed();
    let outcome = finished.unwrap_or_else(|| overrun_outcome(own_limit, budget_left));
    match step_overrun_message(name, outcome, limit, own_limit) {
        Some(line) => warn!("{line}"),
        None => debug!(
            "teardown step {name}: {outcome:?} in {}ms",
            elapsed.as_millis()
        ),
    }
    if outcome.is_overrun() {
        report.overran = true;
    }
    if outcome == StepOutcome::OutOfBudget {
        report.budget_exhausted = true;
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
        let core = match server_arc {
            Some(server) => server.lock().await.take(),
            None => None,
        };
        let stopped_here = match core {
            Some(core) => {
                read_pool = Some(core.db.pool.clone());
                crate::recording::shutdown_server_core(core).await;
                true
            }
            None => false,
        };
        // A watchdog respawn, or a start that was under way when the quit
        // began, may be stopping or retiring another generation right now.
        // Leaving before it is done would exit with its capture or audio
        // still running, so wait for it within this step's limit.
        let waited = crate::recording::wait_for_engine_transitions().await;
        if stopped_here || waited {
            StepOutcome::Done
        } else {
            StepOutcome::Skipped
        }
    })
    .await;

    run_step(
        &mut report,
        "http_drain",
        budget.http_drain,
        deadline,
        async {
            if crate::recording::wait_for_retired_generations(budget.http_drain).await {
                StepOutcome::Done
            } else {
                StepOutcome::TimedOut
            }
        },
    )
    .await;

    // The write pool is private to the database manager and closes with it,
    // which is what a finished drain observed.
    let drained = report.steps.last().map_or(false, |s| {
        s.name == "http_drain" && s.outcome == StepOutcome::Done
    });
    run_step(&mut report, "sqlite_pools", budget.pools, deadline, async {
        match read_pool.take() {
            Some(pool) => {
                pool.close().await;
                info!("{}", pools_closed_message(drained));
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

    // Last: the OS releases the hold when the process ends anyway, so this
    // only keeps a slow quit or a relaunch from holding the machine awake
    // meanwhile. After the critical steps, a wedged keep-awake thread can
    // never cut them short.
    run_step(
        &mut report,
        "keep_awake",
        budget.keep_awake,
        deadline,
        crate::power_awake::release_for_teardown(),
    )
    .await;

    report.elapsed = started.elapsed();
    if report.elapsed > budget.overall {
        report.overran = true;
        report.budget_exhausted = true;
    }
    log_teardown_result(&report, budget.overall);
    report
}

/// What the `sqlite_pools` step can say once it closed the read pool. The
/// write pool is closed only when the database manager is gone, which the
/// `http_drain` step observes (`drained`).
fn pools_closed_message(drained: bool) -> &'static str {
    if drained {
        "all SQLite pools of the engine are closed (read pool closed here, write pool with the database manager)"
    } else {
        "engine read pool closed; the write pool stays open while something still holds the database manager (see http_drain)"
    }
}

/// Log the teardown's closing line (see [`teardown_result_line`]).
fn log_teardown_result(report: &TeardownReport, overall: Duration) {
    let (late, line) = teardown_result_line(report, overall);
    if late {
        warn!("{line}");
    } else {
        info!("{line}");
    }
}

/// The teardown's closing log line, and whether it is a warning. A step
/// that used up its own limit and a teardown that ran out of its overall
/// budget read differently, so the log says which limit was hit.
fn teardown_result_line(report: &TeardownReport, overall: Duration) -> (bool, String) {
    if report.budget_exhausted {
        (
            true,
            format!(
                "pre-exit teardown ran out of its {}s overall budget after {}ms (cut off: {}; over their own limit: {}): {}",
                overall.as_secs(),
                report.elapsed.as_millis(),
                report.names_with(StepOutcome::OutOfBudget),
                report.names_with(StepOutcome::TimedOut),
                report.summary()
            ),
        )
    } else if report.overran {
        (
            true,
            format!(
                "pre-exit teardown finished in {}ms within its overall budget (over their own limit: {}): {}",
                report.elapsed.as_millis(),
                report.names_with(StepOutcome::TimedOut),
                report.summary()
            ),
        )
    } else {
        (
            false,
            format!(
                "pre-exit teardown finished in {}ms: {}",
                report.elapsed.as_millis(),
                report.summary()
            ),
        )
    }
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
                // An update or rollback restart must not end with no app at
                // all. (No tray cleanup here: that needs the main thread,
                // which may be the one that is stuck.)
                if restart_pending() {
                    spawn_pending_restart("teardown guard");
                }
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

/// Leave after a teardown that stopped capture and the engine. On Unix
/// `_exit` (see [`hard_exit`]). On Windows `app.exit` lets Tauri run its own
/// cleanup (the tray icon included), with a guard that ends the process if
/// that takes longer than a few seconds.
fn exit_after_teardown(app: &tauri::AppHandle, code: i32) {
    info!("leaving after teardown with status {code}");
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

/// Remove the tray icon (and Tauri's other per-process resources) before a
/// forced exit, if the main thread answers in time. Windows otherwise keeps a
/// dead icon in the notification area until the mouse passes over it.
async fn release_tray_before_forced_exit(app: &tauri::AppHandle) {
    let handle = app.clone();
    if !on_main_thread(app, MAIN_THREAD_WAIT, move || handle.cleanup_before_exit()).await {
        warn!(
            "main thread did not release the tray icon within {}s",
            MAIN_THREAD_WAIT.as_secs()
        );
    }
}

/// Quit the app: the tray's Quit item and anything else that means "close
/// Handvel". Ignored while a teardown or a protected database recovery runs.
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
        let severity = TeardownSeverity::of(Some(&report));
        match quit_exit(severity) {
            QuitExit::Normal => {
                if severity == TeardownSeverity::SlowCleanup {
                    warn!(
                        "quit: cleanup ran late ({}); capture and engine are stopped, leaving normally",
                        report.overrun_names()
                    );
                }
                exit_after_teardown(&app, QuitExit::Normal.status());
            }
            QuitExit::Forced => {
                error!(
                    "quit: capture or engine did not stop in time ({}); forcing exit",
                    report.overrun_names()
                );
                release_tray_before_forced_exit(&app).await;
                hard_exit(QuitExit::Forced.status());
            }
        }
    });
}

/// Teardown steps whose overrun leaves capture or the audio pipeline half
/// stopped. Returning to Tauri then would run C++ static destructors under
/// live native work (the ggml Metal abort), so the process ends at once.
const CRITICAL_STEPS: [&str; 2] = ["capture", "engine"];

/// What a finished teardown means for leaving the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeardownSeverity {
    /// Every step finished (or had nothing to do) in time.
    Clean,
    /// Capture and the engine stopped; only a later step (`http_drain`,
    /// `sqlite_pools`, `pi`, `keep_awake`) ran late, or the whole run went
    /// a little over the budget. A database handle, Pi session or keep-awake
    /// hold still open is released by the OS when the process ends, so this
    /// is logged, not acted on.
    SlowCleanup,
    /// Capture or the engine did not stop in time, or the teardown thread
    /// panicked: native capture or audio work may still be running.
    Critical,
}

impl TeardownSeverity {
    /// `report` is `None` when the teardown thread panicked.
    pub fn of(report: Option<&TeardownReport>) -> Self {
        let Some(report) = report else {
            return TeardownSeverity::Critical;
        };
        let critical = report
            .steps
            .iter()
            .any(|s| s.outcome.is_overrun() && CRITICAL_STEPS.contains(&s.name));
        if critical {
            TeardownSeverity::Critical
        } else if report.overran || report.budget_exhausted {
            TeardownSeverity::SlowCleanup
        } else {
            TeardownSeverity::Clean
        }
    }
}

/// How a tray quit leaves once its teardown is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitExit {
    /// Leave through Tauri (`app.exit`; `_exit` on Unix) with status 0.
    Normal,
    /// Release the tray icon if the main thread answers, then end the
    /// process at once with status 1.
    Forced,
}

impl QuitExit {
    pub fn status(self) -> i32 {
        match self {
            QuitExit::Normal => 0,
            QuitExit::Forced => 1,
        }
    }
}

/// Pure decision behind `request_quit`: only a capture or engine overrun is
/// worth a forced exit.
pub fn quit_exit(severity: TeardownSeverity) -> QuitExit {
    match severity {
        TeardownSeverity::Clean | TeardownSeverity::SlowCleanup => QuitExit::Normal,
        TeardownSeverity::Critical => QuitExit::Forced,
    }
}

/// What `on_run_exit` does once the teardown is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitFollowUp {
    /// Return to Tauri: it finishes its cleanup (tray icon included), exits
    /// with the status it was asked for and, when `app.restart()` is
    /// pending, starts the replacement.
    Return,
    /// Release the tray icon, then end the process now.
    ForceExit,
    /// Release the tray icon, start the replacement a pending restart would
    /// have started, then end the process now.
    ReplaceThenForceExit,
}

/// Pure decision behind `on_run_exit`. `report` is `None` when the teardown
/// thread panicked, which counts as a critical overrun. A slow HTTP drain,
/// pool close or Pi stop is logged and Tauri is left to finish, restart or
/// not; only a capture or engine overrun ends the process here.
pub fn exit_follow_up(report: Option<&TeardownReport>, restart_pending: bool) -> ExitFollowUp {
    match (TeardownSeverity::of(report), restart_pending) {
        (TeardownSeverity::Clean | TeardownSeverity::SlowCleanup, _) => ExitFollowUp::Return,
        (TeardownSeverity::Critical, false) => ExitFollowUp::ForceExit,
        (TeardownSeverity::Critical, true) => ExitFollowUp::ReplaceThenForceExit,
    }
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
            let _ =
                FINISHED_SIGNAL
                    .1
                    .wait_timeout_while(guard, budget.overall + GUARD_MARGIN, |_| {
                        !TEARDOWN_FINISHED.load(Ordering::SeqCst)
                    });
        }
        debug!("exit: teardown already done");
        return;
    }

    info!("app exiting, running the shared teardown");
    crate::tray::QUIT_REQUESTED.store(true, Ordering::SeqCst);
    arm_exit_guard(budget.overall + GUARD_MARGIN, 1);
    // A dedicated thread: Exit can fire from inside an async context, where
    // blocking on another future would panic.
    let worker_app = app.clone();
    let worker = std::thread::Builder::new()
        .name("exit-teardown".to_string())
        .spawn(move || {
            let report = tauri::async_runtime::block_on(async move {
                teardown(&worker_app, "exit", budget).await
            });
            mark_finished();
            report
        });
    let handle = match worker {
        Ok(handle) => handle,
        Err(e) => {
            error!("could not start the exit teardown thread: {e}");
            mark_finished();
            return;
        }
    };
    let report = handle.join().ok();
    if report.is_none() {
        error!("exit teardown thread panicked");
    }
    let pending = restart_pending();
    let overruns = report
        .as_ref()
        .map_or_else(|| "teardown thread".to_string(), |r| r.overrun_names());
    match exit_follow_up(report.as_ref(), pending) {
        ExitFollowUp::Return => {
            if report.as_ref().map_or(false, |r| r.overran) {
                warn!(
                    "exit: cleanup ran late ({overruns}); capture and engine are stopped, leaving through Tauri{}",
                    if pending { " (app restart pending)" } else { "" }
                );
            }
        }
        // RunEvent::Exit runs on the main thread, so the tray icon and
        // window resources can be released here, as Tauri itself would. The
        // single-instance plugin already released its lock in its own Exit
        // handler, which runs before this one.
        ExitFollowUp::ForceExit => {
            error!("exit: capture or engine did not stop in time ({overruns}); forcing exit");
            app.cleanup_before_exit();
            hard_exit(1);
        }
        ExitFollowUp::ReplaceThenForceExit => {
            error!("exit: capture or engine did not stop in time ({overruns}) while an app restart is pending; starting the replacement before forcing exit");
            app.cleanup_before_exit();
            spawn_pending_restart("exit teardown overran");
            hard_exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Safe relaunch
// ---------------------------------------------------------------------------

/// Environment for the replacement process. `SCREENPIPE_DATA_DIR` is passed
/// on only when this process was itself started with it: the variable wins
/// over the `dataDir` setting (`config::resolve_data_dir`), so setting it to
/// the default directory would move a user's custom recording folder back
/// to `~/.screenpipe` in the replacement.
pub fn relaunch_env(
    home_visible: bool,
    inherited_data_dir: Option<&Path>,
) -> Vec<(&'static str, String)> {
    let mut env = vec![(
        screenpipe_core::env::SCREENPIPE_RELAUNCH_HOME_VISIBLE,
        if home_visible { "true" } else { "false" }.to_string(),
    )];
    if let Some(dir) = inherited_data_dir {
        env.push((
            screenpipe_core::env::SCREENPIPE_DATA_DIR,
            dir.to_string_lossy().into_owned(),
        ));
    }
    env
}

/// `SCREENPIPE_DATA_DIR` as this process received it (nothing in the app
/// sets it).
fn inherited_data_dir() -> Option<PathBuf> {
    std::env::var_os(screenpipe_core::env::SCREENPIPE_DATA_DIR)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
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

static RELAUNCH_HOME_VISIBLE: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();

/// Read `SCREENPIPE_RELAUNCH_HOME_VISIBLE`, remove it from the environment
/// and keep the answer for [`relaunch_home_visible`]. Removing it means
/// nothing this process starts later (Tauri's `restart()`, the update
/// installer's relaunch, the CLI) inherits a stale relaunch flag.
///
/// Call it once, as the first statement of `main`, before any thread or
/// async runtime exists: changing the environment while another thread may
/// read it (C `getenv`, in a library or a runtime worker) is undefined
/// behaviour on Linux and macOS. Later calls change nothing.
pub fn capture_relaunch_flag() {
    capture_home_visible_flag(
        &RELAUNCH_HOME_VISIBLE,
        screenpipe_core::env::SCREENPIPE_RELAUNCH_HOME_VISIBLE,
    );
}

/// The relaunch flag this process was started with, as captured by
/// [`capture_relaunch_flag`]. `None` when this is not a safe-relaunch
/// replacement (or the flag was never captured). Never touches the
/// environment, so it is safe from any thread.
pub fn relaunch_home_visible() -> Option<bool> {
    RELAUNCH_HOME_VISIBLE.get().copied().flatten()
}

/// Fill `cell` from `key` the first time; later calls keep the first answer
/// and leave the environment alone.
fn capture_home_visible_flag(cell: &std::sync::OnceLock<Option<bool>>, key: &str) -> Option<bool> {
    *cell.get_or_init(|| take_home_visible_flag(key))
}

/// Read a relaunch flag and remove it from the environment.
fn take_home_visible_flag(key: &str) -> Option<bool> {
    let raw = std::env::var(key).ok();
    if raw.is_some() {
        std::env::remove_var(key);
    }
    parse_home_visible(raw.as_deref())
}

/// Start-up half of the relaunch: called once the event loop is ready. When
/// the previous instance had Home closed, close it here too. Uses the flag
/// `main` captured; the environment is not read again here.
pub fn on_ready(app: &tauri::AppHandle) {
    match relaunch_home_visible() {
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

/// Extra time a safe relaunch gives an engine that is still starting or
/// stopping after the teardown, before the replacement opens the database.
const RELAUNCH_ENGINE_WAIT: Duration = Duration::from_secs(30);

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

fn detached_command(
    exe: &Path,
    args: &[OsString],
    env: &[(&'static str, String)],
) -> std::process::Command {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args)
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd
}

#[cfg(windows)]
fn spawn_detached(
    exe: &Path,
    args: &[OsString],
    env: &[(&'static str, String)],
) -> std::io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    // Leave our job object if we can, so a job that kills its members when
    // we exit does not take the replacement with us.
    match detached_command(exe, args, env)
        .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
    {
        Ok(child) => Ok(child),
        Err(_) => detached_command(exe, args, env)
            .creation_flags(CREATE_NEW_PROCESS_GROUP)
            .spawn(),
    }
}

#[cfg(not(windows))]
fn spawn_detached(
    exe: &Path,
    args: &[OsString],
    env: &[(&'static str, String)],
) -> std::io::Result<std::process::Child> {
    detached_command(exe, args, env).spawn()
}

/// How long to wait for the main thread to run a release step.
const MAIN_THREAD_WAIT: Duration = Duration::from_secs(2);
/// How long a just-started replacement is watched for an immediate exit.
const REPLACEMENT_WATCH: Duration = Duration::from_secs(1);

/// Run `f` on the event-loop thread and wait up to `limit` for it. False
/// when the main thread did not get to it in time (it may still run later).
async fn on_main_thread<F>(app: &tauri::AppHandle, limit: Duration, f: F) -> bool
where
    F: FnOnce() + Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    if app
        .run_on_main_thread(move || {
            f();
            let _ = tx.send(());
        })
        .is_err()
    {
        return false;
    }
    matches!(tokio::time::timeout(limit, rx).await, Ok(Ok(())))
}

static SINGLE_INSTANCE_RELEASED: AtomicBool = AtomicBool::new(false);

/// Let go of the single-instance lock (the named mutex and its message
/// window on Windows, the socket on macOS). The plugin only does this in its
/// `RunEvent::Exit` handler, which a hard exit never reaches; a replacement
/// that still finds the lock hands its arguments to us and quits. Runs at
/// most once: closing the mutex handle twice could close a handle value the
/// OS has since given to something else.
fn release_single_instance_now(app: &tauri::AppHandle) {
    if SINGLE_INSTANCE_RELEASED.swap(true, Ordering::SeqCst) {
        return;
    }
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    tauri_plugin_single_instance::destroy(app);
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    let _ = app;
}

/// What Tauri's own restart releases before it starts the new process (the
/// tray icon, resource tables, and on Windows the windows themselves), plus
/// the single-instance lock. Both must run on the main thread: it created
/// the tray icon and owns the mutex. When the main thread does not answer,
/// the lock is still released from here (closing the handle frees it), and
/// the tray icon is left for the OS to drop when the process ends.
async fn release_for_replacement(app: &tauri::AppHandle) {
    let handle = app.clone();
    let done = on_main_thread(app, MAIN_THREAD_WAIT, move || {
        release_single_instance_now(&handle);
        handle.cleanup_before_exit();
    })
    .await;
    if !done {
        warn!(
            "main thread did not release the tray and single-instance lock within {}s; releasing the lock from this thread",
            MAIN_THREAD_WAIT.as_secs()
        );
        release_single_instance_now(app);
    }
}

/// True when a just-started replacement exited within `REPLACEMENT_WATCH`.
/// That is what a replacement does when it still found our single-instance
/// lock: it hands its arguments to us and quits at once.
async fn exited_at_once(child: &mut std::process::Child) -> bool {
    let deadline = Instant::now() + REPLACEMENT_WATCH;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                warn!("replacement pid {} exited at once ({status})", child.id());
                return true;
            }
            Ok(None) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            _ => return false,
        }
    }
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
    match TeardownSeverity::of(Some(&report)) {
        TeardownSeverity::Clean => {}
        TeardownSeverity::SlowCleanup => warn!(
            "safe relaunch: cleanup ran late ({}); capture and engine are stopped, relaunching",
            report.overrun_names()
        ),
        // This process ends right after the replacement starts, which takes
        // whatever is still running with it.
        TeardownSeverity::Critical => warn!(
            "safe relaunch: capture or engine did not stop in time ({}); relaunching anyway",
            report.overrun_names()
        ),
    }
    // The replacement opens the same database: an engine this process is
    // still starting (a watchdog respawn caught inside `ServerCore::start`)
    // must be retired first. The teardown already waited within its engine
    // step; a slow migration can outlast that, so wait a little longer here.
    if crate::recording::engine_transitions_pending() > 0 {
        warn!(
            "safe relaunch: an engine is still starting or stopping; waiting up to {}s before starting the replacement",
            RELAUNCH_ENGINE_WAIT.as_secs()
        );
        if tokio::time::timeout(
            RELAUNCH_ENGINE_WAIT,
            crate::recording::wait_for_engine_transitions(),
        )
        .await
        .is_err()
        {
            warn!("safe relaunch: the engine is still busy; relaunching anyway (this process exits right after)");
        }
    }
    stop_control_server(&app).await;

    let env = relaunch_env(home_visible, inherited_data_dir().as_deref());
    // The single-instance lock and the tray icon are released only once the
    // replacement exists: if it cannot be started, this instance keeps
    // running and still needs both.
    match spawn_detached(&exe, &[], &env) {
        Ok(mut child) => {
            info!(
                "safe relaunch: replacement pid {} started from {} (teardown {}ms)",
                child.id(),
                exe.display(),
                report.elapsed.as_millis()
            );
            release_for_replacement(&app).await;
            if exited_at_once(&mut child).await {
                // It reached its single-instance check before the lock was
                // released and deferred to us. The lock is gone now.
                match spawn_detached(&exe, &[], &env) {
                    Ok(again) => info!("safe relaunch: replacement pid {} started", again.id()),
                    Err(e) => error!(
                        "safe relaunch: second start of {} failed: {e}",
                        exe.display()
                    ),
                }
            }
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
            // The teardown gave back the keep-awake hold; take it again.
            crate::power_awake::apply_saved_setting(&app);
            let state = app.state::<RecordingState>();
            if let Err(err) = crate::recording::spawn_screenpipe(state, app.clone(), None).await {
                error!("safe relaunch: could not resume recording: {err}");
            }
            Err(format!("could not start a new Handvel process: {e}"))
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
    fn default_budget_covers_the_engine_shutdown_and_fits_overall() {
        let b = TeardownBudget::default();
        // ServerCore::shutdown waits up to the workers' grace, then stops the
        // scheduler, then gives the audio manager up to its own cap. A slow
        // audio stop that still succeeds must not look like a critical
        // overrun.
        assert_eq!(BACKGROUND_WORKER_GRACE, Duration::from_secs(5));
        assert_eq!(AUDIO_SHUTDOWN_LIMIT, Duration::from_secs(15));
        assert!(b.engine > BACKGROUND_WORKER_GRACE + AUDIO_SHUTDOWN_LIMIT);
        assert_eq!(b.engine, Duration::from_secs(21));
        assert_eq!(b.http_drain, Duration::from_secs(3));
        // Capture and the engine at their own limits still fit the budget.
        // Every other step runs after them, so none can shorten them.
        assert!(b.overall >= b.capture + b.engine);
        assert!(!CRITICAL_STEPS.contains(&"keep_awake"));
        assert!(b.overall >= b.engine + b.http_drain);
        assert_eq!(b.overall + GUARD_MARGIN, Duration::from_secs(32));
    }

    #[test]
    fn the_pool_line_claims_every_pool_closed_only_after_a_drain() {
        assert!(pools_closed_message(true).starts_with("all SQLite pools of the engine are closed"));
        let late = pools_closed_message(false);
        assert!(late.starts_with("engine read pool closed;"), "{late}");
        assert!(!late.contains("all SQLite pools"), "{late}");
    }

    #[tokio::test]
    async fn steps_that_overrun_are_cut_off_and_reported() {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(400);
        let mut report = TeardownReport::default();
        run_step(
            &mut report,
            "fast",
            Duration::from_secs(1),
            deadline,
            async { StepOutcome::Done },
        )
        .await;
        // Its own 50 ms limit runs out long before the overall budget.
        run_step(
            &mut report,
            "slow",
            Duration::from_millis(50),
            deadline,
            async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                StepOutcome::Done
            },
        )
        .await;
        assert!(report.overran);
        assert!(!report.budget_exhausted, "only a step limit was hit so far");
        // Its own limit is longer than what is left of the budget.
        run_step(
            &mut report,
            "stuck",
            Duration::from_secs(10),
            deadline,
            async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                StepOutcome::Done
            },
        )
        .await;
        run_step(
            &mut report,
            "late",
            Duration::from_secs(1),
            deadline,
            async { StepOutcome::Done },
        )
        .await;
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(report.overran);
        assert!(report.budget_exhausted);
        let outcomes: Vec<_> = report.steps.iter().map(|s| (s.name, s.outcome)).collect();
        assert_eq!(
            outcomes,
            vec![
                ("fast", StepOutcome::Done),
                ("slow", StepOutcome::TimedOut),
                ("stuck", StepOutcome::OutOfBudget),
                ("late", StepOutcome::OutOfBudget),
            ]
        );
        assert_eq!(report.names_with(StepOutcome::TimedOut), "slow");
        assert_eq!(report.names_with(StepOutcome::OutOfBudget), "stuck,late");
        assert_eq!(report.names_with(StepOutcome::Skipped), "none");
        assert_eq!(report.overrun_names(), "slow,stuck,late");
    }

    #[test]
    fn an_overrun_names_the_limit_that_was_hit() {
        let s = Duration::from_secs;
        // Plenty of budget left: the step's own limit ended it.
        assert_eq!(overrun_outcome(s(3), s(20)), StepOutcome::TimedOut);
        assert_eq!(overrun_outcome(s(3), s(3)), StepOutcome::TimedOut);
        // Less budget left than the step's own limit: the budget ended it.
        assert_eq!(overrun_outcome(s(17), s(5)), StepOutcome::OutOfBudget);
        assert_eq!(
            overrun_outcome(s(3), Duration::ZERO),
            StepOutcome::OutOfBudget
        );
        assert!(StepOutcome::TimedOut.is_overrun());
        assert!(StepOutcome::OutOfBudget.is_overrun());
        assert!(!StepOutcome::Done.is_overrun());
        assert!(!StepOutcome::Skipped.is_overrun());
    }

    #[test]
    fn step_warnings_say_which_limit_ended_the_step() {
        let ms = Duration::from_millis;
        assert_eq!(
            step_overrun_message("http_drain", StepOutcome::TimedOut, ms(3000), ms(3000))
                .as_deref(),
            Some("teardown step http_drain hit its own 3000ms limit; moving on")
        );
        assert_eq!(
            step_overrun_message("engine", StepOutcome::OutOfBudget, ms(1500), ms(21000)).as_deref(),
            Some("teardown step engine cut off by the overall budget after 1500ms (its own limit is 21000ms)")
        );
        assert_eq!(
            step_overrun_message("pi", StepOutcome::OutOfBudget, Duration::ZERO, ms(3000)).as_deref(),
            Some("teardown step pi not run: the overall budget was already used up (its own limit is 3000ms)")
        );
        assert_eq!(
            step_overrun_message("capture", StepOutcome::Done, ms(6000), ms(6000)),
            None
        );
        assert_eq!(
            step_overrun_message("pi", StepOutcome::Skipped, ms(3000), ms(3000)),
            None
        );
    }

    #[test]
    fn the_closing_line_tells_a_step_limit_from_the_overall_budget() {
        use StepOutcome::*;
        let overall = TeardownBudget::default().overall;

        let clean = report(&[("capture", Done), ("engine", Done), ("pi", Skipped)]);
        let (late, line) = teardown_result_line(&clean, overall);
        assert!(!late);
        assert!(
            line.starts_with("pre-exit teardown finished in 0ms: capture=Done/"),
            "{line}"
        );

        let step_late = report(&[
            ("capture", Done),
            ("engine", Done),
            ("http_drain", TimedOut),
        ]);
        let (late, line) = teardown_result_line(&step_late, overall);
        assert!(late);
        assert!(
            line.starts_with(
                "pre-exit teardown finished in 0ms within its overall budget (over their own limit: http_drain): "
            ),
            "{line}"
        );
        assert!(!line.contains("overall budget after"), "{line}");

        let budget_out = report(&[
            ("capture", Done),
            ("engine", OutOfBudget),
            ("http_drain", OutOfBudget),
            ("pi", TimedOut),
        ]);
        let (late, line) = teardown_result_line(&budget_out, overall);
        assert!(late);
        assert!(
            line.starts_with(
                "pre-exit teardown ran out of its 29s overall budget after 0ms (cut off: engine,http_drain; over their own limit: pi): "
            ),
            "{line}"
        );

        // Slack past the budget with every step on time.
        let mut slack = clean.clone();
        slack.overran = true;
        slack.budget_exhausted = true;
        let (_, line) = teardown_result_line(&slack, overall);
        assert!(
            line.contains("(cut off: none; over their own limit: none)"),
            "{line}"
        );
    }

    #[test]
    fn relaunch_env_carries_home_visibility_and_data_dir() {
        let dir = Path::new("/tmp/fp-data");
        let env = relaunch_env(false, Some(dir));
        assert_eq!(
            env,
            vec![
                ("SCREENPIPE_RELAUNCH_HOME_VISIBLE", "false".to_string()),
                ("SCREENPIPE_DATA_DIR", "/tmp/fp-data".to_string()),
            ]
        );
        assert_eq!(relaunch_env(true, Some(dir))[0].1, "true");
        // Without an inherited data dir the replacement reads the dataDir
        // setting, so a custom recording folder is kept.
        assert_eq!(
            relaunch_env(true, None),
            vec![("SCREENPIPE_RELAUNCH_HOME_VISIBLE", "true".to_string())]
        );
        // The replacement reads back exactly what we wrote.
        assert_eq!(parse_home_visible(Some(&env[0].1)), Some(false));
        assert_eq!(parse_home_visible(Some("true")), Some(true));
        assert_eq!(parse_home_visible(Some(" TRUE ")), Some(true));
        assert_eq!(parse_home_visible(Some("0")), Some(false));
        assert_eq!(parse_home_visible(Some("maybe")), None);
        assert_eq!(parse_home_visible(None), None);
    }

    #[test]
    fn relaunch_flag_is_removed_once_read() {
        // A key of its own, so no other test sees the variable change.
        let key = "FREENPIPE_TEST_W2_26_RELAUNCH_FLAG";
        std::env::set_var(key, "false");
        assert_eq!(take_home_visible_flag(key), Some(false));
        assert!(std::env::var_os(key).is_none());
        // Nothing left for a later restart to inherit.
        assert_eq!(take_home_visible_flag(key), None);
    }

    #[test]
    fn the_relaunch_flag_is_captured_once_and_then_only_read_from_the_cache() {
        let key = "FREENPIPE_TEST_W2_26_RELAUNCH_CAPTURE";
        let cell = std::sync::OnceLock::new();
        std::env::set_var(key, "true");
        assert_eq!(capture_home_visible_flag(&cell, key), Some(true));
        assert!(std::env::var_os(key).is_none());
        // A second capture keeps the first answer and does not touch the
        // environment again (no remove_var once threads may be running).
        std::env::set_var(key, "false");
        assert_eq!(capture_home_visible_flag(&cell, key), Some(true));
        assert_eq!(std::env::var(key).as_deref(), Ok("false"));
        std::env::remove_var(key);
        // Without a capture there is no relaunch flag, whatever the
        // environment says: the getter never reads it.
        let empty: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
        assert_eq!(empty.get().copied().flatten(), None);
    }

    fn report(steps: &[(&'static str, StepOutcome)]) -> TeardownReport {
        TeardownReport {
            steps: steps
                .iter()
                .map(|(name, outcome)| StepReport {
                    name,
                    outcome: *outcome,
                    elapsed: Duration::ZERO,
                })
                .collect(),
            elapsed: Duration::ZERO,
            overran: steps.iter().any(|(_, o)| o.is_overrun()),
            budget_exhausted: steps.iter().any(|(_, o)| *o == StepOutcome::OutOfBudget),
        }
    }

    #[test]
    fn only_capture_or_engine_overruns_are_critical() {
        use StepOutcome::*;
        use TeardownSeverity::*;
        let clean = report(&[
            ("capture", Done),
            ("engine", Done),
            ("http_drain", Done),
            ("sqlite_pools", Skipped),
            ("pi", Done),
        ]);
        assert_eq!(TeardownSeverity::of(Some(&clean)), Clean);
        for slow in ["http_drain", "sqlite_pools", "pi"] {
            for how in [TimedOut, OutOfBudget] {
                let r = report(&[("capture", Done), ("engine", Done), (slow, how)]);
                assert_eq!(
                    TeardownSeverity::of(Some(&r)),
                    SlowCleanup,
                    "{slow} {how:?}"
                );
            }
        }
        for critical in ["capture", "engine"] {
            for how in [TimedOut, OutOfBudget] {
                let r = report(&[(critical, how), ("pi", Done)]);
                assert_eq!(
                    TeardownSeverity::of(Some(&r)),
                    Critical,
                    "{critical} {how:?}"
                );
            }
        }
        // A panicked teardown thread.
        assert_eq!(TeardownSeverity::of(None), Critical);
        // Overall budget blown by scheduling slack, no step overran.
        let mut late = clean.clone();
        late.budget_exhausted = true;
        late.overran = true;
        assert_eq!(TeardownSeverity::of(Some(&late)), SlowCleanup);
    }

    #[test]
    fn a_tray_quit_leaves_normally_with_status_0_unless_capture_or_engine_overran() {
        use StepOutcome::*;
        // The ordinary quit after a pause/resume: the drain waits on a
        // database handle a detached task still holds, and times out.
        let drain_late = report(&[
            ("capture", Done),
            ("engine", Done),
            ("http_drain", TimedOut),
            ("sqlite_pools", Done),
            ("pi", Done),
        ]);
        let exit = quit_exit(TeardownSeverity::of(Some(&drain_late)));
        assert_eq!(exit, QuitExit::Normal);
        assert_eq!(exit.status(), 0);
        let clean = report(&[("capture", Done), ("engine", Done)]);
        assert_eq!(
            quit_exit(TeardownSeverity::of(Some(&clean))),
            QuitExit::Normal
        );
        for (critical, how) in [("capture", TimedOut), ("engine", OutOfBudget)] {
            let r = report(&[(critical, how), ("http_drain", Done)]);
            let exit = quit_exit(TeardownSeverity::of(Some(&r)));
            assert_eq!(exit, QuitExit::Forced, "{critical}");
            assert_eq!(exit.status(), 1);
        }
    }

    #[test]
    fn exit_follow_up_returns_to_tauri_unless_capture_or_engine_overran() {
        use ExitFollowUp::*;
        use StepOutcome::*;
        let clean = report(&[
            ("capture", Done),
            ("engine", Done),
            ("http_drain", Done),
            ("sqlite_pools", Skipped),
            ("pi", Done),
        ]);
        assert_eq!(exit_follow_up(Some(&clean), false), Return);
        assert_eq!(exit_follow_up(Some(&clean), true), Return);

        // Slow HTTP drain, pools or Pi: log and let Tauri finish (tray
        // cleanup, its own exit status, a pending restart), never a hard exit.
        for slow in ["http_drain", "sqlite_pools", "pi"] {
            for how in [TimedOut, OutOfBudget] {
                let r = report(&[("capture", Done), ("engine", Done), (slow, how)]);
                assert_eq!(exit_follow_up(Some(&r), true), Return, "{slow} {how:?}");
                assert_eq!(exit_follow_up(Some(&r), false), Return, "{slow} {how:?}");
            }
        }

        // Capture or engine still running: never return into C++ teardown,
        // but start the replacement first when a restart is pending.
        for critical in ["capture", "engine"] {
            let r = report(&[(critical, TimedOut), ("http_drain", TimedOut)]);
            assert_eq!(
                exit_follow_up(Some(&r), true),
                ReplaceThenForceExit,
                "{critical}"
            );
            assert_eq!(exit_follow_up(Some(&r), false), ForceExit, "{critical}");
            let cut = report(&[(critical, OutOfBudget)]);
            assert_eq!(exit_follow_up(Some(&cut), false), ForceExit, "{critical}");
        }

        // A panicked teardown thread counts as a critical overrun.
        assert_eq!(exit_follow_up(None, true), ReplaceThenForceExit);
        assert_eq!(exit_follow_up(None, false), ForceExit);

        // Overall budget blown without an overrun step (scheduling delay).
        let mut late = clean.clone();
        late.overran = true;
        late.budget_exhausted = true;
        assert_eq!(exit_follow_up(Some(&late), true), Return);
        assert_eq!(exit_follow_up(Some(&late), false), Return);
    }
}
