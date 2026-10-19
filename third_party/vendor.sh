#!/usr/bin/env bash
# Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
#
# Rebuild the vendored dependency trees under third_party/ from cargo's local
# git cache, at the revisions recorded in this file and in third_party/README.md.
#
#   third_party/vendor.sh [--check] [NAME ...]
#
# Nothing is fetched from the network. Each tree is `git archive` of the pinned
# commit in $CARGO_HOME/git/db; git submodules come from cargo's checkout of that
# commit, after their gitlinks are checked against the parent tree. Then the
# script removes what the build does not need (the per-crate prune lists below
# and the common list in `prune_common`), applies third_party/patches/NAME/*.patch
# in name order, and refuses to finish if a symlink, an LFS-tracked file type or
# a path the repository's .gitignore would drop is left behind.
#
# Without --check the named trees (default: all) are replaced in place. With
# --check they are rebuilt in a temporary directory and compared with the
# committed copies; any difference is printed and the exit status is 1.
#
# The cache only holds these revisions while some Cargo.toml still points at
# the git sources (or on a machine that built Freenpipe before W0-13). To move
# a crate to a new revision, point a scratch project at the new git revision,
# `cargo fetch` it, update REV below, and run this script.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/.." && pwd)"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
db_root="$cargo_home/git/db"
co_root="$cargo_home/git/checkouts"

die() { echo "vendor.sh: $*" >&2; exit 2; }

# ---- helpers -------------------------------------------------------------------

# Locate the bare clone cargo keeps for a repository (dir name "<repo>-<hash>")
# that contains the commit.
find_db() {
  local repo=$1 rev=$2 d
  for d in "$db_root/$repo"-*; do
    [ -d "$d" ] || continue
    if git -C "$d" cat-file -e "$rev^{commit}" 2>/dev/null; then echo "$d"; return 0; fi
  done
  die "commit $rev of $repo is not in $db_root (run cargo fetch against it first)"
}

# Locate cargo's working checkout of that commit (needed for submodules only).
find_checkout() {
  local db=$1 rev=$2 d
  for d in "$co_root/$(basename "$db")/${rev:0:7}"*; do
    [ -d "$d" ] && { echo "$d"; return 0; }
  done
  die "no cargo checkout of $rev under $co_root/$(basename "$db")"
}

# start NAME REPO REV: extract the pinned tree into $dest.
start() {
  NAME=$1 REPO=$2 REV=$3
  DB="$(find_db "$REPO" "$REV")"
  CHECKOUT=""
  dest="$stage/$NAME"
  mkdir -p "$dest"
  git -C "$DB" archive --format=tar "$REV" | tar -x -C "$dest"
  declare -gA SUBSHA=()
}

# submodule PATH: fill the submodule at PATH (relative to the crate root). For a
# nested submodule, name the outer one first.
submodule() {
  local path=$1 parent_dir parent_rev rel best="" p
  [ -n "$CHECKOUT" ] || CHECKOUT="$(find_checkout "$DB" "$REV")"
  for p in "${!SUBSHA[@]}"; do
    case "$path" in "$p"/*) [ ${#p} -gt ${#best} ] && best=$p ;; esac
  done
  if [ -n "$best" ]; then
    parent_dir="$CHECKOUT/$best"; parent_rev="${SUBSHA[$best]}"; rel="${path#"$best"/}"
  else
    parent_dir="$DB"; parent_rev="$REV"; rel="$path"
  fi
  local line sha
  line="$(git -C "$parent_dir" ls-tree "$parent_rev" -- "$rel")"
  [ "$(echo "$line" | awk '{print $2}')" = "commit" ] || die "$NAME: $path is not a submodule"
  sha="$(echo "$line" | awk '{print $3}')"
  git -C "$CHECKOUT/$path" cat-file -e "$sha^{commit}" 2>/dev/null \
    || die "$NAME: submodule $path commit $sha missing from the cargo checkout"
  rm -rf "${dest:?}/$path"
  mkdir -p "$dest/$path"
  git -C "$CHECKOUT/$path" archive --format=tar "$sha" | tar -x -C "$dest/$path"
  SUBSHA[$path]=$sha
  echo "  $NAME: submodule $path @ $sha"
}

# drop PATH...: remove files or directories (relative to the crate root).
drop() {
  local p
  for p in "$@"; do
    [ -e "$dest/$p" ] || [ -L "$dest/$p" ] || die "$NAME: nothing to drop at $p (list out of date?)"
    rm -rf "${dest:?}/$p"
  done
}

# keep_only DIR ENTRY...: inside DIR keep only the listed files and directories.
keep_only() {
  local dir=$1 f rel keep e
  shift
  while IFS= read -r -d '' f; do
    rel="${f#"$dest/$dir"/}"
    keep=0
    for e in "$@"; do
      case "$rel" in "$e" | "$e"/*) keep=1; break ;; esac
    done
    [ "$keep" = 1 ] || rm -f "$f"
  done < <(find "$dest/$dir" \( -type f -o -type l \) -print0)
  find "$dest/$dir" -depth -type d -empty -delete
}

# Removed from every tree: VCS and CI metadata, other projects' agent
# instructions, lock files of standalone builds, and Python helper scripts
# (development tooling only; the repository's .gitignore drops *.py anyway).
prune_common() {
  find "$dest" \( -name .git -o -name .github \) -prune -exec rm -rf {} +
  find "$dest" \( -name .gitignore -o -name .gitmodules -o -name .gitattributes \
    -o -name Cargo.lock -o -name CLAUDE.md -o -name AGENTS.md -o -name AGENT.md \
    -o -name '*.py' \) -exec rm -f {} +
  find "$dest" -depth -type d -empty -delete
}

finish() {
  prune_common
  local p
  if [ -d "$here/patches/$NAME" ]; then
    for p in "$here/patches/$NAME"/*.patch; do
      [ -f "$p" ] || continue
      patch -s -p1 -d "$dest" --no-backup-if-mismatch < "$p" || die "$NAME: $(basename "$p") does not apply"
      echo "  $NAME: applied $(basename "$p")"
    done
  fi
  local bad
  bad="$(find "$dest" -type l)"
  [ -z "$bad" ] || die "$NAME: symlinks left: $bad"
  # Types .gitattributes routes through LFS; CI checks out without LFS.
  bad="$(find "$dest" -type f \( -name '*.onnx' -o -name '*.wav' -o -name '*.gif' -o -name '*.mp4' \
    -o -name '*.safetensors' -o -name '*.zip' -o -name '*.tar.gz' \))"
  [ -z "$bad" ] || die "$NAME: LFS-tracked file types left: $bad"
  bad="$(cd "$dest" && find . -type f | sed 's|^\./||' | sed "s|^|third_party/$NAME/|" \
    | git -C "$repo_root" check-ignore --no-index --stdin || true)"
  [ -z "$bad" ] || die "$NAME: paths the repository would ignore: $bad"
}

# ---- the vendored crates -------------------------------------------------------
# Each entry: where it comes from, what is removed, and why (see README.md).

v_whisper_rs() {
  start whisper-rs whisper-rs 97e40705033724dee601e89f3bd701af8096c846
  submodule sys/whisper.cpp
  # examples/full_usage is a separate crate with a WAV sample.
  drop examples/full_usage
  # whisper-rs-sys builds whisper.cpp with CMake from these paths only (the
  # same set as the sys crate's `include` list, plus the attribution files).
  keep_only sys/whisper.cpp AUTHORS CMakeLists.txt LICENSE README.md cmake include src ggml \
    bindings/CMakeLists.txt bindings/javascript/package-tmpl.json
  # ggml backends that no whisper-rs feature can switch on.
  drop sys/whisper.cpp/ggml/src/ggml-{cann,hexagon,musa,opencl,rpc,webgpu,zdnn,zendnn}
  finish
}

v_rusty_tesseract() {
  start rusty-tesseract rusty-tesseract 08346c1de08d122c7121425abc79081c30dbf778
  drop img   # README screenshots
  finish
}

v_cpal() {
  start cpal cpal c7a610aee5c6037c9c69dc5bf7f2a850da8d8b12
  # iOS Xcode demo and the wasm demo crate; container/cross-compile configs.
  drop examples/ios-feedback examples/wasm-beep Cross.toml Dockerfile
  finish
}

v_sck_rs() {
  start sck-rs sck-rs b7f48a1236b2339350baa1375a805c7ce32f88df
  finish
}

v_opf_rs() {
  start opf-rs opf-rs e725ad5e8a89ae46e4cb38022a26e746c37329b6
  # Parity test against a 1.9 MB safetensors golden, and the script that dumps it.
  drop tests tools
  # A redaction benchmark whose fixtures are realistic-looking (fake) API keys;
  # GitHub push protection rejects them. Cargo.toml still names the example,
  # which only matters to `cargo run --example` inside opf-rs itself.
  drop examples/secret_probe.rs
  finish
}

v_tauri_helper() {
  start tauri-helper tauri-helper e25ff1d2c5026189305a0251a1e06788efe1d2b3
  finish
}

v_eventkit_rs() {
  start eventkit-rs eventkit-rs 9dd52787c8435d734ed443e2d5641f45b0177cae
  # nextest config and release/CI helper scripts.
  drop .config ci-check.sh build-universal.sh
  finish
}

v_vad_rs() {
  start vad-rs vad-rs-1 b14b26ec7093f5398ef6658020259cd7c130ea6c
  # The whisper demo is its own crate with a git dependency of its own.
  drop .vscode examples/whisper
  finish
}

v_permission_flow() {
  start permission-flow permission-flow d7376a1b95acb78700e93505d8ca058e5ddb94ce
  # README animation, the Tauri demo app (icons, generated schemas), the Swift
  # package's Xcode demo and CI, and the npm lock of the JS guest bindings.
  drop assets crates/tauri-plugin-permission-flow/examples \
    crates/tauri-plugin-permission-flow/package-lock.json \
    crates/flow/PermissionFlow/Example
  finish
}

v_muda() {
  start muda muda 9dc5de53e84040a63c9a5423b669ec892de8a671
  # Demo programs (tao/winit/wry) and the common-controls demo crate.
  drop .changes renovate.json examples
  finish
}

v_audiopipe() {
  start audiopipe audiopipe 2fd5c360651157a5df504394066e509ad4d1bdc9
  submodule crates/antirez-asr-sys/qwen-asr
  submodule crates/qwen3-asr-sys/qwen3-asr.cpp
  submodule crates/qwen3-asr-sys/qwen3-asr.cpp/ggml
  # antirez-asr-sys compiles the C sources in qwen-asr/; the rest of that
  # repository is sample audio, model cards and a standalone CLI build.
  drop crates/antirez-asr-sys/qwen-asr/samples crates/antirez-asr-sys/qwen-asr/MODEL_CARD_OFFICIAL.md \
    crates/antirez-asr-sys/qwen-asr/MODEL.md crates/antirez-asr-sys/qwen-asr/download_model.sh
  # qwen3-asr-sys builds ggml with CMake and compiles qwen3-asr.cpp/src with cc.
  keep_only crates/qwen3-asr-sys/qwen3-asr.cpp CMakeLists.txt README.md src \
    ggml/AUTHORS ggml/CMakeLists.txt ggml/LICENSE ggml/README.md ggml/ggml.pc.in ggml/cmake ggml/include ggml/src
  # ggml backends that no qwen3-asr-sys feature can switch on.
  drop crates/qwen3-asr-sys/qwen3-asr.cpp/ggml/src/ggml-{cann,hexagon,hip,musa,opencl,rpc,sycl,virtgpu,webgpu,zdnn,zendnn}
  # Model conversion scripts.
  drop scripts
  finish
}

v_hf_hub() {
  start hf-hub hf-hub 437dcf4049233f4a0e0cfc4e1c1dbf5ed2c57760
  drop .pre-commit-config.yaml flake.lock flake.nix
  finish
}

v_windows_icons() {
  start windows-icons windows-icons 4dcd086b36bed286181cb87636ad6eb058ab7684
  drop output   # PNGs the example writes
  finish
}

ALL=(whisper-rs rusty-tesseract cpal sck-rs opf-rs tauri-helper eventkit-rs vad-rs
  permission-flow muda audiopipe hf-hub windows-icons)

# ---- main ----------------------------------------------------------------------

check=0
if [ "${1:-}" = "--check" ]; then check=1; shift; fi
names=("$@")
[ ${#names[@]} -gt 0 ] || names=("${ALL[@]}")

command -v git >/dev/null || die "git not found"
command -v patch >/dev/null || die "patch not found"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

status=0
for n in "${names[@]}"; do
  fn="v_${n//-/_}"
  declare -F "$fn" >/dev/null || die "unknown crate $n (known: ${ALL[*]})"
  echo "vendor.sh: $n"
  "$fn"
  if [ "$check" = 1 ]; then
    if diff -r "$stage/$n" "$here/$n" >/dev/null 2>&1; then
      echo "  $n: matches the committed tree"
    else
      diff -rq "$stage/$n" "$here/$n" || true
      status=1
    fi
  else
    rm -rf "${here:?}/$n"
    cp -R "$stage/$n" "$here/$n"
    echo "  $n: written to third_party/$n"
  fi
done
exit "$status"
