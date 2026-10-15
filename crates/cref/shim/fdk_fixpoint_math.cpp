// Shims for the differential tests of fdk_aac_rs_fdk::fixpoint_math.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_fixpoint_math (see build.rs).

#include "cref_common.h"
#include "common_fix.h"
#include "fixpoint_math.h"

extern "C" {
/* fixpoint_math.h */
INT cref_fIsLessThan_D(FIXP_DBL a_m, INT a_e, FIXP_DBL b_m, INT b_e) { return fIsLessThan(a_m, a_e, b_m, b_e); }
INT cref_fIsLessThan_S(FIXP_SGL a_m, INT a_e, FIXP_SGL b_m, INT b_e) { return fIsLessThan(a_m, a_e, b_m, b_e); }
FIXP_DBL cref_CalcLdData(FIXP_DBL op) { return CalcLdData(op); }
FIXP_DBL cref_CalcInvLdData(FIXP_DBL x) { return CalcInvLdData(x); }
FIXP_DBL cref_CalcLdInt(INT i) { return CalcLdInt(i); }
FIXP_DBL cref_sqrtFixp_lookup(FIXP_DBL x) { return sqrtFixp_lookup(x); }
FIXP_DBL cref_sqrtFixp_lookup_e(FIXP_DBL x, INT *x_e) { return sqrtFixp_lookup(x, x_e); }
FIXP_DBL cref_invSqrtNorm2(FIXP_DBL op, INT *shift) { return invSqrtNorm2(op, shift); }
FIXP_DBL cref_sqrtFixp(FIXP_DBL op) { return sqrtFixp(op); }
FIXP_DBL cref_invFixp(FIXP_DBL op) { return invFixp(op); }
FIXP_DBL cref_invFixp_e(FIXP_DBL op_m, INT *op_e) { return invFixp(op_m, op_e); }
FIXP_DBL cref_schur_div(FIXP_DBL num, FIXP_DBL denum, INT count) { return schur_div(num, denum, count); }
FIXP_DBL cref_fMultNorm_e(FIXP_DBL f1, FIXP_DBL f2, INT *result_e) { return fMultNorm(f1, f2, result_e); }
FIXP_DBL cref_fMultNorm(FIXP_DBL f1, FIXP_DBL f2) { return fMultNorm(f1, f2); }
FIXP_DBL cref_fMultNorm_to(FIXP_DBL f1_m, INT f1_e, FIXP_DBL f2_m, INT f2_e, INT result_e) { return fMultNorm(f1_m, f1_e, f2_m, f2_e, result_e); }
INT cref_fMultI(FIXP_DBL a, INT b) { return fMultI(a, b); }
INT cref_fMultIfloor(FIXP_DBL a, INT b) { return fMultIfloor(a, b); }
INT cref_fMultIceil(FIXP_DBL a, INT b) { return fMultIceil(a, b); }
FIXP_DBL cref_fDivNorm_e(FIXP_DBL num, FIXP_DBL denom, INT *result_e) { return fDivNorm(num, denom, result_e); }
FIXP_DBL cref_fDivNorm(FIXP_DBL num, FIXP_DBL denom) { return fDivNorm(num, denom); }
FIXP_DBL cref_fDivNormSigned_e(FIXP_DBL num, FIXP_DBL denom, INT *result_e) { return fDivNormSigned(num, denom, result_e); }
FIXP_DBL cref_fDivNormSigned(FIXP_DBL num, FIXP_DBL denom) { return fDivNormSigned(num, denom); }
FIXP_DBL cref_fDivNormHighPrec(FIXP_DBL num, FIXP_DBL denom, INT *result_e) { return fDivNormHighPrec(num, denom, result_e); }
FIXP_DBL cref_fAdjust(FIXP_DBL a_m, INT *pA_e) { return fAdjust(a_m, pA_e); }
FIXP_DBL cref_fAddNorm_e(FIXP_DBL a_m, INT a_e, FIXP_DBL b_m, INT b_e, INT *pResult_e) { return fAddNorm(a_m, a_e, b_m, b_e, pResult_e); }
FIXP_DBL cref_fAddNorm_to(FIXP_DBL a_m, INT a_e, FIXP_DBL b_m, INT b_e, INT result_e) { return fAddNorm(a_m, a_e, b_m, b_e, result_e); }
FIXP_DBL cref_f2Pow_e(FIXP_DBL exp_m, INT exp_e, INT *result_e) { return f2Pow(exp_m, exp_e, result_e); }
FIXP_DBL cref_f2Pow(FIXP_DBL exp_m, INT exp_e) { return f2Pow(exp_m, exp_e); }
FIXP_DBL cref_fPow(FIXP_DBL base_m, INT base_e, FIXP_DBL exp_m, INT exp_e, INT *result_e) { return fPow(base_m, base_e, exp_m, exp_e, result_e); }
FIXP_DBL cref_fLdPow_e(FIXP_DBL baseLd_m, INT baseLd_e, FIXP_DBL exp_m, INT exp_e, INT *result_e) { return fLdPow(baseLd_m, baseLd_e, exp_m, exp_e, result_e); }
FIXP_DBL cref_fLdPow(FIXP_DBL baseLd_m, INT baseLd_e, FIXP_DBL exp_m, INT exp_e) { return fLdPow(baseLd_m, baseLd_e, exp_m, exp_e); }
FIXP_DBL cref_fPowInt(FIXP_DBL base_m, INT base_e, INT N, INT *result_e) { return fPowInt(base_m, base_e, N, result_e); }
FIXP_DBL cref_fLog2_e(FIXP_DBL x_m, INT x_e, INT *result_e) { return fLog2(x_m, x_e, result_e); }
FIXP_DBL cref_fLog2(FIXP_DBL x_m, INT x_e) { return fLog2(x_m, x_e); }
FIXP_DBL cref_CalcLog2(FIXP_DBL x_m, INT x_e, INT *result_e) { return CalcLog2(x_m, x_e, result_e); }
void cref_LdDataVector(FIXP_DBL *src, FIXP_DBL *dst, INT n) { LdDataVector(src, dst, n); }
FIXP_SGL cref_fAddSaturate_S(FIXP_SGL a, FIXP_SGL b) { return fAddSaturate(a, b); }
FIXP_DBL cref_fAddSaturate_D(FIXP_DBL a, FIXP_DBL b) { return fAddSaturate(a, b); }
INT cref_fixp_floorToInt(FIXP_DBL f, INT sf) { return fixp_floorToInt(f, sf); }
FIXP_DBL cref_fixp_floor(FIXP_DBL f, INT sf) { return fixp_floor(f, sf); }
INT cref_fixp_ceilToInt(FIXP_DBL f, INT sf) { return fixp_ceilToInt(f, sf); }
FIXP_DBL cref_fixp_ceil(FIXP_DBL f, INT sf) { return fixp_ceil(f, sf); }
INT cref_fixp_truncateToInt(FIXP_DBL f, INT sf) { return fixp_truncateToInt(f, sf); }
FIXP_DBL cref_fixp_truncate(FIXP_DBL f, INT sf) { return fixp_truncate(f, sf); }
INT cref_fixp_roundToInt(FIXP_DBL f, INT sf) { return fixp_roundToInt(f, sf); }
FIXP_DBL cref_fixp_round(FIXP_DBL f, INT sf) { return fixp_round(f, sf); }
FIXP_DBL cref_GetInvInt(INT i) { return GetInvInt(i); }

/* Tables */
const FIXP_SGL *cref_ldCoeff(void) { return ldCoeff; }
const UINT *cref_exp2_tab_long(void) { return exp2_tab_long; }
const UINT *cref_exp2w_tab_long(void) { return exp2w_tab_long; }
const UINT *cref_exp2x_tab_long(void) { return exp2x_tab_long; }
const FIXP_DBL *cref_invSqrtTab(void) { return invSqrtTab; }
const USHORT *cref_sqrt_tab(void) { return sqrt_tab; }
const FIXP_DBL *cref_invCount(void) { return invCount; }
}
