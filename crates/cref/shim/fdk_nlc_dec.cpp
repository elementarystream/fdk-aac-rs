// Shims for the differential tests of fdk_aac_rs_fdk::nlc_dec.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_nlc_dec (see build.rs).
//
// A bitstream is passed as a state array `st` plus the memory `buf` of its bit buffer, with the
// layout of fdk_bitstream.cpp:
//   st[0] CacheWord, st[1] BitsInCache, st[2] ConfigCache, st[3] ValidBits, st[4] ReadOffset,
//   st[5] WriteOffset, st[6] BitNdx, st[7] bufSize, st[8] bufBits

#include "cref_common.h"
#include "nlc_dec.h"

static void nlc_bs_load(FDK_BITSTREAM *bs, const UINT *st, UCHAR *buf) {
  bs->CacheWord = st[0];
  bs->BitsInCache = st[1];
  bs->ConfigCache = st[2];
  bs->hBitBuf.ValidBits = st[3];
  bs->hBitBuf.ReadOffset = st[4];
  bs->hBitBuf.WriteOffset = st[5];
  bs->hBitBuf.BitNdx = st[6];
  bs->hBitBuf.bufSize = st[7];
  bs->hBitBuf.bufBits = st[8];
  bs->hBitBuf.Buffer = buf;
}

static void nlc_bs_store(const FDK_BITSTREAM *bs, UINT *st) {
  st[0] = bs->CacheWord;
  st[1] = bs->BitsInCache;
  st[2] = bs->ConfigCache;
  st[3] = bs->hBitBuf.ValidBits;
  st[4] = bs->hBitBuf.ReadOffset;
  st[5] = bs->hBitBuf.WriteOffset;
  st[6] = bs->hBitBuf.BitNdx;
  st[7] = bs->hBitBuf.bufSize;
  st[8] = bs->hBitBuf.bufBits;
}

extern "C" {
INT cref_nlc_dec_ec_data_pair_dec(UINT *st, UCHAR *buf, INT decoder, SCHAR *out1, SCHAR *out2, SCHAR *history, INT data_type, INT start_band, INT data_bands, INT pair_flag, INT coarse_flag, INT allow_diff_time_back_flag) {
  FDK_BITSTREAM bs;
  nlc_bs_load(&bs, st, buf);
  INT r = EcDataPairDec((DECODER_TYPE)decoder, &bs, out1, out2, history, (DATA_TYPE)data_type,
                        start_band, data_bands, pair_flag, coarse_flag,
                        allow_diff_time_back_flag);
  nlc_bs_store(&bs, st);
  return r;
}
INT cref_nlc_dec_huff_dec_reshape(UINT *st, UCHAR *buf, INT *out, INT num_val) {
  FDK_BITSTREAM bs;
  nlc_bs_load(&bs, st, buf);
  INT r = huff_dec_reshape(&bs, out, num_val);
  nlc_bs_store(&bs, st);
  return r;
}
INT cref_nlc_dec_sym_restore_ipd(UINT *st, UCHAR *buf, INT lav, SCHAR *data) {
  FDK_BITSTREAM bs;
  nlc_bs_load(&bs, st, buf);
  INT r = sym_restoreIPD(&bs, lav, data);
  nlc_bs_store(&bs, st);
  return r;
}
}
