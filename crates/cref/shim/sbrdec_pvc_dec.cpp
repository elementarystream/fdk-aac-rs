// Shims for the differential tests of fdk_aac_rs_sbrdec::pvc_dec.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrdec_pvc_dec (see build.rs).
//
// PVC_STATIC_DATA is passed as memory (PvcStaticData has its layout); PVC_DYNAMIC_DATA lives in
// the shim and is written out field by field (see dump_dynamic()). QMF data is passed as flat
// arrays of slots of 64 values.

#include "cref_common.h"
#include "libSBRdec/src/pvc_dec.h"

/* The values of PVC_DYNAMIC_DATA in the order of tests/sbrdec/pvc_dec.rs (dynamic()); tables
   are written as their contents, NULL as -999. */
static INT dump_dynamic(const PVC_DYNAMIC_DATA *d, INT *out) {
  INT n = 0;
  out[n++] = d->pvc_mode;
  out[n++] = d->pvcBorder0;
  out[n++] = d->kx;
  out[n++] = d->RATE;
  out[n++] = d->ns;
  for (int k = 0; k < PVC_NTIMESLOT; k++) out[n++] = (d->pPvcID == NULL) ? -999 : d->pPvcID[k];
  out[n++] = d->pastEsgSlotsAvail;
  int nsc = (d->ns == 16 || d->ns == 12 || d->ns == 4 || d->ns == 3) ? d->ns : 0;
  out[n++] = (d->pSCcoeffs == NULL) ? -999 : nsc;
  for (int k = 0; k < nsc && d->pSCcoeffs != NULL; k++) out[n++] = d->pSCcoeffs[k];
  for (int k = 0; k <= PVC_NBLOW; k++) out[n++] = d->sg_offset_low[k];
  for (int k = 0; k <= PVC_NBHIGH_MAX; k++) out[n++] = d->sg_offset_high_kx[k];
  out[n++] = d->nbHigh;
  for (int k = 0; k <= PVC_NBLOW; k++) out[n++] = (d->pScalingCoef == NULL) ? -999 : d->pScalingCoef[k];
  out[n++] = (d->pPVCTab1 == NULL) ? -999 : d->pPVCTab1[1];
  out[n++] = (d->pPVCTab2 == NULL) ? -999 : d->pPVCTab2[1];
  for (int k = 0; k < 2; k++) out[n++] = (d->pPVCTab1_dp == NULL) ? -999 : d->pPVCTab1_dp[k];
  for (int t = 0; t < PVC_NTIMESLOT; t++)
    for (int k = 0; k < PVC_NBHIGH_MAX; k++) out[n++] = d->predEsg[t][k];
  for (int t = 0; t < PVC_NTIMESLOT; t++) out[n++] = d->predEsg_exp[t];
  out[n++] = d->predEsg_expMax;
  return n;
}

extern "C" {

/* One PVC frame as in sbr_dec(): pvcInitFrame, pvcDecodeFrame (if the initialization succeeded
   and pvcMode > 0), pvcEndFrame, then expandPredEsg for every time slot (if pvcMode > 0) into
   expanded[16][lengthOut] / expandedExp[16][lengthOut]. Returns the result of pvcInitFrame;
   dyn: the dynamic data after pvcEndFrame. */
INT cref_sbrdec_pvc_frame(void *st, INT pvcMode, INT ns, INT rate, INT kx, INT border0, UCHAR *pvcId, FIXP_DBL *re, FIXP_DBL *im, INT nSlots, INT overlap, INT expOv, INT expCur, INT *dyn, INT *dynLen, INT lengthOut, FIXP_DBL *expanded, SCHAR *expandedExp) {
  PVC_STATIC_DATA *s = (PVC_STATIC_DATA *)st;
  PVC_DYNAMIC_DATA d;
  FDKmemclear(&d, sizeof(d));
  FIXP_DBL *r[128], *i[128];
  for (int k = 0; k < nSlots && k < 128; k++) {
    r[k] = re + 64 * k;
    i[k] = im + 64 * k;
  }
  INT res = pvcInitFrame(s, &d, (UCHAR)pvcMode, (UCHAR)ns, rate, kx, border0, pvcId);
  if (res == 0 && pvcMode > 0) {
    pvcDecodeFrame(s, &d, r, i, overlap, expOv, expCur);
  }
  pvcEndFrame(s, &d);
  if (res == 0 && pvcMode > 0) {
    for (int t = 0; t < PVC_NTIMESLOT; t++) {
      expandPredEsg(&d, t, lengthOut, expanded + t * lengthOut, expandedExp + t * lengthOut);
    }
  }
  *dynLen = dump_dynamic(&d, dyn);
  return res;
}

}  // extern "C"
