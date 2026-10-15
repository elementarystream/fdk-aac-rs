// Shims for the differential tests of fdk_aac_rs_fdk::abs.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_abs (see build.rs).

#include "cref_common.h"
#include "common_fix.h"

extern "C" {
FIXP_DBL cref_fixabs_D(FIXP_DBL x) { return fixabs_D(x); }
INT cref_fixabs_I(INT x) { return fixabs_I(x); }
/* The return type of fixabs_S is FIXP_SGL in abs.h and INT in x86/abs_x86.h; the shim returns
   the value as INT so that the test sees the difference. */
INT cref_fixabs_S(FIXP_SGL x) { return fixabs_S(x); }
}
