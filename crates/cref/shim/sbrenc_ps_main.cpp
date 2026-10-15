// Shims for the differential tests of fdk_aac_rs_sbrenc::ps_main (and ps_encode, which it
// calls).
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_ps_main (see build.rs).
//
// The parametric stereo encoder of the library with what the SBR encoder gives it: two QMF
// analysis filter banks with their states, the PS QMF synthesis filter bank with its states and
// the dynamic RAM of the SBR encoder, set up as sbrEncoder_Open() and sbrEncoder_Init() do.
// The state dump is the PARAMETRIC_STEREO (sbrdump_parametric_stereo()), followed by the
// dynamic RAM and the QMF states.

#include "cref_common.h"
#include <stdlib.h>
#include "libSBRenc/src/ps_main.h"
#include "libSBRenc/src/sbrenc_ram.h"
#include "sbr_encoder.h"
#include "qmf.h"

struct SbrDump {
  INT *buf;
  INT cap;
  INT n;
};
void sbrdump_parametric_stereo(SbrDump *d, const PARAMETRIC_STEREO *h);

#define RAM_DBL (SBR_ENC_DYN_RAM_SIZE / sizeof(FIXP_DBL))

struct PsHarness {
  HANDLE_PARAMETRIC_STEREO ps;
  QMF_FILTER_BANK qmfAna[2];
  FIXP_QAS anaStates[2][640];
  QMF_FILTER_BANK qmfSyn;
  FIXP_DBL synStates[640 / 2];
  FIXP_DBL ram[RAM_DBL];
};

static void dput(SbrDump *d, INT v) {
  if (d->n < d->cap) d->buf[d->n] = v;
  d->n++;
}

extern "C" {
/* cfg: nStereoBands, maxEnvelopes, iidQuantErrorThreshold, noQmfSlots, noQmfBands; returns NULL
   if PSEnc_Init() fails (*err is its result). */
void *cref_ps_main_new(INT *cfg, INT *err) {
  PsHarness *h = (PsHarness *)calloc(1, sizeof(PsHarness));
  if (PSEnc_Create(&h->ps) != PSENC_OK) {
    free(h);
    return NULL;
  }
  INT noQmfSlots = cfg[3], noQmfBands = cfg[4];
  for (INT ch = 0; ch < 2; ch++) {
    h->qmfAna[ch].FilterStates = h->anaStates[ch];
    qmfInitAnalysisFilterBank(&h->qmfAna[ch], h->anaStates[ch], noQmfSlots, noQmfBands, noQmfBands, noQmfBands, 0);
  }
  h->qmfSyn.FilterStates = h->synStates;
  qmfInitSynthesisFilterBank(&h->qmfSyn, h->synStates, noQmfSlots, noQmfBands >> 1, noQmfBands >> 1, noQmfBands >> 1, 0);
  PSENC_CONFIG c;
  FDKmemclear(&c, sizeof(c));
  c.nStereoBands = (PSENC_STEREO_BANDS_CONFIG)cfg[0];
  c.maxEnvelopes = (PSENC_NENV_CONFIG)cfg[1];
  c.iidQuantErrorThreshold = (FIXP_DBL)cfg[2];
  *err = PSEnc_Init(h->ps, &c, noQmfSlots, noQmfBands, (UCHAR *)h->ram);
  cfg[1] = c.maxEnvelopes;
  return h;
}
void cref_ps_main_free(void *p) {
  PsHarness *h = (PsHarness *)p;
  PSEnc_Destroy(&h->ps);
  free(h);
}
/* pcm: two channels of bufSize samples; the downmix is written to the start of channel 0. */
INT cref_ps_main_process(void *p, INT_PCM *pcm, INT bufSize, INT sendHeader, INT *qmfScale) {
  PsHarness *h = (PsHarness *)p;
  INT_PCM *samples[2] = {pcm, pcm + bufSize};
  QMF_FILTER_BANK *ana[2] = {&h->qmfAna[0], &h->qmfAna[1]};
  FIXP_DBL *re[32], *im[32];
  FIXP_DBL *rBuffer = GetRam_Sbr_envRBuffer(0, (UCHAR *)h->ram);
  FIXP_DBL *iBuffer = GetRam_Sbr_envIBuffer(0, (UCHAR *)h->ram);
  for (INT i = 0; i < 32; i++) {
    re[i] = rBuffer + i * 64;
    im[i] = iBuffer + i * 64;
  }
  SCHAR s = 0;
  INT err = FDKsbrEnc_PSEnc_ParametricStereoProcessing(h->ps, samples, bufSize, ana, re, im, pcm, &h->qmfSyn, &s, sendHeader);
  *qmfScale = s;
  return err;
}
/* ps_data() into buf (1024 bytes), or only counted; returns the bit count. */
INT cref_ps_main_write(void *p, INT count, UCHAR *buf, INT *validBits) {
  PsHarness *h = (PsHarness *)p;
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, 1024, 0, BS_WRITER);
  INT n = FDKsbrEnc_PSEnc_WritePSData(h->ps, count ? NULL : &bs);
  FDKsyncCache(&bs);
  *validBits = (INT)FDKgetValidBits(&bs);
  return n;
}
INT cref_ps_main_dump(void *p, INT *buf, INT cap) {
  PsHarness *h = (PsHarness *)p;
  SbrDump d = {buf, cap, 0};
  sbrdump_parametric_stereo(&d, h->ps);
  for (UINT i = 0; i < RAM_DBL; i++) dput(&d, h->ram[i]);
  for (INT ch = 0; ch < 2; ch++)
    for (INT i = 0; i < 640; i++) dput(&d, h->anaStates[ch][i]);
  for (INT i = 0; i < 640 / 2; i++) dput(&d, h->synStates[i]);
  return d.n;
}
}

// The file-static functions of ps_main.cpp, compiled in this file (namespace include). The
// copies compile the fixMax() chains of psFindBestScaling() and DownmixPSQmfData() to the same
// unsigned maxima as the library object (checked in the disassembly of both builds).
#include "libSBRenc/src/ps_bitenc.h"
namespace ref_ps_main {
#include "libSBRenc/src/ps_main.cpp"
}

extern "C" {
/* pHybridData[row][ch][ri][k] = v (rows below HYBRID_READ_OFFSET are __staticHybridData, the
   others are in the dynamic RAM; the pointers are those of the last processed frame). */
void cref_ps_main_poke_hybrid(void *p, INT row, INT ch, INT ri, INT k, INT v) {
  PsHarness *h = (PsHarness *)p;
  h->ps->pHybridData[row][ch][ri][k] = v;
}
/* psFindBestScaling() on the current frame, as FDKsbrEnc_PSEnc_ParametricStereoProcessing()
   calls it. */
void cref_ps_main_find_best_scaling(void *p) {
  PsHarness *h = (PsHarness *)p;
  ref_ps_main::psFindBestScaling(h->ps, &h->ps->pHybridData[HYBRID_READ_OFFSET], h->ps->dynBandScale,
                                 h->ps->maxBandValue, &h->ps->dmxScale);
}
/* DownmixPSQmfData() on the current frame, with the downmix in the SBR QMF buffers of the
   dynamic RAM and the time signal at pcm (as cref_ps_main_process() sets them up). */
INT cref_ps_main_downmix(void *p, INT_PCM *pcm, INT bufSize, INT scale0, INT scale1, INT *qmfScale) {
  PsHarness *h = (PsHarness *)p;
  FIXP_DBL *re[32], *im[32];
  FIXP_DBL *rBuffer = GetRam_Sbr_envRBuffer(0, (UCHAR *)h->ram);
  FIXP_DBL *iBuffer = GetRam_Sbr_envIBuffer(0, (UCHAR *)h->ram);
  for (INT i = 0; i < 32; i++) {
    re[i] = rBuffer + i * 64;
    im[i] = iBuffer + i * 64;
  }
  INT psQmfScale[MAX_PS_CHANNELS] = {scale0, scale1};
  SCHAR s = 0;
  INT err = ref_ps_main::DownmixPSQmfData(h->ps, &h->qmfSyn, re, im, pcm, bufSize,
                                          &h->ps->pHybridData[HYBRID_READ_OFFSET], h->ps->noQmfSlots,
                                          psQmfScale, &s);
  *qmfScale = s;
  return err;
}
}
