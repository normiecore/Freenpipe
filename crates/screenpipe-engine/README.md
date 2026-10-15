# screenpipe-engine

The Freenpipe engine. It builds the `screenpipe` CLI binary (`src/bin/screenpipe-engine.rs`), which runs capture and serves the local HTTP API (port 3030 by default).

Main parts:

- `server.rs`, `routes/` and the `*_api.rs` modules: the HTTP API (search, frames, audio, meetings, pipes, connections, health)
- `event_driven_capture.rs`: screen capture triggered by UI events, pairing each frame with accessibility text
- `meeting_detector.rs`, `meeting_watcher.rs`, `meeting_persister.rs`: meeting detection and storage
- `pipes_api.rs`, `pipe_permissions_middleware.rs`: pipe management and per-pipe API permissions
- `retention.rs`: deletion of old data
- `cli/`: CLI subcommands

Build instructions are in [docs/dev/windows-build.md](../../docs/dev/windows-build.md).
