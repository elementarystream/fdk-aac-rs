// Shims for the differential tests of fdk_aac_rs_fdk::bitstream.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_bitstream (see build.rs).
//
// A bitstream is passed as a state array `st` plus the memory `buf` of its bit buffer:
//   st[0] CacheWord, st[1] BitsInCache, st[2] ConfigCache, st[3] ValidBits, st[4] ReadOffset,
//   st[5] WriteOffset, st[6] BitNdx, st[7] bufSize, st[8] bufBits
// Each shim builds an FDK_BITSTREAM from it, calls the reference function (the inline functions
// of FDK_bitstream.h are compiled into this shim with the flags of the reference build) and
// writes the state back.

#include "cref_common.h"
#include "FDK_bitstream.h"

static void bs_load(FDK_BITSTREAM *bs, const UINT *st, UCHAR *buf) {
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

static void bs_store(const FDK_BITSTREAM *bs, UINT *st) {
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

#define BS_BEGIN    \
  FDK_BITSTREAM bs; \
  bs_load(&bs, st, buf)
#define BS_END bs_store(&bs, st)

extern "C" {
INT cref_bs_fdk_create_bit_stream(UINT *st, UCHAR *buf, UINT bufSize, UINT config) {
  HANDLE_FDK_BITSTREAM h = FDKcreateBitStream(buf, bufSize, (FDK_BS_CFG)config);
  if (h == NULL) return -1;
  INT same_buffer = h->hBitBuf.Buffer == buf;
  bs_store(h, st);
  FDKdeleteBitStream(h);
  return same_buffer;
}
void cref_bs_fdk_init_bit_stream(UINT *st, UCHAR *buf, UINT bufSize, UINT validBits, UINT config) {
  BS_BEGIN;
  FDKinitBitStream(&bs, buf, bufSize, validBits, (FDK_BS_CFG)config);
  BS_END;
}
void cref_bs_fdk_reset_bitbuffer(UINT *st, UCHAR *buf, UINT config) {
  BS_BEGIN;
  FDKresetBitbuffer(&bs, (FDK_BS_CFG)config);
  BS_END;
}
UINT cref_bs_fdk_read_bits(UINT *st, UCHAR *buf, UINT n) {
  BS_BEGIN;
  UINT r = FDKreadBits(&bs, n);
  BS_END;
  return r;
}
UINT cref_bs_fdk_read_bit(UINT *st, UCHAR *buf) {
  BS_BEGIN;
  UINT r = FDKreadBit(&bs);
  BS_END;
  return r;
}
UINT cref_bs_fdk_read2_bits(UINT *st, UCHAR *buf) {
  BS_BEGIN;
  UINT r = FDKread2Bits(&bs);
  BS_END;
  return r;
}
UINT cref_bs_fdk_read_bits_bwd(UINT *st, UCHAR *buf, UINT n) {
  BS_BEGIN;
  UINT r = FDKreadBitsBwd(&bs, n);
  BS_END;
  return r;
}
UINT cref_bs_escaped_value(UINT *st, UCHAR *buf, INT n1, INT n2, INT n3) {
  BS_BEGIN;
  UINT r = escapedValue(&bs, n1, n2, n3);
  BS_END;
  return r;
}
UINT cref_bs_fdk_get_bits(UINT *st, UCHAR *buf, UINT n) {
  BS_BEGIN;
  UINT r = FDKgetBits(&bs, n);
  BS_END;
  return r;
}
UINT cref_bs_fdk_write_bits(UINT *st, UCHAR *buf, UINT value, UINT n) {
  BS_BEGIN;
  UINT r = FDKwriteBits(&bs, value, n);
  BS_END;
  return r;
}
UINT cref_bs_fdk_write_bits_null(UINT value, UINT n) { return FDKwriteBits(NULL, value, n); }
UINT cref_bs_fdk_write_bits_bwd(UINT *st, UCHAR *buf, UINT value, UINT n) {
  BS_BEGIN;
  UINT r = FDKwriteBitsBwd(&bs, value, n);
  BS_END;
  return r;
}
UINT cref_bs_fdk_write_escaped_value(UINT *st, UCHAR *buf, UINT value, UINT n1, UINT n2, UINT n3) {
  BS_BEGIN;
  UINT r = FDKwriteEscapedValue(&bs, value, n1, n2, n3);
  BS_END;
  return r;
}
void cref_bs_fdk_sync_cache(UINT *st, UCHAR *buf) {
  BS_BEGIN;
  FDKsyncCache(&bs);
  BS_END;
}
void cref_bs_fdk_sync_cache_bwd(UINT *st, UCHAR *buf) {
  BS_BEGIN;
  FDKsyncCacheBwd(&bs);
  BS_END;
}
void cref_bs_fdk_byte_align(UINT *st, UCHAR *buf, UINT anchor) {
  BS_BEGIN;
  FDKbyteAlign(&bs, anchor);
  BS_END;
}
void cref_bs_fdk_push_back_cache(UINT *st, UCHAR *buf, UINT n) {
  BS_BEGIN;
  FDKpushBackCache(&bs, n);
  BS_END;
}
void cref_bs_fdk_push_back(UINT *st, UCHAR *buf, UINT n) {
  BS_BEGIN;
  FDKpushBack(&bs, n);
  BS_END;
}
void cref_bs_fdk_push_for(UINT *st, UCHAR *buf, UINT n) {
  BS_BEGIN;
  FDKpushFor(&bs, n);
  BS_END;
}
void cref_bs_fdk_push_bi_directional(UINT *st, UCHAR *buf, INT n) {
  BS_BEGIN;
  FDKpushBiDirectional(&bs, n);
  BS_END;
}
UINT cref_bs_fdk_get_valid_bits(UINT *st, UCHAR *buf) {
  BS_BEGIN;
  UINT r = FDKgetValidBits(&bs);
  BS_END;
  return r;
}
INT cref_bs_fdk_get_free_bits(UINT *st, UCHAR *buf) {
  BS_BEGIN;
  INT r = FDKgetFreeBits(&bs);
  BS_END;
  return r;
}
void cref_bs_fdk_feed_buffer(UINT *st, UCHAR *buf, const UCHAR *input, UINT bufferSize, UINT *bytesValid) {
  BS_BEGIN;
  FDKfeedBuffer(&bs, input, bufferSize, bytesValid);
  BS_END;
}
void cref_bs_fdk_copy_buffer(UINT *dst_st, UCHAR *dst_buf, UINT *src_st, UCHAR *src_buf, UINT *bytesValid) {
  FDK_BITSTREAM d, s;
  bs_load(&d, dst_st, dst_buf);
  bs_load(&s, src_st, src_buf);
  FDKcopyBuffer(&d, &s, bytesValid);
  bs_store(&d, dst_st);
  bs_store(&s, src_st);
}
void cref_bs_fdk_fetch_buffer(UINT *st, UCHAR *buf, UCHAR *out, UINT *writeBytes) {
  BS_BEGIN;
  FDKfetchBuffer(&bs, out, writeBytes);
  BS_END;
}
}
