/* fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
 * A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
 *
 * C-level differential test of the encoder API (scripts/capi_test.sh). The program drives
 * aacenc_lib.h through fixed call sequences and prints every result: return codes, the whole
 * output buffer (as a hash; the encoder clears bufSizes bytes on each call), AACENC_OutArgs,
 * AACENC_InfoStruct with the configuration, every parameter, and LIB_INFO. scripts/capi_test.sh
 * links it once against fdk-aac-rs-capi and once against the reference library and compares
 * the outputs.
 *
 * Not printed: LIB_INFO build_date/build_time contents (PORTING.md, "Deviations from the
 * reference"; only whether they were written).
 *
 * With FDK_AAC_RS_ENC_THREADS (or, if that is unset, FDK_AAC_RS_THREADS) above 1 in the
 * environment (both builds), the loops do not depend on
 * return codes (no early end of the input loop; numInSamples consumed whatever the return
 * code), and the port build (-DFDK_AAC_RS_PORT), whose encoders with SBR then return each
 * call's result with the next call (the output delay of the multi-threading, see
 * crates/aacenc/src/aacenc_lib/pipeline.rs), prints every result with the call it belongs to:
 * the record of a call and everything printed after it wait for the next call. The port then
 * makes one more flush call at the end, whose record is dropped.
 *
 * Usage: capi_enc_diff [CASE-NAME-PREFIX] */

#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "aacenc_lib.h"

/* FDK_AAC_RS_ENC_THREADS / FDK_AAC_RS_THREADS above 1 */
static int threads_env;
#ifdef FDK_AAC_RS_PORT
static const int port = 1;
#else
static const int port = 0;
#endif
/* the encoder of the running case delays its output (port only) */
static int delayed;
/* the call whose result is still to come: its number and numInSamples */
static int pend_valid, pend_k, pend_in;
/* while set, output is kept in held[] (it follows the pending call's record) */
static int holding;
static char *held;
static size_t held_len, held_cap;

static void outf(const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  if (!holding) {
    vprintf(fmt, ap);
  } else {
    char tmp[4096];
    int n = vsnprintf(tmp, sizeof(tmp), fmt, ap);
    if (n > 0) {
      size_t m = (size_t)n < sizeof(tmp) ? (size_t)n : sizeof(tmp) - 1;
      if (held_len + m > held_cap) {
        held_cap = (held_len + m) * 2;
        held = (char *)realloc(held, held_cap);
      }
      memcpy(held + held_len, tmp, m);
      held_len += m;
    }
  }
  va_end(ap);
}

/* Writes the kept output and stops keeping it. */
static void release_held(void) {
  holding = 0;
  if (held_len) fwrite(held, 1, held_len, stdout);
  held_len = 0;
}

#define printf outf

#define MAXP 16

typedef struct {
  int param; /* -1 ends the list */
  unsigned value;
} PV;

typedef struct {
  const char *name;
  unsigned modules, max_channels;
  PV params[MAXP];
  PV mid[4];   /* set before call 6 (reconfiguration while encoding) */
  int channels;
  int frames;  /* samples per channel */
  int chunk[4]; /* samples per channel of each call, cycled (0 ends) */
  int out_size;
  int el_size; /* 2 or 4 */
  int anc;     /* pass ancillary data */
  int metadata; /* pass AACENC_MetaData */
} Case;

static uint64_t fnv(const void *p, size_t n) {
  const unsigned char *b = (const unsigned char *)p;
  uint64_t h = 1469598103934665603ULL;
  size_t i;
  for (i = 0; i < n; i++) {
    h ^= b[i];
    h *= 1099511628211ULL;
  }
  return h;
}

static uint32_t rng = 1;
static uint32_t next_rand(void) {
  rng ^= rng << 13;
  rng ^= rng >> 17;
  rng ^= rng << 5;
  return rng;
}

/* Interleaved test signal: a triangle wave per channel plus noise. */
static void *make_pcm(int frames, int channels, int el_size) {
  size_t n = (size_t)frames * channels;
  void *buf = malloc(n * el_size);
  size_t i;
  rng = 0x12345678u;
  for (i = 0; i < n; i++) {
    int ch = (int)(i % channels);
    int t = (int)(i / channels);
    int period = 40 + 13 * ch;
    int ph = t % period;
    int tri = (ph < period / 2 ? ph : period - ph) * 24000 / period - 6000;
    int s = tri + (int)(next_rand() >> 21) - 1024;
    if (el_size == 4)
      ((int32_t *)buf)[i] = (int32_t)((uint32_t)(int16_t)s << 16 | (next_rand() & 0xffff));
    else
      ((int16_t *)buf)[i] = (int16_t)s;
  }
  return buf;
}

static const int all_params[] = {0x0100, 0x0101, 0x0102, 0x0103, 0x0104, 0x0105, 0x0106,
                                 0x0107, 0x0108, 0x0200, 0x0203, 0x0207, 0x0300, 0x0301,
                                 0x0302, 0x0303, 0x0304, 0x0306, 0x0500, 0x0600, 0xFF00,
                                 0xFFFF, 0x1234};

static void print_params(HANDLE_AACENCODER h) {
  size_t i;
  printf("params");
  for (i = 0; i < sizeof(all_params) / sizeof(all_params[0]); i++)
    printf(" %x=%u", all_params[i], aacEncoder_GetParam(h, (AACENC_PARAM)all_params[i]));
  printf("\n");
}

static void print_info(HANDLE_AACENCODER h) {
  AACENC_InfoStruct info;
  AACENC_ERROR e;
  UINT i;
  memset(&info, 0xa5, sizeof(info));
  e = aacEncInfo(h, &info);
  printf("info rc=%x max_out=%u max_anc=%u fill=%u ch=%u frame=%u delay=%u delay_core=%u "
         "conf_size=%u conf=",
         e, info.maxOutBufBytes, info.maxAncBytes, info.inBufFillLevel, info.inputChannels,
         info.frameLength, info.nDelay, info.nDelayCore, info.confSize);
  for (i = 0; i < 64; i++) printf("%02x", info.confBuf[i]);
  printf("\n");
}

static void fill_metadata(AACENC_MetaData *md, int k) {
  memset(md, 0, sizeof(*md));
  md->drc_profile = AACENC_METADATA_DRC_FILMSTANDARD;
  md->comp_profile = (k & 1) ? AACENC_METADATA_DRC_MUSICLIGHT : AACENC_METADATA_DRC_SPEECH;
  md->drc_TargetRefLevel = -((16 + (k % 8)) << 16);
  md->comp_TargetRefLevel = -(20 << 16);
  md->prog_ref_level_present = 1;
  md->prog_ref_level = -((20 + (k % 4)) << 16);
  md->PCE_mixdown_idx_present = 0;
  md->ETSI_DmxLvl_present = 1;
  md->centerMixLevel = 2;
  md->surroundMixLevel = 3;
  md->dolbySurroundMode = (UCHAR)(k % 3);
  md->drcPresentationMode = 0;
  md->ExtMetaData.extAncDataEnable = (UCHAR)(k % 2);
  md->ExtMetaData.extDownmixLevelEnable = 1;
  md->ExtMetaData.extDownmixLevel_A = 2;
  md->ExtMetaData.extDownmixLevel_B = 4;
  md->ExtMetaData.dmxGainEnable = 1;
  md->ExtMetaData.dmxGain5 = -(3 << 16);
  md->ExtMetaData.dmxGain2 = -(6 << 16);
  md->ExtMetaData.lfeDmxEnable = 1;
  md->ExtMetaData.lfeDmxLevel = 5;
}

/* One aacEncEncode() call; num_in_samples -1 flushes. Returns the return code. */
static AACENC_ERROR encode(HANDLE_AACENCODER h, const Case *c, void *pcm, int num_in_samples,
                           unsigned char *out, int k, int *consumed) {
  AACENC_BufDesc in_desc = {0}, out_desc = {0};
  AACENC_InArgs in_args = {0};
  AACENC_OutArgs out_args;
  void *in_ptr[3], *out_ptr = out;
  INT in_id[3], in_size[3], in_el[3];
  INT out_id = OUT_BITSTREAM_DATA, out_size = c->out_size, out_el = 1;
  unsigned char anc[32];
  AACENC_MetaData md;
  int n = 0, nanc = 0, i;
  AACENC_ERROR e;

  in_ptr[n] = pcm;
  in_id[n] = IN_AUDIO_DATA;
  in_size[n] = (num_in_samples > 0 ? num_in_samples : 0) * c->el_size;
  in_el[n] = c->el_size;
  n++;
  if (c->anc && num_in_samples >= 0) {
    nanc = (k % 7) * 3;
    for (i = 0; i < nanc; i++) anc[i] = (unsigned char)(0x5c ^ (k + i));
    if (nanc > 0) {
      in_ptr[n] = anc;
      in_id[n] = IN_ANCILLRY_DATA;
      in_size[n] = nanc;
      in_el[n] = 1;
      n++;
    }
  }
  if (c->metadata && num_in_samples >= 0) {
    fill_metadata(&md, k);
    in_ptr[n] = &md;
    in_id[n] = IN_METADATA_SETUP;
    in_size[n] = sizeof(md);
    in_el[n] = sizeof(md);
    n++;
  }
  in_desc.numBufs = n;
  in_desc.bufs = in_ptr;
  in_desc.bufferIdentifiers = in_id;
  in_desc.bufSizes = in_size;
  in_desc.bufElSizes = in_el;
  out_desc.numBufs = 1;
  out_desc.bufs = &out_ptr;
  out_desc.bufferIdentifiers = &out_id;
  out_desc.bufSizes = &out_size;
  out_desc.bufElSizes = &out_el;
  in_args.numInSamples = num_in_samples;
  in_args.numAncBytes = nanc;
  memset(&out_args, 0xa5, sizeof(out_args));
  memset(out, 0xa5, c->out_size);

  e = aacEncEncode(h, &in_desc, &out_desc, &in_args, &out_args);
  *consumed = threads_env || e == AACENC_OK ? out_args.numInSamples : 0;
  if (port && threads_env && !delayed && aacEncoder_GetParam(h, AACENC_SBR_RATIO) != 0) {
    /* the output delay starts with this call (SBR is active): it returned no result */
    delayed = 1;
    e = AACENC_OK;
  } else if (delayed) {
    /* the result of the previous call */
    holding = 0;
    if (pend_valid)
      printf("encode %d rc=%x out=%d in=%d anc=%d br=%d buf=%016llx\n", pend_k, e,
             out_args.numOutBytes, pend_in, out_args.numAncBytes, out_args.bitResState,
             (unsigned long long)fnv(out, c->out_size));
    release_held();
  } else {
    printf("encode %d rc=%x out=%d in=%d anc=%d br=%d buf=%016llx\n", k, e,
           out_args.numOutBytes, out_args.numInSamples, out_args.numAncBytes,
           out_args.bitResState, (unsigned long long)fnv(out, c->out_size));
    return e;
  }
  pend_valid = 1;
  pend_k = k;
  pend_in = out_args.numInSamples;
  holding = 1;
  return e;
}

static void run_case(const Case *c) {
  HANDLE_AACENCODER h = NULL;
  AACENC_ERROR e;
  unsigned char *out = (unsigned char *)malloc(c->out_size > 0 ? c->out_size : 1);
  void *pcm = make_pcm(c->frames, c->channels, c->el_size);
  int total = c->frames * c->channels, pos = 0, k = 0, i, failed = 0;

  printf("case %s\n", c->name);
  delayed = pend_valid = 0;
  e = aacEncOpen(&h, c->modules, c->max_channels);
  printf("open rc=%x\n", e);
  if (e != AACENC_OK) goto done;
  for (i = 0; i < MAXP && c->params[i].param != -1; i++) {
    e = aacEncoder_SetParam(h, (AACENC_PARAM)c->params[i].param, c->params[i].value);
    printf("set %x %u rc=%x\n", c->params[i].param, c->params[i].value, e);
  }
  e = aacEncEncode(h, NULL, NULL, NULL, NULL);
  printf("init rc=%x\n", e);
  print_params(h);
  if (e != AACENC_OK) {
    failed = 1;
  } else {
    print_info(h);
  }

  while (!failed && pos < total) {
    int chunk = c->chunk[k % 4] ? c->chunk[k % 4] : c->chunk[0];
    int n = chunk * c->channels, consumed;
    if (n > total - pos) n = total - pos;
    if (k == 6) {
      for (i = 0; i < 4 && c->mid[i].param != 0; i++) {
        e = aacEncoder_SetParam(h, (AACENC_PARAM)c->mid[i].param, c->mid[i].value);
        printf("mid set %x %u rc=%x\n", c->mid[i].param, c->mid[i].value, e);
      }
    }
    e = encode(h, c, (char *)pcm + (size_t)pos * c->el_size, n, out, k, &consumed);
    if (e != AACENC_OK && e != AACENC_ENCODE_ERROR && !threads_env) break;
    pos += consumed > 0 ? consumed : n;
    k++;
    if (k % 10 == 0) {
      print_info(h);
      print_params(h);
    }
  }
  for (i = 0; !failed && i < 200; i++) {
    int consumed;
    e = encode(h, c, pcm, -1, out, k++, &consumed);
    if (e != AACENC_OK) break;
  }
  if (!failed) encode(h, c, pcm, -1, out, k++, &i); /* after the end of the flush */
  if (delayed) {
    /* (the record of the last call: a call the single-threaded run does not make) */
    pend_valid = 0;
    held_len = 0;
    holding = 0;
  }
  print_params(h);
  print_info(h);
done:
  e = aacEncClose(&h);
  printf("close rc=%x h=%s\n", e, h ? "set" : "null");
  free(pcm);
  free(out);
}

#define P(a, b) {a, b}
#define END {-1, 0}

static const Case cases[] = {
    {"lc-stereo-adts", 0, 0,
     {P(AACENC_AOT, 2), P(AACENC_SAMPLERATE, 44100), P(AACENC_CHANNELMODE, MODE_2),
      P(AACENC_CHANNELORDER, 1), P(AACENC_BITRATE, 128000), P(AACENC_TRANSMUX, TT_MP4_ADTS),
      P(AACENC_AFTERBURNER, 1), END},
     {{0, 0}}, 2, 88200, {1024, 700, 2048, 1}, 8192, 2, 0, 0},
    {"he-aac-loas", 0, 0,
     {P(AACENC_AOT, 5), P(AACENC_SAMPLERATE, 48000), P(AACENC_CHANNELMODE, MODE_2),
      P(AACENC_BITRATE, 64000), P(AACENC_TRANSMUX, TT_MP4_LOAS), P(AACENC_SIGNALING_MODE, 2),
      P(AACENC_AUDIOMUXVER, 1), END},
     {{0, 0}}, 2, 96000, {2048, 1500, 0, 0}, 8192, 2, 0, 0},
    {"he-aac-v2-raw", 0, 0,
     {P(AACENC_AOT, 29), P(AACENC_SAMPLERATE, 44100), P(AACENC_CHANNELMODE, MODE_2),
      P(AACENC_BITRATE, 32000), P(AACENC_TRANSMUX, TT_MP4_RAW), END},
     {{0, 0}}, 2, 88200, {2048, 0, 0, 0}, 8192, 2, 0, 0},
    {"he-aac-downsampled-sbr", 0, 0,
     {P(AACENC_AOT, 5), P(AACENC_SAMPLERATE, 48000), P(AACENC_CHANNELMODE, MODE_2),
      P(AACENC_SBR_RATIO, 1), P(AACENC_BITRATE, 64000), P(AACENC_TRANSMUX, TT_MP4_LOAS),
      P(AACENC_SIGNALING_MODE, 1), P(AACENC_AUDIOMUXVER, 1), END},
     {{0, 0}}, 2, 48000, {1024, 0, 0, 0}, 8192, 2, 0, 0},
    {"ld-mono-latm", 0, 0,
     {P(AACENC_AOT, 23), P(AACENC_SAMPLERATE, 32000), P(AACENC_CHANNELMODE, MODE_1),
      P(AACENC_GRANULE_LENGTH, 480), P(AACENC_BITRATE, 48000),
      P(AACENC_TRANSMUX, TT_MP4_LATM_MCP1), P(AACENC_HEADER_PERIOD, 3), END},
     {{0, 0}}, 1, 64000, {480, 100, 1000, 0}, 8192, 2, 0, 0},
    {"eld-sbr-loas", 0, 0,
     {P(AACENC_AOT, 39), P(AACENC_SBR_MODE, 1), P(AACENC_SAMPLERATE, 48000),
      P(AACENC_CHANNELMODE, MODE_2), P(AACENC_BITRATE, 64000), P(AACENC_TRANSMUX, TT_MP4_LOAS),
      END},
     {{0, 0}}, 2, 96000, {512, 333, 0, 0}, 8192, 2, 0, 0},
    {"eld-v2-212", 0, 0,
     {P(AACENC_AOT, 39), P(AACENC_SAMPLERATE, 48000), P(AACENC_CHANNELMODE, MODE_212),
      P(AACENC_BITRATE, 48000), P(AACENC_TRANSMUX, TT_MP4_RAW), END},
     {{0, 0}}, 2, 96000, {512, 0, 0, 0}, 8192, 2, 0, 0},
    {"eld-downscaled", 0, 0,
     {P(AACENC_AOT, 39), P(AACENC_SBR_MODE, 0), P(AACENC_GRANULE_LENGTH, 256),
      P(AACENC_SAMPLERATE, 48000), P(AACENC_CHANNELMODE, MODE_1), P(AACENC_BITRATE, 48000),
      P(AACENC_TRANSMUX, TT_MP4_RAW), END},
     {{0, 0}}, 1, 48000, {256, 0, 0, 0}, 8192, 2, 0, 0},
    {"mpeg2-5.1-crc-metadata-anc", 0, 0,
     {P(AACENC_AOT, 129), P(AACENC_SAMPLERATE, 48000), P(AACENC_CHANNELMODE, MODE_1_2_2_1),
      P(AACENC_BITRATE, 320000), P(AACENC_TRANSMUX, TT_MP4_ADTS), P(AACENC_PROTECTION, 1),
      P(AACENC_METADATA_MODE, 2), END},
     {{0, 0}}, 6, 48000, {1024, 0, 0, 0}, 16384, 2, 1, 1},
    {"lc-vbr-anc-rate", 0, 0,
     {P(AACENC_SAMPLERATE, 44100), P(AACENC_CHANNELMODE, MODE_2), P(AACENC_BITRATEMODE, 3),
      P(AACENC_TRANSMUX, TT_MP4_ADTS), P(AACENC_ANCILLARY_BITRATE, 2000), END},
     {{0, 0}}, 2, 88200, {1024, 3000, 0, 0}, 8192, 2, 1, 0},
    {"lc-7.1-back", 0, 0,
     {P(AACENC_SAMPLERATE, 48000), P(AACENC_CHANNELMODE, MODE_7_1_BACK),
      P(AACENC_BITRATE, 448000), P(AACENC_TRANSMUX, TT_MP4_ADTS), END},
     {{0, 0}}, 8, 48000, {1024, 0, 0, 0}, 32768, 2, 0, 0},
    {"he-aac-5.1", 0, 0,
     {P(AACENC_AOT, 5), P(AACENC_SAMPLERATE, 48000), P(AACENC_CHANNELMODE, MODE_1_2_2_1),
      P(AACENC_CHANNELORDER, 1), P(AACENC_BITRATE, 160000), P(AACENC_TRANSMUX, TT_MP4_ADTS),
      END},
     {{0, 0}}, 6, 48000, {2048, 0, 0, 0}, 16384, 2, 0, 0},
    {"lc-32bit-input", 0, 0,
     {P(AACENC_SAMPLERATE, 44100), P(AACENC_CHANNELMODE, MODE_2), P(AACENC_BITRATE, 128000),
      P(AACENC_TRANSMUX, TT_MP4_ADTS), END},
     {{0, 0}}, 2, 44100, {1024, 777, 0, 0}, 8192, 4, 0, 0},
    {"lc-small-output", 0, 0,
     {P(AACENC_SAMPLERATE, 48000), P(AACENC_CHANNELMODE, MODE_2), P(AACENC_BITRATE, 256000),
      P(AACENC_TRANSMUX, TT_MP4_ADTS), END},
     {{0, 0}}, 2, 24000, {1024, 0, 0, 0}, 300, 2, 0, 0},
    {"lc-adif", 0, 0,
     {P(AACENC_SAMPLERATE, 22050), P(AACENC_CHANNELMODE, MODE_1), P(AACENC_BITRATE, 40000),
      P(AACENC_TRANSMUX, TT_MP4_ADIF), END},
     {{0, 0}}, 1, 22050, {1024, 0, 0, 0}, 8192, 2, 0, 0},
    {"lc-latm-mcp0-subframes", 0, 0,
     {P(AACENC_SAMPLERATE, 32000), P(AACENC_CHANNELMODE, MODE_2), P(AACENC_BITRATE, 96000),
      P(AACENC_TRANSMUX, TT_MP4_LATM_MCP0), P(AACENC_TPSUBFRAMES, 2), END},
     {{0, 0}}, 2, 32000, {1024, 0, 0, 0}, 8192, 2, 0, 0},
    {"lc-peak-bandwidth", 0, 0,
     {P(AACENC_SAMPLERATE, 48000), P(AACENC_CHANNELMODE, MODE_2), P(AACENC_BITRATE, 128000),
      P(AACENC_PEAK_BITRATE, 160000), P(AACENC_BANDWIDTH, 15000),
      P(AACENC_TRANSMUX, TT_MP4_ADTS), END},
     {{0, 0}}, 2, 48000, {1024, 0, 0, 0}, 8192, 2, 0, 0},
    {"lc-8k-mono", 0, 0,
     {P(AACENC_SAMPLERATE, 8000), P(AACENC_CHANNELMODE, MODE_1), P(AACENC_BITRATE, 16000),
      P(AACENC_TRANSMUX, TT_MP4_ADTS), END},
     {{0, 0}}, 1, 16000, {1024, 0, 0, 0}, 8192, 2, 0, 0},
    {"reconfigure", 0, 0,
     {P(AACENC_SAMPLERATE, 44100), P(AACENC_CHANNELMODE, MODE_2), P(AACENC_BITRATE, 96000),
      P(AACENC_TRANSMUX, TT_MP4_ADTS), END},
     {P(AACENC_BITRATE, 64000), P(AACENC_AOT, 5), P(AACENC_SAMPLERATE, 48000), {0, 0}},
     2, 66150, {1024, 0, 0, 0}, 8192, 2, 0, 0},
    /* Error paths. */
    {"open-too-many-channels", 0, 9, {END}, {{0, 0}}, 2, 0, {1024, 0, 0, 0}, 8192, 2, 0, 0},
    {"aot-vs-modules", 1, 2,
     {P(AACENC_AOT, 5), P(AACENC_AOT, 29), P(AACENC_AOT, 39), P(AACENC_CHANNELMODE, MODE_212),
      P(AACENC_CHANNELMODE, MODE_2), P(AACENC_SAMPLERATE, 32000), END},
     {{0, 0}}, 2, 4096, {1024, 0, 0, 0}, 8192, 2, 0, 0},
    {"invalid-values", 0, 2,
     {P(AACENC_SAMPLERATE, 44000), P(AACENC_BITRATEMODE, 6), P(AACENC_CHANNELMODE, MODE_1_2_2_1),
      P(AACENC_GRANULE_LENGTH, 1000), P(AACENC_TRANSMUX, 3), P(AACENC_SIGNALING_MODE, 3),
      P(AACENC_HEADER_PERIOD, 256), P(AACENC_TPSUBFRAMES, 5), P(AACENC_METADATA_MODE, 4),
      P(AACENC_AFTERBURNER, 2), P(AACENC_CHANNELORDER, 3), P(0x9999, 1),
      P(AACENC_CHANNELMODE, MODE_1), P(AACENC_SAMPLERATE, 16000), END},
     {{0, 0}}, 1, 4096, {1024, 0, 0, 0}, 8192, 2, 0, 0},
    {"init-error", 0, 0,
     {P(AACENC_AOT, 29), P(AACENC_CHANNELMODE, MODE_1), P(AACENC_SAMPLERATE, 44100), END},
     {{0, 0}}, 1, 4096, {1024, 0, 0, 0}, 8192, 2, 0, 0},
};

static void null_paths(void) {
  HANDLE_AACENCODER h = NULL;
  AACENC_InfoStruct info;
  AACENC_InArgs in_args = {0};
  AACENC_OutArgs out_args;
  AACENC_BufDesc bad = {0};
  void *null_buf = NULL;
  INT id = IN_AUDIO_DATA, size = 0, el = 2;
  printf("case null-paths\n");
  printf("open(NULL) rc=%x\n", aacEncOpen(NULL, 0, 0));
  printf("close(NULL) rc=%x\n", aacEncClose(NULL));
  printf("close(&NULL) rc=%x\n", aacEncClose(&h));
  printf("encode(NULL) rc=%x\n", aacEncEncode(NULL, NULL, NULL, NULL, NULL));
  printf("info(NULL) rc=%x\n", aacEncInfo(NULL, &info));
  printf("set(NULL) rc=%x\n", aacEncoder_SetParam(NULL, AACENC_AOT, 2));
  printf("get(NULL) value=%u\n", aacEncoder_GetParam(NULL, AACENC_AOT));
  printf("libinfo(NULL) rc=%x\n", aacEncGetLibInfo(NULL));
  printf("open rc=%x\n", aacEncOpen(&h, 0, 2));
  printf("info(h, NULL) rc=%x\n", aacEncInfo(h, NULL));
  /* Every case sets a channel mode and a sample rate first (PORTING.md, "Deviations from the
   * reference"). */
  printf("set channel mode rc=%x\n", aacEncoder_SetParam(h, AACENC_CHANNELMODE, MODE_1));
  printf("set sample rate rc=%x\n", aacEncoder_SetParam(h, AACENC_SAMPLERATE, 48000));
  print_info(h); /* before the initialization */
  printf("encode(inargs only) rc=%x\n", aacEncEncode(h, NULL, NULL, &in_args, NULL));
  memset(&out_args, 0xa5, sizeof(out_args));
  printf("encode(no descriptors) rc=%x\n", aacEncEncode(h, NULL, NULL, &in_args, &out_args));
  printf("outargs %d %d %d %d\n", out_args.numOutBytes, out_args.numInSamples,
         out_args.numAncBytes, out_args.bitResState);
  /* A descriptor with a NULL buffer and samples to read. */
  bad.numBufs = 1;
  bad.bufs = &null_buf;
  bad.bufferIdentifiers = &id;
  bad.bufSizes = &size;
  bad.bufElSizes = &el;
  in_args.numInSamples = 2;
  printf("encode(NULL buffer) rc=%x\n", aacEncEncode(h, &bad, &bad, &in_args, &out_args));
  in_args.numInSamples = 0;
  printf("encode(NULL buffer, 0 samples) rc=%x\n",
         aacEncEncode(h, &bad, &bad, &in_args, &out_args));
  printf("outargs %d %d %d %d\n", out_args.numOutBytes, out_args.numInSamples,
         out_args.numAncBytes, out_args.bitResState);
  print_params(h);
  printf("close rc=%x h=%s\n", aacEncClose(&h), h ? "set" : "null");
}

static const char sentinel[] = "untouched";

static void print_lib_info(const LIB_INFO *info) {
  int i, k;
  for (i = 0; i < FDK_MODULE_LAST; i++) {
    printf("  [%d] id=%d title=%s date=%s time=%s version=%x flags=%x str=", i,
           (int)info[i].module_id, info[i].title == sentinel ? "<untouched>" : info[i].title,
           info[i].build_date == sentinel ? "<untouched>" : "<written>",
           info[i].build_time == sentinel ? "<untouched>" : "<written>", info[i].version,
           info[i].flags);
    for (k = 0; k < 32; k++) printf("%02x", (unsigned char)info[i].versionStr[k]);
    printf("\n");
  }
}

static void lib_info(void) {
  LIB_INFO info[FDK_MODULE_LAST];
  int i;
  printf("case lib-info\n");
  for (i = 0; i < FDK_MODULE_LAST; i++) {
    info[i].title = info[i].build_date = info[i].build_time = sentinel;
    info[i].module_id = FDK_NONE;
    info[i].version = 0x5a5a5a5a;
    info[i].flags = 0xdeadbeef;
    memset(info[i].versionStr, 'A', sizeof(info[i].versionStr));
  }
  /* A module registered by the caller: the library skips it. */
  info[0].module_id = FDK_AACDEC;
  printf("rc=%x\n", aacEncGetLibInfo(info));
  print_lib_info(info);
  printf("second call rc=%x\n", aacEncGetLibInfo(info));
  print_lib_info(info);
  /* No free entry. */
  for (i = 0; i < FDK_MODULE_LAST; i++) info[i].module_id = FDK_PCMDMX;
  printf("full rc=%x\n", aacEncGetLibInfo(info));
  print_lib_info(info);
}

int main(int argc, char **argv) {
  size_t i;
  const char *only = argc > 1 ? argv[1] : NULL;
  /* (as aacEncOpen(): FDK_AAC_RS_ENC_THREADS takes precedence) */
  const char *t = getenv("FDK_AAC_RS_ENC_THREADS");
  if (!t) t = getenv("FDK_AAC_RS_THREADS");
  threads_env = t && atoi(t) > 1;
  setvbuf(stdout, NULL, _IOLBF, 1 << 16);
  if (!only) {
    null_paths();
    lib_info();
  }
  for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++)
    if (!only || strncmp(cases[i].name, only, strlen(only)) == 0) run_case(&cases[i]);
  return 0;
}
