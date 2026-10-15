// Shims for the differential tests of fdk_aac_rs_drc::drc_dec_rom.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::drc_drc_dec_rom (see build.rs).

#include "cref_common.h"
#include "drcDec_types.h"
#include "drcDec_rom.h"

extern "C" {
/* Copies the memory image of table `t` of drcDec_rom.cpp (in the order of the list in
   tests/drc/drc_dec_rom.rs) to `out` (at least `max` bytes). Returns its size in bytes, or -1. */
INT cref_drc_rom_table(INT t, UCHAR *out, INT max) {
  const void *p;
  INT n;
  switch (t) {
    case 0: p = deltaGain_codingProfile_0_1_huffman; n = sizeof(deltaGain_codingProfile_0_1_huffman); break;
    case 1: p = deltaGain_codingProfile_2_huffman; n = sizeof(deltaGain_codingProfile_2_huffman); break;
    case 2: p = slopeSteepness; n = 15 * sizeof(FIXP_SGL); break;
    case 3: p = slopeSteepness_huffman; n = sizeof(slopeSteepness_huffman); break;
    case 4: p = downmixCoeff; n = 16 * sizeof(FIXP_DBL); break;
    case 5: p = downmixCoeffV1; n = 32 * sizeof(FIXP_DBL); break;
    case 6: p = cicpDrcCharSigmoidLeft; n = 6 * sizeof(CUSTOM_DRC_CHAR_SIGMOID); break;
    case 7: p = cicpDrcCharSigmoidRight; n = 6 * sizeof(CUSTOM_DRC_CHAR_SIGMOID); break;
    case 8: p = cicpDrcCharNodesLeft; n = 5 * sizeof(CUSTOM_DRC_CHAR_NODES); break;
    case 9: p = cicpDrcCharNodesRight; n = 5 * sizeof(CUSTOM_DRC_CHAR_NODES); break;
    default: return -1;
  }
  if (n > max) return -1;
  FDKmemcpy(out, p, n);
  return n;
}
}
