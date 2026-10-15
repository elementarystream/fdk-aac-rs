// Shims for the differential tests of fdk_aac_rs_aacenc::aac_enc_rom.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_aac_enc_rom (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/aacEnc_rom.h"

struct RomEntry {
  const char *name;
  const void *ptr;
  INT bytes;
};

#define T(x) {#x, (const void *)&x, (INT)sizeof(x)}

static const RomEntry rom_tables[] = {
    T(FDKaacEnc_huff_ltab1_2), T(FDKaacEnc_huff_ltab3_4), T(FDKaacEnc_huff_ltab5_6),
    T(FDKaacEnc_huff_ltab7_8), T(FDKaacEnc_huff_ltab9_10), T(FDKaacEnc_huff_ltab11),
    T(FDKaacEnc_huff_ltabscf), T(FDKaacEnc_huff_ctab1), T(FDKaacEnc_huff_ctab2),
    T(FDKaacEnc_huff_ctab3), T(FDKaacEnc_huff_ctab4), T(FDKaacEnc_huff_ctab5),
    T(FDKaacEnc_huff_ctab6), T(FDKaacEnc_huff_ctab7), T(FDKaacEnc_huff_ctab8),
    T(FDKaacEnc_huff_ctab9), T(FDKaacEnc_huff_ctab10), T(FDKaacEnc_huff_ctab11),
    T(FDKaacEnc_huff_ctabscf), T(FDKaacEnc_mTab_3_4), T(FDKaacEnc_quantTableQ),
    T(FDKaacEnc_quantTableE), T(FDKaacEnc_mTab_4_3Elc), T(FDKaacEnc_specExpMantTableCombElc),
    T(FDKaacEnc_specExpTableComb),
    /* declared without size in the header: 52 and 16 entries in aacEnc_rom.cpp */
    {"FDKaacEnc_sideInfoTabLong", (const void *)FDKaacEnc_sideInfoTabLong, 52 * 2},
    {"FDKaacEnc_sideInfoTabShort", (const void *)FDKaacEnc_sideInfoTabShort, 16 * 2},
    T(p_FDKaacEnc_8000_long_1024), T(p_FDKaacEnc_8000_short_128),
    T(p_FDKaacEnc_11025_long_1024), T(p_FDKaacEnc_11025_short_128),
    T(p_FDKaacEnc_12000_long_1024), T(p_FDKaacEnc_12000_short_128),
    T(p_FDKaacEnc_16000_long_1024), T(p_FDKaacEnc_16000_short_128),
    T(p_FDKaacEnc_22050_long_1024), T(p_FDKaacEnc_22050_short_128),
    T(p_FDKaacEnc_24000_long_1024), T(p_FDKaacEnc_24000_short_128),
    T(p_FDKaacEnc_32000_long_1024), T(p_FDKaacEnc_32000_short_128),
    T(p_FDKaacEnc_44100_long_1024), T(p_FDKaacEnc_44100_short_128),
    T(p_FDKaacEnc_48000_long_1024), T(p_FDKaacEnc_48000_short_128),
    T(p_FDKaacEnc_64000_long_1024), T(p_FDKaacEnc_64000_short_128),
    T(p_FDKaacEnc_88200_long_1024), T(p_FDKaacEnc_88200_short_128),
    T(p_FDKaacEnc_96000_long_1024), T(p_FDKaacEnc_96000_short_128),
    T(FDKaacEnc_tnsEncCoeff3), T(FDKaacEnc_tnsCoeff3Borders), T(FDKaacEnc_tnsEncCoeff4),
    T(FDKaacEnc_tnsCoeff4Borders),
    T(ELDAnalysis512), T(ELDAnalysis480), T(ELDAnalysis256), T(ELDAnalysis240),
    T(ELDAnalysis128), T(ELDAnalysis120),
};

extern "C" {
/* Number of tables. */
INT cref_aac_enc_rom_count() { return (INT)(sizeof(rom_tables) / sizeof(rom_tables[0])); }
/* Name, memory and size in bytes of table i. */
const char *cref_aac_enc_rom_table(INT i, const void **ptr, INT *bytes) {
  *ptr = rom_tables[i].ptr;
  *bytes = rom_tables[i].bytes;
  return rom_tables[i].name;
}
INT cref_aac_enc_rom_sizeof_sfb_param_long() { return (INT)sizeof(SFB_PARAM_LONG); }
INT cref_aac_enc_rom_sizeof_sfb_param_short() { return (INT)sizeof(SFB_PARAM_SHORT); }
}
