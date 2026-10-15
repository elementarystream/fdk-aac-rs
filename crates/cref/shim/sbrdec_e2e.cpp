// Shims of the end-to-end differential test of the SBR decoder (tests/sbrdec/e2e.rs).
//
// A namespaced copy of the reference AAC decoder (sbrdec_capdec_*.cpp) decodes a stream. Its
// calls of the SBR decoder API and of the QMF domain configuration functions go to the capture
// wrappers below (sbrcap_*, qmfcap_*). Every wrapper
//   1. copies the inputs of the call (bitstream state and buffer, time signals, ...) into the
//      event `ev` and the QMF domain configuration of the decoder into ev.i[2..15],
//   2. runs the call on the replay instance (a second reference SBR decoder with its own QMF
//      domain, driven only by the captured events),
//   3. calls the test's callback, which runs the same call on the port with the event data and
//      compares results and states with the replay instance (cref_sbrdec_e2e_dump),
//   4. runs the call on the original instance of the decoder and compares its results and
//      state with the replay instance (a check of the capture itself).
// MPEG Surround streams are not supported (the MPS decoder uses the QMF domain directly).
//
// The state of the harness (g, g_dump, g_dec, g_scribble) is thread-local, so that the test can
// decode several streams at once on separate threads; the reference library itself has no
// mutable global state.

#include "sbrdec_capdec_prelude.cpp"

#undef sbrDecoder_Open
#undef sbrDecoder_InitElement
#undef sbrDecoder_FreeMem
#undef sbrDecoder_Header
#undef sbrDecoder_SetParam
#undef sbrDecoder_drcFeedChannel
#undef sbrDecoder_drcDisable
#undef sbrDecoder_Parse
#undef sbrDecoder_Apply
#undef sbrDecoder_Close
#undef sbrDecoder_GetLibInfo
#undef sbrDecoder_GetDelay
#undef FDK_QmfDomain_ClearRequested
#undef FDK_QmfDomain_Configure
#undef FDK_QmfDomain_FreeMem
#undef FDK_QmfDomain_ClearPersistentMemory
#undef FDK_QmfDomain_Close

#include <stdlib.h>
#include "libSBRdec/src/sbr_ram.h"
#include "libSBRdec/src/sbr_dec.h"
#include "libSBRdec/src/psdec.h"
#include "libSBRdec/src/hbe.h"

/* HBE_MAX_OUT_SLOTS of hbe.cpp (the number of rows of qmfHBEBufReal_F) */
#define HBE_MAX_OUT_SLOTS (11)

namespace capdec {
#include "aacdecoder_lib.h"
}

enum {
  EV_OPEN = 1,
  EV_INIT_ELEMENT = 2,
  EV_HEADER = 3,
  EV_SET_PARAM = 4,
  EV_PARSE = 5,
  EV_APPLY = 6,
  EV_DRC_FEED = 7,
  EV_DRC_DISABLE = 8,
  EV_FREE_MEM = 9,
  EV_CLOSE = 10,
  EV_GET_DELAY = 11,
  EV_QMF_CLEAR_REQUESTED = 20,
  EV_QMF_CONFIGURE = 21,
  EV_QMF_FREE_MEM = 22,
  EV_QMF_CLEAR_PERSISTENT = 23,
  EV_QMF_CLOSE = 24
};

/* Arrays of an event (cref_sbrdec_e2e_ev_len/ev_copy). */
enum {
  A_BS = 0,       /* bitstream buffer (UCHAR, bufSize) */
  A_DRM_IN = 1,   /* pDrmBsBuffer before the call (UCHAR, drmBsBufferSize) */
  A_DRM_OUT = 2,  /* pDrmBsBuffer after the replay call */
  A_INPUT = 3,    /* Apply input (LONG) */
  A_TIME_IN = 4,  /* Apply timeData before the call (LONG, timeDataSize) */
  A_TIME_OUT = 5, /* Apply timeData after the replay call */
  A_DRC_MAG = 6,  /* drcFeedChannel pNextFact_mag (FIXP_DBL, numBands) */
  A_DRC_TOP = 7,  /* drcFeedChannel pBandTop (USHORT, numBands) */
  A_ACEL = 8,     /* Parse acElFlags (UINT, elementIndex + 1) */
  A_NUM = 9
};

typedef void (*CrefSbrCallback)(void *ctx, INT kind);

struct Event {
  INT i[64];
  void *a[A_NUM];
  INT len[A_NUM]; /* element counts */
  INT elsize[A_NUM];
};

static thread_local struct {
  HANDLE_SBRDECODER rep;
  FDK_QMF_DOMAIN repQd;
  HANDLE_FDK_QMF_DOMAIN origQd;
  CrefSbrCallback cb;
  void *ctx;
  Event ev;
  INT nEvents;
  INT origMismatch;   /* index of the first event where original and replay differ, -1 */
  INT origMismatchAt; /* index of the first different value of the state dump */
  INT checkOrig;
  FDK_QMF_DOMAIN_GC injectGc; /* configuration of the "decoder" in injection mode */
} g;

static void ev_reset(INT kind) {
  for (int k = 0; k < A_NUM; k++) {
    free(g.ev.a[k]);
    g.ev.a[k] = NULL;
    g.ev.len[k] = 0;
    g.ev.elsize[k] = 0;
  }
  memset(g.ev.i, 0, sizeof(g.ev.i));
  g.ev.i[0] = kind;
}

static void *ev_array(int k, int n, int elsize, const void *src) {
  g.ev.a[k] = calloc(n > 0 ? n : 1, elsize);
  g.ev.len[k] = n;
  g.ev.elsize[k] = elsize;
  if (src != NULL && n > 0) memcpy(g.ev.a[k], src, (size_t)n * elsize);
  return g.ev.a[k];
}

/* The configuration fields of the QMF domain that the AAC decoder sets directly. */
static void gc_capture(const FDK_QMF_DOMAIN_GC *gc, INT *o) {
  o[0] = gc->qmfDomainExplicitConfig;
  o[1] = gc->nInputChannels_requested;
  o[2] = gc->nOutputChannels_requested;
  o[3] = gc->parkChannel;
  o[4] = gc->parkChannel_requested;
  o[5] = (INT)gc->flags_requested;
  o[6] = gc->nBandsAnalysis_requested;
  o[7] = gc->nBandsSynthesis_requested;
  o[8] = gc->nQmfTimeSlots_requested;
  o[9] = gc->nQmfOvTimeSlots_requested;
  o[10] = gc->nQmfProcBands_requested;
  o[11] = gc->nQmfProcChannels_requested;
  o[12] = 0;
}
static void gc_apply(FDK_QMF_DOMAIN_GC *gc, const INT *o) {
  gc->qmfDomainExplicitConfig = (UCHAR)o[0];
  gc->nInputChannels_requested = (UCHAR)o[1];
  gc->nOutputChannels_requested = (UCHAR)o[2];
  gc->parkChannel = (UCHAR)o[3];
  gc->parkChannel_requested = (UCHAR)o[4];
  gc->flags_requested = (UINT)o[5];
  gc->nBandsAnalysis_requested = (UCHAR)o[6];
  gc->nBandsSynthesis_requested = (USHORT)o[7];
  gc->nQmfTimeSlots_requested = (UCHAR)o[8];
  gc->nQmfOvTimeSlots_requested = (UCHAR)o[9];
  gc->nQmfProcBands_requested = (UCHAR)o[10];
  gc->nQmfProcChannels_requested = (UCHAR)o[11];
}

/* Fills the stack below the caller with a pattern (g.scribble != 0), so that
   tests/sbrdec/api_fuzz.rs can find calls without a defined reference result. */
static thread_local INT g_scribble;
__attribute__((noinline)) static void scribble(void) {
  volatile UCHAR buf[256 * 1024];
  for (int k = 0; k < (int)sizeof(buf); k++) buf[k] = (UCHAR)(g_scribble * 37 + k * 11);
}

static void ev_begin(INT kind) {
  if (g_scribble != 0) scribble();
  ev_reset(kind);
  gc_capture(g.origQd != NULL ? &g.origQd->globalConf : &g.injectGc, &g.ev.i[2]);
  gc_apply(&g.repQd.globalConf, &g.ev.i[2]);
}

/* Bitstream state: CacheWord, BitsInCache, ConfigCache, ValidBits, ReadOffset, WriteOffset,
   BitNdx, bufSize. */
static void bs_capture(const FDK_BITSTREAM *bs, INT *o) {
  o[0] = (INT)bs->CacheWord;
  o[1] = (INT)bs->BitsInCache;
  o[2] = (INT)bs->ConfigCache;
  o[3] = (INT)bs->hBitBuf.ValidBits;
  o[4] = (INT)bs->hBitBuf.ReadOffset;
  o[5] = (INT)bs->hBitBuf.WriteOffset;
  o[6] = (INT)bs->hBitBuf.BitNdx;
  o[7] = (INT)bs->hBitBuf.bufSize;
}
/* A copy of a bitstream on the buffer copy `buf`. */
static void bs_copy(FDK_BITSTREAM *dst, const FDK_BITSTREAM *src, UCHAR *buf) {
  *dst = *src;
  dst->hBitBuf.Buffer = buf;
}

static void run_callback(void) {
  g.nEvents++;
  if (g.cb != NULL) g.cb(g.ctx, g.ev.i[0]);
  if (g.origQd == NULL) {
    /* injection mode: the "decoder" keeps the configuration the call left */
    INT v[13];
    gc_capture(&g.repQd.globalConf, v);
    gc_apply(&g.injectGc, v);
  }
}

/* ---------------------------------------------------------------------------------------- */
/* State dump. The order is mirrored by dump_port() in tests/sbrdec/e2e.rs.                  */

struct Dump {
  INT *v;
  int n, cap;
};

static void push(Dump *d, INT x) {
  if (d->n == d->cap) {
    d->cap = d->cap ? 2 * d->cap : 65536;
    d->v = (INT *)realloc(d->v, (size_t)d->cap * sizeof(INT));
  }
  d->v[d->n++] = x;
}
static void push_bytes(Dump *d, const void *p, int n) {
  const UCHAR *b = (const UCHAR *)p;
  for (int k = 0; k < n; k++) push(d, b[k]);
}
static void push_ints(Dump *d, const INT *p, int n) {
  for (int k = 0; k < n; k++) push(d, p[k]);
}
/* ILP32 (x86_32): FIXP_DBL is `long`, a type distinct from INT of the same size. */
#if !defined(__LP64__)
static void push_ints(Dump *d, const LONG *p, int n) { push_ints(d, (const INT *)p, n); }
#endif
static void mark(Dump *d, int id) { push(d, 0x5EC00000 + id); }

static void dump_fb(Dump *d, const QMF_FILTER_BANK *fb) {
  push(d, fb->FilterSize);
  push(d, fb->filterScale);
  push(d, fb->no_channels);
  push(d, fb->no_col);
  push(d, fb->lsb);
  push(d, fb->usb);
  push(d, fb->synScalefactor);
  push(d, fb->outScalefactor);
  push(d, fb->outGain_m);
  push(d, fb->outGain_e);
  push(d, (INT)fb->flags);
  push(d, fb->p_stride);
  push(d, fb->FilterStates != NULL);
}

static void dump_qd(Dump *d, const FDK_QMF_DOMAIN *qd) {
  const FDK_QMF_DOMAIN_GC *gc = &qd->globalConf;
  mark(d, 10);
  push(d, gc->qmfDomainExplicitConfig);
  push(d, gc->nInputChannels);
  push(d, gc->nInputChannels_requested);
  push(d, gc->nOutputChannels);
  push(d, gc->nOutputChannels_requested);
  push(d, gc->parkChannel);
  push(d, gc->parkChannel_requested);
  push(d, (INT)gc->flags);
  push(d, (INT)gc->flags_requested);
  push(d, gc->nBandsAnalysis);
  push(d, gc->nBandsAnalysis_requested);
  push(d, gc->nBandsSynthesis);
  push(d, gc->nBandsSynthesis_requested);
  push(d, gc->nQmfTimeSlots);
  push(d, gc->nQmfTimeSlots_requested);
  push(d, gc->nQmfOvTimeSlots);
  push(d, gc->nQmfOvTimeSlots_requested);
  push(d, gc->nQmfProcBands);
  push(d, gc->nQmfProcBands_requested);
  push(d, gc->nQmfProcChannels);
  push(d, gc->nQmfProcChannels_requested);
  for (int k = 0; k < QMF_MAX_WB_SECTIONS; k++) push(d, gc->pWorkBuffer[k] != NULL);
  for (int ch = 0; ch < ((8) + (1)); ch++) {
    const FDK_QMF_DOMAIN_IN *in = &qd->QmfDomainIn[ch];
    mark(d, 11);
    push(d, in->pGlobalConf != NULL);
    dump_fb(d, &in->fb);
    push(d, in->scaling.lb_scale);
    push(d, in->scaling.ov_lb_scale);
    push(d, in->scaling.hb_scale);
    push(d, in->scaling.ov_hb_scale);
    push(d, in->workBuf_nTimeSlots);
    push(d, in->workBuf_nBands);
    push(d, in->workBufferOffset);
    push(d, in->workBufferSectSize);
    push(d, in->pAnaQmfStates != NULL);
    if (in->pAnaQmfStates != NULL) {
      push_ints(d, (const INT *)in->pAnaQmfStates, 10 * in->fb.no_channels);
    }
    push(d, in->pOverlapBuffer != NULL);
    push(d, in->pWorkBuffer != NULL);
    push(d, in->hQmfSlotsReal != NULL);
    push(d, in->hQmfSlotsImag != NULL);
    int nSlots = gc->nQmfTimeSlots + gc->nQmfOvTimeSlots;
    int nBands = gc->nQmfProcBands;
    if (in->hQmfSlotsReal != NULL && in->pGlobalConf != NULL) {
      for (int s = 0; s < nSlots; s++) {
        push(d, in->hQmfSlotsReal[s] != NULL);
        if (in->hQmfSlotsReal[s] != NULL) push_ints(d, in->hQmfSlotsReal[s], nBands);
      }
    }
    if (in->hQmfSlotsImag != NULL && in->pGlobalConf != NULL) {
      for (int s = 0; s < nSlots; s++) {
        push(d, in->hQmfSlotsImag[s] != NULL);
        if (in->hQmfSlotsImag[s] != NULL) push_ints(d, in->hQmfSlotsImag[s], nBands);
      }
    }
  }
  for (int ch = 0; ch < ((8) + (1)); ch++) {
    const FDK_QMF_DOMAIN_OUT *out = &qd->QmfDomainOut[ch];
    mark(d, 12);
    dump_fb(d, &out->fb);
    push(d, out->pSynQmfStates != NULL);
    if (out->pSynQmfStates != NULL) {
      push_ints(d, (const INT *)out->pSynQmfStates, 9 * out->fb.no_channels);
    }
  }
}

static INT ptr_off(const void *p, const void *base) {
  if (p == NULL) return -1;
  if (base == NULL) return -2;
  return (INT)(((const char *)p - (const char *)base) / (INT)sizeof(FIXP_DBL));
}

static void dump_ps(Dump *d, const PS_DEC *ps) {
  mark(d, 20);
  push(d, ps != NULL);
  if (ps == NULL) return;
  push(d, ps->noSubSamples);
  push(d, ps->noChannels);
  push(d, ps->procFrameBased);
  push(d, ps->bPsDataAvail[0]);
  push(d, ps->bPsDataAvail[1]);
  push(d, ps->psDecodedPrv);
  push(d, ps->bsLastSlot);
  push(d, ps->bsReadSlot);
  push(d, ps->processSlot);
  push_bytes(d, &ps->bsData[0].mpeg, sizeof(MPEG_PS_BS_DATA));
  push_bytes(d, &ps->bsData[1].mpeg, sizeof(MPEG_PS_BS_DATA));
  const auto *m = &ps->specificTo.mpeg;
  mark(d, 21);
  push_bytes(d, m->aIidPrevFrameIndex, sizeof(m->aIidPrevFrameIndex));
  push_bytes(d, m->aIccPrevFrameIndex, sizeof(m->aIccPrevFrameIndex));
  push(d, m->bPrevFrameFineIidQ);
  push(d, m->prevFreqResIid);
  push(d, m->prevFreqResIcc);
  push(d, m->lastUsb);
  push_ints(d, m->pHybridAnaStatesLFdmx, 2 * 13 * NO_QMF_BANDS_HYBRID20);
  const FDK_ANA_HYB_FILTER *a = &m->hybridAnalysis;
  mark(d, 22);
  for (int k = 0; k < 3; k++) push(d, ptr_off(a->bufferLFReal[k], a->pLFmemory));
  for (int k = 0; k < 3; k++) push(d, ptr_off(a->bufferLFImag[k], a->pLFmemory));
  push(d, a->bufferLFpos);
  push(d, a->bufferHFpos);
  push(d, a->nrBands);
  push(d, a->cplxBands);
  push(d, a->hfMode);
  push(d, (INT)a->LFmemorySize);
  push(d, (INT)a->HFmemorySize);
  for (int k = 0; k < 2; k++) {
    push(d, m->hybridSynthesis[k].nrBands);
    push(d, m->hybridSynthesis[k].cplxBands);
  }
  const DECORR_DEC *dc = &m->apDecor;
  mark(d, 23);
  push(d, dc->L_stateBufferCplx);
  push(d, dc->L_delayBufferCplx);
  push(d, ptr_off(dc->delayBufferCplx, dc->stateBufferCplx));
  push_ints(d, dc->reverbBandDelayBufferIndex, 4);
  push_bytes(d, dc->stateBufferOffset, 3);
  for (int k = 0; k < (71); k++) {
    push(d, ptr_off(dc->Filter[k].stateCplx, dc->stateBufferCplx));
    push(d, ptr_off(dc->Filter[k].DelayBufferCplx, dc->stateBufferCplx));
  }
  push(d, dc->ducker.hybridBands);
  push(d, dc->ducker.parameterBands);
  push(d, dc->ducker.partiallyComplex);
  push(d, (INT)dc->ducker.duckerType);
  push_ints(d, dc->ducker.SmoothDirRevNrg, 2 * (28));
  push_ints(d, dc->ducker.peakDecay, (28));
  push_ints(d, dc->ducker.peakDiff, (28));
  push(d, dc->ducker.maxValDirectData);
  push(d, dc->ducker.maxValReverbData);
  push(d, dc->ducker.scaleDirectNrg);
  push(d, dc->ducker.scaleReverbNrg);
  push(d, dc->ducker.scaleSmoothDirRevNrg);
  push(d, dc->ducker.headroomSmoothDirRevNrg);
  push(d, dc->numbins);
  push(d, dc->partiallyComplex);
  push(d, dc->stateBufferCplx == m->decorrBufferCplx);
  push_ints(d, m->decorrBufferCplx, (2 * ((825) + (373))));
  mark(d, 24);
  push_ints(d, m->h11rPrev, NO_IID_GROUPS);
  push_ints(d, m->h12rPrev, NO_IID_GROUPS);
  push_ints(d, m->h21rPrev, NO_IID_GROUPS);
  push_ints(d, m->h22rPrev, NO_IID_GROUPS);
}

static void dump_hbe(Dump *d, const struct hbeTransposer *h) {
  mark(d, 40);
  push(d, h != NULL);
  if (h == NULL) return;
  push_ints(d, h->anaQmfStates, HBE_QMF_FILTER_STATE_ANA_SIZE);
  push_ints(d, (const INT *)h->synQmfStates, HBE_QMF_FILTER_STATE_SYN_SIZE);
  push_ints(d, h->xOverQmf, MAX_NUM_PATCHES_HBE);
  push(d, h->maxStretch);
  push(d, h->timeDomainWinLen);
  push(d, h->qmfInBufSize);
  push(d, h->qmfOutBufSize);
  push(d, h->noCols);
  push(d, h->noChannels);
  push(d, h->startBand);
  push(d, h->stopBand);
  push(d, h->bSbr41);
  mark(d, 41);
  push_ints(d, h->inBuf_F, QMF_SYNTH_CHANNELS + 20 + 1);
  for (int s = 0; s < h->qmfInBufSize; s++) push_ints(d, h->qmfInBufReal_F[s], QMF_SYNTH_CHANNELS);
  for (int s = 0; s < h->qmfInBufSize; s++) push_ints(d, h->qmfInBufImag_F[s], QMF_SYNTH_CHANNELS);
  push_ints(d, h->qmfBufferCodecTempSlot_F, QMF_SYNTH_CHANNELS / 2);
  dump_fb(d, &h->HBEAnalysiscQMF);
  dump_fb(d, &h->HBESynthesisQMF);
  mark(d, 42);
  for (int s = 0; s < HBE_MAX_OUT_SLOTS; s++) push_ints(d, h->qmfHBEBufReal_F[s], QMF_SYNTH_CHANNELS);
  for (int s = 0; s < HBE_MAX_OUT_SLOTS; s++) push_ints(d, h->qmfHBEBufImag_F[s], QMF_SYNTH_CHANNELS);
  push_ints(d, h->bXProducts, MAX_STRETCH_HBE);
  push(d, h->kstart);
  push(d, h->synthSize);
  push_ints(d, h->highband_exp, 2);
  push_ints(d, h->target_exp, 2);
}

static void dump_matrix(Dump *d, FIXP_DBL **m, int rows) {
  push(d, m != NULL);
  if (m == NULL) return;
  int nch = rows > 1 ? (int)(m[1] - m[0]) : 0;
  push(d, rows * nch);
  if (rows > 0) push_ints(d, m[0], rows * nch);
}

static void dump_sbr(Dump *d, HANDLE_SBRDECODER h, const FDK_QMF_DOMAIN *qd) {
  mark(d, 1);
  push(d, h != NULL);
  if (h == NULL) {
    dump_qd(d, qd);
    return;
  }
  push(d, h->coreCodec);
  push(d, h->numSbrElements);
  push(d, h->numSbrChannels);
  push(d, h->sampleRateIn);
  push(d, h->sampleRateOut);
  push(d, h->codecFrameSize);
  push(d, h->synDownsampleFac);
  push(d, h->downscaleFactor);
  push(d, h->numDelayFrames);
  push(d, h->harmonicSBR);
  push(d, h->numFlushedFrames);
  push(d, (INT)h->flags);
  push(d, h->sbrInDataHeadroom);
  mark(d, 2);
  for (int el = 0; el < (8); el++) {
    for (int s = 0; s < (1) + 1; s++) {
      SBR_HEADER_DATA c = h->sbrHeader[el][s];
      c.freqBandData.freqBandTable[0] = NULL;
      c.freqBandData.freqBandTable[1] = NULL;
      push_bytes(d, &c, sizeof(c));
    }
  }
  dump_ps(d, h->hParametricStereoDec);
  for (int el = 0; el < (8); el++) {
    const SBR_DECODER_ELEMENT *e = h->pSbrElement[el];
    mark(d, 3);
    push(d, e != NULL);
    if (e == NULL) continue;
    push_bytes(d, &e->transposerSettings, sizeof(TRANSPOSER_SETTINGS));
    push(d, e->elementID);
    push(d, e->nChannels);
    push_bytes(d, e->frameErrorFlag, 2);
    push(d, e->useFrameSlot);
    push_bytes(d, e->useHeaderSlot, 2);
    for (int ch = 0; ch < SBRDEC_MAX_CH_PER_ELEMENT; ch++) {
      const SBR_CHANNEL *c = e->pSbrChannel[ch];
      mark(d, 4);
      push(d, c != NULL);
      if (c == NULL) continue;
      push_bytes(d, &c->frameData[0], sizeof(SBR_FRAME_DATA));
      push_bytes(d, &c->frameData[1], sizeof(SBR_FRAME_DATA));
      push_bytes(d, &c->prevFrameData, sizeof(SBR_PREV_FRAME_DATA));
      const SBR_DEC *sd = &c->SbrDec;
      mark(d, 5);
      SBR_CALCULATE_ENVELOPE ce = sd->SbrCalculateEnvelope;
      ce.prevFreqBandTable[0] = NULL;
      ce.prevFreqBandTable[1] = NULL;
      push_bytes(d, &ce, sizeof(ce));
      mark(d, 6);
      {
        static thread_local SBR_LPP_TRANS lt;
        lt = sd->LppTrans;
        lt.pSettings = NULL;
        push_bytes(d, &lt, sizeof(lt));
      }
      mark(d, 7);
      push_bytes(d, &sd->PvcStaticData, sizeof(PVC_STATIC_DATA));
      push(d, sd->scale_ov);
      push(d, sd->scale_lb);
      push(d, sd->scale_hbe);
      push(d, sd->prev_frame_lSbr);
      push(d, sd->prev_frame_hbeSbr);
      push(d, sd->codecFrameSize);
      push(d, sd->qmfDomainInCh == NULL ? -1 : (INT)(sd->qmfDomainInCh - h->pQmfDomain->QmfDomainIn));
      push(d, sd->qmfDomainOutCh == NULL ? -1 : (INT)(sd->qmfDomainOutCh - h->pQmfDomain->QmfDomainOut));
      push_bytes(d, &sd->sbrDrcChannel, sizeof(SBRDEC_DRC_CHANNEL));
      push(d, sd->savedStates);
      push(d, sd->applySbrProc_old);
      int rows = h->sbrHeader[el][0].numberTimeSlots * h->sbrHeader[el][0].timeStep;
      mark(d, 8);
      dump_matrix(d, sd->hQmfHBESlotsReal, rows);
      dump_matrix(d, sd->hQmfHBESlotsImag, rows);
      dump_matrix(d, sd->codecQMFBufferReal, rows);
      dump_matrix(d, sd->codecQMFBufferImag, rows);
      dump_hbe(d, sd->hHBE);
    }
  }
  dump_qd(d, qd);
}

static thread_local Dump g_dump;

/* ---------------------------------------------------------------------------------------- */
/* Capture wrappers.                                                                          */

static void check_orig(HANDLE_SBRDECODER orig, INT origRet) {
  if (!g.checkOrig || g.origMismatch >= 0) return;
  if (origRet != g.ev.i[1]) {
    g.origMismatch = g.nEvents - 1;
    g.origMismatchAt = -1;
    return;
  }
  Dump a = {0, 0, 0}, b = {0, 0, 0};
  dump_sbr(&a, orig, g.origQd);
  dump_sbr(&b, g.rep, &g.repQd);
  if (a.n != b.n) {
    g.origMismatch = g.nEvents - 1;
    g.origMismatchAt = -2;
  } else {
    for (int k = 0; k < a.n; k++) {
      if (a.v[k] != b.v[k]) {
        g.origMismatch = g.nEvents - 1;
        g.origMismatchAt = k;
        break;
      }
    }
  }
  free(a.v);
  free(b.v);
}


/* Replay of one call: copies the inputs into the event, runs the call on the replay instance,
   then the callback (the port). Used by the capture wrappers and by the injection functions of
   the API fuzz test. */

static void rep_open(void) {
  ev_begin(EV_OPEN);
  if (g.rep != NULL) sbrDecoder_Close(&g.rep);
  FDK_QmfDomain_Close(&g.repQd);
  FDKmemclear(&g.repQd, sizeof(g.repQd));
  g.ev.i[1] = sbrDecoder_Open(&g.rep, &g.repQd);
  run_callback();
}

static void rep_element_params(INT *i, const int sampleRateIn, const int sampleRateOut,
                               const int samplesPerFrame, const AUDIO_OBJECT_TYPE coreCodec,
                               const MP4_ELEMENT_ID elementID, const int elementIndex,
                               const UCHAR harmonicSBR, const UCHAR stereoConfigIndex,
                               const UCHAR configMode, UCHAR configChanged,
                               const INT downscaleFactor) {
  i[16] = sampleRateIn;
  i[17] = sampleRateOut;
  i[18] = samplesPerFrame;
  i[19] = coreCodec;
  i[20] = elementID;
  i[21] = elementIndex;
  i[22] = harmonicSBR;
  i[23] = stereoConfigIndex;
  i[24] = configMode;
  i[25] = configChanged;
  i[26] = downscaleFactor;
}

static void rep_init_element(const int sampleRateIn, const int sampleRateOut,
                             const int samplesPerFrame, const AUDIO_OBJECT_TYPE coreCodec,
                             const MP4_ELEMENT_ID elementID, const int elementIndex,
                             const UCHAR harmonicSBR, const UCHAR stereoConfigIndex,
                             const UCHAR configMode, UCHAR configChanged,
                             const INT downscaleFactor) {
  ev_begin(EV_INIT_ELEMENT);
  INT *i = g.ev.i;
  rep_element_params(i, sampleRateIn, sampleRateOut, samplesPerFrame, coreCodec, elementID,
                     elementIndex, harmonicSBR, stereoConfigIndex, configMode, configChanged,
                     downscaleFactor);
  UCHAR cc = configChanged;
  i[1] = sbrDecoder_InitElement(g.rep, sampleRateIn, sampleRateOut, samplesPerFrame, coreCodec,
                                elementID, elementIndex, harmonicSBR, stereoConfigIndex,
                                configMode, &cc, downscaleFactor);
  i[27] = cc;
  run_callback();
}

static void rep_header(const FDK_BITSTREAM *hBs, const int sampleRateIn, const int sampleRateOut,
                       const int samplesPerFrame, const AUDIO_OBJECT_TYPE coreCodec,
                       const MP4_ELEMENT_ID elementID, const int elementIndex,
                       const UCHAR harmonicSBR, const UCHAR stereoConfigIndex,
                       const UCHAR configMode, UCHAR configChanged, const INT downscaleFactor) {
  ev_begin(EV_HEADER);
  INT *i = g.ev.i;
  rep_element_params(i, sampleRateIn, sampleRateOut, samplesPerFrame, coreCodec, elementID,
                     elementIndex, harmonicSBR, stereoConfigIndex, configMode, configChanged,
                     downscaleFactor);
  bs_capture(hBs, &i[40]);
  UCHAR *buf = (UCHAR *)ev_array(A_BS, hBs->hBitBuf.bufSize, 1, hBs->hBitBuf.Buffer);
  FDK_BITSTREAM bs;
  bs_copy(&bs, hBs, buf);
  UCHAR cc = configChanged;
  i[1] = sbrDecoder_Header(g.rep, &bs, sampleRateIn, sampleRateOut, samplesPerFrame, coreCodec,
                           elementID, elementIndex, harmonicSBR, stereoConfigIndex, configMode,
                           &cc, downscaleFactor);
  i[27] = cc;
  bs_capture(&bs, &i[48]);
  /* the buffer is not written by the parser: restore it for the port */
  memcpy(buf, hBs->hBitBuf.Buffer, hBs->hBitBuf.bufSize);
  run_callback();
}

static void rep_set_param(int selfNull, const SBRDEC_PARAM param, const INT value) {
  ev_begin(EV_SET_PARAM);
  g.ev.i[16] = param;
  g.ev.i[17] = value;
  g.ev.i[18] = selfNull;
  g.ev.i[1] = sbrDecoder_SetParam(selfNull ? NULL : g.rep, param, value);
  run_callback();
}

static void rep_drc_feed(INT ch, UINT numBands, const FIXP_DBL *pNextFact_mag, INT nextFact_exp,
                         SHORT drcInterpolationScheme, UCHAR winSequence,
                         const USHORT *pBandTop) {
  ev_begin(EV_DRC_FEED);
  INT *i = g.ev.i;
  i[16] = ch;
  i[17] = (INT)numBands;
  i[18] = nextFact_exp;
  i[19] = drcInterpolationScheme;
  i[20] = winSequence;
  i[21] = pNextFact_mag == NULL;
  int n = (int)numBands;
  FIXP_DBL *mag = (FIXP_DBL *)ev_array(A_DRC_MAG, n, sizeof(FIXP_DBL), pNextFact_mag);
  USHORT *top = (USHORT *)ev_array(A_DRC_TOP, n, sizeof(USHORT), pBandTop);
  i[1] = sbrDecoder_drcFeedChannel(g.rep, ch, numBands, pNextFact_mag ? mag : NULL, nextFact_exp,
                                   drcInterpolationScheme, winSequence, top);
  run_callback();
}

static void rep_drc_disable(INT ch) {
  ev_begin(EV_DRC_DISABLE);
  g.ev.i[16] = ch;
  sbrDecoder_drcDisable(g.rep, ch);
  run_callback();
}

static void rep_parse(const FDK_BITSTREAM *hBs, const UCHAR *pDrmBsBuffer,
                      USHORT drmBsBufferSize, int count, int bsPayLen, int crcFlag,
                      MP4_ELEMENT_ID prev_element, int element_index, UINT acFlags,
                      const UINT *acElFlags) {
  ev_begin(EV_PARSE);
  INT *i = g.ev.i;
  i[16] = drmBsBufferSize;
  i[17] = count;
  i[18] = bsPayLen;
  i[19] = crcFlag;
  i[20] = prev_element;
  i[21] = element_index;
  i[22] = (INT)acFlags;
  i[25] = pDrmBsBuffer == NULL;
  bs_capture(hBs, &i[40]);
  UCHAR *buf = (UCHAR *)ev_array(A_BS, hBs->hBitBuf.bufSize, 1, hBs->hBitBuf.Buffer);
  int nDrm = pDrmBsBuffer ? drmBsBufferSize : 0;
  ev_array(A_DRM_IN, nDrm, 1, pDrmBsBuffer);
  UCHAR *drm = (UCHAR *)ev_array(A_DRM_OUT, nDrm, 1, pDrmBsBuffer);
  int nAc = element_index >= 0 && element_index < 64 ? element_index + 1 : 0;
  UINT *ac = (UINT *)ev_array(A_ACEL, nAc, sizeof(UINT), acElFlags);
  i[24] = nAc;
  FDK_BITSTREAM bs;
  bs_copy(&bs, hBs, buf);
  int cnt = count;
  i[1] = sbrDecoder_Parse(g.rep, &bs, pDrmBsBuffer ? drm : NULL, drmBsBufferSize, &cnt, bsPayLen,
                          crcFlag, prev_element, element_index, acFlags, ac);
  i[23] = cnt;
  bs_capture(&bs, &i[48]);
  memcpy(buf, hBs->hBitBuf.Buffer, hBs->hBitBuf.bufSize);
  run_callback();
}

/* `input` (nIn values; NULL: input == timeData) and the initial `timeData`. Returns the output
   of the replay instance (A_TIME_OUT). */
static LONG *rep_apply(const LONG *input, int nIn, const LONG *timeData, const int timeDataSize,
                       int numChannels, int sampleRate,
                       const FDK_channelMapDescr *const mapDescr, const int mapIdx,
                       const int coreDecodedOk, UCHAR psDecoded, const INT inDataHeadroom) {
  ev_begin(EV_APPLY);
  INT *i = g.ev.i;
  i[16] = timeDataSize;
  i[17] = numChannels;
  i[18] = sampleRate;
  i[19] = mapIdx;
  i[20] = coreDecodedOk;
  i[21] = psDecoded;
  i[22] = inDataHeadroom;
  i[27] = mapDescr ? (INT)mapDescr->fPassThrough : -1;
  i[28] = mapDescr ? (INT)mapDescr->mapInfoTabLen : -1;
  {
    FDK_channelMapDescr dflt;
    FDK_chMapDescr_init(&dflt, NULL, 0, 0);
    i[29] = mapDescr ? (mapDescr->pMapInfoTab == dflt.pMapInfoTab) : 0;
  }
  if (input == NULL) nIn = 0;
  i[30] = nIn;
  LONG *in = (LONG *)ev_array(A_INPUT, nIn, sizeof(LONG), input);
  ev_array(A_TIME_IN, timeDataSize, sizeof(LONG), timeData);
  LONG *out = (LONG *)ev_array(A_TIME_OUT, timeDataSize, sizeof(LONG), timeData);
  int nch = numChannels, sr = sampleRate, odh = 0;
  UCHAR psd = psDecoded;
  i[1] = sbrDecoder_Apply(g.rep, (input == NULL) ? out : in, out, timeDataSize, &nch, &sr,
                          mapDescr, mapIdx, coreDecodedOk, &psd, inDataHeadroom, &odh);
  i[23] = nch;
  i[24] = sr;
  i[25] = psd;
  i[26] = odh;
  run_callback();
  return out;
}

static void rep_simple(INT kind) {
  ev_begin(kind);
  switch (kind) {
    case EV_FREE_MEM: g.ev.i[1] = sbrDecoder_FreeMem(&g.rep); break;
    case EV_CLOSE: g.ev.i[1] = sbrDecoder_Close(&g.rep); break;
    case EV_GET_DELAY: g.ev.i[1] = (INT)sbrDecoder_GetDelay(g.rep); break;
    case EV_QMF_CLEAR_REQUESTED: FDK_QmfDomain_ClearRequested(&g.repQd.globalConf); break;
    case EV_QMF_CONFIGURE: g.ev.i[1] = FDK_QmfDomain_Configure(&g.repQd); break;
    case EV_QMF_FREE_MEM: FDK_QmfDomain_FreeMem(&g.repQd); break;
    case EV_QMF_CLEAR_PERSISTENT:
      g.ev.i[1] = FDK_QmfDomain_ClearPersistentMemory(&g.repQd);
      break;
    case EV_QMF_CLOSE: FDK_QmfDomain_Close(&g.repQd); break;
    default: break;
  }
  run_callback();
}

/* Capture wrappers: replay, then the original call. */

SBR_ERROR sbrcap_Open(HANDLE_SBRDECODER *pSelf, HANDLE_FDK_QMF_DOMAIN pQmfDomain) {
  g.origQd = pQmfDomain;
  rep_open();
  SBR_ERROR r = sbrDecoder_Open(pSelf, pQmfDomain);
  check_orig(*pSelf, r);
  return r;
}

SBR_ERROR sbrcap_InitElement(HANDLE_SBRDECODER self, const int sampleRateIn,
                             const int sampleRateOut, const int samplesPerFrame,
                             const AUDIO_OBJECT_TYPE coreCodec, const MP4_ELEMENT_ID elementID,
                             const int elementIndex, const UCHAR harmonicSBR,
                             const UCHAR stereoConfigIndex, const UCHAR configMode,
                             UCHAR *configChanged, const INT downscaleFactor) {
  rep_init_element(sampleRateIn, sampleRateOut, samplesPerFrame, coreCodec, elementID,
                   elementIndex, harmonicSBR, stereoConfigIndex, configMode, *configChanged,
                   downscaleFactor);
  SBR_ERROR r = sbrDecoder_InitElement(self, sampleRateIn, sampleRateOut, samplesPerFrame,
                                       coreCodec, elementID, elementIndex, harmonicSBR,
                                       stereoConfigIndex, configMode, configChanged,
                                       downscaleFactor);
  check_orig(self, r);
  return r;
}

INT sbrcap_Header(HANDLE_SBRDECODER self, HANDLE_FDK_BITSTREAM hBs, const INT sampleRateIn,
                  const INT sampleRateOut, const INT samplesPerFrame,
                  const AUDIO_OBJECT_TYPE coreCodec, const MP4_ELEMENT_ID elementID,
                  const INT elementIndex, const UCHAR harmonicSBR,
                  const UCHAR stereoConfigIndex, const UCHAR configMode, UCHAR *configChanged,
                  const INT downscaleFactor) {
  rep_header(hBs, sampleRateIn, sampleRateOut, samplesPerFrame, coreCodec, elementID,
             elementIndex, harmonicSBR, stereoConfigIndex, configMode, *configChanged,
             downscaleFactor);
  INT r = sbrDecoder_Header(self, hBs, sampleRateIn, sampleRateOut, samplesPerFrame, coreCodec,
                            elementID, elementIndex, harmonicSBR, stereoConfigIndex, configMode,
                            configChanged, downscaleFactor);
  check_orig(self, r);
  return r;
}

SBR_ERROR sbrcap_SetParam(HANDLE_SBRDECODER self, const SBRDEC_PARAM param, const INT value) {
  rep_set_param(self == NULL, param, value);
  SBR_ERROR r = sbrDecoder_SetParam(self, param, value);
  check_orig(self, r);
  return r;
}

SBR_ERROR sbrcap_drcFeedChannel(HANDLE_SBRDECODER self, INT ch, UINT numBands,
                                FIXP_DBL *pNextFact_mag, INT nextFact_exp,
                                SHORT drcInterpolationScheme, UCHAR winSequence,
                                USHORT *pBandTop) {
  rep_drc_feed(ch, numBands, pNextFact_mag, nextFact_exp, drcInterpolationScheme, winSequence,
               pBandTop);
  SBR_ERROR r = sbrDecoder_drcFeedChannel(self, ch, numBands, pNextFact_mag, nextFact_exp,
                                          drcInterpolationScheme, winSequence, pBandTop);
  check_orig(self, r);
  return r;
}

void sbrcap_drcDisable(HANDLE_SBRDECODER self, INT ch) {
  rep_drc_disable(ch);
  sbrDecoder_drcDisable(self, ch);
  check_orig(self, 0);
}

SBR_ERROR sbrcap_Parse(HANDLE_SBRDECODER self, HANDLE_FDK_BITSTREAM hBs, UCHAR *pDrmBsBuffer,
                       USHORT drmBsBufferSize, int *count, int bsPayLen, int crcFlag,
                       MP4_ELEMENT_ID prev_element, int element_index, UINT acFlags,
                       UINT acElFlags[]) {
  rep_parse(hBs, pDrmBsBuffer, drmBsBufferSize, *count, bsPayLen, crcFlag, prev_element,
            element_index, acFlags, acElFlags);
  SBR_ERROR r = sbrDecoder_Parse(self, hBs, pDrmBsBuffer, drmBsBufferSize, count, bsPayLen,
                                 crcFlag, prev_element, element_index, acFlags, acElFlags);
  check_orig(self, r);
  return r;
}

SBR_ERROR sbrcap_Apply(HANDLE_SBRDECODER self, LONG *input, LONG *timeData,
                       const int timeDataSize, int *numChannels, int *sampleRate,
                       const FDK_channelMapDescr *const mapDescr, const int mapIdx,
                       const int coreDecodedOk, UCHAR *psDecoded, const INT inDataHeadroom,
                       INT *outDataHeadroom) {
  /* The input is the AAC decoder's workBufferCore2 ((8) * 1024 values); the whole buffer is
     copied. */
  LONG *out = rep_apply((input == timeData) ? NULL : input, (8) * 1024, timeData, timeDataSize,
                        *numChannels, *sampleRate, mapDescr, mapIdx, coreDecodedOk, *psDecoded,
                        inDataHeadroom);
  SBR_ERROR r = sbrDecoder_Apply(self, input, timeData, timeDataSize, numChannels, sampleRate,
                                 mapDescr, mapIdx, coreDecodedOk, psDecoded, inDataHeadroom,
                                 outDataHeadroom);
  check_orig(self, r);
  /* The AAC decoder passes its WorkBufferCore5 ((8) * (1024 * 4) * 2 values, FDKaalloc) with
     the size GetRequiredMemWorkBufferCore5() / sizeof(LONG), which counts the alignment overhead:
     only the first 65536 + sizeof(void *) / 4 values have defined contents and are compared. */
  INT cmpSize = timeDataSize;
  const INT core5 = (8) * (1024 * 4) * 2;
  if (timeDataSize == (INT)(ALGN_SIZE_EXTRES(core5 * sizeof(LONG) + ALIGNMENT_DEFAULT +
                                             sizeof(void *)) /
                            sizeof(LONG)))
    cmpSize = core5 + (INT)(sizeof(void *) / sizeof(LONG));
  if (g.checkOrig && g.origMismatch < 0 && r == 0 &&
      memcmp(out, timeData, (size_t)cmpSize * sizeof(LONG)) != 0) {
    g.origMismatch = g.nEvents - 1;
    g.origMismatchAt = -3;
  }
  return r;
}

SBR_ERROR sbrcap_FreeMem(HANDLE_SBRDECODER *self) {
  rep_simple(EV_FREE_MEM);
  SBR_ERROR r = sbrDecoder_FreeMem(self);
  check_orig(*self, r);
  return r;
}

SBR_ERROR sbrcap_Close(HANDLE_SBRDECODER *self) {
  rep_simple(EV_CLOSE);
  return sbrDecoder_Close(self);
}

INT sbrcap_GetLibInfo(LIB_INFO *info) { return sbrDecoder_GetLibInfo(info); }

UINT sbrcap_GetDelay(const HANDLE_SBRDECODER self) {
  rep_simple(EV_GET_DELAY);
  UINT r = sbrDecoder_GetDelay(self);
  check_orig(self, (INT)r);
  return r;
}

void qmfcap_ClearRequested(HANDLE_FDK_QMF_DOMAIN_GC hgc) {
  rep_simple(EV_QMF_CLEAR_REQUESTED);
  FDK_QmfDomain_ClearRequested(hgc);
}

QMF_DOMAIN_ERROR qmfcap_Configure(HANDLE_FDK_QMF_DOMAIN hqd) {
  rep_simple(EV_QMF_CONFIGURE);
  return FDK_QmfDomain_Configure(hqd);
}

void qmfcap_FreeMem(HANDLE_FDK_QMF_DOMAIN hqd) {
  rep_simple(EV_QMF_FREE_MEM);
  FDK_QmfDomain_FreeMem(hqd);
}

QMF_DOMAIN_ERROR qmfcap_ClearPersistentMemory(HANDLE_FDK_QMF_DOMAIN hqd) {
  rep_simple(EV_QMF_CLEAR_PERSISTENT);
  return FDK_QmfDomain_ClearPersistentMemory(hqd);
}

void qmfcap_Close(HANDLE_FDK_QMF_DOMAIN hqd) {
  rep_simple(EV_QMF_CLOSE);
  FDK_QmfDomain_Close(hqd);
}

/* ---------------------------------------------------------------------------------------- */
/* Injection: the API fuzz test (tests/sbrdec/api_fuzz.rs) drives the replay instance and the
   port directly, without the AAC decoder. The QMF domain configuration of the "decoder" is
   g.injectGc (set with cref_sbrdec_e2e_inject_gc). */

extern "C" {

/* Sets the stack pattern of the replay calls (0: none). */
void cref_sbrdec_e2e_set_scribble(INT v) { g_scribble = v; }

/* Resets the replay instance and starts injection mode (no original instance). */
void cref_sbrdec_e2e_inject_reset(void) {
  g.nEvents = 0;
  g.origMismatch = -1;
  g.origMismatchAt = 0;
  g.origQd = NULL;
  if (g.rep != NULL) sbrDecoder_Close(&g.rep);
  FDK_QmfDomain_Close(&g.repQd);
  FDKmemclear(&g.repQd, sizeof(g.repQd));
  FDKmemclear(&g.injectGc, sizeof(g.injectGc));
}

/* Sets the configuration fields of the "decoder" (the order of gc_capture). */
void cref_sbrdec_e2e_inject_gc(const INT *v) { gc_apply(&g.injectGc, v); }

/* The current configuration of the replay domain (the order of gc_capture). */
void cref_sbrdec_e2e_replay_gc(INT *v) { gc_capture(&g.repQd.globalConf, v); }

void cref_sbrdec_e2e_inject_open(void) { rep_open(); }

void cref_sbrdec_e2e_inject_simple(INT kind) { rep_simple(kind); }

void cref_sbrdec_e2e_inject_init_element(const INT *p) {
  rep_init_element(p[0], p[1], p[2], (AUDIO_OBJECT_TYPE)p[3], (MP4_ELEMENT_ID)p[4], p[5],
                   (UCHAR)p[6], (UCHAR)p[7], (UCHAR)p[8], (UCHAR)p[9], p[10]);
}

/* `buf` holds bufSize bytes (a power of two) with validBits valid bits. */
void cref_sbrdec_e2e_inject_header(const UCHAR *buf, INT bufSize, INT validBits, const INT *p) {
  UCHAR *b = (UCHAR *)malloc(bufSize);
  memcpy(b, buf, bufSize);
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, b, bufSize, validBits, BS_READER);
  rep_header(&bs, p[0], p[1], p[2], (AUDIO_OBJECT_TYPE)p[3], (MP4_ELEMENT_ID)p[4], p[5],
             (UCHAR)p[6], (UCHAR)p[7], (UCHAR)p[8], (UCHAR)p[9], p[10]);
  free(b);
}

void cref_sbrdec_e2e_inject_set_param(INT selfNull, INT param, INT value) {
  rep_set_param(selfNull, (SBRDEC_PARAM)param, value);
}

void cref_sbrdec_e2e_inject_drc_feed(INT ch, INT numBands, const FIXP_DBL *mag, INT exp, INT scheme, INT win, const USHORT *top) {
  rep_drc_feed(ch, (UINT)numBands, mag, exp, (SHORT)scheme, (UCHAR)win, top);
}

void cref_sbrdec_e2e_inject_drc_disable(INT ch) { rep_drc_disable(ch); }

/* p: count, bsPayLen, crcFlag, prevElement, elementIndex, acFlags, drm (0: no DRM buffer,
   otherwise the DRM buffer size); acElFlags has elementIndex + 1 values. */
void cref_sbrdec_e2e_inject_parse(const UCHAR *buf, INT bufSize, INT validBits, const INT *p, const UINT *acElFlags) {
  UCHAR *b = (UCHAR *)malloc(bufSize);
  memcpy(b, buf, bufSize);
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, b, bufSize, validBits, BS_READER);
  UCHAR *drm = p[6] > 0 ? (UCHAR *)calloc(p[6], 1) : NULL;
  rep_parse(&bs, drm, (USHORT)p[6], p[0], p[1], p[2], (MP4_ELEMENT_ID)p[3], p[4], (UINT)p[5],
            acElFlags);
  free(drm);
  free(b);
}

/* p: timeDataSize, numChannels, sampleRate, mapIdx, coreDecodedOk, psDecoded, inDataHeadroom,
   map fPassThrough (-1: NULL descriptor). */
void cref_sbrdec_e2e_inject_apply(const LONG *input, INT nIn, const LONG *timeData, const INT *p) {
  FDK_channelMapDescr map;
  FDK_chMapDescr_init(&map, NULL, 0, p[7] >= 0 ? (UINT)p[7] : 0);
  rep_apply(input, nIn, timeData, p[0], p[1], p[2], p[7] >= 0 ? &map : NULL, p[3], p[4],
            (UCHAR)p[5], p[6]);
}

}  // extern "C"

/* ---------------------------------------------------------------------------------------- */
/* Test interface.                                                                            */

static thread_local capdec::HANDLE_AACDECODER g_dec;

extern "C" {

/* Sets the callback (a `void (*)(void *ctx, INT kind)`) and whether the original instance is
   compared with the replay instance after every event. */
void cref_sbrdec_e2e_set_callback(void *cb, void *ctx, INT checkOrig) {
  g.cb = (CrefSbrCallback)cb;
  g.ctx = ctx;
  g.checkOrig = checkOrig;
}

/* Opens the captured AAC decoder (transport type `tt`). Returns 0 on success. */
INT cref_sbrdec_e2e_open(INT tt) {
  g.nEvents = 0;
  g.origMismatch = -1;
  g.origMismatchAt = 0;
  g.origQd = NULL;
  if (g.rep != NULL) sbrDecoder_Close(&g.rep);
  FDK_QmfDomain_Close(&g.repQd);
  FDKmemclear(&g.repQd, sizeof(g.repQd));
  g_dec = capdec::capdec_aacDecoder_Open((TRANSPORT_TYPE)tt, 1);
  return g_dec == NULL;
}

INT cref_sbrdec_e2e_set_param(INT param, INT value) {
  return capdec::capdec_aacDecoder_SetParam(g_dec, (capdec::AACDEC_PARAM)param, value);
}

INT cref_sbrdec_e2e_config_raw(const UCHAR *asc, INT len) {
  UCHAR *conf[1] = {(UCHAR *)asc};
  UINT l = (UINT)len;
  return capdec::capdec_aacDecoder_ConfigRaw(g_dec, conf, &l);
}

/* Feeds `len` bytes; returns the number of bytes consumed. */
INT cref_sbrdec_e2e_fill(const UCHAR *data, INT len) {
  UCHAR *p = (UCHAR *)data;
  UINT size = (UINT)len, valid = (UINT)len;
  capdec::capdec_aacDecoder_Fill(g_dec, &p, &size, &valid);
  return len - (INT)valid;
}

/* Decodes one frame into `out` (INT_PCM, `cap` values). Returns the error code; the stream
   info is stored in info[0..4]: frameSize, numChannels, sampleRate, aot. */
INT cref_sbrdec_e2e_decode(INT_PCM *out, INT cap, INT flags, INT *info) {
  INT e = capdec::capdec_aacDecoder_DecodeFrame(g_dec, out, cap, (UINT)flags);
  capdec::CStreamInfo *si = capdec::capdec_aacDecoder_GetStreamInfo(g_dec);
  info[0] = si->frameSize;
  info[1] = si->numChannels;
  info[2] = si->sampleRate;
  info[3] = si->aot;
  return e;
}

void cref_sbrdec_e2e_close(void) {
  if (g_dec != NULL) capdec::capdec_aacDecoder_Close(g_dec);
  g_dec = NULL;
}

INT cref_sbrdec_e2e_events(void) { return g.nEvents; }
INT cref_sbrdec_e2e_orig_mismatch(INT *at) {
  *at = g.origMismatchAt;
  return g.origMismatch;
}

/* Event data. */
INT cref_sbrdec_e2e_ev_int(INT k) { return (k >= 0 && k < 64) ? g.ev.i[k] : 0; }
INT cref_sbrdec_e2e_ev_len(INT k) { return (k >= 0 && k < A_NUM) ? g.ev.len[k] : 0; }
void cref_sbrdec_e2e_ev_copy(INT k, void *dst) {
  if (k >= 0 && k < A_NUM && g.ev.len[k] > 0) {
    memcpy(dst, g.ev.a[k], (size_t)g.ev.len[k] * g.ev.elsize[k]);
  }
}

/* Dumps the state of the replay instance; returns the number of values (copied with
   cref_sbrdec_e2e_dump_copy). */
INT cref_sbrdec_e2e_dump(void) {
  g_dump.n = 0;
  dump_sbr(&g_dump, g.rep, &g.repQd);
  return g_dump.n;
}
void cref_sbrdec_e2e_dump_copy(INT *out) { memcpy(out, g_dump.v, (size_t)g_dump.n * sizeof(INT)); }

/* The state dump of an HBE transposer (dump_hbe(); for tests/sbrdec/hbe.rs); then
   cref_sbrdec_e2e_dump_copy. */
INT cref_sbrdec_e2e_dump_hbe(void *h) {
  g_dump.n = 0;
  dump_hbe(&g_dump, (const struct hbeTransposer *)h);
  return g_dump.n;
}

/* The state dump of a PS decoder (dump_ps(); for tests/sbrdec/psdec.rs); then
   cref_sbrdec_e2e_dump_copy. */
INT cref_sbrdec_e2e_dump_ps(void *ps) {
  g_dump.n = 0;
  dump_ps(&g_dump, (const PS_DEC *)ps);
  return g_dump.n;
}

/* Layout checks: sizes of the structures that the test compares as bytes. */
INT cref_sbrdec_e2e_sizeof(INT k) {
  switch (k) {
    case 0: return sizeof(SBR_HEADER_DATA);
    case 1: return sizeof(SBR_FRAME_DATA);
    case 2: return sizeof(SBR_PREV_FRAME_DATA);
    case 3: return sizeof(SBR_CALCULATE_ENVELOPE);
    case 4: return sizeof(SBR_LPP_TRANS);
    case 5: return sizeof(PVC_STATIC_DATA);
    case 6: return sizeof(SBRDEC_DRC_CHANNEL);
    case 7: return sizeof(TRANSPOSER_SETTINGS);
    case 8: return sizeof(MPEG_PS_BS_DATA);
    default: return -1;
  }
}

}  // extern "C"
