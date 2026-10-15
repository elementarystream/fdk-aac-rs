// Shims for the differential tests of fdk_aac_rs_drc::drc_dec_selection_process.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::drc_drc_dec_selection_process (see build.rs).
//
// The handle (HANDLE_DRC_SELECTION_PROCESS) is passed as void*. struct
// s_drcdec_selection_process is private to drcDec_selectionProcess.cpp; it is mirrored here (as
// in drc_drc_dec_lib.cpp) to copy out the state, with DRCDEC_SELECTION_DATA.pInst replaced by
// the index into drcInstructionsUniDrc plus one (0 for NULL). The structures are passed as
// their memory images.

#include "cref_common.h"
#include "drcDec_types.h"
#include "drcDecoder.h"
#include "drcDec_selectionProcess.h"

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
} SP_DRC_FEATURE_REQUEST;

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
  SP_DRC_FEATURE_REQUEST drcFeatureRequest[MAX_REQUESTS_DRC_FEATURE];
  FIXP_SGL boost;
  FIXP_SGL compress;
  UCHAR drcCharacteristicTarget;
} SP_SEL_PROC_INPUT;

typedef struct {
  UCHAR selectionFlag;
  UCHAR downmixIdRequestIndex;
  FIXP_DBL outputPeakLevel;
  FIXP_DBL loudnessNormalizationGainDbAdjusted;
  FIXP_DBL outputLoudness;
  DRC_INSTRUCTIONS_UNI_DRC *pInst;
} SP_DRCDEC_SELECTION_DATA;

typedef struct {
  UCHAR numData;
  SP_DRCDEC_SELECTION_DATA data[(12 + 1 + 6)];
} SP_DRCDEC_SELECTION;

typedef struct {
  INT codecMode;
  SP_SEL_PROC_INPUT selProcInput;
  SP_DRCDEC_SELECTION selectionData[2];
} SP_SELECTION_PROCESS;

/* The size with 8-byte pointers (the structure holds pointers). */
#if defined(__LP64__)
static_assert(sizeof(SP_SELECTION_PROCESS) == 1496, "s_drcdec_selection_process");
#endif

/* The memory image of the selection process, pointers into `cfg` replaced. */
static void sp_state(void *h, UNI_DRC_CONFIG *cfg, UCHAR *state) {
  SP_SELECTION_PROCESS s;
  FDKmemcpy(&s, h, sizeof(s));
  for (int l = 0; l < 2; l++) {
    for (int i = 0; i < 12 + 1 + 6; i++) {
      DRC_INSTRUCTIONS_UNI_DRC *p = s.selectionData[l].data[i].pInst;
      /* a pointer-sized index (8 bytes on LP64, 4 on ILP32) */
      size_t k = p == NULL ? 0 : (size_t)(p - cfg->drcInstructionsUniDrc) + 1;
      FDKmemcpy(&s.selectionData[l].data[i].pInst, &k, sizeof(k));
    }
  }
  FDKmemcpy(state, &s, sizeof(s));
}

/* The configuration of the process calls of a handle (the pointers of the selection data point
   into it, so it has to stay at the same place). */
static UNI_DRC_CONFIG sp_cfg[4];

extern "C" {
void *cref_drc_sel_create(INT *err) {
  HANDLE_DRC_SELECTION_PROCESS h = NULL;
  *err = drcDec_SelectionProcess_Create(&h);
  return h;
}
INT cref_drc_sel_delete(void *h) {
  HANDLE_DRC_SELECTION_PROCESS hs = (HANDLE_DRC_SELECTION_PROCESS)h;
  return drcDec_SelectionProcess_Delete(&hs);
}
INT cref_drc_sel_init(void *h) { return drcDec_SelectionProcess_Init((HANDLE_DRC_SELECTION_PROCESS)h); }
INT cref_drc_sel_set_codec_mode(void *h, INT mode) {
  return drcDec_SelectionProcess_SetCodecMode((HANDLE_DRC_SELECTION_PROCESS)h, (SEL_PROC_CODEC_MODE)mode);
}
INT cref_drc_sel_set_param(void *h, INT type, INT value, INT *diff, INT nullDiff) {
  return drcDec_SelectionProcess_SetParam((HANDLE_DRC_SELECTION_PROCESS)h, (SEL_PROC_USER_PARAM)type, (FIXP_DBL)value, nullDiff ? NULL : diff);
}
INT cref_drc_sel_get_param(void *h, INT type) {
  return drcDec_SelectionProcess_GetParam((HANDLE_DRC_SELECTION_PROCESS)h, (SEL_PROC_USER_PARAM)type);
}
/* drcDec_SelectionProcess_Process with the configuration slot `slot` (0..3) set to `cfg` (and
   copied back), the loudness info set `lis` and the output `out` (in/out). `state` receives the
   selection process. */
INT cref_drc_sel_process(void *h, INT slot, UCHAR *cfg, const UCHAR *lis, UCHAR *out, UCHAR *state) {
  UNI_DRC_CONFIG *c = &sp_cfg[slot & 3];
  FDKmemcpy(c, cfg, sizeof(UNI_DRC_CONFIG));
  LOUDNESS_INFO_SET *l = (LOUDNESS_INFO_SET *)FDKcalloc(1, sizeof(LOUDNESS_INFO_SET));
  FDKmemcpy(l, lis, sizeof(LOUDNESS_INFO_SET));
  SEL_PROC_OUTPUT o;
  FDKmemcpy(&o, out, sizeof(o));
  INT r = drcDec_SelectionProcess_Process((HANDLE_DRC_SELECTION_PROCESS)h, c, l, &o);
  FDKmemcpy(out, &o, sizeof(o));
  FDKmemcpy(cfg, c, sizeof(UNI_DRC_CONFIG));
  FDKfree(l);
  if (h != NULL) sp_state(h, c, state);
  return r;
}
/* The selection process state relative to configuration slot `slot`. */
void cref_drc_sel_state(void *h, INT slot, UCHAR *state) { sp_state(h, &sp_cfg[slot & 3], state); }
}
