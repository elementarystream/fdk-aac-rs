// Shims for the differential tests of fdk_aac_rs_sacdec::sac_qmf.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacdec_sac_qmf (see build.rs).
//
// The filter banks are created and initialized with the shims of the qmf tests
// (fdk_aac_rs_cref::fdk_qmf: a QMF_FILTER_BANK whose FilterStates point to a buffer of the
// test).

#include "cref_common.h"
#include "sac_qmf.h"

extern "C" {
INT cref_sacqmf_analysis(void *fb, const INT *timeSig, FIXP_DBL *re, FIXP_DBL *im) {
  return CalculateSpaceAnalysisQmf((HANDLE_SPACE_ANALYSIS_QMF)fb, (const PCM_MPS *)timeSig, re,
                                   im);
}
/* The filter bank is placed in a FDK_QMF_DOMAIN_OUT with the synthesis states it points to. */
INT cref_sacqmf_synthesis(void *fb, const FIXP_DBL *re, const FIXP_DBL *im, INT stride, INT *timeSig) {
  FDK_QMF_DOMAIN_OUT o;
  FDKmemclear(&o, sizeof(o));
  o.fb = *(QMF_FILTER_BANK *)fb;
  o.pSynQmfStates = (FIXP_QSS *)o.fb.FilterStates;
  INT r = CalculateSpaceSynthesisQmf(&o, re, im, stride, (PCM_MPS *)timeSig);
  *(QMF_FILTER_BANK *)fb = o.fb;
  return r;
}
INT cref_sacqmf_null(void) {
  INT r = CalculateSpaceSynthesisQmf(NULL, NULL, NULL, 1, NULL);
  r = r * 10000 + CalculateSpaceAnalysisQmf(NULL, NULL, NULL, NULL);
  return r;
}
}
