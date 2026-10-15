// Shims for the differential tests of the types of fdk_aac_rs_aacdec::usacdec_acelp.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_usacdec_acelp_types (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/usacdec_acelp.h"

#define AL1(s, f) out[n++] = (INT)(sizeof(((s *)0)->f) / sizeof(((s *)0)->f[0]))
#define AL2(s, f) out[n++] = (INT)(sizeof(((s *)0)->f) / sizeof(((s *)0)->f[0][0]))

extern "C" {
/* The number of elements of the array fields, in the order of the Rust test. */
INT cref_acelpt_array_lengths(INT *out) {
  INT n = 0;
  AL1(CAcelpStaticMem, old_exc_mem);
  AL1(CAcelpStaticMem, old_syn_mem);
  AL1(CAcelpStaticMem, A);
  AL1(CAcelpChannelData, T0);
  AL1(CAcelpChannelData, T0_frac);
  AL1(CAcelpChannelData, ltp_filtering_flag);
  AL2(CAcelpChannelData, icb_index);
  AL1(CAcelpChannelData, icb_index);
  AL1(CAcelpChannelData, gains);
  return n;
}
}
