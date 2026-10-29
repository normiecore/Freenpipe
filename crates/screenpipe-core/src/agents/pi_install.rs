// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Installing the pinned Pi release into `<data dir>/pi-agent`.
//!
//! The desktop app (chat) and the engine (pipes) both install and repair the
//! managed Pi runtime; this module is the one procedure they share:
//!
//! 1. on a machine that needs it, make sure bun's baseline build is present
//!    ([`prepare_bun`]), so neither the install nor Pi itself runs a bun that
//!    dies with an illegal instruction;
//! 2. seed `package.json` with the pins ([`super::pi_runtime::seed_package_json`]);
//! 3. run `bun add <pins> --force --ignore-scripts` with a cache inside the
//!    install directory. A stock bun that dies with `0xC000001D` is recorded,
//!    the baseline build is fetched and the install is retried with it once.
//!    When bun's cache fails with a Windows permission error the install is
//!    retried with npm;
//! 4. apply the runtime patch ([`super::pi_compaction::sync`]); a patch that
//!    does not apply leaves Pi working without it.
//!
//! An install of the pinned release whose files do not take the patch (an
//! older build's patch, another vendor's patch, a package Handvel does not
//! ship) is rebuilt **beside** the live one, in `<install>/.rebuild`, and
//! swapped in only once it is complete and patched. A Pi started meanwhile
//! keeps running the live tree, and when the live tree cannot be moved (on
//! Windows, a running Pi can hold files in it) nothing changes and the next
//! start tries again. Such a rebuild runs at most once per build.
//!
//! Every package comes from the public npm registry; lifecycle scripts never
//! run.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;

use serde_json::json;
use tracing::{debug, info, warn};

use super::bun_runtime::{self, BunBuild};
use super::pi_compaction;
use super::pi_runtime;

/// bun's package cache, inside the install directory so a broken global
/// cache (a known Windows failure) cannot break the install.
pub const BUN_CACHE_DIR: &str = ".bun-cache";
/// Where a rebuild is installed before it replaces the live tree.
pub const STAGING_DIR: &str = ".rebuild";
/// Where the live `node_modules` is moved while the rebuilt one takes its
/// place; removed right after (or at the next start).
pub const OLD_TREE_DIR: &str = ".node_modules.old";
/// Records the build (Pi release and patch fingerprint) the install was last
/// rebuilt for, so a rebuild that does not help is not repeated every start.
pub const REBUILD_MARKER: &str = ".rebuilt-for-patch";
/// Lockfiles that describe a `node_modules` tree.
const LOCKFILES: &[&str] = &["bun.lock", "bun.lockb", "package-lock.json"];

/// Longest piece of installer output kept in an error message.
const MAX_ERROR_CHARS: usize = 800;

/// Whether a failed bun install should be retried with npm: bun's cache on
/// Windows sometimes cannot rename or extract files (`EPERM`) when another
/// process or a virus scanner holds them.
pub fn should_retry_with_npm(output_text: &str) -> bool {
    let lower = output_text.to_lowercase();
    lower.contains("eperm")
        && (lower.contains("ntsetinformationfile")
            || lower.contains("cache dir")
            || lower.contains("extracting tarball")
            || lower.contains("moving"))
}

/// The last [`MAX_ERROR_CHARS`] characters of `text`, trimmed.
pub fn tail_of(text: &str) -> String {
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    if count <= MAX_ERROR_CHARS {
        return trimmed.to_string();
    }
    let tail: String = trimmed.chars().skip(count - MAX_ERROR_CHARS).collect();
    format!("...{tail}")
}

/// One line describing a failed installer run.
pub fn describe_failure(tool: &str, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = if stderr.trim().is_empty() {
        tail_of(&stdout)
    } else {
        tail_of(&stderr)
    };
    let code = output
        .status
        .code()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "signal".to_string());
    format!("{tool} install failed (exit {code}). output: {details}")
}

fn no_window(_cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        _cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

fn run(mut cmd: Command, what: &str) -> Result<Output, String> {
    no_window(&mut cmd);
    cmd.output()
        .map_err(|e| format!("could not start {what}: {e}"))
}

/// The `bun add` command for `install_dir`.
pub fn bun_add_command(install_dir: &Path, bun: &str) -> Command {
    bun_add_command_in(install_dir, &install_dir.join(BUN_CACHE_DIR), bun)
}

/// The `bun add` command for `dir`, with bun's package cache at `cache`.
fn bun_add_command_in(dir: &Path, cache: &Path, bun: &str) -> Command {
    let _ = std::fs::create_dir_all(cache);
    let mut cmd = Command::new(bun);
    cmd.current_dir(dir)
        .env("BUN_INSTALL_CACHE_DIR", cache)
        .args(pi_runtime::bun_install_args());
    cmd
}

/// The npm fallback command for `install_dir`.
pub fn npm_install_command(install_dir: &Path) -> Command {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd.exe");
        c.args(["/C", "npm"]);
        c
    } else {
        Command::new("npm")
    };
    cmd.current_dir(install_dir)
        .args(pi_runtime::npm_install_args());
    cmd
}

/// On a machine that needs bun's baseline build, make sure it is unpacked.
/// Returns the bun to run: `stock` or the baseline build. Blocking (it may
/// download about 40 MB); call it from a background thread.
pub fn prepare_bun(install_dir: &Path, stock: Option<String>) -> Option<String> {
    let (build, reason) = bun_runtime::choose_bun_build(bun_runtime::current_facts(install_dir));
    if build == BunBuild::Baseline {
        match bun_runtime::ensure_baseline_bun(install_dir) {
            Ok(path) => {
                info!(
                    "bun: baseline build selected for Pi ({}): {}",
                    reason,
                    path.display()
                );
                return Some(path.to_string_lossy().into_owned());
            }
            Err(e) => warn!(
                "bun: the baseline build is needed ({}) but could not be set up: {:#}",
                reason, e
            ),
        }
    }
    stock
}

/// Install the pinned Pi release into `install_dir` with `bun`, then apply
/// the runtime patch. See the module documentation for the steps.
pub fn install(install_dir: &Path, bun: &str) -> Result<(), String> {
    install_into(install_dir, install_dir, bun)
}

/// [`install`] into `target`, using the bun cache, the baseline bun and the
/// crash marker of the runtime at `runtime_dir` (the live install directory;
/// `target` is a staging directory during a rebuild).
fn install_into(target: &Path, runtime_dir: &Path, bun: &str) -> Result<(), String> {
    std::fs::create_dir_all(target)
        .map_err(|e| format!("could not create {}: {e}", target.display()))?;
    pi_runtime::seed_package_json(target)
        .map_err(|e| format!("could not seed package.json: {e:#}"))?;

    info!(
        "pi runtime: installing pi {} into {} with {}",
        pi_runtime::PI_VERSION,
        target.display(),
        bun
    );
    let cache = runtime_dir.join(BUN_CACHE_DIR);
    let mut output = run(bun_add_command_in(target, &cache, bun), "bun")?;

    if !output.status.success() && bun_runtime::is_illegal_instruction_exit(output.status.code()) {
        bun_runtime::record_stock_bun_crash(runtime_dir);
        match bun_runtime::ensure_baseline_bun(runtime_dir) {
            Ok(baseline) => {
                let baseline = baseline.to_string_lossy().into_owned();
                if baseline != bun {
                    info!("pi runtime: retrying the install with the baseline bun build");
                    output = run(
                        bun_add_command_in(target, &cache, &baseline),
                        "baseline bun",
                    )?;
                }
            }
            Err(e) => warn!("bun: baseline build unavailable: {:#}", e),
        }
    }

    if !output.status.success() {
        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        let bun_failure = describe_failure("bun", &output);
        if !should_retry_with_npm(&combined) {
            return Err(bun_failure);
        }
        warn!(
            "pi runtime: bun's cache failed with a permission error; retrying with npm: {}",
            bun_failure
        );
        let npm = run(npm_install_command(target), "npm")
            .map_err(|e| format!("{bun_failure}; npm fallback: {e}"))?;
        if !npm.status.success() {
            return Err(format!(
                "{bun_failure}; npm fallback also failed: {}",
                describe_failure("npm", &npm)
            ));
        }
    }

    let installed = pi_runtime::installed_version(target);
    if installed.as_deref() != Some(pi_runtime::PI_VERSION) {
        return Err(format!(
            "the install finished but pi-coding-agent is {:?}, not {}",
            installed,
            pi_runtime::PI_VERSION
        ));
    }
    info!(
        "pi runtime: pi {} installed in {}",
        pi_runtime::PI_VERSION,
        target.display()
    );
    let _ = pi_compaction::sync(target);
    Ok(())
}

/// Packages Pi needs at run time that bun on Windows has failed to hoist
/// before; each must resolve from the agent's `dist` directory.
pub const REQUIRED_DEPENDENCIES: &[&str] = &[
    pi_runtime::PI_AI,
    pi_runtime::ANTHROPIC_SDK,
    pi_runtime::CROSS_SPAWN,
];

/// Where an install directory stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallState {
    /// The pinned release with every required dependency.
    Healthy,
    /// Nothing installed yet.
    Missing,
    /// A complete install of another release (an upgrade is due).
    Outdated(Option<String>),
    /// A partial or damaged install; it is cleared before reinstalling.
    Broken(String),
}

impl InstallState {
    /// A one-line reason for anything but [`InstallState::Healthy`].
    pub fn problem(&self) -> Option<String> {
        match self {
            InstallState::Healthy => None,
            InstallState::Missing => Some("Pi is not installed".to_string()),
            InstallState::Outdated(v) => Some(format!(
                "Pi {} is installed, the runtime needs {}",
                v.as_deref().unwrap_or("of an unknown version"),
                pi_runtime::PI_VERSION
            )),
            InstallState::Broken(why) => Some(why.clone()),
        }
    }
}

/// Find `package` the way Node resolves it from `start`: in `node_modules`
/// of `start` and each parent, up to `stop`.
pub fn resolve_package_from(start: &Path, stop: &Path, package: &str) -> Option<PathBuf> {
    let mut cursor = start.to_path_buf();
    loop {
        let dir = pi_runtime::node_module_dir(&cursor, package);
        if dir.join("package.json").is_file() {
            return Some(dir);
        }
        if cursor == stop || !cursor.pop() {
            return None;
        }
    }
}

/// Classify the install at `install_dir`.
pub fn install_state(install_dir: &Path) -> InstallState {
    let cli = pi_runtime::cli_entrypoint(install_dir);
    if !cli.is_file() {
        return if install_dir.join("node_modules").exists() {
            InstallState::Broken(format!("missing Pi entrypoint at {}", cli.display()))
        } else {
            InstallState::Missing
        };
    }
    let dist = pi_runtime::package_dir(install_dir).join("dist");
    for package in REQUIRED_DEPENDENCIES {
        if resolve_package_from(&dist, install_dir, package).is_none() {
            return InstallState::Broken(format!(
                "missing Pi dependency {} from {}",
                package,
                dist.display()
            ));
        }
    }
    match pi_runtime::installed_version(install_dir) {
        Some(v) if v == pi_runtime::PI_VERSION => InstallState::Healthy,
        other => InstallState::Outdated(other),
    }
}

/// What [`ensure_current`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnsureOutcome {
    /// The install was already the pinned release.
    Current,
    /// Pi was installed or upgraded.
    Installed,
    /// A damaged install was cleared and installed again.
    Repaired,
    /// A healthy install that did not take the patch was rebuilt beside the
    /// live one from stock files and swapped in.
    Rebuilt,
}

/// The app and the embedded engine both bring the runtime up at start;
/// this keeps their installs from running in the same directory at once.
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

/// Install directories a rebuild was already tried for in this process, so
/// the app's and the engine's start-up calls do not both try.
static REBUILD_TRIED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Make the install at `install_dir` the pinned, patched release: set up the
/// baseline bun where needed, install, upgrade or repair Pi, and apply the
/// runtime patch. `stock_bun` is the bun the caller found. Blocking.
pub fn ensure_current(
    install_dir: &Path,
    stock_bun: Option<String>,
) -> Result<EnsureOutcome, String> {
    let _guard = INSTALL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    recover_interrupted_rebuild(install_dir);
    let bun = prepare_bun(install_dir, stock_bun);
    let state = install_state(install_dir);
    if state == InstallState::Healthy {
        return Ok(match rebuild_reason(install_dir) {
            None => EnsureOutcome::Current,
            Some(reason) => rebuild_once(install_dir, bun.as_deref(), &reason),
        });
    }
    let bun = bun.ok_or_else(|| {
        format!(
            "{}, and bun was not found to install it",
            state.problem().unwrap_or_default()
        )
    })?;
    let outcome = match &state {
        InstallState::Broken(why) => {
            warn!(
                "pi runtime: repairing the install in {}: {}",
                install_dir.display(),
                why
            );
            clear_install_artifacts(install_dir);
            EnsureOutcome::Repaired
        }
        _ => {
            if let Some(problem) = state.problem() {
                info!("pi runtime: {}", problem);
            }
            EnsureOutcome::Installed
        }
    };
    install(install_dir, &bun)?;
    if let Some(problem) = install_state(install_dir).problem() {
        return Err(format!(
            "the Pi install finished but verification failed: {problem}"
        ));
    }
    Ok(outcome)
}

/// Remove what an interrupted or broken install leaves behind, so the next
/// install starts from a clean tree. The config directory, the baseline bun
/// and the crash marker are kept.
pub fn clear_install_artifacts(install_dir: &Path) {
    let _ = std::fs::remove_dir_all(install_dir.join("node_modules"));
    for name in LOCKFILES {
        let _ = std::fs::remove_file(install_dir.join(name));
    }
}

/// Why a healthy install of the pinned release must be rebuilt from stock
/// files, if it must. Applies the patch on the way when it can be applied.
fn rebuild_reason(install_dir: &Path) -> Option<String> {
    match pi_compaction::sync(install_dir) {
        pi_compaction::Outcome::Stale(why) => {
            return Some(format!("the runtime patch is from an older build ({why})"));
        }
        pi_compaction::Outcome::Foreign(why) => {
            return Some(format!("Pi's files were changed outside Handvel ({why})"));
        }
        _ => {}
    }
    let excluded = pi_runtime::excluded_packages_present(install_dir);
    (!excluded.is_empty()).then(|| {
        format!(
            "it carries {}, which Handvel does not install",
            excluded.join(", ")
        )
    })
}

/// Whether the install was already rebuilt for this build's Pi release and
/// patch.
fn rebuilt_for_this_build(install_dir: &Path) -> bool {
    let marker = pi_runtime::read_json_object(&install_dir.join(REBUILD_MARKER));
    marker["piVersion"] == pi_runtime::PI_VERSION
        && marker["patch"] == pi_compaction::fingerprint().as_str()
}

/// Rebuild the install for `reason`, at most once per process and once per
/// build. Never fails: when the rebuild cannot run or does not finish, Pi
/// keeps running the installed files and the reason is logged.
fn rebuild_once(install_dir: &Path, bun: Option<&str>, reason: &str) -> EnsureOutcome {
    if rebuilt_for_this_build(install_dir) {
        debug!(
            "pi runtime: {}; it was already rebuilt once for this build, so the installed files stay",
            reason
        );
        return EnsureOutcome::Current;
    }
    let Some(bun) = bun else {
        warn!(
            "pi runtime: {}, but bun was not found to rebuild it; Pi keeps running the installed files",
            reason
        );
        return EnsureOutcome::Current;
    };
    {
        let mut tried = REBUILD_TRIED.lock().unwrap_or_else(|p| p.into_inner());
        if tried.iter().any(|d| d == install_dir) {
            return EnsureOutcome::Current;
        }
        tried.push(install_dir.to_path_buf());
    }
    info!(
        "pi runtime: rebuilding the install in {} beside the live one: {}",
        install_dir.display(),
        reason
    );
    match rebuild_beside(install_dir, bun) {
        Ok(()) => {
            let record = json!({
                "piVersion": pi_runtime::PI_VERSION,
                "patch": pi_compaction::fingerprint(),
                "rebuiltAt": chrono::Utc::now().to_rfc3339(),
                "reason": reason,
            });
            if let Err(e) =
                pi_runtime::write_json_atomic(&install_dir.join(REBUILD_MARKER), &record)
            {
                warn!("pi runtime: could not record the rebuild: {:#}", e);
            }
            info!("pi runtime: the rebuilt install is in place");
            EnsureOutcome::Rebuilt
        }
        Err(e) => {
            warn!(
                "pi runtime: the rebuild did not finish ({}); Pi keeps running the installed files and the next start tries again",
                e
            );
            EnsureOutcome::Current
        }
    }
}

/// Install the pinned release with stock files into `<install>/.rebuild`
/// (bun's cache makes that quick), patch it there, and swap it in for the
/// live tree. The live tree is untouched until the swap; on any failure the
/// staging directory is removed and the live tree stays as it was.
fn rebuild_beside(install_dir: &Path, bun: &str) -> Result<(), String> {
    let staging = install_dir.join(STAGING_DIR);
    if staging.exists() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    std::fs::create_dir_all(&staging)
        .map_err(|e| format!("could not create {}: {e}", staging.display()))?;
    // Keep what the live manifest lists beyond the pins; seeding applies the
    // pins and drops packages Handvel does not install. No lockfile: it
    // would pin the tree being replaced.
    let manifest = install_dir.join("package.json");
    if manifest.is_file() {
        std::fs::copy(&manifest, staging.join("package.json"))
            .map_err(|e| format!("could not copy package.json: {e}"))?;
    }
    let result = install_into(&staging, install_dir, bun)
        .and_then(|()| match install_state(&staging).problem() {
            Some(problem) => Err(format!("the rebuilt install is incomplete: {problem}")),
            None => Ok(()),
        })
        .and_then(|()| swap_in_rebuilt_tree(install_dir, &staging));
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// Put the tree built in `staging` in place of the live one. The live
/// `node_modules` is moved aside first; when that fails (on Windows, a
/// running Pi can hold files in it) nothing has changed.
fn swap_in_rebuilt_tree(install_dir: &Path, staging: &Path) -> Result<(), String> {
    let live = install_dir.join("node_modules");
    let old = install_dir.join(OLD_TREE_DIR);
    if old.exists() {
        let _ = std::fs::remove_dir_all(&old);
    }
    if live.exists() {
        std::fs::rename(&live, &old).map_err(|e| {
            format!("could not move the live tree aside (a running Pi may hold files in it): {e}")
        })?;
    }
    if let Err(e) = std::fs::rename(staging.join("node_modules"), &live) {
        if old.exists() {
            let _ = std::fs::rename(&old, &live);
        }
        return Err(format!("could not move the rebuilt tree into place: {e}"));
    }
    // The manifest and lockfile now describe the tree in place.
    for name in LOCKFILES {
        let _ = std::fs::remove_file(install_dir.join(name));
    }
    for name in std::iter::once(&"package.json").chain(LOCKFILES.iter()) {
        let from = staging.join(name);
        if from.is_file() {
            if let Err(e) = std::fs::rename(&from, install_dir.join(name)) {
                warn!("pi runtime: could not move the rebuilt {}: {}", name, e);
            }
        }
    }
    if let Err(e) = std::fs::remove_dir_all(&old) {
        debug!(
            "pi runtime: the replaced tree in {} is removed at the next start: {}",
            old.display(),
            e
        );
    }
    Ok(())
}

/// Clean up after a rebuild that was cut short: put the live tree back when
/// the process died between the two moves of the swap, and remove the staging
/// directory and the replaced tree.
fn recover_interrupted_rebuild(install_dir: &Path) {
    let live = install_dir.join("node_modules");
    let old = install_dir.join(OLD_TREE_DIR);
    if !live.exists() && old.is_dir() {
        match std::fs::rename(&old, &live) {
            Ok(()) => warn!("pi runtime: restored the install tree after an interrupted rebuild"),
            Err(e) => warn!("pi runtime: could not restore {}: {}", old.display(), e),
        }
    }
    for dir in [old, install_dir.join(STAGING_DIR)] {
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_bun_cache_permission_failures_go_to_npm() {
        assert!(should_retry_with_npm(
            "error: EPERM: operation not permitted, NtSetInformationFile"
        ));
        assert!(should_retry_with_npm(
            "EPERM while extracting tarball for lru-cache"
        ));
        assert!(should_retry_with_npm("eperm moving into cache dir"));
        assert!(!should_retry_with_npm("EPERM: open package.json"));
        assert!(!should_retry_with_npm(
            "404 Not Found @earendil-works/pi-ai"
        ));
    }

    #[test]
    fn long_output_keeps_its_end() {
        let long = format!("{}END", "x".repeat(5000));
        let tail = tail_of(&long);
        assert!(tail.starts_with("..."));
        assert!(tail.ends_with("END"));
        assert_eq!(tail.chars().count(), MAX_ERROR_CHARS + 3);
        assert_eq!(tail_of("  short \n"), "short");
        // Multi-byte characters are never split.
        let wide = "é".repeat(2000);
        assert_eq!(tail_of(&wide).chars().count(), MAX_ERROR_CHARS + 3);
    }

    #[test]
    fn the_bun_command_pins_and_uses_a_local_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = bun_add_command(dir.path(), "bun");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, pi_runtime::bun_install_args());
        let cache = cmd
            .get_envs()
            .find(|(k, _)| *k == "BUN_INSTALL_CACHE_DIR")
            .and_then(|(_, v)| v)
            .map(PathBuf::from);
        assert_eq!(cache, Some(dir.path().join(BUN_CACHE_DIR)));
        assert_eq!(cmd.get_current_dir(), Some(dir.path()));
    }

    #[test]
    fn the_npm_fallback_skips_scripts() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = npm_install_command(dir.path());
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"--ignore-scripts".to_string()));
        assert!(args.contains(&"install".to_string()));
    }

    #[test]
    fn clearing_artifacts_keeps_config_and_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("node_modules").join("x")).unwrap();
        std::fs::create_dir_all(d.join(pi_runtime::CONFIG_DIR_NAME)).unwrap();
        std::fs::create_dir_all(d.join(bun_runtime::BASELINE_DIR_NAME)).unwrap();
        std::fs::write(d.join("bun.lock"), "{}").unwrap();
        std::fs::write(d.join("package.json"), "{}").unwrap();
        clear_install_artifacts(d);
        assert!(!d.join("node_modules").exists());
        assert!(!d.join("bun.lock").exists());
        assert!(d.join("package.json").exists());
        assert!(d.join(pi_runtime::CONFIG_DIR_NAME).exists());
        assert!(d.join(bun_runtime::BASELINE_DIR_NAME).exists());
    }

    #[test]
    fn a_missing_bun_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let err = install(dir.path(), "/nonexistent/handvel-test-bun").unwrap_err();
        assert!(err.contains("could not start bun"), "{err}");
        // The manifest is seeded before bun runs.
        assert!(dir.path().join("package.json").exists());
    }

    fn fake_install(dir: &Path, version: &str, deps: &[&str]) {
        let pkg = pi_runtime::package_dir(dir);
        std::fs::create_dir_all(pkg.join("dist")).unwrap();
        std::fs::write(pkg.join("dist").join("cli.js"), "").unwrap();
        std::fs::write(
            pkg.join("package.json"),
            format!(r#"{{"version":"{version}"}}"#),
        )
        .unwrap();
        for dep in deps {
            let d = pi_runtime::node_module_dir(dir, dep);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("package.json"), "{}").unwrap();
        }
    }

    #[test]
    fn install_states_are_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(install_state(dir.path()), InstallState::Missing);

        std::fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        assert!(matches!(install_state(dir.path()), InstallState::Broken(_)));

        fake_install(dir.path(), pi_runtime::PI_VERSION, &[pi_runtime::PI_AI]);
        let state = install_state(dir.path());
        assert!(
            matches!(&state, InstallState::Broken(why) if why.contains(pi_runtime::ANTHROPIC_SDK)),
            "{state:?}"
        );

        fake_install(dir.path(), "0.75.4", REQUIRED_DEPENDENCIES);
        assert_eq!(
            install_state(dir.path()),
            InstallState::Outdated(Some("0.75.4".into()))
        );
        assert!(install_state(dir.path())
            .problem()
            .unwrap()
            .contains("0.84.1"));

        fake_install(dir.path(), pi_runtime::PI_VERSION, REQUIRED_DEPENDENCIES);
        assert_eq!(install_state(dir.path()), InstallState::Healthy);
        assert_eq!(install_state(dir.path()).problem(), None);
    }

    #[test]
    fn dependencies_nested_under_the_agent_resolve() {
        let dir = tempfile::tempdir().unwrap();
        fake_install(
            dir.path(),
            pi_runtime::PI_VERSION,
            &[pi_runtime::PI_AI, pi_runtime::CROSS_SPAWN],
        );
        let nested = pi_runtime::node_module_dir(
            &pi_runtime::package_dir(dir.path()),
            pi_runtime::ANTHROPIC_SDK,
        );
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("package.json"), "{}").unwrap();
        assert_eq!(install_state(dir.path()), InstallState::Healthy);
    }

    #[test]
    fn a_healthy_install_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        fake_install(dir.path(), pi_runtime::PI_VERSION, REQUIRED_DEPENDENCIES);
        // No bun is needed when nothing has to be installed.
        assert_eq!(ensure_current(dir.path(), None), Ok(EnsureOutcome::Current));
    }

    #[test]
    fn without_bun_a_missing_install_reports_why() {
        let dir = tempfile::tempdir().unwrap();
        let err = ensure_current(dir.path(), None).unwrap_err();
        assert!(
            err.contains("not installed") && err.contains("bun"),
            "{err}"
        );
    }

    /// A real install from the npm registry into a fresh data dir: the
    /// acceptance check "the installed package.json pins 0.84.1 and pi-config
    /// carries the .migrated-from-global marker". Needs network and bun:
    /// `FREENPIPE_PI_TEST_DATA_DIR=<dir> cargo test -p screenpipe-core --lib
    /// pi_install -- --ignored`. The directory may already hold an install
    /// (an outdated or stale one is upgraded).
    #[test]
    #[ignore]
    fn a_real_install_into_a_fresh_data_dir() {
        let Some(data) = std::env::var_os("FREENPIPE_PI_TEST_DATA_DIR").map(PathBuf::from) else {
            eprintln!("FREENPIPE_PI_TEST_DATA_DIR is not set; skipping");
            return;
        };
        let bun = which::which("bun").expect("bun on PATH");
        let install_dir = pi_runtime::install_dir_in(&data);
        let outcome =
            ensure_current(&install_dir, Some(bun.to_string_lossy().into_owned())).unwrap();
        eprintln!("first ensure_current: {outcome:?}");
        assert_eq!(install_state(&install_dir), InstallState::Healthy);

        let pkg = pi_runtime::read_json_object(&install_dir.join("package.json"));
        assert_eq!(pkg["dependencies"][pi_runtime::PI_CODING_AGENT], "0.84.1");
        assert_eq!(pkg["overrides"][pi_runtime::PI_AI], "0.84.1");
        assert_eq!(
            pi_runtime::installed_version(&install_dir).as_deref(),
            Some("0.84.1")
        );
        // The runtime patch is in place.
        let session = pi_runtime::package_dir(&install_dir).join("dist/core/agent-session.js");
        let text = std::fs::read_to_string(session).unwrap();
        assert!(text.contains("freenpipe-patch:proactive-compaction-v1"));
        // A second start finds everything current.
        let again = ensure_current(&install_dir, Some(bun.to_string_lossy().into_owned()));
        assert_eq!(again, Ok(EnsureOutcome::Current));

        // The isolated config, seeded from a stand-in global config.
        let home = data.join("home");
        let global = home.join(".pi").join("agent");
        std::fs::create_dir_all(&global).unwrap();
        std::fs::write(global.join("auth.json"), "{}").unwrap();
        let config = pi_runtime::resolve_config_dir(None, &data);
        let global_dir = pi_runtime::resolve_global_config_dir(None, Some(&home), &config);
        pi_runtime::seed_from_global(&config, global_dir.as_deref()).unwrap();
        assert!(config.join(pi_runtime::MIGRATED_MARKER).is_file());
        assert!(config.join("auth.json").is_file());
    }

    #[test]
    fn a_rebuilt_tree_is_swapped_in_with_its_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("node_modules").join("old-only")).unwrap();
        std::fs::write(d.join("package.json"), r#"{"old":true}"#).unwrap();
        std::fs::write(d.join("bun.lockb"), "old").unwrap();
        std::fs::create_dir_all(d.join(pi_runtime::CONFIG_DIR_NAME)).unwrap();
        let staging = d.join(STAGING_DIR);
        std::fs::create_dir_all(staging.join("node_modules").join("new-only")).unwrap();
        std::fs::write(staging.join("package.json"), r#"{"new":true}"#).unwrap();
        std::fs::write(staging.join("bun.lock"), "new").unwrap();

        swap_in_rebuilt_tree(d, &staging).unwrap();
        assert!(d.join("node_modules").join("new-only").exists());
        assert!(!d.join("node_modules").join("old-only").exists());
        assert_eq!(
            pi_runtime::read_json_object(&d.join("package.json")),
            json!({"new": true})
        );
        assert!(!d.join("bun.lockb").exists());
        assert_eq!(std::fs::read_to_string(d.join("bun.lock")).unwrap(), "new");
        assert!(!d.join(OLD_TREE_DIR).exists());
        assert!(d.join(pi_runtime::CONFIG_DIR_NAME).exists());
    }

    #[test]
    fn a_swap_cut_short_puts_the_live_tree_back() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        // The process died after moving the live tree aside.
        std::fs::create_dir_all(d.join(OLD_TREE_DIR).join("live")).unwrap();
        std::fs::create_dir_all(d.join(STAGING_DIR).join("node_modules")).unwrap();
        recover_interrupted_rebuild(d);
        assert!(d.join("node_modules").join("live").exists());
        assert!(!d.join(OLD_TREE_DIR).exists());
        assert!(!d.join(STAGING_DIR).exists());

        // With the live tree in place the leftovers are simply removed.
        std::fs::create_dir_all(d.join(OLD_TREE_DIR).join("replaced")).unwrap();
        recover_interrupted_rebuild(d);
        assert!(d.join("node_modules").join("live").exists());
        assert!(!d.join(OLD_TREE_DIR).exists());
    }

    /// Write an executable stand-in for bun.
    #[cfg(unix)]
    fn fake_bun(dir: &Path, name: &str, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    /// A healthy 0.84.1 install written by another build: it still lists
    /// and carries `tinfoil`.
    fn foreign_install(dir: &Path) {
        fake_install(dir, pi_runtime::PI_VERSION, REQUIRED_DEPENDENCIES);
        let tinfoil = pi_runtime::node_module_dir(dir, "tinfoil");
        std::fs::create_dir_all(&tinfoil).unwrap();
        std::fs::write(tinfoil.join("package.json"), "{}").unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"dependencies":{"@earendil-works/pi-coding-agent":"0.84.1","tinfoil":"1.0.0"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.join(pi_runtime::CONFIG_DIR_NAME)).unwrap();
        std::fs::write(
            dir.join(pi_runtime::CONFIG_DIR_NAME).join("auth.json"),
            "{}",
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_foreign_install_is_rebuilt_beside_the_live_one_once() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tempfile::tempdir().unwrap();
        let d = dir.path().join("pi-agent");
        foreign_install(&d);
        assert_eq!(install_state(&d), InstallState::Healthy);
        assert_eq!(pi_runtime::excluded_packages_present(&d), vec!["tinfoil"]);

        // Lays out a minimal 0.84.1 tree in the current directory, the way
        // `bun add` would, and records the cache it was given.
        let bun = fake_bun(
            tools.path(),
            "bun",
            r#"[ "$1" = add ] || exit 2
m=node_modules
mkdir -p "$m/@earendil-works/pi-coding-agent/dist" "$m/@earendil-works/pi-ai" "$m/@anthropic-ai/sdk" "$m/cross-spawn"
echo '{"version":"0.84.1"}' > "$m/@earendil-works/pi-coding-agent/package.json"
: > "$m/@earendil-works/pi-coding-agent/dist/cli.js"
echo '{"version":"0.84.1"}' > "$m/@earendil-works/pi-ai/package.json"
echo '{}' > "$m/@anthropic-ai/sdk/package.json"
echo '{}' > "$m/cross-spawn/package.json"
printf '%s' "$BUN_INSTALL_CACHE_DIR" > "$m/.cache-dir"
echo '{}' > bun.lock"#,
        );
        assert_eq!(
            ensure_current(&d, Some(bun.clone())),
            Ok(EnsureOutcome::Rebuilt)
        );
        let nm = d.join("node_modules");
        assert!(!nm.join("tinfoil").exists());
        assert_eq!(
            std::fs::read_to_string(nm.join(".cache-dir")).unwrap(),
            d.join(BUN_CACHE_DIR).to_string_lossy()
        );
        let pkg = pi_runtime::read_json_object(&d.join("package.json"));
        assert!(pkg["dependencies"].get("tinfoil").is_none());
        assert_eq!(pkg["overrides"][pi_runtime::PI_AI], pi_runtime::PI_VERSION);
        assert!(d.join("bun.lock").is_file());
        assert!(!d.join(STAGING_DIR).exists());
        assert!(!d.join(OLD_TREE_DIR).exists());
        assert!(d
            .join(pi_runtime::CONFIG_DIR_NAME)
            .join("auth.json")
            .is_file());
        assert!(rebuilt_for_this_build(&d));
        assert_eq!(install_state(&d), InstallState::Healthy);
        // Nothing is left to rebuild.
        assert_eq!(ensure_current(&d, Some(bun)), Ok(EnsureOutcome::Current));
    }

    #[cfg(unix)]
    #[test]
    fn a_rebuild_that_fails_leaves_the_live_tree_running() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tempfile::tempdir().unwrap();
        let d = dir.path().join("pi-agent");
        foreign_install(&d);
        let calls = tools.path().join("calls");
        let bun = fake_bun(
            tools.path(),
            "bun",
            &format!(
                "echo x >> '{}'\necho 'error: registry unreachable (503)' >&2\nexit 1",
                calls.display()
            ),
        );
        assert_eq!(
            ensure_current(&d, Some(bun.clone())),
            Ok(EnsureOutcome::Current)
        );
        // The live tree is exactly as it was.
        assert!(pi_runtime::node_module_dir(&d, "tinfoil").exists());
        assert_eq!(install_state(&d), InstallState::Healthy);
        assert!(!d.join(STAGING_DIR).exists());
        assert!(!rebuilt_for_this_build(&d));
        // One try per process: the second start-up call does not run bun.
        assert_eq!(ensure_current(&d, Some(bun)), Ok(EnsureOutcome::Current));
        assert_eq!(std::fs::read_to_string(&calls).unwrap().lines().count(), 1);
    }

    #[test]
    fn a_rebuild_already_done_for_this_build_is_not_repeated() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        foreign_install(d);
        assert!(!rebuilt_for_this_build(d));
        pi_runtime::write_json_atomic(
            &d.join(REBUILD_MARKER),
            &json!({"piVersion": pi_runtime::PI_VERSION, "patch": "an older build"}),
        )
        .unwrap();
        assert!(!rebuilt_for_this_build(d));
        pi_runtime::write_json_atomic(
            &d.join(REBUILD_MARKER),
            &json!({"piVersion": pi_runtime::PI_VERSION, "patch": pi_compaction::fingerprint()}),
        )
        .unwrap();
        assert!(rebuilt_for_this_build(d));
        // No bun is needed: the installed files stay.
        assert_eq!(ensure_current(d, None), Ok(EnsureOutcome::Current));
        assert!(pi_runtime::node_module_dir(d, "tinfoil").exists());
    }

    #[test]
    fn stock_bun_is_kept_where_no_baseline_is_needed() {
        // The Linux test machine never needs the baseline build.
        let dir = tempfile::tempdir().unwrap();
        if !cfg!(all(windows, target_arch = "x86_64")) {
            assert_eq!(
                prepare_bun(dir.path(), Some("bun".into())),
                Some("bun".into())
            );
        }
    }
}
