// Shims for the differential tests of fdk_aac_rs_drc::drc_dec_tools.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::drc_drc_dec_tools (see build.rs).
//
// Structures are passed as their memory images (the port has the same layout).

#include "cref_common.h"
#include "drcDec_types.h"
#include "drcDecoder.h"
#include "drcDec_tools.h"

extern "C" {
INT cref_drc_tools_get_delta_tmin(INT sampleRate) { return getDeltaTmin(sampleRate); }
FIXP_DBL cref_drc_tools_db2lin(FIXP_DBL dB_m, INT dB_e, INT *pLin_e) { return dB2lin(dB_m, dB_e, pLin_e); }
FIXP_DBL cref_drc_tools_lin2db(FIXP_DBL lin_m, INT lin_e, INT *pDb_e) { return lin2dB(lin_m, lin_e, pDb_e); }
FIXP_DBL cref_drc_tools_approx_db2lin(FIXP_DBL dB_m, INT dB_e, INT *pLin_e) { return approxDb2lin(dB_m, dB_e, pLin_e); }
FIXP_DBL cref_drc_tools_get_downmix_offset(const UCHAR *down, INT baseChannelCount) {
  DOWNMIX_INSTRUCTIONS d;
  FDKmemcpy(&d, down, sizeof(d));
  return getDownmixOffset(&d, baseChannelCount);
}
/* which: 0 selectDrcCoefficients, 1 selectDrcInstructions, 2 selectDownmixInstructions,
   3 bitstreamContainsMultibandDrc. Returns the index of the element (-1 for NULL) or the
   result. */
INT cref_drc_tools_select(const UCHAR *cfg, INT which, INT arg) {
  UNI_DRC_CONFIG *c = (UNI_DRC_CONFIG *)FDKcalloc(1, sizeof(UNI_DRC_CONFIG));
  FDKmemcpy(c, cfg, sizeof(UNI_DRC_CONFIG));
  INT r = -1;
  switch (which) {
    case 0: {
      DRC_COEFFICIENTS_UNI_DRC *p = selectDrcCoefficients(c, arg);
      if (p) r = (INT)(p - c->drcCoefficientsUniDrc);
    } break;
    case 1: {
      DRC_INSTRUCTIONS_UNI_DRC *p = selectDrcInstructions(c, arg);
      if (p) r = (INT)(p - c->drcInstructionsUniDrc);
    } break;
    case 2: {
      DOWNMIX_INSTRUCTIONS *p = selectDownmixInstructions(c, arg);
      if (p) r = (INT)(p - c->downmixInstructions);
    } break;
    default:
      r = bitstreamContainsMultibandDrc(c, arg);
  }
  FDKfree(c);
  return r;
}
/* deriveDrcChannelGroups; duck/duckGroup are 8 DUCKING_MODIFICATION each (passed as NULL if
   useDuck is 0). */
INT cref_drc_tools_derive_drc_channel_groups(INT drcSetEffect, INT channelCount, const SCHAR *gainSetIndex, const UCHAR *duck, INT useDuck, UCHAR *nDrcChannelGroups, SCHAR *uniqueIndex, SCHAR *groupForChannel, UCHAR *duckGroup) {
  DUCKING_MODIFICATION d[8], dg[8];
  FDKmemcpy(d, duck, sizeof(d));
  FDKmemcpy(dg, duckGroup, sizeof(dg));
  INT r = deriveDrcChannelGroups(drcSetEffect, channelCount, gainSetIndex, useDuck ? d : NULL, nDrcChannelGroups, uniqueIndex, groupForChannel, useDuck ? dg : NULL);
  FDKmemcpy(duckGroup, dg, sizeof(dg));
  return r;
}
}
