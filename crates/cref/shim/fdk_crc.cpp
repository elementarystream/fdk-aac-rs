// Shims for the differential tests of fdk_aac_rs_fdk::crc.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_crc (see build.rs).
//
// The CRC info lives on the C side behind an opaque handle; the bitstream is passed as a state
// array plus its memory, with the layout of fdk_bitstream.cpp.

#include "cref_common.h"
#include "FDK_crc.h"

/* A copy of FDK_crc.cpp, for its file-static lookup tables. */
namespace ref_crc {
#include "libFDK/src/FDK_crc.cpp"
}

static void crc_bs_load(FDK_BITSTREAM *bs, const UINT *st, UCHAR *buf) {
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

static void crc_bs_store(const FDK_BITSTREAM *bs, UINT *st) {
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
void *cref_crc_new(void) { return FDKcalloc(1, sizeof(FDK_CRCINFO)); }
void cref_crc_free(void *h) { FDKfree(h); }
void cref_crc_init(void *h, UINT poly, UINT start, UINT len) {
  FDKcrcInit((HANDLE_FDK_CRCINFO)h, poly, start, len);
}
void cref_crc_reset(void *h) { FDKcrcReset((HANDLE_FDK_CRCINFO)h); }
INT cref_crc_start_reg(void *h, UINT *st, UCHAR *buf, INT mBits) {
  FDK_BITSTREAM bs;
  crc_bs_load(&bs, st, buf);
  INT r = FDKcrcStartReg((HANDLE_FDK_CRCINFO)h, &bs, mBits);
  crc_bs_store(&bs, st);
  return r;
}
INT cref_crc_end_reg(void *h, UINT *st, UCHAR *buf, INT reg) {
  FDK_BITSTREAM bs;
  crc_bs_load(&bs, st, buf);
  INT r = FDKcrcEndReg((HANDLE_FDK_CRCINFO)h, &bs, reg);
  crc_bs_store(&bs, st);
  return r;
}
UINT cref_crc_get_crc(void *h) { return FDKcrcGetCRC((HANDLE_FDK_CRCINFO)h); }
/* The complete state: 3 x {isActive, maxBits, bitBufCntBits, validBits}, then the lookup table
   (0: none, 1: 16_15_5_0, 2: 16_15_2_0, 3: 16_12_5_0, -1: unknown), crcPoly, crcMask,
   startValue, crcLen, regStart, regStop, crcValue. */
void cref_crc_state(void *h, INT *out) {
  const FDK_CRCINFO *c = (const FDK_CRCINFO *)h;
  for (int i = 0; i < MAX_CRC_REGS; i++) {
    out[4 * i + 0] = c->crcRegData[i].isActive;
    out[4 * i + 1] = c->crcRegData[i].maxBits;
    out[4 * i + 2] = c->crcRegData[i].bitBufCntBits;
    out[4 * i + 3] = c->crcRegData[i].validBits;
  }
  INT id = -1;
  if (c->pCrcLookup == NULL) {
    id = 0;
  } else if (c->pCrcLookup[1] == 0x8021) {
    id = 1;
  } else if (c->pCrcLookup[1] == 0x8005) {
    id = 2;
  } else if (c->pCrcLookup[1] == 0x1021) {
    id = 3;
  }
  out[12] = id;
  out[13] = c->crcPoly;
  out[14] = c->crcMask;
  out[15] = c->startValue;
  out[16] = c->crcLen;
  out[17] = (INT)c->regStart;
  out[18] = (INT)c->regStop;
  out[19] = c->crcValue;
}
/* The lookup tables: 0: crcLookup_16_15_2_0, 1: crcLookup_16_12_5_0, 2: crcLookup_16_15_5_0. */
const USHORT *cref_crc_table(INT i, INT *count) {
  *count = 256;
  switch (i) {
    case 0:
      *count = (INT)(sizeof(ref_crc::crcLookup_16_15_2_0) / sizeof(USHORT));
      return ref_crc::crcLookup_16_15_2_0;
    case 1:
      *count = (INT)(sizeof(ref_crc::crcLookup_16_12_5_0) / sizeof(USHORT));
      return ref_crc::crcLookup_16_12_5_0;
    default:
      *count = (INT)(sizeof(ref_crc::crcLookup_16_15_5_0) / sizeof(USHORT));
      return ref_crc::crcLookup_16_15_5_0;
  }
}
}
