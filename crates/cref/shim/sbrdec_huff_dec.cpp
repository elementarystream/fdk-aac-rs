// Shims for the differential tests of fdk_aac_rs_sbrdec::huff_dec.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrdec_huff_dec (see build.rs).

#include "cref_common.h"
#include "libSBRdec/src/huff_dec.h"
#include "libSBRdec/src/sbr_rom.h"

static Huffman book(INT k) {
  switch (k) {
    case 0: return FDK_sbrDecoder_sbr_huffBook_EnvLevel10T;
    case 1: return FDK_sbrDecoder_sbr_huffBook_EnvLevel10F;
    case 2: return FDK_sbrDecoder_sbr_huffBook_EnvBalance10T;
    case 3: return FDK_sbrDecoder_sbr_huffBook_EnvBalance10F;
    case 4: return FDK_sbrDecoder_sbr_huffBook_EnvLevel11T;
    case 5: return FDK_sbrDecoder_sbr_huffBook_EnvLevel11F;
    case 6: return FDK_sbrDecoder_sbr_huffBook_EnvBalance11T;
    case 7: return FDK_sbrDecoder_sbr_huffBook_EnvBalance11F;
    case 8: return FDK_sbrDecoder_sbr_huffBook_NoiseLevel11T;
    case 9: return FDK_sbrDecoder_sbr_huffBook_NoiseBalance11T;
    case 10: return aBookPsIidTimeDecode;
    case 11: return aBookPsIidFreqDecode;
    case 12: return aBookPsIccTimeDecode;
    case 13: return aBookPsIccFreqDecode;
    case 14: return aBookPsIidFineTimeDecode;
    default: return aBookPsIidFineFreqDecode;
  }
}

extern "C" {

/* Decodes n code words of book k; out[2i]: the value, out[2i+1]: the valid bits left after it */
void cref_sbrdec_huff_dec_decode(INT k, UCHAR *buf, INT bufSize, INT validBits, INT n, INT *out) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, validBits, BS_READER);
  for (INT i = 0; i < n; i++) {
    out[2 * i] = DecodeHuffmanCW(book(k), &bs);
    out[2 * i + 1] = (INT)FDKgetValidBits(&bs);
  }
}

}  // extern "C"
