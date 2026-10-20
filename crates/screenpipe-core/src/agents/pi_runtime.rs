// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Layout and environment of the managed Pi runtime.
//!
//! Both the desktop app (chat) and the engine (pipes) run the Pi coding agent
//! from one install under `<data dir>/pi-agent/`. This module owns what the
//! two share:
//!
//! * the pinned Pi release and the `package.json` seeded before every
//!   install, which lists the pins as dependencies **and** as overrides so the
//!   whole `@earendil-works/*` family resolves to the exact release the
//!   runtime patch (see [`super::pi_compaction`]) was written against;
//! * the install command (`bun add <pins> --force --ignore-scripts`);
//! * Pi's configuration directory. Freenpipe keeps its own,
//!   `<data dir>/pi-agent/pi-config`, instead of sharing the user's global
//!   `~/.pi/agent`, so provider entries written for chats and pipes never
//!   rewrite the user's own Pi setup. The first time that directory is used
//!   it is seeded from the global one (sign-ins, trusted folders, installed
//!   Pi packages and saved sessions), once, under a lock, and a
//!   `.migrated-from-global` marker records that it happened;
//! * the environment every Pi process gets ([`child_env`]).
//!
//! Functions that take explicit paths do the work; the argument-free wrappers
//! resolve the real data dir and home directory. Tests only use the former.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use tracing::{debug, info, warn};

use crate::env;

// ---------------------------------------------------------------------------
// Pins
// ---------------------------------------------------------------------------

/// Pi release the runtime installs and the runtime patch targets.
pub const PI_VERSION: &str = "0.84.1";
/// npm scope of the Pi packages.
pub const PI_SCOPE: &str = "@earendil-works";
/// The Pi coding agent CLI package.
pub const PI_CODING_AGENT: &str = "@earendil-works/pi-coding-agent";
/// Pi's model/provider layer, installed as a direct dependency so it is
/// hoisted next to the agent (bun on Windows did not always hoist it).
pub const PI_AI: &str = "@earendil-works/pi-ai";
/// Anthropic SDK, a direct dependency for the same hoisting reason.
pub const ANTHROPIC_SDK: &str = "@anthropic-ai/sdk";
/// The Anthropic SDK release `pi-ai` 0.84.1 declares.
pub const ANTHROPIC_SDK_VERSION: &str = "0.91.1";
/// Spawn helper Pi needs on Windows; checked by the app's integrity test.
pub const CROSS_SPAWN: &str = "cross-spawn";
/// Range for [`CROSS_SPAWN`].
pub const CROSS_SPAWN_RANGE: &str = "^7.0.6";

/// Every package of the Pi family the agent pulls in. They depend on each
/// other with caret ranges, so without overrides a later patch release of
/// one of them could be mixed with 0.84.1 of the others.
pub const PI_FAMILY: &[&str] = &[
    PI_CODING_AGENT,
    PI_AI,
    "@earendil-works/pi-agent-core",
    "@earendil-works/pi-tui",
    "@earendil-works/pi-client",
    "@earendil-works/pi-protocol",
    "@earendil-works/pi-telemetry",
];

/// `hosted-git-info` 9 needs `lru-cache` 11. Kept as a nested override so a
/// package manager that hoists an older, incompatible `lru-cache` cannot
/// break it (bun on Windows once did, with lru-cache 7).
const LRU_CACHE_FOR_HOSTED_GIT_INFO: &str = "^11.1.0";

// ---------------------------------------------------------------------------
// Directory layout
// ---------------------------------------------------------------------------

/// Pi's configuration directory, inside the install directory.
pub const CONFIG_DIR_NAME: &str = "pi-config";
/// Marker written into the config directory once it has been seeded.
pub const MIGRATED_MARKER: &str = ".migrated-from-global";
/// Lock held while the config directory is being seeded.
pub const MIGRATION_LOCK: &str = ".migration.lock";

/// A lock older than this is left over from a process that died mid-seed.
const STALE_LOCK_AGE: Duration = Duration::from_secs(10 * 60);
/// How long a second process waits for another one's seeding to finish.
const LOCK_WAIT: Duration = Duration::from_secs(10);

/// `<data dir>/pi-agent`.
pub fn install_dir_in(data_dir: &Path) -> PathBuf {
    data_dir.join(crate::paths::PI_AGENT_DIR)
}

/// The managed install directory for the active data dir.
pub fn install_dir() -> PathBuf {
    install_dir_in(&crate::paths::default_screenpipe_data_dir())
}

/// `<install>/node_modules/<package>` for a scoped or plain package name.
pub fn node_module_dir(install_dir: &Path, package: &str) -> PathBuf {
    let mut dir = install_dir.join("node_modules");
    for segment in package.split('/') {
        dir.push(segment);
    }
    dir
}

/// Directory of the installed Pi coding agent package.
pub fn package_dir(install_dir: &Path) -> PathBuf {
    node_module_dir(install_dir, PI_CODING_AGENT)
}

/// The JS entry point of the installed agent CLI.
pub fn cli_entrypoint(install_dir: &Path) -> PathBuf {
    package_dir(install_dir).join("dist").join("cli.js")
}

/// Version recorded in the installed agent's `package.json`.
pub fn installed_version(install_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(package_dir(install_dir).join("package.json")).ok()?;
    let parsed: Value = serde_json::from_str(&text).ok()?;
    parsed
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Whether the installed agent is the pinned release.
pub fn is_installed_version_current(install_dir: &Path) -> bool {
    match installed_version(install_dir) {
        Some(v) if v == PI_VERSION => true,
        Some(v) => {
            info!("pi runtime: installed {} differs from pinned {}", v, PI_VERSION);
            false
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------
// package.json seeding and the install command
// ---------------------------------------------------------------------------

/// Dependencies the managed `package.json` must list, with their versions.
fn pinned_dependencies() -> Vec<(&'static str, &'static str)> {
    vec![
        (PI_CODING_AGENT, PI_VERSION),
        (PI_AI, PI_VERSION),
        (ANTHROPIC_SDK, ANTHROPIC_SDK_VERSION),
        (CROSS_SPAWN, CROSS_SPAWN_RANGE),
    ]
}

/// Overrides the managed `package.json` must carry.
fn pinned_overrides() -> Vec<(&'static str, Value)> {
    let mut out: Vec<(&'static str, Value)> =
        PI_FAMILY.iter().map(|p| (*p, json!(PI_VERSION))).collect();
    out.push((ANTHROPIC_SDK, json!(ANTHROPIC_SDK_VERSION)));
    out.push((
        "hosted-git-info",
        json!({ "lru-cache": LRU_CACHE_FOR_HOSTED_GIT_INFO }),
    ));
    out
}

/// The `package.json` written into a fresh install directory.
pub fn expected_package_json() -> Value {
    let mut deps = Map::new();
    for (name, version) in pinned_dependencies() {
        deps.insert(name.to_string(), json!(version));
    }
    let mut overrides = Map::new();
    for (name, value) in pinned_overrides() {
        overrides.insert(name.to_string(), value);
    }
    json!({
        "name": "freenpipe-pi-agent",
        "private": true,
        "dependencies": Value::Object(deps),
        "overrides": Value::Object(overrides),
    })
}

/// What [`seed_package_json`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedOutcome {
    /// The file already carried every pin.
    Unchanged,
    /// Pins were added or corrected in an existing file.
    Updated,
    /// There was no file; a fresh one was written.
    Created,
    /// The file was unreadable or corrupt and was replaced.
    Reseeded,
}

/// Bring an existing manifest in line with the pins. Returns whether anything
/// changed. Keys the runtime does not manage are kept.
fn apply_pins(pkg: &mut Value) -> bool {
    if !pkg.is_object() {
        *pkg = json!({});
    }
    let obj = pkg.as_object_mut().expect("object");
    let mut changed = false;

    let deps = obj.entry("dependencies").or_insert_with(|| json!({}));
    if !deps.is_object() {
        *deps = json!({});
        changed = true;
    }
    let deps = deps.as_object_mut().expect("object");
    // Packages from before the upstream namespace rename.
    let legacy: Vec<String> = deps
        .keys()
        .filter(|k| k.starts_with("@mariozechner/"))
        .cloned()
        .collect();
    for key in legacy {
        deps.remove(&key);
        changed = true;
    }
    for (name, version) in pinned_dependencies() {
        if deps.get(name) != Some(&json!(version)) {
            deps.insert(name.to_string(), json!(version));
            changed = true;
        }
    }

    let overrides = obj.entry("overrides").or_insert_with(|| json!({}));
    if !overrides.is_object() {
        *overrides = json!({});
        changed = true;
    }
    let overrides = overrides.as_object_mut().expect("object");
    for (name, value) in pinned_overrides() {
        if overrides.get(name) != Some(&value) {
            overrides.insert(name.to_string(), value);
            changed = true;
        }
    }
    changed
}

fn remove_lockfiles(install_dir: &Path) {
    for name in ["bun.lock", "bun.lockb", "package-lock.json"] {
        let _ = std::fs::remove_file(install_dir.join(name));
    }
}

/// Write the managed `package.json` before an install, pinning the Pi
/// release as dependencies and overrides. A file that cannot be read or
/// parsed (a partial write once left NUL bytes in it) is replaced. Whenever
/// the manifest changes the lockfiles are dropped, because they would pin the
/// previous dependency tree.
pub fn seed_package_json(install_dir: &Path) -> Result<SeedOutcome> {
    std::fs::create_dir_all(install_dir)
        .with_context(|| format!("create {}", install_dir.display()))?;
    let path = install_dir.join("package.json");
    let existing = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warn!("pi runtime: cannot read {}: {} — re-seeding", path.display(), e);
            Some(Vec::new())
        }
    };

    let (mut pkg, mut outcome) = match existing {
        None => (expected_package_json(), SeedOutcome::Created),
        Some(bytes) => {
            let parsed = std::str::from_utf8(&bytes)
                .ok()
                .filter(|text| !text.contains('\0'))
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .filter(Value::is_object);
            match parsed {
                Some(pkg) => (pkg, SeedOutcome::Unchanged),
                None => {
                    warn!(
                        "pi runtime: {} is corrupt — re-seeding it",
                        path.display()
                    );
                    (expected_package_json(), SeedOutcome::Reseeded)
                }
            }
        }
    };

    if outcome == SeedOutcome::Unchanged && apply_pins(&mut pkg) {
        outcome = SeedOutcome::Updated;
    }
    if outcome != SeedOutcome::Unchanged {
        write_json_atomic(&path, &pkg)?;
        remove_lockfiles(install_dir);
        info!(
            "pi runtime: package.json {:?} (pi {}, overrides for {} packages)",
            outcome,
            PI_VERSION,
            pinned_overrides().len()
        );
    }
    Ok(outcome)
}

/// Arguments for `bun` that install the pinned packages into the current
/// directory. `--force` re-resolves against the pins even when a stale tree
/// is present, and `--ignore-scripts` keeps dependency lifecycle scripts from
/// running.
pub fn bun_install_args() -> Vec<String> {
    let mut args = vec!["add".to_string()];
    for (name, version) in pinned_dependencies() {
        args.push(format!("{name}@{version}"));
    }
    args.push("--force".to_string());
    args.push("--ignore-scripts".to_string());
    args
}

/// Arguments for the `npm` fallback the app uses when bun's cache fails.
pub fn npm_install_args() -> Vec<&'static str> {
    vec!["install", "--no-audit", "--no-fund", "--ignore-scripts"]
}

// ---------------------------------------------------------------------------
// The config directory
// ---------------------------------------------------------------------------

/// Pi's configuration directory: `override_dir` when given (from
/// `SCREENPIPE_PI_AGENT_DIR`), otherwise `<data dir>/pi-agent/pi-config`.
pub fn resolve_config_dir(override_dir: Option<&str>, data_dir: &Path) -> PathBuf {
    match override_dir.map(str::trim).filter(|s| !s.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => install_dir_in(data_dir).join(CONFIG_DIR_NAME),
    }
}

/// The configuration directory for this process (not created).
pub fn config_dir() -> PathBuf {
    resolve_config_dir(
        env::non_empty(env::SCREENPIPE_PI_AGENT_DIR).as_deref(),
        &crate::paths::default_screenpipe_data_dir(),
    )
}

/// The user's global Pi config directory, the seeding source: the directory
/// Pi itself would use (`PI_CODING_AGENT_DIR` when it is set in this process
/// and is not the isolated directory, else `~/.pi/agent`).
pub fn resolve_global_config_dir(
    pi_agent_dir_env: Option<&str>,
    home: Option<&Path>,
    isolated: &Path,
) -> Option<PathBuf> {
    if let Some(dir) = pi_agent_dir_env.map(str::trim).filter(|s| !s.is_empty()) {
        let dir = PathBuf::from(dir);
        if !same_path(&dir, isolated) {
            return Some(dir);
        }
    }
    home.map(|h| h.join(".pi").join("agent"))
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Create the config directory if needed, seed it from the global Pi config
/// once, and return it. Seeding problems are logged, never fatal: Pi then
/// simply starts without the user's global sign-ins.
pub fn ensure_config_dir() -> Result<PathBuf> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create pi config dir {}", dir.display()))?;
    if !dir.join(MIGRATED_MARKER).exists() {
        let global = resolve_global_config_dir(
            env::non_empty(env::PI_CODING_AGENT_DIR).as_deref(),
            dirs::home_dir().as_deref(),
            &dir,
        );
        match seed_from_global(&dir, global.as_deref()) {
            Ok(report) => debug!("pi config seeding: {:?}", report),
            Err(e) => warn!("pi config seeding failed: {:#}", e),
        }
    }
    Ok(dir)
}

/// What [`seed_from_global`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedReport {
    /// The marker was already there.
    AlreadySeeded,
    /// Another process holds the lock and did not finish in time.
    Busy,
    /// Seeding ran; lists what was copied and what could not be.
    Seeded {
        copied: Vec<String>,
        failed: Vec<String>,
    },
}

/// Removes the lock file when seeding ends, however it ends.
struct LockGuard(PathBuf);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn try_take_lock(lock: &Path) -> std::io::Result<Option<LockGuard>> {
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock)
    {
        Ok(mut f) => {
            use std::io::Write;
            let _ = writeln!(f, "{}", std::process::id());
            Ok(Some(LockGuard(lock.to_path_buf())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(e),
    }
}

fn lock_is_stale(lock: &Path) -> bool {
    std::fs::metadata(lock)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|age| age > STALE_LOCK_AGE)
        .unwrap_or(false)
}

/// Seed `config_dir` from the global Pi config at `global_dir`, once.
///
/// Copied, never overwriting what the isolated directory already has:
/// `auth.json` (provider sign-ins), `trust.json` (trusted project folders),
/// the `packages` list of `settings.json` (Pi packages the user installed)
/// and the `sessions/` tree. Everything else the user configured globally is
/// left out on purpose; provider entries for Freenpipe are written by the
/// runtime itself. The `.migrated-from-global` marker is written even when
/// there was no global config, so the check is a single file test afterwards.
pub fn seed_from_global(config_dir: &Path, global_dir: Option<&Path>) -> Result<SeedReport> {
    seed_from_global_waiting(config_dir, global_dir, LOCK_WAIT)
}

/// [`seed_from_global`] with an explicit wait for another seeder's lock.
pub fn seed_from_global_waiting(
    config_dir: &Path,
    global_dir: Option<&Path>,
    lock_wait: Duration,
) -> Result<SeedReport> {
    let marker = config_dir.join(MIGRATED_MARKER);
    if marker.exists() {
        return Ok(SeedReport::AlreadySeeded);
    }
    std::fs::create_dir_all(config_dir)?;
    let lock_path = config_dir.join(MIGRATION_LOCK);

    let started = std::time::Instant::now();
    let _guard = loop {
        if let Some(guard) = try_take_lock(&lock_path)? {
            break guard;
        }
        if marker.exists() {
            return Ok(SeedReport::AlreadySeeded);
        }
        if lock_is_stale(&lock_path) {
            warn!(
                "pi config seeding: removing stale lock {}",
                lock_path.display()
            );
            let _ = std::fs::remove_file(&lock_path);
            continue;
        }
        if started.elapsed() > lock_wait {
            return Ok(SeedReport::Busy);
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    // Another process may have finished between our marker check and the lock.
    if marker.exists() {
        return Ok(SeedReport::AlreadySeeded);
    }

    let mut copied = Vec::new();
    let mut failed = Vec::new();
    let source = global_dir.filter(|g| g.is_dir() && !same_path(g, config_dir));
    if let Some(global) = source {
        for name in ["auth.json", "trust.json"] {
            match copy_file_if_absent(&global.join(name), &config_dir.join(name)) {
                Ok(true) => copied.push(name.to_string()),
                Ok(false) => {}
                Err(e) => failed.push(format!("{name}: {e}")),
            }
        }
        match seed_settings_packages(&global.join("settings.json"), &config_dir.join("settings.json"))
        {
            Ok(true) => copied.push("settings.json#packages".to_string()),
            Ok(false) => {}
            Err(e) => failed.push(format!("settings.json: {e}")),
        }
        let sessions = global.join("sessions");
        if sessions.is_dir() {
            let mut errors = Vec::new();
            let n = copy_tree_missing(&sessions, &config_dir.join("sessions"), &mut errors);
            if n > 0 {
                copied.push(format!("sessions ({n} files)"));
            }
            failed.extend(errors);
        }
    }

    let record = json!({
        "migratedAt": chrono::Utc::now().to_rfc3339(),
        "source": source.map(|p| p.to_string_lossy().into_owned()),
        "copied": copied,
        "failed": failed,
    });
    write_json_atomic(&marker, &record)?;
    if failed.is_empty() {
        info!(
            "pi config: seeded {} from {:?} ({} items)",
            config_dir.display(),
            source,
            copied.len()
        );
    } else {
        warn!(
            "pi config: seeded {} with {} problems: {:?}",
            config_dir.display(),
            failed.len(),
            failed
        );
    }
    Ok(SeedReport::Seeded { copied, failed })
}

/// Copy `from` to `to` when `from` exists and `to` does not.
fn copy_file_if_absent(from: &Path, to: &Path) -> std::io::Result<bool> {
    if !from.is_file() || to.exists() {
        return Ok(false);
    }
    std::fs::copy(from, to)?;
    restrict_permissions(to);
    Ok(true)
}

/// Carry the global `packages` list into the isolated `settings.json` when
/// the latter has none.
fn seed_settings_packages(global: &Path, local: &Path) -> Result<bool> {
    let text = match std::fs::read_to_string(global) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let parsed: Value = serde_json::from_str(&text).context("global settings.json is not JSON")?;
    let Some(packages) = parsed.get("packages").filter(|p| !p.is_null()).cloned() else {
        return Ok(false);
    };
    let mut settings = read_json_object(local);
    let obj = settings.as_object_mut().expect("object");
    if obj.contains_key("packages") {
        return Ok(false);
    }
    obj.insert("packages".to_string(), packages);
    write_json_atomic(local, &settings)?;
    Ok(true)
}

/// Copy every regular file under `from` into `to` that `to` lacks. Returns
/// how many files were copied; failures are appended to `errors`.
fn copy_tree_missing(from: &Path, to: &Path, errors: &mut Vec<String>) -> usize {
    let mut copied = 0;
    if let Err(e) = std::fs::create_dir_all(to) {
        errors.push(format!("{}: {e}", to.display()));
        return 0;
    }
    let entries = match std::fs::read_dir(from) {
        Ok(entries) => entries,
        Err(e) => {
            errors.push(format!("{}: {e}", from.display()));
            return 0;
        }
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if kind.is_dir() {
            copied += copy_tree_missing(&src, &dst, errors);
        } else if kind.is_file() && !dst.exists() {
            match std::fs::copy(&src, &dst) {
                Ok(_) => copied += 1,
                Err(e) => errors.push(format!("{}: {e}", src.display())),
            }
        }
        // Symlinks are skipped so the copy cannot leave the tree.
    }
    copied
}

/// Read a JSON object from `path`; anything else (missing, unreadable, not
/// an object) yields an empty object.
pub fn read_json_object(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

/// Write pretty JSON through a uniquely named temporary file and a rename, so
/// readers never see a half-written file and concurrent writers do not
/// clobber each other's temporary files.
pub fn write_json_atomic(path: &Path, value: &Value) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    write_bytes_atomic(path, text.as_bytes())
}

/// [`write_json_atomic`] for raw bytes.
pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent", path.display()))?;
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("replace {}", path.display()));
    }
    Ok(())
}

/// Owner-only permissions for files holding credentials (no-op on Windows,
/// where the profile directory is already private).
pub fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Record the bash Pi should use (`shellPath` in its `settings.json`), so Pi
/// does not pick a WSL launcher or an app alias on Windows.
pub fn write_shell_path(config_dir: &Path, bash_exe: &Path) -> Result<()> {
    let path = config_dir.join("settings.json");
    let mut settings = read_json_object(&path);
    let wanted = json!(bash_exe.to_string_lossy());
    if settings.get("shellPath") == Some(&wanted) {
        return Ok(());
    }
    settings
        .as_object_mut()
        .expect("object")
        .insert("shellPath".to_string(), wanted);
    write_json_atomic(&path, &settings)?;
    info!("pi config: shellPath set to {}", bash_exe.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Child environment
// ---------------------------------------------------------------------------

/// Environment every Pi process gets:
///
/// * `PI_CODING_AGENT_DIR` (read by Pi) and `SCREENPIPE_PI_AGENT_DIR` (read by
///   bundled extensions) name the isolated config directory;
/// * `PI_SKIP_VERSION_CHECK=1`: the runtime pins Pi, so Pi's own update
///   check would only ever nag;
/// * `JITI_TRY_NATIVE=0`: extensions are compiled by jiti rather than loaded
///   through the runtime's native TypeScript support, which differs between
///   bun releases;
/// * `PI_TELEMETRY=0`: Pi's install telemetry stays off.
pub fn child_env(config_dir: &Path) -> Vec<(&'static str, String)> {
    let dir = config_dir.to_string_lossy().into_owned();
    vec![
        (env::PI_CODING_AGENT_DIR, dir.clone()),
        (env::SCREENPIPE_PI_AGENT_DIR, dir),
        (env::PI_SKIP_VERSION_CHECK, "1".to_string()),
        (env::JITI_TRY_NATIVE, "0".to_string()),
        (env::PI_TELEMETRY, "0".to_string()),
    ]
}

/// Apply [`child_env`] to a std command.
pub fn apply_to_command(cmd: &mut std::process::Command, config_dir: &Path) {
    for (name, value) in child_env(config_dir) {
        cmd.env(name, value);
    }
}

/// Apply [`child_env`] to a tokio command.
pub fn apply_to_tokio_command(cmd: &mut tokio::process::Command, config_dir: &Path) {
    for (name, value) in child_env(config_dir) {
        cmd.env(name, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn a_fresh_install_dir_gets_the_pins_as_dependencies_and_overrides() {
        let dir = tmp();
        assert_eq!(seed_package_json(dir.path()).unwrap(), SeedOutcome::Created);
        let pkg = read_json_object(&dir.path().join("package.json"));
        assert_eq!(pkg["dependencies"][PI_CODING_AGENT], "0.84.1");
        assert_eq!(pkg["dependencies"][PI_AI], "0.84.1");
        assert_eq!(pkg["dependencies"][ANTHROPIC_SDK], ANTHROPIC_SDK_VERSION);
        for name in PI_FAMILY {
            assert_eq!(pkg["overrides"][*name], "0.84.1", "{name}");
        }
        assert_eq!(pkg["overrides"][ANTHROPIC_SDK], ANTHROPIC_SDK_VERSION);
        assert_eq!(
            pkg["overrides"]["hosted-git-info"]["lru-cache"],
            LRU_CACHE_FOR_HOSTED_GIT_INFO
        );
        // A second seed changes nothing.
        assert_eq!(
            seed_package_json(dir.path()).unwrap(),
            SeedOutcome::Unchanged
        );
    }

    #[test]
    fn an_old_manifest_is_upgraded_and_its_lockfiles_dropped() {
        let dir = tmp();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"dependencies":{"@earendil-works/pi-coding-agent":"0.75.4","@mariozechner/pi-ai":"0.50.0","left-pad":"1.0.0"},"overrides":{"hosted-git-info":{"lru-cache":"^10.0.0"},"custom":"1"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("bun.lock"), "{}").unwrap();
        assert_eq!(seed_package_json(dir.path()).unwrap(), SeedOutcome::Updated);
        let pkg = read_json_object(&dir.path().join("package.json"));
        assert_eq!(pkg["dependencies"][PI_CODING_AGENT], "0.84.1");
        assert!(pkg["dependencies"].get("@mariozechner/pi-ai").is_none());
        assert_eq!(pkg["dependencies"]["left-pad"], "1.0.0");
        assert_eq!(pkg["overrides"]["custom"], "1");
        assert_eq!(pkg["overrides"]["@earendil-works/pi-agent-core"], "0.84.1");
        assert!(!dir.path().join("bun.lock").exists());
    }

    #[test]
    fn a_corrupt_manifest_is_replaced() {
        let dir = tmp();
        std::fs::write(dir.path().join("package.json"), b"{\"dependencies\":\0\0\0").unwrap();
        std::fs::write(dir.path().join("bun.lockb"), b"x").unwrap();
        assert_eq!(
            seed_package_json(dir.path()).unwrap(),
            SeedOutcome::Reseeded
        );
        assert_eq!(
            read_json_object(&dir.path().join("package.json")),
            expected_package_json()
        );
        assert!(!dir.path().join("bun.lockb").exists());

        std::fs::write(dir.path().join("package.json"), "[1,2]").unwrap();
        assert_eq!(
            seed_package_json(dir.path()).unwrap(),
            SeedOutcome::Reseeded
        );
    }

    #[test]
    fn the_install_command_pins_versions_and_skips_scripts() {
        let args = bun_install_args();
        assert_eq!(args[0], "add");
        assert!(args.contains(&"@earendil-works/pi-coding-agent@0.84.1".to_string()));
        assert!(args.contains(&"@earendil-works/pi-ai@0.84.1".to_string()));
        assert!(args.contains(&format!("@anthropic-ai/sdk@{ANTHROPIC_SDK_VERSION}")));
        assert!(args.contains(&"--force".to_string()));
        assert!(args.contains(&"--ignore-scripts".to_string()));
        assert!(npm_install_args().contains(&"--ignore-scripts"));
    }

    #[test]
    fn installed_version_is_read_from_the_package() {
        let dir = tmp();
        assert!(!is_installed_version_current(dir.path()));
        let pkg = package_dir(dir.path());
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("package.json"), r#"{"version":"0.75.4"}"#).unwrap();
        assert!(!is_installed_version_current(dir.path()));
        std::fs::write(pkg.join("package.json"), r#"{"version":"0.84.1"}"#).unwrap();
        assert!(is_installed_version_current(dir.path()));
        assert!(cli_entrypoint(dir.path()).ends_with("pi-coding-agent/dist/cli.js"));
    }

    #[test]
    fn the_config_dir_defaults_under_the_data_dir() {
        let data = Path::new("/data");
        assert_eq!(
            resolve_config_dir(None, data),
            data.join("pi-agent").join("pi-config")
        );
        assert_eq!(
            resolve_config_dir(Some("  "), data),
            data.join("pi-agent").join("pi-config")
        );
        assert_eq!(
            resolve_config_dir(Some("/elsewhere/pi"), data),
            PathBuf::from("/elsewhere/pi")
        );
    }

    #[test]
    fn the_global_dir_follows_pi_and_never_points_at_the_isolated_one() {
        let home = Path::new("/home/u");
        let isolated = Path::new("/data/pi-agent/pi-config");
        assert_eq!(
            resolve_global_config_dir(None, Some(home), isolated),
            Some(home.join(".pi").join("agent"))
        );
        assert_eq!(
            resolve_global_config_dir(Some("/custom/pi"), Some(home), isolated),
            Some(PathBuf::from("/custom/pi"))
        );
        assert_eq!(
            resolve_global_config_dir(Some("/data/pi-agent/pi-config"), Some(home), isolated),
            Some(home.join(".pi").join("agent"))
        );
        assert_eq!(resolve_global_config_dir(None, None, isolated), None);
    }

    #[test]
    fn a_fresh_config_dir_without_a_global_config_still_gets_the_marker() {
        let dir = tmp();
        let config = dir.path().join("pi-agent").join("pi-config");
        let missing = dir.path().join("no-such-global");
        let report = seed_from_global(&config, Some(&missing)).unwrap();
        assert_eq!(
            report,
            SeedReport::Seeded {
                copied: vec![],
                failed: vec![]
            }
        );
        assert!(config.join(MIGRATED_MARKER).exists());
        assert!(!config.join(MIGRATION_LOCK).exists());
        assert_eq!(
            seed_from_global(&config, Some(&missing)).unwrap(),
            SeedReport::AlreadySeeded
        );
    }

    #[test]
    fn seeding_copies_the_listed_items_and_nothing_else() {
        let dir = tmp();
        let global = dir.path().join("global");
        std::fs::create_dir_all(global.join("sessions").join("--proj--")).unwrap();
        std::fs::write(global.join("auth.json"), r#"{"anthropic":{"key":"k"}}"#).unwrap();
        std::fs::write(global.join("trust.json"), r#"{"trusted":["/p"]}"#).unwrap();
        std::fs::write(
            global.join("settings.json"),
            r#"{"packages":["npm:pi-foo"],"defaultModel":"x","shellPath":"/bin/zsh"}"#,
        )
        .unwrap();
        std::fs::write(global.join("models.json"), r#"{"providers":{}}"#).unwrap();
        std::fs::write(global.join("sessions").join("--proj--").join("a.jsonl"), "{}\n").unwrap();

        let config = dir.path().join("pi-config");
        let report = seed_from_global(&config, Some(&global)).unwrap();
        let SeedReport::Seeded { copied, failed } = report else {
            panic!("expected seeding");
        };
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(
            copied,
            vec![
                "auth.json".to_string(),
                "trust.json".to_string(),
                "settings.json#packages".to_string(),
                "sessions (1 files)".to_string()
            ]
        );
        assert!(config.join("sessions/--proj--/a.jsonl").exists());
        assert!(!config.join("models.json").exists());
        let settings = read_json_object(&config.join("settings.json"));
        assert_eq!(settings, json!({"packages": ["npm:pi-foo"]}));
        let marker = read_json_object(&config.join(MIGRATED_MARKER));
        assert_eq!(marker["source"], global.to_string_lossy().as_ref());
    }

    #[test]
    fn seeding_never_overwrites_the_isolated_dir() {
        let dir = tmp();
        let global = dir.path().join("global");
        std::fs::create_dir_all(global.join("sessions")).unwrap();
        std::fs::write(global.join("auth.json"), r#"{"g":1}"#).unwrap();
        std::fs::write(global.join("settings.json"), r#"{"packages":["npm:g"]}"#).unwrap();
        std::fs::write(global.join("sessions").join("s.jsonl"), "global").unwrap();

        let config = dir.path().join("pi-config");
        std::fs::create_dir_all(config.join("sessions")).unwrap();
        std::fs::write(config.join("auth.json"), r#"{"local":1}"#).unwrap();
        std::fs::write(config.join("settings.json"), r#"{"packages":[],"shellPath":"b"}"#).unwrap();
        std::fs::write(config.join("sessions").join("s.jsonl"), "local").unwrap();

        seed_from_global(&config, Some(&global)).unwrap();
        assert_eq!(std::fs::read_to_string(config.join("auth.json")).unwrap(), r#"{"local":1}"#);
        assert_eq!(
            read_json_object(&config.join("settings.json")),
            json!({"packages": [], "shellPath": "b"})
        );
        assert_eq!(
            std::fs::read_to_string(config.join("sessions/s.jsonl")).unwrap(),
            "local"
        );
    }

    #[test]
    fn a_held_lock_makes_a_second_seeder_wait_then_give_up() {
        let dir = tmp();
        let config = dir.path().join("pi-config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join(MIGRATION_LOCK), "123").unwrap();
        // Fresh lock, no marker: the second process gives up after the wait.
        let wait = Duration::from_millis(300);
        let started = std::time::Instant::now();
        assert_eq!(
            seed_from_global_waiting(&config, None, wait).unwrap(),
            SeedReport::Busy
        );
        assert!(started.elapsed() >= wait);
        assert!(!config.join(MIGRATED_MARKER).exists());
        // The lock belongs to someone else and is left alone.
        assert!(config.join(MIGRATION_LOCK).exists());
    }

    #[test]
    fn a_stale_lock_is_taken_over() {
        let dir = tmp();
        let config = dir.path().join("pi-config");
        std::fs::create_dir_all(&config).unwrap();
        let lock = config.join(MIGRATION_LOCK);
        std::fs::write(&lock, "123").unwrap();
        let old = SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&lock)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(matches!(
            seed_from_global(&config, None).unwrap(),
            SeedReport::Seeded { .. }
        ));
        assert!(config.join(MIGRATED_MARKER).exists());
        assert!(!lock.exists());
    }

    #[test]
    fn concurrent_seeders_seed_exactly_once() {
        let dir = tmp();
        let global = dir.path().join("global");
        std::fs::create_dir_all(&global).unwrap();
        std::fs::write(global.join("auth.json"), "{}").unwrap();
        let config = dir.path().join("pi-config");
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let config = config.clone();
                let global = global.clone();
                std::thread::spawn(move || seed_from_global(&config, Some(&global)).unwrap())
            })
            .collect();
        let reports: Vec<SeedReport> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let seeded = reports
            .iter()
            .filter(|r| matches!(r, SeedReport::Seeded { .. }))
            .count();
        assert_eq!(seeded, 1, "{reports:?}");
        assert!(config.join(MIGRATED_MARKER).exists());
    }

    #[test]
    fn shell_path_is_merged_into_settings() {
        let dir = tmp();
        std::fs::write(dir.path().join("settings.json"), r#"{"packages":["a"]}"#).unwrap();
        write_shell_path(dir.path(), Path::new("C:/Git/bin/bash.exe")).unwrap();
        assert_eq!(
            read_json_object(&dir.path().join("settings.json")),
            json!({"packages": ["a"], "shellPath": "C:/Git/bin/bash.exe"})
        );
    }

    #[test]
    fn child_env_names_the_isolated_dir_and_turns_off_pi_network_extras() {
        let env: std::collections::HashMap<_, _> =
            child_env(Path::new("/d/pi-agent/pi-config")).into_iter().collect();
        assert_eq!(env["PI_CODING_AGENT_DIR"], "/d/pi-agent/pi-config");
        assert_eq!(env["SCREENPIPE_PI_AGENT_DIR"], "/d/pi-agent/pi-config");
        assert_eq!(env["PI_SKIP_VERSION_CHECK"], "1");
        assert_eq!(env["JITI_TRY_NATIVE"], "0");
        assert_eq!(env["PI_TELEMETRY"], "0");
    }
}
