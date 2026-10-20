# Coordinator handoff

Updated 2026-10-19 after W0-09 merged. Git, PR heads and CI are the source of truth; this file is a snapshot.

## Wave 0 state

Ten of the 154 planned work packages are merged (10/13 in wave 0).

| WP | PR | State | Next step |
|---|---|---|---|
| W0-01..W0-04 | - | merged | - |
| W0-05 | #1 | merged (`63b3c83`) | - |
| W0-06 | #5 | merged (`aa8ec6a`) | - |
| W0-07 | #6 | merged (`55174d9`) | - |
| W0-08 | #7 | merged (`4c31752`) | - |
| W0-09 | #2 | merged (`c83b674`) | - |
| W0-10 | #3 | merged (`7869d46`) | - |
| docs branding scrub | #8 | merged (`935f5ed`) | - |
| W0-11 | #4 | approved at `4134aa3`; main merged as `172a93c` (green); a sync-resolve run is merging main at `c83b674` and checking that W0-06/09/10 database code builds on Rust 1.94.1 and sqlx 0.9 | resolution reviews, CI, `fp.py approve 4 --verification-file ... --compliance-file ...`, mark ready, `fp.py merge 4` |
| W0-13 | - | blocked on W0-11 | vendor pinned dependencies with licence/provenance checks |
| W0-12 | - | blocked on W0-02..W0-11 | user-visible identity and strict vendor guard, last in wave 0 |

The Rust 1.93.1 build cache in `~/.cache/freenpipe/target` was cleared on 2026-10-19 (W0-11 moves the toolchain to 1.94.1), so the next build anywhere is a full rebuild.

## Must-do before W1-01 merges

PR #11 merged an unreviewed W1-01 checkpoint (3b88171) into main on 2026-10-20; it broke the engine tests and was reverted on main as `61f0faf`. Before W1-01's reviewed PR merges: merge origin/main into `wp/W1-01`, then `git revert 61f0faf` on the branch so 3b88171's changes come back, run the checks, push, wait for CI, then approve and merge. Skipping this silently drops the checkpoint's changes from the merge.

## Follow-ups from the wave 0 reviews

Minor findings the reviewers allowed through. None blocks a merge; pick them up in the WP that owns the area or in a small fix PR.

- Privacy, W0-06: `/search?filter_pii=true` chooses the remote enclave from the switchboard and `SCREENPIPE_PRIVACY_FILTER_REPO` alone and ignores `piiBackend=local` (`crates/screenpipe-engine/src/privacy_filter.rs`). Make it honour `piiBackend` like `is_configured_for` does elsewhere.
- Privacy, W0-06: the one-time privacy migration resets `piiBackend`, `analyticsEnabled` and `cloudMediaAnalysisEnabled` but not `enhancedAI` (`store.rs`, `lib/ai-gateway.ts`). An imported screenpipe store with `enhancedAI=true` keeps sending context to its configured provider.
- W0-06: `aiGatewayApiKey` sits in plain `store.bin` unless store encryption is on, like the other provider keys; move provider keys to the SecretStore with W1 settings work.
- W0-06: the gateway URL/key fields only show inside a gateway preset; Recording and Privacy hints point to Settings > AI where no field is visible. Custom presets without a URL get the wrong error text (`src-tauri/src/pi.rs`). The privacy-section hydrate effect can write the media-analysis marker before settings load.
- W0-09: `is_cached_download()` uses `starts_with(cache_dir)`, which misfires when the per-user install dir nests under the cache dir on Windows (`crates/screenpipe-audio/src/speaker/models.rs`). The launcher-rename retry in `cli_runtime.rs` sleeps on async callers.
- W0-09: the release pipeline must set `FREENPIPE_BUILD_CLI=1` (or `FREENPIPE_CLI_BINARY`) and `FREENPIPE_REQUIRE_BUNDLED_RUNTIME=1` so installers ship the CLI (OPEN-QUESTIONS W0-09 q4). Owner to confirm the PII weights' and pyannote's licences before a release (q11).
- W0-09: the MCP server's last API-key fallback runs whatever `screenpipe` is on PATH, which can be the installed post-relicense product.
- Tooling: `bun run build` regenerates `apps/screenpipe-app-tauri/lib/generated/screenpipe-skills.ts`, and the committed copy on main is stale against its SKILL.md sources. Regenerate and commit it once.
- Tooling: `scripts/ci/vendor-guard.test.ts` can time out on its first cold run in a fresh worktree; reruns pass.
- `packages/ai-gateway` still contains vendor hosts the vendor guard reports as informational (W0-06 q11); W0-12 or the owner should exclude or delete it.
- Two merge commits (`f34b726` on W0-06, `5249b8f` on W0-09) lack the Co-Authored-By trailer; history is not rewritten.

## Resume order

1. Bootstrap, read notifications, `df -h /`, `python3 tools/plan/status.py`.
2. Finish W0-11 (above). Then start W0-13 with `tools/cloud/workflows/wp-batch.js` (impl mode), then W0-12.
3. Wave 1 after wave 0 integration.

Workflow notes: `wp-batch.js` has `mode: "review"` for a pushed head whose reviews were cut off; `sync-resolve.js` merges main into an approved branch and runs two resolution reviewers. Approve with the resolution review files (`fp.py approve <PR> --verification-file F --compliance-file F`), mark the PR ready through the GitHub MCP `update_pull_request` (`draft: false`), then `fp.py merge <PR>` and `fp.py publish-status`. Agents cannot `rm -rf`; the coordinator deletes worktree `.next` folders with `rm -r`.
