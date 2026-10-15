// Shims for the differential tests of the file-static functions of
// fdk_aac_rs_sbrenc::ps_encode. Every `cref_*` function defined at the start of a line is bound
// in fdk_aac_rs_cref::sbrenc_ps_encode_static (see build.rs).
//
// ps_encode.cpp is compiled in this file inside a namespace. The copy of calculateICC()
// compiles fixMax(fixp_abs(), fixp_abs()) and CountLeadingBits() as the library object does
// (an unsigned maximum without the sign fold; checked in the disassembly of both builds).

#include "cref_common.h"
#include "libSBRenc/src/ps_main.h"
#include "libSBRenc/src/ps_encode.h"
#include "qmf.h"
#include "libSBRenc/src/sbr_misc.h"
#include "libSBRenc/src/sbrenc_ram.h"
#include "genericStds.h"

namespace ref_ps_encode {
#include "libSBRenc/src/ps_encode.cpp"
}

extern "C" {
/* calculateICC() on [PS_MAX_ENVELOPES][PS_MAX_BANDS] arrays. */
void cref_ps_encode_calculateICC(FIXP_DBL *pwrL, FIXP_DBL *pwrR, FIXP_DBL *pwrCr, FIXP_DBL *pwrCi, FIXP_DBL *icc, INT nEnvelopes, INT psBands) {
  typedef FIXP_DBL Rows[PS_MAX_BANDS];
  ref_ps_encode::calculateICC((Rows *)pwrL, (Rows *)pwrR, (Rows *)pwrCr, (Rows *)pwrCi, (Rows *)icc, nEnvelopes,
                              psBands);
}
}
