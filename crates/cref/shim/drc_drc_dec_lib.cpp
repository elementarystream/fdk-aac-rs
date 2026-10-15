// Shims for the differential tests of fdk_aac_rs_drc::drc_dec_lib.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::drc_drc_dec_lib (see build.rs).
//
// The handle of the public API (HANDLE_DRC_DECODER) is passed as void*. The structures that
// FDK_drcDecLib.cpp and drcDec_selectionProcess.cpp define privately (struct s_drc_decoder,
// struct s_drcdec_selection_process) are mirrored here with the same members (checked by
// static_assert against the sizes measured from the sources), so that the complete state can be
// copied out. Pointers into the UNI_DRC_CONFIG (ACTIVE_DRC.pInst/pCoef,
// DRCDEC_SELECTION_DATA.pInst) are replaced in the copies by the index of the element plus one
// (0 for NULL), which is how the port stores them.

#include <stddef.h>
#include "cref_common.h"
#include "FDK_bitstream.h"
#include "FDK_audio.h"
#include "drcDec_types.h"
#include "drcDecoder.h"
#include "drcDec_gainDecoder.h"
#include "drcDec_selectionProcess.h"
#include "FDK_drcDecLib.h"

/* struct s_drc_decoder of FDK_drcDecLib.cpp */
typedef struct {
  INT codecMode;
  INT functionalRange;
  INT status;
  HANDLE_DRC_GAIN_DECODER hGainDec;
  void *hSelectionProc;
  int selProcInputDiff;
  UNI_DRC_CONFIG uniDrcConfig;
  LOUDNESS_INFO_SET loudnessInfoSet;
  UNI_DRC_GAIN uniDrcGain;
  SEL_PROC_OUTPUT selProcOutput;
} M_DRC_DECODER;

/* DRC_FEATURE_REQUEST, SEL_PROC_INPUT, DRCDEC_SELECTION_DATA, DRCDEC_SELECTION and
   struct s_drcdec_selection_process of drcDec_selectionProcess.cpp (enums as INT) */
typedef union {
  struct {
    UCHAR numRequests;
    UCHAR numRequestsDesired;
    INT request[MAX_REQUESTS_DRC_EFFECT_TYPE];
  } drcEffectType;
  struct {
    INT measurementRequestType;
    UCHAR requestedIsRange;
    FIXP_DBL requestValue;
    FIXP_DBL requestValueMin;
    FIXP_DBL requestValueMax;
  } dynamicRange;
  UCHAR drcCharacteristic;
} M_DRC_FEATURE_REQUEST;

typedef struct {
  SCHAR baseChannelCount;
  SCHAR baseLayout;
  INT targetConfigRequestType;
  UCHAR numDownmixIdRequests;
  UCHAR downmixIdRequested[MAX_REQUESTS_DOWNMIX_ID];
  UCHAR targetLayoutRequested;
  UCHAR targetChannelCountRequested;
  LONG audioSampleRate;
  UCHAR loudnessNormalizationOn;
  FIXP_DBL targetLoudness;
  UCHAR albumMode;
  UCHAR peakLimiterPresent;
  UCHAR loudnessDeviationMax;
  INT loudnessMeasurementMethod;
  INT loudnessMeasurementSystem;
  INT loudnessMeasurementPreProc;
  LONG deviceCutOffFrequency;
  FIXP_DBL loudnessNormalizationGainDbMax;
  FIXP_DBL loudnessNormalizationGainModificationDb;
  FIXP_DBL outputPeakLevelMax;
  UCHAR dynamicRangeControlOn;
  UCHAR numDrcFeatureRequests;
  INT drcFeatureRequestType[MAX_REQUESTS_DRC_FEATURE];
  M_DRC_FEATURE_REQUEST drcFeatureRequest[MAX_REQUESTS_DRC_FEATURE];
  FIXP_SGL boost;
  FIXP_SGL compress;
  UCHAR drcCharacteristicTarget;
} M_SEL_PROC_INPUT;

typedef struct {
  UCHAR selectionFlag;
  UCHAR downmixIdRequestIndex;
  FIXP_DBL outputPeakLevel;
  FIXP_DBL loudnessNormalizationGainDbAdjusted;
  FIXP_DBL outputLoudness;
  DRC_INSTRUCTIONS_UNI_DRC *pInst;
} M_DRCDEC_SELECTION_DATA;

typedef struct {
  UCHAR numData;
  M_DRCDEC_SELECTION_DATA data[(12 + 1 + 6)];
} M_DRCDEC_SELECTION;

typedef struct {
  INT codecMode;
  M_SEL_PROC_INPUT selProcInput;
  M_DRCDEC_SELECTION selectionData[2];
} M_SELECTION_PROCESS;

static_assert(sizeof(M_DRC_FEATURE_REQUEST) == 64, "DRC_FEATURE_REQUEST");
static_assert(sizeof(M_SEL_PROC_INPUT) == 560, "SEL_PROC_INPUT");
/* The sizes with 8-byte pointers (the structures hold pointers). */
#if defined(__LP64__)
static_assert(sizeof(M_DRCDEC_SELECTION) == 464, "DRCDEC_SELECTION");
static_assert(sizeof(M_SELECTION_PROCESS) == 1496, "s_drcdec_selection_process");
static_assert(sizeof(DRC_GAIN_DECODER) == 10472, "DRC_GAIN_DECODER");
#endif

static INT64 inst_index(M_DRC_DECODER *d, const DRC_INSTRUCTIONS_UNI_DRC *p) {
  if (p == NULL) return 0;
  return (INT64)(p - &d->uniDrcConfig.drcInstructionsUniDrc[0]) + 1;
}

static INT64 coef_index(M_DRC_DECODER *d, const DRC_COEFFICIENTS_UNI_DRC *p) {
  if (p == NULL) return 0;
  return (INT64)(p - &d->uniDrcConfig.drcCoefficientsUniDrc[0]) + 1;
}

static void bs_state(FDK_BITSTREAM *bs, UINT *st) {
  st[0] = FDKgetValidBits(bs);
  st[1] = bs->hBitBuf.ReadOffset;
  st[2] = bs->hBitBuf.BitNdx;
}

extern "C" {
/* Sizes of the mirrored structures (UNI_DRC_CONFIG, LOUDNESS_INFO_SET, UNI_DRC_GAIN,
   SEL_PROC_OUTPUT, DRC_GAIN_DECODER, s_drcdec_selection_process). */
void cref_drc_lib_sizes(INT *s) {
  s[0] = sizeof(UNI_DRC_CONFIG);
  s[1] = sizeof(LOUDNESS_INFO_SET);
  s[2] = sizeof(UNI_DRC_GAIN);
  s[3] = sizeof(SEL_PROC_OUTPUT);
  s[4] = sizeof(DRC_GAIN_DECODER);
  s[5] = sizeof(M_SELECTION_PROCESS);
}
void *cref_drc_lib_open(INT functionalRange, INT *err) {
  HANDLE_DRC_DECODER h = NULL;
  *err = FDK_drcDec_Open(&h, (DRC_DEC_FUNCTIONAL_RANGE)functionalRange);
  return h;
}
INT cref_drc_lib_close(void *h) {
  HANDLE_DRC_DECODER hd = (HANDLE_DRC_DECODER)h;
  return FDK_drcDec_Close(&hd);
}
INT cref_drc_lib_set_codec_mode(void *h, INT mode) {
  return FDK_drcDec_SetCodecMode((HANDLE_DRC_DECODER)h, (DRC_DEC_CODEC_MODE)mode);
}
INT cref_drc_lib_init(void *h, INT frameSize, INT sampleRate, INT baseChannelCount) {
  return FDK_drcDec_Init((HANDLE_DRC_DECODER)h, frameSize, sampleRate, baseChannelCount);
}
INT cref_drc_lib_set_param(void *h, INT type, INT value) {
  return FDK_drcDec_SetParam((HANDLE_DRC_DECODER)h, (DRC_DEC_USERPARAM)type, (FIXP_DBL)value);
}
INT cref_drc_lib_get_param(void *h, INT type) {
  return (INT)FDK_drcDec_GetParam((HANDLE_DRC_DECODER)h, (DRC_DEC_USERPARAM)type);
}
/* which: 0 ReadUniDrcConfig, 1 ReadLoudnessInfoSet, 2 ReadUniDrcGain, 3 ReadUniDrc,
   4 ReadLoudnessBox, 5 ReadDownmixInstructions_Box, 6 ReadUniDrcInstructions_Box,
   7 ReadUniDrcCoefficients_Box */
INT cref_drc_lib_read(void *h, INT which, UCHAR *buf, UINT bufSize, UINT validBits, UINT *st) {
  HANDLE_DRC_DECODER hd = (HANDLE_DRC_DECODER)h;
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, validBits, BS_READER);
  INT err;
  switch (which) {
    case 0: err = FDK_drcDec_ReadUniDrcConfig(hd, &bs); break;
    case 1: err = FDK_drcDec_ReadLoudnessInfoSet(hd, &bs); break;
    case 2: err = FDK_drcDec_ReadUniDrcGain(hd, &bs); break;
    case 3: err = FDK_drcDec_ReadUniDrc(hd, &bs); break;
    case 4: err = FDK_drcDec_ReadLoudnessBox(hd, &bs); break;
    case 5: err = FDK_drcDec_ReadDownmixInstructions_Box(hd, &bs); break;
    case 6: err = FDK_drcDec_ReadUniDrcInstructions_Box(hd, &bs); break;
    default: err = FDK_drcDec_ReadUniDrcCoefficients_Box(hd, &bs); break;
  }
  bs_state(&bs, st);
  return err;
}
void cref_drc_lib_set_channel_gains(void *h, INT numChannels, INT frameSize, FIXP_DBL *channelGainDb, FIXP_DBL *audioBuffer, INT audioBufferChannelOffset) {
  FDK_drcDec_SetChannelGains((HANDLE_DRC_DECODER)h, numChannels, frameSize, channelGainDb, audioBuffer, audioBufferChannelOffset);
}
INT cref_drc_lib_preprocess(void *h) { return FDK_drcDec_Preprocess((HANDLE_DRC_DECODER)h); }
INT cref_drc_lib_process_time(void *h, INT delaySamples, INT drcLocation, INT channelOffset, INT drcChannelOffset, INT numChannelsProcessed, FIXP_DBL *realBuffer, INT timeDataChannelOffset) {
  return FDK_drcDec_ProcessTime((HANDLE_DRC_DECODER)h, delaySamples, (DRC_DEC_LOCATION)drcLocation, channelOffset, drcChannelOffset, numChannelsProcessed, realBuffer, timeDataChannelOffset);
}
/* realBuffer/imagBuffer: numChannelsProcessed channels of `stride` values each. */
INT cref_drc_lib_process_freq(void *h, INT delaySamples, INT drcLocation, INT channelOffset, INT drcChannelOffset, INT numChannelsProcessed, INT processSingleTimeslot, FIXP_DBL *realBuffer, FIXP_DBL *imagBuffer, INT stride) {
  FIXP_DBL *re[8], *im[8];
  for (int c = 0; c < 8; c++) {
    re[c] = realBuffer + c * stride;
    im[c] = imagBuffer + c * stride;
  }
  return FDK_drcDec_ProcessFreq((HANDLE_DRC_DECODER)h, delaySamples, (DRC_DEC_LOCATION)drcLocation, channelOffset, drcChannelOffset, numChannelsProcessed, processSingleTimeslot, re, im);
}
INT cref_drc_lib_apply_downmix(void *h, INT *reverseInChannelMap, INT *reverseOutChannelMap, FIXP_DBL *realBuffer, INT *pNChannels) {
  return FDK_drcDec_ApplyDownmix((HANDLE_DRC_DECODER)h, reverseInChannelMap, reverseOutChannelMap, realBuffer, pNChannels);
}
/* The complete state: head[0..6] codecMode, functionalRange, status, hGainDec != NULL,
   hSelectionProc != NULL, selProcInputDiff; the memory images of the structures (gainDec and
   selProc only if allocated, with the pointers replaced, see above). */
void cref_drc_lib_state(void *h, INT *head, UCHAR *cfg, UCHAR *lis, UCHAR *gain, UCHAR *selOut, UCHAR *gainDec, UCHAR *selProc) {
  M_DRC_DECODER *d = (M_DRC_DECODER *)h;
  head[0] = d->codecMode;
  head[1] = d->functionalRange;
  head[2] = d->status;
  head[3] = d->hGainDec != NULL;
  head[4] = d->hSelectionProc != NULL;
  head[5] = d->selProcInputDiff;
  FDKmemcpy(cfg, &d->uniDrcConfig, sizeof(UNI_DRC_CONFIG));
  FDKmemcpy(lis, &d->loudnessInfoSet, sizeof(LOUDNESS_INFO_SET));
  FDKmemcpy(gain, &d->uniDrcGain, sizeof(UNI_DRC_GAIN));
  FDKmemcpy(selOut, &d->selProcOutput, sizeof(SEL_PROC_OUTPUT));
  if (d->hGainDec != NULL) {
    DRC_GAIN_DECODER g;
    FDKmemcpy(&g, d->hGainDec, sizeof(DRC_GAIN_DECODER));
    for (int a = 0; a < MAX_ACTIVE_DRCS; a++) {
      /* pointer-sized indices (8 bytes on LP64, 4 on ILP32) */
      size_t i = (size_t)inst_index(d, d->hGainDec->activeDrc[a].pInst);
      size_t c = (size_t)coef_index(d, d->hGainDec->activeDrc[a].pCoef);
      FDKmemcpy(&g.activeDrc[a].pInst, &i, sizeof(i));
      FDKmemcpy(&g.activeDrc[a].pCoef, &c, sizeof(c));
    }
    FDKmemcpy(gainDec, &g, sizeof(DRC_GAIN_DECODER));
  }
  if (d->hSelectionProc != NULL) {
    M_SELECTION_PROCESS s;
    FDKmemcpy(&s, d->hSelectionProc, sizeof(M_SELECTION_PROCESS));
    for (int l = 0; l < 2; l++) {
      for (int i = 0; i < 12 + 1 + 6; i++) {
        size_t k = (size_t)inst_index(d, s.selectionData[l].data[i].pInst);
        FDKmemcpy(&s.selectionData[l].data[i].pInst, &k, sizeof(k));
      }
    }
    FDKmemcpy(selProc, &s, sizeof(M_SELECTION_PROCESS));
  }
}
/* FDK_drcDec_GetLibInfo on FDK_MODULE_LAST entries with the given module ids (or NULL). Writes
   per entry the module id, version, flags, the 32 bytes of versionStr and the first 31
   characters of the title. */
INT cref_drc_lib_get_lib_info(INT null_info, const INT *ids_in, INT *ids, INT *version, UINT *flags, char *version_str, char *title) {
  LIB_INFO info[FDK_MODULE_LAST];
  FDKmemclear(info, sizeof(info));
  for (int i = 0; i < FDK_MODULE_LAST; i++) {
    info[i].module_id = (FDK_MODULE_ID)ids_in[i];
    info[i].flags = 0x5a5a0000u + i;
    FDKmemset(info[i].versionStr, 'x', sizeof(info[i].versionStr));
  }
  INT r = FDK_drcDec_GetLibInfo(null_info ? NULL : info);
  for (int i = 0; i < FDK_MODULE_LAST; i++) {
    ids[i] = info[i].module_id;
    version[i] = info[i].version;
    flags[i] = info[i].flags;
    FDKmemcpy(version_str + 32 * i, info[i].versionStr, 32);
    FDKmemclear(title + 32 * i, 32);
    if (info[i].title != NULL) strncpy(title + 32 * i, info[i].title, 31);
  }
  return r;
}
}
