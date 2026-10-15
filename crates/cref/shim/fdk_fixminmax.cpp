// Shims for the differential tests of fdk_aac_rs_fdk::fixminmax.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_fixminmax (see build.rs).

#include "cref_common.h"
#include "common_fix.h"

extern "C" {
FIXP_DBL cref_fixmin_D(FIXP_DBL a, FIXP_DBL b) { return fixmin_D(a, b); }
FIXP_DBL cref_fixmax_D(FIXP_DBL a, FIXP_DBL b) { return fixmax_D(a, b); }
FIXP_SGL cref_fixmin_S(FIXP_SGL a, FIXP_SGL b) { return fixmin_S(a, b); }
FIXP_SGL cref_fixmax_S(FIXP_SGL a, FIXP_SGL b) { return fixmax_S(a, b); }
INT cref_fixmin_I(INT a, INT b) { return fixmin_I(a, b); }
INT cref_fixmax_I(INT a, INT b) { return fixmax_I(a, b); }
UINT cref_fixmin_UI(UINT a, UINT b) { return fixmin_UI(a, b); }
UINT cref_fixmax_UI(UINT a, UINT b) { return fixmax_UI(a, b); }
}
