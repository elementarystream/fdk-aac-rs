// Shims for the differential tests of fdk_aac_rs_fdk::common_fix.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_common_fix (see build.rs).

#include "cref_common.h"
#include "common_fix.h"

extern "C" {
/* fMult / fMultDiv2: every overload. */
FIXP_DBL cref_fMult_SS(FIXP_SGL a, FIXP_SGL b) { return fMult(a, b); }
FIXP_DBL cref_fMult_SD(FIXP_SGL a, FIXP_DBL b) { return fMult(a, b); }
FIXP_DBL cref_fMult_DS(FIXP_DBL a, FIXP_SGL b) { return fMult(a, b); }
FIXP_DBL cref_fMult_DD(FIXP_DBL a, FIXP_DBL b) { return fMult(a, b); }
FIXP_DBL cref_fMultDiv2_SS(FIXP_SGL a, FIXP_SGL b) { return fMultDiv2(a, b); }
FIXP_DBL cref_fMultDiv2_SD(FIXP_SGL a, FIXP_DBL b) { return fMultDiv2(a, b); }
FIXP_DBL cref_fMultDiv2_DS(FIXP_DBL a, FIXP_SGL b) { return fMultDiv2(a, b); }
FIXP_DBL cref_fMultDiv2_DD(FIXP_DBL a, FIXP_DBL b) { return fMultDiv2(a, b); }
FIXP_DBL cref_fMultBitExact_DD(FIXP_DBL a, FIXP_DBL b) { return fMultBitExact(a, b); }
FIXP_DBL cref_fMultBitExact_SD(FIXP_SGL a, FIXP_DBL b) { return fMultBitExact(a, b); }
FIXP_DBL cref_fMultBitExact_DS(FIXP_DBL a, FIXP_SGL b) { return fMultBitExact(a, b); }
FIXP_DBL cref_fMultDiv2BitExact_DD(FIXP_DBL a, FIXP_DBL b) { return fMultDiv2BitExact(a, b); }
FIXP_DBL cref_fMultDiv2BitExact_SD(FIXP_SGL a, FIXP_DBL b) { return fMultDiv2BitExact(a, b); }
FIXP_DBL cref_fMultDiv2BitExact_DS(FIXP_DBL a, FIXP_SGL b) { return fMultDiv2BitExact(a, b); }
FIXP_DBL cref_fPow2_D(FIXP_DBL a) { return fPow2(a); }
FIXP_DBL cref_fPow2_S(FIXP_SGL a) { return fPow2(a); }
FIXP_DBL cref_fPow2Div2_D(FIXP_DBL a) { return fPow2Div2(a); }
FIXP_DBL cref_fPow2Div2_S(FIXP_SGL a) { return fPow2Div2(a); }
FIXP_DBL cref_fPow2Add_D(FIXP_DBL x, FIXP_DBL a) { return fPow2Add(x, a); }
FIXP_DBL cref_fPow2Add_S(FIXP_DBL x, FIXP_SGL a) { return fPow2Add(x, a); }
FIXP_DBL cref_fPow2AddDiv2_D(FIXP_DBL x, FIXP_DBL a) { return fPow2AddDiv2(x, a); }
FIXP_DBL cref_fPow2AddDiv2_S(FIXP_DBL x, FIXP_SGL a) { return fPow2AddDiv2(x, a); }

/* fAbs, fNorm, fNormz */
FIXP_DBL cref_fAbs_D(FIXP_DBL x) { return fAbs(x); }
FIXP_SGL cref_fAbs_S(FIXP_SGL x) { return fAbs(x); }
FIXP_DBL cref_fixp_abs_D(FIXP_DBL x) { return fixp_abs(x); }
INT cref_fNormz_Q(INT64 x) { return fNormz(x); }
INT cref_fNormz_D(FIXP_DBL x) { return fNormz(x); }
INT cref_fNormz_S(FIXP_SGL x) { return fNormz(x); }
INT cref_fNorm_D(FIXP_DBL x) { return fNorm(x); }
INT cref_fNorm_S(FIXP_SGL x) { return fNorm(x); }
INT cref_CntLeadingZeros(INT x) { return CntLeadingZeros(x); }
INT cref_CountLeadingBits(INT x) { return CountLeadingBits(x); }

/* fMultAdd family: every overload. */
FIXP_DBL cref_fMultAdd_DD(FIXP_DBL x, FIXP_DBL a, FIXP_DBL b) { return fMultAdd(x, a, b); }
FIXP_DBL cref_fMultAdd_SD(FIXP_DBL x, FIXP_SGL a, FIXP_DBL b) { return fMultAdd(x, a, b); }
FIXP_DBL cref_fMultAdd_DS(FIXP_DBL x, FIXP_DBL a, FIXP_SGL b) { return fMultAdd(x, a, b); }
FIXP_DBL cref_fMultAdd_SS(FIXP_DBL x, FIXP_SGL a, FIXP_SGL b) { return fMultAdd(x, a, b); }
FIXP_DBL cref_fMultAddDiv2_DD(FIXP_DBL x, FIXP_DBL a, FIXP_DBL b) { return fMultAddDiv2(x, a, b); }
FIXP_DBL cref_fMultAddDiv2_SD(FIXP_DBL x, FIXP_SGL a, FIXP_DBL b) { return fMultAddDiv2(x, a, b); }
FIXP_DBL cref_fMultAddDiv2_DS(FIXP_DBL x, FIXP_DBL a, FIXP_SGL b) { return fMultAddDiv2(x, a, b); }
FIXP_DBL cref_fMultAddDiv2_SS(FIXP_DBL x, FIXP_SGL a, FIXP_SGL b) { return fMultAddDiv2(x, a, b); }
FIXP_DBL cref_fMultSub_DD(FIXP_DBL x, FIXP_DBL a, FIXP_DBL b) { return fMultSub(x, a, b); }
FIXP_DBL cref_fMultSub_SD(FIXP_DBL x, FIXP_SGL a, FIXP_DBL b) { return fMultSub(x, a, b); }
FIXP_DBL cref_fMultSub_DS(FIXP_DBL x, FIXP_DBL a, FIXP_SGL b) { return fMultSub(x, a, b); }
FIXP_DBL cref_fMultSub_SS(FIXP_DBL x, FIXP_SGL a, FIXP_SGL b) { return fMultSub(x, a, b); }
FIXP_DBL cref_fMultSubDiv2_DD(FIXP_DBL x, FIXP_DBL a, FIXP_DBL b) { return fMultSubDiv2(x, a, b); }
FIXP_DBL cref_fMultSubDiv2_SD(FIXP_DBL x, FIXP_SGL a, FIXP_DBL b) { return fMultSubDiv2(x, a, b); }
FIXP_DBL cref_fMultSubDiv2_DS(FIXP_DBL x, FIXP_DBL a, FIXP_SGL b) { return fMultSubDiv2(x, a, b); }
FIXP_DBL cref_fMultSubDiv2_SS(FIXP_DBL x, FIXP_SGL a, FIXP_SGL b) { return fMultSubDiv2(x, a, b); }
FIXP_DBL cref_fMultAddDiv2BitExact_DD(FIXP_DBL x, FIXP_DBL a, FIXP_DBL b) { return fMultAddDiv2BitExact(x, a, b); }
FIXP_DBL cref_fMultAddDiv2BitExact_SD(FIXP_DBL x, FIXP_SGL a, FIXP_DBL b) { return fMultAddDiv2BitExact(x, a, b); }
FIXP_DBL cref_fMultAddDiv2BitExact_DS(FIXP_DBL x, FIXP_DBL a, FIXP_SGL b) { return fMultAddDiv2BitExact(x, a, b); }
FIXP_DBL cref_fMultSubDiv2BitExact_DD(FIXP_DBL x, FIXP_DBL a, FIXP_DBL b) { return fMultSubDiv2BitExact(x, a, b); }
FIXP_DBL cref_fMultSubDiv2BitExact_SD(FIXP_DBL x, FIXP_SGL a, FIXP_DBL b) { return fMultSubDiv2BitExact(x, a, b); }
FIXP_DBL cref_fMultSubDiv2BitExact_DS(FIXP_DBL x, FIXP_DBL a, FIXP_SGL b) { return fMultSubDiv2BitExact(x, a, b); }

/* fMin / fMax: every overload. */
FIXP_DBL cref_fMin_D(FIXP_DBL a, FIXP_DBL b) { return fMin(a, b); }
FIXP_DBL cref_fMax_D(FIXP_DBL a, FIXP_DBL b) { return fMax(a, b); }
FIXP_SGL cref_fMin_S(FIXP_SGL a, FIXP_SGL b) { return fMin(a, b); }
FIXP_SGL cref_fMax_S(FIXP_SGL a, FIXP_SGL b) { return fMax(a, b); }
UINT cref_fMin_UI(UINT a, UINT b) { return fMin(a, b); }
UINT cref_fMax_UI(UINT a, UINT b) { return fMax(a, b); }
UCHAR cref_fMin_UC(UCHAR a, UCHAR b) { return fMin(a, b); }
UCHAR cref_fMax_UC(UCHAR a, UCHAR b) { return fMax(a, b); }
FIXP_DBL cref_fixMin(FIXP_DBL a, FIXP_DBL b) { return fixMin(a, b); }
FIXP_DBL cref_fixMax(FIXP_DBL a, FIXP_DBL b) { return fixMax(a, b); }

/* Conversion macros. */
FIXP_SGL cref_FX_DBL2FXCONST_SGL(FIXP_DBL v) { return FX_DBL2FXCONST_SGL(v); }
FIXP_DBL cref_FX_SGL2FX_DBL(FIXP_SGL v) { return FX_SGL2FX_DBL(v); }
FIXP_SGL cref_FX_DBL2FX_SGL(FIXP_DBL v) { return FX_DBL2FX_SGL(v); }
FIXP_SGL cref_FL2FXCONST_SGL(double v) { return FL2FXCONST_SGL(v); }
FIXP_DBL cref_FL2FXCONST_DBL(double v) { return FL2FXCONST_DBL(v); }
FIXP_DBL cref_FL2FXCONST_SPC(double v) { return FL2FXCONST_SPC(v); }
FIXP_SGL cref_FL2FX_SGL(float v) { return FL2FX_SGL(v); }
FIXP_DBL cref_FL2FX_DBL(float v) { return FL2FX_DBL(v); }
float cref_FX_DBL2FL(FIXP_DBL v) { return FX_DBL2FL(v); }
double cref_FX_DBL2DOUBLE(FIXP_DBL v) { return FX_DBL2DOUBLE(v); }
FIXP_DBL cref_FX_PCM2FX_DBL(SHORT v) { return FX_PCM2FX_DBL(v); }
SHORT cref_FX_DBL2FX_PCM(FIXP_DBL v) { return FX_DBL2FX_PCM(v); }

/* Constants (index order of crates/cref/tests/fdk/common_fix.rs). */
INT64 cref_common_fix_const(INT i) {
  static const INT64 v[] = {FRACT_BITS, DFRACT_BITS, ACCU_BITS, (INT64)SGL_MASK, MAX_SHIFT_SGL,
                            MAX_SHIFT_DBL, FRACT_FIX_SCALE, DFRACT_FIX_SCALE, MAXVAL_SGL,
                            MINVAL_SGL, MAXVAL_DBL, MINVAL_DBL, MINVAL_DBL_CONST, MINVAL_SGL_CONST,
                            MAXVAL_FIXP_PCM, MINVAL_FIXP_PCM, (INT64)sizeof(FIXP_SPK),
                            (INT64)sizeof(FIXP_DPK)};
  return v[i];
}
}
