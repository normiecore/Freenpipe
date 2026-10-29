# Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Handvel
#
# Minimal GitHub REST client for the planning and cloud tools (stdlib only).
#
# Transport, in order of preference:
#   1. `gh api` when the gh CLI is on PATH. In Claude Code cloud sessions gh is pre-installed and
#      the GitHub proxy authenticates it (GH_TOKEN reads "proxy-injected" there). Only REST paths
#      are used: the proxy rejects GraphQL, which rules out `gh pr` and `gh issue`.
#   2. urllib against https://api.github.com, sending GITHUB_TOKEN or GH_TOKEN when set. Without a
#      token this still works for reads, because normiecore/Handvel is public (60 requests/hour).
#
# Set FP_GH_TRANSPORT=gh|http to force one transport, FP_REPO=owner/name to target another repo.
import json
import os
import shutil
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request

DEFAULT_REPO = os.environ.get("FP_REPO", "normiecore/Handvel")
API_ROOT = "https://api.github.com"


class GitHubError(Exception):
    def __init__(self, status, message, data=None):
        super().__init__(f"GitHub API {status}: {message}")
        self.status = status
        self.message = message
        self.data = data


class GitHub:
    def __init__(self, repo=DEFAULT_REPO, transport=None):
        self.repo = repo
        forced = transport or os.environ.get("FP_GH_TRANSPORT", "")
        if forced in ("gh", "http"):
            self.transport = forced
        else:
            self.transport = "gh" if shutil.which("gh") else "http"
        self.token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN") or ""

    # -- low level -----------------------------------------------------------------------------
    def request(self, method, path, body=None, params=None):
        """Return the decoded JSON body (or None). Raise GitHubError on HTTP >= 400."""
        path = path.lstrip("/")
        if params:
            path += ("&" if "?" in path else "?") + urllib.parse.urlencode(params)
        if self.transport == "gh":
            return self._gh(method, path, body)
        return self._http(method, path, body)

    def _gh(self, method, path, body):
        cmd = ["gh", "api", "-i", "-X", method, "-H", "Accept: application/vnd.github+json", path]
        data_in = None
        if body is not None:
            cmd += ["--input", "-"]
            data_in = json.dumps(body)
        try:
            p = subprocess.run(cmd, input=data_in, capture_output=True, text=True, encoding="utf-8", timeout=120)
        except (OSError, subprocess.TimeoutExpired) as e:
            raise GitHubError(0, f"gh api failed to run: {e}")
        out = p.stdout.replace("\r\n", "\n")
        status, payload = 0, out
        if out.startswith("HTTP/"):
            head, _, payload = out.partition("\n\n")
            try:
                status = int(head.split(None, 2)[1])
            except (IndexError, ValueError):
                status = 0
        data = None
        if payload.strip():
            try:
                data = json.loads(payload)
            except ValueError:
                data = payload
        if p.returncode != 0 or status >= 400 or status == 0:
            msg = data.get("message") if isinstance(data, dict) else (p.stderr.strip() or str(data)[:300])
            raise GitHubError(status, msg or "gh api error", data)
        return data

    def _http(self, method, path, body):
        try:
            return self._http_once(method, path, body, self.token)
        except GitHubError as e:
            # A placeholder token (cloud sessions export GH_TOKEN=proxy-injected) that no proxy
            # replaced: reads of this public repository still work anonymously.
            if e.status == 401 and method == "GET" and self.token:
                return self._http_once(method, path, body, "")
            raise

    def _http_once(self, method, path, body, token):
        url = f"{API_ROOT}/{path}"
        headers = {
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "handvel-plan-tools",
        }
        data = None
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"
        req = urllib.request.Request(url, data=data, headers=headers, method=method)
        if token:
            # Unredirected: never forward the token to a redirect target (signed log/blob URLs).
            req.add_unredirected_header("Authorization", f"Bearer {token}")
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                raw = r.read().decode("utf-8")
                return json.loads(raw) if raw.strip() else None
        except urllib.error.HTTPError as e:
            raw = e.read().decode("utf-8", "replace")
            try:
                payload = json.loads(raw)
            except ValueError:
                payload = raw
            msg = payload.get("message") if isinstance(payload, dict) else raw[:300]
            raise GitHubError(e.code, msg, payload)
        except (urllib.error.URLError, OSError) as e:
            raise GitHubError(0, f"network error: {e}")

    def get_text(self, path):
        """GET a non-JSON body (job logs). GitHub answers with a redirect to a signed download URL on
        a log storage host, which a restricted network allowlist may block."""
        path = path.lstrip("/")
        if self.transport == "gh":
            p = subprocess.run(["gh", "api", path], capture_output=True, text=True, encoding="utf-8",
                               errors="replace", timeout=300)
            if p.returncode != 0:
                raise GitHubError(0, (p.stderr or p.stdout).strip()[:500] or "gh api error")
            return p.stdout
        req = urllib.request.Request(f"{API_ROOT}/{path}", headers={"User-Agent": "handvel-plan-tools"})
        if self.token:
            req.add_unredirected_header("Authorization", f"Bearer {self.token}")
        try:
            with urllib.request.urlopen(req, timeout=300) as r:
                return r.read().decode("utf-8", "replace")
        except urllib.error.HTTPError as e:
            raise GitHubError(e.code, f"{e.reason} ({e.geturl()})")
        except (urllib.error.URLError, OSError) as e:
            raise GitHubError(0, f"network error: {e}")

    def paginate(self, path, params=None, key=None, max_pages=20):
        """GET every page of a list endpoint (per_page=100). `key` picks the list out of an object body."""
        out = []
        for page in range(1, max_pages + 1):
            q = dict(params or {})
            q.update(per_page=100, page=page)
            data = self.request("GET", path, params=q)
            items = data.get(key, []) if key else (data or [])
            out.extend(items)
            if len(items) < 100:
                break
        return out

    # -- repository helpers --------------------------------------------------------------------
    def r(self, suffix):
        return f"repos/{self.repo}/{suffix.lstrip('/')}"

    def open_pulls(self):
        return self.paginate(self.r("pulls"), {"state": "open"})

    def pull(self, number, wait_mergeable=False):
        pr = self.request("GET", self.r(f"pulls/{number}"))
        # GitHub computes `mergeable` lazily; null means "not computed yet".
        tries = 0
        while wait_mergeable and pr.get("mergeable") is None and pr.get("state") == "open" and tries < 5:
            time.sleep(3)
            tries += 1
            pr = self.request("GET", self.r(f"pulls/{number}"))
        return pr

    def issue_comments(self, number):
        return self.paginate(self.r(f"issues/{number}/comments"))

    def check_runs(self, sha):
        return self.paginate(self.r(f"commits/{sha}/check-runs"), key="check_runs")

    def workflow_runs(self, head_sha):
        data = self.request("GET", self.r("actions/runs"), params={"head_sha": head_sha, "per_page": 50})
        return (data or {}).get("workflow_runs", [])
