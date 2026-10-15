#!/bin/bash
# Runs a differential fuzz target (fuzz/; PORTING.md, "Differential fuzzing") in the fuzz container
# (scripts/docker/fuzz.Dockerfile), so that nothing has to be installed on the host.
#
#   scripts/fuzz.sh TARGET [SECONDS] [JOBS]   fuzz TARGET for SECONDS (default 600) with JOBS
#                                           processes (default 4; libFuzzer fork mode), then
#                                           minimize the new findings and replay them
#   scripts/fuzz.sh all [SECONDS] [JOBS]      every target in turn, SECONDS each
#   scripts/fuzz.sh exec COMMAND...           run a command in the container (from fuzz/)
#
# TARGET: decode_adts decode_adif decode_latm decode_loas decode_raw decode_drm encode
#
# The repository is mounted at the same absolute path (with the main checkout, whose work/ holds
# the reference sources and the cref build cache), the container runs as the calling user, and
# cargo uses target/fuzz (target/fuzz-arm64 for the arm64 image). The corpus is fuzz/corpus/TARGET
# (build it with fuzz/seeds.sh), findings go to fuzz/artifacts/TARGET; both are git-ignored.
#
# Environment:
#   FUZZ_PLATFORM   linux/amd64 (default: the x86 reference variant) or linux/arm64 (the AArch64
#                   variant; native on Apple silicon)
#   FUZZ_MAX_LEN    libFuzzer -max_len (default 16384; encode: 8192)
#   FUZZ_TIMEOUT    libFuzzer -timeout in seconds for one input (default 60)
#   FUZZ_REF_BUDGET_MS  CPU budget of the reference part of one input (default 10000; an
#                   input whose reference part needs more is skipped)
#   FUZZ_TMIN       0: do not minimize findings (default 1)
#   FUZZ_CHECKED    1: build the port with debug assertions and overflow checks (as the checked
#                   profile; target/fuzz*-checked); an overflow panic then shows arithmetic that
#                   relies on wrap-around. Default: release semantics (cargo fuzz -O).
#   FUZZ_ARGS       more libFuzzer arguments
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
PLATFORM="${FUZZ_PLATFORM:-linux/amd64}"
case "$PLATFORM" in
  linux/amd64) SUFFIX="" ;;
  linux/arm64) SUFFIX="-arm64" ;;
  *) echo "unsupported FUZZ_PLATFORM $PLATFORM" >&2; exit 2 ;;
esac
IMAGE="fdk-aac-rs-fuzz$SUFFIX"
TARGETS=(decode_adts decode_adif decode_latm decode_loas decode_raw decode_drm encode)

[ $# -ge 1 ] || { sed -n '2,30p' "$0"; exit 2; }

if [ "${FDK_AAC_RS_FUZZ_IN_CONTAINER:-}" != 1 ]; then
  docker build --platform "$PLATFORM" -q -t "$IMAGE" -f "$REPO/scripts/docker/fuzz.Dockerfile" \
    "$REPO/scripts/docker" >/dev/null
  MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
  mounts=(-v "$MAIN:$MAIN")
  case "$REPO/" in "$MAIN"/*) ;; *) mounts+=(-v "$REPO:$REPO") ;; esac
  if [ -n "${FDK_AAC_RS_WORK_DIR:-}" ]; then
    WORK="$(cd "$FDK_AAC_RS_WORK_DIR" && pwd -P)"
    case "$WORK/" in "$MAIN"/*|"$REPO"/*) ;; *) mounts+=(-v "$WORK:$WORK") ;; esac
    mounts+=(-e "FDK_AAC_RS_WORK_DIR=$WORK")
  fi
  # crates.io downloads persist in the target directory
  REG="$REPO/target/fuzz$SUFFIX/cargo-registry"
  mkdir -p "$REG"
  mounts+=(-v "$REG:/opt/cargo/registry")
  tty=()
  [ -t 1 ] && tty=(-t)
  exec docker run --rm ${tty[@]+"${tty[@]}"} --init --platform "$PLATFORM" --user "$(id -u):$(id -g)" -e HOME=/tmp \
    "${mounts[@]}" -e FDK_AAC_RS_FUZZ_IN_CONTAINER=1 -e CARGO_TARGET_DIR="$REPO/target/fuzz$SUFFIX" \
    -e FDK_FUZZ_REF_BUDGET_MS="${FUZZ_REF_BUDGET_MS:-10000}" \
    -e FUZZ_MAX_LEN -e FUZZ_TIMEOUT -e FUZZ_TMIN -e FUZZ_CHECKED -e FUZZ_ARGS -e FUZZ_PLATFORM="$PLATFORM" \
    -w "$REPO/fuzz" "$IMAGE" "$REPO/scripts/fuzz.sh" "$@"
fi

# --- inside the container ------------------------------------------------------------------
cd "$REPO/fuzz"
# cargo fuzz build options: release semantics (-O: no debug assertions, no overflow checks), no
# sanitizer (the port is safe Rust; the reference is not instrumented).
FZ=(-O -s none)
if [ "${FUZZ_CHECKED:-0}" = 1 ]; then
  FZ=(-O -a -s none)
  export CARGO_TARGET_DIR="$CARGO_TARGET_DIR-checked"
fi
if [ "$1" = exec ]; then
  shift
  exec "$@"
fi
TARGET="$1"
SECONDS_PER="${2:-600}"
JOBS="${3:-4}"
if [ "$TARGET" = all ]; then
  for t in "${TARGETS[@]}"; do "$0" "$t" "$SECONDS_PER" "$JOBS"; done
  exit 0
fi
case " ${TARGETS[*]} " in *" $TARGET "*) ;; *) echo "unknown target $TARGET" >&2; exit 2 ;; esac

if [ "$TARGET" = encode ]; then MAXLEN="${FUZZ_MAX_LEN:-8192}"; else MAXLEN="${FUZZ_MAX_LEN:-16384}"; fi
ART="$REPO/fuzz/artifacts/$TARGET"
mkdir -p "$ART" "$REPO/fuzz/corpus/$TARGET"
before="$(ls "$ART" | sort)"
CARGO_BUILD_JOBS="$JOBS" cargo fuzz build "${FZ[@]}" "$TARGET"
cargo build -q --release -j "$JOBS" --features tools --bin replay
echo "== $TARGET: $SECONDS_PER s, $JOBS jobs, $PLATFORM${FUZZ_CHECKED:+ (checked)}"
# Fork mode: the run continues after findings; libFuzzer writes them to $ART.
# shellcheck disable=SC2086
cargo fuzz run "${FZ[@]}" "$TARGET" "$REPO/fuzz/corpus/$TARGET" -- \
  -fork="$JOBS" -ignore_crashes=1 -ignore_timeouts=1 -ignore_ooms=1 \
  -max_total_time="$SECONDS_PER" -max_len="$MAXLEN" -timeout="${FUZZ_TIMEOUT:-60}" \
  -rss_limit_mb=4096 -print_final_stats=1 -artifact_prefix="$ART/" ${FUZZ_ARGS:-} \
  2>&1 | tee "$ART/../$TARGET.log" | grep --line-buffered -E '^#|INFO: -fork|DONE|CRASH|ERROR|DIFFERENCE|panicked|Test unit written|stat::' || true

new="$(comm -13 <(echo "$before") <(ls "$ART" | sort) | grep -E '^(crash|timeout|oom|leak)-' || true)"
if [ -z "$new" ]; then
  echo "== $TARGET: no new findings"
  exit 0
fi
echo "== $TARGET: new findings:"
echo "$new"
for f in $new; do
  if [ "${FUZZ_TMIN:-1}" = 1 ] && [[ "$f" == crash-* ]]; then
    cargo fuzz tmin "${FZ[@]}" -r 2000 "$TARGET" "$ART/$f" >"$ART/$f.tmin.log" 2>&1 || true
  fi
done
# Replay every new finding (and its minimized form): the report names the first differing call.
for f in $new; do
  m="$ART/minimized-from-${f#crash-}"
  for g in "$ART/$f" "$m"; do
    [ -f "$g" ] || continue
    echo "== replay $g"
    "$CARGO_TARGET_DIR/release/replay" "$TARGET" "$g" || true
  done
done
