// Prelude of the namespaced copy of libAACdec used by the SBR capture harness
// (sbrdec_e2e.cpp). Included by every sbrdec_capdec_*.cpp shim before the libAACdec source it
// compiles into `namespace capdec`; compiled on its own it defines nothing.
//
// The external headers that libAACdec uses are included here at global scope, so that their
// include guards make the includes inside `namespace capdec` empty; the headers of libAACdec
// itself are included inside the namespace. The calls of libAACdec into the SBR decoder API and
// into the QMF domain functions that configure the domain are renamed to the capture wrappers
// of sbrdec_e2e.cpp (sbrcap_*, qmfcap_*), and the public API of the copy to capdec_aacDecoder_*.

#ifndef FDK_AAC_RS_SBRDEC_CAPDEC_PRELUDE
#define FDK_AAC_RS_SBRDEC_CAPDEC_PRELUDE

#include "cref_common.h"

#include "ac_arith_coder.h"
#include "common_fix.h"
#include "dct.h"
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "FDK_core.h"
#include "FDK_crc.h"
#include "FDK_drcDecLib.h"
#include "FDK_lpc.h"
#include "FDK_qmf_domain.h"
#include "FDK_tools_rom.h"
#include "FDK_trigFcts.h"
#include "fft.h"
#include "genericStds.h"
#include "limiter.h"
#include "machine_type.h"
#include "mdct.h"
#include "pcm_utils.h"
#include "pcmdmx_lib.h"
#include "sac_dec_lib.h"
#include "sbrdecoder.h"
#include "scale.h"
#include "tp_data.h"
#include "tpdec_lib.h"

/* Capture wrappers (defined in sbrdec_e2e.cpp). */
SBR_ERROR sbrcap_Open(HANDLE_SBRDECODER *pSelf, HANDLE_FDK_QMF_DOMAIN pQmfDomain);
SBR_ERROR sbrcap_InitElement(HANDLE_SBRDECODER self, const int sampleRateIn,
                             const int sampleRateOut, const int samplesPerFrame,
                             const AUDIO_OBJECT_TYPE coreCodec, const MP4_ELEMENT_ID elementID,
                             const int elementIndex, const UCHAR harmonicSBR,
                             const UCHAR stereoConfigIndex, const UCHAR configMode,
                             UCHAR *configChanged, const INT downscaleFactor);
SBR_ERROR sbrcap_FreeMem(HANDLE_SBRDECODER *self);
INT sbrcap_Header(HANDLE_SBRDECODER self, HANDLE_FDK_BITSTREAM hBs, const INT sampleRateIn,
                  const INT sampleRateOut, const INT samplesPerFrame,
                  const AUDIO_OBJECT_TYPE coreCodec, const MP4_ELEMENT_ID elementID,
                  const INT elementIndex, const UCHAR harmonicSBR,
                  const UCHAR stereoConfigIndex, const UCHAR configMode, UCHAR *configChanged,
                  const INT downscaleFactor);
SBR_ERROR sbrcap_SetParam(HANDLE_SBRDECODER self, const SBRDEC_PARAM param, const INT value);
SBR_ERROR sbrcap_drcFeedChannel(HANDLE_SBRDECODER self, INT ch, UINT numBands,
                                FIXP_DBL *pNextFact_mag, INT nextFact_exp,
                                SHORT drcInterpolationScheme, UCHAR winSequence,
                                USHORT *pBandTop);
void sbrcap_drcDisable(HANDLE_SBRDECODER self, INT ch);
SBR_ERROR sbrcap_Parse(HANDLE_SBRDECODER self, HANDLE_FDK_BITSTREAM hBs, UCHAR *pDrmBsBuffer,
                       USHORT drmBsBufferSize, int *count, int bsPayLen, int crcFlag,
                       MP4_ELEMENT_ID prev_element, int element_index, UINT acFlags,
                       UINT acElFlags[]);
SBR_ERROR sbrcap_Apply(HANDLE_SBRDECODER self, LONG *input, LONG *timeData,
                       const int timeDataSize, int *numChannels, int *sampleRate,
                       const FDK_channelMapDescr *const mapDescr, const int mapIdx,
                       const int coreDecodedOk, UCHAR *psDecoded, const INT inDataHeadroom,
                       INT *outDataHeadroom);
SBR_ERROR sbrcap_Close(HANDLE_SBRDECODER *self);
INT sbrcap_GetLibInfo(LIB_INFO *info);
UINT sbrcap_GetDelay(const HANDLE_SBRDECODER self);
void qmfcap_ClearRequested(HANDLE_FDK_QMF_DOMAIN_GC hgc);
QMF_DOMAIN_ERROR qmfcap_Configure(HANDLE_FDK_QMF_DOMAIN hqd);
void qmfcap_FreeMem(HANDLE_FDK_QMF_DOMAIN hqd);
QMF_DOMAIN_ERROR qmfcap_ClearPersistentMemory(HANDLE_FDK_QMF_DOMAIN hqd);
void qmfcap_Close(HANDLE_FDK_QMF_DOMAIN hqd);

#define sbrDecoder_Open sbrcap_Open
#define sbrDecoder_InitElement sbrcap_InitElement
#define sbrDecoder_FreeMem sbrcap_FreeMem
#define sbrDecoder_Header sbrcap_Header
#define sbrDecoder_SetParam sbrcap_SetParam
#define sbrDecoder_drcFeedChannel sbrcap_drcFeedChannel
#define sbrDecoder_drcDisable sbrcap_drcDisable
#define sbrDecoder_Parse sbrcap_Parse
#define sbrDecoder_Apply sbrcap_Apply
#define sbrDecoder_Close sbrcap_Close
#define sbrDecoder_GetLibInfo sbrcap_GetLibInfo
#define sbrDecoder_GetDelay sbrcap_GetDelay
#define FDK_QmfDomain_ClearRequested qmfcap_ClearRequested
#define FDK_QmfDomain_Configure qmfcap_Configure
#define FDK_QmfDomain_FreeMem qmfcap_FreeMem
#define FDK_QmfDomain_ClearPersistentMemory qmfcap_ClearPersistentMemory
#define FDK_QmfDomain_Close qmfcap_Close

/* The public API of the copy (renamed to cref_* by the prefix header). */
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
#define aacDecoder_AncDataGet capdec_aacDecoder_AncDataGet
#define aacDecoder_AncDataInit capdec_aacDecoder_AncDataInit
#define aacDecoder_Close capdec_aacDecoder_Close
#define aacDecoder_ConfigRaw capdec_aacDecoder_ConfigRaw
#define aacDecoder_DecodeFrame capdec_aacDecoder_DecodeFrame
#define aacDecoder_Fill capdec_aacDecoder_Fill
#define aacDecoder_GetFreeBytes capdec_aacDecoder_GetFreeBytes
#define aacDecoder_GetLibInfo capdec_aacDecoder_GetLibInfo
#define aacDecoder_GetStreamInfo capdec_aacDecoder_GetStreamInfo
#define aacDecoder_Open capdec_aacDecoder_Open
#define aacDecoder_RawISOBMFFData capdec_aacDecoder_RawISOBMFFData
#define aacDecoder_SetParam capdec_aacDecoder_SetParam

#endif
