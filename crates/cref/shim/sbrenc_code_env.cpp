// Shims for the differential tests of fdk_aac_rs_sbrenc::code_env.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_code_env (see build.rs).
//
// SBR_CODE_ENVELOPE instances on the heap, dumped with sbrdump_code_env()
// (sbrenc_sbr_encoder_dump.cpp).

#include "cref_common.h"
#include <stdlib.h>
#include "libSBRenc/src/code_env.h"
#include "libSBRenc/src/bit_sbr.h"

struct SbrDump {
  INT *buf;
  INT cap;
  INT n;
};
void sbrdump_code_env(SbrDump *d, const SBR_CODE_ENVELOPE *h);
void sbrdump_env_data(SbrDump *d, const SBR_ENV_DATA *e);

extern "C" {
void *cref_code_env_new(INT *nSfb, INT deltaTAcrossFrames, FIXP_DBL dF_edge_1stEnv, FIXP_DBL dF_edge_incr) {
  SBR_CODE_ENVELOPE *h = (SBR_CODE_ENVELOPE *)calloc(1, sizeof(SBR_CODE_ENVELOPE));
  FDKsbrEnc_InitSbrCodeEnvelope(h, nSfb, deltaTAcrossFrames, dF_edge_1stEnv, dF_edge_incr);
  return h;
}
void cref_code_env_free(void *h) { free(h); }
/* FDKsbrEnc_InitSbrHuffmanTables() on a cleared SBR_ENV_DATA, which is dumped into buf. */
INT cref_code_env_huffman(void *henv, void *hnoise, INT ampRes, INT *buf, INT cap, INT *n) {
  SBR_ENV_DATA *e = (SBR_ENV_DATA *)calloc(1, sizeof(SBR_ENV_DATA));
  INT r = FDKsbrEnc_InitSbrHuffmanTables(e, (SBR_CODE_ENVELOPE *)henv, (SBR_CODE_ENVELOPE *)hnoise, (AMP_RES)ampRes);
  SbrDump d = {buf, cap, 0};
  sbrdump_env_data(&d, e);
  *n = d.n;
  free(e);
  return r;
}
/* dF_edge_incr_fac is set by the envelope extraction. */
void cref_code_env_set_fac(void *h, INT fac) { ((SBR_CODE_ENVELOPE *)h)->dF_edge_incr_fac = fac; }
void cref_code_env_code(void *h, SCHAR *sfb_nrg, INT *freqRes, INT *directionVec, INT coupling, INT nEnvelopes, INT channel, INT headerActive) {
  FREQ_RES fr[MAX_ENVELOPES];
  for (INT i = 0; i < MAX_ENVELOPES; i++) fr[i] = (FREQ_RES)freqRes[i];
  FDKsbrEnc_codeEnvelope(sfb_nrg, fr, (SBR_CODE_ENVELOPE *)h, directionVec, coupling, nEnvelopes, channel, headerActive);
}
INT cref_code_env_dump(void *h, INT *buf, INT cap) {
  SbrDump d = {buf, cap, 0};
  sbrdump_code_env(&d, (const SBR_CODE_ENVELOPE *)h);
  return d.n;
}
}
