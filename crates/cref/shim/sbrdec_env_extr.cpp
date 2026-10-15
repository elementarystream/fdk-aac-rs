// Shims for the differential tests of fdk_aac_rs_sbrdec::env_extr, env_dec and sbrdec_freq_sca
// (tests/sbrdec/env_extr.rs). Structures are passed as memory (the port's plain data structures
// have the layout of the reference); the frequency band table pointers of a header are set to
// the header's own tables before a call and cleared after it. Bitstreams are given as a buffer
// (a power of two bytes) and the number of valid bits; the bit position after the call is
// returned.

#include "cref_common.h"
#include "libSBRdec/src/env_extr.h"
#include "libSBRdec/src/env_dec.h"
#include "libSBRdec/src/sbrdec_freq_sca.h"

static void fix_header(SBR_HEADER_DATA *h, int set) {
  h->freqBandData.freqBandTable[0] = set ? h->freqBandData.freqBandTableLo : NULL;
  h->freqBandData.freqBandTable[1] = set ? h->freqBandData.freqBandTableHi : NULL;
}

extern "C" {

INT cref_sbrdec_env_extr_init_header(void *h, INT srIn, INT srOut, INT downscale, INT spf, UINT flags, INT setDefault) {
  SBR_HEADER_DATA *hd = (SBR_HEADER_DATA *)h;
  fix_header(hd, 1);
  INT r = initHeaderData(hd, srIn, srOut, downscale, spf, flags, setDefault);
  fix_header(hd, 0);
  return r;
}

void cref_sbrdec_env_extr_init_prev(void *prev, INT timeSlots) { initSbrPrevFrameData((HANDLE_SBR_PREV_FRAME_DATA)prev, timeSlots); }

UINT cref_sbrdec_env_extr_map_rate(UINT fs, UINT isUsac) { return sbrdec_mapToStdSampleRate(fs, isUsac); }

/* returns the header status; *bitsLeft: valid bits after the call */
INT cref_sbrdec_env_extr_get_header(void *h, UCHAR *buf, INT bufSize, INT validBits, UINT flags, INT fIsSbrData, INT configMode, INT *bitsLeft) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, validBits, BS_READER);
  SBR_HEADER_DATA *hd = (SBR_HEADER_DATA *)h;
  if (hd) fix_header(hd, 1);
  INT r = sbrGetHeaderData(hd, &bs, flags, fIsSbrData, (UCHAR)configMode);
  if (hd) fix_header(hd, 0);
  *bitsLeft = (INT)FDKgetValidBits(&bs);
  return r;
}

/* right: NULL for a single channel element; no PS decoder */
INT cref_sbrdec_env_extr_get_channel_element(void *h, void *left, void *right, void *prevLeft, INT pvcModeLast, UCHAR *buf, INT bufSize, INT validBits, UINT flags, INT overlap, INT *bitsLeft) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, validBits, BS_READER);
  SBR_HEADER_DATA *hd = (SBR_HEADER_DATA *)h;
  fix_header(hd, 1);
  INT r = sbrGetChannelElement(hd, (HANDLE_SBR_FRAME_DATA)left, (HANDLE_SBR_FRAME_DATA)right, (HANDLE_SBR_PREV_FRAME_DATA)prevLeft, (UCHAR)pvcModeLast, &bs, NULL, flags, overlap);
  fix_header(hd, 0);
  *bitsLeft = (INT)FDKgetValidBits(&bs);
  return r;
}

void cref_sbrdec_env_extr_decode(void *h, void *left, void *prevLeft, void *right, void *prevRight) {
  SBR_HEADER_DATA *hd = (SBR_HEADER_DATA *)h;
  fix_header(hd, 1);
  decodeSbrData(hd, (HANDLE_SBR_FRAME_DATA)left, (HANDLE_SBR_PREV_FRAME_DATA)prevLeft, (HANDLE_SBR_FRAME_DATA)right, (HANDLE_SBR_PREV_FRAME_DATA)prevRight);
  fix_header(hd, 0);
}

INT cref_sbrdec_env_extr_reset_tables(void *h, UINT flags) {
  SBR_HEADER_DATA *hd = (SBR_HEADER_DATA *)h;
  fix_header(hd, 1);
  INT r = resetFreqBandTables(hd, flags);
  fix_header(hd, 0);
  return r;
}

INT cref_sbrdec_env_extr_update_freq_scale(UCHAR *vkMaster, UCHAR *numMaster, UINT fs, void *h, UINT flags) {
  SBR_HEADER_DATA *hd = (SBR_HEADER_DATA *)h;
  fix_header(hd, 1);
  INT r = sbrdecUpdateFreqScale(vkMaster, numMaster, fs, hd, flags);
  fix_header(hd, 0);
  return r;
}

void cref_sbrdec_env_extr_down_sample_lo_res(UCHAR *vResult, INT numResult, UCHAR *ref, INT numRef) {
  sbrdecDownSampleLoRes(vResult, (UCHAR)numResult, ref, (UCHAR)numRef);
}

void cref_sbrdec_env_extr_shellsort(UCHAR *in, INT n) { shellsort(in, (UCHAR)n); }

}  // extern "C"
