#!/usr/bin/env bash
# Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
#
# Engine smoke test with dev isolation (docs/dev/windows-build.md):
#   1. start the engine on the dev port with the isolated data dir,
#   2. wait for GET /health to answer with JSON and print it,
#   3. stop the engine and delete what it captured (the run uses a
#      throwaway $FP_DD/engine-smoke data dir, removed afterwards),
#   4. fail if ~/.screenpipe (auth.json included) was created or modified.
#
#   scripts/dev/engine-smoke.sh [path/to/screenpipe(.exe)] [timeout-seconds]
#
# Honours FP_DD / FP_PORT / SCREENPIPE_API_KEY when already exported.

set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# shellcheck source=dev-env.sh
. "$here/dev-env.sh" smoke || exit 2

exe="${1:-$root/target/release-dev/screenpipe.exe}"
[ -f "$exe" ] || exe="${exe%.exe}"
timeout_s="${2:-120}"
if [ ! -f "$exe" ]; then
  echo "engine-smoke: no engine binary at $exe" >&2
  echo "  build it: cargo build --profile release-dev -p screenpipe-engine --bin screenpipe" >&2
  exit 2
fi

# The engine links OpenBLAS dynamically (antirez-asr-sys). The app bundle
# ships libopenblas.dll next to its exe; a bare CLI build needs it on PATH.
blas_bin="$root/apps/screenpipe-app-tauri/src-tauri/openblas/bin"
[ -d "$blas_bin" ] && PATH="$blas_bin:$PATH"

real="$HOME/.screenpipe"
real_existed=no
[ -e "$real" ] && real_existed=yes
auth_before=""
[ -f "$real/auth.json" ] && auth_before="$(stat -c %Y "$real/auth.json")"

dd="$FP_DD/engine-smoke"
rm -rf "$dd" && mkdir -p "$dd"
log="$FP_DD/engine-smoke.log"
echo "engine-smoke: starting $exe on :$FP_PORT (log: $log)"
"$exe" record --port "$FP_PORT" --data-dir "$dd" --disable-audio --disable-telemetry \
  > "$log" 2>&1 &
pid=$!

healthy=no
body=""
for _ in $(seq 1 "$timeout_s"); do
  body="$(curl -s -m 3 -H "Authorization: Bearer $SCREENPIPE_API_KEY" \
    "http://127.0.0.1:$FP_PORT/health" 2>/dev/null || true)"
  case "$body" in
    \{*) healthy=yes; break ;;
  esac
  kill -0 "$pid" 2>/dev/null || break
  sleep 1
done

echo "engine-smoke: /health -> ${body:0:800}"

# Stop only the process we started (by PID).
if kill -0 "$pid" 2>/dev/null; then
  winpid="$(cat "/proc/$pid/winpid" 2>/dev/null || true)"
  kill "$pid" 2>/dev/null
  for _ in 1 2 3 4 5 6 7 8 9 10; do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
  if kill -0 "$pid" 2>/dev/null && [ -n "$winpid" ] && command -v taskkill >/dev/null; then
    taskkill //PID "$winpid" //T //F >/dev/null 2>&1
  fi
fi
wait "$pid" 2>/dev/null

# Captured media and the throwaway database go; the log stays for inspection.
rm -rf "$dd" 2>/dev/null

status=0
if [ "$healthy" != yes ]; then
  echo "engine-smoke: FAIL engine never answered /health with JSON (see $log)" >&2
  status=1
fi
if [ "$real_existed" = no ] && [ -e "$real" ]; then
  echo "engine-smoke: FAIL $real was created by this run" >&2
  status=1
fi
if [ -n "$auth_before" ]; then
  auth_after="$(stat -c %Y "$real/auth.json" 2>/dev/null || echo missing)"
  if [ "$auth_after" != "$auth_before" ]; then
    echo "engine-smoke: FAIL $real/auth.json changed ($auth_before -> $auth_after)" >&2
    status=1
  fi
elif [ -e "$real/auth.json" ]; then
  echo "engine-smoke: FAIL $real/auth.json appeared during this run" >&2
  status=1
fi
[ "$status" = 0 ] && echo "engine-smoke: PASS (~/.screenpipe untouched, existed before: $real_existed)"
exit "$status"
