#!/usr/bin/env python3
# Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
"""Free disk in the shared cargo target dir without disturbing running builds.

Cargo hashes workspace crates by their source path, so every WP worktree builds its own copy of the
Freenpipe crates into the shared target dir. Once a worktree is removed those copies are dead. This
script deletes them (found through the dep-info files that name their sources), plus linked test and
binary executables older than --exe-age minutes, which nothing else links against.

    python3 tools/cloud/prune-target.py            # report only
    python3 tools/cloud/prune-target.py --delete
"""
import argparse
import glob
import os
import re
import shutil
import time

state = os.environ.get("FP_STATE", os.path.expanduser("~/.cache/freenpipe"))
ap = argparse.ArgumentParser()
ap.add_argument("--delete", action="store_true")
ap.add_argument("--exe-age", type=int, default=45, help="minutes since an executable was last linked")
ap.add_argument("--target", default=os.environ.get("CARGO_TARGET_DIR", os.path.join(state, "target")))
a = ap.parse_args()

debug = os.path.join(a.target, "debug")
wt_root = os.path.join(state, "wt") + "/"
active = set(os.listdir(wt_root)) if os.path.isdir(wt_root) else set()
wt_re = re.compile(re.escape(wt_root) + r"([^/\s]+)/")

dead = set()
for dep in glob.glob(os.path.join(debug, "deps", "*.d")):
    try:
        with open(dep, errors="replace") as f:
            names = set(wt_re.findall(f.read(20000)))
    except OSError:
        continue
    if names and not names & active:
        dead.add(os.path.basename(dep)[:-2])

paths = []
for stem in dead:
    paths += glob.glob(os.path.join(debug, "deps", stem + "*"))
    paths += glob.glob(os.path.join(debug, "deps", "lib" + stem + ".*"))
    paths.append(os.path.join(debug, ".fingerprint", stem))
deps_dir = os.path.join(debug, "deps")
if os.path.isdir(deps_dir):
    for name in os.listdir(deps_dir):
        p = os.path.join(deps_dir, name)
        if "." not in name and os.path.isfile(p) and os.access(p, os.X_OK):
            if (time.time() - os.path.getmtime(p)) / 60 > a.exe_age:
                paths.append(p)


def size_of(p):
    if os.path.isfile(p):
        return os.path.getsize(p)
    total = 0
    for root, _, files in os.walk(p):
        for x in files:
            try:
                total += os.path.getsize(os.path.join(root, x))
            except OSError:
                pass
    return total


paths = sorted({p for p in paths if os.path.exists(p)})
print(f"active worktrees: {', '.join(sorted(active)) or 'none'}")
print(f"dead crate builds: {len(dead)}; reclaimable: {sum(map(size_of, paths)) / 1e9:.2f} GB in {len(paths)} paths")
if a.delete:
    for p in paths:
        if os.path.isdir(p):
            shutil.rmtree(p, ignore_errors=True)
        else:
            try:
                os.remove(p)
            except OSError:
                pass
    print("deleted")
