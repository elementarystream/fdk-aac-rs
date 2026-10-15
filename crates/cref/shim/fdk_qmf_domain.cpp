// Shims for the differential tests of fdk_aac_rs_fdk::qmf_domain.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_qmf_domain (see build.rs).
//
// An FDK_QMF_DOMAIN lives on the C++ side behind an opaque handle (zero-initialized, like the
// decoder's). The functions call the library's FDK_qmf_domain.cpp. Pointers into the domain's
// memory are dumped as (kind, index, offset): kind 0 NULL, 1 work buffer section, 2 overlap
// buffer of an input channel, -1 another pointer. A pointer belongs to the buffer with the
// nearest base address at or below it (the allocations do not overlap).

#include <stdlib.h>
#include "cref_common.h"
#include "FDK_qmf_domain.h"
#include "FDK_tools_rom.h"

#define QD_CH ((8) + (1))

static void decode_ptr(const FDK_QMF_DOMAIN *qd, const FIXP_DBL *p, INT *out) {
  out[0] = 0;
  out[1] = 0;
  out[2] = 0;
  if (p == NULL) return;
  const FIXP_DBL *best = NULL;
  INT kind = -1, idx = 0;
  for (int k = 0; k < QMF_MAX_WB_SECTIONS; k++) {
    const FIXP_DBL *b = qd->globalConf.pWorkBuffer[k];
    if (b != NULL && b <= p && (best == NULL || b > best)) {
      best = b;
      kind = 1;
      idx = k;
    }
  }
  for (int ch = 0; ch < QD_CH; ch++) {
    const FIXP_DBL *b = qd->QmfDomainIn[ch].pOverlapBuffer;
    if (b != NULL && b <= p && (best == NULL || b > best)) {
      best = b;
      kind = 2;
      idx = ch;
    }
  }
  /* The largest buffers: work buffer sections of QMF_WB_SECTION_SIZE, overlap buffers of
     2 * 12 * 64 values. */
  if (best == NULL || (kind == 1 && p - best >= QMF_WB_SECTION_SIZE) ||
      (kind == 2 && p - best >= 2 * 12 * 64)) {
    out[0] = -1;
    return;
  }
  out[0] = kind;
  out[1] = idx;
  out[2] = (INT)(p - best);
}

/* The fields of a filter bank (as cref_qmf_dump, without the table identities, which the qmf
   tests cover): FilterStates relation, FilterSize, filterScale, no_channels, no_col, lsb, usb,
   synScalefactor, outScalefactor, outGain_m, outGain_e, flags, p_stride, p_filter != NULL,
   t_cos != NULL. */
static void dump_fb(const QMF_FILTER_BANK *q, const void *states, INT *out) {
  out[0] = q->FilterStates == NULL ? 0 : (q->FilterStates == states ? 1 : -1);
  out[1] = q->FilterSize;
  out[2] = q->filterScale;
  out[3] = q->no_channels;
  out[4] = q->no_col;
  out[5] = q->lsb;
  out[6] = q->usb;
  out[7] = q->synScalefactor;
  out[8] = q->outScalefactor;
  out[9] = q->outGain_m;
  out[10] = q->outGain_e;
  out[11] = (INT)q->flags;
  out[12] = q->p_stride;
  out[13] = q->p_filter != NULL;
  out[14] = q->t_cos != NULL;
}

extern "C" {
void *cref_qd_new(void) { return calloc(1, sizeof(FDK_QMF_DOMAIN)); }
void cref_qd_free(void *h) {
  FDK_QmfDomain_Close((HANDLE_FDK_QMF_DOMAIN)h);
  free(h);
}

/* The configuration: v[22] in the order of the fields of FDK_QMF_DOMAIN_GC, without
   pWorkBuffer. */
void cref_qd_set_gc(void *h, const INT *v) {
  FDK_QMF_DOMAIN_GC *g = &((FDK_QMF_DOMAIN *)h)->globalConf;
  g->qmfDomainExplicitConfig = (UCHAR)v[0];
  g->nInputChannels = (UCHAR)v[1];
  g->nInputChannels_requested = (UCHAR)v[2];
  g->nOutputChannels = (UCHAR)v[3];
  g->nOutputChannels_requested = (UCHAR)v[4];
  g->parkChannel = (UCHAR)v[5];
  g->parkChannel_requested = (UCHAR)v[6];
  g->flags = (UINT)v[7];
  g->flags_requested = (UINT)v[8];
  g->nBandsAnalysis = (UCHAR)v[9];
  g->nBandsAnalysis_requested = (UCHAR)v[10];
  g->nBandsSynthesis = (USHORT)v[11];
  g->nBandsSynthesis_requested = (USHORT)v[12];
  g->nQmfTimeSlots = (UCHAR)v[13];
  g->nQmfTimeSlots_requested = (UCHAR)v[14];
  g->nQmfOvTimeSlots = (UCHAR)v[15];
  g->nQmfOvTimeSlots_requested = (UCHAR)v[16];
  g->nQmfProcBands = (UCHAR)v[17];
  g->nQmfProcBands_requested = (UCHAR)v[18];
  g->nQmfProcChannels = (UCHAR)v[19];
  g->nQmfProcChannels_requested = (UCHAR)v[20];
}
/* out[27]: the 21 fields of cref_qd_set_gc, then pWorkBuffer[k] != NULL for 5 sections, then
   IsInitialized. */
void cref_qd_get_gc(void *h, INT *out) {
  FDK_QMF_DOMAIN *qd = (FDK_QMF_DOMAIN *)h;
  FDK_QMF_DOMAIN_GC *g = &qd->globalConf;
  out[0] = g->qmfDomainExplicitConfig;
  out[1] = g->nInputChannels;
  out[2] = g->nInputChannels_requested;
  out[3] = g->nOutputChannels;
  out[4] = g->nOutputChannels_requested;
  out[5] = g->parkChannel;
  out[6] = g->parkChannel_requested;
  out[7] = (INT)g->flags;
  out[8] = (INT)g->flags_requested;
  out[9] = g->nBandsAnalysis;
  out[10] = g->nBandsAnalysis_requested;
  out[11] = g->nBandsSynthesis;
  out[12] = g->nBandsSynthesis_requested;
  out[13] = g->nQmfTimeSlots;
  out[14] = g->nQmfTimeSlots_requested;
  out[15] = g->nQmfOvTimeSlots;
  out[16] = g->nQmfOvTimeSlots_requested;
  out[17] = g->nQmfProcBands;
  out[18] = g->nQmfProcBands_requested;
  out[19] = g->nQmfProcChannels;
  out[20] = g->nQmfProcChannels_requested;
  for (int k = 0; k < QMF_MAX_WB_SECTIONS; k++) out[21 + k] = g->pWorkBuffer[k] != NULL;
  out[26] = FDK_QmfDomain_IsInitialized(qd);
}

/* out[30]: fb (15, see dump_fb), scaling (4), workBuf_nTimeSlots, workBuf_nBands,
   workBufferOffset, workBufferSectSize, pAnaQmfStates != NULL, pOverlapBuffer != NULL,
   pWorkBuffer (0 NULL, 1 globalConf.pWorkBuffer, -1 other), pGlobalConf (0, 1, -1),
   hQmfSlotsReal != NULL, hQmfSlotsImag != NULL, 1 spare. */
void cref_qd_in_state(void *h, INT ch, INT *out) {
  FDK_QMF_DOMAIN *qd = (FDK_QMF_DOMAIN *)h;
  FDK_QMF_DOMAIN_IN *in = &qd->QmfDomainIn[ch];
  dump_fb(&in->fb, in->pAnaQmfStates, out);
  out[15] = in->scaling.lb_scale;
  out[16] = in->scaling.ov_lb_scale;
  out[17] = in->scaling.hb_scale;
  out[18] = in->scaling.ov_hb_scale;
  out[19] = in->workBuf_nTimeSlots;
  out[20] = in->workBuf_nBands;
  out[21] = in->workBufferOffset;
  out[22] = in->workBufferSectSize;
  out[23] = in->pAnaQmfStates != NULL;
  out[24] = in->pOverlapBuffer != NULL;
  out[25] = in->pWorkBuffer == NULL ? 0 : (in->pWorkBuffer == qd->globalConf.pWorkBuffer ? 1 : -1);
  out[26] = in->pGlobalConf == NULL ? 0 : (in->pGlobalConf == &qd->globalConf ? 1 : -1);
  out[27] = in->hQmfSlotsReal != NULL;
  out[28] = in->hQmfSlotsImag != NULL;
  out[29] = 0;
}
/* out[6 * n]: real and imaginary slot pointer of time slots 0..n. */
void cref_qd_in_slots(void *h, INT ch, INT n, INT *out) {
  FDK_QMF_DOMAIN *qd = (FDK_QMF_DOMAIN *)h;
  FDK_QMF_DOMAIN_IN *in = &qd->QmfDomainIn[ch];
  for (int ts = 0; ts < n; ts++) {
    decode_ptr(qd, in->hQmfSlotsReal[ts], out + 6 * ts);
    decode_ptr(qd, in->hQmfSlotsImag[ts], out + 6 * ts + 3);
  }
}
/* out[17]: fb (15), pSynQmfStates != NULL, 1 spare. */
void cref_qd_out_state(void *h, INT ch, INT *out) {
  FDK_QMF_DOMAIN *qd = (FDK_QMF_DOMAIN *)h;
  FDK_QMF_DOMAIN_OUT *o = &qd->QmfDomainOut[ch];
  dump_fb(&o->fb, o->pSynQmfStates, out);
  out[15] = o->pSynQmfStates != NULL;
  out[16] = 0;
}
/* Sets fields of a filter bank as SBR and MPS do: lsb, usb, outGain. */
void cref_qd_set_fb(void *h, INT out, INT ch, INT lsb, INT usb, FIXP_DBL gain_m, INT gain_e) {
  FDK_QMF_DOMAIN *qd = (FDK_QMF_DOMAIN *)h;
  QMF_FILTER_BANK *fb = out ? &qd->QmfDomainOut[ch].fb : &qd->QmfDomainIn[ch].fb;
  fb->lsb = lsb;
  fb->usb = usb;
  fb->outGain_m = gain_m;
  fb->outGain_e = gain_e;
}
void cref_qd_set_scaling(void *h, INT ch, const INT *s) {
  QMF_SCALE_FACTOR *sc = &((FDK_QMF_DOMAIN *)h)->QmfDomainIn[ch].scaling;
  sc->lb_scale = s[0];
  sc->ov_lb_scale = s[1];
  sc->hb_scale = s[2];
  sc->ov_hb_scale = s[3];
}
/* The memory of the domain: kind 0 work buffer section idx, 1 overlap buffer of input channel
   idx, 2 analysis states of input channel idx, 3 synthesis states of output channel idx.
   Returns 0 (and copies nothing) for NULL. */
static FIXP_DBL *mem_of(void *h, INT kind, INT idx) {
  FDK_QMF_DOMAIN *qd = (FDK_QMF_DOMAIN *)h;
  switch (kind) {
    case 0:
      return qd->globalConf.pWorkBuffer[idx];
    case 1:
      return qd->QmfDomainIn[idx].pOverlapBuffer;
    case 2:
      return qd->QmfDomainIn[idx].pAnaQmfStates;
    default:
      return qd->QmfDomainOut[idx].pSynQmfStates;
  }
}
INT cref_qd_get_mem(void *h, INT kind, INT idx, INT n, FIXP_DBL *out) {
  FIXP_DBL *p = mem_of(h, kind, idx);
  if (p == NULL) return 0;
  memcpy(out, p, n * sizeof(FIXP_DBL));
  return 1;
}
INT cref_qd_set_mem(void *h, INT kind, INT idx, INT n, const FIXP_DBL *v) {
  FIXP_DBL *p = mem_of(h, kind, idx);
  if (p == NULL) return 0;
  memcpy(p, v, n * sizeof(FIXP_DBL));
  return 1;
}

INT cref_qd_configure(void *h) { return FDK_QmfDomain_Configure((HANDLE_FDK_QMF_DOMAIN)h); }
void cref_qd_clear_requested(void *h) {
  FDK_QmfDomain_ClearRequested(&((FDK_QMF_DOMAIN *)h)->globalConf);
}
void cref_qd_free_mem(void *h) { FDK_QmfDomain_FreeMem((HANDLE_FDK_QMF_DOMAIN)h); }
void cref_qd_close(void *h) { FDK_QmfDomain_Close((HANDLE_FDK_QMF_DOMAIN)h); }
INT cref_qd_clear_persistent_memory(void *h) {
  return FDK_QmfDomain_ClearPersistentMemory((HANDLE_FDK_QMF_DOMAIN)h);
}
INT cref_qd_init_filter_bank(void *h, UINT extra_flags) {
  return FDK_QmfDomain_InitFilterBank((HANDLE_FDK_QMF_DOMAIN)h, extra_flags);
}
void cref_qd_save_overlap(void *h, INT ch, INT offset) {
  FDK_QmfDomain_SaveOverlap(&((FDK_QMF_DOMAIN *)h)->QmfDomainIn[ch], offset);
}
/* im may be NULL. */
void cref_qd_get_slot(void *h, INT ch, INT ts, INT start_band, INT stop_band, FIXP_DBL *re, FIXP_DBL *im, INT exp_out) {
  FDK_QmfDomain_GetSlot(&((FDK_QMF_DOMAIN *)h)->QmfDomainIn[ch], ts, start_band, stop_band, re,
                        im, exp_out);
}
/* out[6]: the decoded real and imaginary pointers. */
void cref_qd_get_work_buffer(void *h, INT ch, INT ts, INT *out) {
  FDK_QMF_DOMAIN *qd = (FDK_QMF_DOMAIN *)h;
  FIXP_DBL *re, *im;
  FDK_QmfDomain_GetWorkBuffer(&qd->QmfDomainIn[ch], ts, &re, &im);
  decode_ptr(qd, re, out);
  decode_ptr(qd, im, out + 3);
}
void cref_qd_work_buffer2proc_channel(void *h, INT ch) {
  FDK_QmfDomain_WorkBuffer2ProcChannel(&((FDK_QMF_DOMAIN *)h)->QmfDomainIn[ch]);
}
/* The QMF analysis of input channel ch into its time slots after the overlap, and the
   synthesis of output channel ch from all slots of input channel ch, as the SBR decoder runs
   them (sbr_dec.cpp). */
void cref_qd_analysis(void *h, INT ch, const INT *timeIn, INT timeIn_e) {
  FDK_QMF_DOMAIN *qd = (FDK_QMF_DOMAIN *)h;
  FDK_QMF_DOMAIN_IN *in = &qd->QmfDomainIn[ch];
  FIXP_DBL work[2 * 64];
  int ov = qd->globalConf.nQmfOvTimeSlots;
  qmfAnalysisFiltering(&in->fb, in->hQmfSlotsReal + ov, in->hQmfSlotsImag + ov, &in->scaling,
                       (const LONG *)timeIn, timeIn_e, 1, work);
}
void cref_qd_synthesis(void *h, INT ch, INT *timeOut) {
  FDK_QMF_DOMAIN *qd = (FDK_QMF_DOMAIN *)h;
  FDK_QMF_DOMAIN_IN *in = &qd->QmfDomainIn[ch];
  FIXP_DBL work[2 * 64];
  qmfSynthesisFiltering(&qd->QmfDomainOut[ch].fb, in->hQmfSlotsReal, in->hQmfSlotsImag,
                        &in->scaling, qd->globalConf.nQmfOvTimeSlots, (LONG *)timeOut, 1, work);
}
/* hbeRe, hbeIm: slots of 64 values. */
void cref_qd_qmf_data2hbe(void *h, INT ch, FIXP_DBL *hbeRe, FIXP_DBL *hbeIm) {
  FIXP_DBL *pr[64], *pi[64];
  for (int i = 0; i < 64; i++) {
    pr[i] = hbeRe + 64 * i;
    pi[i] = hbeIm + 64 * i;
  }
  FDK_QmfDomain_QmfData2HBE(&((FDK_QMF_DOMAIN *)h)->QmfDomainIn[ch], pr, pi);
}
}
