// Shims for the differential tests of fdk_aac_rs_fdk::cplx_mul.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_cplx_mul (see build.rs).
// Naming: cref_<function>_<type of a>_<type of b>, with D = FIXP_DBL, S = FIXP_SGL, SPK/DPK for
// the packed twiddles; a trailing _sgl marks the FIXP_SGL results.

#include "cref_common.h"
#include "common_fix.h"

static FIXP_SPK spk(SHORT re, SHORT im) {
  FIXP_SPK w;
  w.v.re = re;
  w.v.im = im;
  return w;
}

static FIXP_DPK dpk(FIXP_DBL re, FIXP_DBL im) {
  FIXP_DPK w;
  w.v.re = re;
  w.v.im = im;
  return w;
}

extern "C" {
void cref_cplxMultDiv2_D_D(FIXP_DBL *re, FIXP_DBL *im, FIXP_DBL a_re, FIXP_DBL a_im, FIXP_DBL b_re, FIXP_DBL b_im) {
  cplxMultDiv2(re, im, a_re, a_im, b_re, b_im);
}
void cref_cplxMultDiv2_D_DPK(FIXP_DBL *re, FIXP_DBL *im, FIXP_DBL a_re, FIXP_DBL a_im, FIXP_DBL w_re, FIXP_DBL w_im) {
  cplxMultDiv2(re, im, a_re, a_im, dpk(w_re, w_im));
}
void cref_cplxMultDiv2_D_S(FIXP_DBL *re, FIXP_DBL *im, FIXP_DBL a_re, FIXP_DBL a_im, FIXP_SGL b_re, FIXP_SGL b_im) {
  cplxMultDiv2(re, im, a_re, a_im, b_re, b_im);
}
void cref_cplxMultDiv2_D_SPK(FIXP_DBL *re, FIXP_DBL *im, FIXP_DBL a_re, FIXP_DBL a_im, FIXP_SGL w_re, FIXP_SGL w_im) {
  cplxMultDiv2(re, im, a_re, a_im, spk(w_re, w_im));
}
void cref_cplxMultDiv2_S_S(FIXP_DBL *re, FIXP_DBL *im, FIXP_SGL a_re, FIXP_SGL a_im, FIXP_SGL b_re, FIXP_SGL b_im) {
  cplxMultDiv2(re, im, a_re, a_im, b_re, b_im);
}
void cref_cplxMultDiv2_S_SPK(FIXP_DBL *re, FIXP_DBL *im, FIXP_SGL a_re, FIXP_SGL a_im, FIXP_SGL w_re, FIXP_SGL w_im) {
  cplxMultDiv2(re, im, a_re, a_im, spk(w_re, w_im));
}
void cref_cplxMultDiv2_S_S_sgl(FIXP_SGL *re, FIXP_SGL *im, FIXP_SGL a_re, FIXP_SGL a_im, FIXP_SGL b_re, FIXP_SGL b_im) {
  cplxMultDiv2(re, im, a_re, a_im, b_re, b_im);
}
void cref_cplxMultDiv2_S_SPK_sgl(FIXP_SGL *re, FIXP_SGL *im, FIXP_SGL a_re, FIXP_SGL a_im, FIXP_SGL w_re, FIXP_SGL w_im) {
  cplxMultDiv2(re, im, a_re, a_im, spk(w_re, w_im));
}
void cref_cplxMultSubDiv2_D_S(FIXP_DBL *re, FIXP_DBL *im, FIXP_DBL a_re, FIXP_DBL a_im, FIXP_SGL b_re, FIXP_SGL b_im) {
  cplxMultSubDiv2(re, im, a_re, a_im, b_re, b_im);
}
void cref_cplxMultSubDiv2_D_D(FIXP_DBL *re, FIXP_DBL *im, FIXP_DBL a_re, FIXP_DBL a_im, FIXP_DBL b_re, FIXP_DBL b_im) {
  cplxMultSubDiv2(re, im, a_re, a_im, b_re, b_im);
}
void cref_cplxMult_D_S(FIXP_DBL *re, FIXP_DBL *im, FIXP_DBL a_re, FIXP_DBL a_im, FIXP_SGL b_re, FIXP_SGL b_im) {
  cplxMult(re, im, a_re, a_im, b_re, b_im);
}
void cref_cplxMult_D_SPK(FIXP_DBL *re, FIXP_DBL *im, FIXP_DBL a_re, FIXP_DBL a_im, FIXP_SGL w_re, FIXP_SGL w_im) {
  cplxMult(re, im, a_re, a_im, spk(w_re, w_im));
}
void cref_cplxMult_D_D(FIXP_DBL *re, FIXP_DBL *im, FIXP_DBL a_re, FIXP_DBL a_im, FIXP_DBL b_re, FIXP_DBL b_im) {
  cplxMult(re, im, a_re, a_im, b_re, b_im);
}
void cref_cplxMult_D_DPK(FIXP_DBL *re, FIXP_DBL *im, FIXP_DBL a_re, FIXP_DBL a_im, FIXP_DBL w_re, FIXP_DBL w_im) {
  cplxMult(re, im, a_re, a_im, dpk(w_re, w_im));
}
void cref_cplxMult_S_S_sgl(FIXP_SGL *re, FIXP_SGL *im, FIXP_SGL a_re, FIXP_SGL a_im, FIXP_SGL b_re, FIXP_SGL b_im) {
  cplxMult(re, im, a_re, a_im, b_re, b_im);
}
}
