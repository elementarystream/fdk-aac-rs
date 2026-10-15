// Shims for the differential tests of fdk_aac_rs_fdk::autocorr2nd.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_autocorr2nd (see build.rs).
//
// The buffers start LPC_ORDER values before the pointers passed to the reference functions
// (which read reBuffer[-2] and reBuffer[-1]); the ACORR_COEFS result is returned as 11 INTs in
// the order of the struct.

#include "cref_common.h"
#include "autocorr2nd.h"

static void ac_store(const ACORR_COEFS *ac, INT *out) {
  out[0] = ac->r00r;
  out[1] = ac->r11r;
  out[2] = ac->r22r;
  out[3] = ac->r01r;
  out[4] = ac->r02r;
  out[5] = ac->r12r;
  out[6] = ac->r01i;
  out[7] = ac->r02i;
  out[8] = ac->r12i;
  out[9] = ac->det;
  out[10] = ac->det_scale;
}

static void ac_load(ACORR_COEFS *ac, const INT *in) {
  ac->r00r = in[0];
  ac->r11r = in[1];
  ac->r22r = in[2];
  ac->r01r = in[3];
  ac->r02r = in[4];
  ac->r12r = in[5];
  ac->r01i = in[6];
  ac->r02i = in[7];
  ac->r12i = in[8];
  ac->det = in[9];
  ac->det_scale = in[10];
}

extern "C" {
INT cref_autocorr2nd_real(INT *ac_io, const FIXP_DBL *re, INT len) {
  ACORR_COEFS ac;
  ac_load(&ac, ac_io);
  INT r = autoCorr2nd_real(&ac, re + LPC_ORDER, len);
  ac_store(&ac, ac_io);
  return r;
}
INT cref_autocorr2nd_cplx(INT *ac_io, const FIXP_DBL *re, const FIXP_DBL *im, INT len) {
  ACORR_COEFS ac;
  ac_load(&ac, ac_io);
  INT r = autoCorr2nd_cplx(&ac, re + LPC_ORDER, im + LPC_ORDER, len);
  ac_store(&ac, ac_io);
  return r;
}
}
