// Shims for the differential tests of fdk_aac_rs_aacenc::intensity.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_intensity (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/intensity.h"
#include "libAACenc/src/interface.h"
#include "libAACenc/src/psy_configuration.h"
#include "libAACenc/src/psy_const.h"
#include "libAACenc/src/qc_main.h"
#include "libAACenc/src/bit_cnt.h"
#include "libAACenc/src/band_nrg.h"

/* A private copy of intensity.cpp for the file-static functions. */
namespace ref_is {
#include "libAACenc/src/intensity.cpp"
}

extern "C" {
/* calcSfbMaxScale() */
INT cref_intensity_calcSfbMaxScale(const FIXP_DBL *mdctSpectrum, INT l1, INT l2) {
  return ref_is::calcSfbMaxScale(mdctSpectrum, l1, l2);
}
/* FDKaacEnc_initIsParams() into out (INTENSITY_PARAMETERS, C layout); returns its size. */
INT cref_intensity_initIsParams(void *out) {
  ref_is::FDKaacEnc_initIsParams((ref_is::INTENSITY_PARAMETERS *)out);
  return (INT)sizeof(ref_is::INTENSITY_PARAMETERS);
}
/* FDKaacEnc_prepareIntensityDecision() with isParams (C layout). */
void cref_intensity_prepareIntensityDecision(const FIXP_DBL *sfbEnergyLeft, const FIXP_DBL *sfbEnergyRight, const FIXP_DBL *sfbEnergyLdDataLeft, const FIXP_DBL *sfbEnergyLdDataRight, const FIXP_DBL *mdctSpectrumLeft, const FIXP_DBL *mdctSpectrumRight, const void *isParams, FIXP_DBL *hrrErr, INT *isMask, FIXP_DBL *realScale, FIXP_DBL *normSfbLoudness, INT sfbCnt, INT sfbPerGroup, INT maxSfbPerGroup, const INT *sfbOffset) {
  ref_is::FDKaacEnc_prepareIntensityDecision(
      sfbEnergyLeft, sfbEnergyRight, sfbEnergyLdDataLeft, sfbEnergyLdDataRight, mdctSpectrumLeft,
      mdctSpectrumRight, (const ref_is::INTENSITY_PARAMETERS *)isParams, hrrErr, isMask, realScale,
      normSfbLoudness, sfbCnt, sfbPerGroup, maxSfbPerGroup, sfbOffset);
}
/* FDKaacEnc_finalizeIntensityDecision() with isParams (C layout). */
void cref_intensity_finalizeIntensityDecision(const FIXP_DBL *hrrErr, INT *isMask, const FIXP_DBL *realIsScale, const FIXP_DBL *normSfbLoudness, const void *isParams, INT sfbCnt, INT sfbPerGroup, INT maxSfbPerGroup) {
  ref_is::FDKaacEnc_finalizeIntensityDecision(hrrErr, isMask, realIsScale, normSfbLoudness,
                                              (const ref_is::INTENSITY_PARAMETERS *)isParams,
                                              sfbCnt, sfbPerGroup, maxSfbPerGroup);
}
/* FDKaacEnc_IntensityStereoProcessing() of the library; pnsData0/pnsData1 are PNS_DATA (C
   layout) or NULL. */
void cref_intensity_IntensityStereoProcessing(FIXP_DBL *sfbEnergyLeft, FIXP_DBL *sfbEnergyRight, FIXP_DBL *mdctSpectrumLeft, FIXP_DBL *mdctSpectrumRight, FIXP_DBL *sfbThresholdLeft, FIXP_DBL *sfbThresholdRight, FIXP_DBL *sfbThresholdLdDataRight, FIXP_DBL *sfbSpreadEnLeft, FIXP_DBL *sfbSpreadEnRight, FIXP_DBL *sfbEnergyLdDataLeft, FIXP_DBL *sfbEnergyLdDataRight, INT *msDigest, INT *msMask, INT sfbCnt, INT sfbPerGroup, INT maxSfbPerGroup, const INT *sfbOffset, INT allowIS, INT *isBook, INT *isScale, void *pnsData0, void *pnsData1) {
  PNS_DATA *pnsData[2] = {(PNS_DATA *)pnsData0, (PNS_DATA *)pnsData1};
  FDKaacEnc_IntensityStereoProcessing(
      sfbEnergyLeft, sfbEnergyRight, mdctSpectrumLeft, mdctSpectrumRight, sfbThresholdLeft,
      sfbThresholdRight, sfbThresholdLdDataRight, sfbSpreadEnLeft, sfbSpreadEnRight,
      sfbEnergyLdDataLeft, sfbEnergyLdDataRight, msDigest, msMask, sfbCnt, sfbPerGroup,
      maxSfbPerGroup, sfbOffset, allowIS, isBook, isScale, pnsData);
}
/* FDKaacEnc_CalcBandEnergyOptimLong() of the library (realistic test data). */
INT cref_intensity_CalcBandEnergyOptimLong(const FIXP_DBL *mdctSpectrum, INT *sfbMaxScaleSpec, const INT *bandOffset, INT numBands, FIXP_DBL *bandEnergy, FIXP_DBL *bandEnergyLdData) {
  return FDKaacEnc_CalcBandEnergyOptimLong(mdctSpectrum, sfbMaxScaleSpec, bandOffset, numBands,
                                           bandEnergy, bandEnergyLdData);
}
/* CalcLdData() (realistic test data). */
FIXP_DBL cref_intensity_CalcLdData(FIXP_DBL x) { return CalcLdData(x); }
}
