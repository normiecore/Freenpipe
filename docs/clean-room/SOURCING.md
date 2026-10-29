# Spec sourcing policy

Owner decision 8 ([DECISIONS.md](DECISIONS.md)) froze binary analysis on 2026-10-29. This file says where a spec item may come from now, who may write it, how to record that, and how the pre-freeze specs are re-sourced.

## Folders

- `legacy-specs/` holds the thirteen specs written before the freeze from binary evidence. They are never edited, and only a redactor reads them.
- `specs/` holds re-sourced text only. Implementers read this folder and nothing else about screenpipe's later behaviour.

## Roles

Each role is a different person or a different Claude Code session. Nobody does two roles for the same item.

| Role | Reads | Writes | Never |
|---|---|---|---|
| **Redactor** | `legacy-specs/` | Interop items in `specs/`, tagged `interop`: identifiers only | Writes behaviour text, implements |
| **Behaviour author** | `specs/` (the redacted identifiers), the MIT base, the public sources below | Behaviour items in `specs/`, tagged `own`, `mit` or `public` | Reads `legacy-specs/`, PLAN.md behaviour summaries, or any summary of legacy behaviour; implements |
| **Implementer** | `specs/`, PLAN.md (dependencies, owner paths, acceptance checks), DECISIONS.md, Handvel code | Code, tests, the provenance record | Reads `legacy-specs/`, the analysis workspace, or any outside material about screenpipe |

Contamination between AI sessions travels through what goes into their context, so a coordinator never summarises legacy behaviour in a prompt or brief. Behaviour authors and implementers get item ids and file paths, not descriptions taken from `legacy-specs/`.

`.claude/settings.json` denies `legacy-specs/` to every Claude Code session in this repository. Redaction is therefore done by the owner, or by a session the owner starts outside this repository that is handed one legacy spec file at a time.

## Sources

| Tag | Source | Use it for |
|---|---|---|
| `own` | Handvel's own design | Any behaviour. This is the default whenever the other sources say nothing. |
| `mit` | The MIT base at `892199f` and its history | Behaviour and identifiers the base already had |
| `public` | See "Public sources" below | *What* a feature does, in one or two sentences. Never how it was implemented and never its wording. Record the URL and the date read. |
| `interop` | Identifiers redacted from `legacy-specs/` | HTTP routes and methods, JSON field names and types, SQL table, column, index and trigger names, CLI flags, settings keys, environment variables, event names, deep-link forms, and file and directory layout. A numeric constant counts only when the item names the client or stored format that depends on it. |

### Public sources

Prefer them in this order:

1. Third-party descriptions: press articles, reviews, user write-ups and videos made by people other than the vendor.
2. The vendor's own marketing and help pages on its website, read signed out.

Never use anything served from the upstream GitHub repository (releases, README, `docs/`, wiki, issues, pull requests, commits), post-relicense source, or anything behind a sign-in. Before the first `public` item, the owner reads the website's terms for clauses about using the site and building competing products, and records the version and date read here:

> Website terms reviewed: *not yet*

Claude Code sessions in this repository cannot fetch the vendor's domains (`.claude/settings.json`). A behaviour author who needs a vendor page reads it in a normal browser and records the URL.

## Never a source

- Any screenpipe binary, installer, bundled frontend or memory image, and any new extraction from them.
- The private analysis workspace. Nobody reads it after the freeze, spec authors included. Interop identifiers come only from `legacy-specs/`.
- Post-relicense screenpipe source, diffs, commit messages, pull request or issue text, and anything under upstream's `ee/` directory.

## Writing rules

- **Paraphrase.** Describe behaviour in your own words. Run the verbatim gate on every spec file you add or change (`python3 -I tools/clean-room/verbatim_gate.py <file>`).
- **Prefer your own design.** Where the vendor's choice is not needed for compatibility, write Handvel's own choice and tag it `own`.
- **No evidence pointers.** No hex offsets, `ev_…` ids, frontend chunk names or workspace paths in `specs/`. CI fails on them.

## Tagging

Every delta item in `specs/` ends its first line with a tag. `public` and `mit` give a reference:

```
- audio-012: echo cancellation can be switched per input device. [public: https://example.org/review, read 2026-11-02]
- api-031: `GET /meetings/{id}` returns `meeting_id`, `started_at`, `ended_at`. [interop]
- db-008: retention runs every 6 hours. [own]
```

An item that has both identifiers and behaviour is split: the identifiers as `<id>` tagged `interop`, the behaviour as `<id>b` with its own tag.

## Re-sourcing, item by item

Work is gated per item, not per spec:

1. **Pick the next work package to schedule.** List its item ids from PLAN.md.
2. **Redact.** The redactor copies each item's identifiers from `legacy-specs/<spec>.md` into `specs/<spec>.md`, tagged `interop`, with evidence pointers removed. Items with no identifiers get a placeholder line `- <id>: (behaviour pending)` and no tag.
3. **Write behaviour.** A behaviour author writes the behaviour for each item from the allowed sources, without reading `legacy-specs/`. An item with no public or MIT source is designed as `own` or dropped, with a line in `open-questions/<WP>.md`.
4. **Gate.** The verbatim gate and `bun test scripts/ci/clean-room-guard.test.ts` pass, and the tracker below is updated.
5. **Schedule.** The work package is ready when all its items in `specs/` carry a tag.

PLAN.md's work-package descriptions were written from the legacy specs. Implementers use PLAN.md only for item ids, dependencies, owner paths and acceptance checks. Behaviour comes from `specs/`.

## Merged code and maintenance

Code merged before the freeze stays as it is. A bug fix or refactor of that code may work from the code itself and from the `interop` items of its spec. New behaviour needs re-sourced items. If a fix is unclear without the legacy behaviour, record it in `open-questions/` and decide it as `own`.

## Tracker

| Spec | Items tagged / total | Redacted by | Behaviour by | Notes |
|---|---|---|---|---|
| `api.md` | 0 / ? | | | |
| `db.md` | 0 / ? | | | |
| `vision.md` | 0 / ? | | | |
| `audio.md` | 0 / ? | | | |
| `meetings_memory.md` | 0 / ? | | | |
| `agents_pipes.md` | 0 / ? | | | W4-02 and W4-05 items first |
| `connections.md` | 0 / ? | | | W4-05 items first |
| `frontend.md` | 0 / ? | | | |
| `shell.md` | 0 / ? | | | |
| `cloud.md` | 0 / ? | | | |
| `gap_retention-v2-engine.md` | 0 / ? | | | |
| `gap_workflow-sop-video-and-media.md` | 0 / ? | | | |
| `gap_health-lifecycle-internals.md` | 0 / ? | | | |
