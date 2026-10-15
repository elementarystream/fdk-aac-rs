// Shims for the differential tests of fdk_aac_rs_sbrenc::bit_sbr.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_bit_sbr (see build.rs).
//
// One SBR payload as FDKsbrEnc_EnvEncodeFrame() writes it: the bitstream of a COMMON_DATA on a
// payload buffer of MAX_PAYLOAD_SIZE bytes, FDKsbrEnc_InitSbrBitstream(), the element writer,
// FDKsbrEnc_AssembleSbrBitstream(). The envelope data is built from a flat description (see
// env_from()), the grids and PS_OUT from the INTs of their dumps.

#include "cref_common.h"
#include <stdlib.h>
#include "libSBRenc/src/bit_sbr.h"
#include "libSBRenc/src/code_env.h"
#include "libSBRenc/src/env_bit.h"
#include "libSBRenc/src/ps_main.h"
#include "libSBRenc/src/ps_bitenc.h"
#include "sbr_encoder.h"

struct SbrDump {
  INT *buf;
  INT cap;
  INT n;
};
void sbrdump_cmon_data(SbrDump *d, const COMMON_DATA *h, const UCHAR *base);

/* v: ampRes, ldGrid, noOfEnvelopes, domain_vec[5], domain_vec_noise[5], noOfnoisebands,
   sbr_invf_mode_vec[10], ienvelope[5][48], noScfBands[5], balance, sbr_noise_levels[48],
   addHarmonicFlag, noHarmonics, addHarmonic[48], currentAmpResFF. */
static void env_from(SBR_ENV_DATA *e, const INT *v) {
  SBR_CODE_ENVELOPE henv, hnoise;
  FDKmemclear(e, sizeof(*e));
  FDKmemclear(&henv, sizeof(henv));
  FDKmemclear(&hnoise, sizeof(hnoise));
  INT k = 0;
  FDKsbrEnc_InitSbrHuffmanTables(e, &henv, &hnoise, (AMP_RES)v[k++]);
  e->ldGrid = (UCHAR)v[k++];
  e->noOfEnvelopes = v[k++];
  for (INT i = 0; i < MAX_ENVELOPES; i++) e->domain_vec[i] = v[k++];
  for (INT i = 0; i < MAX_ENVELOPES; i++) e->domain_vec_noise[i] = v[k++];
  e->noOfnoisebands = v[k++];
  for (INT i = 0; i < MAX_NUM_NOISE_VALUES; i++) e->sbr_invf_mode_vec[i] = (INVF_MODE)v[k++];
  for (INT i = 0; i < MAX_ENVELOPES; i++)
    for (INT j = 0; j < MAX_FREQ_COEFFS; j++) e->ienvelope[i][j] = v[k++];
  for (INT i = 0; i < MAX_ENVELOPES; i++) e->noScfBands[i] = v[k++];
  e->balance = v[k++];
  for (INT i = 0; i < MAX_FREQ_COEFFS; i++) e->sbr_noise_levels[i] = (SCHAR)v[k++];
  e->addHarmonicFlag = v[k++];
  e->noHarmonics = v[k++];
  for (INT i = 0; i < MAX_FREQ_COEFFS; i++) e->addHarmonic[i] = (UCHAR)v[k++];
  e->currentAmpResFF = (AMP_RES)v[k++];
}

/* The INTs of sbrdump_grid(). */
static void grid_from(SBR_GRID *g, const INT *v) {
  INT k = 0;
  g->bufferFrameStart = v[k++];
  g->numberTimeSlots = v[k++];
  g->frameClass = (FRAME_CLASS)v[k++];
  g->bs_num_env = v[k++];
  g->bs_abs_bord = v[k++];
  g->n = v[k++];
  g->p = v[k++];
  for (INT i = 0; i < MAX_NUM_REL; i++) g->bs_rel_bord[i] = v[k++];
  for (INT i = 0; i < MAX_ENVELOPES_FIXVAR_VARFIX; i++) g->v_f[i] = v[k++];
  g->bs_abs_bord_0 = v[k++];
  g->bs_abs_bord_1 = v[k++];
  g->bs_num_rel_0 = v[k++];
  g->bs_num_rel_1 = v[k++];
  for (INT i = 0; i < MAX_NUM_REL; i++) g->bs_rel_bord_0[i] = v[k++];
  for (INT i = 0; i < MAX_NUM_REL; i++) g->bs_rel_bord_1[i] = v[k++];
  for (INT i = 0; i < MAX_ENVELOPES_VARVAR; i++) g->v_fLR[i] = v[k++];
}

/* The INTs of sbrdump_ps_out(). */
static void ps_out_from(PS_OUT *o, const INT *v) {
  INT k = 0;
  o->enablePSHeader = v[k++];
  o->enableIID = v[k++];
  o->iidMode = v[k++];
  o->enableICC = v[k++];
  o->iccMode = v[k++];
  o->enableIpdOpd = v[k++];
  o->frameClass = v[k++];
  o->nEnvelopes = v[k++];
  for (INT i = 0; i < PS_MAX_ENVELOPES; i++) o->frameBorder[i] = v[k++];
  PS_DELTA *delta[4] = {o->deltaIID, o->deltaICC, o->deltaIPD, o->deltaOPD};
  INT(*val[4])[PS_MAX_BANDS] = {o->iid, o->icc, o->ipd, o->opd};
  INT *last[4] = {o->iidLast, o->iccLast, o->ipdLast, o->opdLast};
  for (INT p = 0; p < 4; p++) {
    for (INT i = 0; i < PS_MAX_ENVELOPES; i++) delta[p][i] = (PS_DELTA)v[k++];
    for (INT i = 0; i < PS_MAX_ENVELOPES; i++)
      for (INT b = 0; b < PS_MAX_BANDS; b++) val[p][i][b] = v[k++];
    for (INT b = 0; b < PS_MAX_BANDS; b++) last[p][b] = v[k++];
  }
}

extern "C" {
/* The 144 bytes of the SBR_ENV_DATA of env (see env_from()) that follow addHarmonic (reproduced
   from the C layout, see the port, mh_det.rs). */
void cref_bit_sbr_env_tail(INT *env, UCHAR *out) {
  SBR_ENV_DATA *e = (SBR_ENV_DATA *)calloc(1, sizeof(SBR_ENV_DATA));
  env_from(e, env);
  FDKmemcpy(out, (UCHAR *)e->addHarmonic + MAX_FREQ_COEFFS, 3 * MAX_FREQ_COEFFS);
  free(e);
}
/* elem: 1 single channel element, 2 channel pair element, 3 counted channel pair element.
   hdr: the 17 INTs of sbrdump_header_data(); headerActive: SBR_BITSTREAM_DATA::HeaderActive;
   psOut: NULL or the INTs of a PS_OUT (a PARAMETRIC_STEREO is created around it); payload: the
   MAX_PAYLOAD_SIZE payload buffer; ret[0]: the element writer's result, ret[1]: the CRC region,
   ret[2]: the valid bits after the assembly. The COMMON_DATA is dumped into dump. */
void cref_bit_sbr_payload(INT elem, INT *hdr, INT headerActive, INT *envL, INT *gridL, INT *envR, INT *gridR, INT *psOut, UINT flags, UCHAR *payload, INT *ret, INT *dump, INT cap, INT *n) {
  SBR_HEADER_DATA h;
  INT k = 0;
  h.sbr_amp_res = (AMP_RES)hdr[k++];
  h.sbr_start_frequency = hdr[k++];
  h.sbr_stop_frequency = hdr[k++];
  h.sbr_xover_band = hdr[k++];
  h.sbr_noise_bands = hdr[k++];
  h.sbr_data_extra = hdr[k++];
  h.header_extra_1 = hdr[k++];
  h.header_extra_2 = hdr[k++];
  h.sbr_lc_stereo_mode = hdr[k++];
  h.sbr_limiter_bands = hdr[k++];
  h.sbr_limiter_gains = hdr[k++];
  h.sbr_interpol_freq = hdr[k++];
  h.sbr_smoothing_length = hdr[k++];
  h.alterScale = hdr[k++];
  h.freqScale = hdr[k++];
  h.coupling = hdr[k++];
  h.prev_coupling = hdr[k++];
  SBR_BITSTREAM_DATA b;
  FDKmemclear(&b, sizeof(b));
  b.HeaderActive = headerActive;

  SBR_ENV_DATA *l = (SBR_ENV_DATA *)calloc(1, sizeof(SBR_ENV_DATA));
  SBR_ENV_DATA *r = (SBR_ENV_DATA *)calloc(1, sizeof(SBR_ENV_DATA));
  SBR_GRID gl, gr;
  env_from(l, envL);
  grid_from(&gl, gridL);
  l->hSbrBSGrid = &gl;
  if (envR != NULL) {
    env_from(r, envR);
    grid_from(&gr, gridR);
    r->hSbrBSGrid = &gr;
  }
  HANDLE_PARAMETRIC_STEREO ps = NULL;
  if (psOut != NULL) {
    PSEnc_Create(&ps);
    ps_out_from(&ps->psOut[0], psOut);
  }

  COMMON_DATA cmon;
  FDKmemclear(&cmon, sizeof(cmon));
  FDK_CRCINFO crcInfo;
  FDKinitBitStream(&cmon.sbrBitbuf, payload, MAX_PAYLOAD_SIZE, 0, BS_WRITER);
  INT crcReg = FDKsbrEnc_InitSbrBitstream(&cmon, payload, MAX_PAYLOAD_SIZE, &crcInfo, flags);
  INT bits = 0;
  switch (elem) {
    case 1:
      bits = FDKsbrEnc_WriteEnvSingleChannelElement(&h, ps, &b, l, &cmon, flags);
      break;
    case 2:
      bits = FDKsbrEnc_WriteEnvChannelPairElement(&h, ps, &b, l, r, &cmon, flags);
      break;
    default:
      bits = FDKsbrEnc_CountSbrChannelPairElement(&h, ps, &b, l, r, &cmon, flags);
      break;
  }
  FDKsbrEnc_AssembleSbrBitstream(&cmon, &crcInfo, crcReg, flags);
  ret[0] = bits;
  ret[1] = crcReg;
  ret[2] = (INT)FDKgetValidBits(&cmon.sbrBitbuf);
  SbrDump d = {dump, cap, 0};
  sbrdump_cmon_data(&d, &cmon, payload);
  *n = d.n;
  if (ps != NULL) PSEnc_Destroy(&ps);
  free(l);
  free(r);
}
}
