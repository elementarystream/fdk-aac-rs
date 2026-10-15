#!/bin/bash
# Replays fuzz inputs (findings in fuzz/artifacts/<target>/, or any corpus file) and prints what
# each does: match, a stopped reference call or a reference without a defined result (both
# skipped), or a DIFFERENCE with the first differing call; a port panic shows its message
# (fuzz/README.md).
#
#   fuzz/replay.sh [--docker] [--trace] [TARGET|auto] FILE|DIR...
#
# TARGET defaults to `auto` (taken from the path: fuzz/artifacts/<target>/...).
# --docker  replay in the fuzz container (scripts/fuzz.sh; FUZZ_PLATFORM selects the variant, by
#           default linux/amd64 = the x86 reference); without it, replay natively on this host
#           (its own reference variant: findings of the x86 container need --docker on an arm64
#           host).
# --trace   also print both call traces.
set -euo pipefail
FUZZ="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$FUZZ/.." && pwd)"
docker=0
args=()
for a in "$@"; do
  case "$a" in
    --docker) docker=1 ;;
    *) args+=("$a") ;;
  esac
done
[ ${#args[@]} -ge 1 ] || { sed -n '2,15p' "$0"; exit 2; }
case "${args[0]}" in
  --trace) first="${args[1]:-}" ;;
  *) first="${args[0]}" ;;
esac
# Without a target name, infer it from the paths.
case "$first" in
  decode_*|encode|auto) ;;
  *)
    if [ "${args[0]}" = --trace ]; then args=(--trace auto "${args[@]:1}"); else args=(auto "${args[@]}"); fi
    ;;
esac
# Absolute paths (the container sees the repository at the same path).
for i in "${!args[@]}"; do
  if [ -e "${args[$i]}" ]; then args[$i]="$(cd "$(dirname "${args[$i]}")" && pwd)/$(basename "${args[$i]}")"; fi
done
if [ "$docker" = 1 ]; then
  exec "$REPO/scripts/fuzz.sh" exec bash -c \
    'cargo build -q --release --features tools --bin replay && exec "$CARGO_TARGET_DIR/release/replay" "$@"' \
    replay "${args[@]}"
fi
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target/fuzz-host}"
CARGO_TARGET_DIR="$TARGET_DIR" cargo build -q --release --manifest-path "$FUZZ/Cargo.toml" --features tools --bin replay
exec "$TARGET_DIR/release/replay" "${args[@]}"
