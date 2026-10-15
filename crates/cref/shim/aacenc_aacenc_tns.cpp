// Shims for the differential tests of fdk_aac_rs_aacenc::aacenc_tns (and tns_func).
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_aacenc_tns (see build.rs).
//
// The public functions are called in the library; the file-static functions and tables of
// aacenc_tns.cpp are reached by including the source file into a namespace (PORTING.md,
// "Differential tests"). Structures are passed as their memory images (void*).

#include "cref_common.h"
#include "libAACenc/src/aacenc_tns.h"
#include "libAACenc/src/psy_const.h"
#include "libAACenc/src/psy_configuration.h"
#include "libAACenc/src/tns_func.h"
#include "libAACenc/src/aacEnc_rom.h"
#include "FDK_lpc.h"

namespace ref_tns {
#include "libAACenc/src/aacenc_tns.cpp"
}

struct TnsTable {
  const char *name;
  const void *ptr;
  INT bytes;
};

#define T(x) {#x, (const void *)&ref_tns::x, (INT)sizeof(ref_tns::x)}

static const TnsTable tns_tables[] = {
    T(acfWindowLong),     T(acfWindowShort),    T(tnsInfoTab),        T(tnsMaxBandsTab1024),
    T(tnsMaxBandsTab120), T(tnsMaxBandsTab128), T(tnsMaxBandsTab240), T(tnsMaxBandsTab256),
    T(tnsMaxBandsTab480), T(tnsMaxBandsTab512),
};

extern "C" {
/* Number of tables. */
INT cref_aacenc_tns_table_count() { return (INT)(sizeof(tns_tables) / sizeof(tns_tables[0])); }
/* Name, memory and size in bytes of table i. */
const char *cref_aacenc_tns_table(INT i, const void **ptr, INT *bytes) {
  *ptr = tns_tables[i].ptr;
  *bytes = tns_tables[i].bytes;
  return tns_tables[i].name;
}

/* --- library functions --- */
INT cref_aacenc_tns_FreqToBandWidthRounding(INT freq, INT fs, INT numOfBands, const INT *bandStartOffset) {
  return FDKaacEnc_FreqToBandWidthRounding(freq, fs, numOfBands, bandStartOffset);
}
INT cref_aacenc_tns_InitTnsConfiguration(INT bitRate, INT sampleRate, INT channels, INT blockType, INT granuleLength, INT isLowDelay, INT ldSbrPresent, void *tC, void *pC, INT active, INT useTnsPeak) {
  return (INT)FDKaacEnc_InitTnsConfiguration(bitRate, sampleRate, channels, blockType, granuleLength, isLowDelay, ldSbrPresent, (TNS_CONFIG *)tC, (PSY_CONFIGURATION *)pC, active, useTnsPeak);
}
INT cref_aacenc_tns_TnsDetect(void *tnsData, const void *tC, void *tnsInfo, INT sfbCnt, const FIXP_DBL *spectrum, INT subBlockNumber, INT blockType) {
  return FDKaacEnc_TnsDetect((TNS_DATA *)tnsData, (const TNS_CONFIG *)tC, (TNS_INFO *)tnsInfo, sfbCnt, spectrum, subBlockNumber, blockType);
}
void cref_aacenc_tns_TnsSync(void *tnsDataDest, const void *tnsDataSrc, void *tnsInfoDest, void *tnsInfoSrc, INT blockTypeDest, INT blockTypeSrc, const void *tC) {
  FDKaacEnc_TnsSync((TNS_DATA *)tnsDataDest, (const TNS_DATA *)tnsDataSrc, (TNS_INFO *)tnsInfoDest, (TNS_INFO *)tnsInfoSrc, blockTypeDest, blockTypeSrc, (const TNS_CONFIG *)tC);
}
INT cref_aacenc_tns_TnsEncode(void *tnsInfo, void *tnsData, INT numOfSfb, const void *tC, INT lowPassLine, FIXP_DBL *spectrum, INT subBlockNumber, INT blockType) {
  return FDKaacEnc_TnsEncode((TNS_INFO *)tnsInfo, (TNS_DATA *)tnsData, numOfSfb, (const TNS_CONFIG *)tC, lowPassLine, spectrum, subBlockNumber, blockType);
}

/* --- file-static functions (namespace copy) --- */
/* Copies the selected TNS_PARAMETER_TABULATED to out; returns 0 for NULL. */
INT cref_aacenc_tns_GetTnsParam(INT bitRate, INT channels, INT sbrLd, void *out) {
  const TNS_PARAMETER_TABULATED *p = ref_tns::FDKaacEnc_GetTnsParam(bitRate, channels, sbrLd);
  if (p == NULL) return 0;
  FDKmemcpy(out, p, sizeof(*p));
  return 1;
}
INT cref_aacenc_tns_getTnsMaxBands(INT sampleRate, INT granuleLength, INT isShortBlock) {
  return ref_tns::getTnsMaxBands(sampleRate, granuleLength, isShortBlock);
}
INT cref_aacenc_tns_ScaleUpSpectrum(FIXP_DBL *dest, const FIXP_DBL *src, INT startLine, INT stopLine) {
  return ref_tns::FDKaacEnc_ScaleUpSpectrum(dest, src, startLine, stopLine);
}
FIXP_DBL cref_aacenc_tns_CalcAutoCorrValue(const FIXP_DBL *spectrum, INT startLine, INT stopLine, INT lag, INT scale) {
  return ref_tns::FDKaacEnc_CalcAutoCorrValue(spectrum, startLine, stopLine, lag, scale);
}
FIXP_DBL cref_aacenc_tns_AutoCorrNormFac(FIXP_DBL value, INT scale, INT *sc) {
  return ref_tns::FDKaacEnc_AutoCorrNormFac(value, scale, sc);
}
/* acfWindow: FIXP_DBL[MAX_NUM_OF_FILTERS][TNS_MAX_ORDER + 3 + 1] */
void cref_aacenc_tns_MergedAutoCorrelation(const FIXP_DBL *spectrum, INT isLowDelay, const FIXP_DBL *acfWindow, const INT *lpcStartLine, INT lpcStopLine, INT maxOrder, const INT *acfSplit, FIXP_DBL *rxx1, FIXP_DBL *rxx2) {
  ref_tns::FDKaacEnc_MergedAutoCorrelation(spectrum, isLowDelay, (const FIXP_DBL(*)[TNS_MAX_ORDER + 3 + 1])acfWindow, lpcStartLine, lpcStopLine, maxOrder, acfSplit, rxx1, rxx2);
}
void cref_aacenc_tns_CalcGaussWindow(FIXP_DBL *win, INT winSize, INT samplingRate, INT transformResolution, FIXP_DBL timeResolution, INT timeResolution_e) {
  ref_tns::FDKaacEnc_CalcGaussWindow(win, winSize, samplingRate, transformResolution, timeResolution, timeResolution_e);
}
INT cref_aacenc_tns_Search3(SHORT parcor) { return ref_tns::FDKaacEnc_Search3(parcor); }
INT cref_aacenc_tns_Search4(SHORT parcor) { return ref_tns::FDKaacEnc_Search4(parcor); }
void cref_aacenc_tns_Parcor2Index(const SHORT *parcor, INT *index, INT order, INT bitsPerCoeff) {
  ref_tns::FDKaacEnc_Parcor2Index(parcor, index, order, bitsPerCoeff);
}
void cref_aacenc_tns_Index2Parcor(const INT *index, SHORT *parcor, INT order, INT bitsPerCoeff) {
  ref_tns::FDKaacEnc_Index2Parcor(index, parcor, order, bitsPerCoeff);
}
}
