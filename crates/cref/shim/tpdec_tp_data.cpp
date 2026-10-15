// Shims for the differential tests of fdk_aac_rs_tpdec::tp_data.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::tpdec_tp_data (see build.rs).
//
// Besides the tp_data.h helpers, this file holds the pieces shared by all tpdec shims (declared
// again in the other tpdec_*.cpp files, which are linked into the same binary):
//
// - `TpRec`: recording transport decoder callbacks (CSTpCallBacks). Every call appends a record
//   to a log and returns a scripted value; the Rust tests implement the same behavior (see
//   tests/tpdec/tp_data.rs, `Rec`):
//     update_config: 1, configMode, *configChanged, hash(asc), [script], *configChanged, ret
//     sbr:           2, sampleRateIn, sampleRateOut, samplesPerFrame, coreCodec, elementID,
//                    elementIndex, harmonicSbr, stereoConfigIndex, configMode, *configChanged,
//                    downscaleFactor, validBits; skips an SbrDfltHeader (ELD: with the 6
//                    leading bits); [script], *configChanged, ret
//     ssc:           3, coreCodec, samplingRate, frameSize, numChannels, stereoConfigIndex,
//                    coreSbrFrameLengthIndex, configBytes, configMode, *configChanged,
//                    validBits; skips configBytes bytes; [script], *configChanged, ret
//     uni_drc:       4, validBits or -1 (NULL), fullPayloadLength, payloadType, subStreamIndex,
//                    payloadStart, aot; skips fullPayloadLength * 4 bits; [script], ret
//     ctrl:          5, flushCnt, flushStatus, buildUpCnt, buildUpStatus, cfgChanged,
//                    contentChanged, forceCfgChange, [script], ret
//     free_mem:      6, hash(asc), [script], ret
//     usac:          7, [script], ret
//   [script]: call k (counting all callbacks) returns rets[k % 64] and, for the callbacks with a
//   configChanged pointer, sets it to 1 (ccset[k % 64] == 1) or 0 (== 2).
// - `tpdec_asc_dump()`: a CSAudioSpecificConfig as an int array. Of the union m_sc only the
//   member that belongs to m_aot is dumped (the port stores the members separately).

#include "cref_common.h"
#include <stdlib.h>
#include "tp_data.h"
#include "tpdec_lib.h"
#include "FDK_bitstream.h"

/* ------------------------------------------------------------------------------------------ */
/* ASC dump                                                                                    */
/* ------------------------------------------------------------------------------------------ */

static int asc_active(AUDIO_OBJECT_TYPE aot) {
  switch (aot) {
    case AOT_AAC_LC:
    case AOT_AAC_SCAL:
    case AOT_ER_AAC_LC:
    case AOT_ER_AAC_LD:
    case AOT_ER_AAC_SCAL:
    case AOT_ER_BSAC:
      return 1;
    case AOT_ER_AAC_ELD:
      return 2;
    case AOT_USAC:
      return 3;
    default:
      return 0;
  }
}

INT tpdec_asc_dump(const CSAudioSpecificConfig *a, INT *out) {
  INT n = 0;
  out[n++] = a->m_aot;
  out[n++] = (INT)a->m_samplingFrequency;
  out[n++] = (INT)a->m_samplesPerFrame;
  out[n++] = (INT)a->m_directMapping;
  out[n++] = a->m_extensionAudioObjectType;
  out[n++] = (INT)a->m_extensionSamplingFrequency;
  out[n++] = a->m_channelConfiguration;
  out[n++] = a->m_epConfig;
  out[n++] = a->m_vcb11Flag;
  out[n++] = a->m_rvlcFlag;
  out[n++] = a->m_hcrFlag;
  out[n++] = a->m_sbrPresentFlag;
  out[n++] = a->m_psPresentFlag;
  out[n++] = a->m_samplingFrequencyIndex;
  out[n++] = a->m_extensionSamplingFrequencyIndex;
  out[n++] = a->m_extensionChannelConfiguration;
  out[n++] = a->configMode;
  out[n++] = a->AacConfigChanged;
  out[n++] = a->SbrConfigChanged;
  out[n++] = a->SacConfigChanged;
  out[n++] = (INT)a->configBits;
  for (int i = 0; i < TP_USAC_MAX_CONFIG_LEN; i++) out[n++] = a->config[i];
  int act = asc_active(a->m_aot);
  out[n++] = act;
  if (act == 1) {
    const CSGaSpecificConfig *g = &a->m_sc.m_gaSpecificConfig;
    out[n++] = (INT)g->m_frameLengthFlag;
    out[n++] = (INT)g->m_dependsOnCoreCoder;
    out[n++] = (INT)g->m_coreCoderDelay;
    out[n++] = (INT)g->m_extensionFlag;
    out[n++] = (INT)g->m_extensionFlag3;
    out[n++] = (INT)g->m_layer;
    out[n++] = (INT)g->m_numOfSubFrame;
    out[n++] = (INT)g->m_layerLength;
  } else if (act == 2) {
    const CSEldSpecificConfig *e = &a->m_sc.m_eldSpecificConfig;
    out[n++] = e->m_frameLengthFlag;
    out[n++] = e->m_sbrPresentFlag;
    out[n++] = e->m_useLdQmfTimeAlign;
    out[n++] = e->m_sbrSamplingRate;
    out[n++] = e->m_sbrCrcFlag;
    out[n++] = (INT)e->m_downscaledSamplingFrequency;
  } else if (act == 3) {
    const CSUsacConfig *u = &a->m_sc.m_usacConfig;
    out[n++] = u->m_frameLengthFlag;
    out[n++] = u->m_coreSbrFrameLengthIndex;
    out[n++] = u->m_sbrRatioIndex;
    out[n++] = u->m_nUsacChannels;
    out[n++] = u->m_channelConfigurationIndex;
    out[n++] = (INT)u->m_usacNumElements;
    for (int i = 0; i < TP_USAC_MAX_ELEMENTS; i++) {
      const CSUsacElementConfig *e = &u->element[i];
      out[n++] = e->usacElementType;
      out[n++] = e->m_noiseFilling;
      out[n++] = e->m_harmonicSBR;
      out[n++] = e->m_interTes;
      out[n++] = e->m_pvc;
      out[n++] = e->m_stereoConfigIndex;
      out[n++] = e->extElement.usacExtElementType;
      out[n++] = e->extElement.usacExtElementConfigLength;
      out[n++] = e->extElement.usacExtElementDefaultLength;
      out[n++] = e->extElement.usacExtElementPayloadFrag;
      out[n++] = e->extElement.usacExtElementHasAudioPreRoll;
    }
    out[n++] = u->numAudioChannels;
    out[n++] = u->m_usacConfigExtensionPresent;
    out[n++] = u->elementLengthPresent;
    out[n++] = u->UsacConfigBits;
    for (int i = 0; i < TP_USAC_MAX_CONFIG_LEN; i++) out[n++] = u->UsacConfig[i];
  }
  const UCHAR *p = (const UCHAR *)&a->m_progrConfigElement;
  for (int i = 0; i < (int)sizeof(CProgramConfig); i++) out[n++] = p[i];
  return n;
}

/* Maximal length of a dump. */
#define ASC_DUMP_MAX 4096

UINT tpdec_hash(const INT *v, INT n) {
  UINT h = 2166136261u;
  for (INT i = 0; i < n; i++) {
    h ^= (UINT)v[i];
    h *= 16777619u;
  }
  return h;
}

static UINT asc_hash(const CSAudioSpecificConfig *a) {
  INT buf[ASC_DUMP_MAX];
  INT n = tpdec_asc_dump(a, buf);
  return tpdec_hash(buf, n);
}

/* ------------------------------------------------------------------------------------------ */
/* Recording callbacks                                                                         */
/* ------------------------------------------------------------------------------------------ */

struct TpRec {
  INT *log;
  INT nlog;
  INT cap;
  INT rets[64];
  INT ccset[64];
  INT ncalls;
  CSTpCallBacks cb;
};

static void rec_push(TpRec *r, INT v) {
  if (r->nlog == r->cap) {
    r->cap = r->cap ? 2 * r->cap : 4096;
    r->log = (INT *)realloc(r->log, (size_t)r->cap * sizeof(INT));
  }
  r->log[r->nlog++] = v;
}

static INT rec_ret(TpRec *r, UCHAR *cc) {
  INT k = r->ncalls & 63;
  r->ncalls++;
  if (cc != NULL) {
    if (r->ccset[k] == 1) {
      *cc = 1;
    } else if (r->ccset[k] == 2) {
      *cc = 0;
    }
    rec_push(r, *cc);
  }
  rec_push(r, r->rets[k]);
  return r->rets[k];
}

static void rec_skip_sbr(HANDLE_FDK_BITSTREAM hBs, int isUsac) {
  if (!isUsac) FDKpushFor(hBs, 6);
  FDKpushFor(hBs, 8);
  INT e1 = (INT)FDKreadBit(hBs);
  INT e2 = (INT)FDKreadBit(hBs);
  FDKpushFor(hBs, 5 * e1 + 6 * e2);
}

static INT rec_update_config(void *h, const CSAudioSpecificConfig *asc, const UCHAR configMode,
                             UCHAR *configChanged) {
  TpRec *r = (TpRec *)h;
  rec_push(r, 1);
  rec_push(r, configMode);
  rec_push(r, *configChanged);
  rec_push(r, (INT)asc_hash(asc));
  return rec_ret(r, configChanged);
}

static INT rec_free_mem(void *h, const CSAudioSpecificConfig *asc) {
  TpRec *r = (TpRec *)h;
  rec_push(r, 6);
  rec_push(r, (INT)asc_hash(asc));
  return rec_ret(r, NULL);
}

static INT rec_ctrl(void *h, const CCtrlCFGChange *c) {
  TpRec *r = (TpRec *)h;
  rec_push(r, 5);
  rec_push(r, c->flushCnt);
  rec_push(r, c->flushStatus);
  rec_push(r, c->buildUpCnt);
  rec_push(r, c->buildUpStatus);
  rec_push(r, c->cfgChanged);
  rec_push(r, c->contentChanged);
  rec_push(r, c->forceCfgChange);
  return rec_ret(r, NULL);
}

static INT rec_ssc(void *h, HANDLE_FDK_BITSTREAM hBs, const AUDIO_OBJECT_TYPE coreCodec,
                   const INT samplingRate, const INT frameSize, const INT numChannels,
                   const INT stereoConfigIndex, const INT coreSbrFrameLengthIndex,
                   const INT configBytes, const UCHAR configMode, UCHAR *configChanged) {
  TpRec *r = (TpRec *)h;
  rec_push(r, 3);
  rec_push(r, coreCodec);
  rec_push(r, samplingRate);
  rec_push(r, frameSize);
  rec_push(r, numChannels);
  rec_push(r, stereoConfigIndex);
  rec_push(r, coreSbrFrameLengthIndex);
  rec_push(r, configBytes);
  rec_push(r, configMode);
  rec_push(r, *configChanged);
  rec_push(r, (INT)FDKgetValidBits(hBs));
  if (configBytes > 0) FDKpushFor(hBs, (UINT)configBytes * 8);
  return rec_ret(r, configChanged);
}

static INT rec_sbr(void *h, HANDLE_FDK_BITSTREAM hBs, const INT sampleRateIn,
                   const INT sampleRateOut, const INT samplesPerFrame,
                   const AUDIO_OBJECT_TYPE coreCodec, const MP4_ELEMENT_ID elementID,
                   const INT elementIndex, const UCHAR harmonicSbr,
                   const UCHAR stereoConfigIndex, const UCHAR configMode, UCHAR *configChanged,
                   const INT downscaleFactor) {
  TpRec *r = (TpRec *)h;
  rec_push(r, 2);
  rec_push(r, sampleRateIn);
  rec_push(r, sampleRateOut);
  rec_push(r, samplesPerFrame);
  rec_push(r, coreCodec);
  rec_push(r, elementID);
  rec_push(r, elementIndex);
  rec_push(r, harmonicSbr);
  rec_push(r, stereoConfigIndex);
  rec_push(r, configMode);
  rec_push(r, *configChanged);
  rec_push(r, downscaleFactor);
  rec_push(r, (INT)FDKgetValidBits(hBs));
  rec_skip_sbr(hBs, coreCodec == AOT_ER_AAC_ELD ? 0 : 1);
  return rec_ret(r, configChanged);
}

static INT rec_usac(void *h, HANDLE_FDK_BITSTREAM hBs) {
  TpRec *r = (TpRec *)h;
  rec_push(r, 7);
  return rec_ret(r, NULL);
}

static INT rec_uni_drc(void *h, HANDLE_FDK_BITSTREAM hBs, const INT fullPayloadLength,
                       const INT payloadType, const INT subStreamIndex, const INT payloadStart,
                       const AUDIO_OBJECT_TYPE aot) {
  TpRec *r = (TpRec *)h;
  rec_push(r, 4);
  rec_push(r, hBs != NULL ? (INT)FDKgetValidBits(hBs) : -1);
  rec_push(r, fullPayloadLength);
  rec_push(r, payloadType);
  rec_push(r, subStreamIndex);
  rec_push(r, payloadStart);
  rec_push(r, aot);
  if (hBs != NULL && fullPayloadLength > 0) FDKpushFor(hBs, (UINT)(fullPayloadLength * 8) / 2);
  return rec_ret(r, NULL);
}

/* The callbacks of a recorder. `has` selects the optional callbacks (bit 0: ssc, 1: sbr,
   2: usac, 3: uniDrc); update_config, free_mem and ctrl are always set. */
CSTpCallBacks *tpdec_rec_cb(void *rec) { return &((TpRec *)rec)->cb; }

void tpdec_rec_register(void *rec, HANDLE_TRANSPORTDEC hTp, INT has) {
  TpRec *r = (TpRec *)rec;
  transportDec_RegisterAscCallback(hTp, rec_update_config, r);
  transportDec_RegisterFreeMemCallback(hTp, rec_free_mem, r);
  transportDec_RegisterCtrlCFGChangeCallback(hTp, rec_ctrl, r);
  if (has & 1) transportDec_RegisterSscCallback(hTp, rec_ssc, r);
  if (has & 2) transportDec_RegisterSbrCallback(hTp, rec_sbr, r);
  if (has & 4) transportDec_RegisterUsacCallback(hTp, rec_usac, r);
  if (has & 8) transportDec_RegisterUniDrcConfigCallback(hTp, rec_uni_drc, r, NULL);
}

extern "C" {
void *cref_tpdec_rec_new(const INT *rets, const INT *ccset, INT has) {
  TpRec *r = (TpRec *)calloc(1, sizeof(TpRec));
  for (int i = 0; i < 64; i++) {
    r->rets[i] = rets[i];
    r->ccset[i] = ccset[i];
  }
  r->cb.cbUpdateConfig = rec_update_config;
  r->cb.cbUpdateConfigData = r;
  r->cb.cbFreeMem = rec_free_mem;
  r->cb.cbFreeMemData = r;
  r->cb.cbCtrlCFGChange = rec_ctrl;
  r->cb.cbCtrlCFGChangeData = r;
  if (has & 1) {
    r->cb.cbSsc = rec_ssc;
    r->cb.cbSscData = r;
  }
  if (has & 2) {
    r->cb.cbSbr = rec_sbr;
    r->cb.cbSbrData = r;
  }
  if (has & 4) {
    r->cb.cbUsac = rec_usac;
    r->cb.cbUsacData = r;
  }
  if (has & 8) {
    r->cb.cbUniDrc = rec_uni_drc;
    r->cb.cbUniDrcData = r;
  }
  return r;
}
void cref_tpdec_rec_free(void *rec) {
  TpRec *r = (TpRec *)rec;
  free(r->log);
  free(r);
}
/* Copies the log (up to `max` entries) and returns its length. */
INT cref_tpdec_rec_log(void *rec, INT *out, INT max) {
  TpRec *r = (TpRec *)rec;
  for (INT i = 0; i < r->nlog && i < max; i++) out[i] = r->log[i];
  return r->nlog;
}
void cref_tpdec_rec_clear(void *rec) { ((TpRec *)rec)->nlog = 0; }

/* tp_data.h */
UINT cref_tp_data_sampling_rate_table(INT i) { return SamplingRateTable[i]; }
INT cref_tp_data_sampling_rate_table_len(void) {
  return (INT)(sizeof(SamplingRateTable) / sizeof(SamplingRateTable[0]));
}
INT cref_tp_data_get_sampling_rate_index(UINT samplingRate, UINT nBits) {
  return getSamplingRateIndex(samplingRate, nBits);
}
INT cref_tp_data_get_number_of_total_channels(INT channelConfig) {
  return getNumberOfTotalChannels(channelConfig);
}
INT cref_tp_data_get_number_of_effective_channels(INT channelConfig) {
  return getNumberOfEffectiveChannels(channelConfig);
}
/* Sizes and offsets of the structures: sizeof(CProgramConfig), the offsets of its arrays and
   counters, sizeof(CSAudioSpecificConfig), the union offset of
   m_usacConfig.element[0].extElement.usacExtElementHasAudioPreRoll, sizeof of the GA and ELD
   members. */
void cref_tp_data_layout(INT *out) {
  out[0] = (INT)sizeof(CProgramConfig);
  out[1] = (INT)offsetof(CProgramConfig, FrontElementIsCpe);
  out[2] = (INT)offsetof(CProgramConfig, SideElementIsCpe);
  out[3] = (INT)offsetof(CProgramConfig, BackElementIsCpe);
  out[4] = (INT)offsetof(CProgramConfig, Comment);
  out[5] = (INT)offsetof(CProgramConfig, elCounter);
  out[6] = (INT)sizeof(CSAudioSpecificConfig);
  out[7] = (INT)offsetof(CSUsacConfig, element[0].extElement.usacExtElementHasAudioPreRoll);
  out[8] = (INT)sizeof(CSGaSpecificConfig);
  out[9] = (INT)sizeof(CSEldSpecificConfig);
}
/* tpdec_asc_dump() of a zeroed and of an AudioSpecificConfig_Init()ed structure. */
INT cref_tp_data_asc_dump_init(INT init, INT *out) {
  CSAudioSpecificConfig a;
  FDKmemclear(&a, sizeof(a));
  if (init) AudioSpecificConfig_Init(&a);
  return tpdec_asc_dump(&a, out);
}
}
