#!/usr/bin/env bash
# Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
#
# Create or reuse the git worktree for one work package and print its path.
#
#   bash tools/cloud/wp-worktree.sh <WP-ID> <base-commit>
#
# Order of preference: an existing worktree at $FP_STATE/wt/<WP-ID>; the local branch wp/<WP-ID>;
# the pushed branch origin/wp/<WP-ID> (a session that died left work there); a new branch from
# <base-commit>. An existing local branch is fast-forwarded to origin when it is strictly behind.
# Run it from the coordinator checkout (the repository the worktrees belong to).
set -euo pipefail
wp="${1:?usage: wp-worktree.sh <WP-ID> <base-commit>}"
base="${2:?usage: wp-worktree.sh <WP-ID> <base-commit>}"
repo="$(git rev-parse --show-toplevel)"
state="${FP_STATE:-$HOME/.cache/freenpipe}"
wt="$state/wt/$wp"
br="wp/$wp"
mkdir -p "$state/wt"
git -C "$repo" fetch -q origin "+refs/heads/$br:refs/remotes/origin/$br" 2>/dev/null || true
if [ -d "$wt/.git" ] || [ -f "$wt/.git" ]; then
  :
elif git -C "$repo" rev-parse -q --verify "refs/heads/$br" >/dev/null; then
  git -C "$repo" worktree add -q "$wt" "$br"
elif git -C "$repo" rev-parse -q --verify "refs/remotes/origin/$br" >/dev/null; then
  git -C "$repo" worktree add -q "$wt" -b "$br" "origin/$br"
else
  git -C "$repo" worktree add -q "$wt" -b "$br" "$base"
fi
if git -C "$wt" rev-parse -q --verify "refs/remotes/origin/$br" >/dev/null &&
   [ -z "$(git -C "$wt" status --porcelain --untracked-files=no)" ] &&
   git -C "$wt" merge-base --is-ancestor HEAD "origin/$br" &&
   [ "$(git -C "$wt" rev-parse HEAD)" != "$(git -C "$wt" rev-parse "origin/$br")" ]; then
  git -C "$wt" merge -q --ff-only "origin/$br"
fi
# The app crate needs the Linux sidecars and the ../out placeholder that bootstrap.sh puts in the
# coordinator checkout; they are gitignored, so a fresh worktree lacks them.
tauri="$wt/apps/screenpipe-app-tauri/src-tauri"
if [ -d "$tauri" ]; then
  if [ ! -e "$tauri/ffmpeg/ffmpeg" ] && command -v ffmpeg >/dev/null 2>&1; then
    mkdir -p "$tauri/ffmpeg"
    ln -sf "$(command -v ffmpeg)" "$tauri/ffmpeg/ffmpeg"
    if command -v ffprobe >/dev/null 2>&1; then ln -sf "$(command -v ffprobe)" "$tauri/ffmpeg/ffprobe"; fi
  fi
  if [ ! -e "$tauri/tesseract" ] && command -v tesseract >/dev/null 2>&1; then
    ln -sf "$(command -v tesseract)" "$tauri/tesseract"
  fi
  if [ ! -e "$tauri/bun-x86_64-unknown-linux-gnu" ] && command -v bun >/dev/null 2>&1; then
    cp "$(command -v bun)" "$tauri/bun-x86_64-unknown-linux-gnu" && chmod +x "$tauri/bun-x86_64-unknown-linux-gnu"
  fi
  if [ ! -e "$tauri/../out/index.html" ]; then
    mkdir -p "$tauri/../out" && printf '<!doctype html><title>placeholder</title>\n' > "$tauri/../out/index.html"
  fi
fi
echo "$wt"
