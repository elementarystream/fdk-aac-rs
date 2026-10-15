#!/bin/bash
# Runs upstream's test-encode-decode (test/run-test.sh of fdk-aac) with a reference build and
# compares its output with the published hashes:
#
#   x86 builds:      test/ref-{stereo,mono}.txt of v2.0.3
#   AArch64 builds:  test/ref-{stereo,mono}-aarch64.txt of upstream master (added after v2.0.3;
#                    the codec did not change, see PORTING.md, "Version")
#
# The input files are downloaded into work/upstream-test/ and checked against the MD5 sums of
# run-test.sh. The mono file is made with ffmpeg, as run-test.sh does.
#
# Usage: scripts/upstream_test.sh BUILD [TEST_ENCODE_DECODE]
#   BUILD is a directory name below work/build-ref (ref-arm64, ref-x86_64-mac, ref-x86_64).
#   TEST_ENCODE_DECODE overrides the program (e.g. one linked against fdk-aac-rs-capi).
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${FDK_AAC_RS_WORK_DIR:-$REPO/work}"
BUILD="$1"
PROG="${2:-$WORK/build-ref/$BUILD/test-encode-decode}"
DIR="$WORK/upstream-test"
mkdir -p "$DIR"

md5() { if command -v md5sum >/dev/null; then md5sum "$1" | awk '{print $1}'; else md5 -q "$1"; fi; }

if [ ! -f "$DIR/Sample01_4.wav" ]; then
  curl -L -o "$DIR/Sample01_4.wav" https://media.xiph.org/audio/HA_2011/Sample01_4.wav
fi
[ "$(md5 "$DIR/Sample01_4.wav")" = a5c105544c64ce92c6c5c06d280e6b9c ] || { echo "bad Sample01_4.wav" >&2; exit 1; }
if [ ! -f "$DIR/Sample01_4-mono.wav" ]; then
  ffmpeg -loglevel error -i "$DIR/Sample01_4.wav" -ac 1 -fflags +bitexact -y "$DIR/Sample01_4-mono.wav"
fi
[ "$(md5 "$DIR/Sample01_4-mono.wav")" = 3bfccac9f2e527ba3ef888874f09a409 ] || { echo "bad Sample01_4-mono.wav" >&2; exit 1; }

case "$BUILD" in
  ref-arm64*) suffix="-aarch64" ;;
  *) suffix="" ;;
esac
for ch in stereo mono; do
  ref="$DIR/ref-$ch$suffix.txt"
  git -C "$WORK/fdk-aac" show "origin/master:test/ref-$ch$suffix.txt" > "$ref"
  in="$DIR/Sample01_4.wav"; [ $ch = mono ] && in="$DIR/Sample01_4-mono.wav"
  # test-encode-decode writes its temporary files into the current directory.
  out="$(cd "$DIR" && "$PROG" "$in")"
  if [ "$out" = "$(cat "$ref")" ]; then
    echo "$BUILD $ch: OK"
  else
    echo "$BUILD $ch: MISMATCH"; echo "got:"; echo "$out"; echo "expected:"; cat "$ref"
    status=1
  fi
done
exit "${status:-0}"
