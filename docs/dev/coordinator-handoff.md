# Coordinator handoff

Updated 2026-10-20 evening. Git, PR heads and CI are the source of truth; this file is a snapshot.

## State

- Waves 0 and 1 are merged (18 WPs). Wave 1 integration (runbook 4.7) has not been recorded yet: it needs a
  full Linux build of main and was held for disk.
- Wave 2 core merged: W2-05, W2-12, W2-13, W2-16, W2-17, W2-18, W2-19, W2-22, W2-26 (27/155 overall).
- Open: W2-08 (PR #24, approved; CI re-running after the schema-test fix 0ee4802), W2-23 (PR #27, approved,
  CI pending). Building: W2-27 + W2-11, W2-10 + W3-30.
- Core set still to start (decision 6): W2-01, W2-04, W2-09, W3-25, then the wave 3/4 core WPs as their
  dependencies merge.

## How work runs

- `tools/cloud/workflows/wp-batch.js` with `lean: true`. An APPROVE that still lists findings gets one fix
  round and a re-review (`fix_minors`); reviewers rate findings by consequence.
- `cargo` on PATH is `tools/cloud/cargo-wt.py`: crates a worktree changed against origin/main (vendored
  `third_party/` crates included) get per-worktree artifacts in the shared target dir. Without it a worktree
  can link another worktree's build of the same crate.
- Disk is the limit (about 40 GB per session). After each merge: `git worktree remove`, then
  `python3 tools/cloud/prune-target.py --delete`; delete idle worktrees' `.next` folders.
- The account's spend limit stops every agent mid-task about every 2-3 hours of four parallel workers.
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
- W2-23 follow-ups (PR #27): Clear Cache still deletes the user's global `~/.pi/agent` (W8-02); a failed tree
  swap during a Pi rebuild can leave node_modules missing; seed and settings writes can race on first start.

## Older follow-ups (wave 0)

- `/search?filter_pii=true` ignores `piiBackend=local` (privacy_filter.rs).
- Provider keys (`aiGatewayApiKey` and others) sit in plain `store.bin` unless store encryption is on.
- The MCP server's last API-key fallback runs whatever `screenpipe` is on PATH.
- `packages/ai-gateway` still contains vendor hosts the vendor guard reports as informational.
