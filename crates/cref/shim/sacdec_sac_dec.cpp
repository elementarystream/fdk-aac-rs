// Shims for the differential tests of fdk_aac_rs_sacdec::sac_dec (and of the modules that work
// on a spatial decoder: sac_bitdec, sac_calcM1andM2, sac_process, sac_smoothing,
// sac_reshapeBBEnv, sac_stp).
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacdec_sac_dec (see build.rs).
//
// A test instance (SacInst) holds a spatial decoder of the library (FDK_SpatialDecOpen), the
// configuration it points to (pConfigCurrent), a bitstream frame with its lossless states and a
// QMF domain (pQmfDomain). The complete states are dumped as INT arrays in a fixed order (the
// test builds the same arrays from the port's structures); the plain data members can be
// loaded from such an array, so that both sides start from the same random state.
//
// The library's functions are called directly; struct STP_DEC (private to sac_stp.cpp) is made
// visible by including sac_stp.cpp with its external functions renamed.

#include <stdlib.h>
#include "cref_common.h"
#include "FDK_bitstream.h"
#include "FDK_qmf_domain.h"
#include "FDK_tools_rom.h"
#include "sac_dec.h"
#include "sac_bitdec.h"
#include "sac_calcM1andM2.h"
#include "sac_process.h"
#include "sac_smoothing.h"
#include "sac_reshapeBBEnv.h"
#include "sac_stp.h"
#include "sac_rom.h"
#include "sac_tsd.h"
#include "FDK_matrixCalloc.h"

#define subbandTPCreate ref_inst_subbandTPCreate
#define subbandTPInit ref_inst_subbandTPInit
#define subbandTPApply ref_inst_subbandTPApply
#define subbandTPDestroy ref_inst_subbandTPDestroy
#include "libSACdec/src/sac_stp.cpp"
#undef subbandTPCreate
#undef subbandTPInit
#undef subbandTPApply
#undef subbandTPDestroy

/* The static functions of sac_bitdec.cpp (copy in a namespace; its headers first). */
#include "nlc_dec.h"
namespace ref_bitdec {
#include "libSACdec/src/sac_bitdec.cpp"
}

/* struct FDK_HYBRID_SETUP is private to FDK_hybrid.cpp; the same definition, to identify the
   setup of a hybrid filter bank. */
struct FDK_HYBRID_SETUP {
  UCHAR nrQmfBands;
  UCHAR nHybBands[3];
  UCHAR synHybScale[3];
  SCHAR kHybrid[3];
  UCHAR protoLen;
  UCHAR filterDelay;
  const INT *pReadIdxTable;
};
static INT setup_id(const FDK_HYBRID_SETUP *p) {
  return p == NULL ? -1 : p->nrQmfBands * 100 + p->nHybBands[0] * 10 + p->nHybBands[1];
}

struct SacInst {
  spatialDec *self;
  SPATIAL_SPECIFIC_CONFIG ssc;
  SPATIAL_BS_FRAME frame;
  BS_LL_STATE ll;
  FDK_QMF_DOMAIN qd;
};

/* --- writer / reader ------------------------------------------------------------------ */

struct W {
  INT *o;
  INT n;
  void put(INT v) { o[n++] = v; }
  template <typename T>
  void arr(const T *p, INT cnt) {
    for (INT i = 0; i < cnt; i++) o[n++] = (INT)p[i];
  }
};
struct R {
  const INT *in;
  INT n;
  INT get() { return in[n++]; }
  template <typename T>
  void arr(T *p, INT cnt) {
    for (INT i = 0; i < cnt; i++) p[i] = (T)in[n++];
  }
};

/* The kernel tables in the order of kernels_table_index() of the port. */
static const SCHAR *kernel_tables[14] = {
    kernels_4_to_71,  kernels_5_to_71,  kernels_7_to_71,  kernels_10_to_71, kernels_14_to_71,
    kernels_20_to_71, kernels_28_to_71, kernels_4_to_64,  kernels_5_to_64,  kernels_7_to_64,
    kernels_9_to_64,  kernels_12_to_64, kernels_15_to_64, kernels_23_to_64};
static INT kernel_id(const SCHAR *k) {
  if (k == NULL) return -1;
  for (int i = 0; i < 14; i++)
    if (kernel_tables[i] == k) return i;
  return -2;
}

/* The QMF tables in the order of the qmf tests (fdk/qmf.rs). */
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
static INT qmf_table_id(const void *p) {
  if (p == NULL) return 0;
  for (unsigned i = 0; i < sizeof(qmf_tables) / sizeof(qmf_tables[0]); i++)
    if (qmf_tables[i] == p) return (INT)i + 1;
  return -1;
}

/* --- QMF domain -------------------------------------------------------------------------- */

#define QD_CH ((8) + (1))

/* A pointer into the domain's memory: kind 0 NULL, 1 work buffer section, 2 overlap buffer of
   an input channel, -1 other; then index and offset. */
static void decode_ptr(const FDK_QMF_DOMAIN *qd, const FIXP_DBL *p, W &w) {
  if (p == NULL) {
    w.put(0), w.put(0), w.put(0);
    return;
  }
  const FIXP_DBL *best = NULL;
  INT kind = -1, idx = 0;
  for (int k = 0; k < QMF_MAX_WB_SECTIONS; k++) {
    const FIXP_DBL *b = qd->globalConf.pWorkBuffer[k];
    if (b != NULL && b <= p && (best == NULL || b > best)) best = b, kind = 1, idx = k;
  }
  for (int ch = 0; ch < QD_CH; ch++) {
    const FIXP_DBL *b = qd->QmfDomainIn[ch].pOverlapBuffer;
    if (b != NULL && b <= p && (best == NULL || b > best)) best = b, kind = 2, idx = ch;
  }
  if (best == NULL || (kind == 1 && p - best >= QMF_WB_SECTION_SIZE) ||
      (kind == 2 && p - best >= 2 * 12 * 64)) {
    w.put(-1), w.put(0), w.put(0);
    return;
  }
  w.put(kind), w.put(idx), w.put((INT)(p - best));
}

static void dump_fb(const QMF_FILTER_BANK *q, W &w) {
  w.put(qmf_table_id(q->p_filter));
  w.put(q->FilterStates != NULL);
  w.put(q->FilterSize);
  w.put(qmf_table_id(q->t_cos));
  w.put(qmf_table_id(q->t_sin));
  w.put(q->filterScale);
  w.put(q->no_channels);
  w.put(q->no_col);
  w.put(q->lsb);
  w.put(q->usb);
  w.put(q->synScalefactor);
  w.put(q->outScalefactor);
  w.put(q->outGain_m);
  w.put(q->outGain_e);
  w.put((INT)q->flags);
  w.put(q->p_stride);
}

/* The complete QMF domain (see the module comment of the test for the layout). */
static void dump_qd(const FDK_QMF_DOMAIN *qd, W &w) {
  const FDK_QMF_DOMAIN_GC *g = &qd->globalConf;
  w.put(g->qmfDomainExplicitConfig);
  w.put(g->nInputChannels);
  w.put(g->nInputChannels_requested);
  w.put(g->nOutputChannels);
  w.put(g->nOutputChannels_requested);
  w.put(g->parkChannel);
  w.put(g->parkChannel_requested);
  w.put((INT)g->flags);
  w.put((INT)g->flags_requested);
  w.put(g->nBandsAnalysis);
  w.put(g->nBandsAnalysis_requested);
  w.put(g->nBandsSynthesis);
  w.put(g->nBandsSynthesis_requested);
  w.put(g->nQmfTimeSlots);
  w.put(g->nQmfTimeSlots_requested);
  w.put(g->nQmfOvTimeSlots);
  w.put(g->nQmfOvTimeSlots_requested);
  w.put(g->nQmfProcBands);
  w.put(g->nQmfProcBands_requested);
  w.put(g->nQmfProcChannels);
  w.put(g->nQmfProcChannels_requested);
  for (int k = 0; k < QMF_MAX_WB_SECTIONS; k++) w.put(g->pWorkBuffer[k] != NULL);
  INT nAna = 10 * g->nBandsAnalysis;
  INT nOv = g->nQmfOvTimeSlots * g->nQmfProcBands * CMPLX_MOD;
  INT nSlots = g->nQmfTimeSlots + g->nQmfOvTimeSlots;
  for (int ch = 0; ch < QD_CH; ch++) {
    const FDK_QMF_DOMAIN_IN *in = &qd->QmfDomainIn[ch];
    dump_fb(&in->fb, w);
    w.put(in->scaling.lb_scale);
    w.put(in->scaling.ov_lb_scale);
    w.put(in->scaling.hb_scale);
    w.put(in->scaling.ov_hb_scale);
    w.put(in->workBuf_nTimeSlots);
    w.put(in->workBuf_nBands);
    w.put(in->workBufferOffset);
    w.put(in->workBufferSectSize);
    w.put(in->pWorkBuffer == NULL ? 0 : (in->pWorkBuffer == g->pWorkBuffer ? 1 : -1));
    w.put(in->pGlobalConf == NULL ? 0 : (in->pGlobalConf == g ? 1 : -1));
    w.put(in->pAnaQmfStates != NULL);
    if (in->pAnaQmfStates != NULL) w.arr(in->pAnaQmfStates, nAna);
    w.put(in->pOverlapBuffer != NULL);
    if (in->pOverlapBuffer != NULL) w.arr(in->pOverlapBuffer, nOv);
    w.put(in->hQmfSlotsReal != NULL && in->hQmfSlotsImag != NULL);
    if (in->hQmfSlotsReal != NULL && in->hQmfSlotsImag != NULL) {
      for (int ts = 0; ts < nSlots; ts++) {
        decode_ptr(qd, in->hQmfSlotsReal[ts], w);
        decode_ptr(qd, in->hQmfSlotsImag[ts], w);
      }
    }
  }
  for (int ch = 0; ch < QD_CH; ch++) {
    const FDK_QMF_DOMAIN_OUT *o = &qd->QmfDomainOut[ch];
    dump_fb(&o->fb, w);
    w.put(o->pSynQmfStates != NULL);
    if (o->pSynQmfStates != NULL) w.arr(o->pSynQmfStates, 9 * QMF_DOMAIN_MAX_SYNTHESIS_QMF_BANDS);
  }
  for (int k = 0; k < QMF_MAX_WB_SECTIONS; k++) {
    if (g->pWorkBuffer[k] != NULL) w.arr(g->pWorkBuffer[k], QMF_WB_SECTION_SIZE);
  }
}

/* Loads the requested configuration (the 21 fields of the gc in the order of the dump). */
static void load_gc(FDK_QMF_DOMAIN_GC *g, const INT *v) {
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

/* Loads the memory contents (the parts of the dump after the structure: for each input
   channel the analysis states and the overlap buffer, for each output channel the synthesis
   states, then the work buffers), the scalings and the lsb/usb of the filter banks, from a
   dump in the layout of dump_qd() of the same configuration. */
static void load_qd_data(FDK_QMF_DOMAIN *qd, const INT *v) {
  R r = {v, 26};
  FDK_QMF_DOMAIN_GC *g = &qd->globalConf;
  INT nAna = 10 * g->nBandsAnalysis;
  INT nOv = g->nQmfOvTimeSlots * g->nQmfProcBands * CMPLX_MOD;
  INT nSlots = g->nQmfTimeSlots + g->nQmfOvTimeSlots;
  for (int ch = 0; ch < QD_CH; ch++) {
    FDK_QMF_DOMAIN_IN *in = &qd->QmfDomainIn[ch];
    r.n += 8;
    in->fb.lsb = r.get();
    in->fb.usb = r.get();
    r.n += 6;
    in->scaling.lb_scale = r.get();
    in->scaling.ov_lb_scale = r.get();
    in->scaling.hb_scale = r.get();
    in->scaling.ov_hb_scale = r.get();
    r.n += 6;
    if (r.get()) {
      if (in->pAnaQmfStates) r.arr(in->pAnaQmfStates, nAna); else r.n += nAna;
    }
    if (r.get()) {
      if (in->pOverlapBuffer) r.arr(in->pOverlapBuffer, nOv); else r.n += nOv;
    }
    if (r.get()) r.n += 6 * nSlots;
  }
  for (int ch = 0; ch < QD_CH; ch++) {
    FDK_QMF_DOMAIN_OUT *o = &qd->QmfDomainOut[ch];
    r.n += 16;
    if (r.get()) {
      if (o->pSynQmfStates) r.arr(o->pSynQmfStates, 9 * QMF_DOMAIN_MAX_SYNTHESIS_QMF_BANDS);
      else r.n += 9 * QMF_DOMAIN_MAX_SYNTHESIS_QMF_BANDS;
    }
  }
  for (int k = 0; k < QMF_MAX_WB_SECTIONS; k++) {
    if (g->pWorkBuffer[k] != NULL) r.arr(g->pWorkBuffer[k], QMF_WB_SECTION_SIZE);
  }
}

/* --- spatial decoder ------------------------------------------------------------------- */

#define MPAR MAX_PARAMETER_BANDS
#define MSET MAX_PARAMETER_SETS

/* The plain members (in the order of SpatialDec of the port). The arrays have the sizes of
   FDK_SpatialDecOpen() for DECODER_LEVEL_0. */
template <typename IO>
static void io_self(spatialDec *s, IO &io, bool configCurrent, int dir);

/* dir 0: dump (W), 1: load (R) */
#define F(x) io.v(s->x)
#define A(x, n) io.a(&(s->x)[0], n)

struct WIO {
  W w;
  template <typename T> void v(T &x) { w.put((INT)x); }
  template <typename T> void a(T *p, INT n) { w.arr(p, n); }
};
struct RIO {
  R r;
  template <typename T> void v(T &x) { x = (T)r.get(); }
  template <typename T> void a(T *p, INT n) { r.arr(p, n); }
};

template <typename IO>
static void io_self(spatialDec *s, IO &io) {
  F(errInt);
  F(staticDecScale);
  F(samplingFreq);
  F(decoderLevel);
  F(decoderMode);
  F(binauralMode);
  F(createParams.maxNumInputChannels);
  F(createParams.maxNumOutputChannels);
  F(createParams.maxNumQmfBands);
  F(createParams.maxNumHybridBands);
  F(createParams.maxNumXChannels);
  F(createParams.maxNumVChannels);
  F(createParams.maxNumDecorChannels);
  F(createParams.maxNumCmplxQmfBands);
  F(createParams.maxNumCmplxHybBands);
  F(createParams.maxNumResChannels);
  F(createParams.bProcResidual);
  F(createParams.maxNumResidualChannels);
  F(createParams.maxNumOttBoxes);
  F(createParams.maxNumParams);
  F(numComplexProcessingBands);
  F(treeConfig);
  F(numInputChannels);
  F(numOutputChannels);
  F(numOttBoxes);
  F(numM2rows);
  F(numOutputChannelsAT);
  F(quantMode);
  F(arbitraryDownmix);
  F(residualCoding);
  F(nrResidualFrame);
  F(nrArbDownmixResidualFrame);
  F(tempShapeConfig);
  F(decorrType);
  F(decorrConfig);
  F(envQuantMode);
  F(clipProtectGain__FDK);
  F(clipProtectGainSF__FDK);
  F(numDirektSignals);
  F(wStartResidualIdx);
  F(numDecorSignals);
  F(numVChannels);
  F(numXChannels);
  F(timeSlots);
  F(curTimeSlot);
  F(prevTimeSlot);
  F(curPs);
  F(frameLength);
  F(upmixType);
  F(partiallyComplex);
  F(useFDreverb);
  F(bShareDelayWithSBR);
  F(tp_hybBandBorder);
  F(qmfBands);
  F(hybridBands);
  F(TsdTs);
  A(param2hyb, MPAR + 1);
  A(kernels_width, MPAR);
  F(residualSamplingFreq);
  A(residualPresent, MAX_NUM_OTT + MAX_NUM_TTT);
  A(residualBands, MAX_NUM_OTT + MAX_NUM_TTT);
  A(residualQMFBands, MAX_NUM_OTT + MAX_NUM_TTT);
  F(arbdmxFramesPerSpatialFrame);
  F(arbdmxUpdQMF);
  F(numParameterBands);
  F(bitstreamParameterBands);
  A(numOttBands, 1);
  F(extendFrame);
  F(numParameterSetsPrev);
  A(smgTime, MSET);
  io.a(&s->smgData[0][0], MSET * MPAR);
  io.a(&s->ottCLDidxPrev[0][0], MPAR);
  io.a(&s->ottICCidxPrev[0][0], MPAR);
  io.a(&s->arbdmxGainIdxPrev[0][0], MPAR);
  io.a(&s->ottIPDidxPrev[0][0], MPAR);
  io.a(&s->outIdxData[0][0][0], MSET * MPAR);
  io.a(&s->cmpOttCLDidxPrev[0][0], MPAR);
  io.a(&s->cmpOttICCidxPrev[0][0], MPAR);
  io.a(&s->ottICCdiffidx[0][0][0], MSET * MPAR);
  io.a(&s->cmpOttIPDidxPrev[0][0], MPAR);
  io.a(&s->cmpArbdmxGainIdxPrev[0][0], MPAR);
  io.a(&s->ottCLD__FDK[0][0][0], MSET * MPAR);
  io.a(&s->ottICC__FDK[0][0][0], MSET * MPAR);
  io.a(&s->arbdmxGain__FDK[0][0][0], MSET * MPAR);
  A(arbdmxAlpha__FDK, 1);
  A(arbdmxAlphaPrev__FDK, 1);
  F(stereoConfigIndex);
  F(highRateMode);
  F(phaseCoding);
  io.a(&s->ottIPD__FDK[0][0][0], MSET * MPAR);
  A(PhaseLeft__FDK, MPAR);
  A(PhaseRight__FDK, MPAR);
  A(PhasePrevLeft__FDK, MPAR);
  A(PhasePrevRight__FDK, MPAR);
  F(numOttBandsIPD);
  io.a(&s->M2Real__FDK[0][0][0], 2 * 2 * MPAR);
  io.a(&s->M2Imag__FDK[0][0][0], 2 * 2 * MPAR);
  io.a(&s->M2RealPrev__FDK[0][0][0], 2 * 2 * MPAR);
  io.a(&s->M2ImagPrev__FDK[0][0][0], 2 * 2 * MPAR);
  F(pc_filterdelay);
  F(qmfInputDelayBufPos);
  io.a(&s->qmfInputReal__FDK[0][0], 64);
  io.a(&s->qmfInputImag__FDK[0][0], 64);
  io.a(&s->hybInputReal__FDK[0][0], 71);
  io.a(&s->hybInputImag__FDK[0][0], 71);
  F(binGain);
  F(reverbGain);
  F(binCenterGain);
  F(reverbCenterGain);
  io.a(&s->qmfResidualReal__FDK[0][0][0], 64);
  io.a(&s->qmfResidualImag__FDK[0][0][0], 64);
  io.a(&s->hybResidualReal__FDK[0][0], 71);
  io.a(&s->hybResidualImag__FDK[0][0], 71);
  io.a(&s->wReal__FDK[0][0], 2 * 71);
  io.a(&s->wImag__FDK[0][0], 2 * 71);
  io.a(&s->hybOutputRealDry__FDK[0][0], 2 * 71);
  io.a(&s->hybOutputImagDry__FDK[0][0], 2 * 71);
  io.a(&s->hybOutputRealWet__FDK[0][0], 2 * 71);
  io.a(&s->hybOutputImagWet__FDK[0][0], 2 * 71);
  F(smoothState->prevParamSlot);
  F(smoothState->prevSmgTime);
  A(smoothState->prevSmgData, MPAR);
  A(smoothState->opdLeftState__FDK, MPAR);
  A(smoothState->opdRightState__FDK, MPAR);
  F(reshapeBBEnvState->alpha__FDK);
  F(reshapeBBEnvState->beta__FDK);
  io.a(&s->reshapeBBEnvState->partNrgPrev__FDK[0][0], 5 * BB_ENV_SIZE);
  A(reshapeBBEnvState->normNrgPrev__FDK, 5);
  A(reshapeBBEnvState->frameNrgPrev__FDK, 5);
  A(reshapeBBEnvState->partNrgPrevSF, 5);
  A(reshapeBBEnvState->partNrgPrev2SF, 5);
  A(reshapeBBEnvState->normNrgPrevSF, 5);
  A(reshapeBBEnvState->frameNrgPrevSF, 5);
  A(row2channelDmxGES, 2);
  A(hStpDec->runDryEner, 1);
  A(hStpDec->runWetEner, 2);
  A(hStpDec->oldDryEnerLD64, 1);
  A(hStpDec->oldWetEnerLD64, 2);
  A(hStpDec->prev_tp_scale, 2);
  F(hStpDec->update_old_ener);
  F(bOverwriteM1M2prev);
  {
    INT m = s->concealInfo.concealParams.method;
    io.v(m);
    s->concealInfo.concealParams.method = (SpatialDecConcealmentMethod)m;
    INT st = s->concealInfo.concealState;
    F(concealInfo.concealParams.numKeepFrames);
    F(concealInfo.concealParams.numFadeOutFrames);
    F(concealInfo.concealParams.numFadeInFrames);
    F(concealInfo.concealParams.numReleaseFrames);
    io.v(st);
    s->concealInfo.concealState = (SpatialDecConcealmentState)st;
    F(concealInfo.cntStateFrames);
    F(concealInfo.cntValidFrames);
  }
  F(sacInDataHeadroom);
}

static void dump_ana_hyb(const FDK_ANA_HYB_FILTER *h, W &w) {
  for (int k = 0; k < 3; k++)
    w.put(h->bufferLFReal[k] == NULL ? 0 : (INT)(h->bufferLFReal[k] - h->pLFmemory)); /* NULL as 0 */
  for (int k = 0; k < 3; k++)
    w.put(h->bufferLFImag[k] == NULL ? 0 : (INT)(h->bufferLFImag[k] - h->pLFmemory)); /* NULL as 0 */
  for (int k = 0; k < 13; k++)
    w.put(h->bufferHFReal[k] == NULL ? 0 : (INT)(h->bufferHFReal[k] - h->pHFmemory)); /* NULL as 0 */
  for (int k = 0; k < 13; k++)
    w.put(h->bufferHFImag[k] == NULL ? 0 : (INT)(h->bufferHFImag[k] - h->pHFmemory)); /* NULL as 0 */
  w.put(h->bufferLFpos);
  w.put(h->bufferHFpos);
  w.put(h->nrBands);
  w.put(h->cplxBands);
  w.put(h->hfMode);
  w.put((INT)h->LFmemorySize);
  w.put((INT)h->HFmemorySize);
  w.put(setup_id(h->pSetup));
  if (h->pLFmemory) w.arr(h->pLFmemory, h->LFmemorySize / sizeof(FIXP_DBL));
  if (h->pHFmemory) w.arr(h->pHFmemory, h->HFmemorySize / sizeof(FIXP_DBL));
}

#define DECORR_ABSENT (-100)
static void dump4(W &w, const void *p, INT kind) {
  for (int i = 0; i < 4; i++) {
    if (p == NULL) w.put(DECORR_ABSENT);
    else if (kind == 0) w.put(((const REVBAND_FILT_TYPE *)p)[i]);
    else if (kind == 1) w.put(((const UCHAR *)p)[i]);
    else w.put(((const SCHAR *)p)[i]);
  }
}
/* As cref_decorrelate_state() of the decorrelator tests, then the buffer. */
static void dump_decorr(const DECORR_DEC *d, const FIXP_DBL *buf, INT bufLen, W &w) {
  w.put(d->L_stateBufferCplx);
  w.put(d->stateBufferCplx == NULL ? -1 : (INT)(d->stateBufferCplx - buf));
  w.put(d->L_delayBufferCplx);
  w.put(d->delayBufferCplx == NULL ? -1 : (INT)(d->delayBufferCplx - buf));
  dump4(w, d->REV_filtType, 0);
  dump4(w, d->REV_bandOffset, 1);
  dump4(w, d->REV_delay, 1);
  dump4(w, d->REV_filterOrder, 2);
  for (int i = 0; i < 4; i++) w.put(d->reverbBandDelayBufferIndex[i]);
  for (int i = 0; i < 3; i++) w.put(d->stateBufferOffset[i]);
  w.put(d->numbins);
  w.put(d->partiallyComplex);
  const DUCKER_INSTANCE *k = &d->ducker;
  w.put(k->hybridBands);
  w.put(k->parameterBands);
  w.put(k->partiallyComplex);
  w.put(k->duckerType);
  w.put((k->qs_next == NULL || k->mapProcBands2HybBands == NULL)
            ? -1
            : (INT)(k->qs_next - k->mapProcBands2HybBands));
  for (int i = 0; i < 21; i++)
    w.put(k->mapProcBands2HybBands == NULL ? DECORR_ABSENT : k->mapProcBands2HybBands[i]);
  for (int i = 0; i < 65; i++)
    w.put(k->mapHybBands2ProcBands == NULL ? DECORR_ABSENT : k->mapHybBands2ProcBands[i]);
  for (int i = 0; i < 2 * 28; i++) w.put(k->SmoothDirRevNrg[i]);
  for (int i = 0; i < 28; i++) w.put(k->peakDecay[i]);
  for (int i = 0; i < 28; i++) w.put(k->peakDiff[i]);
  w.put(k->maxValDirectData);
  w.put(k->maxValReverbData);
  w.put(k->scaleDirectNrg);
  w.put(k->scaleReverbNrg);
  w.put(k->scaleSmoothDirRevNrg);
  w.put(k->headroomSmoothDirRevNrg);
  for (int i = 0; i < 71; i++) {
    const DECORR_FILTER_INSTANCE *f = &d->Filter[i];
    w.put(f->stateCplx == NULL ? -1 : (INT)(f->stateCplx - buf));
    w.put(f->DelayBufferCplx == NULL ? -1 : (INT)(f->DelayBufferCplx - buf));
    for (int j = 0; j < 3; j++) w.put(f->numeratorReal == NULL ? DECORR_ABSENT : f->numeratorReal[j]);
    w.put(f->coeffsPacked == NULL ? DECORR_ABSENT : f->coeffsPacked[0].v.re);
    w.put(f->coeffsPacked == NULL ? DECORR_ABSENT : f->coeffsPacked[0].v.im);
    w.put(f->coeffsPacked == NULL ? DECORR_ABSENT : f->coeffsPacked[3].v.re);
    w.put(f->coeffsPacked == NULL ? DECORR_ABSENT : f->coeffsPacked[3].v.im);
    w.put(f->denominatorReal != NULL);
  }
  if (buf != NULL) w.arr(buf, bufLen);
}

/* The complete state of a spatial decoder: the plain members, kernels, pConfigCurrent != NULL,
   pActivM2ParamBands != NULL, the STP table pointers, then the hybrid filter banks and the
   decorrelator. */
static INT dump_self(spatialDec *s, INT *out) {
  WIO io = {{out, 0}};
  io_self(s, io);
  W &w = io.w;
  w.put(kernel_id(s->kernels));
  w.put(s->pConfigCurrent != NULL);
  w.put(s->pActivM2ParamBands != NULL);
  w.put(s->hStpDec->BP == NULL ? 0 : (s->hStpDec->BP == BP__FDK ? 1 : -1));
  w.put(s->hStpDec->BP_GF == NULL ? 0 : (s->hStpDec->BP_GF == BP_GF__FDK ? 1 : -1));
  for (int i = 0; i < 2; i++) dump_ana_hyb(&s->hybridAnalysis[i], w);
  for (int i = 0; i < 2; i++) {
    w.put(s->hybridSynthesis[i].nrBands);
    w.put(s->hybridSynthesis[i].cplxBands);
    w.put(setup_id(s->hybridSynthesis[i].pSetup));
  }
  dump_decorr(&s->apDecor[0], s->pDecorBufferCplx[0], 2 * (825 + 373), w);
  return w.n;
}

/* --- frame ------------------------------------------------------------------------------ */

static void io_ll(LOSSLESSDATA *l, W *w, R *r) {
  if (w) {
    w->arr(l->bsXXXDataMode, MSET);
    w->arr(l->bsQuantCoarseXXX, MSET);
    w->arr(l->bsFreqResStrideXXX, MSET);
    w->arr(l->nocmpQuantCoarseXXX, MSET);
  } else {
    r->arr(l->bsXXXDataMode, MSET);
    r->arr(l->bsQuantCoarseXXX, MSET);
    r->arr(l->bsFreqResStrideXXX, MSET);
    r->arr(l->nocmpQuantCoarseXXX, MSET);
  }
}

#define FIO(x)                    \
  if (w) w->put((INT)f->x);       \
  else f->x = (decltype(f->x))r->get();
#define FARR(p, n)                \
  if (w) w->arr(p, n);            \
  else r->arr(p, n);

static void io_frame(SPATIAL_BS_FRAME *f, BS_LL_STATE *ll, W *w, R *r) {
  FIO(bsIndependencyFlag);
  FIO(newBsData);
  FIO(numParameterSets);
  FARR(f->paramSlot, MSET);
  FARR(&f->cmpOttCLDidx[0][0][0], MAX_NUM_OTT * MSET * MPAR);
  FARR(&f->cmpOttICCidx[0][0][0], MAX_NUM_OTT * MSET * MPAR);
  FARR(f->bsSmoothMode, MSET);
  FARR(f->bsSmoothTime, MSET);
  FARR(f->bsFreqResStrideSmg, MSET);
  FARR(&f->bsSmgData[0][0], MSET * MPAR);
  FARR(&f->cmpArbdmxGainIdx[0][0][0], MAX_INPUT_CHANNELS * MSET * MPAR);
  for (int i = 0; i < MAX_NUM_PARAMETERS; i++) io_ll(&f->CLDLosslessData[i], w, r);
  for (int i = 0; i < MAX_NUM_PARAMETERS; i++) io_ll(&f->ICCLosslessData[i], w, r);
  for (int i = 0; i < MAX_NUM_PARAMETERS; i++) io_ll(&f->IPDLosslessData[i], w, r);
  FARR(&f->cmpOttIPDidx[0][0][0], MAX_NUM_OTT * MSET * MPAR);
  FIO(phaseMode);
  FIO(OpdSmoothingMode);
  FARR(f->tempShapeEnableChannelGES, MAX_OUTPUT_CHANNELS);
  FARR(&f->bsEnvShapeData[0][0], MAX_OUTPUT_CHANNELS * MAX_TIME_SLOTS);
  FARR(f->tempShapeEnableChannelSTP, MAX_OUTPUT_CHANNELS);
  FIO(TsdData[0].bsTsdEnable);
  FIO(TsdData[0].numSlots);
  FARR(f->TsdData[0].bsTsdTrPhaseData, MAX_TSD_TIME_SLOTS);
  for (int i = 0; i < MAX_NUM_PARAMETERS; i++) {
    LOSSLESSSTATE *s[3] = {&ll->CLDLosslessState[i], &ll->ICCLosslessState[i],
                           &ll->IPDLosslessState[i]};
    for (int k = 0; k < 3; k++) {
      if (w) {
        w->put(s[k]->bsQuantCoarseXXXprev);
        w->put(s[k]->bsQuantCoarseXXXprevParse);
      } else {
        s[k]->bsQuantCoarseXXXprev = (SCHAR)r->get();
        s[k]->bsQuantCoarseXXXprevParse = (SCHAR)r->get();
      }
    }
  }
  /* the state links of the lossless data */
  if (w) {
    INT ok = 1;
    for (int i = 0; i < MAX_NUM_PARAMETERS; i++) {
      ok &= f->CLDLosslessData[i].state == &ll->CLDLosslessState[i];
      ok &= f->ICCLosslessData[i].state == &ll->ICCLosslessState[i];
      ok &= f->IPDLosslessData[i].state == &ll->IPDLosslessState[i];
    }
    w->put(ok);
  }
}

extern "C" {

/* --- instance ----------------------------------------------------------------------------- */

void *cref_sacinst_new(INT decoderLevel, INT decoderMode, INT maxNumOutputChannels) {
  SacInst *x = (SacInst *)calloc(1, sizeof(SacInst));
  SPATIAL_DEC_CONFIG cfg;
  cfg.decoderLevel = (CFG_LEVEL)decoderLevel;
  cfg.decoderMode = (CFG_EXTENT)decoderMode;
  cfg.maxNumOutputChannels = (CFG_RESTRICTION)maxNumOutputChannels;
  cfg.binauralMode = BINAURAL_NONE;
  x->self = FDK_SpatialDecOpen(&cfg, 0);
  SpatialDecCreateBsFrame(&x->frame, &x->ll);
  if (x->self != NULL) {
    x->self->pQmfDomain = &x->qd;
    x->self->pConfigCurrent = &x->ssc;
  }
  return x;
}
INT cref_sacinst_is_open(void *h) { return ((SacInst *)h)->self != NULL; }
void cref_sacinst_free(void *h) {
  SacInst *x = (SacInst *)h;
  FDK_SpatialDecClose(x->self);
  SpatialDecCloseBsFrame(&x->frame);
  FDK_QmfDomain_Close(&x->qd);
  free(x);
}
INT cref_sacinst_dump(void *h, INT *out) { return dump_self(((SacInst *)h)->self, out); }
INT cref_sacinst_load(void *h, const INT *in) {
  RIO io = {{in, 0}};
  io_self(((SacInst *)h)->self, io);
  return io.r.n;
}
INT cref_sacinst_dump_frame(void *h, INT *out) {
  SacInst *x = (SacInst *)h;
  W w = {out, 0};
  io_frame(&x->frame, &x->ll, &w, NULL);
  return w.n;
}
INT cref_sacinst_load_frame(void *h, const INT *in) {
  SacInst *x = (SacInst *)h;
  R r = {in, 0};
  io_frame(&x->frame, &x->ll, NULL, &r);
  return r.n;
}
void cref_sacinst_get_ssc(void *h, UCHAR *out) {
  FDKmemcpy(out, &((SacInst *)h)->ssc, sizeof(SPATIAL_SPECIFIC_CONFIG));
}
void cref_sacinst_set_ssc(void *h, const UCHAR *in) {
  FDKmemcpy(&((SacInst *)h)->ssc, in, sizeof(SPATIAL_SPECIFIC_CONFIG));
}
INT cref_sacinst_dump_qd(void *h, INT *out) {
  W w = {out, 0};
  dump_qd(&((SacInst *)h)->qd, w);
  return w.n;
}
/* gc: 21 values (see load_gc); then FDK_QmfDomain_Configure(). */
INT cref_sacinst_configure_qd(void *h, const INT *gc) {
  SacInst *x = (SacInst *)h;
  load_gc(&x->qd.globalConf, gc);
  return FDK_QmfDomain_Configure(&x->qd);
}
void cref_sacinst_load_qd_data(void *h, const INT *dump) { load_qd_data(&((SacInst *)h)->qd, dump); }

/* userParams: outputMode, blindEnable, bypassMode, concealMethod, concealNumKeepFrames,
   concealFadeOutSlopeLength, concealFadeInSlopeLength, concealNumReleaseFrames. */
INT cref_sacinst_init(void *h, INT nQmfBands, INT upmixType, const INT *up, UINT initFlags) {
  SacInst *x = (SacInst *)h;
  SPATIALDEC_PARAM p;
  p.outputMode = (UCHAR)up[0];
  p.blindEnable = (UCHAR)up[1];
  p.bypassMode = (UCHAR)up[2];
  p.concealMethod = (UCHAR)up[3];
  p.concealNumKeepFrames = (UINT)up[4];
  p.concealFadeOutSlopeLength = (UINT)up[5];
  p.concealFadeInSlopeLength = (UINT)up[6];
  p.concealNumReleaseFrames = (UINT)up[7];
  return FDK_SpatialDecInit(x->self, &x->frame, &x->ssc, nQmfBands,
                            (SPATIAL_DEC_UPMIX_TYPE)upmixType, &p, initFlags);
}
void cref_sacinst_init_parser_context(void *h) { SpatialDecInitParserContext(((SacInst *)h)->self); }

/* --- sac_bitdec ------------------------------------------------------------------------- */

INT cref_sacinst_parse_frame(void *h, UCHAR *buf, INT bufBytes, INT validBits, INT upmixType, INT fGlobalIndependencyFlag, INT *bitsLeft) {
  SacInst *x = (SacInst *)h;
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufBytes, validBits, BS_READER);
  INT r = SpatialDecParseFrameData(x->self, &x->frame, &bs, &x->ssc, (UPMIXTYPE)upmixType,
                                   fGlobalIndependencyFlag);
  *bitsLeft = (INT)FDKgetValidBits(&bs);
  return r;
}
INT cref_sacinst_decode_frame(void *h) {
  SacInst *x = (SacInst *)h;
  return SpatialDecDecodeFrame(x->self, &x->frame);
}
INT cref_sacinst_decode_header(void *h) {
  SacInst *x = (SacInst *)h;
  return SpatialDecDecodeHeader(x->self, &x->ssc);
}

/* --- sac_calcM1andM2, sac_smoothing ---------------------------------------------------- */

INT cref_sacinst_calc_m1m2(void *h, INT ps) {
  SacInst *x = (SacInst *)h;
  return SpatialDecCalculateM1andM2(x->self, ps, &x->frame);
}
INT cref_sacinst_init_m1m2(void *h, INT initStatesFlag, INT configChanged) {
  return initM1andM2(((SacInst *)h)->self, initStatesFlag, configChanged);
}
INT cref_sacinst_get_residual_index(void *h, INT row) {
  return SpatialDecGetResidualIndex(((SacInst *)h)->self, row);
}
void cref_sacinst_init_smoothing(void *h) { initParameterSmoothing(((SacInst *)h)->self); }
void cref_sacinst_smooth_m1m2(void *h, INT ps) {
  SacInst *x = (SacInst *)h;
  SpatialDecSmoothM1andM2(x->self, &x->frame, ps);
}
void cref_sacinst_smooth_opd(void *h, INT ps) {
  SacInst *x = (SacInst *)h;
  SpatialDecSmoothOPD(x->self, &x->frame, ps);
}

/* --- sac_reshapeBBEnv, sac_stp ------------------------------------------------------- */

void cref_sacinst_init_bbenv(void *h, INT initStatesFlag) { initBBEnv(((SacInst *)h)->self, initStatesFlag); }
void cref_sacinst_reshape_bbenv(void *h, INT ts) {
  SacInst *x = (SacInst *)h;
  SpatialDecReshapeBBEnv(x->self, &x->frame, ts);
}
INT cref_sacinst_stp_init(void *h) { return subbandTPInit(((SacInst *)h)->self->hStpDec); }
INT cref_sacinst_stp_apply(void *h) {
  SacInst *x = (SacInst *)h;
  return subbandTPApply(x->self, &x->frame);
}

/* --- sac_process --------------------------------------------------------------------- */

INT cref_sacinst_qmf_analysis(void *h, const INT *inData, INT ts, INT bypassMode, INT numInputChannels) {
  spatialDec *s = ((SacInst *)h)->self;
  return SpatialDecQMFAnalysis(s, (const PCM_MPS *)inData, ts, bypassMode, s->qmfInputReal__FDK,
                               s->qmfInputImag__FDK, numInputChannels);
}
INT cref_sacinst_feed_qmf(void *h, INT ts, INT bypassMode, INT numInputChannels) {
  spatialDec *s = ((SacInst *)h)->self;
  return SpatialDecFeedQMF(s, NULL, NULL, ts, bypassMode, s->qmfInputReal__FDK,
                           s->qmfInputImag__FDK, numInputChannels);
}
INT cref_sacinst_hybrid_analysis(void *h, INT ts, INT numInputChannels) {
  spatialDec *s = ((SacInst *)h)->self;
  return SpatialDecHybridAnalysis(s, s->qmfInputReal__FDK, s->qmfInputImag__FDK,
                                  s->hybInputReal__FDK, s->hybInputImag__FDK, ts,
                                  numInputChannels);
}
INT cref_sacinst_apply_m1_create_w(void *h) {
  SacInst *x = (SacInst *)h;
  spatialDec *s = x->self;
  FIXP_DBL *pxReal[MAX_NUM_XCHANNELS] = {NULL};
  FIXP_DBL *pxImag[MAX_NUM_XCHANNELS] = {NULL};
  SpatialDecCreateX(s, s->hybInputReal__FDK, s->hybInputImag__FDK, pxReal, pxImag);
  return SpatialDecApplyM1_CreateW_Mode212(s, &x->frame, pxReal, pxImag, s->wReal__FDK,
                                           s->wImag__FDK);
}
/* which: 0 SpatialDecApplyM2, 1 _Mode212, 2 _Mode212_ResidualsPlusPhaseCoding */
INT cref_sacinst_apply_m2(void *h, INT which, INT ps, SHORT alpha) {
  spatialDec *s = ((SacInst *)h)->self;
  switch (which) {
    case 0:
      return SpatialDecApplyM2(s, ps, alpha, s->wReal__FDK, s->wImag__FDK,
                               s->hybOutputRealDry__FDK, s->hybOutputImagDry__FDK,
                               s->hybOutputRealWet__FDK, s->hybOutputImagWet__FDK);
    case 1:
      return SpatialDecApplyM2_Mode212(s, ps, alpha, s->wReal__FDK, s->wImag__FDK,
                                       s->hybOutputRealDry__FDK, s->hybOutputImagDry__FDK);
    default:
      return SpatialDecApplyM2_Mode212_ResidualsPlusPhaseCoding(
          s, ps, alpha, s->wReal__FDK, s->wImag__FDK, s->hybOutputRealDry__FDK,
          s->hybOutputImagDry__FDK);
  }
}
void cref_sacinst_buffer_matrices(void *h) { SpatialDecBufferMatrices(((SacInst *)h)->self); }
void cref_sacinst_apply_phase(void *h, SHORT alpha, INT last) {
  SpatialDecApplyPhase(((SacInst *)h)->self, alpha, last);
}
/* The default channel map descriptor (FDK_chMapDescr_init with no table) with fPassThrough. */
INT cref_sacinst_synthesis(void *h, INT ts, INT *timeOut, INT numInputChannels, INT passThrough) {
  spatialDec *s = ((SacInst *)h)->self;
  FDK_channelMapDescr d;
  FDK_chMapDescr_init(&d, NULL, 0, passThrough);
  return SpatialDecSynthesis(s, ts, s->hybOutputRealDry__FDK, s->hybOutputImagDry__FDK,
                             (PCM_MPS *)timeOut, numInputChannels, &d);
}
FIXP_DBL cref_sacinst_get_ch_gain(void *h, UINT ch, INT *scale) {
  return getChGain(((SacInst *)h)->self, ch, scale);
}

/* --- sac_dec ------------------------------------------------------------------------- */

/* inData: NULL, or the input; inOffset >= 0: the input is timeOut + inOffset. */
INT cref_sacinst_apply_frame(void *h, INT inputMode, const INT *inData, INT inOffset, INT *timeOut, UINT nSamples, UINT *controlFlags, INT numInputChannels, INT passThrough) {
  SacInst *x = (SacInst *)h;
  FDK_channelMapDescr d;
  FDK_chMapDescr_init(&d, NULL, 0, passThrough);
  PCM_MPS *in = inOffset >= 0 ? (PCM_MPS *)timeOut + inOffset : (PCM_MPS *)inData;
  return SpatialDecApplyFrame(x->self, &x->frame, (SPATIALDEC_INPUT_MODE)inputMode, in, NULL,
                              NULL, (PCM_MPS *)timeOut, nSamples, controlFlags,
                              numInputChannels, &d);
}
/* channelType/channelIndices: 8 entries; nulls: bit 0 self, 1 type, 2 indices, 3 map. */
void cref_sacinst_channel_properties(void *h, INT *channelType, UCHAR *channelIndices, INT nulls) {
  SacInst *x = (SacInst *)h;
  FDK_channelMapDescr d;
  FDK_chMapDescr_init(&d, NULL, 0, 0);
  AUDIO_CHANNEL_TYPE t[8];
  for (int i = 0; i < 8; i++) t[i] = (AUDIO_CHANNEL_TYPE)channelType[i];
  SpatialDecChannelProperties((nulls & 1) ? NULL : x->self, (nulls & 2) ? NULL : t,
                              (nulls & 4) ? NULL : channelIndices, (nulls & 8) ? NULL : &d);
  for (int i = 0; i < 8; i++) channelType[i] = t[i];
}
INT cref_sacinst_compare_ssc(const UCHAR *a, const UCHAR *b) {
  return FDK_SpatialDecCompareSpatialSpecificConfigHeader((SPATIAL_SPECIFIC_CONFIG *)a,
                                                          (SPATIAL_SPECIFIC_CONFIG *)b);
}

/* --- sac_bitdec without an instance --------------------------------------------------- */

INT cref_bitdec_parse_ssc(UCHAR *buf, INT bufBytes, INT validBits, INT sacHeaderLen, INT coreCodec, UCHAR *ssc, INT *bitsLeft) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufBytes, validBits, BS_READER);
  INT r = SpatialDecParseSpecificConfig(&bs, (SPATIAL_SPECIFIC_CONFIG *)ssc, sacHeaderLen,
                                        (AUDIO_OBJECT_TYPE)coreCodec);
  *bitsLeft = (INT)FDKgetValidBits(&bs);
  return r;
}
INT cref_bitdec_parse_ssc_header(UCHAR *buf, INT bufBytes, INT validBits, INT coreCodec, INT upmixType, UCHAR *ssc, INT *bitsLeft) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufBytes, validBits, BS_READER);
  INT r = SpatialDecParseSpecificConfigHeader(&bs, (SPATIAL_SPECIFIC_CONFIG *)ssc,
                                              (AUDIO_OBJECT_TYPE)coreCodec,
                                              (SPATIAL_DEC_UPMIX_TYPE)upmixType);
  *bitsLeft = (INT)FDKgetValidBits(&bs);
  return r;
}
INT cref_bitdec_parse_mps212(UCHAR *buf, INT bufBytes, INT validBits, INT samplingRate, INT coreCodec, INT stereoConfigIndex, INT coreSbrFrameLengthIndex, UCHAR *ssc, INT *bitsLeft) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufBytes, validBits, BS_READER);
  INT r = SpatialDecParseMps212Config(&bs, (SPATIAL_SPECIFIC_CONFIG *)ssc, samplingRate,
                                      (AUDIO_OBJECT_TYPE)coreCodec, stereoConfigIndex,
                                      coreSbrFrameLengthIndex);
  *bitsLeft = (INT)FDKgetValidBits(&bs);
  return r;
}
INT cref_bitdec_default_ssc(UCHAR *ssc, INT coreCodec, INT samplingFreq, INT nTimeSlots, INT level, INT isBlind, INT coreChannels) {
  return FDK_SpatialDecInitDefaultSpatialSpecificConfig((SPATIAL_SPECIFIC_CONFIG *)ssc,
                                                        (AUDIO_OBJECT_TYPE)coreCodec,
                                                        coreChannels, samplingFreq, nTimeSlots,
                                                        level, isBlind);
}
/* The static helpers of sac_bitdec.cpp. */
INT cref_bitdec_get_stride_map(INT freqResStride, INT startBand, INT stopBand, INT *aStrides) {
  return ref_bitdec::getStrideMap(freqResStride, startBand, stopBand, aStrides);
}
void cref_bitdec_create_mapping(INT *aMap, INT startBand, INT stopBand, INT stride) {
  ref_bitdec::createMapping(aMap, startBand, stopBand, stride);
}
void cref_bitdec_map_frequency(const SCHAR *in, SCHAR *out, INT *map, INT dataBands) {
  ref_bitdec::mapFrequency(in, out, map, dataBands);
}
void cref_bitdec_coarse2fine(SCHAR *data, INT dataType, INT startBand, INT numBands) {
  ref_bitdec::coarse2fine(data, (DATA_TYPE)dataType, startBand, numBands);
}
void cref_bitdec_fine2coarse(SCHAR *data, INT dataType, INT startBand, INT numBands) {
  ref_bitdec::fine2coarse(data, (DATA_TYPE)dataType, startBand, numBands);
}
INT cref_bitdec_deq_idx(INT value, INT paramType) { return ref_bitdec::deqIdx(value, paramType); }
FIXP_DBL cref_bitdec_factor_funct(FIXP_DBL ottVsTotDb, INT quantMode) {
  return ref_bitdec::factorFunct(ottVsTotDb, quantMode);
}
/* out: idx, ottVsTotDb1, ottVsTotDb2 (in/out) */
void cref_bitdec_factor_cld(INT *io, FIXP_DBL ottVsTotDb, INT mode, INT quantMode) {
  SCHAR idx = (SCHAR)io[0];
  FIXP_DBL d1 = io[1], d2 = io[2];
  ref_bitdec::factorCLD(&idx, ottVsTotDb, &d1, &d2, (SCHAR)mode, quantMode);
  io[0] = idx;
  io[1] = d1;
  io[2] = d2;
}
INT cref_bitdec_n_bits_param_slot(INT i) { return ref_bitdec::nBitsParamSlot(i); }
}

/* --- the dumps for shim/sacdec_sac_dec_lib.cpp (C++ linkage) ------------------------------ */

INT sacshim_dump_self(spatialDec *s, INT *out) { return dump_self(s, out); }
INT sacshim_dump_frame(SPATIAL_BS_FRAME *f, BS_LL_STATE *ll, INT *out) {
  W w = {out, 0};
  io_frame(f, ll, &w, NULL);
  return w.n;
}
INT sacshim_dump_qd(const FDK_QMF_DOMAIN *qd, INT *out) {
  W w = {out, 0};
  dump_qd(qd, w);
  return w.n;
}
void sacshim_load_gc(FDK_QMF_DOMAIN_GC *g, const INT *v) { load_gc(g, v); }
