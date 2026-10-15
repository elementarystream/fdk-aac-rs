// Shims for the differential tests of fdk_aac_rs_aacenc::tonality.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_tonality (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/tonality.h"

/* The static function and constant of tonality.cpp (namespace-include trick). */
namespace ref_tonality {
#include "libAACenc/src/tonality.cpp"
}

extern "C" {
void cref_tonality_CalculateFullTonality(FIXP_DBL *spectrum, INT *sfbMaxScaleSpec, FIXP_DBL *sfbEnergyLD64, SHORT *sfbTonality, INT sfbCnt, const INT *sfbOffset, INT usePns) {
  FDKaacEnc_CalculateFullTonality(spectrum, sfbMaxScaleSpec, sfbEnergyLD64,
                                  (FIXP_SGL *)sfbTonality, sfbCnt, sfbOffset, usePns);
}
void cref_tonality_CalcSfbTonality(FIXP_DBL *spectrum, INT *sfbMaxScaleSpec, FIXP_DBL *chaosMeasure, SHORT *sfbTonality, INT sfbCnt, const INT *sfbOffset, FIXP_DBL *sfbEnergyLD64) {
  ref_tonality::FDKaacEnc_CalcSfbTonality(spectrum, sfbMaxScaleSpec, chaosMeasure,
                                          (FIXP_SGL *)sfbTonality, sfbCnt, sfbOffset,
                                          sfbEnergyLD64);
}
FIXP_DBL cref_tonality_normlog() { return ref_tonality::normlog; }
}
