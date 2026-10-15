// Shims for the differential tests of fdk_aac_rs_aacenc::chaosmeasure.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_chaosmeasure (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/chaosmeasure.h"

/* The static function of chaosmeasure.cpp (namespace-include trick). */
namespace ref_chaosmeasure {
#include "libAACenc/src/chaosmeasure.cpp"
}

extern "C" {
void cref_chaosmeasure_CalculateChaosMeasure(FIXP_DBL *paMDCTDataNM0, INT numberOfLines, FIXP_DBL *chaosMeasure) {
  FDKaacEnc_CalculateChaosMeasure(paMDCTDataNM0, numberOfLines, chaosMeasure);
}
void cref_chaosmeasure_CalculateChaosMeasurePeakFast(FIXP_DBL *paMDCTDataNM0, INT numberOfLines, FIXP_DBL *chaosMeasure) {
  ref_chaosmeasure::FDKaacEnc_FDKaacEnc_CalculateChaosMeasurePeakFast(paMDCTDataNM0,
                                                                      numberOfLines, chaosMeasure);
}
}
