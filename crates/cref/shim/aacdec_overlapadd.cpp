// Shims for the differential tests of fdk_aac_rs_aacdec::overlapadd.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_overlapadd (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/overlapadd.h"

static SPECTRUM spectrum;

extern "C" {
INT cref_ola_overlap_buffer_size(void) { return OverlapBufferSize; }
INT cref_ola_spectrum_len(void) { return (INT)(sizeof(SPECTRUM) / sizeof(FIXP_DBL)); }
/* The offsets (in FIXP_DBL) that SPEC_LONG, SPEC and SPEC_TCX add to a pointer. */
INT cref_ola_spec_long(void) {
  SPECTRAL_PTR p = spectrum;
  return (INT)(SPEC_LONG(p) - p);
}
INT cref_ola_spec(INT w, INT gl) {
  SPECTRAL_PTR p = spectrum;
  return (INT)(SPEC(p, w, gl) - p);
}
INT cref_ola_spec_tcx(INT f, INT gl, INT fb) {
  SPECTRAL_PTR p = spectrum;
  return (INT)(SPEC_TCX(p, f, gl, fb) - p);
}
}
