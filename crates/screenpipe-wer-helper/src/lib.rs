// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Windows Error Reporting runtime exception helper (shell-017).
//!
//! Some crashes never reach Rust's panic hook: a stack buffer overrun, a
//! `__fastfail`, an `abort()` inside native code. Windows hands those to
//! WerFault.exe, which loads every runtime exception module the crashed
//! process registered and calls its `OutOfProcessExceptionEventCallback`.
//! This crate is that module. The callback writes a minidump of the crashed
//! process into the directory the app named in its context block (normally
//! `<data dir>\crash-dumps\pending`), or, when the block cannot be read, into
//! `crash-dumps` next to the DLL. It never claims the crash, so WER's own
//! handling carries on unchanged, and it never sends anything anywhere: the
//! dumps stay on disk until the user shares them.
//!
//! The pure parts (context block layout, file naming, limits) are in
//! [`layout`] and are tested on every platform; the exported callbacks exist
//! only on Windows.

pub mod layout;

#[cfg(windows)]
mod callbacks;
