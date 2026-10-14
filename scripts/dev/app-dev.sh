#!/usr/bin/env bash
# Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
#
# Run the desktop app in development mode on a machine that also has
# screenpipe installed (docs/dev/windows-build.md, "App dev run").
#
#   scripts/dev/app-dev.sh                   # guards, then `bun run tauri dev`
#   scripts/dev/app-dev.sh --check           # guards only
#   scripts/dev/app-dev.sh -- <args>         # extra arguments for `tauri dev`
#
# Isolated: the app's Rust side reads SCREENPIPE_DATA_DIR (its store.bin,
# db, media, logs), SCREENPIPE_PORT and SCREENPIPE_FOCUS_PORT, all exported
# by dev-env.sh, and this script moves the WebView2 profile to
# $FP_DD/webview2.
#
# NOT isolated: the webview still builds ~/.screenpipe paths itself
# (settings store, chats, Pi chat sessions; see the audit in the doc) and
# can still send API calls to :3030. So this script
#   - refuses while ~/.screenpipe exists, unless FP_ALLOW_HOME_SCREENPIPE=1,
#     and then lists what the session changed there;
#   - moves a ~/.screenpipe that the session created to
#     $FP_DD/home-dot-screenpipe.<timestamp> once the session ends;
#   - refuses while something listens on 3030 or 11435 (an installed
#     screenpipe), unless FP_ALLOW_SCREENPIPE_RUNNING=1.
#
# FP_APP_DEV_CMD, when set, runs instead of `bun run tauri dev` (the guard
# tests use it). The exit status is the session's, or 1 when a guard
# refuses or ~/.screenpipe could not be cleaned up.

set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# shellcheck source=dev-env.sh
. "$here/dev-env.sh" app || exit 2

check_only=0
if [ "${1:-}" = "--check" ]; then
  check_only=1
  shift
fi
[ "${1:-}" = "--" ] && shift

port_busy() {
  # True when something listens on 127.0.0.1:$1.
  (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null
}
screenpipe_running() {
  port_busy 3030 || port_busy 11435
}

home_sp="$HOME/.screenpipe"
dd="${FP_DD//\\//}" # FP_DD may arrive as C:\x\y; forward slashes suit every tool below
problems=0

for p in "$SCREENPIPE_PORT" "$SCREENPIPE_FOCUS_PORT"; do
  if port_busy "$p"; then
    echo "app-dev: port $p is already in use; stop that process or export another port" >&2
    problems=1
  fi
done

if screenpipe_running; then
  if [ "${FP_ALLOW_SCREENPIPE_RUNNING:-0}" = 1 ]; then
    echo "app-dev: warning: a screenpipe instance is listening on 3030/11435 and" >&2
    echo "app-dev: FP_ALLOW_SCREENPIPE_RUNNING=1 is set; the dev UI may talk to it." >&2
  else
    cat >&2 <<'EOF'
app-dev: refusing: a screenpipe instance is listening on 3030 and/or 11435.
app-dev:   The dev webview takes its API port from its own settings store and
app-dev:   can send requests to :3030, and dev notifications post to :11435.
app-dev:   Quit screenpipe first, or set FP_ALLOW_SCREENPIPE_RUNNING=1.
EOF
    problems=1
  fi
fi

home_existed=0
if [ -e "$home_sp" ]; then
  home_existed=1
  if [ "${FP_ALLOW_HOME_SCREENPIPE:-0}" = 1 ]; then
    cat >&2 <<EOF
app-dev: ********************************************************************
app-dev: WARNING: FP_ALLOW_HOME_SCREENPIPE=1 and $home_sp exists.
app-dev: This session reads and writes it: the webview's settings store
app-dev: (store.bin), chats/, pi-chat/ and pi-title/. Changes are listed
app-dev: when the session ends.
app-dev: ********************************************************************
EOF
  else
    cat >&2 <<EOF
app-dev: refusing: $home_sp exists.
app-dev:   The app's webview does not honour SCREENPIPE_DATA_DIR yet. A dev
app-dev:   session would load that store.bin (and may save it back at boot),
app-dev:   list those chats, and write dev chats and Pi sessions next to them
app-dev:   (chats/, pi-chat/, pi-title/). See "Remaining hard-coded
app-dev:   ~/.screenpipe paths" in docs/dev/windows-build.md.
app-dev:   Set FP_ALLOW_HOME_SCREENPIPE=1 to run anyway.
EOF
    problems=1
  fi
else
  echo "app-dev: note: the webview will create $home_sp; it is moved into \$FP_DD when the session ends" >&2
fi

[ "$problems" = 0 ] || exit 1

if [ "$check_only" = 1 ]; then
  echo "app-dev: checks passed"
  exit 0
fi

# windows_webview_env.rs otherwise shares %LOCALAPPDATA%\ScreenPipe\WebView2
# with an installed screenpipe; it honours a preset folder.
wv2="$dd/webview2"
command -v cygpath >/dev/null 2>&1 && wv2="$(cygpath -w "$wv2")"
export WEBVIEW2_USER_DATA_FOLDER="${WEBVIEW2_USER_DATA_FOLDER:-$wv2}"

stamp="$dd/.app-dev-session"
: > "$stamp" || exit 1

# Ctrl+C ends `tauri dev`; this script keeps running to clean up.
trap ':' INT
if [ -n "${FP_APP_DEV_CMD:-}" ]; then
  (cd "$root/apps/screenpipe-app-tauri" && sh -c "$FP_APP_DEV_CMD")
else
  (cd "$root/apps/screenpipe-app-tauri" && bun run tauri dev "$@")
fi
status=$?
trap - INT

if [ -e "$home_sp" ]; then
  if [ "$home_existed" = 0 ]; then
    if screenpipe_running; then
      echo "app-dev: warning: this session created $home_sp, but a screenpipe instance" >&2
      echo "app-dev: is now listening on 3030/11435, so it was left in place. Check it" >&2
      echo "app-dev: and move it away by hand if it holds dev data." >&2
      status=1
    else
      dest="$dd/home-dot-screenpipe.$(date +%Y%m%d-%H%M%S)"
      if mv "$home_sp" "$dest"; then
        echo "app-dev: this session created $home_sp; moved it to $dest" >&2
      else
        echo "app-dev: warning: could not move $home_sp (still in use?); move it by hand" >&2
        status=1
      fi
    fi
  else
    changed="$(find "$home_sp" -maxdepth 3 -newer "$stamp" 2>/dev/null | head -n 25)"
    if [ -n "$changed" ]; then
      echo "app-dev: this session changed entries under $home_sp (first 25):" >&2
      printf '%s\n' "$changed" | sed 's/^/app-dev:   /' >&2
    else
      echo "app-dev: no changes detected under $home_sp" >&2
    fi
  fi
fi
rm -f "$stamp"
exit "$status"
