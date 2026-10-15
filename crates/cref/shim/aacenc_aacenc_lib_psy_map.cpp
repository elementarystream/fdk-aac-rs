// Shims for the element mapping tests of fdk_aac_rs_aacenc::aacenc_lib. Every
// `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_aacenc_lib_psy_map (see build.rs).
//
// aacenc_lib.cpp is included at file scope (struct AACENCODER is defined there); its non-static
// functions get other names in this copy.

#include "cref_common.h"

#define cref_aacEncOpen pm_aacEncOpen
#define cref_aacEncClose pm_aacEncClose
#define cref_aacEncEncode pm_aacEncEncode
#define cref_aacEncInfo pm_aacEncInfo
#define cref_aacEncoder_SetParam pm_aacEncoder_SetParam
#define cref_aacEncoder_GetParam pm_aacEncoder_GetParam
#define cref_aacEncGetLibInfo pm_aacEncGetLibInfo
#define aacEncDefaultConfig pm_aacEncDefaultConfig
#define aacenc_SscCallback pm_aacenc_SscCallback
#define Get_AacEncoder pm_Get_AacEncoder
#define Free_AacEncoder pm_Free_AacEncoder
#define GetRequiredMem_AacEncoder pm_GetRequiredMem_AacEncoder
#include "libAACenc/src/aacenc_lib.cpp"
#include "libAACenc/src/aacEnc_ram.h"

extern "C" {
/* The mapping of element el of the reference encoder h: st[ch] = the index of
   psyElement[el]->psyStatic[ch] in pStaticChannels[], po[ch] = the index of
   psyOut[0]->psyOutElement[el]->psyOutChannel[ch] in pPsyOutChannels[] (-1 for NULL, -2 for a
   pointer outside the arrays). Returns -1 without a core encoder. */
INT cref_aacenc_lib_psy_map_element(void *h, INT el, INT *st, INT *po) {
  HANDLE_AACENCODER enc = (HANDLE_AACENCODER)h;
  if (enc == NULL || enc->hAacEnc == NULL || el < 0 || el >= ((8))) return -1;
  PSY_INTERNAL *psy = enc->hAacEnc->psyKernel;
  PSY_OUT *out = enc->hAacEnc->psyOut[0];
  for (int ch = 0; ch < 2; ch++) {
    PSY_STATIC *s = psy->psyElement[el]->psyStatic[ch];
    PSY_OUT_CHANNEL *o = out->psyOutElement[el]->psyOutChannel[ch];
    st[ch] = (s == NULL) ? -1 : -2;
    po[ch] = (o == NULL) ? -1 : -2;
    for (int i = 0; i < ((8)); i++) {
      if (s != NULL && psy->pStaticChannels[i] == s) st[ch] = i;
      if (o != NULL && out->pPsyOutChannels[i] == o) po[ch] = i;
    }
  }
  return 0;
}

/* The 60 avoid-hole flags ahFlag[el][ch][] of FDKaacEnc_adaptThresholdsToPe() as the shared
   dynamic RAM of the reference encoder h holds them now (qcElement[0]->dynMem_Ah_Flag, the start
   of P_BUF_1), between two encode calls. Returns -1 without a core encoder. */
INT cref_aacenc_lib_psy_map_ah_flags(void *h, INT el, INT ch, UCHAR *out) {
  HANDLE_AACENCODER enc = (HANDLE_AACENCODER)h;
  if (enc == NULL || enc->hAacEnc == NULL || el < 0 || el >= ((8)) || ch < 0 || ch > 1)
    return -1;
  const UCHAR *ah = enc->hAacEnc->qcOut[0]->qcElement[0]->dynMem_Ah_Flag;
  FDKmemcpy(out, ah + (el * 2 + ch) * MAX_GROUPED_SFB, MAX_GROUPED_SFB);
  return 0;
}
}
