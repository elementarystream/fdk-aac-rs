#!/bin/bash
# Sets up the C++ reference material used for porting and differential testing (see
# PORTING.md, "Reference" and "Reference material"). Everything is written below work/, which is
# git-ignored:
#
#   work/fdk-aac                    full clone of mstorsjo/fdk-aac (history, other revisions)
#   work/fdk-aac-v2.0.3             git worktree of the tag v2.0.3 (commit 716f439, verified)
#   work/build-ref/<build>/         CMake Release build (static libfdk-aac.a, aac-enc,
#                                   test-encode-decode) plus the reference harness programs
#                                   (harness/fdkdec-ref, harness/fdkenc-ref) and VERSION
#   work/fdk-aac-resolved/<variant> sources with the architecture conditionals resolved by
#                                   unifdef and the license headers stripped (for reading)
#   work/fdk-aac-pp/<build>/        preprocessed translation units (*.ii) of the reference build,
#                                   made from compile_commands.json with -E: the ground truth of
#                                   what is compiled
#   work/fdk-aac-asm/<build>/       disassembly of the reference objects
#
# Builds (PORTING.md, "Reference builds"):
#
#   ref-arm64        macOS on Apple silicon, Apple clang, arm64       (variant aarch64)
#   ref-x86_64-mac   macOS on Apple silicon, Apple clang, x86_64, runs under Rosetta 2
#                    (variant x86; a development aid, not the reference)
#   ref-x86_64       Linux x86-64, clang                              (variant x86; authoritative)
#   ref-i686         Linux x86-64 host, clang -m32 (i686, x87 floating point; variant x86_32)
#
# On macOS arm64 hosts the script builds ref-arm64 and ref-x86_64-mac, on Linux x86-64 hosts
# ref-x86_64. --build NAME selects one build; ref-i686 is built only when selected this way
# (it needs the i386 C and C++ runtime of scripts/docker/linux-x86_64.Dockerfile). The reference is built with clang unless
# CC/CXX select another compiler: undefined shifts behave differently between compilers.
#
# Usage: scripts/setup_reference.sh [--skip-clone] [--skip-build] [--skip-derived] [--build NAME]
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${FDK_AAC_RS_WORK_DIR:-$REPO/work}"
CLONE="$WORK/fdk-aac"
SRC="$WORK/fdk-aac-v2.0.3"
URL="https://github.com/mstorsjo/fdk-aac.git"
TAG="v2.0.3"
COMMIT="716f4394641d53f0d79c9ddac3fa93b03a49f278"

skip_clone=0; skip_build=0; skip_derived=0; only=""
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-clone) skip_clone=1 ;;
    --skip-build) skip_build=1 ;;
    --skip-derived) skip_derived=1 ;;
    --build) only="$2"; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

jobs() { echo $(( $( (sysctl -n hw.ncpu 2>/dev/null || nproc) ) / 2 > 0 ? $( (sysctl -n hw.ncpu 2>/dev/null || nproc) ) / 2 : 1 )); }

# --- Sources -------------------------------------------------------------------------------

mkdir -p "$WORK"
if [ $skip_clone -eq 0 ]; then
  if [ ! -d "$CLONE/.git" ] && [ ! -f "$CLONE/.git" ]; then
    git clone "$URL" "$CLONE"
  fi
  if [ ! -e "$SRC/.git" ]; then
    git -C "$CLONE" fetch --tags origin
    git -C "$CLONE" worktree add --detach "$SRC" "$TAG"
  fi
fi
got="$(git -C "$SRC" rev-parse HEAD)"
if [ "$got" != "$COMMIT" ]; then
  echo "error: $SRC is at $got, expected $TAG = $COMMIT" >&2
  exit 1
fi
if [ "$(git -C "$SRC" rev-parse "$TAG^{commit}")" != "$COMMIT" ]; then
  echo "error: tag $TAG does not point to $COMMIT" >&2
  exit 1
fi
if [ -n "$(git -C "$SRC" status --porcelain --untracked-files=no)" ]; then
  echo "error: $SRC has local modifications" >&2
  exit 1
fi

# --- Builds --------------------------------------------------------------------------------

host="$(uname -s)-$(uname -m)"
case "$host" in
  Darwin-arm64) builds=(ref-arm64 ref-x86_64-mac) ;;
  Linux-x86_64) builds=(ref-x86_64) ;;
  *) echo "error: unsupported host $host" >&2; exit 1 ;;
esac
if [ -n "$only" ]; then builds=("$only"); fi

variant_of() {
  case "$1" in
    ref-arm64) echo aarch64 ;;
    ref-x86_64-mac|ref-x86_64) echo x86 ;;
    ref-i686) echo x86_32 ;;
    *) echo "unknown build $1" >&2; exit 1 ;;
  esac
}

cmake_arch_args() {
  case "$1" in
    ref-arm64) echo "-DCMAKE_OSX_ARCHITECTURES=arm64" ;;
    ref-x86_64-mac) echo "-DCMAKE_OSX_ARCHITECTURES=x86_64" ;;
    ref-x86_64) echo "" ;;
    # CMAKE_<LANG>_FLAGS, not the Release flags: CMAKE_CXX_FLAGS_RELEASE stays -O3 -DNDEBUG.
    ref-i686) echo "-DCMAKE_C_FLAGS=-m32 -DCMAKE_CXX_FLAGS=-m32" ;;
  esac
}

build_one() {
  local name="$1" dir="$WORK/build-ref/$1"
  local cc="${CC:-clang}" cxx="${CXX:-clang++}"
  # shellcheck disable=SC2046
  cmake -S "$SRC" -B "$dir" -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
    -DBUILD_PROGRAMS=ON -DCMAKE_EXPORT_COMPILE_COMMANDS=ON \
    -DCMAKE_C_COMPILER="$cc" -DCMAKE_CXX_COMPILER="$cxx" $(cmake_arch_args "$name")
  cmake --build "$dir" -j"$(jobs)"
  {
    echo "build: $name"
    echo "variant: $(variant_of "$name")"
    echo "source: $TAG $COMMIT"
    echo "host: $host"
    echo "cmake: $(cmake --version | head -1)"
    echo "compiler: $("$cxx" --version | head -1)"
    echo "flags: $(grep -E '^CMAKE_CXX_FLAGS_RELEASE:' "$dir/CMakeCache.txt" | cut -d= -f2-)"
    local extra; extra="$(grep -E '^CMAKE_CXX_FLAGS:' "$dir/CMakeCache.txt" | cut -d= -f2-)"
    [ -n "$extra" ] && echo "extra flags: $extra"
  } > "$dir/VERSION"
  # Reference harness programs (scripts/ref-harness), linked against the static library.
  if [ -f "$REPO/scripts/ref-harness/CMakeLists.txt" ]; then
    # Always from scratch: the build directory may have been configured from another checkout
    # (a worktree), which CMake refuses to reuse.
    rm -rf "$dir/harness"
    # shellcheck disable=SC2046
    cmake -S "$REPO/scripts/ref-harness" -B "$dir/harness" -DCMAKE_BUILD_TYPE=Release \
      -DFDK_AAC_SRC="$SRC" -DFDK_AAC_LIB="$dir/libfdk-aac.a" \
      -DCMAKE_C_COMPILER="$cc" -DCMAKE_CXX_COMPILER="$cxx" $(cmake_arch_args "$name")
    cmake --build "$dir/harness" -j"$(jobs)"
  fi
}

if [ $skip_build -eq 0 ]; then
  for b in "${builds[@]}"; do build_one "$b"; done
fi

[ $skip_derived -eq 1 ] && { echo "reference set up in $WORK"; exit 0; }

# --- Resolved sources ----------------------------------------------------------------------

# Macros that unifdef resolves. Everything else (FUNCTION_xxx overrides, feature switches) is
# left alone: when a function has an arm/ or x86/ variant, read the .ii file.
COMMON_UNDEF=(-U__ANDROID__ -U__gnu_linux__ -U__mips__ -U__mips -U__mips_dsp -U__mips16
  -U__SDE_MIPS__ -U__powerpc__ -U_ARCH_PPC -U__riscv -U__s390x__ -U_MSC_VER -U_M_ARM -U_M_ARM64
  -U_M_IX86 -U_M_X64 -U__TARGET_ARCH_ARM -U__TARGET_FEATURE_DSPMUL -U__TARGET_ARCH_7_A
  -U__TARGET_ARCH_7_R -U__TARGET_ARCH_7_M -U__TARGET_ARCH_7E_M -U__ARM_ARCH_5TE__ -U__ARM_ARCH_6__
  -U__ARM_ARCH_6J__ -U__ARM_ARCH_7_M__ -U__ARM_ARCH_7A__ -U__ARM_ARCH_7E_M__ -U__ARM_ARCH_7EM__
  -U__ARM_ARCH_7M__ -U__ARM_ARCH_7R__ -UFDK_ASSERT_ENABLE -UARCH_PREFER_MULT_16x16
  -UARCH_PREFER_MULT_32x32 -DARCH_PREFER_MULT_32x16 -DSINETABLE_16BIT -DWINDOWTABLE_16BIT
  -DPOW2COEFF_16BIT -DLDCOEFF_16BIT -DQMF_COEFF_16BIT -DNDEBUG -D__GNUC__ -D__clang__)
SYMS_aarch64=(-D__arm__ -D__ARM_ARCH_8__ -D__aarch64__ -U__x86__ -U__x86_64__ -U__i386__
  -U__i486__ -U__i586__ -U__i686__ -D__LP64__ "${COMMON_UNDEF[@]}")
SYMS_x86=(-D__x86__ -D__x86_64__ -U__arm__ -U__ARM_ARCH_8__ -U__aarch64__ -U__i386__ -U__i486__
  -U__i586__ -U__i686__ -D__LP64__ "${COMMON_UNDEF[@]}")
# 32-bit x86: ILP32, so LONG is `long` (a type distinct from INT) and the non-LP64 overloads of
# common_fix.h exist; abs_x86.h (x86-64 only) is not used.
SYMS_x86_32=(-D__x86__ -D__i386__ -D__i686__ -U__x86_64__ -U__arm__ -U__ARM_ARCH_8__
  -U__aarch64__ -U__i486__ -U__i586__ -U__LP64__ "${COMMON_UNDEF[@]}")

resolve() {
  local variant="$1" dst="$WORK/fdk-aac-resolved/$1" syms
  case "$variant" in
    aarch64) syms=("${SYMS_aarch64[@]}") ;;
    x86) syms=("${SYMS_x86[@]}") ;;
    x86_32) syms=("${SYMS_x86_32[@]}") ;;
  esac
  rm -rf "$dst"
  ( cd "$SRC"
    find lib* -type f \( -name '*.cpp' -o -name '*.h' \) | while read -r f; do
      mkdir -p "$dst/$(dirname "$f")"
      unifdef "${syms[@]}" "$f" > "$dst/$f.tmp" || [ $? -le 1 ]
      # Strip the license header (the first comment block of the file).
      awk 'NR==1 && /^\/\* -{5,}/{skip=1} skip && /^-{5,}.*\*\/$/{skip=0; next} !skip' \
        "$dst/$f.tmp" > "$dst/$f"
      rm "$dst/$f.tmp"
    done )
}

# --- Preprocessed sources and disassembly --------------------------------------------------

derive() {
  local name="$1" dir="$WORK/build-ref/$1"
  [ -f "$dir/compile_commands.json" ] || { echo "skip $name: not built" >&2; return; }
  python3 - "$dir" "$SRC" "$WORK/fdk-aac-pp/$name" <<'PY'
import json, os, shlex, subprocess, sys
build, src, out = sys.argv[1:4]
for e in json.load(open(os.path.join(build, 'compile_commands.json'))):
    f = os.path.realpath(e['file'])
    if not f.startswith(os.path.realpath(src) + '/') or not f.endswith('.cpp'):
        continue
    args = e.get('arguments') or shlex.split(e['command'])
    a, skip = [], False
    for x in args:
        if skip:
            skip = False
            continue
        if x in ('-o', '-MF', '-MT', '-MQ'):
            skip = True
            continue
        if x in ('-c', '-MD', '-MMD'):
            continue
        a.append(x)
    rel = os.path.relpath(f, os.path.realpath(src))
    dst = os.path.join(out, rel[:-4] + '.ii')
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    subprocess.run(a + ['-E', '-P', '-o', dst], cwd=e['directory'], check=True)
PY
  local asm="$WORK/fdk-aac-asm/$name"
  rm -rf "$asm"; mkdir -p "$asm"
  find "$dir/CMakeFiles/fdk-aac.dir" -name '*.o' | while read -r o; do
    local rel="${o#"$dir/CMakeFiles/fdk-aac.dir/"}"
    mkdir -p "$asm/$(dirname "$rel")"
    objdump -d -r --no-show-raw-insn -C "$o" > "$asm/${rel%.cpp.o}.s"
  done
}

resolved=" "
for b in "${builds[@]}"; do
  v="$(variant_of "$b")"
  case "$resolved" in *" $v "*) ;; *) resolve "$v"; resolved="$resolved$v " ;; esac
  derive "$b"
done
echo "reference set up in $WORK"
