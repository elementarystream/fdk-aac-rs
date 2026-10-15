// Realistic input data for the differential tests of the quantization and noiseless coding
// modules (line_pe, sf_estim, quantize, bit_cnt, dyn_bits): the scalefactor band layout of real
// configurations, band energies and grouped short block spectra, computed with the reference's
// own psychoacoustic helpers.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_sf_estim_data (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/psy_configuration.h"
#include "libAACenc/src/psy_data.h"
#include "libAACenc/src/band_nrg.h"
#include "libAACenc/src/grp_data.h"
#include "libAACenc/src/sf_estim.h"
#include "fixpoint_math.h"

extern "C" {
/* FDKaacEnc_InitPsyConfiguration (LC filterbank, no IS/MS): sfbOffset[MAX_SFB + 1],
   sfbMinSnrLdData[MAX_SFB], sfbCnt, sfbActive. Returns the error code. */
INT cref_sf_estim_data_psy_conf(INT bitrate, INT samplerate, INT bandwidth, INT blocktype, INT granuleLength, INT *sfbOffset, FIXP_DBL *sfbMinSnrLdData, INT *sfbCnt, INT *sfbActive) {
  PSY_CONFIGURATION conf;
  AAC_ENCODER_ERROR err = FDKaacEnc_InitPsyConfiguration(
      bitrate, samplerate, bandwidth, blocktype, granuleLength, 0, 0, &conf, FB_LC);
  if (err != AAC_ENC_OK) return (INT)err;
  FDKmemcpy(sfbOffset, conf.sfbOffset, sizeof(conf.sfbOffset));
  FDKmemcpy(sfbMinSnrLdData, conf.sfbMinSnrLdData, sizeof(conf.sfbMinSnrLdData));
  *sfbCnt = conf.sfbCnt;
  *sfbActive = conf.sfbActive;
  return 0;
}

/* Long block: band energies (FDKaacEnc_CalcSfbMaxScaleSpec + FDKaacEnc_CalcBandEnergyOptimLong)
   of numBands bands; thresholds and spread energies are fMult(energy, ratio) and
   fMult(energy, sprRatio); threshold ld data through LdDataVector. */
void cref_sf_estim_data_long(const FIXP_DBL *spec, const INT *sfbOffset, INT numBands, const FIXP_DBL *ratio, const FIXP_DBL *sprRatio, FIXP_DBL *energy, FIXP_DBL *energyLd, FIXP_DBL *thr, FIXP_DBL *thrLd, FIXP_DBL *sprEn) {
  INT maxScale[MAX_GROUPED_SFB];
  FDKaacEnc_CalcSfbMaxScaleSpec(spec, sfbOffset, maxScale, numBands);
  FDKaacEnc_CalcBandEnergyOptimLong(spec, maxScale, sfbOffset, numBands, energy, energyLd);
  for (int i = 0; i < numBands; i++) {
    thr[i] = fMult(energy[i], ratio[i]);
    sprEn[i] = fMult(energy[i], sprRatio[i]);
  }
  LdDataVector(thr, thrLd, numBands);
}

/* Short blocks: per window band energies (FDKaacEnc_CalcBandEnergyOptimShort), thresholds and
   spread energies as in the long case (ratio[w * 15 + sfb]), then FDKaacEnc_groupShortData
   (regroups the spectrum in place) and the ld data of psy_main. Outputs have MAX_GROUPED_SFB
   entries (groupedSfbOffset MAX_GROUPED_SFB + 1); entries in the gaps of the groups are 0. */
void cref_sf_estim_data_short(FIXP_DBL *spec, const INT *sfbOffset, INT sfbCnt, INT sfbActive, const FIXP_DBL *sfbMinSnrLdData, INT noOfGroups, const INT *groupLen, const FIXP_DBL *ratio, const FIXP_DBL *sprRatio, INT *groupedSfbOffset, INT *maxSfbPerGroup, FIXP_DBL *groupedMinSnrLd, FIXP_DBL *energy, FIXP_DBL *energyLd, FIXP_DBL *thr, FIXP_DBL *thrLd, FIXP_DBL *sprEn) {
  SFB_THRESHOLD sfbThreshold;
  SFB_ENERGY sfbEnergy, sfbEnergyMS, sfbSpreadEnergy;
  FDKmemclear(&sfbThreshold, sizeof(sfbThreshold));
  FDKmemclear(&sfbEnergy, sizeof(sfbEnergy));
  FDKmemclear(&sfbEnergyMS, sizeof(sfbEnergyMS));
  FDKmemclear(&sfbSpreadEnergy, sizeof(sfbSpreadEnergy));
  for (int w = 0; w < TRANS_FAC; w++) {
    INT maxScale[MAX_SFB_SHORT];
    FDKaacEnc_CalcSfbMaxScaleSpec(spec + w * 128, sfbOffset, maxScale, sfbActive);
    FDKaacEnc_CalcBandEnergyOptimShort(spec + w * 128, maxScale, sfbOffset, sfbActive,
                                       sfbEnergy.Short[w]);
    for (int i = 0; i < sfbActive; i++) {
      sfbThreshold.Short[w][i] = fMult(sfbEnergy.Short[w][i], ratio[w * MAX_SFB_SHORT + i]);
      sfbSpreadEnergy.Short[w][i] = fMult(sfbEnergy.Short[w][i], sprRatio[w * MAX_SFB_SHORT + i]);
      sfbEnergyMS.Short[w][i] = sfbEnergy.Short[w][i];
    }
  }
  FDKmemclear(groupedMinSnrLd, MAX_GROUPED_SFB * sizeof(FIXP_DBL));
  FDKmemclear(groupedSfbOffset, (MAX_GROUPED_SFB + 1) * sizeof(INT));
  FDKaacEnc_groupShortData(spec, &sfbThreshold, &sfbEnergy, &sfbEnergyMS, &sfbSpreadEnergy,
                           sfbCnt, sfbActive, sfbOffset, sfbMinSnrLdData, groupedSfbOffset,
                           maxSfbPerGroup, groupedMinSnrLd, noOfGroups, groupLen, 1024);
  INT noSfb = noOfGroups * sfbCnt;
  FDKmemclear(energyLd, MAX_GROUPED_SFB * sizeof(FIXP_DBL));
  FDKmemclear(thrLd, MAX_GROUPED_SFB * sizeof(FIXP_DBL));
  for (int i = 0; i < MAX_GROUPED_SFB; i++) {
    energy[i] = sfbEnergy.Long[i];
    thr[i] = sfbThreshold.Long[i];
    sprEn[i] = sfbSpreadEnergy.Long[i];
  }
  for (int sfbGrp = 0; sfbGrp < noSfb; sfbGrp += sfbCnt) {
    LdDataVector(&sfbEnergy.Long[sfbGrp], &energyLd[sfbGrp], sfbActive);
    LdDataVector(&sfbThreshold.Long[sfbGrp], &thrLd[sfbGrp], sfbActive);
    for (int sfb = 0; sfb < sfbActive; sfb++) {
      thrLd[sfbGrp + sfb] = fixMax(thrLd[sfbGrp + sfb], FL2FXCONST_DBL(-0.515625f));
    }
  }
}

/* sfbFormFactorLdData[MAX_GROUPED_SFB] of a spectrum through FDKaacEnc_CalcFormFactor (entries
   it does not write are left unchanged). */
void cref_sf_estim_data_form_factor(FIXP_DBL *spec, const INT *sfbOffsets, INT sfbCnt, INT sfbPerGroup, INT maxSfbPerGroup, FIXP_DBL *formFactorLd) {
  PSY_OUT_CHANNEL *psy = (PSY_OUT_CHANNEL *)FDKcalloc(1, sizeof(PSY_OUT_CHANNEL));
  QC_OUT_CHANNEL *qc = (QC_OUT_CHANNEL *)FDKcalloc(1, sizeof(QC_OUT_CHANNEL));
  psy->sfbCnt = sfbCnt;
  psy->sfbPerGroup = sfbPerGroup;
  psy->maxSfbPerGroup = maxSfbPerGroup;
  FDKmemcpy(psy->sfbOffsets, sfbOffsets, (MAX_GROUPED_SFB + 1) * sizeof(INT));
  psy->mdctSpectrum = spec;
  FDKmemcpy(qc->sfbFormFactorLdData, formFactorLd, MAX_GROUPED_SFB * sizeof(FIXP_DBL));
  QC_OUT_CHANNEL *qcs[2] = {qc, NULL};
  PSY_OUT_CHANNEL *psys[2] = {psy, NULL};
  FDKaacEnc_CalcFormFactor(qcs, psys, 1);
  FDKmemcpy(formFactorLd, qc->sfbFormFactorLdData, MAX_GROUPED_SFB * sizeof(FIXP_DBL));
  FDKfree(psy);
  FDKfree(qc);
}
}
