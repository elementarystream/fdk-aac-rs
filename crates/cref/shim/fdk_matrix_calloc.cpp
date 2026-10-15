// Shims for the differential tests of fdk_aac_rs_fdk::matrix_calloc.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_matrix_calloc (see build.rs).
//
// The allocation functions are compared by the layout they produce: NULL or not, the offsets of
// the row pointers in elements from the start of the block, and whether the memory is zero.

#include "cref_common.h"
#include "FDK_matrixCalloc.h"

static INT all_zero(const void *p, UINT bytes) {
  const UCHAR *b = (const UCHAR *)p;
  for (UINT i = 0; i < bytes; i++) {
    if (b[i] != 0) return 0;
  }
  return 1;
}

extern "C" {
/* variant 0: fdkCallocMatrix1D, 1: _aligned, 2: _int, 3: _int_aligned. Returns -1 for NULL,
   else whether the memory is zero. */
INT cref_mc_1d(UINT dim1, UINT size, INT variant) {
  void *p;
  switch (variant) {
    case 0:
      p = fdkCallocMatrix1D(dim1, size);
      break;
    case 1:
      p = fdkCallocMatrix1D_aligned(dim1, size);
      break;
    case 2:
      p = fdkCallocMatrix1D_int(dim1, size, SECT_DATA_L1);
      break;
    default:
      p = fdkCallocMatrix1D_int_aligned(dim1, size, SECT_DATA_L1);
      break;
  }
  if (p == NULL) return -1;
  INT z = all_zero(p, dim1 * size);
  if (variant == 1 || variant == 3) {
    fdkFreeMatrix1D_aligned(p);
  } else {
    fdkFreeMatrix1D(p);
  }
  return z;
}
/* variant 0: fdkCallocMatrix2D, 1: _aligned, 2: _int, 3: _int_aligned. Returns -1 for NULL,
   else whether the memory is zero; rows[i] = (p[i] - p[0]) / size. */
INT cref_mc_2d(UINT dim1, UINT dim2, UINT size, INT variant, INT *rows) {
  void **p;
  switch (variant) {
    case 0:
      p = fdkCallocMatrix2D(dim1, dim2, size);
      break;
    case 1:
      p = fdkCallocMatrix2D_aligned(dim1, dim2, size);
      break;
    case 2:
      p = fdkCallocMatrix2D_int(dim1, dim2, size, SECT_DATA_L1);
      break;
    default:
      p = fdkCallocMatrix2D_int_aligned(dim1, dim2, size, SECT_DATA_L1);
      break;
  }
  if (p == NULL) return -1;
  for (UINT i = 0; i < dim1; i++) rows[i] = (INT)(((char *)p[i] - (char *)p[0]) / size);
  INT z = all_zero(p[0], dim1 * dim2 * size);
  if (variant == 1 || variant == 3) {
    fdkFreeMatrix2D_aligned(p);
  } else {
    fdkFreeMatrix2D(p);
  }
  return z;
}
/* variant 0: fdkCallocMatrix3D, 1: _int. Returns -1 for NULL, else whether the memory is zero;
   planes[i] = p[i] - p[0] (in pointers), rows[i * dim2 + j] = (p[i][j] - p[0][0]) / size. */
INT cref_mc_3d(UINT dim1, UINT dim2, UINT dim3, UINT size, INT variant, INT *planes, INT *rows) {
  void ***p;
  if (variant == 0) {
    p = fdkCallocMatrix3D(dim1, dim2, dim3, size);
  } else {
    p = fdkCallocMatrix3D_int(dim1, dim2, dim3, size, SECT_DATA_L1);
  }
  if (p == NULL) return -1;
  for (UINT i = 0; i < dim1; i++) {
    planes[i] = (INT)(p[i] - p[0]);
    for (UINT j = 0; j < dim2; j++) {
      rows[i * dim2 + j] = (INT)(((char *)p[i][j] - (char *)p[0][0]) / size);
    }
  }
  INT z = all_zero(p[0][0], dim1 * dim2 * dim3 * size);
  fdkFreeMatrix3D(p);
  return z;
}
}
