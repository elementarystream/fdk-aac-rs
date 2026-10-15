// Shims for the differential tests of fdk_aac_rs_fdk::hybrid.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_hybrid (see build.rs).
//
// The hybrid filter banks live on the C++ side behind opaque handles, the analysis filter bank
// together with its LF and HF memory. All functions call the library's own FDK_hybrid.cpp code
// (in which fft_8 is inlined). The setups are read through the pointer of a filter bank with a
// copy of the private struct layout of FDK_hybrid.cpp.

#include <stdlib.h>
#include "cref_common.h"
#include "FDK_hybrid.h"
#include "common_fix.h"

/* The layout of struct FDK_HYBRID_SETUP (private to FDK_hybrid.cpp). */
struct CrefHybridSetup {
  UCHAR nrQmfBands;
  UCHAR nHybBands[3];
  UCHAR synHybScale[3];
  SCHAR kHybrid[3];
  UCHAR protoLen;
  UCHAR filterDelay;
  const INT *pReadIdxTable;
};

struct CrefHybAna {
  FDK_ANA_HYB_FILTER f;
  FIXP_DBL *lf;
  FIXP_DBL *hf;
  INT lfLen;
  INT hfLen;
};

#define HYB_NULL_OFFSET (-100000)

static INT offset_of(const FIXP_DBL *p, const FIXP_DBL *base) {
  if (p == NULL) return HYB_NULL_OFFSET;
  return (INT)(p - base);
}

extern "C" {
/* Constants of fourChannelFiltering(): FL2FXCONST_DBL of 0, -0.707..., 0.707..., -1, 1. */
void cref_hybrid_consts(FIXP_DBL *out) {
  out[0] = FL2FXCONST_DBL(0.f);
  out[1] = FL2FXCONST_DBL(-0.70710678118655f);
  out[2] = FL2FXCONST_DBL(0.70710678118655f);
  out[3] = FL2FXCONST_DBL(-1.f);
  out[4] = FL2FXCONST_DBL(1.f);
}

void *cref_hybrid_ana_new(INT lfLen, INT hfLen) {
  CrefHybAna *c = (CrefHybAna *)calloc(1, sizeof(CrefHybAna));
  c->lf = (FIXP_DBL *)calloc(lfLen > 0 ? lfLen : 1, sizeof(FIXP_DBL));
  c->hf = (FIXP_DBL *)calloc(hfLen > 0 ? hfLen : 1, sizeof(FIXP_DBL));
  c->lfLen = lfLen;
  c->hfLen = hfLen;
  return c;
}
void cref_hybrid_ana_free(void *h) {
  CrefHybAna *c = (CrefHybAna *)h;
  free(c->lf);
  free(c->hf);
  free(c);
}
/* useHf = 0 passes NULL as HF memory. */
INT cref_hybrid_ana_open(void *h, UINT lfBytes, INT useHf, UINT hfBytes) {
  CrefHybAna *c = (CrefHybAna *)h;
  return FDKhybridAnalysisOpen(&c->f, c->lf, lfBytes, useHf ? c->hf : NULL, hfBytes);
}
INT cref_hybrid_ana_init(void *h, INT mode, INT qmfBands, INT cplxBands, INT initStatesFlag) {
  CrefHybAna *c = (CrefHybAna *)h;
  return FDKhybridAnalysisInit(&c->f, (FDK_HYBRID_MODE)mode, qmfBands, cplxBands,
                               initStatesFlag);
}
INT cref_hybrid_ana_scale_states(void *h, INT scalingValue) {
  CrefHybAna *c = (CrefHybAna *)h;
  return FDKhybridAnalysisScaleStates(h ? &c->f : NULL, scalingValue);
}
INT cref_hybrid_ana_apply(void *h, const FIXP_DBL *re, const FIXP_DBL *im, FIXP_DBL *hre, FIXP_DBL *him) {
  CrefHybAna *c = (CrefHybAna *)h;
  return FDKhybridAnalysisApply(&c->f, re, im, hre, him);
}
INT cref_hybrid_ana_close(void *h) {
  CrefHybAna *c = (CrefHybAna *)h;
  return FDKhybridAnalysisClose(h ? &c->f : NULL);
}
void cref_hybrid_ana_set_hf_mode(void *h, INT m) { ((CrefHybAna *)h)->f.hfMode = (UCHAR)m; }
/* out[42]: LF real/imag offsets (3 + 3), HF real/imag offsets (13 + 13), bufferLFpos,
   bufferHFpos, nrBands, cplxBands, hfMode, pLFmemory != NULL, pHFmemory != NULL, LFmemorySize,
   HFmemorySize, setup (0 NULL, 1 + mode of the setup identified by its fields, -1 other),
   2 spare. */
void cref_hybrid_ana_state(void *h, INT *out) {
  CrefHybAna *c = (CrefHybAna *)h;
  FDK_ANA_HYB_FILTER *f = &c->f;
  int k, o = 0;
  for (k = 0; k < 3; k++) out[o++] = offset_of(f->bufferLFReal[k], c->lf);
  for (k = 0; k < 3; k++) out[o++] = offset_of(f->bufferLFImag[k], c->lf);
  for (k = 0; k < 13; k++) out[o++] = offset_of(f->bufferHFReal[k], c->hf);
  for (k = 0; k < 13; k++) out[o++] = offset_of(f->bufferHFImag[k], c->hf);
  out[o++] = f->bufferLFpos;
  out[o++] = f->bufferHFpos;
  out[o++] = f->nrBands;
  out[o++] = f->cplxBands;
  out[o++] = f->hfMode;
  out[o++] = f->pLFmemory != NULL;
  out[o++] = f->pHFmemory != NULL;
  out[o++] = (INT)f->LFmemorySize;
  out[o++] = (INT)f->HFmemorySize;
  out[o++] = f->pSetup == NULL ? 0 : 1;
  out[o++] = 0;
  out[o++] = 0;
}
void cref_hybrid_ana_get_mem(void *h, FIXP_DBL *lf, FIXP_DBL *hf) {
  CrefHybAna *c = (CrefHybAna *)h;
  memcpy(lf, c->lf, c->lfLen * sizeof(FIXP_DBL));
  memcpy(hf, c->hf, c->hfLen * sizeof(FIXP_DBL));
}
void cref_hybrid_ana_set_mem(void *h, const FIXP_DBL *lf, const FIXP_DBL *hf) {
  CrefHybAna *c = (CrefHybAna *)h;
  memcpy(c->lf, lf, c->lfLen * sizeof(FIXP_DBL));
  memcpy(c->hf, hf, c->hfLen * sizeof(FIXP_DBL));
}
/* The setup of the filter bank: out[14]: nrQmfBands, nHybBands[3], synHybScale[3],
   kHybrid[3], protoLen, filterDelay, pReadIdxTable[0..26] follows in idx. Returns 0 without
   setup. */
INT cref_hybrid_ana_setup(void *h, INT *out, INT *idx) {
  CrefHybAna *c = (CrefHybAna *)h;
  const CrefHybridSetup *s = (const CrefHybridSetup *)c->f.pSetup;
  if (s == NULL) return 0;
  int o = 0;
  out[o++] = s->nrQmfBands;
  for (int k = 0; k < 3; k++) out[o++] = s->nHybBands[k];
  for (int k = 0; k < 3; k++) out[o++] = s->synHybScale[k];
  for (int k = 0; k < 3; k++) out[o++] = s->kHybrid[k];
  out[o++] = s->protoLen;
  out[o++] = s->filterDelay;
  for (int k = 0; k < 26; k++) idx[k] = s->pReadIdxTable[k];
  return 1;
}

void *cref_hybrid_syn_new(void) { return calloc(1, sizeof(FDK_SYN_HYB_FILTER)); }
void cref_hybrid_syn_free(void *h) { free(h); }
INT cref_hybrid_syn_init(void *h, INT mode, INT qmfBands, INT cplxBands) {
  return FDKhybridSynthesisInit((HANDLE_FDK_SYN_HYB_FILTER)h, (FDK_HYBRID_MODE)mode, qmfBands,
                                cplxBands);
}
void cref_hybrid_syn_apply(void *h, const FIXP_DBL *hre, const FIXP_DBL *him, FIXP_DBL *re, FIXP_DBL *im) {
  FDKhybridSynthesisApply((HANDLE_FDK_SYN_HYB_FILTER)h, hre, him, re, im);
}
/* out[3]: nrBands, cplxBands, pSetup != NULL */
void cref_hybrid_syn_state(void *h, INT *out) {
  FDK_SYN_HYB_FILTER *f = (FDK_SYN_HYB_FILTER *)h;
  out[0] = f->nrBands;
  out[1] = f->cplxBands;
  out[2] = f->pSetup != NULL;
}
}
