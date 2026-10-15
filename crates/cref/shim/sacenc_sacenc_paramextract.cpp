// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_paramextract.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_paramextract (see build.rs).
//
// The non-static functions of the library; the file-static ones through the copy in namespace
// rpe. TTO_BOX is read as 156 INTs (sacenc_tto_state): pCld__FDK[28], pIcc__FDK[28],
// pCldQuant__FDK[28], the tables that pIccQuantTable__FDK, pCldQuantTableDec__FDK and
// pCldQuantTableEnc__FDK point to (0 NULL, 1 fine, 2 coarse, 3 other), pCldEbQIdx[28],
// pIccDownmixIdx[28], whether pParameterBand2HybridBandOffset and pSubbandImagSign are set,
// and the 11 scalar fields. Hybrid data are row-major INT arrays (re, im).

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

/* The whole of libSACenc in namespace rpe (its headers included there first), so that
   the structures of the .cpp files and the handles that point to them live in the namespace
   with the layout of the library. The extern "C" API functions of the copy are renamed. */
#define FDK_sacenc_open rpe_FDK_sacenc_open
#define FDK_sacenc_init rpe_FDK_sacenc_init
#define FDK_sacenc_close rpe_FDK_sacenc_close
#define FDK_sacenc_encode rpe_FDK_sacenc_encode
#define FDK_sacenc_getInfo rpe_FDK_sacenc_getInfo
#define FDK_sacenc_setParam rpe_FDK_sacenc_setParam
#define FDK_sacenc_getLibInfo rpe_FDK_sacenc_getLibInfo
namespace rpe {
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

/* The state image of a TTO_BOX (of the library or of any namespaced copy: same layout). The
   table pointers are compared with the tables of the library (fdk_sacenc_initTtoBox of the
   library sets them) and of the copy in rpe. */
void sacenc_tto_state(const void *box, INT *out) {
  const rpe::TTO_BOX *b = (const rpe::TTO_BOX *)box;
  INT k = 0;
  for (INT i = 0; i < MAX_NUM_PARAM_BANDS; i++) out[k++] = b->pCld__FDK[i];
  for (INT i = 0; i < MAX_NUM_PARAM_BANDS; i++) out[k++] = b->pIcc__FDK[i];
  for (INT i = 0; i < MAX_NUM_PARAM_BANDS; i++) out[k++] = b->pCldQuant__FDK[i];
  /* The library's tables are file-static; identify them by their contents. */
  out[k++] = b->pIccQuantTable__FDK == NULL ? 0 : (b->pIccQuantTable__FDK[1] == rpe::iccQuantTableFine__FDK[1] ? 1 : 2);
  out[k++] = b->pCldQuantTableDec__FDK == NULL ? 0 : (b->pCldQuantTableDec__FDK[1] == rpe::cldQuantTableFineDec__FDK[1] ? 1 : 2);
  out[k++] = b->pCldQuantTableEnc__FDK == NULL ? 0 : (b->pCldQuantTableEnc__FDK[1] == rpe::cldQuantTableFineEnc__FDK[1] ? 1 : 2);
  for (INT i = 0; i < MAX_NUM_PARAM_BANDS; i++) out[k++] = b->pCldEbQIdx[i];
  for (INT i = 0; i < MAX_NUM_PARAM_BANDS; i++) out[k++] = b->pIccDownmixIdx[i];
  out[k++] = b->pParameterBand2HybridBandOffset != NULL;
  out[k++] = b->pSubbandImagSign != NULL;
  out[k++] = b->nHybridBandsMax;
  out[k++] = b->nParameterBands;
  out[k++] = b->bFrameKeep;
  out[k++] = b->iccCorrelationCoherenceBorder;
  out[k++] = b->boxQuantMode;
  out[k++] = b->nIccQuantSteps;
  out[k++] = b->nIccQuantOffset;
  out[k++] = b->nCldQuantSteps;
  out[k++] = b->nCldQuantOffset;
  out[k++] = b->bUseCoarseQuantCld;
  out[k++] = b->bUseCoarseQuantIcc;
}

static const FIXP_DBL *table(INT id) {
  switch (id) {
    case 0: return rpe::cldQuantTableFineEnc__FDK;
    case 1: return rpe::cldQuantTableCoarseEnc__FDK;
    case 2: return rpe::cldQuantTableFineDec__FDK;
    case 3: return rpe::cldQuantTableCoarseDec__FDK;
    case 4: return rpe::iccQuantTableFine__FDK;
    default: return rpe::iccQuantTableCoarse__FDK;
  }
}

extern "C" {
/* subbandConfig, nParameterBands, iccCorrelationCoherenceBorder of the entry; -1 if none */
INT cref_sacenc_paramextract_getBoxSubbandSetup(INT config, INT *out) {
  const rpe::BOX_SUBBAND_SETUP *s = rpe::getBoxSubbandSetup((rpe::BOX_SUBBAND_CONFIG)config);
  if (s == NULL) return -1;
  out[0] = s->subbandConfig;
  out[1] = s->nParameterBands;
  out[2] = s->iccCorrelationCoherenceBorder;
  return 0;
}
void cref_sacenc_paramextract_ApplyBBCuesFDK(INT *data, INT n) { rpe::ApplyBBCuesFDK((FIXP_DBL *)data, n); }
INT cref_sacenc_paramextract_getNumberParameterBands(INT config) { return rpe::getNumberParameterBands((rpe::BOX_SUBBAND_CONFIG)config); }
INT cref_sacenc_paramextract_getSubband2ParameterIndex(INT config, UCHAR *out) {
  const UCHAR *p = rpe::getSubband2ParameterIndex((rpe::BOX_SUBBAND_CONFIG)config);
  if (p == NULL) return 0;
  FDKmemcpy(out, p, NUM_QMF_BANDS);
  return 1;
}
INT cref_sacenc_paramextract_getIccCorrelationCoherenceBorder(INT config, INT coherenceOnly) {
  return rpe::getIccCorrelationCoherenceBorder((rpe::BOX_SUBBAND_CONFIG)config, coherenceOnly);
}
void cref_sacenc_paramextract_calcParameterBand2HybridBandOffset(INT config, INT nHybridBands, UCHAR *out) {
  fdk_sacenc_calcParameterBand2HybridBandOffset((BOX_SUBBAND_CONFIG)config, nHybridBands, out);
}
void cref_sacenc_paramextract_getSubbandImagSign(INT *out) { FDKmemcpy(out, fdk_sacenc_getSubbandImagSign(), NUM_QMF_BANDS * sizeof(INT)); }
INT cref_sacenc_paramextract_subband2ParamBand(INT config, INT nSubband) {
  return fdk_sacenc_subband2ParamBand((BOX_SUBBAND_CONFIG)config, nSubband);
}
INT cref_sacenc_paramextract_calculateIccFDK(INT n, INT border, const INT *p1, const INT *p2, const INT *re, const INT *im, INT *icc) {
  return rpe::calculateIccFDK(n, border, (const FIXP_DBL *)p1, (const FIXP_DBL *)p2, (const FIXP_DBL *)re, (const FIXP_DBL *)im, (FIXP_DBL *)icc);
}
/* table: 0 cld fine enc, 1 cld coarse enc, 2 cld fine dec, 3 cld coarse dec, 4 icc fine, 5 icc coarse */
void cref_sacenc_paramextract_QuantizeCoefFDK(const INT *in, INT n, INT tab, INT offset, INT steps, SCHAR *out) {
  rpe::QuantizeCoefFDK((const FIXP_DBL *)in, n, table(tab), offset, steps, out);
}
void cref_sacenc_paramextract_deQuantizeCoefFDK(const SCHAR *in, INT n, INT tab, INT offset, INT *out) {
  rpe::deQuantizeCoefFDK(in, n, table(tab), offset, (FIXP_DBL *)out);
}
void cref_sacenc_paramextract_CalculateCldFDK(INT *cld, const INT *p1, const INT *p2, INT s1, const INT *pb1, INT s2, const INT *pb2, INT n) {
  rpe::CalculateCldFDK((FIXP_DBL *)cld, (const FIXP_DBL *)p1, (const FIXP_DBL *)p2, s1, pb1, s2, pb2, n);
}

/* A TTO box of the library. */
void *cref_sacenc_paramextract_create(void) {
  HANDLE_TTO_BOX h = NULL;
  if (fdk_sacenc_createTtoBox(&h) != SACENC_OK) return NULL;
  return h;
}
INT cref_sacenc_paramextract_create_null(void) { return fdk_sacenc_createTtoBox(NULL); }
INT cref_sacenc_paramextract_destroy(void *h) {
  HANDLE_TTO_BOX p = (HANDLE_TTO_BOX)h;
  return fdk_sacenc_destroyTtoBox(&p);
}
void cref_sacenc_paramextract_state(void *h, INT *out) { sacenc_tto_state(h, out); }
/* Sets the parameter values (pCld, pIcc, pCldQuant, pCldEbQIdx, pIccDownmixIdx) of the box. */
void cref_sacenc_paramextract_set_values(void *h, const INT *cld, const INT *icc, const INT *cldq, const SCHAR *ebq, const SCHAR *dmx) {
  rpe::TTO_BOX *b = (rpe::TTO_BOX *)h;
  FDKmemcpy(b->pCld__FDK, cld, sizeof(b->pCld__FDK));
  FDKmemcpy(b->pIcc__FDK, icc, sizeof(b->pIcc__FDK));
  FDKmemcpy(b->pCldQuant__FDK, cldq, sizeof(b->pCldQuant__FDK));
  FDKmemcpy(b->pCldEbQIdx, ebq, sizeof(b->pCldEbQIdx));
  FDKmemcpy(b->pIccDownmixIdx, dmx, sizeof(b->pIccDownmixIdx));
}
/* offsets: the array the box keeps a pointer to (NULL if hasOffsets == 0) */
INT cref_sacenc_paramextract_init(void *h, INT coarseCld, INT coarseIcc, INT coherenceOnly, INT config, INT quantMode, INT nHybridBandsMax, INT frameKeep, UCHAR *offsets) {
  TTO_BOX_CONFIG c;
  c.bUseCoarseQuantCld = coarseCld;
  c.bUseCoarseQuantIcc = coarseIcc;
  c.bUseCoherenceIccOnly = coherenceOnly;
  c.subbandConfig = (BOX_SUBBAND_CONFIG)config;
  c.boxQuantMode = (BOX_QUANTMODE)quantMode;
  c.nHybridBandsMax = nHybridBandsMax;
  c.bFrameKeep = frameKeep;
  return fdk_sacenc_initTtoBox((HANDLE_TTO_BOX)h, &c, offsets);
}
/* d1, d2: slots x bands */
INT cref_sacenc_paramextract_apply(void *h, INT nTimeSlots, INT start, INT nHybridBands, const INT *d1, const INT *d2, INT slots, INT bands, SCHAR *icc, UCHAR *iccCoarse, SCHAR *cld, UCHAR *cldCoarse, INT bbCues, INT s1, INT s2) {
  FIXP_DPK **m1 = sacenc_mat_new(d1, slots, bands);
  FIXP_DPK **m2 = sacenc_mat_new(d2, slots, bands);
  INT r = fdk_sacenc_applyTtoBox((HANDLE_TTO_BOX)h, nTimeSlots, start, nHybridBands, m1, m2, icc, iccCoarse, cld, cldCoarse, bbCues, &s1, &s2);
  sacenc_mat_free(m1, slots);
  sacenc_mat_free(m2, slots);
  return r;
}
}

extern "C" {
/* The file-static tables as INTs: 0..6 subband2Parameter{4,5,7,9,12,15,23}_Ld,
   7 subbandImagSign_Ld, 8..13 cldQuantTable{FineEnc,CoarseEnc,FineDec,CoarseDec}__FDK and
   iccQuantTable{Fine,Coarse}__FDK, 14 boxSubbandSetup as (subbandConfig, nParameterBands, index
   of the table of pSubband2ParameterIndexLd among 0..6, iccCorrelationCoherenceBorder).
   Returns the number of INTs. */
INT cref_sacenc_paramextract_table(INT id, INT *out) {
  const UCHAR *s[7] = {rpe::subband2Parameter4_Ld,  rpe::subband2Parameter5_Ld,
                       rpe::subband2Parameter7_Ld,  rpe::subband2Parameter9_Ld,
                       rpe::subband2Parameter12_Ld, rpe::subband2Parameter15_Ld,
                       rpe::subband2Parameter23_Ld};
  const FIXP_DBL *q[6] = {rpe::cldQuantTableFineEnc__FDK, rpe::cldQuantTableCoarseEnc__FDK,
                          rpe::cldQuantTableFineDec__FDK, rpe::cldQuantTableCoarseDec__FDK,
                          rpe::iccQuantTableFine__FDK,    rpe::iccQuantTableCoarse__FDK};
  const INT qn[6] = {MAX_CLD_QUANT_FINE, MAX_CLD_QUANT_COARSE, MAX_CLD_QUANT_FINE,
                     MAX_CLD_QUANT_COARSE, MAX_ICC_QUANT_FINE, MAX_ICC_QUANT_COARSE};
  if (id < 7) {
    for (INT i = 0; i < NUM_QMF_BANDS; i++) out[i] = s[id][i];
    return NUM_QMF_BANDS;
  }
  if (id == 7) {
    for (INT i = 0; i < NUM_QMF_BANDS; i++) out[i] = rpe::subbandImagSign_Ld[i];
    return NUM_QMF_BANDS;
  }
  if (id < 14) {
    for (INT i = 0; i < qn[id - 8]; i++) out[i] = q[id - 8][i];
    return qn[id - 8];
  }
  INT n = sizeof(rpe::boxSubbandSetup) / sizeof(rpe::boxSubbandSetup[0]);
  for (INT i = 0; i < n; i++) {
    INT k = -1;
    for (INT j = 0; j < 7; j++)
      if (rpe::boxSubbandSetup[i].pSubband2ParameterIndexLd == s[j]) k = j;
    out[4 * i] = rpe::boxSubbandSetup[i].subbandConfig;
    out[4 * i + 1] = rpe::boxSubbandSetup[i].nParameterBands;
    out[4 * i + 2] = k;
    out[4 * i + 3] = rpe::boxSubbandSetup[i].iccCorrelationCoherenceBorder;
  }
  return 4 * n;
}
}
