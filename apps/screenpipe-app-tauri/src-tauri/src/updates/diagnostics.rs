// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Local updater diagnostics.
//!
//! Three files in the app's data directory (`~/.screenpipe`, or
//! `SCREENPIPE_DATA_DIR`) record what happened around an in-app update:
//!
//! - `update-attempt.json`: a marker written just before the app hands over
//!   to the installer (Windows) or restarts into the staged bundle
//!   (macOS/Linux). The next launch compares it with the running version
//!   and stamps an outcome on it.
//! - `update-install.log`: one line per normal step (attempt, restart
//!   committed, relaunch observed, leftover cleanup).
//! - `update-recovery.log`: one line per failure (download or install
//!   failed, update did not apply, attempt abandoned).
//!
//! Nothing here leaves the machine. Every write refuses to go through a
//! symbolic link, so a link planted at one of these names cannot redirect the
//! write elsewhere.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

pub const ATTEMPT_MARKER_FILE: &str = "update-attempt.json";
pub const INSTALL_LOG_FILE: &str = "update-install.log";
pub const RECOVERY_LOG_FILE: &str = "update-recovery.log";

/// Logs are trimmed to their newest half once they grow past this size.
const MAX_LOG_BYTES: u64 = 256 * 1024;

/// What one update attempt looked like when it started, plus what the next
/// launch observed. Every field has a default so a marker written by another
/// build (older, newer, or upstream) still loads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateAttempt {
    /// Version that was running when the attempt started.
    pub from_version: String,
    /// Version being installed.
    pub to_version: String,
    /// Updater platform key, e.g. `windows-x86_64`.
    pub target: String,
    /// Path of the executable that started the attempt.
    pub executable: String,
    /// Whether the Home window was visible when the attempt started.
    pub home_visible: bool,
    /// Attempt start time, RFC 3339 (UTC).
    pub attempt_ts: String,
    /// Attempt start time, seconds since the Unix epoch.
    pub ts_epoch_secs: u64,
    /// True once the app actually handed over to the installer / restarted.
    pub restart_committed: bool,
    /// Set by the next launch: see [`Outcome`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Set by the next launch: the version that was running then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running: Option<String>,
}

impl UpdateAttempt {
    /// A fresh attempt stamped with the current time.
    pub fn new(
        from_version: impl Into<String>,
        to_version: impl Into<String>,
        target: impl Into<String>,
        executable: impl Into<String>,
        home_visible: bool,
    ) -> Self {
        let now = chrono::Utc::now();
        Self {
            from_version: from_version.into(),
            to_version: to_version.into(),
            target: target.into(),
            executable: executable.into(),
            home_visible,
            attempt_ts: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ts_epoch_secs: now.timestamp().max(0) as u64,
            restart_committed: false,
            outcome: None,
            running: None,
        }
    }
}

/// How an attempt ended, as judged on the next launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The app came back up on the version it tried to install.
    RelaunchObserved,
    /// The app restarted for the update but is still on another version.
    NotApplied,
    /// The attempt never reached the restart (crash, kill, or failure before
    /// the handover).
    Abandoned,
    /// Download or install failed in the same session (recorded right away).
    InstallFailed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::RelaunchObserved => "relaunch_observed",
            Outcome::NotApplied => "not_applied",
            Outcome::Abandoned => "abandoned",
            Outcome::InstallFailed => "install_failed",
        }
    }
}

fn symlink_error(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "updater diagnostic path is a symlink, refusing to use it: {}",
            path.display()
        ),
    )
}

/// Fail if `path` exists and is a symbolic link (the link itself, not its
/// target, is inspected). A missing path is fine.
pub fn refuse_symlink(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(symlink_error(path)),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Keep a log line on one line and bounded, whatever the caller passes in.
fn sanitize_line(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = flat.trim();
    if trimmed.chars().count() > 1000 {
        let cut: String = trimmed.chars().take(1000).collect();
        format!("{cut}...")
    } else {
        trimmed.to_string()
    }
}

/// Drop the oldest half of a log that grew past [`MAX_LOG_BYTES`], cutting
/// at a line boundary.
fn trim_log_if_large(path: &Path) -> io::Result<()> {
    let len = match fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if len <= MAX_LOG_BYTES {
        return Ok(());
    }
    let data = fs::read(path)?;
    let keep_from = data.len().saturating_sub((MAX_LOG_BYTES / 2) as usize);
    let start = data[keep_from..]
        .iter()
        .position(|b| *b == b'\n')
        .map(|i| keep_from + i + 1)
        .unwrap_or(data.len());
    write_atomic(path, &data[start..])
}

/// Append one timestamped line to `dir/<file>`.
pub fn append_log(dir: &Path, file: &str, line: &str) -> io::Result<()> {
    let path = dir.join(file);
    refuse_symlink(&path)?;
    trim_log_if_large(&path)?;
    let mut handle = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(handle, "{} {}", timestamp(), sanitize_line(line))
}

/// Write through a sibling temp file and rename, so a crash mid-write never
/// leaves a half-written file behind. Refuses symlinks at both names.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    refuse_symlink(path)?;
    let tmp: PathBuf = {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".tmp");
        path.with_file_name(name)
    };
    refuse_symlink(&tmp)?;
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

pub fn marker_path(dir: &Path) -> PathBuf {
    dir.join(ATTEMPT_MARKER_FILE)
}

pub fn write_marker(dir: &Path, attempt: &UpdateAttempt) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(attempt).map_err(io::Error::other)?;
    write_atomic(&marker_path(dir), &json)
}

/// Read the marker. `Ok(None)` when there is none; an unreadable or
/// malformed marker is an error.
pub fn read_marker(dir: &Path) -> io::Result<Option<UpdateAttempt>> {
    let path = marker_path(dir);
    refuse_symlink(&path)?;
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Record the start of an attempt: marker (not yet committed) plus a line in
/// the install log.
pub fn begin_attempt(dir: &Path, attempt: &UpdateAttempt) -> io::Result<()> {
    write_marker(dir, attempt)?;
    append_log(
        dir,
        INSTALL_LOG_FILE,
        &format!(
            "attempt from_version={} to_version={} target={} home_visible={} executable={}",
            attempt.from_version,
            attempt.to_version,
            attempt.target,
            attempt.home_visible,
            attempt.executable
        ),
    )
}

/// Mark the current attempt as handed over to the installer / restart.
pub fn commit_restart(dir: &Path) -> io::Result<()> {
    let Some(mut attempt) = read_marker(dir)? else {
        return Ok(());
    };
    if attempt.outcome.is_some() {
        return Ok(());
    }
    attempt.restart_committed = true;
    write_marker(dir, &attempt)?;
    append_log(
        dir,
        INSTALL_LOG_FILE,
        &format!("restart_committed to_version={}", attempt.to_version),
    )
}

/// Close the current attempt as failed in this session.
pub fn record_install_failure(dir: &Path, stage: &str, reason: &str) -> io::Result<()> {
    let to_version = match read_marker(dir)? {
        Some(mut attempt) if attempt.outcome.is_none() => {
            attempt.outcome = Some(Outcome::InstallFailed.as_str().to_string());
            write_marker(dir, &attempt)?;
            attempt.to_version
        }
        _ => String::new(),
    };
    append_log(
        dir,
        RECOVERY_LOG_FILE,
        &format!("{stage}_failed to_version={to_version} reason={reason}"),
    )
}

/// Pure judgement used by [`reconcile`].
pub fn judge(attempt: &UpdateAttempt, running_version: &str) -> Outcome {
    if !attempt.restart_committed {
        Outcome::Abandoned
    } else if normalize_version(&attempt.to_version) == normalize_version(running_version) {
        Outcome::RelaunchObserved
    } else {
        Outcome::NotApplied
    }
}

fn normalize_version(v: &str) -> &str {
    v.trim().trim_start_matches('v')
}

/// On launch: if an unjudged marker exists, stamp it with the outcome and
/// the running version, and log the result. Returns the updated attempt and
/// its outcome, or `None` when there was nothing to judge.
pub fn reconcile(dir: &Path, running_version: &str) -> io::Result<Option<(UpdateAttempt, Outcome)>> {
    let Some(mut attempt) = read_marker(dir)? else {
        return Ok(None);
    };
    if attempt.outcome.is_some() {
        return Ok(None);
    }
    let outcome = judge(&attempt, running_version);
    attempt.outcome = Some(outcome.as_str().to_string());
    attempt.running = Some(running_version.to_string());
    write_marker(dir, &attempt)?;

    match outcome {
        Outcome::RelaunchObserved => append_log(
            dir,
            INSTALL_LOG_FILE,
            &format!(
                "relaunch_observed from_version={} running={} (previous update install applied)",
                attempt.from_version, running_version
            ),
        )?,
        Outcome::NotApplied => append_log(
            dir,
            RECOVERY_LOG_FILE,
            &format!(
                "not_applied from_version={} to_version={} running={} (previous update install did NOT apply)",
                attempt.from_version, attempt.to_version, running_version
            ),
        )?,
        Outcome::Abandoned => append_log(
            dir,
            RECOVERY_LOG_FILE,
            &format!(
                "abandoned from_version={} to_version={} running={} (restart was never committed)",
                attempt.from_version, attempt.to_version, running_version
            ),
        )?,
        Outcome::InstallFailed => {}
    }
    Ok(Some((attempt, outcome)))
}

/// Remove the per-update temp folders the Tauri updater leaves behind on
/// Windows (`<app name>-<version>-updater-<random>` under the temp dir) once
/// they are older than `min_age`. Symlinks and anything not matching that
/// pattern are left alone. Returns how many folders were removed.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn remove_leftover_installers(temp_dir: &Path, app_name: &str, min_age: Duration) -> usize {
    let app_name = app_name.trim();
    if app_name.is_empty() {
        return 0;
    }
    let prefix = format!("{app_name}-");
    let Ok(entries) = fs::read_dir(temp_dir) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(&prefix) || !name[prefix.len()..].contains("-updater-") {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            continue;
        }
        // A modification time in the future counts as "just now".
        let old_enough = meta
            .modified()
            .ok()
            .map(|m| now.duration_since(m).unwrap_or(Duration::ZERO))
            .is_some_and(|age| age >= min_age);
        if old_enough && fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> UpdateAttempt {
        UpdateAttempt::new(
            "2.4.1",
            "2.4.2",
            "windows-x86_64",
            "C:\\Program Files\\App\\app.exe",
            true,
        )
    }

    #[test]
    fn marker_round_trips_with_every_field() {
        let dir = tempfile::tempdir().unwrap();
        let attempt = sample();
        write_marker(dir.path(), &attempt).unwrap();

        let raw: serde_json::Value =
            serde_json::from_slice(&fs::read(marker_path(dir.path())).unwrap()).unwrap();
        for key in [
            "from_version",
            "to_version",
            "target",
            "executable",
            "home_visible",
            "attempt_ts",
            "ts_epoch_secs",
            "restart_committed",
        ] {
            assert!(raw.get(key).is_some(), "marker is missing {key}");
        }
        assert!(raw.get("outcome").is_none(), "unjudged marker has no outcome");
        assert_eq!(read_marker(dir.path()).unwrap(), Some(attempt));
    }

    #[test]
    fn marker_with_unknown_and_missing_fields_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            marker_path(dir.path()),
            br#"{"to_version":"3.0.0","something_new":42}"#,
        )
        .unwrap();
        let attempt = read_marker(dir.path()).unwrap().unwrap();
        assert_eq!(attempt.to_version, "3.0.0");
        assert!(!attempt.restart_committed);
    }

    #[test]
    fn begin_and_commit_write_marker_and_install_log() {
        let dir = tempfile::tempdir().unwrap();
        begin_attempt(dir.path(), &sample()).unwrap();
        assert!(!read_marker(dir.path()).unwrap().unwrap().restart_committed);

        commit_restart(dir.path()).unwrap();
        assert!(read_marker(dir.path()).unwrap().unwrap().restart_committed);

        let log = fs::read_to_string(dir.path().join(INSTALL_LOG_FILE)).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("attempt from_version=2.4.1 to_version=2.4.2"));
        assert!(lines[1].contains("restart_committed to_version=2.4.2"));
    }

    #[test]
    fn commit_without_marker_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        commit_restart(dir.path()).unwrap();
        assert!(read_marker(dir.path()).unwrap().is_none());
        assert!(!dir.path().join(INSTALL_LOG_FILE).exists());
    }

    #[test]
    fn judge_covers_every_outcome() {
        let mut attempt = sample();
        assert_eq!(judge(&attempt, "2.4.1"), Outcome::Abandoned);
        attempt.restart_committed = true;
        assert_eq!(judge(&attempt, "2.4.2"), Outcome::RelaunchObserved);
        assert_eq!(judge(&attempt, "v2.4.2"), Outcome::RelaunchObserved);
        assert_eq!(judge(&attempt, "2.4.1"), Outcome::NotApplied);
    }

    #[test]
    fn reconcile_stamps_success_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut attempt = sample();
        attempt.restart_committed = true;
        write_marker(dir.path(), &attempt).unwrap();

        let (judged, outcome) = reconcile(dir.path(), "2.4.2").unwrap().unwrap();
        assert_eq!(outcome, Outcome::RelaunchObserved);
        assert_eq!(judged.outcome.as_deref(), Some("relaunch_observed"));
        assert_eq!(judged.running.as_deref(), Some("2.4.2"));
        assert_eq!(read_marker(dir.path()).unwrap(), Some(judged));
        assert!(fs::read_to_string(dir.path().join(INSTALL_LOG_FILE))
            .unwrap()
            .contains("relaunch_observed"));
        assert!(!dir.path().join(RECOVERY_LOG_FILE).exists());

        // A judged marker is not judged again.
        assert!(reconcile(dir.path(), "2.4.2").unwrap().is_none());
    }

    #[test]
    fn reconcile_logs_failures_to_the_recovery_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut attempt = sample();
        attempt.restart_committed = true;
        write_marker(dir.path(), &attempt).unwrap();
        let (_, outcome) = reconcile(dir.path(), "2.4.1").unwrap().unwrap();
        assert_eq!(outcome, Outcome::NotApplied);
        let recovery = fs::read_to_string(dir.path().join(RECOVERY_LOG_FILE)).unwrap();
        assert!(recovery.contains("not_applied"));
        assert!(recovery.contains("did NOT apply"));
    }

    #[test]
    fn install_failure_closes_the_attempt() {
        let dir = tempfile::tempdir().unwrap();
        begin_attempt(dir.path(), &sample()).unwrap();
        record_install_failure(dir.path(), "install", "disk full\nsecond line").unwrap();
        let attempt = read_marker(dir.path()).unwrap().unwrap();
        assert_eq!(attempt.outcome.as_deref(), Some("install_failed"));
        let recovery = fs::read_to_string(dir.path().join(RECOVERY_LOG_FILE)).unwrap();
        assert_eq!(recovery.lines().count(), 1, "control chars are flattened");
        assert!(recovery.contains("install_failed to_version=2.4.2 reason=disk full second line"));
        // Nothing left to judge on the next launch.
        assert!(reconcile(dir.path(), "2.4.1").unwrap().is_none());
    }

    #[test]
    fn large_logs_are_trimmed_at_a_line_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(INSTALL_LOG_FILE);
        let line = "x".repeat(99);
        let mut big = String::new();
        while (big.len() as u64) <= MAX_LOG_BYTES {
            big.push_str(&line);
            big.push('\n');
        }
        fs::write(&path, &big).unwrap();
        append_log(dir.path(), INSTALL_LOG_FILE, "fresh entry").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!((after.len() as u64) < MAX_LOG_BYTES);
        assert!(after.lines().all(|l| l == line || l.ends_with("fresh entry")));
        assert!(after.ends_with("fresh entry\n"));
    }

    fn try_symlink_file(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
        #[cfg(windows)]
        {
            // Needs Developer Mode or the symlink privilege; skip otherwise.
            std::os::windows::fs::symlink_file(target, link).is_ok()
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, link);
            false
        }
    }

    #[test]
    fn symlinked_diagnostic_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside.txt");
        fs::write(&outside, b"keep me").unwrap();

        for name in [INSTALL_LOG_FILE, ATTEMPT_MARKER_FILE] {
            let link = dir.path().join(name);
            if !try_symlink_file(&outside, &link) {
                eprintln!("symlinks unavailable here; skipping {name}");
                continue;
            }
            assert!(refuse_symlink(&link).is_err());
            let err = if name == INSTALL_LOG_FILE {
                append_log(dir.path(), name, "should not be written").unwrap_err()
            } else {
                write_marker(dir.path(), &sample()).unwrap_err()
            };
            assert!(err.to_string().contains("symlink"), "{err}");
            assert_eq!(fs::read(&outside).unwrap(), b"keep me");
            fs::remove_file(&link).unwrap();
        }
    }

    #[test]
    fn leftover_installer_cleanup_only_touches_matching_folders() {
        let temp = tempfile::tempdir().unwrap();
        let ours = temp.path().join("Freenpipe-2.4.2-updater-AbC123");
        let other_app = temp.path().join("OtherApp-1.0.0-updater-xyz");
        let unrelated = temp.path().join("Freenpipe-cache");
        let file = temp.path().join("Freenpipe-2.4.2-updater-file");
        for dir in [&ours, &other_app, &unrelated] {
            fs::create_dir_all(dir).unwrap();
            fs::write(dir.join("payload.exe"), b"x").unwrap();
        }
        fs::write(&file, b"x").unwrap();

        // Too young: nothing is removed.
        assert_eq!(
            remove_leftover_installers(temp.path(), "Freenpipe", Duration::from_secs(3600)),
            0
        );
        assert_eq!(
            remove_leftover_installers(temp.path(), "Freenpipe", Duration::ZERO),
            1
        );
        assert!(!ours.exists());
        assert!(other_app.exists() && unrelated.exists() && file.exists());
        assert_eq!(remove_leftover_installers(temp.path(), "  ", Duration::ZERO), 0);
    }
}
