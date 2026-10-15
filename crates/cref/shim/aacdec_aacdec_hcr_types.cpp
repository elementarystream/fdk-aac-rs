// Shims for the differential tests of fdk_aac_rs_aacdec::aacdec_hcr_types.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_aacdec_hcr_types (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/aacdec_hcr_types.h"

#define AL1(s, f) out[n++] = (INT)(sizeof(((s *)0)->f) / sizeof(((s *)0)->f[0]))

extern "C" {
/* The constants of aacdec_hcr_types.h, in the order of the Rust test (UINT values as INT). */
INT cref_hcrt_constants(INT *out) {
  INT n = 0;
  out[n++] = LINES_PER_UNIT;
  out[n++] = MAX_SFB_HCR;
  out[n++] = NUMBER_OF_UNIT_GROUPS;
  out[n++] = LINES_PER_UNIT_GROUP;
  out[n++] = FROM_LEFT_TO_RIGHT;
  out[n++] = FROM_RIGHT_TO_LEFT;
  out[n++] = MAX_CB_PAIRS;
  out[n++] = MAX_HCR_SETS;
  out[n++] = ESCAPE_VALUE;
  out[n++] = POSITION_OF_FLAG_A;
  out[n++] = POSITION_OF_FLAG_B;
  out[n++] = MAX_CB;
  out[n++] = MAX_CB_CHECK;
  out[n++] = NUMBER_OF_BIT_IN_WORD;
  out[n++] = THIRTYTWO_LOG_DIV_TWO_LOG;
  out[n++] = EIGHT_LOG_DIV_TWO_LOG;
  out[n++] = FOUR_LOG_DIV_TWO_LOG;
  out[n++] = CPE_TOP_LENGTH;
  out[n++] = SCE_TOP_LENGTH;
  out[n++] = LEN_OF_LONGEST_CW_TOP_LENGTH;
  out[n++] = Q_VALUE_INVALID;
  out[n++] = HCR_DIRAC;
  out[n++] = MASK_LEFT;
  out[n++] = MASK_RIGHT;
  out[n++] = CLR_BIT_10;
  out[n++] = TEST_BIT_10;
  out[n++] = LEFT_OFFSET;
  out[n++] = ERROR_LORSD;
  out[n++] = ERROR_LOLC;
  out[n++] = ERROR_PCW_BODY;
  out[n++] = ERROR_PCW_BODY_SIGN;
  out[n++] = ERROR_PCW_BODY_SIGN_ESC;
  out[n++] = ERROR_PCW_BODY_ONLY_TOO_LONG;
  out[n++] = ERROR_PCW_BODY_SIGN_TOO_LONG;
  out[n++] = ERROR_PCW_BODY_SIGN_ESC_TOO_LONG;
  out[n++] = MODULO_DIVISOR_HCR;
  out[n++] = HCR_ERROR_CONCEALMENT;
  out[n++] = (INT)SEGMENT_OVERRIDE_ERR_PCW_BODY;
  out[n++] = (INT)SEGMENT_OVERRIDE_ERR_PCW_BODY_SIGN;
  out[n++] = (INT)SEGMENT_OVERRIDE_ERR_PCW_BODY_SIGN_ESC;
  out[n++] = (INT)EXTENDED_SORTED_COUNTER_OVERFLOW;
  out[n++] = (INT)TOO_MANY_PCW_BODY_BITS_DECODED;
  out[n++] = (INT)TOO_MANY_PCW_BODY_SIGN_BITS_DECODED;
  out[n++] = (INT)TOO_MANY_PCW_BODY_SIGN_ESC_BITS_DECODED;
  out[n++] = (INT)STATE_ERROR_BODY_ONLY;
  out[n++] = (INT)STATE_ERROR_BODY_SIGN__BODY;
  out[n++] = (INT)STATE_ERROR_BODY_SIGN__SIGN;
  out[n++] = (INT)STATE_ERROR_BODY_SIGN_ESC__BODY;
  out[n++] = (INT)STATE_ERROR_BODY_SIGN_ESC__SIGN;
  out[n++] = (INT)STATE_ERROR_BODY_SIGN_ESC__ESC_PREFIX;
  out[n++] = (INT)STATE_ERROR_BODY_SIGN_ESC__ESC_WORD;
  out[n++] = (INT)HCR_SI_LENGTHS_FAILURE;
  out[n++] = (INT)NUM_SECT_OUT_OF_RANGE_SHORT_BLOCK;
  out[n++] = (INT)NUM_SECT_OUT_OF_RANGE_LONG_BLOCK;
  out[n++] = (INT)LINE_IN_SECT_OUT_OF_RANGE_SHORT_BLOCK;
  out[n++] = (INT)CB_OUT_OF_RANGE_SHORT_BLOCK;
  out[n++] = (INT)LINE_IN_SECT_OUT_OF_RANGE_LONG_BLOCK;
  out[n++] = (INT)CB_OUT_OF_RANGE_LONG_BLOCK;
  out[n++] = (INT)LAV_VIOLATION;
  out[n++] = (INT)BIT_IN_SEGMENTATION_ERROR;
  out[n++] = (INT)HCR_FATAL_PCW_ERROR_MASK;
  out[n++] = PCW_BODY;
  out[n++] = PCW_BODY_SIGN;
  out[n++] = PCW_BODY_SIGN_ESC;
  return n;
}

/* The number of elements of the array fields, in the order of the Rust test. */
INT cref_hcrt_array_lengths(INT *out) {
  INT n = 0;
  AL1(HCR_SEGMENT_INFO, pSegmentBitfield);
  AL1(HCR_SEGMENT_INFO, pCodewordBitfield);
  AL1(HCR_SEGMENT_INFO, pLeftStartOfSegment);
  AL1(HCR_SEGMENT_INFO, pRightStartOfSegment);
  AL1(HCR_SEGMENT_INFO, pRemainingBitsInSegment);
  AL1(HCR_SECTION_INFO, pNumCodewordInSection);
  AL1(HCR_SECTION_INFO, pNumSortedCodewordInSection);
  AL1(HCR_SECTION_INFO, pNumExtendedSortedCodewordInSection);
  AL1(HCR_SECTION_INFO, pNumExtendedSortedSectionsInSets);
  AL1(HCR_SECTION_INFO, pReorderOffset);
  AL1(HCR_SECTION_INFO, pSortedCodebook);
  AL1(HCR_SECTION_INFO, pExtendedSortedCodebook);
  AL1(HCR_SECTION_INFO, pMaxLenOfCbInExtSrtSec);
  AL1(HCR_SECTION_INFO, pCodebookSwitch);
  AL1(HCR_NON_PCW_SIDEINFO, iNode);
  AL1(HCR_NON_PCW_SIDEINFO, iResultPointer);
  AL1(HCR_NON_PCW_SIDEINFO, pEscapeSequenceInfo);
  AL1(HCR_NON_PCW_SIDEINFO, pCodebook);
  AL1(HCR_NON_PCW_SIDEINFO, pCntSign);
  AL1(HCR_NON_PCW_SIDEINFO, pSta);
  return n;
}
}
