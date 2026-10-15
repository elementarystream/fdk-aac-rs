// Shims for the differential tests of fdk_aac_rs_fdk::qmf.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_qmf (see build.rs).
//
// A QMF_FILTER_BANK lives on the C++ side behind an opaque handle; its FilterStates point to a
// buffer owned by the test. The functions call the library's own qmf.cpp code (both the INT_PCM
// and the LONG instantiation of qmf_pcm.h).

#include <stdlib.h>
#include "cref_common.h"
#include "qmf.h"
#include "FDK_tools_rom.h"

/* The QMF tables in a fixed order; the test uses the same order. */
static const void *qmf_tables[] = {
    qmf_pfilt120,
    qmf_pfilt200,
    qmf_pfilt240,
    qmf_pfilt400,
    qmf_pfilt640,
    qmf_cldfb_640,
    qmf_cldfb_320,
    qmf_cldfb_160,
    qmf_cldfb_80,
    qmf_mpsldfb_320,
    qmf_mpsldfb_640,
    qmf_phaseshift_cos16,
    qmf_phaseshift_sin16,
    qmf_phaseshift_cos24,
    qmf_phaseshift_sin24,
    qmf_phaseshift_cos32,
    qmf_phaseshift_sin32,
    qmf_phaseshift_cos_downsamp32,
    qmf_phaseshift_sin_downsamp32,
    qmf_phaseshift_cos40,
    qmf_phaseshift_sin40,
    qmf_phaseshift_cos64,
    qmf_phaseshift_sin64,
    qmf_phaseshift_cos32_cldfb_ana,
    qmf_phaseshift_cos32_cldfb_syn,
    qmf_phaseshift_sin32_cldfb,
    qmf_phaseshift_cos16_cldfb_ana,
    qmf_phaseshift_cos16_cldfb_syn,
    qmf_phaseshift_sin16_cldfb,
    qmf_phaseshift_cos8_cldfb_ana,
    qmf_phaseshift_cos8_cldfb_syn,
    qmf_phaseshift_sin8_cldfb,
    qmf_phaseshift_cos64_cldfb,
    qmf_phaseshift_sin64_cldfb,
};

/* 0 for NULL, 1 + index in qmf_tables, -1 for another pointer. */
static INT table_id(const void *p) {
  if (p == NULL) return 0;
  for (unsigned i = 0; i < sizeof(qmf_tables) / sizeof(qmf_tables[0]); i++)
    if (qmf_tables[i] == p) return (INT)i + 1;
  return -1;
}

/* Defined with external linkage by qmf.cpp (qmf_pcm.h), not declared in a header. */
void qmfSynPrototypeFirSlot(HANDLE_QMF_FILTER_BANK qmf, FIXP_DBL *realSlot, FIXP_DBL *imagSlot,
                            INT_PCM *timeOut, int stride);
void qmfSynPrototypeFirSlot(HANDLE_QMF_FILTER_BANK qmf, FIXP_DBL *realSlot, FIXP_DBL *imagSlot,
                            LONG *timeOut, int stride);

#define QMF_MAX_COLS 64
#define QMF_SLOT 64

extern "C" {
void *cref_qmf_new(void) { return calloc(1, sizeof(QMF_FILTER_BANK)); }
void cref_qmf_free(void *h) { free(h); }

/* out[16]: p_filter, FilterStates != NULL, FilterSize, t_cos, t_sin, filterScale, no_channels,
   no_col, lsb, usb, synScalefactor, outScalefactor, outGain_m, outGain_e, flags, p_stride */
void cref_qmf_dump(void *h, INT *out) {
  QMF_FILTER_BANK *q = (QMF_FILTER_BANK *)h;
  out[0] = table_id(q->p_filter);
  out[1] = q->FilterStates != NULL;
  out[2] = q->FilterSize;
  out[3] = table_id(q->t_cos);
  out[4] = table_id(q->t_sin);
  out[5] = q->filterScale;
  out[6] = q->no_channels;
  out[7] = q->no_col;
  out[8] = q->lsb;
  out[9] = q->usb;
  out[10] = q->synScalefactor;
  out[11] = q->outScalefactor;
  out[12] = q->outGain_m;
  out[13] = q->outGain_e;
  out[14] = (INT)q->flags;
  out[15] = q->p_stride;
}
void cref_qmf_set_bands(void *h, INT lsb, INT usb) {
  QMF_FILTER_BANK *q = (QMF_FILTER_BANK *)h;
  q->lsb = lsb;
  q->usb = usb;
}

INT cref_qmf_init_analysis_16(void *h, SHORT *states, INT noCols, INT lsb, INT usb, INT no_channels, INT flags) {
  return qmfInitAnalysisFilterBank((HANDLE_QMF_FILTER_BANK)h, (FIXP_QAS *)states, noCols, lsb, usb,
                                   no_channels, flags);
}
INT cref_qmf_init_analysis_32(void *h, FIXP_DBL *states, INT noCols, INT lsb, INT usb, INT no_channels, INT flags) {
  return qmfInitAnalysisFilterBank((HANDLE_QMF_FILTER_BANK)h, states, noCols, lsb, usb,
                                   no_channels, flags);
}
INT cref_qmf_init_synthesis(void *h, FIXP_DBL *states, INT noCols, INT lsb, INT usb, INT no_channels, INT flags) {
  return qmfInitSynthesisFilterBank((HANDLE_QMF_FILTER_BANK)h, states, noCols, lsb, usb,
                                    no_channels, flags);
}
void cref_qmf_change_out_scalefactor(void *h, INT sf) {
  qmfChangeOutScalefactor((HANDLE_QMF_FILTER_BANK)h, sf);
}
INT cref_qmf_get_out_scalefactor(void *h) {
  return qmfGetOutScalefactor((HANDLE_QMF_FILTER_BANK)h);
}
void cref_qmf_change_out_gain(void *h, FIXP_DBL m, INT e) {
  qmfChangeOutGain((HANDLE_QMF_FILTER_BANK)h, m, e);
}

void cref_qmf_analysis_slot_16(void *h, FIXP_DBL *re, FIXP_DBL *im, const SHORT *timeIn, INT stride, FIXP_DBL *work) {
  qmfAnalysisFilteringSlot((HANDLE_QMF_FILTER_BANK)h, re, im, (const INT_PCM *)timeIn, stride,
                           work);
}
void cref_qmf_analysis_slot_32(void *h, FIXP_DBL *re, FIXP_DBL *im, const INT *timeIn, INT stride, FIXP_DBL *work) {
  qmfAnalysisFilteringSlot((HANDLE_QMF_FILTER_BANK)h, re, im, (const LONG *)timeIn, stride, work);
}
/* re, im: no_col slots of QMF_SLOT values; scale[4]: the QMF_SCALE_FACTOR (in/out). */
void cref_qmf_analysis_16(void *h, FIXP_DBL *re, FIXP_DBL *im, INT *scale, const SHORT *timeIn, INT timeIn_e, INT stride, FIXP_DBL *work) {
  FIXP_DBL *pr[QMF_MAX_COLS], *pi[QMF_MAX_COLS];
  for (int i = 0; i < QMF_MAX_COLS; i++) {
    pr[i] = re + i * QMF_SLOT;
    pi[i] = im + i * QMF_SLOT;
  }
  QMF_SCALE_FACTOR sf = {scale[0], scale[1], scale[2], scale[3]};
  qmfAnalysisFiltering((HANDLE_QMF_FILTER_BANK)h, pr, pi, &sf, (const INT_PCM *)timeIn, timeIn_e,
                       stride, work);
  scale[0] = sf.lb_scale;
  scale[1] = sf.ov_lb_scale;
  scale[2] = sf.hb_scale;
  scale[3] = sf.ov_hb_scale;
}
void cref_qmf_analysis_32(void *h, FIXP_DBL *re, FIXP_DBL *im, INT *scale, const INT *timeIn, INT timeIn_e, INT stride, FIXP_DBL *work) {
  FIXP_DBL *pr[QMF_MAX_COLS], *pi[QMF_MAX_COLS];
  for (int i = 0; i < QMF_MAX_COLS; i++) {
    pr[i] = re + i * QMF_SLOT;
    pi[i] = im + i * QMF_SLOT;
  }
  QMF_SCALE_FACTOR sf = {scale[0], scale[1], scale[2], scale[3]};
  qmfAnalysisFiltering((HANDLE_QMF_FILTER_BANK)h, pr, pi, &sf, (const LONG *)timeIn, timeIn_e,
                       stride, work);
  scale[0] = sf.lb_scale;
  scale[1] = sf.ov_lb_scale;
  scale[2] = sf.hb_scale;
  scale[3] = sf.ov_hb_scale;
}

void cref_qmf_synthesis_slot_16(void *h, const FIXP_DBL *re, const FIXP_DBL *im, INT sfl, INT sfh, SHORT *timeOut, INT stride, FIXP_DBL *work) {
  qmfSynthesisFilteringSlot((HANDLE_QMF_FILTER_BANK)h, re, im, sfl, sfh, (INT_PCM *)timeOut,
                            stride, work);
}
void cref_qmf_synthesis_slot_32(void *h, const FIXP_DBL *re, const FIXP_DBL *im, INT sfl, INT sfh, INT *timeOut, INT stride, FIXP_DBL *work) {
  qmfSynthesisFilteringSlot((HANDLE_QMF_FILTER_BANK)h, re, im, sfl, sfh, (LONG *)timeOut, stride,
                            work);
}
void cref_qmf_synthesis_16(void *h, FIXP_DBL *re, FIXP_DBL *im, const INT *scale, INT ov_len, SHORT *timeOut, INT stride, FIXP_DBL *work) {
  FIXP_DBL *pr[QMF_MAX_COLS], *pi[QMF_MAX_COLS];
  for (int i = 0; i < QMF_MAX_COLS; i++) {
    pr[i] = re + i * QMF_SLOT;
    pi[i] = im + i * QMF_SLOT;
  }
  QMF_SCALE_FACTOR sf = {scale[0], scale[1], scale[2], scale[3]};
  qmfSynthesisFiltering((HANDLE_QMF_FILTER_BANK)h, pr, pi, &sf, ov_len, (INT_PCM *)timeOut, stride,
                        work);
}
void cref_qmf_synthesis_32(void *h, FIXP_DBL *re, FIXP_DBL *im, const INT *scale, INT ov_len, INT *timeOut, INT stride, FIXP_DBL *work) {
  FIXP_DBL *pr[QMF_MAX_COLS], *pi[QMF_MAX_COLS];
  for (int i = 0; i < QMF_MAX_COLS; i++) {
    pr[i] = re + i * QMF_SLOT;
    pi[i] = im + i * QMF_SLOT;
  }
  QMF_SCALE_FACTOR sf = {scale[0], scale[1], scale[2], scale[3]};
  qmfSynthesisFiltering((HANDLE_QMF_FILTER_BANK)h, pr, pi, &sf, ov_len, (LONG *)timeOut, stride,
                        work);
}
void cref_qmf_syn_prototype_fir_slot_16(void *h, FIXP_DBL *re, FIXP_DBL *im, SHORT *timeOut, INT stride) {
  qmfSynPrototypeFirSlot((HANDLE_QMF_FILTER_BANK)h, re, im, (INT_PCM *)timeOut, stride);
}
void cref_qmf_syn_prototype_fir_slot_32(void *h, FIXP_DBL *re, FIXP_DBL *im, INT *timeOut, INT stride) {
  qmfSynPrototypeFirSlot((HANDLE_QMF_FILTER_BANK)h, re, im, (LONG *)timeOut, stride);
}
}
