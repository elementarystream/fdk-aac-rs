// Shims for the differential tests of fdk_aac_rs_sacdec::sac_dec_conceal.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacdec_sac_dec_conceal (see build.rs).
//
// SpatialDecConcealmentInfo is passed as INT[8]: method, numKeepFrames, numFadeOutFrames,
// numFadeInFrames, numReleaseFrames, concealState, cntStateFrames, cntValidFrames.

#include "cref_common.h"
#include "sac_dec_conceal.h"

static void info_from(SpatialDecConcealmentInfo *i, const INT *v) {
  i->concealParams.method = (SpatialDecConcealmentMethod)v[0];
  i->concealParams.numKeepFrames = (UINT)v[1];
  i->concealParams.numFadeOutFrames = (UINT)v[2];
  i->concealParams.numFadeInFrames = (UINT)v[3];
  i->concealParams.numReleaseFrames = (UINT)v[4];
  i->concealState = (SpatialDecConcealmentState)v[5];
  i->cntStateFrames = (UINT)v[6];
  i->cntValidFrames = (UINT)v[7];
}
static void info_to(const SpatialDecConcealmentInfo *i, INT *v) {
  v[0] = i->concealParams.method;
  v[1] = (INT)i->concealParams.numKeepFrames;
  v[2] = (INT)i->concealParams.numFadeOutFrames;
  v[3] = (INT)i->concealParams.numFadeInFrames;
  v[4] = (INT)i->concealParams.numReleaseFrames;
  v[5] = i->concealState;
  v[6] = (INT)i->cntStateFrames;
  v[7] = (INT)i->cntValidFrames;
}

extern "C" {
void cref_sac_dec_conceal_init(INT *info, UINT flags) {
  SpatialDecConcealmentInfo i;
  info_from(&i, info);
  SpatialDecConcealment_Init(&i, flags);
  info_to(&i, info);
}
/* cmp: [9][28], diff: [9][28] or NULL, idxPrev: [28], mode: [9]. */
INT cref_sac_dec_conceal_apply(const INT *info, const SCHAR *cmp, SCHAR *diff, SCHAR *idxPrev, SCHAR *mode, INT startBand, INT stopBand, INT defaultValue, INT paramType, INT numParamSets) {
  SpatialDecConcealmentInfo i;
  info_from(&i, info);
  SCHAR *rows[9];
  for (int k = 0; k < 9; k++) rows[k] = diff == NULL ? NULL : diff + 28 * k;
  return SpatialDecConcealment_Apply(&i, (const SCHAR(*)[28])cmp, diff == NULL ? NULL : rows,
                                     idxPrev, mode, startBand, stopBand, (SCHAR)defaultValue,
                                     paramType, numParamSets);
}
void cref_sac_dec_conceal_update_state(INT *info, INT frameOk) {
  SpatialDecConcealmentInfo i;
  info_from(&i, info);
  SpatialDecConcealment_UpdateState(&i, frameOk);
  info_to(&i, info);
}
/* info == NULL: the NULL handle. */
INT cref_sac_dec_conceal_set_param(INT *info, INT param, INT value) {
  if (info == NULL) {
    return SpatialDecConcealment_SetParam(NULL, (SAC_DEC_CONCEAL_PARAM)param, value);
  }
  SpatialDecConcealmentInfo i;
  info_from(&i, info);
  INT r = SpatialDecConcealment_SetParam(&i, (SAC_DEC_CONCEAL_PARAM)param, value);
  info_to(&i, info);
  return r;
}
}
