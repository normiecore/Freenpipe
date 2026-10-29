---
name: pull request
about: submit changes to the project
title: "[pr] "
labels: ''
assignees: ''

---

## description

brief description of the changes in this pr.

related issue: #

## before

a screen recording of the app/cli before this change

## after

a screen recording of the app/cli after this change

## clean room (owner decision 8)

- [ ] I did not access any screenpipe binary, installer, bundled frontend, the analysis workspace, `docs/clean-room/legacy-specs/`, or post-relicense screenpipe source while making this change.
- [ ] Work-package PRs: `docs/clean-room/provenance/<WP>.md` is filled in and accurate.

## how to test

add a few steps to test the pr in the most time efficient way.

1. 
2. 
3. 

## desktop app checklist (if applicable)

If this PR adds or changes `#[tauri::command]` handlers or Rust types exported to the frontend, from `apps/screenpipe-app-tauri/`:

- [ ] `bun run bindings:generate` (if bindings changed)
- [ ] `bun run bindings:check`
- [ ] `bun run typecheck`

Commands are auto-collected via the vendored `tauri-helper` crate — no manual handler list edits in `main.rs`.

