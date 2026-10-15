// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_const.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_const (see build.rs).

#include "cref_common.h"
#include "libSACenc/src/sacenc_const.h"

extern "C" {
/* The integer constants of sacenc_const.h and SACENC_FLOAT_EPSILON. */
void cref_sacenc_const_values(INT *out, float *eps) {
  out[0] = NUM_QMF_BANDS;
  out[1] = MAX_QMF_BANDS;
  out[2] = SACENC_MAX_NUM_BOXES;
  out[3] = SACENC_MAX_INPUT_CHANNELS;
  out[4] = SACENC_MAX_OUTPUT_CHANNELS;
  *eps = SACENC_FLOAT_EPSILON;
}
}
