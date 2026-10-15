#!/bin/bash
# Runs the differential tests in the profiles release and checked (PORTING.md, "Running the
# checks").
# On Apple silicon the default is the aarch64 variant only; the x86 variants are checked on an
# x86-64 Linux host (scripts/linux_check.sh), where the default is the native x86 variant. The x86
# variant under Rosetta 2 remains available with --targets x86_64-apple-darwin.
#
# The test binaries of a profile are built first (cargo test --no-run) and then run concurrently,
# at most --test-jobs N at once (default 2; 0: all at once), each with its own output; the outputs
# are collected in the order of the binaries. A binary that fails or crashes fails the run (all
# binaries always run, as with cargo test --no-fail-fast).
#
# --no-corpus runs the tests with a work directory without work/corpus (the git-ignored local
# audio): a directory of symbolic links to everything else in work/ (the reference sources and the
# cref build cache are shared), with its own cargo target directory (target/no-corpus). Tests that
# read the corpus must then pass with their synthetic fallbacks, as on a machine without it.
#
# --targets LIST (comma separated Rust targets, "native" for the host) replaces the default
# targets. The x86_32 variant (against the ref-i686 code, x87) runs on an x86-64 Linux host,
# e.g. in the container:
#
#   scripts/linux_ref.sh scripts/cref_test.sh -j 3 --targets i686-unknown-linux-gnu
#
# --profiles LIST (comma separated, default release,checked) selects the profiles. --log FILE
# writes the complete output (build and test binaries) to FILE (scripts/linux_check.sh uses it).
#
# Usage: scripts/cref_test.sh [-j N] [--test-jobs N] [--targets LIST] [--profiles LIST] [--log FILE]
#                           [--no-corpus] [CARGO TEST ARGS...] [-- TEST BINARY ARGS...]
#   e.g. scripts/cref_test.sh --test fdk -- fixmul::
set -uo pipefail
jobs=3
test_jobs=2
targets_opt=""
profiles=(release checked)
log_file=""
no_corpus=0
while [ $# -gt 0 ]; do
  case "$1" in
    -j) jobs="$2"; shift 2 ;;
    --test-jobs) test_jobs="$2"; shift 2 ;;
    --targets) targets_opt="$2"; shift 2 ;;
    --profiles) IFS=, read -r -a profiles <<< "$2"; shift 2 ;;
    --log) log_file="$2"; shift 2 ;;
    --no-corpus) no_corpus=1; shift ;;
    *) break ;;
  esac
done
# The remaining arguments: cargo test arguments, then (after --) the arguments of the binaries.
cargo_args=()
test_args=()
while [ $# -gt 0 ]; do
  if [ "$1" = -- ]; then
    shift
    test_args=("$@")
    break
  fi
  cargo_args+=("$1")
  shift
done
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) targets=(aarch64-apple-darwin) ;;
  *) targets=("") ;;
esac
if [ -n "$targets_opt" ]; then
  IFS=, read -r -a targets <<< "$targets_opt"
  for i in "${!targets[@]}"; do [ "${targets[$i]}" = native ] && targets[$i]=""; done
fi
repo="$(cd "$(dirname "$0")/.." && pwd)"
if [ $no_corpus = 1 ]; then
  work="$(cd "${FDK_AAC_RS_WORK_DIR:-$repo/work}" && pwd -P)"
  nc="$(mktemp -d "${TMPDIR:-/tmp}/fdk-aac-rs-no-corpus.XXXXXX")"
  trap 'rm -rf "$nc"' EXIT
  for e in "$work"/* "$work"/.[!.]*; do
    [ -e "$e" ] || continue
    [ "$(basename "$e")" = corpus ] && continue
    ln -s "$e" "$nc/$(basename "$e")"
  done
  export FDK_AAC_RS_WORK_DIR="$nc"
  export CARGO_TARGET_DIR="$repo/target/no-corpus"
fi
[ -n "$log_file" ] && : > "$log_file"

# Builds and runs the tests of one target ($1, "" for the host) and profile ($2); the output goes
# to the file $3. Fails if the build or a test binary fails.
run_tests() {
  local t="$1" p="$2" log="$3" dir status=0 bins=() i b s
  local args=(test --no-run --no-fail-fast -j "$jobs" --profile "$p" -p fdk-aac-rs-cref --message-format=json-render-diagnostics)
  [ -n "$t" ] && args+=(--target "$t")
  dir="$(mktemp -d "${TMPDIR:-/tmp}/fdk-aac-rs-cref.XXXXXX")"
  # JSON messages on stdout (the executables), the rendered diagnostics on stderr
  cargo "${args[@]}" ${cargo_args[@]+"${cargo_args[@]}"} > "$dir/build.json" 2> "$log" || status=1
  # the executables, in the order of their names (as cargo test runs them)
  while IFS= read -r b; do
    bins+=("$b")
  done < <(grep -o '"executable":"[^"]*"' "$dir/build.json" | sed 's/^"executable":"//; s/"$//' |
    awk '{n = $0; sub(/.*\//, "", n); sub(/-[0-9a-f]*$/, "", n); print n "\t" $0}' | sort | cut -f2)
  i=0
  for b in ${bins[@]+"${bins[@]}"}; do
    if [ "$test_jobs" -gt 0 ]; then
      while [ "$(jobs -pr | wc -l)" -ge "$test_jobs" ]; do sleep 0.1; done
    fi
    # as cargo test runs them: in the package directory
    (
      cd "$repo/crates/cref" &&
        CARGO_MANIFEST_DIR="$repo/crates/cref" "$b" ${test_args[@]+"${test_args[@]}"} > "$dir/$i.log" 2>&1
      echo $? > "$dir/$i.status"
    ) &
    i=$((i + 1))
  done
  wait
  i=0
  for b in ${bins[@]+"${bins[@]}"}; do
    s="$(cat "$dir/$i.status" 2>/dev/null || echo 255)"
    {
      echo "     Running $b"
      cat "$dir/$i.log"
      if [ "$s" != 0 ]; then
        echo "error: test failed: process didn't exit successfully: \`$b\` (exit status: $s$([ "$s" -gt 128 ] && echo ", signal $((s - 128))"))"
      fi
    } >> "$log"
    [ "$s" = 0 ] || status=1
    i=$((i + 1))
  done
  rm -rf "$dir"
  return $status
}

status=0
summary=()
for t in "${targets[@]}"; do
  for p in "${profiles[@]}"; do
    log="$(mktemp)"
    if run_tests "$t" "$p" "$log"; then r=ok; else r=FAILED; status=1; fi
    counts="$(grep -E '^test result:' "$log" | awk '{p+=$4; f+=$6} END {printf "%d passed, %d failed", p, f}')"
    summary+=("${t:-native} $p: $r ($counts)")
    [ $r = FAILED ] && { echo "--- ${t:-native} $p"; grep -E 'FAILED|panicked|error(\[|:)|^(DIFF|CRASH) #|^FAIL: ' "$log" | head -40; }
    [ -n "$log_file" ] && cat "$log" >> "$log_file"
    rm -f "$log"
  done
done
printf '%s\n' "${summary[@]}"
exit $status
