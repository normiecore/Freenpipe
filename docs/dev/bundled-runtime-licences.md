# Bundled runtime and model licences (release notes for the licence audit)

Owner decision 7 (`docs/clean-room/DECISIONS.md`) requires every shipped build to be usable
commercially. `scripts/ci/licence-guard.test.ts` checks the Rust crates each release links. This
page covers what that gate cannot see: native DLLs and model weights the installers ship or the app
downloads at run time. Each entry lists where it comes from, how it is pinned, its licence and its
status. "Bundled" means a release installer contains it today; "opt-in" means a build or user has to
ask for it; "audit" marks an open item for the release licence audit before the first release.

Third-party Rust and C/C++ sources vendored into the repository are listed in `third_party/README.md`.

## Windows installer: `<install>/` (from `src-tauri/bundled/runtime/`, W2-19)

| File | Source and pin | Licence | Status |
|---|---|---|---|
| `onnxruntime.dll`, `onnxruntime_providers_shared.dll` | NuGet `Microsoft.ML.OnnxRuntime.DirectML` 1.22.0, `runtimes/win-<arch>/native/`; `.nupkg` SHA-256 `29f9872d786236b79aa83f94482f3a17c14297e4833768d6d0ed4883ee732e60` (17,898,472 bytes) | MIT (Microsoft); package `LICENSE` and `ThirdPartyNotices.txt` ship as `onnxruntime-LICENSE.txt` and `onnxruntime-ThirdPartyNotices.txt` | bundled |
| `vulkan-1.dll` (x64 only) | LunarG Vulkan runtime components 1.4.313.0 (`https://sdk.lunarg.com/sdk/download/1.4.313.0/windows/vulkan-runtime-components.zip`), unpacked by `release-app.yml`; SHA-256 and Authenticode status are written to the job summary | Khronos Vulkan loader, Apache-2.0; the licence text from the runtime package (or the SDK's own) ships as `VulkanRT-License.txt` | bundled; audit: pin the SHA-256 after the first release run and confirm the shipped licence text covers the DLL |
| `DirectML.dll` | NuGet `Microsoft.AI.DirectML` 1.15.4, `bin/<arch>-win/`; `.nupkg` SHA-256 `4e7cb7ddce8cf837a7a75dc029209b520ca0101470fcdf275c1f49736a3615b9` (202,292,617 bytes) | Microsoft Software License Terms for DirectML (proprietary; redistribution allowed inside applications that run on Windows or Xbox; includes a data-collection clause). Not one of the licences decision 7 lists | opt-in only (`FREENPIPE_BUNDLE_DIRECTML=1`); audit decides whether releases may ship it. Without it ONNX Runtime uses the copy in Windows (System32) |
| `libopenblas.dll` | OpenMathLib/OpenBLAS v0.3.31 release zips (`scripts/setup_openblas.js`) | BSD-3-Clause | bundled (base) |
| ffmpeg DLLs and executables | GyanD `ffmpeg-8.0.1-full_build-shared` (x64) | audit (the full build includes GPL components) | bundled (base); audit already required by decision 7 |

## Model weights

| File | Installed at | Source and pin | Licence | Status |
|---|---|---|---|---|
| `silero_vad_v5.onnx` | `<install>/vad/`; cache `<cache>/screenpipe/vad/` | snakers4/silero-vad v5.1.2 at commit `6478567951ae5c9979ad7b234185b5515f4be7a1`; SHA-256 `2623a2953f6ff3d2c1e61740c6cdb7168133479b267dfef114a4a3cc5bdd788f` (2,327,524 bytes) | MIT; notice `crates/screenpipe-audio/models/silero/NOTICE.md`, shipped as `<install>/vad/NOTICE.md` | bundled, and the default download |
| `segmentation-3.0.onnx` | `<install>/models/pyannote/` | this repository's LFS storage at a pinned commit (W0-09) | MIT | bundled, default download |
| `wespeaker_en_voxceleb_CAM++.onnx` | `<install>/models/pyannote/` | as above | CC BY 4.0 (attribution in `crates/screenpipe-audio/models/pyannote/NOTICE.md`) | bundled, default download |

The engine looks for each model next to the executable first (`<exe_dir>/vad/`,
`<exe_dir>/models/pyannote/`, then screenpipe's flat `<exe_dir>/models/`), then in the per-user
cache, and downloads the pinned file only when neither has it. A download that does not match its
pin is rejected before it reaches the cache.
