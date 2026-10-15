// Shims for the differential tests of fdk_aac_rs_fdk::dct.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_dct (see build.rs).

#include "cref_common.h"
#include "common_fix.h"
#include "dct.h"
#include "FDK_tools_rom.h"

/* Identifies a sine table: 1 SineTable1024, 2 SineTable480, 3 SineTable384, 4 SineTable80,
   0 NULL, -1 other. */
static INT sine_table_id(const FIXP_STP *t) {
  if (t == NULL) return 0;
  if (t == SineTable1024) return 1;
  if (t == SineTable480) return 2;
  if (t == SineTable384) return 3;
  if (t == SineTable80) return 4;
  return -1;
}

/* Identifies a window slope by its first index in the flattened windowSlopes[0] (36 entries);
   -1 for NULL, -2 for another pointer. */
static INT window_id(const FIXP_WTP *w) {
  if (w == NULL) return -1;
  for (int r = 0; r < 4; r++)
    for (int k = 0; k < 9; k++)
      if (windowSlopes[0][r][k] == w) return r * 9 + k;
  return -2;
}

extern "C" {
/* dct_getTables(&twiddle, &sin_twiddle, &sin_step, length); out[0] = window id of twiddle,
   out[1] = sine table id, out[2] = sin_step. */
void cref_dct_dct_getTables(INT length, INT *out) {
  const FIXP_WTP *twiddle = NULL;
  const FIXP_STP *sin_twiddle = NULL;
  int sin_step = 0;
  dct_getTables(&twiddle, &sin_twiddle, &sin_step, length);
  out[0] = window_id(twiddle);
  out[1] = sine_table_id(sin_twiddle);
  out[2] = sin_step;
}
void cref_dct_dct_II(FIXP_DBL *pDat, FIXP_DBL *tmp, INT L, INT *pDat_e) { dct_II(pDat, tmp, L, pDat_e); }
void cref_dct_dct_III(FIXP_DBL *pDat, FIXP_DBL *tmp, INT L, INT *pDat_e) { dct_III(pDat, tmp, L, pDat_e); }
void cref_dct_dst_III(FIXP_DBL *pDat, FIXP_DBL *tmp, INT L, INT *pDat_e) { dst_III(pDat, tmp, L, pDat_e); }
void cref_dct_dct_IV(FIXP_DBL *pDat, INT L, INT *pDat_e) { dct_IV(pDat, L, pDat_e); }
void cref_dct_dst_IV(FIXP_DBL *pDat, INT L, INT *pDat_e) { dst_IV(pDat, L, pDat_e); }
}
