# Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
#
# Work-package status for the cloud workflow (docs/dev/cloud-runbook.md).
#
# Each WP in docs/clean-room/plan_wps.json gets exactly one status, derived from git and GitHub:
#   merged     origin/main has a merge commit (first-parent chain) whose subject starts
#              "Merge <WP-ID>:"
#   in_review  an open PR from branch wp/<WP-ID> (PR number, CI state, review state recorded)
#   claimed    remote branch wp/<WP-ID> exists with no open PR (claim owner and last commit time
#              recorded); a claim with no commit for --stale-hours (default 6) is stale and released
#   blocked    at least one depends_on WP is not merged
#   ready      every dependency merged and nobody holds the WP
#
# Git is the source of truth for merged/claimed and works offline. GitHub REST (via `gh api`, or
# urllib with GITHUB_TOKEN/GH_TOKEN, or anonymous urllib for this public repo) adds PR state.
# Review markers (approval, block) only count when posted by the repository owner, a member or a
# collaborator; anyone can comment on a public repository.
#
# Usage:
#   python3 tools/plan/status.py                 summary board on stdout (writes nothing)
#   python3 tools/plan/status.py --next          what a worker should take, in order; without
#                                                --worker this is mainly the ready WPs in plan
#                                                order (wave, then id)
#   python3 tools/plan/status.py --next --worker worker-1
#                                                fix-main first, then own resumable claims and
#                                                open PRs, then stale claims anyone may take
#                                                over, then ready WPs
#   python3 tools/plan/status.py --merge-queue   merger actions: merge, reapprove, conflict,
#                                                integrate
#   python3 tools/plan/status.py --json          full status document on stdout
#   python3 tools/plan/status.py --write         write docs/clean-room/status.json and STATUS.md
#                                                (merger only; skipped when nothing changed)
#   python3 tools/plan/status.py --record-wave 2 --result pass --notes-file notes.md
#                                                add a wave-integration record, then --write
#   --offline (no network at all), --no-fetch (skip `git fetch`), --remote NAME (default origin)
import argparse
import json
import os
import re
import subprocess
import time
import sys
from datetime import datetime, timedelta, timezone

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
import ghapi  # noqa: E402

PLAN_JSON = os.path.join(ROOT, "docs", "clean-room", "plan_wps.json")
STATUS_JSON = os.path.join(ROOT, "docs", "clean-room", "status.json")
STATUS_MD = os.path.join(ROOT, "docs", "clean-room", "STATUS.md")

WP_ID = r"W[0-9R]-\d\d"
FIX_BRANCH = r"fix/[A-Za-z0-9._-]+"
MERGE_RE = re.compile(rf"^Merge ({WP_ID}): ")
MERGE_PR_RE = re.compile(rf"^Merge pull request #(\d+) from [^/\s]+/wp/({WP_ID})\b")
CLAIM_RE = re.compile(rf"^claim: ({WP_ID}|{FIX_BRANCH}) by (\S+)")
SYNC_PREFIX = "merge: origin/main into "  # the merger's conflict-resolution commits (fp.py sync-main)
APPROVAL = "fp-review: approved verification+compliance"
BLOCKED = "fp-blocked:"
UNBLOCKED = "fp-unblocked"
HEAD_RE = re.compile(r"^head:\s*([0-9a-f]{7,40})\s*$", re.M)
STATUS_COMMIT = "status:"
TRUSTED_ASSOCIATIONS = {"OWNER", "MEMBER", "COLLABORATOR"}

# The merge gate (docs/dev/ci.md, "Merge gate"): every check run on the head commit completed with
# success/skipped/neutral, and these check runs are among them: Freenpipe CI's windows, linux and
# gates jobs, Secret Scan (job trufflehog) and the Windows CLI Integration Test (job test-windows).
REQUIRED_CHECKS = [c for c in os.environ.get(
    "FP_REQUIRED_CHECKS", "windows,linux,gates,trufflehog,test-windows").split(",") if c]
CI_PASS = {"success", "neutral", "skipped"}


def now_utc():
    return datetime.now(timezone.utc).replace(microsecond=0)


def iso(dt):
    return dt.strftime("%Y-%m-%dT%H:%M:%SZ") if dt else None


def parse_iso(s):
    if not s:
        return None
    dt = datetime.fromisoformat(s.replace("Z", "+00:00"))
    return (dt if dt.tzinfo else dt.replace(tzinfo=timezone.utc)).astimezone(timezone.utc)


def from_epoch(s):
    return datetime.fromtimestamp(int(s), timezone.utc)


def plan_key(wp):
    return (wp["wave"], wp["id"])


# ------------------------------------------------------------------------------------------ plan
def load_plan():
    if os.path.exists(PLAN_JSON):
        with open(PLAN_JSON, encoding="utf-8") as f:
            doc = json.load(f)
        if isinstance(doc, list):  # older list-only format
            wps, names = doc, {}
        else:
            wps, names = doc["wps"], {int(k): v for k, v in doc.get("wave_names", {}).items()}
    else:
        import plan_data as P  # fall back to the source data
        wps, names = P.WPS, dict(P.WAVE_NAMES)
    return sorted(wps, key=plan_key), names


# ------------------------------------------------------------------------------------------- git
class Git:
    def __init__(self, remote, cwd=None):
        self.remote = remote
        # FP_STATUS_GIT_DIR points the git side at another checkout (used by the tests).
        self.cwd = cwd or os.environ.get("FP_STATUS_GIT_DIR") or ROOT

    def run(self, *args, check=True):
        p = subprocess.run(["git", "-C", self.cwd, *args], capture_output=True, text=True, encoding="utf-8",
                           errors="replace")
        if check and p.returncode != 0:
            raise RuntimeError(f"git {' '.join(args)} failed: {p.stderr.strip()}")
        return p.stdout if check else (p.returncode, p.stdout, p.stderr)

    def ok(self, *args):
        return self.run(*args, check=False)[0] == 0

    def fetch(self):
        args = ["fetch", "--quiet", "--no-tags"]
        if self.run("rev-parse", "--is-shallow-repository").strip() == "true":
            args.append("--unshallow")  # merged detection needs main's full first-parent history
        args += [self.remote, f"+refs/heads/main:refs/remotes/{self.remote}/main",
                 f"+refs/heads/wp/*:refs/remotes/{self.remote}/wp/*",
                 f"+refs/heads/fix/*:refs/remotes/{self.remote}/fix/*"]
        for attempt in range(3):  # transient proxy or network errors are common enough to retry
            rc, _, err = self.run(*args, check=False)
            if rc == 0:
                break
            time.sleep(2 * (attempt + 1))
        return rc == 0, err.strip()

    def remote_heads(self, online):
        """({wp_id: sha}, {"fix/<name>": sha}, warning) for the remote's wp/* and fix/* branches.
        Uses `git ls-remote` online, the local remote-tracking refs offline (or if ls-remote fails)."""
        wp, fix, warn = {}, {}, None

        def take(ref, sha, prefix):
            m = re.fullmatch(rf"{prefix}wp/({WP_ID})", ref)
            if m:
                wp[m.group(1)] = sha
            elif re.fullmatch(rf"{prefix}{FIX_BRANCH}", ref):
                fix[ref[len(prefix):]] = sha

        if online:
            rc, out, err = self.run("ls-remote", "--heads", self.remote, check=False)
            if rc == 0:
                for line in out.splitlines():
                    sha, _, ref = line.partition("\t")
                    take(ref.strip(), sha, "refs/heads/")
                return wp, fix, None
            warn = f"git ls-remote failed ({err.strip()[:120]}); using local remote-tracking refs"
        prefix = f"refs/remotes/{self.remote}/"
        out = self.run("for-each-ref", "--format=%(objectname)\t%(refname)", prefix + "wp/", prefix + "fix/")
        for line in out.splitlines():
            sha, _, ref = line.partition("\t")
            take(ref.strip(), sha, prefix)
        return wp, fix, warn

    def main_ref(self):
        for ref in (f"refs/remotes/{self.remote}/main", "refs/heads/main"):
            if self.ok("rev-parse", "--verify", "--quiet", ref):
                return ref
        raise RuntimeError("neither origin/main nor main exists; fetch the repository first")

    def has_commit(self, sha):
        return self.ok("cat-file", "-e", f"{sha}^{{commit}}")

    def log(self, rev_range, first_parent=False):
        args = ["log", "--format=%H%x09%ct%x09%s"] + (["--first-parent"] if first_parent else []) + [rev_range]
        out = self.run(*args)
        rows = []
        for line in out.splitlines():
            parts = line.split("\t", 2)
            if len(parts) == 3:
                rows.append((parts[0], from_epoch(parts[1]), parts[2]))
        return rows


def merged_map(git, main_ref):
    """{wp_id: {sha, at}} from merge-commit subjects on main's first-parent chain (newest first,
    keep the first seen). Only merges into main count, never a merge inside a WP branch."""
    merged = {}
    for sha, at, subj in git.log(main_ref, first_parent=True):
        m = MERGE_RE.match(subj)
        wid = m.group(1) if m else None
        if not wid:
            m2 = MERGE_PR_RE.match(subj)
            wid = m2.group(2) if m2 else None
        if wid and wid not in merged:
            merged[wid] = {"sha": sha, "at": iso(at)}
    return merged


def branch_info(git, main_ref, wid, sha, name=None):
    info = {"name": name or f"wp/{wid}", "sha": sha, "claimed_by": None, "claimed_at": None,
            "last_commit_at": None, "head_subject": None, "commits_ahead": None, "work_commits": None}
    if not git.has_commit(sha):
        info["note"] = "branch objects not fetched"
        return info
    tip, _, subject = git.run("log", "-1", "--format=%ct%x09%s", sha).strip().partition("\t")
    info["last_commit_at"] = iso(from_epoch(tip)) if tip else None
    info["head_subject"] = subject or None
    ahead = git.log(f"{main_ref}..{sha}")
    info["commits_ahead"] = len(ahead)
    claims = [(at, CLAIM_RE.match(s)) for _, at, s in ahead if CLAIM_RE.match(s)]
    info["work_commits"] = len(ahead) - len(claims)
    if claims:  # git log is newest first
        at, m = claims[0]
        info["claimed_by"], info["claimed_at"] = m.group(2), iso(at)
    return info


# ---------------------------------------------------------------------------------------- GitHub
def gate_state(check_runs, required=None):
    """Apply the merge gate to a commit's check runs (latest run per name wins).

    Returns (state, {name: conclusion or status}, [missing required names]):
      failure  a check run completed with anything but success/skipped/neutral
      pending  a check run is still queued or running, or a required check has not appeared yet
      success  everything completed and passed, and every required check is present
      none     no check run at all yet
    """
    required = REQUIRED_CHECKS if required is None else required
    latest = {}
    for cr in check_runs:
        name = cr.get("name") or "?"
        if name not in latest or (cr.get("id") or 0) > (latest[name].get("id") or 0):
            latest[name] = cr
    checks = {n: (c.get("conclusion") or c.get("status")) for n, c in sorted(latest.items())}
    missing = [n for n in required if n not in latest]
    if not latest:
        return "none", checks, missing
    if any(c.get("status") == "completed" and c.get("conclusion") not in CI_PASS for c in latest.values()):
        return "failure", checks, missing
    if missing or any(c.get("status") != "completed" for c in latest.values()):
        return "pending", checks, missing
    return "success", checks, missing


def ci_for_sha(gh, sha):
    """CI state of one commit under the merge gate (docs/dev/ci.md).

    Returns {"state": success|failure|pending|none, "checks": {name: result}, "missing": [...],
    "runs": [...], "max_attempt": n}. "runs" lists the newest workflow run per workflow and event
    (for re-runs and job logs); "max_attempt" is the highest attempt among the failed runs (1 when
    nothing failed or nothing was re-run yet).
    """
    state, checks, missing = gate_state(gh.check_runs(sha))
    latest = {}
    for r in gh.workflow_runs(sha):  # newest run per workflow and triggering event
        key = (r.get("workflow_id") or r.get("name"), r.get("event", ""))
        if key not in latest or (r.get("id") or 0) > (latest[key].get("id") or 0):
            latest[key] = r
    chosen = sorted(latest.values(), key=lambda r: r.get("id") or 0)
    failed = [r for r in chosen if r.get("status") == "completed" and r.get("conclusion") not in CI_PASS]
    return {"state": state, "checks": checks, "missing": missing,
            "max_attempt": max([(r.get("run_attempt") or 1) for r in failed] or [1]),
            "runs": [{"id": r.get("id"), "name": r.get("name"), "event": r.get("event"), "status": r.get("status"),
                      "conclusion": r.get("conclusion"), "attempt": r.get("run_attempt"), "url": r.get("html_url")}
                     for r in chosen]}


def trusted(comment):
    """Review markers count only from the owner, members and collaborators (public repository)."""
    return (comment.get("author_association") or "").upper() in TRUSTED_ASSOCIATIONS


def review_state(comments, head_sha, head_time):
    """approved (for this head) | stale (approved an older head) | blocked | pending."""
    markers = []
    for c in comments:
        if not trusted(c):
            continue
        body = (c.get("body") or "").lstrip()
        if body.startswith(APPROVAL) or body.startswith(BLOCKED) or body.startswith(UNBLOCKED):
            markers.append((c.get("created_at") or "", body, c.get("html_url")))
    markers.sort(key=lambda m: m[0])  # ISO timestamps sort chronologically
    state, url, reason = "pending", None, None
    for created, body, link in markers:
        if body.startswith(BLOCKED):
            rest = body[len(BLOCKED):].strip()
            state, url, reason = "blocked", link, (rest.splitlines()[0][:200] if rest else "")
        elif body.startswith(UNBLOCKED):
            state, url, reason = "pending", None, None
        else:
            m = HEAD_RE.search(body)
            if m:
                ok = head_sha.startswith(m.group(1))
            else:  # an approval without a head line only counts if posted after the head commit
                posted = parse_iso(created) if created else None
                ok = bool(posted and head_time and posted >= head_time)
            state, url, reason = ("approved" if ok else "stale"), link, None
    return {"state": state, "comment": url, "reason": reason}


def pr_info(gh, pr):
    num = pr["number"]
    head_sha = pr["head"]["sha"]
    info = {"number": num, "url": pr.get("html_url"), "head_sha": head_sha, "title": pr.get("title"),
            "created_at": pr.get("created_at"), "updated_at": pr.get("updated_at"),
            "ci": {"state": "unknown", "checks": {}, "runs": []}, "review": {"state": "unknown"},
            "mergeable": None, "mergeable_state": None}
    try:
        info["ci"] = ci_for_sha(gh, head_sha)
    except ghapi.GitHubError as e:
        info["ci"]["error"] = str(e)
    try:
        full = gh.pull(num)
        info["mergeable"], info["mergeable_state"] = full.get("mergeable"), full.get("mergeable_state")
    except ghapi.GitHubError as e:
        info["mergeable_error"] = str(e)
    info["_comments"] = None
    try:
        info["_comments"] = gh.issue_comments(num)
    except ghapi.GitHubError as e:
        info["review"]["error"] = str(e)
    return info


# ------------------------------------------------------------------------------------- compute
def compute(args):
    wps, wave_names = load_plan()
    ids = {w["id"] for w in wps}
    git = Git(args.remote)
    warnings = []
    online = not args.offline

    if online and not args.no_fetch:
        ok, err = git.fetch()
        if not ok:
            warnings.append(f"git fetch failed ({err[:160]}); using refs already present")
    main_ref = git.main_ref()
    main_sha = git.run("rev-parse", main_ref).strip()
    merged = merged_map(git, main_ref)
    heads, fix_heads, warn = git.remote_heads(online)
    if warn:
        warnings.append(warn)

    # Latest main commit that is not a status commit (status commits carry [skip ci]).
    main_code_sha = main_sha
    for sha, _, subj in git.log(main_ref, first_parent=True)[:50]:
        if not subj.startswith(STATUS_COMMIT):
            main_code_sha = sha
            break

    gh, prs, fix_prs, github = None, {}, {}, {"available": False, "transport": None}
    main_ci = {"state": "unknown", "checks": {}, "runs": []}
    if online:
        gh = ghapi.GitHub()
        try:
            for pr in gh.open_pulls():
                ref = pr.get("head", {}).get("ref", "")
                same_repo = (pr.get("head", {}).get("repo") or {}).get("full_name") == gh.repo
                if not same_repo or (pr.get("base") or {}).get("ref") != "main":
                    continue
                m = re.fullmatch(rf"wp/({WP_ID})", ref)
                if m and m.group(1) in ids:
                    prs[m.group(1)] = pr
                elif re.fullmatch(FIX_BRANCH, ref):
                    fix_prs[ref] = pr
            github = {"available": True, "transport": gh.transport}
        except ghapi.GitHubError as e:
            warnings.append(f"GitHub API unavailable ({e}); open PRs are shown as claims")
            gh = None
        if gh:
            try:
                main_ci = ci_for_sha(gh, main_code_sha)
            except ghapi.GitHubError as e:
                main_ci["error"] = str(e)

    def with_review(info, branch):
        comments = info.pop("_comments")
        head_time = parse_iso(branch["last_commit_at"]) if branch and branch.get("last_commit_at") else None
        if comments is not None:
            info["review"] = review_state(comments, info["head_sha"], head_time)
        return info

    now = now_utc()
    stale_after = timedelta(hours=args.stale_hours)
    rows = []
    for w in wps:
        wid = w["id"]
        row = {"id": wid, "wave": w["wave"], "size": w["size"], "title": w["title"],
               "depends_on": list(w["depends_on"])}
        unmet = [d for d in w["depends_on"] if d not in merged]
        if unmet:
            row["unmet_deps"] = unmet
        if wid in merged:
            row["status"] = "merged"
            row["merge_commit"] = merged[wid]["sha"]
            row["merged_at"] = merged[wid]["at"]
        else:
            branch = branch_info(git, main_ref, wid, heads[wid]) if wid in heads else None
            if branch:
                row["branch"] = branch
            if wid in prs and gh:
                row["status"] = "in_review"
                row["pr"] = with_review(pr_info(gh, prs[wid]), branch)
            elif branch:
                row["status"] = "claimed"
                last = parse_iso(branch["last_commit_at"]) if branch.get("last_commit_at") else None
                row["stale"] = bool(last and now - last >= stale_after)
            elif unmet:
                row["status"] = "blocked"
            else:
                row["status"] = "ready"
        rows.append(row)

    summary = {k: sum(1 for r in rows if r["status"] == k)
               for k in ("merged", "in_review", "claimed", "ready", "blocked")}
    summary["total"] = len(rows)
    summary["percent_merged"] = round(100.0 * summary["merged"] / max(1, len(rows)), 1)
    waves = []
    for k in sorted({r["wave"] for r in rows}):
        wr = [r for r in rows if r["wave"] == k]
        entry = {"wave": k, "name": wave_names.get(k, ""), "total": len(wr)}
        for s in ("merged", "in_review", "claimed", "ready", "blocked"):
            entry[s] = sum(1 for r in wr if r["status"] == s)
        entry["complete"] = entry["merged"] == entry["total"]
        waves.append(entry)

    # fix/* branches repair a red main (see docs/dev/cloud-runbook.md, "Main is red").
    fixes = []
    for name in sorted(set(fix_heads) | set(fix_prs)):
        sha = fix_heads.get(name) or fix_prs[name]["head"]["sha"]
        branch = branch_info(git, main_ref, None, sha, name=name)
        fx = {"id": name, "wave": "-", "size": "-", "title": name, "depends_on": [], "branch": branch,
              "status": "claimed"}
        if name in fix_prs and gh:
            fx["status"] = "in_review"
            fx["pr"] = with_review(pr_info(gh, fix_prs[name]), branch)
            fx["title"] = fx["pr"].get("title") or name
        elif git.has_commit(sha) and git.ok("merge-base", "--is-ancestor", sha, main_ref):
            fx["status"] = "merged"
        fixes.append(fx)

    return {
        "generated_at": iso(now),
        "generator": "python3 tools/plan/status.py --write",
        "main": {"ref": main_ref.replace("refs/remotes/", ""), "sha": main_sha, "code_sha": main_code_sha,
                 "ci": main_ci},
        "github": github,
        "options": {"stale_hours": args.stale_hours, "required_checks": REQUIRED_CHECKS},
        "summary": summary,
        "waves": waves,
        "stale_claims": [r["id"] for r in rows if r["status"] == "claimed" and r.get("stale")],
        "fixes": [f for f in fixes if f["status"] != "merged"],
        "wave_integration": {},
        "warnings": warnings,
        "wps": rows,
    }


# ------------------------------------------------------------------------------- action lists
def last_activity(row):
    times = []
    b = row.get("branch") or {}
    if b.get("last_commit_at"):
        times.append(parse_iso(b["last_commit_at"]))
    return max(times) if times else None


def synced_by_merger(r):
    """True when the PR head is the merger's conflict-resolution merge (fp.py sync-main)."""
    return ((r.get("branch") or {}).get("head_subject") or "").startswith(SYNC_PREFIX)


def pr_needs_work(r, idle, stale_after):
    """An open PR a worker should pick up: red CI, green CI without a valid approval, or no CI run
    at all for a long time. Blocked PRs, conflicts and the merger's own sync heads (the merger
    re-approves those) are left alone unless the PR has been idle for the stale period."""
    pr = r["pr"]
    ci, review = pr["ci"]["state"], pr["review"].get("state")
    if review == "blocked":
        return False
    if synced_by_merger(r) and review == "stale" and ci == "success" and (idle is None or idle < stale_after):
        return False  # the merger re-approves its own green sync (merge queue: reapprove)
    if ci == "failure" or (ci == "success" and review in ("pending", "stale")):
        return True
    return ci == "none" and idle is not None and idle >= stale_after


def main_needs_fix(doc):
    """Main's latest code commit is red after one re-run, and no fix/* branch is open for it."""
    ci = doc["main"]["ci"]
    if ci.get("state") != "failure" or (ci.get("max_attempt") or 1) < 2:
        return None
    if any(f["id"].startswith("fix/main-") for f in doc.get("fixes", [])):
        return None  # a fix for main is already claimed or in review
    sha7 = doc["main"]["code_sha"][:7]
    return {"id": f"fix/main-{sha7}", "wave": "-", "size": "-",
            "title": f"CI is red on main {sha7}: find the cause and fix it"}


def next_actions(doc, worker=None, own_idle_minutes=90):
    """[(kind, row)] in the order a worker should consider them:
    fix-main     main is red after a re-run and nobody is fixing it (branch fix/main-<sha7>)
    resume       own claim (no PR) idle >= own_idle_minutes: an earlier session of this worker died
    continue-pr  open PR needing work (red CI, or green CI awaiting reviews/approval): own and idle
                 >= own_idle_minutes, or anyone's idle >= the stale period
    takeover     someone else's stale claim (no PR, no commit for the stale period)
    ready        unclaimed WPs with every dependency merged, in plan order
    """
    now = parse_iso(doc["generated_at"])
    stale_after = timedelta(hours=doc.get("options", {}).get("stale_hours", 6.0))
    own_after = timedelta(minutes=own_idle_minutes)
    rows = doc["wps"]
    out = []
    fix = main_needs_fix(doc)
    if fix:
        out.append(("fix-main", fix))
    for r in rows:
        if r["status"] not in ("claimed", "in_review"):
            continue
        owner = (r.get("branch") or {}).get("claimed_by")
        last = last_activity(r)
        idle = (now - last) if last else None
        own = worker is not None and owner == worker
        if r["status"] == "claimed" and own and idle is not None and idle >= own_after:
            out.append(("resume", r))
        elif r["status"] == "in_review" and idle is not None and (
                (own and idle >= own_after) or idle >= stale_after) and pr_needs_work(r, idle, stale_after):
            out.append(("continue-pr", r))
    for r in rows:
        if r["status"] == "claimed" and r.get("stale") and not (
                worker is not None and (r.get("branch") or {}).get("claimed_by") == worker):
            out.append(("takeover", r))
    for r in rows:
        if r["status"] == "ready":
            out.append(("ready", r))
    return out


def merge_queue(doc):
    """[(kind, row)] for the merger, in plan order:
    merge      CI green, approved for the current head, every dependency merged
    reapprove  the merger's own sync-main head is green but no longer approved: review the
               conflict resolution and approve again
    conflict   GitHub reports a merge conflict with main: run fp.py sync-main
    integrate  every WP of a wave is merged and its integration checklist is not recorded yet
    fix/* PRs come first, because they repair main.
    """
    merged = {r["id"] for r in doc["wps"] if r["status"] == "merged"}
    out = []
    candidates = [f for f in doc.get("fixes", []) if f["status"] == "in_review"]
    candidates += [r for r in doc["wps"] if r["status"] == "in_review"]
    for r in candidates:
        pr = r["pr"]
        deps_ok = all(d in merged for d in r.get("depends_on", []))
        review = pr["review"].get("state")
        if review == "blocked":
            continue
        if pr.get("mergeable") is False or pr.get("mergeable_state") == "dirty":
            out.append(("conflict", r))
        elif pr["ci"]["state"] == "success" and review == "approved" and deps_ok:
            out.append(("merge", r))
        elif pr["ci"]["state"] == "success" and review == "stale" and synced_by_merger(r):
            out.append(("reapprove", r))
    for wv in doc["waves"]:
        if wv["complete"] and str(wv["wave"]) not in doc.get("wave_integration", {}):
            out.append(("integrate", {"id": f"wave-{wv['wave']}", "wave": wv["wave"], "title": wv["name"]}))
    return out


# ------------------------------------------------------------------------------------ rendering
def short(sha):
    return sha[:7] if sha else ""


def bar(done, total, width=10):
    n = int(round(width * done / max(1, total)))
    return "`" + "#" * n + "-" * (width - n) + "`"


def detail(r):
    s = r["status"]
    if s == "merged":
        return f"merge `{short(r.get('merge_commit'))}`"
    if s == "in_review":
        pr = r["pr"]
        return f"PR #{pr['number']}: CI {pr['ci']['state']}, review {pr['review'].get('state')}"
    if s == "claimed":
        b = r.get("branch") or {}
        who = b.get("claimed_by") or "unknown"
        when = b.get("last_commit_at") or "unknown time"
        return f"{who}, last commit {when}" + (" (stale: released)" if r.get("stale") else "")
    if s == "blocked":
        unmet = r.get("unmet_deps", [])
        more = f" and {len(unmet) - 6} more" if len(unmet) > 6 else ""
        return "waits for " + ", ".join(unmet[:6]) + more
    return ""


def ci_note(ci):
    """" (<checks that are not passing>; waiting for <missing required checks>)" or ""."""
    bad = [f"{k}: {v}" for k, v in (ci.get("checks") or {}).items() if v not in CI_PASS]
    if ci.get("missing") and ci.get("state") != "none":
        bad.append("waiting for " + ", ".join(ci["missing"]))
    return f" ({'; '.join(bad)})" if bad else ""


def render_md(doc):
    s = doc["summary"]
    gh = doc["github"]
    L = ["# Freenpipe work-package status", "",
         "> Generated file: the merger routine regenerates it with `python3 tools/plan/status.py --write`. "
         "Do not edit it by hand. The plan is [PLAN.md](PLAN.md); the process is "
         "[docs/dev/cloud-runbook.md](../dev/cloud-runbook.md).", "",
         f"Generated {doc['generated_at']} from `{doc['main']['ref']}` (latest code commit "
         f"`{short(doc['main']['code_sha'])}`). "
         + (f"GitHub data: available (via {gh['transport']})." if gh.get("available")
            else "GitHub data: unavailable, so open PRs appear as claims."), ""]
    if doc.get("warnings"):
        L += ["Warnings:"] + [f"- {w}" for w in doc["warnings"]] + [""]
    L += ["## Progress", "",
          f"**{s['merged']} of {s['total']} work packages merged ({s['percent_merged']}%)**: "
          f"{s['in_review']} in review, {s['claimed']} claimed, {s['ready']} ready, {s['blocked']} blocked.", "",
          "| Wave | Theme | Merged | In review | Claimed | Ready | Blocked | Progress |",
          "|---|---|---|---|---|---|---|---|"]
    for wv in doc["waves"]:
        pct = round(100 * wv["merged"] / max(1, wv["total"]))
        L.append(f"| {wv['wave']} | {wv['name']} | {wv['merged']}/{wv['total']} | {wv['in_review']} | {wv['claimed']} | "
                 f"{wv['ready']} | {wv['blocked']} | {bar(wv['merged'], wv['total'])} {pct}% |")
    L.append(f"| all | | {s['merged']}/{s['total']} | {s['in_review']} | {s['claimed']} | {s['ready']} | {s['blocked']} | "
             f"{bar(s['merged'], s['total'])} {s['percent_merged']}% |")
    mc = doc["main"]["ci"]
    L += ["", f"CI on main (merge gate on the latest non-status commit `{short(doc['main']['code_sha'])}`): "
          f"**{mc['state']}**" + ci_note(mc), ""]
    if doc.get("fixes"):
        L += ["Fixes for main in progress:", ""]
        for f in doc["fixes"]:
            b = f.get("branch") or {}
            L.append(f"- `{f['id']}`: {detail(f)}" + (f" ({b.get('claimed_by')})" if b.get("claimed_by") else ""))
        L.append("")

    rev = [r for r in doc["wps"] if r["status"] == "in_review"]
    L += ["## In review", ""]
    if rev:
        L += ["| WP | PR | Head | CI | Review | Mergeable | Worker |", "|---|---|---|---|---|---|---|"]
        for r in rev:
            pr = r["pr"]
            checks = ci_note(pr["ci"]).strip(" ()")
            mg = {True: "yes", False: "conflict", None: "unknown"}[pr.get("mergeable")]
            L.append(f"| {r['id']} | [#{pr['number']}]({pr['url']}) | `{short(pr['head_sha'])}` | {pr['ci']['state']}"
                     + (f" ({checks})" if checks else "") + f" | {pr['review'].get('state')} | {mg} | "
                     f"{(r.get('branch') or {}).get('claimed_by') or '-'} |")
    else:
        L.append("None.")
    L.append("")

    cl = [r for r in doc["wps"] if r["status"] == "claimed"]
    L += ["## Claimed", ""]
    if cl:
        L += ["| WP | Worker | Last commit (UTC) | Work commits | State |", "|---|---|---|---|---|"]
        for r in cl:
            b = r.get("branch") or {}
            state = "stale: released, any worker may take it over" if r.get("stale") else "active"
            L.append(f"| {r['id']} | {b.get('claimed_by') or 'unknown'} | {b.get('last_commit_at') or 'unknown'} | "
                     f"{b.get('work_commits') if b.get('work_commits') is not None else '?'} | {state} |")
    else:
        L.append("None.")
    L.append("")

    ready = [r for r in doc["wps"] if r["status"] == "ready"]
    L += ["## Ready, in plan order", ""]
    if ready:
        L += ["| WP | Wave | Size | Title |", "|---|---|---|---|"]
        L += [f"| {r['id']} | {r['wave']} | {r['size']} | {r['title']} |" for r in ready]
    else:
        L.append("None.")
    L.append("")

    L += ["## Wave integration log", ""]
    wi = doc.get("wave_integration") or {}
    if wi:
        for k in sorted(wi, key=lambda x: int(x)):
            rec = wi[k]
            L += [f"### Wave {k}: {rec.get('result')} (recorded {rec.get('recorded_at')}, main `{short(rec.get('main_sha'))}`)", ""]
            if rec.get("notes"):
                L += [rec["notes"].rstrip(), ""]
    else:
        L += ["No wave integration has been recorded by the merger yet.", ""]
    pending = [wv["wave"] for wv in doc["waves"] if wv["complete"] and str(wv["wave"]) not in wi]
    if pending:
        L += ["Complete waves awaiting the integration checklist: " + ", ".join(str(x) for x in pending) + ".", ""]

    L += ["## Board", ""]
    for wv in doc["waves"]:
        L += [f"### Wave {wv['wave']}: {wv['name']} ({wv['merged']}/{wv['total']} merged)", "",
              "| WP | Size | Status | Title | Detail |", "|---|---|---|---|---|"]
        for r in doc["wps"]:
            if r["wave"] == wv["wave"]:
                L.append(f"| {r['id']} | {r['size']} | {r['status']} | {r['title']} | {detail(r)} |")
        L.append("")
    return "\n".join(L).rstrip() + "\n"


def comparable(doc):
    """The document minus what changes on every publish: the timestamp and main's tip (each status
    commit moves the tip). Without this, every merger run would publish another status commit."""
    d = dict(doc)
    d.pop("generated_at", None)
    d["main"] = {k: v for k, v in (d.get("main") or {}).items() if k != "sha"}
    return json.dumps(d, sort_keys=True)


def write_files(doc):
    """Write status.json and STATUS.md unless only the timestamp would change. Return True if written."""
    if os.path.exists(STATUS_JSON):
        try:
            with open(STATUS_JSON, encoding="utf-8") as f:
                old = json.load(f)
            if comparable(old) == comparable(doc) and os.path.exists(STATUS_MD):
                return False
        except ValueError:
            pass
    with open(STATUS_JSON, "w", encoding="utf-8", newline="") as f:
        f.write(json.dumps(doc, indent=1, ensure_ascii=True) + "\n")
    with open(STATUS_MD, "w", encoding="utf-8", newline="") as f:
        f.write(render_md(doc))
    return True


def load_recorded_waves():
    if not os.path.exists(STATUS_JSON):
        return {}
    try:
        with open(STATUS_JSON, encoding="utf-8") as f:
            return json.load(f).get("wave_integration", {}) or {}
    except (ValueError, OSError):
        return {}


def print_summary(doc):
    s = doc["summary"]
    print(f"main {doc['main']['ref']} {short(doc['main']['sha'])}  github={doc['github'].get('transport') or 'unavailable'}"
          f"  main-ci={doc['main']['ci']['state']}")
    print(f"merged {s['merged']}/{s['total']} ({s['percent_merged']}%)  in_review {s['in_review']}  claimed {s['claimed']}"
          f"  ready {s['ready']}  blocked {s['blocked']}")
    for wv in doc["waves"]:
        print(f"  wave {wv['wave']}: {wv['merged']:>3}/{wv['total']:<3} merged  {wv['in_review']} review  "
              f"{wv['claimed']} claimed  {wv['ready']} ready  {wv['blocked']} blocked")
    for r in doc.get("fixes", []) + doc["wps"]:
        if r["status"] in ("in_review", "claimed"):
            print(f"  {r['status']:<9} {r['id']}  {detail(r)}")
    ready = [r["id"] for r in doc["wps"] if r["status"] == "ready"]
    print("  ready: " + (", ".join(ready[:20]) + (f" (+{len(ready) - 20})" if len(ready) > 20 else "") if ready else "none"))
    for w in doc["warnings"]:
        print("  warning: " + w)


def main():
    ap = argparse.ArgumentParser(description="Derive Freenpipe work-package status from git and GitHub.")
    ap.add_argument("--remote", default="origin")
    ap.add_argument("--offline", action="store_true", help="no network: local refs only, no GitHub API")
    ap.add_argument("--no-fetch", action="store_true", help="skip `git fetch` (ls-remote and the API still run)")
    ap.add_argument("--stale-hours", type=float, default=6.0)
    ap.add_argument("--own-idle-minutes", type=float, default=90.0,
                    help="with --worker: own claims idle this long are offered for resume")
    ap.add_argument("--worker", default=os.environ.get("FP_WORKER_ID"))
    mode = ap.add_mutually_exclusive_group()
    mode.add_argument("--next", action="store_true", help="print what a worker should pick, in order")
    mode.add_argument("--merge-queue", action="store_true", help="print merger actions")
    mode.add_argument("--json", action="store_true", help="print the status document")
    ap.add_argument("--write", action="store_true", help="write docs/clean-room/status.json and STATUS.md")
    ap.add_argument("--record-wave", type=int, help="record the integration result of a complete wave")
    ap.add_argument("--result", choices=["pass", "fail", "partial"])
    ap.add_argument("--notes", default="")
    ap.add_argument("--notes-file")
    a = ap.parse_args()
    if a.record_wave is not None and not a.result:
        ap.error("--record-wave needs --result")

    doc = compute(a)
    doc["wave_integration"] = load_recorded_waves()
    if a.record_wave is not None:
        notes = a.notes
        if a.notes_file:
            with open(a.notes_file, encoding="utf-8") as f:
                notes = f.read()
        doc["wave_integration"][str(a.record_wave)] = {
            "result": a.result, "recorded_at": doc["generated_at"], "main_sha": doc["main"]["code_sha"],
            "notes": notes.strip()}
        a.write = True

    if a.next:
        for kind, r in next_actions(doc, a.worker, a.own_idle_minutes):
            print(f"{kind}\t{r['id']}\t{r['wave']}\t{r['size']}\t{r['title']}")
    elif a.merge_queue:
        for kind, r in merge_queue(doc):
            if kind == "integrate":
                print(f"integrate\t{r['id']}\t{r['wave']}\t-\t{r['title']}")
            else:
                print(f"{kind}\t{r['id']}\t#{r['pr']['number']}\t{r['pr']['head_sha']}\t{r['title']}")
    elif a.json:
        print(json.dumps(doc, indent=1, ensure_ascii=True))
    elif not a.write:
        print_summary(doc)

    if a.write:
        changed = write_files(doc)
        print("status files " + ("updated" if changed else "unchanged") + ": docs/clean-room/status.json, docs/clean-room/STATUS.md",
              file=sys.stderr if (a.next or a.merge_queue or a.json) else sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
