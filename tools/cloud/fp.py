# Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Handvel
#
# Cloud workflow helper for WORKER and MERGER sessions (docs/dev/cloud-runbook.md).
# Uses git plus the GitHub REST API (`gh api`; GraphQL commands such as `gh pr` are blocked in
# Claude Code cloud sessions). Every subcommand prints what it did. Exit codes:
#   0 done   1 error   2 not available / lost a race / still pending (pick something else or retry)
#   3 already merged   4 merge conflict (resolve, then push)
#
# A TARGET is a work-package id (W2-01) or, for a red main, "fix-main" / a fix/main-<sha7> branch.
#
# WORKER
#   fp.py pick --worker worker-1             claim the first available action from status.py --next
#   fp.py claim W2-01 --worker worker-1      claim one target (ready, own resume, stale takeover,
#                                            continue an open PR, or fix-main)
#   fp.py push                               push the current wp/* or fix/* branch (checkpoint)
#   fp.py template W2-01 > "$FP_STATE/pr.md" PR body skeleton from the plan
#   fp.py open-pr W2-01 --body-file "$FP_STATE/pr.md"
#   fp.py open-pr fix/main-abc1234 --title "..." --body-file F
#                                            --draft opens it as a draft; --update-body replaces the
#                                            description of an already open PR
#   fp.py comment 123 --body-file F          post a comment (review results, notes)
#   fp.py ci 123 [--wait 9] [--why]          merge-gate CI state of the PR head (and why it failed)
#   fp.py log <job-id> [--tail 200]          job log (needs the log host, see cloud-environment.md)
#   fp.py rerun 123 | --main                 re-run failed CI jobs once (CI flake)
#   fp.py approve 123 [--verification-file v.md --compliance-file c.md]
#                                            post the approval marker (summaries default to the
#                                            "### Review" section of the PR body)
#   fp.py block 123 --reason "..." / fp.py unblock 123
# MERGER
#   fp.py merge 123 [--dry-run]              merge commit "Merge <WP-ID>: <title>" after all gates pass
#   fp.py checkout W2-01                     check out a WP branch at its remote head (no claim)
#   fp.py sync-main W2-01                    merge origin/main into wp/W2-01 (conflict resolution)
#   fp.py publish-status [--record-wave N --result pass --notes-file F]
#                                            regenerate STATUS.md/status.json and push a "status:" commit
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(ROOT, "tools", "plan"))
import ghapi  # noqa: E402
import status as S  # noqa: E402

TRAILER = "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
REMOTE = os.environ.get("FP_REMOTE", "origin")
OK, ERR, TAKEN, MERGED, CONFLICT = 0, 1, 2, 3, 4
STATUS_FILES = ["docs/clean-room/STATUS.md", "docs/clean-room/status.json"]
BRANCH_RE = re.compile(rf"(wp/({S.WP_ID})|{S.FIX_BRANCH})")


def say(*a):
    print(*a, flush=True)


def git():
    return S.Git(REMOTE)


def git_input(args, text):
    p = subprocess.run(["git", "-C", git().cwd, *args], input=text, capture_output=True, text=True,
                       encoding="utf-8", errors="replace")
    if p.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} failed: {p.stderr.strip()}")
    return p.stdout


def plan():
    wps, _ = S.load_plan()
    return {w["id"]: w for w in wps}


def wp_id(raw):
    wid = raw.strip().upper()
    if not re.fullmatch(S.WP_ID, wid) or wid not in plan():
        raise SystemExit(f"unknown work package {raw!r} (see docs/clean-room/PLAN.md)")
    return wid


def resolve_target(raw):
    """(kind, name, branch): ("wp", "W2-01", "wp/W2-01"), ("fix", "fix/main-abc1234", same) or
    ("fix-main", None, None) for the not-yet-named fix of a red main."""
    raw = raw.strip()
    if raw.lower() == "fix-main":
        return "fix-main", None, None
    if raw.startswith("fix/"):
        if not re.fullmatch(S.FIX_BRANCH, raw):
            raise SystemExit(f"bad fix branch name {raw!r}")
        return "fix", raw, raw
    if raw.startswith("wp/"):
        raw = raw[3:]
    wid = wp_id(raw)
    return "wp", wid, f"wp/{wid}"


def status_doc(fetch=False):
    offline = os.environ.get("FP_OFFLINE") == "1"  # tests: git only, no ls-remote/GitHub API
    doc = S.compute(SimpleNamespace(remote=REMOTE, offline=offline, no_fetch=not fetch, stale_hours=6.0))
    doc["wave_integration"] = S.load_recorded_waves()
    return doc


def main_code_sha(g):
    """Newest commit on origin/main's first-parent chain that is not a status commit."""
    for sha, _, subj in g.log(g.main_ref(), first_parent=True)[:50]:
        if not subj.startswith(S.STATUS_COMMIT):
            return sha
    return g.run("rev-parse", g.main_ref()).strip()


def tracked_dirty(g, allow=()):
    out = g.run("status", "--porcelain", "--untracked-files=no")
    return [ln[3:] for ln in out.splitlines() if ln[3:].strip() not in allow]


def push_branch(g, branch):
    """git push to refs/heads/<branch>. Returns (rc, kind, stderr) with kind ok|rejected|error."""
    rc, out, err = g.run("push", "-u", REMOTE, f"HEAD:refs/heads/{branch}", check=False)
    if rc == 0:
        return rc, "ok", err
    low = err.lower()
    if "[rejected]" in low or "non-fast-forward" in low or "fetch first" in low or "stale info" in low:
        return rc, "rejected", err
    return rc, "error", err


def remote_sha(g, branch):
    rc, out, _ = g.run("ls-remote", REMOTE, f"refs/heads/{branch}", check=False)
    return out.split("\t", 1)[0].strip() if rc == 0 and out.strip() else None


def current_branch(g):
    branch = g.run("rev-parse", "--abbrev-ref", "HEAD").strip()
    return branch if BRANCH_RE.fullmatch(branch) else None


# ------------------------------------------------------------------------------------- worker
def claim_one(raw, worker, own_idle_minutes=90.0, dry_run=False):
    """Claim one target. Returns (exit code, kind or None, branch or None)."""
    tkind, name, branch = resolve_target(raw)
    g = git()
    ok, err = g.fetch()
    if not ok:
        say(f"git fetch failed: {err}")
        return ERR, None, None
    doc = status_doc()
    actions = S.next_actions(doc, worker, own_idle_minutes)

    if tkind in ("fix-main", "fix"):
        fixes = [r for k, r in actions if k == "fix-main"]
        if not fixes or (branch and fixes[0]["id"] != branch):
            say("main does not need a new fix right now (CI not red after a re-run, or a fix/* branch is open)")
            return TAKEN, None, None
        row, kind = fixes[0], "fix-main"
        name = branch = row["id"]
    else:
        row = next(r for r in doc["wps"] if r["id"] == name)
        if row["status"] == "merged":
            say(f"{name} is already merged ({S.short(row.get('merge_commit'))})")
            return MERGED, None, None
        kind = {r["id"]: k for k, r in actions if k != "fix-main"}.get(name)
        if not kind:
            say(f"{name} is not available to {worker}: {row['status']} {S.detail(row)}".rstrip())
            return TAKEN, None, None

    dirty = tracked_dirty(g)
    if dirty:
        say("tracked files have uncommitted changes; commit (or discard your own) first: " + ", ".join(dirty[:10]))
        return ERR, None, None
    remote_ref = f"refs/remotes/{REMOTE}/{branch}"
    main_ref = f"refs/remotes/{REMOTE}/main"
    has_remote = g.ok("rev-parse", "--verify", "--quiet", remote_ref)
    if g.ok("rev-parse", "--verify", "--quiet", f"refs/heads/{branch}"):
        excl = [main_ref] + ([remote_ref] if has_remote else [])
        subjects = g.run("log", "--format=%s", f"refs/heads/{branch}", "--not", *excl).splitlines()
        real = [s for s in subjects if not S.CLAIM_RE.match(s)]
        if real:
            say(f"local {branch} has {len(real)} commit(s) that are on no remote branch; push or inspect them first")
            return ERR, None, None
    fresh = kind in ("ready", "fix-main")
    if not fresh and not has_remote:
        say(f"{branch} is not on {REMOTE}; cannot {kind}")
        return ERR, None, None
    g.run("switch", "-q", "--no-track", "-C", branch, main_ref if fresh else remote_ref)
    if kind == "continue-pr":
        # No claim commit on an open PR: it would re-run CI and void the approval for the head.
        # A competing session is caught by `fp.py push` (non-fast-forward = stop).
        pr = row["pr"]
        say(f"continuing PR #{pr['number']} for {name} on {branch} at {S.short(g.run('rev-parse', 'HEAD').strip())}: "
            f"CI {pr['ci']['state']}, review {pr['review'].get('state')}")
        return OK, kind, branch
    prev = (row.get("branch") or {}).get("claimed_by")
    note = {"ready": "", "fix-main": "", "resume": " (resume)",
            "takeover": f" (takeover from {prev or 'unknown'})"}[kind]
    git_input(["commit", "-q", "--allow-empty", "-F", "-"],
              f"claim: {name} by {worker}{note}\n\n{row['title']}\n\n{TRAILER}\n")
    head = g.run("rev-parse", "HEAD").strip()
    if dry_run:
        say(f"dry run: would push {branch} at {S.short(head)} ({kind})")
        return OK, kind, branch
    rc, res, err = push_branch(g, branch)
    if res == "rejected":
        say(f"lost the race for {name}: {branch} already exists or moved on {REMOTE}. Pick another.")
        return TAKEN, None, None
    if res == "error":
        say(f"push failed: {err.strip()}")
        return ERR, None, None
    if remote_sha(g, branch) != head:
        say(f"claim not confirmed: {REMOTE}/{branch} does not point at {S.short(head)}. Pick another.")
        return TAKEN, None, None
    say(f"claimed {name} as {worker} ({kind}); branch {branch} at {S.short(head)}")
    return OK, kind, branch


def worker_id(a):
    worker = a.worker or os.environ.get("FP_WORKER_ID")
    if not worker or not re.fullmatch(r"[A-Za-z0-9._-]+", worker):
        raise SystemExit("give --worker (or FP_WORKER_ID), e.g. worker-1")
    return worker


def cmd_claim(a):
    rc, _, _ = claim_one(a.target, worker_id(a), a.own_idle_minutes, a.dry_run)
    return rc


def cmd_pick(a):
    worker = worker_id(a)
    g = git()
    ok, err = g.fetch()
    if not ok:
        say(f"git fetch failed: {err}")
        return ERR
    tried = set()
    for _ in range(12):  # each lost race recomputes the list
        actions = [(k, r) for k, r in S.next_actions(status_doc(), worker, a.own_idle_minutes)
                   if (k, r["id"]) not in tried]
        if not actions:
            break
        kind, row = actions[0]
        tried.add((kind, row["id"]))
        rc, got, branch = claim_one("fix-main" if kind == "fix-main" else row["id"], worker,
                                    a.own_idle_minutes, a.dry_run)
        if rc == OK:
            say(f"picked\t{got}\t{row['id']}\t{branch}\t{row['title']}")
            return OK
        if rc not in (TAKEN, MERGED):
            return rc
    say("nothing to pick: no ready WP, no resumable claim, no open PR needing work")
    return TAKEN


def cmd_checkout(a):
    _, name, branch = resolve_target(a.target)
    if not branch:
        raise SystemExit("checkout needs a WP id or a fix/* branch")
    g = git()
    ok, err = g.fetch()
    if not ok:
        say(f"git fetch failed: {err}")
        return ERR
    if tracked_dirty(g):
        say("tracked files have uncommitted changes; commit or discard your own first")
        return ERR
    remote_ref = f"refs/remotes/{REMOTE}/{branch}"
    if not g.ok("rev-parse", "--verify", "--quiet", remote_ref):
        say(f"{branch} is not on {REMOTE}")
        return ERR
    g.run("switch", "-q", "--no-track", "-C", branch, remote_ref)
    say(f"on {branch} at {S.short(g.run('rev-parse', 'HEAD').strip())} (no claim made)")
    return OK


def cmd_push(a):
    g = git()
    branch = current_branch(g)
    if not branch:
        say("the current branch is not a wp/<WP-ID> or fix/* branch")
        return ERR
    for _ in range(2):
        rc, res, err = push_branch(g, branch)
        if res == "ok":
            say(f"pushed {branch} at {S.short(g.run('rev-parse', 'HEAD').strip())}")
            return OK
        if res == "error":
            say(f"push failed: {err.strip()}")
            return ERR
        g.run("fetch", "--quiet", REMOTE, f"+refs/heads/{branch}:refs/remotes/{REMOTE}/{branch}")
        incoming = g.run("log", "--format=%s", f"HEAD..refs/remotes/{REMOTE}/{branch}").splitlines()
        claims = [s for s in incoming if S.CLAIM_RE.match(s)]
        syncs = [s for s in incoming if s.startswith(S.SYNC_PREFIX)]
        if claims or len(syncs) != len(incoming):
            say(f"STOP: {REMOTE}/{branch} has commits from another session ({'; '.join(incoming[:3])}). "
                "Do not force-push. Leave this branch and pick other work.")
            return TAKEN
        # Only the merger's conflict-resolution merges arrived: take them and retry once.
        rc2, _, err2 = g.run("merge", "--no-edit", f"refs/remotes/{REMOTE}/{branch}", check=False)
        if rc2 != 0:
            say("merging the merger's sync commit conflicted; resolve, `git commit --no-edit`, then fp.py push again\n" + err2)
            return CONFLICT
    say("push still rejected after syncing; inspect the branch")
    return ERR


def cmd_template(a):
    tkind, name, branch = resolve_target(a.target)
    worker = a.worker or os.environ.get("FP_WORKER_ID") or "<worker-id>"
    if tkind == "wp":
        w = plan()[name]
        L = [f"## {name}: {w['title']}", "",
             f"Plan: `docs/clean-room/PLAN.md`, section `#### {name}` (wave {w['wave']}, size {w['size']}). "
             f"Worker: {worker}.", "", "### Items", ""] + [f"- {it}" for it in w["items"]]
        checks = w["acceptance"]
    else:
        L = [f"## {branch or 'fix/main-<sha7>'}: <one-line summary>", "",
             f"Repairs CI on main. Worker: {worker}.", "",
             "### Cause", "", "<!-- The failing job and step, the commit that broke it, and why. -->"]
        checks = ["Every check green on this PR (merge gate in docs/dev/ci.md)"]
    L += ["", "### What changed", "", "<!-- 3-8 bullets: behaviour, files, anything a reviewer must know -->", "",
          "### Acceptance checks", "",
          "<!-- One line per check. Mark [x] only if it passed. Say WHERE it ran: VM (Linux), CI windows job,",
          "     CI linux job, or Windows smoke checklist (entry id), or why it cannot run yet. -->", ""]
    L += [f"- [ ] {c}\n  - result:" for c in checks]
    L += ["", "### Tests run", "", "- `cargo test -p <crate>` ...", "",
          "### Verbatim gate", "",
          "`python3 -I tools/clean-room/verbatim_gate.py <changed files>`: <N> hit(s).",
          "<!-- For each hit: file:line, the shared run, and why it is an interop identifier (route, JSON field,",
          "     SQL, CLI flag, settings key, env var, file name) or moved MIT base code. Prose hits must be rewritten. -->",
          "", "### Windows hardware smoke items", "",
          "<!-- Entries added to docs/dev/smoke/<WP>.md (ids), or \"none\". -->", "",
          "### Review", "",
          "- Verification reviewer (head `<sha>`): <verdict and summary>",
          "- Compliance reviewer (head `<sha>`): <verdict and summary>", "",
          "### Spec decisions and open questions", "",
          "<!-- Ambiguities you decided; each is also added to docs/clean-room/open-questions/<WP>.md. Or \"none\". -->", "",
          "### Follow-ups", "", "<!-- Work left for later WPs, or \"none\". -->", "",
          f"<!-- fp-target: {name or 'fix-main'} -->"]
    say("\n".join(L))
    return OK


def find_open_pr(gh, branch):
    owner = gh.repo.split("/")[0]
    prs = gh.request("GET", gh.r("pulls"), params={"state": "open", "head": f"{owner}:{branch}"})
    return prs[0] if prs else None


def cmd_open_pr(a):
    tkind, name, branch = resolve_target(a.target)
    if tkind == "fix-main":
        raise SystemExit("open-pr needs the fix branch name, e.g. fix/main-abc1234")
    if tkind == "wp":
        title = f"{name}: {plan()[name]['title']}"
    else:
        if not a.title:
            raise SystemExit("a fix PR needs --title \"<what was broken and how it is fixed>\"")
        title = f"fix(main): {a.title.strip()}"
    g, gh = git(), ghapi.GitHub()
    if not remote_sha(g, branch):
        say(f"{branch} is not on {REMOTE}; run fp.py push first")
        return ERR
    with open(a.body_file, encoding="utf-8") as f:
        body = f.read()
    existing = find_open_pr(gh, branch)
    if existing:
        if a.update_body:
            gh.request("PATCH", gh.r(f"pulls/{existing['number']}"), body={"body": body})
            say(f"PR #{existing['number']} already open; description updated: {existing['html_url']}")
        else:
            say(f"PR #{existing['number']} already open: {existing['html_url']}")
        return OK
    pr = gh.request("POST", gh.r("pulls"),
                    body={"title": title, "head": branch, "base": "main", "body": body, "draft": bool(a.draft)})
    say(f"opened {'draft ' if a.draft else ''}PR #{pr['number']}: {pr['html_url']}")
    return OK


def cmd_comment(a):
    with open(a.body_file, encoding="utf-8") as f:
        post_comment(ghapi.GitHub(), a.pr, f.read())
    return OK


def pr_head(gh, number):
    pr = gh.pull(number)
    ref = pr["head"]["ref"]
    m = BRANCH_RE.fullmatch(ref)
    wid = m.group(2) if m else None
    return pr, wid, pr["head"]["sha"]


def explain_failure(gh, ci):
    for run in ci.get("runs", []):
        if run.get("conclusion") in (None, "success", "skipped", "neutral"):
            continue
        say(f"run {run['id']} ({run['event']}, attempt {run.get('attempt')}): {run.get('conclusion')} {run.get('url')}")
        jobs = gh.paginate(gh.r(f"actions/runs/{run['id']}/jobs"), key="jobs", max_pages=3)
        for job in jobs:
            if job.get("conclusion") in ("success", "skipped", "neutral", None):
                continue
            failed = [s["name"] for s in job.get("steps", []) if s.get("conclusion") == "failure"]
            say(f"  job {job['id']} {job['name']}: {job.get('conclusion')}; failed step(s): {', '.join(failed) or '?'}")
            try:
                anns = gh.paginate(gh.r(f"check-runs/{job['id']}/annotations"), max_pages=1)
            except ghapi.GitHubError:
                anns = []
            for an in anns[:30]:
                loc = f"{an.get('path')}:{an.get('start_line')}"
                say(f"    {an.get('annotation_level')} {loc}: {(an.get('message') or '').strip()[:400]}")
            say(f"    full log: python3 tools/cloud/fp.py log {job['id']} --tail 300")


def cmd_ci(a):
    gh = ghapi.GitHub()
    if a.main:
        g = git()
        g.fetch()
        sha = main_code_sha(g)
        label = f"main {S.short(sha)}"
    elif a.sha:
        sha, label = a.sha, f"commit {S.short(a.sha)}"
    else:
        _, _, sha = pr_head(gh, a.pr)
        label = f"PR #{a.pr} head {S.short(sha)}"
    deadline = time.time() + 60 * min(a.wait, 9)
    while True:
        ci = S.ci_for_sha(gh, sha)
        if ci["state"] not in ("pending", "none") or time.time() >= deadline:
            break
        time.sleep(60)
    say(f"{label}: CI {ci['state']}" + (f" (waiting for: {', '.join(ci['missing'])})" if ci.get("missing") and ci["state"] == "pending" else ""))
    for k, v in ci.get("checks", {}).items():
        say(f"  {k}: {v}")
    for r in ci.get("runs", []):
        say(f"  run {r['id']} ({r['event']}, attempt {r.get('attempt')}): {r.get('status')} {r.get('conclusion') or ''} {r.get('url')}")
    if a.why and ci["state"] == "failure":
        explain_failure(gh, ci)
    return {"success": OK, "failure": ERR}.get(ci["state"], TAKEN)


def cmd_log(a):
    gh = ghapi.GitHub()
    try:
        text = gh.get_text(gh.r(f"actions/jobs/{a.job_id}/logs"))
    except ghapi.GitHubError as e:
        say(f"could not download the log: {e}\nThe log API redirects to a log storage host; if the network "
            "allowlist blocks it, use `fp.py ci <PR> --why` (annotations) instead. See docs/dev/cloud-environment.md.")
        return ERR
    lines = text.splitlines()
    if a.grep:
        rx = re.compile(a.grep, re.I)
        lines = [ln for ln in lines if rx.search(ln)]
    say("\n".join(lines[-a.tail:]))
    return OK


def cmd_rerun(a):
    gh = ghapi.GitHub()
    if a.main:
        g = git()
        g.fetch()
        sha, label = main_code_sha(g), "main"
    else:
        _, _, sha = pr_head(gh, a.pr)
        label = f"PR #{a.pr}"
    ci = S.ci_for_sha(gh, sha)
    failed = [r for r in ci.get("runs", []) if r.get("status") == "completed" and r.get("conclusion") not in S.CI_PASS]
    if not failed:
        say(f"{label}: nothing to re-run (CI {ci['state']})")
        return OK
    rc = OK
    for r in failed:
        if (r.get("attempt") or 1) >= 2 and not a.force:
            say(f"run {r['id']} was already re-run (attempt {r.get('attempt')}); treat the failure as real")
            rc = ERR
            continue
        gh.request("POST", gh.r(f"actions/runs/{r['id']}/rerun-failed-jobs"))
        say(f"re-running failed jobs of run {r['id']} ({r['url']})")
    return rc


FOOTER = "\n\n---\n_Generated by [Claude Code](https://claude.ai/code)_\n"


def with_footer(body):
    body = body.rstrip()
    return body if body.endswith("_Generated by [Claude Code](https://claude.ai/code)_") else body + FOOTER


def post_comment(gh, number, body):
    c = gh.request("POST", gh.r(f"issues/{number}/comments"), body={"body": with_footer(body)})
    say(f"commented on PR #{number}: {c.get('html_url')}")


def review_section(body):
    """The "### Review" section of a PR body (up to the next ### heading), or None."""
    m = re.search(r"^###\s+Review\s*$(.*?)(?=^###\s|\Z)", body or "", re.M | re.S)
    text = m.group(1).strip() if m else ""
    return text if text and "<verdict" not in text else None


def cmd_approve(a):
    gh, g = ghapi.GitHub(), git()
    pr, _, sha = pr_head(gh, a.pr)
    local = g.run("rev-parse", "HEAD").strip()
    if local != sha and not a.force:
        say(f"local HEAD {S.short(local)} is not the PR head {S.short(sha)}; the reviewers must have reviewed the "
            "PR head (fp.py checkout <target>, review, then approve)")
        return ERR
    ci = S.ci_for_sha(gh, sha)
    if ci["state"] != "success" and not a.force:
        say(f"CI is {ci['state']} on {S.short(sha)}; approve only when every check is green")
        return TAKEN
    if a.verification_file and a.compliance_file:
        with open(a.verification_file, encoding="utf-8") as f:
            ver = f.read().strip()
        with open(a.compliance_file, encoding="utf-8") as f:
            comp = f.read().strip()
        summaries = f"### Verification reviewer\n\n{ver}\n\n### Compliance reviewer\n\n{comp}\n"
    elif a.verification_file or a.compliance_file:
        raise SystemExit("give both --verification-file and --compliance-file, or neither")
    else:
        section = review_section(pr.get("body"))
        if not section:
            say("the PR body has no filled-in \"### Review\" section; pass --verification-file and --compliance-file")
            return ERR
        reviewed = re.findall(r"head\s+`?([0-9a-f]{7,40})`?", section)
        if not reviewed or any(not sha.startswith(s) for s in reviewed):
            say(f"the PR body's reviews name head(s) {', '.join(reviewed) or '(none)'}, not the PR head {S.short(sha)}; "
                "re-run both reviewers on the new commits and pass --verification-file and --compliance-file")
            return ERR
        summaries = f"### Reviews (from the PR description)\n\n{section}\n"
    note = f"\n{a.note.strip()}\n" if a.note else ""
    post_comment(gh, a.pr, f"{S.APPROVAL}\nhead: {sha}\n{note}\n{summaries}")
    return OK


def cmd_block(a):
    post_comment(ghapi.GitHub(), a.pr, f"{S.BLOCKED} {a.reason.strip()}")
    return OK


def cmd_unblock(a):
    post_comment(ghapi.GitHub(), a.pr, f"{S.UNBLOCKED}: {a.note.strip()}" if a.note else S.UNBLOCKED)
    return OK


# ------------------------------------------------------------------------------------- merger
def cmd_merge(a):
    gh, g = ghapi.GitHub(), git()
    pr = gh.pull(a.pr, wait_mergeable=True)
    if pr.get("merged"):
        say(f"PR #{a.pr} is already merged")
        return MERGED
    if pr.get("state") != "open":
        say(f"PR #{a.pr} is {pr.get('state')}")
        return ERR
    if (pr.get("base") or {}).get("ref") != "main":
        say(f"PR #{a.pr} does not target main")
        return ERR
    ref, sha = pr["head"]["ref"], pr["head"]["sha"]
    if (pr["head"].get("repo") or {}).get("full_name") != gh.repo:
        say(f"PR #{a.pr} comes from a fork; only branches of {gh.repo} are merged")
        return ERR
    m = BRANCH_RE.fullmatch(ref)
    if not m:
        say(f"PR #{a.pr} is not from a wp/<WP-ID> or fix/* branch")
        return ERR
    wid = m.group(2)
    if wid and wid not in plan():
        say(f"PR #{a.pr}: {wid} is not in the plan")
        return ERR
    ci = S.ci_for_sha(gh, sha)
    if ci["state"] != "success":
        say(f"not merging {ref}: CI is {ci['state']} on {S.short(sha)}")
        return TAKEN
    commit = gh.request("GET", gh.r(f"commits/{sha}"))
    head_time = S.parse_iso(((commit.get("commit") or {}).get("committer") or {}).get("date"))
    review = S.review_state(gh.issue_comments(a.pr), sha, head_time)
    if review["state"] != "approved":
        say(f"not merging {ref}: review state is {review['state']} for head {S.short(sha)}")
        return TAKEN
    ok, err = g.fetch()
    if not ok:
        say(f"git fetch failed: {err}")
        return ERR
    if wid:
        w = plan()[wid]
        merged = S.merged_map(g, g.main_ref())
        unmet = [d for d in w["depends_on"] if d not in merged]
        if unmet:
            say(f"not merging {wid}: dependencies not merged yet: {', '.join(unmet)}")
            return TAKEN
        title = f"Merge {wid}: {w['title']}"
    else:
        title = f"Merge {ref}: {pr.get('title') or ref}"
    if pr.get("mergeable") is False:
        say(f"PR #{a.pr} ({ref}) conflicts with main ({pr.get('mergeable_state')}); run fp.py sync-main {wid or ref}")
        return CONFLICT
    if pr.get("mergeable") is None:
        say(f"GitHub has not computed mergeability of PR #{a.pr} yet; retry later")
        return TAKEN
    body = {"merge_method": "merge", "sha": sha, "commit_title": title,
            "commit_message": f"PR #{a.pr}. All checks green on {S.short(sha)}; verification and compliance "
                              f"reviews approved.\n\n{TRAILER}"}
    if a.dry_run:
        say(f"dry run: PUT pulls/{a.pr}/merge {json.dumps(body)}")
        return OK
    try:
        res = gh.request("PUT", gh.r(f"pulls/{a.pr}/merge"), body=body)
    except ghapi.GitHubError as e:
        if e.status == 405:
            say(f"GitHub refused the merge of PR #{a.pr}: {e.message}")
            return CONFLICT if "conflict" in (e.message or "").lower() else TAKEN
        if e.status == 409:
            say(f"PR #{a.pr} head moved since the checks; re-check next run: {e.message}")
            return TAKEN
        raise
    say(f"merged {ref} (PR #{a.pr}) as {S.short(res.get('sha'))}: {title}")
    g.fetch()
    return OK


def cmd_sync_main(a):
    _, name, branch = resolve_target(a.target)
    if not branch:
        raise SystemExit("sync-main needs a WP id or a fix/* branch")
    g = git()
    ok, err = g.fetch()
    if not ok:
        say(f"git fetch failed: {err}")
        return ERR
    if tracked_dirty(g):
        say("tracked files have uncommitted changes; commit or discard your own first")
        return ERR
    g.run("switch", "-q", "--no-track", "-C", branch, f"refs/remotes/{REMOTE}/{branch}")
    rc, out, err = g.run("merge", "--no-ff", "-m", f"{S.SYNC_PREFIX}{branch}\n\n{TRAILER}",
                         f"refs/remotes/{REMOTE}/main", check=False)
    if rc != 0:
        files = g.run("diff", "--name-only", "--diff-filter=U").split()
        say(f"conflicts in: {', '.join(files) or '(see git status)'}\nResolve keeping both sides' intent "
            "(regenerate tauri.ts / Cargo.lock / bun.lock instead of hand-merging), `git add` them, "
            "`git commit --no-edit`, then `python3 tools/cloud/fp.py push`.")
        return CONFLICT
    return cmd_push(a)


def cmd_publish_status(a):
    """Regenerate the status files on top of origin/main in a throwaway worktree and push one
    "status:" commit to main. The session's own checkout and branch are never touched."""
    g = git()
    notes_file = os.path.abspath(a.notes_file) if a.notes_file else None
    for _ in range(3):
        ok, err = g.fetch()
        if not ok:
            say(f"git fetch failed: {err}")
            return ERR
        tmp = tempfile.mkdtemp(prefix="fp-status-")
        try:
            g.run("worktree", "add", "-q", "--detach", "-f", tmp, f"refs/remotes/{REMOTE}/main")
            cmd = [sys.executable, os.path.join(tmp, "tools", "plan", "status.py"), "--no-fetch", "--write"]
            if os.environ.get("FP_OFFLINE") == "1":
                cmd.append("--offline")
            if a.record_wave is not None:
                cmd += ["--record-wave", str(a.record_wave), "--result", a.result]
                if notes_file:
                    cmd += ["--notes-file", notes_file]
            env = dict(os.environ, FP_STATUS_GIT_DIR=tmp)
            p = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", env=env)
            if p.returncode != 0:
                say(f"status.py failed:\n{p.stdout}{p.stderr}")
                return ERR
            wt = S.Git(REMOTE, cwd=tmp)
            if not wt.run("status", "--porcelain", "--", *STATUS_FILES).strip():
                say("status unchanged; nothing to publish")
                return OK
            with open(os.path.join(tmp, "docs", "clean-room", "status.json"), encoding="utf-8") as f:
                s = json.load(f)["summary"]
            subject = (f"status: wave {a.record_wave} integration {a.result}" if a.record_wave is not None else
                       f"status: {s['merged']}/{s['total']} merged, {s['in_review']} in review, "
                       f"{s['claimed']} claimed, {s['ready']} ready")
            wt.run("add", "--", *STATUS_FILES)
            p = subprocess.run(["git", "-C", tmp, "commit", "-q", "-F", "-"], input=f"{subject} [skip ci]\n\n{TRAILER}\n",
                               capture_output=True, text=True, encoding="utf-8", errors="replace")
            if p.returncode != 0:
                say(f"commit failed: {p.stderr.strip()}")
                return ERR
            if a.dry_run:
                say(f"dry run: would push {subject!r} to main")
                return OK
            rc, out, err = wt.run("push", REMOTE, "HEAD:refs/heads/main", check=False)
            if rc == 0:
                say(f"published: {subject}")
                return OK
            low = err.lower()
            if not ("[rejected]" in low or "non-fast-forward" in low or "fetch first" in low):
                say(f"push to main failed: {err.strip()}")
                return ERR
            say("main moved while publishing; regenerating")
        finally:
            g.run("worktree", "remove", "--force", tmp, check=False)
            shutil.rmtree(tmp, ignore_errors=True)
            g.run("worktree", "prune", check=False)
    return ERR


def main():
    ap = argparse.ArgumentParser(description="Handvel cloud workflow helper (see docs/dev/cloud-runbook.md)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("pick", help="claim the first available action (status.py --next order)")
    p.add_argument("--worker")
    p.add_argument("--own-idle-minutes", type=float, default=90.0)
    p.add_argument("--dry-run", action="store_true")
    p.set_defaults(fn=cmd_pick)

    p = sub.add_parser("claim", help="claim a WP (ready, own resume, stale takeover, open PR) or fix-main")
    p.add_argument("target")
    p.add_argument("--worker")
    p.add_argument("--own-idle-minutes", type=float, default=90.0)
    p.add_argument("--dry-run", action="store_true")
    p.set_defaults(fn=cmd_claim)

    p = sub.add_parser("checkout", help="check out a wp/* or fix/* branch at its remote head (no claim)")
    p.add_argument("target")
    p.set_defaults(fn=cmd_checkout)

    p = sub.add_parser("push", help="push the current wp/* or fix/* branch")
    p.set_defaults(fn=cmd_push)

    p = sub.add_parser("template", help="print a PR body skeleton")
    p.add_argument("target")
    p.add_argument("--worker")
    p.set_defaults(fn=cmd_template)

    p = sub.add_parser("open-pr", help="open the PR for a wp/* or fix/* branch")
    p.add_argument("target")
    p.add_argument("--body-file", required=True)
    p.add_argument("--title", help="fix PRs only: what was broken and how it is fixed")
    p.add_argument("--draft", action="store_true", help="open the PR as a draft")
    p.add_argument("--update-body", action="store_true", help="if the PR is already open, replace its description")
    p.set_defaults(fn=cmd_open_pr)

    p = sub.add_parser("comment", help="post a comment (for example review results) on a PR")
    p.add_argument("pr", type=int)
    p.add_argument("--body-file", required=True)
    p.set_defaults(fn=cmd_comment)

    p = sub.add_parser("ci", help="merge-gate CI state for a PR head, a commit or main")
    p.add_argument("pr", nargs="?", type=int)
    p.add_argument("--sha")
    p.add_argument("--main", action="store_true")
    p.add_argument("--wait", type=float, default=0, help="minutes to poll while pending (max 9)")
    p.add_argument("--why", action="store_true", help="on failure, list failed jobs, steps and annotations")
    p.set_defaults(fn=cmd_ci)

    p = sub.add_parser("log", help="print a CI job log")
    p.add_argument("job_id", type=int)
    p.add_argument("--tail", type=int, default=200)
    p.add_argument("--grep")
    p.set_defaults(fn=cmd_log)

    p = sub.add_parser("rerun", help="re-run failed CI jobs of a PR (or of main) once")
    p.add_argument("pr", nargs="?", type=int)
    p.add_argument("--main", action="store_true")
    p.add_argument("--force", action="store_true")
    p.set_defaults(fn=cmd_rerun)

    p = sub.add_parser("approve", help="post the fp-review approval comment for the PR head")
    p.add_argument("pr", type=int)
    p.add_argument("--verification-file")
    p.add_argument("--compliance-file")
    p.add_argument("--note", default="")
    p.add_argument("--force", action="store_true")
    p.set_defaults(fn=cmd_approve)

    p = sub.add_parser("block", help="mark a PR blocked (the merger skips it)")
    p.add_argument("pr", type=int)
    p.add_argument("--reason", required=True)
    p.set_defaults(fn=cmd_block)

    p = sub.add_parser("unblock", help="clear a block marker")
    p.add_argument("pr", type=int)
    p.add_argument("--note", default="")
    p.set_defaults(fn=cmd_unblock)

    p = sub.add_parser("merge", help="merge a PR after every gate passes (merger)")
    p.add_argument("pr", type=int)
    p.add_argument("--dry-run", action="store_true")
    p.set_defaults(fn=cmd_merge)

    p = sub.add_parser("sync-main", help="merge origin/main into a wp/* or fix/* branch (conflict resolution)")
    p.add_argument("target")
    p.set_defaults(fn=cmd_sync_main)

    p = sub.add_parser("publish-status", help="regenerate and push STATUS.md/status.json (merger)")
    p.add_argument("--record-wave", type=int)
    p.add_argument("--result", choices=["pass", "fail", "partial"])
    p.add_argument("--notes-file")
    p.add_argument("--dry-run", action="store_true")
    p.set_defaults(fn=cmd_publish_status)

    a = ap.parse_args()
    if a.cmd == "ci" and not (a.pr or a.sha or a.main):
        ap.error("ci needs a PR number, --sha or --main")
    if a.cmd == "rerun" and not (a.pr or a.main):
        ap.error("rerun needs a PR number or --main")
    if a.cmd == "publish-status" and a.record_wave is not None and not a.result:
        ap.error("--record-wave needs --result")
    try:
        return a.fn(a)
    except ghapi.GitHubError as e:
        say(f"GitHub API error: {e}")
        return ERR
    except RuntimeError as e:
        say(str(e))
        return ERR


if __name__ == "__main__":
    sys.exit(main())
