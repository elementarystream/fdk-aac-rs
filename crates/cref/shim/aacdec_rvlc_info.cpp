// Shims for the differential tests of fdk_aac_rs_aacdec::rvlc_info.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_rvlc_info (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/rvlc_info.h"

extern "C" {
/* The constants of rvlc_info.h, in the order of the Rust test. */
INT cref_rvlci_constants(INT *out) {
  INT n = 0;
  out[n++] = FWD;
  out[n++] = BWD;
  out[n++] = MAX_RVL;
  out[n++] = MIN_RVL;
  out[n++] = MAX_ALLOWED_DPCM_INDEX;
  out[n++] = TABLE_OFFSET;
  out[n++] = MAX_LEN_RVLC_CODE_WORD;
  out[n++] = MAX_LEN_RVLC_ESCAPE_WORD;
  out[n++] = DPCM_NOISE_NRG_BITS;
  out[n++] = SF_OFFSET;
  out[n++] = CONCEAL_MAX_INIT;
  out[n++] = CONCEAL_MIN_INIT;
  out[n++] = RVLC_MAX_SFB;
  return n;
}
}
