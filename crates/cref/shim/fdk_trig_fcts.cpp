// Shims for the differential tests of fdk_aac_rs_fdk::trig_fcts.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_trig_fcts (see build.rs).
//
// The functions of FDK_trigFcts.cpp are called in the reference library; the inline functions of
// FDK_trigFcts.h are compiled into this shim with the flags of the reference build.

#include "cref_common.h"
#include "FDK_trigFcts.h"
#include "fixpoint_math.h"

/* Defined in FDK_trigFcts.cpp, but not declared in FDK_trigFcts.h. */
void fixp_cos_sin(FIXP_DBL x, int scale, FIXP_DBL *cos, FIXP_DBL *sin);

/* A copy of FDK_trigFcts.cpp, for its file-static table. */
namespace ref_trig {
#include "libFDK/src/FDK_trigFcts.cpp"
}

extern "C" {
FIXP_DBL cref_trig_fcts_fixp_atan(FIXP_DBL x) { return fixp_atan(x); }
FIXP_DBL cref_trig_fcts_fixp_atan2(FIXP_DBL y, FIXP_DBL x) { return fixp_atan2(y, x); }
FIXP_DBL cref_trig_fcts_fixp_cos(FIXP_DBL x, INT scale) { return fixp_cos(x, scale); }
FIXP_DBL cref_trig_fcts_fixp_sin(FIXP_DBL x, INT scale) { return fixp_sin(x, scale); }
void cref_trig_fcts_fixp_cos_sin(FIXP_DBL x, INT scale, FIXP_DBL *out) {
  fixp_cos_sin(x, scale, &out[0], &out[1]);
}
void cref_trig_fcts_inline_fixp_cos_sin(FIXP_DBL x1, FIXP_DBL x2, INT scale, FIXP_DBL *out) {
  inline_fixp_cos_sin(x1, x2, scale, out);
}
FIXP_DBL cref_trig_fcts_fixp_sin_cos_residual_inline(FIXP_DBL x, INT scale, FIXP_DBL *out) {
  return fixp_sin_cos_residual_inline(x, scale, &out[0], &out[1]);
}
const FIXP_DBL *cref_trig_fcts_f_atan_expand_range(INT *len) {
  *len = (INT)(sizeof(ref_trig::f_atan_expand_range) / sizeof(FIXP_DBL));
  return ref_trig::f_atan_expand_range;
}
}
