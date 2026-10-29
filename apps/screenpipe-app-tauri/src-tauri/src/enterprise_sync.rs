// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Managed-deployment activity upload: permanently off in Handvel.
//!
//! The base app could push captured frames, audio, UI events, memories and
//! snapshots to a vendor ingest service in its enterprise build. Handvel
//! drops that channel entirely. `spawn` is kept so the startup sequence in
//! `main.rs` compiles unchanged; it never starts anything.

/// Start the upload worker and return its shutdown sender. Handvel has no
/// upload worker, so this always returns `None`.
pub fn spawn(_app: &tauri::AppHandle) -> Option<tokio::sync::watch::Sender<bool>> {
    None
}
