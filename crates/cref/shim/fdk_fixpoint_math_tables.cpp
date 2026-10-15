// Shims for the table tests of fdk_aac_rs_fdk::fixpoint_math.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_fixpoint_math_tables (see build.rs).
//
// fixpoint_math.cpp is compiled here in a namespace together with its headers (they are not
// included at global scope before), so that the overloads of the copy do not hide the global
// ones.

#include "cref_common.h"

/* A copy of fixpoint_math.cpp, for its file-static tables. */
namespace ref_fixpoint_math {
#include "libFDK/src/fixpoint_math.cpp"
}

extern "C" {
/* The file-static table `name` of fixpoint_math.cpp (the copy) as memory: pow2Coeff,
   ldIntCoeff. Returns NULL for another name. */
const void *cref_fixpoint_math_table(const char *name, INT *bytes) {
#define T(x)                                     \
  if (strcmp(name, #x) == 0) {                   \
    *bytes = (INT)sizeof(ref_fixpoint_math::x); \
    return ref_fixpoint_math::x;                 \
  }
  T(pow2Coeff)
  T(ldIntCoeff)
#undef T
  *bytes = 0;
  return NULL;
}
}
