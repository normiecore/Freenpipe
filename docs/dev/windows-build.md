# Building and running Freenpipe on Windows

Windows (x86_64-pc-windows-msvc) is the primary platform. This page is the
working recipe for a Git Bash shell on Windows 11. It covers the toolchain,
the native sidecars, the engine and app builds, and the rules that keep a
development build away from an installed screenpipe on the same machine.

## 1. Toolchain

| Tool | Version used | Notes |
|---|---|---|
| Rust | pinned by `rust-toolchain.toml` (1.93.1) | `rustup` installs it on first `cargo` call |
| Visual Studio 2022 Build Tools | 17.x, MSVC v143 14.44, Windows 11 SDK 10.0.26100 | workload "Desktop development with C++" |
| LLVM | any recent release in `C:\Program Files\LLVM` | `libclang.dll` for bindgen |
| CMake | the one bundled with VS 2022, or a standalone install | generator must be "Visual Studio 17 2022" |
| bun | 1.x | JS package manager and test runner |
| Git for Windows | 2.4x | provides Git Bash |
| WebView2 runtime | preinstalled on Windows 11 | needed to run the app |

## 2. Shell environment

Native dependencies (whisper-rs-sys, antirez-asr-sys, libsamplerate-sys,
knf-rs-sys, onnxruntime, ...) are compiled with MSVC through cmake and cc,
and bindgen needs the MSVC and SDK headers. Plain Git Bash has none of this.
Load the environment into the current shell first:

```bash
source scripts/dev/msvc-env.sh
```

The script asks `vswhere` for the VS 2022 install that has the x64 C++
tools, runs its `vcvars64.bat`, and imports `INCLUDE`, `LIB`, `LIBPATH`,
`PATH` and the SDK variables. It also sets, when not already set:

- `LIBCLANG_PATH=C:\Program Files\LLVM\bin`. Without it bindgen reports
  `'stdio.h' file not found` or cannot load libclang.
- `CMAKE_GENERATOR=Visual Studio 17 2022`. A second, empty "VS 18" Build
  Tools install otherwise wins and CMake picks a generator with no C++
  toolset. If CMake later complains "Does not match the generator used
  previously", delete `target/<profile>/build/<crate>-*/out/build` for the
  crate it names and rebuild. Setting it as a user environment variable
  is the most robust fix.
- `OPENBLAS_PATH=<repo>\apps\screenpipe-app-tauri\src-tauri\openblas`,
  required by antirez-asr-sys.

Opening an "x64 Native Tools Command Prompt for VS 2022" and setting the
three variables above by hand is equivalent.

Git Bash on some machines intermittently fails with `cygheap read copy
failed` or `fork: Resource temporarily unavailable`. Retry the command and
keep pipelines short.

## 3. Native sidecars

The app bundles ffmpeg, OpenBLAS, ONNX Runtime (CPU), the VC++ runtime and
a bun binary. Fetch them once (they are gitignored):

```bash
cd apps/screenpipe-app-tauri
bun install
bun scripts/pre_build.js
```

ffmpeg comes from the GyanD GitHub mirror. The files land in
`apps/screenpipe-app-tauri/src-tauri/` (`ffmpeg/`, `openblas/`,
`onnxruntime-win-x64-*/`, `vcredist/`, `bun-x86_64-pc-windows-msvc.exe`).

## 4. Engine (CLI) build

```bash
cargo build --profile release-dev -p screenpipe-engine --bin screenpipe
```

The output is `target/release-dev/screenpipe.exe`. `release-dev` is the
release profile with thin LTO and incremental compilation; a warm rebuild
takes a few minutes, a cold one considerably longer.

To run the bare CLI, put OpenBLAS on `PATH` first; it is linked
dynamically (antirez-asr-sys) and the app bundle normally ships
`libopenblas.dll` next to its exe. Without it the exe exits at once with
"error while loading shared libraries: libopenblas.dll".

```bash
PATH="$PWD/apps/screenpipe-app-tauri/src-tauri/openblas/bin:$PATH" \
  target/release-dev/screenpipe.exe record --port 3035 --data-dir "$FP_DD" --disable-audio
```

### Reusing a warm `target/` from another checkout

Copying `target/` from another clone or worktree saves most of a cold
build, but the CMake-based sys crates keep absolute paths in
`target/<profile>/build/<crate>-*/out/build/CMakeCache.txt`. When such a
build script reruns, CMake stops with "The current CMakeCache.txt
directory ... is different than the directory ... where CMakeCache.txt was
created". Delete the `out/build` directory of each crate it names
(knf-rs-sys and libsamplerate-sys in practice) and rebuild; only those C
libraries are reconfigured.

### The debug-profile CRT mismatch

The plain `dev` profile used to fail at link time with `LNK2038`
(`RuntimeLibrary` / `_ITERATOR_DEBUG_LEVEL` mismatch) and `LNK1120`:

- `knf-rs-sys` chooses the CMake configuration from its build script's own
  `debug_assertions`. In the dev profile build scripts have them on, so it
  compiled with the debug CRT (`/MDd`, iterator debug level 2).
- `whisper-rs-sys` and the other C/C++ dependencies are built in Release
  configuration (dependencies are compiled with `opt-level = 2`) and use the
  release CRT (`/MD`).

MSVC refuses to link the two runtimes together. Both Cargo workspaces (the
root one and `apps/screenpipe-app-tauri/src-tauri`) now set

```toml
[profile.dev.package.knf-rs-sys]
debug-assertions = false
```

A named-package override also applies to that package's build script, so
knf-rs-sys configures Release and the graph links against one runtime.
Do not "simplify" this to a global `[profile.dev.build-override]`: that
also turns debug assertions off in proc macros, and tauri-codegen then
emits ACL code without the debug-only `referenced_by` field that
tauri-utils (a normal dependency, still with debug assertions) expects,
so `tauri::generate_context!` fails with E0063 in the app's dev/test
build. If a future C++ dependency reintroduces the mismatch, add the same
override for it, or use `--profile release-dev` for builds and tests
(`cargo test -p <crate> --profile release-dev`).

## 5. Desktop app build

The Tauri app is its own Cargo workspace with its own `target/` directory
and lockfile: `apps/screenpipe-app-tauri/src-tauri`. Its build embeds the
static frontend from `apps/screenpipe-app-tauri/out`, so build that first:

```bash
cd apps/screenpipe-app-tauri
bun run build                       # next build, static export into out/
cd src-tauri
cargo build --profile release-dev   # target/release-dev/screenpipe-app.exe
```

For a compile-only check of the Rust side, a placeholder
`apps/screenpipe-app-tauri/out/index.html` is enough.

The base app compiled three modules (`enterprise_policy.rs`,
`enterprise_install_metadata.rs`, `enterprise_sync.rs`) from upstream's
proprietary `ee/` directory, which is not part of the fork. They are now
small original stubs, the `enterprise-build` Cargo feature is gone, and the
`@ee` TypeScript alias is replaced by `lib/app-update-policy.ts`. No `ee/`
directory is needed for any build.

Frontend checks, from `apps/screenpipe-app-tauri`:

```bash
bun run test               # vitest suite + bun:test suite
bun run bindings:check     # fails if lib/utils/tauri.ts drifted from Rust
bun run bindings:generate  # regenerate lib/utils/tauri.ts after changing commands
```

The two `bindings:*` scripts run a `cargo test` of the app crate in the dev
profile, so the first run compiles the whole dependency graph once more.

## 6. Dev isolation

Freenpipe keeps screenpipe's drop-in defaults: data in `~/.screenpipe`, API
on port 3030, the app control server on 11435, CLI binary `screenpipe`. A
development machine may also run a real screenpipe, so a dev build must never
use any of those. Load the isolation variables in every dev shell:

```bash
source scripts/dev/dev-env.sh [name]
```

| Variable | Default | Read by |
|---|---|---|
| `FP_DD`, `SCREENPIPE_DATA_DIR` | `%LOCALAPPDATA%/freenpipe-dev[/name]` | engine (`--data-dir`), app (store.bin, db, media), every default-data-dir reader |
| `FP_PORT`, `SCREENPIPE_PORT` | 3035 | engine `--port`; the app's embedded engine |
| `SCREENPIPE_FOCUS_PORT` | 11436 | the app's control server |
| `SCREENPIPE_API_KEY` | `fp-dev-key` | engine and app API auth; keeps the key resolver off any `auth.json` |

The script refuses a data dir inside `~/.screenpipe` and the ports 3030 and
11435.

### Engine smoke run

```bash
scripts/dev/engine-smoke.sh            # uses target/release-dev/screenpipe.exe
```

It starts the engine with `--port $FP_PORT --data-dir $FP_DD/engine-smoke
--disable-audio`, waits for `GET /health` to return JSON, stops the engine
by PID, deletes the throwaway data dir, and fails if `~/.screenpipe` (or its
`auth.json`) was created or modified during the run.

### App dev run

```bash
scripts/dev/app-dev.sh --check   # port checks only
scripts/dev/app-dev.sh           # then `bun run tauri dev`
```

The dev bundle identifier differs from the released app's, so webview
storage and the single-instance lock are separate as well.

### What the code no longer does

- **No blind port kills.** The base app force-killed every process it found
  on its API and control-server ports before binding, which would terminate
  an installed screenpipe. `server::port_holders::report` now only lists the
  listeners (PID and image name, via `netstat`/`tasklist` or `lsof`), logs a
  warning, and leaves them running; the bind then retries or fails loudly.
- **No auth.json outside the data dir.** The engine's API-key resolver used
  to read, and on key changes delete, `~/.screenpipe/auth.json` whatever
  `--data-dir` said. It now uses `<data_dir>/auth.json`, which is the same
  file for a default install.
- **No vendor endpoints by default.** Remote base URLs go through the
  endpoint switchboard (`screenpipe_core::endpoints`, `lib/web-url.ts`).
  Every entry is unset unless the user configures it.

### Remaining hard-coded `~/.screenpipe` paths (audit, 2026-10-15)

These bypass `--data-dir` and `SCREENPIPE_DATA_DIR`. None is on the engine
smoke path; they matter for pipe/agent runs and app sessions. Owners are the
work packages that already edit those files.

| Location | Access | Notes |
|---|---|---|
| `crates/screenpipe-core/src/agents/pi.rs` `cloud_media_analysis_enabled` | read marker | cloud media analysis (W0-06) |
| `crates/screenpipe-core/src/agents/pi.rs` enterprise.json lookup | read | enterprise gates (W0-04/W0-05) |
| `apps/.../src-tauri/src/commands.rs` `set_cloud_media_analysis_skill` | write marker | W0-06 |
| `crates/screenpipe-engine/src/cli/team.rs` enterprise.json | read | team CLI (W0-05) |
| `crates/screenpipe-redact/src/adapters/{onnx,opf,rfdetr,rfdetr_mlx}.rs` | model cache writes | PII model downloads (W0-09) |
| `crates/screenpipe-config/src/persistence.rs` `default_config_path` | none today | helper has no callers; the CLI uses `<data_dir>/config.toml` |
| `apps/.../src-tauri/src/audio_exclusions.rs`, `crates/screenpipe-audio/src/core/process_tap.rs` | read/write | `.screenpipe/audio-exclusions.json`, env override exists |
| `apps/.../src-tauri/src/notifications/client.rs`, `updates.rs` | HTTP | post to `127.0.0.1:11435` regardless of `SCREENPIPE_FOCUS_PORT` |
| `~/.pi/agent` (core and app `pi.rs`) | read/write | Pi agent config is shared with any other Pi install (W2-23 isolates it) |

`crates/screenpipe-core/src/agents/bash_env.rs` was fixed in place: the
bash-env wrapper now goes to the default data dir resolver, which honours
`SCREENPIPE_DATA_DIR`.
