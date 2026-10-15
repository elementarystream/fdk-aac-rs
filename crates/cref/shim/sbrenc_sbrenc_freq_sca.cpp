// Shims for the differential tests of fdk_aac_rs_sbrenc::sbrenc_freq_sca.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_sbrenc_freq_sca (see build.rs).
//
// The public functions are called in the library; the local tables of getStartFreq() and
// getStopFreq() are reached through a copy of sbrenc_freq_sca.cpp in a namespace.

#include "cref_common.h"
#include "libSBRenc/src/sbrenc_freq_sca.h"
#include "libSBRenc/src/sbr_misc.h"
#include "genericStds.h"

namespace ref_freq_sca {
#include "libSBRenc/src/sbrenc_freq_sca.cpp"
}

extern "C" {
INT cref_freq_sca_start_freq_raw(INT startFreq, INT fsCore) {
  return FDKsbrEnc_getSbrStartFreqRAW(startFreq, fsCore);
}
INT cref_freq_sca_stop_freq_raw(INT stopFreq, INT fsCore) {
  return FDKsbrEnc_getSbrStopFreqRAW(stopFreq, fsCore);
}
INT cref_freq_sca_get_start_freq(INT fsCore, INT startFreq) {
  return ref_freq_sca::getStartFreq(fsCore, startFreq);
}
INT cref_freq_sca_get_stop_freq(INT fsCore, INT stopFreq) {
  return ref_freq_sca::getStopFreq(fsCore, stopFreq);
}
INT cref_freq_sca_find_start_and_stop_band(INT srSbr, INT srCore, INT noChannels, INT startFreq, INT stopFreq, INT *k0, INT *k2) {
  return FDKsbrEnc_FindStartAndStopBand(srSbr, srCore, noChannels, startFreq, stopFreq, k0, k2);
}
INT cref_freq_sca_update_freq_scale(UCHAR *v_k_master, INT *numBands, INT k0, INT k2, INT freqScale, INT alterScale) {
  return FDKsbrEnc_UpdateFreqScale(v_k_master, numBands, k0, k2, freqScale, alterScale);
}
INT cref_freq_sca_update_hi_res(UCHAR *hires, INT *numHires, UCHAR *v_k_master, INT numMaster, INT *xoverBand) {
  return FDKsbrEnc_UpdateHiRes(hires, numHires, v_k_master, numMaster, xoverBand);
}
void cref_freq_sca_update_lo_res(UCHAR *lores, INT *numLores, UCHAR *hires, INT numHires) {
  FDKsbrEnc_UpdateLoRes(lores, numLores, hires, numHires);
}
}
