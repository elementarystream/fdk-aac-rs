// Shims for the differential tests of fdk_aac_rs_fdk::fft.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_fft (see build.rs).
//
// The kernels of fft.cpp are file-local or inline; they are tested through the library's fft(),
// which dispatches to every kernel, so that the code compiled into the library is measured.
// fft_4 and fft_8 are static inline functions of fft.h (fft_8 is also used by FDK_hybrid.cpp);
// the shim compiles its own copy.

#include "cref_common.h"
#include "common_fix.h"
#include "fft.h"

extern "C" {
void cref_fft_fft(INT length, FIXP_DBL *x, INT *scalefactor) { fft(length, x, scalefactor); }
void cref_fft_fft_4(FIXP_DBL *x) { fft_4(x); }
void cref_fft_fft_8(FIXP_DBL *x) { fft_8(x); }
}

/* A copy of fft.cpp, for its file-static tables. */
namespace ref_fft {
#include "libFDK/src/fft.cpp"
}

extern "C" {
/* The file-static table `name` of fft.cpp (the copy) as memory: fft16_w16, fft32_w32. Returns
   NULL for another name. */
const void *cref_fft_table(const char *name, INT *bytes) {
  if (strcmp(name, "fft16_w16") == 0) {
    *bytes = (INT)sizeof(ref_fft::fft16_w16);
    return ref_fft::fft16_w16;
  }
  if (strcmp(name, "fft32_w32") == 0) {
    *bytes = (INT)sizeof(ref_fft::fft32_w32);
    return ref_fft::fft32_w32;
  }
  *bytes = 0;
  return NULL;
}
}
