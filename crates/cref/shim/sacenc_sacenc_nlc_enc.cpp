// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_nlc_enc.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_nlc_enc (see build.rs).
//
// fdk_sacenc_ecDataPairEnc/SingleEnc are the functions of the library. The file-static
// functions are reached through a copy of sacenc_nlc_enc.cpp in a namespace.
//
// Bitstreams: a writer over a caller buffer (a power of two bytes), into which `pre` bits of
// `preval` are written first; its state is returned as out[0..6] (CacheWord, BitsInCache,
// ValidBits, ReadOffset, WriteOffset, BitNdx) without flushing the cache.

#include "cref_common.h"
#include "genericStds.h"
#include "fixpoint_math.h"
#include "FDK_bitstream.h"
#include "libSACenc/src/sacenc_nlc_enc.h"
#include "libSACenc/src/sacenc_const.h"
#include "libSACenc/src/sacenc_huff_tab.h"
#include "libSACenc/src/sacenc_paramextract.h"

namespace ref_sacenc_nlc {
#include "libSACenc/src/sacenc_nlc_enc.cpp"
}
using namespace ref_sacenc_nlc;

void sacenc_bs_begin(FDK_BITSTREAM *bs, UCHAR *buf, UINT size, INT pre, UINT preval) {
  FDKinitBitStream(bs, buf, size, 0, BS_WRITER);
  while (pre > 0) {
    INT n = pre > 25 ? 25 : pre;
    FDKwriteBits(bs, preval, n);
    pre -= n;
  }
}
void sacenc_bs_state(FDK_BITSTREAM *bs, UINT *out) {
  out[0] = bs->CacheWord;
  out[1] = bs->BitsInCache;
  out[2] = bs->hBitBuf.ValidBits;
  out[3] = bs->hBitBuf.ReadOffset;
  out[4] = bs->hBitBuf.WriteOffset;
  out[5] = bs->hBitBuf.BitNdx;
}

extern "C" {
/* fdk_sacenc_ecDataPairEnc (pair != 0) or fdk_sacenc_ecDataSingleEnc of the library. data:
   `rows` x MAXBANDS. Returns the return value. */
INT cref_sacenc_nlc_enc_ecData(UCHAR *buf, UINT size, INT pre, UINT preval, const SHORT *data, INT rows, const SHORT *hist, INT dataType, INT setIdx, INT startBand, INT dataBands, INT coarse, INT indep, INT pair, UINT *state) {
  FDK_BITSTREAM bs;
  SHORT in[4][MAXBANDS];
  SHORT h[MAXBANDS];
  FDKmemcpy(in, data, rows * MAXBANDS * sizeof(SHORT));
  FDKmemcpy(h, hist, sizeof(h));
  sacenc_bs_begin(&bs, buf, size, pre, preval);
  INT r = pair ? ::fdk_sacenc_ecDataPairEnc(&bs, in, h, (DATA_TYPE)dataType, setIdx, startBand,
                                            dataBands, coarse, indep)
               : ::fdk_sacenc_ecDataSingleEnc(&bs, in, h, (DATA_TYPE)dataType, setIdx, startBand,
                                              dataBands, coarse, indep);
  sacenc_bs_state(&bs, state);
  return r;
}
void cref_sacenc_nlc_enc_split_lsb(const SHORT *in, SHORT offset, INT n, SHORT *lsb, SHORT *msb) {
  split_lsb(in, offset, n, lsb, msb);
}
void cref_sacenc_nlc_enc_apply_lsb_coding(UCHAR *buf, UINT size, INT pre, UINT preval, const SHORT *in, UINT numLsb, INT n, UINT *state) {
  FDK_BITSTREAM bs;
  sacenc_bs_begin(&bs, buf, size, pre, preval);
  apply_lsb_coding(&bs, in, numLsb, n);
  sacenc_bs_state(&bs, state);
}
void cref_sacenc_nlc_enc_calc_diff_freq(const SHORT *in, SHORT *out, INT n) { calc_diff_freq(in, out, n); }
void cref_sacenc_nlc_enc_calc_diff_time(const SHORT *in, const SHORT *prev, SHORT *out, INT n) {
  calc_diff_time(in, prev, out, n);
}
INT cref_sacenc_nlc_enc_sym_check(SHORT *data, INT lav, SHORT *symBits) { return sym_check(data, lav, symBits); }
INT cref_sacenc_nlc_enc_ilog2(UINT i) { return ilog2(i); }
SHORT cref_sacenc_nlc_enc_calc_pcm_bits(SHORT numVal, SHORT numLevels) { return calc_pcm_bits(numVal, numLevels); }
/* which: bit 0 in1 present, bit 1 in2 present */
void cref_sacenc_nlc_enc_apply_pcm_coding(UCHAR *buf, UINT size, INT pre, UINT preval, const SHORT *in1, const SHORT *in2, INT which, SHORT offset, SHORT n, SHORT levels, UINT *state) {
  FDK_BITSTREAM bs;
  sacenc_bs_begin(&bs, buf, size, pre, preval);
  apply_pcm_coding(&bs, (which & 1) ? in1 : NULL, (which & 2) ? in2 : NULL, offset, n, levels);
  sacenc_bs_state(&bs, state);
}
/* withStrm == 0: NULL bitstream. */
UINT cref_sacenc_nlc_enc_huff_enc_1D(UCHAR *buf, UINT size, INT pre, UINT preval, INT withStrm, INT dataType, INT dim1, SHORT *in, SHORT n, SHORT p0, UINT *state) {
  FDK_BITSTREAM bs;
  sacenc_bs_begin(&bs, buf, size, pre, preval);
  UINT r = huff_enc_1D(withStrm ? &bs : NULL, (DATA_TYPE)dataType, dim1, in, n, p0);
  sacenc_bs_state(&bs, state);
  return r;
}
/* in: pairs (n entries of 2 SHORT); p0 values used where p0has bit 0/1 is set. */
UINT cref_sacenc_nlc_enc_huff_enc_2D(UCHAR *buf, UINT size, INT pre, UINT preval, INT withStrm, INT dataType, SHORT tab0, SHORT tab1, SHORT lavIdx, SHORT *in, SHORT n, SHORT stride, INT p0has, SHORT p00, SHORT p01, UINT *state) {
  FDK_BITSTREAM bs;
  SHORT tab[2] = {tab0, tab1};
  SHORT v0 = p00, v1 = p01;
  SHORT *p0[2] = {(p0has & 1) ? &v0 : NULL, (p0has & 2) ? &v1 : NULL};
  sacenc_bs_begin(&bs, buf, size, pre, preval);
  UINT r = huff_enc_2D(withStrm ? &bs : NULL, (DATA_TYPE)dataType, tab, lavIdx, (SHORT(*)[2])in, n, stride, p0);
  sacenc_bs_state(&bs, state);
  return r;
}
void cref_sacenc_nlc_enc_getHuffEntry(INT lav, INT dataType, INT i, SHORT tab0, SHORT tab1, const SHORT *in, UINT *entry, UINT *escape) {
  SHORT tab[2] = {tab0, tab1};
  HUFF_ENTRY e = {*entry}, x = {*escape};
  getHuffEntry(lav, (DATA_TYPE)dataType, i, tab, (const SHORT(*)[2])in, &e, &x);
  *entry = e.packed;
  *escape = x.packed;
}
INT cref_sacenc_nlc_enc_get_next_lav_step(INT lav, INT dataType) { return get_next_lav_step(lav, (DATA_TYPE)dataType); }
/* which: bit 0 in1 present, bit 1 in2 present; lav[2] and scheme are outputs. */
SHORT cref_sacenc_nlc_enc_calc_huff_bits(SHORT *in1, SHORT *in2, INT which, INT dataType, INT dt1, INT dt2, SHORT n, SHORT *lav, SHORT *scheme) {
  return calc_huff_bits((which & 1) ? in1 : NULL, (which & 2) ? in2 : NULL, (DATA_TYPE)dataType,
                        (DIFF_TYPE)dt1, (DIFF_TYPE)dt2, n, lav, scheme);
}
void cref_sacenc_nlc_enc_apply_huff_coding(UCHAR *buf, UINT size, INT pre, UINT preval, SHORT *in1, SHORT *in2, INT which, INT dataType, INT dt1, INT dt2, SHORT n, const SHORT *lav, SHORT scheme, UINT *state) {
  FDK_BITSTREAM bs;
  sacenc_bs_begin(&bs, buf, size, pre, preval);
  apply_huff_coding(&bs, (which & 1) ? in1 : NULL, (which & 2) ? in2 : NULL, (DATA_TYPE)dataType,
                    (DIFF_TYPE)dt1, (DIFF_TYPE)dt2, n, lav, scheme);
  sacenc_bs_state(&bs, state);
}
}

extern "C" {
/* The file-static tables as INTs: 0 lavHuffVal, 1 lavHuffLen, 2 lav_step_CLD, 3 lav_step_ICC.
   Returns the number of entries. */
INT cref_sacenc_nlc_enc_table(INT id, INT *out) {
  const UCHAR *t[4] = {lavHuffVal, lavHuffLen, lav_step_CLD, lav_step_ICC};
  const INT n[4] = {sizeof(lavHuffVal), sizeof(lavHuffLen), sizeof(lav_step_CLD), sizeof(lav_step_ICC)};
  for (INT i = 0; i < n[id]; i++) out[i] = t[id][i];
  return n[id];
}
}
