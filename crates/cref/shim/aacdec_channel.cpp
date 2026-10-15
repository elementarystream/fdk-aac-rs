// Shims for the differential tests of fdk_aac_rs_aacdec::channel.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_channel (see build.rs).
//
// The functions use the block test context of aacdec_block_ctx.cpp: its two channels are the
// channels of the element.

#include "cref_common.h"
#include "libAACdec/src/channel.h"
#include "libAACdec/src/aacdecoder.h"
#include "libAACdec/src/block.h"
#include "libAACdec/src/aacdec_pns.h"

CAacDecoderChannelInfo *u10ctx_ci(void *c, int ch);
CAacDecoderStaticChannelInfo *u10ctx_sci(void *c, int ch);
CAacDecoderCommonData *u10ctx_com(void *c);
SamplingRateInfo *u10ctx_sri(void *c);
FIXP_DBL *u10ctx_scratch(void *c);
HANDLE_FDK_BITSTREAM u10ctx_bs(void *c);

extern "C" {
/* CChannelElement_Read on the context (pTpDec NULL); staticNull passes NULL static channel
   infos (the CCE dummy channel of the decoder). */
INT cref_channel_element_read(void *c, INT aot, UINT flags, UINT elFlags, UINT frameLength, INT nch, INT epConfig, INT staticNull) {
  CAacDecoderChannelInfo *p[2] = {u10ctx_ci(c, 0), u10ctx_ci(c, 1)};
  CAacDecoderStaticChannelInfo *s[2] = {u10ctx_sci(c, 0), u10ctx_sci(c, 1)};
  return (INT)CChannelElement_Read(u10ctx_bs(c), p, staticNull ? NULL : s, (AUDIO_OBJECT_TYPE)aot, u10ctx_sri(c),
                                   flags, elFlags, frameLength, (UCHAR)nch, (SCHAR)epConfig, NULL);
}

/* CChannelElement_Decode on the context, with the pointers that the decoder sets before it:
   the joint stereo scratch buffer and, for AAC images, the PNS pointers. */
void cref_channel_element_decode(void *c, UINT flags, UINT elFlags, INT elChannels, INT usacImage) {
  CAacDecoderChannelInfo *p[2] = {u10ctx_ci(c, 0), u10ctx_ci(c, 1)};
  CAacDecoderStaticChannelInfo *s[2] = {u10ctx_sci(c, 0), u10ctx_sci(c, 1)};
  if (s[0]->pCpeStaticData != NULL) s[0]->pCpeStaticData->jointStereoPersistentData.scratchBuffer = u10ctx_scratch(c);
  if (!usacImage) {
    for (int ch = 0; ch < 2; ch++) {
      p[ch]->data.aac.PnsData.pPnsInterChannelData = &u10ctx_com(c)->pnsInterChannelData;
      CPns_UpdateNoiseState(&p[ch]->data.aac.PnsData, &s[ch]->pnsCurrentSeed, u10ctx_com(c)->pnsRandomSeed);
    }
  }
  CChannelElement_Decode(p, s, u10ctx_sri(c), flags, elFlags, elChannels);
}

void cref_channel_codebook_table_init(void *c, INT ch) { CChannel_CodebookTableInit(u10ctx_ci(c, ch)); }

/* Skips nbits of the bitstream of the context. */
void cref_channel_bs_skip(void *c, INT nbits) { FDKpushFor(u10ctx_bs(c), (UINT)nbits); }
}
