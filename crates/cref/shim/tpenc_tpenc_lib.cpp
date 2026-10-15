// Shims for the differential tests of fdk_aac_rs_tpenc::tpenc_lib.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::tpenc_tpenc_lib (see build.rs).
//
// The transport encoder of the reference is driven through the functions of the library. Its
// internal state is read through a copy of libMpegTPEnc included into a namespace (same
// definition of struct TRANSPORTENC, same layout; the copy itself is only called for the static
// getPceRepetitionRate()). The callbacks are the recorders of tpenc_tpenc_asc.cpp.

#include "cref_common.h"
#include <stdlib.h>
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "FDK_crc.h"
#include "tp_data.h"

/* The whole of libMpegTPEnc in a namespace (its headers included there first), so that
   struct TRANSPORTENC and HANDLE_TRANSPORTENC both live in the namespace. */
namespace ref_tpenc_lib {
#include "tpenc_lib.h"
#include "libMpegTPEnc/src/tpenc_asc.h"
#include "libMpegTPEnc/src/tpenc_adts.h"
#include "libMpegTPEnc/src/tpenc_adif.h"
#include "libMpegTPEnc/src/tpenc_latm.h"
#include "libMpegTPEnc/src/tpenc_lib.cpp"
#include "libMpegTPEnc/src/tpenc_asc.cpp"
#include "libMpegTPEnc/src/tpenc_adts.cpp"
#include "libMpegTPEnc/src/tpenc_adif.cpp"
#include "libMpegTPEnc/src/tpenc_latm.cpp"
}

/* The global declarations of the library API (the guard was set inside the namespace). */
#undef TPENC_LIB_H
#include "tpenc_lib.h"

/* tpenc_tpenc_asc.cpp, tpenc_tpenc_adts.cpp */
void tpenc_pat(HANDLE_FDK_BITSTREAM hBs, INT nbits, UINT seed);
void tpenc_cc_from(CODER_CONFIG *cc, const INT *v, const UCHAR *raw);
void tpenc_cc_to(const CODER_CONFIG *cc, INT *v, UCHAR *raw);
void tpenc_bs_dump(FDK_BITSTREAM *bs, INT *out);
INT tpenc_rec_sbr(void *self, HANDLE_FDK_BITSTREAM hBs, const INT sampleRateIn,
                  const INT sampleRateOut, const INT samplesPerFrame,
                  const AUDIO_OBJECT_TYPE coreCodec, const MP4_ELEMENT_ID elementID,
                  const INT elementIndex, const UCHAR harmonicSbr,
                  const UCHAR stereoConfigIndex, const UCHAR configMode, UCHAR *configChanged,
                  const INT downscaleFactor);
INT tpenc_rec_ssc(void *self, HANDLE_FDK_BITSTREAM hBs, const AUDIO_OBJECT_TYPE coreCodec,
                  const INT samplingRate, const INT frameSize, const INT numChannels,
                  const INT stereoConfigIndex, const INT coreSbrFrameLengthIndex,
                  const INT configBytes, const UCHAR configMode, UCHAR *configChanged);
INT tpenc_rec_usac(void *self, HANDLE_FDK_BITSTREAM hBs);
UINT64 tpenc_crc_table_id(const USHORT *p);

static ref_tpenc_lib::TRANSPORTENC *in(void *h) { return (ref_tpenc_lib::TRANSPORTENC *)h; }

extern "C" {
void *cref_tpenc_lib_open(void) {
  HANDLE_TRANSPORTENC h = NULL;
  if (transportEnc_Open(&h) != TRANSPORTENC_OK) return NULL;
  return h;
}
INT cref_tpenc_lib_open_null(void) { return transportEnc_Open(NULL); }
void cref_tpenc_lib_close(void *h) {
  HANDLE_TRANSPORTENC t = (HANDLE_TRANSPORTENC)h;
  transportEnc_Close(&t);
}
/* Registers the recorder for the callbacks of `has` (bit 0 SBR, 1 SSC, 2 USAC) and NULL for the
   others; returns the three return values in out[0..3]. h may be NULL. */
void cref_tpenc_lib_register(void *h, void *rec, INT has, INT *out) {
  HANDLE_TRANSPORTENC t = (HANDLE_TRANSPORTENC)h;
  out[0] = transportEnc_RegisterSbrCallback(t, (has & 1) ? tpenc_rec_sbr : NULL, rec);
  out[1] = transportEnc_RegisterSscCallback(t, (has & 2) ? tpenc_rec_ssc : NULL, rec);
  out[2] = transportEnc_RegisterUsacCallback(t, (has & 4) ? tpenc_rec_usac : NULL, rec);
}
INT cref_tpenc_lib_init(void *h, UCHAR *buf, INT size, INT fmt, INT *ccv, UCHAR *raw, UINT flags) {
  CODER_CONFIG cc;
  tpenc_cc_from(&cc, ccv, raw);
  return transportEnc_Init((HANDLE_TRANSPORTENC)h, buf, size, (TRANSPORT_TYPE)fmt, &cc, flags);
}
INT cref_tpenc_lib_add_other(void *h, INT n) {
  return transportEnc_AddOtherDataBits((HANDLE_TRANSPORTENC)h, n);
}
INT cref_tpenc_lib_write_au(void *h, INT bits, INT bf, INT ncc) {
  return transportEnc_WriteAccessUnit((HANDLE_TRANSPORTENC)h, bits, bf, ncc);
}
INT cref_tpenc_lib_end_au(void *h, INT *bits) {
  return transportEnc_EndAccessUnit((HANDLE_TRANSPORTENC)h, bits);
}
INT cref_tpenc_lib_get_frame(void *h, INT *n) {
  return transportEnc_GetFrame((HANDLE_TRANSPORTENC)h, n);
}
INT cref_tpenc_lib_static_bits(void *h, INT au) {
  return transportEnc_GetStaticBits((HANDLE_TRANSPORTENC)h, au);
}
INT cref_tpenc_lib_crc_start(void *h, INT m) {
  return transportEnc_CrcStartReg((HANDLE_TRANSPORTENC)h, m);
}
void cref_tpenc_lib_crc_end(void *h, INT reg) { transportEnc_CrcEndReg((HANDLE_TRANSPORTENC)h, reg); }
/* transportEnc_GetConf() into the bitstream `bs` (cref_tpenc_bs_new); `ccv`/`raw` receive the
   configuration afterwards. */
INT cref_tpenc_lib_get_conf(void *h, INT *ccv, UCHAR *raw, void *bs, UINT *confType) {
  CODER_CONFIG cc;
  tpenc_cc_from(&cc, ccv, raw);
  INT r = transportEnc_GetConf((HANDLE_TRANSPORTENC)h, &cc, (FDK_BITSTREAM *)bs, confType);
  tpenc_cc_to(&cc, ccv, raw);
  return r;
}
/* Writes pattern bits into the bitstream of transportEnc_GetBitstream(). */
void cref_tpenc_lib_payload(void *h, INT nbits, UINT seed) {
  tpenc_pat(transportEnc_GetBitstream((HANDLE_TRANSPORTENC)h), nbits, seed);
}
UINT cref_tpenc_lib_valid_bits(void *h) {
  return FDKgetValidBits(transportEnc_GetBitstream((HANDLE_TRANSPORTENC)h));
}
/* The high half of the address of hTpEnc->config (the bytes 36..40 of the writer union after
   LATM, see the port). */
UINT cref_tpenc_lib_cfg_ptr_hi(void *h) { return (UINT)(((UINT64)(size_t)&in(h)->config) >> 32); }
UINT cref_tpenc_lib_cfg_ptr_lo(void *h) { return (UINT)(size_t)&in(h)->config; }
/* State: out[0..23] config, out[23..39] rawConfig (16 ints), out[39] transportFmt,
   out[40..48] bitstream, out[48] bsBufferSize, out[49] pceFrameCounter, out[50..56] callbacks
   (cbSbr, cbSbrData, cbSsc, cbSscData, cbUsac, cbUsacData != NULL), out[56] bsBuffer == buf,
   wr[0..120] the writer union with config[0][0] == &config as 1 and pCrcLookup as its table
   index. */
void cref_tpenc_lib_state(void *h, INT *out, UCHAR *wr, UCHAR *buf) {
  ref_tpenc_lib::TRANSPORTENC *t = in(h);
  UCHAR raw[64];
  tpenc_cc_to(&t->config, out, raw);
  FDKmemcpy(out + 23, raw, 64);
  out[39] = t->transportFmt;
  tpenc_bs_dump(&t->bitStream, out + 40);
  out[48] = t->bsBufferSize;
  out[49] = t->pceFrameCounter;
  out[50] = t->callbacks.cbSbr != NULL;
  out[51] = t->callbacks.cbSbrData != NULL;
  out[52] = t->callbacks.cbSsc != NULL;
  out[53] = t->callbacks.cbSscData != NULL;
  out[54] = t->callbacks.cbUsac != NULL;
  out[55] = t->callbacks.cbUsacData != NULL;
  out[56] = t->bsBuffer == buf;
  FDKmemcpy(wr, &t->writer, sizeof(t->writer));
  /* The pointers are pointer-sized: 8 bytes on LP64, 4 bytes on ILP32. */
  const size_t cfg = offsetof(ref_tpenc_lib::LATM_STREAM, config);
  const size_t crc = offsetof(ref_tpenc_lib::STRUCT_ADTS, crcInfo) + offsetof(FDK_CRCINFO, pCrcLookup);
  size_t p;
  FDKmemcpy(&p, wr + cfg, sizeof(p));
  if (p == (size_t)&t->config) {
    p = 1;
    FDKmemcpy(wr + cfg, &p, sizeof(p));
  }
  FDKmemcpy(&p, wr + crc, sizeof(p));
  if (p == (size_t)t->writer.adts.crcInfo.pCrcLookup) {
    p = (size_t)tpenc_crc_table_id(t->writer.adts.crcInfo.pCrcLookup);
    if (p != 4) FDKmemcpy(wr + crc, &p, sizeof(p));
  }
}
INT cref_tpenc_lib_writer_size(void) { return (INT)sizeof(in(NULL)->writer); }
INT cref_tpenc_lib_pce_rate(INT mode, INT czero, INT fmt, INT hp, INT mma) {
  return ref_tpenc_lib::getPceRepetitionRate((CHANNEL_MODE)mode, czero, (TRANSPORT_TYPE)fmt, hp,
                                             mma);
}
/* transportEnc_GetLibInfo() on FDK_MODULE_LAST entries of which the first `prefill` are used;
   out: return value, index of the FDK_TPENC entry (-1), version, flags; title and versionStr
   (32 bytes each). `null`: pass NULL. */
INT cref_tpenc_lib_lib_info(INT prefill, INT null, INT *out, UCHAR *title, UCHAR *vstr) {
  LIB_INFO info[FDK_MODULE_LAST];
  FDKinitLibInfo(info);
  for (INT i = 0; i < prefill && i < FDK_MODULE_LAST; i++) info[i].module_id = (FDK_MODULE_ID)(100 + i);
  INT r = transportEnc_GetLibInfo(null ? NULL : info);
  out[0] = r;
  out[1] = -1;
  for (INT i = 0; i < FDK_MODULE_LAST; i++) {
    if (info[i].module_id == FDK_TPENC) {
      out[1] = i;
      out[2] = info[i].version;
      out[3] = (INT)info[i].flags;
      FDKmemclear(title, 32);
      FDKmemclear(vstr, 32);
      for (INT k = 0; k < 31 && info[i].title[k]; k++) title[k] = (UCHAR)info[i].title[k];
      FDKmemcpy(vstr, info[i].versionStr, 32);
      break;
    }
  }
  return r;
}
}
