// Shims for the differential tests of fdk_aac_rs_fdk::mdct.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_mdct (see build.rs).
//
// An mdct_t is wrapped with its overlap buffer and the buffers its pFacZir / pAsymOvlp pointers
// refer to. Window slopes are passed as (length, shape) and looked up with FDKgetWindowSlope.

#include "cref_common.h"
#include "common_fix.h"
#include "mdct.h"
#include "FDK_tools_rom.h"

#define CREF_MDCT_OVL_MAX 4096

struct CrefMdct {
  mdct_t m;
  FIXP_DBL ovl[CREF_MDCT_OVL_MAX];
  FIXP_DBL zir[1024];
  FIXP_DBL asym[1024];
};

static const FIXP_WTP *slope(INT len, INT shape) {
  return len > 0 ? FDKgetWindowSlope(len, shape) : NULL;
}

extern "C" {
/* A zeroed mdct_t (mdct_init sets neither ov_offset nor prev_wrs), initialized with
   mdct_init(&m, ovl, ovSize). The overlap buffer is filled with `fill`. */
void *cref_mdct_new(INT ovSize, FIXP_DBL fill) {
  CrefMdct *s = new CrefMdct;
  memset(s, 0, sizeof(*s));
  for (int i = 0; i < CREF_MDCT_OVL_MAX; i++) s->ovl[i] = fill;
  mdct_init(&s->m, s->ovl, ovSize);
  return s;
}
void cref_mdct_free(void *h) { delete (CrefMdct *)h; }
void cref_mdct_mdct_init(void *h, INT ovSize) {
  CrefMdct *s = (CrefMdct *)h;
  mdct_init(&s->m, s->ovl, ovSize);
}
/* The state: f[0..9] = prev_tl, prev_nr, prev_fr, ov_offset, ov_size, prevAliasSymmetry,
   prevPrevAliasSymmetry, pFacZir != NULL, pAsymOvlp != NULL; ovl[0..ov_size] = overlap. */
void cref_mdct_get_state(void *h, INT *f, FIXP_DBL *ovl) {
  CrefMdct *s = (CrefMdct *)h;
  f[0] = s->m.prev_tl;
  f[1] = s->m.prev_nr;
  f[2] = s->m.prev_fr;
  f[3] = s->m.ov_offset;
  f[4] = s->m.ov_size;
  f[5] = s->m.prevAliasSymmetry;
  f[6] = s->m.prevPrevAliasSymmetry;
  f[7] = s->m.pFacZir != NULL;
  f[8] = s->m.pAsymOvlp != NULL;
  memcpy(ovl, s->ovl, s->m.ov_size * sizeof(FIXP_DBL));
}
/* Sets the fields f[0..7] of get_state except ov_size (f[4]), and
   prev_wrs = FDKgetWindowSlope(wrsLen, wrsShape) (NULL for wrsLen <= 0). */
void cref_mdct_set_state(void *h, const INT *f, INT wrsLen, INT wrsShape) {
  CrefMdct *s = (CrefMdct *)h;
  s->m.prev_tl = f[0];
  s->m.prev_nr = f[1];
  s->m.prev_fr = f[2];
  s->m.ov_offset = f[3];
  s->m.prevAliasSymmetry = f[5];
  s->m.prevPrevAliasSymmetry = f[6];
  s->m.prev_wrs = slope(wrsLen, wrsShape);
}
/* Writes ovl[0..n] to the overlap buffer. */
void cref_mdct_set_overlap(void *h, const FIXP_DBL *ovl, INT n) {
  CrefMdct *s = (CrefMdct *)h;
  memcpy(s->ovl, ovl, n * sizeof(FIXP_DBL));
}
/* The index in lens/shapes of the first window slope equal to prev_wrs; -1 for NULL, -2 for
   none. */
INT cref_mdct_wrs_id(void *h, const INT *lens, const INT *shapes, INT n) {
  CrefMdct *s = (CrefMdct *)h;
  if (s->m.prev_wrs == NULL) return -1;
  for (int i = 0; i < n; i++)
    if (slope(lens[i], shapes[i]) == s->m.prev_wrs) return i;
  return -2;
}
/* pFacZir = the given n values (NULL for n < 0). */
void cref_mdct_set_fac_zir(void *h, const FIXP_DBL *v, INT n) {
  CrefMdct *s = (CrefMdct *)h;
  if (n < 0) {
    s->m.pFacZir = NULL;
    return;
  }
  memcpy(s->zir, v, n * sizeof(FIXP_DBL));
  s->m.pFacZir = s->zir;
}
/* pAsymOvlp = the given n values (NULL for n < 0). */
void cref_mdct_set_asym_ovlp(void *h, const FIXP_DBL *v, INT n) {
  CrefMdct *s = (CrefMdct *)h;
  if (n < 0) {
    s->m.pAsymOvlp = NULL;
    return;
  }
  memcpy(s->asym, v, n * sizeof(FIXP_DBL));
  s->m.pAsymOvlp = s->asym;
}
INT cref_mdct_mdct_block(void *h, const SHORT *timeData, INT noInSamples, FIXP_DBL *mdctData, INT nSpec, INT tl, INT wrsLen, INT wrsShape, INT fr, SHORT *pMdctData_e) {
  CrefMdct *s = (CrefMdct *)h;
  return mdct_block(&s->m, (const INT_PCM *)timeData, noInSamples, mdctData, nSpec, tl,
                    slope(wrsLen, wrsShape), fr, pMdctData_e);
}
void cref_mdct_imdct_gain(FIXP_DBL *pGain_m, INT *pGain_e, INT tl) { imdct_gain(pGain_m, pGain_e, tl); }
INT cref_mdct_imdct_drain(void *h, FIXP_DBL *output, INT nrSamplesRoom) {
  CrefMdct *s = (CrefMdct *)h;
  return imdct_drain(&s->m, output, nrSamplesRoom);
}
INT cref_mdct_imdct_copy_ov_and_nr(void *h, FIXP_DBL *pTimeData, INT nrSamples) {
  CrefMdct *s = (CrefMdct *)h;
  return imdct_copy_ov_and_nr(&s->m, pTimeData, nrSamples);
}
void cref_mdct_imdct_adapt_parameters(void *h, INT *pfl, INT *pnl, INT tl, INT wlsLen, INT wlsShape, INT noOutSamples) {
  CrefMdct *s = (CrefMdct *)h;
  imdct_adapt_parameters(&s->m, pfl, pnl, tl, slope(wlsLen, wlsShape), noOutSamples);
}
INT cref_mdct_imlt_block(void *h, FIXP_DBL *output, FIXP_DBL *spectrum, const SHORT *scalefactor, INT nSpec, INT noOutSamples, INT tl, INT wlsLen, INT wlsShape, INT fl, INT wrsLen, INT wrsShape, INT fr, FIXP_DBL gain, INT flags) {
  CrefMdct *s = (CrefMdct *)h;
  return imlt_block(&s->m, output, spectrum, scalefactor, nSpec, noOutSamples, tl,
                    slope(wlsLen, wlsShape), fl, slope(wrsLen, wrsShape), fr, gain, flags);
}
}
