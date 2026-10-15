// Shims for the differential tests of fdk_aac_rs_sacdec::sac_tsd.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacdec_sac_tsd (see build.rs).
//
// TSD_DATA is passed as INT[66]: bsTsdEnable, numSlots, bsTsdTrPhaseData[64].

#include "cref_common.h"
#include "FDK_bitstream.h"
#include "sac_tsd.h"

static void tsd_from(TSD_DATA *t, const INT *v) {
  t->bsTsdEnable = (UCHAR)v[0];
  t->numSlots = (UCHAR)v[1];
  for (int i = 0; i < MAX_TSD_TIME_SLOTS; i++) t->bsTsdTrPhaseData[i] = (SCHAR)v[2 + i];
}
static void tsd_to(const TSD_DATA *t, INT *v) {
  v[0] = t->bsTsdEnable;
  v[1] = t->numSlots;
  for (int i = 0; i < MAX_TSD_TIME_SLOTS; i++) v[2 + i] = t->bsTsdTrPhaseData[i];
}

extern "C" {
/* TsdRead on a bitstream over buf (bufBytes: a power of two, all bits valid). tsd: in/out.
   *bits: the number of bits read. */
INT cref_tsd_read(UCHAR *buf, INT bufBytes, INT numSlots, INT *tsd, INT *bits) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufBytes, bufBytes * 8, BS_READER);
  TSD_DATA t;
  tsd_from(&t, tsd);
  INT r = TsdRead(&bs, numSlots, &t);
  tsd_to(&t, tsd);
  *bits = bufBytes * 8 - (INT)FDKgetValidBits(&bs);
  return r;
}
/* Returns 0 if the decorrelator reads from the direct signal, 1 if from the nonTr signal. */
INT cref_tsd_generate_non_tr(INT numHybridBands, const INT *tsd, INT ts, FIXP_DBL *vdRe, FIXP_DBL *vdIm, FIXP_DBL *vnRe, FIXP_DBL *vnIm) {
  TSD_DATA t;
  tsd_from(&t, tsd);
  FIXP_DBL *inRe = NULL, *inIm = NULL;
  TsdGenerateNonTr(numHybridBands, &t, ts, vdRe, vdIm, vnRe, vnIm, &inRe, &inIm);
  if (inRe == vdRe && inIm == vdIm) return 0;
  if (inRe == vnRe && inIm == vnIm) return 1;
  return -1;
}
void cref_tsd_apply(INT numHybridBands, const INT *tsd, INT *tsdTs, const FIXP_DBL *vdRe, const FIXP_DBL *vdIm, FIXP_DBL *dRe, FIXP_DBL *dIm) {
  TSD_DATA t;
  tsd_from(&t, tsd);
  TsdApply(numHybridBands, &t, tsdTs, vdRe, vdIm, dRe, dIm);
}
}
