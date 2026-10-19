# Continuous integration

Cloud sessions are Linux VMs and cannot build for Windows, so GitHub Actions
is where Windows code compiles and its tests run. Every work-package PR is
gated on the checks below, and a merge waits for all of them to pass.

## What runs where

| Workflow (file) | Runs on | Triggers | Typical wall time |
|---|---|---|---|
| **Freenpipe CI** (`freenpipe-ci.yml`) | windows-2022, ubuntu-24.04 | every PR (any base), push to `main` and `ci/**`, dispatch | windows 70 min cold, linux 40 min cold, gates < 1 min (see Durations) |
| Windows CLI Integration Test (`windows-integration-test.yml`) | windows-2022 | every PR, push to `main` and `ci/**`, dispatch | 18-30 min |
| Secret Scan (trufflehog) (`gitleaks.yml`) | ubuntu | every PR, push to `main` | 1 min |
| Meeting Detection Eval (`eval-meeting-detection.yml`) | ubuntu | PRs that touch the meeting detector or `crates/screenpipe-meeting-eval`, dispatch | under 30 min (its timeout) |
| Release CLI (`release-cli.yml`) | all three OSes | `v*` tag, dispatch | release only |
| Release MCP (`release-mcp.yml`) | ubuntu | `mcp-v*` tag, dispatch | release only |
| Every other upstream workflow | | dispatch only (see the table at the end) | |

Cloud sessions cannot push tags (the git proxy refuses them), so no release
workflow can start from a cloud session except by dispatch.

### Freenpipe CI jobs

`windows` (windows-2022, 4 vCPU, 16 GB) and `linux` (ubuntu-24.04, the same
image as the cloud VM) run the same steps:

1. Checkout with `lfs: false` (Git LFS bandwidth is limited; nothing here
   needs LFS objects).
2. Rust from `rust-toolchain.toml`, two `Swatinem/rust-cache` caches (keys
   `engine` for the root workspace, `app` for
   `apps/screenpipe-app-tauri/src-tauri`), bun 1.3.10. Windows adds Python
   3.12, the MSVC environment and ONNX Runtime 1.22.0 (CPU; `ORT_DYLIB_PATH`
   points the tests at it). Linux installs the apt packages that upstream's
   Linux docs and `release-app.yml` list, including the `liblibopenblas`
   symlink that antirez-asr-sys needs.
3. `bun scripts/pre_build.js` fetches the sidecars: ffmpeg from the GyanD
   GitHub mirror, OpenBLAS and the bun binary on Windows (it exports
   `FFMPEG_DIR` and `OPENBLAS_PATH`); on Linux it links the apt ffmpeg and
   tesseract.
4. `cargo build --profile release-dev -p screenpipe-engine --bin screenpipe`.
5. **Engine tests:** `cargo test --profile release-dev --workspace --exclude
   screenpipe-rfdetr-mlx --no-fail-fast` through `scripts/ci/cargo_test.py`
   (quarantine below). That is every crate's unit tests, integration tests
   and doc tests, about 2,400 tests; tests that need model files, an API key
   or real hardware are `#[ignore]`d in the source and stay skipped.
   `screenpipe-rfdetr-mlx` is macOS-only.
6. **Quarantined engine tests (informational):** runs only the quarantined
   tests and reports each one; never fails the job.
7. `bun install --frozen-lockfile`, `bun run build` (the type-checked Next
   static export), `bun run test` (vitest suite plus the bun:test files).
8. App crate: `cargo build --profile release-dev` in `src-tauri`, against
   the real `out/` (a placeholder if the frontend build failed, so a Rust
   error is still reported).
9. **Tauri bindings check:** the same test as `bun run bindings:check`
   (`tauri_bindings_are_current`), run in the release-dev profile so it reuses
   step 8's build instead of compiling the whole graph again in `dev`.
10. **App crate unit tests:** the rest of the `screenpipe-app` binary's unit
    tests (about 230). The bindings check already compiled this binary, so
    this step only runs it. The `tests/` integration tests of the app crate
    are not run.

The environment sets `CARGO_PROFILE_{DEV,TEST,RELEASE_DEV}_DEBUG=0`, turns
off release-dev's thin LTO and incremental compilation (CI binaries are
never shipped, and LTO would be paid for every test binary), and caps
`CARGO_BUILD_JOBS` at 4. On windows-2022 only VS 2022 is installed, so the
`CMAKE_GENERATOR` workaround from `windows-build.md` is not needed;
`LIBCLANG_PATH` is set to the runner's LLVM.

Every step after the engine build runs even when an earlier one failed
(`!cancelled()`), so one run reports all independent failures.

`gates` (ubuntu, under a minute):

- **Vendor-host guard:** `bun test scripts/ci/vendor-guard.test.ts`, the way
  its header says, in report mode until W0-12 switches the default to strict.
  The remaining hits go to the job summary.
- **Third-party dependency guard:** `bun test scripts/ci/third-party.test.ts`
  fails when a Cargo.toml or Cargo.lock points at a git repository of the
  vendor or its contributors again (they are vendored in `third_party/`, see
  its README), when `third_party/` stops being excluded from the workspace,
  when a path dependency into `third_party/` is broken, or when
  `third_party/README.md` misses a vendored tree or a remaining git source.
- **Clean-room verbatim gate:** `scripts/ci/verbatim-diff.sh` collects the
  lines the change adds (PR: base...head; push: before...after; a new
  branch or a force push falls back to the merge base with `main`), from
  non-binary files outside `third_party/`, renames detected so a pure move
  adds nothing. It stages each touched file whole from the head commit and
  runs `tools/clean-room/verbatim_gate.py` over them, so line context is
  kept, then keeps the hits on added lines. Hits are printed, become
  `::warning` annotations and a table in the job summary. **Report-only**:
  it exits 0 whatever it finds, because moved or adapted upstream MIT code
  legitimately matches; the PR description must justify every hit. It exits
  2 (fails) only when it cannot run.
  Locally: `bash scripts/ci/verbatim-diff.sh origin/main HEAD`.

### What CI does not cover

Hosted runners have a virtual desktop but no real monitor, camera,
microphone or GPU, and nobody using the machine. Real screen capture, WASAPI
loopback, UI Automation trees of live apps, DirectML, the tray, the
installer and the updater need a real Windows machine; those checks go on the
owner's [Windows smoke checklist](windows-smoke-checklist.md). The Windows
CLI Integration Test is the closest CI gets: it runs the engine on the
runner with a virtual audio driver and checks that it records without
crashing.

## Durations

Measured on this repository's runners (minutes):

| Step | windows cold | linux cold |
|---|---|---|
| setup, sidecars | 1 | 3 |
| engine build | 17 | 10 |
| engine tests (compile + run) | 20 | 11 |
| frontend install, build, tests | 6 | 2 |
| app crate build | 22 | 14 |
| bindings check | 4 | 2 |
| cache save | 2 | 1 |
| **job** | **71** | **42** |

With warm caches from `main` the Rust steps drop to roughly a third; a PR
that touches only one crate rebuilds that crate and its dependents. The
`windows` job has a 240-minute timeout and `linux` 180 minutes.

## Caches

Only pushes to `main` save the Rust caches; PR and `ci/**` runs restore
them. A run can restore caches from its own ref and from the default
branch, and one full set (two OSes, engine + app) takes most of the
repository's 10 GB cache budget, so a second writer would evict main's.
Until the first main push after this workflow lands, every run builds cold.

```bash
gh api "repos/normiecore/Freenpipe/actions/cache/usage"
gh api "repos/normiecore/Freenpipe/actions/caches?per_page=100" \
  --jq '.actions_caches[] | [.ref, .key, (.size_in_bytes/1e9|tostring)] | @tsv'
# drop stale caches of a ref:
gh api -X DELETE "repos/normiecore/Freenpipe/actions/caches?key=<key>&ref=refs/heads/<branch>"
```

## Test quarantine

`scripts/ci/test-quarantine.txt` lists tests that fail for reasons a PR did
not cause, one per line: platform, suite (`engine` or `app`), the exact
libtest name, and why. `scripts/ci/cargo_test.py` passes them to libtest as
`--exact --skip <name>`, so the gating step ignores them, and the
informational step runs only them and warns when one passes again.

The initial list holds 14 engine tests that fail only on windows-2022, all in
files unchanged since the fork's base import: path-separator assertions, a
hard-coded `/tmp` path, file sizes of open files read through NTFS directory
entries, timer granularity (these flake rather than always fail), and an
`Instant` underflow on a freshly booted runner. They all pass on Linux.
Fixing them is ordinary work-package work; when a fix lands, delete its line
in the same PR.

To quarantine a new test: only for a failure that also happens on `main`,
with the reason and the source location, in its own commit.

**Crashed test binaries.** On Windows, the engine integration tests that
build the whole HTTP server with an audio manager (`tags_test`,
`transcribe_test`, `endpoint_test`) sometimes die with
`STATUS_ACCESS_VIOLATION` (0xc0000005) while their tests run in parallel:
`transcribe_test` crashed in the first run and `tags_test` in the second,
and each passed in the other run. `cargo_test.py` therefore retries a binary that failed without a
single failed test once, with the same package selection plus cargo's
target hint (`--test tags_test`) and `--test-threads=1`. A pass turns into a
`flaky test crash` warning annotation and a "FLAKY CRASH" line in the
summary; a second crash, or more than three crashed binaries, fails the
step. Treat a recurring warning as a bug to fix, not as noise.

## Reading results from a cloud session

`gh api` (REST) works from cloud sessions; GraphQL-based subcommands (`gh pr`,
`gh run`, `gh issue`) do not. Read endpoints of this public repository need no
auth at all; job logs, reruns and dispatches do.

```bash
R=repos/normiecore/Freenpipe
SHA=$(git rev-parse HEAD)

# Every check on a commit: the merge gate in one call
gh api "$R/commits/$SHA/check-runs?per_page=100" \
  --jq '.check_runs[] | [.name, .status, (.conclusion // "-"), .id] | @tsv'

# Workflow runs for a branch or a commit, newest first
gh api "$R/actions/runs?branch=wp/W1-02&per_page=5" \
  --jq '.workflow_runs[] | [.id, .name, .head_sha[0:7], .status, (.conclusion // "-")] | @tsv'
gh api "$R/actions/runs?head_sha=$SHA" --jq '.workflow_runs[] | [.id, .name, .status, .conclusion] | @tsv'

# Jobs and their steps (which step failed)
gh api "$R/actions/runs/<run_id>/jobs" \
  --jq '.jobs[] | .name as $j | .steps[] | select(.conclusion == "failure") | [$j, .name] | @tsv'

# Failing test names without downloading a log: check-run annotations
gh api "$R/check-runs/<job_id>/annotations" --jq '.[] | [.annotation_level, .title, .message] | @tsv'

# Full job log (plain text), then the summary block cargo_test.py prints last
gh api "$R/actions/jobs/<job_id>/logs" > job.log
grep -n -A40 '==== cargo-test summary' job.log
grep -n 'error\[E[0-9]*\]\|^error:' job.log | head    # compile errors
```

The job id is also the check-run id. A Windows run takes over an hour, longer
than a foreground command may run in a cloud session, so poll from a background
command and push your work before waiting:

```bash
until s=$(gh api "$R/actions/runs/<run_id>" --jq .status) && [ "$s" = completed ]; do sleep 120; done
gh api "$R/actions/runs/<run_id>" --jq .conclusion
```

### Merge gate

A PR may merge when every check run on its head commit has completed with
`success`, `skipped` or `neutral`, and these five check runs (job names) are
among them: `windows`, `linux` and `gates` (Freenpipe CI), `trufflehog`
(Secret Scan) and `test-windows` (Windows CLI Integration Test). Meeting
Detection Eval (`eval`) counts when it ran. A pending check is not a pass.
`python3 tools/cloud/fp.py ci` applies this gate (`tools/plan/status.py`
holds the list, overridable with `FP_REQUIRED_CHECKS`).

The verbatim gate passes even with hits, so the reviewers read its summary
(or the `verbatim gate` warning annotations) and check that the PR
description justifies each hit. The quarantine step never fails a job;
its warnings only say that a quarantined test passes again.

### Rerunning and triggering

```bash
gh api -X POST "$R/actions/runs/<run_id>/rerun-failed-jobs"   # only the failed jobs
gh api -X POST "$R/actions/runs/<run_id>/rerun"               # the whole run
gh api -X POST "$R/actions/jobs/<job_id>/rerun"               # one job
gh api -X POST "$R/actions/runs/<run_id>/cancel"
gh api -X POST "$R/actions/workflows/freenpipe-ci.yml/dispatches" -f ref=<branch>
```

Rerun a failed job only when the failure looks transient (a network fetch, a
runner lost); a real test failure fails again. To test CI itself, or any
commit without a PR, push it to a `ci/<name>` branch: Freenpipe CI and the
Windows integration test run on `ci/**` pushes. A newer push to the same PR
or branch cancels the run in progress (`main` runs always finish).

## Upstream workflows

None of upstream's workflow files are deleted. Those that make no sense for
the fork keep only `workflow_dispatch`, with a one-line `# Freenpipe:`
comment on the trigger saying why:

| Workflow | Was | Now | Why |
|---|---|---|---|
| Rust CI (`ci.yml`) | push/PR (path-filtered) | dispatch | Freenpipe CI builds and tests the pinned toolchain on Windows and Linux instead |
| Docs (`docs.yml`) | push/PR (docs paths) | dispatch | validates upstream's Mintlify site, which the fork does not publish |
| E2E Tests (`e2e-test.yml`) | push/PR to main, dispatch | dispatch | 90-minute UI e2e on three OSes built around upstream flows; it failed on every main push |
| Audio Eval (`eval-diarization.yml`) | PR (audio paths), dispatch | dispatch | pulls the pyannote models through Git LFS (limited bandwidth) plus about 2.2 GB of datasets per cold run |
| Code Quality (`style.yml`) | every push and PR | dispatch | upstream's clippy/fmt/knip/machete gate fails on the inherited base; re-enable once the tree is clean |
| sync-skills (`sync-skills.yml`) | push to main (skill paths), dispatch | dispatch | would open bot PRs mirroring product skills into `.claude/skills`, which cloud sessions load |
| test-cli-npm-e2e (`test-cli-npm-e2e.yml`) | push/PR (npm CLI paths) | dispatch | macOS-only test of the npm CLI wrapper, which the fork does not publish yet |
| Frontend Tests (`test-frontend.yml`) | push/PR (app paths) | dispatch | Freenpipe CI runs these suites and the build on Windows and Linux |
| Benchmark (`benchmark.yml`) | daily schedule, dispatch | dispatch | needs upstream's self-hosted macOS and Windows runners |
| close-inactive (`close-inactive.yml`) | daily schedule, dispatch | dispatch | the stale bot would close PRs idle for 7 days |
| E2E macOS (`e2e-macos.yml`) | 4-hourly schedule, dispatch | dispatch | a macOS UI e2e of upstream flows every 4 hours; Windows is the fork's target |
| Release App (`release-app.yml`) | push to main (commit-message gated), dispatch | dispatch | releases are deliberate; W0-03 made signing optional |
| Release CLI (`release-cli.yml`) | push to main (gated), `v*` tags, dispatch | `v*` tags, dispatch | as above |
| Secret Scan (`gitleaks.yml`) | PR into main, push to main | PR into any branch, push to main | kept as a merge check |
| Windows CLI Integration Test | PR into main, push to main | any PR, push to main and `ci/**` | kept as a merge check: the only job that runs the engine on Windows |

Unchanged: Meeting Detection Eval (path-filtered PR gate, no LFS), Release
MCP (`mcp-v*` tags), and the dispatch-only release-browser-extension,
test-cli-npm and test-signing. The release workflows skip signing when the
signing secrets are absent (W0-03).
