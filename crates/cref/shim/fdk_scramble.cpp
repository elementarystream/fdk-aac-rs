// Shims for the differential tests of fdk_aac_rs_fdk::scramble.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_scramble (see build.rs).

#include "cref_common.h"
#include "common_fix.h"
#include "scramble.h"

extern "C" {
/* scramble() is inline in scramble.h; this is the copy compiled into the shim. */
void cref_scramble_scramble(FIXP_DBL *x, INT length) { scramble(x, length); }
}
