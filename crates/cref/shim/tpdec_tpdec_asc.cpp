// Shims for the differential tests of fdk_aac_rs_tpdec::tpdec_asc.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::tpdec_tpdec_asc (see build.rs).
//
// A CProgramConfig is passed as its bytes (it consists of UCHARs only). A bitstream is passed
// as a state array plus its memory (layout of fdk_bitstream.cpp):
//   st[0] CacheWord, st[1] BitsInCache, st[2] ConfigCache, st[3] ValidBits, st[4] ReadOffset,
//   st[5] WriteOffset, st[6] BitNdx, st[7] bufSize, st[8] bufBits
// Callbacks are the recorders of tpdec_tp_data.cpp (`rec`, NULL for no callbacks); a
// CSAudioSpecificConfig is returned as tpdec_asc_dump().

#include "cref_common.h"
#include "tp_data.h"
#include "tpdec_lib.h"
#include "libMpegTPDec/src/tpdec_drm.h"
#include "FDK_bitstream.h"

/* tpdec_tp_data.cpp */
INT tpdec_asc_dump(const CSAudioSpecificConfig *a, INT *out);
CSTpCallBacks *tpdec_rec_cb(void *rec);

/* Defined in tpdec_asc.cpp without a declaration in a header. */
TRANSPORTDEC_ERROR Drm_xHEAACStaticConfig(CSAudioSpecificConfig *asc, HANDLE_FDK_BITSTREAM bs,
                                          int audioMode, CSTpCallBacks *cb);

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

static void pce_load(CProgramConfig *p, const UCHAR *b) { FDKmemcpy(p, b, sizeof(*p)); }
static void pce_store(const CProgramConfig *p, UCHAR *b) { FDKmemcpy(b, p, sizeof(*p)); }

extern "C" {
void cref_asc_pce_init(UCHAR *pce) {
  CProgramConfig p;
  pce_load(&p, pce);
  CProgramConfig_Init(&p);
  pce_store(&p, pce);
}
void cref_asc_pce_reset(UCHAR *pce) {
  CProgramConfig p;
  pce_load(&p, pce);
  CProgramConfig_Reset(&p);
  pce_store(&p, pce);
}
INT cref_asc_pce_is_valid(const UCHAR *pce) {
  CProgramConfig p;
  pce_load(&p, pce);
  return CProgramConfig_IsValid(&p);
}
void cref_asc_pce_read(UCHAR *pce, UINT *st, UCHAR *buf, UINT alignAnchor) {
  CProgramConfig p;
  FDK_BITSTREAM bs;
  pce_load(&p, pce);
  bs_load(&bs, st, buf);
  CProgramConfig_Read(&p, &bs, alignAnchor);
  bs_store(&bs, st);
  pce_store(&p, pce);
}
INT cref_asc_pce_compare(const UCHAR *a, const UCHAR *b) {
  CProgramConfig p1, p2;
  pce_load(&p1, a);
  pce_load(&p2, b);
  return CProgramConfig_Compare(&p1, &p2);
}
void cref_asc_pce_get_default(UCHAR *pce, UINT channelConfig) {
  CProgramConfig p;
  pce_load(&p, pce);
  CProgramConfig_GetDefault(&p, channelConfig);
  pce_store(&p, pce);
}
INT cref_asc_pce_lookup_element(UCHAR *pce, UINT channelConfig, UINT tag, UINT channelIdx, UCHAR *chMapping, INT *chType, UCHAR *chIndex, UINT chDescrLen, UCHAR *elMapping, INT *elList, INT elType) {
  CProgramConfig p;
  pce_load(&p, pce);
  INT r = CProgramConfig_LookupElement(&p, channelConfig, tag, channelIdx, chMapping,
                                       (AUDIO_CHANNEL_TYPE *)chType, chIndex, chDescrLen,
                                       elMapping, (MP4_ELEMENT_ID *)elList,
                                       (MP4_ELEMENT_ID)elType);
  pce_store(&p, pce);
  return r;
}
void cref_asc_pce_get_channel_description(UINT chConfig, const UCHAR *pce, INT *chType, UCHAR *chIndex) {
  CProgramConfig p;
  if (pce != NULL) pce_load(&p, pce);
  CProgramConfig_GetChannelDescription(chConfig, pce != NULL ? &p : NULL,
                                       (AUDIO_CHANNEL_TYPE *)chType, chIndex);
}
INT cref_asc_pce_get_pce_ch_map(const UCHAR *pce, UCHAR *pceChMap, UINT pceChMapLen) {
  CProgramConfig p;
  pce_load(&p, pce);
  return CProgramConfig_GetPceChMap(&p, pceChMap, pceChMapLen);
}
INT cref_asc_pce_get_element_table(const UCHAR *pce, INT *elList, INT elListSize, UCHAR *pChMapIdx) {
  CProgramConfig p;
  pce_load(&p, pce);
  return CProgramConfig_GetElementTable(&p, (MP4_ELEMENT_ID *)elList, elListSize, pChMapIdx);
}

/* AudioSpecificConfig_Init(): the dump; returns its length. */
INT cref_asc_init(INT *dump) {
  CSAudioSpecificConfig asc;
  FDKmemset(&asc, 0x5a, sizeof(asc));
  AudioSpecificConfig_Init(&asc);
  return tpdec_asc_dump(&asc, dump);
}

/* AudioSpecificConfig_Parse(): returns the error code, writes the dump of the ASC and its
   length. `rec` holds the callbacks (the function requires them). */
INT cref_asc_parse(UINT *st, UCHAR *buf, INT fExplicit, void *rec, UCHAR configMode, UCHAR configChanged, INT aot, INT *dump, INT *dumpLen) {
  CSAudioSpecificConfig asc;
  FDK_BITSTREAM bs;
  FDKmemset(&asc, 0x5a, sizeof(asc));
  bs_load(&bs, st, buf);
  TRANSPORTDEC_ERROR e = AudioSpecificConfig_Parse(&asc, &bs, fExplicit, tpdec_rec_cb(rec),
                                                   configMode, configChanged,
                                                   (AUDIO_OBJECT_TYPE)aot);
  bs_store(&bs, st);
  *dumpLen = tpdec_asc_dump(&asc, dump);
  return e;
}

/* DrmRawSdcAudioConfig_Parse() with the callbacks of `rec` or (rec == NULL) without. */
INT cref_asc_drm_parse(UINT *st, UCHAR *buf, void *rec, UCHAR configMode, UCHAR configChanged, INT *dump, INT *dumpLen) {
  CSAudioSpecificConfig asc;
  FDK_BITSTREAM bs;
  FDKmemset(&asc, 0x5a, sizeof(asc));
  bs_load(&bs, st, buf);
  TRANSPORTDEC_ERROR e = DrmRawSdcAudioConfig_Parse(
      &asc, &bs, rec != NULL ? tpdec_rec_cb(rec) : NULL, configMode, configChanged);
  bs_store(&bs, st);
  *dumpLen = tpdec_asc_dump(&asc, dump);
  return e;
}

/* Drm_xHEAACStaticConfig() on an ASC after AudioSpecificConfig_Init() with the given aot,
   sampling frequency (and index) and configMode. */
INT cref_asc_drm_xheaac_static_config(UINT *st, UCHAR *buf, INT audioMode, void *rec, INT aot, UINT samplingFrequency, UCHAR sfIndex, UCHAR configMode, INT *dump, INT *dumpLen) {
  CSAudioSpecificConfig asc;
  FDK_BITSTREAM bs;
  AudioSpecificConfig_Init(&asc);
  asc.m_aot = (AUDIO_OBJECT_TYPE)aot;
  asc.m_samplingFrequency = samplingFrequency;
  asc.m_samplingFrequencyIndex = sfIndex;
  asc.configMode = configMode;
  bs_load(&bs, st, buf);
  TRANSPORTDEC_ERROR e =
      Drm_xHEAACStaticConfig(&asc, &bs, audioMode, rec != NULL ? tpdec_rec_cb(rec) : NULL);
  bs_store(&bs, st);
  *dumpLen = tpdec_asc_dump(&asc, dump);
  return e;
}
}
