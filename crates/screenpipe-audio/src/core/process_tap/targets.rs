// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Capture targets and the decisions around them, kept free of OS calls so
//! they are unit-tested on every platform.
//!
//! The Windows loopback engine (`core/wasapi_loopback.rs`) captures one of
//! these targets at a time:
//!
//! * `default-render-endpoint`: classic loopback of the default output
//!   device; a stream on it is rebuilt when the default changes.
//! * `endpoint:<name>`: classic loopback of one named output device (a
//!   device the user pinned that is not the default).
//! * `process-tree:<pid>`: only the audio of that process tree (per-process
//!   loopback, Windows build 20348 or later).
//! * `exclude-process-tree:<pid>`: everything except that process tree. This
//!   is how `audio-exclusions.json` applies on Windows.
//!
//! The rules here decide which target a stream should be on right now, when
//! to rebuild, when to give up on a per-process target, and how a set of
//! process ids maps to one process-tree root.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::time::{Duration, Instant};

/// First Windows build whose audio stack supports per-process loopback
/// (`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`).
pub const MIN_PROCESS_LOOPBACK_BUILD: u32 = 20348;

/// Executable of the Teams desktop app. Its audio is rendered by helper
/// processes, so its ancestry decides the tree root (see
/// [`resolve_include_root`]).
pub const TEAMS_DESKTOP_EXE: &str = "ms-teams.exe";

/// Label prefixes used in logs and errors for each target kind.
pub const DEFAULT_RENDER_ENDPOINT_LABEL: &str = "default-render-endpoint";
pub const PROCESS_TREE_PREFIX: &str = "process-tree:";
pub const EXCLUDE_PROCESS_TREE_PREFIX: &str = "exclude-process-tree:";
pub const ENDPOINT_PREFIX: &str = "endpoint:";

/// One loopback capture target.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LoopbackTarget {
    DefaultRenderEndpoint,
    Endpoint { name: String },
    ProcessTree { pid: u32 },
    ExcludeProcessTree { pid: u32 },
}

impl LoopbackTarget {
    /// Per-process targets need [`MIN_PROCESS_LOOPBACK_BUILD`] and go through
    /// the virtual process-loopback device instead of an endpoint.
    pub fn is_per_process(&self) -> bool {
        matches!(
            self,
            LoopbackTarget::ProcessTree { .. } | LoopbackTarget::ExcludeProcessTree { .. }
        )
    }

    /// The process this target watches, if any.
    pub fn watched_pid(&self) -> Option<u32> {
        match self {
            LoopbackTarget::ProcessTree { pid } | LoopbackTarget::ExcludeProcessTree { pid } => {
                Some(*pid)
            }
            _ => None,
        }
    }

    /// Whether a stream that is not bound to one device (the `default`
    /// request, a meeting tap) rebuilds this target when the default render
    /// endpoint changes. Named endpoints stay where they are; per-process
    /// targets do not depend on an endpoint, but rebuilding them is cheap and
    /// keeps the client on the device the user now hears.
    pub fn follows_default_endpoint(&self) -> bool {
        !matches!(self, LoopbackTarget::Endpoint { .. })
    }
}

impl fmt::Display for LoopbackTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoopbackTarget::DefaultRenderEndpoint => f.write_str(DEFAULT_RENDER_ENDPOINT_LABEL),
            LoopbackTarget::Endpoint { name } => write!(f, "{ENDPOINT_PREFIX}{name}"),
            LoopbackTarget::ProcessTree { pid } => write!(f, "{PROCESS_TREE_PREFIX}{pid}"),
            LoopbackTarget::ExcludeProcessTree { pid } => {
                write!(f, "{EXCLUDE_PROCESS_TREE_PREFIX}{pid}")
            }
        }
    }
}

impl std::str::FromStr for LoopbackTarget {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s == DEFAULT_RENDER_ENDPOINT_LABEL {
            return Ok(LoopbackTarget::DefaultRenderEndpoint);
        }
        // The exclude prefix contains the include prefix, so test it first.
        if let Some(rest) = s.strip_prefix(EXCLUDE_PROCESS_TREE_PREFIX) {
            return parse_pid(rest).map(|pid| LoopbackTarget::ExcludeProcessTree { pid });
        }
        if let Some(rest) = s.strip_prefix(PROCESS_TREE_PREFIX) {
            return parse_pid(rest).map(|pid| LoopbackTarget::ProcessTree { pid });
        }
        if let Some(rest) = s.strip_prefix(ENDPOINT_PREFIX) {
            if rest.trim().is_empty() {
                return Err("endpoint target needs a device name".to_string());
            }
            return Ok(LoopbackTarget::Endpoint {
                name: rest.to_string(),
            });
        }
        Err(format!("unknown loopback target '{s}'"))
    }
}

fn parse_pid(raw: &str) -> Result<u32, String> {
    match raw.trim().parse::<u32>() {
        Ok(0) => Err("loopback target pid must not be 0".to_string()),
        Ok(pid) => Ok(pid),
        Err(_) => Err(format!("invalid loopback target pid '{raw}'")),
    }
}

/// What a stream is for. Decided when the stream opens; a stream opened for
/// a named output device switches between [`CaptureRole::SystemDefault`] and
/// [`CaptureRole::NamedEndpoint`] as its device gains or loses the default
/// (see [`named_role_change`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureRole {
    /// The system-audio stream: the default output device, with the
    /// exclusion list applied.
    SystemDefault,
    /// An output device that is not the default, captured in full.
    NamedEndpoint(String),
    /// Only the audio of these processes (meeting tap). The pids are reduced
    /// to one tree root when the stream opens.
    ProcessTree(Vec<u32>),
}

/// How a stream opened for a named output device reacts to the current
/// default render endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleChange {
    Keep,
    /// The device became the default: it becomes the system-audio stream and
    /// the exclusion list applies to it.
    Promote,
    /// The device is no longer the default: it is captured as its own
    /// endpoint, without exclusions.
    Demote,
}

/// Role rule for a stream bound to one named device. Such a stream always
/// records its own device, so device names in the database stay true and
/// two pinned outputs never end up recording the same endpoint; only the
/// role moves with the default. `device_is_default` is `None` when there is
/// no default output at all (every device unplugged), which keeps the role.
pub fn named_role_change(is_system_role: bool, device_is_default: Option<bool>) -> RoleChange {
    match (is_system_role, device_is_default) {
        (false, Some(true)) => RoleChange::Promote,
        (true, Some(false)) => RoleChange::Demote,
        _ => RoleChange::Keep,
    }
}

/// Whether per-process loopback is usable on this Windows build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessLoopbackSupport {
    Available { build: u32 },
    TooOld { build: u32 },
    Unknown,
}

impl ProcessLoopbackSupport {
    pub fn from_build(build: Option<u32>) -> Self {
        match build {
            Some(b) if b >= MIN_PROCESS_LOOPBACK_BUILD => Self::Available { build: b },
            Some(b) => Self::TooOld { build: b },
            None => Self::Unknown,
        }
    }

    pub fn is_available(self) -> bool {
        matches!(self, Self::Available { .. })
    }

    /// One-line status for logs and diagnostics.
    pub fn describe(self) -> String {
        match self {
            Self::Available { build } => {
                format!("per-process audio loopback available (Windows build {build})")
            }
            Self::TooOld { build } => format!(
                "per-process audio loopback unavailable: Windows build {build} is older than {MIN_PROCESS_LOOPBACK_BUILD}"
            ),
            Self::Unknown => "per-process audio loopback assumed unavailable: the Windows build could not be determined".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Process trees
// ---------------------------------------------------------------------------

/// One row of a process list snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent_pid: u32,
    /// Executable file name, lower-case (`chrome.exe`).
    pub exe_name: String,
}

/// A process list indexed by pid.
#[derive(Debug, Default, Clone)]
pub struct ProcessSnapshot {
    by_pid: HashMap<u32, ProcessEntry>,
}

impl ProcessSnapshot {
    pub fn new(entries: impl IntoIterator<Item = ProcessEntry>) -> Self {
        let by_pid = entries
            .into_iter()
            .filter(|e| e.pid != 0)
            .map(|mut e| {
                e.exe_name = e.exe_name.to_lowercase();
                (e.pid, e)
            })
            .collect();
        Self { by_pid }
    }

    pub fn get(&self, pid: u32) -> Option<&ProcessEntry> {
        self.by_pid.get(&pid)
    }

    pub fn contains(&self, pid: u32) -> bool {
        self.by_pid.contains_key(&pid)
    }

    /// The live parent of `pid`, or `None` at the top of the tree. A parent
    /// pid that points at itself or at a process that is gone ends the walk.
    fn parent_of(&self, pid: u32) -> Option<&ProcessEntry> {
        let entry = self.by_pid.get(&pid)?;
        if entry.parent_pid == 0 || entry.parent_pid == pid {
            return None;
        }
        self.by_pid.get(&entry.parent_pid)
    }

    /// `pid` followed by its live ancestors, nearest first. Cycles (possible
    /// with recycled pids) stop the walk.
    pub fn ancestry(&self, pid: u32) -> Vec<&ProcessEntry> {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut current = self.by_pid.get(&pid);
        while let Some(entry) = current {
            if !seen.insert(entry.pid) {
                break;
            }
            chain.push(entry);
            current = self.parent_of(entry.pid);
        }
        chain
    }

    /// The top of the run of same-executable ancestors that starts at `pid`
    /// (for a browser: the main browser process above its renderers).
    pub fn same_exe_root(&self, pid: u32) -> Option<u32> {
        let chain = self.ancestry(pid);
        let first = chain.first()?;
        let mut root = first.pid;
        for entry in chain.iter().skip(1) {
            if entry.exe_name != first.exe_name {
                break;
            }
            root = entry.pid;
        }
        Some(root)
    }

    /// Whether `ancestor` is `pid` or one of its ancestors.
    pub fn is_ancestor_or_self(&self, ancestor: u32, pid: u32) -> bool {
        self.ancestry(pid).iter().any(|e| e.pid == ancestor)
    }

    /// Number of live processes below `root`.
    pub fn descendant_count(&self, root: u32) -> usize {
        self.by_pid
            .keys()
            .filter(|&&pid| pid != root && self.is_ancestor_or_self(root, pid))
            .count()
    }

    /// Distinct tree roots of every running process named `exe_name`.
    pub fn roots_of_exe(&self, exe_name: &str) -> Vec<u32> {
        let wanted = exe_name.to_lowercase();
        let mut roots: Vec<u32> = self
            .by_pid
            .values()
            .filter(|e| e.exe_name == wanted)
            .filter_map(|e| self.same_exe_root(e.pid))
            .collect();
        roots.sort_unstable();
        roots.dedup();
        roots
    }
}

/// How a set of requested pids maps onto one process tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncludeRoot {
    /// One tree covers every requested pid.
    Single { root: u32 },
    /// The pids live in several independent trees. WASAPI includes a single
    /// tree, so the caller captures the whole endpoint instead.
    MultipleRoots { roots: Vec<u32> },
    /// None of the pids is running.
    NotRunning,
}

/// The tree root for one requested pid. Normally that is the top of its
/// same-executable run; when the Teams desktop app is in its ancestry, the
/// topmost `ms-teams.exe` ancestor wins so Teams' helper processes resolve
/// to the Teams tree.
pub fn include_root_for(snapshot: &ProcessSnapshot, pid: u32) -> Option<u32> {
    let chain = snapshot.ancestry(pid);
    if chain.is_empty() {
        return None;
    }
    if let Some(teams) = chain.iter().rev().find(|e| e.exe_name == TEAMS_DESKTOP_EXE) {
        return Some(teams.pid);
    }
    snapshot.same_exe_root(pid)
}

/// Reduces requested pids to one process-tree root when possible.
pub fn resolve_include_root(snapshot: &ProcessSnapshot, pids: &[u32]) -> IncludeRoot {
    let mut roots: Vec<u32> = pids
        .iter()
        .filter_map(|&pid| include_root_for(snapshot, pid))
        .collect();
    roots.sort_unstable();
    roots.dedup();
    // A root that sits inside another root's tree is already covered.
    let independent: Vec<u32> = roots
        .iter()
        .copied()
        .filter(|&r| {
            !roots
                .iter()
                .any(|&other| other != r && snapshot.is_ancestor_or_self(other, r))
        })
        .collect();
    match independent.as_slice() {
        [] => IncludeRoot::NotRunning,
        [root] => IncludeRoot::Single { root: *root },
        many => IncludeRoot::MultipleRoots {
            roots: many.to_vec(),
        },
    }
}

/// The process tree to exclude: the first configured executable (file
/// order) that is running. When it runs as several independent trees, the
/// tree with the most processes is taken (the main instance), lowest pid on
/// a tie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExclusionChoice {
    pub exe_name: String,
    pub root_pid: u32,
    /// Other independent trees of the same executable that stay audible.
    pub uncovered_roots: Vec<u32>,
}

pub fn choose_exclusion(
    snapshot: &ProcessSnapshot,
    exe_names: &[String],
) -> Option<ExclusionChoice> {
    for exe in exe_names {
        let roots = snapshot.roots_of_exe(exe);
        if roots.is_empty() {
            continue;
        }
        let best = *roots
            .iter()
            .max_by(|&&a, &&b| {
                snapshot
                    .descendant_count(a)
                    .cmp(&snapshot.descendant_count(b))
                    .then(b.cmp(&a))
            })
            .expect("roots is not empty");
        let uncovered_roots = roots.into_iter().filter(|&r| r != best).collect();
        return Some(ExclusionChoice {
            exe_name: exe.to_lowercase(),
            root_pid: best,
            uncovered_roots,
        });
    }
    None
}

// ---------------------------------------------------------------------------
// Target planning for the system-audio stream
// ---------------------------------------------------------------------------

/// The target the system-audio stream should be on.
///
/// Per-process exclusion is used only when the build supports it and the
/// chosen root is not in `untappable` (roots waiting out a retry delay after
/// a failed activation, see [`UntappableRoots::blocked`]); every other case
/// is the plain default-endpoint loopback, so capture never goes silent
/// because of an exclusion.
pub fn desired_system_target(
    support: ProcessLoopbackSupport,
    exclusion: Option<&ExclusionChoice>,
    untappable: &HashSet<u32>,
) -> LoopbackTarget {
    match exclusion {
        Some(choice) if support.is_available() && !untappable.contains(&choice.root_pid) => {
            LoopbackTarget::ExcludeProcessTree {
                pid: choice.root_pid,
            }
        }
        _ => LoopbackTarget::DefaultRenderEndpoint,
    }
}

/// The target a per-process include request starts on.
pub fn initial_include_target(
    support: ProcessLoopbackSupport,
    resolution: &IncludeRoot,
) -> LoopbackTarget {
    match resolution {
        IncludeRoot::Single { root } if support.is_available() => {
            LoopbackTarget::ProcessTree { pid: *root }
        }
        _ => LoopbackTarget::DefaultRenderEndpoint,
    }
}

/// Debounces target changes: a new desired target must be seen on two
/// consecutive polls, and exclusion-driven rebuilds are at least
/// `min_interval` apart, so a helper process that lives for a moment does
/// not rebuild the stream twice.
#[derive(Debug)]
pub struct TargetDebounce {
    min_interval: Duration,
    candidate: Option<LoopbackTarget>,
    last_switch: Option<Instant>,
}

impl TargetDebounce {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            candidate: None,
            last_switch: None,
        }
    }

    /// Feed the current and desired targets; returns the target to switch to
    /// when a rebuild is due.
    pub fn observe(
        &mut self,
        current: &LoopbackTarget,
        desired: LoopbackTarget,
        now: Instant,
    ) -> Option<LoopbackTarget> {
        if &desired == current {
            self.candidate = None;
            return None;
        }
        if self.candidate.as_ref() != Some(&desired) {
            self.candidate = Some(desired);
            return None;
        }
        if let Some(last) = self.last_switch {
            if now.duration_since(last) < self.min_interval {
                return None;
            }
        }
        self.candidate = None;
        self.last_switch = Some(now);
        Some(desired)
    }

    /// Records a switch made for another reason (default-device change,
    /// device loss) so the interval also covers it.
    pub fn note_switch(&mut self, now: Instant) {
        self.last_switch = Some(now);
        self.candidate = None;
    }
}

/// Tracks a change of the default render endpoint for a stream that is not
/// bound to one device (the `default` request, a meeting tap that fell back
/// to the whole output). The stream waits a short grace before moving: when
/// recording follows the system default, the device monitor replaces such
/// streams within that window, and moving early would record the same audio
/// twice. A stream still alive after the grace moves to the new default so
/// it keeps recording what the user hears. Streams opened for a named device
/// never move; they follow [`named_role_change`] instead.
#[derive(Debug)]
pub struct DefaultEndpointFollow {
    grace: Duration,
    current_id: Option<String>,
    pending: Option<(String, Instant)>,
}

impl DefaultEndpointFollow {
    pub fn new(grace: Duration, current_id: Option<String>) -> Self {
        Self {
            grace,
            current_id,
            pending: None,
        }
    }

    pub fn current_id(&self) -> Option<&str> {
        self.current_id.as_deref()
    }

    /// Whether a change has been seen and is waiting out the grace.
    pub fn pending_id(&self) -> Option<&str> {
        self.pending.as_ref().map(|(id, _)| id.as_str())
    }

    /// Feed the latest default endpoint id (`None` when there is no default
    /// right now). Returns the new id once a change has outlasted the grace.
    pub fn observe(&mut self, latest: Option<String>, now: Instant) -> Option<String> {
        let Some(latest) = latest else {
            // No default output at all (every device unplugged): keep the
            // current client until a device comes back.
            return None;
        };
        if self.current_id.as_deref() == Some(latest.as_str()) {
            self.pending = None;
            return None;
        }
        match &self.pending {
            Some((id, since)) if *id == latest => {
                if now.duration_since(*since) >= self.grace {
                    self.pending = None;
                    Some(latest)
                } else {
                    None
                }
            }
            _ => {
                self.pending = Some((latest, now));
                if self.grace.is_zero() {
                    let id = self.pending.take().map(|(id, _)| id);
                    return id;
                }
                None
            }
        }
    }

    /// Records the endpoint the stream is on after a (re)build.
    pub fn set_current(&mut self, id: Option<String>) {
        self.current_id = id;
        self.pending = None;
    }
}

/// Delay before a root whose tap failed is tried again.
pub const UNTAPPABLE_RETRY_BASE: Duration = Duration::from_secs(60);
/// Longest delay between tries while a root keeps failing.
pub const UNTAPPABLE_RETRY_MAX: Duration = Duration::from_secs(15 * 60);

/// What [`UntappableRoots::mark`] recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UntappableMark {
    /// First failure of this root while it runs: the caller warns once.
    /// Later failures are only worth a debug line.
    pub first: bool,
    /// How long the root is skipped before the next try.
    pub retry_in: Duration,
}

#[derive(Debug, Clone, Copy)]
struct RetryBackoff {
    failures: u32,
    retry_at: Instant,
}

/// Per-process roots whose tap failed.
///
/// Most failures are passing conditions (the audio service is slow after
/// wake, an endpoint is being swapped), and an exclusion that is off records
/// audio the user asked to leave out, so a failed root is only skipped until
/// its retry time: [`UNTAPPABLE_RETRY_BASE`] after the first failure,
/// doubling up to [`UNTAPPABLE_RETRY_MAX`] while it keeps failing. A tap on
/// the root that works resets the delay. Each root is reported once while it
/// runs; an exited root is forgotten, so a relaunched app starts afresh.
#[derive(Debug)]
pub struct UntappableRoots {
    base: Duration,
    max: Duration,
    failing: HashMap<u32, RetryBackoff>,
    reported: HashSet<u32>,
}

impl Default for UntappableRoots {
    fn default() -> Self {
        Self::new(UNTAPPABLE_RETRY_BASE, UNTAPPABLE_RETRY_MAX)
    }
}

impl UntappableRoots {
    pub fn new(base: Duration, max: Duration) -> Self {
        Self {
            base,
            max: max.max(base),
            failing: HashMap::new(),
            reported: HashSet::new(),
        }
    }

    /// Records a failed tap on `pid` at `now` and schedules the next try.
    pub fn mark(&mut self, pid: u32, now: Instant) -> UntappableMark {
        let entry = self.failing.entry(pid).or_insert(RetryBackoff {
            failures: 0,
            retry_at: now,
        });
        entry.failures = entry.failures.saturating_add(1);
        let retry_in = retry_delay(self.base, self.max, entry.failures);
        entry.retry_at = now + retry_in;
        UntappableMark {
            first: self.reported.insert(pid),
            retry_in,
        }
    }

    /// Whether `pid` is still waiting out its retry delay at `now`.
    pub fn is_blocked(&self, pid: u32, now: Instant) -> bool {
        self.failing
            .get(&pid)
            .is_some_and(|backoff| now < backoff.retry_at)
    }

    /// The roots that must not be tried at `now`.
    pub fn blocked(&self, now: Instant) -> HashSet<u32> {
        self.failing
            .iter()
            .filter(|(_, backoff)| now < backoff.retry_at)
            .map(|(pid, _)| *pid)
            .collect()
    }

    /// A tap on `pid` works: a later failure starts from the base delay and is
    /// reported again, since it is a new outage.
    pub fn note_tapped(&mut self, pid: u32) {
        self.failing.remove(&pid);
        self.reported.remove(&pid);
    }

    /// Forgets roots that are no longer running, so a relaunched app (new
    /// pid) gets a fresh attempt.
    pub fn retain_running(&mut self, snapshot: &ProcessSnapshot) {
        self.failing.retain(|pid, _| snapshot.contains(*pid));
        self.reported.retain(|pid| snapshot.contains(*pid));
    }
}

/// `base` after the first failure, doubled per further failure, capped.
fn retry_delay(base: Duration, max: Duration, failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    base.saturating_mul(1u32 << doublings).min(max)
}

/// Counts engine failures per device in a sliding window. While a device
/// has `max_failures` inside the window the caller skips the engine for it
/// and uses the cpal loopback; once enough of them are older than the window
/// the engine is allowed again.
#[derive(Debug)]
pub struct FailureWindow {
    window: Duration,
    max_failures: usize,
    failures: HashMap<String, Vec<Instant>>,
}

impl FailureWindow {
    pub fn new(window: Duration, max_failures: usize) -> Self {
        Self {
            window,
            max_failures,
            failures: HashMap::new(),
        }
    }

    pub fn record(&mut self, device: &str, now: Instant) {
        let entry = self.failures.entry(device.to_string()).or_default();
        entry.push(now);
        let window = self.window;
        entry.retain(|t| now.duration_since(*t) <= window);
    }

    pub fn is_exhausted(&self, device: &str, now: Instant) -> bool {
        self.failures
            .get(device)
            .map(|times| {
                times
                    .iter()
                    .filter(|t| now.duration_since(**t) <= self.window)
                    .count()
                    >= self.max_failures
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent_pid: u32, exe: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent_pid,
            exe_name: exe.to_string(),
        }
    }

    /// explorer(10) -> chrome(100) -> chrome(101, 102, 103)
    /// explorer(10) -> spotify(200) -> spotify(201)
    /// svchost(4) -> ms-teams(300) -> msedgewebview2(301) -> msedgewebview2(302)
    fn sample() -> ProcessSnapshot {
        ProcessSnapshot::new(vec![
            p(4, 0, "System"),
            p(10, 4, "explorer.exe"),
            p(100, 10, "Chrome.exe"),
            p(101, 100, "chrome.exe"),
            p(102, 100, "chrome.exe"),
            p(103, 101, "chrome.exe"),
            p(200, 10, "Spotify.exe"),
            p(201, 200, "spotify.exe"),
            p(300, 4, "ms-teams.exe"),
            p(301, 300, "msedgewebview2.exe"),
            p(302, 301, "msedgewebview2.exe"),
        ])
    }

    #[test]
    fn labels_round_trip() {
        for t in [
            LoopbackTarget::DefaultRenderEndpoint,
            LoopbackTarget::Endpoint {
                name: "Speakers (Realtek(R) Audio)".to_string(),
            },
            LoopbackTarget::ProcessTree { pid: 42 },
            LoopbackTarget::ExcludeProcessTree { pid: 7 },
        ] {
            let parsed: LoopbackTarget = t.to_string().parse().unwrap();
            assert_eq!(parsed, t);
        }
        assert_eq!(
            LoopbackTarget::ExcludeProcessTree { pid: 9 }.to_string(),
            "exclude-process-tree:9"
        );
        assert_eq!(
            LoopbackTarget::ProcessTree { pid: 9 }.to_string(),
            "process-tree:9"
        );
        assert_eq!(
            LoopbackTarget::DefaultRenderEndpoint.to_string(),
            "default-render-endpoint"
        );
    }

    #[test]
    fn bad_labels_are_rejected() {
        assert!("process-tree:".parse::<LoopbackTarget>().is_err());
        assert!("process-tree:0".parse::<LoopbackTarget>().is_err());
        assert!("exclude-process-tree:abc"
            .parse::<LoopbackTarget>()
            .is_err());
        assert!("endpoint: ".parse::<LoopbackTarget>().is_err());
        assert!("loopback".parse::<LoopbackTarget>().is_err());
    }

    #[test]
    fn build_gate() {
        assert!(ProcessLoopbackSupport::from_build(Some(20348)).is_available());
        assert!(ProcessLoopbackSupport::from_build(Some(26100)).is_available());
        assert!(!ProcessLoopbackSupport::from_build(Some(19045)).is_available());
        assert!(!ProcessLoopbackSupport::from_build(None).is_available());
        assert!(ProcessLoopbackSupport::from_build(Some(19045))
            .describe()
            .contains("19045"));
        assert!(ProcessLoopbackSupport::from_build(Some(22631))
            .describe()
            .contains("22631"));
    }

    #[test]
    fn target_properties() {
        assert!(LoopbackTarget::ProcessTree { pid: 1 }.is_per_process());
        assert!(LoopbackTarget::ExcludeProcessTree { pid: 1 }.is_per_process());
        assert!(!LoopbackTarget::DefaultRenderEndpoint.is_per_process());
        assert_eq!(
            LoopbackTarget::ExcludeProcessTree { pid: 5 }.watched_pid(),
            Some(5)
        );
        assert_eq!(LoopbackTarget::DefaultRenderEndpoint.watched_pid(), None);
        assert!(!LoopbackTarget::Endpoint {
            name: "x".to_string()
        }
        .follows_default_endpoint());
        assert!(LoopbackTarget::DefaultRenderEndpoint.follows_default_endpoint());
    }

    #[test]
    fn same_exe_root_climbs_to_main_browser() {
        let s = sample();
        assert_eq!(s.same_exe_root(103), Some(100));
        assert_eq!(s.same_exe_root(100), Some(100));
        assert_eq!(s.same_exe_root(201), Some(200));
        assert_eq!(s.same_exe_root(9999), None);
        assert_eq!(s.descendant_count(100), 3);
    }

    #[test]
    fn ancestry_survives_cycles_and_dead_parents() {
        let s = ProcessSnapshot::new(vec![
            p(1, 2, "a.exe"),
            p(2, 1, "a.exe"),
            p(3, 77, "b.exe"),
            p(5, 5, "c.exe"),
        ]);
        assert_eq!(s.ancestry(1).len(), 2);
        assert_eq!(s.same_exe_root(1), Some(2));
        assert_eq!(s.ancestry(3).len(), 1);
        assert_eq!(s.same_exe_root(5), Some(5));
    }

    #[test]
    fn roots_of_exe_is_case_insensitive() {
        let s = sample();
        assert_eq!(s.roots_of_exe("CHROME.EXE"), vec![100]);
        assert_eq!(s.roots_of_exe("spotify.exe"), vec![200]);
        assert!(s.roots_of_exe("zoom.exe").is_empty());
    }

    #[test]
    fn exclusion_takes_first_running_app_in_file_order() {
        let s = sample();
        let names = vec![
            "zoom.exe".to_string(),
            "spotify.exe".to_string(),
            "chrome.exe".to_string(),
        ];
        let choice = choose_exclusion(&s, &names).unwrap();
        assert_eq!(choice.exe_name, "spotify.exe");
        assert_eq!(choice.root_pid, 200);
        assert!(choice.uncovered_roots.is_empty());
        assert!(choose_exclusion(&s, &["zoom.exe".to_string()]).is_none());
        assert!(choose_exclusion(&s, &[]).is_none());
    }

    #[test]
    fn exclusion_with_two_trees_prefers_the_larger() {
        let s = ProcessSnapshot::new(vec![
            p(10, 4, "explorer.exe"),
            p(50, 10, "chrome.exe"),
            p(60, 10, "chrome.exe"),
            p(61, 60, "chrome.exe"),
            p(62, 60, "chrome.exe"),
        ]);
        let choice = choose_exclusion(&s, &["chrome.exe".to_string()]).unwrap();
        assert_eq!(choice.root_pid, 60);
        assert_eq!(choice.uncovered_roots, vec![50]);

        // Equal trees: lowest pid wins.
        let s = ProcessSnapshot::new(vec![p(70, 10, "x.exe"), p(80, 10, "x.exe")]);
        assert_eq!(
            choose_exclusion(&s, &["x.exe".to_string()])
                .unwrap()
                .root_pid,
            70
        );
    }

    #[test]
    fn include_root_single_tree() {
        let s = sample();
        assert_eq!(
            resolve_include_root(&s, &[101, 103]),
            IncludeRoot::Single { root: 100 }
        );
        assert_eq!(
            resolve_include_root(&s, &[100, 102]),
            IncludeRoot::Single { root: 100 }
        );
    }

    #[test]
    fn include_root_multiple_independent_trees() {
        let s = sample();
        assert_eq!(
            resolve_include_root(&s, &[103, 201]),
            IncludeRoot::MultipleRoots {
                roots: vec![100, 200]
            }
        );
    }

    #[test]
    fn include_root_not_running() {
        let s = sample();
        assert_eq!(resolve_include_root(&s, &[5555]), IncludeRoot::NotRunning);
        assert_eq!(resolve_include_root(&s, &[]), IncludeRoot::NotRunning);
    }

    #[test]
    fn teams_helpers_resolve_to_the_teams_root() {
        let s = sample();
        // A webview helper alone would resolve to its own run (301); the
        // Teams ancestry rule lifts it to ms-teams.exe (300).
        assert_eq!(include_root_for(&s, 302), Some(300));
        assert_eq!(
            resolve_include_root(&s, &[300, 302]),
            IncludeRoot::Single { root: 300 }
        );
        // Without Teams in the chain the same-exe rule applies.
        let plain = ProcessSnapshot::new(vec![
            p(4, 0, "svchost.exe"),
            p(301, 4, "msedgewebview2.exe"),
            p(302, 301, "msedgewebview2.exe"),
        ]);
        assert_eq!(include_root_for(&plain, 302), Some(301));
    }

    #[test]
    fn nested_roots_collapse_to_the_outer_tree() {
        // A helper with its own executable runs inside the browser tree. Its
        // same-exe root (150) sits below the browser root (100), so the
        // browser tree already covers it.
        let s = ProcessSnapshot::new(vec![
            p(10, 4, "explorer.exe"),
            p(100, 10, "chrome.exe"),
            p(101, 100, "chrome.exe"),
            p(150, 101, "helper.exe"),
            p(151, 150, "helper.exe"),
        ]);
        assert_eq!(include_root_for(&s, 151), Some(150));
        assert_eq!(
            resolve_include_root(&s, &[151, 101]),
            IncludeRoot::Single { root: 100 }
        );
    }

    #[test]
    fn system_target_falls_back_when_untappable_or_unsupported() {
        let choice = ExclusionChoice {
            exe_name: "chrome.exe".to_string(),
            root_pid: 100,
            uncovered_roots: vec![],
        };
        let none = HashSet::new();
        let ok = ProcessLoopbackSupport::Available { build: 22631 };
        assert_eq!(
            desired_system_target(ok, Some(&choice), &none),
            LoopbackTarget::ExcludeProcessTree { pid: 100 }
        );
        assert_eq!(
            desired_system_target(ok, None, &none),
            LoopbackTarget::DefaultRenderEndpoint
        );
        let old = ProcessLoopbackSupport::TooOld { build: 19045 };
        assert_eq!(
            desired_system_target(old, Some(&choice), &none),
            LoopbackTarget::DefaultRenderEndpoint
        );
        assert_eq!(
            desired_system_target(ProcessLoopbackSupport::Unknown, Some(&choice), &none),
            LoopbackTarget::DefaultRenderEndpoint
        );
        let bad: HashSet<u32> = [100].into_iter().collect();
        assert_eq!(
            desired_system_target(ok, Some(&choice), &bad),
            LoopbackTarget::DefaultRenderEndpoint
        );
    }

    #[test]
    fn include_target_falls_back_to_whole_endpoint() {
        let ok = ProcessLoopbackSupport::Available { build: 22000 + 348 };
        assert_eq!(
            initial_include_target(ok, &IncludeRoot::Single { root: 9 }),
            LoopbackTarget::ProcessTree { pid: 9 }
        );
        assert_eq!(
            initial_include_target(ok, &IncludeRoot::MultipleRoots { roots: vec![1, 2] }),
            LoopbackTarget::DefaultRenderEndpoint
        );
        assert_eq!(
            initial_include_target(ok, &IncludeRoot::NotRunning),
            LoopbackTarget::DefaultRenderEndpoint
        );
        assert_eq!(
            initial_include_target(
                ProcessLoopbackSupport::TooOld { build: 19045 },
                &IncludeRoot::Single { root: 9 }
            ),
            LoopbackTarget::DefaultRenderEndpoint
        );
    }

    #[test]
    fn debounce_needs_two_polls_and_an_interval() {
        let start = Instant::now();
        let mut d = TargetDebounce::new(Duration::from_secs(5));
        let cur = LoopbackTarget::DefaultRenderEndpoint;
        let want = LoopbackTarget::ExcludeProcessTree { pid: 100 };
        assert_eq!(d.observe(&cur, want.clone(), start), None);
        assert_eq!(
            d.observe(&cur, want.clone(), start + Duration::from_secs(2)),
            Some(want.clone())
        );
        // Back to default right away: seen twice but inside the interval.
        let cur = want;
        let back = LoopbackTarget::DefaultRenderEndpoint;
        assert_eq!(
            d.observe(&cur, back.clone(), start + Duration::from_secs(3)),
            None
        );
        assert_eq!(
            d.observe(&cur, back.clone(), start + Duration::from_secs(4)),
            None
        );
        assert_eq!(
            d.observe(&cur, back.clone(), start + Duration::from_secs(8)),
            Some(back)
        );
    }

    #[test]
    fn debounce_resets_when_desired_matches_current() {
        let t = Instant::now();
        let mut d = TargetDebounce::new(Duration::ZERO);
        let cur = LoopbackTarget::DefaultRenderEndpoint;
        let want = LoopbackTarget::ExcludeProcessTree { pid: 1 };
        assert_eq!(d.observe(&cur, want.clone(), t), None);
        assert_eq!(d.observe(&cur, cur.clone(), t), None);
        // The flap reset the candidate, so two more polls are needed.
        assert_eq!(d.observe(&cur, want.clone(), t), None);
        assert_eq!(d.observe(&cur, want.clone(), t), Some(want));
    }

    #[test]
    fn default_follow_waits_out_the_grace() {
        let t = Instant::now();
        let mut f = DefaultEndpointFollow::new(Duration::from_secs(4), Some("A".to_string()));
        assert_eq!(f.observe(Some("A".to_string()), t), None);
        assert_eq!(f.observe(Some("B".to_string()), t), None);
        assert_eq!(f.pending_id(), Some("B"));
        assert_eq!(
            f.observe(Some("B".to_string()), t + Duration::from_secs(2)),
            None
        );
        assert_eq!(
            f.observe(Some("B".to_string()), t + Duration::from_secs(4)),
            Some("B".to_string())
        );
        f.set_current(Some("B".to_string()));
        assert_eq!(f.current_id(), Some("B"));
        assert_eq!(
            f.observe(Some("B".to_string()), t + Duration::from_secs(5)),
            None
        );
    }

    #[test]
    fn default_follow_ignores_flaps_and_missing_default() {
        let t = Instant::now();
        let mut f = DefaultEndpointFollow::new(Duration::from_secs(4), Some("A".to_string()));
        assert_eq!(f.observe(Some("B".to_string()), t), None);
        // Back to A before the grace ended: nothing to do.
        assert_eq!(
            f.observe(Some("A".to_string()), t + Duration::from_secs(1)),
            None
        );
        assert_eq!(f.pending_id(), None);
        // No default at all keeps the current client.
        assert_eq!(f.observe(None, t + Duration::from_secs(10)), None);
        // A third device restarts the grace.
        assert_eq!(
            f.observe(Some("C".to_string()), t + Duration::from_secs(11)),
            None
        );
        assert_eq!(
            f.observe(Some("D".to_string()), t + Duration::from_secs(16)),
            None
        );
        assert_eq!(
            f.observe(Some("D".to_string()), t + Duration::from_secs(20)),
            Some("D".to_string())
        );
    }

    #[test]
    fn named_streams_swap_roles_with_the_default() {
        // Pinned speakers (system role) lose the default to a headset.
        assert_eq!(named_role_change(true, Some(false)), RoleChange::Demote);
        // The pinned headset stream picks the role up.
        assert_eq!(named_role_change(false, Some(true)), RoleChange::Promote);
        // Nothing to do while each stream already has the right role.
        assert_eq!(named_role_change(true, Some(true)), RoleChange::Keep);
        assert_eq!(named_role_change(false, Some(false)), RoleChange::Keep);
        // No default output at all keeps whatever role the stream had.
        assert_eq!(named_role_change(true, None), RoleChange::Keep);
        assert_eq!(named_role_change(false, None), RoleChange::Keep);
    }

    #[test]
    fn default_follow_without_grace_moves_at_once() {
        let mut f = DefaultEndpointFollow::new(Duration::ZERO, None);
        assert_eq!(
            f.observe(Some("A".to_string()), Instant::now()),
            Some("A".to_string())
        );
    }

    #[test]
    fn untappable_roots_retry_with_backoff() {
        let t = Instant::now();
        let secs = Duration::from_secs;
        let mut u = UntappableRoots::default();
        assert!(!u.is_blocked(100, t));

        // First failure: reported, skipped for a minute.
        let first = u.mark(100, t);
        assert_eq!(
            first,
            UntappableMark {
                first: true,
                retry_in: secs(60)
            }
        );
        assert!(u.is_blocked(100, t + secs(59)));
        assert!(u.blocked(t + secs(59)).contains(&100));
        // The mark expires: the exclusion is tried again.
        assert!(!u.is_blocked(100, t + secs(60)));
        assert!(u.blocked(t + secs(60)).is_empty());

        // Each further failure doubles the delay, quietly, up to 15 min.
        let mut at = t + secs(60);
        for expected in [120, 240, 480, 900, 900] {
            let mark = u.mark(100, at);
            assert!(!mark.first);
            assert_eq!(mark.retry_in, secs(expected));
            assert!(u.is_blocked(100, at + secs(expected) - secs(1)));
            at += secs(expected);
            assert!(!u.is_blocked(100, at));
        }

        // A tap that works resets the delay, and the next outage is reported again.
        u.note_tapped(100);
        assert!(!u.is_blocked(100, at));
        let again = u.mark(100, at);
        assert!(again.first);
        assert_eq!(again.retry_in, secs(60));
        assert!(!u.mark(100, at + secs(60)).first);

        // Other roots are independent.
        assert!(!u.is_blocked(200, at));
        assert!(u.mark(200, at).first);
    }

    #[test]
    fn untappable_roots_forget_exited_processes() {
        let t = Instant::now();
        let mut u = UntappableRoots::new(Duration::from_secs(10), Duration::from_secs(40));
        assert!(u.mark(100, t).first);
        assert!(u.mark(200, t).first);
        u.retain_running(&ProcessSnapshot::new(vec![p(200, 1, "x.exe")]));
        // 100 exited: not blocked, and a new failure is reported again.
        assert!(!u.is_blocked(100, t));
        assert!(u.is_blocked(200, t));
        let fresh = u.mark(100, t);
        assert!(fresh.first);
        assert_eq!(fresh.retry_in, Duration::from_secs(10));
        // 200 kept its count: second failure, doubled delay.
        assert_eq!(u.mark(200, t).retry_in, Duration::from_secs(20));
        assert_eq!(u.mark(200, t).retry_in, Duration::from_secs(40));
        assert_eq!(u.mark(200, t).retry_in, Duration::from_secs(40));
    }

    #[test]
    fn untappable_roots_feed_the_system_target() {
        let t = Instant::now();
        let choice = ExclusionChoice {
            exe_name: "chrome.exe".to_string(),
            root_pid: 100,
            uncovered_roots: vec![],
        };
        let ok = ProcessLoopbackSupport::Available { build: 22631 };
        let mut u = UntappableRoots::default();
        u.mark(100, t);
        assert_eq!(
            desired_system_target(ok, Some(&choice), &u.blocked(t + Duration::from_secs(30))),
            LoopbackTarget::DefaultRenderEndpoint
        );
        assert_eq!(
            desired_system_target(ok, Some(&choice), &u.blocked(t + Duration::from_secs(60))),
            LoopbackTarget::ExcludeProcessTree { pid: 100 }
        );
    }

    #[test]
    fn failure_window_exhausts_and_recovers() {
        let t = Instant::now();
        let mut w = FailureWindow::new(Duration::from_secs(600), 3);
        w.record("Speakers", t);
        w.record("Speakers", t + Duration::from_secs(10));
        assert!(!w.is_exhausted("Speakers", t + Duration::from_secs(11)));
        w.record("Speakers", t + Duration::from_secs(20));
        assert!(w.is_exhausted("Speakers", t + Duration::from_secs(21)));
        assert!(!w.is_exhausted("Headset", t + Duration::from_secs(21)));
        assert!(!w.is_exhausted("Speakers", t + Duration::from_secs(700)));
    }
}
