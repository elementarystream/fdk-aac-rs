/* fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
 * A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
 *
 * C-level differential test of the decoder API (scripts/capi_test.sh). The program drives
 * aacdecoder_lib.h through call sequences over the streams given on the command line and
 * prints every result: return codes, bytesValid, free bytes, the whole output buffer of every
 * aacDecoder_DecodeFrame() call (as a hash; the buffer is filled with a pattern before each
 * call, so samples the decoder writes or clears beyond the frame count too), every CStreamInfo
 * field including the eight entries of pChannelType and pChannelIndices (read through the
 * pointer of the first aacDecoder_GetStreamInfo() call, which must stay valid), ancillary data
 * (offsets into the buffer, sizes and contents), the NULL paths C handles, parameters, and
 * LIB_INFO. scripts/capi_test.sh links it against fdk-aac-rs-capi (static and shared) and against
 * the reference library and compares the outputs.
 *
 * aacDecoder_RawISOBMFFData() is called only when built with -DWITH_RAW_ISOBMFF (the static
 * libraries; the shared library does not export it). Those lines start with "isobmff".
 *
 * With FDK_AAC_RS_THREADS > 1 (frame pipelining of fdk-aac-rs: every frame comes one call
 * later), the calls do not depend on the results other than through the input consumed (for
 * both libraries), the decoder is drained at the end of each stream, and the program prints the
 * frames (the results other than AAC_DEC_NOT_ENOUGH_BITS, with their stream information and
 * ancillary data) without the calls that returned them, and no stream information between
 * calls once frames were returned: the output must then be that of the reference as well.
 *
 * Not printed: LIB_INFO build_date/build_time contents (PORTING.md, "Deviations from the
 * reference"; only whether they were written).
 *
 * Usage: capi_dec_diff [--builtin] [CASE...]
 *   CASE = NAME|TRANSPORT|FILE[|OPTIONS]
 *   TRANSPORT: adts, adif, loas, latm1, mp4 (the first audio track, raw access units with the
 *   AudioSpecificConfig of the file)
 *   OPTIONS, comma-separated: chunk=N[:N...] (Fill sizes, cycled), param=ID:VALUE (repeatable,
 *   hexadecimal ID), anc=SIZE (ancillary data buffer), out=SIZE (timeDataSize), corrupt=SEED
 *   (flips bytes after the first 600), frames=N (stop after N decode calls). */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "aacdecoder_lib.h"
#include "aacenc_lib.h"
#include "mp4demux.h"

#define MAXP 8

typedef struct {
  char name[128];
  int transport; /* TRANSPORT_TYPE, or -2 for mp4 */
  const char *file;
  int chunks[8], n_chunks;
  int params[MAXP][2], n_params;
  int anc;
  int out_size;
  unsigned corrupt;
  int frames;
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

/* The state of one decoder run. */
typedef struct {
  HANDLE_AACDECODER h;
  const CStreamInfo *si; /* the pointer of the first aacDecoder_GetStreamInfo() */
  INT_PCM *pcm;
  int out_size;
  UCHAR *anc;
  int anc_size;
  int k; /* decode calls */
} Run;

static void print_si(const Run *r) {
  const CStreamInfo *s = r->si;
  int i;
  printf("  si same=%d rate=%d frame=%d ch=%d type=", aacDecoder_GetStreamInfo(r->h) == s,
         s->sampleRate, s->frameSize, s->numChannels);
  for (i = 0; i < 8; i++) printf("%d.", (int)s->pChannelType[i]);
  printf(" idx=");
  for (i = 0; i < 8; i++) printf("%d.", s->pChannelIndices[i]);
  printf(" aac_rate=%d profile=%d aot=%d cc=%d br=%d aac_spf=%d aac_ch=%d ext_aot=%d ext_rate=%d"
         " delay=%u flags=%x ep=%d lost=%d bytes=%lld bad_bytes=%lld aus=%lld bad_aus=%lld"
         " prl=%d pres=%d loud=%d\n",
         s->aacSampleRate, s->profile, (int)s->aot, s->channelConfig, s->bitRate,
         s->aacSamplesPerFrame, s->aacNumChannels, (int)s->extAot, s->extSamplingRate,
         s->outputDelay, s->flags, (int)s->epConfig, s->numLostAccessUnits,
         (long long)s->numTotalBytes, (long long)s->numBadBytes, (long long)s->numTotalAccessUnits,
         (long long)s->numBadAccessUnits, (int)s->drcProgRefLev, (int)s->drcPresMode,
         s->outputLoudness);
}

static void print_anc(Run *r) {
  int i;
  for (i = -1; i < 9; i++) {
    UCHAR *p = (UCHAR *)(uintptr_t)1;
    int size = -7;
    AAC_DECODER_ERROR e = aacDecoder_AncDataGet(r->h, i, &p, &size);
    if (p == NULL) {
      printf("  anc %d rc=%x size=%d ptr=null\n", i, e, size);
    } else {
      long off = (long)(p - r->anc);
      printf("  anc %d rc=%x size=%d off=%ld data=%016llx\n", i, e, size, off,
             (off >= 0 && size >= 0 && off + size <= r->anc_size)
                 ? (unsigned long long)fnv(p, (size_t)size)
                 : 0ULL);
    }
  }
}

/* The flags of decode call k. */
static UINT flags_of(int k) {
  switch (k) {
    case 5: return AACDEC_CONCEAL;
    case 9: return AACDEC_INTR;
    case 13: return AACDEC_CLRHIST;
    default: return 0;
  }
}

/* FDK_AAC_RS_THREADS > 1: fdk-aac-rs decodes with frame pipelining (aacDecoder_Open() reads the
 * variable; the reference ignores it). FDK_AAC_RS_DEC_THREADS takes precedence when it is set
 * (as in aacDecoder_Open()). */
static int pipelined(void) {
  const char *v = getenv("FDK_AAC_RS_DEC_THREADS");
  if (!v) v = getenv("FDK_AAC_RS_THREADS");
  return v && atoi(v) > 1;
}

static AAC_DECODER_ERROR decode(Run *r, UINT flags) {
  AAC_DECODER_ERROR e;
  int i;
  for (i = 0; i < r->out_size; i++) r->pcm[i] = (INT_PCM)(0x5a5a ^ (i * 7));
  e = aacDecoder_DecodeFrame(r->h, r->pcm, r->out_size, flags);
  if (pipelined()) {
    /* Only the frames, without the call that returned them. */
    r->k++;
    if (e == AAC_DEC_NOT_ENOUGH_BITS) return e;
    printf("frame rc=%x pcm=%016llx\n", e,
           (unsigned long long)fnv(r->pcm, (size_t)r->out_size * sizeof(INT_PCM)));
    print_si(r);
    if (r->anc && IS_OUTPUT_VALID(e)) print_anc(r);
    return e;
  }
  printf("decode %d flags=%x rc=%x pcm=%016llx\n", r->k, flags, e,
         (unsigned long long)fnv(r->pcm, (size_t)r->out_size * sizeof(INT_PCM)));
  print_si(r);
  if (r->anc && IS_OUTPUT_VALID(e)) print_anc(r);
  r->k++;
  return e;
}

static UINT fill(Run *r, UCHAR *data, UINT size) {
  UINT valid = size, free_bytes = 0xa5a5a5a5u;
  AAC_DECODER_ERROR e = aacDecoder_GetFreeBytes(r->h, &free_bytes);
  /* (frame pipelining: the frames are printed one call later, so not between the fills) */
  if (!pipelined()) printf("free rc=%x n=%u\n", e, free_bytes);
  e = aacDecoder_Fill(r->h, &data, &size, &valid);
  if (!pipelined()) printf("fill size=%u rc=%x valid=%u\n", size, e, valid);
  return size - valid;
}

static unsigned char *read_file(const char *path, size_t *size) {
  FILE *f = fopen(path, "rb");
  unsigned char *d;
  long n;
  if (!f) return NULL;
  fseek(f, 0, SEEK_END);
  n = ftell(f);
  fseek(f, 0, SEEK_SET);
  d = (unsigned char *)malloc(n > 0 ? (size_t)n : 1);
  if (fread(d, 1, (size_t)n, f) != (size_t)n) {
    fclose(f);
    free(d);
    return NULL;
  }
  fclose(f);
  *size = (size_t)n;
  return d;
}

/* Flips bytes of d[skip..n] (deterministic). */
static void corrupt(unsigned char *d, size_t n, unsigned seed, size_t skip) {
  uint32_t x = seed | 1;
  size_t i;
  for (i = skip; i < n; i++) {
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    if (x % 53 == 0) d[i] ^= (unsigned char)((x >> 8) | 1);
  }
}

static void parse_case(char *spec, Case *c);

/* Runs case c over data (size bytes, released here). */
static void run_data(const Case *c, unsigned char *data, size_t size) {
  Run r;
  size_t pos = 0;
  Mp4Track trk;
  char err[256];
  int i, n_chunk = 0;
  AAC_DECODER_ERROR clear_rc;

  memset(&r, 0, sizeof(r));
  memset(&trk, 0, sizeof(trk));
  if (c->transport == -2 && mp4_demux(data, size, 0, &trk, err, sizeof(err))) {
    printf("mp4: %s\n", err);
    free(data);
    return;
  }
  if (c->corrupt) {
    if (c->transport == -2) {
      for (i = 0; i < (int)trk.n_samples; i++)
        if (i % 5 == 4 && trk.samples[i].offset + trk.samples[i].size <= size)
          corrupt(data + trk.samples[i].offset, trk.samples[i].size, c->corrupt + i, 0);
    } else {
      corrupt(data, size, c->corrupt, 600);
    }
  }
  r.out_size = c->out_size;
  r.pcm = (INT_PCM *)malloc(sizeof(INT_PCM) * (size_t)(r.out_size > 0 ? r.out_size : 1));
  r.h = aacDecoder_Open(c->transport == -2 ? TT_MP4_RAW : (TRANSPORT_TYPE)c->transport, 1);
  printf("open ok=%d\n", r.h != NULL);
  if (!r.h) goto done;
  r.si = aacDecoder_GetStreamInfo(r.h);
  print_si(&r);
  if (c->transport == -2) {
    UCHAR *conf = (UCHAR *)trk.asc;
    UINT len = trk.asc_len;
    printf("config len=%u rc=%x\n", len, aacDecoder_ConfigRaw(r.h, &conf, &len));
    print_si(&r);
  }
  for (i = 0; i < c->n_params; i++) {
    printf("set %x %d rc=%x\n", c->params[i][0], c->params[i][1],
           aacDecoder_SetParam(r.h, (AACDEC_PARAM)c->params[i][0], c->params[i][1]));
  }
  if (c->anc) {
    r.anc_size = c->anc;
    r.anc = (UCHAR *)calloc((size_t)c->anc, 1);
    printf("anc_init rc=%x\n", aacDecoder_AncDataInit(r.h, r.anc, c->anc));
  }

  if (c->transport == -2) {
    for (i = 0; i < (int)trk.n_samples && (!c->frames || r.k < c->frames); i++) {
      UCHAR *au = data + trk.samples[i].offset;
      fill(&r, au, trk.samples[i].size);
      decode(&r, flags_of(r.k));
    }
  } else {
    while (pos < size && (!c->frames || r.k < c->frames)) {
      UINT n = (UINT)c->chunks[n_chunk++ % c->n_chunks];
      int j;
      if (n > size - pos) n = (UINT)(size - pos);
      pos += fill(&r, data + pos, n);
      for (j = 0; j < 16; j++) {
        AAC_DECODER_ERROR e;
        if (pipelined()) {
          /* The return value is that of the previous frame (the call that decodes the first
           * frame returns AAC_DEC_NOT_ENOUGH_BITS), so the calls depend only on whether a call
           * consumed input (the free bytes of the input buffer, the same for both libraries).
           * An empty aacDecoder_Fill() before every further call keeps a call after
           * AAC_DEC_NOT_ENOUGH_BITS from returning the pending frame (a drain). */
          UINT free_before = 0, free_after = 0;
          if (j > 0) fill(&r, data + pos, 0);
          aacDecoder_GetFreeBytes(r.h, &free_before);
          e = decode(&r, flags_of(r.k));
          aacDecoder_GetFreeBytes(r.h, &free_after);
          if (e == AAC_DEC_NOT_ENOUGH_BITS && free_after == free_before) break;
          continue;
        }
        e = decode(&r, flags_of(r.k));
        if (e == AAC_DEC_NOT_ENOUGH_BITS || !IS_OUTPUT_VALID(e)) break;
      }
    }
  }
  for (i = 0; i < 3; i++) decode(&r, AACDEC_FLUSH);
  clear_rc = aacDecoder_SetParam(r.h, AAC_TPDEC_CLEAR_BUFFER, 1);
  if (!pipelined()) {
    printf("set clear-buffer rc=%x\n", clear_rc);
    print_si(&r);
  }
  if (decode(&r, 0) == AAC_DEC_NOT_ENOUGH_BITS && pipelined()) {
    /* fdk-aac-rs with FDK_AAC_RS_THREADS > 1 returns every frame one call later: a call that
     * finds not enough input directly after AAC_DEC_NOT_ENOUGH_BITS (without aacDecoder_Fill())
     * returns the pending frame, until AAC_DEC_NOT_ENOUGH_BITS comes twice in a row. (The
     * reference returns AAC_DEC_NOT_ENOUGH_BITS at once.) The last flush frame comes here, so
     * the clear-buffer result is printed after it; the stream information between the calls
     * is that of the last frame returned (not printed). */
    while (decode(&r, 0) != AAC_DEC_NOT_ENOUGH_BITS) {
    }
    printf("set clear-buffer rc=%x\n", clear_rc);
  }
  aacDecoder_Close(r.h);
  printf("closed\n");
done:
  free(r.pcm);
  free(r.anc);
  free(trk.samples);
  free(data);
}

static void run_case(const Case *c) {
  size_t size = 0;
  unsigned char *data = read_file(c->file, &size);
  printf("case %s\n", c->name);
  if (!data) {
    printf("cannot read %s\n", c->file);
    return;
  }
  run_data(c, data, size);
}

/* An ADTS stream with ancillary data, made with the encoder of the library under test
 * (aacEncEncode with IN_ANCILLRY_DATA and AACENC_METADATA_MODE 3; the encoder is bit-exact,
 * scripts/capi_test.sh checks it separately). */
static unsigned char *anc_stream(size_t *size) {
  HANDLE_AACENCODER h = NULL;
  size_t cap = 1 << 20, n = 0;
  unsigned char *out = (unsigned char *)malloc(cap), anc[64], buf[4096];
  INT_PCM pcm[2 * 1024];
  int k, i;
  aacEncOpen(&h, 0, 2);
  aacEncoder_SetParam(h, AACENC_SAMPLERATE, 48000);
  aacEncoder_SetParam(h, AACENC_CHANNELMODE, MODE_2);
  aacEncoder_SetParam(h, AACENC_BITRATE, 96000);
  aacEncoder_SetParam(h, AACENC_TRANSMUX, TT_MP4_ADTS);
  aacEncoder_SetParam(h, AACENC_METADATA_MODE, 3);
  aacEncEncode(h, NULL, NULL, NULL, NULL);
  for (k = 0; k < 60; k++) {
    AACENC_BufDesc in = {0}, o = {0};
    AACENC_InArgs ia = {0};
    AACENC_OutArgs oa = {0};
    void *in_ptr[2] = {pcm, anc}, *out_ptr = buf;
    INT in_id[2] = {IN_AUDIO_DATA, IN_ANCILLRY_DATA}, in_size[2], in_el[2] = {2, 1};
    INT out_id = OUT_BITSTREAM_DATA, out_size = sizeof(buf), out_el = 1;
    int nanc = (k % 6) * 7;
    for (i = 0; i < 2 * 1024; i++) pcm[i] = (INT_PCM)(((i * 7919 + k * 131) % 20011) - 10005);
    for (i = 0; i < nanc; i++) anc[i] = (unsigned char)(k * 3 + i);
    in_size[0] = sizeof(pcm);
    in_size[1] = nanc;
    in.numBufs = nanc ? 2 : 1;
    in.bufs = in_ptr;
    in.bufferIdentifiers = in_id;
    in.bufSizes = in_size;
    in.bufElSizes = in_el;
    o.numBufs = 1;
    o.bufs = &out_ptr;
    o.bufferIdentifiers = &out_id;
    o.bufSizes = &out_size;
    o.bufElSizes = &out_el;
    ia.numInSamples = 2 * 1024;
    ia.numAncBytes = nanc;
    if (aacEncEncode(h, &in, &o, &ia, &oa) != AACENC_OK) break;
    memcpy(out + n, buf, (size_t)oa.numOutBytes);
    n += (size_t)oa.numOutBytes;
  }
  aacEncClose(&h);
  *size = n;
  return out;
}

static void anc_cases(void) {
  static const char *specs[] = {"anc-64|adts|-|chunk=1500:200,anc=64",
                                "anc-small|adts|-|anc=9",
                                "anc-none|adts|-|anc=0"};
  size_t i;
  for (i = 0; i < sizeof(specs) / sizeof(specs[0]); i++) {
    char spec[128];
    Case c;
    size_t size;
    unsigned char *data = anc_stream(&size);
    snprintf(spec, sizeof(spec), "%s", specs[i]);
    parse_case(spec, &c);
    printf("case %s (%u bytes)\n", c.name, (unsigned)size);
    run_data(&c, data, size);
  }
}

static void parse_case(char *spec, Case *c) {
  char *name = strtok(spec, "|"), *tp = strtok(NULL, "|"), *file = strtok(NULL, "|");
  char *opts = strtok(NULL, "|");
  memset(c, 0, sizeof(*c));
  snprintf(c->name, sizeof(c->name), "%s", name ? name : "?");
  c->file = file ? file : "";
  c->chunks[0] = 4096;
  c->n_chunks = 1;
  c->out_size = 8 * 2048;
  if (!tp) tp = "adts";
  c->transport = !strcmp(tp, "mp4")     ? -2
                 : !strcmp(tp, "adif")  ? TT_MP4_ADIF
                 : !strcmp(tp, "loas")  ? TT_MP4_LOAS
                 : !strcmp(tp, "latm1") ? TT_MP4_LATM_MCP1
                                        : TT_MP4_ADTS;
  if (opts) {
    char *o;
    for (o = strtok(opts, ","); o; o = strtok(NULL, ",")) {
      if (!strncmp(o, "chunk=", 6)) {
        char *p = o + 6;
        c->n_chunks = 0;
        while (*p && c->n_chunks < 8) {
          c->chunks[c->n_chunks++] = (int)strtol(p, &p, 10);
          if (*p == ':') p++;
        }
      } else if (!strncmp(o, "param=", 6) && c->n_params < MAXP) {
        char *p = o + 6;
        c->params[c->n_params][0] = (int)strtol(p, &p, 16);
        c->params[c->n_params][1] = (int)strtol(p + 1, NULL, 10);
        c->n_params++;
      } else if (!strncmp(o, "anc=", 4)) {
        c->anc = atoi(o + 4);
      } else if (!strncmp(o, "out=", 4)) {
        c->out_size = atoi(o + 4);
      } else if (!strncmp(o, "corrupt=", 8)) {
        c->corrupt = (unsigned)strtoul(o + 8, NULL, 10);
      } else if (!strncmp(o, "frames=", 7)) {
        c->frames = atoi(o + 7);
      }
    }
  }
}

static void null_paths(void) {
  INT_PCM pcm[64];
  UINT free_bytes = 77;
  HANDLE_AACDECODER h;
  int i, t;
  static const int transports[] = {-1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 99};
  printf("case null-paths\n");
  for (i = 0; i < 64; i++) pcm[i] = 7;
  printf("decode(NULL) rc=%x pcm=%016llx\n", aacDecoder_DecodeFrame(NULL, pcm, 64, 0),
         (unsigned long long)fnv(pcm, sizeof(pcm)));
  printf("set(NULL) rc=%x\n", aacDecoder_SetParam(NULL, AAC_PCM_LIMITER_ENABLE, 0));
  printf("free(NULL) rc=%x n=%u\n", aacDecoder_GetFreeBytes(NULL, &free_bytes), free_bytes);
  printf("si(NULL)=%s\n", aacDecoder_GetStreamInfo(NULL) ? "set" : "null");
  aacDecoder_Close(NULL);
  printf("libinfo(NULL) rc=%d\n", aacDecoder_GetLibInfo(NULL));
  for (i = 0; i < (int)(sizeof(transports) / sizeof(transports[0])); i++) {
    t = transports[i];
    h = aacDecoder_Open((TRANSPORT_TYPE)t, 1);
    printf("open(%d) %s", t, h ? "ok" : "null");
    if (h) {
      UCHAR *p = (UCHAR *)(uintptr_t)1;
      int size = -1;
      const CStreamInfo *s = aacDecoder_GetStreamInfo(h);
      printf(" free rc=%x", aacDecoder_GetFreeBytes(h, &free_bytes));
      printf(" n=%u", free_bytes);
      printf(" anc rc=%x", aacDecoder_AncDataGet(h, 0, &p, &size));
      printf(" ptr=%s size=%d", p ? "set" : "null", size);
      printf(" anc_init(-1) rc=%x", aacDecoder_AncDataInit(h, NULL, -1));
      printf(" profile=%d aot=%d ep=%d prl=%d pres=%d loud=%d\n", s->profile, (int)s->aot,
             (int)s->epConfig, (int)s->drcProgRefLev, (int)s->drcPresMode, s->outputLoudness);
      for (i = 0; i < 64; i++) pcm[i] = 7;
      printf("  decode rc=%x pcm=%016llx\n", aacDecoder_DecodeFrame(h, pcm, 40, 0),
             (unsigned long long)fnv(pcm, sizeof(pcm)));
      {
        AAC_DECODER_ERROR e = aacDecoder_DecodeFrame(h, pcm, 64, AACDEC_FLUSH);
        /* (frame pipelining: the result of the flush call comes with the drain) */
        if (pipelined() && e == AAC_DEC_NOT_ENOUGH_BITS) e = aacDecoder_DecodeFrame(h, pcm, 64, 0);
        printf("  decode(flush) rc=%x pcm=%016llx\n", e, (unsigned long long)fnv(pcm, sizeof(pcm)));
      }
      aacDecoder_Close(h);
    } else {
      printf("\n");
    }
  }
  /* nrOfLayers 0: the decoder fills nothing. */
  h = aacDecoder_Open(TT_MP4_ADTS, 0);
  if (h) {
    UCHAR b[4] = {0xff, 0xf1, 0, 0}, *bp = b;
    UINT n = 4, v = 4;
    printf("layers 0: fill rc=%x", aacDecoder_Fill(h, &bp, &n, &v));
    printf(" valid=%u\n", v);
    aacDecoder_Close(h);
  }
}

static void params(void) {
  static const int ids[] = {0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0011, 0x0012, 0x0020,
                            0x0021, 0x0100, 0x0200, 0x0201, 0x0202, 0x0203, 0x0204, 0x0205,
                            0x0206, 0x0207, 0x0300, 0x0603, 0x0001, 0x0400, 0x9999};
  static const int values[] = {-3, -2, -1, 0, 1, 2, 3, 4, 6, 7, 8, 9, 15, 16, 39, 40, 96, 127, 128, 1000};
  HANDLE_AACDECODER h = aacDecoder_Open(TT_MP4_ADTS, 1);
  size_t i, j;
  printf("case params\n");
  for (i = 0; i < sizeof(ids) / sizeof(ids[0]); i++) {
    printf("param %x:", ids[i]);
    for (j = 0; j < sizeof(values) / sizeof(values[0]); j++)
      printf(" %x", aacDecoder_SetParam(h, (AACDEC_PARAM)ids[i], values[j]));
    printf("\n");
  }
  aacDecoder_Close(h);
}

#ifdef WITH_RAW_ISOBMFF
static void isobmff(void) {
  /* Each box is followed by zero padding (PORTING.md, "Deviations from the reference"). */
  static UCHAR boxes[][256] = {
      {0, 0, 0, 8},
      {0, 0, 0, 12, 'f', 'r', 'e', 'e', 1, 2, 3, 4},
      {0, 0, 0, 12, 'l', 'u', 'd', 't', 0, 0, 0, 0},
      {0, 0, 0, 16, 'd', 'm', 'i', 'x', 0, 0, 0, 0, 1, 2, 3, 4},
      {0, 0, 0, 16, 'u', 'd', 'i', '2', 0, 0, 0, 0, 0, 0, 0, 0},
      {0, 0, 0, 16, 'u', 'd', 'c', '2', 0, 0, 0, 0, 0x10, 0, 0, 0},
      {0, 0, 0, 30, 'l', 'u', 'd', 't', 0, 0, 0, 0},
  };
  static const UINT lengths[] = {4, 12, 12, 16, 16, 16, 24};
  HANDLE_AACDECODER h = aacDecoder_Open(TT_MP4_RAW, 1);
  size_t i;
  printf("isobmff case\n");
  for (i = 0; i < sizeof(lengths) / sizeof(lengths[0]); i++)
    printf("isobmff %u rc=%x\n", (unsigned)i, aacDecoder_RawISOBMFFData(h, boxes[i], lengths[i]));
  aacDecoder_Close(h);
}
#endif

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
  info[0].module_id = FDK_AACENC;
  printf("rc=%d\n", aacDecoder_GetLibInfo(info));
  print_lib_info(info);
  printf("second call rc=%d\n", aacDecoder_GetLibInfo(info));
  print_lib_info(info);
  for (i = 0; i < FDK_MODULE_LAST; i++) info[i].module_id = FDK_PCMDMX;
  printf("full rc=%d\n", aacDecoder_GetLibInfo(info));
  print_lib_info(info);
}

int main(int argc, char **argv) {
  int i;
  setvbuf(stdout, NULL, _IOFBF, 1 << 20);
  for (i = 1; i < argc; i++) {
    if (!strcmp(argv[i], "--builtin")) {
      null_paths();
      params();
      lib_info();
      anc_cases();
#ifdef WITH_RAW_ISOBMFF
      isobmff();
#endif
    } else {
      Case c;
      parse_case(argv[i], &c);
      run_case(&c);
    }
  }
  return 0;
}
