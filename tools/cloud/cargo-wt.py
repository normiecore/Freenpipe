#!/usr/bin/env python3
# Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
"""`cargo` wrapper that keeps WP worktrees from sharing builds of the Freenpipe crates.

All worktrees share one CARGO_TARGET_DIR so third-party crates are built once. Cargo names a path
crate's artifacts from a hash that no longer depends on its absolute path, and it judges freshness by
mtime, so worktree B could link the screenpipe-db that worktree A built a minute earlier and test A's
code. This wrapper gives every worktree under $FP_STATE/wt its own `codegen-units` override for each
Freenpipe package the worktree changed against origin/main. The profile is part of cargo's artifact
hash, and a crate's hash includes its dependencies' hashes, so a changed crate and everything that
depends on it get their own artifacts in that worktree. Unchanged crates and all registry and vendored
dependencies stay shared, which keeps the disk cost to what the WP actually touched.

bootstrap.sh installs it as $FP_STATE/bin/cargo and puts that directory first on PATH. Outside a
worktree (the coordinator checkout, other repositories) it runs the real cargo unchanged.
"""
import fcntl
import json
import os
import sys
import time

STATE = os.environ.get("FP_STATE", os.path.expanduser("~/.cache/freenpipe"))
WT_ROOT = os.path.join(STATE, "wt")
CFG_DIR = os.path.join(STATE, "cargo-wt")
REAL = os.environ.get("FP_REAL_CARGO", os.path.expanduser("~/.cargo/bin/cargo"))
SKIP_DIRS = {".git", "target", "third_party", "node_modules", ".next", "out", "dist"}
DEFAULT_CGU = 256
MAX_SLOTS = 128
REFRESH_SECONDS = 60


def worktree_of(path):
    """Return the WP id when `path` lies inside $FP_STATE/wt/<id>, else None."""
    rel = os.path.relpath(os.path.abspath(path), WT_ROOT)
    if rel.startswith(".."):
        return None
    head = rel.split(os.sep, 1)[0]
    return head if head not in (".", "") else None


def target_worktree(args):
    for i, a in enumerate(args):
        if a == "--manifest-path" and i + 1 < len(args):
            return worktree_of(args[i + 1])
        if a.startswith("--manifest-path="):
            return worktree_of(a.split("=", 1)[1])
    return worktree_of(os.getcwd())


def slot_for(wp):
    """Give each live worktree a distinct small number, kept stable while the worktree exists."""
    os.makedirs(CFG_DIR, exist_ok=True)
    with open(os.path.join(CFG_DIR, "slots.lock"), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        path = os.path.join(CFG_DIR, "slots.json")
        try:
            with open(path) as f:
                slots = json.load(f)
        except (OSError, ValueError):
            slots = {}
        live = set(os.listdir(WT_ROOT)) if os.path.isdir(WT_ROOT) else set()
        slots = {k: v for k, v in slots.items() if k in live}
        if wp not in slots:
            used = set(slots.values())
            slots[wp] = next(n for n in range(1, MAX_SLOTS + 1) if n not in used)
        tmp = path + ".tmp"
        with open(tmp, "w") as f:
            json.dump(slots, f, indent=1, sort_keys=True)
        os.replace(tmp, path)
        return slots[wp]


def packages(top):
    """Map each package directory (relative to `top`) to its package name."""
    import tomllib

    found = {}
    for root, dirs, files in os.walk(top):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        if "Cargo.toml" in files:
            try:
                with open(os.path.join(root, "Cargo.toml"), "rb") as f:
                    name = tomllib.load(f).get("package", {}).get("name")
            except (OSError, ValueError):
                continue
            if isinstance(name, str):
                found[os.path.relpath(root, top)] = name
    return found


def changed_paths(top):
    """Files whose content differs from origin/main (either side moved), plus local edits.

    Comparing trees rather than using the merge base means a worktree on an older base also gets its
    own artifacts for crates main has changed since, so the shared artifacts always match origin/main.
    """
    import subprocess

    def git(*a):
        return subprocess.run(["git", "-C", top, *a], capture_output=True, text=True, check=True).stdout

    paths = set(git("diff", "--name-only", "origin/main", "HEAD").split())
    for line in git("status", "--porcelain", "--untracked-files=all").splitlines():
        paths.add(line[3:].split(" -> ")[-1].strip('"'))
    return paths


def changed_packages(top):
    pkgs = packages(top)
    names = set()
    for path in changed_paths(top):
        d = os.path.dirname(path)
        while True:
            if d in pkgs:
                names.add(pkgs[d])
                break
            if not d:
                break
            d = os.path.dirname(d)
    return sorted(names)


def config_for(wp):
    cfg = os.path.join(CFG_DIR, f"{wp}.toml")
    try:
        if time.time() - os.path.getmtime(cfg) < REFRESH_SECONDS:
            return cfg
    except OSError:
        pass
    cgu = DEFAULT_CGU - slot_for(wp)
    lines = [f"# Generated by tools/cloud/cargo-wt.py for worktree {wp}; do not edit.\n"]
    for name in changed_packages(os.path.join(WT_ROOT, wp)):
        lines.append(f'[profile.dev.package."{name}"]\ncodegen-units = {cgu}\n')
    os.makedirs(CFG_DIR, exist_ok=True)
    tmp = f"{cfg}.{os.getpid()}.tmp"
    with open(tmp, "w") as f:
        f.writelines(lines)
    os.replace(tmp, cfg)
    return cfg


def main():
    args = sys.argv[1:]
    toolchain = [args.pop(0)] if args and args[0].startswith("+") else []
    wp = target_worktree(args)
    extra = []
    if wp:
        try:
            extra = ["--config", config_for(wp)]
        except Exception as exc:  # never block a build on the wrapper itself
            print(f"cargo-wt: no per-worktree profile ({exc}); using the shared artifacts", file=sys.stderr)
    os.execv(REAL, [REAL, *toolchain, *extra, *args])


if __name__ == "__main__":
    main()
