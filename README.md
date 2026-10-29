# Handvel

Handvel records what is on your screen and what is said through your microphone and speakers, stores it on your own machine, and makes it searchable by you and by the AI tools you connect to it.

It is a fork of screenpipe at commit `892199f` (2026-06-10), the last upstream commit released under the MIT licence. Upstream moved to a commercial licence after that commit. Handvel stays MIT. It stays compatible with screenpipe's data, API, pipes and MCP configurations, and builds its own features beyond that. Specs for some later behaviour were first written from static analysis of the shipped v2.7.104 binary, never from post-relicense source. That analysis stopped on 2026-10-29, and those specs are being rewritten from public material and Handvel's own design. The method, the sourcing policy and the specifications are in [docs/clean-room/README.md](docs/clean-room/README.md).

Handvel is not affiliated with or endorsed by the screenpipe project or its owners. See [NOTICE.md](NOTICE.md).

## Status

Handvel is in active development and has no release yet. Work is organised in numbered waves of work packages:

- [docs/clean-room/PLAN.md](docs/clean-room/PLAN.md) is the plan: waves 0 to 8, each work package with its dependencies, owned files and acceptance checks.
- [docs/clean-room/STATUS.md](docs/clean-room/STATUS.md) is the generated progress board.
- [docs/clean-room/DECISIONS.md](docs/clean-room/DECISIONS.md) records the owner decisions that bind every work package.

Wave 0 removed the upstream vendor's services, telemetry and paid-plan gates and renamed the product. The desktop app is called Handvel, has its own bundle identifier (`com.normiecore.handvel`, and `com.normiecore.handvel.dev` for development builds), and opens `handvel://` links.

## What works from the base

The MIT base already provides:

- **Screen capture.** Text is read from the operating system's accessibility tree; OCR is the fallback when no accessibility data is available. App names, window titles and browser URLs are stored with each frame.
- **Audio capture and transcription.** System audio and microphone input, transcribed locally with Whisper or another local engine, with speaker identification.
- **Local storage and search.** A SQLite database with full-text search in the data directory, plus media files on disk.
- **Local HTTP API** on `http://localhost:3030` for search, frames, audio, health and pipe management.
- **MCP server** in [packages/screenpipe-mcp](packages/screenpipe-mcp), so MCP clients such as Claude Desktop or Cursor can query your history.
- **Pipes.** Scheduled agents, each defined by a `pipe.md` file that holds a prompt and a schedule. Built-in examples are in [crates/screenpipe-core/assets/pipes](crates/screenpipe-core/assets/pipes).
- **Desktop app** built with Tauri, in [apps/screenpipe-app-tauri](apps/screenpipe-app-tauri).

## Drop-in compatibility

Handvel keeps screenpipe's defaults so that existing setups keep working (decision 1 in [DECISIONS.md](docs/clean-room/DECISIONS.md)):

- The CLI binary is still named `screenpipe`.
- Data lives in `~/.screenpipe`.
- The API listens on `localhost:3030`, and the app's control server on port 11435.
- Existing pipes, skills and MCP client configurations work unchanged.
- Links in screenpipe's `screenpipe://` form (from pipes, skills or notifications) still open inside Handvel. Handvel registers only `handvel://` with the operating system, so an installed screenpipe keeps its own link handler.

Because it uses the same paths and ports, Handvel does not run alongside an installed screenpipe. Quit one before starting the other. Crate names, package names, HTTP routes, JSON fields, settings keys and `SCREENPIPE_*` environment variables keep their upstream names for the same reason.

## Building from source

Windows (x86_64-pc-windows-msvc) is the primary platform. The macOS and Linux code paths are kept compiling but are not exercised in this fork.

You need:

- Rust, at the version pinned in [rust-toolchain.toml](rust-toolchain.toml) (`rustup` installs it on the first `cargo` call)
- [bun](https://bun.sh) for the JavaScript and TypeScript parts
- on Windows, Visual Studio 2022 Build Tools with the C++ workload, LLVM and CMake

The full Windows recipe, including the native sidecars, the desktop app build and the rules that keep a development build away from an installed screenpipe, is in [docs/dev/windows-build.md](docs/dev/windows-build.md). In short, from Git Bash:

```bash
source scripts/dev/msvc-env.sh
cargo build --profile release-dev -p screenpipe-engine --bin screenpipe
```

The engine binary is written to `target/release-dev/screenpipe.exe`. `screenpipe record` starts capture and serves the API. Put OpenBLAS on `PATH` before running the bare exe, or it exits at once (the build guide shows how; the files come from the native sidecar step).

To connect an MCP client, build the server in `packages/screenpipe-mcp` (`bun install && bun run build`) and follow [its README](packages/screenpipe-mcp/README.md).

Tests: `cargo test` for Rust and `bun test` for TypeScript. [TESTING.md](TESTING.md) has the regression checklist.

## Privacy

- Captured data stays on your machine, in the data directory.
- There is no telemetry and no crash reporting. Both were removed in wave 0.
- Handvel does not use the upstream vendor's services. Remote service URLs go through an endpoint switchboard (`crates/screenpipe-core/src/endpoints.rs`) in which every entry is unset until you configure it. A CI check (`scripts/ci/vendor-guard.test.ts`) fails the build if a vendor host, key or the upstream identity comes back.
- Cloud AI features use your own API keys, or a local model server such as Ollama. Features that upstream ran on its own servers (sync, archive, pipe store, team management) come back as bring-your-own or self-hosted options (decision 5 in [DECISIONS.md](docs/clean-room/DECISIONS.md)).

## Licence

MIT. See [LICENSE.md](LICENSE.md) for the licence and the upstream copyright, and [NOTICE.md](NOTICE.md) for provenance.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) first. Contributions follow the clean-room rules in [CLAUDE.md](CLAUDE.md): no post-relicense upstream material, original code, and upstream names only for interoperability identifiers.

Report bugs and propose changes through [GitHub issues](https://github.com/normiecore/Freenpipe/issues).
