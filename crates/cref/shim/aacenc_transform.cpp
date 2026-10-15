// Shims for the differential tests of fdk_aac_rs_aacenc::transform.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_transform (see build.rs).
//
// The encoder's mdct_t (PSY_STATIC::mdctPers, initialized with mdct_init(&mdctPers, NULL, 0)) is
// wrapped in an opaque handle; its window slope is identified by (length, shape).

#include "cref_common.h"
#include "common_fix.h"
#include "mdct.h"
#include "FDK_tools_rom.h"
#include "libAACenc/src/transform.h"

extern "C" {
/* A zeroed mdct_t initialized like FDKaacEnc_psyInit() does. */
void *cref_transform_mdct_new() {
  mdct_t *m = new mdct_t;
  memset(m, 0, sizeof(*m));
  mdct_init(m, NULL, 0);
  return m;
}
void cref_transform_mdct_free(void *h) { delete (mdct_t *)h; }
/* f[0..5] = prev_tl, prev_nr, prev_fr, ov_offset, ov_size, overlap.freq != NULL. Returns the
   index of the first (lens[i], shapes[i]) whose window slope is prev_wrs, -1 for NULL, -2 for
   none. */
INT cref_transform_mdct_state(void *h, INT *f, const INT *lens, const INT *shapes, INT n) {
  mdct_t *m = (mdct_t *)h;
  f[0] = m->prev_tl;
  f[1] = m->prev_nr;
  f[2] = m->prev_fr;
  f[3] = m->ov_offset;
  f[4] = m->ov_size;
  f[5] = m->overlap.freq != NULL;
  if (m->prev_wrs == NULL) return -1;
  for (int i = 0; i < n; i++)
    if (FDKgetWindowSlope(lens[i], shapes[i]) == m->prev_wrs) return i;
  return -2;
}
INT cref_transform_Transform_Real(const SHORT *pTimeData, FIXP_DBL *mdctData, INT blockType, INT windowShape, INT *prevWindowShape, void *mdctPers, INT frameLength, INT *pMdctData_e, INT filterType) {
  return FDKaacEnc_Transform_Real((const INT_PCM *)pTimeData, mdctData, blockType, windowShape,
                                  prevWindowShape, (H_MDCT)mdctPers, frameLength, pMdctData_e,
                                  filterType);
}
INT cref_transform_Transform_Real_Eld(const SHORT *pTimeData, FIXP_DBL *mdctData, INT blockType, INT windowShape, INT *prevWindowShape, INT frameLength, INT *mdctData_e, INT filterType, FIXP_DBL *overlapAddBuffer) {
  return FDKaacEnc_Transform_Real_Eld((const INT_PCM *)pTimeData, mdctData, blockType,
                                      windowShape, prevWindowShape, frameLength, mdctData_e,
                                      filterType, overlapAddBuffer);
}
}
