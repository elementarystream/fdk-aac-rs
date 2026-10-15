// Shims for the differential tests of fdk_aac_rs_sbrdec::hbe (tests/sbrdec/hbe.rs).
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrdec_hbe (see build.rs).
//
// One transposer instance lives in the shim (the reference's functions); its state is compared
// with the state dump of shim/sbrdec_e2e.cpp (cref_sbrdec_e2e_dump_hbe). The file-static
// helpers are reached through a copy of hbe.cpp in a namespace. QMF data is passed as flat
// arrays of slots of 64 values.

#include "cref_common.h"
#include "libSBRdec/src/hbe.h"
#include "libSBRdec/src/env_extr.h"
#include "qmf.h"

// (the public functions of the copy take arguments of global types: argument dependent lookup
// would also find the global declarations)
#define QmfTransposerCreate hbn_QmfTransposerCreate
#define QmfTransposerReInit hbn_QmfTransposerReInit
#define QmfTransposerClose hbn_QmfTransposerClose
#define QmfTransposerApply hbn_QmfTransposerApply
#define GetxOverBandQmfTransposer hbn_GetxOverBandQmfTransposer
#define Get41SbrQmfTransposer hbn_Get41SbrQmfTransposer
namespace hbn {
void QmfTransposerClose(HANDLE_HBE_TRANSPOSER hQmfTransposer);
#include "libSBRdec/src/hbe.cpp"
}
#undef QmfTransposerCreate
#undef QmfTransposerReInit
#undef QmfTransposerClose
#undef QmfTransposerApply
#undef GetxOverBandQmfTransposer
#undef Get41SbrQmfTransposer

static HANDLE_HBE_TRANSPOSER g_hbe;

extern "C" {

/* which: 2: invFourthRootNorm2, 3: invCubeRootNorm2, 4: inv3EigthRootNorm2 */
FIXP_DBL cref_sbrdec_hbe_inv_root(INT which, FIXP_DBL m, INT *e) {
  switch (which) {
    case 2: return hbn::invFourthRootNorm2(m, e);
    case 3: return hbn::invCubeRootNorm2(m, e);
    default: return hbn::inv3EigthRootNorm2(m, e);
  }
}

void cref_sbrdec_hbe_scale_up(FIXP_DBL *re, FIXP_DBL *im, INT *e) { hbn::scaleUp(re, im, e); }

void cref_sbrdec_hbe_center(FIXP_DBL gr, FIXP_DBL gi, FIXP_DBL *center, INT *e, INT stretch, INT mult) { hbn::calculateCenterFIXP(gr, gi, &center[0], &center[1], e, stretch, mult); }

INT cref_sbrdec_hbe_scale_factor_frame(INT bSbr41, INT maxStretch, INT pitchInBins) { return hbn::getHBEScaleFactorFrame(bSbr41, maxStretch, pitchInBins); }

/* acc: real and imaginary accumulator (updated) */
void cref_sbrdec_hbe_add_high_band(FIXP_DBL gr, FIXP_DBL gi, INT ge, FIXP_DBL mult, FIXP_DBL cr, FIXP_DBL ci, INT ce, INT stretch, INT scaleFactorHbe, FIXP_DBL *acc) { hbn::addHighBandPart(gr, gi, ge, mult, cr, ci, ce, stretch, scaleFactorHbe, &acc[0], &acc[1]); }

void *cref_sbrdec_hbe_handle(void) { return g_hbe; }

INT cref_sbrdec_hbe_create(INT frameSize, INT bDisableCrossProducts, INT bSbr41) {
  if (g_hbe != NULL) QmfTransposerClose(g_hbe);
  g_hbe = NULL;
  return QmfTransposerCreate(&g_hbe, frameSize, bDisableCrossProducts, bSbr41);
}

INT cref_sbrdec_hbe_reinit(UCHAR *lo, UCHAR *hi, UCHAR nLo, UCHAR nHi) {
  UCHAR *t[2] = {lo, hi};
  UCHAR n[2] = {nLo, nHi};
  return QmfTransposerReInit(g_hbe, t, n);
}

void cref_sbrdec_hbe_close(void) {
  QmfTransposerClose(g_hbe);
  g_hbe = NULL;
}

/* re/im: nRows slots; the codec input starts at slot codecOffset, the output at slot 0.
   lpcRe/lpcIm: 14 rows of 64 values. */
void cref_sbrdec_hbe_apply(FIXP_DBL *re, FIXP_DBL *im, INT nRows, INT codecOffset, INT nColsIn, FIXP_DBL *lpcRe, FIXP_DBL *lpcIm, INT pitchInBins, INT scaleLb, INT scaleHbe, INT *scaleHb, INT timeStep, INT firstSlotOffset, INT ovLen, INT mode) {
  FIXP_DBL *r[160], *i[160];
  for (int k = 0; k < nRows && k < 160; k++) {
    r[k] = re + 64 * k;
    i[k] = im + 64 * k;
  }
  QmfTransposerApply(g_hbe, r + codecOffset, i + codecOffset, nColsIn, r, i, (FIXP_DBL(*)[64])lpcRe, (FIXP_DBL(*)[64])lpcIm, pitchInBins, scaleLb, scaleHbe, scaleHb, timeStep, firstSlotOffset, ovLen, (KEEP_STATES_SYNCED_MODE)mode);
}

/* GetxOverBandQmfTransposer (MAX_NUM_PATCHES values) and Get41SbrQmfTransposer */
INT cref_sbrdec_hbe_get(INT *xOver) {
  int *x = GetxOverBandQmfTransposer(g_hbe);
  for (int k = 0; k < MAX_NUM_PATCHES; k++) xOver[k] = (x == NULL) ? -999 : x[k];
  return Get41SbrQmfTransposer(g_hbe);
}

}  // extern "C"
