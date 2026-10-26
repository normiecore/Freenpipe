# Coordinator handoff

Updated 2026-10-26 17:45 UTC. Git, PR heads and CI are the source of truth; this file is a snapshot.

## State

- Waves 0 and 1 are merged (18 WPs). Wave 1 integration (runbook 4.7) has not been recorded yet: it needs a
  full Linux build of main and was held for disk.
- Wave 2 core is complete: W2-01, W2-04, W2-05, W2-08, W2-09, W2-10, W2-11, W2-12, W2-13, W2-16, W2-17,
  W2-18, W2-19, W2-22, W2-23, W2-26, W2-27. Wave 3 core merged: W3-30 (36/155 overall). Since W2-01 the read
  pool is `query_only`: writes go through `write_pool()` or the write queue.
- The account's weekly spend limit stopped all agents from 2026-10-21 04:40 to 2026-10-26 17:00 UTC.
- Building (relaunched 2026-10-26 on main df61031): W3-11 (from checkpoint 1975c92) + W3-12 (from f9cf9da),
  W3-25 + W3-14 (third attempts; nothing was committed before the earlier stops).
- Core set still to start (decision 6), all ready: W3-02, W3-06, W3-08, W3-13, W4-05; then W4-02 (after
  W3-06).

## How work runs

- `tools/cloud/workflows/wp-batch.js` with `lean: true`. An APPROVE that still lists findings gets one fix
  round and a re-review (`fix_minors`); reviewers rate findings by consequence.
- `cargo` on PATH is `tools/cloud/cargo-wt.py`: crates a worktree changed against origin/main (vendored
  `third_party/` crates included) get per-worktree artifacts in the shared target dir. Without it a worktree
  can link another worktree's build of the same crate.
- Disk is the limit (about 40 GB per session). After each merge: `git worktree remove`, then
  `python3 tools/cloud/prune-target.py --delete`; delete idle worktrees' `.next` folders.
- The account's spend limit stops every agent mid-task about every 2-3 hours of four parallel workers, and
  a weekly limit can stop work for days.
  After the reset, re-launch each WP with a resume note naming its pushed checkpoint and uncommitted files;
  WPs whose implementation was pushed only need `mode: "review"`.
- Before merging an approved PR whose base is behind main, merge main locally in its worktree and run a
  `cargo check` of the touched crates (and `bun run bindings:check` when tauri.ts auto-merged).

## Before the first commercial release

- Release licence audit (decision 7): the PII model weights on screenpipe's Hugging Face (licence unknown),
  the bundled ffmpeg build, pyannote models, and `DirectML.dll` (Microsoft licence; only bundled with
  `FREENPIPE_BUNDLE_DIRECTML=1`). `docs/dev/bundled-runtime-licences.md` lists every shipped DLL and model.
- `release-app.yml`: fill `VULKAN_SDK_SHA256` and `VULKAN_RUNTIME_SHA256` (empty, so the hash check only
  warns; Authenticode is enforced).
- The redactor's ONNX sessions in the x64 release use the System32 `DirectML.dll` without W2-19's
  export/adapter/`SCREENPIPE_DIRECTML` checks (W2-19 open question 16): route them through the same policy
  (W4-15) or ship the redactor on CPU.
- Trademark: the CLI binary is still named `screenpipe`.
- W2-27 follow-ups (PR #31): whisper/ggml must not run AVX2 code at static init or compatibility mode never
  engages (only SMK-W2-27-12 covers it); off Windows a refused second launch only writes to stderr; the
  CpuCompatMode notice names Parakeet even in builds without it.
- W3-30 follow-up (PR #29): a capture-only settings save restarts recording during a low-disk pause (W5-20,
  parked, should restart only if recording was on before the save).
- W2-11 follow-ups (PR #30): a resume racing a stop can answer 500 although the pause is lifted; a stale
  detached pause can re-pause a display after a resume in a narrow sequence.
- W2-10 follow-up: with tower_http debug logging on, `?token=` frame URLs put the local API key in the logs
  (the app's filter sets `tower_http=error`); strip the query in the TraceLayer span.
- W2-23 follow-ups (PR #27, approval comment 6302600646): Clear Cache still deletes the user's global
  `~/.pi/agent` (W8-02); a failed tree swap plus a failed restore during a Pi rebuild leaves node_modules
  missing until the next start; seed and settings writes can race on first start (the engine starts the pipe
  scheduler before `ensure_installed`); `ensure_bash_available` runs on an async worker and can block it
  up to 25 s when a bash candidate hangs.

## Older follow-ups (wave 0)

- `/search?filter_pii=true` ignores `piiBackend=local` (privacy_filter.rs).
- Provider keys (`aiGatewayApiKey` and others) sit in plain `store.bin` unless store encryption is on.
- The MCP server's last API-key fallback runs whatever `screenpipe` is on PATH.
- `packages/ai-gateway` still contains vendor hosts the vendor guard reports as informational.
