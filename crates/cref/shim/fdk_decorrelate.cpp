// Shims for the differential tests of fdk_aac_rs_fdk::decorrelate.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_decorrelate (see build.rs).
//
// A decorrelator lives on the C++ side (zero-initialized like in the library) together with its
// buffer, behind an opaque handle. cref_decorrelate_state() dumps its complete state in the
// layout that the test computes from the Rust struct (pointers into the buffer as offsets,
// pointers to tables by their first values).

#include <stdlib.h>
#include "cref_common.h"
#include "FDK_decorrelate.h"

/* A copy of FDK_decorrelate.cpp, for its file-static tables and functions. */
namespace ref_decorr {
#include "libFDK/src/FDK_decorrelate.cpp"
}

struct CrefDecorr {
  DECORR_DEC dec;
  FIXP_DBL *buf;
  INT bufLen;
};

#define DECORR_ABSENT (-100)

static void dump4(INT **o, const void *p, INT kind) {
  for (int i = 0; i < 4; i++) {
    if (p == NULL) {
      *(*o)++ = DECORR_ABSENT;
    } else if (kind == 0) {
      *(*o)++ = ((const REVBAND_FILT_TYPE *)p)[i];
    } else if (kind == 1) {
      *(*o)++ = ((const UCHAR *)p)[i];
    } else {
      *(*o)++ = ((const SCHAR *)p)[i];
    }
  }
}

extern "C" {
/* The buffer is followed by DECORR_SLACK zeros: the reference has no defined result for delay
   line indices past the buffer (the port reads 0 there and drops the writes; within the test
   they are only read before they are written). */
#define DECORR_SLACK 256
void *cref_decorrelate_new(INT bufLen) {
  CrefDecorr *c = (CrefDecorr *)calloc(1, sizeof(CrefDecorr));
  c->buf = (FIXP_DBL *)calloc((bufLen > 0 ? bufLen : 0) + DECORR_SLACK, sizeof(FIXP_DBL));
  c->bufLen = bufLen;
  return c;
}
void cref_decorrelate_free(void *h) {
  CrefDecorr *c = (CrefDecorr *)h;
  free(c->buf);
  free(c);
}
INT cref_decorrelate_open(void *h) {
  CrefDecorr *c = (CrefDecorr *)h;
  return FDKdecorrelateOpen(&c->dec, c->buf, c->bufLen);
}
INT cref_decorrelate_init(void *h, INT nrHybBands, INT decorrType, INT duckerType, INT decorrConfig, INT seed, INT partiallyComplex, INT useFractDelay, INT isLegacyPS, INT initStatesFlag) {
  CrefDecorr *c = (CrefDecorr *)h;
  return FDKdecorrelateInit(&c->dec, nrHybBands, (FDK_DECORR_TYPE)decorrType,
                            (FDK_DUCKER_TYPE)duckerType, decorrConfig, seed, partiallyComplex,
                            useFractDelay, isLegacyPS, initStatesFlag);
}
INT cref_decorrelate_apply(void *h, FIXP_DBL *inRe, FIXP_DBL *inIm, FIXP_DBL *outRe, FIXP_DBL *outIm, INT start) {
  CrefDecorr *c = (CrefDecorr *)h;
  return FDKdecorrelateApply(&c->dec, inRe, inIm, outRe, outIm, start);
}
INT cref_decorrelate_close(void *h) {
  CrefDecorr *c = (CrefDecorr *)h;
  return FDKdecorrelateClose(&c->dec);
}
void cref_decorrelate_set_max_val(void *h, FIXP_DBL v) {
  CrefDecorr *c = (CrefDecorr *)h;
  *getAddrDirectSignalMaxVal(&c->dec) = v;
}
void cref_decorrelate_set_buffer(void *h, const FIXP_DBL *v, INT n) {
  CrefDecorr *c = (CrefDecorr *)h;
  FDKmemcpy(c->buf, v, n * sizeof(FIXP_DBL));
}
void cref_decorrelate_get_buffer(void *h, FIXP_DBL *v, INT n) {
  CrefDecorr *c = (CrefDecorr *)h;
  FDKmemcpy(v, c->buf, n * sizeof(FIXP_DBL));
}
INT cref_decorrelate_state(void *h, INT *out) {
  CrefDecorr *c = (CrefDecorr *)h;
  const DECORR_DEC *d = &c->dec;
  INT *o = out;
  *o++ = d->L_stateBufferCplx;
  *o++ = d->stateBufferCplx == NULL ? -1 : (INT)(d->stateBufferCplx - c->buf);
  *o++ = d->L_delayBufferCplx;
  *o++ = d->delayBufferCplx == NULL ? -1 : (INT)(d->delayBufferCplx - c->buf);
  dump4(&o, d->REV_filtType, 0);
  dump4(&o, d->REV_bandOffset, 1);
  dump4(&o, d->REV_delay, 1);
  dump4(&o, d->REV_filterOrder, 2);
  for (int i = 0; i < 4; i++) *o++ = d->reverbBandDelayBufferIndex[i];
  for (int i = 0; i < 3; i++) *o++ = d->stateBufferOffset[i];
  *o++ = d->numbins;
  *o++ = d->partiallyComplex;
  const DUCKER_INSTANCE *k = &d->ducker;
  *o++ = k->hybridBands;
  *o++ = k->parameterBands;
  *o++ = k->partiallyComplex;
  *o++ = k->duckerType;
  *o++ = (k->qs_next == NULL || k->mapProcBands2HybBands == NULL)
             ? -1
             : (INT)(k->qs_next - k->mapProcBands2HybBands);
  for (int i = 0; i < 21; i++)
    *o++ = k->mapProcBands2HybBands == NULL ? DECORR_ABSENT : k->mapProcBands2HybBands[i];
  for (int i = 0; i < 65; i++)
    *o++ = k->mapHybBands2ProcBands == NULL ? DECORR_ABSENT : k->mapHybBands2ProcBands[i];
  for (int i = 0; i < 2 * 28; i++) *o++ = k->SmoothDirRevNrg[i];
  for (int i = 0; i < 28; i++) *o++ = k->peakDecay[i];
  for (int i = 0; i < 28; i++) *o++ = k->peakDiff[i];
  *o++ = k->maxValDirectData;
  *o++ = k->maxValReverbData;
  *o++ = k->scaleDirectNrg;
  *o++ = k->scaleReverbNrg;
  *o++ = k->scaleSmoothDirRevNrg;
  *o++ = k->headroomSmoothDirRevNrg;
  for (int i = 0; i < 71; i++) {
    const DECORR_FILTER_INSTANCE *f = &d->Filter[i];
    *o++ = f->stateCplx == NULL ? -1 : (INT)(f->stateCplx - c->buf);
    *o++ = f->DelayBufferCplx == NULL ? -1 : (INT)(f->DelayBufferCplx - c->buf);
    for (int j = 0; j < 3; j++)
      *o++ = f->numeratorReal == NULL ? DECORR_ABSENT : f->numeratorReal[j];
    *o++ = f->coeffsPacked == NULL ? DECORR_ABSENT : f->coeffsPacked[0].v.re;
    *o++ = f->coeffsPacked == NULL ? DECORR_ABSENT : f->coeffsPacked[0].v.im;
    *o++ = f->coeffsPacked == NULL ? DECORR_ABSENT : f->coeffsPacked[3].v.re;
    *o++ = f->coeffsPacked == NULL ? DECORR_ABSENT : f->coeffsPacked[3].v.im;
    *o++ = f->denominatorReal != NULL;
  }
  return (INT)(o - out);
}
/* DecorrFilterApplyPASS with dataImagIn == NULL (not reachable through FDKdecorrelateApply):
   filter[start] delays into `delay` at the offset `delayOffset`. */
INT cref_decorrelate_apply_pass_real(FIXP_DBL *delay, INT delayOffset, FIXP_DBL *inRe, FIXP_DBL *outRe, INT start, INT stop, INT noSampleDelay, INT index) {
  DECORR_FILTER_INSTANCE filter[71];
  FDKmemclear(filter, sizeof(filter));
  filter[start].DelayBufferCplx = delay + delayOffset;
  return ref_decorr::DecorrFilterApplyPASS(filter, inRe, NULL, outRe, NULL, start, stop,
                                           noSampleDelay, index);
}
/* The file-static tables (as INT values). */
INT cref_decorrelate_table(const char *name, INT *out) {
#define DT(sym)                                                       \
  if (strcmp(name, #sym) == 0) {                                      \
    INT n = 0;                                                        \
    for (size_t i = 0; i < sizeof(ref_decorr::sym) / sizeof(ref_decorr::sym[0]); i++) { \
      out[n++] = ref_decorr::sym[i];                                  \
    }                                                                 \
    return n;                                                         \
  }
#define DT2(sym)                                                      \
  if (strcmp(name, #sym) == 0) {                                      \
    INT n = 0;                                                        \
    for (size_t i = 0; i < sizeof(ref_decorr::sym) / sizeof(ref_decorr::sym[0]); i++) { \
      for (size_t j = 0; j < sizeof(ref_decorr::sym[0]) / sizeof(ref_decorr::sym[0][0]); j++) { \
        out[n++] = ref_decorr::sym[i][j];                             \
      }                                                               \
    }                                                                 \
    return n;                                                         \
  }
  DT2(REV_bandOffset_MPS_HQ)
  DT(REV_bandOffset_PS_HQ)
  DT(REV_bandOffset_PS_LP)
  DT2(REV_bandOffset_LD)
  DT(REV_delay_MPS)
  DT(REV_filterOrder_MPS)
  DT(REV_delay_PS_HQ)
  DT(REV_delay_PS_LP)
  DT(REV_filterOrder_PS)
  DT(REV_delay_USAC)
  DT(REV_filterOrder_USAC)
  DT(REV_filtType_MPS)
  DT(REV_filtType_PS)
  DT(stateBufferOffsetInit)
  DT(REV_filtType_LD)
  DT(kernels_20_to_71_PS)
  DT(kernels_20_to_71_offset_PS)
  DT(kernels_28_to_71)
  DT(kernels_28_to_71_offset)
  DT(kernels_23_to_64)
  DT(kernels_23_to_64_offset)
  DT2(DecorrNumeratorReal0_USAC)
  DT2(DecorrNumeratorReal1_USAC)
  DT2(DecorrNumeratorReal2_USAC)
  DT2(DecorrNumeratorReal3_USAC)
  DT2(DecorrNumeratorReal1_LD)
  DT2(DecorrNumeratorReal2_LD)
  DT2(DecorrNumeratorReal3_LD)
  if (strcmp(name, "DecorrPsCoeffsCplx") == 0) {
    INT n = 0;
    for (int i = 0; i < 30; i++) {
      for (int j = 0; j < 4; j++) {
        out[n++] = ref_decorr::DecorrPsCoeffsCplx[i][j].v.re;
        out[n++] = ref_decorr::DecorrPsCoeffsCplx[i][j].v.im;
      }
    }
    return n;
  }
  return -1;
#undef DT
#undef DT2
}
}
