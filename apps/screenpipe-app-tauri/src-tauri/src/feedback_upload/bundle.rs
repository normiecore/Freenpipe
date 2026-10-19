// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Writes the diagnostics zip.
//!
//! Pure file work: no Tauri handle, no network. The caller gathers the
//! runtime pieces (versions, health snapshot, settings) and hands them over
//! in a [`BundleInput`]. The zip is first written as `<name>.zip.partial`
//! and renamed when complete, so a half-written bundle never looks finished.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use base64::Engine as _;
use chrono::{DateTime, Datelike, Local, Timelike};
use serde::Serialize;
use serde_json::{json, Value};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::redact::{mask_sensitive_settings, FeedbackRedactor};
use crate::log_files::read_tail;

/// Sub-directory of the data dir that holds bundles.
pub const FEEDBACK_DIR: &str = "feedback";
/// Directory (in the data dir) where crash dumps are harvested.
pub const CRASH_DUMP_DIR: &str = "crash-dumps";

/// Bytes kept from the end of each log file.
pub const LOG_TAIL_BYTES: u64 = 1024 * 1024;
/// Text budget for all log tails together, before compression.
pub const LOG_BUDGET_BYTES: u64 = 24 * 1024 * 1024;
/// Newest files kept per log family.
const APP_LOGS: usize = 5;
const ENGINE_LOGS: usize = 4;
const OTHER_LOGS: usize = 4;
/// Newest crash dumps kept, and the size above which one is skipped.
const CRASH_DUMPS: usize = 3;
const MAX_CRASH_DUMP_BYTES: u64 = 64 * 1024 * 1024;
/// Screen recording attachment limit.
pub const MAX_VIDEO_BYTES: u64 = 50 * 1024 * 1024;
/// Screenshot attachment limit (decoded).
const MAX_SCREENSHOT_BYTES: usize = 10 * 1024 * 1024;
/// Caps on text the UI hands over.
const MAX_FEEDBACK_BYTES: usize = 64 * 1024;
const MAX_CONSOLE_BYTES: usize = 1024 * 1024;
const MAX_CHAT_BYTES: usize = 256 * 1024;
/// Bundles kept in the feedback directory; older ones are deleted.
pub const KEEP_BUNDLES: usize = 10;

/// Everything the bundle writer needs.
#[derive(Debug, Clone)]
pub struct BundleInput {
    /// Data directory: logs are read from here and the bundle goes to
    /// `<data_dir>/feedback/`.
    pub data_dir: PathBuf,
    /// Further directories to look for logs in (the configured data dir
    /// when it differs from the default one).
    pub extra_log_dirs: Vec<PathBuf>,
    pub created_at: DateTime<Local>,
    pub app_version: String,
    pub os_platform: String,
    pub os_version: String,
    pub os_arch: String,
    /// Where the report came from (`feedback`, `server-not-running`, ...).
    pub source: String,
    pub feedback_text: String,
    pub console_log: Option<String>,
    pub chat_history: Option<String>,
    pub screenshot_data_url: Option<String>,
    pub video_path: Option<PathBuf>,
    /// Settings to include, masked here whatever their origin.
    pub settings: Option<Value>,
    /// Health snapshot gathered by the caller.
    pub health: Value,
}

/// One file inside the zip.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleEntry {
    pub name: String,
    pub bytes: u64,
    /// Only the end of the source was kept.
    pub truncated: bool,
    /// Text that went through the scrubber.
    pub redacted: bool,
}

/// Something that was considered and left out, with the reason.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkippedItem {
    pub name: String,
    pub reason: String,
}

/// What [`write_bundle`] produced.
#[derive(Debug, Clone)]
pub struct BundleSummary {
    pub path: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
    pub entries: Vec<BundleEntry>,
    pub skipped: Vec<SkippedItem>,
}

/// Log families, used to pick the newest few of each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogKind {
    App,
    Engine,
    Panic,
    Update,
    Recovery,
    Other,
}

/// Which family a top-level data-dir file belongs to, if any.
pub fn classify_log(name: &str) -> Option<LogKind> {
    let lower = name.to_ascii_lowercase();
    let is_log = lower.ends_with(".log") || lower.contains(".log.");
    if lower.starts_with("last-panic.log") {
        return Some(LogKind::Panic);
    }
    if lower.starts_with("update-") && (is_log || lower.ends_with(".json")) {
        return Some(LogKind::Update);
    }
    if !is_log {
        return None;
    }
    if lower.contains("recovery") {
        Some(LogKind::Recovery)
    } else if lower.starts_with("screenpipe-app") {
        Some(LogKind::App)
    } else if lower.starts_with("screenpipe") {
        Some(LogKind::Engine)
    } else {
        Some(LogKind::Other)
    }
}

fn keep_per_kind(kind: LogKind) -> usize {
    match kind {
        LogKind::App => APP_LOGS,
        LogKind::Engine => ENGINE_LOGS,
        LogKind::Other => OTHER_LOGS,
        LogKind::Panic | LogKind::Update | LogKind::Recovery => usize::MAX,
    }
}

#[derive(Debug, Clone)]
struct Candidate {
    path: PathBuf,
    name: String,
    kind: LogKind,
    modified: SystemTime,
    len: u64,
}

/// Regular files directly inside `dir` (symlinks and directories skipped,
/// so a planted link cannot pull an unrelated file into the bundle).
fn regular_files(dir: &Path) -> Vec<(PathBuf, String, fs::Metadata)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let meta = fs::symlink_metadata(entry.path()).ok()?;
            if !meta.file_type().is_file() {
                return None;
            }
            let name = entry.file_name().to_str()?.to_string();
            Some((entry.path(), name, meta))
        })
        .collect()
}

/// Logs to include, newest first within each family.
fn select_logs(dirs: &[PathBuf]) -> Vec<Candidate> {
    let mut seen_dirs = BTreeSet::new();
    let mut all: Vec<Candidate> = Vec::new();
    for dir in dirs {
        let key = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        if !seen_dirs.insert(key) {
            continue;
        }
        for (path, name, meta) in regular_files(dir) {
            if let Some(kind) = classify_log(&name) {
                all.push(Candidate {
                    path,
                    name,
                    kind,
                    modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    len: meta.len(),
                });
            }
        }
    }
    all.sort_by(|a, b| a.kind.cmp(&b.kind).then(b.modified.cmp(&a.modified)));
    let mut picked = Vec::new();
    let mut count = 0usize;
    let mut current: Option<LogKind> = None;
    for c in all {
        if current != Some(c.kind) {
            current = Some(c.kind);
            count = 0;
        }
        if count < keep_per_kind(c.kind) {
            picked.push(c);
            count += 1;
        }
    }
    picked
}

/// Newest crash dumps in `<dir>/crash-dumps/`.
fn select_crash_dumps(dirs: &[PathBuf]) -> Vec<Candidate> {
    let mut seen = BTreeSet::new();
    let mut dumps: Vec<Candidate> = Vec::new();
    for dir in dirs {
        let dump_dir = dir.join(CRASH_DUMP_DIR);
        let key = dump_dir.canonicalize().unwrap_or_else(|_| dump_dir.clone());
        if !seen.insert(key) {
            continue;
        }
        for (path, name, meta) in regular_files(&dump_dir) {
            if name.to_ascii_lowercase().ends_with(".dmp") {
                dumps.push(Candidate {
                    path,
                    name,
                    kind: LogKind::Other,
                    modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    len: meta.len(),
                });
            }
        }
    }
    dumps.sort_by(|a, b| b.modified.cmp(&a.modified));
    dumps
}

/// Keep at most `max` bytes of `text`, from the end, on a char boundary.
fn tail_str(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    (&text[start..], true)
}

/// Keep at most `max` bytes of `text`, from the start, on a char boundary.
fn head_str(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

/// Decode a `data:image/<type>;base64,...` URL into bytes and an extension.
pub fn decode_screenshot(data_url: &str) -> Result<(Vec<u8>, &'static str), String> {
    let rest = data_url
        .strip_prefix("data:")
        .ok_or("screenshot is not a data URL")?;
    let (meta, payload) = rest.split_once(',').ok_or("screenshot data URL has no payload")?;
    let meta = meta.to_ascii_lowercase();
    if !meta.ends_with(";base64") {
        return Err("screenshot data URL is not base64".into());
    }
    let ext = match meta.trim_end_matches(";base64") {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        other => return Err(format!("unsupported screenshot type {other}")),
    };
    if payload.len() > MAX_SCREENSHOT_BYTES / 3 * 4 + 4 {
        return Err("screenshot is larger than 10 MB".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|e| format!("screenshot is not valid base64: {e}"))?;
    Ok((bytes, ext))
}

/// Check the recording attachment and return its zip extension.
pub fn check_video(path: &Path) -> Result<(&'static str, u64), String> {
    let ext = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("mp4") => "mp4",
        Some("mov") => "mov",
        _ => return Err("recording must be an .mp4 or .mov file".into()),
    };
    let meta = fs::symlink_metadata(path).map_err(|e| format!("recording not readable: {e}"))?;
    if !meta.file_type().is_file() {
        return Err("recording is not a regular file".into());
    }
    if meta.len() > MAX_VIDEO_BYTES {
        return Err(format!(
            "recording is {} MB, the limit is {} MB",
            meta.len().div_ceil(1024 * 1024),
            MAX_VIDEO_BYTES / (1024 * 1024)
        ));
    }
    Ok((ext, meta.len()))
}

/// `<YYYYMMDD-HHMMSS>.zip`, with `-2`, `-3`, ... when a bundle of that
/// second already exists.
fn bundle_file_name(dir: &Path, created_at: &DateTime<Local>) -> String {
    let stamp = created_at.format("%Y%m%d-%H%M%S").to_string();
    let mut name = format!("{stamp}.zip");
    let mut n = 2;
    while dir.join(&name).exists() || dir.join(format!("{name}.partial")).exists() {
        name = format!("{stamp}-{n}.zip");
        n += 1;
    }
    name
}

/// True for names [`bundle_file_name`] produces.
pub fn is_bundle_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".zip") else {
        return false;
    };
    let (stamp, suffix) = match stem.split_once('-') {
        Some((date, rest)) => match rest.split_once('-') {
            Some((time, n)) => (format!("{date}-{time}"), Some(n)),
            None => (stem.to_string(), None),
        },
        None => return false,
    };
    let b = stamp.as_bytes();
    let stamp_ok = b.len() == 15
        && b[8] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 8 || c.is_ascii_digit());
    let suffix_ok = suffix.is_none_or(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()));
    stamp_ok && suffix_ok
}

/// Delete all but the newest `keep` bundles, and any leftover partial file.
/// Only files this module names are touched.
pub fn prune_bundles(dir: &Path, keep: usize) -> Vec<PathBuf> {
    let mut bundles: Vec<(SystemTime, PathBuf)> = Vec::new();
    let mut removed = Vec::new();
    for (path, name, meta) in regular_files(dir) {
        if name.ends_with(".zip.partial") && is_bundle_name(name.trim_end_matches(".partial")) {
            if fs::remove_file(&path).is_ok() {
                removed.push(path);
            }
        } else if is_bundle_name(&name) {
            bundles.push((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), path));
        }
    }
    // Newest first; the name breaks ties because it carries the timestamp.
    bundles.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    for (_, path) in bundles.into_iter().skip(keep) {
        if fs::remove_file(&path).is_ok() {
            removed.push(path);
        }
    }
    removed
}

struct Writer<'a> {
    zip: ZipWriter<BufWriter<File>>,
    entries: Vec<BundleEntry>,
    names: BTreeSet<String>,
    time: zip::DateTime,
    redactor: &'a FeedbackRedactor,
}

impl Writer<'_> {
    fn options(&self, method: CompressionMethod) -> SimpleFileOptions {
        SimpleFileOptions::default()
            .compression_method(method)
            .last_modified_time(self.time)
            .unix_permissions(0o644)
    }

    /// Pick a unique entry name (`logs/a.log`, then `logs/a (2).log`).
    fn unique(&mut self, name: &str) -> String {
        if self.names.insert(name.to_string()) {
            return name.to_string();
        }
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > name.rfind('/').map_or(0, |s| s + 1) => (&name[..i], &name[i..]),
            _ => (name, ""),
        };
        let mut n = 2;
        loop {
            let candidate = format!("{stem} ({n}){ext}");
            if self.names.insert(candidate.clone()) {
                return candidate;
            }
            n += 1;
        }
    }

    fn add_bytes(
        &mut self,
        name: &str,
        bytes: &[u8],
        method: CompressionMethod,
        truncated: bool,
        redacted: bool,
    ) -> io::Result<()> {
        let name = self.unique(name);
        let options = self.options(method);
        self.zip.start_file(name.as_str(), options).map_err(io::Error::other)?;
        self.zip.write_all(bytes)?;
        self.entries.push(BundleEntry {
            name,
            bytes: bytes.len() as u64,
            truncated,
            redacted,
        });
        Ok(())
    }

    /// Scrub `text` and add it.
    fn add_text(&mut self, name: &str, text: &str, truncated: bool) -> io::Result<()> {
        let clean = self.redactor.redact(text);
        self.add_bytes(name, clean.as_bytes(), CompressionMethod::Deflated, truncated, true)
    }

    fn add_file(&mut self, name: &str, path: &Path, method: CompressionMethod) -> io::Result<()> {
        let name = self.unique(name);
        let options = self.options(method);
        let mut file = File::open(path)?;
        self.zip.start_file(name.as_str(), options).map_err(io::Error::other)?;
        let copied = io::copy(&mut file, &mut self.zip)?;
        self.entries.push(BundleEntry {
            name,
            bytes: copied,
            truncated: false,
            redacted: false,
        });
        Ok(())
    }
}

fn zip_time(t: &DateTime<Local>) -> zip::DateTime {
    zip::DateTime::from_date_and_time(
        t.year().clamp(1980, 2107) as u16,
        t.month() as u8,
        t.day() as u8,
        t.hour() as u8,
        t.minute() as u8,
        t.second() as u8,
    )
    .unwrap_or_default()
}

/// Build the bundle. Returns an error only when the zip itself cannot be
/// written; unreadable sources are listed in `skipped` instead.
pub fn write_bundle(input: &BundleInput, redactor: &FeedbackRedactor) -> io::Result<BundleSummary> {
    let out_dir = input.data_dir.join(FEEDBACK_DIR);
    fs::create_dir_all(&out_dir)?;
    let file_name = bundle_file_name(&out_dir, &input.created_at);
    let final_path = out_dir.join(&file_name);
    let partial_path = out_dir.join(format!("{file_name}.partial"));

    let result = write_into(&partial_path, input, redactor);
    match result {
        Ok((entries, skipped)) => {
            fs::rename(&partial_path, &final_path)?;
            let size_bytes = fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
            Ok(BundleSummary {
                path: final_path,
                file_name,
                size_bytes,
                entries,
                skipped,
            })
        }
        Err(e) => {
            let _ = fs::remove_file(&partial_path);
            Err(e)
        }
    }
}

fn write_into(
    partial_path: &Path,
    input: &BundleInput,
    redactor: &FeedbackRedactor,
) -> io::Result<(Vec<BundleEntry>, Vec<SkippedItem>)> {
    let file = File::create(partial_path)?;
    let mut w = Writer {
        zip: ZipWriter::new(BufWriter::new(file)),
        entries: Vec::new(),
        names: BTreeSet::new(),
        time: zip_time(&input.created_at),
        redactor,
    };
    let mut skipped: Vec<SkippedItem> = Vec::new();
    let skip = |skipped: &mut Vec<SkippedItem>, name: &str, reason: String| {
        skipped.push(SkippedItem {
            name: name.to_string(),
            reason,
        })
    };

    // README first, so it is the first thing a reader sees.
    w.add_bytes(
        "README.txt",
        readme(input).as_bytes(),
        CompressionMethod::Deflated,
        false,
        false,
    )?;

    // The user's own words.
    let (feedback, cut) = head_str(input.feedback_text.trim(), MAX_FEEDBACK_BYTES);
    if !feedback.is_empty() {
        w.add_text("feedback.txt", &format!("{feedback}\n"), cut)?;
    }

    // Settings, masked, then scrubbed as text for anything the key rule missed.
    match &input.settings {
        Some(settings) => {
            let masked = mask_sensitive_settings(settings);
            let pretty = serde_json::to_string_pretty(&masked).unwrap_or_default();
            w.add_text("settings.json", &format!("{pretty}\n"), false)?;
        }
        None => skip(&mut skipped, "settings.json", "settings could not be read".into()),
    }

    let health = serde_json::to_string_pretty(&input.health).unwrap_or_default();
    w.add_text("health.json", &format!("{health}\n"), false)?;

    // Logs.
    let mut dirs = vec![input.data_dir.clone()];
    dirs.extend(input.extra_log_dirs.iter().cloned());
    let mut budget = LOG_BUDGET_BYTES;
    for log in select_logs(&dirs) {
        let zip_name = format!("logs/{}", log.name);
        if budget == 0 {
            skip(&mut skipped, &zip_name, "log size budget used up".into());
            continue;
        }
        if log.len == 0 {
            continue;
        }
        match read_tail(&log.path, LOG_TAIL_BYTES.min(budget)) {
            Ok(tail) => {
                budget = budget.saturating_sub(tail.shown_bytes);
                let truncated = tail.truncated();
                w.add_text(&zip_name, &tail.with_marker(), truncated)?;
            }
            Err(e) => skip(&mut skipped, &zip_name, format!("could not read: {e}")),
        }
    }

    // Crash dumps are binary process memory: copied as-is, never parsed.
    for (i, dump) in select_crash_dumps(&dirs).into_iter().enumerate() {
        let zip_name = format!("{CRASH_DUMP_DIR}/{}", dump.name);
        if i >= CRASH_DUMPS {
            skip(&mut skipped, &zip_name, format!("only the newest {CRASH_DUMPS} dumps are kept"));
        } else if dump.len > MAX_CRASH_DUMP_BYTES {
            skip(&mut skipped, &zip_name, "larger than 64 MB".into());
        } else if let Err(e) = w.add_file(&zip_name, &dump.path, CompressionMethod::Deflated) {
            skip(&mut skipped, &zip_name, format!("could not read: {e}"));
        }
    }

    // What the UI collected.
    if let Some(console) = input.console_log.as_deref().filter(|c| !c.trim().is_empty()) {
        let (console, cut) = tail_str(console, MAX_CONSOLE_BYTES);
        w.add_text("console.log", console, cut)?;
    }
    if let Some(chat) = input.chat_history.as_deref().filter(|c| !c.trim().is_empty()) {
        let (chat, cut) = head_str(chat, MAX_CHAT_BYTES);
        w.add_text("chat-history.txt", chat, cut)?;
    }
    if let Some(url) = input.screenshot_data_url.as_deref().filter(|u| !u.is_empty()) {
        match decode_screenshot(url) {
            Ok((bytes, ext)) => w.add_bytes(
                &format!("attachments/screenshot.{ext}"),
                &bytes,
                CompressionMethod::Stored,
                false,
                false,
            )?,
            Err(reason) => skip(&mut skipped, "attachments/screenshot", reason),
        }
    }
    if let Some(video) = &input.video_path {
        match check_video(video) {
            Ok((ext, _)) => {
                let name = format!("attachments/recording.{ext}");
                if let Err(e) = w.add_file(&name, video, CompressionMethod::Stored) {
                    skip(&mut skipped, &name, format!("could not read: {e}"));
                }
            }
            Err(reason) => skip(&mut skipped, "attachments/recording", reason),
        }
    }

    // The manifest goes last so it can list everything above.
    let report = json!({
        "format": 1,
        "createdAt": input.created_at.to_rfc3339(),
        "app": { "name": "Freenpipe", "version": input.app_version },
        "os": {
            "platform": input.os_platform,
            "version": input.os_version,
            "arch": input.os_arch,
        },
        "source": input.source,
        "uploaded": false,
        "entries": w.entries,
        "skipped": skipped,
    });
    let report = serde_json::to_string_pretty(&report).unwrap_or_default();
    w.add_text("report.json", &format!("{report}\n"), false)?;

    let mut inner = w.zip.finish().map_err(io::Error::other)?;
    inner.flush()?;
    inner.get_ref().sync_all()?;
    Ok((w.entries, skipped))
}

fn readme(input: &BundleInput) -> String {
    format!(
        "Freenpipe diagnostics bundle\n\
         ============================\n\
         \n\
         Created {created} by Freenpipe {version} on {platform} {os_version} ({arch}).\n\
         Reason: {source}.\n\
         \n\
         This file was written on your computer and has not been sent anywhere.\n\
         Share it only if you choose to, for example by attaching it to an issue at\n\
         https://github.com/normiecore/Freenpipe/issues\n\
         \n\
         What is inside\n\
         - feedback.txt: what you wrote, if anything.\n\
         - logs/: the end of recent app and engine logs, panic, update and recovery logs.\n\
         - settings.json: your settings, with keys that look like secrets, accounts,\n\
           prompts, paths, URLs, hosts or window lists replaced by placeholders.\n\
         - health.json: engine and recording status when the bundle was made.\n\
         - console.log, chat-history.txt: only when the app supplied them.\n\
         - crash-dumps/: Windows crash dumps, if any were collected.\n\
         - attachments/: a screenshot or screen recording, only if you added one.\n\
         - report.json: the list of files above and anything that was left out.\n\
         \n\
         Before sharing\n\
         All text files were scrubbed on this computer: e-mail addresses, phone numbers,\n\
         API keys and tokens, ID and card numbers, IP addresses (except loopback), the\n\
         user name in home-folder paths, your login name, and your own name, e-mail and\n\
         computer name were replaced by placeholders such as [EMAIL] or [USER].\n\
         Automatic scrubbing can miss things, so read the text files before you\n\
         share them.\n\
         Crash dumps, screenshots and recordings are copied as they are and are NOT\n\
         scrubbed; they can show anything that was on screen or in memory.\n",
        created = input.created_at.to_rfc3339(),
        version = input.app_version,
        platform = input.os_platform,
        os_version = input.os_version,
        arch = input.os_arch,
        source = input.source,
    )
}

/// Read one entry back out of a bundle (tests only).
#[cfg(test)]
fn read_entry(zip_path: &Path, name: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = File::open(zip_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name(name).ok()?;
    let mut out = Vec::new();
    entry.read_to_end(&mut out).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::io::Write as _;

    fn input(data_dir: &Path) -> BundleInput {
        BundleInput {
            data_dir: data_dir.to_path_buf(),
            extra_log_dirs: Vec::new(),
            created_at: Local.with_ymd_and_hms(2026, 10, 15, 14, 30, 5).unwrap(),
            app_version: "2.5.28".into(),
            os_platform: "windows".into(),
            os_version: "10.0.22631".into(),
            os_arch: "x86_64".into(),
            source: "feedback".into(),
            feedback_text: "Timeline is blank. Reach me at ada@example.org".into(),
            console_log: Some("console says hi from 10.1.2.3\n".into()),
            chat_history: None,
            screenshot_data_url: None,
            video_path: None,
            settings: Some(json!({ "port": 3030, "openaiApiKey": "sk-abc-should-not-leak", "fps": 1 })),
            health: json!({ "engine": { "status": "healthy" } }),
        }
    }

    fn write(path: &Path, body: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        File::create(path).unwrap().write_all(body).unwrap();
    }

    fn names(zip_path: &Path) -> Vec<String> {
        let archive = zip::ZipArchive::new(File::open(zip_path).unwrap()).unwrap();
        archive.file_names().map(str::to_string).collect()
    }

    fn text(zip_path: &Path, name: &str) -> String {
        String::from_utf8(read_entry(zip_path, name).unwrap_or_else(|| panic!("{name} missing")))
            .unwrap()
    }

    #[test]
    fn classifies_data_dir_files() {
        assert_eq!(classify_log("screenpipe-app.2026-10-15.log"), Some(LogKind::App));
        assert_eq!(classify_log("screenpipe.2026-10-15.0.log"), Some(LogKind::Engine));
        assert_eq!(classify_log("last-panic.log"), Some(LogKind::Panic));
        assert_eq!(classify_log("last-panic.log.prev"), Some(LogKind::Panic));
        assert_eq!(classify_log("update-install.log"), Some(LogKind::Update));
        assert_eq!(classify_log("update-attempt.json"), Some(LogKind::Update));
        assert_eq!(classify_log("recording-recovery.log"), Some(LogKind::Recovery));
        assert_eq!(classify_log("pipe-runner.log"), Some(LogKind::Other));
        assert_eq!(classify_log("db.sqlite"), None);
        assert_eq!(classify_log("store.bin"), None);
        assert_eq!(classify_log("auth.json"), None);
    }

    #[test]
    fn writes_a_complete_scrubbed_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        write(&d.join("screenpipe-app.2026-10-15.log"), b"INFO started\nuser ada@example.org logged C:\\Users\\ada\\x\n");
        write(&d.join("screenpipe.2026-10-15.log"), b"engine line\n");
        write(&d.join("last-panic.log"), b"PANIC at main.rs\n");
        write(&d.join("last-panic.log.prev"), b"older panic\n");
        write(&d.join("update-recovery.log"), b"update failed\n");
        write(&d.join("recording-recovery.log"), b"recovered 3 chunks\n");
        write(&d.join("db.sqlite"), b"not a log");
        write(&d.join(CRASH_DUMP_DIR).join("app-1.dmp"), b"MDMP fake dump");

        let summary = write_bundle(&input(d), &FeedbackRedactor::default()).unwrap();
        assert_eq!(summary.file_name, "20261015-143005.zip");
        assert_eq!(summary.path, d.join(FEEDBACK_DIR).join("20261015-143005.zip"));
        assert!(summary.size_bytes > 0);
        assert!(!d.join(FEEDBACK_DIR).join("20261015-143005.zip.partial").exists());

        let entries = names(&summary.path);
        for expected in [
            "README.txt",
            "feedback.txt",
            "settings.json",
            "health.json",
            "logs/screenpipe-app.2026-10-15.log",
            "logs/screenpipe.2026-10-15.log",
            "logs/last-panic.log",
            "logs/last-panic.log.prev",
            "logs/update-recovery.log",
            "logs/recording-recovery.log",
            "crash-dumps/app-1.dmp",
            "console.log",
            "report.json",
        ] {
            assert!(entries.iter().any(|n| n == expected), "{expected} missing from {entries:?}");
        }
        assert!(!entries.iter().any(|n| n.contains("db.sqlite")));

        let app_log = text(&summary.path, "logs/screenpipe-app.2026-10-15.log");
        assert!(!app_log.contains("ada@example.org"), "{app_log}");
        assert!(!app_log.contains(r"\Users\ada"), "{app_log}");
        assert!(app_log.contains("INFO started"));

        let settings = text(&summary.path, "settings.json");
        assert!(!settings.contains("sk-abc-should-not-leak"), "{settings}");
        assert!(settings.contains("3030"));

        assert!(!text(&summary.path, "feedback.txt").contains("ada@example.org"));
        assert!(!text(&summary.path, "console.log").contains("10.1.2.3"));
        assert_eq!(read_entry(&summary.path, "crash-dumps/app-1.dmp").unwrap(), b"MDMP fake dump");

        let report: Value = serde_json::from_str(&text(&summary.path, "report.json")).unwrap();
        assert_eq!(report["uploaded"], json!(false));
        assert_eq!(report["source"], json!("feedback"));
        assert!(report["entries"].as_array().unwrap().len() >= 12);
    }

    #[test]
    fn long_logs_are_tailed_with_a_marker() {
        let dir = tempfile::tempdir().unwrap();
        let line = "2026-10-15T00:00:00Z INFO filler line for the tail test\n";
        let body = line.repeat((LOG_TAIL_BYTES as usize / line.len()) + 500);
        write(&dir.path().join("screenpipe-app.2026-10-15.log"), body.as_bytes());
        let summary = write_bundle(&input(dir.path()), &FeedbackRedactor::default()).unwrap();
        let log = text(&summary.path, "logs/screenpipe-app.2026-10-15.log");
        assert!(log.starts_with("[earlier lines omitted"), "{}", &log[..80]);
        assert!(log.len() as u64 <= LOG_TAIL_BYTES + 200);
        let entry = summary
            .entries
            .iter()
            .find(|e| e.name == "logs/screenpipe-app.2026-10-15.log")
            .unwrap();
        assert!(entry.truncated && entry.redacted);
    }

    #[test]
    fn keeps_only_the_newest_logs_of_each_family() {
        let dir = tempfile::tempdir().unwrap();
        for day in 1..=8 {
            let path = dir.path().join(format!("screenpipe-app.2026-10-0{day}.log"));
            write(&path, b"x\n");
            let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + day * 100);
            File::options().write(true).open(&path).unwrap().set_modified(t).unwrap();
        }
        let picked = select_logs(&[dir.path().to_path_buf()]);
        let names: Vec<_> = picked.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names.len(), APP_LOGS);
        assert_eq!(names[0], "screenpipe-app.2026-10-08.log");
        assert!(!names.contains(&"screenpipe-app.2026-10-01.log"));
    }

    #[test]
    fn attachments_are_added_or_explained() {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("clip.mp4");
        write(&video, b"fake mp4");
        let mut inp = input(dir.path());
        // 1x1 transparent PNG.
        inp.screenshot_data_url = Some("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=".into());
        inp.video_path = Some(video);
        let summary = write_bundle(&inp, &FeedbackRedactor::default()).unwrap();
        let entries = names(&summary.path);
        assert!(entries.contains(&"attachments/screenshot.png".to_string()));
        assert!(entries.contains(&"attachments/recording.mp4".to_string()));

        let mut bad = input(dir.path());
        bad.screenshot_data_url = Some("https://example.org/x.png".into());
        bad.video_path = Some(dir.path().join("notes.txt"));
        let summary = write_bundle(&bad, &FeedbackRedactor::default()).unwrap();
        let reasons: Vec<_> = summary.skipped.iter().map(|s| s.name.as_str()).collect();
        assert!(reasons.contains(&"attachments/screenshot"));
        assert!(reasons.contains(&"attachments/recording"));
    }

    #[test]
    fn video_rules() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("a.MOV");
        write(&ok, b"x");
        assert_eq!(check_video(&ok).unwrap().0, "mov");
        assert!(check_video(&dir.path().join("a.exe")).is_err());
        assert!(check_video(&dir.path().join("missing.mp4")).is_err());
        let big = dir.path().join("big.mp4");
        File::create(&big).unwrap().set_len(MAX_VIDEO_BYTES + 1).unwrap();
        assert!(check_video(&big).unwrap_err().contains("limit"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_logs_are_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("id_rsa");
        write(&secret, b"PRIVATE");
        std::os::unix::fs::symlink(&secret, dir.path().join("screenpipe-app.evil.log")).unwrap();
        assert!(select_logs(&[dir.path().to_path_buf()]).is_empty());
    }

    #[test]
    fn bundle_names_are_unique_and_recognised() {
        let dir = tempfile::tempdir().unwrap();
        let t = Local.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        let first = bundle_file_name(dir.path(), &t);
        assert_eq!(first, "20260102-030405.zip");
        write(&dir.path().join(&first), b"");
        let second = bundle_file_name(dir.path(), &t);
        assert_eq!(second, "20260102-030405-2.zip");
        assert!(is_bundle_name(&first));
        assert!(is_bundle_name(&second));
        for not_ours in ["notes.zip", "2026-01-02.zip", "20260102-030405.tar", "20260102-03040x.zip", "20260102-030405-.zip"] {
            assert!(!is_bundle_name(not_ours), "{not_ours}");
        }
    }

    #[test]
    fn prune_keeps_the_newest_and_ignores_other_files() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..13u64 {
            let path = dir.path().join(format!("20260101-0000{:02}.zip", i));
            write(&path, b"z");
            let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + i);
            File::options().write(true).open(&path).unwrap().set_modified(t).unwrap();
        }
        write(&dir.path().join("20260101-000099.zip.partial"), b"half");
        write(&dir.path().join("keep-me.zip"), b"user file");
        let removed = prune_bundles(dir.path(), KEEP_BUNDLES);
        assert_eq!(removed.len(), 4);
        assert!(dir.path().join("keep-me.zip").exists());
        assert!(dir.path().join("20260101-000012.zip").exists());
        assert!(!dir.path().join("20260101-000000.zip").exists());
        assert!(!dir.path().join("20260101-000099.zip.partial").exists());
    }

    #[test]
    fn text_caps_respect_char_boundaries() {
        // "aé" is three bytes: 'a' at 0, 'é' at 1..3, then 'a' at 3, 'é' at 4..6.
        let s = "aé".repeat(10);
        // A limit on a boundary keeps exactly that many bytes.
        assert_eq!(head_str(&s, 4), ("aéa", true));
        // A limit inside an 'é' steps back to the start of it.
        assert_eq!(head_str(&s, 5), ("aéa", true));
        assert_eq!(head_str(&s, 2), ("a", true));
        assert_eq!(head_str(&s, 100), (s.as_str(), false));

        // 30 bytes; the last 4 start at byte 26, inside an 'é', so the tail
        // moves forward to the next character.
        let (tail, cut) = tail_str(&s, 4);
        assert!(cut);
        assert_eq!(tail, "aé");
        assert!(s.ends_with(tail));
        assert_eq!(tail_str(&s, 100), (s.as_str(), false));
    }
}
