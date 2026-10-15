// Stubbed reference decoder for the differential tests of fdk_aac_rs_aacdec::aacdecoder and
// aacdecoder_lib. Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_aacdecoder_stub (see build.rs).
//
// aacdecoder.cpp, aacdecoder_lib.cpp and aacdec_drc.cpp of the reference are compiled into this
// file with their global functions renamed to stb_*, and with every call of the SBR decoder and
// of the MPEG Surround decoder redirected to stubs that reject every configuration. Everything
// else (transport, core, DRC, PCM utilities) is the library's code. The copy is only used for
// the CCE check below, on streams for which the stubs are not observable (AAC-LC without SBR).
//
// The check: the result of CChannelElement_Read() for a coupling channel element (CCE) must not
// depend on the contents of its dummy channel. Before every CCE read, the hook below repeats the
// read on copies of the dummy channel filled with different byte patterns, and counts the reads
// whose result (return value, bits read, element instance tag) depends on the pattern.

#define CAacDecoder_AncDataGet stb_CAacDecoder_AncDataGet
#define CAacDecoder_AncDataInit stb_CAacDecoder_AncDataInit
#define CAacDecoder_ApplyCrossFade stb_CAacDecoder_ApplyCrossFade
#define CAacDecoder_Close stb_CAacDecoder_Close
#define CAacDecoder_CtrlCFGChange stb_CAacDecoder_CtrlCFGChange
#define CAacDecoder_DecodeFrame stb_CAacDecoder_DecodeFrame
#define CAacDecoder_FreeMem stb_CAacDecoder_FreeMem
#define CAacDecoder_GetStreamInfo stb_CAacDecoder_GetStreamInfo
#define CAacDecoder_Init stb_CAacDecoder_Init
#define CAacDecoder_Open stb_CAacDecoder_Open
#define CAacDecoder_PrepareCrossFade stb_CAacDecoder_PrepareCrossFade
#define CAacDecoder_PreRollExtensionPayloadParse stb_CAacDecoder_PreRollExtensionPayloadParse
#define CAacDecoder_SignalInterruption stb_CAacDecoder_SignalInterruption
#define CAacDecoder_SyncQmfMode stb_CAacDecoder_SyncQmfMode
#define aacDecoder_drcDisable stb_aacDecoder_drcDisable
#define aacDecoder_drcReset stb_aacDecoder_drcReset
#define aacDecoder_drcInit stb_aacDecoder_drcInit
#define aacDecoder_drcInitChannelData stb_aacDecoder_drcInitChannelData
#define aacDecoder_drcSetParam stb_aacDecoder_drcSetParam
#define aacDecoder_drcMarkPayload stb_aacDecoder_drcMarkPayload
#define aacDecoder_drcApply stb_aacDecoder_drcApply
#define aacDecoder_drcProlog stb_aacDecoder_drcProlog
#define aacDecoder_drcEpilog stb_aacDecoder_drcEpilog
#define aacDecoder_drcGetInfo stb_aacDecoder_drcGetInfo
#define applyDrcLevelNormalization stb_applyDrcLevelNormalization
#undef aacDecoder_AncDataGet
#undef aacDecoder_AncDataInit
#undef aacDecoder_Close
#undef aacDecoder_ConfigRaw
#undef aacDecoder_DecodeFrame
#undef aacDecoder_Fill
#undef aacDecoder_GetFreeBytes
#undef aacDecoder_GetLibInfo
#undef aacDecoder_GetStreamInfo
#undef aacDecoder_Open
#undef aacDecoder_RawISOBMFFData
#undef aacDecoder_SetParam
#define aacDecoder_AncDataGet stb_aacDecoder_AncDataGet
#define aacDecoder_AncDataInit stb_aacDecoder_AncDataInit
#define aacDecoder_Close stb_aacDecoder_Close
#define aacDecoder_ConfigRaw stb_aacDecoder_ConfigRaw
#define aacDecoder_DecodeFrame stb_aacDecoder_DecodeFrame
#define aacDecoder_Fill stb_aacDecoder_Fill
#define aacDecoder_GetFreeBytes stb_aacDecoder_GetFreeBytes
#define aacDecoder_GetLibInfo stb_aacDecoder_GetLibInfo
#define aacDecoder_GetStreamInfo stb_aacDecoder_GetStreamInfo
#define aacDecoder_Open stb_aacDecoder_Open
#define aacDecoder_RawISOBMFFData stb_aacDecoder_RawISOBMFFData
#define aacDecoder_SetParam stb_aacDecoder_SetParam
#define sbrDecoder_Open stb_sbrDecoder_Open
#define sbrDecoder_InitElement stb_sbrDecoder_InitElement
#define sbrDecoder_FreeMem stb_sbrDecoder_FreeMem
#define sbrDecoder_Header stb_sbrDecoder_Header
#define sbrDecoder_SetParam stb_sbrDecoder_SetParam
#define sbrDecoder_drcFeedChannel stb_sbrDecoder_drcFeedChannel
#define sbrDecoder_drcDisable stb_sbrDecoder_drcDisable
#define sbrDecoder_Parse stb_sbrDecoder_Parse
#define sbrDecoder_Apply stb_sbrDecoder_Apply
#define sbrDecoder_Close stb_sbrDecoder_Close
#define sbrDecoder_GetDelay stb_sbrDecoder_GetDelay
#define mpegSurroundDecoder_Open stb_mpegSurroundDecoder_Open
#define mpegSurroundDecoder_IsFullMpegSurroundDecoderInstanceAvailable \
  stb_mpegSurroundDecoder_IsFull
#define mpegSurroundDecoder_Config stb_mpegSurroundDecoder_Config
#define mpegSurroundDecoder_ConfigureQmfDomain stb_mpegSurroundDecoder_ConfigureQmfDomain
#define mpegSurroundDecoder_Parse stb_mpegSurroundDecoder_Parse
#define mpegSurroundDecoder_ParseNoHeader stb_mpegSurroundDecoder_ParseNoHeader
#define mpegSurroundDecoder_Apply stb_mpegSurroundDecoder_Apply
#define mpegSurroundDecoder_Close stb_mpegSurroundDecoder_Close
#define mpegSurroundDecoder_FreeMem stb_mpegSurroundDecoder_FreeMem
#define mpegSurroundDecoder_SetParam stb_mpegSurroundDecoder_SetParam
#define mpegSurroundDecoder_GetDelay stb_mpegSurroundDecoder_GetDelay
#define mpegSurroundDecoder_IsPseudoLR stb_mpegSurroundDecoder_IsPseudoLR

#include "cref_common.h"
#include "aac_ram.h"
#include "aac_rom.h"
#include "aacdec_drc.h"
#include "aacdec_pns.h"
#include "aacdecoder_lib.h"
#include "aacdecoder.h"
#include "ac_arith_coder.h"
#include "block.h"
#include "channel.h"
#include "conceal.h"
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "FDK_core.h"
#include "FDK_crc.h"
#include "pcm_utils.h"
#include "sac_dec_lib.h"
#include "sbrdecoder.h"
#include "tpdec_lib.h"
#include "usacdec_lpd.h"
#include <stdlib.h>

/* ---- the CCE hook -------------------------------------------------------------------------- */

#define CChannelElement_Read stb_CChannelElement_Read_hook
AAC_DECODER_ERROR stb_CChannelElement_Read_hook(
    HANDLE_FDK_BITSTREAM hBs, CAacDecoderChannelInfo *pAacDecoderChannelInfo[],
    CAacDecoderStaticChannelInfo *pAacDecoderStaticChannelInfo[],
    const AUDIO_OBJECT_TYPE aot, SamplingRateInfo *pSamplingRateInfo, const UINT flags,
    const UINT elFlags, const UINT frame_length, const UCHAR numberOfChannels,
    const SCHAR epConfig, HANDLE_TRANSPORTDEC pTpDec);

#include "libAACdec/src/aacdec_drc.cpp"
#include "libAACdec/src/aacdecoder.cpp"
#include "libAACdec/src/aacdecoder_lib.cpp"

#undef CChannelElement_Read

static thread_local INT g_cce_reads, g_cce_pattern_dependent;

AAC_DECODER_ERROR stb_CChannelElement_Read_hook(
    HANDLE_FDK_BITSTREAM hBs, CAacDecoderChannelInfo *pAacDecoderChannelInfo[],
    CAacDecoderStaticChannelInfo *pAacDecoderStaticChannelInfo[],
    const AUDIO_OBJECT_TYPE aot, SamplingRateInfo *pSamplingRateInfo, const UINT flags,
    const UINT elFlags, const UINT frame_length, const UCHAR numberOfChannels,
    const SCHAR epConfig, HANDLE_TRANSPORTDEC pTpDec) {
  if (elFlags == AC_EL_GA_CCE) {
    static const UCHAR patterns[3] = {0x00, 0xFF, 0x5A};
    CAacDecoderChannelInfo *orig = pAacDecoderChannelInfo[0];
    INT res[3][3];
    for (int k = 0; k < 3; k++) {
      CAacDecoderChannelInfo *t = (CAacDecoderChannelInfo *)malloc(sizeof(*t));
      CAacDecoderCommonData *cd = (CAacDecoderCommonData *)malloc(sizeof(*cd));
      CAacDecoderCommonStaticData csd;
      CWorkBufferCore1 *wb = (CWorkBufferCore1 *)malloc(sizeof(*wb));
      FIXP_DBL *spec = (FIXP_DBL *)calloc(1024, sizeof(FIXP_DBL));
      FDK_BITSTREAM b = *hBs;
      CAacDecoderChannelInfo *tp[1] = {t};
      memset(t, patterns[k], sizeof(*t));
      memset(cd, patterns[k], sizeof(*cd));
      memset(&csd, patterns[k], sizeof(csd));
      memset(wb, patterns[k], sizeof(*wb));
      /* The members CAacDecoder_DecodeFrame() initializes. */
      cd->pnsInterChannelData = orig->pComData->pnsInterChannelData;
      t->data.aac.PnsData = orig->data.aac.PnsData;
      t->granuleLength = orig->granuleLength;
      t->pDynData = cd->pAacDecoderDynamicData;
      t->pComData = cd;
      csd.pWorkBufferCore1 = wb;
      t->pComStaticData = &csd;
      t->pSpectralCoefficient = (SPECTRAL_PTR)spec;
      res[k][0] = CChannelElement_Read(&b, tp, pAacDecoderStaticChannelInfo, aot,
                                       pSamplingRateInfo, flags, elFlags, frame_length,
                                       numberOfChannels, epConfig, NULL);
      res[k][1] = (INT)FDKgetValidBits(&b);
      res[k][2] = t->ElementInstanceTag;
      free(t);
      free(cd);
      free(wb);
      free(spec);
    }
    g_cce_reads++;
    if (memcmp(res[0], res[1], sizeof(res[0])) || memcmp(res[0], res[2], sizeof(res[0])))
      g_cce_pattern_dependent++;
  }
  return CChannelElement_Read(hBs, pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo, aot,
                              pSamplingRateInfo, flags, elFlags, frame_length,
                              numberOfChannels, epConfig, pTpDec);
}

/* ---- SBR and MPEG Surround stubs ---------------------------------------------------------- */

SBR_ERROR stb_sbrDecoder_Open(HANDLE_SBRDECODER *pSelf, HANDLE_FDK_QMF_DOMAIN pQmfDomain) {
  *pSelf = (HANDLE_SBRDECODER)calloc(1, 64);
  return SBRDEC_OK;
}
SBR_ERROR stb_sbrDecoder_InitElement(HANDLE_SBRDECODER self, const int sampleRateIn,
                                     const int sampleRateOut, const int samplesPerFrame,
                                     const AUDIO_OBJECT_TYPE coreCodec,
                                     const MP4_ELEMENT_ID elementID, const int elementIndex,
                                     const UCHAR harmonicSBR, const UCHAR stereoConfigIndex,
                                     const UCHAR configMode, UCHAR *configChanged,
                                     const INT downscaleFactor) {
  return SBRDEC_UNSUPPORTED_CONFIG;
}
SBR_ERROR stb_sbrDecoder_FreeMem(HANDLE_SBRDECODER *self) { return SBRDEC_OK; }
INT stb_sbrDecoder_Header(HANDLE_SBRDECODER self, HANDLE_FDK_BITSTREAM hBs,
                          const INT sampleRateIn, const INT sampleRateOut,
                          const INT samplesPerFrame, const AUDIO_OBJECT_TYPE coreCodec,
                          const MP4_ELEMENT_ID elementID, const INT elementIndex,
                          const UCHAR harmonicSBR, const UCHAR stereoConfigIndex,
                          const UCHAR configMode, UCHAR *configChanged,
                          const INT downscaleFactor) {
  return SBRDEC_UNSUPPORTED_CONFIG;
}
SBR_ERROR stb_sbrDecoder_SetParam(HANDLE_SBRDECODER self, const SBRDEC_PARAM param,
                                  const INT value) {
  switch (param) {
    case SBR_SYSTEM_BITSTREAM_DELAY:
      if (value < 0 || value > 1) return SBRDEC_SET_PARAM_FAIL;
      return self == NULL ? SBRDEC_NOT_INITIALIZED : SBRDEC_OK;
    case SBR_QMF_MODE:
    case SBR_LD_QMF_TIME_ALIGN:
    case SBR_BS_INTERRUPTION:
    case SBR_SKIP_QMF:
      return self == NULL ? SBRDEC_NOT_INITIALIZED : SBRDEC_OK;
    case SBR_FLUSH_DATA:
    case SBR_CLEAR_HISTORY:
      return (value != 0 && self == NULL) ? SBRDEC_NOT_INITIALIZED : SBRDEC_OK;
    default:
      return SBRDEC_SET_PARAM_FAIL;
  }
}
SBR_ERROR stb_sbrDecoder_drcFeedChannel(HANDLE_SBRDECODER self, INT ch, UINT numBands,
                                        FIXP_DBL *pNextFact_mag, INT nextFact_exp,
                                        SHORT drcInterpolationScheme, UCHAR winSequence,
                                        USHORT *pBandTop) {
  return SBRDEC_NOT_INITIALIZED;
}
void stb_sbrDecoder_drcDisable(HANDLE_SBRDECODER self, INT ch) {}
SBR_ERROR stb_sbrDecoder_Parse(HANDLE_SBRDECODER self, HANDLE_FDK_BITSTREAM hBs,
                               UCHAR *pDrmBsBuffer, USHORT drmBsBufferSize, int *count,
                               int bsPayLen, int crcFlag, MP4_ELEMENT_ID prev_element,
                               int element_index, UINT acFlags, UINT acElFlags[]) {
  return SBRDEC_NOT_INITIALIZED;
}
SBR_ERROR stb_sbrDecoder_Apply(HANDLE_SBRDECODER self, LONG *input, LONG *timeData,
                               const int timeDataSize, int *numChannels, int *sampleRate,
                               const FDK_channelMapDescr *const mapDescr, const int mapIdx,
                               const int coreDecodedOk, UCHAR *psDecoded,
                               const INT inDataHeadroom, INT *outDataHeadroom) {
  return SBRDEC_NOT_INITIALIZED;
}
SBR_ERROR stb_sbrDecoder_Close(HANDLE_SBRDECODER *self) {
  free(*self);
  *self = NULL;
  return SBRDEC_OK;
}
UINT stb_sbrDecoder_GetDelay(const HANDLE_SBRDECODER self) { return 0; }

SACDEC_ERROR stb_mpegSurroundDecoder_Open(CMpegSurroundDecoder **p, int stereoConfigIndex,
                                          HANDLE_FDK_QMF_DOMAIN pQmfDomain) {
  if (*p == NULL) *p = (CMpegSurroundDecoder *)calloc(1, 64);
  return MPS_OK;
}
SAC_INSTANCE_AVAIL stb_mpegSurroundDecoder_IsFull(CMpegSurroundDecoder *p) {
  return SAC_INSTANCE_NOT_FULL_AVAILABLE;
}
SACDEC_ERROR stb_mpegSurroundDecoder_Config(CMpegSurroundDecoder *p, HANDLE_FDK_BITSTREAM hBs,
                                            AUDIO_OBJECT_TYPE coreCodec, INT samplingRate,
                                            INT frameSize, INT numChannels,
                                            INT stereoConfigIndex,
                                            INT coreSbrFrameLengthIndex, INT configBytes,
                                            const UCHAR configMode, UCHAR *configChanged) {
  return MPS_UNSUPPORTED_CONFIG;
}
SACDEC_ERROR stb_mpegSurroundDecoder_ConfigureQmfDomain(CMpegSurroundDecoder *p,
                                                        SAC_INPUT_CONFIG sac_dec_interface,
                                                        UINT coreSamplingRate,
                                                        AUDIO_OBJECT_TYPE coreCodec) {
  return MPS_NOTOK;
}
int stb_mpegSurroundDecoder_ParseNoHeader(CMpegSurroundDecoder *p, HANDLE_FDK_BITSTREAM hBs,
                                          int *pMpsDataBits, int fGlobalIndependencyFlag) {
  return MPS_NOTOK;
}
int stb_mpegSurroundDecoder_Parse(CMpegSurroundDecoder *p, HANDLE_FDK_BITSTREAM hBs,
                                  int *pMpsDataBits, AUDIO_OBJECT_TYPE coreCodec,
                                  int sampleRate, int frameSize,
                                  int fGlobalIndependencyFlag) {
  return MPS_NOTOK;
}
int stb_mpegSurroundDecoder_Apply(CMpegSurroundDecoder *p, PCM_MPS *input, PCM_MPS *pTimeData,
                                  const int timeDataSize, int timeDataFrameSize,
                                  int *nChannels, int *frameSize, int sampleRate,
                                  AUDIO_OBJECT_TYPE coreCodec,
                                  AUDIO_CHANNEL_TYPE channelType[], UCHAR channelIndices[],
                                  const FDK_channelMapDescr *const mapDescr,
                                  const INT inDataHeadroom, INT *outDataHeadroom) {
  return MPS_NOTOK;
}
void stb_mpegSurroundDecoder_Close(CMpegSurroundDecoder *p) { free(p); }
SACDEC_ERROR stb_mpegSurroundDecoder_FreeMem(CMpegSurroundDecoder *p) { return MPS_OK; }
SACDEC_ERROR stb_mpegSurroundDecoder_SetParam(CMpegSurroundDecoder *p, const SACDEC_PARAM param,
                                              const INT value) {
  SACDEC_ERROR err = p ? MPS_OK : MPS_INVALID_HANDLE;
  switch (param) {
    case SACDEC_OUTPUT_MODE:
      if (value != SACDEC_OUT_MODE_NORMAL && value != SACDEC_OUT_MODE_STEREO)
        err = MPS_INVALID_PARAMETER;
      return err;
    case SACDEC_INTERFACE:
      if (value < 0 || value > 1) err = MPS_INVALID_PARAMETER;
      return err;
    case SACDEC_BS_INTERRUPTION:
    case SACDEC_CLEAR_HISTORY:
      return err;
    case SACDEC_CONCEAL_NUM_KEEP_FRAMES:
    case SACDEC_CONCEAL_FADE_OUT_SLOPE_LENGTH:
    case SACDEC_CONCEAL_FADE_IN_SLOPE_LENGTH:
    case SACDEC_CONCEAL_NUM_RELEASE_FRAMES:
      if (value < 0) err = MPS_INVALID_PARAMETER;
      return err;
    default:
      return MPS_INVALID_PARAMETER;
  }
}
UINT stb_mpegSurroundDecoder_GetDelay(const CMpegSurroundDecoder *self) { return 0; }
SACDEC_ERROR stb_mpegSurroundDecoder_IsPseudoLR(CMpegSurroundDecoder *p, int *bsPseudoLr) {
  if (p == NULL) return MPS_INVALID_HANDLE;
  *bsPseudoLr = 0;
  return MPS_OK;
}

/* ---- API --------------------------------------------------------------------------------- */

extern "C" {

void *cref_aacdecoder_stub_open(INT tt, UINT layers) {
  return (void *)stb_aacDecoder_Open((TRANSPORT_TYPE)tt, layers);
}

void cref_aacdecoder_stub_close(void *h) { stb_aacDecoder_Close((HANDLE_AACDECODER)h); }

INT cref_aacdecoder_stub_config_raw(void *h, UCHAR *conf, UINT len) {
  UCHAR *c[1] = {conf};
  UINT l[1] = {len};
  return stb_aacDecoder_ConfigRaw((HANDLE_AACDECODER)h, c, l);
}

INT cref_aacdecoder_stub_fill(void *h, UCHAR *buf, UINT size, UINT *valid) {
  UCHAR *b[1] = {buf};
  UINT s[1] = {size};
  return stb_aacDecoder_Fill((HANDLE_AACDECODER)h, b, s, valid);
}

INT cref_aacdecoder_stub_decode(void *h, SHORT *out, INT size, UINT flags) {
  return stb_aacDecoder_DecodeFrame((HANDLE_AACDECODER)h, out, size, flags);
}

INT cref_aacdecoder_stub_set_param(void *h, INT param, INT value) {
  return stb_aacDecoder_SetParam((HANDLE_AACDECODER)h, (AACDEC_PARAM)param, value);
}

INT cref_aacdecoder_stub_free_bytes(void *h, UINT *free_bytes) {
  return stb_aacDecoder_GetFreeBytes((HANDLE_AACDECODER)h, free_bytes);
}

INT cref_aacdecoder_stub_stream_info(void *h, INT *o) {
  CStreamInfo *si = stb_aacDecoder_GetStreamInfo((HANDLE_AACDECODER)h);
  INT n = 0, c;
  if (si == NULL) return 0;
  o[n++] = si->sampleRate;
  o[n++] = si->frameSize;
  o[n++] = si->numChannels;
  for (c = 0; c < 8; c++) o[n++] = si->pChannelType ? si->pChannelType[c] : -99;
  for (c = 0; c < 8; c++) o[n++] = si->pChannelIndices ? si->pChannelIndices[c] : -99;
  o[n++] = si->aacSampleRate;
  o[n++] = si->profile;
  o[n++] = si->aot;
  o[n++] = si->channelConfig;
  o[n++] = si->bitRate;
  o[n++] = si->aacSamplesPerFrame;
  o[n++] = si->aacNumChannels;
  o[n++] = si->extAot;
  o[n++] = si->extSamplingRate;
  o[n++] = (INT)si->outputDelay;
  o[n++] = (INT)si->flags;
  o[n++] = si->epConfig;
  o[n++] = si->numLostAccessUnits;
  o[n++] = (INT)si->numTotalBytes;
  o[n++] = (INT)(si->numTotalBytes >> 32);
  o[n++] = (INT)si->numBadBytes;
  o[n++] = (INT)(si->numBadBytes >> 32);
  o[n++] = (INT)si->numTotalAccessUnits;
  o[n++] = (INT)(si->numTotalAccessUnits >> 32);
  o[n++] = (INT)si->numBadAccessUnits;
  o[n++] = (INT)(si->numBadAccessUnits >> 32);
  o[n++] = si->drcProgRefLev;
  o[n++] = si->drcPresMode;
  o[n++] = si->outputLoudness;
  return n;
}

INT cref_aacdecoder_stub_anc_init(void *h, UCHAR *buf, INT size) {
  return stb_aacDecoder_AncDataInit((HANDLE_AACDECODER)h, buf, size);
}

INT cref_aacdecoder_stub_anc_get(void *h, INT index, UCHAR *base, INT *offset, INT *size) {
  UCHAR *p = NULL;
  INT r = stb_aacDecoder_AncDataGet((HANDLE_AACDECODER)h, index, &p, size);
  *offset = p ? (INT)(p - base) : -1;
  return r;
}

void cref_aacdecoder_stub_poison(void *h, UCHAR pattern) {
  AAC_DECODER_INSTANCE *s = (AAC_DECODER_INSTANCE *)h;
  if (s->workBufferCore1) memset(s->workBufferCore1, pattern, sizeof(CWorkBufferCore1));
  if (s->workBufferCore2) memset(s->workBufferCore2, pattern, (8) * 1024 * sizeof(FIXP_DBL));
  /* timeData2Size (GetRequiredMemWorkBufferCore5()) includes the alignment overhead of the
     allocation: the buffer itself has (8) * (1024 * 4) * 2 samples. */
  if (s->pTimeData2) memset(s->pTimeData2, pattern, (8) * (1024 * 4) * 2 * sizeof(PCM_DEC));
  if (s->pAacDecoderChannelInfo[0] && s->pAacDecoderChannelInfo[0]->pComStaticData &&
      s->pAacDecoderChannelInfo[0]->pComStaticData->pWorkBufferCore1)
    memset(s->pAacDecoderChannelInfo[0]->pComStaticData->pWorkBufferCore1, pattern,
           sizeof(CWorkBufferCore1));
}

/* Copies the first n values of the decoder's pTimeData2. */
void cref_aacdecoder_stub_time_data2(void *h, INT *out, INT n) {
  AAC_DECODER_INSTANCE *s = (AAC_DECODER_INSTANCE *)h;
  memcpy(out, s->pTimeData2, n * sizeof(INT));
}

/* CCE check counters: CCE reads checked, and those whose result depended on the pattern. */
void cref_aacdecoder_stub_cce_stats(INT *reads, INT *pattern_dependent) {
  *reads = g_cce_reads;
  *pattern_dependent = g_cce_pattern_dependent;
}
}
