// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

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

use std::fmt;

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
    if id.starts_with('.') {
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
        for id in ["../x", "..", ".", "a/b", "a\\b", "..\\..\\windows", "/etc", "C:\\x", "C:x"] {
            assert!(validate_pipe_id(id).is_err(), "{id:?} should be refused");
        }
        let err = validate_pipe_id("../x").unwrap_err();
        assert_eq!(err.id(), "../x");
        assert!(err.to_string().contains("invalid pipe identifier"), "{err}");
    }

    #[test]
    fn windows_unsafe_names_are_refused() {
        for id in [
            "CON", "con", "Nul", "aux.md", "com1", "COM9.txt", "lpt3", "con .txt", "CONOUT$",
            "com¹", "trailing.", "trailing ", "what?", "a*b", "pipe|x", "<x>", "quote\"d",
        ] {
            assert!(validate_pipe_id(id).is_err(), "{id:?} should be refused");
        }
        // Names that only start like a device name are fine.
        for id in ["console", "com10", "lpt", "auxiliary", "null-pipe", "prn-report"] {
            assert!(validate_pipe_id(id).is_ok(), "{id:?} should be valid");
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
