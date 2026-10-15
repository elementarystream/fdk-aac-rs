#!/bin/bash
# Checks the C ABI of fdk-aac-rs-capi as a drop-in replacement for libfdk-aac
# (PORTING.md, "C ABI"), for each architecture variant:
#
#   1. install: builds the release cdylib and staticlib and installs them with the public
#      headers, fdk-aac.pc and the CMake package files into a staging prefix
#      (libfdk-aac.a, libfdk-aac.2.dylib or libfdk-aac.so.2, as upstream names them);
#   2. exports: the shared library exports exactly the 18 symbols of fdk-aac.sym; the static
#      library defines them and aacDecoder_RawISOBMFFData, which the shared library must not
#      export (as upstream);
#   3. pkg-config: FFmpeg's configure check (check_pkg_config libfdk_aac fdk-aac
#      "fdk-aac/aacenc_lib.h" aacEncOpen) compiles and links, and the same check for
#      aacDecoder_Open; the Name field;
#   4. cmake: find_package(fdk-aac) and the target FDK-AAC::fdk-aac (shared and static), with
#      upstream's aac-enc (run) and test-encode-decode (built);
#   5. C differential test of the encoder: crates/capi/tests/capi_enc_diff.c linked against
#      the port (static and shared) and against the reference library
#      (work/build-ref/<ref>/libfdk-aac.a), with identical output required;
#   6. aac-enc: upstream's aac-enc.c built against the installed port, byte-identical output to
#      the reference aac-enc (work/build-ref/<ref>/aac-enc) for several configurations;
#   7. C differential test of the decoder: crates/capi/tests/capi_dec_diff.c (with
#      scripts/ref-harness/mp4demux.c) linked against the port (static and shared) and against the
#      reference library, with identical output required. It runs its built-in cases and the
#      streams of DEC_CASES (those present); the shared build does not call
#      aacDecoder_RawISOBMFFData, and its output is compared without those lines;
#   8. test-encode-decode: upstream's test-encode-decode.c built against the installed port
#      (shared and static), run by scripts/upstream_test.sh, which compares its output with the
#      published hashes (AArch64 hashes for arm64 builds, x86 hashes otherwise).
#
# Variants: on Apple silicon aarch64-apple-darwin (ref-arm64) and x86_64-apple-darwin
# (ref-x86_64-mac, under Rosetta 2); on x86-64 Linux x86_64-unknown-linux-gnu (ref-x86_64).
# Inputs: Sample01_4.wav (stereo) and its mono version as in scripts/upstream_test.sh, a 5.1 file
# made from work/corpus/c51_48k.raw (or a synthetic one) with ffmpeg, and the decoder streams of
# DEC_CASES from work/ (conformance streams and local real-world streams; missing ones are
# skipped and listed); nothing is committed.
#
# Usage: scripts/capi_test.sh [-j N] [VARIANT...]   (VARIANT: a Rust target triple)
set -uo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
if [ -n "${FDK_AAC_RS_WORK_DIR:-}" ]; then
  WORK="$FDK_AAC_RS_WORK_DIR"
elif [ -d "$REPO/work" ]; then
  WORK="$REPO/work"
else
  # A worktree uses the main checkout's work/.
  WORK="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")/work"
fi
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target}"
OUT="$TARGET_DIR/capi-test"
SRC="$WORK/fdk-aac-v2.0.3"
CAPI="$REPO/crates/capi"
jobs=2
if [ "${1:-}" = "-j" ]; then jobs="$2"; shift 2; fi

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) all_variants=(aarch64-apple-darwin x86_64-apple-darwin) ;;
  Linux-x86_64) all_variants=(x86_64-unknown-linux-gnu) ;;
  *) echo "unsupported host" >&2; exit 2 ;;
esac
if [ $# -gt 0 ]; then variants=("$@"); else variants=("${all_variants[@]}"); fi

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi; }
md5() { if command -v md5sum >/dev/null; then md5sum "$1" | awk '{print $1}'; else md5 -q "$1"; fi; }

status=0
summary=()
report() { # NAME RESULT DETAIL
  summary+=("$1: $2${3:+ ($3)}")
  [ "$2" = ok ] || status=1
}

# --- inputs ---------------------------------------------------------------------------------
INPUTS="$WORK/upstream-test"
mkdir -p "$INPUTS" "$OUT"
if [ ! -f "$INPUTS/Sample01_4.wav" ]; then
  curl -L -o "$INPUTS/Sample01_4.wav" https://media.xiph.org/audio/HA_2011/Sample01_4.wav
fi
[ "$(md5 "$INPUTS/Sample01_4.wav")" = a5c105544c64ce92c6c5c06d280e6b9c ] || { echo "bad Sample01_4.wav" >&2; exit 1; }
if [ ! -f "$INPUTS/Sample01_4-mono.wav" ]; then
  ffmpeg -loglevel error -i "$INPUTS/Sample01_4.wav" -ac 1 -fflags +bitexact -y "$INPUTS/Sample01_4-mono.wav"
fi
[ "$(md5 "$INPUTS/Sample01_4-mono.wav")" = 3bfccac9f2e527ba3ef888874f09a409 ] || { echo "bad Sample01_4-mono.wav" >&2; exit 1; }
C51="$OUT/c51.wav"
if [ ! -f "$C51" ]; then
  if [ -f "$WORK/corpus/c51_48k.raw" ]; then
    ffmpeg -loglevel error -f s16le -ar 48000 -ac 6 -i "$WORK/corpus/c51_48k.raw" -t 8 \
      -fflags +bitexact -y "$C51"
  else
    ffmpeg -loglevel error -f lavfi -i "sine=f=220:d=8:r=48000" -f lavfi -i "sine=f=330:d=8:r=48000" \
      -f lavfi -i "sine=f=440:d=8:r=48000" -f lavfi -i "sine=f=55:d=8:r=48000" \
      -f lavfi -i "anoisesrc=d=8:r=48000:a=0.1" -f lavfi -i "sine=f=660:d=8:r=48000" \
      -filter_complex "amerge=inputs=6" -c:a pcm_s16le -fflags +bitexact -y "$C51"
  fi
fi

# aac-enc configurations: NAME|OPTIONS|INPUT (stereo, mono, c51). aac-enc always writes ADTS,
# which LD and ELD do not support: those configurations check that both fail alike.
AAC_ENC_CASES=(
  "lc-64k|-t 2 -r 64000|stereo"
  "lc-128k-noab|-t 2 -r 128000 -a 0|stereo"
  "he-48k|-t 5 -r 48000|stereo"
  "hev2-32k|-t 29 -r 32000|stereo"
  "ld-adts-init-error|-t 23 -r 64000|stereo"
  "eld-sbr-adts-init-error|-t 39 -s 1 -r 48000|mono"
  "vbr3|-v 3|stereo"
  "vbr5|-v 5|stereo"
  "mpeg2-lc-96k|-t 129 -r 96000|stereo"
  "mpeg2-he-48k|-t 132 -r 48000|stereo"
  "mono-lc-48k|-t 2 -r 48000|mono"
  "mono-he-32k|-t 5 -r 32000|mono"
  "c51-lc-320k|-t 2 -r 320000|c51"
  "c51-he-160k|-t 5 -r 160000|c51"
  "c51-vbr4|-v 4|c51"
)

# capi_dec_diff cases: NAME|TRANSPORT|FILE|OPTIONS (see crates/capi/tests/capi_dec_diff.c).
# {c51} is the 5.1 ADTS stream of the aac-enc step (ref-c51-lc-320k.aac).
CONF="$WORK/conformance"
DEC_CASES=(
  "lc-adts|adts|$WORK/rw/apple_lc256.aac|chunk=1000:333:4096:17,anc=64"
  "lc-adts-small-out|adts|$WORK/rw/apple_lc256.aac|out=1000,frames=40"
  "lc-loas|loas|$CONF/14496-26/compressedAss/add-opt/al14_12_amv1.ass|chunk=700:5000"
  "he-adts|adts|$WORK/rw/apple_he64.aac|param=4:1,param=12:1"
  "hev2-adts|adts|$WORK/rw/apple_hev2_32.aac|chunk=2500:100,param=100:2"
  "eld-sbr|mp4|$CONF/14496-26/compressedMp4/er_eld_sbr_s_2_48.mp4"
  "eldv2|mp4|$CONF/14496-26/compressedMp4/er_eld_ldmps_16khz.mp4|param=11:2"
  "usac-esbr|mp4|$CONF/23003-7/compressedMp4/eSbr_1_c3_Pvc_0x03.mp4"
  "usac-mps212|mp4|$CONF/23003-7/compressedMp4/Mps_2_c3_fr1_Sc1_0x03.mp4|param=12:1"
  "usac-drc|mp4|$CONF/23003-7/compressedMp4/Ext_2_c1_LnDrc1_0x03.mp4|param=202:64,param=206:3"
  "c51-downmix|adts|{c51}|param=12:2,param=20:1"
  "c51-mpeg-order|adts|{c51}|param=3:0,chunk=1200"
  "lc-adts-corrupt|adts|$WORK/rw/apple_lc256.aac|corrupt=7,chunk=3000"
  "hev2-adts-corrupt|adts|$WORK/rw/apple_hev2_32.aac|corrupt=11"
  "usac-corrupt|mp4|$CONF/23003-7/compressedMp4/Mps_2_c3_fr1_Sc1_0x03.mp4|corrupt=5"
  "eld-corrupt|mp4|$CONF/14496-26/compressedMp4/er_eld_sbr_s_2_48.mp4|corrupt=3"
)

for triple in "${variants[@]}"; do
  case "$triple" in
    aarch64-apple-darwin) arch=arm64; ref=ref-arm64; dylib=libfdk-aac.2.dylib ;;
    x86_64-apple-darwin) arch=x86_64; ref=ref-x86_64-mac; dylib=libfdk-aac.2.dylib ;;
    x86_64-unknown-linux-gnu) arch=; ref=ref-x86_64; dylib=libfdk-aac.so.2 ;;
    *) echo "unknown variant $triple" >&2; exit 2 ;;
  esac
  CC=(cc -O2)
  [ -n "$arch" ] && CC+=(-arch "$arch")
  V="$OUT/$triple"
  PREFIX="$V/prefix"
  rm -rf "$V"
  mkdir -p "$V"
  echo "== $triple (reference $ref)"

  # 1. Build and install.
  if ! cargo build -q -j "$jobs" --release -p fdk-aac-rs-capi --target "$triple"; then
    report "$triple build" FAILED; continue
  fi
  LIB="$TARGET_DIR/$triple/release"
  case "$triple" in
    *-apple-darwin) native_libs="-lSystem -lc -lm"; rust_dylib=libfdk_aac.dylib ;;
    *) native_libs="-lgcc_s -lutil -lrt -lpthread -lm -ldl -lc"; rust_dylib=libfdk_aac.so ;;
  esac
  mkdir -p "$PREFIX/include/fdk-aac" "$PREFIX/lib/pkgconfig" "$PREFIX/lib/cmake/fdk-aac"
  cp "$CAPI"/include/fdk-aac/*.h "$PREFIX/include/fdk-aac/"
  cp "$LIB/libfdk_aac.a" "$PREFIX/lib/libfdk-aac.a"
  cp "$LIB/$rust_dylib" "$PREFIX/lib/$dylib"
  case "$dylib" in
    *.dylib) ln -s "$dylib" "$PREFIX/lib/libfdk-aac.dylib" ;;
    *) ln -s "$dylib" "$PREFIX/lib/libfdk-aac.so" ;;
  esac
  sed -e "s|@prefix@|$PREFIX|" -e 's|@exec_prefix@|${prefix}|' -e 's|@libdir@|${exec_prefix}/lib|' \
    -e 's|@includedir@|${prefix}/include|' -e 's|@PACKAGE_VERSION@|2.0.3|' \
    -e 's|@LIBS_PUBLIC@||' -e "s|@LIBS_PRIVATE@|$native_libs|" \
    "$CAPI/fdk-aac.pc.in" > "$PREFIX/lib/pkgconfig/fdk-aac.pc"
  sed -e 's|@PACKAGE_VERSION@|2.0.3|' "$CAPI/fdk-aac-config-version.cmake.in" \
    > "$PREFIX/lib/cmake/fdk-aac/fdk-aac-config-version.cmake"
  config_for() { # TYPE FILE SONAME LINK
    sed -e "s|@FDK_AAC_LIBRARY_TYPE@|$1|g" -e "s|@FDK_AAC_LIBRARY@|lib/$2|g" \
      -e "s|@FDK_AAC_SONAME@|$3|g" -e "s|@FDK_AAC_LINK_LIBRARIES@|$4|g" \
      "$CAPI/fdk-aac-config.cmake.in"
  }
  case "$dylib" in
    *.dylib) soname="@rpath/$dylib" ;;
    *) soname="$dylib" ;;
  esac
  config_for SHARED "$dylib" "$soname" "" > "$PREFIX/lib/cmake/fdk-aac/fdk-aac-config.cmake"
  report "$triple install" ok "$PREFIX"

  # 2. Exports.
  if [ -n "$arch" ]; then
    nm -gU "$PREFIX/lib/$dylib" | awk '{print $3}' | sed 's/^_//' | sort > "$V/exports.txt"
    install_name="$(otool -D "$PREFIX/lib/$dylib" | tail -1)"
  else
    nm -D --defined-only "$PREFIX/lib/$dylib" | awk '{print $3}' \
      | grep -v -E '^(_init|_fini|_edata|_end|__bss_start)$' | sort > "$V/exports.txt"
    install_name="$(objdump -p "$PREFIX/lib/$dylib" | awk '/SONAME/ {print $2}')"
  fi
  sort "$CAPI/fdk-aac.sym" > "$V/expected-exports.txt"
  if [ -n "$arch" ]; then
    nm -gU "$PREFIX/lib/libfdk-aac.a" 2>/dev/null | awk 'NF == 3 {print $3}' | sed 's/^_//' \
      | grep -E '^aac(Enc|Decoder)' | sort -u > "$V/static-symbols.txt"
  else
    nm -g --defined-only "$PREFIX/lib/libfdk-aac.a" 2>/dev/null | awk 'NF == 3 {print $3}' \
      | grep -E '^aac(Enc|Decoder)' | sort -u > "$V/static-symbols.txt"
  fi
  { cat "$V/expected-exports.txt"; echo aacDecoder_RawISOBMFFData; } | sort > "$V/expected-static.txt"
  if cmp -s "$V/exports.txt" "$V/expected-exports.txt" \
    && cmp -s "$V/static-symbols.txt" "$V/expected-static.txt"; then
    report "$triple exports" ok "$(wc -l < "$V/exports.txt" | tr -d ' ') symbols in the shared library ($install_name), $(wc -l < "$V/static-symbols.txt" | tr -d ' ') API symbols in the static library (with aacDecoder_RawISOBMFFData)"
  else
    report "$triple exports" FAILED "see $V/exports.txt, $V/static-symbols.txt"
  fi

  # 3. pkg-config: FFmpeg's check_pkg_config.
  export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig"
  cat > "$V/ffmpeg_check.c" <<'EOF'
#include <stdint.h>
#include <fdk-aac/aacenc_lib.h>
long check_aacEncOpen(void) { return (long) aacEncOpen; }
int main(void) { int ret = 0;
 ret |= ((intptr_t)check_aacEncOpen) & 0xFFFF;
return 0; }
EOF
  sed -e 's|aacenc_lib.h|aacdecoder_lib.h|' -e 's|aacEncOpen|aacDecoder_Open|g' "$V/ffmpeg_check.c" \
    > "$V/ffmpeg_check_dec.c"
  name="$(grep '^Name:' "$PREFIX/lib/pkgconfig/fdk-aac.pc")"
  # shellcheck disable=SC2046
  if pkg-config --exists fdk-aac \
    && "${CC[@]}" $(pkg-config --cflags fdk-aac) "$V/ffmpeg_check.c" -o "$V/ffmpeg_check" \
      $(pkg-config --libs fdk-aac) -Wl,-rpath,"$PREFIX/lib" \
    && "$V/ffmpeg_check" \
    && "${CC[@]}" $(pkg-config --cflags fdk-aac) "$V/ffmpeg_check.c" -o "$V/ffmpeg_check_static" \
      "$PREFIX/lib/libfdk-aac.a" $(pkg-config --static --libs-only-l fdk-aac | sed 's/-lfdk-aac//') \
    && "$V/ffmpeg_check_static" \
    && "${CC[@]}" $(pkg-config --cflags fdk-aac) "$V/ffmpeg_check_dec.c" -o "$V/ffmpeg_check_dec" \
      $(pkg-config --libs fdk-aac) -Wl,-rpath,"$PREFIX/lib" \
    && "$V/ffmpeg_check_dec" \
    && [ "$name" = "Name: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android" ]; then
    report "$triple pkg-config" ok "version $(pkg-config --modversion fdk-aac)"
  else
    report "$triple pkg-config" FAILED
  fi

  # Upstream programs include "libAACenc/include/aacenc_lib.h": they are copied (unchanged) out
  # of the source tree, and a tree of links to the installed headers takes its place.
  mkdir -p "$V/srcinc/libAACenc/include" "$V/srcinc/libAACdec/include" "$V/srcinc/libSYS/include" \
    "$V/upstream"
  for h in "$PREFIX"/include/fdk-aac/*.h; do
    for d in libAACenc libAACdec libSYS; do ln -s "$h" "$V/srcinc/$d/include/"; done
  done
  cp "$SRC/aac-enc.c" "$SRC/test-encode-decode.c" "$SRC/wavreader.c" "$SRC/wavreader.h" \
    "$SRC/sha1.c" "$SRC/sha1.h" "$V/upstream/"

  # 4. CMake package (shared, then static).
  mkdir -p "$V/cmake-src"
  cat > "$V/cmake-src/CMakeLists.txt" <<EOF
cmake_minimum_required(VERSION 3.10)
project(capi_cmake_check C)
find_package(fdk-aac 2.0 CONFIG REQUIRED)
add_executable(aac-enc "$V/upstream/aac-enc.c" "$V/upstream/wavreader.c")
target_include_directories(aac-enc PRIVATE "$V/srcinc")
target_link_libraries(aac-enc PRIVATE FDK-AAC::fdk-aac)
add_executable(test-encode-decode "$V/upstream/test-encode-decode.c" "$V/upstream/wavreader.c"
  "$V/upstream/sha1.c")
target_include_directories(test-encode-decode PRIVATE "$V/srcinc/libAACenc/include")
target_link_libraries(test-encode-decode PRIVATE FDK-AAC::fdk-aac)
EOF
  cmake_ok=1
  for kind in shared static; do
    if [ $kind = static ]; then
      config_for STATIC libfdk-aac.a "" "$native_libs" > "$PREFIX/lib/cmake/fdk-aac/fdk-aac-config.cmake"
    fi
    cmake_args=(-S "$V/cmake-src" -B "$V/cmake-$kind" -DCMAKE_PREFIX_PATH="$PREFIX" -DCMAKE_BUILD_TYPE=Release)
    [ -n "$arch" ] && cmake_args+=(-DCMAKE_OSX_ARCHITECTURES="$arch")
    if ! { cmake "${cmake_args[@]}" >"$V/cmake-$kind.log" 2>&1 \
      && cmake --build "$V/cmake-$kind" >>"$V/cmake-$kind.log" 2>&1 \
      && "$V/cmake-$kind/aac-enc" -r 64000 "$INPUTS/Sample01_4-mono.wav" "$V/cmake-$kind.aac" 2>>"$V/cmake-$kind.log"; }; then
      cmake_ok=0
    fi
  done
  config_for SHARED "$dylib" "$soname" "" > "$PREFIX/lib/cmake/fdk-aac/fdk-aac-config.cmake"
  if [ $cmake_ok = 1 ] && cmp -s "$V/cmake-shared.aac" "$V/cmake-static.aac"; then
    report "$triple cmake" ok "shared and static FDK-AAC::fdk-aac (aac-enc, test-encode-decode)"
  else
    report "$triple cmake" FAILED "see $V/cmake-*.log"
  fi

  # 5. C differential test of the encoder.
  DIFF_C="$CAPI/tests/capi_enc_diff.c"
  # shellcheck disable=SC2046
  if "${CC[@]}" -DFDK_AAC_RS_PORT -I "$PREFIX/include/fdk-aac" "$DIFF_C" -o "$V/diff-port-static" \
      "$PREFIX/lib/libfdk-aac.a" $native_libs \
    && "${CC[@]}" -DFDK_AAC_RS_PORT -I "$PREFIX/include/fdk-aac" "$DIFF_C" -o "$V/diff-port-shared" \
      $(pkg-config --libs fdk-aac) -Wl,-rpath,"$PREFIX/lib" \
    && "${CC[@]}" -I "$SRC/libAACenc/include" -I "$SRC/libSYS/include" "$DIFF_C" \
      -o "$V/diff-ref" "$WORK/build-ref/$ref/libfdk-aac.a" -lm; then
    "$V/diff-ref" > "$V/diff-ref.log"; r1=$?
    "$V/diff-port-static" > "$V/diff-port-static.log"; r2=$?
    "$V/diff-port-shared" > "$V/diff-port-shared.log"; r3=$?
    if [ $r1 = 0 ] && [ $r2 = 0 ] && [ $r3 = 0 ] && cmp -s "$V/diff-ref.log" "$V/diff-port-static.log" \
      && cmp -s "$V/diff-ref.log" "$V/diff-port-shared.log"; then
      report "$triple C differential (encoder)" ok "$(grep -c '^case ' "$V/diff-ref.log") cases, $(grep -c '^encode ' "$V/diff-ref.log") encode calls, $(wc -l < "$V/diff-ref.log" | tr -d ' ') lines identical, sha256 $(sha256 "$V/diff-ref.log" | cut -c1-16)"
    else
      report "$triple C differential (encoder)" FAILED "exit $r1/$r2/$r3; diff $V/diff-ref.log $V/diff-port-static.log"
      diff "$V/diff-ref.log" "$V/diff-port-static.log" | head -20
    fi
  else
    report "$triple C differential (encoder)" FAILED "build"
  fi

  # 6. aac-enc against the installed port, compared with the reference aac-enc.
  # shellcheck disable=SC2046
  if "${CC[@]}" -I "$V/srcinc" "$V/upstream/aac-enc.c" "$V/upstream/wavreader.c" -o "$V/aac-enc" \
      $(pkg-config --libs fdk-aac) -Wl,-rpath,"$PREFIX/lib"; then
    n=0; bad=0
    for c in "${AAC_ENC_CASES[@]}"; do
      cname="${c%%|*}"; rest="${c#*|}"; opts="${rest%%|*}"; input="${rest#*|}"
      case "$input" in
        stereo) in="$INPUTS/Sample01_4.wav" ;;
        mono) in="$INPUTS/Sample01_4-mono.wav" ;;
        c51) in="$C51" ;;
      esac
      # shellcheck disable=SC2086
      "$WORK/build-ref/$ref/aac-enc" $opts "$in" "$V/ref-$cname.aac" 2>"$V/ref-$cname.err"; e1=$?
      # shellcheck disable=SC2086
      "$V/aac-enc" $opts "$in" "$V/port-$cname.aac" 2>"$V/port-$cname.err"; e2=$?
      n=$((n + 1))
      if [ $e1 = 0 ] && [ $e2 = 0 ] && cmp -s "$V/ref-$cname.aac" "$V/port-$cname.aac"; then
        echo "  aac-enc $cname: ok $(wc -c < "$V/port-$cname.aac" | tr -d ' ') bytes $(sha256 "$V/port-$cname.aac" | cut -c1-16)"
      elif [ $e1 != 0 ] && [ $e1 = $e2 ] && cmp -s "$V/ref-$cname.err" "$V/port-$cname.err"; then
        echo "  aac-enc $cname: ok, both exit $e1: $(head -1 "$V/port-$cname.err")"
      else
        echo "  aac-enc $cname: MISMATCH (exit $e1/$e2)"; bad=$((bad + 1))
      fi
    done
    if [ $bad = 0 ]; then report "$triple aac-enc" ok "$n configurations byte-identical"; else report "$triple aac-enc" FAILED "$bad of $n differ"; fi
  else
    report "$triple aac-enc" FAILED "build"
  fi

  # 7. C differential test of the decoder (after step 6, which makes the 5.1 stream).
  DEC_C="$CAPI/tests/capi_dec_diff.c"
  MP4="$REPO/scripts/ref-harness/mp4demux.c"
  dec_args=(--builtin)
  missing=()
  for c in "${DEC_CASES[@]}"; do
    c="${c//\{c51\}/$V/ref-c51-lc-320k.aac}"
    f="$(echo "$c" | cut -d'|' -f3)"
    if [ -f "$f" ]; then dec_args+=("$c"); else missing+=("${c%%|*}"); fi
  done
  # shellcheck disable=SC2046
  if "${CC[@]}" -DWITH_RAW_ISOBMFF -I "$PREFIX/include/fdk-aac" -I "$REPO/scripts/ref-harness" \
      "$DEC_C" "$MP4" -o "$V/dec-diff-port-static" "$PREFIX/lib/libfdk-aac.a" $native_libs \
    && "${CC[@]}" -I "$PREFIX/include/fdk-aac" -I "$REPO/scripts/ref-harness" "$DEC_C" "$MP4" \
      -o "$V/dec-diff-port-shared" $(pkg-config --libs fdk-aac) -Wl,-rpath,"$PREFIX/lib" \
    && "${CC[@]}" -DWITH_RAW_ISOBMFF -I "$SRC/libAACdec/include" -I "$SRC/libAACenc/include" \
      -I "$SRC/libSYS/include" -I "$REPO/scripts/ref-harness" "$DEC_C" "$MP4" -o "$V/dec-diff-ref" \
      "$WORK/build-ref/$ref/libfdk-aac.a" -lm; then
    "$V/dec-diff-ref" "${dec_args[@]}" > "$V/dec-diff-ref.log"; r1=$?
    "$V/dec-diff-port-static" "${dec_args[@]}" > "$V/dec-diff-port-static.log"; r2=$?
    "$V/dec-diff-port-shared" "${dec_args[@]}" > "$V/dec-diff-port-shared.log"; r3=$?
    grep -v '^isobmff' "$V/dec-diff-ref.log" > "$V/dec-diff-ref-shared.log"
    if [ $r1 = 0 ] && [ $r2 = 0 ] && [ $r3 = 0 ] \
      && cmp -s "$V/dec-diff-ref.log" "$V/dec-diff-port-static.log" \
      && cmp -s "$V/dec-diff-ref-shared.log" "$V/dec-diff-port-shared.log"; then
      report "$triple C differential (decoder)" ok "$(grep -c '^case ' "$V/dec-diff-ref.log") cases ($((${#dec_args[@]} - 1)) streams), $(grep -c '^decode ' "$V/dec-diff-ref.log") decode calls, $(grep -c '^fill ' "$V/dec-diff-ref.log") fill calls, $(grep -c '^isobmff [0-9]' "$V/dec-diff-ref.log") RawISOBMFFData calls (static), $(wc -l < "$V/dec-diff-ref.log" | tr -d ' ') lines identical, sha256 $(sha256 "$V/dec-diff-ref.log" | cut -c1-16)${missing[*]:+; missing streams skipped: ${missing[*]}}"
    else
      report "$triple C differential (decoder)" FAILED "exit $r1/$r2/$r3; diff $V/dec-diff-ref.log $V/dec-diff-port-static.log"
      diff "$V/dec-diff-ref.log" "$V/dec-diff-port-static.log" | head -20
    fi
  else
    report "$triple C differential (decoder)" FAILED "build"
  fi

  # 8. Upstream's test-encode-decode against the installed port (shared and static). It does not
  # drain the decoder at the end, so a pipelined decoder (more than one decoder thread, which
  # returns every frame one call later) would lose the last frame: the decoder runs with one
  # thread here (FDK_AAC_RS_DEC_THREADS=1); the encoder keeps the thread count of the environment.
  ted_ok=1
  ted=()
  ted_note=""
  if [ -n "${FDK_AAC_RS_THREADS:-}${FDK_AAC_RS_DEC_THREADS:-}" ]; then
    ted_note=" (decoder with FDK_AAC_RS_DEC_THREADS=1: upstream's program does not drain)"
  fi
  for kind in shared static; do
    if [ $kind = shared ]; then
      # shellcheck disable=SC2207
      link=($(pkg-config --libs fdk-aac) -Wl,-rpath,"$PREFIX/lib")
    else
      # shellcheck disable=SC2206
      link=("$PREFIX/lib/libfdk-aac.a" $native_libs)
    fi
    if "${CC[@]}" -I "$PREFIX/include/fdk-aac" "$V/upstream/test-encode-decode.c" \
        "$V/upstream/wavreader.c" "$V/upstream/sha1.c" -o "$V/test-encode-decode-$kind" "${link[@]}" \
      && FDK_AAC_RS_DEC_THREADS=1 FDK_AAC_RS_WORK_DIR="$WORK" "$REPO/scripts/upstream_test.sh" "$ref" \
        "$V/test-encode-decode-$kind" > "$V/test-encode-decode-$kind.log" 2>&1; then
      ted+=("$kind: $(grep -E ': (OK|MISMATCH)$' "$V/test-encode-decode-$kind.log" | tr '\n' ' ')")
    else
      ted_ok=0
      ted+=("$kind: FAILED, see $V/test-encode-decode-$kind.log")
    fi
  done
  sed 's/^/  /' "$V/test-encode-decode-shared.log" | grep -E 'OK$|MISMATCH|failures'
  if [ $ted_ok = 1 ]; then
    report "$triple test-encode-decode" ok "${ted[*]}$ted_note"
  else
    report "$triple test-encode-decode" FAILED "${ted[*]}"
  fi
  unset PKG_CONFIG_PATH
done

printf '%s\n' "${summary[@]}"
exit $status
