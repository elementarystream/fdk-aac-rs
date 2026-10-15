// Shims for the differential tests of fdk_aac_rs_tpdec::tpdec_latm.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::tpdec_tpdec_latm (see build.rs).
//
// A session holds a CLatmDemux, the ASC array (pAsc, 2 entries as in the transport decoder)
// and a recorder of tpdec_tp_data.cpp, so that sequences of calls can be compared. The
// CLatmDemux is passed as an int array `s`:
//   s[0..4] m_linfo[0][0] {m_frameLengthType, m_bufferFullness, m_streamID,
//   m_frameLengthInBits}, s[4] m_taraBufferFullness, s[5] m_otherDataLength,
//   s[6] m_audioMuxLengthBytes, s[7] m_useSameStreamMux, s[8] m_AudioMuxVersion,
//   s[9] m_AudioMuxVersionA, s[10] m_allStreamsSameTimeFraming, s[11] m_noSubFrames,
//   s[12] m_numProgram, s[13] m_numLayer[0], s[14] m_otherDataPresent, s[15] m_crcCheckPresent,
//   s[16] BufferFullnessAchieved, s[17] usacExplicitCfgChanged, s[18] applyAsc,
//   s[19] newCfgHasAudioPreRoll
// Bitstreams as in tpdec_tpdec_asc.cpp.

#include "cref_common.h"
#include <stdlib.h>
#include "tp_data.h"
#include "tpdec_lib.h"
#include "libMpegTPDec/src/tpdec_latm.h"
#include "FDK_bitstream.h"

/* tpdec_tp_data.cpp */
INT tpdec_asc_dump(const CSAudioSpecificConfig *a, INT *out);
CSTpCallBacks *tpdec_rec_cb(void *rec);

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

struct LatmSess {
  CLatmDemux d;
  CSAudioSpecificConfig asc[2];
  void *rec;
};

extern "C" {
void *cref_latm_new(void *rec) {
  LatmSess *s = (LatmSess *)calloc(1, sizeof(LatmSess));
  s->rec = rec;
  return s;
}
void cref_latm_free(void *s) { free(s); }
void cref_latm_set_state(void *p, const INT *s) {
  CLatmDemux *d = &((LatmSess *)p)->d;
  d->m_linfo[0][0].m_frameLengthType = (UINT)s[0];
  d->m_linfo[0][0].m_bufferFullness = (UINT)s[1];
  d->m_linfo[0][0].m_streamID = (UINT)s[2];
  d->m_linfo[0][0].m_frameLengthInBits = (UINT)s[3];
  d->m_taraBufferFullness = (UINT)s[4];
  d->m_otherDataLength = (UINT)s[5];
  d->m_audioMuxLengthBytes = (UINT)s[6];
  d->m_useSameStreamMux = (UCHAR)s[7];
  d->m_AudioMuxVersion = (UCHAR)s[8];
  d->m_AudioMuxVersionA = (UCHAR)s[9];
  d->m_allStreamsSameTimeFraming = (UCHAR)s[10];
  d->m_noSubFrames = (UCHAR)s[11];
  d->m_numProgram = (UCHAR)s[12];
  d->m_numLayer[0] = (UCHAR)s[13];
  d->m_otherDataPresent = (UCHAR)s[14];
  d->m_crcCheckPresent = (UCHAR)s[15];
  d->BufferFullnessAchieved = (SCHAR)s[16];
  d->usacExplicitCfgChanged = (UCHAR)s[17];
  d->applyAsc = (UCHAR)s[18];
  d->newCfgHasAudioPreRoll = (UCHAR)s[19];
}
void cref_latm_get_state(void *p, INT *s) {
  const CLatmDemux *d = &((LatmSess *)p)->d;
  s[0] = (INT)d->m_linfo[0][0].m_frameLengthType;
  s[1] = (INT)d->m_linfo[0][0].m_bufferFullness;
  s[2] = (INT)d->m_linfo[0][0].m_streamID;
  s[3] = (INT)d->m_linfo[0][0].m_frameLengthInBits;
  s[4] = (INT)d->m_taraBufferFullness;
  s[5] = (INT)d->m_otherDataLength;
  s[6] = (INT)d->m_audioMuxLengthBytes;
  s[7] = d->m_useSameStreamMux;
  s[8] = d->m_AudioMuxVersion;
  s[9] = d->m_AudioMuxVersionA;
  s[10] = d->m_allStreamsSameTimeFraming;
  s[11] = d->m_noSubFrames;
  s[12] = d->m_numProgram;
  s[13] = d->m_numLayer[0];
  s[14] = d->m_otherDataPresent;
  s[15] = d->m_crcCheckPresent;
  s[16] = d->BufferFullnessAchieved;
  s[17] = d->usacExplicitCfgChanged;
  s[18] = d->applyAsc;
  s[19] = d->newCfgHasAudioPreRoll;
}
INT cref_latm_asc(void *p, INT idx, INT *dump) {
  return tpdec_asc_dump(&((LatmSess *)p)->asc[idx], dump);
}
/* AudioSpecificConfig_Init() of both ASCs. */
void cref_latm_asc_init(void *p) {
  LatmSess *s = (LatmSess *)p;
  AudioSpecificConfig_Init(&s->asc[0]);
  AudioSpecificConfig_Init(&s->asc[1]);
}
/* AudioSpecificConfig_Parse() into asc[idx] (to set up a previous configuration). */
INT cref_latm_asc_parse(void *p, INT idx, UINT *st, UCHAR *buf) {
  LatmSess *s = (LatmSess *)p;
  FDK_BITSTREAM bs;
  bs_load(&bs, st, buf);
  INT r = AudioSpecificConfig_Parse(&s->asc[idx], &bs, 1, tpdec_rec_cb(s->rec), AC_CM_ALLOC_MEM,
                                    0, AOT_NULL_OBJECT);
  bs_store(&bs, st);
  return r;
}
INT cref_latm_read_stream_mux_config(void *p, UINT *st, UCHAR *buf, INT *configFound, UCHAR configMode, UCHAR configChanged) {
  LatmSess *s = (LatmSess *)p;
  FDK_BITSTREAM bs;
  bs_load(&bs, st, buf);
  INT r = CLatmDemux_ReadStreamMuxConfig(&bs, &s->d, tpdec_rec_cb(s->rec), s->asc, configFound,
                                         configMode, configChanged);
  bs_store(&bs, st);
  return r;
}
INT cref_latm_read(void *p, UINT *st, UCHAR *buf, INT tt, INT *configFound, INT ignoreBufferFullness) {
  LatmSess *s = (LatmSess *)p;
  FDK_BITSTREAM bs;
  bs_load(&bs, st, buf);
  INT r = CLatmDemux_Read(&bs, &s->d, (TRANSPORT_TYPE)tt, tpdec_rec_cb(s->rec), s->asc,
                          configFound, ignoreBufferFullness);
  bs_store(&bs, st);
  return r;
}
INT cref_latm_read_payload_length_info(void *p, UINT *st, UCHAR *buf) {
  LatmSess *s = (LatmSess *)p;
  FDK_BITSTREAM bs;
  bs_load(&bs, st, buf);
  INT r = CLatmDemux_ReadPayloadLengthInfo(&bs, &s->d);
  bs_store(&bs, st);
  return r;
}
/* The getters: 0 GetFrameLengthInBits(a, b), 1 GetOtherDataPresentFlag, 2 GetOtherDataLength,
   3 GetNrOfSubFrames, 4 GetNrOfLayers(a). */
UINT cref_latm_getter(void *p, INT which, UINT a, UINT b) {
  CLatmDemux *d = &((LatmSess *)p)->d;
  switch (which) {
    case 0:
      return CLatmDemux_GetFrameLengthInBits(d, a, b);
    case 1:
      return CLatmDemux_GetOtherDataPresentFlag(d);
    case 2:
      return CLatmDemux_GetOtherDataLength(d);
    case 3:
      return CLatmDemux_GetNrOfSubFrames(d);
    default:
      return CLatmDemux_GetNrOfLayers(d, a);
  }
}
}
