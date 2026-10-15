// Namespaced copy of libAACdec/src/usacdec_acelp.cpp for the SBR capture harness (see
// sbrdec_capdec_prelude.cpp and sbrdec_e2e.cpp).

#include "sbrdec_capdec_prelude.cpp"

namespace capdec {
#include "usacdec_acelp.cpp"
}  // namespace capdec
