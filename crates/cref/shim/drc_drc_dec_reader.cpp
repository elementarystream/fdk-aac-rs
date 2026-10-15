// Shims for the differential tests of fdk_aac_rs_drc::drc_dec_reader.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::drc_drc_dec_reader (see build.rs).
//
// The structures (UNI_DRC_CONFIG, LOUDNESS_INFO_SET, UNI_DRC_GAIN) are passed as their memory
// images (byte arrays of sizeof(struct)); the port has the same memory layout. The bitstream is
// a reader over `buf` (bufSize bytes, a power of two) with `validBits` valid bits; the state of
// the bit buffer after the call is returned in `st`:
//   st[0] ValidBits (after FDKgetValidBits), st[1] ReadOffset, st[2] BitNdx

#include "cref_common.h"
#include "FDK_bitstream.h"
#include "drcDec_types.h"
#include "drcDecoder.h"
#include "drcDec_reader.h"

static void bs_state(FDK_BITSTREAM *bs, UINT *st) {
  st[0] = FDKgetValidBits(bs);
  st[1] = bs->hBitBuf.ReadOffset;
  st[2] = bs->hBitBuf.BitNdx;
}

extern "C" {
INT cref_drc_reader_read_uni_drc_config(UCHAR *buf, UINT bufSize, UINT validBits, UCHAR *cfg, UINT *st) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, validBits, BS_READER);
  UNI_DRC_CONFIG *c = (UNI_DRC_CONFIG *)FDKcalloc(1, sizeof(UNI_DRC_CONFIG));
  FDKmemcpy(c, cfg, sizeof(UNI_DRC_CONFIG));
  INT err = drcDec_readUniDrcConfig(&bs, c);
  FDKmemcpy(cfg, c, sizeof(UNI_DRC_CONFIG));
  FDKfree(c);
  bs_state(&bs, st);
  return err;
}
INT cref_drc_reader_read_loudness_info_set(UCHAR *buf, UINT bufSize, UINT validBits, UCHAR *lis, UINT *st) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, validBits, BS_READER);
  LOUDNESS_INFO_SET *l = (LOUDNESS_INFO_SET *)FDKcalloc(1, sizeof(LOUDNESS_INFO_SET));
  FDKmemcpy(l, lis, sizeof(LOUDNESS_INFO_SET));
  INT err = drcDec_readLoudnessInfoSet(&bs, l);
  FDKmemcpy(lis, l, sizeof(LOUDNESS_INFO_SET));
  FDKfree(l);
  bs_state(&bs, st);
  return err;
}
INT cref_drc_reader_read_uni_drc_gain(UCHAR *buf, UINT bufSize, UINT validBits, UCHAR *cfg, INT frameSize, INT deltaTminDefault, UCHAR *gain, UINT *st) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, validBits, BS_READER);
  UNI_DRC_CONFIG *c = (UNI_DRC_CONFIG *)FDKcalloc(1, sizeof(UNI_DRC_CONFIG));
  FDKmemcpy(c, cfg, sizeof(UNI_DRC_CONFIG));
  UNI_DRC_GAIN g;
  FDKmemcpy(&g, gain, sizeof(UNI_DRC_GAIN));
  INT err = drcDec_readUniDrcGain(&bs, c, frameSize, deltaTminDefault, &g);
  FDKmemcpy(gain, &g, sizeof(UNI_DRC_GAIN));
  FDKmemcpy(cfg, c, sizeof(UNI_DRC_CONFIG));
  FDKfree(c);
  bs_state(&bs, st);
  return err;
}
INT cref_drc_reader_read_uni_drc(UCHAR *buf, UINT bufSize, UINT validBits, UCHAR *cfg, UCHAR *lis, INT frameSize, INT deltaTminDefault, UCHAR *gain, UINT *st) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, validBits, BS_READER);
  UNI_DRC_CONFIG *c = (UNI_DRC_CONFIG *)FDKcalloc(1, sizeof(UNI_DRC_CONFIG));
  FDKmemcpy(c, cfg, sizeof(UNI_DRC_CONFIG));
  LOUDNESS_INFO_SET *l = (LOUDNESS_INFO_SET *)FDKcalloc(1, sizeof(LOUDNESS_INFO_SET));
  FDKmemcpy(l, lis, sizeof(LOUDNESS_INFO_SET));
  UNI_DRC_GAIN g;
  FDKmemcpy(&g, gain, sizeof(UNI_DRC_GAIN));
  INT err = drcDec_readUniDrc(&bs, c, l, frameSize, deltaTminDefault, &g);
  FDKmemcpy(gain, &g, sizeof(UNI_DRC_GAIN));
  FDKmemcpy(cfg, c, sizeof(UNI_DRC_CONFIG));
  FDKmemcpy(lis, l, sizeof(LOUDNESS_INFO_SET));
  FDKfree(c);
  FDKfree(l);
  bs_state(&bs, st);
  return err;
}
}
