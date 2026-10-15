// Shims for the differential tests of fdk_aac_rs_aacenc::noisedet.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_noisedet (see build.rs).
//
// FDKaacEnc_noiseDetect is called in the library; the file-static FDKaacEnc_fuzzyIsSmaller is
// reached by including noisedet.cpp into a namespace (PORTING.md, "Differential tests").
// NOISEPARAMS is passed as its memory image (void*).

#include "cref_common.h"
#include "libAACenc/src/noisedet.h"
#include "libAACenc/src/aacenc_pns.h"
#include "libAACenc/src/pnsparam.h"

namespace ref_noisedet {
#include "libAACenc/src/noisedet.cpp"
}

extern "C" {
void cref_noisedet_noiseDetect(FIXP_DBL *mdctSpectrum, INT *sfbMaxScaleSpec, INT sfbActive, const INT *sfbOffset, FIXP_SGL *noiseFuzzyMeasure, void *np, FIXP_SGL *sfbtonality) {
  FDKaacEnc_noiseDetect(mdctSpectrum, sfbMaxScaleSpec, sfbActive, sfbOffset, noiseFuzzyMeasure, (NOISEPARAMS *)np, sfbtonality);
}
FIXP_SGL cref_noisedet_fuzzyIsSmaller(FIXP_DBL testVal, FIXP_DBL refVal, FIXP_DBL loLim, FIXP_DBL hiLim) {
  return ref_noisedet::FDKaacEnc_fuzzyIsSmaller(testVal, refVal, loLim, hiLim);
}
}
