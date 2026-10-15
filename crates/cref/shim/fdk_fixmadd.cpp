// Shims for the differential tests of fdk_aac_rs_fdk::fixmadd.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_fixmadd (see build.rs).

#include "cref_common.h"
#include "common_fix.h"

extern "C" {
FIXP_DBL cref_fixmadddiv2_DD(FIXP_DBL x, FIXP_DBL a, FIXP_DBL b) { return fixmadddiv2_DD(x, a, b); }
FIXP_DBL cref_fixmadddiv2_SD(FIXP_DBL x, FIXP_SGL a, FIXP_DBL b) { return fixmadddiv2_SD(x, a, b); }
FIXP_DBL cref_fixmadddiv2_DS(FIXP_DBL x, FIXP_DBL a, FIXP_SGL b) { return fixmadddiv2_DS(x, a, b); }
FIXP_DBL cref_fixmadddiv2_SS(FIXP_DBL x, FIXP_SGL a, FIXP_SGL b) { return fixmadddiv2_SS(x, a, b); }
FIXP_DBL cref_fixmsubdiv2_DD(FIXP_DBL x, FIXP_DBL a, FIXP_DBL b) { return fixmsubdiv2_DD(x, a, b); }
FIXP_DBL cref_fixmsubdiv2_SD(FIXP_DBL x, FIXP_SGL a, FIXP_DBL b) { return fixmsubdiv2_SD(x, a, b); }
FIXP_DBL cref_fixmsubdiv2_DS(FIXP_DBL x, FIXP_DBL a, FIXP_SGL b) { return fixmsubdiv2_DS(x, a, b); }
FIXP_DBL cref_fixmsubdiv2_SS(FIXP_DBL x, FIXP_SGL a, FIXP_SGL b) { return fixmsubdiv2_SS(x, a, b); }
FIXP_DBL cref_fixmadddiv2BitExact_DD(FIXP_DBL x, FIXP_DBL a, FIXP_DBL b) { return fixmadddiv2BitExact_DD(x, a, b); }
FIXP_DBL cref_fixmadddiv2BitExact_SD(FIXP_DBL x, FIXP_SGL a, FIXP_DBL b) { return fixmadddiv2BitExact_SD(x, a, b); }
FIXP_DBL cref_fixmadddiv2BitExact_DS(FIXP_DBL x, FIXP_DBL a, FIXP_SGL b) { return fixmadddiv2BitExact_DS(x, a, b); }
FIXP_DBL cref_fixmsubdiv2BitExact_DD(FIXP_DBL x, FIXP_DBL a, FIXP_DBL b) { return fixmsubdiv2BitExact_DD(x, a, b); }
FIXP_DBL cref_fixmsubdiv2BitExact_SD(FIXP_DBL x, FIXP_SGL a, FIXP_DBL b) { return fixmsubdiv2BitExact_SD(x, a, b); }
FIXP_DBL cref_fixmsubdiv2BitExact_DS(FIXP_DBL x, FIXP_DBL a, FIXP_SGL b) { return fixmsubdiv2BitExact_DS(x, a, b); }
FIXP_DBL cref_fixmadd_DD(FIXP_DBL x, FIXP_DBL a, FIXP_DBL b) { return fixmadd_DD(x, a, b); }
FIXP_DBL cref_fixmadd_SD(FIXP_DBL x, FIXP_SGL a, FIXP_DBL b) { return fixmadd_SD(x, a, b); }
FIXP_DBL cref_fixmadd_DS(FIXP_DBL x, FIXP_DBL a, FIXP_SGL b) { return fixmadd_DS(x, a, b); }
FIXP_DBL cref_fixmadd_SS(FIXP_DBL x, FIXP_SGL a, FIXP_SGL b) { return fixmadd_SS(x, a, b); }
FIXP_DBL cref_fixmsub_DD(FIXP_DBL x, FIXP_DBL a, FIXP_DBL b) { return fixmsub_DD(x, a, b); }
FIXP_DBL cref_fixmsub_SD(FIXP_DBL x, FIXP_SGL a, FIXP_DBL b) { return fixmsub_SD(x, a, b); }
FIXP_DBL cref_fixmsub_DS(FIXP_DBL x, FIXP_DBL a, FIXP_SGL b) { return fixmsub_DS(x, a, b); }
FIXP_DBL cref_fixmsub_SS(FIXP_DBL x, FIXP_SGL a, FIXP_SGL b) { return fixmsub_SS(x, a, b); }
FIXP_DBL cref_fixpadddiv2_D(FIXP_DBL x, FIXP_DBL a) { return fixpadddiv2_D(x, a); }
FIXP_DBL cref_fixpadd_D(FIXP_DBL x, FIXP_DBL a) { return fixpadd_D(x, a); }
FIXP_DBL cref_fixpadddiv2_S(FIXP_DBL x, FIXP_SGL a) { return fixpadddiv2_S(x, a); }
FIXP_DBL cref_fixpadd_S(FIXP_DBL x, FIXP_SGL a) { return fixpadd_S(x, a); }
}
