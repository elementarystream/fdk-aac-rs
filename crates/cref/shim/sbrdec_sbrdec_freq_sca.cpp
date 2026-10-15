// Shims for the differential tests of fdk_aac_rs_sbrdec::sbrdec_freq_sca.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrdec_sbrdec_freq_sca (see build.rs).
//
// The file-static functions are reached through a copy of sbrdec_freq_sca.cpp in a namespace;
// the public ones are the reference's own (see also shim/sbrdec_env_extr.cpp).

#include "cref_common.h"
#include "libSBRdec/src/sbrdec_freq_sca.h"
#include "libSBRdec/src/sbr_rom.h"
#include "libSBRdec/src/env_extr.h"
#include "libSBRdec/src/transcendent.h"
#include "genericStds.h"

// (called with arguments of global types: argument dependent lookup would also find the
// global declaration)
#define sbrdecUpdateFreqScale fsq_sbrdecUpdateFreqScale
namespace fsq {
#include "libSBRdec/src/sbrdec_freq_sca.cpp"
}
#undef sbrdecUpdateFreqScale

extern "C" {

UCHAR cref_sbrdec_freq_sca_start_band(UINT fs, UCHAR startFreq, UINT flags) { return fsq::getStartBand(fs, startFreq, flags); }

UCHAR cref_sbrdec_freq_sca_stop_band(UINT fs, UCHAR stopFreq, UINT flags, UCHAR k0) { return fsq::getStopBand(fs, stopFreq, flags, k0); }

SHORT cref_sbrdec_freq_sca_factor_per_band(INT kStart, INT kStop, INT numBands) { return fsq::calcFactorPerBand(kStart, kStop, numBands); }

INT cref_sbrdec_freq_sca_number_of_bands(SHORT bpo, INT start, INT stop, INT warp) { return fsq::numberOfBands((FIXP_SGL)bpo, start, stop, warp); }

void cref_sbrdec_freq_sca_calc_bands(UCHAR *diff, UCHAR start, UCHAR stop, UCHAR numBands) { fsq::CalcBands(diff, start, stop, numBands); }

void cref_sbrdec_freq_sca_cum_sum(UCHAR startValue, UCHAR *diff, UCHAR length, UCHAR *out) { fsq::cumSum(startValue, diff, length, out); }

INT cref_sbrdec_freq_sca_modify_bands(UCHAR maxBandPrevious, UCHAR *diff, UCHAR length) { return fsq::modifyBands(maxBandPrevious, diff, length); }

void cref_sbrdec_freq_sca_hi_res(UCHAR *hires, UCHAR *numHires, UCHAR *vkMaster, UCHAR numBands, UCHAR xover) { fsq::sbrdecUpdateHiRes(hires, numHires, vkMaster, numBands, xover); }

void cref_sbrdec_freq_sca_lo_res(UCHAR *lores, UCHAR *numLores, UCHAR *hires, UCHAR numHires) { fsq::sbrdecUpdateLoRes(lores, numLores, hires, numHires); }

void cref_sbrdec_freq_sca_down_sample_lo_res(UCHAR *vResult, UCHAR numResult, UCHAR *ref, UCHAR numRef) { sbrdecDownSampleLoRes(vResult, numResult, ref, numRef); }

void cref_sbrdec_freq_sca_shellsort(UCHAR *in, UCHAR n) { shellsort(in, n); }

}  // extern "C"
