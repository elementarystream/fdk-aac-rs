// Shims for the differential tests of fdk_aac_rs_fdk::fixmul.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_fixmul (see build.rs).

#include "cref_common.h"
#include "common_fix.h"

extern "C" {
FIXP_DBL cref_fixmuldiv2_DD(FIXP_DBL a, FIXP_DBL b) { return fixmuldiv2_DD(a, b); }
FIXP_DBL cref_fixmuldiv2BitExact_DD(FIXP_DBL a, FIXP_DBL b) { return fixmuldiv2BitExact_DD(a, b); }
FIXP_DBL cref_fixmul_DD(FIXP_DBL a, FIXP_DBL b) { return fixmul_DD(a, b); }
FIXP_DBL cref_fixmulBitExact_DD(FIXP_DBL a, FIXP_DBL b) { return fixmulBitExact_DD(a, b); }
FIXP_DBL cref_fixmuldiv2_SS(SHORT a, SHORT b) { return fixmuldiv2_SS(a, b); }
FIXP_DBL cref_fixmul_SS(SHORT a, SHORT b) { return fixmul_SS(a, b); }
FIXP_DBL cref_fixmuldiv2_SD(SHORT a, FIXP_DBL b) { return fixmuldiv2_SD(a, b); }
FIXP_DBL cref_fixmuldiv2_DS(FIXP_DBL a, SHORT b) { return fixmuldiv2_DS(a, b); }
FIXP_DBL cref_fixmuldiv2BitExact_SD(SHORT a, FIXP_DBL b) { return fixmuldiv2BitExact_SD(a, b); }
FIXP_DBL cref_fixmuldiv2BitExact_DS(FIXP_DBL a, SHORT b) { return fixmuldiv2BitExact_DS(a, b); }
FIXP_DBL cref_fixmul_SD(SHORT a, FIXP_DBL b) { return fixmul_SD(a, b); }
FIXP_DBL cref_fixmul_DS(FIXP_DBL a, SHORT b) { return fixmul_DS(a, b); }
FIXP_DBL cref_fixmulBitExact_SD(SHORT a, FIXP_DBL b) { return fixmulBitExact_SD(a, b); }
FIXP_DBL cref_fixmulBitExact_DS(FIXP_DBL a, SHORT b) { return fixmulBitExact_DS(a, b); }
FIXP_DBL cref_fixpow2div2_D(FIXP_DBL a) { return fixpow2div2_D(a); }
FIXP_DBL cref_fixpow2_D(FIXP_DBL a) { return fixpow2_D(a); }
FIXP_DBL cref_fixpow2div2_S(SHORT a) { return fixpow2div2_S(a); }
FIXP_DBL cref_fixpow2_S(SHORT a) { return fixpow2_S(a); }
}
