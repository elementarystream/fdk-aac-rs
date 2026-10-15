// Shims for the differential tests of fdk_aac_rs_aacenc::spreading.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_spreading (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/spreading.h"

extern "C" {
void cref_spreading_SpreadingMax(INT pbCnt, const FIXP_DBL *maskLowFactor, const FIXP_DBL *maskHighFactor, FIXP_DBL *pbSpreadEnergy) {
  FDKaacEnc_SpreadingMax(pbCnt, maskLowFactor, maskHighFactor, pbSpreadEnergy);
}
}
