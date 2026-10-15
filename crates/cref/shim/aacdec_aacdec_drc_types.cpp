// Shims for the differential tests of fdk_aac_rs_aacdec::aacdec_drc_types.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_aacdec_drc_types (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/aacdec_drc_types.h"

#define AL1(s, f) out[n++] = (INT)(sizeof(((s *)0)->f) / sizeof(((s *)0)->f[0]))

extern "C" {
/* The constants of aacdec_drc_types.h, in the order of the Rust test. */
INT cref_drct_constants(INT *out) {
  INT n = 0;
  out[n++] = MAX_DRC_THREADS;
  out[n++] = MAX_DRC_BANDS;
  out[n++] = UNKNOWN_PAYLOAD;
  out[n++] = MPEG_DRC_EXT_DATA;
  out[n++] = DVB_DRC_ANC_DATA;
  out[n++] = DISABLED_PARAMETER_HANDLING;
  out[n++] = ENABLED_PARAMETER_HANDLING;
  out[n++] = DRC_PRESENTATION_MODE_1;
  out[n++] = DRC_PRESENTATION_MODE_2;
  return n;
}

/* The number of elements of the array fields, in the order of the Rust test. */
INT cref_drct_array_lengths(INT *out) {
  INT n = 0;
  AL1(CDrcChannelData, bandTop);
  AL1(CDrcChannelData, drcValue);
  AL1(CDrcInfo, drcPayloadPosition);
  return n;
}
}
