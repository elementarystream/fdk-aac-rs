#!/bin/bash
# One-screen summary of a running fuzz campaign (scripts/fuzz.sh) on a Linux host: per target the
# latest libFuzzer statistics and the findings so far, the campaign time, the CPU use and the
# memory use of the host. Runs on the host (not in the container); reads only.
#
#   scripts/fuzz_status.sh [--duration SECONDS]   SECONDS: the campaign length (default 43200)
#
# Per target: the last status line of fuzz/artifacts/TARGET.log (execs: total executions, exec/s,
# cov: coverage, ft: features, corp: corpus entries, crash/timeout/oom: libFuzzer's counts) and the
# files in fuzz/artifacts/TARGET (crash-, timeout-, oom- findings). A target is "done" when its
# fuzz.sh container has ended.
set -uo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
ART="$REPO/fuzz/artifacts"
DURATION=43200
while [ $# -gt 0 ]; do
  case "$1" in
    --duration) DURATION="$2"; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done
TARGETS=(decode_adts decode_adif decode_latm decode_loas decode_raw decode_drm encode)

# CPU busy share over 2 seconds (/proc/stat)
cpu_sample() { awk '/^cpu / {idle = $5 + $6; tot = 0; for (i = 2; i <= NF; i++) tot += $i; print idle, tot}' /proc/stat; }
read -r i0 t0 < <(cpu_sample)

# running fuzz containers: those whose command is scripts/fuzz.sh (matched by command, not by image:
# a later scripts/fuzz.sh call rebuilds the image tag, which then names another image ID)
FUZZ_IDS="$(docker ps --no-trunc --format '{{.ID}} {{.Command}}' | awk '/scripts\/fuzz\.sh/ {print $1}')"
declare -A running
oldest=
while read -r id; do
  [ -n "$id" ] || continue
  args="$(docker inspect -f '{{join .Args " "}}' "$id" 2>/dev/null)"
  for t in "${TARGETS[@]}"; do
    case " $args " in *" $t "*) running[$t]=1 ;; esac
  done
  s="$(docker inspect -f '{{.State.StartedAt}}' "$id" 2>/dev/null)"
  s="$(date -d "$s" +%s 2>/dev/null)"
  if [ -n "$s" ] && { [ -z "$oldest" ] || [ "$s" -lt "$oldest" ]; }; then oldest="$s"; fi
done <<< "$FUZZ_IDS"

hm() { printf '%d:%02d' $(($1 / 3600)) $(($1 % 3600 / 60)); }
now="$(date +%s)"
if [ -n "$oldest" ]; then
  el=$((now - oldest)); left=$((DURATION - el)); [ "$left" -lt 0 ] && left=0
  camp="$(hm "$el") of $(hm "$DURATION") elapsed, $(hm "$left") left, ${#running[@]} of ${#TARGETS[@]} targets running"
else
  camp="no fuzz container running"
fi
echo "fuzz status $(date '+%Y-%m-%d %H:%M %Z') - $camp"

printf '%-12s %-7s %12s %7s %6s %7s %6s %-11s %s\n' target state execs exec/s cov ft corp c/t/oom findings
tot_exec=0
for t in "${TARGETS[@]}"; do
  log="$ART/$t.log"
  line="$(grep -a '^#[0-9]' "$log" 2>/dev/null | tail -1)"
  st=done; [ -n "${running[$t]:-}" ] && st=running
  f="$(ls "$ART/$t" 2>/dev/null | grep -cE '^(crash|timeout|oom|leak)-')"
  if [ -n "$line" ]; then
    read -r ex cov ft corp eps cto < <(echo "$line" | awk '{
      for (i = 1; i <= NF; i++) {
        if ($i == "cov:") cov = $(i + 1); if ($i == "ft:") ft = $(i + 1)
        if ($i == "corp:") corp = $(i + 1); if ($i == "exec/s:") eps = $(i + 1)
        if ($i == "oom/timeout/crash:") cto = $(i + 1)
      }
      ex = substr($1, 2); sub(/:$/, "", ex); print ex, cov, ft, corp, eps, cto}')
    # libFuzzer prints oom/timeout/crash; shown as crash/timeout/oom
    IFS=/ read -r o tm c <<< "$cto"
    printf '%-12s %-7s %12s %7s %6s %7s %6s %-11s %s\n' "$t" "$st" "$ex" "$eps" "$cov" "$ft" "$corp" "$c/$tm/$o" "$f"
    tot_exec=$((tot_exec + ex))
  elif [ "$st" = running ]; then
    printf '%-12s %-7s %s\n' "$t" "$st" "(no status line yet: loading the corpus), findings $f"
  else
    printf '%-12s %-7s %s\n' "$t" "$st" "(no status line), findings $f"
  fi
done
echo "total executions: $tot_exec"
grep -l DIFFERENCE "$ART"/*.log 2>/dev/null | sed 's|.*/||; s|\.log$||' | tr '\n' ' ' | sed 's/^./DIFFERENCE reports in: &/; s/ $/\n/'

sleep 2
read -r i1 t1 < <(cpu_sample)
busy="$(awk -v i="$((i1 - i0))" -v t="$((t1 - t0))" 'BEGIN { printf "%.1f", t ? 100 * (t - i) / t : 0 }')"
echo "CPU: $busy% busy of $(nproc) threads, load $(cut -d' ' -f1-3 /proc/loadavg)"

mem="$(awk '/^MemTotal:/ {t = $2} /^MemAvailable:/ {a = $2} END {printf "%.1f of %.1f GiB used (%.1f GiB available)", (t - a) / 1048576, t / 1048576, a / 1048576}' /proc/meminfo)"
fz=
[ -n "$FUZZ_IDS" ] && fz="$(docker stats --no-stream --format '{{.Container}} {{.MemUsage}}' $FUZZ_IDS 2>/dev/null \
  | awk '{v = $2; u = v; sub(/[0-9.]+/, "", u); sub(/[A-Za-z]+$/, "", v);
          f = (u == "GiB") ? 1 : (u == "MiB") ? 1 / 1024 : (u == "KiB") ? 1 / 1048576 : (u == "B") ? 1 / 1073741824 : 0;
          s += v * f} END {if (NR) printf "%.1f", s}')"
echo "Memory: $mem${fz:+, fuzz containers $fz GiB}"
