// Shims for the table tests of fdk_aac_rs_fdk::hybrid.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_hybrid_tables (see build.rs).
//
// FDK_hybrid.cpp is compiled here in a namespace together with its headers (they are not
// included at global scope before), for its file-static tables.

#include "cref_common.h"

namespace ref_hybrid {
#include "libFDK/src/FDK_hybrid.cpp"
}

extern "C" {
/* The file-static table `name` of FDK_hybrid.cpp (the copy) as memory: ringbuffIdxTab,
   HybFilterCoef8, HybFilterCoef2, HybFilterCoef4. Returns NULL for another name. */
const void *cref_hybrid_table(const char *name, INT *bytes) {
#define T(x)                              \
  if (strcmp(name, #x) == 0) {            \
    *bytes = (INT)sizeof(ref_hybrid::x); \
    return ref_hybrid::x;                 \
  }
  T(ringbuffIdxTab)
  T(HybFilterCoef8)
  T(HybFilterCoef2)
  T(HybFilterCoef4)
#undef T
  *bytes = 0;
  return NULL;
}
}
