// Shims for the differential tests of fdk_aac_rs_tpdec::tpdec_adts.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::tpdec_tpdec_adts (see build.rs).
//
// STRUCT_ADTS is passed as an int array `s`:
//   s[0..15]  bs: mpeg_id, layer, protection_absent, profile, sample_freq_index, private_bit,
//             channel_config, original, home, copyright_id, copyright_start, frame_length,
//             adts_fullness, num_raw_blocks, num_pce_bits
//   s[15] decoderCanDoMpeg4, s[16] BufferFullnesStartFlag, s[17] crcReadValue,
//   s[18..22] rawDataBlockDist[4],
//   s[22..34] crcInfo.crcRegData[3] {isActive, maxBits, bitBufCntBits, validBits},
//   s[34] regStart, s[35] regStop, s[36] crcValue (loaded),
//   s[37] crcPoly, s[38] crcMask, s[39] startValue, s[40] crcLen (stored only; loading starts
//   from adtsRead_CrcInit())
// Bitstreams as in tpdec_tpdec_asc.cpp.

#include "cref_common.h"
#include "tp_data.h"
#include "tpdec_lib.h"
#include "libMpegTPDec/src/tpdec_adts.h"
#include "FDK_bitstream.h"

/* tpdec_tp_data.cpp */
INT tpdec_asc_dump(const CSAudioSpecificConfig *a, INT *out);

#define ADTS_STATE 41

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

static void adts_load(STRUCT_ADTS *a, const INT *s) {
  FDKmemclear(a, sizeof(*a));
  adtsRead_CrcInit(a);
  a->bs.mpeg_id = (UCHAR)s[0];
  a->bs.layer = (UCHAR)s[1];
  a->bs.protection_absent = (UCHAR)s[2];
  a->bs.profile = (UCHAR)s[3];
  a->bs.sample_freq_index = (UCHAR)s[4];
  a->bs.private_bit = (UCHAR)s[5];
  a->bs.channel_config = (UCHAR)s[6];
  a->bs.original = (UCHAR)s[7];
  a->bs.home = (UCHAR)s[8];
  a->bs.copyright_id = (UCHAR)s[9];
  a->bs.copyright_start = (UCHAR)s[10];
  a->bs.frame_length = (USHORT)s[11];
  a->bs.adts_fullness = (USHORT)s[12];
  a->bs.num_raw_blocks = (UCHAR)s[13];
  a->bs.num_pce_bits = (UCHAR)s[14];
  a->decoderCanDoMpeg4 = (UCHAR)s[15];
  a->BufferFullnesStartFlag = (UCHAR)s[16];
  a->crcReadValue = (USHORT)s[17];
  for (int i = 0; i < 4; i++) a->rawDataBlockDist[i] = (USHORT)s[18 + i];
  for (int i = 0; i < 3; i++) {
    a->crcInfo.crcRegData[i].isActive = (UCHAR)s[22 + 4 * i];
    a->crcInfo.crcRegData[i].maxBits = s[23 + 4 * i];
    a->crcInfo.crcRegData[i].bitBufCntBits = s[24 + 4 * i];
    a->crcInfo.crcRegData[i].validBits = s[25 + 4 * i];
  }
  a->crcInfo.regStart = (UINT)s[34];
  a->crcInfo.regStop = (UINT)s[35];
  a->crcInfo.crcValue = (USHORT)s[36];
}

static void adts_store(const STRUCT_ADTS *a, INT *s) {
  s[0] = a->bs.mpeg_id;
  s[1] = a->bs.layer;
  s[2] = a->bs.protection_absent;
  s[3] = a->bs.profile;
  s[4] = a->bs.sample_freq_index;
  s[5] = a->bs.private_bit;
  s[6] = a->bs.channel_config;
  s[7] = a->bs.original;
  s[8] = a->bs.home;
  s[9] = a->bs.copyright_id;
  s[10] = a->bs.copyright_start;
  s[11] = a->bs.frame_length;
  s[12] = a->bs.adts_fullness;
  s[13] = a->bs.num_raw_blocks;
  s[14] = a->bs.num_pce_bits;
  s[15] = a->decoderCanDoMpeg4;
  s[16] = a->BufferFullnesStartFlag;
  s[17] = a->crcReadValue;
  for (int i = 0; i < 4; i++) s[18 + i] = a->rawDataBlockDist[i];
  for (int i = 0; i < 3; i++) {
    s[22 + 4 * i] = a->crcInfo.crcRegData[i].isActive;
    s[23 + 4 * i] = a->crcInfo.crcRegData[i].maxBits;
    s[24 + 4 * i] = a->crcInfo.crcRegData[i].bitBufCntBits;
    s[25 + 4 * i] = a->crcInfo.crcRegData[i].validBits;
  }
  s[34] = (INT)a->crcInfo.regStart;
  s[35] = (INT)a->crcInfo.regStop;
  s[36] = a->crcInfo.crcValue;
  s[37] = a->crcInfo.crcPoly;
  s[38] = a->crcInfo.crcMask;
  s[39] = a->crcInfo.startValue;
  s[40] = a->crcInfo.crcLen;
}

extern "C" {
INT cref_adts_state_len(void) { return ADTS_STATE; }
void cref_adts_crc_init(INT *s) {
  STRUCT_ADTS a;
  FDKmemclear(&a, sizeof(a));
  adtsRead_CrcInit(&a);
  adts_store(&a, s);
}
INT cref_adts_crc_start_reg(INT *s, UINT *st, UCHAR *buf, INT mBits) {
  STRUCT_ADTS a;
  FDK_BITSTREAM bs;
  adts_load(&a, s);
  bs_load(&bs, st, buf);
  INT r = adtsRead_CrcStartReg(&a, &bs, mBits);
  bs_store(&bs, st);
  adts_store(&a, s);
  return r;
}
void cref_adts_crc_end_reg(INT *s, UINT *st, UCHAR *buf, INT reg) {
  STRUCT_ADTS a;
  FDK_BITSTREAM bs;
  adts_load(&a, s);
  bs_load(&bs, st, buf);
  adtsRead_CrcEndReg(&a, &bs, reg);
  bs_store(&bs, st);
  adts_store(&a, s);
}
INT cref_adts_crc_check(INT *s) {
  STRUCT_ADTS a;
  adts_load(&a, s);
  INT r = adtsRead_CrcCheck(&a);
  adts_store(&a, s);
  return r;
}
/* adtsRead_DecodeHeader() with an ASC after AudioSpecificConfig_Init() whose PCE is `pce`. */
INT cref_adts_decode_header(INT *s, const UCHAR *pce, UINT *st, UCHAR *buf, INT ignoreBufferFullness, INT *dump, INT *dumpLen) {
  STRUCT_ADTS a;
  FDK_BITSTREAM bs;
  CSAudioSpecificConfig asc;
  AudioSpecificConfig_Init(&asc);
  FDKmemcpy(&asc.m_progrConfigElement, pce, sizeof(CProgramConfig));
  adts_load(&a, s);
  bs_load(&bs, st, buf);
  INT r = adtsRead_DecodeHeader(&a, &asc, &bs, ignoreBufferFullness);
  bs_store(&bs, st);
  adts_store(&a, s);
  *dumpLen = tpdec_asc_dump(&asc, dump);
  return r;
}
INT cref_adts_get_raw_data_block_length(const INT *s, INT blockNum) {
  STRUCT_ADTS a;
  adts_load(&a, s);
  return adtsRead_GetRawDataBlockLength(&a, blockNum);
}
}
