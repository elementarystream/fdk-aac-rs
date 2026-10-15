// Shims for the differential tests of fdk_aac_rs_tpenc::tpenc_adif.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::tpenc_tpenc_adif (see build.rs).
//
// The functions run on a copy of tpenc_adif.cpp in a namespace (same source and flags as the
// library); ADIF_INFO is passed as its byte image (36 bytes).

#include "cref_common.h"
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "tp_data.h"
#include "tpenc_lib.h"
#include "libMpegTPEnc/src/tpenc_asc.h"

namespace ref_tpenc_adif {
#include "libMpegTPEnc/src/tpenc_adif.cpp"
}

typedef ref_tpenc_adif::ADIF_INFO ADIF;

extern "C" {
INT cref_tpenc_adif_size(void) { return (INT)sizeof(ADIF); }
INT cref_tpenc_adif_encode_header(UCHAR *img, void *bs, INT bf) {
  ADIF a;
  FDKmemcpy(&a, img, sizeof(a));
  INT r = ref_tpenc_adif::adifWrite_EncodeHeader(&a, (HANDLE_FDK_BITSTREAM)bs, bf);
  FDKmemcpy(img, &a, sizeof(a));
  return r;
}
INT cref_tpenc_adif_get_header_bits(UCHAR *img) {
  ADIF a;
  FDKmemcpy(&a, img, sizeof(a));
  INT r = ref_tpenc_adif::adifWrite_GetHeaderBits(&a);
  FDKmemcpy(img, &a, sizeof(a));
  return r;
}
}
