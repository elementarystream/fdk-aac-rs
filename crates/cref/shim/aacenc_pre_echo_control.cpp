// Shims for the differential tests of fdk_aac_rs_aacenc::pre_echo_control.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_pre_echo_control (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/pre_echo_control.h"

extern "C" {
void cref_pre_echo_control_InitPreEchoControl(FIXP_DBL *pbThresholdNm1, INT *calcPreEcho, INT numPb, FIXP_DBL *sfbPcmQuantThreshold, INT *mdctScalenm1) {
  FDKaacEnc_InitPreEchoControl(pbThresholdNm1, calcPreEcho, numPb, sfbPcmQuantThreshold,
                               mdctScalenm1);
}
void cref_pre_echo_control_PreEchoControl(FIXP_DBL *pbThresholdNm1, INT calcPreEcho, INT numPb, INT maxAllowedIncreaseFactor, SHORT minRemainingThresholdFactor, FIXP_DBL *pbThreshold, INT mdctScale, INT *mdctScalenm1) {
  FDKaacEnc_PreEchoControl(pbThresholdNm1, calcPreEcho, numPb, maxAllowedIncreaseFactor,
                           (FIXP_SGL)minRemainingThresholdFactor, pbThreshold, mdctScale,
                           mdctScalenm1);
}
}
