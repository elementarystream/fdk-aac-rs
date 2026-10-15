// Shims for the differential tests of fdk_aac_rs_aacenc::quantize.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_quantize (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/quantize.h"

extern "C" {
void cref_quantize_QuantizeSpectrum(INT sfbCnt, INT maxSfbPerGroup, INT sfbPerGroup, const INT *sfbOffset, const FIXP_DBL *mdctSpectrum, INT globalGain, const INT *scalefactors, SHORT *quantizedSpectrum, INT dZoneQuantEnable) {
  FDKaacEnc_QuantizeSpectrum(sfbCnt, maxSfbPerGroup, sfbPerGroup, sfbOffset, mdctSpectrum,
                             globalGain, scalefactors, quantizedSpectrum, dZoneQuantEnable);
}
FIXP_DBL cref_quantize_calcSfbDist(const FIXP_DBL *mdctSpectrum, SHORT *quantSpectrum, INT noOfLines, INT gain, INT dZoneQuantEnable) {
  return FDKaacEnc_calcSfbDist(mdctSpectrum, quantSpectrum, noOfLines, gain, dZoneQuantEnable);
}
void cref_quantize_calcSfbQuantEnergyAndDist(FIXP_DBL *mdctSpectrum, SHORT *quantSpectrum, INT noOfLines, INT gain, FIXP_DBL *en, FIXP_DBL *dist) {
  FDKaacEnc_calcSfbQuantEnergyAndDist(mdctSpectrum, quantSpectrum, noOfLines, gain, en, dist);
}
}
