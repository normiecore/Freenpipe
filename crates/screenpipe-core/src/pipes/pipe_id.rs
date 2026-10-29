// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Pipe identifiers.
//!
//! A pipe's id is the name of its folder under the pipes directory, and ids
//! arrive from HTTP paths, CLI arguments, store slugs, sync manifests and
//! install sources. Every one of them is checked here before it is joined
//! onto the pipes directory, so a pipe folder can never resolve outside it.
//!
//! A valid id is a single path component that every supported platform can
//! store unchanged: no separator of either kind, no `.`/`..`, nothing Windows
//! rejects or silently rewrites (reserved punctuation, trailing dots or
//! spaces, device names such as `CON` or `com1.txt`), no control characters,
//! and no leading dot, which keeps the pipes directory's hidden sidecar files
//! (`.local-overrides.json`, `.tombstones.json`, ...) out of the id space.
//! Any other character, including non-ASCII letters and inner spaces, is
//! allowed so existing pipe folders keep working.
//!
//! These rules apply in full to names that are about to create a folder
//! (install, store install, sync). Operations on a pipe that is already
//! installed use [`validate_existing_pipe_id`], which also accepts the exact
//! name of a folder already in the pipes directory, so a folder made before
//! these rules (or by hand) can still be enabled, edited, stopped and
//! deleted.

use std::ffi::OsStr;
use std::fmt;
use std::path::Path;

/// Longest id accepted, in bytes (the common file-name limit).
pub const MAX_PIPE_ID_BYTES: usize = 255;

/// Characters Windows does not allow in a file name.
const WINDOWS_FORBIDDEN: &[char] = &['<', '>', ':', '"', '|', '?', '*'];

/// Device names Windows reserves in every directory, with or without an
/// extension.
const WINDOWS_DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM0", "COM1", "COM2", "COM3", "COM4",
    "COM5", "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3",
    "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// Why a pipe id was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPipeId {
    id: String,
    reason: &'static str,
}

impl InvalidPipeId {
    /// The refused id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Short explanation of the rule it broke.
    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

impl fmt::Display for InvalidPipeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Debug-format the id so control characters and quotes stay visible.
        write!(
            f,
            "invalid pipe identifier {:?}: {}; a pipe id must be a single folder name",
            self.id, self.reason
        )
    }
}

impl std::error::Error for InvalidPipeId {}

/// Accept `id` only if it is one portable path component (see the module
/// docs for the rules).
pub fn validate_pipe_id(id: &str) -> Result<(), InvalidPipeId> {
    check(id, false)
}

/// Accept `id` for an operation on an installed pipe: any id that passes
/// [`validate_pipe_id`], and also the exact name of a folder that already
/// exists in `pipes_dir`. A directory listing only yields single components,
/// so such a name cannot reach outside `pipes_dir`.
///
/// On Windows the exception covers only the leading-dot rule: Win32 path
/// handling drops trailing dots and spaces and maps device names, so a folder
/// with such a name (made through a `\\?\` path or another OS) could open as
/// a different pipe's folder.
pub fn validate_existing_pipe_id(pipes_dir: &Path, id: &str) -> Result<(), InvalidPipeId> {
    let strict = validate_pipe_id(id);
    if strict.is_err() && opens_unchanged_here(id) && names_existing_folder(pipes_dir, id) {
        return Ok(());
    }
    strict
}

/// True when `pipes_dir` holds a folder named exactly `id`.
fn names_existing_folder(pipes_dir: &Path, id: &str) -> bool {
    if id.is_empty() || id == "." || id == ".." || id.contains('/') {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(pipes_dir) else {
        return false;
    };
    entries
        .flatten()
        .any(|entry| entry.file_name() == OsStr::new(id) && entry.path().is_dir())
}

/// Whether a folder named `id` is opened under that exact name on this
/// platform.
#[cfg(windows)]
fn opens_unchanged_here(id: &str) -> bool {
    check(id, true).is_ok()
}

#[cfg(not(windows))]
fn opens_unchanged_here(_id: &str) -> bool {
    true
}

fn check(id: &str, leading_dot_ok: bool) -> Result<(), InvalidPipeId> {
    let refuse = |reason| {
        Err(InvalidPipeId {
            id: id.to_string(),
            reason,
        })
    };
    if id.is_empty() {
        return refuse("it is empty");
    }
    if id.len() > MAX_PIPE_ID_BYTES {
        return refuse("it is longer than 255 bytes");
    }
    if id == "." || id == ".." {
        return refuse("'.' and '..' name directories relative to the current one");
    }
    if id.contains(['/', '\\']) {
        return refuse("it contains a path separator");
    }
    if id.chars().any(char::is_control) {
        return refuse("it contains a control character");
    }
    if id.contains(WINDOWS_FORBIDDEN) {
        return refuse("it contains a character Windows forbids in file names (< > : \" | ? *)");
    }
    if id.starts_with('.') && !leading_dot_ok {
        return refuse("it starts with a dot, which is kept for hidden files");
    }
    if id.starts_with(char::is_whitespace) {
        return refuse("it starts with whitespace");
    }
    if id.ends_with(['.', ' ']) || id.ends_with(char::is_whitespace) {
        return refuse("it ends with a dot or whitespace, which Windows drops");
    }
    if is_windows_device_name(id) {
        return refuse("it is a device name Windows reserves");
    }
    Ok(())
}

/// True when `id` passes [`validate_pipe_id`].
pub fn is_valid_pipe_id(id: &str) -> bool {
    validate_pipe_id(id).is_ok()
}

fn is_windows_device_name(id: &str) -> bool {
    // "nul.md" and "con .txt" are reserved too: Windows looks at the part
    // before the first dot, ignoring trailing spaces.
    // ASCII case folding also covers the superscript-digit names, whose
    // non-ASCII digit has no case.
    let stem = id.split('.').next().unwrap_or(id).trim_end();
    WINDOWS_DEVICE_NAMES
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_ids_pass() {
        for id in [
            "day-recap",
            "meeting_summary",
            "obsidian-sync.v2",
            "My Pipe",
            "résumé-builder",
            "会議メモ",
            "a",
            &"x".repeat(MAX_PIPE_ID_BYTES),
        ] {
            assert!(validate_pipe_id(id).is_ok(), "{id:?} should be valid");
        }
    }

    #[test]
    fn traversal_is_refused() {
        for id in [
            "../x",
            "..",
            ".",
            "a/b",
            "a\\b",
            "..\\..\\windows",
            "/etc",
            "C:\\x",
            "C:x",
        ] {
            assert!(validate_pipe_id(id).is_err(), "{id:?} should be refused");
        }
        let err = validate_pipe_id("../x").unwrap_err();
        assert_eq!(err.id(), "../x");
        assert!(err.to_string().contains("invalid pipe identifier"), "{err}");
    }

    #[test]
    fn windows_unsafe_names_are_refused() {
        for id in [
            "CON",
            "con",
            "Nul",
            "aux.md",
            "com1",
            "COM9.txt",
            "lpt3",
            "con .txt",
            "CONOUT$",
            "com¹",
            "trailing.",
            "trailing ",
            "what?",
            "a*b",
            "pipe|x",
            "<x>",
            "quote\"d",
        ] {
            assert!(validate_pipe_id(id).is_err(), "{id:?} should be refused");
        }
        // Names that only start like a device name are fine.
        for id in [
            "console",
            "com10",
            "lpt",
            "auxiliary",
            "null-pipe",
            "prn-report",
        ] {
            assert!(validate_pipe_id(id).is_ok(), "{id:?} should be valid");
        }
    }

    #[test]
    fn existing_folders_keep_working_under_their_own_name() {
        let tmp = tempfile::tempdir().unwrap();
        let pipes = tmp.path();
        std::fs::create_dir_all(pipes.join(".hidden-pipe")).unwrap();
        std::fs::write(pipes.join(".sidecar.json"), "{}").unwrap();
        assert!(validate_existing_pipe_id(pipes, ".hidden-pipe").is_ok());
        // Files are not pipes, and a name with no folder gets the full rules.
        assert!(validate_existing_pipe_id(pipes, ".sidecar.json").is_err());
        assert!(validate_existing_pipe_id(pipes, ".missing").is_err());
        // Traversal never names a listed folder.
        for id in ["..", ".", "../x", ""] {
            assert!(validate_existing_pipe_id(pipes, id).is_err(), "{id:?}");
        }
        assert!(validate_existing_pipe_id(pipes, "ordinary").is_ok());
    }

    #[cfg(not(windows))]
    #[test]
    fn existing_folders_with_windows_unsafe_names_keep_working_elsewhere() {
        let tmp = tempfile::tempdir().unwrap();
        for id in ["aux", "trailing.", "what?", "daily.md?raw=1"] {
            std::fs::create_dir_all(tmp.path().join(id)).unwrap();
            assert!(validate_pipe_id(id).is_err(), "{id:?} is not portable");
            assert!(
                validate_existing_pipe_id(tmp.path(), id).is_ok(),
                "{id:?} exists and should be usable"
            );
        }
    }

    #[test]
    fn hidden_empty_long_and_control_ids_are_refused() {
        for id in [
            "",
            ".hidden",
            ".local-overrides.json",
            " leading",
            "tab\tname",
            "new\nline",
            "nul\0byte",
            &"x".repeat(MAX_PIPE_ID_BYTES + 1),
        ] {
            assert!(validate_pipe_id(id).is_err(), "{id:?} should be refused");
        }
    }
}
