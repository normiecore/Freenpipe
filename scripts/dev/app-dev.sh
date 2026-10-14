#!/usr/bin/env bash
# Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
#
# Run the desktop app in development mode, isolated from an installed
# screenpipe (docs/dev/windows-build.md, "Dev isolation").
#
#   scripts/dev/app-dev.sh            # checks, then `bun run tauri dev`
#   scripts/dev/app-dev.sh --check    # checks only
#
# The app reads SCREENPIPE_DATA_DIR (store.bin, db, media), SCREENPIPE_PORT
# (engine API) and SCREENPIPE_FOCUS_PORT (control server), all exported by
# dev-env.sh. The dev bundle identifier differs from the released app's, so
# app-level state (webview storage, single-instance lock) is separate too.

set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# shellcheck source=dev-env.sh
. "$here/dev-env.sh" app || exit 2

port_busy() {
  # True when something listens on 127.0.0.1:$1.
  (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null
}

problems=0
for p in "$SCREENPIPE_PORT" "$SCREENPIPE_FOCUS_PORT"; do
  if port_busy "$p"; then
    echo "app-dev: port $p is already in use; stop that process or export another port" >&2
    problems=1
  fi
done
if port_busy 3030 || port_busy 11435; then
  echo "app-dev: note: a screenpipe instance seems to be running (3030/11435 busy)." >&2
  echo "app-dev: Freenpipe dev will not touch it, but notifications posted by the dev" >&2
  echo "app-dev: build still target :11435 until the notification client honours" >&2
  echo "app-dev: SCREENPIPE_FOCUS_PORT; quit screenpipe first for a clean session." >&2
fi
[ "$problems" = 0 ] || exit 1

if [ "${1:-}" = "--check" ]; then
  echo "app-dev: checks passed"
  exit 0
fi

cd "$root/apps/screenpipe-app-tauri" || exit 1
exec bun run tauri dev
