# CLAUDE.md

Handvel captures accessibility trees (OCR as fallback), audio and conversations, and indexes them locally for AI consumption. It is a fork of screenpipe's last MIT-licensed commit. It stays compatible with screenpipe's data, API, pipes and MCP configurations; features beyond that are Handvel's own design.

## Provenance and clean-room rules (read first)

- **Base:** upstream commit `892199f` (2026-06-10), the last commit before screenpipe moved from MIT to the Screenpipe Commercial License. Everything imported in the first commit is MIT, Copyright (c) 2024-2026 louis030195. Keep `LICENSE.md` and `NOTICE.md` intact.
- **Never consult post-relicense upstream material.** That covers screenpipe source, commits, diffs, PRs, issues, docs or changelogs dated after 2026-06-10, the `main` branch of github.com/screenpipe/screenpipe (or mediar-ai/screenpipe), and anything under upstream's `ee/` directory (proprietary, excluded). Do not clone, fetch or web-browse them. The one exception is for behaviour authors, who may use the public sources listed in `docs/clean-room/SOURCING.md` (third-party descriptions, and the vendor's own website read signed out; never anything served from the upstream GitHub repository).
- **Binary analysis is frozen (owner decision 8, 2026-10-29).** Never examine, run or extract anything from a screenpipe binary, installer or bundled frontend, and never read the private analysis workspace (including `C:\sp`). `.claude/settings.json` denies those reads.
- **Implement only from re-sourced items** in `docs/clean-room/specs/`. Never read `docs/clean-room/legacy-specs/` (the pre-freeze specs); only a redactor does, under `docs/clean-room/SOURCING.md`. The session or person that redacted or wrote an item never implements it. Use PLAN.md only for item ids, dependencies, owner paths and acceptance checks. Every work package adds a provenance record (`docs/clean-room/provenance/<WP>.md`), checked by `scripts/ci/clean-room-guard.test.ts`.
- **Write original code.** Do not paste decompiled code, extracted frontend JavaScript, or extracted prose/LLM-prompt text verbatim. Implement from the spec in your own words.
- **Interoperability identifiers may match exactly:** HTTP route paths, JSON field names, SQL table/column/index names, CLI flag names, settings keys, environment variable names, and file/directory layout. Handvel should be able to read data written by screenpipe.
- **No phoning home.** Never send data to screenpi.pe / Mediar / Negentropy services, and never embed their API keys, PostHog keys or Sentry DSNs. Telemetry stays off. Cloud-only features get a local or bring-your-own-key equivalent.

## Product vision
Read `VISION.md` before making product decisions, adding features, or changing UX. Stability over features.

## Design
Refer to `DESIGN.md` before making design decisions.

## File headers
New source files (.rs, .ts, .tsx, .js, .jsx, .swift, .py) carry this header as their first comment (below any shebang or directive that must come first):

```
// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe
```

Use `#` for Python. When you edit an existing upstream file, leave its original header alone; it is attribution.

## Platform
Windows (x86_64-pc-windows-msvc) is the primary target and must build and run. Do not break the macOS/Linux `cfg` paths; keep them compiling in principle even if they are not exercised here.

## Package manager
- `bun` for JS/TS (not npm or pnpm)
- `cargo` for Rust (toolchain pinned by `rust-toolchain.toml`)

## Testing
- `cargo test` for Rust, `bun test` for JS/TS
- **Tauri TypeScript bindings** (from `apps/screenpipe-app-tauri/`):
  - `bun run bindings:check` fails if `lib/utils/tauri.ts` drifted from the Rust surface
  - `bun run bindings:generate` regenerates `lib/utils/tauri.ts` after adding or changing commands
  - Commands need **both** `#[tauri::command]` and `#[specta::specta]`; without the latter a command is silently left out of `tauri.ts`
- **Regression checklist:** read `TESTING.md` before changing window management, tray, monitors or audio.

## git usage
Other agents are working in this repository in parallel. Never delete code you did not write, and never run `git reset --hard`, `git clean`, `git checkout -- .` or `git stash` over the shared tree. Commit only your own files.

## context
- always use progressive disclosure when designing agentic systems

## Cloud operation
Work packages are built by Claude Code cloud sessions (scheduled WORKER and MERGER routines), and the owner's PC is usually off.
- Current mode: one COORDINATOR session plays every role with Workflow batches (`docs/dev/cloud-coordinator.md`). Subagents it starts follow the prompts they are given.
- In a cloud session, read `docs/dev/cloud-runbook.md` first and follow it for your role. It is self-contained. Owner setup is in `docs/dev/cloud-environment.md`, routine prompts in `docs/dev/cloud-routines.md`.
- Start with `bash tools/cloud/bootstrap.sh`, and begin later build or test commands with `. ~/.freenpipe-env.sh &&`. For status, run `python3 tools/plan/status.py` (the board is `docs/clean-room/STATUS.md`). Workflow commands are in `python3 tools/cloud/fp.py`.
- Work only on your `wp/<WP-ID>` (or `fix/main-<sha7>`) branch, never on the default `claude/` branch. Only the merger changes `main` (REST merges and `status:` commits). Never force-push or delete branches. Every commit ends with the `Co-Authored-By` trailer. PRs and comments go through the REST API (`gh api`), because `gh pr` and `gh issue` are blocked.
- The VM runs Linux. Write `cfg(windows)` code anyway: CI's windows job compiles and tests it (`docs/dev/ci.md`). Hardware and runtime checks go into `docs/dev/windows-smoke-checklist.md`.
- Dev isolation still applies: never use `~/.screenpipe` or ports 3030/11435 in tests. Use a temp data dir and port 3035.
