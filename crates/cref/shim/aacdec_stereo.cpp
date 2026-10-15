// Shims for the differential tests of fdk_aac_rs_aacdec::stereo.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_stereo (see build.rs).
//
// The functions on a context use the block test context of aacdec_block_ctx.cpp.

#include "cref_common.h"
#include "libAACdec/src/stereo.h"
#include "libAACdec/src/aac_rom.h"
#include "FDK_bitstream.h"
#include "libAACdec/src/channelinfo.h"
#include "FDK_audio.h"
#include "libAACdec/src/block.h"

/* The file-static CJointStereo_filterAndAdd() and CJointStereo_GenerateMSOutput(). */
namespace ref_stereo {
#include "libAACdec/src/stereo.cpp"
}

CAacDecoderChannelInfo *u10ctx_ci(void *c, int ch);
CAacDecoderStaticChannelInfo *u10ctx_sci(void *c, int ch);
CAacDecoderCommonData *u10ctx_com(void *c);
SamplingRateInfo *u10ctx_sri(void *c);
HANDLE_FDK_BITSTREAM u10ctx_bs(void *c);

extern "C" {
/* CJointStereo_filterAndAdd with row `row` of mdst_filt_coef_curr (curr = 1) or
   mdst_filt_coef_prev (curr = 0). */
void cref_stereo_filter_and_add(FIXP_DBL *in, INT len, INT windowLen, INT curr, INT row, FIXP_DBL *out, INT isCurrent) {
  const FIXP_FILT *coeff = curr ? mdst_filt_coef_curr[row] : mdst_filt_coef_prev[row];
  ref_stereo::CJointStereo_filterAndAdd(in, len, windowLen, coeff, out, (UCHAR)isCurrent);
}

void cref_stereo_generate_ms_output(FIXP_DBL *l, FIXP_DBL *r, UINT leftScale, UINT rightScale, UINT n) { ref_stereo::CJointStereo_GenerateMSOutput(l, r, leftScale, rightScale, n); }

/* CJointStereo_Read on the context: the persistent data of the CPE (usePersist) and the complex
   prediction data (useCplx) or NULL. */
INT cref_stereo_read(void *c, INT windowGroups, INT sfbt, INT clear, INT usePersist, INT useCplx, INT cplxActiv, INT sfbTotal, INT windowSequence, UINT flags) {
  CAacDecoderStaticChannelInfo *s = u10ctx_sci(c, 0);
  CAacDecoderChannelInfo *p = u10ctx_ci(c, 0);
  return CJointStereo_Read(u10ctx_bs(c), &u10ctx_com(c)->jointStereoData, windowGroups, sfbt, clear,
                           usePersist ? &s->pCpeStaticData->jointStereoPersistentData : NULL,
                           useCplx ? p->pComStaticData->cplxPredictionData : NULL, cplxActiv, sfbTotal,
                           windowSequence, flags);
}

/* CJointStereo_ApplyMS as CChannelElement_Decode() calls it, on the two channels. */
void cref_stereo_apply_ms(void *c, INT windowGroups, INT maxSfbSteOutside, INT sfbtL, INT sfbtR, INT mainband) {
  CAacDecoderChannelInfo *p[2] = {u10ctx_ci(c, 0), u10ctx_ci(c, 1)};
  CAacDecoderStaticChannelInfo *s[2] = {u10ctx_sci(c, 0), u10ctx_sci(c, 1)};
  CJointStereo_ApplyMS(p, s, p[0]->pSpectralCoefficient, p[1]->pSpectralCoefficient, p[0]->pDynData->aSfbScale,
                       p[1]->pDynData->aSfbScale, p[0]->specScale, p[1]->specScale,
                       GetScaleFactorBandOffsets(&p[0]->icsInfo, u10ctx_sri(c)),
                       GetWindowGroupLengthTable(&p[0]->icsInfo), windowGroups, maxSfbSteOutside, sfbtL, sfbtR,
                       p[0]->pComData->jointStereoData.store_dmx_re_prev,
                       &p[0]->pComData->jointStereoData.store_dmx_re_prev_e, mainband);
}

void cref_stereo_apply_is(void *c, INT windowGroups, INT sfbt) {
  CAacDecoderChannelInfo *p[2] = {u10ctx_ci(c, 0), u10ctx_ci(c, 1)};
  CJointStereo_ApplyIS(p, GetScaleFactorBandOffsets(&p[0]->icsInfo, u10ctx_sri(c)),
                       GetWindowGroupLengthTable(&p[0]->icsInfo), windowGroups, sfbt);
}
}
