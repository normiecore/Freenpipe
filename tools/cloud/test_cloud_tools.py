# Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
#
# Self-test for tools/plan/status.py and tools/cloud/fp.py. Offline: a throwaway bare repository
# stands in for GitHub, so nothing touches the network or the real remote.
#
#   python3 tools/cloud/test_cloud_tools.py        (or: python3 -m unittest discover -s tools/cloud)
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timedelta, timezone

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
FP = os.path.join(HERE, "fp.py")
STATUS = os.path.join(ROOT, "tools", "plan", "status.py")
sys.path.insert(0, os.path.join(ROOT, "tools", "plan"))
import status as S  # noqa: E402

GIT_ENV = {"GIT_AUTHOR_NAME": "fp-test", "GIT_AUTHOR_EMAIL": "fp-test@example.invalid",
           "GIT_COMMITTER_NAME": "fp-test", "GIT_COMMITTER_EMAIL": "fp-test@example.invalid",
           "GIT_CONFIG_NOSYSTEM": "1"}


TRANSIENT = ("Could not read from remote repository", "fork: retry", "Resource temporarily unavailable")


def run(args, cwd=None, env=None, check=True):
    e = dict(os.environ, **GIT_ENV, **(env or {}))
    for _ in range(3):  # Git for Windows sometimes fails to spawn its helpers; retry those runs
        p = subprocess.run(args, cwd=cwd, env=e, capture_output=True, text=True, encoding="utf-8", errors="replace")
        if p.returncode == 0 or not any(t in p.stdout + p.stderr for t in TRANSIENT):
            break
    if check and p.returncode != 0:
        raise AssertionError(f"{args} failed ({p.returncode}):\n{p.stdout}\n{p.stderr}")
    return p


def git(cwd, *args, env=None, check=True):
    return run(["git", *args], cwd=cwd, env=env, check=check)


class Sandbox:
    """A bare 'origin' plus clones; main carries the plan files and a merge of W0-01."""

    def __init__(self):
        self.dir = tempfile.mkdtemp(prefix="fp-selftest-")
        self.origin = os.path.join(self.dir, "origin.git")
        git(self.dir, "init", "-q", "--bare", "-b", "main", self.origin)
        seed = os.path.join(self.dir, "seed")
        git(self.dir, "clone", "-q", self.origin, seed)
        git(seed, "switch", "-q", "-c", "main")
        for rel in ("tools/plan/status.py", "tools/plan/ghapi.py", "docs/clean-room/plan_wps.json"):
            os.makedirs(os.path.join(seed, os.path.dirname(rel)), exist_ok=True)
            shutil.copy(os.path.join(ROOT, rel), os.path.join(seed, rel))
        git(seed, "add", "-f", ".")
        git(seed, "commit", "-q", "-m", "seed")
        git(seed, "commit", "-q", "--allow-empty", "-m", "Merge W0-01: Build bootstrap")
        git(seed, "push", "-q", "origin", "main")

    def clone(self, name):
        path = os.path.join(self.dir, name)
        git(self.dir, "clone", "-q", self.origin, path)
        return path

    def fp(self, clone, *args, check=False):
        return run([sys.executable, FP, *args], cwd=clone, check=check,
                   env={"FP_OFFLINE": "1", "FP_STATUS_GIT_DIR": clone})

    def status(self, clone, *args):
        p = run([sys.executable, STATUS, "--offline", *args], cwd=clone, env={"FP_STATUS_GIT_DIR": clone})
        return p.stdout

    def remote_branches(self):
        out = git(self.origin, "for-each-ref", "--format=%(refname:short)", "refs/heads/").stdout
        return set(out.split())

    def close(self):
        shutil.rmtree(self.dir, ignore_errors=True)


class ClaimFlow(unittest.TestCase):
    def setUp(self):
        self.sb = Sandbox()
        self.a = self.sb.clone("a")
        self.b = self.sb.clone("b")

    def tearDown(self):
        self.sb.close()

    def test_claim_then_second_worker_is_refused(self):
        p = self.sb.fp(self.a, "claim", "W0-02", "--worker", "worker-1")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertIn("wp/W0-02", self.sb.remote_branches())
        subj = git(self.sb.origin, "log", "-1", "--format=%s", "wp/W0-02").stdout.strip()
        self.assertEqual(subj, "claim: W0-02 by worker-1")
        body = git(self.sb.origin, "log", "-1", "--format=%B", "wp/W0-02").stdout
        self.assertIn("Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>", body)

        p = self.sb.fp(self.b, "claim", "W0-02", "--worker", "worker-2")
        self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
        out = self.sb.status(self.b, "--json")
        row = next(r for r in json.loads(out)["wps"] if r["id"] == "W0-02")
        self.assertEqual(row["status"], "claimed")
        self.assertEqual(row["branch"]["claimed_by"], "worker-1")

    def test_push_race_is_detected(self):
        # Clone b prepares a claim from a stale view, then clone a claims first.
        git(self.b, "switch", "-q", "-c", "wp/W0-03", "origin/main")
        git(self.b, "commit", "-q", "--allow-empty", "-m", "claim: W0-03 by worker-2")
        self.assertEqual(self.sb.fp(self.a, "claim", "W0-03", "--worker", "worker-1").returncode, 0)
        os.environ["FP_STATUS_GIT_DIR"] = self.b
        try:
            sys.path.insert(0, HERE)
            import fp
            rc, kind, _ = fp.push_branch(fp.git(), "wp/W0-03")
        finally:
            os.environ.pop("FP_STATUS_GIT_DIR", None)
        self.assertEqual(kind, "rejected")

    def test_pick_skips_claimed_and_takes_next_ready(self):
        self.assertEqual(self.sb.fp(self.a, "claim", "W0-02", "--worker", "worker-1").returncode, 0)
        p = self.sb.fp(self.b, "pick", "--worker", "worker-2")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertIn("picked\tready\tW0-03", p.stdout)
        self.assertEqual(git(self.b, "rev-parse", "--abbrev-ref", "HEAD").stdout.strip(), "wp/W0-03")

    def test_stale_claim_takeover_and_own_resume(self):
        old = (datetime.now(timezone.utc) - timedelta(hours=7)).strftime("%Y-%m-%dT%H:%M:%SZ")
        git(self.a, "switch", "-q", "-c", "wp/W0-04", "origin/main")
        git(self.a, "commit", "-q", "--allow-empty", "-m", "claim: W0-04 by worker-1",
            env={"GIT_COMMITTER_DATE": old, "GIT_AUTHOR_DATE": old})
        git(self.a, "push", "-q", "origin", "HEAD:refs/heads/wp/W0-04")
        git(self.b, "fetch", "-q", "origin", "+refs/heads/wp/*:refs/remotes/origin/wp/*")  # --offline reads local refs
        nxt = self.sb.status(self.b, "--next", "--worker", "worker-2")
        self.assertIn("takeover\tW0-04", nxt)
        nxt_own = self.sb.status(self.b, "--next", "--worker", "worker-1")
        self.assertIn("resume\tW0-04", nxt_own)
        p = self.sb.fp(self.b, "claim", "W0-04", "--worker", "worker-2")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        subj = git(self.sb.origin, "log", "-1", "--format=%s", "wp/W0-04").stdout.strip()
        self.assertEqual(subj, "claim: W0-04 by worker-2 (takeover from worker-1)")

    def test_merged_wp_reports_merged(self):
        p = self.sb.fp(self.a, "claim", "W0-01", "--worker", "worker-1")
        self.assertEqual(p.returncode, 3, p.stdout + p.stderr)

    def test_blocked_wp_is_not_available(self):
        p = self.sb.fp(self.a, "claim", "W1-01", "--worker", "worker-1")
        self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
        self.assertNotIn("wp/W1-01", self.sb.remote_branches())

    def test_publish_status_pushes_a_status_commit(self):
        self.assertEqual(self.sb.fp(self.a, "claim", "W0-02", "--worker", "worker-1").returncode, 0)
        p = self.sb.fp(self.b, "publish-status")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        subj = git(self.sb.origin, "log", "-1", "--format=%s", "main").stdout.strip()
        with open(os.path.join(ROOT, "docs/clean-room/plan_wps.json")) as f:
            total = len(json.load(f)["wps"])
        self.assertTrue(subj.startswith(f"status: 1/{total} merged, 0 in review, 1 claimed"), subj)
        self.assertTrue(subj.endswith("[skip ci]"), subj)
        md = git(self.sb.origin, "show", "main:docs/clean-room/STATUS.md").stdout
        self.assertIn("| W0-02 | worker-1 |", md)
        # Nothing changed: the second publish is a no-op.
        p = self.sb.fp(self.b, "publish-status")
        self.assertIn("nothing to publish", p.stdout)
        # Status commits never count as merges.
        out = json.loads(self.sb.status(self.b, "--json", "--no-fetch"))
        self.assertEqual(out["summary"]["merged"], 1)


def comment(body, assoc="OWNER", at="2026-10-15T10:00:00Z"):
    return {"body": body, "author_association": assoc, "created_at": at, "html_url": "u"}


class ReviewMarkers(unittest.TestCase):
    head = "a" * 40
    head_time = datetime(2026, 10, 15, 9, 0, tzinfo=timezone.utc)

    def test_approval_for_head(self):
        c = [comment(f"{S.APPROVAL}\nhead: {self.head}\n\nok")]
        self.assertEqual(S.review_state(c, self.head, self.head_time)["state"], "approved")

    def test_approval_for_older_head_is_stale(self):
        c = [comment(f"{S.APPROVAL}\nhead: {'b' * 40}\n")]
        self.assertEqual(S.review_state(c, self.head, self.head_time)["state"], "stale")

    def test_untrusted_commenter_is_ignored(self):
        c = [comment(f"{S.APPROVAL}\nhead: {self.head}\n", assoc="NONE")]
        self.assertEqual(S.review_state(c, self.head, self.head_time)["state"], "pending")

    def test_block_then_unblock(self):
        c = [comment(f"{S.APPROVAL}\nhead: {self.head}\n", at="2026-10-15T10:00:00Z"),
             comment(f"{S.BLOCKED} spec gap", at="2026-10-15T11:00:00Z")]
        st = S.review_state(c, self.head, self.head_time)
        self.assertEqual((st["state"], st["reason"]), ("blocked", "spec gap"))
        c.append(comment(S.UNBLOCKED, at="2026-10-15T12:00:00Z"))
        self.assertEqual(S.review_state(c, self.head, self.head_time)["state"], "pending")


def check(name, conclusion="success", status="completed", cid=1):
    return {"name": name, "status": status, "conclusion": conclusion if status == "completed" else None, "id": cid}


class MergeGate(unittest.TestCase):
    req = ["windows", "linux", "gates", "trufflehog", "test-windows"]

    def test_all_required_green(self):
        runs = [check(n) for n in self.req] + [check("eval", "skipped")]
        self.assertEqual(S.gate_state(runs, self.req)[0], "success")

    def test_missing_required_is_pending(self):
        state, _, missing = S.gate_state([check(n) for n in self.req[:-1]], self.req)
        self.assertEqual((state, missing), ("pending", ["test-windows"]))

    def test_any_failure_fails_even_while_others_run(self):
        runs = [check("windows", status="in_progress"), check("gates", "failure")]
        self.assertEqual(S.gate_state(runs, self.req)[0], "failure")

    def test_latest_run_per_name_wins(self):
        runs = [check(n, cid=5) for n in self.req] + [check("linux", "failure", cid=2)]
        self.assertEqual(S.gate_state(runs, self.req)[0], "success")

    def test_nothing_yet(self):
        self.assertEqual(S.gate_state([], self.req)[0], "none")


class PrBody(unittest.TestCase):
    def test_review_section(self):
        sys.path.insert(0, HERE)
        import fp
        body = "## W2-01\n\n### Review\n\n- Verification reviewer (head `abc1234`): APPROVE\n\n### Spec decisions\n\nnone\n"
        self.assertIn("abc1234", fp.review_section(body))
        self.assertIsNone(fp.review_section("### Review\n\n- Verification reviewer (head `<sha>`): <verdict and summary>\n"))


def doc_with(rows, main_ci=None, fixes=None, now="2026-10-15T12:00:00Z"):
    return {"generated_at": now, "options": {"stale_hours": 6.0},
            "main": {"code_sha": "c" * 40, "ci": main_ci or {"state": "success"}},
            "fixes": fixes or [], "waves": [], "wave_integration": {}, "wps": rows}


def pr_row(wid, ci, review, last, worker="worker-1", subject="wip", mergeable=True, deps=()):
    return {"id": wid, "wave": 2, "size": "L", "title": wid, "depends_on": list(deps), "status": "in_review",
            "branch": {"claimed_by": worker, "last_commit_at": last, "head_subject": subject},
            "pr": {"number": 7, "head_sha": "d" * 40, "ci": {"state": ci}, "review": {"state": review},
                   "mergeable": mergeable}}


class Queues(unittest.TestCase):
    def test_own_green_pr_awaiting_approval_is_continued_after_idle(self):
        d = doc_with([pr_row("W2-01", "success", "pending", "2026-10-15T10:00:00Z")])
        kinds = [k for k, _ in S.next_actions(d, "worker-1")]
        self.assertEqual(kinds, ["continue-pr"])
        self.assertEqual([k for k, _ in S.next_actions(d, "worker-2")], [])  # not stale for others yet

    def test_pending_ci_is_left_alone(self):
        d = doc_with([pr_row("W2-01", "pending", "pending", "2026-10-15T08:00:00Z")])
        self.assertEqual(S.next_actions(d, "worker-1"), [])

    def test_merger_sync_head_goes_to_reapprove_not_workers(self):
        r = pr_row("W2-01", "success", "stale", "2026-10-15T10:00:00Z", subject=S.SYNC_PREFIX + "wp/W2-01")
        d = doc_with([r])
        self.assertEqual(S.next_actions(d, "worker-1"), [])
        self.assertEqual([k for k, _ in S.merge_queue(d)], ["reapprove"])
        # A red CI after the merger's sync goes back to the worker.
        r["pr"]["ci"]["state"] = "failure"
        self.assertEqual([k for k, _ in S.next_actions(d, "worker-1")], ["continue-pr"])
        self.assertEqual(S.merge_queue(d), [])

    def test_merge_needs_deps_and_approval(self):
        ok = pr_row("W2-01", "success", "approved", "2026-10-15T10:00:00Z")
        dep = pr_row("W2-02", "success", "approved", "2026-10-15T10:00:00Z", deps=["W2-01"])
        conflict = pr_row("W2-03", "success", "approved", "2026-10-15T10:00:00Z", mergeable=False)
        d = doc_with([ok, dep, conflict])
        self.assertEqual([(k, r["id"]) for k, r in S.merge_queue(d)], [("merge", "W2-01"), ("conflict", "W2-03")])

    def test_red_main_after_rerun_offers_fix_main_first(self):
        ready = {"id": "W2-05", "wave": 2, "size": "L", "title": "x", "depends_on": [], "status": "ready"}
        d = doc_with([ready], main_ci={"state": "failure", "max_attempt": 2})
        self.assertEqual([k for k, _ in S.next_actions(d, "worker-1")], ["fix-main", "ready"])
        d = doc_with([ready], main_ci={"state": "failure", "max_attempt": 1})
        self.assertEqual([k for k, _ in S.next_actions(d, "worker-1")], ["ready"])
        d = doc_with([ready], main_ci={"state": "failure", "max_attempt": 2},
                     fixes=[{"id": "fix/main-ccccccc", "status": "claimed"}])
        self.assertEqual([k for k, _ in S.next_actions(d, "worker-1")], ["ready"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
