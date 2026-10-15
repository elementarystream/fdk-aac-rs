// Shims for the differential tests of fdk_aac_rs_tpenc::tpenc_adts.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::tpenc_tpenc_adts (see build.rs).
//
// The functions run on a copy of tpenc_adts.cpp in a namespace (same source and flags as the
// library). STRUCT_ADTS is passed as its byte image (120 bytes); the pointer
// crcInfo.pCrcLookup is stored in the image as the index of its table (0: NULL, 1: 0x8021,
// 2: 0x8005, 3: 0x1021), as the port does.

#include "cref_common.h"
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "FDK_crc.h"
#include "tp_data.h"
#include "tpenc_lib.h"
#include "libMpegTPEnc/src/tpenc_asc.h"

namespace ref_tpenc_adts {
#include "libMpegTPEnc/src/tpenc_adts.cpp"
}

typedef ref_tpenc_adts::STRUCT_ADTS ADTS;

/* tpenc_tpenc_asc.cpp */
void tpenc_cc_from(CODER_CONFIG *cc, const INT *v, const UCHAR *raw);

const USHORT *tpenc_crc_table(INT id) {
  FDK_CRCINFO t;
  switch (id) {
    case 1:
      FDKcrcInit(&t, 0x8021, 0, 16);
      return t.pCrcLookup;
    case 2:
      FDKcrcInit(&t, 0x8005, 0, 16);
      return t.pCrcLookup;
    case 3:
      FDKcrcInit(&t, 0x1021, 0, 16);
      return t.pCrcLookup;
    default:
      return NULL;
  }
}

UINT64 tpenc_crc_table_id(const USHORT *p) {
  if (p == NULL) return 0;
  for (INT i = 1; i <= 3; i++)
    if (p == tpenc_crc_table(i)) return (UINT64)i;
  return 4;
}

/* The image holds the table index in the bytes of pCrcLookup (pointer-sized: 8 bytes at 88 on
   LP64, 4 bytes at 88 on ILP32). */
#define ADTS_CRC_LOOKUP (offsetof(ADTS, crcInfo) + offsetof(FDK_CRCINFO, pCrcLookup))

static void adts_in(ADTS *a, const UCHAR *img) {
  FDKmemcpy(a, img, sizeof(ADTS));
  size_t id;
  FDKmemcpy(&id, img + ADTS_CRC_LOOKUP, sizeof(id));
  a->crcInfo.pCrcLookup = tpenc_crc_table((INT)id);
}

static void adts_out(const ADTS *a, UCHAR *img) {
  FDKmemcpy(img, a, sizeof(ADTS));
  size_t id = (size_t)tpenc_crc_table_id(a->crcInfo.pCrcLookup);
  FDKmemcpy(img + ADTS_CRC_LOOKUP, &id, sizeof(id));
}

extern "C" {
INT cref_tpenc_adts_size(void) { return (INT)sizeof(ADTS); }
INT cref_tpenc_adts_init(UCHAR *img, INT *ccv, UCHAR *raw) {
  ADTS a;
  CODER_CONFIG cc;
  adts_in(&a, img);
  tpenc_cc_from(&cc, ccv, raw);
  INT r = ref_tpenc_adts::adtsWrite_Init(&a, &cc);
  adts_out(&a, img);
  return r;
}
INT cref_tpenc_adts_get_header_bits(UCHAR *img) {
  ADTS a;
  adts_in(&a, img);
  INT r = ref_tpenc_adts::adtsWrite_GetHeaderBits(&a);
  adts_out(&a, img);
  return r;
}
INT cref_tpenc_adts_encode_header(UCHAR *img, void *bs, INT bf, INT fl) {
  ADTS a;
  adts_in(&a, img);
  INT r = ref_tpenc_adts::adtsWrite_EncodeHeader(&a, (HANDLE_FDK_BITSTREAM)bs, bf, fl);
  adts_out(&a, img);
  return r;
}
void cref_tpenc_adts_end_raw_data_block(UCHAR *img, void *bs, INT *bits) {
  ADTS a;
  adts_in(&a, img);
  ref_tpenc_adts::adtsWrite_EndRawDataBlock(&a, (HANDLE_FDK_BITSTREAM)bs, bits);
  adts_out(&a, img);
}
INT cref_tpenc_adts_crc_start_reg(UCHAR *img, void *bs, INT mBits) {
  ADTS a;
  adts_in(&a, img);
  INT r = ref_tpenc_adts::adtsWrite_CrcStartReg(&a, (HANDLE_FDK_BITSTREAM)bs, mBits);
  adts_out(&a, img);
  return r;
}
void cref_tpenc_adts_crc_end_reg(UCHAR *img, void *bs, INT reg) {
  ADTS a;
  adts_in(&a, img);
  ref_tpenc_adts::adtsWrite_CrcEndReg(&a, (HANDLE_FDK_BITSTREAM)bs, reg);
  adts_out(&a, img);
}
}
