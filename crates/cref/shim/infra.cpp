// Smoke tests of the cref infrastructure itself: linking, the API prefix, the variant.
#include "cref_common.h"
#include "aacdecoder_lib.h"
#include "common_fix.h"

extern "C" {
INT cref_infra_open_close(INT transport) {
  HANDLE_AACDECODER h = aacDecoder_Open((TRANSPORT_TYPE)transport, 1);
  if (h == NULL) return -1;
  aacDecoder_Close(h);
  return 0;
}
FIXP_DBL cref_infra_fmult(FIXP_DBL a, FIXP_DBL b) { return fMult(a, b); }
}
