// Shims for the differential tests of fdk_aac_rs_aacenc::aacenc_lib.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_aacenc_lib (see build.rs).
//
// The reference API (aacEnc*, renamed to cref_aacEnc* by the prefix header) is wrapped as is:
// the buffer descriptors, argument structures and AACENC_InfoStruct are passed as pointers to
// the port's #[repr(C)] structures (layout checked by cref_aacenc_lib_layout), the handle as an
// opaque pointer. The file-static functions and tables of aacenc_lib.cpp are in
// aacenc_aacenc_lib_static.cpp.

#include "cref_common.h"
#include "aacenc_lib.h"
#include "FDK_audio.h"
#include "FDK_core.h"
#include "sbr_encoder.h"
#include "tpenc_lib.h"
#include "sacenc_lib.h"
#include "libSBRenc/src/sbrenc_ram.h"
#include "libAACenc/src/aacenc.h"
#include "libAACenc/src/aacEnc_ram.h"
#include "libAACenc/src/channel_map.h"
#include "libAACenc/src/psy_const.h"
#include "libAACenc/src/bitenc.h"
#include "libAACenc/src/metadata_main.h"
#include "libAACenc/src/mps_main.h"
#include "pcm_utils.h"

#include <stddef.h>

extern "C" {
/* aacEncOpen(&h, encModules, maxChannels); *ph = h (NULL on failure). */
INT cref_aacenc_lib_open(void **ph, UINT encModules, UINT maxChannels) {
  HANDLE_AACENCODER h = NULL;
  INT e = aacEncOpen(&h, encModules, maxChannels);
  *ph = (void *)h;
  return e;
}
/* aacEncClose(&h) */
INT cref_aacenc_lib_close(void *h) {
  HANDLE_AACENCODER hh = (HANDLE_AACENCODER)h;
  return aacEncClose(&hh);
}
INT cref_aacenc_lib_set_param(void *h, INT param, UINT value) {
  return aacEncoder_SetParam((HANDLE_AACENCODER)h, (AACENC_PARAM)param, value);
}
UINT cref_aacenc_lib_get_param(void *h, INT param) {
  return aacEncoder_GetParam((HANDLE_AACENCODER)h, (AACENC_PARAM)param);
}
/* aacEncInfo(h, info): info is an AACENC_InfoStruct (or NULL) */
INT cref_aacenc_lib_info(void *h, void *info) {
  return aacEncInfo((HANDLE_AACENCODER)h, (AACENC_InfoStruct *)info);
}
/* aacEncEncode(h, inBufDesc, outBufDesc, inargs, outargs); each may be NULL. */
INT cref_aacenc_lib_encode(void *h, const void *inBufDesc, const void *outBufDesc, const void *inargs, void *outargs) {
  return aacEncEncode((HANDLE_AACENCODER)h, (const AACENC_BufDesc *)inBufDesc,
                      (const AACENC_BufDesc *)outBufDesc, (const AACENC_InArgs *)inargs,
                      (AACENC_OutArgs *)outargs);
}
/* aacEncGetLibInfo on a zeroed array (FDKinitLibInfo); the entries up to the first FDK_NONE:
   module_id, version, flags, title (64 bytes each, NUL-terminated), versionStr (32 bytes each).
   *n = number of entries. */
INT cref_aacenc_lib_get_lib_info(INT *n, INT *ids, INT *versions, UINT *flags, char *titles, char *versionStrs) {
  LIB_INFO info[FDK_MODULE_LAST];
  memset(info, 0, sizeof(info));
  FDKinitLibInfo(info);
  INT e = aacEncGetLibInfo(info);
  INT i;
  for (i = 0; i < FDK_MODULE_LAST && info[i].module_id != FDK_NONE; i++) {
    ids[i] = info[i].module_id;
    versions[i] = info[i].version;
    flags[i] = info[i].flags;
    memset(titles + 64 * i, 0, 64);
    if (info[i].title) strncpy(titles + 64 * i, info[i].title, 63);
    memcpy(versionStrs + 32 * i, info[i].versionStr, 32);
  }
  *n = i;
  return e;
}
/* aacEncGetLibInfo(NULL) */
INT cref_aacenc_lib_get_lib_info_null(void) {
  return aacEncGetLibInfo(NULL);
}
/* Sizes and member offsets of the public structures:
   v[0..9]   AACENC_InfoStruct: sizeof, maxOutBufBytes, maxAncBytes, inBufFillLevel,
             inputChannels, frameLength, nDelay, nDelayCore, confBuf, confSize
   v[10..15] AACENC_BufDesc: sizeof, numBufs, bufs, bufferIdentifiers, bufSizes, bufElSizes
   v[16..18] AACENC_InArgs: sizeof, numInSamples, numAncBytes
   v[19..23] AACENC_OutArgs: sizeof, numOutBytes, numInSamples, numAncBytes, bitResState
   v[24..45] AACENC_MetaData: sizeof, drc_profile, comp_profile, drc_TargetRefLevel,
             comp_TargetRefLevel, prog_ref_level_present, prog_ref_level,
             PCE_mixdown_idx_present, ETSI_DmxLvl_present, centerMixLevel, surroundMixLevel,
             dolbySurroundMode, drcPresentationMode, ExtMetaData, ExtMetaData.extAncDataEnable,
             .extDownmixLevelEnable, .extDownmixLevel_A, .extDownmixLevel_B, .dmxGainEnable,
             .dmxGain5, .dmxGain2, .lfeDmxEnable, .lfeDmxLevel (relative to ExtMetaData) */
void cref_aacenc_lib_layout(INT *v) {
  INT k = 0;
  v[k++] = sizeof(AACENC_InfoStruct);
  v[k++] = offsetof(AACENC_InfoStruct, maxOutBufBytes);
  v[k++] = offsetof(AACENC_InfoStruct, maxAncBytes);
  v[k++] = offsetof(AACENC_InfoStruct, inBufFillLevel);
  v[k++] = offsetof(AACENC_InfoStruct, inputChannels);
  v[k++] = offsetof(AACENC_InfoStruct, frameLength);
  v[k++] = offsetof(AACENC_InfoStruct, nDelay);
  v[k++] = offsetof(AACENC_InfoStruct, nDelayCore);
  v[k++] = offsetof(AACENC_InfoStruct, confBuf);
  v[k++] = offsetof(AACENC_InfoStruct, confSize);
  v[k++] = sizeof(AACENC_BufDesc);
  v[k++] = offsetof(AACENC_BufDesc, numBufs);
  v[k++] = offsetof(AACENC_BufDesc, bufs);
  v[k++] = offsetof(AACENC_BufDesc, bufferIdentifiers);
  v[k++] = offsetof(AACENC_BufDesc, bufSizes);
  v[k++] = offsetof(AACENC_BufDesc, bufElSizes);
  v[k++] = sizeof(AACENC_InArgs);
  v[k++] = offsetof(AACENC_InArgs, numInSamples);
  v[k++] = offsetof(AACENC_InArgs, numAncBytes);
  v[k++] = sizeof(AACENC_OutArgs);
  v[k++] = offsetof(AACENC_OutArgs, numOutBytes);
  v[k++] = offsetof(AACENC_OutArgs, numInSamples);
  v[k++] = offsetof(AACENC_OutArgs, numAncBytes);
  v[k++] = offsetof(AACENC_OutArgs, bitResState);
  v[k++] = sizeof(AACENC_MetaData);
  v[k++] = offsetof(AACENC_MetaData, drc_profile);
  v[k++] = offsetof(AACENC_MetaData, comp_profile);
  v[k++] = offsetof(AACENC_MetaData, drc_TargetRefLevel);
  v[k++] = offsetof(AACENC_MetaData, comp_TargetRefLevel);
  v[k++] = offsetof(AACENC_MetaData, prog_ref_level_present);
  v[k++] = offsetof(AACENC_MetaData, prog_ref_level);
  v[k++] = offsetof(AACENC_MetaData, PCE_mixdown_idx_present);
  v[k++] = offsetof(AACENC_MetaData, ETSI_DmxLvl_present);
  v[k++] = offsetof(AACENC_MetaData, centerMixLevel);
  v[k++] = offsetof(AACENC_MetaData, surroundMixLevel);
  v[k++] = offsetof(AACENC_MetaData, dolbySurroundMode);
  v[k++] = offsetof(AACENC_MetaData, drcPresentationMode);
  v[k++] = offsetof(AACENC_MetaData, ExtMetaData);
  v[k++] = offsetof(AACENC_MetaData, ExtMetaData.extAncDataEnable) - offsetof(AACENC_MetaData, ExtMetaData);
  v[k++] = offsetof(AACENC_MetaData, ExtMetaData.extDownmixLevelEnable) - offsetof(AACENC_MetaData, ExtMetaData);
  v[k++] = offsetof(AACENC_MetaData, ExtMetaData.extDownmixLevel_A) - offsetof(AACENC_MetaData, ExtMetaData);
  v[k++] = offsetof(AACENC_MetaData, ExtMetaData.extDownmixLevel_B) - offsetof(AACENC_MetaData, ExtMetaData);
  v[k++] = offsetof(AACENC_MetaData, ExtMetaData.dmxGainEnable) - offsetof(AACENC_MetaData, ExtMetaData);
  v[k++] = offsetof(AACENC_MetaData, ExtMetaData.dmxGain5) - offsetof(AACENC_MetaData, ExtMetaData);
  v[k++] = offsetof(AACENC_MetaData, ExtMetaData.dmxGain2) - offsetof(AACENC_MetaData, ExtMetaData);
  v[k++] = offsetof(AACENC_MetaData, ExtMetaData.lfeDmxEnable) - offsetof(AACENC_MetaData, ExtMetaData);
  v[k++] = offsetof(AACENC_MetaData, ExtMetaData.lfeDmxLevel) - offsetof(AACENC_MetaData, ExtMetaData);
}
}
