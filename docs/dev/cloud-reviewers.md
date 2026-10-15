# Reviewer briefs for cloud sessions

Every WP, fix and conflict resolution gets two **independent** reviews before approval
([cloud-runbook.md](cloud-runbook.md), sections 3.5 and 4.4). Start both reviewers in parallel as
subagents (Agent tool, general-purpose). Give each the brief below with the placeholders filled in.
Neither reviewer sees the other's report. Neither edits tracked files: they read, build, run tests and
write scratch files under `$FP_STATE/review/` only.

## Shared preamble (put it at the top of both briefs)

```text
You review Freenpipe work package <WP-ID> ("<title>") on branch <branch>, head <sha>, in the
repository checkout at <repo path>. Before any build or test command, run `. ~/.freenpipe-env.sh`.

Read first:
- docs/clean-room/PLAN.md, section "#### <WP-ID> ..." (items, depends_on, specs, owner paths,
  acceptance checks) and sections 2, 5 and 6 (ground rules, integration seams, shared hotspot files)
- docs/clean-room/DECISIONS.md (binding; it wins over PLAN.md)
- the referenced sections of docs/clean-room/specs/*.md
- CLAUDE.md
- the change itself: `git diff origin/main...<sha>` and `git log --stat origin/main..<sha>`
  <for a re-review: "only the commits since <reviewed-sha>: git diff <reviewed-sha>..<sha>">

Do not modify tracked files and do not commit or push. Scratch files go in $FP_STATE/review/.
The VM is Linux. Windows-only code cannot be built here; judge it by reading, and say what CI's
windows job must prove.

Answer in exactly this shape:
VERDICT: APPROVE        (or VERDICT: CHANGES)
FINDINGS:
- [blocker|major|minor] <file>:<line> <what is wrong> -> <what would fix it>
  (or "none")
SUMMARY:
<3 to 6 lines for the PR description: what you checked, what you ran and the results>

Request CHANGES only for blocker or major findings. Minor findings go in the list but still allow
APPROVE.
```

## Verification reviewer

```text
Your job: is the WP correct and complete, and do its checks really pass?

1. Re-run the checks yourself; do not trust the PR text. At least `cargo check --tests` and
   `cargo test` for every crate the change touches, the app crate check if it is touched, `bun test`
   for touched frontend code (if node_modules exists), and
   `bun test scripts/ci/vendor-guard.test.ts`. Record each command and its result.
2. Go through every acceptance check in the PLAN.md section. For each one, decide whether it passed
   here, is correctly left to CI (windows or linux job), or is correctly recorded as a Windows smoke
   checklist entry with clear steps and an expected result. A check marked done without evidence is
   a blocker.
3. Probe edge cases the tests miss: empty and huge inputs, missing files and directories, concurrent
   calls, error paths, restarts, upgrade from a screenpipe-shaped data dir (interop identifiers must
   match the spec exactly). Write small scratch tests under $FP_STATE/review/ if that helps, and
   report a missing test as a finding.
4. Check every item id the WP lists against the spec text: behaviour, defaults, identifiers,
   numeric constants.
5. Windows code under cfg(windows): read it for type errors, missing imports, wrong feature flags
   in Cargo.toml and wrong API use, and say what the windows job must show.
6. Regressions: changed behaviour outside the WP's items, especially in shared hotspot files
   (PLAN.md section 6), TESTING.md areas (windows, tray, monitors, audio), and generated files
   (tauri.ts must be regenerated, not hand-edited).
```

## Compliance reviewer

```text
Your job: does the change respect the clean-room, de-cloud, scope and security rules?

1. Clean room. Run `bash scripts/ci/verbatim-diff.sh origin/main <sha>` and judge every hit: an
   interop identifier, SQL or moved MIT base code is acceptable when the PR justifies it; prose,
   UI copy, prompts or comments shared with the target are blockers. Look for code that reads like
   decompiled output or bundled frontend JavaScript (minified names, odd control flow) and for text
   that may come from post-relicense upstream material (anything after 2026-06-10 or from upstream's
   ee/). Built-in pipes, skills and prompts must be original writing.
2. Interop identifiers (routes, JSON fields, SQL names, CLI flags, settings keys, env vars, event
   and file names) must match the spec exactly; everything else must be original.
3. De-cloud. No vendor host (screenpi.pe, screenpipe.com, mediar, PostHog, Sentry and the hosts the
   vendor guard lists), no API keys, DSNs or vendor OAuth client ids, no telemetry on by default.
   New network calls must be user-configured (BYO) or third-party and user-initiated, and must go
   through the endpoint switchboard where the plan says so.
4. Scope. Only the WP's owner paths and regions changed (PLAN.md section 6 for shared files); no
   unrelated refactors or formatting churn; no new settings keys outside W1-02's rule; no new
   migrations after W1-01; DECISIONS.md respected.
5. Security. Path traversal in file APIs, missing auth on new HTTP routes, shell and command
   injection, secrets in logs or plain files (use the secret store), unsafe deserialisation, CORS
   and Origin checks on local servers, and network listeners bound beyond 127.0.0.1 without a
   setting.
6. Hygiene. New source files carry the two-line Freenpipe header; upstream headers stay on edited
   files; no local machine paths; every commit message ends with the
   "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" trailer
   (`git log --format=%B origin/main..<sha>`); new dependencies have licences compatible with MIT
   distribution, and no new git dependency points at the vendor's organisations.
7. Dev isolation. Tests and scripts never use ~/.screenpipe or ports 3030/11435.
```

## Merge resolution variant (merger, `reapprove`)

Use the shared preamble and the brief for each role, but replace the "change itself" line with:

```text
- only the conflict resolution: $FP_STATE/resolution.diff (from `git show --remerge-diff <sha>`).
  It shows how the merge commit differs from a plain textual merge of main into the branch.
  Judge whether both sides' intent survived; ignore the rest of the WP, which was approved before.
```

The verification reviewer re-runs the checks for the crates the resolution touches. The compliance
reviewer checks that the resolution adds nothing outside both sides' changes.
