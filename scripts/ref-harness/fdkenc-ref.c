/* fdkenc-ref: reference encoder harness of fdk-aac-rs.
 *
 * Drives the aacEnc* API of the reference library with a call sequence fully determined by the
 * command line, and writes the bitstream and a JSON Lines log of every API call. The Rust CLI
 * (fdk-aac-rs-enc) implements the same interface. The interface and the log format are specified
 * in PORTING.md, "Reference harness"; keep both in sync. */
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "aacenc_lib.h"
#include "common.h"

#define DEFAULT_OUT_SIZE 20480

static const struct {
  const char *name;
  AACENC_PARAM id;
} PARAMS[] = {
    {"AACENC_AOT", AACENC_AOT},
    {"AACENC_BITRATE", AACENC_BITRATE},
    {"AACENC_BITRATEMODE", AACENC_BITRATEMODE},
    {"AACENC_SAMPLERATE", AACENC_SAMPLERATE},
    {"AACENC_SBR_MODE", AACENC_SBR_MODE},
    {"AACENC_GRANULE_LENGTH", AACENC_GRANULE_LENGTH},
    {"AACENC_CHANNELMODE", AACENC_CHANNELMODE},
    {"AACENC_CHANNELORDER", AACENC_CHANNELORDER},
    {"AACENC_SBR_RATIO", AACENC_SBR_RATIO},
    {"AACENC_AFTERBURNER", AACENC_AFTERBURNER},
    {"AACENC_BANDWIDTH", AACENC_BANDWIDTH},
    {"AACENC_PEAK_BITRATE", AACENC_PEAK_BITRATE},
    {"AACENC_TRANSMUX", AACENC_TRANSMUX},
    {"AACENC_HEADER_PERIOD", AACENC_HEADER_PERIOD},
    {"AACENC_SIGNALING_MODE", AACENC_SIGNALING_MODE},
    {"AACENC_TPSUBFRAMES", AACENC_TPSUBFRAMES},
    {"AACENC_AUDIOMUXVER", AACENC_AUDIOMUXVER},
    {"AACENC_PROTECTION", AACENC_PROTECTION},
    {"AACENC_ANCILLARY_BITRATE", AACENC_ANCILLARY_BITRATE},
    {"AACENC_METADATA_MODE", AACENC_METADATA_MODE},
    {"AACENC_CONTROL_STATE", AACENC_CONTROL_STATE},
};
#define N_PARAMS (sizeof(PARAMS) / sizeof(PARAMS[0]))

/* Fields of AACENC_MetaData, settable with --meta KEY=VALUE. */
enum { F_INT, F_UCHAR, F_SCHAR };
static const struct {
  const char *name;
  size_t offset;
  int type;
} META[] = {
    {"drc_profile", offsetof(AACENC_MetaData, drc_profile), F_INT},
    {"comp_profile", offsetof(AACENC_MetaData, comp_profile), F_INT},
    {"drc_TargetRefLevel", offsetof(AACENC_MetaData, drc_TargetRefLevel), F_INT},
    {"comp_TargetRefLevel", offsetof(AACENC_MetaData, comp_TargetRefLevel), F_INT},
    {"prog_ref_level_present", offsetof(AACENC_MetaData, prog_ref_level_present), F_INT},
    {"prog_ref_level", offsetof(AACENC_MetaData, prog_ref_level), F_INT},
    {"PCE_mixdown_idx_present", offsetof(AACENC_MetaData, PCE_mixdown_idx_present), F_UCHAR},
    {"ETSI_DmxLvl_present", offsetof(AACENC_MetaData, ETSI_DmxLvl_present), F_UCHAR},
    {"centerMixLevel", offsetof(AACENC_MetaData, centerMixLevel), F_SCHAR},
    {"surroundMixLevel", offsetof(AACENC_MetaData, surroundMixLevel), F_SCHAR},
    {"dolbySurroundMode", offsetof(AACENC_MetaData, dolbySurroundMode), F_UCHAR},
    {"drcPresentationMode", offsetof(AACENC_MetaData, drcPresentationMode), F_UCHAR},
    {"extAncDataEnable", offsetof(AACENC_MetaData, ExtMetaData.extAncDataEnable), F_UCHAR},
    {"extDownmixLevelEnable", offsetof(AACENC_MetaData, ExtMetaData.extDownmixLevelEnable),
     F_UCHAR},
    {"extDownmixLevel_A", offsetof(AACENC_MetaData, ExtMetaData.extDownmixLevel_A), F_UCHAR},
    {"extDownmixLevel_B", offsetof(AACENC_MetaData, ExtMetaData.extDownmixLevel_B), F_UCHAR},
    {"dmxGainEnable", offsetof(AACENC_MetaData, ExtMetaData.dmxGainEnable), F_UCHAR},
    {"dmxGain5", offsetof(AACENC_MetaData, ExtMetaData.dmxGain5), F_INT},
    {"dmxGain2", offsetof(AACENC_MetaData, ExtMetaData.dmxGain2), F_INT},
    {"lfeDmxEnable", offsetof(AACENC_MetaData, ExtMetaData.lfeDmxEnable), F_UCHAR},
    {"lfeDmxLevel", offsetof(AACENC_MetaData, ExtMetaData.lfeDmxLevel), F_UCHAR},
};
#define N_META (sizeof(META) / sizeof(META[0]))

enum { EV_PARAM, EV_META };

typedef struct {
  const char *input, *out_path, *log_path, *au_sizes_path, *anc_path;
  int raw, rate, channels, modules, max_channels, no_init, use_meta;
  long long frame_input;
  INT out_size;
  EventList params, events;
  AACENC_MetaData meta;
} Opts;

static int param_index(const char *name) {
  size_t i;
  for (i = 0; i < N_PARAMS; i++)
    if (!strcmp(PARAMS[i].name, name)) return (int)i;
  return -1;
}

static Event parse_param(const char *s, long long at) {
  Event e;
  const char *eq = strchr(s, '=');
  int k;
  memset(&e, 0, sizeof(e));
  if (!eq || (size_t)(eq - s) >= sizeof(e.name)) die("expected NAME=VALUE: '%s'", s);
  memcpy(e.name, s, (size_t)(eq - s));
  e.name[eq - s] = 0;
  k = param_index(e.name);
  if (k >= 0) {
    e.id = PARAMS[k].id;
  } else {
    size_t i;
    e.id = parse_int(e.name, "parameter (unknown name)");
    for (i = 0; i < N_PARAMS; i++)
      if (PARAMS[i].id == e.id) snprintf(e.name, sizeof(e.name), "%s", PARAMS[i].name);
  }
  e.value = parse_int(eq + 1, "parameter value");
  e.kind = EV_PARAM;
  e.at = at;
  return e;
}

static Event parse_meta(const char *s, long long at) {
  Event e;
  const char *eq = strchr(s, '=');
  size_t i;
  memset(&e, 0, sizeof(e));
  if (!eq || (size_t)(eq - s) >= sizeof(e.name)) die("expected KEY=VALUE: '%s'", s);
  memcpy(e.name, s, (size_t)(eq - s));
  e.name[eq - s] = 0;
  e.id = -1;
  for (i = 0; i < N_META; i++)
    if (!strcmp(META[i].name, e.name)) e.id = (long long)i;
  if (e.id < 0) die("unknown metadata field '%s'", e.name);
  e.value = parse_int(eq + 1, "metadata value");
  e.kind = EV_META;
  e.at = at;
  return e;
}

static void apply_meta(AACENC_MetaData *m, const Event *e) {
  char *p = (char *)m + META[e->id].offset;
  switch (META[e->id].type) {
    case F_INT: {
      INT v = (INT)e->value;
      memcpy(p, &v, sizeof(v));
      break;
    }
    case F_UCHAR: *(UCHAR *)p = (UCHAR)e->value; break;
    default: *(SCHAR *)p = (SCHAR)e->value; break;
  }
}

static void usage(void) {
  fputs(
      "usage: fdkenc-ref [options] INPUT\n"
      "  -f, --format wav|s16le  input format (default wav)\n"
      "  --rate N, --channels N  sample rate and channels of s16le input\n"
      "  --param NAME=VALUE      aacEncoder_SetParam after aacEncOpen (repeatable)\n"
      "  --param-at N:NAME=VALUE aacEncoder_SetParam before aacEncEncode call N\n"
      "  --modules N             encModules of aacEncOpen (default 0)\n"
      "  --max-channels N        maxChannels of aacEncOpen (default: input channels)\n"
      "  --no-init               no aacEncEncode(h, NULL, NULL, NULL, NULL) call\n"
      "  --frame-input N         samples per channel per call (default: frameLength)\n"
      "  --anc FILE              ancillary data: line N holds the hex bytes of call N\n"
      "  --meta KEY=VALUE        AACENC_MetaData field, passed on every call\n"
      "  --meta-at N:KEY=VALUE   changes a metadata field from call N on\n"
      "  --out-size N            output buffer size in bytes (default 20480)\n"
      "  -o, --output FILE       bitstream output (all output bytes in call order)\n"
      "  --au-sizes FILE         size of each non-empty output, one per line\n"
      "  -l, --log FILE          JSON Lines log ('-': stdout)\n",
      stderr);
  exit(2);
}

static void parse_args(Opts *o, int argc, char **argv) {
  int i;
  o->out_size = DEFAULT_OUT_SIZE;
  for (i = 1; i < argc; i++) {
    const char *a = argv[i], *v = NULL;
    char opt[64];
    const char *eq;
    if (a[0] != '-' || strcmp(a, "-") == 0) {
      if (o->input) die("more than one input");
      o->input = a;
      continue;
    }
    eq = strchr(a, '=');
    if (a[1] == '-' && eq && (size_t)(eq - a) < sizeof(opt)) {
      memcpy(opt, a, (size_t)(eq - a));
      opt[eq - a] = 0;
      v = eq + 1;
      a = opt;
    }
#define VAL() (v ? v : (i + 1 < argc ? argv[++i] : (die("missing value for %s", a), "")))
    if (!strcmp(a, "-f") || !strcmp(a, "--format")) {
      const char *f = VAL();
      if (!strcmp(f, "s16le")) o->raw = 1;
      else if (!strcmp(f, "wav")) o->raw = 0;
      else die("unknown format %s", f);
    } else if (!strcmp(a, "--rate")) {
      o->rate = (int)parse_int(VAL(), a);
    } else if (!strcmp(a, "--channels")) {
      o->channels = (int)parse_int(VAL(), a);
    } else if (!strcmp(a, "--param")) {
      events_push(&o->params, parse_param(VAL(), -1));
    } else if (!strcmp(a, "--param-at")) {
      const char *rest;
      long long at = split_at(VAL(), &rest, a);
      events_push(&o->events, parse_param(rest, at));
    } else if (!strcmp(a, "--modules")) {
      o->modules = (int)parse_int(VAL(), a);
    } else if (!strcmp(a, "--max-channels")) {
      o->max_channels = (int)parse_int(VAL(), a);
    } else if (!strcmp(a, "--no-init")) {
      o->no_init = 1;
    } else if (!strcmp(a, "--frame-input")) {
      o->frame_input = parse_int(VAL(), a);
      if (o->frame_input <= 0) die("--frame-input must be positive");
    } else if (!strcmp(a, "--anc")) {
      o->anc_path = VAL();
    } else if (!strcmp(a, "--meta")) {
      Event e = parse_meta(VAL(), -1);
      apply_meta(&o->meta, &e);
      o->use_meta = 1;
    } else if (!strcmp(a, "--meta-at")) {
      const char *rest;
      long long at = split_at(VAL(), &rest, a);
      events_push(&o->events, parse_meta(rest, at));
      o->use_meta = 1;
    } else if (!strcmp(a, "--out-size")) {
      o->out_size = (INT)parse_int(VAL(), a);
      if (o->out_size < 0) die("--out-size must not be negative");
    } else if (!strcmp(a, "-o") || !strcmp(a, "--output")) {
      o->out_path = VAL();
    } else if (!strcmp(a, "--au-sizes")) {
      o->au_sizes_path = VAL();
    } else if (!strcmp(a, "-l") || !strcmp(a, "--log")) {
      o->log_path = VAL();
    } else if (!strcmp(a, "-h") || !strcmp(a, "--help")) {
      usage();
    } else {
      die("unknown option %s", a);
    }
#undef VAL
  }
  if (!o->input) usage();
  if (o->no_init && !o->frame_input) die("--no-init requires --frame-input");
}

static uint32_t le32(const uint8_t *p) {
  return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

/* Parses a 16-bit PCM WAV file (RIFF chunks, WAVE_FORMAT_PCM or WAVE_FORMAT_EXTENSIBLE). */
static void parse_wav(const uint8_t *d, size_t n, int *rate, int *channels, const uint8_t **pcm,
                      size_t *pcm_bytes) {
  size_t pos = 12;
  int have_fmt = 0;
  if (n < 12 || memcmp(d, "RIFF", 4) || memcmp(d + 8, "WAVE", 4)) die("not a RIFF/WAVE file");
  while (pos + 8 <= n) {
    uint32_t len = le32(d + pos + 4);
    const uint8_t *c = d + pos + 8;
    size_t avail = n - pos - 8;
    if (!memcmp(d + pos, "fmt ", 4)) {
      unsigned fmt, bits;
      if (len < 16 || avail < 16) die("WAV: short 'fmt ' chunk");
      fmt = c[0] | c[1] << 8;
      *channels = c[2] | c[3] << 8;
      *rate = (int)le32(c + 4);
      bits = c[14] | c[15] << 8;
      if (fmt == 0xfffe) {
        if (len < 28 || avail < 28) die("WAV: short WAVE_FORMAT_EXTENSIBLE");
        fmt = c[24] | c[25] << 8;
      }
      if (fmt != 1) die("WAV: unsupported format %u", fmt);
      if (bits != 16) die("WAV: unsupported sample size %u", bits);
      have_fmt = 1;
    } else if (!memcmp(d + pos, "data", 4)) {
      if (!have_fmt) die("WAV: 'data' before 'fmt '");
      *pcm = c;
      *pcm_bytes = (len == 0 || len >= 0x7fff0000u || len > avail) ? avail : len;
      return;
    }
    if (len > avail) break;
    pos += 8 + (size_t)len + (len & 1);
  }
  die("WAV: no 'data' chunk");
}

static void log_lib_info(Json *j) {
  LIB_INFO info[FDK_MODULE_LAST];
  int i;
  AACENC_ERROR ret;
  /* Zeroed first: drcDec_GetLibInfo does not write the flags of its entry, so they would be
   * whatever the caller's array held. */
  memset(info, 0, sizeof(info));
  FDKinitLibInfo(info);
  ret = aacEncGetLibInfo(info);
  j_begin(j, "get_lib_info");
  j_int(j, "ret", ret);
  j_arr_begin(j, "modules");
  for (i = 0; i < FDK_MODULE_LAST && info[i].module_id != FDK_NONE; i++) {
    j_obj_begin(j, NULL);
    j_int(j, "module_id", info[i].module_id);
    j_str(j, "title", info[i].title ? info[i].title : "");
    j_int(j, "version", info[i].version);
    j_str(j, "versionStr", info[i].versionStr);
    j_uint(j, "flags", info[i].flags);
    j_obj_end(j);
  }
  j_arr_end(j);
  j_end(j);
}

static void set_param(Json *j, HANDLE_AACENCODER h, const Event *e) {
  AACENC_ERROR ret = aacEncoder_SetParam(h, (AACENC_PARAM)e->id, (UINT)e->value);
  j_begin(j, "set_param");
  j_str(j, "param", e->name);
  j_int(j, "id", e->id);
  j_uint(j, "value", (UINT)e->value);
  j_int(j, "ret", ret);
  j_end(j);
}

static AACENC_InfoStruct log_info(Json *j, HANDLE_AACENCODER h) {
  AACENC_InfoStruct info;
  AACENC_ERROR ret;
  memset(&info, 0, sizeof(info));
  ret = aacEncInfo(h, &info);
  j_begin(j, "info");
  j_int(j, "ret", ret);
  j_obj_begin(j, "info");
  j_uint(j, "maxOutBufBytes", info.maxOutBufBytes);
  j_uint(j, "maxAncBytes", info.maxAncBytes);
  j_uint(j, "inBufFillLevel", info.inBufFillLevel);
  j_uint(j, "inputChannels", info.inputChannels);
  j_uint(j, "frameLength", info.frameLength);
  j_uint(j, "nDelay", info.nDelay);
  j_uint(j, "nDelayCore", info.nDelayCore);
  j_hex(j, "confBuf", info.confBuf, info.confSize <= 64 ? info.confSize : 64);
  j_uint(j, "confSize", info.confSize);
  j_obj_end(j);
  j_end(j);
  return info;
}

static void log_get_params(Json *j, HANDLE_AACENCODER h) {
  size_t i;
  for (i = 0; i < N_PARAMS; i++) {
    UINT v = aacEncoder_GetParam(h, PARAMS[i].id);
    j_begin(j, "get_param");
    j_str(j, "param", PARAMS[i].name);
    j_int(j, "id", PARAMS[i].id);
    j_uint(j, "value", v);
    j_end(j);
  }
}

static int channel_mode(int channels) {
  static const int modes[] = {MODE_INVALID, MODE_1,       MODE_2,   MODE_1_2,     MODE_1_2_1,
                              MODE_1_2_2,   MODE_1_2_2_1, MODE_6_1, MODE_7_1_BACK};
  return channels >= 1 && channels <= 8 ? modes[channels] : MODE_INVALID;
}

int main(int argc, char **argv) {
  Opts o;
  Json j = {0, 0};
  uint8_t *file, *anc_txt = NULL;
  size_t file_size, pcm_bytes = 0, k;
  const uint8_t *pcm_le = NULL;
  INT_PCM *pcm, dummy[8] = {0};
  long long n_samples, pos = 0, call = 0, n_enc = 0;
  int rate = 0, channels = 0, reinit = 0, zero_calls = 0;
  char **anc_lines = NULL;
  size_t n_anc = 0;
  HANDLE_AACENCODER h = NULL;
  AACENC_ERROR ret;
  AACENC_InfoStruct info;
  UCHAR *outbuf;
  FILE *out, *au;
  const char *stop = "eof";

  memset(&o, 0, sizeof(o));
  parse_args(&o, argc, argv);
  file = read_file(o.input, &file_size);
  if (o.raw) {
    if (o.rate <= 0 || o.channels <= 0) die("s16le input needs --rate and --channels");
    rate = o.rate;
    channels = o.channels;
    pcm_le = file;
    pcm_bytes = file_size;
  } else {
    parse_wav(file, file_size, &rate, &channels, &pcm_le, &pcm_bytes);
  }
  if (channels <= 0) die("invalid channel count");
  n_samples = (long long)(pcm_bytes / 2);
  pcm = (INT_PCM *)malloc((size_t)(n_samples ? n_samples : 1) * sizeof(INT_PCM));
  if (!pcm) die("out of memory");
  for (k = 0; k < (size_t)n_samples; k++)
    pcm[k] = (INT_PCM)(int16_t)(pcm_le[2 * k] | (pcm_le[2 * k + 1] << 8));

  if (o.anc_path) {
    size_t n;
    char *line, *p;
    anc_txt = read_file(o.anc_path, &n);
    anc_txt = (uint8_t *)realloc(anc_txt, n + 1);
    anc_txt[n] = 0;
    /* Keep empty lines: line N belongs to call N. */
    for (p = (char *)anc_txt;;) {
      line = p;
      p = strchr(p, '\n');
      if (p) *p++ = 0;
      if (!p && !*line) break;
      anc_lines = (char **)realloc(anc_lines, (n_anc + 1) * sizeof(char *));
      anc_lines[n_anc++] = line;
      if (!p) break;
    }
  }

  j.f = open_out(o.log_path);
  out = open_out(o.out_path);
  au = open_out(o.au_sizes_path);
  outbuf = (UCHAR *)malloc(o.out_size > 0 ? (size_t)o.out_size : 1);

  log_lib_info(&j);

  {
    int max_ch = o.max_channels ? o.max_channels : channels;
    ret = aacEncOpen(&h, (UINT)o.modules, (UINT)max_ch);
    j_begin(&j, "open");
    j_int(&j, "encModules", o.modules);
    j_int(&j, "maxChannels", max_ch);
    j_int(&j, "ret", ret);
    j_end(&j);
    if (ret != AACENC_OK) {
      fprintf(stderr, "fdkenc-ref: aacEncOpen failed\n");
      return 1;
    }
  }

  {
    int have_rate = 0, have_mode = 0;
    for (k = 0; k < o.params.n; k++) {
      set_param(&j, h, &o.params.v[k]);
      if (o.params.v[k].id == AACENC_SAMPLERATE) have_rate = 1;
      if (o.params.v[k].id == AACENC_CHANNELMODE) have_mode = 1;
    }
    if (!have_rate) {
      Event e;
      memset(&e, 0, sizeof(e));
      snprintf(e.name, sizeof(e.name), "AACENC_SAMPLERATE");
      e.id = AACENC_SAMPLERATE;
      e.value = rate;
      set_param(&j, h, &e);
    }
    if (!have_mode) {
      Event e;
      memset(&e, 0, sizeof(e));
      snprintf(e.name, sizeof(e.name), "AACENC_CHANNELMODE");
      e.id = AACENC_CHANNELMODE;
      e.value = channel_mode(channels);
      set_param(&j, h, &e);
    }
  }

  memset(&info, 0, sizeof(info));
  if (!o.no_init) {
    ret = aacEncEncode(h, NULL, NULL, NULL, NULL);
    j_begin(&j, "init");
    j_int(&j, "ret", ret);
    j_end(&j);
    info = log_info(&j, h);
    log_get_params(&j, h);
  }
  if (!o.frame_input) {
    if (info.frameLength == 0) {
      stop = "no_frame_length";
      goto end;
    }
    o.frame_input = info.frameLength;
  }

  for (;; call++) {
    AACENC_BufDesc in_buf, out_buf;
    AACENC_InArgs in_args;
    AACENC_OutArgs out_args;
    void *in_ptr[3], *out_ptr = outbuf;
    INT in_id[3], in_size[3], in_el[3], out_id = OUT_BITSTREAM_DATA, out_size = o.out_size,
        out_el = 1;
    uint8_t *anc = NULL;
    size_t anc_len = 0;
    long long chunk = o.frame_input * channels;
    int nb = 0;

    for (k = 0; k < o.events.n; k++) {
      const Event *e = &o.events.v[k];
      if (e->at != call) continue;
      if (e->kind == EV_PARAM) {
        set_param(&j, h, e);
        reinit = 1;
      } else {
        apply_meta(&o.meta, e);
      }
    }

    if (chunk > n_samples - pos) chunk = n_samples - pos;
    memset(&in_buf, 0, sizeof(in_buf));
    memset(&out_buf, 0, sizeof(out_buf));
    memset(&in_args, 0, sizeof(in_args));
    memset(&out_args, 0, sizeof(out_args));
    in_ptr[nb] = chunk > 0 ? (void *)(pcm + pos) : (void *)dummy;
    in_id[nb] = IN_AUDIO_DATA;
    in_size[nb] = (INT)(chunk * 2);
    in_el[nb] = 2;
    nb++;
    in_args.numInSamples = chunk > 0 ? (INT)chunk : -1;
    if (o.anc_path) {
      if ((size_t)call < n_anc) anc_len = parse_hex(anc_lines[call], &anc, "--anc line");
      in_ptr[nb] = anc ? (void *)anc : (void *)dummy;
      in_id[nb] = IN_ANCILLRY_DATA;
      in_size[nb] = (INT)anc_len;
      in_el[nb] = 1;
      nb++;
      in_args.numAncBytes = (INT)anc_len;
    }
    if (o.use_meta) {
      in_ptr[nb] = &o.meta;
      in_id[nb] = IN_METADATA_SETUP;
      in_size[nb] = (INT)sizeof(o.meta);
      in_el[nb] = 1;
      nb++;
    }
    in_buf.numBufs = nb;
    in_buf.bufs = in_ptr;
    in_buf.bufferIdentifiers = in_id;
    in_buf.bufSizes = in_size;
    in_buf.bufElSizes = in_el;
    out_buf.numBufs = 1;
    out_buf.bufs = &out_ptr;
    out_buf.bufferIdentifiers = &out_id;
    out_buf.bufSizes = &out_size;
    out_buf.bufElSizes = &out_el;

    ret = aacEncEncode(h, &in_buf, &out_buf, &in_args, &out_args);
    n_enc++;

    j_begin(&j, "encode");
    j_int(&j, "n", call);
    j_obj_begin(&j, "inArgs");
    j_int(&j, "numInSamples", in_args.numInSamples);
    j_int(&j, "numAncBytes", in_args.numAncBytes);
    j_obj_end(&j);
    j_int(&j, "ret", ret);
    j_obj_begin(&j, "outArgs");
    j_int(&j, "numOutBytes", out_args.numOutBytes);
    j_int(&j, "numInSamples", out_args.numInSamples);
    j_int(&j, "numAncBytes", out_args.numAncBytes);
    j_int(&j, "bitResState", out_args.bitResState);
    j_obj_end(&j);
    if (out_args.numOutBytes > 0 && out_args.numOutBytes <= o.out_size) {
      char hex[65];
      if (j.f) { /* the hash is only logged */
        sha256_hex(outbuf, (size_t)out_args.numOutBytes, hex);
        j_str(&j, "out", hex);
      }
      if (out && fwrite(outbuf, 1, (size_t)out_args.numOutBytes, out) !=
                     (size_t)out_args.numOutBytes)
        die("cannot write output");
      if (au) fprintf(au, "%d\n", out_args.numOutBytes);
    }
    j_end(&j);
    free(anc);

    if (ret == AACENC_ENCODE_EOF) break;
    if (ret != AACENC_OK) {
      stop = "error";
      break;
    }
    if (reinit || (o.no_init && call == 0)) {
      log_info(&j, h);
      log_get_params(&j, h);
      reinit = 0;
    }
    pos += out_args.numInSamples;
    if (chunk > 0 && out_args.numInSamples == 0 && out_args.numOutBytes == 0) {
      if (++zero_calls >= 2) {
        stop = "stall";
        break;
      }
    } else {
      zero_calls = 0;
    }
  }

end:
  j_begin(&j, "end");
  j_str(&j, "reason", stop);
  j_int(&j, "encodes", n_enc);
  j_end(&j);
  ret = aacEncClose(&h);
  j_begin(&j, "close");
  j_int(&j, "ret", ret);
  j_end(&j);

  if (out && out != stdout) fclose(out);
  if (au && au != stdout) fclose(au);
  if (j.f && j.f != stdout) fclose(j.f);
  return 0;
}
