// Shims for the table tests of fdk_aac_rs_aacenc::psy_main.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_psy_main_tables (see build.rs).
//
// psy_main.cpp is compiled here in a namespace together with its headers (they are not included
// at global scope before), for its file-static table.

#include "cref_common.h"

namespace ref_psy_main {
#include "libAACenc/src/psy_main.cpp"
}

extern "C" {
/* The file-static table `name` of the copy (ref_psy_main) as memory: fadeOutFactor. Returns NULL
   for another name. */
const void *cref_psy_main_table(const char *name, INT *bytes) {
  if (strcmp(name, "fadeOutFactor") == 0) {
    *bytes = (INT)sizeof(ref_psy_main::fadeOutFactor);
    return ref_psy_main::fadeOutFactor;
  }
  *bytes = 0;
  return NULL;
}
}
