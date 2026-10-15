// Shims for the differential tests of fdk_aac_rs_sbrenc: state dumps.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_sbr_encoder_dump (see build.rs).
//
// The dump functions write every member of an SBR encoder structure, in declaration order, as
// INT values (pointers to tables as their contents or an identifier, pointers into owned
// memory as the memory they point to). tests/sbrenc/sbr_encoder_dump.rs writes the same
// sequence for the port's structures. They are C++ functions with external linkage, so that
// the other shims of libSBRenc can call them (they declare the prototypes they use).

#include "cref_common.h"
#include "libSBRenc/src/sbr.h"
#include "libSBRenc/src/sbrenc_ram.h"
#include "libSBRenc/src/ps_main.h"
#include "libSBRenc/src/ps_encode.h"
#include "libSBRenc/src/sbrenc_rom.h"

struct SbrDump {
  INT *buf;
  INT cap;
  INT n;
};

static inline void dput(SbrDump *d, INT v) {
  if (d->n < d->cap) d->buf[d->n] = v;
  d->n++;
}
static void dputs(SbrDump *d, const INT *v, INT n) {
  for (INT i = 0; i < n; i++) dput(d, v[i]);
}
/* ILP32 (x86_32): FIXP_DBL is `long`, a type distinct from INT of the same size. */
#if !defined(__LP64__)
static void dputs(SbrDump *d, const LONG *v, INT n) { dputs(d, (const INT *)v, n); }
#endif
static void dputu8(SbrDump *d, const UCHAR *v, INT n) {
  for (INT i = 0; i < n; i++) dput(d, v[i]);
}
static void dputs8(SbrDump *d, const SCHAR *v, INT n) {
  for (INT i = 0; i < n; i++) dput(d, v[i]);
}
/* A pointer to a table of INTs: 0, or 1 and the first n values. */
static void dputp(SbrDump *d, const INT *p, INT n) {
  if (p == NULL) {
    dput(d, 0);
  } else {
    dput(d, 1);
    dputs(d, p, n);
  }
}
/* ILP32 (x86_32): FIXP_DBL is `long`, a type distinct from INT of the same size. */
#if !defined(__LP64__)
static void dputp(SbrDump *d, const LONG *p, INT n) { dputp(d, (const INT *)p, n); }
#endif

void sbrdump_grid(SbrDump *d, const SBR_GRID *g) {
  dput(d, g->bufferFrameStart);
  dput(d, g->numberTimeSlots);
  dput(d, g->frameClass);
  dput(d, g->bs_num_env);
  dput(d, g->bs_abs_bord);
  dput(d, g->n);
  dput(d, g->p);
  dputs(d, g->bs_rel_bord, MAX_NUM_REL);
  dputs(d, g->v_f, MAX_ENVELOPES_FIXVAR_VARFIX);
  dput(d, g->bs_abs_bord_0);
  dput(d, g->bs_abs_bord_1);
  dput(d, g->bs_num_rel_0);
  dput(d, g->bs_num_rel_1);
  dputs(d, g->bs_rel_bord_0, MAX_NUM_REL);
  dputs(d, g->bs_rel_bord_1, MAX_NUM_REL);
  dputs(d, g->v_fLR, MAX_ENVELOPES_VARVAR);
}

void sbrdump_frame_info(SbrDump *d, const SBR_FRAME_INFO *f) {
  dput(d, f->nEnvelopes);
  dputs(d, f->borders, MAX_ENVELOPES + 1);
  for (INT i = 0; i < MAX_ENVELOPES; i++) dput(d, f->freqRes[i]);
  dput(d, f->shortEnv);
  dput(d, f->nNoiseEnvelopes);
  dputs(d, f->bordersNoise, MAX_NOISE_ENVELOPES + 1);
}

void sbrdump_env_frame(SbrDump *d, const SBR_ENVELOPE_FRAME *h) {
  dput(d, h->frameMiddleSlot);
  dput(d, h->staticFraming);
  dput(d, h->numEnvStatic);
  dput(d, h->freq_res_fixfix[0]);
  dput(d, h->freq_res_fixfix[1]);
  dput(d, h->fResTransIsLow);
  dputp(d, h->v_tuningSegm, 6);
  dputp(d, h->v_tuningFreq, 3);
  dput(d, h->dmin);
  dput(d, h->dmax);
  dput(d, h->allowSpread);
  dput(d, h->frameClassOld);
  dput(d, h->spreadFlag);
  dputs(d, h->v_bord, 2 * MAX_ENVELOPES_VARVAR + 1);
  dput(d, h->length_v_bord);
  dputs(d, h->v_freq, 2 * MAX_ENVELOPES_VARVAR + 1);
  dput(d, h->length_v_freq);
  dputs(d, h->v_bordFollow, MAX_ENVELOPES_VARVAR);
  dput(d, h->length_v_bordFollow);
  dput(d, h->i_tranFollow);
  dput(d, h->i_fillFollow);
  dputs(d, h->v_freqFollow, MAX_ENVELOPES_VARVAR);
  dput(d, h->length_v_freqFollow);
  sbrdump_grid(d, &h->SbrGrid);
  sbrdump_frame_info(d, &h->SbrFrameInfo);
}

void sbrdump_tran_det(SbrDump *d, const SBR_TRANSIENT_DETECTOR *h) {
  dputs(d, h->transients, 32 + (32 / 2));
  dputs(d, h->thresholds, 64);
  dput(d, h->tran_thr);
  dput(d, h->split_thr_m);
  dput(d, h->split_thr_e);
  dput(d, h->prevLowBandEnergy);
  dput(d, h->prevHighBandEnergy);
  dput(d, h->tran_fc);
  dput(d, h->no_cols);
  dput(d, h->no_rows);
  dput(d, h->mode);
  dput(d, h->frameShift);
  dput(d, h->tran_off);
}

void sbrdump_fast_tran_det(SbrDump *d, const FAST_TRAN_DETECTOR *h) {
  dputs(d, h->transientCandidates, 32 + TRAN_DET_LOOKAHEAD);
  dput(d, h->nTimeSlots);
  dput(d, h->lookahead);
  dput(d, h->startBand);
  dput(d, h->stopBand);
  dputs(d, h->dBf_m, 64);
  dputs(d, h->dBf_e, 64);
  dputs(d, h->energy_timeSlots, 32 + TRAN_DET_LOOKAHEAD);
  dputs(d, h->energy_timeSlots_scale, 32 + TRAN_DET_LOOKAHEAD);
  dputs(d, h->delta_energy, 32 + TRAN_DET_LOOKAHEAD);
  dputs(d, h->delta_energy_scale, 32 + TRAN_DET_LOOKAHEAD);
  dputs(d, h->lowpass_energy, 32 + TRAN_DET_LOOKAHEAD);
  dputs(d, h->lowpass_energy_scale, 32 + TRAN_DET_LOOKAHEAD);
}

void sbrdump_inv_filt(SbrDump *d, const SBR_INV_FILT_EST *h) {
  dput(d, h->numberOfStrongest);
  dputs(d, h->prevRegionSbr, MAX_NUM_NOISE_VALUES);
  dputs(d, h->prevRegionOrig, MAX_NUM_NOISE_VALUES);
  dputs(d, h->freqBandTableInvFilt, MAX_NUM_NOISE_VALUES);
  dput(d, h->noDetectorBands);
  dput(d, h->noDetectorBandsMax);
  const DETECTOR_PARAMETERS *p = h->detectorParams;
  if (p == NULL) {
    dput(d, 0);
  } else {
    dput(d, 1);
    dputs(d, p->quantStepsSbr, 4);
    dputs(d, p->quantStepsOrig, 4);
    dputs(d, p->nrgBorders, 4);
    dput(d, p->numRegionsSbr);
    dput(d, p->numRegionsOrig);
    dput(d, p->numRegionsNrg);
    for (INT i = 0; i < 5; i++)
      for (INT j = 0; j < 5; j++) dput(d, p->regionSpace[i][j]);
    for (INT i = 0; i < 5; i++)
      for (INT j = 0; j < 5; j++) dput(d, p->regionSpaceTransient[i][j]);
    dputs(d, p->EnergyCompFactor, 5);
  }
  for (INT i = 0; i < MAX_NUM_NOISE_VALUES; i++) dput(d, h->prevInvfMode[i]);
  for (INT i = 0; i < MAX_NUM_NOISE_VALUES; i++) {
    const DETECTOR_VALUES *v = &h->detectorValues[i];
    dputs(d, v->origQuotaMean, INVF_SMOOTHING_LENGTH + 1);
    dputs(d, v->sbrQuotaMean, INVF_SMOOTHING_LENGTH + 1);
    dputs(d, v->origQuotaMeanStrongest, INVF_SMOOTHING_LENGTH + 1);
    dputs(d, v->sbrQuotaMeanStrongest, INVF_SMOOTHING_LENGTH + 1);
    dput(d, v->origQuotaMeanFilt);
    dput(d, v->sbrQuotaMeanFilt);
    dput(d, v->origQuotaMeanStrongestFilt);
    dput(d, v->sbrQuotaMeanStrongestFilt);
    dput(d, v->origQuotaMax);
    dput(d, v->sbrQuotaMax);
    dput(d, v->avgNrg);
  }
  dput(d, h->nrgAvg);
  dputs(d, h->wmQmf, MAX_NUM_NOISE_VALUES);
}

void sbrdump_nf_est(SbrDump *d, const SBR_NOISE_FLOOR_ESTIMATE *h) {
  for (INT i = 0; i < NF_SMOOTHING_LENGTH; i++) dputs(d, h->prevNoiseLevels[i], MAX_NUM_NOISE_VALUES);
  dputs(d, h->noiseFloorOffset, MAX_NUM_NOISE_VALUES);
  dputp(d, h->smoothFilter, 4);
  dput(d, h->ana_max_level);
  dput(d, h->weightFac);
  dputs(d, h->freqBandTableQmf, MAX_NUM_NOISE_VALUES + 1);
  dput(d, h->noNoiseBands);
  dput(d, h->noiseBands);
  dput(d, h->timeSlots);
  dput(d, h->diffThres);
}

/* Only the entries of tonalityDiff, sfmOrig and sfmSbr with a defined value (the first nSfb of
   each row; see the port, mh_det.rs) are dumped. */
void sbrdump_mh_det(SbrDump *d, const SBR_MISSING_HARMONICS_DETECTOR *h) {
  dput(d, h->qmfNoChannels);
  dput(d, h->nSfb);
  dput(d, h->sampleFreq);
  dput(d, h->previousTransientFlag);
  dput(d, h->previousTransientFrame);
  dput(d, h->previousTransientPos);
  dput(d, h->noVecPerFrame);
  dput(d, h->transientPosOffset);
  dput(d, h->move);
  dput(d, h->totNoEst);
  dput(d, h->noEstPerFrame);
  dput(d, h->timeSlots);
  dputu8(d, h->guideScfb, MAX_FREQ_COEFFS);
  dputu8(d, h->prevEnvelopeCompensation, MAX_FREQ_COEFFS);
  dputu8(d, h->detectionVectors[0], MAX_NO_OF_ESTIMATES * MAX_FREQ_COEFFS);
  INT n = h->nSfb < 0 ? 0 : (h->nSfb > MAX_FREQ_COEFFS ? MAX_FREQ_COEFFS : h->nSfb);
  for (INT i = 0; i < MAX_NO_OF_ESTIMATES / 2; i++) dputs(d, h->tonalityDiff[i], n);
  for (INT i = 0; i < MAX_NO_OF_ESTIMATES / 2; i++) dputs(d, h->sfmOrig[i], n);
  for (INT i = 0; i < MAX_NO_OF_ESTIMATES / 2; i++) dputs(d, h->sfmSbr[i], n);
  const DETECTOR_PARAMETERS_MH *p = h->mhParams;
  if (p == NULL) {
    dput(d, 0);
  } else {
    dput(d, 1);
    dput(d, p->deltaTime);
    const THRES_HOLDS *t = &p->thresHolds;
    dput(d, t->thresHoldDiff);
    dput(d, t->thresHoldDiffGuide);
    dput(d, t->thresHoldTone);
    dput(d, t->invThresHoldTone);
    dput(d, t->thresHoldToneGuide);
    dput(d, t->sfmThresSbr);
    dput(d, t->sfmThresOrig);
    dput(d, t->decayGuideOrig);
    dput(d, t->decayGuideDiff);
    dput(d, t->derivThresMaxLD64);
    dput(d, t->derivThresBelowLD64);
    dput(d, t->derivThresAboveLD64);
    dput(d, p->maxComp);
  }
  dputs(d, h->guideVectors[0].guideVectorDiff, MAX_NO_OF_ESTIMATES * MAX_FREQ_COEFFS);
  dputs(d, h->guideVectors[0].guideVectorOrig, MAX_NO_OF_ESTIMATES * MAX_FREQ_COEFFS);
  dputu8(d, h->guideVectors[0].guideVectorDetected, MAX_NO_OF_ESTIMATES * MAX_FREQ_COEFFS);
}

void sbrdump_ton_corr(SbrDump *d, const SBR_TON_CORR_EST *h) {
  dput(d, h->switchInverseFilt);
  dput(d, h->noQmfChannels);
  dput(d, h->bufferLength);
  dput(d, h->stepSize);
  dput(d, h->numberOfEstimates);
  dput(d, (INT)h->numberOfEstimatesPerFrame);
  dputs(d, h->lpcLength, 2);
  dput(d, h->nextSample);
  dput(d, h->move);
  dput(d, h->frameStartIndex);
  dput(d, h->startIndexMatrix);
  dput(d, h->frameStartIndexInvfEst);
  dput(d, h->prevTransientFlag);
  dput(d, h->transientNextFrame);
  dput(d, h->transientPosOffset);
  dputs(d, h->signMatrix[0], MAX_NO_OF_ESTIMATES * 64);
  dputs(d, h->quotaMatrix[0], MAX_NO_OF_ESTIMATES * 64);
  dputs(d, h->nrgVector, MAX_NO_OF_ESTIMATES);
  dputs(d, h->nrgVectorFreq, 64);
  dputs8(d, h->indexVector, 64);
  for (INT i = 0; i < MAX_NUM_PATCHES; i++) {
    const PATCH_PARAM *p = &h->patchParam[i];
    dput(d, p->sourceStartBand);
    dput(d, p->sourceStopBand);
    dput(d, p->guardStartBand);
    dput(d, p->targetStartBand);
    dput(d, p->targetBandOffs);
    dput(d, p->numBandsInPatch);
  }
  dput(d, h->guard);
  dput(d, h->shiftStartSb);
  dput(d, h->noOfPatches);
  sbrdump_mh_det(d, &h->sbrMissingHarmonicsDetector);
  sbrdump_nf_est(d, &h->sbrNoiseFloorEstimate);
  sbrdump_inv_filt(d, &h->sbrInvFilt);
}

/* A Huffman table of sbrenc_rom: 0 for NULL, 1 + its index in this list, -1 if unknown. */
static INT tabid(const void *p) {
  static const void *const t[] = {
      v_Huff_envelopeLevelC10T, v_Huff_envelopeLevelL10T, v_Huff_envelopeLevelC10F,
      v_Huff_envelopeLevelL10F, bookSbrEnvBalanceC10T,    bookSbrEnvBalanceL10T,
      bookSbrEnvBalanceC10F,    bookSbrEnvBalanceL10F,    v_Huff_envelopeLevelC11T,
      v_Huff_envelopeLevelL11T, v_Huff_envelopeLevelC11F, v_Huff_envelopeLevelL11F,
      bookSbrEnvBalanceC11T,    bookSbrEnvBalanceL11T,    bookSbrEnvBalanceC11F,
      bookSbrEnvBalanceL11F,    v_Huff_NoiseLevelC11T,    v_Huff_NoiseLevelL11T,
      bookSbrNoiseBalanceC11T,  bookSbrNoiseBalanceL11T};
  if (p == NULL) return 0;
  for (INT i = 0; i < (INT)(sizeof(t) / sizeof(t[0])); i++)
    if (t[i] == p) return i + 1;
  return -1;
}

void sbrdump_code_env(SbrDump *d, const SBR_CODE_ENVELOPE *h) {
  dput(d, h->offset);
  dput(d, h->upDate);
  dputs(d, h->nSfb, 2);
  dputs8(d, h->sfb_nrg_prev, MAX_FREQ_COEFFS);
  dput(d, h->deltaTAcrossFrames);
  dput(d, h->dF_edge_1stEnv);
  dput(d, h->dF_edge_incr);
  dput(d, h->dF_edge_incr_fac);
  dput(d, h->codeBookScfLavTime);
  dput(d, h->codeBookScfLavFreq);
  dput(d, h->codeBookScfLavLevelTime);
  dput(d, h->codeBookScfLavLevelFreq);
  dput(d, h->codeBookScfLavBalanceTime);
  dput(d, h->codeBookScfLavBalanceFreq);
  dput(d, h->start_bits);
  dput(d, h->start_bits_balance);
  dput(d, tabid(h->hufftableTimeL));
  dput(d, tabid(h->hufftableFreqL));
  dput(d, tabid(h->hufftableLevelTimeL));
  dput(d, tabid(h->hufftableBalanceTimeL));
  dput(d, tabid(h->hufftableLevelFreqL));
  dput(d, tabid(h->hufftableBalanceFreqL));
}

/* hSbrBSGrid is dumped by the caller (it points into an envelope channel). */
void sbrdump_env_data(SbrDump *d, const SBR_ENV_DATA *e) {
  dput(d, e->sbr_xpos_ctrl);
  dput(d, e->freq_res_fixfix[0]);
  dput(d, e->freq_res_fixfix[1]);
  dput(d, e->fResTransIsLow);
  dput(d, e->sbr_invf_mode);
  for (INT i = 0; i < MAX_NUM_NOISE_VALUES; i++) dput(d, e->sbr_invf_mode_vec[i]);
  dput(d, e->sbr_xpos_mode);
  for (INT i = 0; i < MAX_ENVELOPES; i++) dputs(d, e->ienvelope[i], MAX_FREQ_COEFFS);
  dput(d, e->codeBookScfLavBalance);
  dput(d, e->codeBookScfLav);
  dput(d, tabid(e->hufftableTimeC));
  dput(d, tabid(e->hufftableFreqC));
  dput(d, tabid(e->hufftableTimeL));
  dput(d, tabid(e->hufftableFreqL));
  dput(d, tabid(e->hufftableLevelTimeC));
  dput(d, tabid(e->hufftableBalanceTimeC));
  dput(d, tabid(e->hufftableLevelFreqC));
  dput(d, tabid(e->hufftableBalanceFreqC));
  dput(d, tabid(e->hufftableLevelTimeL));
  dput(d, tabid(e->hufftableBalanceTimeL));
  dput(d, tabid(e->hufftableLevelFreqL));
  dput(d, tabid(e->hufftableBalanceFreqL));
  dput(d, tabid(e->hufftableNoiseTimeL));
  dput(d, tabid(e->hufftableNoiseTimeC));
  dput(d, tabid(e->hufftableNoiseFreqL));
  dput(d, tabid(e->hufftableNoiseFreqC));
  dput(d, tabid(e->hufftableNoiseLevelTimeL));
  dput(d, tabid(e->hufftableNoiseLevelTimeC));
  dput(d, tabid(e->hufftableNoiseBalanceTimeL));
  dput(d, tabid(e->hufftableNoiseBalanceTimeC));
  dput(d, tabid(e->hufftableNoiseLevelFreqL));
  dput(d, tabid(e->hufftableNoiseLevelFreqC));
  dput(d, tabid(e->hufftableNoiseBalanceFreqL));
  dput(d, tabid(e->hufftableNoiseBalanceFreqC));
  dput(d, e->noHarmonics);
  dput(d, e->addHarmonicFlag);
  dputu8(d, e->addHarmonic, MAX_FREQ_COEFFS);
  dput(d, e->si_sbr_start_env_bits_balance);
  dput(d, e->si_sbr_start_env_bits);
  dput(d, e->si_sbr_start_noise_bits_balance);
  dput(d, e->si_sbr_start_noise_bits);
  dput(d, e->noOfEnvelopes);
  dputs(d, e->noScfBands, MAX_ENVELOPES);
  dputs(d, e->domain_vec, MAX_ENVELOPES);
  dputs(d, e->domain_vec_noise, MAX_ENVELOPES);
  dputs8(d, e->sbr_noise_levels, MAX_FREQ_COEFFS);
  dput(d, e->noOfnoisebands);
  dput(d, e->balance);
  dput(d, e->init_sbr_amp_res);
  dput(d, e->currentAmpResFF);
  dputs(d, e->ton_HF, SBR_GLOBAL_TONALITY_VALUES);
  dput(d, e->global_tonality);
  dput(d, e->extended_data);
  dput(d, e->extension_size);
  dput(d, e->extension_id);
  dputu8(d, e->extended_data_buffer, SBR_EXTENDED_DATA_MAX_CNT);
  dput(d, e->ldGrid);
}

void sbrdump_header_data(SbrDump *d, const SBR_HEADER_DATA *h) {
  dput(d, h->sbr_amp_res);
  dput(d, h->sbr_start_frequency);
  dput(d, h->sbr_stop_frequency);
  dput(d, h->sbr_xover_band);
  dput(d, h->sbr_noise_bands);
  dput(d, h->sbr_data_extra);
  dput(d, h->header_extra_1);
  dput(d, h->header_extra_2);
  dput(d, h->sbr_lc_stereo_mode);
  dput(d, h->sbr_limiter_bands);
  dput(d, h->sbr_limiter_gains);
  dput(d, h->sbr_interpol_freq);
  dput(d, h->sbr_smoothing_length);
  dput(d, h->alterScale);
  dput(d, h->freqScale);
  dput(d, h->coupling);
  dput(d, h->prev_coupling);
}

void sbrdump_bitstream_data(SbrDump *d, const SBR_BITSTREAM_DATA *h) {
  dput(d, h->TotalBits);
  dput(d, h->PayloadBits);
  dput(d, h->FillBits);
  dput(d, h->HeaderActive);
  dput(d, h->HeaderActiveDelay);
  dput(d, h->NrSendHeaderData);
  dput(d, h->CountSendHeaderData);
  dput(d, h->rightBorderFIX);
}

/* A bitstream: its state, the buffer as an offset from base (in bytes). */
void sbrdump_bitstream(SbrDump *d, const FDK_BITSTREAM *b, const UCHAR *base) {
  dput(d, (INT)b->CacheWord);
  dput(d, (INT)b->BitsInCache);
  dput(d, (INT)b->hBitBuf.ValidBits);
  dput(d, (INT)b->hBitBuf.ReadOffset);
  dput(d, (INT)b->hBitBuf.WriteOffset);
  dput(d, (INT)b->hBitBuf.BitNdx);
  dput(d, b->hBitBuf.Buffer == NULL ? -1 : (INT)(b->hBitBuf.Buffer - base));
  dput(d, (INT)b->hBitBuf.bufSize);
  dput(d, (INT)b->hBitBuf.bufBits);
  dput(d, (INT)b->ConfigCache);
}

void sbrdump_cmon_data(SbrDump *d, const COMMON_DATA *h, const UCHAR *base) {
  dput(d, h->sbrHdrBits);
  dput(d, h->sbrDataBits);
  dput(d, h->sbrFillBits);
  sbrdump_bitstream(d, &h->sbrBitbuf, base);
  sbrdump_bitstream(d, &h->tmpWriteBitbuf, base);
  dput(d, h->xOverFreq);
  dput(d, h->dynBwEnabled);
  dput(d, h->sbrNumChannels);
  dput(d, h->dynXOverFreqEnc);
}

void sbrdump_ps_data(SbrDump *d, const PS_DATA *p) {
  dput(d, p->iidEnable);
  dput(d, p->iidEnableLast);
  dput(d, p->iidQuantMode);
  dput(d, p->iidQuantModeLast);
  dputs(d, p->iidDiffMode, PS_MAX_ENVELOPES);
  for (INT i = 0; i < PS_MAX_ENVELOPES; i++) dputs(d, p->iidIdx[i], PS_MAX_BANDS);
  dputs(d, p->iidIdxLast, PS_MAX_BANDS);
  dput(d, p->iccEnable);
  dput(d, p->iccEnableLast);
  dput(d, p->iccQuantMode);
  dput(d, p->iccQuantModeLast);
  dputs(d, p->iccDiffMode, PS_MAX_ENVELOPES);
  for (INT i = 0; i < PS_MAX_ENVELOPES; i++) dputs(d, p->iccIdx[i], PS_MAX_BANDS);
  dputs(d, p->iccIdxLast, PS_MAX_BANDS);
  dput(d, p->nEnvelopesLast);
  dput(d, p->headerCnt);
  dput(d, p->iidTimeCnt);
  dput(d, p->iccTimeCnt);
  dput(d, p->noEnvCnt);
}

void sbrdump_ps_encode(SbrDump *d, const PS_ENCODE *h) {
  sbrdump_ps_data(d, &h->psData);
  dput(d, h->psEncMode);
  dput(d, h->nQmfIidGroups);
  dput(d, h->nSubQmfIidGroups);
  dputs(d, h->iidGroupBorders, QMF_GROUPS_HI_RES + SUBQMF_GROUPS_HI_RES + 1);
  dputs(d, h->subband2parameterIndex, QMF_GROUPS_HI_RES + SUBQMF_GROUPS_HI_RES);
  dputu8(d, h->iidGroupWidthLd, QMF_GROUPS_HI_RES + SUBQMF_GROUPS_HI_RES);
  dput(d, h->iidQuantErrorThreshold);
  dputu8(d, h->psBandNrgScale, PS_MAX_BANDS);
}

void sbrdump_ps_out(SbrDump *d, const PS_OUT *o) {
  dput(d, o->enablePSHeader);
  dput(d, o->enableIID);
  dput(d, o->iidMode);
  dput(d, o->enableICC);
  dput(d, o->iccMode);
  dput(d, o->enableIpdOpd);
  dput(d, o->frameClass);
  dput(d, o->nEnvelopes);
  dputs(d, o->frameBorder, PS_MAX_ENVELOPES);
  for (INT i = 0; i < PS_MAX_ENVELOPES; i++) dput(d, o->deltaIID[i]);
  for (INT i = 0; i < PS_MAX_ENVELOPES; i++) dputs(d, o->iid[i], PS_MAX_BANDS);
  dputs(d, o->iidLast, PS_MAX_BANDS);
  for (INT i = 0; i < PS_MAX_ENVELOPES; i++) dput(d, o->deltaICC[i]);
  for (INT i = 0; i < PS_MAX_ENVELOPES; i++) dputs(d, o->icc[i], PS_MAX_BANDS);
  dputs(d, o->iccLast, PS_MAX_BANDS);
  for (INT i = 0; i < PS_MAX_ENVELOPES; i++) dput(d, o->deltaIPD[i]);
  for (INT i = 0; i < PS_MAX_ENVELOPES; i++) dputs(d, o->ipd[i], PS_MAX_BANDS);
  dputs(d, o->ipdLast, PS_MAX_BANDS);
  for (INT i = 0; i < PS_MAX_ENVELOPES; i++) dput(d, o->deltaOPD[i]);
  for (INT i = 0; i < PS_MAX_ENVELOPES; i++) dputs(d, o->opd[i], PS_MAX_BANDS);
  dputs(d, o->opdLast, PS_MAX_BANDS);
}

/* A buffer pointer of a hybrid filter as an offset into its memory; the unused (NULL)
   pointers are 0, as the port's offsets of unused buffers. */
static INT hyb_off(const FIXP_DBL *p, const FIXP_DBL *mem) { return p == NULL ? 0 : (INT)(p - mem); }

static void sbrdump_ana_hyb(SbrDump *d, const FDK_ANA_HYB_FILTER *h) {
  for (INT i = 0; i < 3; i++) dput(d, hyb_off(h->bufferLFReal[i], h->pLFmemory));
  for (INT i = 0; i < 3; i++) dput(d, hyb_off(h->bufferLFImag[i], h->pLFmemory));
  for (INT i = 0; i < 13; i++) dput(d, hyb_off(h->bufferHFReal[i], h->pHFmemory));
  for (INT i = 0; i < 13; i++) dput(d, hyb_off(h->bufferHFImag[i], h->pHFmemory));
  dput(d, h->bufferLFpos);
  dput(d, h->bufferHFpos);
  dput(d, h->nrBands);
  dput(d, h->cplxBands);
  dput(d, h->hfMode);
  dputs(d, h->pLFmemory, (INT)(h->LFmemorySize / sizeof(FIXP_DBL)));
  dputs(d, h->pHFmemory, (INT)(h->HFmemorySize / sizeof(FIXP_DBL)));
  dput(d, (INT)h->LFmemorySize);
  dput(d, (INT)h->HFmemorySize);
  dput(d, h->pSetup != NULL);
}

/* The hybrid data in the dynamic RAM is part of the dump of the RAM. */
void sbrdump_parametric_stereo(SbrDump *d, const PARAMETRIC_STEREO *h) {
  dput(d, h->hPsEncode != NULL);
  if (h->hPsEncode != NULL) sbrdump_ps_encode(d, h->hPsEncode);
  sbrdump_ps_out(d, &h->psOut[0]);
  sbrdump_ps_out(d, &h->psOut[1]);
  dputs(d, &h->__staticHybridData[0][0][0][0], HYBRID_READ_OFFSET * MAX_PS_CHANNELS * 2 * MAX_HYBRID_BANDS);
  dputs(d, &h->qmfDelayLines[0][0][0], 2 * (32 >> 1) * 64);
  dput(d, h->qmfDelayScale);
  dput(d, h->psDelay);
  dput(d, (INT)h->maxEnvelopes);
  dputu8(d, h->dynBandScale, PS_MAX_BANDS);
  dputs(d, h->maxBandValue, PS_MAX_BANDS);
  dput(d, h->dmxScale);
  dput(d, h->initPS);
  dput(d, h->noQmfSlots);
  dput(d, h->noQmfBands);
  for (INT ch = 0; ch < MAX_PS_CHANNELS; ch++) sbrdump_ana_hyb(d, &h->fdkHybAnaFilter[ch]);
  dput(d, h->fdkHybSynFilter.nrBands);
  dput(d, h->fdkHybSynFilter.cplxBands);
  dput(d, h->fdkHybSynFilter.pSetup != NULL);
}

/* The SBR encoder: every element, channel, QMF bank, the dynamic RAM and the PS encoder.
   Pointers into owned memory are dumped as a kind and an offset: rows of YBuffer as 0/offset
   (p_YBuffer) or 1/offset (dynamic RAM, in FIXP_DBL), rBuffer/iBuffer as offsets into the
   dynamic RAM; channel and QMF bank pointers as their index in the encoder (-1 for NULL); table
   pointers as a presence flag and their first values. */
static INT ram_off(const SBR_ENCODER *e, const FIXP_DBL *p) {
  const FIXP_DBL *ram = (const FIXP_DBL *)e->dynamicRam;
  return (p == NULL) ? -1 : (INT)(p - ram);
}

static void sbrdump_qmf(SbrDump *d, const QMF_FILTER_BANK *q) {
  dput(d, q->p_filter != NULL);
  if (q->p_filter != NULL) {
    dput(d, q->p_filter[0]);
    dput(d, q->p_filter[1]);
  }
  dput(d, q->FilterStates != NULL);
  dput(d, q->FilterSize);
  dput(d, q->t_cos != NULL);
  if (q->t_cos != NULL) dput(d, q->t_cos[1]);
  dput(d, q->t_sin != NULL);
  if (q->t_sin != NULL) dput(d, q->t_sin[1]);
  dput(d, q->filterScale);
  dput(d, q->no_channels);
  dput(d, q->no_col);
  dput(d, q->lsb);
  dput(d, q->usb);
  dput(d, q->synScalefactor);
  dput(d, q->outScalefactor);
  dput(d, q->outGain_m);
  dput(d, q->outGain_e);
  dput(d, (INT)q->flags);
  dput(d, q->p_stride);
}

static void sbrdump_downsampler(SbrDump *d, const DOWNSAMPLER *ds) {
  for (INT i = 0; i < MAXNR_SECTIONS + 1; i++) dputs(d, ds->downFilter.states[i], 2);
  dput(d, ds->downFilter.coeffa != NULL);
  if (ds->downFilter.coeffa != NULL) dput(d, ds->downFilter.coeffa[0]);
  dput(d, ds->downFilter.gain);
  dput(d, ds->downFilter.Wc);
  dput(d, ds->downFilter.noCoeffs);
  dput(d, ds->downFilter.ptr);
  dput(d, ds->ratio);
  dput(d, ds->delay);
  dput(d, ds->pending);
}

static void sbrdump_extract(SbrDump *d, const SBR_ENCODER *e, const SBR_EXTRACT_ENVELOPE *x) {
  for (INT i = 0; i < 32; i++) dput(d, ram_off(e, x->rBuffer[i]));
  for (INT i = 0; i < 32; i++) dput(d, ram_off(e, x->iBuffer[i]));
  dput(d, x->p_YBuffer != NULL);
  if (x->p_YBuffer != NULL) dputs(d, x->p_YBuffer, 32 / 2 * 64);
  for (INT i = 0; i < 32; i++) {
    const FIXP_DBL *p = x->YBuffer[i];
    if (p == NULL) {
      dput(d, -1);
      dput(d, 0);
    } else if (x->p_YBuffer != NULL && p >= x->p_YBuffer && p < x->p_YBuffer + 32 / 2 * 64) {
      dput(d, 0);
      dput(d, (INT)(p - x->p_YBuffer));
    } else {
      dput(d, 1);
      dput(d, ram_off(e, p));
    }
  }
  dputs(d, x->YBufferScale, 2);
  dputu8(d, x->envelopeCompensation, MAX_FREQ_COEFFS);
  dputu8(d, x->pre_transient_info, 2);
  dput(d, x->YBufferWriteOffset);
  dput(d, x->YBufferSzShift);
  dput(d, x->rBufferReadOffset);
  dput(d, x->no_cols);
  dput(d, x->no_rows);
  dput(d, x->start_index);
  dput(d, x->time_slots);
  dput(d, x->time_step);
}

/* hSbrBSGrid: the index of the channel whose SbrEnvFrame.SbrGrid it points to. */
static INT grid_owner(const SBR_ENCODER *e, const SBR_GRID *g) {
  if (g == NULL) return -1;
  for (INT k = 0; k < 8; k++)
    if (e->pSbrChannel[k] != NULL && &e->pSbrChannel[k]->hEnvChannel.SbrEnvFrame.SbrGrid == g) return k;
  return -2;
}

static void sbrdump_env_channel(SbrDump *d, const SBR_ENCODER *e, const ENV_CHANNEL *h) {
  sbrdump_fast_tran_det(d, &h->sbrFastTransientDetector);
  sbrdump_tran_det(d, &h->sbrTransientDetector);
  sbrdump_code_env(d, &h->sbrCodeEnvelope);
  sbrdump_code_env(d, &h->sbrCodeNoiseFloor);
  sbrdump_extract(d, e, &h->sbrExtractEnvelope);
  sbrdump_env_frame(d, &h->SbrEnvFrame);
  sbrdump_ton_corr(d, &h->TonCorr);
  sbrdump_env_data(d, &h->encEnvData);
  dput(d, grid_owner(e, h->encEnvData.hSbrBSGrid));
  dput(d, h->qmfScale);
  dput(d, h->fLevelProtect);
}

static void sbrdump_config_data(SbrDump *d, const SBR_CONFIG_DATA *c) {
  dput(d, (INT)c->sbrSyntaxFlags);
  dput(d, c->nChannels);
  dputs(d, c->nSfb, 2);
  dput(d, c->num_Master);
  dput(d, c->sampleFreq);
  dput(d, c->frameSize);
  dput(d, c->xOverFreq);
  dput(d, c->dynXOverFreq);
  dput(d, c->noQmfBands);
  dput(d, c->noQmfSlots);
  dput(d, c->freqBandTable[LO] != NULL);
  if (c->freqBandTable[LO] != NULL) dputu8(d, c->freqBandTable[LO], MAX_FREQ_COEFFS / 2 + 1);
  dput(d, c->freqBandTable[HI] != NULL);
  if (c->freqBandTable[HI] != NULL) dputu8(d, c->freqBandTable[HI], MAX_FREQ_COEFFS + 1);
  dput(d, c->v_k_master != NULL);
  if (c->v_k_master != NULL) dputu8(d, c->v_k_master, MAX_FREQ_COEFFS + 1);
  dput(d, c->stereoMode);
  dput(d, c->noEnvChannels);
  dput(d, c->useWaveCoding);
  dput(d, c->useParametricCoding);
  dput(d, c->xposCtrlSwitch);
  dput(d, c->switchTransposers);
  dput(d, c->initAmpResFF);
  dput(d, c->thresholdAmpResFF_m);
  dput(d, c->thresholdAmpResFF_e);
}

void sbrdump_sbr_encoder(SbrDump *d, const SBR_ENCODER *e) {
  for (INT el = 0; el < 8; el++) {
    const SBR_ELEMENT *s = e->sbrElement[el];
    dput(d, s != NULL);
    if (s == NULL) continue;
    for (INT ch = 0; ch < 2; ch++) {
      INT idx = -1;
      for (INT k = 0; k < 8; k++)
        if (s->sbrChannel[ch] != NULL && e->pSbrChannel[k] == s->sbrChannel[ch]) idx = k;
      dput(d, idx);
    }
    for (INT ch = 0; ch < 2; ch++)
      dput(d, s->hQmfAnalysis[ch] == NULL ? -1 : (INT)(s->hQmfAnalysis[ch] - e->QmfAnalysis));
    sbrdump_config_data(d, &s->sbrConfigData);
    sbrdump_header_data(d, &s->sbrHeaderData);
    sbrdump_bitstream_data(d, &s->sbrBitstreamData);
    sbrdump_cmon_data(d, &s->CmonData, &s->payloadDelayLine[0][0]);
    dputs(d, s->dynXOverFreqDelay, 5);
    dput(d, s->elInfo.elType);
    dput(d, s->elInfo.bitRate);
    dput(d, s->elInfo.instanceTag);
    dput(d, s->elInfo.fParametricStereo);
    dput(d, s->elInfo.fDualMono);
    dput(d, s->elInfo.nChannelsInEl);
    dputu8(d, s->elInfo.ChannelIndex, 2);
    dputu8(d, &s->payloadDelayLine[0][0], (1 + MAX_DELAY_FRAMES) * MAX_PAYLOAD_SIZE);
    for (INT i = 0; i < 1 + MAX_DELAY_FRAMES; i++) dput(d, (INT)s->payloadDelayLineSize[i]);
  }
  for (INT ch = 0; ch < 8; ch++) {
    const SBR_CHANNEL *c = e->pSbrChannel[ch];
    dput(d, c != NULL);
    if (c == NULL) continue;
    sbrdump_env_channel(d, e, &c->hEnvChannel);
    sbrdump_downsampler(d, &c->downSampler);
  }
  for (INT q = 0; q < 8; q++) {
    sbrdump_qmf(d, &e->QmfAnalysis[q]);
    if (e->QmfAnalysis[q].FilterStates != NULL) {
      const FIXP_QAS *st = (const FIXP_QAS *)e->QmfAnalysis[q].FilterStates;
      for (INT i = 0; i < 640; i++) dput(d, st[i]);
    }
  }
  sbrdump_downsampler(d, &e->lfeDownSampler);
  dput(d, e->lfeChIdx);
  dput(d, e->noElements);
  dput(d, e->nChannels);
  dput(d, e->frameSize);
  dput(d, e->bufferOffset);
  dput(d, e->downsampledOffset);
  dput(d, e->downmixSize);
  dput(d, e->downSampleFactor);
  dput(d, e->downsamplingMethod);
  dput(d, e->nBitstrDelay);
  dput(d, e->sbrDecDelay);
  dput(d, e->estimateBitrate);
  dput(d, e->inputDataDelay);
  dput(d, e->dynamicRam != NULL);
  if (e->dynamicRam != NULL) dputs(d, (const INT *)e->dynamicRam, (INT)(SBR_ENC_DYN_RAM_SIZE / sizeof(FIXP_DBL)));
  dput(d, e->hParametricStereo != NULL);
  if (e->hParametricStereo != NULL) sbrdump_parametric_stereo(d, e->hParametricStereo);
  sbrdump_qmf(d, &e->qmfSynthesisPS);
  if (e->qmfSynthesisPS.FilterStates != NULL) dputs(d, (const INT *)e->qmfSynthesisPS.FilterStates, 640 / 2);
  dput(d, e->maxElements);
  dput(d, e->maxChannels);
  dput(d, e->supportPS);
}

extern "C" {
/* The dump functions are called through the shims of the modules; this file has no C entry
   points of its own apart from this marker (build.rs binds every cref_ function). */
INT cref_sbr_encoder_dump_version(void) { return 1; }
}
