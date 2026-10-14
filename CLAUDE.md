# CLAUDE.md

Freenpipe captures accessibility trees (OCR as fallback), audio and conversations, and indexes them locally for AI consumption. It is a fork of screenpipe's last MIT-licensed commit, extended to feature parity with screenpipe v2.7.104 through clean-room reverse engineering.

## Provenance and clean-room rules (read first)

- **Base:** upstream commit `892199f` (2026-06-10), the last commit before screenpipe moved from MIT to the Screenpipe Commercial License. Everything imported in the first commit is MIT, Copyright (c) 2024-2026 louis030195. Keep `LICENSE.md` and `NOTICE.md` intact.
- **Never consult post-relicense upstream material.** That covers screenpipe source, commits, diffs, PRs, issues, docs or changelogs dated after 2026-06-10, the `main` branch of github.com/screenpipe/screenpipe (or mediar-ai/screenpipe), and anything under upstream's `ee/` directory (proprietary, excluded). Do not clone, fetch or web-browse them.
- **Delta features (v2.5.27 → v2.7.104) come only from REA evidence** about the installed shipped binary, never from upstream source. That evidence is binary strings, embedded schema text, recovered frontend behaviour, and API/route/flag inventories.
- **Write original code.** Do not paste decompiled code, extracted frontend JavaScript, or extracted prose/LLM-prompt text verbatim. Turn the evidence into a spec, then implement it in your own words.
- **Interoperability identifiers may match exactly:** HTTP route paths, JSON field names, SQL table/column/index names, CLI flag names, settings keys, environment variable names, and file/directory layout. Freenpipe should be able to read data written by screenpipe.
- **No phoning home.** Never send data to screenpi.pe / Mediar / Negentropy services, and never embed their API keys, PostHog keys or Sentry DSNs. Telemetry stays off. Cloud-only features get a local or bring-your-own-key equivalent.

## Product vision
Read `VISION.md` before making product decisions, adding features, or changing UX. Stability over features.

## Design
Refer to `DESIGN.md` before making design decisions.

## File headers
New source files (.rs, .ts, .tsx, .js, .jsx, .swift, .py) carry this header as their first comment (below any shebang or directive that must come first):

```
// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
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
