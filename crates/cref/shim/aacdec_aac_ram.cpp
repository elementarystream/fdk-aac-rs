// Shims for the differential tests of fdk_aac_rs_aacdec::aac_ram.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_aac_ram (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/aac_ram.h"

/* 1 if the `bytes` bytes at p are all zero. */
static INT all_zero(const void *p, UINT bytes) {
  const UCHAR *b = (const UCHAR *)p;
  for (UINT i = 0; i < bytes; i++)
    if (b[i] != 0) return 0;
  return 1;
}

extern "C" {
/* The constants of aac_ram.h / aac_ram.cpp and the GetRequiredMem values that do not depend on
   the layout of a structure, in the order of the Rust test. */
INT cref_ram_constants(INT *out) {
  INT n = 0;
  out[n++] = MAX_SYNCHS;
  out[n++] = SAMPL_FREQS;
  out[n++] = TIME_DATA_FLUSH_SIZE;
  out[n++] = (INT)GetRequiredMemOverlapBuffer();
  out[n++] = (INT)GetRequiredMemTimeDataFlush();
  out[n++] = (INT)GetRequiredMemWorkBufferCore2();
  out[n++] = (INT)GetRequiredMemWorkBufferCore1();
  out[n++] = (INT)GetRequiredMemWorkBufferCore5();
  return n;
}

/* Allocates every buffer of aac_ram.cpp once and returns, per buffer, 1 if its memory is
   zero-filled (in the order of the Rust test). */
INT cref_ram_zeroed(INT *out) {
  INT n = 0;
  CAacDecoderStaticChannelInfo *s = GetAacDecoderStaticChannelInfo(0);
  out[n++] = all_zero(s, sizeof(*s));
  FreeAacDecoderStaticChannelInfo(&s);
  out[n++] = (s == NULL);
  CAacDecoderChannelInfo *ci = GetAacDecoderChannelInfo(7);
  out[n++] = all_zero(ci, sizeof(*ci));
  FreeAacDecoderChannelInfo(&ci);
  FIXP_DBL *ov = GetOverlapBuffer(0);
  out[n++] = all_zero(ov, OverlapBufferSize * sizeof(FIXP_DBL));
  FreeOverlapBuffer(&ov);
  CDrcInfo *d = GetDrcInfo(0);
  out[n++] = all_zero(d, sizeof(*d));
  FreeDrcInfo(&d);
  CpePersistentData *cpe = GetCpePersistentData(0);
  out[n++] = all_zero(cpe, sizeof(*cpe));
  FreeCpePersistentData(&cpe);
  CCplxPredictionData *cp = GetCplxPredictionData(0);
  out[n++] = all_zero(cp, sizeof(*cp));
  FreeCplxPredictionData(&cp);
  PCM_DEC *tf = GetTimeDataFlush(0);
  out[n++] = all_zero(tf, TIME_DATA_FLUSH_SIZE * sizeof(PCM_DEC));
  FreeTimeDataFlush(&tf);
  FIXP_DBL *w2 = GetWorkBufferCore2(0);
  out[n++] = all_zero(w2, 8 * 1024 * sizeof(FIXP_DBL));
  FreeWorkBufferCore2(&w2);
  SCHAR *w6 = GetWorkBufferCore6(0);
  out[n++] = all_zero(w6, sizeof(CAacDecoderCommonData));
  FreeWorkBufferCore6(&w6);
  CWorkBufferCore1 *w1 = GetWorkBufferCore1(0);
  out[n++] = all_zero(w1, sizeof(*w1));
  FreeWorkBufferCore1(&w1);
  PCM_DEC *w5 = GetWorkBufferCore5(0);
  out[n++] = all_zero(w5, 8 * (1024 * 4) * 2 * sizeof(PCM_DEC));
  FreeWorkBufferCore5(&w5);
  return n;
}
}
