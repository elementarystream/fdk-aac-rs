// Shims for the differential tests of fdk_aac_rs_tpdec::tpdec_adif.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::tpdec_tpdec_adif (see build.rs).
//
// CAdifHeader is passed as {NumProgramConfigElements, BitRate, CopyrightIdPresent,
// OriginalCopy, Home, BitstreamType}, a CProgramConfig as its bytes, a bitstream as in
// tpdec_tpdec_asc.cpp.

#include "cref_common.h"
#include "tp_data.h"
#include "tpdec_lib.h"
#include "libMpegTPDec/src/tpdec_adif.h"
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

extern "C" {
INT cref_adif_decode_header(INT *h, UCHAR *pce, UINT *st, UCHAR *buf) {
  CAdifHeader a;
  CProgramConfig p;
  FDK_BITSTREAM bs;
  a.NumProgramConfigElements = h[0];
  a.BitRate = (UINT)h[1];
  a.CopyrightIdPresent = (UCHAR)h[2];
  a.OriginalCopy = (UCHAR)h[3];
  a.Home = (UCHAR)h[4];
  a.BitstreamType = (UCHAR)h[5];
  FDKmemcpy(&p, pce, sizeof(p));
  bs_load(&bs, st, buf);
  INT r = adifRead_DecodeHeader(&a, &p, &bs);
  bs_store(&bs, st);
  FDKmemcpy(pce, &p, sizeof(p));
  h[0] = a.NumProgramConfigElements;
  h[1] = (INT)a.BitRate;
  h[2] = a.CopyrightIdPresent;
  h[3] = a.OriginalCopy;
  h[4] = a.Home;
  h[5] = a.BitstreamType;
  return r;
}
}
