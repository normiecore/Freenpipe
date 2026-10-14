# Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
#
# Dev isolation for Git Bash on Windows (also works on macOS/Linux shells).
#
#   source scripts/dev/dev-env.sh [name]
#
# Freenpipe keeps screenpipe's drop-in defaults (~/.screenpipe, API port 3030,
# control server 11435). A development machine may also run the real
# screenpipe, so every dev run must use its own data dir, ports and API key.
# This script exports them; it never touches ~/.screenpipe.
#
#   FP_DD / SCREENPIPE_DATA_DIR  isolated data dir (default
#                                $LOCALAPPDATA/freenpipe-dev[/<name>])
#   FP_PORT / SCREENPIPE_PORT    engine API port (default 3035)
#   SCREENPIPE_FOCUS_PORT        app control server port (default 11436)
#   SCREENPIPE_API_KEY           fixed dev key, so nothing reads or rewrites
#                                another install's auth.json
#
# Already-set values are kept, so callers can override any of them first.

_fp_name="${1:-}"
_fp_base="${LOCALAPPDATA:-$HOME/.local/share}"
_fp_base="${_fp_base//\\//}"
if [ -z "${FP_DD:-}" ]; then
  FP_DD="$_fp_base/freenpipe-dev${_fp_name:+/$_fp_name}"
fi

# Refuse anything that resolves to the real data dir. Checked before the
# directory is created, case-insensitively (Windows paths).
_fp_abs() {
  local p="$1"
  # Git Bash: turn C:\x or C:/x into /c/x first so both spellings compare.
  if command -v cygpath >/dev/null 2>&1; then p="$(cygpath -u "$p")"; fi
  { realpath -m "$p" 2>/dev/null || printf '%s\n' "$p"; } | tr '[:upper:]' '[:lower:]'
}
_fp_real="$(_fp_abs "$HOME/.screenpipe")"
_fp_dd_abs="$(_fp_abs "$FP_DD")"
if [ -z "$_fp_real" ] || [ -z "$_fp_dd_abs" ]; then
  # A failed command substitution (Git Bash fork errors) leaves these empty;
  # fail closed instead of guessing.
  echo "dev-env: could not resolve FP_DD or ~/.screenpipe (shell fork failure?); retry" >&2
  unset FP_DD
  unset -f _fp_abs
  return 1 2>/dev/null || exit 1
fi
case "$_fp_dd_abs" in
  "$_fp_real"|"$_fp_real"/*)
    echo "dev-env: refusing FP_DD=$FP_DD, it points into ~/.screenpipe" >&2
    unset FP_DD
    unset -f _fp_abs
    return 1 2>/dev/null || exit 1
    ;;
esac
mkdir -p "$FP_DD" || { echo "dev-env: cannot create $FP_DD" >&2; return 1 2>/dev/null || exit 1; }
unset -f _fp_abs

export FP_DD
export SCREENPIPE_DATA_DIR="$FP_DD"
export FP_PORT="${FP_PORT:-3035}"
export SCREENPIPE_PORT="$FP_PORT"
export SCREENPIPE_FOCUS_PORT="${SCREENPIPE_FOCUS_PORT:-11436}"
export SCREENPIPE_API_KEY="${SCREENPIPE_API_KEY:-fp-dev-key}"

for _fp_p in "$FP_PORT" "$SCREENPIPE_FOCUS_PORT"; do
  case "$_fp_p" in
    3030|11435)
      echo "dev-env: port $_fp_p is reserved for an installed screenpipe; pick another" >&2
      return 1 2>/dev/null || exit 1
      ;;
  esac
done

# Shell helpers for the acceptance harness.
api() { local p="$1"; shift; curl -s -H "Authorization: Bearer $SCREENPIPE_API_KEY" "$@" "http://127.0.0.1:$FP_PORT$p"; }

unset _fp_name _fp_base _fp_real _fp_dd_abs _fp_p
echo "dev-env: FP_DD=$FP_DD port=$FP_PORT focus=$SCREENPIPE_FOCUS_PORT"
