// Shims for the differential tests of fdk_aac_rs_fdk::scale.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_scale (see build.rs).

#include "cref_common.h"
#include "common_fix.h"
#include "scale.h"

extern "C" {
FIXP_DBL cref_scaleValue_D(FIXP_DBL v, INT s) { return scaleValue(v, s); }
FIXP_SGL cref_scaleValue_S(FIXP_SGL v, INT s) { return scaleValue(v, s); }
FIXP_DBL cref_scaleValueSaturate(FIXP_DBL v, INT s) { return scaleValueSaturate(v, s); }
FIXP_DBL cref_scaleValueInPlace(FIXP_DBL v, INT s) {
  scaleValueInPlace(&v, s);
  return v;
}

/* The saturating shift macros with run-time arguments. */
INT cref_SATURATE_RIGHT_SHIFT(INT src, INT scale, INT dBits) { return SATURATE_RIGHT_SHIFT(src, scale, dBits); }
INT cref_SATURATE_LEFT_SHIFT(INT src, INT scale, INT dBits) { return SATURATE_LEFT_SHIFT(src, scale, dBits); }
INT cref_SATURATE_SHIFT(INT src, INT scale, INT dBits) { return SATURATE_SHIFT(src, scale, dBits); }
INT cref_SATURATE_LEFT_SHIFT_ALT(INT src, INT scale, INT dBits) { return SATURATE_LEFT_SHIFT_ALT(src, scale, dBits); }
INT cref_SATURATE_RIGHT_SHIFT_ALT(INT src, INT scale, INT dBits) { return SATURATE_RIGHT_SHIFT_ALT(src, scale, dBits); }
INT cref_SATURATE_INT_PCM_RIGHT_SHIFT(INT src, INT scale) { return SATURATE_INT_PCM_RIGHT_SHIFT(src, scale); }
INT cref_SATURATE_INT_PCM_LEFT_SHIFT(INT src, INT scale) { return SATURATE_INT_PCM_LEFT_SHIFT(src, scale); }
/* The macros with the constant dBits of the library's call sites. */
INT cref_SATURATE_SHIFT_32(INT src, INT scale) { return SATURATE_SHIFT(src, scale, DFRACT_BITS); }
INT cref_SATURATE_RIGHT_SHIFT_16(INT src, INT scale) { return SATURATE_RIGHT_SHIFT(src, scale, 16); }
INT cref_SATURATE_LEFT_SHIFT_16(INT src, INT scale) { return SATURATE_LEFT_SHIFT(src, scale, 16); }
INT cref_SATURATE_RIGHT_SHIFT_32(INT src, INT scale) { return SATURATE_RIGHT_SHIFT(src, scale, 32); }
INT cref_SATURATE_LEFT_SHIFT_32(INT src, INT scale) { return SATURATE_LEFT_SHIFT(src, scale, 32); }

/* scale.cpp */
void cref_scaleValues_S(FIXP_SGL *v, INT len, INT sf) { scaleValues(v, len, sf); }
void cref_scaleValues_D(FIXP_DBL *v, INT len, INT sf) { scaleValues(v, len, sf); }
void cref_scaleValues_DD(FIXP_DBL *dst, const FIXP_DBL *src, INT len, INT sf) { scaleValues(dst, src, len, sf); }
void cref_scaleValues_PCMD(SHORT *dst, const FIXP_DBL *src, INT len, INT sf) { scaleValues((FIXP_PCM *)dst, src, len, sf); }
void cref_scaleValues_SS(FIXP_SGL *dst, const FIXP_SGL *src, INT len, INT sf) { scaleValues(dst, src, len, sf); }
void cref_scaleValuesWithFactor(FIXP_DBL *v, FIXP_DBL factor, INT len, INT sf) { scaleValuesWithFactor(v, factor, len, sf); }
void cref_scaleValuesSaturate_D(FIXP_DBL *v, INT len, INT sf) { scaleValuesSaturate(v, len, sf); }
void cref_scaleValuesSaturate_DD(FIXP_DBL *dst, const FIXP_DBL *src, INT len, INT sf) { scaleValuesSaturate(dst, src, len, sf); }
void cref_scaleValuesSaturate_SD(FIXP_SGL *dst, const FIXP_DBL *src, INT len, INT sf) { scaleValuesSaturate(dst, src, len, sf); }
void cref_scaleValuesSaturate_S(FIXP_SGL *v, INT len, INT sf) { scaleValuesSaturate(v, len, sf); }
void cref_scaleValuesSaturate_SS(FIXP_SGL *dst, const FIXP_SGL *src, INT len, INT sf) { scaleValuesSaturate(dst, src, len, sf); }
INT cref_getScalefactorShort(const SHORT *v, INT len) { return getScalefactorShort(v, len); }
INT cref_getScalefactorPCM(const SHORT *v, INT len, INT stride) { return getScalefactorPCM((const INT_PCM *)v, len, stride); }
INT cref_getScalefactor_D(const FIXP_DBL *v, INT len) { return getScalefactor(v, len); }
INT cref_getScalefactor_S(const FIXP_SGL *v, INT len) { return getScalefactor(v, len); }
}
