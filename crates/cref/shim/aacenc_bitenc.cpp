// Shims for the differential tests of fdk_aac_rs_aacenc::bitenc.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_bitenc (see build.rs).
//
// bitenc.cpp is included into a namespace for its file-static functions. The element writer is
// called on structures built from the port's byte images: QC_OUT_CHANNEL, ELEMENT_INFO,
// TOOLSINFO and TNS_INFO have the port's layout, PSY_OUT_CHANNEL the port's layout up to
// isScale (its pointers are set here).

#include "cref_common.h"
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "tpenc_lib.h"
#include "libAACenc/src/bitenc.h"
#include "libAACenc/src/bit_cnt.h"
#include "libAACenc/src/dyn_bits.h"
#include "libAACenc/src/qc_data.h"
#include "libAACenc/src/interface.h"
#include "libAACenc/src/aacEnc_ram.h"
#include "FDK_tools_rom.h"

#include <stddef.h>
#include <stdlib.h>

/* The non-static functions of the copy get other names (their declarations in bitenc.h would
   make the calls inside the namespace ambiguous). */
#define FDKaacEnc_ChannelElementWrite ref_bitenc_FDKaacEnc_ChannelElementWrite
#define FDKaacEnc_WriteBitstream ref_bitenc_FDKaacEnc_WriteBitstream
#define FDKaacEnc_writeExtensionData ref_bitenc_FDKaacEnc_writeExtensionData
namespace ref_bitenc {
#include "libAACenc/src/bitenc.cpp"
}
#undef FDKaacEnc_ChannelElementWrite
#undef FDKaacEnc_WriteBitstream
#undef FDKaacEnc_writeExtensionData

/* A transport encoder with its own output buffer. */
struct TpBox {
  HANDLE_TRANSPORTENC h;
  UCHAR buf[8192];
};

extern "C" {
/* transportEnc_Open + transportEnc_Init(fmt) with a CODER_CONFIG of
   cfg = {aot, channelMode, samplingRate, samplesPerFrame, noChannels, flags, headerPeriod,
   bitRate}. Returns NULL if the initialization fails. */
void *cref_bitenc_tp_open(INT fmt, const INT *cfg) {
  TpBox *b = (TpBox *)calloc(1, sizeof(TpBox));
  if (transportEnc_Open(&b->h) != 0) {
    free(b);
    return NULL;
  }
  CODER_CONFIG cc;
  memset(&cc, 0, sizeof(cc));
  cc.aot = (AUDIO_OBJECT_TYPE)cfg[0];
  cc.channelMode = (CHANNEL_MODE)cfg[1];
  cc.samplingRate = cfg[2];
  cc.extSamplingRate = cfg[2];
  cc.samplesPerFrame = cfg[3];
  cc.noChannels = cfg[4];
  cc.flags = (UINT)cfg[5];
  cc.headerPeriod = (UCHAR)cfg[6];
  cc.bitRate = cfg[7];
  cc.nSubFrames = 1;
  if (transportEnc_Init(b->h, b->buf, sizeof(b->buf), (TRANSPORT_TYPE)fmt, &cc, 0) != 0) {
    transportEnc_Close(&b->h);
    free(b);
    return NULL;
  }
  return b;
}
void cref_bitenc_tp_close(void *t) {
  TpBox *b = (TpBox *)t;
  transportEnc_Close(&b->h);
  free(b);
}
/* the HANDLE_TRANSPORTENC of a transport encoder of cref_bitenc_tp_open */
void *cref_bitenc_tp_handle(void *t) {
  return ((TpBox *)t)->h;
}
/* FDKgetValidBits of the transport encoder's bitstream; copies the first n bytes of its
   buffer to out. */
INT cref_bitenc_tp_bits(void *t, UCHAR *out, INT n) {
  TpBox *b = (TpBox *)t;
  INT v = (INT)FDKgetValidBits(transportEnc_GetBitstream(b->h));
  memcpy(out, b->buf, n);
  return v;
}
/* transportEnc_WriteAccessUnit(h, totalBits, bufferFullness, ncc) */
INT cref_bitenc_tp_write_au(void *t, INT totalBits, INT fullness, INT ncc) {
  return transportEnc_WriteAccessUnit(((TpBox *)t)->h, totalBits, fullness, ncc);
}
/* transportEnc_EndAccessUnit(h, &bits); returns bits */
INT cref_bitenc_tp_end_au(void *t, INT bits) {
  transportEnc_EndAccessUnit(((TpBox *)t)->h, &bits);
  return bits;
}
/* FDKaacEnc_writeExtensionData(t ? tp : NULL, {type, nPayloadBits, payload}, tag, anchor,
   syntaxFlags, aot, epConfig) */
INT cref_bitenc_write_ext(void *t, INT type, INT nPayloadBits, UCHAR *payload, INT tag, UINT anchor, UINT flags, INT aot, INT ep) {
  QC_OUT_EXTENSION e;
  e.type = (EXT_PAYLOAD_TYPE)type;
  e.nPayloadBits = nPayloadBits;
  e.pPayload = payload;
  return FDKaacEnc_writeExtensionData(t ? ((TpBox *)t)->h : NULL, &e, tag, anchor, flags,
                                      (AUDIO_OBJECT_TYPE)aot, (SCHAR)ep);
}
/* FDKaacEnc_ChannelElementWrite(t ? tp : NULL, elInfo, qc ? {qc, qc + 1} : NULL,
   psyOutElement, psyOutChannel, flags, aot, ep, bitDemand, minCnt).
   qc: 2 QC_OUT_CHANNEL (or NULL); psy: 2 PSY_OUT_CHANNEL prefixes of psySize bytes each;
   psyOutElement = {commonWindow, *toolsInfo}. */
INT cref_bitenc_element_write(void *t, const void *elInfo, void *qc, INT commonWindow, const void *toolsInfo, const UCHAR *psy, INT psySize, UINT flags, INT aot, INT ep, INT *bitDemand, INT minCnt) {
  static PSY_OUT_CHANNEL pch[2];
  static PSY_OUT_ELEMENT pe;
  memset(pch, 0, sizeof(pch));
  memset(&pe, 0, sizeof(pe));
  QC_OUT_CHANNEL *q = (QC_OUT_CHANNEL *)qc;
  for (INT i = 0; i < 2; i++) {
    memcpy(&pch[i], psy + i * psySize, psySize);
    if (q) {
      pch[i].mdctSpectrum = q[i].mdctSpectrum;
      pch[i].sfbEnergy = q[i].sfbEnergy;
      pch[i].sfbSpreadEnergy = q[i].sfbSpreadEnergy;
      pch[i].sfbThresholdLdData = q[i].sfbThresholdLdData;
      pch[i].sfbMinSnrLdData = q[i].sfbMinSnrLdData;
      pch[i].sfbEnergyLdData = q[i].sfbEnergyLdData;
    }
  }
  pe.commonWindow = commonWindow;
  memcpy(&pe.toolsInfo, toolsInfo, sizeof(TOOLSINFO));
  pe.psyOutChannel[0] = &pch[0];
  pe.psyOutChannel[1] = &pch[1];
  QC_OUT_CHANNEL *qp[2] = {q, q ? q + 1 : NULL};
  return FDKaacEnc_ChannelElementWrite(t ? ((TpBox *)t)->h : NULL, (ELEMENT_INFO *)elInfo,
                                       q ? qp : NULL, &pe, pe.psyOutChannel, flags,
                                       (AUDIO_OBJECT_TYPE)aot, (SCHAR)ep, bitDemand,
                                       (UCHAR)minCnt);
}
/* offsetof(PSY_OUT_CHANNEL, isScale) + sizeof(isScale), sizeof(QC_OUT_CHANNEL),
   sizeof(ELEMENT_INFO), sizeof(TOOLSINFO), sizeof(TNS_INFO), sizeof(SECTION_DATA) */
void cref_bitenc_sizes(INT *v) {
  v[0] = offsetof(PSY_OUT_CHANNEL, isScale) + sizeof(((PSY_OUT_CHANNEL *)0)->isScale);
  v[1] = sizeof(QC_OUT_CHANNEL);
  v[2] = sizeof(ELEMENT_INFO);
  v[3] = sizeof(TOOLSINFO);
  v[4] = sizeof(TNS_INFO);
  v[5] = sizeof(SECTION_DATA);
}
/* The file-static writers on a bitstream over buf (1024 bytes; NULL bitstream if !useBs).
   which: 0 encodeGlobalGain(a0, a1, bs, a2); 1 encodeIcsInfo(a0..a3, bs, (UINT)a4);
   2 encodeMSInfo(a0, a1, a2, a3, jsFlags = ints, bs); 3 encodeTnsDataPresent(tns, a0, bs);
   4 encodeTnsData(tns, a0, bs); 5 encodeGainControlData(bs); 6 encodePulseData(bs);
   7 writeExtensionPayload(bs, a0, bytes, a1); 8 encodeSectionData(sectionData, bs, a0);
   9 ByteAlignment(bs, a0) (returns 0).
   tns: TNS_INFO (or NULL for which 3/4 if !ptr), sectionData: SECTION_DATA. *validBits gets
   the bits written. */
INT cref_bitenc_static(INT which, INT useBs, const INT *a, const INT *ints, const void *ptr, const UCHAR *bytes, UCHAR *buf, INT *validBits) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, 1024, 0, BS_WRITER);
  HANDLE_FDK_BITSTREAM h = useBs ? &bs : NULL;
  INT r = 0;
  switch (which) {
    case 0:
      r = ref_bitenc::FDKaacEnc_encodeGlobalGain(a[0], a[1], h, a[2]);
      break;
    case 1:
      r = ref_bitenc::FDKaacEnc_encodeIcsInfo(a[0], a[1], a[2], a[3], h, (UINT)a[4]);
      break;
    case 2:
      r = ref_bitenc::FDKaacEnc_encodeMSInfo(a[0], a[1], a[2], a[3], (INT *)ints, h);
      break;
    case 3:
      r = ref_bitenc::FDKaacEnc_encodeTnsDataPresent((TNS_INFO *)ptr, a[0], h);
      break;
    case 4:
      r = ref_bitenc::FDKaacEnc_encodeTnsData((TNS_INFO *)ptr, a[0], h);
      break;
    case 5:
      r = ref_bitenc::FDKaacEnc_encodeGainControlData(h);
      break;
    case 6:
      r = ref_bitenc::FDKaacEnc_encodePulseData(h);
      break;
    case 7:
      r = ref_bitenc::FDKaacEnc_writeExtensionPayload(h, (EXT_PAYLOAD_TYPE)a[0], bytes, a[1]);
      break;
    case 8:
      r = ref_bitenc::FDKaacEnc_encodeSectionData((SECTION_DATA *)ptr, h, (UINT)a[0]);
      break;
    case 9:
      if (h) ref_bitenc::FDKaacEnc_ByteAlignment(h, a[0]);
      break;
  }
  *validBits = (INT)FDKgetValidBits(&bs);
  return r;
}
}
