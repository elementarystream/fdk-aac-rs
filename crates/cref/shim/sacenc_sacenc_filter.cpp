// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_filter.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_filter (see build.rs).
//
// The DC filter of the library; its state is read and set through the copy of the structure
// in namespace rsf.

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

/* The whole of libSACenc in namespace rsf (its headers included there first), so that
   the structures of the .cpp files and the handles that point to them live in the namespace
   with the layout of the library. The extern "C" API functions of the copy are renamed. */
#define FDK_sacenc_open rsf_FDK_sacenc_open
#define FDK_sacenc_init rsf_FDK_sacenc_init
#define FDK_sacenc_close rsf_FDK_sacenc_close
#define FDK_sacenc_encode rsf_FDK_sacenc_encode
#define FDK_sacenc_getInfo rsf_FDK_sacenc_getInfo
#define FDK_sacenc_setParam rsf_FDK_sacenc_setParam
#define FDK_sacenc_getLibInfo rsf_FDK_sacenc_getLibInfo
namespace rsf {
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

static rsf::DC_FILTER *dcf(void *h) { return (rsf::DC_FILTER *)h; }

extern "C" {
void *cref_sacenc_filter_create(void) {
  HANDLE_DC_FILTER h = NULL;
  if (fdk_sacenc_createDCFilter(&h) != SACENC_OK) return NULL;
  return h;
}
INT cref_sacenc_filter_create_null(void) { return fdk_sacenc_createDCFilter(NULL); }
INT cref_sacenc_filter_destroy(void *h) {
  HANDLE_DC_FILTER p = (HANDLE_DC_FILTER)h;
  INT r = fdk_sacenc_destroyDCFilter(&p);
  return r + (p == NULL ? 0 : 1000);
}
INT cref_sacenc_filter_init(void *h, UINT sampleRate) { return fdk_sacenc_initDCFilter((HANDLE_DC_FILTER)h, sampleRate); }
void cref_sacenc_filter_get(void *h, INT *c, INT *state) {
  *c = dcf(h)->c__FDK;
  *state = dcf(h)->state__FDK;
}
void cref_sacenc_filter_set(void *h, INT c, INT state) {
  dcf(h)->c__FDK = c;
  dcf(h)->state__FDK = state;
}
/* inPlace: signalOut == signalIn (out is used for both). */
INT cref_sacenc_filter_apply(void *h, const SHORT *in, SHORT *out, INT n, INT inPlace) {
  return fdk_sacenc_applyDCFilter((HANDLE_DC_FILTER)h, inPlace ? out : in, out, n);
}
}
