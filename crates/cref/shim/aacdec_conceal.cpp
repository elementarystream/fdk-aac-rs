// Shims for the differential tests of fdk_aac_rs_aacdec::conceal.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_conceal (see build.rs).
//
// The functions use the block test context of aacdec_block_ctx.cpp (its concealment parameters
// are the pConcealParams of both channels).

#include "cref_common.h"
#include "libAACdec/src/conceal.h"
#include "libAACdec/src/channelinfo.h"

CAacDecoderChannelInfo *u10ctx_ci(void *c, int ch);
CAacDecoderStaticChannelInfo *u10ctx_sci(void *c, int ch);
SamplingRateInfo *u10ctx_sri(void *c);
CConcealParams *u10ctx_params(void *c);
FIXP_DBL *u10ctx_out(void *c, int ch);

extern "C" {
void cref_conceal_init_common_data(void *c, INT isNull) { CConcealment_InitCommonData(isNull ? NULL : u10ctx_params(c)); }

INT cref_conceal_get_method(void *c, INT isNull) { return (INT)CConcealment_GetMethod(isNull ? NULL : u10ctx_params(c)); }

UINT cref_conceal_get_delay(void *c, INT isNull) { return CConcealment_GetDelay(isNull ? NULL : u10ctx_params(c)); }

INT cref_conceal_get_state(void *c, INT ch, INT isNull) { return (INT)CConcealment_GetState(isNull ? NULL : &u10ctx_sci(c, ch)->concealmentInfo); }

INT cref_conceal_get_last_frame_ok(void *c, INT ch, INT isNull, INT fBeforeApply) { return CConcealment_GetLastFrameOk(isNull ? NULL : &u10ctx_sci(c, ch)->concealmentInfo, fBeforeApply); }

INT cref_conceal_set_params(void *c, INT isNull, INT method, INT fadeOutSlope, INT fadeInSlope, INT muteRelease, FIXP_DBL comfNoiseLevel) {
  return (INT)CConcealment_SetParams(isNull ? NULL : u10ctx_params(c), method, fadeOutSlope, fadeInSlope, muteRelease,
                                     comfNoiseLevel);
}

INT cref_conceal_set_attenuation(void *c, INT isNull, const SHORT *fadeOut, const SHORT *fadeIn) {
  return (INT)CConcealment_SetAttenuation(isNull ? NULL : u10ctx_params(c), fadeOut, fadeIn);
}

void cref_conceal_init_channel_data(void *c, INT ch, INT renderMode, INT samplesPerFrame) {
  CConcealment_InitChannelData(&u10ctx_sci(c, ch)->concealmentInfo, u10ctx_params(c), (AACDEC_RENDER_MODE)renderMode,
                               samplesPerFrame);
}

void cref_conceal_store(void *c, INT ch) {
  CConcealment_Store(&u10ctx_sci(c, ch)->concealmentInfo, u10ctx_ci(c, ch), u10ctx_sci(c, ch));
}

INT cref_conceal_apply(void *c, INT ch, INT samplesPerFrame, INT lastLpdMode, INT frameOk, UINT flags) {
  CAacDecoderStaticChannelInfo *s = u10ctx_sci(c, ch);
  return CConcealment_Apply(&s->concealmentInfo, u10ctx_ci(c, ch), s, u10ctx_sri(c), samplesPerFrame,
                            (UCHAR)lastLpdMode, frameOk, flags);
}

INT cref_conceal_td_fading(void *c, INT ch, INT len, INT headroom) {
  CAacDecoderStaticChannelInfo *s = u10ctx_sci(c, ch);
  return CConcealment_TDFading(len, &s, headroom, u10ctx_out(c, ch), NULL);
}
}
