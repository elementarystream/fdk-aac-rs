// Shims for the differential tests of fdk_aac_rs_aacenc::line_pe.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_line_pe (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/line_pe.h"

extern "C" {
/* FDKaacEnc_prepareSfbPe on a PE_CHANNEL_DATA. */
void cref_line_pe_prepareSfbPe(void *peChanData, const FIXP_DBL *sfbEnergyLdData, const FIXP_DBL *sfbThresholdLdData, const FIXP_DBL *sfbFormFactorLdData, const INT *sfbOffset, INT sfbCnt, INT sfbPerGroup, INT maxSfbPerGroup) {
  FDKaacEnc_prepareSfbPe((PE_CHANNEL_DATA *)peChanData, sfbEnergyLdData, sfbThresholdLdData,
                         sfbFormFactorLdData, sfbOffset, sfbCnt, sfbPerGroup, maxSfbPerGroup);
}
/* FDKaacEnc_calcSfbPe on a PE_CHANNEL_DATA. */
void cref_line_pe_calcSfbPe(void *peChanData, const FIXP_DBL *sfbEnergyLdData, const FIXP_DBL *sfbThresholdLdData, INT sfbCnt, INT sfbPerGroup, INT maxSfbPerGroup, const INT *isBook, const INT *isScale) {
  FDKaacEnc_calcSfbPe((PE_CHANNEL_DATA *)peChanData, sfbEnergyLdData, sfbThresholdLdData, sfbCnt,
                      sfbPerGroup, maxSfbPerGroup, isBook, isScale);
}
}
