// Shims for the differential tests of fdk_aac_rs_aacdec::usacdec_ace_d4t64.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_usacdec_ace_d4t64 (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/usacdec_ace_d4t64.h"

extern "C" {
/* D_ACELP_decode_4t64(index[8], nbits, code[64]) */
void cref_d4t64_decode_4t64(SHORT *index, INT nbits, SHORT *code) { D_ACELP_decode_4t64(index, nbits, code); }
}
