// Shims for the differential tests of fdk_aac_rs_sacdec::sac_dec_errorcodes.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacdec_sac_dec_errorcodes (see build.rs).

#include "cref_common.h"
#include "sac_dec_errorcodes.h"

extern "C" {
/* The values of SACDEC_ERROR in declaration order. */
INT cref_sacdec_error_codes(INT *out) {
  INT n = 0;
  out[n++] = __mps_error_start;
  out[n++] = MPS_OK;
  out[n++] = MPS_NOTOK;
  out[n++] = MPS_OUTOFMEMORY;
  out[n++] = MPS_INVALID_HANDLE;
  out[n++] = MPS_INVALID_PARAMETER;
  out[n++] = MPS_UNSUPPORTED_HRTFMODEL;
  out[n++] = MPS_UNSUPPORTED_HRTFFREQ;
  out[n++] = MPS_UNSUPPORTED_UPMIX_TYPE;
  out[n++] = MPS_UNSUPPORTED_FORMAT;
  out[n++] = MPS_OUTPUT_BUFFER_TOO_SMALL;
  out[n++] = MPS_INVALID_PARAMETERBANDS;
  out[n++] = MPS_INVALID_TREECONFIG;
  out[n++] = MPS_INVALID_HRTFSET;
  out[n++] = MPS_INVALID_TTT;
  out[n++] = MPS_INVALID_BOXIDX;
  out[n++] = MPS_INVALID_SETIDX;
  out[n++] = MPS_INVALID_QUANTMODE;
  out[n++] = MPS_UNEQUAL_SSC;
  out[n++] = MPS_UNSUPPORTED_CONFIG;
  out[n++] = MPS_PARSE_ERROR;
  out[n++] = MPS_INVALID_TEMPSHAPE;
  out[n++] = MPS_WRONG_PARAMETERSETS;
  out[n++] = MPS_WRONG_PARAMETERBANDS;
  out[n++] = MPS_WRONG_TREECONFIG;
  out[n++] = MPS_WRONG_BLINDCONFIG;
  out[n++] = MPS_WRONG_OTT;
  out[n++] = MPS_WRONG_QUANTMODE;
  out[n++] = MPS_RESDEC_ERROR;
  out[n++] = MPS_APPLY_M2_ERROR;
  out[n++] = __mps_error_end;
  return n;
}
}
