// Shims for the differential tests of fdk_aac_rs_aacdec::usacdec_lpc.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_usacdec_lpc (see build.rs).
//
// The file-static functions are reached through a copy of usacdec_lpc.cpp in a namespace; the
// exported functions are the library's own.
//
// A bitstream is passed as a state array `st` plus the memory `buf` of its bit buffer:
//   st[0] CacheWord, st[1] BitsInCache, st[2] ConfigCache, st[3] ValidBits, st[4] ReadOffset,
//   st[5] WriteOffset, st[6] BitNdx, st[7] bufSize, st[8] bufBits

#include "cref_common.h"
#include "libAACdec/src/usacdec_lpc.h"
#include "usacdec_rom.h"
#include "FDK_trigFcts.h"

namespace ref_lpc {
#include "libAACdec/src/usacdec_lpc.cpp"
}

/* Non-static functions of usacdec_lpc.cpp that no header declares: the library's own code. */
void nearest_neighbor_2D8(FIXP_DBL x[8], int y[8]);
void RE8_PPV(FIXP_DBL x[], SHORT y[], int r);

static void lpc_bs_load(FDK_BITSTREAM *bs, const UINT *st, UCHAR *buf) {
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

static void lpc_bs_store(const FDK_BITSTREAM *bs, UINT *st) {
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
void cref_lpc_nearest_neighbor_2d8(FIXP_DBL *x, INT *y) { nearest_neighbor_2D8(x, y); }
void cref_lpc_re8_ppv(FIXP_DBL *x, SHORT *y, INT r) { RE8_PPV(x, y, r); }
/* re8_decode_base_index(&n, index, y); returns the n written back */
INT cref_lpc_re8_decode_base_index(INT n, UINT index, SHORT *y) {
  ref_lpc::re8_decode_base_index(&n, index, y);
  return n;
}
INT cref_lpc_re8_dec(INT n, INT I, INT *k, FIXP_DBL *y) { return ref_lpc::RE8_dec(n, I, k, y); }
void cref_lpc_re8_k2y(INT *k, INT r, SHORT *y) { ref_lpc::re8_k2y(k, r, y); }
void cref_lpc_lsf_weight_2st(SHORT *lsfq, FIXP_DBL *xq, INT nk_mode) { ref_lpc::lsf_weight_2st(lsfq, xq, nk_mode); }
void cref_lpc_reorder_lsf(SHORT *lsf, SHORT min_dist, INT n) { ref_lpc::reorder_lsf(lsf, min_dist, n); }
void cref_lpc_e_lpc_a_weight(SHORT *wA, const SHORT *A, INT m) { E_LPC_a_weight(wA, A, m); }
void cref_lpc_decode_gain(FIXP_DBL *gain, INT *gain_e, INT gain_code) { CLpd_DecodeGain(gain, gain_e, gain_code); }
void cref_lpc_lsp_a_conversion(SHORT *lsp, SHORT *a, INT *a_exp) { E_LPC_f_lsp_a_conversion(lsp, a, a_exp); }
void cref_lpc_conceal(SHORT *lsp, SHORT *lpc4_lsf, SHORT *adaptive_mean, INT first_lpd_flag) {
  CLpc_Conceal((FIXP_LPC(*)[M_LP_FILTER_ORDER])lsp, lpc4_lsf, adaptive_mean, first_lpd_flag);
}
INT cref_lpc_decode_avq(UINT *st, UCHAR *buf, FIXP_DBL *out, INT nk_mode, INT no_qn, INT length) {
  FDK_BITSTREAM bs;
  lpc_bs_load(&bs, st, buf);
  INT r = CLpc_DecodeAVQ(&bs, out, nk_mode, no_qn, length);
  lpc_bs_store(&bs, st);
  return r;
}
/* CLpc_Read(hBs, lsp[5][16], lpc4_lsf, lsf_adaptive_mean_cand, pStability[4], mod[4], ...) */
INT cref_lpc_read(UINT *st, UCHAR *buf, SHORT *lsp, SHORT *lpc4_lsf, SHORT *mean_cand, SHORT *stab, UCHAR *mod, INT first_lpd_flag, INT last_lpc_lost, INT last_frame_ok) {
  FDK_BITSTREAM bs;
  lpc_bs_load(&bs, st, buf);
  INT r = CLpc_Read(&bs, (FIXP_LPC(*)[M_LP_FILTER_ORDER])lsp, lpc4_lsf, mean_cand, stab, mod, first_lpd_flag, last_lpc_lost, last_frame_ok);
  lpc_bs_store(&bs, st);
  return r;
}
}
