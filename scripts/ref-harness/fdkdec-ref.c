/* fdkdec-ref: reference decoder harness of fdk-aac-rs.
 *
 * Drives the aacDecoder_* API of the reference library with a call sequence fully determined by
 * the command line, and writes the PCM output and a JSON Lines log of every API call. The Rust
 * CLI (fdk-aac-rs-dec) implements the same interface. The interface and the log format are
 * specified in PORTING.md, "Reference harness"; keep both in sync. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "aacdecoder_lib.h"
#include "common.h"
#include "mp4demux.h"

#define DEFAULT_OUT_SIZE 65536
#define DEFAULT_FILL_SIZE 4096
#define DEFAULT_MAX_ERRORS 1000

static const struct {
  const char *name;
  AACDEC_PARAM id;
} PARAMS[] = {
    {"AAC_PCM_DUAL_CHANNEL_OUTPUT_MODE", AAC_PCM_DUAL_CHANNEL_OUTPUT_MODE},
    {"AAC_PCM_OUTPUT_CHANNEL_MAPPING", AAC_PCM_OUTPUT_CHANNEL_MAPPING},
    {"AAC_PCM_LIMITER_ENABLE", AAC_PCM_LIMITER_ENABLE},
    {"AAC_PCM_LIMITER_ATTACK_TIME", AAC_PCM_LIMITER_ATTACK_TIME},
    {"AAC_PCM_LIMITER_RELEAS_TIME", AAC_PCM_LIMITER_RELEAS_TIME},
    {"AAC_PCM_MIN_OUTPUT_CHANNELS", AAC_PCM_MIN_OUTPUT_CHANNELS},
    {"AAC_PCM_MAX_OUTPUT_CHANNELS", AAC_PCM_MAX_OUTPUT_CHANNELS},
    {"AAC_METADATA_PROFILE", AAC_METADATA_PROFILE},
    {"AAC_METADATA_EXPIRY_TIME", AAC_METADATA_EXPIRY_TIME},
    {"AAC_CONCEAL_METHOD", AAC_CONCEAL_METHOD},
    {"AAC_DRC_BOOST_FACTOR", AAC_DRC_BOOST_FACTOR},
    {"AAC_DRC_ATTENUATION_FACTOR", AAC_DRC_ATTENUATION_FACTOR},
    {"AAC_DRC_REFERENCE_LEVEL", AAC_DRC_REFERENCE_LEVEL},
    {"AAC_DRC_HEAVY_COMPRESSION", AAC_DRC_HEAVY_COMPRESSION},
    {"AAC_DRC_DEFAULT_PRESENTATION_MODE", AAC_DRC_DEFAULT_PRESENTATION_MODE},
    {"AAC_DRC_ENC_TARGET_LEVEL", AAC_DRC_ENC_TARGET_LEVEL},
    {"AAC_UNIDRC_SET_EFFECT", AAC_UNIDRC_SET_EFFECT},
    {"AAC_UNIDRC_ALBUM_MODE", AAC_UNIDRC_ALBUM_MODE},
    {"AAC_QMF_LOWPOWER", AAC_QMF_LOWPOWER},
    {"AAC_TPDEC_CLEAR_BUFFER", AAC_TPDEC_CLEAR_BUFFER},
};

static const struct {
  const char *name;
  int tt;
} TRANSPORTS[] = {
    {"raw", TT_MP4_RAW},       {"adif", TT_MP4_ADIF},        {"adts", TT_MP4_ADTS},
    {"latm-mcp1", TT_MP4_LATM_MCP1}, {"latm-mcp0", TT_MP4_LATM_MCP0}, {"loas", TT_MP4_LOAS},
    {"drm", TT_DRM},           {"mp4", TT_MP4_RAW},
};

enum { EV_PARAM, EV_FLAGS };

typedef struct {
  /* options */
  const char *input, *pcm_path, *log_path, *au_sizes_path;
  int transport, mp4, layers;
  uint32_t track;
  uint8_t *asc[8];
  UINT asc_len[8];
  int n_asc;
  UINT *fill_sizes;
  int n_fill_sizes;
  INT out_size;
  int poison, anc_size, decode_once, decode_loop_opt, free_bytes, drain;
  long long max_calls, max_errors, conceal_every;
  EventList init_params, events;
  /* state */
  HANDLE_AACDECODER h;
  Json j;
  FILE *pcm;
  INT_PCM *out;
  UCHAR *anc_buf;
  long long n_dec, n_fill, errors_in_row;
  unsigned long long samples_total;
  const char *stop;
} St;

static int param_id(const char *name, long long *id) {
  size_t i;
  for (i = 0; i < sizeof(PARAMS) / sizeof(PARAMS[0]); i++)
    if (strcmp(PARAMS[i].name, name) == 0) {
      *id = PARAMS[i].id;
      return 1;
    }
  return 0;
}

static const char *param_name(long long id) {
  size_t i;
  for (i = 0; i < sizeof(PARAMS) / sizeof(PARAMS[0]); i++)
    if (PARAMS[i].id == id) return PARAMS[i].name;
  return NULL;
}

/* NAME=VALUE, NAME being a header name or a numeric parameter ID. */
static Event parse_param(const char *s, long long at) {
  Event e;
  const char *eq = strchr(s, '=');
  memset(&e, 0, sizeof(e));
  if (!eq || (size_t)(eq - s) >= sizeof(e.name)) die("expected NAME=VALUE: '%s'", s);
  memcpy(e.name, s, (size_t)(eq - s));
  e.name[eq - s] = 0;
  if (!param_id(e.name, &e.id)) {
    const char *known;
    e.id = parse_int(e.name, "parameter (unknown name)");
    known = param_name(e.id);
    if (known) snprintf(e.name, sizeof(e.name), "%s", known);
  }
  e.value = parse_int(eq + 1, "parameter value");
  e.kind = EV_PARAM;
  e.at = at;
  return e;
}

static UINT parse_flags(const char *s) {
  UINT f = 0;
  char buf[128], *tok, *save = NULL;
  snprintf(buf, sizeof(buf), "%s", s);
  for (tok = strtok_r(buf, "|,+", &save); tok; tok = strtok_r(NULL, "|,+", &save)) {
    if (strcmp(tok, "CONCEAL") == 0) f |= AACDEC_CONCEAL;
    else if (strcmp(tok, "FLUSH") == 0) f |= AACDEC_FLUSH;
    else if (strcmp(tok, "INTR") == 0) f |= AACDEC_INTR;
    else if (strcmp(tok, "CLRHIST") == 0) f |= AACDEC_CLRHIST;
    else f |= (UINT)parse_int(tok, "flags");
  }
  return f;
}

static void usage(void) {
  fputs(
      "usage: fdkdec-ref -t TRANSPORT [options] INPUT\n"
      "  -t, --transport T      raw|adif|adts|latm-mcp1|latm-mcp0|loas|drm|mp4 (or a number)\n"
      "  --asc HEX              configuration for aacDecoder_ConfigRaw (repeat per layer)\n"
      "  --asc-file FILE        same, from a binary file\n"
      "  --layers N             nrOfLayers of aacDecoder_Open (default 1)\n"
      "  --track ID             mp4: track ID (default: first sound track)\n"
      "  --au-sizes FILE        packet input: one access unit size per line\n"
      "  --fill-size N[,N...]   bytes per aacDecoder_Fill, cycled (default 4096)\n"
      "  --param NAME=VALUE     aacDecoder_SetParam before decoding (repeatable)\n"
      "  --param-at N:NAME=VALUE  aacDecoder_SetParam before DecodeFrame call N\n"
      "  --flags-at N:FLAGS     OR FLAGS (CONCEAL|FLUSH|INTR|CLRHIST or a number) into call N\n"
      "  --conceal-at N, --flush-at N, --intr-at N, --clrhist-at N\n"
      "  --conceal-every N      AACDEC_CONCEAL on calls N-1, 2N-1, ...\n"
      "  --drain N              N DecodeFrame calls with AACDEC_FLUSH after the input\n"
      "  --decode-once          one DecodeFrame call per Fill call (then a loop at the end);\n"
      "                         the default for packet input (mp4, --au-sizes), without the loop\n"
      "  --decode-loop          packet input: decode until NOT_ENOUGH_BITS after each Fill\n"
      "  --max-calls N          stop after N DecodeFrame calls\n"
      "  --max-errors N         stop after N error returns in a row (default 1000, 0: off)\n"
      "  --out-size N           timeDataSize in samples (default 65536)\n"
      "  --poison-output        log writes into the output buffer beyond the returned samples\n"
      "  --anc SIZE             ancillary data buffer of SIZE bytes\n"
      "  --no-free-bytes        do not call aacDecoder_GetFreeBytes before each Fill\n"
      "  -o, --pcm FILE         PCM output (s16le, interleaved)\n"
      "  -l, --log FILE         JSON Lines log ('-': stdout)\n",
      stderr);
  exit(2);
}

static void parse_args(St *st, int argc, char **argv) {
  int i;
  st->transport = -100;
  st->layers = 1;
  st->out_size = DEFAULT_OUT_SIZE;
  st->free_bytes = 1;
  st->max_errors = DEFAULT_MAX_ERRORS;
  for (i = 1; i < argc; i++) {
    const char *a = argv[i], *v = NULL;
    char opt[64];
    const char *eq;
    if (a[0] != '-' || strcmp(a, "-") == 0) {
      if (st->input) die("more than one input");
      st->input = a;
      continue;
    }
    /* --name=value */
    eq = strchr(a, '=');
    if (a[1] == '-' && eq && (size_t)(eq - a) < sizeof(opt)) {
      memcpy(opt, a, (size_t)(eq - a));
      opt[eq - a] = 0;
      v = eq + 1;
      a = opt;
    }
#define VAL() (v ? v : (i + 1 < argc ? argv[++i] : (die("missing value for %s", a), "")))
    if (!strcmp(a, "-t") || !strcmp(a, "--transport")) {
      const char *t = VAL();
      size_t k;
      for (k = 0; k < sizeof(TRANSPORTS) / sizeof(TRANSPORTS[0]); k++)
        if (!strcmp(TRANSPORTS[k].name, t)) st->transport = TRANSPORTS[k].tt;
      if (!strcmp(t, "mp4")) st->mp4 = 1;
      if (st->transport == -100) st->transport = (int)parse_int(t, "transport");
    } else if (!strcmp(a, "--asc") || !strcmp(a, "--asc-file")) {
      int file = !strcmp(a, "--asc-file");
      const char *s = VAL();
      if (st->n_asc == 8) die("too many --asc");
      if (file) {
        size_t n;
        st->asc[st->n_asc] = read_file(s, &n);
        st->asc_len[st->n_asc] = (UINT)n;
      } else {
        st->asc_len[st->n_asc] = (UINT)parse_hex(s, &st->asc[st->n_asc], "--asc");
      }
      st->n_asc++;
    } else if (!strcmp(a, "--layers")) {
      st->layers = (int)parse_int(VAL(), a);
    } else if (!strcmp(a, "--track")) {
      st->track = (uint32_t)parse_int(VAL(), a);
    } else if (!strcmp(a, "--au-sizes")) {
      st->au_sizes_path = VAL();
    } else if (!strcmp(a, "--fill-size")) {
      char buf[1024], *tok, *save = NULL;
      snprintf(buf, sizeof(buf), "%s", VAL());
      st->n_fill_sizes = 0;
      for (tok = strtok_r(buf, ",", &save); tok; tok = strtok_r(NULL, ",", &save)) {
        st->fill_sizes = (UINT *)realloc(st->fill_sizes, (st->n_fill_sizes + 1) * sizeof(UINT));
        st->fill_sizes[st->n_fill_sizes] = (UINT)parse_int(tok, "--fill-size");
        if (st->fill_sizes[st->n_fill_sizes] == 0) die("--fill-size must be positive");
        st->n_fill_sizes++;
      }
    } else if (!strcmp(a, "--param")) {
      events_push(&st->init_params, parse_param(VAL(), -1));
    } else if (!strcmp(a, "--param-at")) {
      const char *rest;
      long long at = split_at(VAL(), &rest, a);
      events_push(&st->events, parse_param(rest, at));
    } else if (!strcmp(a, "--flags-at") || !strcmp(a, "--conceal-at") ||
               !strcmp(a, "--flush-at") || !strcmp(a, "--intr-at") ||
               !strcmp(a, "--clrhist-at")) {
      Event e;
      memset(&e, 0, sizeof(e));
      e.kind = EV_FLAGS;
      if (!strcmp(a, "--flags-at")) {
        const char *rest;
        e.at = split_at(VAL(), &rest, a);
        e.value = parse_flags(rest);
      } else {
        e.at = parse_int(VAL(), a);
        e.value = !strcmp(a, "--conceal-at") ? AACDEC_CONCEAL
                  : !strcmp(a, "--flush-at") ? AACDEC_FLUSH
                  : !strcmp(a, "--intr-at")  ? AACDEC_INTR
                                             : AACDEC_CLRHIST;
      }
      events_push(&st->events, e);
    } else if (!strcmp(a, "--conceal-every")) {
      st->conceal_every = parse_int(VAL(), a);
    } else if (!strcmp(a, "--drain")) {
      st->drain = (int)parse_int(VAL(), a);
    } else if (!strcmp(a, "--decode-once")) {
      st->decode_once = 1;
    } else if (!strcmp(a, "--decode-loop")) {
      st->decode_loop_opt = 1;
    } else if (!strcmp(a, "--max-calls")) {
      st->max_calls = parse_int(VAL(), a);
    } else if (!strcmp(a, "--max-errors")) {
      st->max_errors = parse_int(VAL(), a);
    } else if (!strcmp(a, "--out-size")) {
      st->out_size = (INT)parse_int(VAL(), a);
      if (st->out_size < 0) die("--out-size must not be negative");
    } else if (!strcmp(a, "--poison-output")) {
      st->poison = 1;
    } else if (!strcmp(a, "--anc")) {
      st->anc_size = (int)parse_int(VAL(), a);
    } else if (!strcmp(a, "--no-free-bytes")) {
      st->free_bytes = 0;
    } else if (!strcmp(a, "-o") || !strcmp(a, "--pcm")) {
      st->pcm_path = VAL();
    } else if (!strcmp(a, "-l") || !strcmp(a, "--log")) {
      st->log_path = VAL();
    } else if (!strcmp(a, "-h") || !strcmp(a, "--help")) {
      usage();
    } else {
      die("unknown option %s", a);
    }
#undef VAL
  }
  if (!st->input || st->transport == -100) usage();
  if (!st->n_fill_sizes) {
    st->fill_sizes = (UINT *)malloc(sizeof(UINT));
    st->fill_sizes[0] = DEFAULT_FILL_SIZE;
    st->n_fill_sizes = 1;
  }
}

/* ---- API calls with logging ---------------------------------------------------------------- */

static void log_lib_info(St *st) {
  LIB_INFO info[FDK_MODULE_LAST];
  int i, ret;
  /* Zeroed first: drcDec_GetLibInfo does not write the flags of its entry, so they would be
   * whatever the caller's array held. */
  memset(info, 0, sizeof(info));
  FDKinitLibInfo(info);
  ret = aacDecoder_GetLibInfo(info);
  j_begin(&st->j, "get_lib_info");
  j_int(&st->j, "ret", ret);
  j_arr_begin(&st->j, "modules");
  for (i = 0; i < FDK_MODULE_LAST && info[i].module_id != FDK_NONE; i++) {
    j_obj_begin(&st->j, NULL);
    j_int(&st->j, "module_id", info[i].module_id);
    j_str(&st->j, "title", info[i].title ? info[i].title : "");
    j_int(&st->j, "version", info[i].version);
    j_str(&st->j, "versionStr", info[i].versionStr);
    j_uint(&st->j, "flags", info[i].flags);
    j_obj_end(&st->j);
  }
  j_arr_end(&st->j);
  j_end(&st->j);
}

static void set_param(St *st, const Event *e) {
  AAC_DECODER_ERROR ret = aacDecoder_SetParam(st->h, (AACDEC_PARAM)e->id, (INT)e->value);
  j_begin(&st->j, "set_param");
  j_str(&st->j, "param", e->name);
  j_int(&st->j, "id", e->id);
  j_int(&st->j, "value", (INT)e->value);
  j_int(&st->j, "ret", ret);
  j_end(&st->j);
}

static void log_stream_info(St *st, const CStreamInfo *si) {
  int c, n = si->numChannels < 0 ? 0 : si->numChannels > 8 ? 8 : si->numChannels;
  Json *j = &st->j;
  j_obj_begin(j, "info");
  j_int(j, "sampleRate", si->sampleRate);
  j_int(j, "frameSize", si->frameSize);
  j_int(j, "numChannels", si->numChannels);
  if (si->pChannelType) {
    j_arr_begin(j, "pChannelType");
    for (c = 0; c < n; c++) j_arr_int(j, si->pChannelType[c]);
    j_arr_end(j);
  } else {
    j_null(j, "pChannelType");
  }
  if (si->pChannelIndices) {
    j_arr_begin(j, "pChannelIndices");
    for (c = 0; c < n; c++) j_arr_int(j, si->pChannelIndices[c]);
    j_arr_end(j);
  } else {
    j_null(j, "pChannelIndices");
  }
  j_int(j, "aacSampleRate", si->aacSampleRate);
  j_int(j, "profile", si->profile);
  j_int(j, "aot", si->aot);
  j_int(j, "channelConfig", si->channelConfig);
  j_int(j, "bitRate", si->bitRate);
  j_int(j, "aacSamplesPerFrame", si->aacSamplesPerFrame);
  j_int(j, "aacNumChannels", si->aacNumChannels);
  j_int(j, "extAot", si->extAot);
  j_int(j, "extSamplingRate", si->extSamplingRate);
  j_uint(j, "outputDelay", si->outputDelay);
  j_uint(j, "flags", si->flags);
  j_int(j, "epConfig", si->epConfig);
  j_int(j, "numLostAccessUnits", si->numLostAccessUnits);
  j_int(j, "numTotalBytes", si->numTotalBytes);
  j_int(j, "numBadBytes", si->numBadBytes);
  j_int(j, "numTotalAccessUnits", si->numTotalAccessUnits);
  j_int(j, "numBadAccessUnits", si->numBadAccessUnits);
  j_int(j, "drcProgRefLev", si->drcProgRefLev);
  j_int(j, "drcPresMode", si->drcPresMode);
  j_int(j, "outputLoudness", si->outputLoudness);
  j_obj_end(j);
}

/* Poison pattern: odd, so never 0 (the decoder clears buffers with zeros). */
static INT_PCM poison_value(INT i) { return (INT_PCM)(((UINT)i * 40503u + 0x6A5Du) | 1u); }

static AAC_DECODER_ERROR decode(St *st, UINT flags) {
  AAC_DECODER_ERROR ret;
  CStreamInfo *si;
  INT n = 0, i;
  size_t k;
  int valid;

  for (k = 0; k < st->events.n; k++) {
    const Event *e = &st->events.v[k];
    if (e->at != st->n_dec) continue;
    if (e->kind == EV_PARAM) set_param(st, e);
    else flags |= (UINT)e->value;
  }
  if (st->conceal_every > 0 && st->n_dec % st->conceal_every == st->conceal_every - 1)
    flags |= AACDEC_CONCEAL;
  if (st->poison)
    for (i = 0; i < st->out_size; i++) st->out[i] = poison_value(i);

  ret = aacDecoder_DecodeFrame(st->h, st->out, st->out_size, flags);
  si = aacDecoder_GetStreamInfo(st->h);
  valid = IS_OUTPUT_VALID(ret);
  if (valid && si) {
    long long m = (long long)si->frameSize * si->numChannels;
    n = m < 0 ? 0 : m > st->out_size ? st->out_size : (INT)m;
  }

  j_begin(&st->j, "decode_frame");
  j_int(&st->j, "n", st->n_dec);
  j_uint(&st->j, "flags", flags);
  j_int(&st->j, "ret", ret);
  j_int(&st->j, "samples", n);
  if (n > 0) {
    uint8_t *le = (uint8_t *)malloc((size_t)n * 2);
    char hex[65];
    for (i = 0; i < n; i++) {
      le[2 * i] = (uint8_t)(st->out[i] & 0xff);
      le[2 * i + 1] = (uint8_t)((UINT)st->out[i] >> 8);
    }
    if (st->j.f) { /* the hash is only logged; skipping it keeps benchmarks fair */
      sha256_hex(le, (size_t)n * 2, hex);
      j_str(&st->j, "pcm", hex);
    }
    if (st->pcm && fwrite(le, 2, (size_t)n, st->pcm) != (size_t)n) die("cannot write PCM");
    free(le);
    st->samples_total += (unsigned long long)n;
  }
  if (st->poison) {
    INT modified = 0, beyond = 0, beyond_zero = 0, max = -1;
    for (i = 0; i < st->out_size; i++)
      if (st->out[i] != poison_value(i)) {
        modified++;
        if (i >= n) {
          beyond++;
          if (st->out[i] == 0) beyond_zero++;
        }
        max = i;
      }
    j_obj_begin(&st->j, "poison");
    j_int(&st->j, "modified", modified);
    j_int(&st->j, "beyond", beyond);
    j_int(&st->j, "beyond_zero", beyond_zero);
    j_int(&st->j, "max", max);
    j_obj_end(&st->j);
  }
  if (si) log_stream_info(st, si);
  else j_null(&st->j, "info");
  j_end(&st->j);
  st->n_dec++;

  if (valid && st->anc_buf) {
    int idx;
    for (idx = 0;; idx++) {
      UCHAR *p = NULL;
      int size = 0;
      AAC_DECODER_ERROR r = aacDecoder_AncDataGet(st->h, idx, &p, &size);
      j_begin(&st->j, "anc_data_get");
      j_int(&st->j, "index", idx);
      j_int(&st->j, "ret", r);
      j_int(&st->j, "size", size);
      if (p && size > 0) {
        j_int(&st->j, "offset", (long long)(p - st->anc_buf));
        j_hex(&st->j, "data", p, (size_t)size);
      }
      j_end(&st->j);
      if (r != AAC_DEC_OK || size <= 0 || idx >= 16) break;
    }
  }

  /* Loop guard: some error states never consume input nor return AAC_DEC_NOT_ENOUGH_BITS (for
   * example an ADIF stream whose last bits are not a complete raw_data_block returns concealed
   * output with AAC_DEC_DECODE_FRAME_ERROR forever). */
  if (ret == AAC_DEC_OK || ret == AAC_DEC_NOT_ENOUGH_BITS) {
    st->errors_in_row = 0;
  } else {
    st->errors_in_row++;
    if (st->max_errors > 0 && st->errors_in_row >= st->max_errors) st->stop = "errors";
  }
  return ret;
}

static void decode_loop(St *st) {
  for (;;) {
    AAC_DECODER_ERROR ret;
    if (st->max_calls > 0 && st->n_dec >= st->max_calls) {
      st->stop = "max_calls";
      return;
    }
    ret = decode(st, 0);
    if (st->stop || st->decode_once || ret == AAC_DEC_NOT_ENOUGH_BITS) return;
  }
}

/* Fills [p, p + size) and returns the number of bytes consumed. */
static UINT fill(St *st, const uint8_t *p, UINT size) {
  UCHAR *ptr = (UCHAR *)p;
  UINT valid = size, buf_size = size;
  AAC_DECODER_ERROR ret;
  if (st->free_bytes) {
    UINT free_bytes = 0;
    ret = aacDecoder_GetFreeBytes(st->h, &free_bytes);
    j_begin(&st->j, "get_free_bytes");
    j_int(&st->j, "ret", ret);
    j_uint(&st->j, "free", free_bytes);
    j_end(&st->j);
  }
  ret = aacDecoder_Fill(st->h, &ptr, &buf_size, &valid);
  j_begin(&st->j, "fill");
  j_int(&st->j, "n", st->n_fill);
  j_uint(&st->j, "size", size);
  j_int(&st->j, "ret", ret);
  j_uint(&st->j, "bytesValid", valid);
  j_end(&st->j);
  st->n_fill++;
  return size - valid;
}

/* Feeds [p, p + size) completely (or until a stop condition), decoding after each Fill. */
static void feed(St *st, const uint8_t *p, UINT size, int packet, UINT *fill_idx) {
  UINT pos = 0;
  int zero_fills = 0;
  do {
    UINT chunk = size - pos, consumed;
    if (!packet) {
      UINT fs = st->fill_sizes[*fill_idx % (UINT)st->n_fill_sizes];
      if (chunk > fs) chunk = fs;
      (*fill_idx)++;
    }
    consumed = fill(st, p + pos, chunk);
    pos += consumed;
    if (chunk > 0 && consumed == 0) {
      if (++zero_fills >= 2) {
        st->stop = "stall";
        return;
      }
    } else {
      zero_fills = 0;
    }
    decode_loop(st);
  } while (!st->stop && pos < size);
}

int main(int argc, char **argv) {
  St st;
  uint8_t *data;
  size_t size;
  Mp4Track trk;
  uint32_t *au_sizes = NULL;
  size_t n_au = 0, k;
  UINT fill_idx = 0;

  memset(&st, 0, sizeof(st));
  parse_args(&st, argc, argv);
  data = read_file(st.input, &size);
  st.j.f = open_out(st.log_path);
  st.pcm = open_out(st.pcm_path);
  st.out = (INT_PCM *)calloc((size_t)st.out_size + 1, sizeof(INT_PCM));
  if (!st.out) die("out of memory");

  if (st.au_sizes_path) {
    size_t n;
    char *txt = (char *)read_file(st.au_sizes_path, &n), *line, *save = NULL;
    txt = (char *)realloc(txt, n + 1);
    txt[n] = 0;
    for (line = strtok_r(txt, "\r\n", &save); line; line = strtok_r(NULL, "\r\n", &save)) {
      if (!*line) continue;
      au_sizes = (uint32_t *)realloc(au_sizes, (n_au + 1) * sizeof(uint32_t));
      au_sizes[n_au++] = (uint32_t)parse_int(line, "--au-sizes entry");
    }
    free(txt);
  }

  log_lib_info(&st);

  if (st.mp4) {
    char err[256];
    if (mp4_demux(data, size, st.track, &trk, err, sizeof(err))) {
      j_begin(&st.j, "demux");
      j_str(&st.j, "error", "unsupported");
      j_end(&st.j);
      fprintf(stderr, "fdkdec-ref: %s: %s\n", st.input, err);
      return 3;
    }
    j_begin(&st.j, "demux");
    j_uint(&st.j, "track", trk.track_id);
    j_str(&st.j, "codec", trk.codec);
    j_int(&st.j, "oti", trk.oti);
    j_hex(&st.j, "asc", trk.asc, trk.asc_len);
    j_uint(&st.j, "samples", trk.n_samples);
    j_bool(&st.j, "fragmented", trk.fragmented);
    j_end(&st.j);
    if (!st.n_asc && trk.asc_len) {
      st.asc[0] = (uint8_t *)trk.asc;
      st.asc_len[0] = trk.asc_len;
      st.n_asc = 1;
    }
  }

  st.h = aacDecoder_Open((TRANSPORT_TYPE)st.transport, (UINT)st.layers);
  j_begin(&st.j, "open");
  j_int(&st.j, "transport", st.transport);
  j_int(&st.j, "layers", st.layers);
  j_bool(&st.j, "ok", st.h != NULL);
  j_end(&st.j);
  if (!st.h) {
    fprintf(stderr, "fdkdec-ref: aacDecoder_Open failed\n");
    return 1;
  }

  if (st.n_asc) {
    UCHAR *conf[8] = {0};
    UINT len[8] = {0};
    AAC_DECODER_ERROR ret;
    int l;
    for (l = 0; l < st.n_asc; l++) {
      conf[l] = st.asc[l];
      len[l] = st.asc_len[l];
    }
    ret = aacDecoder_ConfigRaw(st.h, conf, len);
    j_begin(&st.j, "config_raw");
    j_arr_begin(&st.j, "length");
    for (l = 0; l < st.n_asc; l++) j_arr_int(&st.j, len[l]);
    j_arr_end(&st.j);
    j_int(&st.j, "ret", ret);
    j_end(&st.j);
  }

  for (k = 0; k < st.init_params.n; k++) set_param(&st, &st.init_params.v[k]);

  if (st.anc_size > 0) {
    AAC_DECODER_ERROR ret;
    st.anc_buf = (UCHAR *)calloc((size_t)st.anc_size, 1);
    ret = aacDecoder_AncDataInit(st.h, st.anc_buf, st.anc_size);
    j_begin(&st.j, "anc_data_init");
    j_int(&st.j, "size", st.anc_size);
    j_int(&st.j, "ret", ret);
    j_end(&st.j);
  }

  /* Packet input: one DecodeFrame call per access unit, as applications such as FFmpeg do. A
   * decode loop would call DecodeFrame on an empty buffer, which some configurations (USAC)
   * answer with AAC_DEC_DECODE_FRAME_ERROR instead of AAC_DEC_NOT_ENOUGH_BITS. */
  int packet_once = (st.mp4 || au_sizes) && !st.decode_loop_opt && !st.decode_once;
  if (packet_once) st.decode_once = 1;

  if (st.mp4) {
    for (k = 0; k < trk.n_samples && !st.stop; k++)
      feed(&st, data + trk.samples[k].offset, trk.samples[k].size, 1, &fill_idx);
  } else if (au_sizes) {
    size_t off = 0;
    for (k = 0; k < n_au && !st.stop; k++) {
      if (au_sizes[k] > size - off) die("--au-sizes exceed the input size");
      feed(&st, data + off, au_sizes[k], 1, &fill_idx);
      off += au_sizes[k];
    }
  } else {
    size_t off = 0;
    while (off < size && !st.stop) {
      UINT part = size - off > 0x40000000 ? 0x40000000 : (UINT)(size - off);
      feed(&st, data + off, part, 0, &fill_idx);
      off += part;
    }
  }

  /* --decode-once: the access units still buffered after the last Fill call are decoded by a
   * normal decode loop. */
  if (!st.stop && st.decode_once && !packet_once) {
    st.decode_once = 0;
    decode_loop(&st);
  }

  if (!st.stop) {
    int d;
    for (d = 0; d < st.drain && !st.stop; d++) {
      if (st.max_calls > 0 && st.n_dec >= st.max_calls) {
        st.stop = "max_calls";
        break;
      }
      decode(&st, AACDEC_FLUSH);
    }
  }

  j_begin(&st.j, "end");
  j_str(&st.j, "reason", st.stop ? st.stop : "eof");
  j_int(&st.j, "fills", st.n_fill);
  j_int(&st.j, "decodes", st.n_dec);
  j_uint(&st.j, "samples", st.samples_total);
  j_end(&st.j);

  aacDecoder_Close(st.h);
  j_begin(&st.j, "close");
  j_end(&st.j);

  if (st.pcm && st.pcm != stdout) fclose(st.pcm);
  if (st.j.f && st.j.f != stdout) fclose(st.j.f);
  return 0;
}
