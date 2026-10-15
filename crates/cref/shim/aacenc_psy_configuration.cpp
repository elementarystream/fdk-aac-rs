// Shims for the differential tests of fdk_aac_rs_aacenc::psy_configuration.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_psy_configuration (see build.rs).
//
// FDKaacEnc_InitPsyConfiguration and FDKaacEnc_initSfbTable are called in the library; the
// file-static functions and tables of psy_configuration.cpp are reached by including the source
// file into a namespace (PORTING.md, "Differential tests"). PSY_CONFIGURATION is passed as its
// memory image (void*).

#include "cref_common.h"
#include "libAACenc/src/psy_configuration.h"
#include "libAACenc/src/adj_thr.h"
#include "libAACenc/src/aacEnc_rom.h"
#include "genericStds.h"
#include "FDK_trigFcts.h"

/* Defined in psy_configuration.cpp, declared in no header. */
AAC_ENCODER_ERROR FDKaacEnc_initSfbTable(const LONG sampleRate, const INT blockType, const INT granuleLength, INT *const sfbOffset, INT *const sfbCnt);

namespace ref_psyconf {
#include "libAACenc/src/psy_configuration.cpp"
}

extern "C" {
/* --- tables --- */
/* Memory of the SFB_PARAM_LONG table i (0..5: p_22050_long_512 .. p_44100_long_480). */
const void *cref_psy_configuration_param_long(INT i) {
  static const SFB_PARAM_LONG *const t[] = {
      &ref_psyconf::p_22050_long_512, &ref_psyconf::p_32000_long_512, &ref_psyconf::p_44100_long_512,
      &ref_psyconf::p_22050_long_480, &ref_psyconf::p_32000_long_480, &ref_psyconf::p_44100_long_480,
  };
  return t[i];
}
/* Number of entries of SFB_INFO_TAB table t (0: sfbInfoTab, 1: sfbInfoTabLD512, 2: sfbInfoTabLD480). */
INT cref_psy_configuration_info_count(INT t) {
  switch (t) {
    case 0: return (INT)(sizeof(ref_psyconf::sfbInfoTab) / sizeof(ref_psyconf::sfbInfoTab[0]));
    case 1: return (INT)(sizeof(ref_psyconf::sfbInfoTabLD512) / sizeof(ref_psyconf::sfbInfoTabLD512[0]));
    default: return (INT)(sizeof(ref_psyconf::sfbInfoTabLD480) / sizeof(ref_psyconf::sfbInfoTabLD480[0]));
  }
}
/* Entry i of SFB_INFO_TAB table t: returns the sample rate, the memory the pointers refer to
   (NULL stays NULL). */
INT cref_psy_configuration_info(INT t, INT i, const void **paramLong, const void **paramShort) {
  const ref_psyconf::SFB_INFO_TAB *e = t == 0   ? &ref_psyconf::sfbInfoTab[i]
                                       : t == 1 ? &ref_psyconf::sfbInfoTabLD512[i]
                                                : &ref_psyconf::sfbInfoTabLD480[i];
  *paramLong = e->paramLong;
  *paramShort = e->paramShort;
  return e->sampleRate;
}

/* --- library functions --- */
INT cref_psy_configuration_InitPsyConfiguration(INT bitrate, INT samplerate, INT bandwidth, INT blocktype, INT granuleLength, INT useIS, INT useMS, void *psyConf, INT filterbank) {
  return (INT)FDKaacEnc_InitPsyConfiguration(bitrate, samplerate, bandwidth, blocktype, granuleLength, useIS, useMS, (PSY_CONFIGURATION *)psyConf, (FB_TYPE)filterbank);
}
INT cref_psy_configuration_initSfbTable(INT sampleRate, INT blockType, INT granuleLength, INT *sfbOffset, INT *sfbCnt) {
  return (INT)FDKaacEnc_initSfbTable(sampleRate, blockType, granuleLength, sfbOffset, sfbCnt);
}

/* --- file-static functions (namespace copy) --- */
FIXP_DBL cref_psy_configuration_BarcLineValue(INT noOfLines, INT fftLine, INT samplingFreq) {
  return ref_psyconf::FDKaacEnc_BarcLineValue(noOfLines, fftLine, samplingFreq);
}
void cref_psy_configuration_InitMinPCMResolution(INT numPb, INT *pbOffset, FIXP_DBL *sfbPCMquantThreshold) {
  ref_psyconf::FDKaacEnc_InitMinPCMResolution(numPb, pbOffset, sfbPCMquantThreshold);
}
FIXP_DBL cref_psy_configuration_getMaskFactor(FIXP_DBL dbVal_fix, INT dbVal_e, FIXP_DBL ten_fix, INT ten_e) {
  return ref_psyconf::getMaskFactor(dbVal_fix, dbVal_e, ten_fix, ten_e);
}
void cref_psy_configuration_initSpreading(INT numPb, FIXP_DBL *pbBarcValue, FIXP_DBL *pbMaskLoFactor, FIXP_DBL *pbMaskHiFactor, FIXP_DBL *pbMaskLoFactorSprEn, FIXP_DBL *pbMaskHiFactorSprEn, INT bitrate, INT blockType) {
  ref_psyconf::FDKaacEnc_initSpreading(numPb, pbBarcValue, pbMaskLoFactor, pbMaskHiFactor, pbMaskLoFactorSprEn, pbMaskHiFactorSprEn, bitrate, blockType);
}
void cref_psy_configuration_initBarcValues(INT numPb, INT *pbOffset, INT numLines, INT samplingFrequency, FIXP_DBL *pbBval) {
  ref_psyconf::FDKaacEnc_initBarcValues(numPb, pbOffset, numLines, samplingFrequency, pbBval);
}
void cref_psy_configuration_initMinSnr(INT bitrate, INT samplerate, INT numLines, const INT *sfbOffset, INT sfbActive, INT blockType, FIXP_DBL *sfbMinSnrLdData) {
  ref_psyconf::FDKaacEnc_initMinSnr(bitrate, samplerate, numLines, sfbOffset, sfbActive, blockType, sfbMinSnrLdData);
}
}
