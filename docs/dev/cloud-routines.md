# Cloud routines: prompts and schedules

Five routines run the cloud phase: three WORKERS and two copies of the MERGER (two copies because a
routine can fire at most once an hour, and the merger should run every 30 minutes). Every fire
starts a fresh cloud session that reads [cloud-runbook.md](cloud-runbook.md) and acts from git and
GitHub state alone. Set up the environment first: [cloud-environment.md](cloud-environment.md).

## Create each routine

At <https://claude.ai/code/routines>, choose **New routine** and set:

| Field | Value |
|---|---|
| Name | `freenpipe worker-1` (and so on, see the table below) |
| Instructions | the prompt below, exactly |
| Model | the strongest model you have (use the same one for all five) |
| Repository | `normiecore/Handvel` only |
| Environment | `freenpipe` |
| Trigger | Schedule, hourly (then set the exact minute, see "Schedules") |
| Connectors | remove all; none is needed |

## Prompts

### worker-1, worker-2, worker-3

Use the same text and change only the worker id:

```text
You are Handvel cloud worker "worker-1". Act as WORKER per docs/dev/cloud-runbook.md: read that
file first and follow it exactly, passing --worker worker-1 to every fp.py and status.py command
that takes one. Work only on the wp/<WP-ID> or fix/main-<sha7> branch that fp.py gives you, push
only through tools/cloud/fp.py, and never push to main or to a claude/ branch. Use the GitHub REST
API (fp.py, gh api) for pull requests; gh pr and gh issue do not work here. End with the session
report from runbook section 3.11.
```

### merger (two routines with the same prompt)

```text
You are the Handvel cloud MERGER. Act as MERGER per docs/dev/cloud-runbook.md: read that file
first and follow section 4 exactly. Change main only through tools/cloud/fp.py (REST merges and
publish-status), and push to wp/* or fix/* branches only to resolve merge conflicts with
fp.py sync-main. Never push to a claude/ branch and never implement work packages yourself. End
with the report from runbook section 4.8.
```

## Schedules

The routine form offers presets. To get these exact minutes, create each routine with the
**hourly** preset, then run `/schedule update` in the Claude Code CLI and set the cron expression.
The minimum interval is one hour, so `*/30` is rejected, which is why the merger has two copies.
Minutes past the hour avoid the late starts that runs scheduled exactly on the hour can get.

| Routine | Cron | Fires |
|---|---|---|
| `freenpipe worker-1` | `5 * * * *` | hourly at :05 |
| `freenpipe worker-2` | `25 * * * *` | hourly at :25 |
| `freenpipe worker-3` | `45 * * * *` | hourly at :45 |
| `freenpipe merger-a` | `15 * * * *` | hourly at :15 |
| `freenpipe merger-b` | `50 * * * *` | hourly at :50 |

Workers fire hourly, 20 minutes apart. Together the two merger routines run about every 30 minutes.

**Overlapping runs are fine.** A worker session often runs longer than an hour, so the next fire
starts while the previous session still works. Claims prevent collisions. A claim is a pushed
`wp/<WP-ID>` branch, and creating it fails for everyone but the first session. A session's own claim
stays protected while it pushes at least every 45 minutes, and it is only offered for `resume` after
90 minutes without a push. Two merger runs at once are also safe. A merge that already happened
fails harmlessly, and `publish-status` retries on a moved `main`.

**Usage.** Every run draws on your plan's usage. When the limit is reached, runs are rejected until
the window resets, and the next fire continues from git. If you hit the limit often, fire the
workers every two hours instead (`5 */2 * * *`, `25 */2 * * *`, `45 */2 * * *`) or pause one worker.
Merger runs with nothing to do are short.

## Going live (switching from the PC to the cloud)

1. On the PC, finish the running wave. Wave 1 must be merged and pushed to `origin/main`, and no local
   `wp/*` work may be left unpushed. The cloud only sees what is on GitHub.
2. Make sure the handoff changes (these docs, `tools/cloud/`, `tools/plan/`, Handvel CI) are on `main`.
3. Do the one-time setup in [cloud-environment.md](cloud-environment.md), including its check session.
4. Create the five routines. Open `freenpipe merger-a` and choose **Run now**. Its run publishes a
   fresh `docs/clean-room/STATUS.md`.
5. **Run now** on `freenpipe worker-1`. When it has claimed a WP (a `wp/<WP-ID>` branch appears on
   GitHub), let the schedules take over.

## Day to day

- **Progress**: `docs/clean-room/STATUS.md` on GitHub, or the routine run list at
  <https://claude.ai/code/routines>. A green run status only means the session ended without an
  infrastructure error, so read a run's transcript to see what it did.
- **Needs you**: PRs with an `fp-blocked:` comment (STATUS.md lists their review state as `blocked`),
  failed or partial wave integrations in STATUS.md, new questions in `docs/clean-room/OPEN-QUESTIONS.md`,
  and new entries in [windows-smoke-checklist.md](windows-smoke-checklist.md) to run when the PC is on.
  To release a block after deciding, comment `fp-unblocked` on the PR (with your decision in the same
  comment, or in OPEN-QUESTIONS.md).
- **Pause**: switch the routines off on their detail pages. Claims and PRs stay. When they are
  switched on again, stale claims are resumed or taken over automatically.
