// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Managed-deployment activity upload: permanently off in Freenpipe.
//!
//! The base app could push captured frames, audio, UI events, memories and
//! snapshots to a vendor ingest service in its enterprise build. Freenpipe
//! drops that channel entirely. `spawn` is kept so the startup sequence in
//! `main.rs` compiles unchanged; it never starts anything.

/// Start the upload worker and return its shutdown sender. Freenpipe has no
/// upload worker, so this always returns `None`.
pub fn spawn(_app: &tauri::AppHandle) -> Option<tokio::sync::watch::Sender<bool>> {
    None
}
