// Namespaced copy of libAACdec/src/aac_rom.cpp for the SBR capture harness (see
// sbrdec_capdec_prelude.cpp and sbrdec_e2e.cpp).

#include "sbrdec_capdec_prelude.cpp"

namespace capdec {
#include "aac_rom.cpp"
}  // namespace capdec
