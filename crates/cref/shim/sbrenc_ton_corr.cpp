// Shims for the differential tests of fdk_aac_rs_sbrenc::ton_corr (and of invf_est, nf_est
// and mh_det, which it calls).
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_ton_corr (see build.rs).
//
// The tonality correction module of the library on a heap allocated SBR_TON_CORR_EST, dumped
// with sbrdump_ton_corr() (sbrenc_sbr_encoder_dump.cpp). The missing harmonics vector is the
// member addHarmonic of an SBR_ENV_DATA, as in the encoder, because the detector reads the
// members that follow it (reproduced from the C layout by the port, mh_det.rs).

#include "cref_common.h"
#include <stddef.h>
#include <stdlib.h>
#include "libSBRenc/src/ton_corr.h"
#include "libSBRenc/src/bit_sbr.h"

struct SbrDump {
  INT *buf;
  INT cap;
  INT n;
};
void sbrdump_ton_corr(SbrDump *d, const SBR_TON_CORR_EST *h);

static void rows(FIXP_DBL *v, FIXP_DBL **r) {
  for (INT i = 0; i < 32; i++) r[i] = v + 64 * i;
}

static void frame_info(SBR_FRAME_INFO *f, const INT *v) {
  INT k = 0;
  f->nEnvelopes = v[k++];
  for (INT i = 0; i < MAX_ENVELOPES + 1; i++) f->borders[i] = v[k++];
  for (INT i = 0; i < MAX_ENVELOPES; i++) f->freqRes[i] = (FREQ_RES)v[k++];
  f->shortEnv = v[k++];
  f->nNoiseEnvelopes = v[k++];
  for (INT i = 0; i < MAX_NOISE_ENVELOPES + 1; i++) f->bordersNoise[i] = v[k++];
}

/* cfg: sbrSyntaxFlags, noQmfSlots, sampleFreq, noQmfBands, num_Master, nSfb[LO], nSfb[HI]. */
static void config(SBR_CONFIG_DATA *c, const INT *cfg, UCHAR *vkm, UCHAR *lo, UCHAR *hi) {
  FDKmemclear(c, sizeof(*c));
  c->sbrSyntaxFlags = (UINT)cfg[0];
  c->noQmfSlots = cfg[1];
  c->sampleFreq = cfg[2];
  c->noQmfBands = cfg[3];
  c->num_Master = cfg[4];
  c->nSfb[LO] = cfg[5];
  c->nSfb[HI] = cfg[6];
  c->v_k_master = vkm;
  c->freqBandTable[LO] = lo;
  c->freqBandTable[HI] = hi;
}

extern "C" {
/* The layout facts the port relies on: offsetof(SBR_ENV_DATA, addHarmonic) and
   sizeof(SBR_ENV_DATA). */
void cref_ton_corr_env_data_layout(INT *out) {
  out[0] = (INT)offsetof(SBR_ENV_DATA, addHarmonic);
  out[1] = (INT)sizeof(SBR_ENV_DATA);
}
void *cref_ton_corr_new(INT chan) {
  SBR_TON_CORR_EST *h = (SBR_TON_CORR_EST *)calloc(1, sizeof(SBR_TON_CORR_EST));
  if (FDKsbrEnc_CreateTonCorrParamExtr(h, chan) != 0) return NULL;
  return h;
}
void cref_ton_corr_free(void *h) {
  FDKsbrEnc_DeleteTonCorrParamExtr((SBR_TON_CORR_EST *)h);
  free(h);
}
INT cref_ton_corr_init(void *h, INT frameSize, INT *cfg, UCHAR *vkm, UCHAR *lo, UCHAR *hi, INT timeSlots, INT xposCtrl, INT anaMaxLevel, INT noiseBands, INT noiseFloorOffset, UINT useSpeechConfig) {
  SBR_CONFIG_DATA c;
  config(&c, cfg, vkm, lo, hi);
  return FDKsbrEnc_InitTonCorrParamExtr(frameSize, (SBR_TON_CORR_EST *)h, &c, timeSlots, xposCtrl, anaMaxLevel, noiseBands, noiseFloorOffset, useSpeechConfig);
}
/* switchInverseFilt is set by the encoder after the init (initEnvChannel()). */
void cref_ton_corr_set_switch(void *h, INT v) { ((SBR_TON_CORR_EST *)h)->switchInverseFilt = v; }
INT cref_ton_corr_reset(void *h, INT xposctrl, INT highBandStartSb, UCHAR *vkm, INT numMaster, INT fs, UCHAR *lo, UCHAR *hi, INT *nSfb, INT noQmfChannels) {
  UCHAR *fbt[2] = {lo, hi};
  return FDKsbrEnc_ResetTonCorrParamExtr((SBR_TON_CORR_EST *)h, xposctrl, highBandStartSb, vkm, numMaster, fs, fbt, nSfb, noQmfChannels);
}
/* re, im: 32 rows of 64 values. */
void cref_ton_corr_quotas(void *h, FIXP_DBL *re, FIXP_DBL *im, INT usb, INT qmfScale) {
  FIXP_DBL *r[32], *i[32];
  rows(re, r);
  rows(im, i);
  FDKsbrEnc_CalculateTonalityQuotas((SBR_TON_CORR_EST *)h, r, i, usb, qmfScale);
}
/* addHarm: addHarmonic[48] and the 144 bytes that follow it in SBR_ENV_DATA (in and out);
   fi: the frame info as 17 INTs (the order of sbrdump_frame_info). */
void cref_ton_corr_extr(void *h, INT *infVec, FIXP_DBL *noiseLevels, INT *mhFlag, UCHAR *addHarm, UCHAR *envComp, INT *fi, UCHAR *tranInfo, UCHAR *fbtHi, INT nSfb, INT xposType, UINT flags) {
  SBR_ENV_DATA *e = (SBR_ENV_DATA *)calloc(1, sizeof(SBR_ENV_DATA));
  UCHAR *base = (UCHAR *)e + offsetof(SBR_ENV_DATA, addHarmonic);
  memcpy(base, addHarm, 4 * MAX_FREQ_COEFFS);
  SBR_FRAME_INFO f;
  frame_info(&f, fi);
  INVF_MODE inf[MAX_NUM_NOISE_VALUES];
  for (INT i = 0; i < MAX_NUM_NOISE_VALUES; i++) inf[i] = (INVF_MODE)infVec[i];
  FDKsbrEnc_TonCorrParamExtr((SBR_TON_CORR_EST *)h, inf, noiseLevels, mhFlag, e->addHarmonic, envComp, &f, tranInfo, fbtHi, nSfb, (XPOS_MODE)xposType, flags);
  for (INT i = 0; i < MAX_NUM_NOISE_VALUES; i++) infVec[i] = inf[i];
  memcpy(addHarm, base, 4 * MAX_FREQ_COEFFS);
  free(e);
}
INT cref_ton_corr_dump(void *h, INT *buf, INT cap) {
  SbrDump d = {buf, cap, 0};
  sbrdump_ton_corr(&d, (const SBR_TON_CORR_EST *)h);
  return d.n;
}
}
