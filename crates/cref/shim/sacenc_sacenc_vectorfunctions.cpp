// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_vectorfunctions.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_vectorfunctions (see build.rs).
//
// The functions of the library (libfdkaac_ref.a) are called. Complex vectors are passed as
// interleaved INT arrays (re, im) and copied into aligned FIXP_DPK arrays; two-dimensional
// arrays are row-major and get a row pointer array.

#include "cref_common.h"
#include <stdlib.h>
#include "libSACenc/src/sacenc_vectorfunctions.h"

/* A copy of `rows` x `cols` complex values with row pointers (freed with sacenc_mat_free). */
FIXP_DPK **sacenc_mat_new(const INT *v, INT rows, INT cols) {
  FIXP_DPK **p = (FIXP_DPK **)malloc(sizeof(FIXP_DPK *) * (rows > 0 ? rows : 1));
  FIXP_DPK *d = (FIXP_DPK *)malloc(sizeof(FIXP_DPK) * (rows * cols > 0 ? rows * cols : 1));
  for (INT i = 0; i < rows * cols; i++) {
    d[i].v.re = v[2 * i];
    d[i].v.im = v[2 * i + 1];
  }
  for (INT r = 0; r < rows; r++) p[r] = d + r * cols;
  if (rows > 0) p[0] = d;
  return p;
}
void sacenc_mat_get(FIXP_DPK **p, INT *v, INT rows, INT cols) {
  for (INT i = 0; i < rows * cols; i++) {
    v[2 * i] = p[0][i].v.re;
    v[2 * i + 1] = p[0][i].v.im;
  }
}
void sacenc_mat_free(FIXP_DPK **p, INT rows) {
  if (rows > 0) free(p[0]);
  free(p);
}

extern "C" {
INT cref_sacenc_vectorfunctions_sumUpCplxPow2(const INT *x, INT scaleMode, INT inScaleFactor, INT *outScaleFactor, INT n) {
  FIXP_DPK **m = sacenc_mat_new(x, 1, n);
  INT r = sumUpCplxPow2(m[0], scaleMode, inScaleFactor, outScaleFactor, n);
  sacenc_mat_free(m, 1);
  return r;
}
INT cref_sacenc_vectorfunctions_sumUpCplxPow2Dim2(const INT *x, INT rows, INT cols, INT scaleMode, INT inScaleFactor, INT *outScaleFactor, INT sDim1, INT nDim1, INT sDim2, INT nDim2) {
  FIXP_DPK **m = sacenc_mat_new(x, rows, cols);
  INT r = sumUpCplxPow2Dim2(m, scaleMode, inScaleFactor, outScaleFactor, sDim1, nDim1, sDim2, nDim2);
  sacenc_mat_free(m, rows);
  return r;
}
void cref_sacenc_vectorfunctions_copyCplxVec(INT *z, const INT *x, INT n) {
  FIXP_DPK **mz = sacenc_mat_new(z, 1, n);
  FIXP_DPK **mx = sacenc_mat_new(x, 1, n);
  copyCplxVec(mz[0], mx[0], n);
  sacenc_mat_get(mz, z, 1, n);
  sacenc_mat_free(mz, 1);
  sacenc_mat_free(mx, 1);
}
void cref_sacenc_vectorfunctions_setCplxVec(INT *z, INT a, INT n) {
  FIXP_DPK **mz = sacenc_mat_new(z, 1, n);
  setCplxVec(mz[0], a, n);
  sacenc_mat_get(mz, z, 1, n);
  sacenc_mat_free(mz, 1);
}
void cref_sacenc_vectorfunctions_cplxScalarProduct(INT *z, const INT *x, const INT *y, INT rows, INT cols, INT scaleX, INT scaleY, INT *scaleZ, INT sDim1, INT nDim1, INT sDim2, INT nDim2) {
  FIXP_DPK **mx = sacenc_mat_new(x, rows, cols);
  FIXP_DPK **my = sacenc_mat_new(y, rows, cols);
  FIXP_DPK r;
  r.v.re = z[0];
  r.v.im = z[1];
  cplx_cplxScalarProduct(&r, mx, my, scaleX, scaleY, scaleZ, sDim1, nDim1, sDim2, nDim2);
  z[0] = r.v.re;
  z[1] = r.v.im;
  sacenc_mat_free(mx, rows);
  sacenc_mat_free(my, rows);
}
void cref_sacenc_vectorfunctions_FDKcalcCorrelationVec(INT *z, const INT *pr12, const INT *p1, const INT *p2, INT n) {
  FDKcalcCorrelationVec((FIXP_DBL *)z, (const FIXP_DBL *)pr12, (const FIXP_DBL *)p1, (const FIXP_DBL *)p2, n);
}
void cref_sacenc_vectorfunctions_calcCoherenceVec(INT *z, const INT *p12r, const INT *p12i, const INT *p1, const INT *p2, INT scaleP12, INT scaleP, INT n) {
  calcCoherenceVec((FIXP_DBL *)z, (const FIXP_DBL *)p12r, (const FIXP_DBL *)p12i, (const FIXP_DBL *)p1, (const FIXP_DBL *)p2, scaleP12, scaleP, n);
}
void cref_sacenc_vectorfunctions_addWeightedCplxVec(INT *z, const INT *a, const INT *x, const INT *b, const INT *y, INT rows, INT cols, INT scale, INT *scaleCh1, INT scaleCh2, const UCHAR *offsets, INT nParameterBands, INT nTimeSlots, INT startTimeSlot) {
  FIXP_DPK **mz = sacenc_mat_new(z, rows, cols);
  FIXP_DPK **mx = sacenc_mat_new(x, rows, cols);
  FIXP_DPK **my = sacenc_mat_new(y, rows, cols);
  addWeightedCplxVec(mz, (const FIXP_DBL *)a, mx, (const FIXP_DBL *)b, my, scale, scaleCh1, scaleCh2, offsets, nParameterBands, nTimeSlots, startTimeSlot);
  sacenc_mat_get(mz, z, rows, cols);
  sacenc_mat_free(mz, rows);
  sacenc_mat_free(mx, rows);
  sacenc_mat_free(my, rows);
}
void cref_sacenc_vectorfunctions_FDKcalcPbScaleFactor(const INT *x, INT rows, INT cols, const UCHAR *offsets, INT *outScaleFactor, INT startTimeSlot, INT nTimeSlots, INT nParamBands) {
  FIXP_DPK **mx = sacenc_mat_new(x, rows, cols);
  FDKcalcPbScaleFactor(mx, offsets, outScaleFactor, startTimeSlot, nTimeSlots, nParamBands);
  sacenc_mat_free(mx, rows);
}
INT cref_sacenc_vectorfunctions_FDKcalcScaleFactor(const INT *x, const INT *y, INT n) {
  return FDKcalcScaleFactor((const FIXP_DBL *)x, (const FIXP_DBL *)y, n);
}
INT cref_sacenc_vectorfunctions_FDKcalcScaleFactorDPK(const INT *x, INT n, INT startBand, INT bands) {
  FIXP_DPK **m = sacenc_mat_new(x, 1, n);
  INT r = FDKcalcScaleFactorDPK(m[0], startBand, bands);
  sacenc_mat_free(m, 1);
  return r;
}
}

extern "C" {
/* The templates FDKmemcpy_flex and FDKmemset_flex (INT instantiation); dst and src point into
   the arrays at the offsets. */
void cref_sacenc_vectorfunctions_memcpy_flex(INT *dst, INT dstOff, INT dstStride, const INT *src, INT srcOff, INT srcStride, INT n) {
  FDKmemcpy_flex(dst + dstOff, dstStride, src + srcOff, srcStride, n);
}
void cref_sacenc_vectorfunctions_memset_flex(INT *x, INT c, INT n) { FDKmemset_flex(x, c, n); }
}
