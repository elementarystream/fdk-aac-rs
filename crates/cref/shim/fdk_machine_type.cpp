// Shims for the differential tests of fdk_aac_rs_fdk::machine_type.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_machine_type (see build.rs).

#include "cref_common.h"
#include "common_fix.h"

extern "C" {
/* Constants (index order of crates/cref/tests/fdk/machine_type.rs). */
INT64 cref_machine_type_const(INT i) {
  static const INT64 v[] = {SHORT_BITS, CHAR_BITS, (INT64)sizeof(INT_PCM), MAXVAL_PCM, MINVAL_PCM,
                            WAV_BITS, SAMPLE_BITS, SAMPLE_MAX, SAMPLE_MIN, ALIGNMENT_DEFAULT,
                            ALIGNMENT_EXTRES, (INT64)sizeof(INT), (INT64)sizeof(LONG),
                            (INT64)sizeof(SHORT), (INT64)sizeof(INT64)};
  return v[i];
}
UINT cref_ALIGN_SIZE(UINT a) { return (UINT)ALIGN_SIZE(a); }
UINT cref_ALGN_SIZE_EXTRES(UINT a) { return (UINT)ALGN_SIZE_EXTRES(a); }
}
