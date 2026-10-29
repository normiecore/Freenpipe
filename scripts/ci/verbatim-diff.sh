#!/usr/bin/env bash
# Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
#
# Clean-room verbatim gate over the lines a change ADDS.
#
#   scripts/ci/verbatim-diff.sh [BASE [HEAD]]
#
# With no arguments the range comes from the GitHub Actions event:
#   pull_request   BASE = pull_request.base.sha, HEAD = pull_request.head.sha
#   push           BASE = before, HEAD = after
#   anything else  BASE = merge-base(origin/main, HEAD), HEAD = HEAD
# If BASE is missing, all zeros (a new branch) or not in the clone (a force
# push), the merge base with origin/main is used instead, then HEAD's parent.
# The diff is always the three-dot form BASE...HEAD (from the merge base).
#
# Every non-binary file the range adds or modifies outside third_party/ is
# copied whole from HEAD's tree into a temp dir, so line numbers and ```sql
# fences keep their context. tools/clean-room/verbatim_gate.py runs over those
# copies and only hits on added lines are kept. Renames are detected, so a pure
# move adds no lines. Hits go to stdout, to ::warning annotations and to
# $GITHUB_STEP_SUMMARY when it is set.
#
# Report-only: exit 0 whatever the gate finds, because moved or adapted
# upstream MIT code can legitimately match. Reviewers justify each hit in the
# PR description. Exit 2 when the range cannot be resolved or the gate cannot
# run, so a broken gate never passes silently.
set -euo pipefail

die() { echo "verbatim-diff: $*" >&2; exit 2; }

root="$(git rev-parse --show-toplevel)" || die "not inside a git checkout"
cd "$root"

gate="$root/tools/clean-room/verbatim_gate.py"
index="$root/tools/clean-room/corpus-shingles-8.bin"
[ -f "$gate" ] || die "missing tools/clean-room/verbatim_gate.py"
[ -f "$index" ] || die "missing tools/clean-room/corpus-shingles-8.bin"

py=""
for c in python3 python; do
  if command -v "$c" >/dev/null 2>&1 && "$c" -c 'import sys; sys.exit(sys.version_info < (3, 8))' 2>/dev/null; then
    py="$c"; break
  fi
done
[ -n "$py" ] || die "python 3.8+ not found"

# ---- resolve the range -------------------------------------------------------
base="${1:-}"
head="${2:-}"
if [ -z "$base" ] && [ -n "${GITHUB_EVENT_PATH:-}" ] && [ -f "${GITHUB_EVENT_PATH}" ]; then
  case "${GITHUB_EVENT_NAME:-}" in
    pull_request|pull_request_target) keys="pull_request.base.sha pull_request.head.sha" ;;
    push) keys="before after" ;;
    *) keys="" ;;
  esac
  if [ -n "$keys" ]; then
    read -r base head < <("$py" -I -c '
import json, sys
event = json.load(open(sys.argv[1], encoding="utf-8"))
def get(path):
    v = event
    for k in path.split("."):
        v = v.get(k) if isinstance(v, dict) else None
    return v or "-"
print(get(sys.argv[2]), get(sys.argv[3]))
' "$GITHUB_EVENT_PATH" $keys)
    [ "$base" = "-" ] && base=""
    [ "$head" = "-" ] && head=""
  fi
fi
head="${head:-HEAD}"
git cat-file -e "${head}^{commit}" 2>/dev/null || die "HEAD commit $head is not in this clone (fetch-depth 0?)"

is_commit() { [ -n "$1" ] && [ "$1" != "0000000000000000000000000000000000000000" ] && git cat-file -e "${1}^{commit}" 2>/dev/null; }

if ! is_commit "$base"; then
  [ -n "$base" ] && echo "verbatim-diff: base $base not usable, falling back"
  base=""
  if git rev-parse -q --verify origin/main >/dev/null; then
    mb="$(git merge-base origin/main "$head" || true)"
    if [ -n "$mb" ] && [ "$mb" != "$(git rev-parse "$head")" ]; then base="$mb"; fi
  fi
  if [ -z "$base" ] && git rev-parse -q --verify "${head}^" >/dev/null; then base="$(git rev-parse "${head}^")"; fi
  [ -n "$base" ] || die "cannot pick a base for $head"
fi
echo "verbatim-diff: range ${base:0:12}...$(git rev-parse --short=12 "$head")"

# ---- collect added lines -----------------------------------------------------
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
stage="$tmp/stage"
mkdir -p "$stage"

# -U0 hunks; "+++ b/<path>" headers name the new file; binary files have no
# hunks. Header lines are only trusted between "diff --git" and the first "@@",
# so an added line that itself starts with "++ " is not mistaken for a header.
git -c core.quotePath=false -c diff.renameLimit=20000 diff --no-color --no-ext-diff \
    --src-prefix=a/ --dst-prefix=b/ --find-renames -U0 "$base...$head" -- . ':(exclude)third_party' \
  | awk '
      /^diff --git / { inhdr = 1; path = ""; next }
      inhdr && /^\+\+\+ / {
        p = substr($0, 5)
        if (p == "/dev/null") { path = "" } else { sub(/^"/, "", p); sub(/"$/, "", p); sub(/^b\//, "", p); path = p }
        next
      }
      /^@@ / {
        inhdr = 0
        if (match($0, /\+[0-9]+/)) { ln = substr($0, RSTART + 1, RLENGTH - 1) + 0 }
        next
      }
      !inhdr && /^\+/ { if (path != "") print path "\t" ln; ln++; next }
    ' > "$tmp/added.tsv"

added_lines=$(wc -l < "$tmp/added.tsv" | tr -d ' ')
cut -f1 "$tmp/added.tsv" | sort -u > "$tmp/files.txt"
nfiles=$(wc -l < "$tmp/files.txt" | tr -d ' ')
echo "verbatim-diff: $added_lines added line(s) in $nfiles file(s)"

# ---- stage whole files from HEAD's tree --------------------------------------
staged=0
while IFS= read -r f; do
  [ -n "$f" ] || continue
  mkdir -p "$stage/$(dirname "$f")"
  if git cat-file -e "$head:$f" 2>/dev/null && git show "$head:$f" > "$stage/$f" 2>/dev/null; then
    staged=$((staged + 1))
  else
    rm -f "$stage/$f"
  fi
done < "$tmp/files.txt"

# ---- run the gate in chunks (argument-length safety) -------------------------
mapfile -d '' files < <(cd "$stage" && find . -type f -print0 | sort -z)
total=${#files[@]}
i=0
while [ "$i" -lt "$total" ]; do
  chunk=("${files[@]:i:500}")
  # The gate exits 1 on hits (and on a crash); the JSON file only appears after
  # a complete scan, so its presence is the success signal.
  (cd "$stage" && "$py" -I "$gate" --index "$index" --json "$tmp/hits-$i.json" "${chunk[@]}" > "$tmp/gate-$i.log" 2>&1) || true
  if [ ! -f "$tmp/hits-$i.json" ]; then
    cat "$tmp/gate-$i.log" >&2
    die "verbatim_gate.py failed"
  fi
  i=$((i + 500))
done

# ---- keep hits on added lines, report -----------------------------------------
"$py" -I - "$tmp" "$base" "$(git rev-parse "$head")" "$staged" "$added_lines" <<'PY'
import glob, json, os, sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
tmp, base, head, staged, added_lines = sys.argv[1:6]
added = {}
with open(os.path.join(tmp, "added.tsv"), encoding="utf-8", errors="replace") as fh:
    for row in fh:
        path, _, ln = row.rstrip("\n").rpartition("\t")
        if path and ln.isdigit():
            added.setdefault(path, set()).add(int(ln))

hits = []
for name in sorted(glob.glob(os.path.join(tmp, "hits-*.json"))):
    with open(name, encoding="utf-8") as fh:
        for h in json.load(fh):
            f = h["file"].replace("\\", "/")
            if f.startswith("./"):
                f = f[2:]
            if h["line"] in added.get(f, ()):
                hits.append((f, h["line"], h.get("runs", []), h.get("text", "")))
hits.sort()

def esc_data(s):
    return s.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")

def esc_prop(s):
    return esc_data(s).replace(":", "%3A").replace(",", "%2C")

print(f"verbatim-diff: scanned {staged} file(s), {added_lines} added line(s): {len(hits)} hit(s)")
for f, ln, runs, text in hits:
    print(f"{f}:{ln}: {text[:160]}")
    run = runs[0] if runs else ""
    print(f"::warning file={esc_prop(f)},line={ln},title=verbatim gate::"
          + esc_data(f"8-word prose run shared with the target corpus: \"{run}\". Justify it in the PR description or rewrite it."))

summary = os.environ.get("GITHUB_STEP_SUMMARY")
if summary:
    def cell(s):
        return s.replace("|", "\\|").replace("\n", " ")
    with open(summary, "a", encoding="utf-8") as out:
        out.write("## Clean-room verbatim gate (added lines, report-only)\n\n")
        out.write(f"Range `{base[:12]}...{head[:12]}`: {staged} file(s), {added_lines} added line(s) scanned, "
                  f"**{len(hits)} hit(s)**.\n\n")
        if hits:
            out.write("Each hit shares an 8-word prose run with the shipped-binary corpus. "
                      "The PR description must justify it (for example moved MIT base code) or the line must be rewritten.\n\n")
            out.write("| File | Line | Shared run(s) |\n|---|---:|---|\n")
            for f, ln, runs, _ in hits:
                out.write(f"| `{cell(f)}` | {ln} | {cell('; '.join(runs))} |\n")
            out.write("\n")
PY
exit 0
