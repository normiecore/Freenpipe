// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Who holds one of our local ports, and whether Freenpipe may take it back.
//!
//! The base app terminated every other process it found listening on the API
//! and control ports. That is unsafe on a machine that also runs another
//! screenpipe-family app, a development server, or anything else that happens
//! to use the same port. This module classifies each listener first and acts
//! only in the one case where acting is safe:
//!
//! | class        | what it is                                              | action     |
//! |--------------|---------------------------------------------------------|------------|
//! | `OwnProcess` | this process (an engine generation still closing)       | wait       |
//! | `Healthy`    | a screenpipe-family instance that answers on the port   | never kill |
//! | `Unhealthy`  | another copy of *this executable* that does not answer  | terminate  |
//! | `Unverified` | a copy of this executable sharing the port with others  | never kill |
//! | `Foreign`    | anything else                                           | never kill |
//!
//! "This executable" means the same image path as `std::env::current_exe()`.
//! A process whose image path cannot be read is never treated as ours, so it
//! can never be terminated. Before terminating, the holder must also fail a
//! second probe and must still resolve to our executable and still listen on
//! the port, which guards against a PID being reused in between.
//!
//! The HTTP probe reaches whichever socket accepts on `127.0.0.1:<port>`, so
//! its answer describes a holder only when that holder is the only process
//! listening. With several listeners nobody is classified from the probe;
//! while this process is one of them, nothing is decided until it lets go.
//!
//! The decision logic is pure (`classify`, `plan`, the output parsers) and is
//! unit-tested with `netstat` and `lsof` fixtures. The platform operations sit
//! behind [`PortOps`], so the orchestration in [`resolve`] runs against fakes
//! in tests.

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// How long one HTTP probe of the port may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// Pause between the first silent probe and the confirming one.
const CONFIRM_PAUSE: Duration = Duration::from_secs(2);
/// How long to wait for a terminated holder to release the port.
const RELEASE_WAIT: Duration = Duration::from_secs(10);
const RELEASE_POLL: Duration = Duration::from_millis(250);
/// Classify, terminate and re-check rounds before giving up.
const MAX_ROUNDS: usize = 3;
/// How long one round waits for this process to stop listening (an engine
/// generation that is still closing) before classifying the others.
const SELF_RELEASE_WAIT: Duration = Duration::from_secs(10);
/// The same conflict notification is not repeated more often than this.
const NOTIFY_COOLDOWN: Duration = Duration::from_secs(600);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// What a listener on the port turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HolderClass {
    /// This process. Usually an engine generation that has not finished
    /// closing; the caller waits for it instead of acting.
    OwnProcess,
    /// A screenpipe-family instance that answers HTTP on the port. Never
    /// terminated: the user is told another instance owns the port.
    Healthy,
    /// Another copy of this executable that gives no HTTP answer. The only
    /// class Freenpipe terminates.
    Unhealthy,
    /// Another copy of this executable whose health could not be checked
    /// because other processes listen on the same port: the probe may have
    /// been answered by any of them. Never terminated.
    Unverified,
    /// Anything else: another product, a dev server, a process whose image
    /// cannot be read, an unresponsive copy of a different executable.
    /// Never terminated.
    Foreign,
}

impl HolderClass {
    pub fn as_str(self) -> &'static str {
        match self {
            HolderClass::OwnProcess => "self",
            HolderClass::Healthy => "healthy",
            HolderClass::Unhealthy => "unhealthy",
            HolderClass::Unverified => "unverified",
            HolderClass::Foreign => "foreign",
        }
    }
}

/// What the OS told us about a process.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessIdentity {
    pub pid: u32,
    /// File name of the image, for messages ("node.exe").
    pub image_name: Option<String>,
    /// Full image path, used for the own-executable test.
    pub exe_path: Option<PathBuf>,
}

impl ProcessIdentity {
    pub fn unknown(pid: u32) -> Self {
        Self {
            pid,
            ..Default::default()
        }
    }

    /// "name (pid N)" or "pid N".
    pub fn describe(&self) -> String {
        match &self.image_name {
            Some(name) => format!("{name} (pid {})", self.pid),
            None => format!("pid {}", self.pid),
        }
    }
}

fn describe_all(ids: &[ProcessIdentity]) -> String {
    ids.iter()
        .map(ProcessIdentity::describe)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Result of one HTTP probe of `GET /health` on the port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthProbe {
    /// A screenpipe-style health document came back (any HTTP status).
    ScreenpipeHealth,
    /// Some HTTP response came back, but not a screenpipe health document.
    OtherHttp,
    /// No HTTP answer within the timeout: refused, reset or hung.
    NoAnswer,
}

impl HealthProbe {
    pub fn answered(self) -> bool {
        !matches!(self, HealthProbe::NoAnswer)
    }
}

/// A classified listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortHolder {
    pub identity: ProcessIdentity,
    pub class: HolderClass,
    /// Image name looks like screenpipe or Freenpipe. Only changes the
    /// wording of the notification (another screenpipe app vs some program).
    pub screenpipe_family: bool,
    /// Same executable as this process.
    pub own_executable: bool,
}

/// What to do about the listeners found on a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictPlan {
    /// Nobody but this process listens.
    Clear,
    /// This process still listens next to other processes. A probe could be
    /// answered by either, so nobody is classified from it: wait for ours
    /// to close, then look again.
    WaitForSelf,
    /// Terminate these unhealthy copies of our executable, then re-check.
    Reclaim(Vec<u32>),
    /// Leave the port alone; this holder keeps us from binding.
    Blocked(PortHolder),
}

/// Final answer of [`resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReclaimOutcome {
    /// No other process listens on the port.
    Free,
    /// Unhealthy copies of our executable were terminated and the port is free.
    Reclaimed { terminated: Vec<ProcessIdentity> },
    /// A healthy or foreign process holds the port. Nothing was terminated.
    Blocked(PortHolder),
    /// Unhealthy copies of our executable could not be terminated, or the
    /// port stayed busy after they were.
    ReclaimFailed { holders: Vec<ProcessIdentity> },
}

impl ReclaimOutcome {
    /// True when the caller may go on and bind the port.
    pub fn port_available(&self) -> bool {
        matches!(
            self,
            ReclaimOutcome::Free | ReclaimOutcome::Reclaimed { .. }
        )
    }
}

/// Which of our ports is being checked; only changes messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortPurpose {
    EngineApi,
    ControlServer,
}

impl PortPurpose {
    fn label(self) -> &'static str {
        match self {
            PortPurpose::EngineApi => "engine API",
            PortPurpose::ControlServer => "control server",
        }
    }
}

// ---------------------------------------------------------------------------
// Pure decision logic
// ---------------------------------------------------------------------------

/// Whether two image paths name the same executable. Windows paths compare
/// case-insensitively, with either slash, and without the `\\?\` verbatim
/// prefix that `canonicalize` adds.
pub fn same_executable(a: &Path, b: &Path) -> bool {
    same_executable_with(a, b, cfg!(windows))
}

fn same_executable_with(a: &Path, b: &Path, windows_rules: bool) -> bool {
    fn norm(p: &Path, windows_rules: bool) -> String {
        let raw = p.to_string_lossy();
        let mut s = if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else {
            raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string()
        };
        if windows_rules {
            s = s.replace('/', "\\").to_lowercase();
        }
        s.trim_end_matches(['/', '\\']).to_string()
    }
    let (na, nb) = (norm(a, windows_rules), norm(b, windows_rules));
    !na.is_empty() && na == nb
}

/// True when the identity's image is our own executable. Unknown paths are
/// never ours.
pub fn is_own_executable(identity: &ProcessIdentity, own_exe: Option<&Path>) -> bool {
    match (own_exe, identity.exe_path.as_deref()) {
        (Some(own), Some(theirs)) => same_executable(own, theirs),
        _ => false,
    }
}

/// Image name looks like a screenpipe-family binary.
pub fn is_screenpipe_family(identity: &ProcessIdentity) -> bool {
    let name = identity
        .image_name
        .clone()
        .or_else(|| {
            identity
                .exe_path
                .as_deref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
        .to_lowercase();
    name.contains("screenpipe") || name.contains("freenpipe")
}

/// Classify one listener from its identity and the probe result that
/// belongs to it. `probe` is `None` when the port's probe cannot be
/// attributed to this holder (several processes listen): then only its
/// identity counts, and nothing is ever `Unhealthy` or `Healthy`.
pub fn classify(
    identity: &ProcessIdentity,
    self_pid: u32,
    own_exe: Option<&Path>,
    probe: Option<HealthProbe>,
) -> PortHolder {
    let own_executable = is_own_executable(identity, own_exe);
    let screenpipe_family = own_executable || is_screenpipe_family(identity);
    let class = if identity.pid == self_pid {
        HolderClass::OwnProcess
    } else if own_executable {
        match probe {
            Some(p) if p.answered() => HolderClass::Healthy,
            Some(_) => HolderClass::Unhealthy,
            None => HolderClass::Unverified,
        }
    } else if probe == Some(HealthProbe::ScreenpipeHealth) {
        HolderClass::Healthy
    } else {
        HolderClass::Foreign
    };
    PortHolder {
        identity: identity.clone(),
        class,
        screenpipe_family,
        own_executable,
    }
}

/// The holder a blocked port is reported against: a healthy one first (the
/// message can name another running instance), then a foreign one, then an
/// unverified copy of ours. `None` when no holder blocks.
fn first_blocker(holders: &[PortHolder]) -> Option<&PortHolder> {
    [
        HolderClass::Healthy,
        HolderClass::Foreign,
        HolderClass::Unverified,
    ]
    .iter()
    .find_map(|class| holders.iter().find(|h| h.class == *class))
}

/// Decide what to do about a set of classified listeners. While this
/// process is one of several listeners nothing is decided (`WaitForSelf`).
/// A healthy, foreign or unverified holder blocks the port, so nothing is
/// terminated while one is present (terminating our own unhealthy copy
/// would not free the port). Only `Unhealthy` holders ever appear in a
/// `Reclaim` list.
pub fn plan(holders: &[PortHolder]) -> ConflictPlan {
    let ours = holders.iter().any(|h| h.class == HolderClass::OwnProcess);
    let others = holders.iter().any(|h| h.class != HolderClass::OwnProcess);
    if ours && others {
        return ConflictPlan::WaitForSelf;
    }
    if let Some(blocker) = first_blocker(holders) {
        return ConflictPlan::Blocked(blocker.clone());
    }
    let kill: Vec<u32> = holders
        .iter()
        .filter(|h| h.class == HolderClass::Unhealthy)
        .map(|h| h.identity.pid)
        .collect();
    if kill.is_empty() {
        ConflictPlan::Clear
    } else {
        ConflictPlan::Reclaim(kill)
    }
}

/// Whether an HTTP body is a screenpipe health document: a JSON object with
/// a string `status` and at least one of the capture-status fields.
pub fn looks_like_screenpipe_health(body: &str) -> bool {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    let has_status = map.get("status").map_or(false, |v| v.is_string());
    let has_capture_field = [
        "frame_status",
        "audio_status",
        "last_frame_timestamp",
        "last_audio_timestamp",
        "status_code",
    ]
    .iter()
    .any(|k| map.contains_key(*k));
    has_status && has_capture_field
}

/// PIDs of TCP sockets listening on `port`, from `netstat -ano` output.
/// A listener is recognised by its foreign port being 0, which (unlike the
/// state column) is not localised.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_netstat_listeners(text: &str, port: u16) -> Vec<u32> {
    let wanted = port.to_string();
    let mut pids = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 5 || !cols[0].to_ascii_uppercase().starts_with("TCP") {
            continue;
        }
        let local_port = cols[1].rsplit_once(':').map(|(_, p)| p);
        let foreign_port = cols[2].rsplit_once(':').map(|(_, p)| p);
        if local_port != Some(wanted.as_str()) || foreign_port != Some("0") {
            continue;
        }
        if let Ok(pid) = cols[cols.len() - 1].parse::<u32>() {
            if pid != 0 && !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }
    pids
}

/// Listener PIDs and command names from `lsof -Fpc` output: a `p<pid>`
/// line followed by its `c<command>` line.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn parse_lsof_fields(text: &str) -> Vec<(u32, Option<String>)> {
    let mut out: Vec<(u32, Option<String>)> = Vec::new();
    let mut current: Option<usize> = None;
    for line in text.lines() {
        if let Some(pid) = line.strip_prefix('p') {
            current = None;
            if let Ok(pid) = pid.trim().parse::<u32>() {
                current = Some(match out.iter().position(|(p, _)| *p == pid) {
                    Some(i) => i,
                    None => {
                        out.push((pid, None));
                        out.len() - 1
                    }
                });
            }
        } else if let Some(cmd) = line.strip_prefix('c') {
            if let Some(i) = current {
                if out[i].1.is_none() && !cmd.trim().is_empty() {
                    out[i].1 = Some(cmd.trim().to_string());
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

/// Platform operations [`resolve`] needs. [`SystemPortOps`] is the real
/// one; tests use fakes.
pub(crate) trait PortOps {
    /// PIDs listening on the TCP port (may include this process).
    async fn listeners(&self, port: u16) -> Vec<u32>;
    async fn identify(&self, pid: u32) -> ProcessIdentity;
    async fn probe(&self, port: u16) -> HealthProbe;
    /// Terminate the process. True when the OS accepted the request.
    async fn terminate(&self, pid: u32) -> bool;
    async fn sleep(&self, duration: Duration);
}

/// Classify every listener on `port`. The port is probed only when exactly
/// one process listens and it is not this one: the probe reaches whichever
/// socket accepts on `127.0.0.1:<port>`, so with several listeners its
/// answer could belong to any of them and is never used to classify one.
async fn classify_listeners<P: PortOps>(
    ops: &P,
    port: u16,
    self_pid: u32,
    own_exe: Option<&Path>,
) -> Vec<PortHolder> {
    let pids = ops.listeners(port).await;
    let probe = match pids.as_slice() {
        [only] if *only != self_pid => Some(ops.probe(port).await),
        _ => None,
    };
    let mut holders = Vec::with_capacity(pids.len());
    for pid in pids {
        let identity = if pid == self_pid {
            ProcessIdentity::unknown(pid)
        } else {
            ops.identify(pid).await
        };
        holders.push(classify(&identity, self_pid, own_exe, probe));
    }
    holders
}

/// Wait (up to `SELF_RELEASE_WAIT`) for this process to stop listening on
/// `port`, which is what an engine generation that is still closing does.
async fn wait_for_self_release<P: PortOps>(ops: &P, port: u16, self_pid: u32) {
    let polls = (SELF_RELEASE_WAIT.as_millis() / RELEASE_POLL.as_millis()).max(1);
    for _ in 0..polls {
        if !ops.listeners(port).await.contains(&self_pid) {
            return;
        }
        ops.sleep(RELEASE_POLL).await;
    }
}

/// Classify the holders of `port` and terminate only unhealthy copies of our
/// own executable. Healthy, foreign and unverified processes are never
/// touched.
pub(crate) async fn resolve<P: PortOps>(
    ops: &P,
    port: u16,
    self_pid: u32,
    own_exe: Option<&Path>,
) -> ReclaimOutcome {
    let mut terminated: Vec<ProcessIdentity> = Vec::new();
    let mut last_unhealthy: Vec<ProcessIdentity> = Vec::new();

    for round in 0..MAX_ROUNDS {
        let holders = classify_listeners(ops, port, self_pid, own_exe).await;
        for h in holders
            .iter()
            .filter(|h| h.class != HolderClass::OwnProcess)
        {
            info!(
                "port {port}: {} classified as {}{}",
                h.identity.describe(),
                h.class.as_str(),
                if h.own_executable {
                    " (same executable)"
                } else {
                    ""
                }
            );
        }
        let pids = match plan(&holders) {
            ConflictPlan::Clear => {
                return if terminated.is_empty() {
                    ReclaimOutcome::Free
                } else {
                    ReclaimOutcome::Reclaimed { terminated }
                };
            }
            ConflictPlan::Blocked(holder) => return ReclaimOutcome::Blocked(holder),
            ConflictPlan::WaitForSelf => {
                info!(
                    "port {port}: this process still listens next to another; waiting for it to let go before classifying (round {round})"
                );
                wait_for_self_release(ops, port, self_pid).await;
                continue;
            }
            ConflictPlan::Reclaim(pids) => pids,
        };
        last_unhealthy = holders
            .iter()
            .filter(|h| pids.contains(&h.identity.pid))
            .map(|h| h.identity.clone())
            .collect();

        // Confirm: a busy instance gets a second chance to answer. The whole
        // classification is repeated, so a listener that appeared meanwhile
        // makes the probe unattributable and nothing is terminated.
        ops.sleep(CONFIRM_PAUSE).await;
        let confirm = classify_listeners(ops, port, self_pid, own_exe).await;
        if plan(&confirm) != ConflictPlan::Reclaim(pids.clone()) {
            debug!(
                "port {port}: the holders changed or answered the confirming probe (round {round})"
            );
            continue;
        }

        let mut asked = Vec::new();
        for pid in pids {
            // Re-check identity and listening right before acting, in case
            // the PID was reused or the holder went away on its own.
            let again = ops.identify(pid).await;
            if !is_own_executable(&again, own_exe) {
                warn!("port {port}: pid {pid} no longer resolves to this executable; leaving it");
                continue;
            }
            if !ops.listeners(port).await.contains(&pid) {
                debug!("port {port}: pid {pid} stopped listening by itself");
                continue;
            }
            warn!(
                "port {port}: terminating an unresponsive copy of this executable, {}",
                again.describe()
            );
            if ops.terminate(pid).await {
                asked.push(again);
            } else {
                warn!("port {port}: could not terminate pid {pid}");
            }
        }

        // Wait for the terminated holders to let go of the port.
        let polls = (RELEASE_WAIT.as_millis() / RELEASE_POLL.as_millis()).max(1);
        for _ in 0..polls {
            if asked.is_empty() {
                break;
            }
            let now = ops.listeners(port).await;
            if !asked.iter().any(|id| now.contains(&id.pid)) {
                break;
            }
            ops.sleep(RELEASE_POLL).await;
        }
        for id in asked {
            if !terminated.iter().any(|t| t.pid == id.pid) {
                terminated.push(id);
            }
        }
    }

    // Rounds exhausted: report whatever still holds the port.
    let holders = classify_listeners(ops, port, self_pid, own_exe).await;
    match plan(&holders) {
        ConflictPlan::Clear if !terminated.is_empty() => ReclaimOutcome::Reclaimed { terminated },
        ConflictPlan::Clear => ReclaimOutcome::Free,
        ConflictPlan::Blocked(holder) => ReclaimOutcome::Blocked(holder),
        // This process never let go. The others were classified without a
        // probe (foreign or unverified), so report one and touch none.
        ConflictPlan::WaitForSelf => match first_blocker(&holders) {
            Some(holder) => ReclaimOutcome::Blocked(holder.clone()),
            None => ReclaimOutcome::Free,
        },
        ConflictPlan::Reclaim(pids) => ReclaimOutcome::ReclaimFailed {
            holders: if last_unhealthy.is_empty() {
                holders
                    .into_iter()
                    .filter(|h| pids.contains(&h.identity.pid))
                    .map(|h| h.identity)
                    .collect()
            } else {
                last_unhealthy
            },
        },
    }
}

// ---------------------------------------------------------------------------
// Real platform operations
// ---------------------------------------------------------------------------

/// Real [`PortOps`]: `netstat`/`lsof`, the OS process table, HTTP, and
/// `taskkill`/`kill`.
pub(crate) struct SystemPortOps;

impl PortOps for SystemPortOps {
    async fn listeners(&self, port: u16) -> Vec<u32> {
        system::listeners(port).await
    }

    async fn identify(&self, pid: u32) -> ProcessIdentity {
        tokio::task::spawn_blocking(move || system::identify(pid))
            .await
            .unwrap_or_else(|_| ProcessIdentity::unknown(pid))
    }

    async fn probe(&self, port: u16) -> HealthProbe {
        probe_health(port).await
    }

    async fn terminate(&self, pid: u32) -> bool {
        system::terminate(pid).await
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// `GET http://127.0.0.1:<port>/health` without credentials (the route is
/// open), classified as screenpipe health, other HTTP, or no answer.
pub async fn probe_health(port: u16) -> HealthProbe {
    let client = match reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .no_proxy()
        .build()
    {
        Ok(c) => c,
        Err(_) => return HealthProbe::NoAnswer,
    };
    let response = match client
        .get(format!("http://127.0.0.1:{port}/health"))
        .header("Cache-Control", "no-cache")
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return HealthProbe::NoAnswer,
    };
    match response.text().await {
        Ok(body) if looks_like_screenpipe_health(&body) => HealthProbe::ScreenpipeHealth,
        _ => HealthProbe::OtherHttp,
    }
}

/// Whether something (this process included) holds `127.0.0.1:<port>`.
/// Cheap check used while waiting for a port to free up.
pub async fn port_in_use(port: u16) -> bool {
    tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .is_err()
}

/// Whether the engine could take `port` now: nothing accepts on
/// `127.0.0.1:<port>` and the OS lists no other process listening on it
/// (which also catches a wildcard listener that a loopback bind would not
/// collide with on Windows).
pub async fn port_free_for_engine(port: u16) -> bool {
    if port_in_use(port).await {
        return false;
    }
    let me = std::process::id();
    !system::listeners(port).await.iter().any(|pid| *pid != me)
}

mod system {
    use super::ProcessIdentity;
    #[allow(unused_imports)]
    use tracing::debug;

    #[cfg(windows)]
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Run a short diagnostic command with a 5 s ceiling (lsof can hang on
    /// macOS). `kill_on_drop` reaps the child if the timeout fires.
    #[cfg(any(windows, unix))]
    async fn run_with_timeout(mut cmd: tokio::process::Command) -> Option<std::process::Output> {
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let child = cmd.spawn().ok()?;
        match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output())
            .await
        {
            Ok(Ok(output)) => Some(output),
            _ => {
                debug!("port probe command timed out or failed");
                None
            }
        }
    }

    #[cfg(windows)]
    pub async fn listeners(port: u16) -> Vec<u32> {
        let mut netstat = tokio::process::Command::new("netstat");
        netstat
            .args(["-ano", "-p", "tcp"])
            .creation_flags(CREATE_NO_WINDOW);
        match run_with_timeout(netstat).await {
            Some(o) => super::parse_netstat_listeners(&String::from_utf8_lossy(&o.stdout), port),
            None => Vec::new(),
        }
    }

    #[cfg(unix)]
    pub async fn listeners(port: u16) -> Vec<u32> {
        let mut lsof = tokio::process::Command::new("lsof");
        lsof.args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-Fpc"]);
        match run_with_timeout(lsof).await {
            Some(o) => super::parse_lsof_fields(&String::from_utf8_lossy(&o.stdout))
                .into_iter()
                .map(|(pid, _)| pid)
                .collect(),
            None => Vec::new(),
        }
    }

    #[cfg(not(any(windows, unix)))]
    pub async fn listeners(_port: u16) -> Vec<u32> {
        Vec::new()
    }

    /// Image path through `QueryFullProcessImageNameW`, which only needs
    /// `PROCESS_QUERY_LIMITED_INFORMATION` (granted even for most elevated
    /// processes).
    #[cfg(windows)]
    pub fn identify(pid: u32) -> ProcessIdentity {
        use windows::core::PWSTR;
        use windows::Win32::Foundation::{CloseHandle, BOOL};
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        };

        let mut identity = ProcessIdentity::unknown(pid);
        // SAFETY: plain Win32 calls on a handle we own and close; the buffer
        // outlives the call and its capacity is passed in `len`.
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL::from(false), pid)
            else {
                return identity;
            };
            let mut buf = vec![0u16; 32768];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(handle);
            if ok {
                let path = std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
                identity.image_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                identity.exe_path = Some(path);
            }
        }
        identity
    }

    #[cfg(not(windows))]
    pub fn identify(pid: u32) -> ProcessIdentity {
        use sysinfo::{Pid, PidExt, ProcessExt, System, SystemExt};
        let mut identity = ProcessIdentity::unknown(pid);
        let mut sys = System::new();
        let spid = Pid::from_u32(pid);
        if sys.refresh_process(spid) {
            if let Some(p) = sys.process(spid) {
                let exe = p.exe();
                if !exe.as_os_str().is_empty() {
                    identity.exe_path = Some(exe.to_path_buf());
                }
                let name = p.name();
                identity.image_name = if name.is_empty() {
                    exe.file_name().map(|n| n.to_string_lossy().into_owned())
                } else {
                    Some(name.to_string())
                };
            }
        }
        identity
    }

    #[cfg(windows)]
    pub async fn terminate(pid: u32) -> bool {
        let mut taskkill = tokio::process::Command::new("taskkill");
        taskkill
            .args(["/PID", &pid.to_string(), "/F"])
            .creation_flags(CREATE_NO_WINDOW);
        matches!(run_with_timeout(taskkill).await, Some(o) if o.status.success())
    }

    /// SIGTERM, then SIGKILL if the process is still there after 2 s.
    #[cfg(unix)]
    pub async fn terminate(pid: u32) -> bool {
        let signal = |sig: &'static str| {
            let mut kill = tokio::process::Command::new("kill");
            kill.args([sig, &pid.to_string()]);
            run_with_timeout(kill)
        };
        if !matches!(signal("-TERM").await, Some(o) if o.status.success()) {
            return false;
        }
        for _ in 0..8 {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            if !matches!(signal("-0").await, Some(o) if o.status.success()) {
                return true;
            }
        }
        matches!(signal("-KILL").await, Some(o) if o.status.success())
    }

    #[cfg(not(any(windows, unix)))]
    pub async fn terminate(_pid: u32) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// App-facing entry points
// ---------------------------------------------------------------------------

/// The last outcome that kept the engine from starting, so the watchdog can
/// tell "the port is taken" from other start failures.
static LAST_ENGINE_BLOCK: Lazy<Mutex<Option<ReclaimOutcome>>> = Lazy::new(|| Mutex::new(None));

pub fn last_engine_block() -> Option<ReclaimOutcome> {
    LAST_ENGINE_BLOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub fn clear_engine_block() {
    *LAST_ENGINE_BLOCK.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Check `port` before binding it, terminate an unhealthy copy of ourselves
/// if that is what holds it, and tell the user when something else does.
pub async fn resolve_before_bind(
    app: &tauri::AppHandle,
    port: u16,
    purpose: PortPurpose,
) -> ReclaimOutcome {
    let own_exe = std::env::current_exe().ok();
    let outcome = resolve(&SystemPortOps, port, std::process::id(), own_exe.as_deref()).await;
    match &outcome {
        ReclaimOutcome::Free => debug!("{} port {port}: no other listener", purpose.label()),
        ReclaimOutcome::Reclaimed { terminated } => info!(
            "{} port {port}: reclaimed from {}",
            purpose.label(),
            describe_all(terminated)
        ),
        ReclaimOutcome::Blocked(holder) => warn!(
            "{} port {port} is held by {} ({}); not terminating it",
            purpose.label(),
            holder.identity.describe(),
            holder.class.as_str()
        ),
        ReclaimOutcome::ReclaimFailed { holders } => warn!(
            "{} port {port}: could not reclaim it from {}",
            purpose.label(),
            describe_all(holders)
        ),
    }
    if purpose == PortPurpose::EngineApi {
        let mut guard = LAST_ENGINE_BLOCK.lock().unwrap_or_else(|e| e.into_inner());
        *guard = (!outcome.port_available()).then(|| outcome.clone());
    }
    notify(app, port, purpose, &outcome).await;
    outcome
}

/// Short text for an error message or log line.
pub fn describe_block(port: u16, outcome: &ReclaimOutcome) -> String {
    match outcome {
        ReclaimOutcome::Free | ReclaimOutcome::Reclaimed { .. } => format!("port {port} is free"),
        ReclaimOutcome::Blocked(h) if h.class == HolderClass::Healthy => format!(
            "port {port} is served by another running instance ({})",
            h.identity.describe()
        ),
        ReclaimOutcome::Blocked(h) if h.class == HolderClass::Unverified => format!(
            "port {port} is shared by several processes, including another copy of Freenpipe ({})",
            h.identity.describe()
        ),
        ReclaimOutcome::Blocked(h) => {
            format!("port {port} is in use by {}", h.identity.describe())
        }
        ReclaimOutcome::ReclaimFailed { holders } => format!(
            "port {port} is held by an unresponsive copy of Freenpipe that could not be stopped ({})",
            describe_all(holders)
        ),
    }
}

/// Notification title and body for an outcome, or `None` when the user does
/// not need to hear about it.
fn notification_text(
    port: u16,
    purpose: PortPurpose,
    outcome: &ReclaimOutcome,
    focused_other: Option<bool>,
) -> Option<(String, String)> {
    let setting_hint = match purpose {
        PortPurpose::EngineApi => "or change the API port under Settings > Recording",
        PortPurpose::ControlServer => "or set SCREENPIPE_FOCUS_PORT to a free port",
    };
    let what = purpose.label();
    match outcome {
        ReclaimOutcome::Free => None,
        ReclaimOutcome::Reclaimed { terminated } => Some((
            "Recovered a stuck Freenpipe process".to_string(),
            format!(
                "A copy of Freenpipe that had stopped responding kept local port {port} busy ({}). It was closed so Freenpipe can continue.",
                describe_all(terminated)
            ),
        )),
        ReclaimOutcome::Blocked(h) if h.class == HolderClass::Healthy && h.own_executable => {
            Some((
                "Freenpipe is already open".to_string(),
                if focused_other == Some(true) {
                    "Another Freenpipe instance is running and its window was brought forward. This copy will not record.".to_string()
                } else {
                    format!(
                        "Another Freenpipe instance is serving port {port}, but its window could not be brought forward. Quit one of the two copies from the tray."
                    )
                },
            ))
        }
        ReclaimOutcome::Blocked(h) if h.class == HolderClass::Healthy && h.screenpipe_family => {
            Some((
                "Another screenpipe is running".to_string(),
                format!(
                    "{} already answers on port {port}, so Freenpipe cannot start its {what} there. Quit the other app, {setting_hint}.",
                    h.identity.describe()
                ),
            ))
        }
        ReclaimOutcome::Blocked(h) if h.class == HolderClass::Healthy => Some((
            format!("Port {port} is busy"),
            format!(
                "{} answers on port {port} like a screenpipe server, so Freenpipe cannot start its {what} there. Quit that program, {setting_hint}.",
                h.identity.describe()
            ),
        )),
        ReclaimOutcome::Blocked(h) if h.class == HolderClass::Unverified => Some((
            format!("Port {port} is shared"),
            format!(
                "Another copy of Freenpipe, {}, listens on local port {port} together with other programs, so Freenpipe could not check whether it still responds and left it running. Quit the copy you do not need from its tray icon or Task Manager, {setting_hint}.",
                h.identity.describe()
            ),
        )),
        ReclaimOutcome::Blocked(h) if h.screenpipe_family => Some((
            "Another screenpipe app holds the port".to_string(),
            format!(
                "{} is listening on local port {port} but did not answer like a running screenpipe, so Freenpipe cannot start its {what} there. Freenpipe does not close other screenpipe apps: quit it from its tray icon or Task Manager, {setting_hint}.",
                h.identity.describe()
            ),
        )),
        ReclaimOutcome::Blocked(h) => Some((
            format!("Port {port} is busy"),
            format!(
                "{} is using local port {port}, which Freenpipe needs for its {what}. Freenpipe does not close other programs: quit that one, {setting_hint}.",
                h.identity.describe()
            ),
        )),
        ReclaimOutcome::ReclaimFailed { .. } => Some((
            format!("Could not free port {port}"),
            format!(
                "An unresponsive copy of Freenpipe still holds local port {port} and could not be closed. End it from Task Manager (Activity Monitor on macOS), {setting_hint}."
            ),
        )),
    }
}

/// Whether this process's control server is bound, so in-app notification
/// panels (which go through its `/notify` route) reach this process.
static CONTROL_SERVER_BOUND: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Marks the control server as bound for as long as it is held. The
/// server's future owns it, so cancelling or ending the server clears it.
pub(crate) struct ControlServerBound(());

impl ControlServerBound {
    pub(crate) fn mark() -> Self {
        CONTROL_SERVER_BOUND.store(true, std::sync::atomic::Ordering::SeqCst);
        Self(())
    }
}

impl Drop for ControlServerBound {
    fn drop(&mut self) {
        CONTROL_SERVER_BOUND.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// How a conflict notice reaches the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeRoute {
    /// The in-app panel, through our own control server's `/notify`.
    Panel,
    /// The operating system's notification centre (no local port involved).
    System,
}

/// The panel is posted to the control port, so it is used only when that
/// port is ours. A control-server conflict means the port is someone
/// else's: posting there would hand the message to the very process that
/// holds it.
fn notice_route(purpose: PortPurpose, control_server_bound: bool) -> NoticeRoute {
    match purpose {
        PortPurpose::ControlServer => NoticeRoute::System,
        PortPurpose::EngineApi if control_server_bound => NoticeRoute::Panel,
        PortPurpose::EngineApi => NoticeRoute::System,
    }
}

fn show_system_notification(app: &tauri::AppHandle, title: String, body: String) {
    use tauri_plugin_notification::NotificationExt;
    let app = app.clone();
    // Same as the update notices: the native call can block briefly.
    std::thread::spawn(move || {
        if let Err(e) = app.notification().builder().title(title).body(body).show() {
            warn!("port conflict: could not show a system notification: {e}");
        }
    });
}

/// Ask the other instance's control server to show its window. Only done
/// for a healthy copy of our own executable, and only when that copy (not
/// this process) owns the control port.
async fn focus_other_instance(holder_pid: u32) -> bool {
    let focus_port: u16 = std::env::var(screenpipe_core::env::SCREENPIPE_FOCUS_PORT)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(11435);
    if !system::listeners(focus_port).await.contains(&holder_pid) {
        return false;
    }
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
    else {
        return false;
    };
    matches!(
        client
            .post(format!("http://127.0.0.1:{focus_port}/focus"))
            .json(&serde_json::json!({ "args": [] }))
            .send()
            .await,
        Ok(r) if r.status().is_success()
    )
}

type NotifyKey = (PortPurpose, u16, &'static str);
static LAST_NOTIFIED: Lazy<Mutex<HashMap<NotifyKey, Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn outcome_key(outcome: &ReclaimOutcome) -> &'static str {
    match outcome {
        ReclaimOutcome::Free => "free",
        ReclaimOutcome::Reclaimed { .. } => "reclaimed",
        ReclaimOutcome::Blocked(h) => h.class.as_str(),
        ReclaimOutcome::ReclaimFailed { .. } => "reclaim_failed",
    }
}

async fn notify(app: &tauri::AppHandle, port: u16, purpose: PortPurpose, outcome: &ReclaimOutcome) {
    if matches!(outcome, ReclaimOutcome::Free) {
        return;
    }
    let key = (purpose, port, outcome_key(outcome));
    {
        let mut last = LAST_NOTIFIED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = last.get(&key) {
            if at.elapsed() < NOTIFY_COOLDOWN {
                return;
            }
        }
        last.insert(key, Instant::now());
    }
    let focused = match outcome {
        ReclaimOutcome::Blocked(h) if h.class == HolderClass::Healthy && h.own_executable => {
            Some(focus_other_instance(h.identity.pid).await)
        }
        _ => None,
    };
    let Some((title, body)) = notification_text(port, purpose, outcome, focused) else {
        return;
    };
    let route = notice_route(
        purpose,
        CONTROL_SERVER_BOUND.load(std::sync::atomic::Ordering::SeqCst),
    );
    info!(
        "{} port {port}: telling the user through the {}: {title}",
        purpose.label(),
        match route {
            NoticeRoute::Panel => "notification panel",
            NoticeRoute::System => "system notification centre",
        }
    );
    match route {
        NoticeRoute::Panel => {
            crate::notifications::client::send_typed(title, body, "system", Some(30_000))
        }
        NoticeRoute::System => show_system_notification(app, title, body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const SELF_PID: u32 = 1000;
    const OWN_EXE: &str = r"C:\Program Files\Freenpipe\freenpipe.exe";
    const VENDOR_EXE: &str = r"C:\Users\me\AppData\Local\screenpipe\screenpipe-app.exe";
    const NODE_EXE: &str = r"C:\Program Files\nodejs\node.exe";

    /// `netstat -ano -p tcp` from a Windows machine: listeners on 3030 and
    /// lookalike ports, clients connected to 3030, a UDP socket on 3030 and
    /// a localised state column.
    const NETSTAT_WINDOWS: &str = "
Active Connections

  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1100
  TCP    127.0.0.1:3030         0.0.0.0:0              LISTENING       4242
  TCP    127.0.0.1:30301        0.0.0.0:0              LISTENING       5555
  TCP    127.0.0.1:52000        127.0.0.1:3030         ESTABLISHED     7777
  TCP    127.0.0.1:3030         127.0.0.1:52000        ESTABLISHED     4242
  TCP    [::]:3030              [::]:0                 ABH\u{d6}REN         4242
  TCP    [::1]:3030             [::]:0                 LISTENING       6060
  UDP    0.0.0.0:3030           *:*                                    8080
  TCP    127.0.0.1:3035         0.0.0.0:0              LISTENING       1000
  TCP    127.0.0.1:11435        0.0.0.0:0              LISTENING       2020
";

    /// One listener on 3030 and our control server port.
    const NETSTAT_ONE: &str = "
  Proto  Local Address          Foreign Address        State           PID
  TCP    127.0.0.1:3030         0.0.0.0:0              LISTENING       4242
  TCP    127.0.0.1:11435        0.0.0.0:0              LISTENING       2020
";

    /// `lsof -nP -iTCP:3030 -sTCP:LISTEN -Fpc` from macOS or Linux.
    const LSOF_UNIX: &str = "p4242\ncscreenpipe\nf12\np6060\ncnode\nf20\np4242\ncscreenpipe\n";

    #[test]
    fn netstat_only_matches_listeners_on_the_exact_port() {
        // Clients connected *to* 3030 (pid 7777), ports that merely contain
        // "3030" (pid 5555) and UDP sockets are not listeners.
        assert_eq!(
            parse_netstat_listeners(NETSTAT_WINDOWS, 3030),
            vec![4242, 6060]
        );
        assert_eq!(parse_netstat_listeners(NETSTAT_WINDOWS, 135), vec![1100]);
        assert_eq!(parse_netstat_listeners(NETSTAT_WINDOWS, 3035), vec![1000]);
        assert!(parse_netstat_listeners(NETSTAT_WINDOWS, 4000).is_empty());
    }

    #[test]
    fn netstat_with_crlf_line_endings() {
        let text = NETSTAT_WINDOWS.replace('\n', "\r\n");
        assert_eq!(parse_netstat_listeners(&text, 3030), vec![4242, 6060]);
    }

    #[test]
    fn lsof_fields_pair_pids_with_commands() {
        assert_eq!(
            parse_lsof_fields(LSOF_UNIX),
            vec![
                (4242, Some("screenpipe".into())),
                (6060, Some("node".into()))
            ]
        );
        assert!(parse_lsof_fields("").is_empty());
    }

    #[test]
    fn executable_paths_compare_like_the_os() {
        let a = Path::new(OWN_EXE);
        assert!(same_executable_with(
            a,
            Path::new(r"\\?\c:\program files\FREENPIPE\Freenpipe.EXE"),
            true
        ));
        assert!(same_executable_with(
            a,
            Path::new("C:/Program Files/Freenpipe/freenpipe.exe"),
            true
        ));
        assert!(!same_executable_with(a, Path::new(VENDOR_EXE), true));
        // Case matters on Unix.
        assert!(!same_executable_with(
            Path::new("/opt/Freenpipe/freenpipe"),
            Path::new("/opt/freenpipe/freenpipe"),
            false
        ));
        assert!(same_executable_with(
            Path::new("/opt/freenpipe/freenpipe"),
            Path::new("/opt/freenpipe/freenpipe"),
            false
        ));
        assert!(!same_executable_with(Path::new(""), Path::new(""), false));
    }

    fn ident(pid: u32, path: Option<&str>) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            image_name: path.and_then(|p| p.rsplit(['\\', '/']).next().map(String::from)),
            exe_path: path.map(PathBuf::from),
        }
    }

    /// `same_executable` follows the host's rules, so the fixtures use a
    /// path spelled identically on both sides.
    fn own() -> Option<&'static Path> {
        Some(Path::new(OWN_EXE))
    }

    #[test]
    fn classify_covers_self_healthy_unhealthy_foreign() {
        use HealthProbe::*;
        let c = |id: &ProcessIdentity, probe| classify(id, SELF_PID, own(), Some(probe)).class;

        // Self, whatever the probe says.
        assert_eq!(
            c(&ident(SELF_PID, Some(OWN_EXE)), NoAnswer),
            HolderClass::OwnProcess
        );
        assert_eq!(
            c(&ProcessIdentity::unknown(SELF_PID), ScreenpipeHealth),
            HolderClass::OwnProcess
        );

        // Our executable: healthy when it answers anything, unhealthy when silent.
        let twin = ident(4242, Some(OWN_EXE));
        assert_eq!(c(&twin, ScreenpipeHealth), HolderClass::Healthy);
        assert_eq!(c(&twin, OtherHttp), HolderClass::Healthy);
        assert_eq!(c(&twin, NoAnswer), HolderClass::Unhealthy);

        // Another screenpipe-family executable: healthy only with a health
        // document, otherwise foreign. Never unhealthy, because it is not ours.
        let vendor = ident(4243, Some(VENDOR_EXE));
        assert_eq!(c(&vendor, ScreenpipeHealth), HolderClass::Healthy);
        assert_eq!(c(&vendor, OtherHttp), HolderClass::Foreign);
        assert_eq!(c(&vendor, NoAnswer), HolderClass::Foreign);
        assert!(classify(&vendor, SELF_PID, own(), Some(NoAnswer)).screenpipe_family);

        // Unrelated program and unreadable image: foreign.
        let node = ident(6060, Some(NODE_EXE));
        assert_eq!(c(&node, OtherHttp), HolderClass::Foreign);
        assert_eq!(c(&node, NoAnswer), HolderClass::Foreign);
        assert_eq!(
            c(&ProcessIdentity::unknown(6061), NoAnswer),
            HolderClass::Foreign
        );

        // Without knowing our own path nothing is ever "ours".
        assert_eq!(
            classify(&twin, SELF_PID, None, Some(NoAnswer)).class,
            HolderClass::Foreign
        );

        // No probe that belongs to this holder: our copy is unverified (never
        // killed, never "healthy"), anything else is foreign, even a vendor
        // screenpipe that some listener's health document might suggest.
        assert_eq!(
            classify(&twin, SELF_PID, own(), None).class,
            HolderClass::Unverified
        );
        assert_eq!(
            classify(&vendor, SELF_PID, own(), None).class,
            HolderClass::Foreign
        );
        assert_eq!(
            classify(&node, SELF_PID, own(), None).class,
            HolderClass::Foreign
        );
    }

    fn holder(pid: u32, class: HolderClass) -> PortHolder {
        PortHolder {
            identity: ProcessIdentity::unknown(pid),
            class,
            screenpipe_family: false,
            own_executable: class == HolderClass::Unhealthy,
        }
    }

    #[test]
    fn plan_never_selects_foreign_or_healthy_for_kill() {
        use HolderClass::*;
        assert_eq!(plan(&[]), ConflictPlan::Clear);
        assert_eq!(plan(&[holder(SELF_PID, OwnProcess)]), ConflictPlan::Clear);
        assert_eq!(
            plan(&[holder(1, Unhealthy), holder(2, Unhealthy)]),
            ConflictPlan::Reclaim(vec![1, 2])
        );
        // While this process still listens next to others, nothing is
        // decided, whatever the others look like.
        for other in [Healthy, Unhealthy, Unverified, Foreign] {
            assert_eq!(
                plan(&[holder(SELF_PID, OwnProcess), holder(1, other)]),
                ConflictPlan::WaitForSelf,
                "{other:?}"
            );
        }
        // An unverified copy of ours blocks like a foreign holder.
        assert_eq!(
            plan(&[holder(1, Unhealthy), holder(5, Unverified)]),
            ConflictPlan::Blocked(holder(5, Unverified))
        );
        // A foreign or healthy holder blocks, and nothing is killed even if
        // an unhealthy copy of ours sits next to it.
        assert_eq!(
            plan(&[holder(1, Unhealthy), holder(3, Foreign)]),
            ConflictPlan::Blocked(holder(3, Foreign))
        );
        assert_eq!(
            plan(&[holder(3, Foreign), holder(4, Healthy)]),
            ConflictPlan::Blocked(holder(4, Healthy))
        );
        // Exhaustive: whatever the mix, a Reclaim list holds only unhealthy pids.
        let classes = [OwnProcess, Healthy, Unhealthy, Unverified, Foreign];
        for a in classes {
            for b in classes {
                for c in classes {
                    let hs = [holder(10, a), holder(11, b), holder(12, c)];
                    if let ConflictPlan::Reclaim(pids) = plan(&hs) {
                        for pid in pids {
                            let h = hs.iter().find(|h| h.identity.pid == pid).unwrap();
                            assert_eq!(h.class, Unhealthy, "{a:?} {b:?} {c:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn screenpipe_health_documents_are_recognised() {
        assert!(looks_like_screenpipe_health(
            r#"{"status":"healthy","frame_status":"ok","audio_status":"ok","last_frame_timestamp":null}"#
        ));
        assert!(looks_like_screenpipe_health(
            r#"{"status":"degraded","status_code":503}"#
        ));
        assert!(!looks_like_screenpipe_health(r#"{"status":"ok"}"#));
        assert!(!looks_like_screenpipe_health("<html>404</html>"));
        assert!(!looks_like_screenpipe_health("[]"));
    }

    // -- orchestration against a fake OS --------------------------------

    /// A fake OS built from a netstat fixture. Probes answer in order (the
    /// last one repeats); `kill_frees` makes a terminate drop the pid's
    /// lines from the listener table.
    struct FakeOs {
        netstat: RefCell<String>,
        identities: HashMap<u32, ProcessIdentity>,
        probes: RefCell<Vec<HealthProbe>>,
        probe_calls: RefCell<usize>,
        terminated: RefCell<Vec<u32>>,
        kill_frees: bool,
        /// After this many listener queries, this process stops listening
        /// (an engine generation that finished closing).
        self_leaves_after: Option<usize>,
        listener_calls: RefCell<usize>,
    }

    impl FakeOs {
        fn new(netstat: &str, probes: Vec<HealthProbe>) -> Self {
            Self {
                netstat: RefCell::new(netstat.to_string()),
                identities: HashMap::new(),
                probes: RefCell::new(probes),
                probe_calls: RefCell::new(0),
                terminated: RefCell::new(Vec::new()),
                kill_frees: true,
                self_leaves_after: None,
                listener_calls: RefCell::new(0),
            }
        }
        fn with(mut self, id: ProcessIdentity) -> Self {
            self.identities.insert(id.pid, id);
            self
        }
        fn drop_pid(&self, pid: u32) {
            let suffix = format!(" {pid}");
            let kept: Vec<String> = self
                .netstat
                .borrow()
                .lines()
                .filter(|l| !l.trim_end().ends_with(&suffix))
                .map(String::from)
                .collect();
            *self.netstat.borrow_mut() = kept.join("\n");
        }
    }

    impl PortOps for FakeOs {
        async fn listeners(&self, port: u16) -> Vec<u32> {
            let calls = {
                let mut c = self.listener_calls.borrow_mut();
                *c += 1;
                *c
            };
            if self.self_leaves_after.map_or(false, |n| calls > n) {
                self.drop_pid(SELF_PID);
            }
            parse_netstat_listeners(&self.netstat.borrow(), port)
        }
        async fn identify(&self, pid: u32) -> ProcessIdentity {
            self.identities
                .get(&pid)
                .cloned()
                .unwrap_or_else(|| ProcessIdentity::unknown(pid))
        }
        async fn probe(&self, _port: u16) -> HealthProbe {
            *self.probe_calls.borrow_mut() += 1;
            let mut p = self.probes.borrow_mut();
            if p.len() > 1 {
                p.remove(0)
            } else {
                p.first().copied().unwrap_or(HealthProbe::NoAnswer)
            }
        }
        async fn terminate(&self, pid: u32) -> bool {
            self.terminated.borrow_mut().push(pid);
            if self.kill_frees {
                self.drop_pid(pid);
            }
            true
        }
        async fn sleep(&self, _d: Duration) {}
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn free_port_needs_no_action() {
        let os = FakeOs::new(NETSTAT_ONE, vec![HealthProbe::NoAnswer]);
        assert_eq!(
            block_on(resolve(&os, 3035, SELF_PID, own())),
            ReclaimOutcome::Free
        );
        assert!(os.terminated.borrow().is_empty());
    }

    #[test]
    fn own_process_on_the_port_is_left_alone() {
        let netstat = NETSTAT_ONE.replace("4242", &SELF_PID.to_string());
        let os = FakeOs::new(&netstat, vec![HealthProbe::NoAnswer]);
        assert_eq!(
            block_on(resolve(&os, 3030, SELF_PID, own())),
            ReclaimOutcome::Free
        );
        assert!(os.terminated.borrow().is_empty());
    }

    #[test]
    fn unhealthy_copy_of_our_executable_is_terminated() {
        let os =
            FakeOs::new(NETSTAT_ONE, vec![HealthProbe::NoAnswer]).with(ident(4242, Some(OWN_EXE)));
        let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
        assert_eq!(
            outcome,
            ReclaimOutcome::Reclaimed {
                terminated: vec![ident(4242, Some(OWN_EXE))]
            }
        );
        assert_eq!(*os.terminated.borrow(), vec![4242]);
        assert!(outcome.port_available());
    }

    #[test]
    fn copy_that_answers_the_confirming_probe_is_spared() {
        // Silent first, then it answers: classified healthy, nothing killed.
        let os = FakeOs::new(
            NETSTAT_ONE,
            vec![HealthProbe::NoAnswer, HealthProbe::ScreenpipeHealth],
        )
        .with(ident(4242, Some(OWN_EXE)));
        let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
        assert!(
            matches!(outcome, ReclaimOutcome::Blocked(ref h) if h.class == HolderClass::Healthy),
            "{outcome:?}"
        );
        assert!(os.terminated.borrow().is_empty());
    }

    #[test]
    fn foreign_holder_is_never_terminated() {
        for probe in [HealthProbe::NoAnswer, HealthProbe::OtherHttp] {
            let os = FakeOs::new(NETSTAT_WINDOWS, vec![probe])
                .with(ident(4242, Some(r"C:\tools\devserver.exe")))
                .with(ident(6060, Some(NODE_EXE)));
            let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
            assert!(
                matches!(outcome, ReclaimOutcome::Blocked(ref h) if h.class == HolderClass::Foreign),
                "{outcome:?}"
            );
            assert!(os.terminated.borrow().is_empty());
            assert!(!outcome.port_available());
        }
    }

    #[test]
    fn healthy_vendor_screenpipe_is_never_terminated() {
        let os = FakeOs::new(NETSTAT_ONE, vec![HealthProbe::ScreenpipeHealth])
            .with(ident(4242, Some(VENDOR_EXE)));
        let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
        assert!(
            matches!(outcome, ReclaimOutcome::Blocked(ref h)
                if h.class == HolderClass::Healthy && !h.own_executable),
            "{outcome:?}"
        );
        assert!(os.terminated.borrow().is_empty());
    }

    #[test]
    fn unresponsive_vendor_screenpipe_is_foreign_and_not_killed() {
        let os = FakeOs::new(NETSTAT_ONE, vec![HealthProbe::NoAnswer])
            .with(ident(4242, Some(VENDOR_EXE)));
        let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
        assert!(
            matches!(outcome, ReclaimOutcome::Blocked(ref h)
                if h.class == HolderClass::Foreign && h.screenpipe_family),
            "{outcome:?}"
        );
        assert!(os.terminated.borrow().is_empty());
    }

    #[test]
    fn unreadable_image_is_never_terminated() {
        let os = FakeOs::new(NETSTAT_ONE, vec![HealthProbe::NoAnswer]);
        let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
        assert!(
            matches!(outcome, ReclaimOutcome::Blocked(ref h) if h.class == HolderClass::Foreign),
            "{outcome:?}"
        );
        assert!(os.terminated.borrow().is_empty());
    }

    #[test]
    fn mixed_holders_kill_nothing_while_a_foreign_one_remains() {
        // 4242 is our unresponsive twin, 6060 is node: killing ours would not
        // free the port, so nothing is killed.
        let os = FakeOs::new(NETSTAT_WINDOWS, vec![HealthProbe::NoAnswer])
            .with(ident(4242, Some(OWN_EXE)))
            .with(ident(6060, Some(NODE_EXE)));
        let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
        assert!(
            matches!(outcome, ReclaimOutcome::Blocked(ref h) if h.identity.pid == 6060),
            "{outcome:?}"
        );
        assert!(os.terminated.borrow().is_empty());
    }

    #[test]
    fn stubborn_copy_reports_reclaim_failed() {
        let mut os =
            FakeOs::new(NETSTAT_ONE, vec![HealthProbe::NoAnswer]).with(ident(4242, Some(OWN_EXE)));
        os.kill_frees = false;
        let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
        assert_eq!(
            outcome,
            ReclaimOutcome::ReclaimFailed {
                holders: vec![ident(4242, Some(OWN_EXE))]
            }
        );
        assert_eq!(os.terminated.borrow().len(), MAX_ROUNDS);
    }

    #[test]
    fn pid_reused_by_another_program_is_not_terminated() {
        // Classified as our twin, but by the time we act the pid belongs to
        // another image.
        struct Reused(FakeOs, RefCell<u32>);
        impl PortOps for Reused {
            async fn listeners(&self, port: u16) -> Vec<u32> {
                self.0.listeners(port).await
            }
            async fn identify(&self, pid: u32) -> ProcessIdentity {
                let mut calls = self.1.borrow_mut();
                *calls += 1;
                if *calls == 1 {
                    ident(pid, Some(OWN_EXE))
                } else {
                    ident(pid, Some(r"C:\Windows\notepad.exe"))
                }
            }
            async fn probe(&self, port: u16) -> HealthProbe {
                self.0.probe(port).await
            }
            async fn terminate(&self, pid: u32) -> bool {
                self.0.terminate(pid).await
            }
            async fn sleep(&self, d: Duration) {
                self.0.sleep(d).await
            }
        }
        let os = Reused(
            FakeOs::new(NETSTAT_ONE, vec![HealthProbe::NoAnswer]),
            RefCell::new(0),
        );
        let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
        assert!(os.0.terminated.borrow().is_empty());
        assert!(
            matches!(outcome, ReclaimOutcome::Blocked(ref h) if h.class == HolderClass::Foreign),
            "{outcome:?}"
        );
    }

    /// Our engine generation (SELF_PID) still listens on 3030 next to an
    /// unresponsive twin (4242).
    fn netstat_self_and_twin() -> String {
        format!(
            "{NETSTAT_ONE}  TCP    0.0.0.0:3030           0.0.0.0:0              LISTENING       {SELF_PID}\n"
        )
    }

    #[test]
    fn nothing_is_decided_while_this_process_still_listens() {
        // Our closing generation never lets go: the twin is never probed on
        // its own, so it is reported as unverified and left alone.
        let os = FakeOs::new(&netstat_self_and_twin(), vec![HealthProbe::NoAnswer])
            .with(ident(4242, Some(OWN_EXE)));
        let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
        assert!(
            matches!(outcome, ReclaimOutcome::Blocked(ref h)
                if h.identity.pid == 4242 && h.class == HolderClass::Unverified),
            "{outcome:?}"
        );
        assert!(os.terminated.borrow().is_empty());
        assert_eq!(*os.probe_calls.borrow(), 0, "a shared port is never probed");
    }

    #[test]
    fn the_twin_is_judged_on_its_own_once_this_process_lets_go() {
        let mut os = FakeOs::new(&netstat_self_and_twin(), vec![HealthProbe::NoAnswer])
            .with(ident(4242, Some(OWN_EXE)));
        os.self_leaves_after = Some(2);
        let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
        assert_eq!(
            outcome,
            ReclaimOutcome::Reclaimed {
                terminated: vec![ident(4242, Some(OWN_EXE))]
            }
        );
        assert_eq!(*os.terminated.borrow(), vec![4242]);
    }

    #[test]
    fn our_answer_never_makes_a_vendor_screenpipe_look_healthy() {
        // Whatever answers on 127.0.0.1 may be our own engine; the vendor
        // copy next to it is not classified from that answer.
        let os = FakeOs::new(
            &netstat_self_and_twin(),
            vec![HealthProbe::ScreenpipeHealth],
        )
        .with(ident(4242, Some(VENDOR_EXE)));
        let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
        assert!(
            matches!(outcome, ReclaimOutcome::Blocked(ref h)
                if h.identity.pid == 4242 && h.class == HolderClass::Foreign),
            "{outcome:?}"
        );
        assert_eq!(*os.probe_calls.borrow(), 0);
    }

    #[test]
    fn two_silent_copies_of_ours_are_never_terminated() {
        // Two other copies on the same port: a silent probe could come from
        // either (or from neither, if only one accepts on 127.0.0.1).
        let netstat = format!(
            "{NETSTAT_ONE}  TCP    0.0.0.0:3030           0.0.0.0:0              LISTENING       4343\n"
        );
        let os = FakeOs::new(&netstat, vec![HealthProbe::NoAnswer])
            .with(ident(4242, Some(OWN_EXE)))
            .with(ident(4343, Some(OWN_EXE)));
        let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
        assert!(
            matches!(outcome, ReclaimOutcome::Blocked(ref h) if h.class == HolderClass::Unverified),
            "{outcome:?}"
        );
        assert!(os.terminated.borrow().is_empty());
        assert_eq!(*os.probe_calls.borrow(), 0);
    }

    #[test]
    fn a_listener_that_appears_before_the_confirmation_stops_the_kill() {
        // Round 1 sees only the silent twin; by the confirming look a node
        // server listens too, so the second probe cannot be attributed.
        struct Joining(FakeOs, RefCell<usize>);
        impl PortOps for Joining {
            async fn listeners(&self, port: u16) -> Vec<u32> {
                *self.1.borrow_mut() += 1;
                if *self.1.borrow() == 2 {
                    let joined = format!(
                        "{}\n  TCP    [::1]:3030             [::]:0                 LISTENING       6060",
                        self.0.netstat.borrow()
                    );
                    *self.0.netstat.borrow_mut() = joined;
                }
                self.0.listeners(port).await
            }
            async fn identify(&self, pid: u32) -> ProcessIdentity {
                self.0.identify(pid).await
            }
            async fn probe(&self, port: u16) -> HealthProbe {
                self.0.probe(port).await
            }
            async fn terminate(&self, pid: u32) -> bool {
                self.0.terminate(pid).await
            }
            async fn sleep(&self, d: Duration) {
                self.0.sleep(d).await
            }
        }
        let os = Joining(
            FakeOs::new(NETSTAT_ONE, vec![HealthProbe::NoAnswer])
                .with(ident(4242, Some(OWN_EXE)))
                .with(ident(6060, Some(NODE_EXE))),
            RefCell::new(0),
        );
        let outcome = block_on(resolve(&os, 3030, SELF_PID, own()));
        assert!(os.0.terminated.borrow().is_empty());
        assert!(
            matches!(outcome, ReclaimOutcome::Blocked(ref h) if h.identity.pid == 6060),
            "{outcome:?}"
        );
        assert_eq!(
            *os.0.probe_calls.borrow(),
            1,
            "only the lone-holder look is probed"
        );
    }

    #[test]
    fn lsof_fixture_drives_the_same_decisions() {
        // On macOS and Linux the listener list comes from lsof; the decision
        // path is the same once the pids are known.
        let pids: Vec<u32> = parse_lsof_fields(LSOF_UNIX)
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        let own_unix = "/Applications/Freenpipe.app/Contents/MacOS/freenpipe";
        let holders: Vec<PortHolder> = [
            ident(pids[0], Some(own_unix)),
            ident(pids[1], Some("/usr/local/bin/node")),
        ]
        .iter()
        .map(|id| {
            classify(
                id,
                SELF_PID,
                Some(Path::new(own_unix)),
                Some(HealthProbe::NoAnswer),
            )
        })
        .collect();
        assert_eq!(
            holders.iter().map(|h| h.class).collect::<Vec<_>>(),
            vec![HolderClass::Unhealthy, HolderClass::Foreign]
        );
        assert!(matches!(plan(&holders), ConflictPlan::Blocked(ref h) if h.identity.pid == 6060));
        assert_eq!(plan(&holders[..1]), ConflictPlan::Reclaim(vec![4242]));
    }

    #[test]
    fn notifications_name_the_holder() {
        let foreign = ReclaimOutcome::Blocked(PortHolder {
            identity: ident(6060, Some(NODE_EXE)),
            class: HolderClass::Foreign,
            screenpipe_family: false,
            own_executable: false,
        });
        let (title, body) =
            notification_text(3030, PortPurpose::EngineApi, &foreign, None).unwrap();
        assert!(title.contains("3030"));
        assert!(body.contains("node.exe (pid 6060)"));
        assert!(
            notification_text(3030, PortPurpose::EngineApi, &ReclaimOutcome::Free, None).is_none()
        );

        let twin = ReclaimOutcome::Blocked(PortHolder {
            identity: ident(4242, Some(OWN_EXE)),
            class: HolderClass::Healthy,
            screenpipe_family: true,
            own_executable: true,
        });
        let (_, focused) =
            notification_text(3030, PortPurpose::EngineApi, &twin, Some(true)).unwrap();
        let (_, unfocused) =
            notification_text(3030, PortPurpose::EngineApi, &twin, Some(false)).unwrap();
        assert_ne!(focused, unfocused);
        assert!(describe_block(3030, &foreign).contains("node.exe"));
    }

    #[test]
    fn another_screenpipe_app_gets_its_own_wording() {
        let blocked = |id: ProcessIdentity, class, family| {
            ReclaimOutcome::Blocked(PortHolder {
                identity: id,
                class,
                screenpipe_family: family,
                own_executable: false,
            })
        };
        let text =
            |o: &ReclaimOutcome| notification_text(3030, PortPurpose::EngineApi, o, None).unwrap();

        // Unresponsive vendor screenpipe vs an unrelated program: both are
        // left alone, but the user is told which kind of app it is.
        let vendor_silent = text(&blocked(
            ident(4243, Some(VENDOR_EXE)),
            HolderClass::Foreign,
            true,
        ));
        let node = text(&blocked(
            ident(6060, Some(NODE_EXE)),
            HolderClass::Foreign,
            false,
        ));
        assert_ne!(vendor_silent.0, node.0);
        assert!(vendor_silent.0.contains("screenpipe"));
        assert!(vendor_silent.1.contains("screenpipe-app.exe (pid 4243)"));
        assert!(!node.0.contains("screenpipe"));

        // Something that answers like screenpipe but is not named like one.
        let family = text(&blocked(
            ident(4243, Some(VENDOR_EXE)),
            HolderClass::Healthy,
            true,
        ));
        let lookalike = text(&blocked(
            ident(7000, Some(r"C:\tools\proxy.exe")),
            HolderClass::Healthy,
            false,
        ));
        assert_eq!(family.0, "Another screenpipe is running");
        assert_ne!(family.0, lookalike.0);
        assert!(lookalike.1.contains("proxy.exe (pid 7000)"));

        // An unverified copy of ours.
        let shared = ReclaimOutcome::Blocked(PortHolder {
            identity: ident(4242, Some(OWN_EXE)),
            class: HolderClass::Unverified,
            screenpipe_family: true,
            own_executable: true,
        });
        let (title, body) = text(&shared);
        assert!(title.contains("3030"));
        assert!(body.contains("freenpipe.exe (pid 4242)"));
        assert!(describe_block(3030, &shared).contains("several processes"));
    }

    #[test]
    fn control_port_conflicts_never_go_through_the_control_port() {
        assert_eq!(
            notice_route(PortPurpose::ControlServer, false),
            NoticeRoute::System
        );
        assert_eq!(
            notice_route(PortPurpose::ControlServer, true),
            NoticeRoute::System
        );
        assert_eq!(
            notice_route(PortPurpose::EngineApi, true),
            NoticeRoute::Panel
        );
        // Our control server is not up (someone else may hold its port).
        assert_eq!(
            notice_route(PortPurpose::EngineApi, false),
            NoticeRoute::System
        );
    }

    #[test]
    fn control_server_bound_flag_follows_the_guard() {
        let guard = ControlServerBound::mark();
        assert!(CONTROL_SERVER_BOUND.load(std::sync::atomic::Ordering::SeqCst));
        drop(guard);
        assert!(!CONTROL_SERVER_BOUND.load(std::sync::atomic::Ordering::SeqCst));
    }
}
