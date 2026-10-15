// Shims for the differential tests of fdk_aac_rs_aacenc::band_nrg.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_band_nrg (see build.rs).
//
// cref_band_nrg_psy_conf() gives the tests of this group realistic psychoacoustic configurations
// (scalefactor band offsets, spreading slopes, thresholds) from the reference.

#include "cref_common.h"
#include "libAACenc/src/band_nrg.h"
#include "libAACenc/src/psy_configuration.h"

extern "C" {
void cref_band_nrg_CalcSfbMaxScaleSpec(const FIXP_DBL *mdctSpectrum, const INT *bandOffset, INT *sfbMaxScaleSpec, INT numBands) {
  FDKaacEnc_CalcSfbMaxScaleSpec(mdctSpectrum, bandOffset, sfbMaxScaleSpec, numBands);
}
FIXP_DBL cref_band_nrg_CheckBandEnergyOptim(const FIXP_DBL *mdctSpectrum, const INT *sfbMaxScaleSpec, const INT *bandOffset, INT numBands, FIXP_DBL *bandEnergy, FIXP_DBL *bandEnergyLdData, INT minSpecShift) {
  return FDKaacEnc_CheckBandEnergyOptim(mdctSpectrum, sfbMaxScaleSpec, bandOffset, numBands,
                                        bandEnergy, bandEnergyLdData, minSpecShift);
}
INT cref_band_nrg_CalcBandEnergyOptimLong(const FIXP_DBL *mdctSpectrum, INT *sfbMaxScaleSpec, const INT *bandOffset, INT numBands, FIXP_DBL *bandEnergy, FIXP_DBL *bandEnergyLdData) {
  return FDKaacEnc_CalcBandEnergyOptimLong(mdctSpectrum, sfbMaxScaleSpec, bandOffset, numBands,
                                           bandEnergy, bandEnergyLdData);
}
void cref_band_nrg_CalcBandEnergyOptimShort(const FIXP_DBL *mdctSpectrum, INT *sfbMaxScaleSpec, const INT *bandOffset, INT numBands, FIXP_DBL *bandEnergy) {
  FDKaacEnc_CalcBandEnergyOptimShort(mdctSpectrum, sfbMaxScaleSpec, bandOffset, numBands,
                                     bandEnergy);
}
void cref_band_nrg_CalcBandNrgMSOpt(const FIXP_DBL *mdctSpectrumLeft, const FIXP_DBL *mdctSpectrumRight, INT *sfbMaxScaleSpecLeft, INT *sfbMaxScaleSpecRight, const INT *bandOffset, INT numBands, FIXP_DBL *bandEnergyMid, FIXP_DBL *bandEnergySide, INT calcLdData, FIXP_DBL *bandEnergyMidLdData, FIXP_DBL *bandEnergySideLdData) {
  FDKaacEnc_CalcBandNrgMSOpt(mdctSpectrumLeft, mdctSpectrumRight, sfbMaxScaleSpecLeft,
                             sfbMaxScaleSpecRight, bandOffset, numBands, bandEnergyMid,
                             bandEnergySide, calcLdData, bandEnergyMidLdData,
                             bandEnergySideLdData);
}
/* FDKaacEnc_InitPsyConfiguration() into `out` (a PSY_CONFIGURATION); returns the error code. */
INT cref_band_nrg_psy_conf(INT bitrate, INT samplerate, INT bandwidth, INT blocktype, INT granuleLength, INT useIS, INT useMS, INT filterbank, void *out) {
  return (INT)FDKaacEnc_InitPsyConfiguration(bitrate, samplerate, bandwidth, blocktype,
                                             granuleLength, useIS, useMS,
                                             (PSY_CONFIGURATION *)out, (FB_TYPE)filterbank);
}
INT cref_band_nrg_sizeof_psy_conf() { return (INT)sizeof(PSY_CONFIGURATION); }
}
