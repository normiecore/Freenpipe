// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Pipe run trajectories: an opt-in JSONL recording of what the agent did in
//! each pipe run, for evaluating and debugging pipes.
//!
//! Off unless `SCREENPIPE_TRAJECTORIES_DIR` is set. Its value is the folder
//! the files go to; `1`, `true`, `on` or `yes` pick `<data dir>/trajectories`,
//! and a relative path is taken relative to the data directory. Each run
//! writes one file named after its start time in UTC
//! (`%Y-%m-%dT%H-%M-%S%.3fZ.jsonl`). After every run only the newest
//! `SCREENPIPE_TRAJECTORIES_MAX_RUNS` files (default [`DEFAULT_MAX_RUNS`])
//! are kept. Nothing leaves the machine.
//!
//! Every line is one JSON object with `type`, `seq` and `at` (RFC 3339 UTC):
//!
//! - `run_start`: pipe, execution id, trigger, event and key, model,
//!   provider, preset, attempt, and the prompts the agent was given;
//! - `agent_event`: one line of the agent's event stream, as it arrived (a
//!   delta of the run), parsed when it is JSON and kept as `text` otherwise;
//! - `status`: a scheduler note during the run (a model fallback);
//! - `truncated`: written once when the file reached [`MAX_FILE_BYTES`]; later
//!   events are counted but not written;
//! - `run_end`: success, error type, duration and event count.
//!
//! Credentials are removed from every record the way `.trigger-context.json`
//! removes them (see [`redact`](super::redact)). Trajectories still contain
//! whatever the agent read from the user's screen history, which is why they
//! are opt-in.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, SecondsFormat, Utc};
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use tracing::{debug, warn};

use super::redact::{redact_text, redact_value};
use crate::env::{SCREENPIPE_TRAJECTORIES_DIR, SCREENPIPE_TRAJECTORIES_MAX_RUNS};

/// Runs kept when `SCREENPIPE_TRAJECTORIES_MAX_RUNS` is unset or invalid.
pub const DEFAULT_MAX_RUNS: usize = 100;
/// Highest accepted `SCREENPIPE_TRAJECTORIES_MAX_RUNS`.
pub const MAX_MAX_RUNS: usize = 10_000;
/// Folder used when the variable only switches trajectories on.
pub const DEFAULT_DIR_NAME: &str = "trajectories";
/// Largest trajectory file; later events are not written.
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Longest agent event line written as is; longer lines are cut.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Where trajectories go and how many are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrajectoryConfig {
    pub dir: PathBuf,
    pub max_runs: usize,
}

impl TrajectoryConfig {
    /// Read the configuration from the environment. `None` when
    /// trajectories are off.
    pub fn from_env(data_dir: &Path) -> Option<Self> {
        Self::from_lookup(data_dir, |name| std::env::var(name).ok())
    }

    /// [`from_env`](Self::from_env) with the environment supplied by the
    /// caller.
    pub fn from_lookup(data_dir: &Path, env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let raw = env(SCREENPIPE_TRAJECTORIES_DIR)?;
        let value = raw.trim();
        let dir = match value.to_ascii_lowercase().as_str() {
            "" | "0" | "false" | "off" | "no" => return None,
            "1" | "true" | "on" | "yes" => data_dir.join(DEFAULT_DIR_NAME),
            _ => {
                let path = match value
                    .strip_prefix("~/")
                    .or_else(|| value.strip_prefix("~\\"))
                {
                    Some(rest) => dirs::home_dir()?.join(rest),
                    None => PathBuf::from(value),
                };
                if path.is_absolute() {
                    path
                } else {
                    data_dir.join(path)
                }
            }
        };
        let max_runs = env(SCREENPIPE_TRAJECTORIES_MAX_RUNS)
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| *n > 0)
            .map(|n| n.min(MAX_MAX_RUNS))
            .unwrap_or(DEFAULT_MAX_RUNS);
        Some(Self { dir, max_runs })
    }
}

/// What a trajectory's `run_start` record says about the run.
#[derive(Debug, Clone, Default)]
pub struct TrajectoryMeta {
    pub pipe: String,
    pub execution_id: Option<i64>,
    /// `manual`, `scheduled`, `event` or `connection`.
    pub trigger: String,
    pub trigger_event: Option<String>,
    pub trigger_key: Option<String>,
    pub model: String,
    pub provider: Option<String>,
    pub preset: Option<String>,
    /// 1 for the first try, higher for a fallback preset.
    pub attempt: u32,
    pub prompt: String,
    pub system_prompt: Option<String>,
}

/// What a trajectory's `run_end` record says.
#[derive(Debug, Clone, Default)]
pub struct TrajectoryEnd {
    pub success: bool,
    pub error_type: Option<String>,
    pub duration_ms: i64,
}

/// Files being written right now; pruning never removes them.
static ACTIVE: Lazy<Mutex<HashSet<PathBuf>>> = Lazy::new(|| Mutex::new(HashSet::new()));

fn active() -> std::sync::MutexGuard<'static, HashSet<PathBuf>> {
    ACTIVE.lock().unwrap_or_else(|p| p.into_inner())
}

fn now_text() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// File name of a trajectory started at `at`.
pub fn file_name(at: DateTime<Utc>) -> String {
    format!("{}.jsonl", at.format("%Y-%m-%dT%H-%M-%S%.3fZ"))
}

/// Whether `name` looks like a file this module wrote.
fn is_trajectory_file(name: &str) -> bool {
    name.ends_with(".jsonl")
        && name.len() >= "2026-01-01T00-00-00.000Z.jsonl".len()
        && name.as_bytes()[..4].iter().all(u8::is_ascii_digit)
        && name.as_bytes()[10] == b'T'
}

/// Writes one run's trajectory.
pub struct TrajectoryRecorder {
    writer: BufWriter<File>,
    path: PathBuf,
    written: u64,
    seq: u64,
    events: u64,
    truncated: bool,
    failed: bool,
}

impl TrajectoryRecorder {
    /// Create the run's file in `config.dir` and write its `run_start`.
    pub fn create(config: &TrajectoryConfig, meta: &TrajectoryMeta) -> std::io::Result<Self> {
        std::fs::create_dir_all(&config.dir)?;
        let started = Utc::now();
        let base = file_name(started);
        let (file, path) = open_new(&config.dir, &base)?;
        active().insert(path.clone());
        let mut recorder = Self {
            writer: BufWriter::new(file),
            path,
            written: 0,
            seq: 0,
            events: 0,
            truncated: false,
            failed: false,
        };
        recorder.write_record(json!({
            "type": "run_start",
            "pipe": meta.pipe,
            "execution_id": meta.execution_id,
            "trigger": meta.trigger,
            "trigger_event": meta.trigger_event,
            "trigger_key": meta.trigger_key,
            "model": meta.model,
            "provider": meta.provider,
            "preset": meta.preset,
            "attempt": meta.attempt,
            "started_at": started.to_rfc3339_opts(SecondsFormat::Millis, true),
            "prompt": redact_text(&meta.prompt),
            "system_prompt": meta.system_prompt.as_deref().map(redact_text),
        }));
        Ok(recorder)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Record one line of the agent's event stream.
    pub fn agent_event(&mut self, line: &str) {
        self.events += 1;
        if self.truncated || self.failed {
            return;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        let event = if line.len() > MAX_LINE_BYTES {
            let mut cut = MAX_LINE_BYTES;
            while !line.is_char_boundary(cut) {
                cut -= 1;
            }
            json!({ "text": redact_text(&line[..cut]), "cut": true })
        } else {
            match serde_json::from_str::<Value>(line) {
                Ok(v) => redact_value(&v),
                Err(_) => json!({ "text": redact_text(line) }),
            }
        };
        self.write_record(json!({ "type": "agent_event", "event": event }));
    }

    /// Record a scheduler note (for example a model fallback).
    pub fn status(&mut self, note: &Value) {
        if self.truncated || self.failed {
            return;
        }
        self.write_record(json!({ "type": "status", "status": redact_value(note) }));
    }

    /// Write `run_end`, flush and close the file. Returns its path.
    pub fn finish(mut self, end: &TrajectoryEnd) -> PathBuf {
        let record = json!({
            "type": "run_end",
            "success": end.success,
            "error_type": end.error_type,
            "duration_ms": end.duration_ms,
            "events": self.events,
            "truncated": self.truncated,
        });
        // The end record is written even past the size limit.
        self.truncated = false;
        self.write_record(record);
        if let Err(e) = self.writer.flush() {
            warn!("trajectory {}: flush failed: {}", self.path.display(), e);
        }
        active().remove(&self.path);
        self.path.clone()
    }

    fn write_record(&mut self, mut record: Value) {
        if let Value::Object(ref mut map) = record {
            map.insert("seq".into(), json!(self.seq));
            map.insert("at".into(), json!(now_text()));
        }
        self.seq += 1;
        let mut line = serde_json::to_vec(&record).unwrap_or_default();
        line.push(b'\n');
        let is_end = record.get("type").and_then(Value::as_str) == Some("run_end");
        if !is_end && self.written + line.len() as u64 > MAX_FILE_BYTES {
            self.truncated = true;
            let mut note = serde_json::to_vec(&json!({
                "type": "truncated",
                "seq": self.seq,
                "at": now_text(),
                "limit_bytes": MAX_FILE_BYTES,
            }))
            .unwrap_or_default();
            note.push(b'\n');
            self.seq += 1;
            self.raw_write(&note);
            return;
        }
        self.raw_write(&line);
    }

    fn raw_write(&mut self, bytes: &[u8]) {
        if self.failed {
            return;
        }
        match self.writer.write_all(bytes) {
            Ok(()) => self.written += bytes.len() as u64,
            Err(e) => {
                self.failed = true;
                warn!(
                    "trajectory {}: write failed, recording stops: {}",
                    self.path.display(),
                    e
                );
            }
        }
    }
}

impl Drop for TrajectoryRecorder {
    fn drop(&mut self) {
        let _ = self.writer.flush();
        active().remove(&self.path);
    }
}

/// Open a new file `<dir>/<base>`, or `<stem>-<n>.jsonl` when a run started
/// in the same millisecond already has that name.
fn open_new(dir: &Path, base: &str) -> std::io::Result<(File, PathBuf)> {
    let stem = base.trim_end_matches(".jsonl");
    for n in 0..100u32 {
        let name = if n == 0 {
            base.to_string()
        } else {
            format!("{stem}-{n}.jsonl")
        };
        let path = dir.join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free trajectory file name",
    ))
}

/// Delete all but the newest `max_runs` trajectory files in `dir`. Files
/// still being written and files this module did not name are left alone.
/// Returns how many were deleted.
pub fn prune(dir: &Path, max_runs: usize) -> std::io::Result<usize> {
    let mut files: Vec<(String, PathBuf)> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            is_trajectory_file(&name).then(|| (name, entry.path()))
        })
        .collect();
    // Names start with the UTC start time, so they sort by age.
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let busy = active().clone();
    let mut removed = 0;
    for (_, path) in files.into_iter().skip(max_runs.max(1)) {
        if busy.contains(&path) {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(e) => debug!(
                "trajectory prune: could not remove {}: {}",
                path.display(),
                e
            ),
        }
    }
    Ok(removed)
}

/// A run's trajectory as the run paths share it: the stream drainer adds
/// agent events while the run owns the start and the end.
#[derive(Clone)]
pub struct RunTrajectory {
    recorder: Arc<Mutex<Option<TrajectoryRecorder>>>,
    config: TrajectoryConfig,
}

impl RunTrajectory {
    /// Start recording, when trajectories are on. A failure to create the
    /// file is logged and the run goes on without a trajectory.
    pub fn start(config: Option<&TrajectoryConfig>, meta: &TrajectoryMeta) -> Option<Self> {
        let config = config?;
        match TrajectoryRecorder::create(config, meta) {
            Ok(recorder) => {
                debug!(
                    "pipe '{}': recording trajectory to {}",
                    meta.pipe,
                    recorder.path().display()
                );
                Some(Self {
                    recorder: Arc::new(Mutex::new(Some(recorder))),
                    config: config.clone(),
                })
            }
            Err(e) => {
                warn!(
                    "pipe '{}': could not start a trajectory in {}: {}",
                    meta.pipe,
                    config.dir.display(),
                    e
                );
                None
            }
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut TrajectoryRecorder) -> R) -> Option<R> {
        let mut guard = self.recorder.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_mut().map(f)
    }

    pub fn agent_event(&self, line: &str) {
        self.with(|r| r.agent_event(line));
    }

    pub fn status(&self, note: &Value) {
        self.with(|r| r.status(note));
    }

    /// Close the trajectory and prune old ones (off the async threads when a
    /// runtime is there). Later calls do nothing.
    pub fn finish(&self, end: &TrajectoryEnd) -> Option<PathBuf> {
        let recorder = self
            .recorder
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()?;
        let path = recorder.finish(end);
        let dir = self.config.dir.clone();
        let keep = self.config.max_runs;
        let prune_now = move || match prune(&dir, keep) {
            Ok(n) if n > 0 => debug!("trajectories: removed {} old run(s)", n),
            Ok(_) => {}
            Err(e) => warn!("trajectories: pruning {} failed: {}", dir.display(), e),
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn_blocking(prune_now);
            }
            Err(_) => prune_now(),
        }
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn trajectories_are_off_unless_the_variable_is_set() {
        let data = Path::new("/data");
        assert_eq!(TrajectoryConfig::from_lookup(data, env(&[])), None);
        assert_eq!(
            TrajectoryConfig::from_lookup(data, env(&[(SCREENPIPE_TRAJECTORIES_DIR, " ")])),
            None
        );
        assert_eq!(
            TrajectoryConfig::from_lookup(data, env(&[(SCREENPIPE_TRAJECTORIES_DIR, "off")])),
            None
        );
        let on = TrajectoryConfig::from_lookup(data, env(&[(SCREENPIPE_TRAJECTORIES_DIR, "1")]))
            .unwrap();
        assert_eq!(on.dir, data.join("trajectories"));
        assert_eq!(on.max_runs, DEFAULT_MAX_RUNS);
        let rel = TrajectoryConfig::from_lookup(
            data,
            env(&[
                (SCREENPIPE_TRAJECTORIES_DIR, "runs/traj"),
                (SCREENPIPE_TRAJECTORIES_MAX_RUNS, "7"),
            ]),
        )
        .unwrap();
        assert_eq!(rel.dir, data.join("runs/traj"));
        assert_eq!(rel.max_runs, 7);
        let abs = std::env::temp_dir().join("fp-traj");
        let abs_cfg = TrajectoryConfig::from_lookup(
            data,
            env(&[
                (SCREENPIPE_TRAJECTORIES_DIR, abs.to_str().unwrap()),
                (SCREENPIPE_TRAJECTORIES_MAX_RUNS, "0"),
            ]),
        )
        .unwrap();
        assert_eq!(abs_cfg.dir, abs);
        assert_eq!(abs_cfg.max_runs, DEFAULT_MAX_RUNS);
        let huge = TrajectoryConfig::from_lookup(
            data,
            env(&[
                (SCREENPIPE_TRAJECTORIES_DIR, "on"),
                (SCREENPIPE_TRAJECTORIES_MAX_RUNS, "999999999"),
            ]),
        )
        .unwrap();
        assert_eq!(huge.max_runs, MAX_MAX_RUNS);
    }

    fn read_records(path: &Path) -> Vec<Value> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn a_run_is_recorded_line_by_line_without_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let config = TrajectoryConfig {
            dir: dir.path().join("t"),
            max_runs: 5,
        };
        let meta = TrajectoryMeta {
            pipe: "digest".into(),
            execution_id: Some(7),
            trigger: "event".into(),
            trigger_event: Some("meeting_ended".into()),
            trigger_key: Some("42".into()),
            model: "llama3".into(),
            provider: Some("ollama".into()),
            preset: Some("local".into()),
            attempt: 1,
            prompt: "Summarize. Authorization: Bearer abcdef0123456789".into(),
            system_prompt: None,
        };
        let traj = RunTrajectory::start(Some(&config), &meta).unwrap();
        traj.agent_event(r#"{"type":"message_update","delta":"Hel"}"#);
        traj.agent_event(r#"{"type":"tool","args":{"api_key":"sk-live-0000000000000000"}}"#);
        traj.agent_event("plain text line");
        traj.status(&json!({"type": "status", "kind": "model_fallback"}));
        let path = traj
            .finish(&TrajectoryEnd {
                success: true,
                error_type: None,
                duration_ms: 12,
            })
            .unwrap();
        assert!(traj.finish(&TrajectoryEnd::default()).is_none());
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(is_trajectory_file(&name), "{name}");

        let records = read_records(&path);
        let kinds: Vec<&str> = records
            .iter()
            .map(|r| r["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            vec![
                "run_start",
                "agent_event",
                "agent_event",
                "agent_event",
                "status",
                "run_end"
            ]
        );
        assert_eq!(records[0]["pipe"], "digest");
        assert_eq!(records[0]["trigger_key"], "42");
        assert!(!records[0]["prompt"]
            .as_str()
            .unwrap()
            .contains("abcdef0123456789"));
        assert_eq!(records[1]["event"]["delta"], "Hel");
        assert_eq!(records[2]["event"]["args"]["api_key"], "[redacted]");
        assert_eq!(records[3]["event"]["text"], "plain text line");
        assert_eq!(records[5]["events"], 3);
        assert_eq!(records[5]["success"], true);
        for (i, r) in records.iter().enumerate() {
            assert_eq!(r["seq"], i as u64);
            assert!(r["at"].as_str().unwrap().ends_with('Z'));
        }
    }

    #[test]
    fn only_the_newest_runs_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let names = [
            "2026-01-01T00-00-00.000Z.jsonl",
            "2026-01-02T00-00-00.000Z.jsonl",
            "2026-01-03T00-00-00.000Z.jsonl",
            "2026-01-03T00-00-00.000Z-1.jsonl",
        ];
        for n in names {
            std::fs::write(dir.path().join(n), "{}\n").unwrap();
        }
        // Not ours: never touched.
        std::fs::write(dir.path().join("notes.jsonl"), "x").unwrap();
        assert_eq!(prune(dir.path(), 2).unwrap(), 2);
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                "2026-01-03T00-00-00.000Z-1.jsonl".to_string(),
                "2026-01-03T00-00-00.000Z.jsonl".to_string(),
                "notes.jsonl".to_string()
            ]
        );
    }

    #[test]
    fn oversized_runs_stop_writing_but_still_end() {
        let dir = tempfile::tempdir().unwrap();
        let config = TrajectoryConfig {
            dir: dir.path().to_path_buf(),
            max_runs: 5,
        };
        let mut rec = TrajectoryRecorder::create(&config, &TrajectoryMeta::default()).unwrap();
        // Pretend the file is almost full.
        rec.written = MAX_FILE_BYTES - 10;
        rec.agent_event(r#"{"type":"x"}"#);
        rec.agent_event(r#"{"type":"y"}"#);
        let path = rec.finish(&TrajectoryEnd::default());
        let records = read_records(&path);
        let kinds: Vec<&str> = records
            .iter()
            .map(|r| r["type"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, vec!["run_start", "truncated", "run_end"]);
        assert_eq!(records[2]["events"], 2);
        assert_eq!(records[2]["truncated"], true);
    }
}
