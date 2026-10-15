// Shims for the negative finalShift test of fdk_aac_rs_aacenc::psy_main.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_psy_main_amplified (see build.rs).
//
// FDKaacEnc_psyMain() shifts the spectrum left by `finalShift`, which is negative only for
// spectral values that the transform of 16-bit PCM does not produce. This file compiles a
// copy of psy_main.cpp (in the namespace ref_amplified, with the library's flags) whose two
// transform calls are redirected to wrappers: they call the library's transform and then
// multiply the spectrum by gain/256, saturating (cref_psy_main_amplified_set_gain). The port applies
// the same operation through psy_main_set_spectrum_hook(). Run the copy with
// cref_psy_main_frame_fn() of aacenc_psy_main.cpp and the function pointer from
// cref_psy_main_amplified_fn().
//
// The copy's `spectrum[line] <<= finalShift` loop must compile to the same code as the
// library's psy_main.o (checked against work/fdk-aac-asm, see the test).

#include "cref_common.h"
/* every header that psy_main.cpp includes, at global scope first */
#include "libAACenc/src/psy_const.h"
#include "libAACenc/src/block_switch.h"
#include "libAACenc/src/transform.h"
#include "libAACenc/src/spreading.h"
#include "libAACenc/src/pre_echo_control.h"
#include "libAACenc/src/band_nrg.h"
#include "libAACenc/src/psy_configuration.h"
#include "libAACenc/src/psy_data.h"
#include "libAACenc/src/ms_stereo.h"
#include "libAACenc/src/interface.h"
#include "libAACenc/src/psy_main.h"
#include "libAACenc/src/grp_data.h"
#include "libAACenc/src/tns_func.h"
#include "libAACenc/src/pns_func.h"
#include "libAACenc/src/tonality.h"
#include "libAACenc/src/aacEnc_ram.h"
#include "libAACenc/src/intensity.h"

static INT amplifyGain = 256;

/* x[i] * amplifyGain / 256 (rounded down), saturated to MINVAL_DBL..MAXVAL_DBL. */
static void amplifySpectrum(FIXP_DBL *x, INT n) {
  for (INT i = 0; i < n; i++) {
    INT64 v = ((INT64)x[i] * amplifyGain) >> 8;
    if (v > (INT64)MAXVAL_DBL) v = (INT64)MAXVAL_DBL;
    if (v < (INT64)MINVAL_DBL) v = (INT64)MINVAL_DBL;
    x[i] = (FIXP_DBL)v;
  }
}

static INT ampTransformReal(const INT_PCM *pTimeData, FIXP_DBL *mdctData, const INT blockType,
                            const INT windowShape, INT *prevWindowShape, H_MDCT mdctPers,
                            const INT frameLength, INT *pMdctData_e, INT filterType) {
  INT r = FDKaacEnc_Transform_Real(pTimeData, mdctData, blockType, windowShape, prevWindowShape,
                                   mdctPers, frameLength, pMdctData_e, filterType);
  if (r == 0) amplifySpectrum(mdctData, frameLength);
  return r;
}

static INT ampTransformRealEld(const INT_PCM *pTimeData, FIXP_DBL *mdctData,
                               const INT blockType, const INT windowShape,
                               INT *prevWindowShape, const INT frameLength, INT *mdctData_e,
                               INT filterType, FIXP_DBL *overlapAddBuffer) {
  INT r = FDKaacEnc_Transform_Real_Eld(pTimeData, mdctData, blockType, windowShape,
                                       prevWindowShape, frameLength, mdctData_e, filterType,
                                       overlapAddBuffer);
  if (r == 0) amplifySpectrum(mdctData, frameLength);
  return r;
}

#define FDKaacEnc_Transform_Real ampTransformReal
#define FDKaacEnc_Transform_Real_Eld ampTransformRealEld
namespace ref_amplified {
#include "libAACenc/src/psy_main.cpp"
}
#undef FDKaacEnc_Transform_Real
#undef FDKaacEnc_Transform_Real_Eld

extern "C" {
/* The gain (in 1/256, up to 2^22) of the transform wrappers of the copy. */
void cref_psy_main_amplified_set_gain(INT gain) { amplifyGain = gain; }
/* The copy's FDKaacEnc_psyMain, for cref_psy_main_frame_fn(). */
void *cref_psy_main_amplified_fn(void) { return (void *)&ref_amplified::FDKaacEnc_psyMain; }
}
