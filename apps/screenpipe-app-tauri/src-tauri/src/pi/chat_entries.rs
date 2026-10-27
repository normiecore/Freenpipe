// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Listing of saved chat conversations, newest first.
//!
//! The chat stores one JSON file per conversation under `<data dir>/chats`.
//! Sorting them by modification time from the webview takes one `stat` call
//! per file; `list_chat_entries_by_mtime` does the whole listing in one
//! command.

use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// One conversation file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatEntry {
    /// File name, `<conversation id>.json`.
    pub name: String,
    /// Absolute path of the file.
    pub path: String,
    /// When the file last changed, in ms since the Unix epoch (its creation
    /// time where no modification time is available; 0 when neither is).
    pub mtime_ms: u64,
    /// File size in bytes.
    pub size_bytes: u64,
}

/// `<data dir>/chats`.
pub fn chats_dir() -> PathBuf {
    screenpipe_core::paths::default_screenpipe_data_dir().join("chats")
}

fn epoch_ms(time: std::io::Result<SystemTime>) -> u64 {
    time.ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The `.json` files directly in `dir`, newest first (ties broken by name,
/// descending, like the chat sidebar). Sub-directories and other files are
/// skipped; a missing directory is an empty list.
pub fn list_by_mtime(
    dir: &Path,
    offset: usize,
    limit: Option<usize>,
) -> std::io::Result<Vec<ChatEntry>> {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut entries = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".json") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let mtime_ms = match epoch_ms(meta.modified()) {
            0 => epoch_ms(meta.created()),
            ms => ms,
        };
        entries.push(ChatEntry {
            name,
            path: entry.path().to_string_lossy().into_owned(),
            mtime_ms,
            size_bytes: meta.len(),
        });
    }
    entries.sort_by(|a, b| {
        b.mtime_ms
            .cmp(&a.mtime_ms)
            .then_with(|| b.name.cmp(&a.name))
    });
    let page = entries.into_iter().skip(offset);
    Ok(match limit {
        Some(limit) => page.take(limit).collect(),
        None => page.collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn touch(dir: &Path, name: &str, age_secs: u64) {
        let path = dir.join(name);
        std::fs::write(&path, b"{}").unwrap();
        let when = SystemTime::now() - Duration::from_secs(age_secs);
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(when).unwrap();
    }

    #[test]
    fn lists_json_files_newest_first_with_paging() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "old.json", 3_000);
        touch(dir.path(), "new.json", 10);
        touch(dir.path(), "mid.json", 600);
        touch(dir.path(), "notes.txt", 1);
        std::fs::create_dir(dir.path().join("folder.json")).unwrap();

        let all = list_by_mtime(dir.path(), 0, None).unwrap();
        let names: Vec<_> = all.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["new.json", "mid.json", "old.json"]);
        assert!(all.iter().all(|e| e.size_bytes == 2 && e.mtime_ms > 0));
        assert!(all[0].path.ends_with("new.json"));

        let page: Vec<_> = list_by_mtime(dir.path(), 1, Some(1))
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(page, ["mid.json"]);
        assert!(list_by_mtime(dir.path(), 5, Some(10)).unwrap().is_empty());
    }

    #[test]
    fn a_missing_directory_is_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("chats");
        assert!(list_by_mtime(&missing, 0, None).unwrap().is_empty());
    }

    #[test]
    fn equal_times_are_ordered_by_name_descending() {
        let dir = tempfile::tempdir().unwrap();
        let when = SystemTime::now() - Duration::from_secs(60);
        for name in ["a.json", "c.json", "b.json"] {
            let path = dir.path().join(name);
            std::fs::write(&path, b"{}").unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(when).unwrap();
        }
        let names: Vec<_> = list_by_mtime(dir.path(), 0, None)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["c.json", "b.json", "a.json"]);
    }
}
