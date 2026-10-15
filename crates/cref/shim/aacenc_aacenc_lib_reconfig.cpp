// Shims for the reconfiguration tests of fdk_aac_rs_aacenc::aacenc_lib. Every
// `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_aacenc_lib_reconfig (see build.rs).
//
// The tests can save the bytes in front of the core encoder's dynamic RAM before a reference
// encode call and restore them after it (FDK_RECONFIG_FULL).
//
// aacenc_lib.cpp is included at file scope (struct AACENCODER is defined there); its non-static
// functions get other names in this copy.

#include "cref_common.h"

#define cref_aacEncOpen rc_aacEncOpen
#define cref_aacEncClose rc_aacEncClose
#define cref_aacEncEncode rc_aacEncEncode
#define cref_aacEncInfo rc_aacEncInfo
#define cref_aacEncoder_SetParam rc_aacEncoder_SetParam
#define cref_aacEncoder_GetParam rc_aacEncoder_GetParam
#define cref_aacEncGetLibInfo rc_aacEncGetLibInfo
#define aacEncDefaultConfig rc_aacEncDefaultConfig
#define aacenc_SscCallback rc_aacenc_SscCallback
#define Get_AacEncoder rc_Get_AacEncoder
#define Free_AacEncoder rc_Free_AacEncoder
#define GetRequiredMem_AacEncoder rc_GetRequiredMem_AacEncoder
#include "libAACenc/src/aacenc_lib.cpp"
#include "libAACenc/src/aacEnc_ram.h"

static UCHAR *rc_dynamic_ram(void *h) {
  HANDLE_AACENCODER enc = (HANDLE_AACENCODER)h;
  if (enc == NULL || enc->hAacEnc == NULL) return NULL;
  return (UCHAR *)enc->hAacEnc->dynamic_RAM;
}

extern "C" {
/* Copies the n bytes in front of the dynamic RAM of the core encoder to buf; returns 0, or -1
   without a core encoder. */
INT cref_aacenc_lib_reconfig_save(void *h, UCHAR *buf, INT n) {
  UCHAR *ram = rc_dynamic_ram(h);
  if (ram == NULL) return -1;
  FDKmemcpy(buf, ram - n, n);
  return 0;
}
/* Restores the n bytes in front of the dynamic RAM from buf; returns the number of bytes that
   differed since cref_aacenc_lib_reconfig_save, or -1. */
INT cref_aacenc_lib_reconfig_restore(void *h, const UCHAR *buf, INT n) {
  UCHAR *ram = rc_dynamic_ram(h);
  if (ram == NULL) return -1;
  INT changed = 0;
  for (INT i = 0; i < n; i++) {
    if (ram[i - n] != buf[i]) changed++;
  }
  FDKmemcpy(ram - n, buf, n);
  return changed;
}
/* sizeof(QC_OUT_CHANNEL), its stride in the dynamic RAM, the offsets of mdctSpectrum and of
   the last member sfbSpreadEnergy, MAX_GROUPED_SFB, P_BUF_0 */
void cref_aacenc_lib_reconfig_layout(INT *out) {
  out[0] = (INT)sizeof(QC_OUT_CHANNEL);
  out[1] = (INT)ALIGN_SIZE(sizeof(QC_OUT_CHANNEL));
  out[2] = (INT)offsetof(QC_OUT_CHANNEL, mdctSpectrum);
  out[3] = (INT)offsetof(QC_OUT_CHANNEL, sfbSpreadEnergy);
  out[4] = (INT)MAX_GROUPED_SFB;
  out[5] = (INT)P_BUF_0;
}
}
