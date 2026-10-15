// Shims for the differential tests of fdk_aac_rs_aacenc::sf_estim.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_sf_estim (see build.rs).
//
// The port's PsyOutChannel is PSY_OUT_CHANNEL without the six pointers at its end (which point
// into QC_OUT_CHANNEL); the shims take the channels as the port's byte images, rebuild the C
// structs with the pointers set as FDKaacEnc_EncodeFrame() sets them, run the function and copy
// the structs back.

#include "cref_common.h"
#include "libAACenc/src/sf_estim.h"

#include <stddef.h>

static PSY_OUT_CHANNEL *make_psy(const void *prefix, INT prefixSize, QC_OUT_CHANNEL *qc) {
  PSY_OUT_CHANNEL *psy = (PSY_OUT_CHANNEL *)FDKcalloc(1, sizeof(PSY_OUT_CHANNEL));
  FDKmemcpy(psy, prefix, prefixSize);
  psy->mdctSpectrum = qc->mdctSpectrum;
  psy->sfbEnergy = qc->sfbEnergy;
  psy->sfbSpreadEnergy = qc->sfbSpreadEnergy;
  psy->sfbThresholdLdData = qc->sfbThresholdLdData;
  psy->sfbMinSnrLdData = qc->sfbMinSnrLdData;
  psy->sfbEnergyLdData = qc->sfbEnergyLdData;
  return psy;
}

extern "C" {
/* offsetof(PSY_OUT_CHANNEL, mdctSpectrum): the size of the port's PsyOutChannel. */
INT cref_sf_estim_psy_prefix_size() { return (INT)offsetof(PSY_OUT_CHANNEL, mdctSpectrum); }

/* FDKaacEnc_CalcFormFactor on nChannels (1 or 2) channels: qc is QC_OUT_CHANNEL[nChannels],
   psy the port's PsyOutChannel[nChannels] (prefixSize bytes each). */
void cref_sf_estim_CalcFormFactor(void *qc, void *psy, INT prefixSize, INT nChannels) {
  QC_OUT_CHANNEL *qcs[2] = {NULL, NULL};
  PSY_OUT_CHANNEL *psys[2] = {NULL, NULL};
  for (int ch = 0; ch < nChannels; ch++) {
    qcs[ch] = (QC_OUT_CHANNEL *)qc + ch;
    psys[ch] = make_psy((UCHAR *)psy + ch * prefixSize, prefixSize, qcs[ch]);
  }
  FDKaacEnc_CalcFormFactor(qcs, psys, nChannels);
  for (int ch = 0; ch < nChannels; ch++) {
    FDKmemcpy((UCHAR *)psy + ch * prefixSize, psys[ch], prefixSize);
    FDKfree(psys[ch]);
  }
}

/* FDKaacEnc_EstimateScaleFactors, channels as in cref_sf_estim_CalcFormFactor. */
void cref_sf_estim_EstimateScaleFactors(void *psy, void *qc, INT prefixSize, INT invQuant, INT dZoneQuantEnable, INT nChannels) {
  QC_OUT_CHANNEL *qcs[2] = {NULL, NULL};
  PSY_OUT_CHANNEL *psys[2] = {NULL, NULL};
  for (int ch = 0; ch < nChannels; ch++) {
    qcs[ch] = (QC_OUT_CHANNEL *)qc + ch;
    psys[ch] = make_psy((UCHAR *)psy + ch * prefixSize, prefixSize, qcs[ch]);
  }
  FDKaacEnc_EstimateScaleFactors(psys, qcs, invQuant, dZoneQuantEnable, nChannels);
  for (int ch = 0; ch < nChannels; ch++) {
    FDKmemcpy((UCHAR *)psy + ch * prefixSize, psys[ch], prefixSize);
    FDKfree(psys[ch]);
  }
}
}
