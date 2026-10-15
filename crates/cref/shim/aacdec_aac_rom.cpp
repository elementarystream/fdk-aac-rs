// Shims for the differential tests of fdk_aac_rs_aacdec::aac_rom.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_aac_rom (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/aac_rom.h"
#include "libAACdec/src/aacdec_hcrs.h"

/* A copy of aac_rom.cpp, for the tables that are file-local in the library (`static`, or `const`
   without a declaration in a header, which gives them internal linkage) and for the size of the
   tables that the header declares without a size. */
namespace ref_rom {
#include "libAACdec/src/aac_rom.cpp"
}

struct TableEntry {
  const char *name;
  const void *ptr;
  INT bytes;
  INT elem_bytes;
};

/* The library's table, whose header declaration has a size. */
#define TG(x) {#x, (const void *)::x, (INT)sizeof(::x), (INT)sizeof(::x[0])}
/* The library's table, whose header declaration has no size (the size of the definition). */
#define TGN(x) {#x, (const void *)::x, (INT)sizeof(ref_rom::x), (INT)sizeof(ref_rom::x[0])}
/* A table that is file-local in the library (the copy). */
#define TN(x) {#x, (const void *)ref_rom::x, (INT)sizeof(ref_rom::x), (INT)sizeof(ref_rom::x[0])}

static const TableEntry tables[] = {
    TG(InverseQuantTable), TG(MantissaTable), TG(ExponentTable),
    TN(sfb_96_1024), TN(sfb_96_128), TN(sfb_64_1024), TN(sfb_64_128), TN(sfb_48_1024),
    TN(sfb_48_128), TN(sfb_32_1024), TN(sfb_24_1024), TN(sfb_24_128), TN(sfb_16_1024),
    TN(sfb_16_128), TN(sfb_8_1024), TN(sfb_8_128), TN(sfb_96_960), TN(sfb_96_120),
    TN(sfb_64_960), TN(sfb_64_120), TN(sfb_48_960), TN(sfb_48_120), TN(sfb_32_960),
    TN(sfb_24_960), TN(sfb_24_120), TN(sfb_16_960), TN(sfb_16_120), TN(sfb_8_960),
    TN(sfb_8_120), TN(sfb_96_768), TN(sfb_96_96), TN(sfb_64_768), TN(sfb_64_96),
    TN(sfb_48_768), TN(sfb_48_96), TN(sfb_32_768), TN(sfb_24_768), TN(sfb_24_96),
    TN(sfb_16_768), TN(sfb_16_96), TN(sfb_8_768), TN(sfb_8_96), TN(sfb_48_512),
    TN(sfb_32_512), TN(sfb_24_512), TN(sfb_48_480), TN(sfb_32_480), TN(sfb_24_480),
    TN(HuffmanCodeBook_1), TN(HuffmanCodeBook_2), TN(HuffmanCodeBook_3), TN(HuffmanCodeBook_4),
    TN(HuffmanCodeBook_5), TN(HuffmanCodeBook_6), TN(HuffmanCodeBook_7), TN(HuffmanCodeBook_8),
    TN(HuffmanCodeBook_9), TN(HuffmanCodeBook_10), TN(HuffmanCodeBook_11),
    TN(HuffmanCodeBook_SCL),
    TN(aHuffTree41), TN(aHuffTree42), TN(aHuffTree43), TN(aHuffTree44), TN(aHuffTree21),
    TN(aHuffTree22), TN(aHuffTree23), TN(aHuffTree24), TN(aHuffTree25), TN(aHuffTree26),
    TN(aHuffTree27),
    TN(aValTab41), TN(aValTab42), TN(aValTab21), TN(aValTab22), TN(aValTab23), TN(aValTab24),
    TGN(aMaxCwLen), TGN(aDimCb), TGN(aDimCbShift), TGN(aSignCb), TGN(aMinOfCbPair),
    TGN(aMaxOfCbPair), TGN(aCbPriority), TGN(aCodebook2StartInt), TGN(aLargestAbsoluteValue),
    TGN(aHuffTreeRvlcEscape), TGN(aHuffTreeRvlCodewds),
    TG(LowDelaySynthesis256), TG(LowDelaySynthesis240), TG(LowDelaySynthesis160),
    TG(LowDelaySynthesis128), TG(LowDelaySynthesis120), TG(LowDelaySynthesis512),
    TG(LowDelaySynthesis480),
    TG(tns_max_bands_tbl), TG(tns_max_bands_tbl_480), TG(tns_max_bands_tbl_512),
    TG(FDKaacDec_tnsCoeff3), TG(FDKaacDec_tnsCoeff4), TGN(FDKaacDec_tnsCoeff3_gain_ld),
    TGN(FDKaacDec_tnsCoeff4_gain_ld), TG(elementsTab), TG(AacDec_randomSign),
    TG(mdst_filt_coef_curr), TG(mdst_filt_coef_prev),
};

#define CP(x) {#x, (const void *)ref_rom::x}
struct Named {
  const char *name;
  const void *ptr;
};
/* The tables of the copy that the pointer tables can point to. */
static const Named copies[] = {
    CP(sfb_96_1024), CP(sfb_96_128), CP(sfb_64_1024), CP(sfb_64_128), CP(sfb_48_1024),
    CP(sfb_48_128), CP(sfb_32_1024), CP(sfb_24_1024), CP(sfb_24_128), CP(sfb_16_1024),
    CP(sfb_16_128), CP(sfb_8_1024), CP(sfb_8_128), CP(sfb_96_960), CP(sfb_96_120),
    CP(sfb_64_960), CP(sfb_64_120), CP(sfb_48_960), CP(sfb_48_120), CP(sfb_32_960),
    CP(sfb_24_960), CP(sfb_24_120), CP(sfb_16_960), CP(sfb_16_120), CP(sfb_8_960),
    CP(sfb_8_120), CP(sfb_96_768), CP(sfb_96_96), CP(sfb_64_768), CP(sfb_64_96),
    CP(sfb_48_768), CP(sfb_48_96), CP(sfb_32_768), CP(sfb_24_768), CP(sfb_24_96),
    CP(sfb_16_768), CP(sfb_16_96), CP(sfb_8_768), CP(sfb_8_96), CP(sfb_48_512),
    CP(sfb_32_512), CP(sfb_24_512), CP(sfb_48_480), CP(sfb_32_480), CP(sfb_24_480),
    CP(HuffmanCodeBook_1), CP(HuffmanCodeBook_2), CP(HuffmanCodeBook_3), CP(HuffmanCodeBook_4),
    CP(HuffmanCodeBook_5), CP(HuffmanCodeBook_6), CP(HuffmanCodeBook_7), CP(HuffmanCodeBook_8),
    CP(HuffmanCodeBook_9), CP(HuffmanCodeBook_10), CP(HuffmanCodeBook_11),
    CP(HuffmanCodeBook_SCL),
    CP(aHuffTree41), CP(aHuffTree42), CP(aHuffTree43), CP(aHuffTree44), CP(aHuffTree21),
    CP(aHuffTree22), CP(aHuffTree23), CP(aHuffTree24), CP(aHuffTree25), CP(aHuffTree26),
    CP(aHuffTree27),
    CP(aValTab41), CP(aValTab42), CP(aValTab21), CP(aValTab22), CP(aValTab23), CP(aValTab24),
};

/* The name of the table of the copy that `p` points to: "NULL" for NULL, "?" for none. */
static const char *copy_name(const void *p) {
  if (p == NULL) return "NULL";
  for (INT i = 0; i < (INT)(sizeof(copies) / sizeof(copies[0])); i++) {
    if (copies[i].ptr == p) return copies[i].name;
  }
  return "?";
}

/* The STATEFUNC entries: NULL and the seven state functions, in the order of the constants. */
static const STATEFUNC states[] = {
    NULL,
    Hcr_State_BODY_ONLY,
    Hcr_State_BODY_SIGN__BODY,
    Hcr_State_BODY_SIGN__SIGN,
    Hcr_State_BODY_SIGN_ESC__BODY,
    Hcr_State_BODY_SIGN_ESC__SIGN,
    Hcr_State_BODY_SIGN_ESC__ESC_PREFIX,
    Hcr_State_BODY_SIGN_ESC__ESC_WORD,
};

extern "C" {
INT cref_rom_num_tables(void) { return (INT)(sizeof(tables) / sizeof(tables[0])); }
const void *cref_rom_table(INT i, INT *bytes, INT *elem_bytes) {
  *bytes = tables[i].bytes;
  *elem_bytes = tables[i].elem_bytes;
  return tables[i].ptr;
}
const char *cref_rom_table_name(INT i) { return tables[i].name; }

/* sfbOffsetTables[i][j]: the names of the tables the copy's entry points to, the library's
   pointers, and the library's scalar fields. */
void cref_rom_sfb_info(INT i, INT j, const char **long_name, const char **short_name, const SHORT **lib_long, const SHORT **lib_short, INT *num_long, INT *num_short) {
  *long_name = copy_name(ref_rom::sfbOffsetTables[i][j].sfbOffsetLong);
  *short_name = copy_name(ref_rom::sfbOffsetTables[i][j].sfbOffsetShort);
  *lib_long = ::sfbOffsetTables[i][j].sfbOffsetLong;
  *lib_short = ::sfbOffsetTables[i][j].sfbOffsetShort;
  *num_long = ::sfbOffsetTables[i][j].numberOfSfbLong;
  *num_short = ::sfbOffsetTables[i][j].numberOfSfbShort;
}

/* AACcodeBookDescriptionTable[i] (i < 13) or AACcodeBookDescriptionSCL (i == 13). */
void cref_rom_codebook(INT i, const char **name, const USHORT **lib_cb, INT *fields) {
  const CodeBookDescription *c = (i < 13) ? &ref_rom::AACcodeBookDescriptionTable[i]
                                          : &ref_rom::AACcodeBookDescriptionSCL;
  const CodeBookDescription *l =
      (i < 13) ? &::AACcodeBookDescriptionTable[i] : &::AACcodeBookDescriptionSCL;
  *name = copy_name(c->CodeBook);
  *lib_cb = (const USHORT *)l->CodeBook;
  fields[0] = l->Dimension;
  fields[1] = l->numBits;
  fields[2] = l->Offset;
}

INT cref_rom_num_huff_table(void) { return (INT)(sizeof(ref_rom::aHuffTable) / sizeof(ref_rom::aHuffTable[0])); }
const char *cref_rom_huff_table(INT i, const UINT **lib) {
  *lib = ::aHuffTable[i];
  return copy_name(ref_rom::aHuffTable[i]);
}
INT cref_rom_num_quant_table(void) { return (INT)(sizeof(ref_rom::aQuantTable) / sizeof(ref_rom::aQuantTable[0])); }
const char *cref_rom_quant_table(INT i, const SCHAR **lib) {
  *lib = ::aQuantTable[i];
  return copy_name(ref_rom::aQuantTable[i]);
}

/* aStateConstant2State[i] of the library as an index into `states` (-1: unknown). */
INT cref_rom_num_state_constant2state(void) {
  return (INT)(sizeof(ref_rom::aStateConstant2State) / sizeof(ref_rom::aStateConstant2State[0]));
}
INT cref_rom_state_constant2state(INT i) {
  for (INT k = 0; k < (INT)(sizeof(states) / sizeof(states[0])); k++) {
    if (::aStateConstant2State[i] == states[k]) return k;
  }
  return -1;
}

/* The constants of aac_rom.h and the state constants of aacdec_hcrs.h, in the order of the Rust
   test. */
INT cref_rom_constants(INT *out) {
  INT n = 0;
  out[n++] = AACDEC_MAX_CH_CONF;
  out[n++] = AACDEC_CH_ELEMENTS_TAB_SIZE;
  out[n++] = AAC_NF_NO_RANDOM_VAL;
  out[n++] = INV_QUANT_TABLESIZE;
  out[n++] = NUM_LD_COEF_512;
  out[n++] = NUM_LD_COEF_480;
  out[n++] = WTS0;
  out[n++] = WTS1;
  out[n++] = WTS2;
  out[n++] = HuffmanBits;
  out[n++] = HuffmanEntries;
  out[n++] = SF_FNA_COEFFS;
  out[n++] = (INT)sizeof(PCM_AAC);
  out[n++] = (INT)sizeof(PCM_DEC);
  out[n++] = MAXVAL_PCM_DEC;
  out[n++] = MINVAL_PCM_DEC;
  out[n++] = PCM_DEC_BITS;
  out[n++] = (INT)sizeof(FIXP_TCC);
  out[n++] = (INT)sizeof(FIXP_FILT);
  out[n++] = STOP_THIS_STATE;
  out[n++] = BODY_ONLY;
  out[n++] = BODY_SIGN__BODY;
  out[n++] = BODY_SIGN__SIGN;
  out[n++] = BODY_SIGN_ESC__BODY;
  out[n++] = BODY_SIGN_ESC__SIGN;
  out[n++] = BODY_SIGN_ESC__ESC_PREFIX;
  out[n++] = BODY_SIGN_ESC__ESC_WORD;
  return n;
}

FIXP_DBL cref_rom_fixp_dbl2pcm_dec(FIXP_DBL x) { return FIXP_DBL2PCM_DEC(x); }
FIXP_DBL cref_rom_pcm_dec2fixp_dbl(FIXP_DBL x) { return PCM_DEC2FIXP_DBL(x); }
SHORT cref_rom_pcm_dec2fx_pcm(FIXP_DBL x) { return PCM_DEC2FX_PCM(x); }
FIXP_DBL cref_rom_fx_pcm2pcm_dec(SHORT x) { return FX_PCM2PCM_DEC(x); }
}
