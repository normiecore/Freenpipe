# Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
#
# Load the MSVC (VS 2022 Build Tools) + Windows SDK build environment into a
# Git Bash shell, the equivalent of opening an "x64 Native Tools" prompt.
#
#   source scripts/dev/msvc-env.sh
#
# Needed by every native build in this repo: bindgen (whisper-rs-sys,
# antirez-asr-sys, libsamplerate-sys) cannot find stdio.h without INCLUDE and
# LIBCLANG_PATH, and the cmake-based -sys crates need the VS 2022 generator.
# See docs/dev/windows-build.md for the full recipe.

_fp_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
_fp_vswhere="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
if [ ! -x "$_fp_vswhere" ]; then
  echo "msvc-env: vswhere.exe not found; install Visual Studio 2022 Build Tools (C++ workload)" >&2
  return 1 2>/dev/null || exit 1
fi

# VS 2022 only: an empty VS 18 Build Tools install otherwise wins "-latest"
# and makes cmake pick a generator without a C++ toolset.
_fp_vs="$("$_fp_vswhere" -latest -products '*' -version '[17.0,18.0)' \
  -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 \
  -property installationPath | tr -d '\r')"
if [ -z "$_fp_vs" ]; then
  echo "msvc-env: no VS 2022 install with the x64 C++ tools found" >&2
  return 1 2>/dev/null || exit 1
fi

_fp_tmp="$(mktemp -d)"
# (cmd syntax inside the .bat: NUL is the Windows null device)
printf '@call "%s\\VC\\Auxiliary\\Build\\vcvars64.bat" >NUL 2>&1\r\n@set\r\n' "$_fp_vs" > "$_fp_tmp/vcvars.bat"
_fp_envdump="$(cmd.exe //d //c "$(cygpath -w "$_fp_tmp/vcvars.bat")" | tr -d '\r')"
rm -rf "$_fp_tmp"

_fp_keep=" INCLUDE LIB LIBPATH Platform DevEnvDir ExtensionSdkDir UCRTVersion UniversalCRTSdkDir \
VCINSTALLDIR VCToolsInstallDir VCToolsRedistDir VCToolsVersion VSINSTALLDIR VisualStudioVersion \
VSCMD_ARG_HOST_ARCH VSCMD_ARG_TGT_ARCH VSCMD_VER WindowsSDKLibVersion WindowsSDKVersion \
WindowsSdkBinPath WindowsSdkDir "
_fp_winpath=""
while IFS= read -r _fp_line; do
  _fp_k="${_fp_line%%=*}"
  _fp_v="${_fp_line#*=}"
  case "$_fp_k" in
    Path|PATH) _fp_winpath="$_fp_v" ;;
    *) case "$_fp_keep" in *" $_fp_k "*) export "$_fp_k=$_fp_v" ;; esac ;;
  esac
done <<< "$_fp_envdump"

if [ -z "${INCLUDE:-}" ] || [ -z "$_fp_winpath" ]; then
  echo "msvc-env: vcvars64.bat produced no environment" >&2
  return 1 2>/dev/null || exit 1
fi
export PATH="$(cygpath -u -p "$_fp_winpath")"

# LLVM for bindgen, CMake generator, OpenBLAS for antirez-asr-sys.
if [ -z "${LIBCLANG_PATH:-}" ] && [ -d "/c/Program Files/LLVM/bin" ]; then
  export LIBCLANG_PATH='C:\Program Files\LLVM\bin'
fi
export CMAKE_GENERATOR="${CMAKE_GENERATOR:-Visual Studio 17 2022}"
_fp_blas="$_fp_root/apps/screenpipe-app-tauri/src-tauri/openblas"
if [ -z "${OPENBLAS_PATH:-}" ] && [ -d "$_fp_blas" ]; then
  export OPENBLAS_PATH="$(cygpath -w "$_fp_blas")"
fi

echo "msvc-env: VS at $_fp_vs (MSVC $VCToolsVersion, SDK ${WindowsSDKVersion%\\})"
unset _fp_root _fp_vswhere _fp_vs _fp_tmp _fp_envdump _fp_keep _fp_winpath _fp_line _fp_k _fp_v _fp_blas
