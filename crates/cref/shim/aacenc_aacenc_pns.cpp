// Shims for the differential tests of fdk_aac_rs_aacenc::aacenc_pns (and pns_func).
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_aacenc_pns (see build.rs).
//
// The public functions are called in the library; the file-static functions of aacenc_pns.cpp
// are reached by including the source file into a namespace (PORTING.md, "Differential tests").
// PNS_CONFIG and PNS_DATA are passed as their memory images (void*).

#include "cref_common.h"
#include "libAACenc/src/aacenc_pns.h"
#include "libAACenc/src/psy_data.h"
#include "libAACenc/src/pnsparam.h"
#include "libAACenc/src/noisedet.h"
#include "libAACenc/src/bit_cnt.h"
#include "libAACenc/src/interface.h"
#include "libAACenc/src/pns_func.h"

namespace ref_pns {
#include "libAACenc/src/aacenc_pns.cpp"
}

extern "C" {
/* --- library functions --- */
INT cref_aacenc_pns_InitPnsConfiguration(void *pnsConf, INT bitRate, INT sampleRate, INT usePns, INT sfbCnt, const INT *sfbOffset, INT numChan, INT isLC) {
  return (INT)FDKaacEnc_InitPnsConfiguration((PNS_CONFIG *)pnsConf, bitRate, sampleRate, usePns, sfbCnt, sfbOffset, numChan, isLC);
}
void cref_aacenc_pns_PnsDetect(void *pnsConf, void *pnsData, INT lastWindowSequence, INT sfbActive, INT maxSfbPerGroup, FIXP_DBL *sfbThresholdLdData, const INT *sfbOffset, FIXP_DBL *mdctSpectrum, INT *sfbMaxScaleSpec, FIXP_SGL *sfbtonality, INT tnsOrder, INT tnsPredictionGain, INT tnsActive, FIXP_DBL *sfbEnergyLdData, INT *noiseNrg) {
  FDKaacEnc_PnsDetect((PNS_CONFIG *)pnsConf, (PNS_DATA *)pnsData, lastWindowSequence, sfbActive, maxSfbPerGroup, sfbThresholdLdData, sfbOffset, mdctSpectrum, sfbMaxScaleSpec, sfbtonality, tnsOrder, tnsPredictionGain, tnsActive, sfbEnergyLdData, noiseNrg);
}
void cref_aacenc_pns_CodePnsChannel(INT sfbActive, void *pnsConf, INT *pnsFlag, FIXP_DBL *sfbEnergy, INT *noiseNrg, FIXP_DBL *sfbThreshold) {
  FDKaacEnc_CodePnsChannel(sfbActive, (PNS_CONFIG *)pnsConf, pnsFlag, sfbEnergy, noiseNrg, sfbThreshold);
}
void cref_aacenc_pns_PreProcessPnsChannelPair(INT sfbActive, FIXP_DBL *sfbEnergyLeft, FIXP_DBL *sfbEnergyRight, FIXP_DBL *sfbEnergyLeftLD, FIXP_DBL *sfbEnergyRightLD, FIXP_DBL *sfbEnergyMid, void *pnsConfLeft, void *pnsDataLeft, void *pnsDataRight) {
  FDKaacEnc_PreProcessPnsChannelPair(sfbActive, sfbEnergyLeft, sfbEnergyRight, sfbEnergyLeftLD, sfbEnergyRightLD, sfbEnergyMid, (PNS_CONFIG *)pnsConfLeft, (PNS_DATA *)pnsDataLeft, (PNS_DATA *)pnsDataRight);
}
void cref_aacenc_pns_PostProcessPnsChannelPair(INT sfbActive, void *pnsConf, void *pnsDataLeft, void *pnsDataRight, INT *msMask, INT *msDigest) {
  FDKaacEnc_PostProcessPnsChannelPair(sfbActive, (PNS_CONFIG *)pnsConf, (PNS_DATA *)pnsDataLeft, (PNS_DATA *)pnsDataRight, msMask, msDigest);
}

/* --- file-static functions (namespace copy) --- */
void cref_aacenc_pns_noiseDetection(void *pnsConf, void *pnsData, INT sfbActive, const INT *sfbOffset, INT tnsOrder, INT tnsPredictionGain, INT tnsActive, FIXP_DBL *mdctSpectrum, INT *sfbMaxScaleSpec, FIXP_SGL *sfbtonality) {
  ref_pns::FDKaacEnc_FDKaacEnc_noiseDetection((PNS_CONFIG *)pnsConf, (PNS_DATA *)pnsData, sfbActive, sfbOffset, tnsOrder, tnsPredictionGain, tnsActive, mdctSpectrum, sfbMaxScaleSpec, sfbtonality);
}
void cref_aacenc_pns_CalcNoiseNrgs(INT sfbActive, INT *pnsFlag, FIXP_DBL *sfbEnergyLdData, INT *noiseNrg) {
  ref_pns::FDKaacEnc_CalcNoiseNrgs(sfbActive, pnsFlag, sfbEnergyLdData, noiseNrg);
}
}
