// Namespaced copy of libAACdec/src/aacdec_tns.cpp for the SBR capture harness (see
// sbrdec_capdec_prelude.cpp and sbrdec_e2e.cpp).

#include "sbrdec_capdec_prelude.cpp"

namespace capdec {
#include "aacdec_tns.cpp"
}  // namespace capdec
