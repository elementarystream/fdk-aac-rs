// Shims for the differential tests of fdk_aac_rs_pcmutils::pcm_utils.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::pcmutils_pcm_utils (see build.rs).

#include "cref_common.h"
#include "pcm_utils.h"

extern "C" {
void cref_pcm_utils_interleave_dbl_long(const INT *pIn, INT *pOut, UINT channels, UINT frameSize, UINT length) {
  FDK_interleave((const FIXP_DBL *)pIn, (LONG *)pOut, channels, frameSize, length);
}
void cref_pcm_utils_interleave_dbl_sgl(const INT *pIn, SHORT *pOut, UINT channels, UINT frameSize, UINT length) {
  FDK_interleave((const FIXP_DBL *)pIn, pOut, channels, frameSize, length);
}
void cref_pcm_utils_interleave_sgl_sgl(const SHORT *pIn, SHORT *pOut, UINT channels, UINT frameSize, UINT length) {
  FDK_interleave((const FIXP_SGL *)pIn, pOut, channels, frameSize, length);
}
void cref_pcm_utils_deinterleave_long_sgl(const INT *pIn, SHORT *pOut, UINT channels, UINT frameSize, UINT length) {
  FDK_deinterleave((const LONG *)pIn, pOut, channels, frameSize, length);
}
void cref_pcm_utils_deinterleave_long_long(const INT *pIn, INT *pOut, UINT channels, UINT frameSize, UINT length) {
  FDK_deinterleave((const LONG *)pIn, (LONG *)pOut, channels, frameSize, length);
}
void cref_pcm_utils_deinterleave_sgl_sgl(const SHORT *pIn, SHORT *pOut, UINT channels, UINT frameSize, UINT length) {
  FDK_deinterleave(pIn, pOut, channels, frameSize, length);
}
void cref_pcm_utils_deinterleave_sgl_long(const SHORT *pIn, INT *pOut, UINT channels, UINT frameSize, UINT length) {
  FDK_deinterleave(pIn, (LONG *)pOut, channels, frameSize, length);
}
}
