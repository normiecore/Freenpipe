# Cloud coordinator

One Claude Code cloud session, the **coordinator**, drives the plan: it runs the work packages (WPs)
as Workflow batches inside its own VM, reviews them with independent subagents, and merges them.
It replaces the three worker routines and two merger routines in [cloud-routines.md](cloud-routines.md)
while usage limits, not VM count, are the bottleneck. The roles and rules are still those of
[cloud-runbook.md](cloud-runbook.md); this file only says how one session plays all of them.

**GitHub is the only state.** Pushed `wp/<WP-ID>` branches, draft PRs, their descriptions and the
review-record comments carry everything. A coordinator that lost its VM (idle reclaim, usage limit)
or a brand-new coordinator session continues from there with nothing else.

## Start or resume (every session, every check-in)

1. Environment: `bash tools/cloud/bootstrap.sh` (Bash timeout 600000 ms; repeat it if it timed out).
   It is idempotent and quick when the VM is warm.
2. State:
   ```bash
   . ~/.freenpipe-env.sh && git fetch -q origin && python3 tools/plan/status.py
   gh api 'repos/normiecore/Freenpipe/pulls?state=open&per_page=100' --jq '.[] | [.number, .head.ref, .draft, .head.sha[0:8]] | @tsv'
   ```
   Also check the session's own background work (`/workflows`, task notifications). A workflow that
   is still running owns its WPs; do not start a second one for them.
3. Subscribe to every open WP PR (`subscribe_pr_activity`, idempotent) so CI results wake the session.
4. Work the queue below, in order, then publish the status (`python3 tools/cloud/fp.py publish-status`).

## The queue

1. **Red `main`.** `fp.py ci --main --why`. Fix it first on `main` (small, validated, one push), or on
   a `fix/main-<sha7>` branch with a PR when the fix is not obvious.
2. **Open WP PRs with green CI and an approving review record for the head** (the PR description's
   `### Review` section names the head and both verdicts are APPROVE):
   ```bash
   python3 tools/cloud/fp.py approve <PR>            # posts the fp-review marker for that head
   ```
   mark the PR ready (GitHub MCP `update_pull_request` with `draft: false`; REST cannot), then
   `python3 tools/cloud/fp.py merge <PR>`. Merge in plan order (wave, then id); `fp.py merge` refuses
   a WP whose dependencies are not merged yet.
   - **Conflict (exit 4).** Small or generated-file conflicts: merge locally into `main` instead, with
     the title `fp.py merge` would use (`git merge --no-ff -m "Merge <WP-ID>: <title>" origin/wp/<WP-ID>`
     plus the commit trailer), resolve per runbook 4.3 (Cargo.lock, bun.lock and tauri.ts are
     regenerated, append-only docs keep both sides), run the affected Linux checks, push `main`.
     GitHub then shows the PR as merged. Conflicts in logic: run `tools/cloud/workflows/sync-resolve.js`
     (args `{wp, title, pr, session_url, other_wps}`; `other_wps` says what the merged WPs did in the
     conflicting areas). It merges origin/main into the branch, resolves keeping both sides, pushes,
     and runs both resolution reviewers. When CI is green on the new head, approve with
     `fp.py approve <PR> --verification-file F --compliance-file F` (the files hold the two
     resolution reviews) and merge.
3. **Open WP PRs with red CI.** Launch the workflow (below) for that WP with `mode: "ci-fix"` and a
   `ci_failure` note from `fp.py ci <PR> --why`. A failure that is also red on `main` is fixed on main.
4. **Open WP PRs whose review record says changes are still open** (`fp-blocked:` comment): read the
   findings. Fix them with an `impl` run (the worker resumes the pushed branch and the reviewers run
   again), or decide and `fp.py unblock` with the reason.
5. **Pushed `wp/*` branches without a PR and with no workflow running for them** (a VM died mid-WP):
   launch an `impl` run for each; the worker continues from the pushed commits.
6. **Ready WPs** (`status.py --next`, kind `ready`): launch `impl` runs in plan order.
7. **Wave integration** when every WP of a wave is merged: runbook section 4.7.

## Core set and lean mode (owner decision 6, 2026-10-20)

- Launch only the 32 core work packages listed in DECISIONS.md decision 6, in dependency order, even when
  `status.py` lists other WPs as ready. Everything else is parked until the owner widens the set.
- Start every new run with `lean: true` in the workflow args: one combined reviewer per WP that reads the
  diff (no full builds; CI builds and tests every PR) and at most one fix round. Under decision 7 an
  APPROVE that still lists findings also gets that fix round and a re-review (`fix_minors`, on by
  default), and reviewers rate findings by consequence: a user-visible regression, a hang path or
  touching the wrong process is at least major however rare the path.
- A merge of main into an approved branch with no code conflict needs no resolution review: merge it in
  the WP's worktree, run the vendor guard and the verbatim gate, push, wait for green CI, then approve with
  `--verification-file`/`--compliance-file` notes that name the reviewed head and state that the only
  change since is a conflict-free merge of main verified by CI. Use `sync-resolve.js` only when code
  conflicts.
- Owner decision 8: schedule a WP only when every item it lists is in `docs/clean-room/specs/`
  with a source tag (`docs/clean-room/SOURCING.md`), and merge it only with a filled-in
  `docs/clean-room/provenance/<WP>.md`. Redaction and behaviour writing are separate jobs, each in a
  different session from the implementer. Never put descriptions of legacy behaviour into any prompt
  or brief: give item ids and file paths only. W4-02 and W4-05 were opened before the freeze and are
  grandfathered; say so in their PRs.
- Spec decisions and Windows smoke checks go to per-WP files (`docs/clean-room/open-questions/<WP>.md`,
  `docs/dev/smoke/<WP>.md`), so parallel WPs no longer conflict in the two shared logs.

## Launching work

The workflow script lives at `tools/cloud/workflows/wp-batch.js`. Start it with the Workflow tool:

```text
Workflow({scriptPath: "/home/user/Freenpipe/tools/cloud/workflows/wp-batch.js",
          args: {base: "<origin/main sha>", session_url: "<this session's URL>",
                 wps: [{id: "W2-01", title: "<title from PLAN.md>"},
                       {id: "W2-02", title: "...", mode: "ci-fix", ci_failure: "..."}]}})
```

- Per WP it runs: implement (or fix CI) in `$FP_STATE/wt/<WP-ID>` → verification and compliance
  reviewers in parallel → up to two fix rounds → publish (push, open or update the draft PR with the
  filled-in template, post the full review record as a comment, block the PR if changes stay open).
- The worker pushes checkpoints itself (`fp.py push`) at least every 45 minutes.
- A workflow runs at most `min(16, CPUs - 2)` agents at once: two on the 4-CPU cloud VM. Give each
  run one or two WPs and run up to four runs side by side. Builds share `$CARGO_TARGET_DIR` and queue
  on its lock; keep at least 6 GB free (`df -h /`; clear `$CARGO_TARGET_DIR/debug/incremental` only
  when no workflow is running).
- `extra` adds WP-specific instructions (shared files with other WPs of the same wave, owner
  decisions). Note in it which other WPs of the wave edit the same files.
- `mode: "publish"` with `result` re-runs only the publish step for a finished result.
- `mode: "review"` with `head` (and optionally the earlier worker's `impl` result) skips the worker
  and runs the review, fix and publish steps on an already pushed head, for example when reviews
  were cut off by a usage limit or main was merged in after the implementation finished.
- Disk is the VM's tightest resource (about 30 GB in all). At every check-in run `df -h /`; below
  8 GB free, delete the worktrees' `apps/screenpipe-app-tauri/.next` folders (about 1 GB each,
  regenerated by the next `bun run build`) and, when no build is running, the shared target dir's
  `debug/incremental`. Remove a WP's worktree (`git worktree remove`) once its PR is merged, then run
  `python3 tools/cloud/prune-target.py --delete` (add `--stale-hours 6` when space is short). It deletes
  a removed worktree's own builds and old linked test executables, so it is safe while other builds run.
- Worktrees share one target dir, and cargo would let one worktree link another's build of a Freenpipe
  crate (artifact names no longer depend on the source path, and freshness is judged by mtime).
  bootstrap.sh therefore puts `tools/cloud/cargo-wt.py` first on PATH as `cargo`: inside
  `$FP_STATE/wt/<WP>` it gives every crate that differs from origin/main a per-worktree profile, so that
  crate and its dependents get their own artifacts there. Everything else stays shared.

## Usage limits and check-ins

An hourly routine fires into the coordinator session with a check-in prompt. When the account's usage
limit is reached, agents and turns fail; work pushed so far is safe on GitHub. The first check-in
after the limit resets resumes from "Start or resume" above. If the VM was reclaimed meanwhile,
worktrees and the build cache are gone: bootstrap rebuilds them, and `impl` runs resume each pushed
branch.

## Starting a fresh coordinator session

If the coordinator session itself is gone (archived, or its conversation is unusable), start a new
Claude Code cloud session on `normiecore/Freenpipe` in the Freenpipe environment with this prompt:

```text
You are the Freenpipe COORDINATOR. Read docs/dev/cloud-coordinator.md and follow it exactly:
bootstrap, read the state from git and GitHub, work the queue, and keep going wave by wave
toward the full plan in docs/clean-room/PLAN.md. Use the Workflow tool with
tools/cloud/workflows/wp-batch.js for all WP work. Create an hourly check-in routine into this
session (create_trigger, default target) named "freenpipe coordinator check-in" if none exists,
with the prompt: "Coordinator check-in: follow docs/dev/cloud-coordinator.md from 'Start or
resume'." Delete any check-in routine that fires into a different, archived session.
```

## Parallel coordinators

Two coordinators must not work the same WPs. If a second coordinator session is ever started to add
throughput, give each a disjoint set of waves or WP ids in its prompt; claims (pushed branches) and
`fp.py push` STOP messages are the safety net, not the plan.
