# Building and running Freenpipe on Windows

Windows (x86_64-pc-windows-msvc) is the primary platform. This page is the
working recipe for a Git Bash shell on Windows 11. It covers the toolchain,
the native sidecars, the engine and app builds, and the rules that keep a
development build away from an installed screenpipe on the same machine.

## 1. Toolchain

| Tool | Version used | Notes |
|---|---|---|
| Rust | pinned by `rust-toolchain.toml` (1.94.1) | `rustup` installs it on first `cargo` call |
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

`pre_build.js` also stages the runtime the installer puts next to the app
(`scripts/stage_bundled_runtime.js`, output in `src-tauri/bundled/`): the MCP
server built from `packages/screenpipe-mcp` into one file, the two pyannote
speaker models (from an LFS checkout, otherwise downloaded from this
repository's LFS storage and checked against their SHA-256), the Silero VAD
model (`bundled/vad/`, downloaded from snakers4/silero-vad at a pinned
commit), the Windows GPU runtime (`bundled/runtime/`, see section 5.1) and
the `screenpipe` CLI. The CLI is only staged when you ask for it, because it
needs a release build of the engine:

```bash
FREENPIPE_BUILD_CLI=1 bun scripts/pre_build.js                        # builds and stages it
FREENPIPE_CLI_BINARY=/path/to/screenpipe.exe bun scripts/pre_build.js  # stages a prebuilt one
```

Installer builds should also set `FREENPIPE_REQUIRE_BUNDLED_RUNTIME=1`, which
turns a missing piece into a build error instead of a warning.

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

### 5.1 GPU builds: Vulkan whisper and DirectML

The x64 release build enables two GPU paths (W2-19). Both fall back to CPU
on their own, so a GPU build also runs on machines without a usable GPU.

| Feature | What it does | Build needs | Runtime needs |
|---|---|---|---|
| `vulkan` | whisper.cpp on ggml-vulkan | Vulkan SDK, `VULKAN_SDK` set | `vulkan-1.dll` (load-time import) |
| `directml` | Parakeet's ONNX sessions on a DirectX 12 GPU | nothing extra (`ort` is load-dynamic) | the DirectML build of `onnxruntime.dll`, and `DirectML.dll` |

**Vulkan SDK.** Install the LunarG Vulkan SDK (the release uses 1.4.313.0)
with its default components, then open a new shell so `VULKAN_SDK` is set
and `%VULKAN_SDK%\Bin` (glslc) is on `PATH`:

```bash
echo "$VULKAN_SDK"; ls "$VULKAN_SDK/Lib/vulkan-1.lib"; glslc --version
cargo build --profile release-dev -p screenpipe-engine --bin screenpipe --features vulkan,directml
```

The ggml-vulkan shader generator nests its build directories deeply; keep
the checkout (or `CARGO_TARGET_DIR`) at a short path, or enable Windows long
paths, if CMake reports paths over 260 characters. Linux and macOS builds
never enable `vulkan`, so they need no SDK. (`vulkan-qwen3-ggml` is a
separate opt-in feature for audiopipe's ggml Qwen3 engine; no release uses
it.)

**What the installer ships** (`src-tauri/bundled/runtime/`, mapped into the
install root by `tauri.windows.conf.json`):

- `onnxruntime.dll`, `onnxruntime_providers_shared.dll` and their licence
  texts: ONNX Runtime 1.22.0 built with the DirectML provider, from the
  `Microsoft.ML.OnnxRuntime.DirectML` NuGet package (MIT), pinned by
  SHA-256. It contains the CPU provider too and loads `DirectML.dll` only when
  DirectML is used. Without network access the script falls back to a CPU
  build already unpacked under `src-tauri/onnxruntime-win-<arch>-<version>/`.
- `vulkan-1.dll` and `VulkanRT-License.txt`, from the folder named by
  `FREENPIPE_VULKAN_RUNTIME_DIR` (the release unpacks LunarG's
  `vulkan-runtime-components.zip` there). A build with `vulkan` cannot start
  without this DLL on a machine whose GPU driver installed no Vulkan runtime,
  so set the variable for any installer you build with `vulkan`.
- `DirectML.dll` only with `FREENPIPE_BUNDLE_DIRECTML=1`. It is Microsoft's
  redistributable under Microsoft's own licence terms, so it stays out until
  the release licence audit clears it (see
  `docs/dev/bundled-runtime-licences.md`). Without it ONNX Runtime uses the
  `DirectML.dll` in System32 (Windows 10 1903 and later); when that copy is
  missing or too old, Parakeet runs on CPU.

**Run-time switches and logs.**

- `SCREENPIPE_DIRECTML`: unset or empty picks DirectML when an adapter
  qualifies; `1`/`true` asks for it; any other value (`0`, `false`) forces CPU.
  The engine logs the decision once, for example
  `parakeet: DirectML adapter ordinal 0: <name> (<n> MiB dedicated video memory, UMA=false)`
  or `parakeet: SCREENPIPE_DIRECTML explicitly disabled DirectML; using CPU`.
- whisper logs `whisper context: GPU backend Vulkan (...)` or
  `whisper context: GPU unavailable; using CPU fallback (...)` when it loads
  a model.
- `SCREENPIPE_WHISPER_GPU_FAULT=inference` (or `state`) makes GPU whisper
  sessions fail on purpose, to exercise the CPU recovery: the log shows
  `GPU inference failed`, then `CPU state installed` and
  `CPU inference finished after GPU recovery`, and the transcript still
  arrives.

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
| `FP_DD`, `SCREENPIPE_DATA_DIR` | `%LOCALAPPDATA%/freenpipe-dev[/name]` | engine (`--data-dir`), every Rust reader that goes through `screenpipe_core::paths::default_screenpipe_data_dir()`, including the app's Rust side (its `store.bin`, db, media, logs, `pi-agent/`). The app's webview opens the same `store.bin` and asks the backend for the data dir (`lib/data-dirs.ts`), but some of its other paths still build `~/.screenpipe/...` (see "Remaining hard-coded paths") |
| `FP_PORT`, `SCREENPIPE_PORT` | 3035 | engine `--port`; the app's Rust side (embedded engine port, `get_local_api_config`). The webview can override it with `settings.port` from its own store (see "Webview API port") |
| `SCREENPIPE_FOCUS_PORT` | 11436 | the app's control server (but not the notification client, which still posts to :11435) |
| `SCREENPIPE_API_KEY` | `fp-dev-key` | engine and app API auth; keeps the key resolver off any `auth.json` |

The script refuses a data dir inside `~/.screenpipe` and the ports 3030 and
11435. These variables isolate the engine completely (the smoke run below
checks it). They do **not** yet isolate an app dev run; see below.

### Engine smoke run

```bash
scripts/dev/engine-smoke.sh            # uses target/release-dev/screenpipe.exe
```

It starts the engine with `--port $FP_PORT --data-dir $FP_DD/engine-smoke
--disable-audio`, waits for `GET /health` to return JSON, stops the engine
by PID, deletes the throwaway data dir, and fails if `~/.screenpipe` (or its
`auth.json`) was created or modified during the run.

### App dev run (not isolated on disk yet)

```bash
scripts/dev/app-dev.sh --check   # guards only
scripts/dev/app-dev.sh           # guards, then `bun run tauri dev`
```

The app's Rust side honours `SCREENPIPE_DATA_DIR`, and so do the
webview's settings store and data-dir lookups, which go through the
backend (`lib/data-dirs.ts`). The webview still builds its chat history,
Pi chat sessions and a few other paths as `homeDir()/.screenpipe/...`
(audit below). Until the owning work packages route those through a
data-dir command, an app dev session reads and writes the real
`~/.screenpipe`: it lists its chats at boot, puts dev chats and Pi
sessions next to them, and the shortcut overlay reads the installed
screenpipe's `store.bin`. If `~/.screenpipe` does not exist, the session
creates it, and a later screenpipe start would adopt the dev chats.

`app-dev.sh` contains the damage rather than claiming isolation:

- It refuses to start while `~/.screenpipe` exists. Set
  `FP_ALLOW_HOME_SCREENPIPE=1` to run anyway; afterwards the script lists
  what the session changed there.
- If `~/.screenpipe` did not exist before the session and exists
  afterwards, it moves it to `$FP_DD/home-dot-screenpipe.<timestamp>`.
  It skips the move, with a warning, while something listens on 3030 or
  11435, because that directory might then belong to a screenpipe that
  started during the session.
- It refuses while something listens on 3030 or 11435 (an installed
  screenpipe), because the webview can still address :3030 (next
  section) and the notification client posts to :11435. Quit screenpipe,
  or set `FP_ALLOW_SCREENPIPE_RUNNING=1`.
- It refuses when the dev ports themselves (`SCREENPIPE_PORT`,
  `SCREENPIPE_FOCUS_PORT`) are taken.

`bun test scripts/dev/app-dev.test.ts` (from the repository root) checks
these guards in a throwaway HOME; it never touches the real one.

Arguments after `--` go to `tauri dev`. The dev bundle identifier
(`screenpi.pe.dev`) differs from the released app's, so the
single-instance lock is separate. The WebView2 profile is not covered by
the identifier: `windows_webview_env.rs` puts it in
`%LOCALAPPDATA%\ScreenPipe\WebView2` for every build, so any screenpipe
built from this code base shares it. It honours a preset `WEBVIEW2_USER_DATA_FOLDER`,
so `app-dev.sh` exports `$FP_DD/webview2`. localStorage and IndexedDB
are keyed by origin, and the dev origin (`http://localhost:1420`) differs
from the bundled one anyway.

### Webview API port

`lib/api.ts` loads the engine port and key from `get_local_api_config`,
which honours `SCREENPIPE_PORT` once the engine is up. The settings
provider then calls `configureApi({ port: settings.port ?? 3030 })` at load
and on every `updateSettings` (`lib/hooks/use-settings.tsx`). Whichever
runs last wins. The store is the backend's `store.bin`, so `settings.port`
is the port the embedded engine is configured with; only the
`SCREENPIPE_PORT` override is lost. A dev store that keeps the default port
therefore sends the dev UI's requests to :3030, and if an installed
screenpipe is listening there, the dev UI drives the real engine. Setting
`port` to `$FP_PORT` in the dev `store.bin` avoids it. The fix (let the
backend report the configured port even before the engine is up, then drop
the override) is recorded in docs/clean-room/open-questions/W1-02.md,
question 19.

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
smoke path. The Rust ones matter for pipe and agent runs; the webview ones
matter for every app dev run. Owners are the work packages whose files or
items cover each site.

#### Rust

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

#### Webview (TypeScript)

Each of these joins `homeDir()` (Tauri's path API, which no environment
variable can redirect) with `.screenpipe`. All paths are under the
`apps/screenpipe-app-tauri/` directory, and line numbers are as of this
audit.

| Location | Path | Access, and when | Owner |
|---|---|---|---|
| `app/shortcut-reminder/page.tsx:153` | `store.bin` | Read by the shortcut overlay on every store change. The fix is `resolveStorePath()` from `lib/data-dirs.ts`. | W5-14/W6-19 (the file's owners) |
| `lib/chat-storage.ts:63` `getChatsDir` | `chats/` | Listed at boot (`lib/stores/pi-event-router.ts` hydrate calls `listConversations`). Created and written on every chat save, delete and archive. | W5-15 |
| `components/standalone-chat.tsx:4746, 5839, 5954, 6504, 6787` | `pi-chat/` | Passed to `piStart` as the Pi session working dir when chat starts, restarts after a crash, re-authenticates or auto-spawns Pi | W4-05 |
| `components/standalone-chat.tsx:780` `externalizeLargeContextIfNeeded` | `pi-chat/large-context/<session>/...` | Written when a prompt is too large to send inline | W4-05 |
| `lib/utils/generate-title-with-preset.ts:106` | `pi-title/` | Pi working dir for the chat-title session after a first message | W4-05 |
| `lib/hooks/use-enterprise-pipes.ts:79, 136` | `pipes/<name>/pipe.md` | Writes managed pipes and reads pipe dirs for the enterprise heartbeat. Unreachable while `is_enterprise_build()` returns false (W0-01 stub). | W0-05 / W1-04 |
| `components/settings/usage-section.tsx:59` | `usage-cache.json` | Read and written when Settings > Usage opens | W0-05 (removes the section) |
| `components/onboarding/engine-startup.tsx:818` `openLogsFolder` | the dir itself | Opens it in Explorer; a dev run's logs are in `$FP_DD` | W0-10 |

The target resolves these paths through Tauri commands, and that is how
the gap closes:

- `get_screenpipe_base_dir` returns the directory of `store.bin`
  (specs/shell.md shell-003, W1-02).
- `get_active_data_dir` returns the recording data dir (W1-02 lists it
  in its owner paths).
- `get_chats_dir` returns the chats directory (specs/frontend.md command
  table, frontend-009, W5-15).

Implemented over `screenpipe_core::paths::default_screenpipe_data_dir()`,
they honour `SCREENPIPE_DATA_DIR`. W1-02 added the first two and the
shared TS helper, `lib/data-dirs.ts`: `resolveStoreDir`, `resolveStorePath`
and `resolveActiveDataDir`, plus `resolveDirWith` for any other directory
command such as `get_chats_dir`. It asks the backend and falls back to
`homeDir()/.screenpipe` only outside Tauri or with an older backend that
lacks the command; an error the command returns is thrown instead. The
settings store (`getStore`) and `getDataDir` in `lib/hooks/use-settings.tsx`
already use it, and each row above is now a one-line change for its
owner. When the table is empty, drop the
`~/.screenpipe` guard from `scripts/dev/app-dev.sh` and the "not isolated
on disk" warning in this document.

Other `homeDir()` uses do not touch `~/.screenpipe`, but two of them
reach real files outside the dev sandbox:

- `components/onboarding/connect-apps.tsx` and
  `lib/hooks/use-hardcoded-tiles.ts` edit other tools' config files when
  you click connect: Claude Desktop's `claude_desktop_config.json`,
  `~/.cursor/mcp.json` and `~/.codex/config.toml`. They also read
  Obsidian's `obsidian.json`. A dev build that connects one of these
  replaces the installed screenpipe's `screenpipe` MCP entry, so do not
  click them in a dev session, or restore the file afterwards. W0-09 owns
  the MCP snippet and W4-07 the agent setup.
- `components/settings/connections-section.tsx` only probes for
  installed apps. `components/notification-bell.tsx` and
  `app/notification-panel/page.tsx` only expand `~/` in links the user
  clicks.

### Running the Rust test suites safely

Some upstream tests touch the real home directory. Export
`SCREENPIPE_DATA_DIR` (dev-env.sh does) before `cargo test`, and expect:

- `screenpipe-core` `agents::pi::tests::test_ensure_pi_config_adds_ollama_provider`
  writes `~/.pi/agent/models.json` regardless of any variable; delete it
  afterwards if `~/.pi` did not exist before.
- `screenpipe-app` `config::tests::test_resolve_{default,empty,relative_path_falls_back}`
  assert that the default data dir ends in `.screenpipe` and create
  `<default>/data`. With `SCREENPIPE_DATA_DIR` set they fail instead of
  creating `~/.screenpipe/data`; that failure is expected in an isolated
  shell.
- `screenpipe-core` `pipes::tests::test_should_run_cron_stale_last_run_waits`
  depends on the local time zone and fails at some hours of the day.
