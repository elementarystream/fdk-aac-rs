// Shims for the end-to-end tests of fdk_aac_rs_sacdec::sac_dec_lib: the reference AAC decoder
// decodes real MPEG Surround streams while the calls of its MPEG Surround decoder are recorded.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacdec_sac_dec_lib_e2e (see build.rs).
//
// aacdecoder.cpp and aacdecoder_lib.cpp are included here with their external functions
// renamed to sacx_* (the same code as the library's, compiled with the same flags) and with the
// mpegSurroundDecoder_* functions they call replaced by the capture functions sacx_mps_* of
// shim/sacdec_sac_dec_lib.cpp, which call the reference MPEG Surround decoder and record each
// call (cref_saclib_capture() switches the recording on for the calling thread).

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
#define aacDecoder_AncDataGet sacx_aacDecoder_AncDataGet
#define aacDecoder_AncDataInit sacx_aacDecoder_AncDataInit
#define aacDecoder_Close sacx_aacDecoder_Close
#define aacDecoder_ConfigRaw sacx_aacDecoder_ConfigRaw
#define aacDecoder_DecodeFrame sacx_aacDecoder_DecodeFrame
#define aacDecoder_Fill sacx_aacDecoder_Fill
#define aacDecoder_GetFreeBytes sacx_aacDecoder_GetFreeBytes
#define aacDecoder_GetLibInfo sacx_aacDecoder_GetLibInfo
#define aacDecoder_GetStreamInfo sacx_aacDecoder_GetStreamInfo
#define aacDecoder_Open sacx_aacDecoder_Open
#define aacDecoder_RawISOBMFFData sacx_aacDecoder_RawISOBMFFData
#define aacDecoder_SetParam sacx_aacDecoder_SetParam

#define CAacDecoder_AncDataGet sacx_CAacDecoder_AncDataGet
#define CAacDecoder_AncDataInit sacx_CAacDecoder_AncDataInit
#define CAacDecoder_ApplyCrossFade sacx_CAacDecoder_ApplyCrossFade
#define CAacDecoder_Close sacx_CAacDecoder_Close
#define CAacDecoder_CtrlCFGChange sacx_CAacDecoder_CtrlCFGChange
#define CAacDecoder_DecodeFrame sacx_CAacDecoder_DecodeFrame
#define CAacDecoder_FreeMem sacx_CAacDecoder_FreeMem
#define CAacDecoder_GetStreamInfo sacx_CAacDecoder_GetStreamInfo
#define CAacDecoder_Init sacx_CAacDecoder_Init
#define CAacDecoder_Open sacx_CAacDecoder_Open
#define CAacDecoder_PrepareCrossFade sacx_CAacDecoder_PrepareCrossFade
#define CAacDecoder_PreRollExtensionPayloadParse sacx_CAacDecoder_PreRollExtensionPayloadParse
#define CAacDecoder_SignalInterruption sacx_CAacDecoder_SignalInterruption
#define CAacDecoder_SyncQmfMode sacx_CAacDecoder_SyncQmfMode

#define mpegSurroundDecoder_IsFullMpegSurroundDecoderInstanceAvailable sacx_mps_IsFull
#define mpegSurroundDecoder_Open sacx_mps_Open
#define mpegSurroundDecoder_Init sacx_mps_Init
#define mpegSurroundDecoder_Config sacx_mps_Config
#define mpegSurroundDecoder_ConfigureQmfDomain sacx_mps_ConfigureQmfDomain
#define mpegSurroundDecoder_ParseNoHeader sacx_mps_ParseNoHeader
#define mpegSurroundDecoder_Parse sacx_mps_Parse
#define mpegSurroundDecoder_Apply sacx_mps_Apply
#define mpegSurroundDecoder_Close sacx_mps_Close
#define mpegSurroundDecoder_FreeMem sacx_mps_FreeMem
#define mpegSurroundDecoder_SetParam sacx_mps_SetParam
#define mpegSurroundDecoder_GetLibInfo sacx_mps_GetLibInfo
#define mpegSurroundDecoder_GetDelay sacx_mps_GetDelay
#define mpegSurroundDecoder_IsPseudoLR sacx_mps_IsPseudoLR

#include <stdlib.h>
#include "cref_common.h"
#include "libAACdec/src/aacdecoder.cpp"
#include "libAACdec/src/aacdecoder_lib.cpp"
#include "aacenc_lib.h"

extern "C" {

/* --- the decoder ------------------------------------------------------------------------- */

void *cref_sacx_open(INT transportFmt) {
  return sacx_aacDecoder_Open((TRANSPORT_TYPE)transportFmt, 1);
}
INT cref_sacx_config_raw(void *h, UCHAR *conf, UINT len) {
  UCHAR *c[1] = {conf};
  UINT l[1] = {len};
  return sacx_aacDecoder_ConfigRaw((HANDLE_AACDECODER)h, c, l);
}
INT cref_sacx_set_param(void *h, INT param, INT value) {
  return sacx_aacDecoder_SetParam((HANDLE_AACDECODER)h, (AACDEC_PARAM)param, value);
}
/* Fills the access unit (if any) and decodes a frame. info: numChannels, frameSize,
   sampleRate, aot, outputDelay. */
INT cref_sacx_decode(void *h, UCHAR *au, UINT len, SHORT *pcm, INT pcmSize, UINT flags, INT *info) {
  HANDLE_AACDECODER d = (HANDLE_AACDECODER)h;
  if (au != NULL) {
    UCHAR *b[1] = {au};
    UINT sz[1] = {len};
    UINT valid = len;
    INT e = sacx_aacDecoder_Fill(d, b, sz, &valid);
    if (e != AAC_DEC_OK) return e;
  }
  INT err = sacx_aacDecoder_DecodeFrame(d, (INT_PCM *)pcm, pcmSize, flags);
  CStreamInfo *si = sacx_aacDecoder_GetStreamInfo(d);
  info[0] = si->numChannels;
  info[1] = si->frameSize;
  info[2] = si->sampleRate;
  info[3] = si->aot;
  info[4] = (INT)si->outputDelay;
  return err;
}
void cref_sacx_close(void *h) { sacx_aacDecoder_Close((HANDLE_AACDECODER)h); }

/* --- an ELDv2 (LD MPEG Surround) encoder of the reference -------------------------------- */

/* Opens an encoder: AOT_ER_AAC_ELD, MODE_212, raw access units; sbr 0/1. Returns NULL on
   failure. */
void *cref_sacx_enc_open(INT sampleRate, INT bitRate, INT sbr) {
  HANDLE_AACENCODER e = NULL;
  if (aacEncOpen(&e, 0, 2) != AACENC_OK) return NULL;
  INT ok = 1;
  ok &= aacEncoder_SetParam(e, AACENC_AOT, AOT_ER_AAC_ELD) == AACENC_OK;
  ok &= aacEncoder_SetParam(e, AACENC_CHANNELMODE, MODE_212) == AACENC_OK;
  ok &= aacEncoder_SetParam(e, AACENC_SAMPLERATE, sampleRate) == AACENC_OK;
  ok &= aacEncoder_SetParam(e, AACENC_BITRATE, bitRate) == AACENC_OK;
  ok &= aacEncoder_SetParam(e, AACENC_SBR_MODE, sbr) == AACENC_OK;
  ok &= aacEncoder_SetParam(e, AACENC_TRANSMUX, TT_MP4_RAW) == AACENC_OK;
  ok &= aacEncEncode(e, NULL, NULL, NULL, NULL) == AACENC_OK;
  if (!ok) {
    aacEncClose(&e);
    return NULL;
  }
  return e;
}
/* The AudioSpecificConfig (returns its length) and the frame length. */
INT cref_sacx_enc_info(void *h, UCHAR *conf, INT *frameLength) {
  AACENC_InfoStruct info;
  if (aacEncInfo((HANDLE_AACENCODER)h, &info) != AACENC_OK) return -1;
  FDKmemcpy(conf, info.confBuf, info.confSize);
  *frameLength = (INT)info.frameLength;
  return (INT)info.confSize;
}
/* Encodes interleaved stereo samples (n samples per channel; NULL flushes). Returns the size
   of the access unit (0: none), or -1. */
INT cref_sacx_enc_encode(void *h, SHORT *pcm, INT n, UCHAR *out, INT outSize) {
  AACENC_BufDesc in = {0}, ob = {0};
  AACENC_InArgs ia = {0};
  AACENC_OutArgs oa = {0};
  void *inPtr = pcm;
  INT inId = IN_AUDIO_DATA, inSize = 2 * n * (INT)sizeof(SHORT), inEl = (INT)sizeof(SHORT);
  void *outPtr = out;
  INT outId = OUT_BITSTREAM_DATA, outEl = 1;
  in.numBufs = 1;
  in.bufs = &inPtr;
  in.bufferIdentifiers = &inId;
  in.bufSizes = &inSize;
  in.bufElSizes = &inEl;
  ob.numBufs = 1;
  ob.bufs = &outPtr;
  ob.bufferIdentifiers = &outId;
  ob.bufSizes = &outSize;
  ob.bufElSizes = &outEl;
  ia.numInSamples = pcm == NULL ? -1 : 2 * n;
  AACENC_ERROR e = aacEncEncode((HANDLE_AACENCODER)h, &in, &ob, &ia, &oa);
  if (e == AACENC_ENCODE_EOF) return 0;
  if (e != AACENC_OK) return -1;
  return oa.numOutBytes;
}
void cref_sacx_enc_close(void *h) {
  HANDLE_AACENCODER e = (HANDLE_AACENCODER)h;
  aacEncClose(&e);
}
}
