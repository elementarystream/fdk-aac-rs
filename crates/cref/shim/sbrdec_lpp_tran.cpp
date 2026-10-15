// Shims for the differential tests of fdk_aac_rs_sbrdec::lpp_tran (tests/sbrdec/lpp_tran.rs).
// The transposer state and the settings are passed as memory (SbrLppTrans and
// TransposerSettings have the layout of SBR_LPP_TRANS and TRANSPOSER_SETTINGS); QMF data as flat
// arrays of slots of 64 values.

#include "cref_common.h"
#include "libSBRdec/src/lpp_tran.h"

extern "C" {

INT cref_sbrdec_lpp_tran_create(void *hs, void *settings, INT highBandStartSb, UCHAR *vkMaster, INT numMaster, INT usb, INT timeSlots, INT nCols, UCHAR *noiseBandTable, INT noNoiseBands, UINT fs, INT chan, INT overlap) {
  SBR_LPP_TRANS *h = (SBR_LPP_TRANS *)hs;
  INT r = createLppTransposer(h, (TRANSPOSER_SETTINGS *)settings, highBandStartSb, vkMaster, numMaster, usb, timeSlots, nCols, noiseBandTable, noNoiseBands, fs, chan, overlap);
  h->pSettings = NULL;
  return r;
}

INT cref_sbrdec_lpp_tran_reset(void *settings, INT highBandStartSb, UCHAR *vkMaster, INT numMaster, UCHAR *noiseBandTable, INT noNoiseBands, INT usb, UINT fs) {
  SBR_LPP_TRANS h;
  FDKmemclear(&h, sizeof(h));
  h.pSettings = (TRANSPOSER_SETTINGS *)settings;
  return resetLppTransposer(&h, (UCHAR)highBandStartSb, vkMaster, (UCHAR)numMaster, noiseBandTable, (UCHAR)noNoiseBands, (UCHAR)usb, fs);
}

/* scale: QMF_SCALE_FACTOR (4 values, updated); re/im: nSlots slots */
void cref_sbrdec_lpp_tran_apply(void *hs, void *settings, INT *scale, FIXP_DBL *re, FIXP_DBL *degreeAlias, FIXP_DBL *im, INT nSlots, INT useLP, INT preWhitening, INT vkMaster0, INT timeStep, INT firstSlot, INT lastSlot, INT nInvfBands, INT *invf, INT *invfPrev) {
  SBR_LPP_TRANS *h = (SBR_LPP_TRANS *)hs;
  h->pSettings = (TRANSPOSER_SETTINGS *)settings;
  FIXP_DBL *r[128], *i[128];
  for (int k = 0; k < nSlots && k < 128; k++) {
    r[k] = re + 64 * k;
    i[k] = im + 64 * k;
  }
  QMF_SCALE_FACTOR sf = {scale[0], scale[1], scale[2], scale[3]};
  INVF_MODE m[MAX_INVF_BANDS], mp[MAX_INVF_BANDS];
  for (int k = 0; k < MAX_INVF_BANDS; k++) {
    m[k] = (INVF_MODE)invf[k];
    mp[k] = (INVF_MODE)invfPrev[k];
  }
  lppTransposer(h, &sf, r, degreeAlias, i, useLP, preWhitening, vkMaster0, timeStep, firstSlot, lastSlot, nInvfBands, m, mp);
  h->pSettings = NULL;
  scale[0] = sf.lb_scale;
  scale[1] = sf.ov_lb_scale;
  scale[2] = sf.hb_scale;
  scale[3] = sf.ov_hb_scale;
}

/* lppTransposerHBE with the transposer hbe (the instance of shim/sbrdec_hbe.cpp) */
void cref_sbrdec_lpp_tran_apply_hbe(void *hs, void *settings, void *hbe, INT *scale, FIXP_DBL *re, FIXP_DBL *im, INT nSlots, INT timeStep, INT firstSlot, INT lastSlot, INT nInvfBands, INT *invf, INT *invfPrev) {
  SBR_LPP_TRANS *h = (SBR_LPP_TRANS *)hs;
  h->pSettings = (TRANSPOSER_SETTINGS *)settings;
  FIXP_DBL *r[128], *i[128];
  for (int k = 0; k < nSlots && k < 128; k++) {
    r[k] = re + 64 * k;
    i[k] = im + 64 * k;
  }
  QMF_SCALE_FACTOR sf = {scale[0], scale[1], scale[2], scale[3]};
  INVF_MODE m[MAX_INVF_BANDS], mp[MAX_INVF_BANDS];
  for (int k = 0; k < MAX_INVF_BANDS; k++) {
    m[k] = (INVF_MODE)invf[k];
    mp[k] = (INVF_MODE)invfPrev[k];
  }
  lppTransposerHBE(h, (HANDLE_HBE_TRANSPOSER)hbe, &sf, r, i, timeStep, firstSlot, lastSlot, nInvfBands, m, mp);
  h->pSettings = NULL;
  scale[0] = sf.lb_scale;
  scale[1] = sf.ov_lb_scale;
  scale[2] = sf.hb_scale;
  scale[3] = sf.ov_hb_scale;
}

}  // extern "C"
