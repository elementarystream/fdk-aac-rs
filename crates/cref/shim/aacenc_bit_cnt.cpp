// Shims for the differential tests of fdk_aac_rs_aacenc::bit_cnt.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_bit_cnt (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/bit_cnt.h"
#include "libAACenc/src/aacEnc_rom.h"
#include "FDK_bitstream.h"

extern "C" {
INT cref_bit_cnt_bitCount(const SHORT *values, INT width, INT maxVal, INT *bitCount) {
  return FDKaacEnc_bitCount(values, width, maxVal, bitCount);
}
INT cref_bit_cnt_countValues(SHORT *values, INT width, INT codeBook) {
  return FDKaacEnc_countValues(values, width, codeBook);
}
INT cref_bit_cnt_bitCountScalefactorDelta(INT delta) {
  return FDKaacEnc_bitCountScalefactorDelta(delta);
}
/* The byte offsets of the Huffman tables FDKaacEnc_huff_ltab1_2 .. FDKaacEnc_huff_ctabscf
   relative to FDKaacEnc_huff_ltab1_2 in the linked binary (out[0..18]), and the end of
   FDKaacEnc_huff_ctabscf (out[19]). */
void cref_bit_cnt_huff_layout(INT *out) {
  const char *base = (const char *)FDKaacEnc_huff_ltab1_2;
  const void *t[19] = {FDKaacEnc_huff_ltab1_2, FDKaacEnc_huff_ltab3_4, FDKaacEnc_huff_ltab5_6,
                       FDKaacEnc_huff_ltab7_8, FDKaacEnc_huff_ltab9_10, FDKaacEnc_huff_ltab11,
                       FDKaacEnc_huff_ltabscf, FDKaacEnc_huff_ctab1, FDKaacEnc_huff_ctab2,
                       FDKaacEnc_huff_ctab3, FDKaacEnc_huff_ctab4, FDKaacEnc_huff_ctab5,
                       FDKaacEnc_huff_ctab6, FDKaacEnc_huff_ctab7, FDKaacEnc_huff_ctab8,
                       FDKaacEnc_huff_ctab9, FDKaacEnc_huff_ctab10, FDKaacEnc_huff_ctab11,
                       FDKaacEnc_huff_ctabscf};
  for (int i = 0; i < 19; i++) out[i] = (INT)((const char *)t[i] - base);
  out[19] = (INT)((const char *)FDKaacEnc_huff_ctabscf + sizeof(FDKaacEnc_huff_ctabscf) - base);
}

/* A bit writer on buf[bufSize] (bufSize a power of two) that first writes nPre bits of pre, then
   runs the calls; the calls are encoded as ops[4 * k + 0..3] = (kind, a, b, c):
   kind 0: FDKaacEnc_codeValues(values + a, b, c), kind 1: FDKaacEnc_codeScalefactorDelta(a).
   Returns the sum of the return values; *validBits is the number of bits written (after
   FDKsyncCache). */
INT cref_bit_cnt_write(UCHAR *buf, UINT bufSize, UINT pre, INT nPre, SHORT *values, const INT *ops, INT nOps, UINT *validBits) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, 0, BS_WRITER);
  FDKwriteBits(&bs, pre, nPre);
  INT ret = 0;
  for (int k = 0; k < nOps; k++) {
    const INT *op = ops + 4 * k;
    if (op[0] == 0)
      ret += FDKaacEnc_codeValues(values + op[1], op[2], op[3], &bs);
    else
      ret += FDKaacEnc_codeScalefactorDelta(op[1], &bs);
  }
  FDKsyncCache(&bs);
  *validBits = FDKgetValidBits(&bs);
  return ret;
}
}
