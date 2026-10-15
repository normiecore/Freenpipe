// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Where the command-line runtime that ships with the desktop app lives, and
//! how agent processes get to it.
//!
//! Freenpipe never asks a package registry for "the newest CLI" or "the
//! newest MCP server" at run time. Both are built from this repository and
//! installed with the app:
//!
//! * the `screenpipe` CLI binary sits in the install root next to the main
//!   executable (and its runtime DLLs on Windows);
//! * the MCP server is a single-file JavaScript build of
//!   `packages/screenpipe-mcp` at `mcp/screenpipe-mcp.js`, started with the
//!   bundled `bun`.
//!
//! Anything that needs the CLI gets an absolute path. Agent processes receive
//! it three ways, so every kind of tool call works:
//!
//! 1. `SCREENPIPE_CLI` in the environment;
//! 2. a launcher directory (`<data dir>/pi-agent/bin`) prepended to `PATH`,
//!    holding a `screenpipe` script (plus `screenpipe.cmd` on Windows) that
//!    forwards to the bundled binary;
//! 3. a `screenpipe` shell function in the bash startup shim (see
//!    [`super::bash_env`]), which wins over any other `screenpipe` on `PATH`.
//!
//! There is deliberately no `PATH` search for the CLI itself: a `screenpipe`
//! found on `PATH` could be an unrelated install, and running it from an
//! agent would mix two products' data handling.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::{debug, warn};

/// Environment variable carrying the absolute path of the CLI binary.
pub const CLI_ENV: &str = "SCREENPIPE_CLI";

/// Launcher directory, relative to the data dir.
pub const SHIM_RELATIVE_DIR: &str = "pi-agent/bin";

/// MCP server entry point, relative to an install root.
pub const MCP_ENTRY_RELATIVE: &str = "mcp/screenpipe-mcp.js";

/// Product directory names the Linux packages install resources under
/// (`/usr/lib/<name>/`).
const LINUX_RESOURCE_DIR_NAMES: [&str; 2] = ["Freenpipe", "freenpipe"];

/// File name of the CLI binary on this platform.
pub fn cli_file_name() -> &'static str {
    if cfg!(windows) {
        "screenpipe.exe"
    } else {
        "screenpipe"
    }
}

/// File name of the bun binary on this platform.
pub fn bun_file_name() -> &'static str {
    if cfg!(windows) {
        "bun.exe"
    } else {
        "bun"
    }
}

/// Directories that may hold files installed with the app, derived from the
/// path of the running executable, in lookup order: the executable's own
/// directory (Windows install root, macOS `Contents/MacOS`, Cargo's target
/// dir during development), the macOS `Contents/Resources` directory, and the
/// Linux `/usr/lib/<product>` resource directory.
pub fn install_roots(exe: &Path) -> Vec<PathBuf> {
    let Some(exe_dir) = exe.parent() else {
        return Vec::new();
    };
    let mut roots = vec![exe_dir.to_path_buf()];
    if let Some(parent) = exe_dir.parent() {
        roots.push(parent.join("Resources"));
        for name in LINUX_RESOURCE_DIR_NAMES {
            roots.push(parent.join("lib").join(name));
        }
    }
    roots
}

fn current_install_roots() -> Vec<PathBuf> {
    std::env::current_exe()
        .map(|exe| install_roots(&exe))
        .unwrap_or_default()
}

/// Pure CLI lookup: an absolute, existing `SCREENPIPE_CLI` wins; otherwise the
/// first install root that contains the CLI binary.
pub fn resolve_cli_with(
    env_value: Option<&OsStr>,
    roots: &[PathBuf],
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if let Some(raw) = env_value.filter(|v| !v.is_empty()) {
        let candidate = PathBuf::from(raw);
        if candidate.is_absolute() && is_file(&candidate) {
            return Some(candidate);
        }
        warn!(
            "{CLI_ENV}={} is not an absolute path to an existing file; ignoring it",
            candidate.display()
        );
    }
    roots
        .iter()
        .map(|root| root.join(cli_file_name()))
        .find(|candidate| is_file(candidate))
}

/// The CLI binary that ships with this install, if any.
pub fn bundled_cli() -> Option<PathBuf> {
    let env_value = std::env::var_os(CLI_ENV);
    resolve_cli_with(env_value.as_deref(), &current_install_roots(), |p| {
        p.is_file()
    })
}

/// Pure lookup of a file shipped with the app: the first root that contains
/// `relative`.
pub fn resolve_bundled_file_with(
    relative: &Path,
    roots: &[PathBuf],
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    roots
        .iter()
        .map(|root| root.join(relative))
        .find(|candidate| is_file(candidate))
}

/// The bundled MCP server entry point. `extra_roots` are searched after the
/// install roots (the desktop app passes its development staging directory).
pub fn bundled_mcp_entry(extra_roots: &[PathBuf]) -> Option<PathBuf> {
    let mut roots = current_install_roots();
    roots.extend_from_slice(extra_roots);
    resolve_bundled_file_with(Path::new(MCP_ENTRY_RELATIVE), &roots, |p| p.is_file())
}

/// Single-quote a string for POSIX sh.
fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Body of the POSIX launcher. On Windows the target is written with forward
/// slashes, which the MSYS/Git bash shipped for agents accepts as a native
/// path.
pub fn posix_shim(cli: &Path, windows_style: bool) -> String {
    let mut target = cli.to_string_lossy().into_owned();
    if windows_style {
        target = target.replace('\\', "/");
    }
    format!(
        "#!/bin/sh\n\
         # Freenpipe CLI launcher, regenerated when an agent starts. Forwards to\n\
         # the screenpipe binary installed with the desktop app.\n\
         exec {} \"$@\"\n",
        sh_quote(&target)
    )
}

/// Body of the Windows `cmd` launcher.
pub fn cmd_shim(cli: &Path) -> String {
    // `%` would start a variable expansion inside a batch file.
    let target = cli.to_string_lossy().replace('%', "%%");
    format!(
        "@echo off\r\n\
         rem Freenpipe CLI launcher, regenerated when an agent starts.\r\n\
         \"{target}\" %*\r\n\
         exit /b %ERRORLEVEL%\r\n"
    )
}

/// Replace `path` with `content` unless it already holds exactly that. The
/// new content goes to a temporary file in the same directory that is then
/// renamed over `path`, so an agent starting at the same moment runs either
/// the old launcher or the new one, never a half-written file.
fn write_if_changed(path: &Path, content: &str, executable: bool) -> Result<bool> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(false);
    }
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let result = (|| -> Result<()> {
        std::fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
        #[cfg(unix)]
        if executable {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        #[cfg(not(unix))]
        let _ = executable;
        // On Windows a scanner or another agent can hold the old launcher
        // open for a moment; retry briefly before giving up.
        let mut renamed = std::fs::rename(&tmp, path);
        for _ in 0..4 {
            if renamed.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
            renamed = std::fs::rename(&tmp, path);
        }
        renamed.with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    })();
    if let Err(err) = result {
        let _ = std::fs::remove_file(&tmp);
        // Another process may have written the same launcher first (on
        // Windows a file in use cannot be replaced); that is success too.
        if std::fs::read_to_string(path).ok().as_deref() == Some(content) {
            return Ok(false);
        }
        return Err(err);
    }
    Ok(true)
}

/// Write the launchers for `cli` into `<data_dir>/pi-agent/bin` and return
/// that directory. Files are only rewritten when their content changes.
pub fn ensure_shims(data_dir: &Path, cli: &Path) -> Result<PathBuf> {
    let dir = data_dir.join(SHIM_RELATIVE_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let posix = dir.join("screenpipe");
    write_if_changed(&posix, &posix_shim(cli, cfg!(windows)), true)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&posix)?.permissions();
        if perms.mode() & 0o111 != 0o111 {
            perms.set_mode(0o755);
            std::fs::set_permissions(&posix, perms)?;
        }
    }
    if cfg!(windows) {
        write_if_changed(&dir.join("screenpipe.cmd"), &cmd_shim(cli), false)?;
    }
    Ok(dir)
}

/// `dir` followed by the entries of `current` (without duplicates of `dir`).
/// Returns `current` unchanged when `dir` cannot be represented in a `PATH`
/// (for example because it contains the separator character).
pub fn prepend_to_path(dir: &Path, current: Option<&OsStr>) -> Option<OsString> {
    let rest: Vec<PathBuf> = current
        .map(|c| std::env::split_paths(c).filter(|p| p != dir).collect())
        .unwrap_or_default();
    let joined = std::iter::once(dir.to_path_buf()).chain(rest);
    match std::env::join_paths(joined) {
        Ok(path) => Some(path),
        Err(e) => {
            warn!("cannot add {} to PATH: {e}", dir.display());
            current.map(OsStr::to_os_string)
        }
    }
}

/// Environment an agent process needs to reach the bundled CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCliEnv {
    /// Value for `SCREENPIPE_CLI`.
    pub cli: PathBuf,
    /// New `PATH`, with the launcher directory first. `None` when the
    /// launchers could not be written; `SCREENPIPE_CLI` is still exported.
    pub path: Option<OsString>,
}

/// Build the agent environment for `cli`, writing launchers under
/// `data_dir`. `current_path` is the `PATH` the child would otherwise get.
pub fn agent_cli_env_for(
    cli: PathBuf,
    data_dir: &Path,
    current_path: Option<&OsStr>,
) -> AgentCliEnv {
    let path = match ensure_shims(data_dir, &cli) {
        Ok(dir) => prepend_to_path(&dir, current_path),
        Err(e) => {
            warn!("could not write the screenpipe CLI launcher: {e:#}");
            None
        }
    };
    AgentCliEnv { cli, path }
}

/// Agent environment for this install, or `None` when no CLI is bundled
/// (development builds without a staged CLI). Callers then skip the
/// variables; agents fall back to the local REST API.
pub fn agent_cli_env(current_path: Option<&OsStr>) -> Option<AgentCliEnv> {
    let Some(cli) = bundled_cli() else {
        debug!("no bundled screenpipe CLI found; agents will not get {CLI_ENV}");
        return None;
    };
    let data_dir = crate::paths::default_screenpipe_data_dir();
    Some(agent_cli_env_for(cli, &data_dir, current_path))
}

/// `PATH` a command will pass to its child: its own override when it set
/// one, otherwise the inherited value.
fn effective_path<'a>(
    mut overrides: impl Iterator<Item = (&'a OsStr, Option<&'a OsStr>)>,
) -> Option<OsString> {
    match overrides.find(|(key, _)| key.eq_ignore_ascii_case("PATH")) {
        Some((_, value)) => value.map(OsStr::to_os_string),
        None => std::env::var_os("PATH"),
    }
}

/// Export `SCREENPIPE_CLI` and the launcher `PATH` on a std command. Call it
/// after any other code that sets `PATH` on the same command.
pub fn apply_to_command(cmd: &mut std::process::Command) {
    let current = effective_path(cmd.get_envs());
    if let Some(env) = agent_cli_env(current.as_deref()) {
        cmd.env(CLI_ENV, &env.cli);
        if let Some(path) = env.path {
            cmd.env("PATH", path);
        }
    }
}

/// Same as [`apply_to_command`] for a tokio command.
pub fn apply_to_tokio_command(cmd: &mut tokio::process::Command) {
    let current = effective_path(cmd.as_std().get_envs());
    if let Some(env) = agent_cli_env(current.as_deref()) {
        cmd.env(CLI_ENV, &env.cli);
        if let Some(path) = env.path {
            cmd.env("PATH", path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn files<P: AsRef<Path>>(paths: &[P]) -> impl Fn(&Path) -> bool {
        let set: HashSet<PathBuf> = paths.iter().map(|p| p.as_ref().to_path_buf()).collect();
        move |p: &Path| set.contains(p)
    }

    fn abs(parts: &[&str]) -> PathBuf {
        let mut p = if cfg!(windows) {
            PathBuf::from(r"C:\")
        } else {
            PathBuf::from("/")
        };
        for part in parts {
            p.push(part);
        }
        p
    }

    #[test]
    fn install_roots_cover_each_platform_layout() {
        let exe = abs(&["opt", "Freenpipe", "bin", "freenpipe-app"]);
        let roots = install_roots(&exe);
        assert_eq!(roots[0], abs(&["opt", "Freenpipe", "bin"]));
        assert!(roots.contains(&abs(&["opt", "Freenpipe", "Resources"])));
        assert!(roots.contains(&abs(&["opt", "Freenpipe", "lib", "Freenpipe"])));
    }

    #[test]
    fn cli_next_to_the_executable_is_found() {
        let root = abs(&["apps", "Freenpipe"]);
        let cli = root.join(cli_file_name());
        let found = resolve_cli_with(None, &[root.clone()], files(&[&cli]));
        assert_eq!(found, Some(cli));
    }

    #[test]
    fn env_override_wins_when_it_points_at_a_file() {
        let root = abs(&["apps", "Freenpipe"]);
        let bundled = root.join(cli_file_name());
        let custom = abs(&["dev", "target", "release", cli_file_name()]);
        let found = resolve_cli_with(
            Some(custom.as_os_str()),
            &[root],
            files(&[&bundled, &custom]),
        );
        assert_eq!(found, Some(custom));
    }

    #[test]
    fn unusable_env_override_falls_back_to_the_bundled_cli() {
        let root = abs(&["apps", "Freenpipe"]);
        let bundled = root.join(cli_file_name());
        for bad in ["relative/screenpipe", ""] {
            let found =
                resolve_cli_with(Some(OsStr::new(bad)), &[root.clone()], files(&[&bundled]));
            assert_eq!(found.as_deref(), Some(bundled.as_path()), "{bad:?}");
        }
        let missing = abs(&["nowhere", cli_file_name()]);
        let found = resolve_cli_with(Some(missing.as_os_str()), &[root], files(&[&bundled]));
        assert_eq!(found, Some(bundled));
    }

    #[test]
    fn no_cli_means_none_not_a_registry_fallback() {
        assert_eq!(resolve_cli_with(None, &[abs(&["empty"])], |_| false), None);
    }

    #[test]
    fn mcp_entry_is_found_in_the_first_root_that_has_it() {
        let a = abs(&["a"]);
        let b = abs(&["b"]);
        let entry_b = b.join(MCP_ENTRY_RELATIVE);
        let found =
            resolve_bundled_file_with(Path::new(MCP_ENTRY_RELATIVE), &[a, b], files(&[&entry_b]));
        assert_eq!(found, Some(entry_b));
    }

    #[test]
    fn posix_shim_quotes_the_target_and_forwards_arguments() {
        let shim = posix_shim(Path::new("/opt/it's here/screenpipe"), false);
        assert!(shim.starts_with("#!/bin/sh\n"));
        assert!(shim.contains(r"exec '/opt/it'\''s here/screenpipe' "));
        assert!(shim.trim_end().ends_with("\"$@\""));
    }

    #[test]
    fn posix_shim_uses_forward_slashes_for_windows_paths() {
        let shim = posix_shim(
            Path::new(r"C:\Program Files\Freenpipe\screenpipe.exe"),
            true,
        );
        assert!(shim.contains("'C:/Program Files/Freenpipe/screenpipe.exe'"));
        assert!(!shim.contains('\\'));
    }

    #[test]
    fn cmd_shim_quotes_and_escapes_percent() {
        let shim = cmd_shim(Path::new(r"C:\Users\a%b\Freenpipe\screenpipe.exe"));
        assert!(shim.starts_with("@echo off\r\n"));
        assert!(shim.contains("\"C:\\Users\\a%%b\\Freenpipe\\screenpipe.exe\" %*"));
        assert!(shim.contains("exit /b %ERRORLEVEL%"));
    }

    #[test]
    fn prepend_to_path_puts_the_dir_first_once() {
        let dir = abs(&["data", "pi-agent", "bin"]);
        let other = abs(&["usr", "bin"]);
        let current = std::env::join_paths([other.clone(), dir.clone()]).unwrap();
        let joined = prepend_to_path(&dir, Some(&current)).unwrap();
        let parts: Vec<PathBuf> = std::env::split_paths(&joined).collect();
        assert_eq!(parts, vec![dir.clone(), other]);
        let alone = prepend_to_path(&dir, None).unwrap();
        assert_eq!(std::env::split_paths(&alone).collect::<Vec<_>>(), vec![dir]);
    }

    #[test]
    fn ensure_shims_writes_launchers_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let cli = tmp.path().join("install").join(cli_file_name());
        let dir = ensure_shims(tmp.path(), &cli).unwrap();
        assert_eq!(dir, tmp.path().join(SHIM_RELATIVE_DIR));
        let posix = dir.join("screenpipe");
        let first = std::fs::read_to_string(&posix).unwrap();
        assert_eq!(first, posix_shim(&cli, cfg!(windows)));
        assert_eq!(dir.join("screenpipe.cmd").exists(), cfg!(windows));
        let mtime = std::fs::metadata(&posix).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        ensure_shims(tmp.path(), &cli).unwrap();
        assert_eq!(
            std::fs::metadata(&posix).unwrap().modified().unwrap(),
            mtime,
            "unchanged launcher must not be rewritten"
        );
    }

    #[test]
    fn stale_launchers_are_replaced_whole_and_leave_no_temp_files() {
        let tmp = tempfile::tempdir().unwrap();
        let cli = tmp.path().join("install").join(cli_file_name());
        let dir = tmp.path().join(SHIM_RELATIVE_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("screenpipe"),
            "#!/bin/sh\nexec /old/screenpipe \"$@\"\n",
        )
        .unwrap();

        // Agents starting together all get a complete launcher.
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (data, cli) = (tmp.path().to_path_buf(), cli.clone());
                std::thread::spawn(move || ensure_shims(&data, &cli).unwrap())
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        let posix = dir.join("screenpipe");
        assert_eq!(
            std::fs::read_to_string(&posix).unwrap(),
            posix_shim(&cli, cfg!(windows))
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&posix).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "launcher must be executable");
        }
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut expected = vec!["screenpipe".to_string()];
        if cfg!(windows) {
            expected.push("screenpipe.cmd".to_string());
        }
        assert_eq!(names, expected);
    }

    #[test]
    fn agent_env_exports_cli_and_prefixes_path() {
        let tmp = tempfile::tempdir().unwrap();
        let cli = tmp.path().join(cli_file_name());
        let inherited = std::env::join_paths([abs(&["usr", "bin"])]).unwrap();
        let env = agent_cli_env_for(cli.clone(), tmp.path(), Some(&inherited));
        assert_eq!(env.cli, cli);
        let path = env.path.expect("launcher dir should be on PATH");
        let first = std::env::split_paths(&path).next().unwrap();
        assert_eq!(first, tmp.path().join(SHIM_RELATIVE_DIR));
    }

    #[test]
    fn effective_path_prefers_the_command_override() {
        let mut cmd = std::process::Command::new("true");
        cmd.env("PATH", "/only/here");
        assert_eq!(
            effective_path(cmd.get_envs()).as_deref(),
            Some(OsStr::new("/only/here"))
        );
        let mut removed = std::process::Command::new("true");
        removed.env_remove("PATH");
        assert_eq!(effective_path(removed.get_envs()), None);
    }

    /// The POSIX launcher really runs the target with the caller's arguments.
    #[test]
    #[cfg(unix)]
    fn posix_launcher_forwards_arguments_end_to_end() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let fake_cli = tmp.path().join("fake cli");
        std::fs::write(&fake_cli, "#!/bin/sh\nprintf '%s|' \"$@\"\n").unwrap();
        std::fs::set_permissions(&fake_cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = agent_cli_env_for(
            fake_cli.clone(),
            tmp.path(),
            Some(OsStr::new("/usr/bin:/bin")),
        );
        let out = std::process::Command::new("sh")
            .env("PATH", env.path.unwrap())
            .arg("-c")
            .arg("screenpipe pipe list 'two words'")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "pipe|list|two words|");
    }
}
