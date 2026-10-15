// Shims for the differential tests of fdk_aac_rs_aacdec::usacdec_ace_ltp.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_usacdec_ace_ltp (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/usacdec_ace_ltp.h"

extern "C" {
/* Pred_lt4(buf + off, T0, frac) */
void cref_ltp_pred_lt4(FIXP_DBL *buf, INT off, INT T0, INT frac) { Pred_lt4(buf + off, T0, frac); }
/* Pred_lt4_postfilter(buf + off) */
void cref_ltp_pred_lt4_postfilter(FIXP_DBL *buf, INT off) { Pred_lt4_postfilter(buf + off); }
}
