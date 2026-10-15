// Namespaced copy of libAACdec/src/usacdec_lpc.cpp for the SBR capture harness (see
// sbrdec_capdec_prelude.cpp and sbrdec_e2e.cpp).

#include "sbrdec_capdec_prelude.cpp"

namespace capdec {
#include "usacdec_lpc.cpp"
}  // namespace capdec
