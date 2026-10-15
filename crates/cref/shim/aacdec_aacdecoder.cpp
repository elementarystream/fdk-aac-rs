// Shims for the differential tests of fdk_aac_rs_aacdec::aacdecoder.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_aacdecoder (see build.rs).
//
// aacdecoder.cpp is compiled into this file with its global functions renamed to decs_*, so that
// its static helpers are reachable without clashing with the library.

#define CAacDecoder_AncDataGet decs_CAacDecoder_AncDataGet
#define CAacDecoder_AncDataInit decs_CAacDecoder_AncDataInit
#define CAacDecoder_ApplyCrossFade decs_CAacDecoder_ApplyCrossFade
#define CAacDecoder_Close decs_CAacDecoder_Close
#define CAacDecoder_CtrlCFGChange decs_CAacDecoder_CtrlCFGChange
#define CAacDecoder_DecodeFrame decs_CAacDecoder_DecodeFrame
#define CAacDecoder_FreeMem decs_CAacDecoder_FreeMem
#define CAacDecoder_GetStreamInfo decs_CAacDecoder_GetStreamInfo
#define CAacDecoder_Init decs_CAacDecoder_Init
#define CAacDecoder_Open decs_CAacDecoder_Open
#define CAacDecoder_PrepareCrossFade decs_CAacDecoder_PrepareCrossFade
#define CAacDecoder_PreRollExtensionPayloadParse decs_CAacDecoder_PreRollExtensionPayloadParse
#define CAacDecoder_SignalInterruption decs_CAacDecoder_SignalInterruption
#define CAacDecoder_SyncQmfMode decs_CAacDecoder_SyncQmfMode

#include "cref_common.h"
#include "common_fix.h"
#include "FDK_bitstream.h"
#include "channel.h"
#include "tpdec_lib.h"
#include "FDK_audio.h"
#include "block.h"
#include "genericStds.h"
#include "FDK_qmf_domain.h"
#include "sbrdecoder.h"
#include "aacdec_drc.h"
#include "pcmdmx_lib.h"
#include "FDK_drcDecLib.h"
#include "limiter.h"
#include "FDK_delay.h"
#include "aac_rom.h"
#include "aac_ram.h"
#include "aacdec_pns.h"
#include "sac_dec_lib.h"
#include "aacdec_hcr.h"
#include "rvlc.h"
#include "usacdec_lpd.h"
#include "ac_arith_coder.h"
#include "conceal.h"
#include "FDK_crc.h"
#include <stdlib.h>

#include "aacdecoder.h"
#include "libAACdec/src/aacdecoder.cpp"

extern "C" {

INT cref_dec_get_el_channels(INT type, INT sci) {
  return CAacDecoder_GetELChannels((MP4_ELEMENT_ID)type, (UCHAR)sci);
}

/* Ancillary data: init with (size), then parse `n` chunks of the bitstream buf (each anc_bytes
   values from the array `counts`), then report offsets[8], nrElements, buffer contents and the
   bits left. Returns the OR of the parse return values shifted per call (first 8 calls). */
INT cref_dec_anc(INT null_buffer, INT size, UCHAR *buf, INT len, INT *counts, INT n, UCHAR *anc, INT *offsets, INT *nr, INT *rets, INT *bits) {
  CAncData a;
  memset(&a, 0, sizeof(a));
  FDK_BITSTREAM bs;
  INT r = CAacDecoder_AncDataInit(&a, null_buffer ? NULL : anc, size);
  FDKinitBitStream(&bs, buf, (UINT)len, (UINT)len * 8, BS_READER);
  for (int i = 0; i < n; i++) rets[i] = CAacDecoder_AncDataParse(&a, &bs, counts[i]);
  for (int i = 0; i < 8; i++) offsets[i] = a.offset[i];
  *nr = a.nrElements;
  *bits = (INT)FDKgetValidBits(&bs);
  return r;
}

/* CAacDecoder_AncDataGet after parsing as in cref_dec_anc. */
INT cref_dec_anc_get(UCHAR *buf, INT len, INT *counts, INT n, UCHAR *anc, INT size, INT index, INT *off, INT *sz) {
  CAncData a;
  memset(&a, 0, sizeof(a));
  FDK_BITSTREAM bs;
  UCHAR *p = NULL;
  INT r;
  CAacDecoder_AncDataInit(&a, anc, size);
  FDKinitBitStream(&bs, buf, (UINT)len, (UINT)len * 8, BS_READER);
  for (int i = 0; i < n; i++) CAacDecoder_AncDataParse(&a, &bs, counts[i]);
  r = CAacDecoder_AncDataGet(&a, index, &p, sz);
  *off = p ? (INT)(p - anc) : -1;
  return r;
}

void cref_dec_cross_fade(INT apply, FIXP_DBL *td, FIXP_DBL *flush, INT numChannels, INT frameSize, INT interleaved) {
  PCM_DEC *f[8];
  for (int i = 0; i < 8; i++) f[i] = flush + i * TIME_DATA_FLUSH_SIZE;
  if (apply)
    CAacDecoder_ApplyCrossFade(td, f, numChannels, frameSize, interleaved);
  else
    CAacDecoder_PrepareCrossFade(td, f, numChannels, frameSize, interleaved);
}

/* CProgramConfigElement_Read on buf (with a raw transport decoder for the CRC calls) starting
   from the PCE given as bytes (CProgramConfig layout); returns the status, the PCE bytes and the
   bits left. */
INT cref_dec_pce_read(UCHAR *buf, INT len, UCHAR *pce, INT channelConfig, UINT anchor, INT *bits) {
  FDK_BITSTREAM bs;
  HANDLE_TRANSPORTDEC tp = transportDec_Open(TT_MP4_RAW, TP_FLAG_MPEG4, 1);
  INT r;
  FDKinitBitStream(&bs, buf, (UINT)len, (UINT)len * 8, BS_READER);
  r = CProgramConfigElement_Read(&bs, tp, (CProgramConfig *)pce, (UINT)channelConfig, anchor);
  *bits = (INT)FDKgetValidBits(&bs);
  transportDec_Close(&tp);
  return r;
}

/* validateElementInstanceTags with the element instance tags of n channels. */
INT cref_dec_validate_tags(UCHAR *pce, UCHAR *tags, INT nChannels, INT *elements, INT nElements) {
  CAacDecoderChannelInfo ci[8], *pci[8];
  MP4_ELEMENT_ID el[(3 * ((8) * 2) + (((8) * 2)) / 2 + 4 * (1) + 1)];
  for (int i = 0; i < 8; i++) {
    ci[i].ElementInstanceTag = tags[i];
    pci[i] = &ci[i];
  }
  for (int i = 0; i < nElements; i++) el[i] = (MP4_ELEMENT_ID)elements[i];
  return validateElementInstanceTags((CProgramConfig *)pce, pci, nChannels, el, nElements);
}

/* CStreamInfoInit on a zeroed struct, as the INT image of the shim of aacdecoder_lib. */
INT cref_dec_stream_info_init(INT *o) {
  CStreamInfo si;
  INT n = 0;
  memset(&si, 0, sizeof(si));
  CStreamInfoInit(&si);
  o[n++] = si.sampleRate;
  o[n++] = si.frameSize;
  o[n++] = si.numChannels;
  o[n++] = si.aacSampleRate;
  o[n++] = si.profile;
  o[n++] = si.aot;
  o[n++] = si.channelConfig;
  o[n++] = si.bitRate;
  o[n++] = si.aacSamplesPerFrame;
  o[n++] = si.aacNumChannels;
  o[n++] = si.extAot;
  o[n++] = si.extSamplingRate;
  o[n++] = (INT)si.outputDelay;
  o[n++] = (INT)si.flags;
  o[n++] = si.epConfig;
  o[n++] = si.drcProgRefLev;
  o[n++] = si.drcPresMode;
  o[n++] = si.outputLoudness;
  return n;
}
}

extern "C" {
/* CAacDecoder_Init of a USAC configuration with SBR (sbrRatioIndex 1..3) on a decoder opened with
   CAacDecoder_Open plus the downmix and MPEG-D DRC modules: the requested QMF domain
   configuration (map_sbrRatio_2_nAnaBands and the slot counts). out: err, configChanged,
   nInputChannels, nOutputChannels, nBandsAnalysis, nBandsSynthesis, nQmfTimeSlots,
   nQmfOvTimeSlots, nQmfProcBands, nQmfProcChannels, parkChannel, explicitConfig, aacChannels. */
void cref_dec_init_usac_sbr(INT sbrRatioIndex, INT elType, INT stereoConfigIndex, INT *out) {
  HANDLE_AACDECODER self = decs_CAacDecoder_Open(TT_MP4_RAW);
  CSAudioSpecificConfig *asc = (CSAudioSpecificConfig *)calloc(1, sizeof(*asc));
  UCHAR changed = 0;
  pcmDmx_Open(&self->hPcmUtils);
  FDK_drcDec_Open(&self->hUniDrcDecoder, DRC_DEC_ALL);
  asc->m_aot = AOT_USAC;
  asc->m_samplingFrequency = 48000;
  asc->m_samplingFrequencyIndex = 3;
  asc->m_samplesPerFrame = 1024;
  asc->m_channelConfiguration = elType == ID_USAC_CPE ? 2 : 1;
  asc->m_epConfig = -1;
  asc->m_sbrPresentFlag = 1;
  asc->m_extensionSamplingFrequency = 96000;
  asc->m_sc.m_usacConfig.m_sbrRatioIndex = (UCHAR)sbrRatioIndex;
  asc->m_sc.m_usacConfig.m_usacNumElements = 1;
  asc->m_sc.m_usacConfig.m_nUsacChannels = elType == ID_USAC_CPE ? 2 : 1;
  asc->m_sc.m_usacConfig.element[0].usacElementType = (MP4_ELEMENT_ID)elType;
  asc->m_sc.m_usacConfig.element[0].m_stereoConfigIndex = (UCHAR)stereoConfigIndex;
  out[0] = decs_CAacDecoder_Init(self, asc, AC_CM_DET_CFG_CHANGE, &changed);
  out[0] |= decs_CAacDecoder_Init(self, asc, AC_CM_ALLOC_MEM, &changed) << 16;
  out[1] = changed;
  out[2] = self->qmfDomain.globalConf.nInputChannels_requested;
  out[3] = self->qmfDomain.globalConf.nOutputChannels_requested;
  out[4] = self->qmfDomain.globalConf.nBandsAnalysis_requested;
  out[5] = self->qmfDomain.globalConf.nBandsSynthesis_requested;
  out[6] = self->qmfDomain.globalConf.nQmfTimeSlots_requested;
  out[7] = self->qmfDomain.globalConf.nQmfOvTimeSlots_requested;
  out[8] = self->qmfDomain.globalConf.nQmfProcBands_requested;
  out[9] = self->qmfDomain.globalConf.nQmfProcChannels_requested;
  out[10] = self->qmfDomain.globalConf.parkChannel_requested;
  out[11] = self->qmfDomain.globalConf.qmfDomainExplicitConfig;
  out[12] = self->aacChannels;
  FDK_drcDec_Close(&self->hUniDrcDecoder);
  pcmDmx_Close(&self->hPcmUtils);
  decs_CAacDecoder_Close(self);
  free(asc);
}
}
