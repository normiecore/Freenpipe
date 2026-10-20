# third_party

Copies of Rust dependencies that used to be fetched from git repositories owned by the upstream
vendor's GitHub organisation (`screenpipe`) or by its contributors' personal accounts
(`divanshu-go`, `neo773`, `tribhuwan-kumar`). The owner decided to keep them in this repository so
that a Freenpipe build never depends on those repositories staying online or unchanged
(`docs/clean-room/DECISIONS.md`, decision 2; work package W0-13).

Everything under this directory except `README.md`, `vendor.sh` and `patches/` is third-party code
under its own licence. It is not covered by Freenpipe's MIT licence, it does not carry the Freenpipe
file header, and its licence files and source headers are kept exactly as upstream wrote them. The
clean-room verbatim gate (`scripts/ci/verbatim-diff.sh`) skips this directory and the vendor-host
guard (`scripts/ci/vendor-guard.test.ts`) does not scan it: upstream text naming the vendor is
expected here and is not Freenpipe's own writing.

## How the crates are consumed

- Every consumer uses a `path` dependency (or, for `muda` and `hf-hub`, a `[patch.crates-io]` entry
  with a path). That covers both workspaces: the root workspace and the Tauri app crate in
  `apps/screenpipe-app-tauri/src-tauri`, including the `cfg(windows)` and `cfg(target_os = "macos")`
  dependency tables.
- The root `Cargo.toml` lists `third_party` under `[workspace] exclude`, so these crates are path
  dependencies and not workspace members: `cargo test --workspace` does not run their tests and
  their dev-dependencies stay out of `Cargo.lock`.
- Switching from git to path changed nothing in either `Cargo.lock` except the removal of the
  `source = "git+..."` line of each vendored package. No other crate moved version.

## Vendored crates

All revisions are the ones the git dependencies were pinned to before W0-13. For the `screenpipe`
organisation's repositories the pinned commit's committer date was checked against the
2026-06-10 relicensing cut-off; all of them predate it. Dates below are committer dates.

| Directory | Crates (version) | Upstream | Revision | Date | Licence | Used by |
|---|---|---|---|---|---|---|
| `whisper-rs` | whisper-rs 0.16.0, whisper-rs-sys 0.15.0 | https://github.com/screenpipe/whisper-rs | `97e40705033724dee601e89f3bd701af8096c846` | 2026-03-24 | Unlicense (`LICENSE`) | screenpipe-audio, audiopipe |
| `rusty-tesseract` | rusty-tesseract 1.1.10 | https://github.com/screenpipe/rusty-tesseract | `08346c1de08d122c7121425abc79081c30dbf778` | 2024-07-23 | MIT (`LICENSE`) | screenpipe-screen |
| `cpal` | cpal 0.15.3, asio-sys 0.2.2 | https://github.com/screenpipe/cpal | `c7a610aee5c6037c9c69dc5bf7f2a850da8d8b12` | 2026-06-05 | Apache-2.0 (`LICENSE`) | screenpipe-audio |
| `sck-rs` | sck-rs 0.1.0 | https://github.com/screenpipe/sck-rs | `b7f48a1236b2339350baa1375a805c7ce32f88df` | 2026-05-30 | MIT (Cargo.toml only, see below) | screenpipe-screen (macOS) |
| `opf-rs` | opf 0.1.0 | https://github.com/screenpipe/opf-rs | `e725ad5e8a89ae46e4cb38022a26e746c37329b6` | 2026-05-08 | **CC BY-NC 4.0** (`LICENSE`, `NOTICE`), see below | screenpipe-redact (`opf-text`, opt-in only; not in any shipped build) |
| `tauri-helper` | tauri-helper 0.2.1, tauri_helper_core 0.2.1, tauri_helper_macros 0.1.4 | https://github.com/divanshu-go/tauri-helper | `e25ff1d2c5026189305a0251a1e06788efe1d2b3` | 2026-05-31 | MIT (`LICENSE`) | app crate (dependency and build dependency) |
| `eventkit-rs` | eventkit-rs 0.5.6 | https://github.com/divanshu-go/eventkit-rs | `9dd52787c8435d734ed443e2d5641f45b0177cae` | 2026-06-07 | Apache-2.0 (`LICENSE`) | screenpipe-connect (macOS), app crate |
| `vad-rs` | vad-rs 0.2.0 | https://github.com/divanshu-go/vad-rs-1 | `b14b26ec7093f5398ef6658020259cd7c130ea6c` | 2026-04-18 | MIT (Cargo.toml only, see below) | screenpipe-audio |
| `permission-flow` | permission-flow 0.1.40, tauri-plugin-permission-flow 0.1.40 (permission-flow-iced 0.1.40 is a member of its workspace, unused) | https://github.com/divanshu-go/permission-flow | `d7376a1b95acb78700e93505d8ca058e5ddb94ce` | 2026-06-09 | MIT (`LICENSE`; Swift package `crates/flow/PermissionFlow/LICENSE`, MIT) | app crate |
| `muda` | muda 0.19.2 | https://github.com/divanshu-go/muda | `9dc5de53e84040a63c9a5423b669ec892de8a671` | 2026-06-04 | Apache-2.0 OR MIT (`LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE.spdx`) | app crate (`[patch.crates-io]`; used by tauri and tray-icon) |
| `audiopipe` | audiopipe 0.2.0, antirez-asr-sys 0.1.0, qwen3-asr-sys 0.1.0 | https://github.com/divanshu-go/audiopipe | `2fd5c360651157a5df504394066e509ad4d1bdc9` | 2026-04-18 | MIT (Cargo.toml only, see below) | screenpipe-audio (`qwen3-asr`, `parakeet`), screenpipe-audio-eval |
| `hf-hub` | hf-hub 0.3.2 | https://github.com/neo773/hf-hub (fork of huggingface/hf-hub) | `437dcf4049233f4a0e0cfc4e1c1dbf5ed2c57760` | 2024-12-16 | Apache-2.0 (Cargo.toml only, see below) | screenpipe-audio (workspace dependency and root `[patch.crates-io]`) |
| `windows-icons` | windows-icons 0.1.1 | https://github.com/tribhuwan-kumar/windows-icons | `4dcd086b36bed286181cb87636ad6eb058ab7684` | 2025-02-01 | MIT (`LICENSE`) | app crate (Windows) |

Git submodules inside those repositories, copied at the commit their parent tree records:

| Path | Upstream | Revision | Date | Licence |
|---|---|---|---|---|
| `whisper-rs/sys/whisper.cpp` | https://github.com/ggerganov/whisper.cpp | `2eeeba56e9edd762b4b38467bab96c2517163158` | 2026-01-15 | MIT (`LICENSE`) |
| `audiopipe/crates/antirez-asr-sys/qwen-asr` | https://github.com/antirez/qwen-asr | `b00b789b17051aea61e9717458171100662318a4` | 2026-02-15 | MIT (`LICENSE`) |
| `audiopipe/crates/qwen3-asr-sys/qwen3-asr.cpp` | https://github.com/predict-woo/qwen3-asr.cpp | `f4df368778bb38bca1a0c1987c65d778cf791146` | 2026-02-11 | MIT, declared in its `README.md` (no licence file upstream) |
| `audiopipe/crates/qwen3-asr-sys/qwen3-asr.cpp/ggml` | https://github.com/ggerganov/ggml | `a8db410a252c8c8f2d120c6f2e7133ebe032f35d` | 2026-01-30 | MIT (`LICENSE`) |

Size: 1,543 files, 20.8 MB in a checkout. Most of it is the two copies of ggml (whisper.cpp's and
qwen3-asr.cpp's, at different revisions), which the C++ builds need; many of their files are
identical, so git stores 17.5 MB of distinct content, about 4 MB compressed.

### Licence notes

- **opf-rs is not open source.** Its `LICENSE` is Creative Commons Attribution-NonCommercial 4.0
  (CC BY-NC 4.0), with `NOTICE` explaining the arrangement and `LICENSE.upstream-apache2.txt`
  carrying the Apache-2.0 text of the upstream model it re-implements. CC BY-NC allows copying and
  redistribution with attribution for non-commercial purposes, which is why the copy is here,
  unmodified apart from the removals listed below (the licence asks for changes to be indicated;
  this file is that indication). Commercial use is not granted. Its `Cargo.toml` declares no
  `license` field; this file and the crate's `LICENSE` are the only declaration.
  **Status since W1-05 (owner decision 7):** opf-rs is reachable only through opt-in Cargo
  features: `screenpipe-redact/opf-text`, forwarded as `redact-opf-text` by `screenpipe-engine` and
  by the app crate. No default feature set and no feature set in `release-app.yml` or
  `release-cli.yml` enables them, so no build the project ships links opf-rs; a build that turns
  `redact-opf-text` on is for non-commercial use only. Without it the text PII worker goes from the
  v45 ONNX model straight to regex-only redaction. `scripts/ci/licence-guard.test.ts` (the CI
  `gates` job) fails if a default or release feature set reaches this crate again.
- **No licence file upstream:** sck-rs, vad-rs, audiopipe and the hf-hub fork ship no `LICENSE`
  file at the pinned revision; their `Cargo.toml` `license` field (SPDX) is the only grant, and the
  copies keep that field as it is. qwen3-asr.cpp declares MIT in its README and also has no
  licence file. Nothing was added on their behalf, because a licence file needs the copyright
  holder's name and upstream did not give one.
- whisper-rs is released under the Unlicense (a public-domain dedication).

## Local changes

Nothing in a vendored tree is edited by hand. `vendor.sh` builds each tree from the pinned commit,
removes the files listed here and applies the patches in `patches/<directory>/`.

Removed from every tree:

- version-control and CI metadata: `.git`, `.github/`, `.gitignore`, `.gitmodules`, `.gitattributes`;
- `Cargo.lock` files (they only apply when the crate is built on its own; the Freenpipe lockfiles
  decide the versions here);
- `CLAUDE.md`, `AGENTS.md`, `AGENT.md`: other projects' instructions for coding agents, which agents
  working in this repository would otherwise pick up;
- `*.py` scripts (model conversion, code generation and test helpers; no build step runs them, and
  the repository's `.gitignore` drops `*.py`).

Per tree:

| Tree | Also removed | Patches |
|---|---|---|
| whisper-rs | `examples/full_usage` (separate example crate with a WAV sample). In `sys/whisper.cpp` only what whisper-rs-sys's build uses is kept: `CMakeLists.txt`, `cmake/`, `include/`, `src/`, `ggml/` (its `CMakeLists.txt`, `cmake/`, `include/`, `src/`), `bindings/CMakeLists.txt`, `bindings/javascript/package-tmpl.json`, plus `LICENSE`, `AUTHORS` and `README.md` (the same set as the sys crate's own `include` list, plus the attribution files). Dropped: examples, tests, models, samples, scripts, grammars, CI, the language bindings, and the ggml backends no whisper-rs feature can enable (`ggml-cann`, `ggml-hexagon`, `ggml-musa`, `ggml-opencl`, `ggml-rpc`, `ggml-webgpu`, `ggml-zdnn`, `ggml-zendnn`). The CPU, BLAS, CUDA, HIP, Metal, SYCL and Vulkan backends stay. | none |
| rusty-tesseract | `img/` (README screenshots) | none |
| cpal | `examples/ios-feedback` (Xcode demo), `examples/wasm-beep` (separate demo crate), `Cross.toml`, `Dockerfile` | none |
| sck-rs | nothing beyond the common list | none |
| opf-rs | `tests/` (a parity test and its 1.9 MB safetensors golden), `tools/` (the script that dumps the golden), `examples/secret_probe.rs` (a benchmark whose fixtures are fake but realistic-looking API keys, which GitHub push protection rejects; `Cargo.toml` still declares the example, which only matters to `cargo run --example` inside opf-rs) | none |
| tauri-helper | nothing beyond the common list | none |
| eventkit-rs | `.config/` (nextest settings), `ci-check.sh`, `build-universal.sh` | none |
| vad-rs | `.vscode/`, `examples/whisper` (a demo crate with its own git dependency; `Cargo.toml` still lists it as a workspace member, which cargo ignores when the crate is a dependency) | none |
| permission-flow | `assets/` (README animation), `crates/tauri-plugin-permission-flow/examples` (Tauri demo app), `crates/tauri-plugin-permission-flow/package-lock.json`, `crates/flow/PermissionFlow/Example` (Xcode demo app) | `0001-regenerated-command-permissions.patch`: the five `permissions/autogenerated/commands/*.toml` files as the plugin's build script (tauri-plugin 2.5.3) regenerates them on every build, without upstream's `platforms = ["macOS"]` lines. That regenerated content is what the plugin has always embedded; committing it keeps builds from modifying the repository. |
| muda | `.changes/`, `renovate.json`, `examples/` (demo programs and a demo crate) | none |
| audiopipe | `scripts/` (model export scripts). In `qwen-asr`: `samples/` (18 MB of audio), `MODEL.md`, `MODEL_CARD_OFFICIAL.md`, `download_model.sh`. In `qwen3-asr.cpp` only `CMakeLists.txt`, `README.md`, `src/` and ggml's `AUTHORS`, `CMakeLists.txt`, `LICENSE`, `README.md`, `ggml.pc.in`, `cmake/`, `include/`, `src/` are kept, without the ggml backends no qwen3-asr-sys feature can enable (`ggml-cann`, `ggml-hexagon`, `ggml-hip`, `ggml-musa`, `ggml-opencl`, `ggml-rpc`, `ggml-sycl`, `ggml-virtgpu`, `ggml-webgpu`, `ggml-zdnn`, `ggml-zendnn`). | `0001-whisper-rs-path.patch`: the optional whisper-rs dependency points at `../whisper-rs` instead of the screenpipe/whisper-rs git repository (same revision, so one whisper-rs-sys stays in the graph). `0002-ggml-metal-prepatch.patch`: the edit qwen3-asr-sys's build script makes to two ggml-metal sources on every Metal build, applied ahead of time so macOS builds do not modify the repository (the build script sees its markers and leaves the files alone). `0003-explicit-execution-provider.patch` (W2-19): an `Accelerator` argument (`Auto`, `Cpu`, `DirectMl { device_id }`) for the ONNX Parakeet engine (`Model::from_pretrained_cache_only_with`, `ParakeetEngine::from_pretrained_cache_only_with` / `from_dir_with`), so screenpipe-audio picks the DirectML adapter and handles session recycling and CPU recovery itself; an explicit DirectML request that cannot register the provider is an error instead of a silent CPU fallback. The existing functions keep their behaviour. |
| hf-hub | `.pre-commit-config.yaml`, `flake.nix`, `flake.lock` | none |
| windows-icons | `output/` (PNGs the example writes) | none |

Trimming the ggml backends means a ggml option that no Cargo feature exposes (for example
`GGML_OPENCL`) cannot be switched on from these copies; re-run `vendor.sh` without the matching
`drop` line if that is ever needed.

## Git dependencies that stay

Established third-party repositories, not owned by the vendor or its contributors, stay as pinned
git dependencies:

| Crates | Upstream | Revision | Date | Licence | Used by |
|---|---|---|---|---|---|
| ffmpeg-sidecar 2.5.0 | https://github.com/nathanbabcock/ffmpeg-sidecar | `b0f48a514235d1278d860f4a0af66aa9d6e1618d` | 2026-05-02 | MIT | screenpipe-core |
| accessibility 0.3.0, accessibility-sys 0.2.0 | https://github.com/eiz/accessibility | `173e898d4a52ee0afe0224ecb458d324057856e7` | 2025-12-31 | MIT OR Apache-2.0 | screenpipe-core, screenpipe-screen (macOS) |
| tinfoil 0.1.0 | https://github.com/tinfoilsh/tinfoil-rs | `82143d767b540195dea265df0b01c4b4fc93c424` | 2026-05-08 | **AGPL-3.0** | screenpipe-engine, screenpipe-redact (both through the opt-in `tinfoil-enclave` feature only; not in any shipped build) |
| cidre 0.15.1 | https://github.com/yury/cidre | `8481f3f0dc7dd39bb5be80d9424bfac0cad3801a` | 2026-05-17 | MIT | screenpipe-engine, screenpipe-audio, screenpipe-screen, vendored cpal (macOS) |
| cidre 0.15.1 (unpinned `git` requirement of vendored sck-rs) | https://github.com/yury/cidre | root lockfile: `8481f3f0dc7dd39bb5be80d9424bfac0cad3801a`; app lockfile: `7049fbfbb92a5b3ed034d3c3162e2fccf3dd5f3e` | 2026-05-17; 2026-05-25 | MIT | sck-rs (macOS) |
| fix-path-env 0.0.0 | https://github.com/tauri-apps/fix-path-env-rs | `c4c45d503ea115a839aae718d02f79e7c7f0f673` | 2025-08-25 | Apache-2.0 OR MIT | app crate |
| nokhwa-bindings-macos 0.2.0, nokhwa-core 0.1.0 | https://github.com/CapSoftware/nokhwa | `0d3d1f30a78bb4616b4a4d0939a29ad0c1a8e14f` | 2024-11-13 | Apache-2.0 | app crate (macOS) |
| tauri-nspanel 2.0.1 | https://github.com/ahkohd/tauri-nspanel | `18ffb9a201fbf6fedfaa382fd4b92315ea30ab1a` | 2025-06-15 | Apache-2.0 OR MIT (licence files; no `license` field) | app crate (macOS) |

The sck-rs requirement on cidre names no revision, so `cargo update` may move those two lock
entries to cidre's newest commit; update them deliberately (`cargo update -p cidre@0.15.1` with the
matching source) rather than as a side effect.

tinfoil is AGPL-3.0, a copyleft licence that applies to the binaries linking it. It is the client
for a privacy enclave the user runs, which Freenpipe only contacts when the user configures an
endpoint (W0-06). **Since W1-05 (owner decision 7) it is optional:** only the opt-in
`tinfoil-enclave` feature of `screenpipe-redact`, `screenpipe-engine` and the app crate links it,
and no default or release feature set enables that feature. In a build without it the enclave
adapters and the search-time `filter_pii` enclave path report "not built with enclave support",
the settings page does not offer the enclave backend, and redaction stays on the device. It stays
in both lockfiles because cargo records optional dependencies there; `cargo tree` for a shipped
feature set does not reach it, which `scripts/ci/licence-guard.test.ts` checks in CI.

The Swift package in `permission-flow/crates/flow/PermissionFlowShim` resolves
https://github.com/brendonovich/swift-rs (pinned in its `Package.resolved`) through SwiftPM when the
app is built on macOS.

## Build notes

- Path dependencies are fingerprinted by file modification time, not by revision. A fresh checkout
  (every CI run) therefore recompiles these crates and everything that depends on them, including
  the whisper.cpp CMake build and, in the app workspace, `tauri` and `tray-icon` through `muda`.
  Expect CI builds to take a few minutes longer than with git dependencies; incremental local
  builds are unaffected.
- Compiler warnings in these crates are shown, as for any path dependency (cargo caps lints only
  for registry and git packages). They are upstream's warnings; do not fix them in place.
- Two build scripts write into their own source directory. Both writes are committed as patches,
  so a build leaves `git status` clean: tauri-plugin-permission-flow regenerates its command
  permission files on every app build (`patches/permission-flow/0001-*`), and qwen3-asr-sys edits
  two ggml-metal sources on macOS Metal builds (`patches/audiopipe/0002-*`; its build script then
  prints "ggml-metal patch already applied"). If a dependency update changes what they generate,
  `git status` shows it after the first build: refresh the patch rather than committing the
  modified files directly.

## Updating a crate

`vendor.sh` rebuilds a tree from cargo's local git cache (`$CARGO_HOME/git/db` and
`$CARGO_HOME/git/checkouts`) without touching the network:

```bash
third_party/vendor.sh audiopipe        # rebuild one tree in place
third_party/vendor.sh --check          # rebuild every tree in a temp dir and diff against the repo
```

To move a crate to another revision: make cargo fetch that revision once (a scratch project with a
git dependency on it, then `cargo fetch`), change the revision passed to `start` in the crate's
`v_*` function in `vendor.sh`, refresh or drop its patches, run the script, update the tables above and run
`cargo metadata --locked --offline` for both workspaces. Never take a revision of a `screenpipe`
organisation repository committed after 2026-06-10 (`CLAUDE.md`, provenance rules).
