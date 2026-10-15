// Shims for the differential tests of fdk_aac_rs_sbrdec::env_calc (tests/sbrdec/env_calc.rs).
//
// A copy of env_calc.cpp is compiled in a namespace so that its static functions can be called.
// Structures are passed as memory (the port's plain data structures have the layout of the
// reference); QMF data as flat arrays of slots of 64 values; the frequency band table pointers
// of a header are set to the header's own tables before a call and cleared after it.

#include "cref_common.h"
#include "libSBRdec/src/env_calc.h"
#include "libSBRdec/src/sbrdec_freq_sca.h"
#include "libSBRdec/src/env_extr.h"
#include "libSBRdec/src/transcendent.h"
#include "libSBRdec/src/sbr_ram.h"
#include "libSBRdec/src/sbr_rom.h"
#include "genericStds.h"
#include "fixpoint_math.h"

/* The x86 variant of schur_div() (inside fDivNorm()) has no defined result for a zero divisor,
   while the AArch64 variant computes a value. The copy of env_calc.cpp calls this wrapper,
   which records a division by zero (and returns 0); every other division is the reference's
   own. The shims report a recorded division as -1, and the tests do not compare those cases. */
static thread_local int g_div_by_zero;
static FIXP_DBL guarded_fDivNorm(FIXP_DBL num, FIXP_DBL denom, INT *result_e) {
  if (num != (FIXP_DBL)0 && denom == (FIXP_DBL)0) {
    g_div_by_zero = 1;
    *result_e = 0;
    return (FIXP_DBL)0;
  }
  return fDivNorm(num, denom, result_e);
}
#define fDivNorm guarded_fDivNorm


namespace tbl {
#include "libSBRdec/src/env_calc.cpp"
}
#undef fDivNorm
#define FPE_GUARD_BEGIN g_div_by_zero = 0;
#define FPE_GUARD_END \
  if (g_div_by_zero) return -1;

#define SLOTS 128



static void slots(FIXP_DBL *flat, INT n, FIXP_DBL **p) {
  for (INT i = 0; i < n; i++) p[i] = flat ? flat + i * 64 : NULL;
}

static void fix_header(SBR_HEADER_DATA *h, int set) {
  h->freqBandData.freqBandTable[0] = set ? h->freqBandData.freqBandTableLo : NULL;
  h->freqBandData.freqBandTable[1] = set ? h->freqBandData.freqBandTableHi : NULL;
}

extern "C" {

INT cref_sbrdec_env_calc_sizeof_nrgs(void) { return sizeof(tbl::ENV_CALC_NRGS); }

void cref_sbrdec_env_calc_nrg_per_subband(FIXP_DBL *re, FIXP_DBL *im, INT nSlots, INT lo, INT hi, INT start, INT next, INT frameExp, FIXP_DBL *nrgEst, SCHAR *nrgEst_e) {
  FIXP_DBL *r[SLOTS], *i[SLOTS];
  slots(re, nSlots, r);
  slots(im, nSlots, i);
  tbl::calcNrgPerSubband(r, im ? i : NULL, lo, hi, start, next, (SCHAR)frameExp, nrgEst, nrgEst_e);
}

void cref_sbrdec_env_calc_nrg_per_sfb(FIXP_DBL *re, FIXP_DBL *im, INT nSlots, INT nSfb, UCHAR *table, INT start, INT next, INT inputE, FIXP_DBL *nrgEst, SCHAR *nrgEst_e) {
  FIXP_DBL *r[SLOTS], *i[SLOTS];
  slots(re, nSlots, r);
  slots(im, nSlots, i);
  tbl::calcNrgPerSfb(r, im ? i : NULL, nSfb, table, start, next, (SCHAR)inputE, nrgEst, nrgEst_e);
}

/* returns -1 if the wrapper above recorded a zero divisor (no defined result), else 0 */
INT cref_sbrdec_env_calc_subband_gain(FIXP_DBL nrgRef, INT nrgRefE, void *nrgs, INT i, FIXP_DBL tmpNoise, INT tmpNoiseE, INT sinePresent, INT sineMapped, INT noNoise) {
  FPE_GUARD_BEGIN
  tbl::calcSubbandGain(nrgRef, (SCHAR)nrgRefE, (tbl::ENV_CALC_NRGS *)nrgs, i, tmpNoise, (SCHAR)tmpNoiseE, (UCHAR)sinePresent, (UCHAR)sineMapped, noNoise);
  FPE_GUARD_END
  return 0;
}

void cref_sbrdec_env_calc_avg_gain(void *nrgs, INT lo, INT hi, INT *out) {
  FIXP_DBL sumRef, avg;
  SCHAR sumRefE, avgE;
  tbl::calcAvgGain((tbl::ENV_CALC_NRGS *)nrgs, lo, hi, &sumRef, &sumRefE, &avg, &avgE);
  out[0] = sumRef;
  out[1] = sumRefE;
  out[2] = avg;
  out[3] = avgE;
}

void cref_sbrdec_env_calc_aliasing_reduction(FIXP_DBL *degreeAlias, void *nrgs, UCHAR *useAliasReduction, INT noSubbands) {
  tbl::aliasingReduction(degreeAlias, (tbl::ENV_CALC_NRGS *)nrgs, useAliasReduction, noSubbands);
}

void cref_sbrdec_env_calc_equalize(FIXP_DBL *filtBuffer, SCHAR *filtBufferE, FIXP_DBL *nrgGain, SCHAR *nrgGainE, INT subbands) {
  tbl::equalizeFiltBufferExp(filtBuffer, filtBufferE, nrgGain, nrgGainE, subbands);
}

/* slot: 64 values (ptrReal = slot + lowSubband) */
void cref_sbrdec_env_calc_adjust_lc(FIXP_DBL *slot, INT lowSubband, void *nrgs, UCHAR *harmIndex, INT noSubbands, INT scaleChange, INT noNoise, INT *phaseIndex) {
  tbl::adjustTimeSlotLC(slot + lowSubband, (tbl::ENV_CALC_NRGS *)nrgs, harmIndex, lowSubband, noSubbands, scaleChange, noNoise, phaseIndex);
}

void cref_sbrdec_env_calc_adjust_eld(FIXP_DBL *slot, INT lowSubband, void *nrgs, UCHAR *harmIndex, INT noSubbands, INT scaleChange, INT noNoise, INT *phaseIndex, INT scaleDiffLow) {
  tbl::adjustTimeSlot_EldGrid(slot + lowSubband, (tbl::ENV_CALC_NRGS *)nrgs, harmIndex, lowSubband, noSubbands, scaleChange, noNoise, phaseIndex, scaleDiffLow);
}

/* mode 0: adjustTimeSlotHQ, 1: _GainAndNoise, 2: _AddHarmonics */
void cref_sbrdec_env_calc_adjust_hq(INT mode, FIXP_DBL *re, FIXP_DBL *im, void *calEnv, void *nrgs, INT lowSubband, INT noSubbands, INT scaleChange, INT smoothRatio, INT noNoise, INT filtShift) {
  HANDLE_SBR_CALCULATE_ENVELOPE h = (HANDLE_SBR_CALCULATE_ENVELOPE)calEnv;
  tbl::ENV_CALC_NRGS *n = (tbl::ENV_CALC_NRGS *)nrgs;
  switch (mode) {
    case 0: tbl::adjustTimeSlotHQ(re + lowSubband, im + lowSubband, h, n, lowSubband, noSubbands, scaleChange, (FIXP_SGL)smoothRatio, noNoise, filtShift); break;
    case 1: tbl::adjustTimeSlotHQ_GainAndNoise(re + lowSubband, im + lowSubband, h, n, lowSubband, noSubbands, scaleChange, (FIXP_SGL)smoothRatio, noNoise, filtShift); break;
    default: tbl::adjustTimeSlotHQ_AddHarmonics(re + lowSubband, im + lowSubband, h, n, lowSubband, noSubbands, scaleChange); break;
  }
}

void cref_sbrdec_env_calc_map_sine_flags(UCHAR *table, INT nSfb, UINT *addHarmonics, UINT *harmFlagsPrev, UINT *harmFlagsPrevActive, INT tranEnv, SCHAR *sineMapped) {
  tbl::mapSineFlags(table, nSfb, (ULONG *)addHarmonics, (ULONG *)harmFlagsPrev, (ULONG *)harmFlagsPrevActive, tranEnv, sineMapped);
}

void cref_sbrdec_env_calc_map_sine_flags_pvc(UCHAR *table, INT nSfb, UINT *harmFlagsPrev, UINT *harmFlagsPrevActive, SCHAR *sineMapped, INT sinusoidalPos, SCHAR *sinusoidalPosPrev, INT trailing) {
  tbl::mapSineFlagsPvc(table, nSfb, (ULONG *)harmFlagsPrev, (ULONG *)harmFlagsPrevActive, sineMapped, sinusoidalPos, sinusoidalPosPrev, trailing);
}

/* xOverQmf: NULL or 6 values */
INT cref_sbrdec_env_calc_reset_limiter_bands(UCHAR *limiterBandTable, UCHAR *noLimiterBands, UCHAR *table, INT noFreqBands, void *patchParam, INT noPatches, INT limiterBands, INT patchingMode, INT *xOverQmf, INT b41) {
  return tbl::ResetLimiterBands(limiterBandTable, noLimiterBands, table, noFreqBands, (const PATCH_PARAM *)patchParam, noPatches, limiterBands, (UCHAR)patchingMode, xOverQmf, b41);
}

/* scale: lb_scale, ov_lb_scale, hb_scale, ov_hb_scale */
void cref_sbrdec_env_calc_inter_tes(FIXP_DBL *re, FIXP_DBL *im, INT nSlots, INT *scale, SCHAR *exp, INT rate, INT start, INT stop, INT low, INT nb, INT gammaIdx) {
  FIXP_DBL *r[SLOTS], *i[SLOTS];
  slots(re, nSlots, r);
  slots(im, nSlots, i);
  QMF_SCALE_FACTOR sf = {scale[0], scale[1], scale[2], scale[3]};
  tbl::apply_inter_tes(r, i, &sf, exp, rate, start, stop, low, nb, (UCHAR)gammaIdx);
}

INT cref_sbrdec_env_calc_max_subband_sample(FIXP_DBL *re, FIXP_DBL *im, INT nSlots, INT lo, INT hi, INT start, INT next) {
  FIXP_DBL *r[SLOTS], *i[SLOTS];
  slots(re, nSlots, r);
  slots(im, nSlots, i);
  return tbl::maxSubbandSample(r, im ? i : NULL, lo, hi, start, next);
}

void cref_sbrdec_env_calc_rescale(FIXP_DBL *re, FIXP_DBL *im, INT nSlots, INT lo, INT hi, INT start, INT next, INT shift) {
  FIXP_DBL *r[SLOTS], *i[SLOTS];
  slots(re, nSlots, r);
  slots(im, nSlots, i);
  tbl::rescaleSubbandSamples(r, im ? i : NULL, lo, hi, start, next, shift);
}

/* Legacy SBR (pvc_mode 0). scale: QMF_SCALE_FACTOR (4 values, updated). */
/* returns -1 if the wrapper above recorded a zero divisor (no defined result; the results are
   then not compared), else 0 */
INT cref_sbrdec_env_calc_calculate(INT *scale, void *calEnv, void *header, void *frame, FIXP_DBL *re, FIXP_DBL *im, INT nSlots, INT useLP, FIXP_DBL *degreeAlias, UINT flags, INT frameErrorFlag) {
  FIXP_DBL *r[SLOTS], *i[SLOTS];
  slots(re, nSlots, r);
  slots(im, nSlots, i);
  QMF_SCALE_FACTOR sf = {scale[0], scale[1], scale[2], scale[3]};
  PVC_DYNAMIC_DATA pvc;
  FDKmemclear(&pvc, sizeof(pvc));
  SBR_HEADER_DATA *h = (SBR_HEADER_DATA *)header;
  fix_header(h, 1);
  FPE_GUARD_BEGIN
  tbl::calculateSbrEnvelope(&sf, (HANDLE_SBR_CALCULATE_ENVELOPE)calEnv, h, (HANDLE_SBR_FRAME_DATA)frame, &pvc, r, i, useLP, degreeAlias, flags, frameErrorFlag);
  FPE_GUARD_END
  fix_header(h, 0);
  scale[0] = sf.lb_scale;
  scale[1] = sf.ov_lb_scale;
  scale[2] = sf.hb_scale;
  scale[3] = sf.ov_hb_scale;
  return 0;
}

INT cref_sbrdec_env_calc_create(void *calEnv, void *header, INT chan, UINT flags) {
  SBR_HEADER_DATA *h = (SBR_HEADER_DATA *)header;
  fix_header(h, 1);
  INT r = tbl::createSbrEnvelopeCalc((HANDLE_SBR_CALCULATE_ENVELOPE)calEnv, h, chan, flags);
  fix_header(h, 0);
  return r;
}

}  // extern "C"
