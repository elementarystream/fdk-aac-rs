// Shims for the differential tests of fdk_aac_rs_sbrenc::tran_det.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_tran_det (see build.rs).
//
// The transient detectors and the frame splitter of the library on heap allocated detector
// structures, dumped with sbrdump_tran_det() and sbrdump_fast_tran_det()
// (sbrenc_sbr_encoder_dump.cpp). The energies are passed as 32 rows of 64 values.

#include "cref_common.h"
#include <stdlib.h>
#include "libSBRenc/src/tran_det.h"

struct SbrDump {
  INT *buf;
  INT cap;
  INT n;
};
void sbrdump_tran_det(SbrDump *d, const SBR_TRANSIENT_DETECTOR *h);
void sbrdump_fast_tran_det(SbrDump *d, const FAST_TRAN_DETECTOR *h);

static void rows(FIXP_DBL *nrg, FIXP_DBL **r) {
  for (INT i = 0; i < 32; i++) r[i] = nrg + 64 * i;
}

extern "C" {
void *cref_tran_det_new(void) { return calloc(1, sizeof(SBR_TRANSIENT_DETECTOR)); }
void *cref_tran_det_fast_new(void) { return calloc(1, sizeof(FAST_TRAN_DETECTOR)); }
void cref_tran_det_free(void *h) { free(h); }
/* params: tran_thr, standardBitrate, nChannels, bitRate, tran_det_mode. */
INT cref_tran_det_init(void *h, UINT flags, INT frameSize, INT sampleFreq, INT *params, INT tran_fc, INT no_cols, INT no_rows, INT wo, INT szShift, INT frameShift, INT tran_off) {
  sbrConfiguration cfg;
  FDKmemclear(&cfg, sizeof(cfg));
  cfg.tran_thr = params[0];
  cfg.codecSettings.standardBitrate = params[1];
  cfg.codecSettings.nChannels = params[2];
  cfg.codecSettings.bitRate = params[3];
  cfg.tran_det_mode = params[4];
  return FDKsbrEnc_InitSbrTransientDetector((SBR_TRANSIENT_DETECTOR *)h, flags, frameSize, sampleFreq, &cfg, tran_fc, no_cols, no_rows, wo, szShift, frameShift, tran_off);
}
void cref_tran_det_detect(void *h, FIXP_DBL *nrg, INT *scale, UCHAR *info, INT wo, INT szShift, INT timeStep, INT frameMiddleBorder) {
  FIXP_DBL *r[32];
  rows(nrg, r);
  FDKsbrEnc_transientDetect((SBR_TRANSIENT_DETECTOR *)h, r, scale, info, wo, szShift, timeStep, frameMiddleBorder);
}
void cref_tran_det_split(void *h, FIXP_DBL *nrg, INT *scale, UCHAR *freqBandTable, UCHAR *info, INT wo, INT szShift, INT nSfb, INT timeStep, INT no_cols, FIXP_DBL *tonality) {
  FIXP_DBL *r[32];
  rows(nrg, r);
  FDKsbrEnc_frameSplitter(r, scale, (SBR_TRANSIENT_DETECTOR *)h, freqBandTable, info, wo, szShift, nSfb, timeStep, no_cols, tonality);
}
INT cref_tran_det_fast_init(void *h, INT slots, INT bw, INT noQmf, INT firstBand) {
  return FDKsbrEnc_InitSbrFastTransientDetector((FAST_TRAN_DETECTOR *)h, slots, bw, noQmf, firstBand);
}
void cref_tran_det_fast_detect(void *h, FIXP_DBL *nrg, INT *scale, INT wo, UCHAR *info) {
  FIXP_DBL *r[32];
  rows(nrg, r);
  FDKsbrEnc_fastTransientDetect((FAST_TRAN_DETECTOR *)h, r, scale, wo, info);
}
INT cref_tran_det_dump(void *h, INT *buf, INT cap) {
  SbrDump d = {buf, cap, 0};
  sbrdump_tran_det(&d, (const SBR_TRANSIENT_DETECTOR *)h);
  return d.n;
}
INT cref_tran_det_fast_dump(void *h, INT *buf, INT cap) {
  SbrDump d = {buf, cap, 0};
  sbrdump_fast_tran_det(&d, (const FAST_TRAN_DETECTOR *)h);
  return d.n;
}
}
