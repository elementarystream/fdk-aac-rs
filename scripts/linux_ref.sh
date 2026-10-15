#!/bin/bash
# Runs a command in the Linux x86-64 container (scripts/docker/linux-x86_64.Dockerfile), with the
# repository mounted at the same absolute path, so that the git worktree links under work/ keep
# working. Without arguments it builds ref-x86_64 with scripts/setup_reference.sh. A work directory
# outside the repository (FDK_AAC_RS_WORK_DIR, e.g. the main checkout's work/ when running from a
# worktree) is mounted at its own path as well and passed on.
#
# Cargo runs inside the container use their own target directory (target/linux), so that they do
# not clobber the host's build artifacts.
#
# LINUX_REF_PERF=1 adds the capability PERFMON, so that perf works in the container on hosts that
# restrict it (kernel.perf_event_paranoid > 2) without changing the host. A capability is only
# effective for root, so this mode runs as root and passes the calling user as
# FDK_AAC_RS_OWNER (scripts/perf_profile.py gives its output files back to that user).
#
# Usage: scripts/linux_ref.sh [COMMAND ...]
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
IMAGE=fdk-aac-rs-linux-x86_64
docker build --platform linux/amd64 -q -t "$IMAGE" -f "$REPO/scripts/docker/linux-x86_64.Dockerfile" \
  "$REPO/scripts/docker" >/dev/null
[ $# -eq 0 ] && set -- scripts/setup_reference.sh --skip-clone
mounts=(-v "$REPO:$REPO")
if [ -n "${FDK_AAC_RS_WORK_DIR:-}" ]; then
  WORK="$(cd "$FDK_AAC_RS_WORK_DIR" && pwd -P)"
  case "$WORK/" in "$REPO"/*) ;; *) mounts+=(-v "$WORK:$WORK") ;; esac
  mounts+=(-e "FDK_AAC_RS_WORK_DIR=$WORK")
fi
# Run as the calling user, so that the files written below work/ and target/ belong to them.
user="$(id -u):$(id -g)"
if [ "${LINUX_REF_PERF:-}" = 1 ]; then
  mounts+=(--cap-add PERFMON -e "FDK_AAC_RS_OWNER=$user")
  user=0:0
fi
exec docker run --rm --platform linux/amd64 --user "$user" -e HOME=/tmp "${mounts[@]}" \
  -e CARGO_TARGET_DIR="$REPO/target/linux" -w "$REPO" "$IMAGE" "$@"
