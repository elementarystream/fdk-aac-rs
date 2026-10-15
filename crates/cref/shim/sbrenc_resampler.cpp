// Shims for the differential tests of fdk_aac_rs_sbrenc::resampler.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_resampler (see build.rs).
//
// The downsampler of the library is driven through its public functions; the filter tables
// (file-static) are reached through a copy of resampler.cpp in a namespace.

#include "cref_common.h"
#include <stdlib.h>
#include "libSBRenc/src/resampler.h"

namespace ref_resampler {
#include "libSBRenc/src/resampler.cpp"
}

/* The index of an SOS table (sos48, sos45, sos41, sos35, sos25), -1 for NULL. The library's
   tables are found by their content (they are file-static in the library). */
static INT sos_index(const FIXP_SGL *p) {
  const FIXP_SGL *sos[5] = {ref_resampler::sos48, ref_resampler::sos45, ref_resampler::sos41,
                            ref_resampler::sos35, ref_resampler::sos25};
  const size_t len[5] = {sizeof(ref_resampler::sos48), sizeof(ref_resampler::sos45),
                         sizeof(ref_resampler::sos41), sizeof(ref_resampler::sos35),
                         sizeof(ref_resampler::sos25)};
  if (p == NULL) return -1;
  for (INT i = 0; i < 5; i++)
    if (memcmp(p, sos[i], len[i]) == 0) return i;
  return -2;
}

extern "C" {
void *cref_resampler_new(void) { return calloc(1, sizeof(DOWNSAMPLER)); }
void cref_resampler_free(void *h) { free(h); }
INT cref_resampler_init(void *h, INT wc, INT ratio) {
  return FDKaacEnc_InitDownsampler((DOWNSAMPLER *)h, wc, ratio);
}
/* FDKaacEnc_Downsample() in place on buf: input at inOff, output at outOff. */
INT cref_resampler_downsample(void *h, SHORT *buf, INT inOff, INT n, INT outOff, INT *nOut) {
  return FDKaacEnc_Downsample((DOWNSAMPLER *)h, buf + inOff, n, buf + outOff, nOut);
}
/* State: out[0..32] states, out[32] coeffa index, out[33] gain, out[34] Wc, out[35] noCoeffs,
   out[36] ptr, out[37] ratio, out[38] delay, out[39] pending. */
void cref_resampler_state(void *h, INT *out) {
  DOWNSAMPLER *d = (DOWNSAMPLER *)h;
  INT k = 0;
  for (INT i = 0; i < MAXNR_SECTIONS + 1; i++)
    for (INT j = 0; j < 2; j++) out[k++] = d->downFilter.states[i][j];
  out[k++] = sos_index(d->downFilter.coeffa);
  out[k++] = d->downFilter.gain;
  out[k++] = d->downFilter.Wc;
  out[k++] = d->downFilter.noCoeffs;
  out[k++] = d->downFilter.ptr;
  out[k++] = d->ratio;
  out[k++] = d->delay;
  out[k++] = d->pending;
}
/* The tables: id 0..4 sos48..sos25, 5..9 the parameter sets (coeffa index, g, Wc, noCoeffs,
   delay), 10 filter_paramSet (indices). Returns the number of values, -1 for an unknown id. */
INT cref_resampler_table(INT id, INT *out) {
  const FIXP_SGL *sos[5] = {ref_resampler::sos48, ref_resampler::sos45, ref_resampler::sos41,
                            ref_resampler::sos35, ref_resampler::sos25};
  const INT len[5] = {sizeof(ref_resampler::sos48) / 2, sizeof(ref_resampler::sos45) / 2,
                      sizeof(ref_resampler::sos41) / 2, sizeof(ref_resampler::sos35) / 2,
                      sizeof(ref_resampler::sos25) / 2};
  const struct ref_resampler::FILTER_PARAM *ps[5] = {
      &ref_resampler::param_set48, &ref_resampler::param_set45, &ref_resampler::param_set41,
      &ref_resampler::param_set35, &ref_resampler::param_set25};
  if (id >= 0 && id < 5) {
    for (INT i = 0; i < len[id]; i++) out[i] = sos[id][i];
    return len[id];
  }
  if (id >= 5 && id < 10) {
    const struct ref_resampler::FILTER_PARAM *p = ps[id - 5];
    out[0] = sos_index(p->coeffa);
    out[1] = p->g;
    out[2] = p->Wc;
    out[3] = p->noCoeffs;
    out[4] = p->delay;
    return 5;
  }
  if (id == 10) {
    INT n = sizeof(ref_resampler::filter_paramSet) / sizeof(ref_resampler::filter_paramSet[0]);
    for (INT i = 0; i < n; i++) {
      out[i] = -1;
      for (INT j = 0; j < 5; j++)
        if (ref_resampler::filter_paramSet[i] == ps[j]) out[i] = j;
    }
    return n;
  }
  return -1;
}
}
