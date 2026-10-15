// Shims for the differential tests of fdk_aac_rs_aacenc::ms_stereo.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_ms_stereo (see build.rs).
//
// The port's PsyData is PSY_DATA without its first member (the mdctSpectrum pointer), its
// QcOutChannel is QC_OUT_CHANNEL. The shim builds the PSY_DATA and PSY_OUT_CHANNEL structures of
// FDKaacEnc_psyMain(): mdctSpectrum, sfbEnergyLdData and sfbThresholdLdData point into the
// QC_OUT_CHANNELs.

#include "cref_common.h"
#include "libAACenc/src/ms_stereo.h"
#include "libAACenc/src/psy_data.h"
#include "libAACenc/src/qc_data.h"

#include <stddef.h>

static const size_t psyDataTail = offsetof(PSY_DATA, sfbThreshold);

extern "C" {
/* psyData0/1: the port's PsyData; qc0/1: QC_OUT_CHANNEL; isBook may be NULL. */
void cref_ms_stereo_MsStereoProcessing(void *psyData0, void *psyData1, void *qc0, void *qc1, const INT *isBook, INT *msDigest, INT *msMask, INT allowMS, INT sfbCnt, INT sfbPerGroup, INT maxSfbPerGroup, const INT *sfbOffset) {
  void *pd[2] = {psyData0, psyData1};
  QC_OUT_CHANNEL *qc[2] = {(QC_OUT_CHANNEL *)qc0, (QC_OUT_CHANNEL *)qc1};
  PSY_DATA *psyData[2];
  PSY_OUT_CHANNEL poc[2];
  PSY_OUT_CHANNEL *psyOutChannel[2];
  for (int ch = 0; ch < 2; ch++) {
    psyData[ch] = new PSY_DATA;
    memset(psyData[ch], 0, sizeof(PSY_DATA));
    memcpy((char *)psyData[ch] + psyDataTail, pd[ch], sizeof(PSY_DATA) - psyDataTail);
    psyData[ch]->mdctSpectrum = qc[ch]->mdctSpectrum;
    memset(&poc[ch], 0, sizeof(PSY_OUT_CHANNEL));
    poc[ch].mdctSpectrum = qc[ch]->mdctSpectrum;
    poc[ch].sfbEnergy = qc[ch]->sfbEnergy;
    poc[ch].sfbSpreadEnergy = qc[ch]->sfbSpreadEnergy;
    poc[ch].sfbThresholdLdData = qc[ch]->sfbThresholdLdData;
    poc[ch].sfbMinSnrLdData = qc[ch]->sfbMinSnrLdData;
    poc[ch].sfbEnergyLdData = qc[ch]->sfbEnergyLdData;
    psyOutChannel[ch] = &poc[ch];
  }
  FDKaacEnc_MsStereoProcessing(psyData, psyOutChannel, isBook, msDigest, msMask, allowMS, sfbCnt,
                               sfbPerGroup, maxSfbPerGroup, sfbOffset);
  for (int ch = 0; ch < 2; ch++) {
    memcpy(pd[ch], (char *)psyData[ch] + psyDataTail, sizeof(PSY_DATA) - psyDataTail);
    delete psyData[ch];
  }
}
}
