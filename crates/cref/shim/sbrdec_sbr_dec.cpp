// Shims for the differential tests of fdk_aac_rs_sbrdec::sbr_dec (tests/sbrdec/sbr_dec.rs).
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrdec_sbr_dec (see build.rs).
//
// The file-static copyHarmonicSpectrum() is reached through a copy of sbr_dec.cpp in a
// namespace. sbr_dec(), createSbrDec(), deleteSbrDec() and resetSbrDec() are tested through
// the SBR decoder API (tests/sbrdec/e2e.rs and api_fuzz.rs).

#include "cref_common.h"
#include "libSBRdec/src/sbr_dec.h"
#include "libSBRdec/src/sbr_ram.h"
#include "libSBRdec/src/env_extr.h"
#include "libSBRdec/src/env_calc.h"
#include "libSBRdec/src/hbe.h"
#include "libSBRdec/src/sbrdec_drc.h"
#include "scale.h"
#include "FDK_matrixCalloc.h"
#include "genericStds.h"

// (the public functions of the copy take arguments of global types: argument dependent lookup
// would also find the global declarations)
#define sbr_dec sd_sbr_dec
#define createSbrDec sd_createSbrDec
#define deleteSbrDec sd_deleteSbrDec
#define resetSbrDec sd_resetSbrDec
namespace sd {
#include "libSBRdec/src/sbr_dec.cpp"
}
#undef sbr_dec
#undef createSbrDec
#undef deleteSbrDec
#undef resetSbrDec

extern "C" {

/* re/im: nRows slots of 64 values; xOverQmf: MAX_NUM_PATCHES values */
void cref_sbrdec_sbr_dec_copy_harmonic(INT *xOverQmf, FIXP_DBL *re, FIXP_DBL *im, INT nRows, INT noCols, INT overlap, INT mode) {
  FIXP_DBL *r[160], *i[160];
  for (int k = 0; k < nRows && k < 160; k++) {
    r[k] = re + 64 * k;
    i[k] = im + 64 * k;
  }
  sd::copyHarmonicSpectrum(xOverQmf, r, i, noCols, overlap, (KEEP_STATES_SYNCED_MODE)mode);
}

}  // extern "C"
