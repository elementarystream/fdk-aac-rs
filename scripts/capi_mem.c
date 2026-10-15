/* fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
 * A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
 *
 * Heap use of one decoder or encoder instance through the C API (scripts/capi_mem.sh). The
 * program replaces glibc's malloc family (as glibc allows: the executable defines malloc, free,
 * calloc, realloc, memalign, posix_memalign, aligned_alloc, valloc and pvalloc and forwards them
 * to the __libc_* functions) and counts the usable size of every live block, in all threads. It
 * opens an instance, configures it, decodes or encodes a whole input and closes the instance,
 * and prints the heap in use after the open (the instance), the peak while processing and what
 * is left after the close, each relative to the heap before the open. Thread stacks and other
 * mmap'ed memory outside malloc are not counted.
 *
 * scripts/capi_mem.sh links it once against fdk-aac-rs-capi and once against the reference
 * library.
 *
 * Usage: capi_mem dec TRANSPORT FILE          (TRANSPORT: adts or loas)
 *        capi_mem enc CHANNELS AOT BITRATE TRANSMUX AFTERBURNER SBR_MODE FILE
 *        (FILE: interleaved s16le PCM at 44.1 kHz; SBR_MODE -1: not set) */

#define _GNU_SOURCE
#include <malloc.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "aacdecoder_lib.h"
#include "aacenc_lib.h"

extern void *__libc_malloc(size_t);
extern void __libc_free(void *);
extern void *__libc_calloc(size_t, size_t);
extern void *__libc_realloc(void *, size_t);
extern void *__libc_memalign(size_t, size_t);

static size_t cur, peak;

static void count_add(void *p) {
  if (!p) return;
  size_t c = __atomic_add_fetch(&cur, malloc_usable_size(p), __ATOMIC_RELAXED);
  size_t m = __atomic_load_n(&peak, __ATOMIC_RELAXED);
  while (c > m && !__atomic_compare_exchange_n(&peak, &m, c, 1, __ATOMIC_RELAXED, __ATOMIC_RELAXED)) {
  }
}

static void count_sub(void *p) {
  if (p) __atomic_sub_fetch(&cur, malloc_usable_size(p), __ATOMIC_RELAXED);
}

void *malloc(size_t n) {
  void *p = __libc_malloc(n);
  count_add(p);
  return p;
}
void free(void *p) {
  count_sub(p);
  __libc_free(p);
}
void *calloc(size_t a, size_t b) {
  void *p = __libc_calloc(a, b);
  count_add(p);
  return p;
}
void *realloc(void *q, size_t n) {
  size_t old = q ? malloc_usable_size(q) : 0;
  void *p = __libc_realloc(q, n);
  if (p || n == 0) {
    __atomic_sub_fetch(&cur, old, __ATOMIC_RELAXED);
    count_add(p);
  }
  return p;
}
void *memalign(size_t a, size_t n) {
  void *p = __libc_memalign(a, n);
  count_add(p);
  return p;
}
int posix_memalign(void **r, size_t a, size_t n) {
  void *p = __libc_memalign(a, n);
  if (!p) return 12; /* ENOMEM */
  count_add(p);
  *r = p;
  return 0;
}
void *aligned_alloc(size_t a, size_t n) { return memalign(a, n); }
void *valloc(size_t n) { return memalign(4096, n); }
void *pvalloc(size_t n) { return memalign(4096, (n + 4095) & ~(size_t)4095); }

static size_t base;

static void start(void) {
  base = __atomic_load_n(&cur, __ATOMIC_RELAXED);
  __atomic_store_n(&peak, base, __ATOMIC_RELAXED);
}

static long rel(size_t v) { return (long)v - (long)base; }

static uint8_t in_buf[1 << 16];
static INT_PCM out_buf[8 * 4096];

static int dec(const char *tt_name, const char *path, long *res) {
  TRANSPORT_TYPE tt = strcmp(tt_name, "loas") == 0 ? TT_MP4_LOAS : TT_MP4_ADTS;
  FILE *f = fopen(path, "rb");
  if (!f) return 2;
  setvbuf(f, NULL, _IONBF, 0);
  start();
  HANDLE_AACDECODER h = aacDecoder_Open(tt, 1);
  if (!h) return 3;
  long open = rel(__atomic_load_n(&cur, __ATOMIC_RELAXED));
  const INT out_len = (INT)(sizeof out_buf / sizeof *out_buf);
  long frames = 0;
  UINT n;
  while ((n = (UINT)fread(in_buf, 1, sizeof in_buf, f)) > 0) {
    UINT valid = n;
    while (valid > 0) {
      UCHAR *p = in_buf + (n - valid);
      UINT size = valid;
      if (aacDecoder_Fill(h, &p, &size, &valid) != AAC_DEC_OK) return 4;
      AAC_DECODER_ERROR e;
      while ((e = aacDecoder_DecodeFrame(h, out_buf, out_len, 0)) != AAC_DEC_NOT_ENOUGH_BITS)
        if (e == AAC_DEC_OK) frames++;
    }
  }
  /* The output of a multi-threaded port decoder is one call late: drain until NOT_ENOUGH_BITS
   * comes twice in a row. */
  for (int nb = 0; nb < 2;) {
    AAC_DECODER_ERROR e = aacDecoder_DecodeFrame(h, out_buf, out_len, 0);
    if (e == AAC_DEC_NOT_ENOUGH_BITS) {
      nb++;
    } else {
      nb = 0;
      if (e == AAC_DEC_OK) frames++;
    }
  }
  long pk = rel(__atomic_load_n(&peak, __ATOMIC_RELAXED));
  aacDecoder_Close(h);
  long left = rel(__atomic_load_n(&cur, __ATOMIC_RELAXED));
  fclose(f);
  res[0] = open, res[1] = pk, res[2] = left, res[3] = frames;
  return 0;
}

static int enc(int argc, char **argv, long *res) {
  int ch = atoi(argv[2]), aot = atoi(argv[3]), br = atoi(argv[4]), tm = atoi(argv[5]);
  int ab = atoi(argv[6]), sbr = atoi(argv[7]);
  FILE *f = fopen(argv[8], "rb");
  if (!f) return 2;
  setvbuf(f, NULL, _IONBF, 0);
  (void)argc;
  static INT_PCM pcm[2048 * 8];
  static UCHAR out[8192 * 8];
  start();
  HANDLE_AACENCODER h;
  if (aacEncOpen(&h, 0, (UINT)ch) != AACENC_OK) return 3;
  aacEncoder_SetParam(h, AACENC_AOT, (UINT)aot);
  aacEncoder_SetParam(h, AACENC_SAMPLERATE, 44100);
  aacEncoder_SetParam(h, AACENC_CHANNELMODE, ch == 6 ? MODE_1_2_2_1 : (CHANNEL_MODE)ch);
  aacEncoder_SetParam(h, AACENC_BITRATE, (UINT)br);
  aacEncoder_SetParam(h, AACENC_TRANSMUX, (UINT)tm);
  aacEncoder_SetParam(h, AACENC_AFTERBURNER, (UINT)ab);
  if (sbr >= 0) aacEncoder_SetParam(h, AACENC_SBR_MODE, (UINT)sbr);
  if (aacEncEncode(h, NULL, NULL, NULL, NULL) != AACENC_OK) return 4;
  long open = rel(__atomic_load_n(&cur, __ATOMIC_RELAXED));
  AACENC_InfoStruct info;
  aacEncInfo(h, &info);
  long frames = 0;
  int eof = 0;
  for (;;) {
    size_t want = (size_t)info.inputChannels * info.frameLength;
    size_t got = eof ? 0 : fread(pcm, sizeof *pcm, want, f);
    if (got < want) eof = 1;
    void *ib = pcm, *ob = out;
    INT iid = IN_AUDIO_DATA, oid = OUT_BITSTREAM_DATA;
    INT isz = (INT)(got * sizeof *pcm), iel = (INT)sizeof *pcm, osz = (INT)sizeof out, oel = 1;
    AACENC_BufDesc ibd = {1, &ib, &iid, &isz, &iel}, obd = {1, &ob, &oid, &osz, &oel};
    AACENC_InArgs ia = {(INT)got, 0};
    AACENC_OutArgs oa;
    memset(&oa, 0, sizeof oa);
    if (got == 0) ia.numInSamples = -1;
    AACENC_ERROR e = aacEncEncode(h, &ibd, &obd, &ia, &oa);
    if (e == AACENC_ENCODE_EOF) break;
    if (e != AACENC_OK) return 5;
    if (oa.numOutBytes > 0) frames++;
  }
  long pk = rel(__atomic_load_n(&peak, __ATOMIC_RELAXED));
  aacEncClose(&h);
  long left = rel(__atomic_load_n(&cur, __ATOMIC_RELAXED));
  fclose(f);
  res[0] = open, res[1] = pk, res[2] = left, res[3] = frames;
  return 0;
}

/* The case runs twice in the process: the first run's figures include one-time allocations of
 * the library (lazily initialized tables and the like); the heap left after the second run's
 * close, relative to its own open, shows what each instance leaks. */
int main(int argc, char **argv) {
  long r[2][4];
  for (int i = 0; i < 2; i++) {
    int e;
    if (argc == 4 && strcmp(argv[1], "dec") == 0)
      e = dec(argv[2], argv[3], r[i]);
    else if (argc == 9 && strcmp(argv[1], "enc") == 0)
      e = enc(argc, argv, r[i]);
    else {
      fprintf(stderr, "usage: see the comment at the top of scripts/capi_mem.c\n");
      return 2;
    }
    if (e) return e;
  }
  printf("open %ld peak %ld closed %ld frames %ld leak %ld\n", r[0][0], r[0][1], r[0][2], r[0][3],
         r[1][2]);
  return 0;
}
