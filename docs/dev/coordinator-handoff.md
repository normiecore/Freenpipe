# Coordinator handoff

Resumed 2026-10-15 23:41 UTC on the owner's instruction to action all open PRs; the hourly check-in
routine "freenpipe coordinator check-in" is enabled again. If it is disabled, re-enable it (or start a
coordinator session with the prompt in [cloud-coordinator.md](cloud-coordinator.md)) to resume. Every branch below is pushed.
This file is a snapshot; git and the PRs are the source of truth.

## Wave 0 state

| WP | PR | State | Next step |
|---|---|---|---|
| W0-01..W0-04 | - | merged before this session | - |
| W0-07 | #6 | merged (55174d9) | - |
| W0-08 | #7 | merged (4c31752) | - |
| docs branding scrub | #8 | merged (935f5ed) | - |
| W0-05 | #1 | approved; CI green at 8f99d28; conflicts with main after W0-07 (6 TSX files: Gmail/Google cards, connections-section, post-install modal) | run `tools/cloud/workflows/sync-resolve.js` (an earlier run was stopped mid-merge and aborted cleanly), then approve with the resolution reviews and merge |
| W0-11 | #4 | approved; CI green at 4134aa3 (Rust 1.94.1, sqlx 0.9) | merge **last** in wave 0 (it moves the toolchain), then `cargo clean --target-dir ~/.cache/freenpipe/target` to drop the 1.93.1 cache |
| W0-06 | #5 | one major finding open (privacy_filter.rs: filter_pii fails closed without an enclave) | fix round running via `wp-batch.js` |
| W0-09 | #2 | one major finding open (cli_reminder.rs: unrequested GitHub release check); model URL verified | fix round running via `wp-batch.js` |
| W0-10 | #3 | both reviewers approved 4c6fc1e; review record being published to the PR; conflicts with main | after W0-05 merges: `sync-resolve.js`, then approve with the resolution reviews and merge |
| W0-13 | - | blocked on W0-11 | after W0-11 |
| W0-12 | - | blocked on all of wave 0 | last; README and docs already rebranded, app identity remains |

## Lessons recorded in the tooling

- Workers must run `bun run bindings:generate` for any Tauri command change and `bun run build` for
  frontend changes (CI fails otherwise); both are in `wp-batch.js` and the reviewer brief.
- Disk on the cloud VM is about 30 GB: delete worktrees' `.next` after builds, clean idle target dirs
  with `cargo clean --target-dir` (plain `rm -rf` on caches may be refused by the permission mode).
- WPs that touch the same settings components conflict in logic after each merge; use
  `sync-resolve.js` rather than resolving by hand on main.
