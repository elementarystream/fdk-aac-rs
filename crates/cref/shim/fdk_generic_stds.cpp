// Shims for the differential tests of fdk_aac_rs_fdk::generic_stds.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_generic_stds (see build.rs).

#include "cref_common.h"

extern "C" {
void cref_FDKmemcpy(UCHAR *dst, const UCHAR *src, UINT size) { FDKmemcpy(dst, src, size); }
void cref_FDKmemmove(UCHAR *dst, const UCHAR *src, UINT size) { FDKmemmove(dst, src, size); }
void cref_FDKmemclear(UCHAR *dst, UINT size) { FDKmemclear(dst, size); }
void cref_FDKmemset(UCHAR *dst, INT value, UINT size) { FDKmemset(dst, value, size); }
INT cref_FDKmemcmp(const UCHAR *s1, const UCHAR *s2, UINT size) { return FDKmemcmp(s1, s2, size); }
INT cref_FDKstrcmp(const char *s1, const char *s2) { return FDKstrcmp(s1, s2); }
INT cref_FDKstrncmp(const char *s1, const char *s2, UINT size) { return FDKstrncmp(s1, s2, size); }
UINT cref_FDKstrlen(const char *s) { return FDKstrlen(s); }
INT cref_IS_LITTLE_ENDIAN(void) { return IS_LITTLE_ENDIAN(); }
UINT cref_TO_LITTLE_ENDIAN(UINT v) { return TO_LITTLE_ENDIAN(v); }
INT cref_FDKmax(INT a, INT b) { return FDKmax(a, b); }
INT cref_FDKmin(INT a, INT b) { return FDKmin(a, b); }

/* Constants (index order of crates/cref/tests/fdk/generic_stds.rs). */
INT64 cref_generic_stds_const(INT i) {
  static const INT64 v[] = {SECT_DATA_L1, SECT_DATA_L2, SECT_DATA_L1_A, SECT_DATA_L1_B,
                            SECT_CONSTDATA_L1, SECT_DATA_EXTERN, SECT_CONSTDATA_EXTERN,
                            FDK_MAX_OVERLAYS, FDK_INT_MAX, FDK_INT_MIN, FDK_EXITCODE_OK,
                            FDK_EXITCODE_USAGE, FDK_EXITCODE_DATAERROR, FDK_EXITCODE_NOINPUT,
                            FDK_EXITCODE_UNAVAILABLE, FDK_EXITCODE_SOFTWARE,
                            FDK_EXITCODE_CANTCREATE, FDK_EXITCODE_IOERROR};
  return v[i];
}
double cref_M_PI(void) { return M_PI; }
}
