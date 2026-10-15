// Shims for the differential tests of fdk_aac_rs_sbrenc::sbr_misc.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_sbr_misc (see build.rs).

#include "cref_common.h"
#include "libSBRenc/src/sbr_misc.h"

extern "C" {
void cref_sbr_misc_shellsort_fract(FIXP_DBL *in, INT n) { FDKsbrEnc_Shellsort_fract(in, n); }
void cref_sbr_misc_shellsort_int(INT *in, INT n) { FDKsbrEnc_Shellsort_int(in, n); }
void cref_sbr_misc_add_left(INT *v, INT *len, INT value) { FDKsbrEnc_AddLeft(v, len, value); }
void cref_sbr_misc_add_right(INT *v, INT *len, INT value) { FDKsbrEnc_AddRight(v, len, value); }
void cref_sbr_misc_add_vec_left(INT *dst, INT *len, INT *src, INT n) {
  FDKsbrEnc_AddVecLeft(dst, len, src, n);
}
void cref_sbr_misc_add_vec_right(INT *dst, INT *len, INT *src, INT n) {
  FDKsbrEnc_AddVecRight(dst, len, src, n);
}
FIXP_DBL cref_sbr_misc_lsi_divide_scale_fract(FIXP_DBL num, FIXP_DBL denom, FIXP_DBL scale) {
  return FDKsbrEnc_LSI_divide_scale_fract(num, denom, scale);
}
}
