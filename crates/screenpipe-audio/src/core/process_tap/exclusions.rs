// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Platform-neutral reader for `audio-exclusions.json`.
//!
//! The file is shared with the desktop app (`audio_exclusions.rs` writes it)
//! and with the macOS Process Tap backend, which keeps its own reader in
//! `macos.rs`. Its shape is
//!
//! ```json
//! { "excluded_apps": [{ "bundle_id": "spotify.exe", "name": "Spotify", "icon": "data:..." }] }
//! ```
//!
//! On Windows `bundle_id` carries the executable file name. The Windows
//! loopback engine only needs the ids, in file order, plus the file's mtime
//! so it can notice edits cheaply. Every failure (missing file, unreadable
//! file, malformed JSON, wrong shape) reads as an empty list: losing the
//! exclusion filter is far better for the user than losing system audio.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Environment variable that overrides the exclusion file location. The
/// desktop app honours the same variable, so both sides agree on the path.
pub const ENV_OVERRIDE: &str = "SCREENPIPE_AUDIO_EXCLUSIONS_PATH";

/// Location of the file relative to the user's home directory.
pub const DEFAULT_RELATIVE_PATH: &str = ".screenpipe/audio-exclusions.json";

/// The exclusion file path: the env override when set and non-empty,
/// otherwise `<home>/.screenpipe/audio-exclusions.json`.
pub fn config_path() -> PathBuf {
    let override_val = std::env::var(ENV_OVERRIDE).ok();
    resolve_path(override_val.as_deref(), dirs::home_dir().as_deref())
}

/// Pure path resolution, separated so tests never touch the real home
/// directory or the process environment.
pub fn resolve_path(env_override: Option<&str>, home: Option<&Path>) -> PathBuf {
    if let Some(p) = env_override.map(str::trim).filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    home.map(Path::to_path_buf)
        .unwrap_or_default()
        .join(DEFAULT_RELATIVE_PATH)
}

/// The `bundle_id` values of a parsed exclusion file, in file order.
/// Entries without a string `bundle_id`, and blank ids, are skipped.
pub fn parse_bundle_ids(body: &str) -> Vec<String> {
    let parsed: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    parsed
        .get("excluded_apps")
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("bundle_id").and_then(|v| v.as_str()))
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Reads the ids and the file's mtime. A missing file gives `(vec![], None)`;
/// an unreadable or malformed file gives an empty list with its mtime.
pub fn read_bundle_ids(path: &Path) -> (Vec<String>, Option<SystemTime>) {
    let Ok(meta) = fs::metadata(path) else {
        return (Vec::new(), None);
    };
    let mtime = meta.modified().ok();
    match fs::read_to_string(path) {
        Ok(body) => (parse_bundle_ids(&body), mtime),
        Err(_) => (Vec::new(), mtime),
    }
}

/// Turns a stored `bundle_id` into the lower-case executable file name the
/// Windows process list reports. A full path keeps only its file name, and a
/// bare name without an extension gets `.exe` (so `chrome` matches
/// `chrome.exe`). Returns `None` for ids that cannot name a file.
pub fn normalize_exe_name(bundle_id: &str) -> Option<String> {
    let trimmed = bundle_id.trim().trim_matches('"');
    let file = trimmed
        .rsplit(['\\', '/'])
        .next()
        .map(str::trim)
        .filter(|f| !f.is_empty())?;
    let lower = file.to_lowercase();
    if Path::new(&lower).extension().is_some() {
        Some(lower)
    } else {
        Some(format!("{lower}.exe"))
    }
}

/// Normalised, de-duplicated executable names in file order.
pub fn exe_names(bundle_ids: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in bundle_ids {
        if let Some(name) = normalize_exe_name(id) {
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// Cheap change detector for the exclusion file. `poll` re-reads the file
/// only when its mtime (or existence) changed since the last read.
#[derive(Debug, Default)]
pub struct ExclusionFile {
    path: PathBuf,
    mtime: Option<SystemTime>,
    exists: bool,
    loaded: bool,
    exe_names: Vec<String>,
}

impl ExclusionFile {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            ..Self::default()
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The executable names currently configured.
    pub fn exe_names(&self) -> &[String] {
        &self.exe_names
    }

    /// Refreshes the list when the file changed. Returns `true` when the
    /// configured names differ from the previous poll.
    pub fn poll(&mut self) -> bool {
        let meta = fs::metadata(&self.path).ok();
        let exists = meta.is_some();
        let mtime = meta.and_then(|m| m.modified().ok());
        if self.loaded && exists == self.exists && mtime == self.mtime {
            return false;
        }
        self.loaded = true;
        self.exists = exists;
        self.mtime = mtime;
        let (ids, _) = read_bundle_ids(&self.path);
        let names = exe_names(&ids);
        if names == self.exe_names {
            return false;
        }
        self.exe_names = names;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(dir: &tempfile::TempDir, body: &str) -> PathBuf {
        let path = dir.path().join("audio-exclusions.json");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        path
    }

    #[test]
    fn env_override_wins_over_home() {
        let p = resolve_path(Some("D:\\cfg\\ex.json"), Some(Path::new("C:\\Users\\a")));
        assert_eq!(p, PathBuf::from("D:\\cfg\\ex.json"));
    }

    #[test]
    fn blank_override_falls_back_to_home() {
        let p = resolve_path(Some("  "), Some(Path::new("/home/a")));
        assert_eq!(
            p,
            PathBuf::from("/home/a/.screenpipe/audio-exclusions.json")
        );
    }

    #[test]
    fn default_path_is_under_home() {
        let p = resolve_path(None, Some(Path::new("/home/a")));
        assert_eq!(
            p,
            PathBuf::from("/home/a/.screenpipe/audio-exclusions.json")
        );
    }

    #[test]
    fn parse_keeps_file_order_and_skips_bad_entries() {
        let ids = parse_bundle_ids(
            r#"{"excluded_apps":[{"bundle_id":"spotify.exe"},{},{"bundle_id":"  "},{"bundle_id":3},{"bundle_id":"Chrome.exe","name":"Chrome"}]}"#,
        );
        assert_eq!(
            ids,
            vec!["spotify.exe".to_string(), "Chrome.exe".to_string()]
        );
    }

    #[test]
    fn parse_tolerates_garbage() {
        assert!(parse_bundle_ids("{ nope").is_empty());
        assert!(parse_bundle_ids(r#"{"excluded_apps": "x"}"#).is_empty());
        assert!(parse_bundle_ids(r#"[1,2]"#).is_empty());
        assert!(parse_bundle_ids("").is_empty());
    }

    #[test]
    fn read_missing_file_is_empty_without_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let (ids, mtime) = read_bundle_ids(&dir.path().join("absent.json"));
        assert!(ids.is_empty());
        assert!(mtime.is_none());
    }

    #[test]
    fn read_malformed_file_keeps_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_tmp(&dir, "{ not json");
        let (ids, mtime) = read_bundle_ids(&path);
        assert!(ids.is_empty());
        assert!(mtime.is_some());
    }

    #[test]
    fn exe_names_are_normalised() {
        assert_eq!(
            normalize_exe_name("Spotify.EXE").as_deref(),
            Some("spotify.exe")
        );
        assert_eq!(normalize_exe_name("chrome").as_deref(), Some("chrome.exe"));
        assert_eq!(
            normalize_exe_name(r"C:\Program Files\Google\Chrome\Application\chrome.exe").as_deref(),
            Some("chrome.exe")
        );
        assert_eq!(
            normalize_exe_name("/opt/x/app.exe").as_deref(),
            Some("app.exe")
        );
        assert_eq!(normalize_exe_name("  ").as_deref(), None);
        assert_eq!(normalize_exe_name(r"C:\dir\").as_deref(), None);
    }

    #[test]
    fn exe_names_dedupe_in_order() {
        let ids = vec![
            "chrome.exe".to_string(),
            "Spotify.exe".to_string(),
            "CHROME".to_string(),
        ];
        assert_eq!(exe_names(&ids), vec!["chrome.exe", "spotify.exe"]);
    }

    #[test]
    fn poll_reports_changes_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio-exclusions.json");
        let mut file = ExclusionFile::new(path.clone());
        // Missing file: nothing configured, no change from the empty start.
        assert!(!file.poll());
        assert!(file.exe_names().is_empty());

        fs::write(&path, r#"{"excluded_apps":[{"bundle_id":"chrome.exe"}]}"#).unwrap();
        assert!(file.poll());
        assert_eq!(file.exe_names(), ["chrome.exe".to_string()]);
        // Unchanged file: no change reported.
        assert!(!file.poll());

        fs::remove_file(&path).unwrap();
        assert!(file.poll());
        assert!(file.exe_names().is_empty());
    }
}
