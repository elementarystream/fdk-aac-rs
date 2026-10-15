// Shims for the differential tests of the file-static functions of fdk_aac_rs_sbrdec::psbitdec
// (tests/sbrdec/psdec.rs), reached through a copy of psbitdec.cpp in a namespace. ReadPsData
// and DecodePs are tested through shim/sbrdec_psdec.cpp.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrdec_psbitdec (see build.rs).

#include "cref_common.h"
#include "libSBRdec/src/psbitdec.h"
#include "libSBRdec/src/psdec.h"
#include "libSBRdec/src/sbr_rom.h"
#include "libSBRdec/src/huff_dec.h"

// (the public functions of the copy take arguments of global types: argument dependent lookup
// would also find the global declarations)
#define ResetPsDec psb_ResetPsDec
#define DecodePs psb_DecodePs
#define ReadPsData psb_ReadPsData
namespace psb {
#include "libSBRdec/src/psbitdec.cpp"
}
#undef ResetPsDec
#undef DecodePs
#undef ReadPsData

static Huffman book(INT k) {
  switch (k) {
    case 0: return aBookPsIidTimeDecode;
    case 1: return aBookPsIidFreqDecode;
    case 2: return aBookPsIccTimeDecode;
    case 3: return aBookPsIccFreqDecode;
    case 4: return aBookPsIidFineTimeDecode;
    default: return aBookPsIidFineFreqDecode;
  }
}

extern "C" {

/* Decodes n code words of book k; out[2i]: the value, out[2i+1]: the valid bits left after it */
void cref_sbrdec_psbitdec_huff(INT k, UCHAR *buf, INT bufSize, INT validBits, INT n, INT *out) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, validBits, BS_READER);
  for (INT i = 0; i < n; i++) {
    out[2 * i] = psb::decode_huff_cw(book(k), &bs, NULL);
    out[2 * i + 1] = (INT)FDKgetValidBits(&bs);
  }
}

SCHAR cref_sbrdec_psbitdec_limit(SCHAR i, SCHAR min, SCHAR max) { return psb::limitMinMax(i, min, max); }

void cref_sbrdec_psbitdec_delta_decode(SCHAR enable, SCHAR *aIndex, SCHAR *aPrev, SCHAR dtDf, UCHAR nrElements, UCHAR stride, SCHAR minIdx, SCHAR maxIdx) { psb::deltaDecodeArray(enable, aIndex, aPrev, dtDf, nrElements, stride, minIdx, maxIdx); }

void cref_sbrdec_psbitdec_map34(SCHAR *aIndex, UCHAR noBins) { psb::map34IndexTo20(aIndex, noBins); }

}  // extern "C"
