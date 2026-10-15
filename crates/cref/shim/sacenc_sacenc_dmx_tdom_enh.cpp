// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_dmx_tdom_enh.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_dmx_tdom_enh (see build.rs).
//
// The enhanced time domain downmix of the library; its state is read and set as an INT image
// (21 values, see cref_sacenc_dmx_get) and the sine window through the copy of the structure
// in namespace rdx, which also gives the file-static functions.

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

/* The whole of libSACenc in namespace rdx (its headers included there first), so that
   the structures of the .cpp files and the handles that point to them live in the namespace
   with the layout of the library. The extern "C" API functions of the copy are renamed. */
#define FDK_sacenc_open rdx_FDK_sacenc_open
#define FDK_sacenc_init rdx_FDK_sacenc_init
#define FDK_sacenc_close rdx_FDK_sacenc_close
#define FDK_sacenc_encode rdx_FDK_sacenc_encode
#define FDK_sacenc_getInfo rdx_FDK_sacenc_getInfo
#define FDK_sacenc_setParam rdx_FDK_sacenc_setParam
#define FDK_sacenc_getLibInfo rdx_FDK_sacenc_getLibInfo
namespace rdx {
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

static rdx::ENHANCED_TIME_DOMAIN_DMX *edm(void *h) { return (rdx::ENHANCED_TIME_DOMAIN_DMX *)h; }

extern "C" {
void *cref_sacenc_dmx_open(INT framelength) {
  HANDLE_ENHANCED_TIME_DOMAIN_DMX h = NULL;
  if (fdk_sacenc_open_enhancedTimeDomainDmx(&h, framelength) != SACENC_OK) return NULL;
  return h;
}
INT cref_sacenc_dmx_open_null(void) { return fdk_sacenc_open_enhancedTimeDomainDmx(NULL, 64); }
INT cref_sacenc_dmx_close(void *h) {
  HANDLE_ENHANCED_TIME_DOMAIN_DMX p = (HANDLE_ENHANCED_TIME_DOMAIN_DMX)h;
  return fdk_sacenc_close_enhancedTimeDomainDmx(&p);
}
INT cref_sacenc_dmx_close_null(void) { return fdk_sacenc_close_enhancedTimeDomainDmx(NULL); }
/* maxFramelength, framelength, prev_gain_m[2], prev_gain_e, prev_H1_m[2], prev_H1_e,
   sinusWindow_e, prev_Left_m, prev_Left_e, prev_Right_m, prev_Right_e, prev_XNrg_m,
   prev_XNrg_e, lin_bbCld_weight_m, lin_bbCld_weight_e, gain_weight_m[2], gain_weight_e */
void cref_sacenc_dmx_get(void *h, INT *v, INT *window) {
  rdx::ENHANCED_TIME_DOMAIN_DMX *d = edm(h);
  INT k = 0;
  v[k++] = d->maxFramelength;
  v[k++] = d->framelength;
  v[k++] = d->prev_gain_m[0];
  v[k++] = d->prev_gain_m[1];
  v[k++] = d->prev_gain_e;
  v[k++] = d->prev_H1_m[0];
  v[k++] = d->prev_H1_m[1];
  v[k++] = d->prev_H1_e;
  v[k++] = d->sinusWindow_e;
  v[k++] = d->prev_Left_m;
  v[k++] = d->prev_Left_e;
  v[k++] = d->prev_Right_m;
  v[k++] = d->prev_Right_e;
  v[k++] = d->prev_XNrg_m;
  v[k++] = d->prev_XNrg_e;
  v[k++] = d->lin_bbCld_weight_m;
  v[k++] = d->lin_bbCld_weight_e;
  v[k++] = d->gain_weight_m[0];
  v[k++] = d->gain_weight_m[1];
  v[k++] = d->gain_weight_e;
  if (window != NULL) FDKmemcpy(window, d->sinusWindow_m, (1 + d->maxFramelength) * sizeof(FIXP_DBL));
}
void cref_sacenc_dmx_set(void *h, const INT *v) {
  rdx::ENHANCED_TIME_DOMAIN_DMX *d = edm(h);
  INT k = 1;
  d->framelength = v[k++];
  d->prev_gain_m[0] = v[k++];
  d->prev_gain_m[1] = v[k++];
  d->prev_gain_e = v[k++];
  d->prev_H1_m[0] = v[k++];
  d->prev_H1_m[1] = v[k++];
  d->prev_H1_e = v[k++];
  d->sinusWindow_e = (SCHAR)v[k++];
  d->prev_Left_m = v[k++];
  d->prev_Left_e = v[k++];
  d->prev_Right_m = v[k++];
  d->prev_Right_e = v[k++];
  d->prev_XNrg_m = v[k++];
  d->prev_XNrg_e = v[k++];
  d->lin_bbCld_weight_m = v[k++];
  d->lin_bbCld_weight_e = v[k++];
  d->gain_weight_m[0] = v[k++];
  d->gain_weight_m[1] = v[k++];
  d->gain_weight_e = v[k++];
}
INT cref_sacenc_dmx_init(void *h, const INT *inGain, INT inGainE, INT outGain, INT outGainE, INT framelength) {
  return fdk_sacenc_init_enhancedTimeDomainDmx((HANDLE_ENHANCED_TIME_DOMAIN_DMX)h, (const FIXP_DBL *)inGain, inGainE, outGain, outGainE, framelength);
}
INT cref_sacenc_dmx_apply(void *h, const SHORT *left, const SHORT *right, SHORT *out, INT delay) {
  const INT_PCM *in[2] = {left, right};
  return fdk_sacenc_apply_enhancedTimeDomainDmx((HANDLE_ENHANCED_TIME_DOMAIN_DMX)h, in, out, delay);
}
INT cref_sacenc_dmx_invSqrtNorm2(INT m, INT e, INT *re) { return rdx::invSqrtNorm2(m, e, re); }
INT cref_sacenc_dmx_sqrtFixp(INT m, INT e, INT *re) { return rdx::sqrtFixp(m, e, re); }
INT cref_sacenc_dmx_fixpAdd(INT m1, INT e1, INT m2, INT e2, INT *de) { return rdx::fixpAdd(m1, e1, m2, e2, de); }
void cref_sacenc_dmx_fixpAddNorm(INT v1, INT q1, INT *v2, INT *q2) { rdx::fixpAddNorm(v1, q1, (FIXP_DBL *)v2, q2); }
void cref_sacenc_dmx_calculateRatio(INT sm, INT se, INT lm, INT le, INT im, INT ie, INT *g, INT *ge) {
  rdx::calculateRatio(sm, se, lm, le, im, ie, (FIXP_DBL *)g, ge);
}
void cref_sacenc_dmx_calculateDmxGains(INT lm, INT le, INT l2m, INT l2e, INT im, INT ie, const INT *g, INT ge, INT *h1, INT *h1e) {
  rdx::calculateDmxGains(lm, le, l2m, l2e, im, ie, (const FIXP_DBL *)g, ge, (FIXP_DBL *)h1, h1e);
}
}
