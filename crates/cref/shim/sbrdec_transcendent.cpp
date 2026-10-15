// Shims for the differential tests of fdk_aac_rs_sbrdec::transcendent.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrdec_transcendent (see build.rs).

#include "cref_common.h"
#include "libSBRdec/src/transcendent.h"

extern "C" {

SHORT cref_sbrdec_get_num_octaves_div8(INT a, INT b) { return FDK_getNumOctavesDiv8(a, b); }

/* out[0]: mantissa, out[1]: exponent */
void cref_sbrdec_add_mant_exp_sgl(SHORT a_m, SCHAR a_e, SHORT b_m, SCHAR b_e, INT *out) {
  FIXP_SGL m;
  SCHAR e;
  FDK_add_MantExp((FIXP_SGL)a_m, a_e, (FIXP_SGL)b_m, b_e, &m, &e);
  out[0] = m;
  out[1] = e;
}

void cref_sbrdec_add_mant_exp(INT a_m, SCHAR a_e, INT b_m, SCHAR b_e, INT *out) {
  FIXP_DBL m;
  SCHAR e;
  FDK_add_MantExp((FIXP_DBL)a_m, a_e, (FIXP_DBL)b_m, b_e, &m, &e);
  out[0] = m;
  out[1] = e;
}

void cref_sbrdec_divide_mant_exp_sgl(SHORT a_m, SCHAR a_e, SHORT b_m, SCHAR b_e, INT *out) {
  FIXP_SGL m;
  SCHAR e;
  FDK_divide_MantExp((FIXP_SGL)a_m, a_e, (FIXP_SGL)b_m, b_e, &m, &e);
  out[0] = m;
  out[1] = e;
}

void cref_sbrdec_divide_mant_exp(INT a_m, SCHAR a_e, INT b_m, SCHAR b_e, INT *out) {
  FIXP_DBL m;
  SCHAR e;
  FDK_divide_MantExp((FIXP_DBL)a_m, a_e, (FIXP_DBL)b_m, b_e, &m, &e);
  out[0] = m;
  out[1] = e;
}

/* sameScale != 0: destScale is the exponent itself */
void cref_sbrdec_sqrt_mant_exp(INT m, SCHAR e, INT sameScale, SCHAR destScale, INT *out) {
  FIXP_DBL mm = (FIXP_DBL)m;
  SCHAR ee = e;
  FDK_sqrt_MantExp(&mm, &ee, sameScale ? &ee : &destScale);
  out[0] = mm;
  out[1] = ee;
}

}  // extern "C"
