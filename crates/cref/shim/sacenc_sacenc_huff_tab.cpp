// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_huff_tab.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_huff_tab (see build.rs).

#include "cref_common.h"
#include "libSACenc/src/sacenc_huff_tab.h"

extern "C" {
/* Copies the tables of the library (as their ULONG words) and returns their sizes in words. */
INT cref_sacenc_huff_tab_cld(UINT *out) {
  FDKmemcpy(out, &fdk_sacenc_huffCLDTab, sizeof(fdk_sacenc_huffCLDTab));
  return sizeof(fdk_sacenc_huffCLDTab) / sizeof(ULONG);
}
INT cref_sacenc_huff_tab_icc(UINT *out) {
  FDKmemcpy(out, &fdk_sacenc_huffICCTab, sizeof(fdk_sacenc_huffICCTab));
  return sizeof(fdk_sacenc_huffICCTab) / sizeof(ULONG);
}
INT cref_sacenc_huff_tab_part0(UINT *out) {
  FDKmemcpy(out, &fdk_sacenc_huffPart0Tab, sizeof(fdk_sacenc_huffPart0Tab));
  return sizeof(fdk_sacenc_huffPart0Tab) / sizeof(ULONG);
}
/* HUFF_VALUE and HUFF_LENGTH of a packed entry. */
UINT cref_sacenc_huff_tab_value(UINT packed) {
  HUFF_ENTRY e = {packed};
  return HUFF_VALUE(e);
}
UINT cref_sacenc_huff_tab_length(UINT packed) {
  HUFF_ENTRY e = {packed};
  return HUFF_LENGTH(e);
}
/* Offsets of fields in the structures, to check the layout. */
INT cref_sacenc_huff_tab_offsets(INT *out) {
  out[0] = offsetof(HUFF_CLD_TABLE, h2D) / 4;
  out[1] = offsetof(HUFF_CLD_TAB_2D, lav5) / 4;
  out[2] = offsetof(HUFF_CLD_TAB_2D, lav7) / 4;
  out[3] = offsetof(HUFF_CLD_TAB_2D, lav9) / 4;
  out[4] = sizeof(HUFF_CLD_TAB_2D) / 4;
  out[5] = offsetof(HUFF_ICC_TABLE, h2D) / 4;
  out[6] = offsetof(HUFF_ICC_TAB_2D, lav3) / 4;
  out[7] = offsetof(HUFF_ICC_TAB_2D, lav5) / 4;
  out[8] = offsetof(HUFF_ICC_TAB_2D, lav7) / 4;
  out[9] = sizeof(HUFF_ICC_TAB_2D) / 4;
  out[10] = offsetof(HUFF_PT0_TABLE, icc) / 4;
  out[11] = sizeof(HUFF_RES_TABLE) / 4;
  out[12] = sizeof(LAV6_2D) / 4;
  out[13] = sizeof(LAV12_2D) / 4;
  return 14;
}
}
