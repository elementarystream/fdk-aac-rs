// Shims for the differential tests of fdk_aac_rs_aacdec::usacdec_fac.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_usacdec_fac (see build.rs).

#include "cref_common.h"

/* A copy of usacdec_fac.cpp, compiled together with its headers (they are not included at global
   scope before), for its file-static table. */
namespace ref_fac {
#include "libAACdec/src/usacdec_fac.cpp"
}

extern "C" {
/* The file-static table `name` of the copy (ref_fac) as memory: gainFac.
   Returns NULL for another name. */
const void *cref_fac_table(const char *name, INT *bytes) {
  if (strcmp(name, "gainFac") == 0) {
    *bytes = (INT)sizeof(ref_fac::gainFac);
    return ref_fac::gainFac;
  }
  *bytes = 0;
  return NULL;
}
}
