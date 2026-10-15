#!/bin/bash
# Builds the seed corpora of the fuzz targets into fuzz/corpus/<target>/ (git-ignored) from the
# material in work/ (fuzz/README.md):
#
#   - the conformance streams (work/conformance: ADTS, ADIF, LOAS .ass, MP4 demuxed into ASC and
#     access units) and the real-world ADTS streams (work/rw);
#   - streams of the reference encoder harness (work/build-ref/<build>/harness/fdkenc-ref, or the
#     port's fdk-aac-rs-enc with the same interface if no harness runs on this machine) from the
#     synthetic signals of work/corpus-gen, in every transport (raw and LATM MCP0 with their
#     configuration, LATM MCP1 as access units);
#   - synthetic DRM configurations with ER AAC LC / USAC access units, and encoder parameter
#     programs.
#
# Each seed holds at most MAX_BYTES bytes of bitstream (default 12288). Seeds are named by a hash
# of their content, so reruns add nothing twice.
#
# Usage: fuzz/seeds.sh [MAX_BYTES]
set -euo pipefail
MAX="${1:-12288}"
FUZZ="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$FUZZ/.." && pwd)"
if [ -n "${FDK_AAC_RS_WORK_DIR:-}" ]; then
  WORK="$FDK_AAC_RS_WORK_DIR"
else
  WORK="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")/work"
fi
[ -d "$WORK" ] || { echo "no work directory at $WORK" >&2; exit 1; }
CORPUS="$FUZZ/corpus"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# seedgen (stable toolchain, own target directory; inside the container: CARGO_TARGET_DIR)
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target/fuzz-host}"
CARGO_TARGET_DIR="$TARGET_DIR" cargo build -q --release --manifest-path "$FUZZ/Cargo.toml" --features tools --bin seedgen
SEEDGEN="$TARGET_DIR/release/seedgen"

count() { find "$CORPUS/$1" -type f 2>/dev/null | wc -l | tr -d ' '; }

# --- conformance and real-world streams --------------------------------------------------
find "$WORK/conformance" -name '*.adts' -print0 | xargs -0 -r "$SEEDGEN" stream decode_adts "$CORPUS/decode_adts" "$MAX"
find "$WORK/rw" -name '*.aac' -print0 2>/dev/null | xargs -0 -r "$SEEDGEN" stream decode_adts "$CORPUS/decode_adts" "$MAX"
find "$WORK/conformance" -name '*.adif' -print0 | xargs -0 -r "$SEEDGEN" stream decode_adif "$CORPUS/decode_adif" "$MAX"
find "$WORK/conformance" -name '*.ass' -print0 | xargs -0 -r "$SEEDGEN" stream decode_loas "$CORPUS/decode_loas" "$MAX"
find "$WORK/conformance" -name '*.mp4' -print0 | xargs -0 -r "$SEEDGEN" mp4 "$CORPUS/decode_raw" "$MAX"
find "$WORK/conformance" -name '*.mp4' -print0 | xargs -0 -r "$SEEDGEN" drm "$CORPUS/decode_drm" "$MAX"

# --- encoder streams -----------------------------------------------------------------------
ENC=""
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) builds="ref-arm64" ;;
  Darwin-x86_64) builds="ref-x86_64-mac" ;;
  Linux-x86_64) builds="ref-x86_64" ;;
  *) builds="" ;;
esac
for b in $builds; do
  if [ -x "$WORK/build-ref/$b/harness/fdkenc-ref" ]; then ENC="$WORK/build-ref/$b/harness/fdkenc-ref"; fi
done
if [ -z "$ENC" ]; then
  # The port's encoder CLI has the interface of fdkenc-ref and the same output.
  CARGO_TARGET_DIR="$TARGET_DIR" cargo build -q --release --manifest-path "$REPO/Cargo.toml" -p fdk-aac-rs-cli
  ENC="$TARGET_DIR/release/fdk-aac-rs-enc"
fi
echo "encoder: $ENC"

# name|rate|layout|channels|params
configs=(
  "lc|44100|2|2|AACENC_AOT=2 AACENC_BITRATE=128000"
  "lc_mono|48000|1|1|AACENC_AOT=2 AACENC_BITRATE=64000"
  "lc_51|48000|5.1|6|AACENC_AOT=2 AACENC_BITRATE=320000"
  "lc_vbr|32000|2|2|AACENC_AOT=2 AACENC_BITRATEMODE=4"
  "he|44100|2|2|AACENC_AOT=5 AACENC_BITRATE=64000"
  "he_51|48000|5.1|6|AACENC_AOT=5 AACENC_BITRATE=160000"
  "hev2|48000|2|2|AACENC_AOT=29 AACENC_BITRATE=32000"
  "ld|48000|2|2|AACENC_AOT=23 AACENC_BITRATE=128000 AACENC_GRANULE_LENGTH=512"
  "ld480|32000|1|1|AACENC_AOT=23 AACENC_BITRATE=48000 AACENC_GRANULE_LENGTH=480"
  "eld|48000|2|2|AACENC_AOT=39 AACENC_BITRATE=96000"
  "eld_sbr|48000|2|2|AACENC_AOT=39 AACENC_SBR_MODE=1 AACENC_BITRATE=64000"
  "eldv2|48000|2|2|AACENC_AOT=39 AACENC_CHANNELMODE=128 AACENC_BITRATE=64000"
  "eld256|16000|1|1|AACENC_AOT=39 AACENC_GRANULE_LENGTH=256 AACENC_BITRATE=24000"
  "mpeg2lc|32000|1|1|AACENC_AOT=129 AACENC_BITRATE=48000"
  "mpeg2he|44100|2|2|AACENC_AOT=132 AACENC_BITRATE=64000"
)
signals=(sweep white_noise sine_full impulses)
# transport name|TRANSMUX|extra params
transports=("raw|0|" "adif|1|" "adts|2|" "adts_crc|2|AACENC_PROTECTION=1" "latm_mcp1|6|" "latm_mcp0|7|" "loas|10|" "loas_amv1|10|AACENC_AUDIOMUXVER=1 AACENC_SIGNALING_MODE=2")
k=0
for c in "${configs[@]}"; do
  IFS='|' read -r name rate layout ch params <<<"$c"
  sig="${signals[$((k % ${#signals[@]}))]}"
  k=$((k + 1))
  pcm="$WORK/corpus-gen/syn_${sig}_${rate}_${layout}.s16le"
  [ -f "$pcm" ] || pcm="$WORK/corpus-gen/syn_sweep_${rate}_${layout}.s16le"
  [ -f "$pcm" ] || { echo "skipping $name: no PCM for $rate Hz $layout"; continue; }
  # half a second of input
  head -c $((rate * ch)) "$pcm" >"$TMP/in.s16le"
  for t in "${transports[@]}"; do
    IFS='|' read -r tname tt textra <<<"$t"
    args=(-f s16le --rate "$rate" --channels "$ch" --param AACENC_SAMPLERATE="$rate")
    case "$params" in *AACENC_CHANNELMODE*) ;; *) args+=(--param AACENC_CHANNELMODE="$ch") ;; esac
    for p in $params AACENC_TRANSMUX="$tt" $textra; do args+=(--param "$p"); done
    out="$TMP/$name-$tname"
    "$ENC" "${args[@]}" -o "$out.bs" --au-sizes "$out.sizes" -l "$out.jsonl" "$TMP/in.s16le" >/dev/null 2>&1 || true
    [ -s "$out.bs" ] || continue
    conf="$(sed -n 's/.*"call":"info".*"confBuf":"\([0-9a-f]*\)".*/\1/p' "$out.jsonl" | head -1)"
    case "$tname" in
      raw) "$SEEDGEN" packets decode_raw "$CORPUS/decode_raw" "$MAX" "${conf:--}" "$out.sizes" "$out.bs" ;;
      latm_mcp0) "$SEEDGEN" packets decode_raw_mcp0 "$CORPUS/decode_raw" "$MAX" "${conf:--}" "$out.sizes" "$out.bs" ;;
      latm_mcp1) "$SEEDGEN" packets decode_latm "$CORPUS/decode_latm" "$MAX" - "$out.sizes" "$out.bs" ;;
      adif) "$SEEDGEN" stream decode_adif "$CORPUS/decode_adif" "$MAX" "$out.bs" ;;
      adts*) "$SEEDGEN" stream decode_adts "$CORPUS/decode_adts" "$MAX" "$out.bs"
             "$SEEDGEN" packets decode_adts "$CORPUS/decode_adts" "$MAX" - "$out.sizes" "$out.bs" ;;
      loas*) "$SEEDGEN" stream decode_loas "$CORPUS/decode_loas" "$MAX" "$out.bs" ;;
    esac
  done
done

# --- encoder programs ----------------------------------------------------------------------
"$SEEDGEN" enc "$CORPUS/encode"

for t in decode_adts decode_adif decode_latm decode_loas decode_raw decode_drm encode; do
  echo "$t: $(count "$t") seeds"
done
