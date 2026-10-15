// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_framewindowing.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_framewindowing (see build.rs).
//
// The frame windowing of the library; FRAMEWINDOW (72 INTs) is read and set as an INT image
// through the copy in namespace rfw, which also gives the file-static functions. FRAMEWIN_LIST
// is passed as 5 INTs (dat[0].slot, dat[0].hold, dat[1].slot, dat[1].hold, n), FRAMINGINFO as
// 4 bytes, the two analysis windows as 2 x 48 INTs.

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

/* The whole of libSACenc in namespace rfw (its headers included there first), so that
   the structures of the .cpp files and the handles that point to them live in the namespace
   with the layout of the library. The extern "C" API functions of the copy are renamed. */
#define FDK_sacenc_open rfw_FDK_sacenc_open
#define FDK_sacenc_init rfw_FDK_sacenc_init
#define FDK_sacenc_close rfw_FDK_sacenc_close
#define FDK_sacenc_encode rfw_FDK_sacenc_encode
#define FDK_sacenc_getInfo rfw_FDK_sacenc_getInfo
#define FDK_sacenc_setParam rfw_FDK_sacenc_setParam
#define FDK_sacenc_getLibInfo rfw_FDK_sacenc_getLibInfo
namespace rfw {
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
void sacenc_mat_get(FIXP_DPK **p, INT *v, INT rows, INT cols);
void sacenc_mat_free(FIXP_DPK **p, INT rows);

static void list_from(rfw::FRAMEWIN_LIST *l, const INT *v) {
  l->dat[0].slot = v[0];
  l->dat[0].hold = (rfw::FW_SLOTTYPE)v[1];
  l->dat[1].slot = v[2];
  l->dat[1].hold = (rfw::FW_SLOTTYPE)v[3];
  l->n = v[4];
}
static void list_to(const rfw::FRAMEWIN_LIST *l, INT *v) {
  v[0] = l->dat[0].slot;
  v[1] = l->dat[0].hold;
  v[2] = l->dat[1].slot;
  v[3] = l->dat[1].hold;
  v[4] = l->n;
}

extern "C" {
INT cref_sacenc_framewindowing_size(void) { return sizeof(rfw::FRAMEWINDOW) / sizeof(INT); }
void *cref_sacenc_framewindowing_create(void) {
  HANDLE_FRAMEWINDOW h = NULL;
  if (fdk_sacenc_frameWindow_Create(&h) != SACENC_OK) return NULL;
  return h;
}
INT cref_sacenc_framewindowing_create_null(void) { return fdk_sacenc_frameWindow_Create(NULL); }
INT cref_sacenc_framewindowing_destroy(void *h) {
  HANDLE_FRAMEWINDOW p = (HANDLE_FRAMEWINDOW)h;
  return fdk_sacenc_frameWindow_Destroy(&p);
}
void cref_sacenc_framewindowing_get(void *h, INT *v) { FDKmemcpy(v, h, sizeof(rfw::FRAMEWINDOW)); }
void cref_sacenc_framewindowing_set(void *h, const INT *v) { FDKmemcpy(h, v, sizeof(rfw::FRAMEWINDOW)); }
INT cref_sacenc_framewindowing_init(void *h, INT nTimeSlotsMax, INT bFrameKeep) {
  FRAMEWINDOW_CONFIG c;
  c.nTimeSlotsMax = nTimeSlotsMax;
  c.bFrameKeep = bFrameKeep;
  return fdk_sacenc_frameWindow_Init((HANDLE_FRAMEWINDOW)h, &c);
}
INT cref_sacenc_framewindowing_getWindow(void *h, INT *trPos, INT timeSlots, UCHAR *framing, INT *win0, INT *win1, INT *list, INT avoidKeep) {
  FRAMINGINFO fi;
  FRAMEWIN_LIST l;
  FIXP_WIN *w[MAX_NUM_PARAMS] = {(FIXP_DBL *)win0, (FIXP_DBL *)win1};
  FDKmemcpy(&fi, framing, sizeof(fi));
  list_from((rfw::FRAMEWIN_LIST *)&l, list);
  INT r = fdk_sacenc_frameWindow_GetWindow((HANDLE_FRAMEWINDOW)h, trPos, timeSlots, &fi, w, &l, avoidKeep);
  FDKmemcpy(framing, &fi, sizeof(fi));
  list_to((rfw::FRAMEWIN_LIST *)&l, list);
  return r;
}
/* inPlace: out is the input as well (FW_LEAVE_DIM). Matrices are rows x cols. */
INT cref_sacenc_framewindowing_analysisWindowing(INT nTimeSlots, INT start, INT *win, const INT *in, INT inRows, INT inCols, INT *out, INT outRows, INT outCols, INT nHybridBands, INT dim, INT inPlace) {
  FIXP_DPK **mo = sacenc_mat_new(out, outRows, outCols);
  FIXP_DPK **mi = inPlace ? mo : sacenc_mat_new(in, inRows, inCols);
  INT r = fdk_sacenc_analysisWindowing(nTimeSlots, start, (FIXP_WIN *)win, mi, mo, nHybridBands, dim);
  sacenc_mat_get(mo, out, outRows, outCols);
  if (!inPlace) sacenc_mat_free(mi, inRows);
  sacenc_mat_free(mo, outRows);
  return r;
}
void cref_sacenc_framewindowing_calcTaperWin(INT *w, INT timeSlots) { rfw::calcTaperWin((FIXP_DBL *)w, timeSlots); }
/* op: 0 Reset, 1 Add(a, b), 2 Remove(a), 3 Limit(a, b) */
INT cref_sacenc_framewindowing_list(INT *list, INT op, INT a, INT b) {
  rfw::FRAMEWIN_LIST l;
  list_from(&l, list);
  INT r = 0;
  switch (op) {
    case 0: r = rfw::FrameWinList_Reset(&l); break;
    case 1: r = rfw::FrameWindowList_Add(&l, a, (rfw::FW_SLOTTYPE)b); break;
    case 2: r = rfw::FrameWindowList_Remove(&l, a); break;
    default: r = rfw::FrameWindowList_Limit(&l, a, b); break;
  }
  list_to(&l, list);
  return r;
}
}
