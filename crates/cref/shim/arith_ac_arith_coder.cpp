// Shims for the differential tests of fdk_aac_rs_arith::ac_arith_coder.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::arith_ac_arith_coder (see build.rs).
//
// A bitstream is passed as a state array `st` plus the memory `buf` of its bit buffer, with the
// layout of fdk_bitstream.cpp:
//   st[0] CacheWord, st[1] BitsInCache, st[2] ConfigCache, st[3] ValidBits, st[4] ReadOffset,
//   st[5] WriteOffset, st[6] BitNdx, st[7] bufSize, st[8] bufBits
// The decoder state `Tastat` is passed as INT[3] (low, high, vobf).
//
// The file-static functions are reached through a copy of ac_arith_coder.cpp in a namespace;
// CArco_DecodeArithData, CArco_Create and CArco_Destroy are the library's own functions.

#include "cref_common.h"
#include "common_fix.h"
#include "FDK_bitstream.h"
#include "FDK_audio.h"
#include "ac_arith_coder.h"

#include <stddef.h>

/* A copy of ac_arith_coder.cpp, for its file-static functions. */
namespace ref_arith {
#include "libArithCoding/src/ac_arith_coder.cpp"
}

static void arith_bs_load(FDK_BITSTREAM *bs, const UINT *st, UCHAR *buf) {
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

static void arith_bs_store(const FDK_BITSTREAM *bs, UINT *st) {
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
INT cref_ac_arith_coder_sizeof_CArcoData(void) { return (INT)sizeof(CArcoData); }
INT cref_ac_arith_coder_offsetof_c_prev(void) { return (INT)offsetof(CArcoData, c_prev); }
void *cref_ac_arith_coder_CArco_Create(void) { return CArco_Create(); }
void cref_ac_arith_coder_CArco_Destroy(void *p) { CArco_Destroy((CArcoData *)p); }
INT cref_ac_arith_coder_mul_sbc_14bits(INT r, INT c) { return ref_arith::mul_sbc_14bits(r, c); }
UINT cref_ac_arith_coder_get_pk_v2(UINT s) { return ref_arith::get_pk_v2(s); }
INT cref_ac_arith_coder_ari_decode_14bits(UINT *st, UCHAR *buf, INT *as, const SHORT *c_freq, INT cfl) {
  FDK_BITSTREAM bs;
  arith_bs_load(&bs, st, buf);
  ref_arith::Tastat s;
  s.low = as[0];
  s.high = as[1];
  s.vobf = as[2];
  INT r = ref_arith::ari_decode_14bits(&bs, &s, c_freq, cfl);
  as[0] = s.low;
  as[1] = s.high;
  as[2] = s.vobf;
  arith_bs_store(&bs, st);
  return r;
}
void cref_ac_arith_coder_copyTableAmrwbArith2(UCHAR *tab, INT sizeIn, INT sizeOut) {
  ref_arith::copyTableAmrwbArith2(tab, sizeIn, sizeOut);
}
INT cref_ac_arith_coder_decode2(UINT *st, UCHAR *buf, UCHAR *c_prev, FIXP_DBL *spec, INT n, INT nt) {
  FDK_BITSTREAM bs;
  arith_bs_load(&bs, st, buf);
  INT r = ref_arith::decode2(&bs, c_prev, spec, n, nt);
  arith_bs_store(&bs, st);
  return r;
}
INT cref_ac_arith_coder_CArco_DecodeArithData(UINT *st, UCHAR *buf, void *arco, FIXP_DBL *spec, INT lg, INT lg_max, INT reset) {
  FDK_BITSTREAM bs;
  arith_bs_load(&bs, st, buf);
  INT r = CArco_DecodeArithData((CArcoData *)arco, &bs, spec, lg, lg_max, reset);
  arith_bs_store(&bs, st);
  return r;
}
}

extern "C" {
/* The file-static table `name` of the copy (ref_arith) as memory: ari_merged_hash_ps, ari_pk,
   ari_lsb2. Returns NULL for another name. */
const void *cref_ac_arith_coder_table(const char *name, INT *bytes) {
#define T(x)                             \
  if (strcmp(name, #x) == 0) {           \
    *bytes = (INT)sizeof(ref_arith::x); \
    return ref_arith::x;                 \
  }
  T(ari_merged_hash_ps)
  T(ari_pk)
  T(ari_lsb2)
#undef T
  *bytes = 0;
  return NULL;
}
}
