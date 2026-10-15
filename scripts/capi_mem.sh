#!/bin/bash
# Heap use of the decoder and encoder instances, fdk-aac-rs-capi against the reference library
# (scripts/capi_mem.c: counts every live malloc block through the C API). Linux only (it replaces
# glibc's malloc); run it in the container:
#
#   scripts/linux_ref.sh scripts/capi_mem.sh [--i686] [--threads LIST]
#
# The inputs are those of scripts/bench.py (work/bench; run bench.py once first). For each case it
# prints the heap in use after the open (the instance), the peak while processing the whole
# input and what is left after the close (one-time allocations of the library included), and
# what is left after a second open, process and close in the same process (a leak per instance),
# in KiB, for the reference and for the port at every thread count of LIST (default 1;
# FDK_AAC_RS_DEC_THREADS / FDK_AAC_RS_ENC_THREADS).
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${FDK_AAC_RS_WORK_DIR:-$REPO/work}"
SRC="$WORK/fdk-aac-v2.0.3"
BENCH="$WORK/bench"
triple=x86_64-unknown-linux-gnu ref=ref-x86_64 m=
threads=1
while [ $# -gt 0 ]; do
  case "$1" in
    --i686) triple=i686-unknown-linux-gnu ref=ref-i686 m=-m32 ;;
    --threads) threads="$2"; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target/linux}"
OUT="$TARGET_DIR/capi-mem/$triple"
mkdir -p "$OUT"
cargo build -q --release -p fdk-aac-rs-capi --target "$triple" --target-dir "$TARGET_DIR"
CC=(cc -O2 $m)
"${CC[@]}" -I "$REPO/crates/capi/include/fdk-aac" "$REPO/scripts/capi_mem.c" -o "$OUT/mem-port" \
  "$TARGET_DIR/$triple/release/libfdk_aac.a" -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc
"${CC[@]}" -I "$SRC/libAACdec/include" -I "$SRC/libAACenc/include" -I "$SRC/libSYS/include" \
  "$REPO/scripts/capi_mem.c" -o "$OUT/mem-ref" "$WORK/build-ref/$ref/libfdk-aac.a" -lm

# name | arguments
CASES=(
  "dec lc128|dec adts $BENCH/lc128.adts"
  "dec he64|dec adts $BENCH/he64.adts"
  "dec hev2-32|dec adts $BENCH/hev2-32.adts"
  "dec ld128|dec loas $BENCH/ld128.loas"
  "dec eld-sbr64|dec loas $BENCH/eld-sbr64.loas"
  "dec lc384-5.1|dec adts $BENCH/lc384-5.1.adts"
  "enc lc128|enc 2 2 128000 2 1 -1 $BENCH/stereo.s16le"
  "enc he64|enc 2 5 64000 2 1 -1 $BENCH/stereo.s16le"
  "enc hev2-32|enc 2 29 32000 2 1 -1 $BENCH/stereo.s16le"
  "enc ld128|enc 2 23 128000 10 1 -1 $BENCH/stereo.s16le"
  "enc eld-sbr64|enc 2 39 64000 10 1 1 $BENCH/stereo.s16le"
  "enc lc384-5.1|enc 6 2 384000 2 1 -1 $BENCH/5.1.s16le"
)

kib() { echo "$1" | awk '{printf "%.1f/%.1f/%.1f/%.1f", $2 / 1024, $4 / 1024, $6 / 1024, $10 / 1024}'; }
IFS=, read -ra TH <<< "$threads"
printf '%-16s %-26s' "case" "ref open/peak/closed/leak"
for t in "${TH[@]}"; do printf ' %-26s' "port -j $t"; done
echo
for c in "${CASES[@]}"; do
  name="${c%%|*}"
  read -ra args <<< "${c#*|}"
  r="$("$OUT/mem-ref" "${args[@]}")"
  printf '%-16s %-26s' "$name" "$(kib "$r")"
  for t in "${TH[@]}"; do
    p="$(FDK_AAC_RS_DEC_THREADS=$t FDK_AAC_RS_ENC_THREADS=$t "$OUT/mem-port" "${args[@]}")"
    [ "$(echo "$p" | awk '{print $8}')" = "$(echo "$r" | awk '{print $8}')" ] \
      || echo "frame count differs: ref $r, port $p" >&2
    printf ' %-26s' "$(kib "$p")"
  done
  echo
done
