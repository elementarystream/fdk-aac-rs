// Shims for the table tests of fdk_aac_rs_aacdec::conceal.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_conceal_tables (see build.rs).
//
// conceal.cpp is compiled here in a namespace together with its headers (they are not included
// at global scope before), for its file-static table.

#include "cref_common.h"

namespace ref_conceal {
#include "libAACdec/src/conceal.cpp"
}

extern "C" {
/* The file-static table `name` of the copy (ref_conceal) as memory: facMod4Table.
   Returns NULL for another name. */
const void *cref_conceal_table(const char *name, INT *bytes) {
  if (strcmp(name, "facMod4Table") == 0) {
    *bytes = (INT)sizeof(ref_conceal::facMod4Table);
    return ref_conceal::facMod4Table;
  }
  *bytes = 0;
  return NULL;
}
}
