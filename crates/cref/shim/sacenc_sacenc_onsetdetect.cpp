// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_onsetdetect.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_onsetdetect (see build.rs).
//
// The onset detection of the library; its state (6 INTs: maxTimeSlots, minTransientDistance,
// avgEnergyDistance, lower and upper bound, avgEnergyDistanceScale; the energy history and its
// scales) is read and set through the copy of the structure in namespace ron. Hybrid data are
// row-major INT arrays (re, im) as in sacenc_sacenc_vectorfunctions.cpp.

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

/* The whole of libSACenc in namespace ron (its headers included there first), so that
   the structures of the .cpp files and the handles that point to them live in the namespace
   with the layout of the library. The extern "C" API functions of the copy are renamed. */
#define FDK_sacenc_open ron_FDK_sacenc_open
#define FDK_sacenc_init ron_FDK_sacenc_init
#define FDK_sacenc_close ron_FDK_sacenc_close
#define FDK_sacenc_encode ron_FDK_sacenc_encode
#define FDK_sacenc_getInfo ron_FDK_sacenc_getInfo
#define FDK_sacenc_setParam ron_FDK_sacenc_setParam
#define FDK_sacenc_getLibInfo ron_FDK_sacenc_getLibInfo
namespace ron {
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

FIXP_DPK **sacenc_mat_new(const INT *v, INT rows, INT cols);
void sacenc_mat_free(FIXP_DPK **p, INT rows);

static ron::ONSET_DETECT *ond(void *h) { return (ron::ONSET_DETECT *)h; }

extern "C" {
void *cref_sacenc_onset_open(UINT maxTimeSlots) {
  HANDLE_ONSET_DETECT h = NULL;
  if (fdk_sacenc_onsetDetect_Open(&h, maxTimeSlots) != SACENC_OK) return NULL;
  return h;
}
INT cref_sacenc_onset_open_null(void) { return fdk_sacenc_onsetDetect_Open(NULL, 16); }
INT cref_sacenc_onset_close(void *h) {
  HANDLE_ONSET_DETECT p = (HANDLE_ONSET_DETECT)h;
  return fdk_sacenc_onsetDetect_Close(&p);
}
/* n: the number of history values (16 + maxTimeSlots of the open call) */
void cref_sacenc_onset_get(void *h, INT *v, INT *hist, SCHAR *scale, INT n) {
  ron::ONSET_DETECT *o = ond(h);
  v[0] = o->maxTimeSlots;
  v[1] = o->minTransientDistance;
  v[2] = o->avgEnergyDistance;
  v[3] = o->lowerBoundOnsetDetection;
  v[4] = o->upperBoundOnsetDetection;
  v[5] = o->avgEnergyDistanceScale;
  FDKmemcpy(hist, o->pEnergyHist__FDK, n * sizeof(FIXP_DBL));
  FDKmemcpy(scale, o->pEnergyHistScale, n);
}
void cref_sacenc_onset_set(void *h, const INT *v, const INT *hist, const SCHAR *scale, INT n) {
  ron::ONSET_DETECT *o = ond(h);
  o->maxTimeSlots = v[0];
  o->minTransientDistance = v[1];
  o->avgEnergyDistance = v[2];
  o->lowerBoundOnsetDetection = v[3];
  o->upperBoundOnsetDetection = v[4];
  o->avgEnergyDistanceScale = (SCHAR)v[5];
  FDKmemcpy(o->pEnergyHist__FDK, hist, n * sizeof(FIXP_DBL));
  FDKmemcpy(o->pEnergyHistScale, scale, n);
}
INT cref_sacenc_onset_init(void *h, INT maxTimeSlots, INT lower, INT upper, UINT flags) {
  ONSET_DETECT_CONFIG c;
  c.maxTimeSlots = maxTimeSlots;
  c.lowerBoundOnsetDetection = lower;
  c.upperBoundOnsetDetection = upper;
  return fdk_sacenc_onsetDetect_Init((HANDLE_ONSET_DETECT)h, &c, flags);
}
INT cref_sacenc_onset_update(void *h, INT timeSlots) { return fdk_sacenc_onsetDetect_Update((HANDLE_ONSET_DETECT)h, timeSlots); }
INT cref_sacenc_onset_apply(void *h, INT nTimeSlots, INT nHybridBands, const INT *data, INT rows, INT cols, INT scale, INT prevPos, INT *pos) {
  FIXP_DPK **m = sacenc_mat_new(data, rows, cols);
  INT r = fdk_sacenc_onsetDetect_Apply((HANDLE_ONSET_DETECT)h, nTimeSlots, nHybridBands, m, scale, prevPos, pos);
  sacenc_mat_free(m, rows);
  return r;
}
}
