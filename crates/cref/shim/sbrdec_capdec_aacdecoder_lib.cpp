// Namespaced copy of libAACdec/src/aacdecoder_lib.cpp for the SBR capture harness (see
// sbrdec_capdec_prelude.cpp and sbrdec_e2e.cpp).

#include "sbrdec_capdec_prelude.cpp"

namespace capdec {
#include "aacdecoder_lib.cpp"
}  // namespace capdec
