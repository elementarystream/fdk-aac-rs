// Shims for the differential tests of fdk_aac_rs_fdk::archdef.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_archdef (see build.rs).

#include "cref_common.h"
#include "common_fix.h"

extern "C" {
FIXP_SGL cref_STC(FIXP_DBL a) { return STC(a); }
FIXP_SGL cref_WTC(FIXP_DBL a) { return WTC(a); }
FIXP_SGL cref_FX_DBL2FX_WTB(FIXP_DBL a) { return FX_DBL2FX_WTB(a); }
INT cref_STCP(FIXP_DBL c, FIXP_DBL s, SHORT *re, SHORT *im) {
  const FIXP_STP w = STCP(c, s);
  *re = w.v.re;
  *im = w.v.im;
  return (INT)sizeof(FIXP_STP);
}
INT cref_WTCP(FIXP_DBL a, FIXP_DBL b, SHORT *re, SHORT *im) {
  const FIXP_WTP w = WTCP(a, b);
  *re = w.v.re;
  *im = w.v.im;
  return (INT)sizeof(FIXP_WTP);
}
INT cref_sizeof_FIXP_STB(void) { return (INT)sizeof(FIXP_STB); }
INT cref_sizeof_FIXP_WTB(void) { return (INT)sizeof(FIXP_WTB); }
/* The configuration macros of FDK_archdef.h. */
INT cref_archdef_flags(void) {
  INT f = 0;
#ifdef ARCH_PREFER_MULT_32x16
  f |= 1;
#endif
#ifdef SINETABLE_16BIT
  f |= 2;
#endif
#ifdef POW2COEFF_16BIT
  f |= 4;
#endif
#ifdef LDCOEFF_16BIT
  f |= 8;
#endif
#ifdef WINDOWTABLE_16BIT
  f |= 16;
#endif
#if defined(__arm__) && defined(__ARM_ARCH_8__)
  f |= 32;
#endif
#ifdef __x86__
  f |= 64;
#endif
  return f;
}
}
