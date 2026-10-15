// Shims for the differential tests of fdk_aac_rs_fdk::bitbuffer.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_bitbuffer (see build.rs).
//
// A bit buffer is passed as a state array `st` plus its memory `buf`:
//   st[3] ValidBits, st[4] ReadOffset, st[5] WriteOffset, st[6] BitNdx, st[7] bufSize,
//   st[8] bufBits
// (st[0..3] hold the cache of a bitstream, see fdk_bitstream.cpp). Each shim builds an
// FDK_BITBUF from it, calls the reference function and writes the state back.

#include "cref_common.h"
#include "FDK_bitbuffer.h"

/* Defined in FDK_bitbuffer.cpp without a declaration in a header. */
void CopyAlignedBlock(HANDLE_FDK_BITBUF h_BitBufSrc, UCHAR *dstBuffer, UINT bToRead);

static void bb_load(FDK_BITBUF *b, const UINT *st, UCHAR *buf) {
  b->ValidBits = st[3];
  b->ReadOffset = st[4];
  b->WriteOffset = st[5];
  b->BitNdx = st[6];
  b->bufSize = st[7];
  b->bufBits = st[8];
  b->Buffer = buf;
}

static void bb_store(const FDK_BITBUF *b, UINT *st) {
  st[3] = b->ValidBits;
  st[4] = b->ReadOffset;
  st[5] = b->WriteOffset;
  st[6] = b->BitNdx;
  st[7] = b->bufSize;
  st[8] = b->bufBits;
}

extern "C" {
const UINT *cref_bb_bit_mask(INT *count) {
  *count = (INT)(sizeof(BitMask) / sizeof(BitMask[0]));
  return BitMask;
}
void cref_bb_fdk_create_bit_buffer(UINT *st, UCHAR *buf, UINT bufSize) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  HANDLE_FDK_BITBUF h = &b;
  FDK_CreateBitBuffer(&h, buf, bufSize);
  bb_store(&b, st);
}
void cref_bb_fdk_init_bit_buffer(UINT *st, UCHAR *buf, UINT bufSize, UINT validBits) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  FDK_InitBitBuffer(&b, buf, bufSize, validBits);
  bb_store(&b, st);
}
void cref_bb_fdk_reset_bit_buffer(UINT *st, UCHAR *buf) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  FDK_ResetBitBuffer(&b);
  bb_store(&b, st);
}
void cref_bb_fdk_delete_bit_buffer(UINT *st, UCHAR *buf) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  FDK_DeleteBitBuffer(&b);
  bb_store(&b, st);
}
INT cref_bb_fdk_get(UINT *st, UCHAR *buf, UINT n) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  INT r = FDK_get(&b, n);
  bb_store(&b, st);
  return r;
}
INT cref_bb_fdk_get32(UINT *st, UCHAR *buf) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  INT r = FDK_get32(&b);
  bb_store(&b, st);
  return r;
}
INT cref_bb_fdk_get_bwd(UINT *st, UCHAR *buf, UINT n) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  INT r = FDK_getBwd(&b, n);
  bb_store(&b, st);
  return r;
}
void cref_bb_fdk_put(UINT *st, UCHAR *buf, UINT value, UINT n) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  FDK_put(&b, value, n);
  bb_store(&b, st);
}
void cref_bb_fdk_put_bwd(UINT *st, UCHAR *buf, UINT value, UINT n) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  FDK_putBwd(&b, value, n);
  bb_store(&b, st);
}
void cref_bb_fdk_push_back(UINT *st, UCHAR *buf, UINT n, UCHAR config) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  FDK_pushBack(&b, n, config);
  bb_store(&b, st);
}
void cref_bb_fdk_push_forward(UINT *st, UCHAR *buf, UINT n, UCHAR config) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  FDK_pushForward(&b, n, config);
  bb_store(&b, st);
}
UINT cref_bb_fdk_get_valid_bits(UINT *st, UCHAR *buf) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  return FDK_getValidBits(&b);
}
INT cref_bb_fdk_get_free_bits(UINT *st, UCHAR *buf) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  return FDK_getFreeBits(&b);
}
void cref_bb_fdk_feed(UINT *st, UCHAR *buf, const UCHAR *input, UINT bufferSize, UINT *bytesValid) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  FDK_Feed(&b, input, bufferSize, bytesValid);
  bb_store(&b, st);
}
void cref_bb_copy_aligned_block(UINT *st, UCHAR *buf, UCHAR *dst, UINT bToRead) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  CopyAlignedBlock(&b, dst, bToRead);
  bb_store(&b, st);
}
void cref_bb_fdk_copy(UINT *dst_st, UCHAR *dst_buf, UINT *src_st, UCHAR *src_buf, UINT *bytesValid) {
  FDK_BITBUF d, s;
  bb_load(&d, dst_st, dst_buf);
  bb_load(&s, src_st, src_buf);
  FDK_Copy(&d, &s, bytesValid);
  bb_store(&d, dst_st);
  bb_store(&s, src_st);
}
void cref_bb_fdk_fetch(UINT *st, UCHAR *buf, UCHAR *out, UINT *writeBytes) {
  FDK_BITBUF b;
  bb_load(&b, st, buf);
  FDK_Fetch(&b, out, writeBytes);
  bb_store(&b, st);
}
}
