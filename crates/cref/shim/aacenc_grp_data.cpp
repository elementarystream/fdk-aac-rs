// Shims for the differential tests of fdk_aac_rs_aacenc::grp_data.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_grp_data (see build.rs).
//
// The SFB_THRESHOLD / SFB_ENERGY unions are passed as flat arrays of their 120 words.

#include "cref_common.h"
#include "libAACenc/src/grp_data.h"

extern "C" {
void cref_grp_data_groupShortData(FIXP_DBL *mdctSpectrum, FIXP_DBL *sfbThreshold, FIXP_DBL *sfbEnergy, FIXP_DBL *sfbEnergyMS, FIXP_DBL *sfbSpreadEnergy, INT sfbCnt, INT sfbActive, const INT *sfbOffset, const FIXP_DBL *sfbMinSnrLdData, INT *groupedSfbOffset, INT *maxSfbPerGroup, FIXP_DBL *groupedSfbMinSnrLdData, INT noOfGroups, const INT *groupLen, INT granuleLength) {
  FDKaacEnc_groupShortData(mdctSpectrum, (SFB_THRESHOLD *)sfbThreshold, (SFB_ENERGY *)sfbEnergy,
                           (SFB_ENERGY *)sfbEnergyMS, (SFB_ENERGY *)sfbSpreadEnergy, sfbCnt,
                           sfbActive, sfbOffset, sfbMinSnrLdData, groupedSfbOffset,
                           maxSfbPerGroup, groupedSfbMinSnrLdData, noOfGroups, groupLen,
                           granuleLength);
}
INT cref_grp_data_sizeof_sfb_threshold() { return (INT)sizeof(SFB_THRESHOLD); }
}
