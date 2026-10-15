// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_staticgain.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_staticgain (see build.rs).
//
// The static gain module of the library. STATIC_GAIN_CONFIG (3 INTs) and STATIC_GAIN
// (6 INTs: encMode, fixedGainDMX, preGainFactorDb, PostGain, PreGain[2]) are read and set as
// INT images (the latter through the copy in namespace rsg).

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

/* The whole of libSACenc in namespace rsg (its headers included there first), so that
   the structures of the .cpp files and the handles that point to them live in the namespace
   with the layout of the library. The extern "C" API functions of the copy are renamed. */
#define FDK_sacenc_open rsg_FDK_sacenc_open
#define FDK_sacenc_init rsg_FDK_sacenc_init
#define FDK_sacenc_close rsg_FDK_sacenc_close
#define FDK_sacenc_encode rsg_FDK_sacenc_encode
#define FDK_sacenc_getInfo rsg_FDK_sacenc_getInfo
#define FDK_sacenc_setParam rsg_FDK_sacenc_setParam
#define FDK_sacenc_getLibInfo rsg_FDK_sacenc_getLibInfo
namespace rsg {
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
INT cref_sacenc_staticgain_sizes(INT *v) {
  v[0] = sizeof(STATIC_GAIN_CONFIG) / sizeof(INT);
  v[1] = sizeof(rsg::STATIC_GAIN) / sizeof(INT);
  return 2;
}
void *cref_sacenc_staticgain_open_config(void) {
  HANDLE_STATIC_GAIN_CONFIG h = NULL;
  if (fdk_sacenc_staticGain_OpenConfig(&h) != SACENC_OK) return NULL;
  return h;
}
void *cref_sacenc_staticgain_open(void) {
  HANDLE_STATIC_GAIN h = NULL;
  if (fdk_sacenc_staticGain_Open(&h) != SACENC_OK) return NULL;
  return h;
}
/* The error codes of OpenConfig, Open, CloseConfig, Close, InitDefaultConfig, SetDmxGain and
   SetEncMode with NULL handles. */
void cref_sacenc_staticgain_null(INT *v) {
  HANDLE_STATIC_GAIN_CONFIG c = NULL;
  HANDLE_STATIC_GAIN g = NULL;
  v[0] = fdk_sacenc_staticGain_OpenConfig(NULL);
  v[1] = fdk_sacenc_staticGain_Open(NULL);
  v[2] = fdk_sacenc_staticGain_CloseConfig(&c);
  v[3] = fdk_sacenc_staticGain_Close(&g);
  v[4] = fdk_sacenc_staticGain_InitDefaultConfig(NULL);
  v[5] = fdk_sacenc_staticGain_SetDmxGain(NULL, SACENC_DMXGAIN_0_dB);
  v[6] = fdk_sacenc_staticGain_SetEncMode(NULL, SACENC_212);
  v[7] = fdk_sacenc_staticGain_CloseConfig(NULL);
  v[8] = fdk_sacenc_staticGain_Close(NULL);
}
INT cref_sacenc_staticgain_close_config(void *h) {
  HANDLE_STATIC_GAIN_CONFIG p = (HANDLE_STATIC_GAIN_CONFIG)h;
  return fdk_sacenc_staticGain_CloseConfig(&p);
}
INT cref_sacenc_staticgain_close(void *h) {
  HANDLE_STATIC_GAIN p = (HANDLE_STATIC_GAIN)h;
  return fdk_sacenc_staticGain_Close(&p);
}
void cref_sacenc_staticgain_get_config(void *h, INT *v) { FDKmemcpy(v, h, sizeof(STATIC_GAIN_CONFIG)); }
void cref_sacenc_staticgain_set_config(void *h, const INT *v) { FDKmemcpy(h, v, sizeof(STATIC_GAIN_CONFIG)); }
void cref_sacenc_staticgain_get(void *h, INT *v) { FDKmemcpy(v, h, sizeof(rsg::STATIC_GAIN)); }
void cref_sacenc_staticgain_set(void *h, const INT *v) { FDKmemcpy(h, v, sizeof(rsg::STATIC_GAIN)); }
INT cref_sacenc_staticgain_init_default(void *c) {
  return fdk_sacenc_staticGain_InitDefaultConfig((HANDLE_STATIC_GAIN_CONFIG)c);
}
INT cref_sacenc_staticgain_set_dmx_gain(void *c, INT g) {
  return fdk_sacenc_staticGain_SetDmxGain((HANDLE_STATIC_GAIN_CONFIG)c, (MP4SPACEENC_DMX_GAIN)g);
}
INT cref_sacenc_staticgain_set_enc_mode(void *c, INT m) {
  return fdk_sacenc_staticGain_SetEncMode((HANDLE_STATIC_GAIN_CONFIG)c, (MP4SPACEENC_MODE)m);
}
INT cref_sacenc_staticgain_init(void *g, void *c, INT *scale) {
  return fdk_sacenc_staticGain_Init((HANDLE_STATIC_GAIN)g, (HANDLE_STATIC_GAIN_CONFIG)c, scale);
}
INT cref_sacenc_staticgain_apply(void *g, SHORT *pcm, INT n, INT scale) {
  return fdk_sacenc_staticPostGain_ApplyFDK((HANDLE_STATIC_GAIN)g, pcm, n, scale);
}
/* getPreGainPtrFDK (both values, and whether it points into the instance), getPostGainFDK,
   GetDmxGain */
void cref_sacenc_staticgain_getters(void *g, INT *v) {
  HANDLE_STATIC_GAIN h = (HANDLE_STATIC_GAIN)g;
  FIXP_GAIN *p = fdk_sacenc_getPreGainPtrFDK(h);
  v[0] = p[0];
  v[1] = p[1];
  v[2] = (p == ((rsg::STATIC_GAIN *)g)->pPreGain__FDK);
  v[3] = fdk_sacenc_getPostGainFDK(h);
  v[4] = fdk_sacenc_staticGain_GetDmxGain(h);
  v[5] = (fdk_sacenc_getPreGainPtrFDK(NULL) == NULL);
}
}

extern "C" {
/* The file-static tables: 0 preGainFactorTable__FDK, 1 dmxGainTable__FDK. Returns the number
   of entries. */
INT cref_sacenc_staticgain_table(INT id, INT *out) {
  const FIXP_DBL *t = id == 0 ? rsg::preGainFactorTable__FDK : rsg::dmxGainTable__FDK;
  INT n = id == 0 ? sizeof(rsg::preGainFactorTable__FDK) / sizeof(FIXP_DBL)
                  : sizeof(rsg::dmxGainTable__FDK) / sizeof(FIXP_DBL);
  for (INT i = 0; i < n; i++) out[i] = t[i];
  return n;
}
}
