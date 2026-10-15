// Test data for the differential tests of fdk_aac_rs_fdk::nlc_dec: the MPEG Surround encoder of
// the reference (libSACenc, sacenc_nlc_enc.cpp) writes valid CLD/ICC data, which the decoders
// read back. A separate shim because sacenc_nlc_enc.h defines its own DATA_TYPE.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_nlc_dec_enc (see build.rs).

#include "cref_common.h"
#include "libSACenc/src/sacenc_nlc_enc.h"

extern "C" {
/* Encodes data[0..2][MAXBANDS] (a pair, or data[0] alone) into buf (a power of two bytes) and
   returns the number of bits written. data_type: 0 CLD, 1 ICC. */
INT cref_nlc_dec_enc_ec_data(UCHAR *buf, UINT buf_size, SHORT *data, SHORT *history, INT data_type, INT start_band, INT data_bands, INT coarse_flag, INT independency_flag, INT pair) {
  FDK_BITSTREAM bs;
  SHORT in[2][MAXBANDS];
  SHORT hist[MAXBANDS];
  FDKmemcpy(in, data, sizeof(in));
  FDKmemcpy(hist, history, sizeof(hist));
  FDKinitBitStream(&bs, buf, buf_size, 0, BS_WRITER);
  if (pair) {
    fdk_sacenc_ecDataPairEnc(&bs, in, hist, (DATA_TYPE)data_type, 0, start_band, data_bands,
                             coarse_flag, independency_flag);
  } else {
    fdk_sacenc_ecDataSingleEnc(&bs, in, hist, (DATA_TYPE)data_type, 0, start_band, data_bands,
                               coarse_flag, independency_flag);
  }
  INT bits = (INT)FDKgetValidBits(&bs);
  FDKsyncCache(&bs);
  return bits;
}
INT cref_nlc_dec_enc_maxbands(void) { return MAXBANDS; }
}
