// Shims for the differential tests of fdk_aac_rs_aacdec::ldfiltbank.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_ldfiltbank (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/ldfiltbank.h"
#include "libAACdec/src/aac_rom.h"
#include "dct.h"
#include "FDK_tools_rom.h"
#include "mdct.h"

/* The file-static multE2_DinvF_fdk(). */
namespace ref_ldfb {
#include "libAACdec/src/ldfiltbank.cpp"
}

extern "C" {
INT cref_ldfb_inv_mdct(FIXP_DBL *mdctData, INT mdctData_e, FIXP_DBL *output, FIXP_DBL *fs_buffer, INT N) { return InvMdctTransformLowDelay_fdk(mdctData, mdctData_e, output, fs_buffer, N); }

/* multE2_DinvF_fdk with the window of the frame length N (as InvMdctTransformLowDelay_fdk
   selects it). */
void cref_ldfb_mult_e2(FIXP_DBL *output, FIXP_DBL *x, FIXP_DBL *z, INT N) {
  const FIXP_WTB *coef;
  switch (N) {
    case 256: coef = LowDelaySynthesis256; break;
    case 240: coef = LowDelaySynthesis240; break;
    case 160: coef = LowDelaySynthesis160; break;
    case 128: coef = LowDelaySynthesis128; break;
    case 120: coef = LowDelaySynthesis120; break;
    case 512: coef = LowDelaySynthesis512; break;
    default: coef = LowDelaySynthesis480; break;
  }
  ref_ldfb::multE2_DinvF_fdk(output, x, coef, z, N);
}
}
