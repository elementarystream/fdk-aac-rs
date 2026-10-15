// Shims for the differential tests of the block modules (block, channel, stereo, conceal, ...):
// a decoder-like context with all structures that these functions use, and a flat image of it
// (one INT per scalar field) to load the same state into the C++ and the Rust structures and to
// compare them afterwards. The Rust side is crates/cref/tests/aacdec/block_ctx.rs; the field
// order of both must be the same.
//
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_block_ctx (see build.rs). The shims of the other modules reach the context
// through the accessor functions u10ctx_* (C++ linkage, declared in each shim).

#include "cref_common.h"
#include "libAACdec/src/channelinfo.h"
#include "libAACdec/src/conceal.h"
#include "libAACdec/src/aac_rom.h"
#include "libAACdec/src/overlapadd.h"
#include "ac_arith_coder.h"
#include "FDK_tools_rom.h"
#include "FDK_bitstream.h"

#define CTX_SPEC_LEN (8 * 1024)
#define CTX_SCRATCH_LEN (4096)
#define CTX_OUT_LEN (2048)
#define CTX_ARCO_BYTES 518
#define CTX_BS_MAX 65536

struct U10Ctx {
  CAacDecoderChannelInfo ci[2];
  CAacDecoderStaticChannelInfo sci[2];
  CAacDecoderCommonData com;
  CAacDecoderCommonStaticData comStatic;
  CCplxPredictionData cplx;
  CWorkBufferCore1 wb1;
  CpePersistentData cpe;
  CConcealParams params;
  SamplingRateInfo sri;
  FIXP_DBL spec[CTX_SPEC_LEN];
  FIXP_DBL overlap[2][OverlapBufferSize];
  FIXP_DBL scratch[CTX_SCRATCH_LEN];
  FIXP_DBL out[2][CTX_OUT_LEN];
  FIXP_DBL facZirDummy[256];
  FIXP_DBL facScratchDummy[256];
  CArcoData *arco[2];
  int usac; /* which members of the unions are in the image */
  UCHAR bsbuf[CTX_BS_MAX];
  FDK_BITSTREAM bs;
  /* distinct window slope tables, in the order of FDKgetWindowSlope(len, shape) for the lengths
     of `lens` (cref_u10ctx_new), shape 0..2 */
  const FIXP_WTP *slopes[256];
  int nSlopes;
};

/* ---------------------------------------------------------------------------------------- */
/* image                                                                                     */
/* ---------------------------------------------------------------------------------------- */

struct Img {
  INT *v;
  INT n;
  int load;
};

#define F(x)                                       \
  do {                                             \
    if (im->load)                                  \
      (x) = (__typeof__(x))im->v[im->n];           \
    else                                           \
      im->v[im->n] = (INT)(x);                     \
    im->n++;                                       \
  } while (0)
#define FA(a, cnt)                          \
  do {                                      \
    for (int _i = 0; _i < (int)(cnt); _i++) \
      F((a)[_i]);                           \
  } while (0)
#define FA2(a, n1, n2)                  \
  do {                                  \
    for (int _j = 0; _j < (int)(n1); _j++) \
      FA((a)[_j], n2);                  \
  } while (0)

static void img_ics(Img *im, CIcsInfo *p) {
  FA(p->WindowGroupLength, 8);
  F(p->WindowGroups);
  F(p->Valid);
  F(p->WindowShape);
  F(p->WindowSequence);
  F(p->MaxSfBands);
  F(p->max_sfb_ste);
  F(p->ScaleFactorGrouping);
  F(p->TotalSfBands);
}

static void img_filter(Img *im, CFilter *f) {
  FA(f->Coeff, TNS_MAXIMUM_ORDER);
  F(f->StartBand);
  F(f->StopBand);
  F(f->Direction);
  F(f->Resolution);
  F(f->Order);
}

static void img_tns(Img *im, CTnsData *t) {
  for (int w = 0; w < TNS_MAX_WINDOWS; w++)
    for (int f = 0; f < TNS_MAXIMUM_FILTERS; f++) img_filter(im, &t->Filter[w][f]);
  FA(t->NumberOfFilters, TNS_MAX_WINDOWS);
  F(t->DataPresent);
  F(t->Active);
  F(t->GainLd);
}

static void img_pulse(Img *im, CPulseData *p) {
  F(p->PulseDataPresent);
  F(p->NumberPulse);
  F(p->PulseStartBand);
  FA(p->PulseOffset, N_MAX_LINES);
  FA(p->PulseAmp, N_MAX_LINES);
}

static void img_dyn(Img *im, CAacDecoderDynamicData *d, int usac) {
  FA(d->aScaleFactor, 8 * 16);
  FA(d->aSfbScale, 8 * 16);
  FA(d->aCodeBook, 8 * 16);
  FA(d->band_is_noise, 8 * 16);
  img_tns(im, &d->TnsData);
  F(d->RawDataInfo.CommonWindow);
  F(d->RawDataInfo.GlobalGain);
  if (!usac) {
    img_pulse(im, &d->specificTo.aac.PulseData);
    FA(d->specificTo.aac.aNumLineInSec4Hcr, MAX_SFB_HCR);
    FA(d->specificTo.aac.aCodeBooks4Hcr, MAX_SFB_HCR);
    F(d->specificTo.aac.lenOfReorderedSpectralData);
    F(d->specificTo.aac.lenOfLongestCodeword);
    F(d->specificTo.aac.numberSection);
    F(d->specificTo.aac.rvlcCurrentScaleFactorOK);
    F(d->specificTo.aac.rvlcIntensityUsed);
  } else {
    F(d->specificTo.usac.fd_noise_level_and_offset);
    F(d->specificTo.usac.tns_active);
    F(d->specificTo.usac.tns_on_lr);
    FA(d->specificTo.usac.tcx_noise_factor, 4);
    FA(d->specificTo.usac.tcx_global_gain, 4);
  }
}

static void img_pns(Img *im, CPnsData *p) {
  FA(p->pnsUsed, NO_OFBANDS);
  F(p->CurrentEnergy);
  F(p->PnsActive);
}

static void img_acelp_ch(Img *im, CAcelpChannelData *a) {
  F(a->acelp_core_mode);
  F(a->mean_energy);
  FA(a->T0, NB_SUBFR);
  FA(a->T0_frac, NB_SUBFR);
  FA(a->ltp_filtering_flag, NB_SUBFR);
  FA2(a->icb_index, NB_SUBFR, 8);
  FA(a->gains, NB_SUBFR);
}

/* fac_data[k]: 0 NULL, 1 fac_data0, 2 another pointer, 1000 + i pSpectralCoefficient + i */
static void img_fac_ptr(Img *im, U10Ctx *ctx, CAacDecoderChannelInfo *c, FIXP_DBL **pp) {
  INT code = 0;
  if (!im->load) {
    FIXP_DBL *p = *pp;
    if (p == NULL)
      code = 0;
    else if (p == c->data.usac.fac_data0)
      code = 1;
    else if (p >= c->pSpectralCoefficient && p < ctx->spec + CTX_SPEC_LEN)
      code = 1000 + (INT)(p - c->pSpectralCoefficient);
    else
      code = 2;
  }
  F(code);
  if (im->load) {
    if (code == 0)
      *pp = NULL;
    else if (code == 1)
      *pp = c->data.usac.fac_data0;
    else if (code >= 1000 && code < 1000 + 1024)
      *pp = c->pSpectralCoefficient + (code - 1000);
    else
      *pp = ctx->facScratchDummy;
  }
}

static void img_ci(Img *im, U10Ctx *ctx, CAacDecoderChannelInfo *c, int usac) {
  if (usac) {
    FA(c->data.usac.fac_data0, LFAC);
    FA(c->data.usac.fac_data_e, 4);
    for (int k = 0; k < 4; k++) img_fac_ptr(im, ctx, c, &c->data.usac.fac_data[k]);
    F(c->data.usac.core_mode);
    F(c->data.usac.core_mode_last);
    F(c->data.usac.lpd_mode_last);
    FA(c->data.usac.mod, 4);
    F(c->data.usac.bpf_control_info);
    FA2(c->data.usac.lsp_coeff, 5, M_LP_FILTER_ORDER);
    FA2(c->data.usac.lp_coeff, 5, M_LP_FILTER_ORDER);
    FA(c->data.usac.lp_coeff_exp, 5);
    FA(c->data.usac.lsf_adaptive_mean_cand, M_LP_FILTER_ORDER);
    FA(c->data.usac.aStability, 4);
    for (int k = 0; k < 4; k++) img_acelp_ch(im, &c->data.usac.acelp[k]);
    FA(c->data.usac.tcx_gain, 4);
    FA(c->data.usac.tcx_gain_e, 4);
  } else {
    img_pns(im, &c->data.aac.PnsData);
  }
  FA(c->specScale, 8);
  img_ics(im, &c->icsInfo);
  F(c->granuleLength);
  F(c->ElementInstanceTag);
  F(c->renderMode);
  F(c->currAliasingSymmetry);
  img_dyn(im, c->pDynData, usac);
}

static void img_slope(Img *im, U10Ctx *ctx, const FIXP_WTP **pp) {
  INT code = 0;
  if (!im->load) {
    code = -1;
    if (*pp == NULL) code = 0;
    for (int i = 0; i < ctx->nSlopes; i++)
      if (ctx->slopes[i] == *pp) code = i + 1;
  }
  F(code);
  if (im->load) *pp = (code > 0 && code <= ctx->nSlopes) ? ctx->slopes[code - 1] : NULL;
}

static void img_mdct(Img *im, U10Ctx *ctx, mdct_t *m, FIXP_DBL *ov) {
  FA(ov, OverlapBufferSize);
  img_slope(im, ctx, &m->prev_wrs);
  F(m->prev_tl);
  F(m->prev_nr);
  F(m->prev_fr);
  F(m->ov_offset);
  F(m->ov_size);
  F(m->prevAliasSymmetry);
  F(m->prevPrevAliasSymmetry);
  INT fz = (m->pFacZir != NULL), ao = (m->pAsymOvlp != NULL);
  F(fz);
  F(ao);
  if (im->load) {
    m->pFacZir = fz ? ctx->facZirDummy : NULL;
    m->pAsymOvlp = NULL;
  }
}

static void img_acelp_static(Img *im, CAcelpStaticMem *a) {
  FA(a->old_exc_mem, PIT_MAX_MAX + L_INTERPOL);
  FA(a->old_syn_mem, M_LP_FILTER_ORDER);
  FA(a->A, M_LP_FILTER_ORDER);
  F(a->A_exp);
  F(a->gc_threshold);
  F(a->de_emph_mem);
  F(a->past_gpit);
  F(a->past_gcode);
  F(a->old_T0);
  F(a->old_T0_frac);
  F(a->deemph_mem_wsyn);
  F(a->wsyn_rms);
  F(a->seed_ace);
}

static void img_conceal(Img *im, CConcealmentInfo *c) {
  FA(c->spectralCoefficient, 1024);
  FA(c->specScale, 8);
  F(c->iRandomPhase);
  FA(c->prevFrameOk, 2);
  F(c->cntValidFrames);
  F(c->cntFadeFrames);
  FA(c->winGrpOffset, 2);
  FA(c->attGrpOffset, 2);
  F(c->lastRenderMode);
  F(c->windowShape);
  F(c->windowSequence);
  F(c->lastWinGrpLen);
  F(c->concealState);
  F(c->concealState_old);
  F(c->fade_old);
  F(c->lastFadingType);
  FA(c->aRvlcPreviousScaleFactor, RVLC_MAX_SFB);
  FA(c->aRvlcPreviousCodebook, RVLC_MAX_SFB);
  F(c->rvlcPreviousScaleFactorOK);
  F(c->rvlcPreviousBlockType);
  FA(c->lsf4, M_LP_FILTER_ORDER);
  F(c->last_tcx_gain);
  F(c->last_tcx_gain_e);
  F(c->TDNoiseSeed);
  FA(c->TDNoiseStates, 3);
  FA(c->TDNoiseCoef, 3);
  F(c->TDNoiseAtt);
}

static void img_sci(Img *im, U10Ctx *ctx, int ch) {
  CAacDecoderStaticChannelInfo *s = &ctx->sci[ch];
  img_mdct(im, ctx, &s->IMdct, ctx->overlap[ch]);
  INT arco = (s->hArCo != NULL);
  F(arco);
  if (im->load) s->hArCo = arco ? ctx->arco[ch] : NULL;
  {
    UCHAR *b = (UCHAR *)ctx->arco[ch];
    FA(b, CTX_ARCO_BYTES);
  }
  F(s->pnsCurrentSeed);
  FA(s->old_synth, PIT_MAX_MAX - L_SUBFR);
  FA(s->old_T_pf, SYN_SFD);
  FA(s->old_gain_pf, SYN_SFD);
  FA(s->mem_bpf, L_FILT + L_SUBFR);
  F(s->old_bpf_control_info);
  F(s->last_core_mode);
  F(s->last_lpd_mode);
  F(s->last_last_lpd_mode);
  F(s->last_lpc_lost);
  FA(s->lpc4_lsf, M_LP_FILTER_ORDER);
  FA(s->lsf_adaptive_mean, M_LP_FILTER_ORDER);
  FA2(s->lp_coeff_old, 2, M_LP_FILTER_ORDER);
  FA(s->lp_coeff_old_exp, 2);
  F(s->oldStability);
  F(s->numLostLpdFrames);
  F(s->last_tcx_gain);
  F(s->last_tcx_gain_e);
  FA(s->last_alfd_gains, 32);
  F(s->last_tcx_pitch);
  F(s->last_tcx_noise_factor);
  img_acelp_static(im, &s->acelp);
  F(s->nfRandomSeed);
  img_conceal(im, &s->concealmentInfo);
  INT cpe = (s->pCpeStaticData != NULL);
  F(cpe);
  if (im->load) s->pCpeStaticData = cpe ? &ctx->cpe : NULL;
}

static void img_jsd(Img *im, CJointStereoData *j) {
  F(j->MsMaskPresent);
  FA(j->MsUsed, JointStereoMaximumBands);
  F(j->IGF_MsMaskPresent);
  F(j->cplx_pred_flag);
  F(j->igf_cplx_pred_flag);
  FA(j->store_dmx_re_prev, 1024);
  F(j->store_dmx_re_prev_e);
}

static void img_cplx(Img *im, CCplxPredictionData *c) {
  F(c->pred_dir);
  F(c->igf_pred_dir);
  F(c->complex_coef);
  F(c->use_prev_frame);
  FA2(c->alpha_q_re, JointStereoMaximumGroups, JointStereoMaximumBands);
  FA2(c->alpha_q_im, JointStereoMaximumGroups, JointStereoMaximumBands);
}

static void img_jsp(Img *im, CJointStereoPersistentData *j) {
  F(j->clearSpectralCoeffs);
  FA2(j->alpha_q_re_prev, JointStereoMaximumGroups, JointStereoMaximumBands);
  FA2(j->alpha_q_im_prev, JointStereoMaximumGroups, JointStereoMaximumBands);
  F(j->winSeqPrev);
  F(j->winShapePrev);
  F(j->winGroupsPrev);
}

static void img_params(Img *im, CConcealParams *p) {
  FA(p->fadeOutFactor, CONCEAL_MAX_NUM_FADE_FACTORS);
  FA(p->fadeInFactor, CONCEAL_MAX_NUM_FADE_FACTORS);
  F(p->method);
  F(p->numFadeOutFrames);
  F(p->numFadeInFrames);
  F(p->numMuteReleaseFrames);
  F(p->comfortNoiseLevel);
}

static void img_ctx(Img *im, U10Ctx *ctx) {
  for (int ch = 0; ch < 2; ch++) img_ci(im, ctx, &ctx->ci[ch], ctx->usac);
  for (int ch = 0; ch < 2; ch++) img_sci(im, ctx, ch);
  FA(ctx->com.pnsInterChannelData.correlated, NO_OFBANDS);
  FA(ctx->com.pnsRandomSeed, 8 * 16);
  img_jsd(im, &ctx->com.jointStereoData);
  INT cplx = (ctx->comStatic.cplxPredictionData != NULL);
  F(cplx);
  if (im->load) ctx->comStatic.cplxPredictionData = cplx ? &ctx->cplx : NULL;
  img_cplx(im, &ctx->cplx);
  img_jsp(im, &ctx->cpe.jointStereoPersistentData);
  FA(ctx->wb1.workBuffer, WB_SECTION_SIZE);
  FA(ctx->spec, CTX_SPEC_LEN);
  img_params(im, &ctx->params);
  FA(ctx->scratch, CTX_SCRATCH_LEN);
  FA2(ctx->out, 2, CTX_OUT_LEN);
}

/* ---------------------------------------------------------------------------------------- */
/* accessors for the other shims                                                             */
/* ---------------------------------------------------------------------------------------- */

CAacDecoderChannelInfo *u10ctx_ci(void *c, int ch) { return &((U10Ctx *)c)->ci[ch]; }
CAacDecoderStaticChannelInfo *u10ctx_sci(void *c, int ch) { return &((U10Ctx *)c)->sci[ch]; }
CAacDecoderCommonData *u10ctx_com(void *c) { return &((U10Ctx *)c)->com; }
SamplingRateInfo *u10ctx_sri(void *c) { return &((U10Ctx *)c)->sri; }
CConcealParams *u10ctx_params(void *c) { return &((U10Ctx *)c)->params; }
FIXP_DBL *u10ctx_scratch(void *c) { return ((U10Ctx *)c)->scratch; }
FIXP_DBL *u10ctx_out(void *c, int ch) { return ((U10Ctx *)c)->out[ch]; }
FIXP_DBL *u10ctx_spec(void *c) { return ((U10Ctx *)c)->spec; }
HANDLE_FDK_BITSTREAM u10ctx_bs(void *c) { return &((U10Ctx *)c)->bs; }

extern "C" {
void *cref_u10ctx_new(void) {
  U10Ctx *ctx = (U10Ctx *)FDKcalloc(1, sizeof(U10Ctx));
  ctx->comStatic.pWorkBufferCore1 = &ctx->wb1;
  ctx->comStatic.cplxPredictionData = &ctx->cplx;
  for (int ch = 0; ch < 2; ch++) {
    CAacDecoderChannelInfo *c = &ctx->ci[ch];
    c->pDynData = &ctx->com.pAacDecoderDynamicData[ch];
    c->pComData = &ctx->com;
    c->pComStaticData = &ctx->comStatic;
    c->pSpectralCoefficient = ctx->spec + ch * 1024;
    CAacDecoderStaticChannelInfo *s = &ctx->sci[ch];
    s->pOverlapBuffer = ctx->overlap[ch];
    s->IMdct.overlap.freq = ctx->overlap[ch];
    ctx->arco[ch] = CArco_Create();
    s->concealmentInfo.pConcealParams = &ctx->params;
  }
  ctx->cpe.jointStereoPersistentData.scratchBuffer = ctx->scratch;
  for (int ch = 0; ch < 2; ch++) {
    ctx->cpe.jointStereoPersistentData.spectralCoeffs[ch] =
        ctx->sci[ch].concealmentInfo.spectralCoefficient;
    ctx->cpe.jointStereoPersistentData.specScale[ch] = ctx->sci[ch].concealmentInfo.specScale;
  }
  ctx->nSlopes = 0;
  static const int lens[] = {4,   8,   12,  15,  16,  24,  30,   32,   48,   60,   64,  96,
                             120, 128, 192, 240, 256, 384, 480,  512,  768,  960,  1024};
  for (unsigned l = 0; l < sizeof(lens) / sizeof(lens[0]); l++) {
    int len = lens[l];
    for (int shape = 0; shape < 3; shape++) {
      const FIXP_WTP *w = FDKgetWindowSlope(len, shape);
      if (w == NULL) continue;
      int found = 0;
      for (int i = 0; i < ctx->nSlopes; i++)
        if (ctx->slopes[i] == w) found = 1;
      if (!found && ctx->nSlopes < 256) ctx->slopes[ctx->nSlopes++] = w;
    }
  }
  return ctx;
}

void cref_u10ctx_free(void *c) {
  U10Ctx *ctx = (U10Ctx *)c;
  for (int ch = 0; ch < 2; ch++) CArco_Destroy(ctx->arco[ch]);
  FDKfree(ctx);
}

INT cref_u10ctx_num_slopes(void *c) { return ((U10Ctx *)c)->nSlopes; }

/* Loads (load = 1) or dumps (load = 0) the image; returns its length. usac selects the members of
   the unions. With v == NULL only the length is returned. */
INT cref_u10ctx_image(void *c, INT *v, INT load, INT usac) {
  U10Ctx *ctx = (U10Ctx *)c;
  ctx->usac = usac;
  static INT tmp[1 << 17];
  Img im;
  im.v = v ? v : tmp;
  im.n = 0;
  im.load = v ? load : 0;
  img_ctx(&im, ctx);
  return im.n;
}

/* getSamplingRateInfo(&ctx->sri, spf, sri, sr); returns its error code. */
INT cref_u10ctx_sri(void *c, UINT spf, UINT sri, UINT sr) {
  U10Ctx *ctx = (U10Ctx *)c;
  FDKmemclear(&ctx->sri, sizeof(ctx->sri));
  return (INT)getSamplingRateInfo(&ctx->sri, spf, sri, sr);
}

/* Initializes the bitstream of the context with nbytes bytes of data in a buffer of bufBytes (a
   power of two, at least nbytes); validBits as given. */
void cref_u10ctx_bs(void *c, const UCHAR *data, INT nbytes, INT bufBytes, INT validBits) {
  U10Ctx *ctx = (U10Ctx *)c;
  FDKmemclear(ctx->bsbuf, sizeof(ctx->bsbuf));
  FDKmemcpy(ctx->bsbuf, data, nbytes);
  FDKinitBitStream(&ctx->bs, ctx->bsbuf, (UINT)bufBytes, (UINT)validBits, BS_READER);
}

INT cref_u10ctx_bs_valid(void *c) { return (INT)FDKgetValidBits(&((U10Ctx *)c)->bs); }

/* sizeof/offsetof checks used by the Rust side. */
INT cref_u10ctx_ci_specscale_ics_gap(void) {
  return (INT)(offsetof(CAacDecoderChannelInfo, icsInfo) -
               (offsetof(CAacDecoderChannelInfo, specScale) + 8 * sizeof(SHORT)));
}
}
