// Shims for the differential tests of fdk_aac_rs_aacenc::dyn_bits.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_dyn_bits (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/dyn_bits.h"
#include "libAACenc/src/bit_cnt.h"
#include "libAACenc/src/psy_const.h"

extern "C" {
/* FDKaacEnc_dynBitCount with a BITCNTR_STATE whose look-up tables are filled with a pseudo-random
   pattern derived from `poison` before the call (as stale contents of the shared dynamic RAM).
   sectionData is a SECTION_DATA; scalefac may be NULL. */
INT cref_dyn_bits_dynBitCount(UINT poison, const SHORT *quantSpectrum, const UINT *maxValueInSfb, const INT *scalefac, INT blockType, INT sfbCnt, INT maxSfbPerGroup, INT sfbPerGroup, const INT *sfbOffset, void *sectionData, const INT *noiseNrg, const INT *isBook, const INT *isScale, UINT syntaxFlags) {
  INT bitLookUp[MAX_SFB_LONG * (CODE_BOOK_ESC_NDX + 1)];
  INT mergeGainLookUp[MAX_SFB_LONG];
  UINT x = poison;
  for (int i = 0; i < MAX_SFB_LONG * (CODE_BOOK_ESC_NDX + 1); i++) {
    x = x * 1664525u + 1013904223u;
    bitLookUp[i] = (INT)(x >> 3);
  }
  for (int i = 0; i < MAX_SFB_LONG; i++) {
    x = x * 1664525u + 1013904223u;
    mergeGainLookUp[i] = (INT)x;
  }
  BITCNTR_STATE bc;
  bc.bitLookUp = bitLookUp;
  bc.mergeGainLookUp = mergeGainLookUp;
  return FDKaacEnc_dynBitCount(&bc, quantSpectrum, maxValueInSfb, scalefac, blockType, sfbCnt,
                               maxSfbPerGroup, sfbPerGroup, sfbOffset,
                               (SECTION_DATA *)sectionData, noiseNrg, isBook, isScale,
                               syntaxFlags);
}
}
