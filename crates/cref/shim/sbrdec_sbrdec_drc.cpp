// Shims for the differential tests of fdk_aac_rs_sbrdec::sbrdec_drc (tests/sbrdec/sbrdec_drc.rs).
// The channel state is passed as memory (SbrdecDrcChannel has the layout of SBRDEC_DRC_CHANNEL),
// QMF data as flat arrays of slots of 64 values.

#include "cref_common.h"
#include "libSBRdec/src/sbrdec_drc.h"

extern "C" {

void cref_sbrdec_drc_init_channel(void *h) { sbrDecoder_drcInitChannel((HANDLE_SBR_DRC_CHANNEL)h); }

void cref_sbrdec_drc_update_channel(void *h) { sbrDecoder_drcUpdateChannel((HANDLE_SBR_DRC_CHANNEL)h); }

/* im: NULL for low power */
void cref_sbrdec_drc_apply_slot(void *h, FIXP_DBL *re, FIXP_DBL *im, INT col, INT numQmfSubSamples, INT maxShift) {
  sbrDecoder_drcApplySlot((HANDLE_SBR_DRC_CHANNEL)h, re, im, col, numQmfSubSamples, maxShift);
}

void cref_sbrdec_drc_apply(void *h, FIXP_DBL *re, FIXP_DBL *im, INT numQmfSubSamples, INT *scaleFactor) {
  FIXP_DBL *r[64], *i[64];
  for (int k = 0; k < numQmfSubSamples && k < 64; k++) {
    r[k] = re + 64 * k;
    i[k] = im ? im + 64 * k : NULL;
  }
  sbrDecoder_drcApply((HANDLE_SBR_DRC_CHANNEL)h, r, im ? i : NULL, numQmfSubSamples, scaleFactor);
}

}  // extern "C"
