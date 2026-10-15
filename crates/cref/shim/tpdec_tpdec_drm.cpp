// Shims for the differential tests of fdk_aac_rs_tpdec::tpdec_drm.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::tpdec_tpdec_drm (see build.rs).
//
// STRUCT_DRM is passed as an int array `s`:
//   s[0..12] crcInfo.crcRegData[3] {isActive, maxBits, bitBufCntBits, validBits},
//   s[12] regStart, s[13] regStop, s[14] crcValue, s[15] crcReadValue (loaded),
//   s[16] crcPoly, s[17] crcMask, s[18] startValue, s[19] crcLen (stored only; loading starts
//   from drmRead_CrcInit())
// Bitstreams as in tpdec_tpdec_asc.cpp.

#include "cref_common.h"
#include "tp_data.h"
#include "tpdec_lib.h"
#include "libMpegTPDec/src/tpdec_drm.h"
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

static void drm_load(STRUCT_DRM *d, const INT *s) {
  FDKmemclear(d, sizeof(*d));
  drmRead_CrcInit(d);
  for (int i = 0; i < 3; i++) {
    d->crcInfo.crcRegData[i].isActive = (UCHAR)s[4 * i];
    d->crcInfo.crcRegData[i].maxBits = s[4 * i + 1];
    d->crcInfo.crcRegData[i].bitBufCntBits = s[4 * i + 2];
    d->crcInfo.crcRegData[i].validBits = s[4 * i + 3];
  }
  d->crcInfo.regStart = (UINT)s[12];
  d->crcInfo.regStop = (UINT)s[13];
  d->crcInfo.crcValue = (USHORT)s[14];
  d->crcReadValue = (USHORT)s[15];
}

static void drm_store(const STRUCT_DRM *d, INT *s) {
  for (int i = 0; i < 3; i++) {
    s[4 * i] = d->crcInfo.crcRegData[i].isActive;
    s[4 * i + 1] = d->crcInfo.crcRegData[i].maxBits;
    s[4 * i + 2] = d->crcInfo.crcRegData[i].bitBufCntBits;
    s[4 * i + 3] = d->crcInfo.crcRegData[i].validBits;
  }
  s[12] = (INT)d->crcInfo.regStart;
  s[13] = (INT)d->crcInfo.regStop;
  s[14] = d->crcInfo.crcValue;
  s[15] = d->crcReadValue;
  s[16] = d->crcInfo.crcPoly;
  s[17] = d->crcInfo.crcMask;
  s[18] = d->crcInfo.startValue;
  s[19] = d->crcInfo.crcLen;
}

extern "C" {
void cref_drm_crc_init(INT *s) {
  STRUCT_DRM d;
  FDKmemclear(&d, sizeof(d));
  drmRead_CrcInit(&d);
  drm_store(&d, s);
}
INT cref_drm_crc_start_reg(INT *s, UINT *st, UCHAR *buf, INT mBits) {
  STRUCT_DRM d;
  FDK_BITSTREAM bs;
  drm_load(&d, s);
  bs_load(&bs, st, buf);
  INT r = drmRead_CrcStartReg(&d, &bs, mBits);
  bs_store(&bs, st);
  drm_store(&d, s);
  return r;
}
void cref_drm_crc_end_reg(INT *s, UINT *st, UCHAR *buf, INT reg) {
  STRUCT_DRM d;
  FDK_BITSTREAM bs;
  drm_load(&d, s);
  bs_load(&bs, st, buf);
  drmRead_CrcEndReg(&d, &bs, reg);
  bs_store(&bs, st);
  drm_store(&d, s);
}
INT cref_drm_crc_check(INT *s) {
  STRUCT_DRM d;
  drm_load(&d, s);
  INT r = drmRead_CrcCheck(&d);
  drm_store(&d, s);
  return r;
}
}
