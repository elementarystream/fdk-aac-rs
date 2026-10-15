// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_delay.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_delay (see build.rs).
//
// The delay module of the library. struct DELAY (23 INTs: the 12 of DELAY_CONFIG, then the
// buffers and infos) is read and set as an INT image through the copy in namespace rsd.

#include "cref_common.h"
#include <stdlib.h>
#include "genericStds.h"
#include "common_fix.h"
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "FDK_matrixCalloc.h"
#include "FDK_trigFcts.h"
#include "fixpoint_math.h"
#include "FDK_core.h"
#include "qmf.h"

/* The whole of libSACenc in namespace rsd (its headers included there first), so that
   the structures of the .cpp files and the handles that point to them live in the namespace
   with the layout of the library. The extern "C" API functions of the copy are renamed. */
#define FDK_sacenc_open rsd_FDK_sacenc_open
#define FDK_sacenc_init rsd_FDK_sacenc_init
#define FDK_sacenc_close rsd_FDK_sacenc_close
#define FDK_sacenc_encode rsd_FDK_sacenc_encode
#define FDK_sacenc_getInfo rsd_FDK_sacenc_getInfo
#define FDK_sacenc_setParam rsd_FDK_sacenc_setParam
#define FDK_sacenc_getLibInfo rsd_FDK_sacenc_getLibInfo
namespace rsd {
#include "sacenc_lib.h"
#include "libSACenc/src/sacenc_const.h"
#include "libSACenc/src/sacenc_bitstream.h"
#include "libSACenc/src/sacenc_nlc_enc.h"
#include "libSACenc/src/sacenc_huff_tab.h"
#include "libSACenc/src/sacenc_vectorfunctions.h"
#include "libSACenc/src/sacenc_paramextract.h"
#include "libSACenc/src/sacenc_framewindowing.h"
#include "libSACenc/src/sacenc_tree.h"
#include "libSACenc/src/sacenc_onsetdetect.h"
#include "libSACenc/src/sacenc_filter.h"
#include "libSACenc/src/sacenc_staticgain.h"
#include "libSACenc/src/sacenc_delay.h"
#include "libSACenc/src/sacenc_dmx_tdom_enh.h"
/* sacenc_dmx_tdom_enh.cpp overloads these functions at file scope; in the namespace its
   overloads would hide the global ones. */
using ::invSqrtNorm2;
using ::sqrtFixp;
#include "libSACenc/src/sacenc_bitstream.cpp"
#include "libSACenc/src/sacenc_delay.cpp"
#include "libSACenc/src/sacenc_dmx_tdom_enh.cpp"
#include "libSACenc/src/sacenc_filter.cpp"
#include "libSACenc/src/sacenc_framewindowing.cpp"
#include "libSACenc/src/sacenc_huff_tab.cpp"
#include "libSACenc/src/sacenc_lib.cpp"
#include "libSACenc/src/sacenc_nlc_enc.cpp"
#include "libSACenc/src/sacenc_onsetdetect.cpp"
#include "libSACenc/src/sacenc_paramextract.cpp"
#include "libSACenc/src/sacenc_staticgain.cpp"
#include "libSACenc/src/sacenc_tree.cpp"
#include "libSACenc/src/sacenc_vectorfunctions.cpp"
}
#undef FDK_sacenc_open
#undef FDK_sacenc_init
#undef FDK_sacenc_close
#undef FDK_sacenc_encode
#undef FDK_sacenc_getInfo
#undef FDK_sacenc_setParam
#undef FDK_sacenc_getLibInfo
/* The global declarations of the library (the guards were set inside the namespace). */
#undef SACENC_LIB_H
#undef SACENC_CONST_H
#undef SACENC_BITSTREAM_H
#undef SACENC_NLC_ENC_H
#undef SACENC_HUFF_TAB_H
#undef SACENC_VECTORFUNCTIONS_H
#undef SACENC_PARAMEXTRACT_H
#undef SACENC_FRAMEWINDOWING_H
#undef SACENC_TREE_H
#undef SACENC_ONSETDETECT_H
#undef SACENC_FILTER_H
#undef SACENC_STATICGAIN_H
#undef SACENC_DELAY_H
#undef SACENC_DMX_TDOM_ENH_H
#include "sacenc_lib.h"
#include "libSACenc/src/sacenc_const.h"
#include "libSACenc/src/sacenc_bitstream.h"
#include "libSACenc/src/sacenc_nlc_enc.h"
#include "libSACenc/src/sacenc_huff_tab.h"
#include "libSACenc/src/sacenc_vectorfunctions.h"
#include "libSACenc/src/sacenc_paramextract.h"
#include "libSACenc/src/sacenc_framewindowing.h"
#include "libSACenc/src/sacenc_tree.h"
#include "libSACenc/src/sacenc_onsetdetect.h"
#include "libSACenc/src/sacenc_filter.h"
#include "libSACenc/src/sacenc_staticgain.h"
#include "libSACenc/src/sacenc_delay.h"
#include "libSACenc/src/sacenc_dmx_tdom_enh.h"

extern "C" {
INT cref_sacenc_delay_size(void) { return sizeof(rsd::DELAY) / sizeof(INT); }
void *cref_sacenc_delay_open(void) {
  HANDLE_DELAY h = NULL;
  if (fdk_sacenc_delay_Open(&h) != SACENC_OK) return NULL;
  return h;
}
INT cref_sacenc_delay_open_null(void) { return fdk_sacenc_delay_Open(NULL); }
INT cref_sacenc_delay_close(void *h) {
  HANDLE_DELAY p = (HANDLE_DELAY)h;
  return fdk_sacenc_delay_Close(&p);
}
INT cref_sacenc_delay_close_null(void) { return fdk_sacenc_delay_Close(NULL); }
void cref_sacenc_delay_get(void *h, INT *v) { FDKmemcpy(v, h, sizeof(rsd::DELAY)); }
void cref_sacenc_delay_set(void *h, const INT *v) { FDKmemcpy(h, v, sizeof(rsd::DELAY)); }
INT cref_sacenc_delay_init(void *h, INT q, INT f, INT core, INT mux) {
  return fdk_sacenc_delay_Init((HANDLE_DELAY)h, q, f, core, mux);
}
INT cref_sacenc_delay_calc(void *h) { return fdk_sacenc_delay_SubCalulateBufferDelays((HANDLE_DELAY)h); }
/* which: 0 DmxAlign, 1 TimeDomDmx, 2 SacTimeAlignmentDynamicOut, 3 NSacTimeAlignment, 4 MinimizeDelay */
INT cref_sacenc_delay_setter(void *h, INT which, INT v) {
  HANDLE_DELAY d = (HANDLE_DELAY)h;
  switch (which) {
    case 0: return fdk_sacenc_delay_SetDmxAlign(d, v);
    case 1: return fdk_sacenc_delay_SetTimeDomDmx(d, v);
    case 2: return fdk_sacenc_delay_SetSacTimeAlignmentDynamicOut(d, v);
    case 3: return fdk_sacenc_delay_SetNSacTimeAlignment(d, v);
    default: return fdk_sacenc_delay_SetMinimizeDelay(d, v);
  }
}
/* The values of the nine getters. */
void cref_sacenc_delay_getters(void *h, INT *v) {
  HANDLE_DELAY d = (HANDLE_DELAY)h;
  v[0] = fdk_sacenc_delay_GetOutputAudioBufferDelay(d);
  v[1] = fdk_sacenc_delay_GetSurroundAnalysisBufferDelay(d);
  v[2] = fdk_sacenc_delay_GetArbDmxAnalysisBufferDelay(d);
  v[3] = fdk_sacenc_delay_GetBitstreamFrameBufferSize(d);
  v[4] = fdk_sacenc_delay_GetDmxAlignBufferDelay(d);
  v[5] = fdk_sacenc_delay_GetDiscardOutFrames(d);
  v[6] = fdk_sacenc_delay_GetInfoDmxDelay(d);
  v[7] = fdk_sacenc_delay_GetInfoCodecDelay(d);
  v[8] = fdk_sacenc_delay_GetInfoDecoderDelay(d);
}
}
