// Shims for the differential tests of fdk_aac_rs_sbrdec::hfgen_pre_flat.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrdec_hfgen_pre_flat (see build.rs).
//
// The file-static functions are reached through a copy of HFgen_preFlat.cpp in a namespace.
// QMF data is passed as flat arrays of slots of 64 values.

#include "cref_common.h"
#include "libSBRdec/src/HFgen_preFlat.h"

namespace hfp {
#include "libSBRdec/src/HFgen_preFlat.cpp"
}

extern "C" {

/* x, x_sf: POLY_ORDER + 1 values (input and output) */
void cref_sbrdec_hfgen_backsubst_fw(INT numBands, FIXP_DBL *b, FIXP_DBL *x, INT *x_sf) { hfp::backsubst_fw(numBands, b, x, x_sf); }

void cref_sbrdec_hfgen_backsubst_bw(INT numBands, FIXP_DBL *b, FIXP_DBL *x, INT *x_sf) { hfp::backsubst_bw(numBands, b, x, x_sf); }

void cref_sbrdec_hfgen_cholesky_solve(INT numBands, FIXP_DBL *b, INT *b_sf) { hfp::choleskySolve(numBands, b, b_sf); }

void cref_sbrdec_hfgen_polyfit(INT numBands, FIXP_DBL *y, INT y_sf, FIXP_DBL *p, INT *p_sf) { hfp::polyfit(numBands, y, y_sf, p, p_sf); }

FIXP_DBL cref_sbrdec_hfgen_polyval(FIXP_DBL *p, INT *p_sf, INT x, INT *out_sf) { return hfp::polyval(p, p_sf, x, out_sf); }

/* re/im: nSlots slots of 64 values; im NULL: low power */
void cref_sbrdec_hfgen_gain_vec(FIXP_DBL *re, FIXP_DBL *im, INT nSlots, INT eOverlap, INT eCurrent, INT overlap, FIXP_DBL *gainVec, INT *gainVecExp, INT numBands, INT startSample, INT stopSample) {
  FIXP_DBL *r[128], *i[128];
  for (int k = 0; k < nSlots && k < 128; k++) {
    r[k] = re + 64 * k;
    i[k] = (im == NULL) ? NULL : im + 64 * k;
  }
  sbrDecoder_calculateGainVec(r, i, eOverlap, eCurrent, overlap, gainVec, gainVecExp, numBands, startSample, stopSample);
}

}  // extern "C"
