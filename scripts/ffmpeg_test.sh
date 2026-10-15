#!/bin/bash
# FFmpeg verification of fdk-aac-rs-capi (PORTING.md, "C ABI"):
# FFmpeg built against the port must produce the same outputs as FFmpeg built against the
# reference library.
#
# For each architecture variant, this script
#
#   1. fetches the FFmpeg release tarball (FF_VERSION, checked against FF_SHA256) into
#      work/ffmpeg/ and unpacks it there;
#   2. installs three fdk-aac prefixes under work/ffmpeg/<variant>/:
#        ref-prefix          the reference library (cmake --install of work/build-ref/<ref>,
#                            static libfdk-aac.a, as built there),
#        port-prefix         fdk-aac-rs-capi (release build), installed as scripts/capi_test.sh
#                            does: libfdk-aac.a, libfdk-aac.2.dylib (libfdk-aac.so.2 on Linux),
#                            headers, fdk-aac.pc; FFmpeg links the shared library,
#        port-static-prefix  the same without the shared library, so FFmpeg links the static
#                            library;
#   3. builds FFmpeg out of tree against each prefix (build-ref, build-port, build-port-static)
#      with a minimal configuration (CONFIGURE below) found through pkg-config as FFmpeg's own
#      check_pkg_config does (PKG_CONFIG_LIBDIR points at the prefix only, so nothing from the
#      system is picked up). A build is reused when its configure line and the installed
#      library files are unchanged; it is relinked when only the library changed. The linkage
#      is checked (the reference build contains the reference's C++ symbols, the port builds
#      none; the shared port build loads the port's dylib/so);
#   4. runs the same ffmpeg command lines with each build (JOBS parallel jobs) and compares
#      the exit status, the output files (SHA-256) and the log (stderr at log level verbose,
#      without collapsing repeated messages, object addresses masked, the speed line removed,
#      grouped per component as described at normalize_log, since ffmpeg's threads interleave
#      their lines nondeterministically) with the reference build. Raw stderr that differs only
#      in this interleaving is counted but not treated as a difference. A command whose exit
#      status and outputs match but whose logs differ is re-run up to three times with both
#      builds (ffmpeg occasionally splits a line around another thread's line); it counts as
#      identical when a log of one build then equals a log of the other, and the count of such
#      commands is reported.
#
# Command categories:
#   enc-mx       work/mx/*.raw (converted once to WAV with the reference build; read by the
#                wav demuxer) x every configuration of ENC_CONFIGS, ADTS and MP4 (and LATM for
#                some), the libfdk_aac encoder;
#   enc-gen      work/corpus-gen/*.s16le (read with -f s16le, rate and layout from the json
#                sidecar) x two configurations of ENC_CONFIGS each, round robin;
#   dec-al       work/conformance/14496-26/compressedMp4/al* (not als_*: ALS is not AAC);
#   dec-eld      work/conformance/14496-26/compressedMp4/er_eld*;
#   dec-usac     work/conformance/23003-7/compressedMp4/*;
#   dec-rw       work/rw/*.aac (ADTS);
#   dec-fh       work/fraunhofer/*.mp4, *.m4a;
#   dec-enc      the outputs of the reference build's enc-mx and enc-gen commands (ADTS and MP4;
#                LATM is not decodable by FFmpeg's libfdk_aac wrapper);
#   dec-opts     decoder options (DEC_OPTS: -downmix, -drc_*, -level_limit, -album_mode,
#                -conceal) on selected streams;
#   dec-corrupt  work/rw/*.aac with bytes flipped (generated under work/ffmpeg/inputs), with
#                the three concealment methods.
# Every decode writes WAV and s16le outputs from one ffmpeg run. Missing input directories
# are skipped. Outputs of decodes are hashed and deleted (KEEP=1 keeps them); every difference
# is listed and its command is re-run for the reference and the differing build into
# work/ffmpeg/<variant>/diff/ with all outputs kept.
#
# Variants: on Apple silicon aarch64-apple-darwin (ref-arm64) and x86_64-apple-darwin
# (ref-x86_64-mac, FFmpeg built for x86_64 and run under Rosetta 2); on x86-64 Linux
# x86_64-unknown-linux-gnu (ref-x86_64). Nothing here is committed; FFmpeg is never installed.
#
# Usage: scripts/ffmpeg_test.sh [-j N] [--quick] [--noise] [--only CATEGORY[,CATEGORY]] [VARIANT...]
#   --quick  a small subset of every category (every 10th command)
#   --noise  also runs every command a second time with the reference build ("ref2") and
#            compares it like a port build, which shows how deterministic the comparison is
#   --only   only these categories (dec-enc and the encoded streams of dec-opts need the
#            encoder categories in the same run)
set -uo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
SELF="$REPO/scripts/ffmpeg_test.sh"
if [ -n "${FDK_AAC_RS_WORK_DIR:-}" ]; then
  WORK="$FDK_AAC_RS_WORK_DIR"
elif [ -d "$REPO/work" ]; then
  WORK="$REPO/work"
else
  # A worktree uses the main checkout's work/.
  WORK="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")/work"
fi
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target}"
FFW="$WORK/ffmpeg"
FF_VERSION=8.1.3
FF_SHA256=7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3
FF_URL="https://ffmpeg.org/releases/ffmpeg-$FF_VERSION.tar.xz"
FF_SRC="$FFW/ffmpeg-$FF_VERSION"
FF_BUILDS=(ref port port-static)

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi; }
sha256_stdin() { if command -v sha256sum >/dev/null; then sha256sum | awk '{print $1}'; else shasum -a 256 | awk '{print $1}'; fi; }

# Masks object addresses and drops the final progress line (speed and time vary).
filter_log() {
  sed -E -e 's/@ 0x[0-9a-f]+/@ PTR/g' -e '/ speed=|elapsed=/d'
}

# The compared log. ffmpeg runs demuxing, decoding, filtering, encoding and muxing in separate
# threads, so the order of lines from different components varies from run to run. The lines
# are grouped by their context prefix ("[aist#0:0/aac @ PTR] [dec:libfdk_aac @ PTR]", ...;
# indented unprefixed lines such as the stream information are grouped by the preceding
# "Input #N", "Output #N" or "Stream mapping:" line, other unprefixed lines form one group),
# keeping the order within each group. Thread
# start/stop/EOF notices are dropped, and when ffmpeg failed (exit status $1 non-zero) also
# the input and I/O statistics, which depend on how far the demuxer got before the failure.
normalize_log() {
  local failed=1
  [ "$1" = 0 ] && failed=0
  awk -v failed="$failed" '
    /Starting thread|Terminating thread|thread received EOF|returned EOF/ { next }
    failed && (/^\[(in#[0-9]+\/[^]]*|AVIOContext @ PTR)\] / || /All consumers .*are done/) { next }
    /^(Input|Output) #[0-9]+|^Stream mapping:/ { block = $0 }
    { key = ""; rest = $0
      while (match(rest, /^\[[^]]*\] ?/)) { key = key substr(rest, 1, RLENGTH); rest = substr(rest, RLENGTH + 1) }
      if (key == "") key = (/^ / || $0 == block) ? "~" block : "~~"
      printf "%s\t%08d\t%s\n", key, NR, $0 }' | sort -t "$(printf '\t')" -k1,1 -k2,2n | cut -f3-
}

# --- job mode: __job VARIANT_DIR KEEP OUTROOT BUILD LINE ---------------------------------------
# LINE: ID<TAB>CATEGORY<TAB>ARGS (ffmpeg arguments, separated by spaces; @REFRUN@ stands for
# the reference build's run directory). Runs ffmpeg of build-BUILD (BUILD ref2: build-ref
# again) in OUTROOT/BUILD/ID and writes its result there:
# ID<TAB>CATEGORY<TAB>EXIT<TAB>LOG_SHA (log.txt, see normalize_log)<TAB>STDERR_SHA<TAB>OUTPUTS
if [ "${1:-}" = __job ]; then
  V="$2"; keep="$3"; b="$5"; outdir="$4/$b"; line="$6"
  IFS=$'\t' read -r id cat args <<< "$line"
  args="${args//@REFRUN@/$V/run/ref}"
  dir="$outdir/$id"
  rm -rf "$dir"; mkdir -p "$dir"; cd "$dir" || exit 1
  # shellcheck disable=SC2086
  "$V/build-${b%2}/ffmpeg" -nostdin -hide_banner -nostats -v repeat+verbose $args 2> stderr.raw
  e=$?
  filter_log < stderr.raw > stderr.txt
  normalize_log "$e" < stderr.txt > log.txt
  rm -f stderr.raw
  outs=""
  for f in out.*; do
    [ -f "$f" ] || continue
    outs+="$f=$(sha256 "$f") "
    case "$f" in *.wav|*.s16) [ "$keep" = 1 ] || rm -f "$f" ;; esac
  done
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$id" "$cat" "$e" "$(sha256_stdin < log.txt)" \
    "$(sha256_stdin < stderr.txt)" "${outs% }" > result
  exit 0
fi

# --- retry mode: __retry VARIANT_DIR LINE REF_LOG_SHA BUILD_LOG_SHA BUILD ---------------------------
# Re-runs LINE with the reference build and BUILD (in VARIANT_DIR/retry) up to three times and
# prints its ID when a log of one build equals a log of the other.
if [ "${1:-}" = __retry ]; then
  V="$2"; line="$3"; b="$6"; id="${line%%$'\t'*}"
  refs=("$4"); others=("$5")
  for try in 1 2 3; do
    bash "$0" __job "$V" 0 "$V/retry/$try" ref "$line"
    bash "$0" __job "$V" 0 "$V/retry/$try" "$b" "$line"
    refs+=("$(cut -f4 "$V/retry/$try/ref/$id/result")")
    others+=("$(cut -f4 "$V/retry/$try/$b/$id/result")")
    for r in "${refs[@]}"; do for o in "${others[@]}"; do
      [ "$r" = "$o" ] && { echo "$id"; exit 0; }
    done; done
  done
  exit 0
fi

jobs=4
quick=0
noise=0
only=""
while [ $# -gt 0 ]; do
  case "$1" in
    -j) jobs="$2"; shift 2 ;;
    --quick) quick=1; shift ;;
    --noise) noise=1; shift ;;
    --only) only=",$2,"; shift 2 ;;
    *) break ;;
  esac
done

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) all_variants=(aarch64-apple-darwin x86_64-apple-darwin) ;;
  Linux-x86_64) all_variants=(x86_64-unknown-linux-gnu) ;;
  *) echo "unsupported host" >&2; exit 2 ;;
esac
if [ $# -gt 0 ]; then variants=("$@"); else variants=("${all_variants[@]}"); fi

# Builds whose runs are compared with the reference build's (the first entry).
BUILDS=(ref port port-static)
[ $noise = 1 ] && BUILDS+=(ref2)

want() { [ -z "$only" ] || [[ "$only" == *",$1,"* ]]; }

# --- FFmpeg sources -----------------------------------------------------------------------------
mkdir -p "$FFW"
if [ ! -f "$FFW/ffmpeg-$FF_VERSION.tar.xz" ]; then
  curl -L -o "$FFW/ffmpeg-$FF_VERSION.tar.xz" "$FF_URL" || exit 1
fi
[ "$(sha256 "$FFW/ffmpeg-$FF_VERSION.tar.xz")" = "$FF_SHA256" ] || { echo "bad FFmpeg tarball" >&2; exit 1; }
if [ ! -f "$FF_SRC/configure" ]; then
  tar -C "$FFW" -xf "$FFW/ffmpeg-$FF_VERSION.tar.xz" || exit 1
fi

# Minimal configuration: the libfdk_aac encoder and decoder, PCM s16le, the containers used
# here, the file protocol, the filters ffmpeg itself needs, no autodetected or other external
# libraries, no network.
CONFIGURE=(
  --disable-everything --disable-autodetect --disable-network --disable-doc
  --disable-ffplay --disable-ffprobe --enable-ffmpeg
  --enable-nonfree --enable-libfdk-aac
  --enable-encoder=libfdk_aac,pcm_s16le --enable-decoder=libfdk_aac,pcm_s16le
  --enable-demuxer=aac,loas,mov,wav,pcm_s16le
  --enable-muxer=adts,latm,mp4,wav,pcm_s16le
  --enable-parser=aac,aac_latm --enable-protocol=file,pipe
  --enable-filter=aresample,aformat,anull,atrim
)

# --- inputs ---------------------------------------------------------------------------------------
INPUTS="$FFW/inputs"
mkdir -p "$INPUTS"

# Corrupted copies of the ADTS streams in work/rw: flip bits at a fixed stride after the
# first 4 KiB (xor 0x5a every 1531 bytes; xor 0x01 every 7919 bytes).
for f in "$WORK"/rw/*.aac; do
  [ -f "$f" ] || continue
  n="$(basename "$f" .aac)"
  [ -f "$INPUTS/$n-c1531.aac" ] || perl -e 'local $/; open F, "<", $ARGV[0] or die; binmode F; $d = <F>;
    for ($i = 4096; $i < length $d; $i += 1531) { substr($d, $i, 1) = chr(ord(substr($d, $i, 1)) ^ 0x5a) }
    open O, ">", $ARGV[1] or die; binmode O; print O $d' "$f" "$INPUTS/$n-c1531.aac"
  [ -f "$INPUTS/$n-c7919.aac" ] || perl -e 'local $/; open F, "<", $ARGV[0] or die; binmode F; $d = <F>;
    for ($i = 4096; $i < length $d; $i += 7919) { substr($d, $i, 1) = chr(ord(substr($d, $i, 1)) ^ 0x01) }
    open O, ">", $ARGV[1] or die; binmode O; print O $d' "$f" "$INPUTS/$n-c7919.aac"
done

# Encoder configurations: NAME|CONTAINERS|OPTIONS. Invalid combinations (HE-AAC v2 or ELDv2
# with other than two channels, LD/ELD in ADTS, ...) are kept: both builds must fail alike.
ENC_CONFIGS=(
  "lc128|adts,mp4,latm|-profile:a aac_low -b:a 128k"
  "lc64-noab|adts,mp4|-profile:a aac_low -b:a 64k -afterburner 0"
  "lc320|adts,mp4|-profile:a aac_low -b:a 320k"
  "lc-default|adts,mp4|-profile:a aac_low"
  "lc96-cutoff|adts,mp4|-profile:a aac_low -b:a 96k -cutoff 12000"
  "lc128-960|adts,mp4|-profile:a aac_low -b:a 128k -frame_length 960"
  "lc128-drc|adts,mp4|-profile:a aac_low -b:a 128k -drc_profile 1 -drc_target_ref -20 -prog_ref -23"
  "lc128-etsi|adts,mp4|-profile:a aac_low -b:a 128k -drc_profile 2 -drc_target_ref -16 -comp_profile 3 -comp_target_ref -14 -prog_ref -20"
  "lc128-loas|latm|-profile:a aac_low -b:a 128k -latm 1 -header_period 4"
  "he64|adts,mp4,latm|-profile:a aac_he -b:a 64k"
  "he48-implicit|adts,mp4|-profile:a aac_he -b:a 48k -signaling implicit"
  "he32-explicit-sbr|adts,mp4|-profile:a aac_he -b:a 32k -signaling explicit_sbr"
  "he64-drc|adts,mp4|-profile:a aac_he -b:a 64k -drc_profile 3 -drc_target_ref -18 -prog_ref -20"
  "he48-loas|latm|-profile:a aac_he -b:a 48k -latm 1 -header_period 2"
  "hev2-32|adts,mp4|-profile:a aac_he_v2 -b:a 32k"
  "hev2-24-explicit|adts,mp4|-profile:a aac_he_v2 -b:a 24k -signaling explicit_hierarchical"
  "hev2-default|adts,mp4|-profile:a aac_he_v2"
  "ld128|adts,mp4|-profile:a aac_ld -b:a 128k"
  "ld64-480|mp4|-profile:a aac_ld -b:a 64k -frame_length 480"
  "eld64|adts,mp4|-profile:a aac_eld -b:a 64k"
  "eld96-480|mp4|-profile:a aac_eld -b:a 96k -frame_length 480"
  "eld-sbr48|mp4|-profile:a aac_eld -eld_sbr 1 -b:a 48k"
  "eld-sbr32-480|mp4|-profile:a aac_eld -eld_sbr 1 -b:a 32k -frame_length 480"
  "eld-v2-48|mp4|-profile:a aac_eld -eld_v2 1 -b:a 48k"
  "eld-v2-sbr32|mp4|-profile:a aac_eld -eld_sbr 1 -eld_v2 1 -b:a 32k"
  "vbr1|adts,mp4|-profile:a aac_low -vbr 1"
  "vbr2|adts,mp4|-profile:a aac_low -vbr 2"
  "vbr3|adts,mp4|-profile:a aac_low -vbr 3"
  "vbr4|adts,mp4|-profile:a aac_low -vbr 4"
  "vbr5|adts,mp4|-profile:a aac_low -vbr 5"
  "he-vbr2|adts,mp4|-profile:a aac_he -vbr 2"
  "hev2-vbr1|adts,mp4|-profile:a aac_he_v2 -vbr 1"
  "eld-vbr3|mp4|-profile:a aac_eld -vbr 3"
  "mpeg2-lc96|adts,mp4|-profile:a mpeg2_aac_low -b:a 96k"
  "mpeg2-he48|adts,mp4|-profile:a mpeg2_aac_he -b:a 48k"
)

# Decoder option sets for dec-opts: NAME|OPTIONS.
DEC_OPTS=(
  "dmx-stereo|-downmix stereo"
  "dmx-mono|-downmix mono"
  "drc-full|-drc_level 80 -drc_cut 127 -drc_boost 127"
  "drc-heavy|-drc_heavy 1 -drc_level 124"
  "drc-off|-drc_level -2 -drc_cut 0 -drc_boost 0"
  "unidrc|-drc_effect 6 -album_mode 1 -drc_level 96"
  "nolimit|-level_limit 0"
  "conceal-spectral|-conceal spectral -level_limit 1"
)

ext_of() { case "$1" in adts) echo aac ;; mp4) echo m4a ;; latm) echo latm ;; esac; }

# mx clip format, from the file name.
mx_rate() { case "$1" in *96k*) echo 96000 ;; *44k*) echo 44100 ;; *) echo 48000 ;; esac; }
mx_layout() { case "$1" in c51*) echo 5.1 ;; *mono*) echo mono ;; *) echo stereo ;; esac; }
# corpus-gen layout names (json "layout") to FFmpeg channel layouts accepted by the libfdk_aac
# wrapper; 7.1back (MODE_7_1_BACK) has no FFmpeg equivalent and is fed as 7.1(wide).
gen_layout() {
  case "$1" in
    1) echo mono ;; 2) echo stereo ;; 3) echo 3.0 ;; 4) echo 4.0 ;; 5) echo 5.0 ;;
    5.1) echo 5.1 ;; 6.1) echo "6.1(back)" ;; 7.1|7.1back) echo "7.1(wide)" ;;
    7.1rear) echo 7.1 ;; 7.1top) echo "5.1.2(back)" ;; *) echo unknown ;;
  esac
}
json_field() { sed -n -E "s/^ *\"$2\": \"?([^\",]*)\"?,?\$/\\1/p" "$1" | head -1; }

# Writes the encoder jobs to stdout. $1: WAV directory of the mx clips.
gen_enc_jobs() {
  local wavdir="$1" c name conts opts cont f clip i k n
  if want enc-mx; then
    for f in "$wavdir"/*.wav; do
      [ -f "$f" ] || continue
      clip="$(basename "$f" .wav)"
      for c in "${ENC_CONFIGS[@]}"; do
        name="${c%%|*}"; conts="${c#*|}"; opts="${conts#*|}"; conts="${conts%%|*}"
        for cont in ${conts//,/ }; do
          printf 'enc-mx.%s.%s.%s\tenc-mx\t-i %s -c:a libfdk_aac %s -fflags +bitexact -flags:a +bitexact -f %s out.%s\n' \
            "$clip" "$name" "$cont" "$f" "$opts" "$cont" "$(ext_of "$cont")"
        done
      done
    done
  fi
  if want enc-gen; then
    i=0
    n=${#ENC_CONFIGS[@]}
    for j in "$WORK"/corpus-gen/*.json; do
      [ -f "$j" ] || continue
      f="$WORK/corpus-gen/$(json_field "$j" file)"
      [ -f "$f" ] || continue
      rate="$(json_field "$j" rate)"
      layout="$(gen_layout "$(json_field "$j" layout)")"
      for k in 0 1; do
        c="${ENC_CONFIGS[$(((i * 2 + k) % n))]}"
        name="${c%%|*}"; conts="${c#*|}"; opts="${conts#*|}"; conts="${conts%%|*}"
        IFS=, read -r -a cl <<< "$conts"
        cont="${cl[$(((i + k) % ${#cl[@]}))]}"
        printf 'enc-gen.%s.%s.%s\tenc-gen\t-f s16le -ar %s -ch_layout %s -i %s -c:a libfdk_aac %s -fflags +bitexact -flags:a +bitexact -f %s out.%s\n' \
          "$(basename "$f" .s16le)" "$name" "$cont" "$rate" "$layout" "$f" "$opts" "$cont" "$(ext_of "$cont")"
      done
      i=$((i + 1))
    done
  fi
}

dec_line() { # ID CATEGORY INPUT [DECODER OPTIONS...]
  local id="$1" cat="$2" in="$3"
  shift 3
  printf '%s\t%s\t-c:a libfdk_aac %s-i %s -map 0:a:0 -fflags +bitexact -f wav out.wav -map 0:a:0 -fflags +bitexact -f s16le out.s16\n' \
    "$id" "$cat" "${*:+$* }" "$in"
}

# Writes the decoder jobs to stdout. $1: the reference build's encoder results.
gen_dec_jobs() {
  local encres="$1" f n o name opts C26="$WORK/conformance/14496-26/compressedMp4"
  local C37="$WORK/conformance/23003-7/compressedMp4"
  if want dec-al; then
    for f in "$C26"/al*.mp4; do
      [ -f "$f" ] || continue
      case "$(basename "$f")" in als_*) continue ;; esac
      dec_line "dec-al.$(basename "$f" .mp4)" dec-al "$f"
    done
  fi
  if want dec-eld; then
    for f in "$C26"/er_eld*.mp4; do [ -f "$f" ] && dec_line "dec-eld.$(basename "$f" .mp4)" dec-eld "$f"; done
  fi
  if want dec-usac; then
    for f in "$C37"/*.mp4; do [ -f "$f" ] && dec_line "dec-usac.$(basename "$f" .mp4)" dec-usac "$f"; done
  fi
  if want dec-rw; then
    for f in "$WORK"/rw/*.aac; do [ -f "$f" ] && dec_line "dec-rw.$(basename "$f" .aac)" dec-rw "$f"; done
  fi
  if want dec-fh; then
    for f in "$WORK"/fraunhofer/*.mp4 "$WORK"/fraunhofer/*.m4a; do
      [ -f "$f" ] || continue
      case "$f" in *" "*) continue ;; esac
      n="$(basename "$f")"; dec_line "dec-fh.${n%.*}" dec-fh "$f"
    done
  fi
  if want dec-enc && [ -f "$encres" ]; then
    # Successful encodes of the reference build (ADTS and MP4).
    awk -F'\t' '$3 == 0 && $1 !~ /\.latm$/ {print $1}' "$encres" | while read -r n; do
      o="$(ls "$FFV/run/ref/$n"/out.* 2>/dev/null | head -1)"
      [ -n "$o" ] && [ -s "$o" ] && dec_line "dec-enc.${n#enc-}" dec-enc "@REFRUN@/$n/$(basename "$o")"
    done
  fi
  if want dec-opts; then
    local sel=()
    for f in "$WORK"/rw/*.aac "$WORK"/fraunhofer/*.mp4 "$WORK"/fraunhofer/*.m4a \
      "$C26"/al_sbr_gen_48_*.mp4 "$C26"/al_sbr_cm_48_5.1.mp4 "$C26"/al_sbr_cm_48_5.mp4 \
      "$C26"/al22_*_48.mp4 "$C26"/al_sbr_ps_00.mp4 "$C26"/al05_48.mp4 "$C26"/er_eld_ldmps_16khz.mp4 \
      "$C26"/er_eld_sbr_s_2_48.mp4 "$C37"/Ext_*.mp4 "$C37"/eSbr_2_c3_Mps1_0x03.mp4 \
      "$C37"/Mps_2_c3_fr1_Sc1_0x03.mp4; do
      [ -f "$f" ] || continue
      case "$f" in *" "*) continue ;; esac
      sel+=("$f")
    done
    for f in "${sel[@]}"; do
      n="$(basename "$f")"; n="${n%.*}"
      for o in "${DEC_OPTS[@]}"; do
        name="${o%%|*}"; opts="${o#*|}"
        dec_line "dec-opts.$n.$name" dec-opts "$f" "$opts"
      done
    done
    # Multichannel encodes of the reference build.
    for n in enc-mx.c51_48k.lc128.adts enc-mx.c51_48k.he64.mp4 enc-mx.c51_48k.vbr4.mp4; do
      o="$(ls "$FFV/run/ref/$n"/out.* 2>/dev/null | head -1)"
      [ -n "$o" ] || continue
      for o2 in "${DEC_OPTS[@]}"; do
        name="${o2%%|*}"; opts="${o2#*|}"
        dec_line "dec-opts.${n#enc-}.$name" dec-opts "@REFRUN@/$n/$(basename "$o")" "$opts"
      done
    done
  fi
  if want dec-corrupt; then
    for f in "$INPUTS"/*-c1531.aac "$INPUTS"/*-c7919.aac; do
      [ -f "$f" ] || continue
      for o in spectral noise energy; do
        dec_line "dec-corrupt.$(basename "$f" .aac).$o" dec-corrupt "$f" "-conceal $o"
      done
    done
  fi
}

# Runs JOBFILE with every build (and every job in parallel), then collects the results.
run_jobs() { # JOBFILE
  local jf="$1" b
  for b in "${BUILDS[@]}"; do
    while IFS= read -r l; do printf '%s\0%s\0' "$b" "$l"; done < "$jf"
  done | xargs -0 -P "$jobs" -n 2 bash "$SELF" __job "$FFV" "${KEEP:-0}" "$FFV/run"
}

collect() { # JOBFILE BUILD -> results on stdout, in job order
  local jf="$1" b="$2"
  cut -f1 "$jf" | while read -r id; do
    if [ -f "$FFV/run/$b/$id/result" ]; then cat "$FFV/run/$b/$id/result"
    else printf '%s\t?\tmissing\t-\t-\t-\n' "$id"; fi
  done
}

status=0
summary=()
for triple in "${variants[@]}"; do
  case "$triple" in
    aarch64-apple-darwin) arch=arm64; ref=ref-arm64; dylib=libfdk-aac.2.dylib ;;
    x86_64-apple-darwin) arch=x86_64; ref=ref-x86_64-mac; dylib=libfdk-aac.2.dylib ;;
    x86_64-unknown-linux-gnu) arch=; ref=ref-x86_64; dylib=libfdk-aac.so.2 ;;
    *) echo "unknown variant $triple" >&2; exit 2 ;;
  esac
  FFV="$FFW/$triple"
  mkdir -p "$FFV"
  echo "== $triple (reference $ref)"

  # --- prefixes -----------------------------------------------------------------------------------
  # The reference is installed again only when its library changed: on macOS the install runs
  # ranlib, which gives the installed archive a new checksum each time.
  P="$FFV/ref-prefix"
  refsum="$(sha256 "$WORK/build-ref/$ref/libfdk-aac.a")"
  if [ "$(cat "$P/.source-sha256" 2>/dev/null)" != "$refsum" ]; then
    rm -rf "$P"
    if ! cmake --install "$WORK/build-ref/$ref" --prefix "$P" > "$FFV/ref-install.log" 2>&1; then
      summary+=("$triple: reference install FAILED (see $FFV/ref-install.log)"); status=1; continue
    fi
    sed -i.bak -e "s|^prefix=.*|prefix=$P|" "$P/lib/pkgconfig/fdk-aac.pc" && rm -f "$P/lib/pkgconfig/fdk-aac.pc.bak"
    echo "$refsum" > "$P/.source-sha256"
  fi

  if ! cargo build -q -j "$jobs" --release -p fdk-aac-rs-capi --target "$triple"; then
    summary+=("$triple: port build FAILED"); status=1; continue
  fi
  LIB="$TARGET_DIR/$triple/release"
  case "$triple" in
    *-apple-darwin) native_libs="-lSystem -lc -lm"; rust_dylib=libfdk_aac.dylib ;;
    *) native_libs="-lgcc_s -lutil -lrt -lpthread -lm -ldl -lc"; rust_dylib=libfdk_aac.so ;;
  esac
  for kind in port port-static; do
    P="$FFV/$kind-prefix"
    rm -rf "$P"
    mkdir -p "$P/include/fdk-aac" "$P/lib/pkgconfig"
    cp "$REPO"/crates/capi/include/fdk-aac/*.h "$P/include/fdk-aac/"
    cp "$LIB/libfdk_aac.a" "$P/lib/libfdk-aac.a"
    if [ $kind = port ]; then
      cp "$LIB/$rust_dylib" "$P/lib/$dylib"
      case "$dylib" in
        *.dylib) ln -s "$dylib" "$P/lib/libfdk-aac.dylib" ;;
        *) ln -s "$dylib" "$P/lib/libfdk-aac.so" ;;
      esac
    fi
    sed -e "s|@prefix@|$P|" -e 's|@exec_prefix@|${prefix}|' -e 's|@libdir@|${exec_prefix}/lib|' \
      -e 's|@includedir@|${prefix}/include|' -e 's|@PACKAGE_VERSION@|2.0.3|' \
      -e 's|@LIBS_PUBLIC@||' -e "s|@LIBS_PRIVATE@|$native_libs|" \
      "$REPO/crates/capi/fdk-aac.pc.in" > "$P/lib/pkgconfig/fdk-aac.pc"
  done

  # --- FFmpeg builds ------------------------------------------------------------------------------
  build_ok=1
  for b in "${FF_BUILDS[@]}"; do
    P="$FFV/$b-prefix"
    B="$FFV/build-$b"
    cfg=("${CONFIGURE[@]}")
    case "$triple" in
      x86_64-apple-darwin)
        cfg+=(--enable-cross-compile --arch=x86_64 --target-os=darwin --cc="cc -arch x86_64"
          --disable-x86asm) ;;
      aarch64-apple-darwin) cfg+=(--cc="cc -arch arm64") ;;
      x86_64-unknown-linux-gnu) cfg+=(--cc=clang --cxx=clang++ --disable-x86asm) ;;
    esac
    case "$b" in
      port) cfg+=(--extra-ldflags="-Wl,-rpath,$P/lib") ;;
      *) cfg+=(--pkg-config-flags=--static) ;;
    esac
    cfgline="FFmpeg $FF_VERSION ${cfg[*]}"
    libsum="$(for f in "$P"/lib/libfdk-aac.a "$P/lib/$dylib"; do [ -f "$f" ] && sha256 "$f"; done | tr '\n' ' ')"
    mkdir -p "$B"
    if [ -x "$B/ffmpeg" ] && [ "$(cat "$B/.stamp-configure" 2>/dev/null)" = "$cfgline" ] \
      && [ "$(cat "$B/.stamp-lib" 2>/dev/null)" = "$libsum" ]; then
      echo "  build-$b: reused"
    else
      if [ "$(cat "$B/.stamp-configure" 2>/dev/null)" != "$cfgline" ]; then
        echo "  build-$b: configure: ${cfg[*]}"
        rm -rf "$B"; mkdir -p "$B"
        if ! (cd "$B" && PKG_CONFIG_LIBDIR="$P/lib/pkgconfig" PKG_CONFIG_PATH= \
          "$FF_SRC/configure" "${cfg[@]}" > configure.log 2>&1); then
          echo "  build-$b: configure FAILED (see $B/configure.log, $B/ffbuild/config.log)"
          build_ok=0; continue
        fi
        echo "$cfgline" > "$B/.stamp-configure"
      else
        echo "  build-$b: relinking against the changed library"
        rm -f "$B/ffmpeg" "$B/ffmpeg_g"
      fi
      if ! make -C "$B" -j "$jobs" ffmpeg > "$B/make.log" 2>&1; then
        echo "  build-$b: make FAILED (see $B/make.log)"; build_ok=0; continue
      fi
      echo "$libsum" > "$B/.stamp-lib"
    fi
    # Linkage checks.
    if [ -n "$arch" ]; then
      deps="$(otool -L "$B/ffmpeg" | tail -n +2 | awk '{print $1}' | tr '\n' ' ')"
      [ "$(lipo -archs "$B/ffmpeg")" = "$arch" ] || { echo "  build-$b: not $arch"; build_ok=0; }
    else
      deps="$(readelf -d "$B/ffmpeg" | awk '/NEEDED/ {print $5}' | tr '\n' ' ')"
    fi
    nref="$(nm "$B/ffmpeg_g" 2>/dev/null | grep -c 'FDKaacEnc_Initialize\|CAacDecoder_Init')"
    case "$b" in
      ref) [ "$nref" -gt 0 ] && [[ "$deps" != *fdk-aac* ]] || { echo "  build-$b: linkage check FAILED ($nref, $deps)"; build_ok=0; } ;;
      port-static) [ "$nref" = 0 ] && [[ "$deps" != *fdk-aac* ]] && [ "$(nm "$B/ffmpeg_g" | grep -c ' T _\{0,1\}aacEncOpen$')" = 1 ] \
        || { echo "  build-$b: linkage check FAILED ($nref, $deps)"; build_ok=0; } ;;
      port) [ "$nref" = 0 ] && [[ "$deps" == *"libfdk-aac.2.dylib"* || "$deps" == *"libfdk-aac.so.2"* ]] \
        || { echo "  build-$b: linkage check FAILED ($nref, $deps)"; build_ok=0; } ;;
    esac
    echo "  build-$b: linked against: $deps"
  done
  if [ $build_ok = 0 ]; then summary+=("$triple: FFmpeg builds FAILED"); status=1; continue; fi

  # mx clips as WAV (once, with the reference build).
  WAVDIR="$INPUTS/mx-wav"
  mkdir -p "$WAVDIR"
  for f in "$WORK"/mx/*.raw; do
    [ -f "$f" ] || continue
    n="$(basename "$f" .raw)"
    [ -f "$WAVDIR/$n.wav" ] && continue
    "$FFV/build-ref/ffmpeg" -nostdin -v error -f s16le -ar "$(mx_rate "$n")" -ch_layout "$(mx_layout "$n")" \
      -i "$f" -c:a pcm_s16le -fflags +bitexact -f wav "$WAVDIR/$n.wav.tmp" && mv "$WAVDIR/$n.wav.tmp" "$WAVDIR/$n.wav"
  done

  # --- run ----------------------------------------------------------------------------------------
  subsample() { if [ $quick = 1 ]; then awk 'NR % 10 == 1'; else cat; fi; }
  gen_enc_jobs "$WAVDIR" | subsample > "$FFV/jobs-enc.txt"
  echo "  encode: $(wc -l < "$FFV/jobs-enc.txt" | tr -d ' ') commands x ${#BUILDS[@]} builds"
  run_jobs "$FFV/jobs-enc.txt"
  collect "$FFV/jobs-enc.txt" ref > "$FFV/results-enc-ref.txt"
  gen_dec_jobs "$FFV/results-enc-ref.txt" | subsample > "$FFV/jobs-dec.txt"
  echo "  decode: $(wc -l < "$FFV/jobs-dec.txt" | tr -d ' ') commands x ${#BUILDS[@]} builds"
  run_jobs "$FFV/jobs-dec.txt"
  cat "$FFV/jobs-enc.txt" "$FFV/jobs-dec.txt" > "$FFV/jobs-all.txt"
  for b in "${BUILDS[@]}"; do collect "$FFV/jobs-all.txt" "$b" > "$FFV/results-$b.txt"; done

  # --- compare ------------------------------------------------------------------------------------
  rm -rf "$FFV/diff"
  rm -rf "$FFV/retry"
  for b in "${BUILDS[@]:1}"; do
    # Commands with identical exit status and outputs whose logs differ are re-run (both
    # builds, up to three times): ffmpeg occasionally splits a log line around a line from
    # another thread, which the grouping cannot undo.
    paste "$FFV/results-ref.txt" "$FFV/results-$b.txt" \
      | awk -F'\t' '$3 == $9 && $6 == $12 && $4 != $10 {print $1 "\t" $4 "\t" $10}' \
      | while IFS=$'\t' read -r id s1 s2; do
          printf '%s\0%s\0%s\0%s\0' "$(awk -F'\t' -v id="$id" '$1 == id' "$FFV/jobs-all.txt")" "$s1" "$s2" "$b"
        done | xargs -0 -P "$jobs" -n 4 bash "$SELF" __retry "$FFV" > "$FFV/retry-$b.txt"
    paste "$FFV/results-ref.txt" "$FFV/results-$b.txt" | awk -F'\t' -v b="$b" -v retried="$FFV/retry-$b.txt" '
      BEGIN { while ((getline id < retried) > 0) rerun[id] = 1 }
      { cat = $2; n[cat]++; if ($3 == 0) ok[cat]++
        if ($1 != $7) { print "BUG: job order"; exit 1 }
        d = ""
        if ($3 != $9) d = d " exit(" $3 "/" $9 ")"
        if ($12 != $6) d = d " outputs"
        if ($10 != $4) { if ($1 in rerun) rr[cat]++; else d = d " log" }
        if ($11 != $5) raw[cat]++
        if (d == "") same[cat]++; else { diff[cat]++; print "  DIFF " b " " $1 ":" d }
        if (!(cat in seen)) { seen[cat] = 1; order[++nc] = cat } }
      END { for (i = 1; i <= nc; i++) { c = order[i]
              printf "  %s %s: %d commands (%d exit 0 with the reference), %d identical, %d different (logs equal only after a re-run: %d; raw stderr differs by interleaving: %d)\n",
                b, c, n[c], ok[c], same[c], diff[c], rr[c], raw[c] } }' > "$FFV/compare-$b.txt"
    grep '^  DIFF' "$FFV/compare-$b.txt"
    grep -v '^  DIFF' "$FFV/compare-$b.txt"
    ndiff="$(grep -c '^  DIFF' "$FFV/compare-$b.txt")"
    total="$(wc -l < "$FFV/results-ref.txt" | tr -d ' ')"
    if [ "$ndiff" = 0 ]; then
      summary+=("$triple $b: ok ($total commands identical to the reference build)")
    else
      summary+=("$triple $b: $ndiff of $total commands DIFFER (see $FFV/compare-$b.txt, $FFV/diff/)")
      status=1
      # Re-run the differing commands with all outputs kept.
      grep '^  DIFF' "$FFV/compare-$b.txt" | awk '{print $3}' | sed 's/:$//' | while read -r id; do
        l="$(awk -F'\t' -v id="$id" '$1 == id' "$FFV/jobs-all.txt")"
        for bb in ref "$b"; do bash "$SELF" __job "$FFV" 1 "$FFV/diff" "$bb" "$l"; done
      done
    fi
  done
done

printf '%s\n' "${summary[@]}"
exit $status
