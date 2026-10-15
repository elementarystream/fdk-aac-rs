// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_tree.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_tree (see build.rs).
//
// The space tree of the library; its state is read through the copy of the structure in
// namespace rst: mode, descr (3), the TTO box image of sacenc_tto_state (156), nParamBands,
// bUseCoarseQuantTtoIcc, bUseCoarseQuantTtoCld, quantMode, frameCount, bFrameKeep,
// pCld_prev[28], pIcc_prev[28], nChannelsInMax, nHybridBandsMax (= 224 INTs). The hybrid data
// of the two channels are passed as one INT array (channel, slot, band, re/im) and used as
// both pppHybrid__FDK and pppHybridIn__FDK, as the encoder does.

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

/* The whole of libSACenc in namespace rst (its headers included there first), so that
   the structures of the .cpp files and the handles that point to them live in the namespace
   with the layout of the library. The extern "C" API functions of the copy are renamed. */
#define FDK_sacenc_open rst_FDK_sacenc_open
#define FDK_sacenc_init rst_FDK_sacenc_init
#define FDK_sacenc_close rst_FDK_sacenc_close
#define FDK_sacenc_encode rst_FDK_sacenc_encode
#define FDK_sacenc_getInfo rst_FDK_sacenc_getInfo
#define FDK_sacenc_setParam rst_FDK_sacenc_setParam
#define FDK_sacenc_getLibInfo rst_FDK_sacenc_getLibInfo
namespace rst {
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
void sacenc_tto_state(const void *box, INT *out);

extern "C" {
INT cref_sacenc_tree_state(void *h, INT *v) {
  rst::SPACE_TREE *t = (rst::SPACE_TREE *)h;
  INT k = 0;
  v[k++] = t->mode;
  v[k++] = t->descr.nOttBoxes;
  v[k++] = t->descr.nInChannels;
  v[k++] = t->descr.nOutChannels;
  if (t->ttoBox[0] != NULL) {
    sacenc_tto_state(t->ttoBox[0], v + k);
  } else {
    for (INT i = 0; i < 156; i++) v[k + i] = -12345;
  }
  k += 156;
  v[k++] = t->nParamBands;
  v[k++] = t->bUseCoarseQuantTtoIcc;
  v[k++] = t->bUseCoarseQuantTtoCld;
  v[k++] = t->quantMode;
  v[k++] = t->frameCount;
  v[k++] = t->bFrameKeep;
  for (INT i = 0; i < MAX_NUM_PARAM_BANDS; i++) v[k++] = t->pCld_prev[0][i];
  for (INT i = 0; i < MAX_NUM_PARAM_BANDS; i++) v[k++] = t->pIcc_prev[0][i];
  v[k++] = t->nChannelsInMax;
  v[k++] = t->nHybridBandsMax;
  return k;
}
void *cref_sacenc_tree_open(void) {
  HANDLE_SPACE_TREE h = NULL;
  if (fdk_sacenc_spaceTree_Open(&h) != SACENC_OK) return NULL;
  return h;
}
INT cref_sacenc_tree_open_null(void) { return fdk_sacenc_spaceTree_Open(NULL); }
INT cref_sacenc_tree_close(void *h) {
  HANDLE_SPACE_TREE p = (HANDLE_SPACE_TREE)h;
  return fdk_sacenc_spaceTree_Close(&p);
}
INT cref_sacenc_tree_close_null(void) { return fdk_sacenc_spaceTree_Close(NULL); }
/* offsets: the array the TTO boxes keep a pointer to (NULL if hasOffsets == 0) */
INT cref_sacenc_tree_init(void *h, INT nParamBands, INT coarseCld, INT coarseIcc, INT quantMode, INT mode, INT nChannelsInMax, INT nHybridBandsMax, UCHAR *offsets, INT frameKeep) {
  SPACE_TREE_SETUP s;
  s.nParamBands = nParamBands;
  s.bUseCoarseQuantTtoCld = coarseCld;
  s.bUseCoarseQuantTtoIcc = coarseIcc;
  s.quantMode = (QUANTMODE)quantMode;
  s.mode = (SPACETREE_MODE)mode;
  s.nChannelsInMax = nChannelsInMax;
  s.nHybridBandsMax = nHybridBandsMax;
  return fdk_sacenc_spaceTree_Init((HANDLE_SPACE_TREE)h, &s, offsets, frameKeep);
}
INT cref_sacenc_tree_getDescription(void *h, UCHAR *d) {
  SPACE_TREE_DESCRIPTION s;
  INT r = fdk_sacenc_spaceTree_GetDescription((HANDLE_SPACE_TREE)h, &s);
  d[0] = s.nOttBoxes;
  d[1] = s.nInChannels;
  d[2] = s.nOutChannels;
  return r;
}
INT cref_sacenc_tree_Hybrid2ParamBand(INT nParamBands, INT band) { return fdk_sacenc_spaceTree_Hybrid2ParamBand(nParamBands, band); }
/* hyb: 2 channels x slots x bands (re, im); frame: SPATIALFRAME image */
INT cref_sacenc_tree_apply(void *h, INT paramSet, INT nChannelsIn, INT nTimeSlots, INT start, INT nHybridBands, INT *win, INT *hyb, INT slots, INT bands, UCHAR *frame, INT avoidKeep, INT *scale) {
  FIXP_DPK **ch[2];
  SPATIALFRAME f;
  FDKmemcpy(&f, frame, sizeof(f));
  for (INT c = 0; c < 2; c++) ch[c] = sacenc_mat_new(hyb + 2 * c * slots * bands, slots, bands);
  INT r = fdk_sacenc_spaceTree_Apply((HANDLE_SPACE_TREE)h, paramSet, nChannelsIn, nTimeSlots, start, nHybridBands, (FIXP_WIN *)win, ch, ch, &f, avoidKeep, scale);
  for (INT c = 0; c < 2; c++) {
    sacenc_mat_get(ch[c], hyb + 2 * c * slots * bands, slots, bands);
    sacenc_mat_free(ch[c], slots);
  }
  FDKmemcpy(frame, &f, sizeof(f));
  return r;
}
}

extern "C" {
/* The file-static tables as INTs: 0 treeConfigTable as (mode, nOttBoxes, nInChannels,
   nOutChannels), 1 treeSetupTable as (mode, nChannelsIn, nChannelsOut, nTtoBoxes, boxId,
   inCh1, inCh2, inCh3, inCh4, wCh1, wCh2) per entry. Returns the number of INTs. */
INT cref_sacenc_tree_table(INT id, INT *out) {
  INT k = 0;
  if (id == 0) {
    for (UINT i = 0; i < sizeof(rst::treeConfigTable) / sizeof(rst::treeConfigTable[0]); i++) {
      out[k++] = rst::treeConfigTable[i].mode;
      out[k++] = rst::treeConfigTable[i].treeDescription.nOttBoxes;
      out[k++] = rst::treeConfigTable[i].treeDescription.nInChannels;
      out[k++] = rst::treeConfigTable[i].treeDescription.nOutChannels;
    }
  } else {
    for (UINT i = 0; i < sizeof(rst::treeSetupTable) / sizeof(rst::treeSetupTable[0]); i++) {
      const rst::TREE_SETUP *s = &rst::treeSetupTable[i];
      const rst::TTO_DESCRIPTOR *d = &s->tto_descriptor[0];
      INT v[11] = {s->mode, s->nChannelsIn, s->nChannelsOut, s->nTtoBoxes, d->boxId, d->inCh1,
                   d->inCh2, d->inCh3, d->inCh4, d->wCh1, d->wCh2};
      for (INT j = 0; j < 11; j++) out[k++] = v[j];
    }
  }
  return k;
}
}
