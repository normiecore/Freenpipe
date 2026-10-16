# Coordinator handoff

Updated 2026-10-16 after W0-05 merged. Git, PR heads and CI are the source of truth; this file is a snapshot.

## Completed this session

- Recovered the state from GitHub after the shell could not reach its configured network proxy.
- Synced `main` at `8b13dff` into `wp/W0-05` as `3cd1bff`. Resolved the settings/OAuth overlaps while preserving commerce removal and BYO OAuth setup, readiness guidance and errors. Kept both sides' decisions and Windows smoke steps; removed one stale diff3 marker from the previous notes merge.
- Two independent resolution reviewers approved `3cd1bff`. Seven TSX files parsed locally; all 27 parent blobs matched their GitHub hashes. The complete remote merge tree matched all 1,806 intended file entries.
- All required checks passed on that PR head: Windows and Linux engine/app builds and tests, frontend build/tests, Tauri bindings, vendor guard, verbatim gate, Windows CLI integration and secret scan. [CI run](https://github.com/normiecore/Freenpipe/actions/runs/38840487237).
- Published the [head-specific review record](https://github.com/normiecore/Freenpipe/pull/1#issuecomment-6208422140), marked PR #1 ready, and merged it as `63b3c8388ee18abd8267db33344e959d5b154121`. No force pushes or branch deletions.

## Wave 0 state

Seven of the 154 planned work packages are merged (7/13 in wave 0). Four work-package PRs remain open.

| WP | PR | State at handoff | Next step |
|---|---|---|---|
| W0-01..W0-04 | - | merged before this session | - |
| W0-05 | #1 | merged (`63b3c83`) | - |
| W0-07 | #6 | merged (`55174d9`) | - |
| W0-08 | #7 | merged (`4c31752`) | - |
| docs branding scrub | #8 | merged (`935f5ed`) | - |
| W0-06 | #5 | draft at `c433f80`; latest head's CI green; conflicts with current main; PR body still a checkpoint and no fresh review record | the named PII finding was addressed by `ad1ad50` (local model after regex when no enclave); inspect the fix and finish acceptance/results; sync with current main, then run both resolution reviews and current-head CI |
| W0-09 | #2 | draft at `ed4f0ac`; latest head's CI green; conflicts with current main; PR body still a checkpoint and no fresh review record | the automatic GitHub release check was removed by `2f18483`; final CLI runtime checkpoint still needs review. On sync, keep W0-05's network-free `cli_reminder.rs` and no startup update-check spawn |
| W0-10 | #3 | draft at `4c6fc1e`; both reviewers approved that old head; conflicts with current main | now that W0-05 is merged, sync/resolve while preserving both sides; regenerate bindings rather than hand-merging; run both resolution reviewers, current-head CI and approval |
| W0-11 | #4 | draft at `4134aa3`; old head approved and green; conflicts with current main | merge after W0-06/09/10 among the current wave-0 PRs; it moves Rust to 1.94.1/sqlx 0.9. Sync and review the resulting head, then confirm current-head CI |
| W0-13 | - | blocked on W0-11 | vendor pinned dependencies after W0-11, with licence/provenance checks |
| W0-12 | - | blocked on preceding wave-0 work | finish user-visible identity and strict vendor guard last |

The earlier handoff's named W0-06 and W0-09 blockers describe findings before the latest fixes. Do not repeat those fixes blindly: review the pushed code and publish fresh results.

## Resume order and shared-file hazards

1. Run bootstrap and inspect current main CI first. W0-05's PR-head gate passed; its new main merge starts a separate CI run. At this handoff, gates and secret scan passed while the Windows, Linux and CLI integration jobs were still running. Fix a reproducible red main before other work.
2. Check for background work/active coordinator ownership before taking over a WP. Last observed implementation push was W0-06 at 2026-10-16 00:00:48 UTC; this alone does not prove whether the previous coordinator is still running.
3. Continue the open PR queue in `docs/dev/cloud-coordinator.md`. W0-10 can now be synced after W0-05. W0-06 and W0-09 need fresh verification/compliance review and complete PR descriptions.
4. Keep the W0-05 `use-usage-status.tsx` stub or drop its import/test block when W0-06's deletion wins. Check overlapping AI presets, privacy, standalone chat, onboarding and pi.rs changes.
5. Merge W0-11 after the other current PRs. Clear the obsolete Rust 1.93.1 target cache with `cargo clean --target-dir ~/.cache/freenpipe/target`, then continue W0-13 and W0-12.
6. Regenerate/publish the status board after each merge queue pass. Never hand-edit generated status files.

## Environment and routines

The status board was regenerated with the repository's unchanged `tools/plan/status.py` computation and writer, using a read-only connector snapshot adapter for git history, refs and REST reads. The adapter verified the generator's source hashes and complete first-parent history; the published JSON identifies its transport as `GitHub connector snapshot`. A normal `fp.py publish-status` run can refresh it from the next full checkout.

This Codex workspace had no checkout, Bun, Cargo or app dependencies. Shell HTTPS calls failed immediately because the configured proxy was unreachable; a network permission request did not complete. Work proceeded through the connected GitHub API, exact local file snapshots and GitHub CI. A fresh full build session should run `bash tools/cloud/bootstrap.sh`; local parser checks are not a replacement for the build.

The prior handoff said the hourly `freenpipe coordinator check-in` routine was enabled. This session did not inspect, create, re-enable or remove schedules. Check the actual routine/session state before starting a duplicate coordinator.

Disk/build lessons still apply: regenerate Tauri bindings for command changes, run the frontend build for frontend edits, remove only idle build output when disk is low, and preserve both sides' intent in settings-component conflicts.
