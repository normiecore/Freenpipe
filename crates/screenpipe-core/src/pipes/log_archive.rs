// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Periodic archive sweep of per-run pipe logs.
//!
//! Every pipe run writes one `<pipe>/logs/%Y%m%d_%H%M%S.json` file. Left
//! alone these pile up: a pipe that runs every five minutes writes over
//! a hundred thousand a year. The sweep moves old ones into one compressed
//! file per month, `<pipe>/logs/archive/<YYYY-MM>.jsonl.gz`, one JSON line
//! per run (`{"file": <original name>, "log": <the run log>}`). Each sweep
//! appends a gzip member, so a month file is read with a multi-member gzip
//! reader ([`read_archive`]). Next to it, `<YYYY-MM>.names` lists the file
//! names the archive holds, one per line, so a log whose original could not
//! be deleted (for example a file another program holds open on Windows) is
//! not appended again on the next sweep: that sweep only retries the delete.
//!
//! What it never touches:
//!
//! - logs of a pipe that has a run queued or running (the caller passes
//!   those names; the run writes its log when it ends);
//! - the newest [`ArchivePolicy::keep_recent`] logs of each pipe, and any
//!   log younger than [`ArchivePolicy::min_age`] by both its name and its
//!   modification time;
//! - files whose names it did not write.
//!
//! A log is deleted only after its archive member has been written and
//! synced to disk. If writing fails the originals stay and the failure is
//! counted. One sweep handles at most [`ArchivePolicy::max_files`] logs; the
//! rest wait for the next sweep. Month archives are kept for good unless
//! [`ArchivePolicy::keep_months`] is set: run logs are the only record of a
//! run older than the execution rows the database keeps.
//!
//! The sweep does blocking file I/O and is meant to run on a blocking
//! thread, away from the scheduler loop.

use std::collections::{BTreeMap, HashSet};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, NaiveDateTime, Utc};
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Value};

/// Folder of the per-run logs inside a pipe's directory.
pub const LOGS_DIR: &str = "logs";
/// Folder of the month archives inside [`LOGS_DIR`].
pub const ARCHIVE_DIR: &str = "archive";
/// How often the scheduler runs the sweep.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// How long after the scheduler starts the first sweep runs (kept off the
/// startup path).
pub const FIRST_SWEEP_DELAY: Duration = Duration::from_secs(5 * 60);

/// Which logs a sweep may archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePolicy {
    /// Newest logs of each pipe that always stay in place.
    pub keep_recent: usize,
    /// Logs younger than this stay in place.
    pub min_age: Duration,
    /// Most logs archived in one sweep.
    pub max_files: usize,
    /// Month archives kept per pipe; `None` (the default) keeps them all.
    pub keep_months: Option<usize>,
}

impl Default for ArchivePolicy {
    fn default() -> Self {
        Self {
            keep_recent: 50,
            min_age: Duration::from_secs(7 * 24 * 60 * 60),
            max_files: 2_000,
            keep_months: None,
        }
    }
}

/// What one sweep did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveReport {
    /// Pipe directories looked at.
    pub pipes: usize,
    /// Per-run log files found.
    pub scanned: usize,
    /// Logs moved into an archive.
    pub archived: usize,
    /// Size of the archived logs before compression.
    pub archived_bytes: u64,
    /// Logs that could not be archived, or whose original could not be
    /// deleted after archiving (they stay in place).
    pub failed: usize,
    /// Logs found already in their month archive (an earlier sweep could not
    /// delete them); this sweep deleted the leftover copy without adding it
    /// again.
    pub already_archived: usize,
    /// Pipes left alone because a run was queued or running.
    pub skipped_running: usize,
    /// Old month archives removed.
    pub archives_removed: usize,
    /// The sweep stopped at [`ArchivePolicy::max_files`].
    pub limited: bool,
}

impl ArchiveReport {
    /// The line the scheduler logs.
    pub fn summary(&self) -> String {
        format!(
            "scanned={} archived={} bytes={} failed={} already_archived={} pipes={} skipped_running={} archives_removed={}{}",
            self.scanned,
            self.archived,
            self.archived_bytes,
            self.failed,
            self.already_archived,
            self.pipes,
            self.skipped_running,
            self.archives_removed,
            if self.limited { " (limit reached)" } else { "" }
        )
    }
}

/// A per-run log file and the start time in its name (UTC).
#[derive(Debug, Clone)]
struct RunLogFile {
    path: PathBuf,
    name: String,
    started: DateTime<Utc>,
    size: u64,
    modified: SystemTime,
}

/// Start time encoded in a per-run log file name (`%Y%m%d_%H%M%S.json`, or
/// `%Y%m%d_%H%M%S_<n>.json` for a second run started in the same second),
/// `None` for other files.
fn parse_log_name(name: &str) -> Option<DateTime<Utc>> {
    let stem = name.strip_suffix(".json")?;
    let stamp = stem.get(..15)?;
    let rest = &stem[15..];
    if !(rest.is_empty()
        || (rest.len() > 1
            && rest.starts_with('_')
            && rest[1..].bytes().all(|b| b.is_ascii_digit())))
    {
        return None;
    }
    NaiveDateTime::parse_from_str(stamp, "%Y%m%d_%H%M%S")
        .ok()
        .map(|naive| naive.and_utc())
}

/// Write a run log into `<pipe_dir>/logs/` under its start time, without
/// replacing the log of another run that started in the same second.
pub fn write_run_log(
    pipe_dir: &Path,
    started_at: DateTime<Utc>,
    body: &str,
) -> std::io::Result<PathBuf> {
    let dir = pipe_dir.join(LOGS_DIR);
    std::fs::create_dir_all(&dir)?;
    let stamp = started_at.format("%Y%m%d_%H%M%S").to_string();
    for n in 0..1000u32 {
        let name = if n == 0 {
            format!("{stamp}.json")
        } else {
            format!("{stamp}_{n}.json")
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(body.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free run log name",
    ))
}

/// Archive month (`YYYY-MM`) of a run started at `started`.
fn month_of(started: DateTime<Utc>) -> String {
    started.format("%Y-%m").to_string()
}

/// Sweep every pipe directory under `pipes_dir`. `busy` names the pipes
/// with a run queued or running.
pub fn sweep(
    pipes_dir: &Path,
    busy: &HashSet<String>,
    policy: &ArchivePolicy,
    now: SystemTime,
) -> ArchiveReport {
    let mut report = ArchiveReport::default();
    let Ok(entries) = std::fs::read_dir(pipes_dir) else {
        return report;
    };
    let mut pipes: Vec<(String, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
        .filter(|(name, _)| !name.starts_with('.'))
        .collect();
    pipes.sort();
    for (name, dir) in pipes {
        if report.limited {
            break;
        }
        report.pipes += 1;
        if busy.contains(&name) {
            report.skipped_running += 1;
            continue;
        }
        sweep_pipe(&dir.join(LOGS_DIR), policy, now, &mut report);
    }
    report
}

fn sweep_pipe(
    logs_dir: &Path,
    policy: &ArchivePolicy,
    now: SystemTime,
    report: &mut ArchiveReport,
) {
    let Ok(entries) = std::fs::read_dir(logs_dir) else {
        return;
    };
    let mut logs: Vec<RunLogFile> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let started = parse_log_name(&name)?;
            Some(RunLogFile {
                path: e.path(),
                name,
                started,
                size: meta.len(),
                modified: meta.modified().unwrap_or(now),
            })
        })
        .collect();
    report.scanned += logs.len();
    logs.sort_by(|a, b| b.started.cmp(&a.started).then_with(|| b.name.cmp(&a.name)));

    let now_utc: DateTime<Utc> = now.into();
    let min_age = chrono::Duration::from_std(policy.min_age).unwrap_or(chrono::Duration::MAX);
    let mut by_month: BTreeMap<String, Vec<RunLogFile>> = BTreeMap::new();
    for log in logs.into_iter().skip(policy.keep_recent) {
        let old_by_name = now_utc.signed_duration_since(log.started) >= min_age;
        let old_by_mtime = now
            .duration_since(log.modified)
            .map(|age| age >= policy.min_age)
            .unwrap_or(false);
        if !(old_by_name && old_by_mtime) {
            continue;
        }
        let budget = policy
            .max_files
            .saturating_sub(report.archived + report.failed);
        let queued: usize = by_month.values().map(Vec::len).sum();
        if queued >= budget {
            report.limited = true;
            break;
        }
        by_month.entry(month_of(log.started)).or_default().push(log);
    }

    if !by_month.is_empty() {
        let archive_dir = logs_dir.join(ARCHIVE_DIR);
        if let Err(e) = std::fs::create_dir_all(&archive_dir) {
            let n: usize = by_month.values().map(Vec::len).sum();
            report.failed += n;
            tracing::warn!(
                "pipe log archive: cannot create {}: {}",
                archive_dir.display(),
                e
            );
            return;
        }
        for (month, mut files) in by_month {
            // Oldest first inside the member.
            files.reverse();
            let target = archive_dir.join(format!("{month}.jsonl.gz"));
            let names_path = archive_dir.join(format!("{month}{NAMES_SUFFIX}"));
            let known = archived_names(&target, &names_path);
            let (leftovers, files): (Vec<RunLogFile>, Vec<RunLogFile>) =
                files.into_iter().partition(|f| known.contains(&f.name));
            for file in leftovers {
                match std::fs::remove_file(&file.path) {
                    Ok(()) => report.already_archived += 1,
                    Err(e) => {
                        report.failed += 1;
                        tracing::debug!(
                            "pipe log archive: {} is archived but still cannot be removed: {}",
                            file.path.display(),
                            e
                        );
                    }
                }
            }
            if files.is_empty() {
                continue;
            }
            match append_member(&target, &files) {
                Ok(()) => {
                    record_names(&names_path, &files);
                    for file in files {
                        match std::fs::remove_file(&file.path) {
                            Ok(()) => {
                                report.archived += 1;
                                report.archived_bytes += file.size;
                            }
                            Err(e) => {
                                report.failed += 1;
                                tracing::debug!(
                                    "pipe log archive: archived but could not remove {}: {}",
                                    file.path.display(),
                                    e
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    report.failed += files.len();
                    tracing::warn!(
                        "pipe log archive: could not write {}: {}",
                        target.display(),
                        e
                    );
                }
            }
        }
    }

    if let Some(keep) = policy.keep_months {
        report.archives_removed += prune_months(&logs_dir.join(ARCHIVE_DIR), keep);
    }
}

/// Suffix of the list of file names a month archive holds.
const NAMES_SUFFIX: &str = ".names";

/// Names of the logs already in the month archive `target`: from its names
/// list, or, when that list is missing, from the archive itself (the list is
/// then written again). Empty when there is no archive.
fn archived_names(target: &Path, names_path: &Path) -> HashSet<String> {
    if !target.exists() {
        // A list without its archive vouches for nothing.
        let _ = std::fs::remove_file(names_path);
        return HashSet::new();
    }
    if let Ok(text) = std::fs::read_to_string(names_path) {
        return text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
    }
    let names: HashSet<String> = match read_archive(target) {
        Ok(records) => records
            .iter()
            .filter_map(|r| r.get("file").and_then(Value::as_str))
            .map(str::to_string)
            .collect(),
        Err(e) => {
            tracing::warn!("pipe log archive: cannot read {}: {}", target.display(), e);
            HashSet::new()
        }
    };
    let mut sorted: Vec<&String> = names.iter().collect();
    sorted.sort();
    let mut text = String::new();
    for name in sorted {
        text.push_str(name);
        text.push('\n');
    }
    if let Err(e) = std::fs::write(names_path, text) {
        tracing::debug!(
            "pipe log archive: cannot write {}: {}",
            names_path.display(),
            e
        );
    }
    names
}

/// Add the names of `files`, just appended to the month archive, to its
/// names list. When that fails the list is removed, so the next sweep reads
/// the names from the archive itself rather than trusting a stale list.
fn record_names(names_path: &Path, files: &[RunLogFile]) {
    let mut text = String::new();
    for file in files {
        text.push_str(&file.name);
        text.push('\n');
    }
    let written = OpenOptions::new()
        .create(true)
        .append(true)
        .open(names_path)
        .and_then(|mut out| {
            out.write_all(text.as_bytes())?;
            out.sync_all()
        });
    if let Err(e) = written {
        tracing::warn!(
            "pipe log archive: cannot update {}: {}",
            names_path.display(),
            e
        );
        let _ = std::fs::remove_file(names_path);
    }
}

/// Append one gzip member holding `files` to `target`, synced to disk.
fn append_member(target: &Path, files: &[RunLogFile]) -> std::io::Result<()> {
    let mut lines: Vec<u8> = Vec::new();
    for file in files {
        let mut raw = Vec::new();
        std::fs::File::open(&file.path)?.read_to_end(&mut raw)?;
        let log = serde_json::from_slice::<Value>(&raw)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&raw).into_owned()));
        serde_json::to_writer(&mut lines, &json!({ "file": file.name, "log": log }))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        lines.push(b'\n');
    }
    let out = OpenOptions::new().create(true).append(true).open(target)?;
    let mut encoder = GzEncoder::new(out, Compression::default());
    encoder.write_all(&lines)?;
    let out = encoder.finish()?;
    out.sync_all()
}

/// Keep the newest `keep` month archives in `archive_dir`; returns how many
/// were removed.
fn prune_months(archive_dir: &Path, keep: usize) -> usize {
    let Ok(entries) = std::fs::read_dir(archive_dir) else {
        return 0;
    };
    let mut months: Vec<(String, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let month = name.strip_suffix(".jsonl.gz")?;
            // `YYYY-MM`
            let valid = month.len() == 7
                && month.as_bytes()[4] == b'-'
                && month
                    .bytes()
                    .enumerate()
                    .all(|(i, b)| i == 4 || b.is_ascii_digit());
            valid.then(|| (name.clone(), e.path()))
        })
        .collect();
    months.sort_by(|a, b| b.0.cmp(&a.0));
    let mut removed = 0;
    for (name, path) in months.into_iter().skip(keep.max(1)) {
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
            if let Some(month) = name.strip_suffix(".jsonl.gz") {
                let _ = std::fs::remove_file(archive_dir.join(format!("{month}{NAMES_SUFFIX}")));
            }
        }
    }
    removed
}

/// Every record of a month archive, in the order written. A log that was
/// appended twice (the sweep stopped between writing a member and noting its
/// names) is returned once.
pub fn read_archive(path: &Path) -> std::io::Result<Vec<Value>> {
    let mut text = String::new();
    MultiGzDecoder::new(std::fs::File::open(path)?).read_to_string(&mut text)?;
    let mut seen: HashSet<String> = HashSet::new();
    Ok(text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|record| match record.get("file").and_then(Value::as_str) {
            Some(name) => seen.insert(name.to_string()),
            None => true,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    fn write_log(logs: &Path, started: DateTime<Utc>, age: Duration, now: SystemTime) -> PathBuf {
        std::fs::create_dir_all(logs).unwrap();
        let path = logs.join(format!("{}.json", started.format("%Y%m%d_%H%M%S")));
        std::fs::write(
            &path,
            serde_json::to_string_pretty(
                &json!({"pipe_name": "p", "success": true, "stdout": "ok"}),
            )
            .unwrap(),
        )
        .unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(now - age).unwrap();
        path
    }

    fn policy() -> ArchivePolicy {
        ArchivePolicy {
            keep_recent: 2,
            min_age: 7 * DAY,
            max_files: 100,
            keep_months: None,
        }
    }

    #[test]
    fn old_logs_move_into_month_archives_and_recent_ones_stay() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let now_utc: DateTime<Utc> = now.into();
        let logs = dir.path().join("digest").join(LOGS_DIR);
        let mut old = Vec::new();
        for days in [40u64, 39, 38, 10] {
            let age = DAY * days as u32;
            let started = now_utc - chrono::Duration::from_std(age).unwrap();
            old.push(write_log(&logs, started, age, now));
        }
        // Two recent logs: inside keep_recent and younger than min_age.
        let recent_a = write_log(&logs, now_utc - chrono::Duration::days(1), DAY, now);
        let recent_b = write_log(
            &logs,
            now_utc - chrono::Duration::hours(1),
            Duration::from_secs(3600),
            now,
        );
        // Not a per-run log: never touched.
        std::fs::write(logs.join("notes.txt"), "keep me").unwrap();

        let report = sweep(dir.path(), &HashSet::new(), &policy(), now);
        assert_eq!(report.pipes, 1);
        assert_eq!(report.scanned, 6);
        assert_eq!(report.archived, 4, "{report:?}");
        assert_eq!(report.failed, 0);
        assert!(report.archived_bytes > 0);
        assert!(report.summary().contains("scanned=6 archived=4"));
        for p in &old {
            assert!(!p.exists(), "{} should be archived", p.display());
        }
        assert!(recent_a.exists() && recent_b.exists());
        assert!(logs.join("notes.txt").exists());

        let mut records = Vec::new();
        let mut listed = 0;
        for entry in std::fs::read_dir(logs.join(ARCHIVE_DIR)).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if name.ends_with(NAMES_SUFFIX) {
                listed += std::fs::read_to_string(&path).unwrap().lines().count();
            } else {
                records.extend(read_archive(&path).unwrap());
            }
        }
        assert_eq!(listed, 4);
        assert_eq!(records.len(), 4);
        assert!(records.iter().all(|r| r["log"]["stdout"] == "ok"));
        assert!(records
            .iter()
            .all(|r| parse_log_name(r["file"].as_str().unwrap()).is_some()));

        // A second sweep finds nothing more to do, and later members append.
        let again = sweep(dir.path(), &HashSet::new(), &policy(), now);
        assert_eq!(again.archived, 0);
        assert_eq!(again.scanned, 2);
    }

    #[test]
    fn a_pipe_with_a_run_in_progress_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let now_utc: DateTime<Utc> = now.into();
        let logs = dir.path().join("busy").join(LOGS_DIR);
        let mut paths = Vec::new();
        for days in 20..25u64 {
            let age = DAY * days as u32;
            paths.push(write_log(
                &logs,
                now_utc - chrono::Duration::from_std(age).unwrap(),
                age,
                now,
            ));
        }
        let busy: HashSet<String> = ["busy".to_string()].into_iter().collect();
        let report = sweep(dir.path(), &busy, &policy(), now);
        assert_eq!(report.skipped_running, 1);
        assert_eq!(report.archived, 0);
        assert!(paths.iter().all(|p| p.exists()));
    }

    #[test]
    fn a_recently_rewritten_old_log_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let now_utc: DateTime<Utc> = now.into();
        let logs = dir.path().join("p").join(LOGS_DIR);
        let mut paths = Vec::new();
        for days in 30..34u64 {
            let started = now_utc - chrono::Duration::days(days as i64);
            // Old by name but written a minute ago.
            paths.push(write_log(&logs, started, Duration::from_secs(60), now));
        }
        let report = sweep(dir.path(), &HashSet::new(), &policy(), now);
        assert_eq!(report.archived, 0);
        assert!(paths.iter().all(|p| p.exists()));
    }

    #[test]
    fn one_sweep_is_bounded_and_old_months_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let now_utc: DateTime<Utc> = now.into();
        let logs = dir.path().join("p").join(LOGS_DIR);
        for days in 100..110u64 {
            let age = DAY * days as u32;
            write_log(
                &logs,
                now_utc - chrono::Duration::from_std(age).unwrap(),
                age,
                now,
            );
        }
        let archive = logs.join(ARCHIVE_DIR);
        std::fs::create_dir_all(&archive).unwrap();
        for month in ["2001-01", "2001-02", "2001-03"] {
            std::fs::write(archive.join(format!("{month}.jsonl.gz")), b"").unwrap();
        }
        let bounded = ArchivePolicy {
            max_files: 3,
            keep_months: Some(2),
            ..policy()
        };
        let report = sweep(dir.path(), &HashSet::new(), &bounded, now);
        assert_eq!(report.archived, 3);
        assert!(report.limited);
        // The new month file(s) are the newest; the 2001 ones go.
        assert!(report.archives_removed >= 2);
        assert!(!archive.join("2001-01.jsonl.gz").exists());
        let next = sweep(dir.path(), &HashSet::new(), &bounded, now);
        assert_eq!(next.archived, 3);
    }

    #[test]
    fn month_archives_are_kept_unless_pruning_is_asked_for() {
        assert_eq!(ArchivePolicy::default().keep_months, None);
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let now_utc: DateTime<Utc> = now.into();
        let logs = dir.path().join("p").join(LOGS_DIR);
        for days in 100..105u64 {
            let age = DAY * days as u32;
            write_log(
                &logs,
                now_utc - chrono::Duration::from_std(age).unwrap(),
                age,
                now,
            );
        }
        let archive = logs.join(ARCHIVE_DIR);
        std::fs::create_dir_all(&archive).unwrap();
        // Twelve old months plus the one this sweep writes.
        for month in 1..=12 {
            std::fs::write(archive.join(format!("2001-{month:02}.jsonl.gz")), b"").unwrap();
        }
        let report = sweep(dir.path(), &HashSet::new(), &policy(), now);
        assert_eq!(report.archived, 3);
        assert_eq!(report.archives_removed, 0);
        assert!(archive.join("2001-01.jsonl.gz").exists());
    }

    #[test]
    fn a_log_whose_delete_failed_is_not_archived_twice() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let now_utc: DateTime<Utc> = now.into();
        let logs = dir.path().join("p").join(LOGS_DIR);
        let started = now_utc - chrono::Duration::days(40);
        let old = write_log(&logs, started, DAY * 40, now);
        let body = std::fs::read(&old).unwrap();
        for hours in 1..=2 {
            write_log(
                &logs,
                now_utc - chrono::Duration::hours(hours),
                Duration::from_secs(3600),
                now,
            );
        }
        let first = sweep(dir.path(), &HashSet::new(), &policy(), now);
        assert_eq!(first.archived, 1);
        let target = logs
            .join(ARCHIVE_DIR)
            .join(format!("{}.jsonl.gz", month_of(started)));
        let names = target.with_file_name(format!("{}{NAMES_SUFFIX}", month_of(started)));
        assert!(names.exists());

        // The original comes back, as when its delete failed (another
        // program held it open).
        let leftover = |path: &Path| {
            std::fs::write(path, &body).unwrap();
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(now - DAY * 40)
                .unwrap();
        };
        leftover(&old);
        let again = sweep(dir.path(), &HashSet::new(), &policy(), now);
        assert_eq!(again.archived, 0, "{again:?}");
        assert_eq!(again.already_archived, 1);
        assert_eq!(again.failed, 0);
        assert!(!old.exists());
        assert!(again.summary().contains("already_archived=1"));

        // Without the names list the archive itself is read.
        std::fs::remove_file(&names).unwrap();
        leftover(&old);
        let rebuilt = sweep(dir.path(), &HashSet::new(), &policy(), now);
        assert_eq!(rebuilt.already_archived, 1, "{rebuilt:?}");
        assert!(names.exists());

        // The raw archive holds the log once.
        let mut text = String::new();
        MultiGzDecoder::new(std::fs::File::open(&target).unwrap())
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text.lines().filter(|l| !l.trim().is_empty()).count(), 1);
        assert_eq!(read_archive(&target).unwrap().len(), 1);
    }

    #[test]
    fn a_log_appended_twice_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let now_utc: DateTime<Utc> = now.into();
        let log = write_log(dir.path(), now_utc, DAY, now);
        let file = RunLogFile {
            path: log.clone(),
            name: log.file_name().unwrap().to_string_lossy().to_string(),
            started: now_utc,
            size: 1,
            modified: now,
        };
        let target = dir.path().join("2026-01.jsonl.gz");
        append_member(&target, std::slice::from_ref(&file)).unwrap();
        append_member(&target, std::slice::from_ref(&file)).unwrap();
        assert_eq!(read_archive(&target).unwrap().len(), 1);
    }

    #[test]
    fn run_logs_of_the_same_second_get_their_own_files() {
        let dir = tempfile::tempdir().unwrap();
        let at = DateTime::parse_from_rfc3339("2026-03-04T05:06:07Z")
            .unwrap()
            .with_timezone(&Utc);
        let a = write_run_log(dir.path(), at, "{}").unwrap();
        let b = write_run_log(dir.path(), at, "{}").unwrap();
        assert_eq!(a.file_name().unwrap(), "20260304_050607.json");
        assert_eq!(b.file_name().unwrap(), "20260304_050607_1.json");
        assert_eq!(parse_log_name("20260304_050607_1.json"), Some(at));
        assert_eq!(parse_log_name("20260304_050607.json"), Some(at));
        assert_eq!(parse_log_name("20260304_050607_x.json"), None);
        assert_eq!(parse_log_name("20260304_050607_.json"), None);
        assert_eq!(parse_log_name("notes.json"), None);
    }

    #[test]
    fn missing_or_empty_pipes_dirs_are_fine() {
        let dir = tempfile::tempdir().unwrap();
        let report = sweep(
            &dir.path().join("nope"),
            &HashSet::new(),
            &policy(),
            SystemTime::now(),
        );
        assert_eq!(report, ArchiveReport::default());
        std::fs::create_dir_all(dir.path().join("p")).unwrap();
        let report = sweep(dir.path(), &HashSet::new(), &policy(), SystemTime::now());
        assert_eq!(report.pipes, 1);
        assert_eq!(report.scanned, 0);
    }
}
