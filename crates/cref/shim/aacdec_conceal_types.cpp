// Shims for the differential tests of fdk_aac_rs_aacdec::conceal_types.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_conceal_types (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/channelinfo.h"

#define AL1(s, f) out[n++] = (INT)(sizeof(((s *)0)->f) / sizeof(((s *)0)->f[0]))

extern "C" {
/* The constants of conceal_types.h, in the order of the Rust test. */
INT cref_cnclt_constants(INT *out) {
  INT n = 0;
  out[n++] = CONCEAL_MAX_NUM_FADE_FACTORS;
  out[n++] = (INT)sizeof(FIXP_CNCL);
  out[n++] = CNCL_FRACT_BITS;
  out[n++] = ConcealMethodNone;
  out[n++] = ConcealMethodMute;
  out[n++] = ConcealMethodNoise;
  out[n++] = ConcealMethodInter;
  out[n++] = ConcealMethodTonal;
  out[n++] = ConcealState_Ok;
  out[n++] = ConcealState_Single;
  out[n++] = ConcealState_FadeIn;
  out[n++] = ConcealState_Mute;
  out[n++] = ConcealState_FadeOut;
  out[n++] = FADE_TIMEDOMAIN_TOSPECTRALMUTE;
  out[n++] = FADE_TIMEDOMAIN_FROMSPECTRALMUTE;
  out[n++] = FADE_TIMEDOMAIN;
  return n;
}

/* The number of elements of the array fields, in the order of the Rust test. */
INT cref_cnclt_array_lengths(INT *out) {
  INT n = 0;
  AL1(CConcealParams, fadeOutFactor);
  AL1(CConcealParams, fadeInFactor);
  AL1(CConcealmentInfo, spectralCoefficient);
  AL1(CConcealmentInfo, specScale);
  AL1(CConcealmentInfo, prevFrameOk);
  AL1(CConcealmentInfo, winGrpOffset);
  AL1(CConcealmentInfo, attGrpOffset);
  AL1(CConcealmentInfo, aRvlcPreviousScaleFactor);
  AL1(CConcealmentInfo, aRvlcPreviousCodebook);
  AL1(CConcealmentInfo, lsf4);
  AL1(CConcealmentInfo, TDNoiseStates);
  AL1(CConcealmentInfo, TDNoiseCoef);
  return n;
}
}
