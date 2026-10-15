// Shims for the differential tests of the file-static functions and tables of
// fdk_aac_rs_aacenc::aacenc_lib. Every `cref_*` function defined at the start of a line is bound
// in fdk_aac_rs_cref::aacenc_aacenc_lib_static (see build.rs).
//
// aacenc_lib.cpp is included at file scope (struct AACENCODER is defined there and must match
// the handle type of aacenc_lib.h); its non-static functions get other names in this copy.

#include "cref_common.h"

#define cref_aacEncOpen st_aacEncOpen
#define cref_aacEncClose st_aacEncClose
#define cref_aacEncEncode st_aacEncEncode
#define cref_aacEncInfo st_aacEncInfo
#define cref_aacEncoder_SetParam st_aacEncoder_SetParam
#define cref_aacEncoder_GetParam st_aacEncoder_GetParam
#define cref_aacEncGetLibInfo st_aacEncGetLibInfo
#define aacEncDefaultConfig st_aacEncDefaultConfig
#define aacenc_SscCallback st_aacenc_SscCallback
#define Get_AacEncoder st_Get_AacEncoder
#define Free_AacEncoder st_Free_AacEncoder
#define GetRequiredMem_AacEncoder st_GetRequiredMem_AacEncoder
#include "libAACenc/src/aacenc_lib.cpp"

extern "C" {
/* eldSbrAutoConfigTab as UINTs: nChannels, samplingRate, bitrateRange, sbrMode, chMode per
   entry; returns the number of entries. */
INT cref_aacenc_lib_static_eld_tab(UINT *out) {
  INT n = (INT)(sizeof(eldSbrAutoConfigTab) / sizeof(eldSbrAutoConfigTab[0]));
  for (INT i = 0; i < n; i++) {
    out[5 * i + 0] = eldSbrAutoConfigTab[i].nChannels;
    out[5 * i + 1] = eldSbrAutoConfigTab[i].samplingRate;
    out[5 * i + 2] = eldSbrAutoConfigTab[i].bitrateRange;
    out[5 * i + 3] = eldSbrAutoConfigTab[i].sbrMode;
    out[5 * i + 4] = (UINT)eldSbrAutoConfigTab[i].chMode;
  }
  return n;
}
/* eldSbrConfigurator: the index of the entry, -1 for NULL */
INT cref_aacenc_lib_static_eld_configurator(UINT nChannels, UINT samplingRate, UINT bitrate) {
  const ELD_SBR_CONFIGURATOR *p = eldSbrConfigurator(nChannels, samplingRate, bitrate);
  return p ? (INT)(p - eldSbrAutoConfigTab) : -1;
}
/* getSbrSignalingMode */
INT cref_aacenc_lib_static_signaling(INT aot, INT tt, INT signaling, UINT sbrRatio) {
  return getSbrSignalingMode((AUDIO_OBJECT_TYPE)aot, (TRANSPORT_TYPE)tt, (UCHAR)signaling,
                             sbrRatio);
}
/* GetCoreChannelMode, isPsActive */
INT cref_aacenc_lib_static_core_mode(INT mode, INT aot) {
  return GetCoreChannelMode((CHANNEL_MODE)mode, (AUDIO_OBJECT_TYPE)aot) |
         (isPsActive((AUDIO_OBJECT_TYPE)aot) << 16);
}
/* isSbrActive and FDKaacEnc_GetCBRBitrate on an AACENC_CONFIG (the port's layout) */
INT cref_aacenc_lib_static_cbr(const void *config, INT userSbrRatio, INT *sbrActive) {
  *sbrActive = isSbrActive((HANDLE_AACENC_CONFIG)config);
  return FDKaacEnc_GetCBRBitrate((HANDLE_AACENC_CONFIG)config, userSbrRatio);
}
/* aacEncoder_LimitBitrate(NULL, ...) */
INT cref_aacenc_lib_static_limit(INT samplingRate, INT frameLength, INT nChannels, INT channelMode, INT bitRate, INT nSubFrames, INT sbrActive, INT sbrDownSampleRate, UINT syntaxFlags, INT aot) {
  return aacEncoder_LimitBitrate(NULL, samplingRate, frameLength, nChannels,
                                 (CHANNEL_MODE)channelMode, bitRate, nSubFrames, sbrActive,
                                 sbrDownSampleRate, syntaxFlags, (AUDIO_OBJECT_TYPE)aot);
}
/* aacEncDistributeSbrBits(cm, sbrElInfo, bitRate) for the channel mapping of (mode, order);
   per element: ChannelIndex[0], ChannelIndex[1], elType, bitRate, instanceTag, nChannelsInEl,
   fParametricStereo, fDualMono (8 INTs); returns nElements. */
INT cref_aacenc_lib_static_distribute(INT mode, INT order, INT bitRate, INT *out) {
  CHANNEL_MAPPING cm;
  memset(&cm, 0, sizeof(cm));
  FDKaacEnc_InitChannelMapping((CHANNEL_MODE)mode, (CHANNEL_ORDER)order, &cm);
  SBR_ELEMENT_INFO el[8];
  memset(el, 0x5a, sizeof(el));
  aacEncDistributeSbrBits(&cm, el, bitRate);
  for (INT i = 0; i < cm.nElements; i++) {
    out[8 * i + 0] = el[i].ChannelIndex[0];
    out[8 * i + 1] = el[i].ChannelIndex[1];
    out[8 * i + 2] = el[i].elType;
    out[8 * i + 3] = el[i].bitRate;
    out[8 * i + 4] = el[i].instanceTag;
    out[8 * i + 5] = el[i].nChannelsInEl;
    out[8 * i + 6] = el[i].fParametricStereo;
    out[8 * i + 7] = el[i].fDualMono;
  }
  return cm.nElements;
}
/* FDKaacEnc_MapConfig(cc, extCfg, sbrSignaling, config): extCfg fields
   u = {userSamplerate, userTpNsubFrames, userTpProtection, userTpHeaderPeriod, userTpType,
   userPceAdditions}; the CODER_CONFIG as INTs: aot, extAOT, channelMode, channelConfigZero,
   samplingRate, extSamplingRate, downscaleSamplingRate, bitRate, samplesPerFrame,
   noChannels, bitsFrame, nSubFrames, flags, matrixMixdownA, headerPeriod, sbrSignaling,
   sbrPresent, psPresent (18). */
void cref_aacenc_lib_static_map_config(const UINT *u, INT sbrSignaling, const void *config, INT *out) {
  USER_PARAM p;
  memset(&p, 0, sizeof(p));
  p.userSamplerate = u[0];
  p.userTpNsubFrames = (UCHAR)u[1];
  p.userTpProtection = (UCHAR)u[2];
  p.userTpHeaderPeriod = (UCHAR)u[3];
  p.userTpType = (TRANSPORT_TYPE)u[4];
  p.userPceAdditions = u[5];
  CODER_CONFIG cc;
  memset(&cc, 0x5a, sizeof(cc));
  FDKaacEnc_MapConfig(&cc, &p, (SBR_PS_SIGNALING)sbrSignaling, (HANDLE_AACENC_CONFIG)config);
  INT k = 0;
  out[k++] = cc.aot;
  out[k++] = cc.extAOT;
  out[k++] = cc.channelMode;
  out[k++] = cc.channelConfigZero;
  out[k++] = cc.samplingRate;
  out[k++] = cc.extSamplingRate;
  out[k++] = cc.downscaleSamplingRate;
  out[k++] = cc.bitRate;
  out[k++] = cc.samplesPerFrame;
  out[k++] = cc.noChannels;
  out[k++] = cc.bitsFrame;
  out[k++] = cc.nSubFrames;
  out[k++] = (INT)cc.flags;
  out[k++] = cc.matrixMixdownA;
  out[k++] = cc.headerPeriod;
  out[k++] = cc.sbrSignaling;
  out[k++] = cc.sbrPresent;
  out[k++] = cc.psPresent;
}
}
