#!/bin/bash
# The checks of the x86 variant on Linux x86-64, the authoritative reference platform
# (PORTING.md, "Reference builds"). Run it inside the Linux container: scripts/linux_ref.sh
# scripts/linux_check.sh [JOBS]. It needs work/ prepared by scripts/setup_reference.sh (ref-x86_64),
# scripts/fetch_conformance.py --usac and scripts/corpus.py gen.
#
#   1. the reference harness: it writes work/sums/SHA256SUMS.ref-x86_64 if the file is missing,
#      and otherwise checks that it reproduces it;
#   2. the cref tests of the port in the release and checked profiles;
#   3. the Rust CLIs against work/sums/SHA256SUMS.ref-x86_64 (after check 1 if that wrote it).
#
# scripts/linux_check.sh [JOBS] --i686 runs the same checks for the x86_32 variant: ref-i686
# (built first with scripts/setup_reference.sh --skip-clone --build ref-i686 if it is missing)
# with work/sums/SHA256SUMS.ref-i686, and the cref tests and the Rust CLIs for the target
# i686-unknown-linux-gnu.
#
# scripts/linux_check.sh [JOBS] --v3 runs the checks of the x86 variant with the optional x86-64-v3
# build of the port (RUSTFLAGS='-C target-cpu=x86-64-v3': AVX2, BMI1/2, LZCNT, FMA and MOVBE as
# the baseline, so that the compiler vectorizes the portable code with AVX2 as well; the reference
# and its cref build are unchanged). The CLIs are built in $CARGO_TARGET_DIR/x86-64-v3, the cref
# tests in their own target directories; the CPU must support x86-64-v3.
#
# JOBS (default 8) is the number of jobs of each cargo build and of each end-to-end run.
set -uo pipefail
JOBS=8
BUILD=ref-x86_64
TGT=()
CREF_TGT=native
OUT=""
TAG=linux
V3=0
for a in "$@"; do
  case "$a" in
    --i686)
      BUILD=ref-i686
      TGT=(--target i686-unknown-linux-gnu)
      CREF_TGT=i686-unknown-linux-gnu
      OUT=i686-unknown-linux-gnu/
      TAG=linux-i686
      ;;
    --v3) V3=1 ;;
    *) JOBS="$a" ;;
  esac
done
REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO"
WORK="${FDK_AAC_RS_WORK_DIR:-$REPO/work}"
TARGET="${CARGO_TARGET_DIR:-$REPO/target}"
CLI_TARGET="$TARGET"
if [ $V3 = 1 ]; then
  if [ "$BUILD" = ref-i686 ]; then
    echo "--v3 is an x86-64 build; it cannot be combined with --i686" >&2
    exit 2
  fi
  for f in avx2 bmi1 bmi2 abm fma movbe; do
    grep -qw "$f" /proc/cpuinfo || { echo "--v3: the CPU lacks $f (x86-64-v3)" >&2; exit 2; }
  done
  # RUSTFLAGS applies to every Rust compilation of these builds (also the build scripts, which
  # run on this CPU); it does not change the C++ compiler flags of the cref build.
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-cpu=x86-64-v3"
  TAG=linux-v3
  CLI_TARGET="$TARGET/x86-64-v3"
fi
mkdir -p "$TARGET"
status=0
if [ "$BUILD" = ref-i686 ] && [ ! -x "$WORK/build-ref/ref-i686/harness/fdkdec-ref" ]; then
  scripts/setup_reference.sh --skip-clone --build ref-i686 > "$TARGET/$TAG-setup.log" 2>&1 || status=1
fi
# The four checks are independent; they run concurrently, each with its own log, and the summary
# is printed in a fixed order. The cref tests of each profile (and of each variant: x86_64 and
# --i686 may run at the same time) build in their own target directory, so that the builds do
# not wait for each other's cargo lock; scripts/cref_test.sh then runs all test binaries of a
# profile at once. Only when the hash file does not exist yet, the Rust CLIs are run after the
# reference harness has written it.
SUMS="$WORK/sums/SHA256SUMS.$BUILD"
have_sums=0
[ -f "$SUMS" ] && have_sums=1
rust_e2e() {
  python3 scripts/e2e.py --manifest scripts/corpus.json --build "$BUILD" --impl rust \
    --dec-exe "$CLI_TARGET/${OUT}release/fdk-aac-rs-dec" --enc-exe "$CLI_TARGET/${OUT}release/fdk-aac-rs-enc" \
    --jobs "$JOBS" > "$TARGET/$TAG-rust-e2e.log" 2>&1
}
python3 scripts/e2e.py --manifest scripts/corpus.json --build "$BUILD" --jobs "$JOBS" > "$TARGET/$TAG-ref-e2e.log" 2>&1 &
ref_pid=$!
cref_pids=()
for p in release checked; do
  CARGO_TARGET_DIR="$TARGET/$TAG-cref-$p" scripts/cref_test.sh -j "$JOBS" --test-jobs 0 --targets "$CREF_TGT" \
    --profiles "$p" --log "$TARGET/$TAG-cref-$p.log" > /dev/null 2>&1 &
  cref_pids+=($!)
done
{
  CARGO_TARGET_DIR="$CLI_TARGET" cargo build -j "$JOBS" --release ${TGT[@]+"${TGT[@]}"} -p fdk-aac-rs-cli \
    > "$TARGET/$TAG-cli-build.log" 2>&1 &&
    if [ $have_sums = 1 ]; then rust_e2e; fi
} &
rust_pid=$!
wait "$ref_pid" || status=1
if [ $have_sums = 1 ]; then ref_mode="checked $SUMS"; else ref_mode="wrote $SUMS"; fi
echo "reference e2e ($ref_mode): $(tail -1 "$TARGET/$TAG-ref-e2e.log")"
i=0
for p in release checked; do
  wait "${cref_pids[$i]}" || status=1
  i=$((i + 1))
  log="$TARGET/$TAG-cref-$p.log"
  echo "cref $p: $(grep -E '^test result:' "$log" | awk '{a+=$4; f+=$6} END {printf "%d passed, %d failed", a, f}') ($(grep -cE "^error(\\[|: could not compile)" "$log") build errors, $(grep -c "process didn't exit successfully" "$log") crashed test binaries)"
done
wait "$rust_pid"
rust_status=$?
if [ $rust_status = 0 ] && [ $have_sums = 0 ]; then
  rust_e2e
  rust_status=$?
fi
[ $rust_status = 0 ] || status=1
echo "rust e2e: $(tail -1 "$TARGET/$TAG-rust-e2e.log" 2>/dev/null || echo "CLI build failed, see $TARGET/$TAG-cli-build.log")"
exit $status
