// Shims for the differential tests of fdk_aac_rs_aacdec::usacdec_const.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_usacdec_const (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/usacdec_const.h"

extern "C" {
/* The constants of usacdec_const.h, in the order of the Rust test. */
INT cref_uconst_constants(INT *out) {
  INT n = 0;
  out[n++] = SF_CODE;
  out[n++] = SF_GAIN_C;
  out[n++] = SF_EXC;
  out[n++] = SF_GAIN_P;
  out[n++] = SF_PFAC;
  out[n++] = SF_SYNTH;
  out[n++] = SF_A_COEFFS;
  out[n++] = SF_STAB;
  out[n++] = M_LP_FILTER_ORDER;
  out[n++] = LP_FILTER_SCALE;
  out[n++] = PIT_MIN_12k8;
  out[n++] = PIT_MAX_12k8;
  out[n++] = FSCALE_DENOM;
  out[n++] = FAC_FSCALE_MIN;
  out[n++] = LPD_MAX_CORE_SR;
  out[n++] = FAC_FSCALE_MAX;
  out[n++] = PIT_MAX_TMP;
  out[n++] = PIT_MAX_MAX;
  out[n++] = NB_DIV;
  out[n++] = L_SUBFR;
  out[n++] = BPF_SFD;
  out[n++] = BPF_DELAY;
  out[n++] = L_FILT;
  out[n++] = L_EXTRA;
  out[n++] = L_INTERPOL;
  out[n++] = L_FRAME_PLUS_1024;
  out[n++] = L_DIV_1024;
  out[n++] = NB_SUBFR_1024;
  out[n++] = NB_SUBFR_SUPERFR_1024;
  out[n++] = AAC_SFD_1024;
  out[n++] = AAC_DELAY_1024;
  out[n++] = SYN_SFD_1024;
  out[n++] = SYN_DELAY_1024;
  out[n++] = LFAC_1024;
  out[n++] = LFAC_SHORT_1024;
  out[n++] = FDNS_NPTS_1024;
  out[n++] = L_FRAME_PLUS_768;
  out[n++] = L_DIV_768;
  out[n++] = NB_SUBFR_768;
  out[n++] = NB_SUBFR_SUPERFR_768;
  out[n++] = AAC_SFD_768;
  out[n++] = AAC_DELAY_768;
  out[n++] = SYN_SFD_768;
  out[n++] = SYN_DELAY_768;
  out[n++] = LFAC_768;
  out[n++] = LFAC_SHORT_768;
  out[n++] = L_FRAME_PLUS;
  out[n++] = L_DIV;
  out[n++] = NB_SUBFR;
  out[n++] = NB_SUBFR_SUPERFR;
  out[n++] = AAC_SFD;
  out[n++] = AAC_DELAY;
  out[n++] = SYN_SFD;
  out[n++] = SYN_DELAY;
  out[n++] = LFAC;
  out[n++] = LFAC_SHORT;
  out[n++] = FDNS_NPTS;
  return n;
}
}
