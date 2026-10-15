// Shims for the differential tests of fdk_aac_rs_fdk::clz.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_clz (see build.rs).

#include "cref_common.h"
#include "common_fix.h"

extern "C" {
INT cref_fixnormz_D(INT x) { return fixnormz_D(x); }
INT cref_fixnorm_D(FIXP_DBL x) { return fixnorm_D(x); }
INT cref_fixnormz_S(SHORT x) { return fixnormz_S(x); }
INT cref_fixnorm_S(FIXP_SGL x) { return fixnorm_S(x); }
}
