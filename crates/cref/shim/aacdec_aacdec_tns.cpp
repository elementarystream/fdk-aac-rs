// Shims for the differential tests of fdk_aac_rs_aacdec::aacdec_tns.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_aacdec_tns (see build.rs).
//
// The functions use the block test context of aacdec_block_ctx.cpp.

#include "cref_common.h"
#include "libAACdec/src/aacdec_tns.h"
#include "libAACdec/src/channelinfo.h"
#include "libAACdec/src/block.h"

CAacDecoderChannelInfo *u10ctx_ci(void *c, int ch);
SamplingRateInfo *u10ctx_sri(void *c);
HANDLE_FDK_BITSTREAM u10ctx_bs(void *c);

extern "C" {
void cref_tns_reset(void *c, INT ch) { CTns_Reset(&u10ctx_ci(c, ch)->pDynData->TnsData); }

void cref_tns_read_data_present_flag(void *c, INT ch) { CTns_ReadDataPresentFlag(u10ctx_bs(c), &u10ctx_ci(c, ch)->pDynData->TnsData); }

INT cref_tns_read(void *c, INT ch, UINT flags) {
  CAacDecoderChannelInfo *p = u10ctx_ci(c, ch);
  return (INT)CTns_Read(u10ctx_bs(c), &p->pDynData->TnsData, &p->icsInfo, flags);
}

/* CTns_ReadDataPresentUsac on the two channels (tns_on_lr of channel 0, its CIcsInfo). */
void cref_tns_read_data_present_usac(void *c, UINT flags, UINT elFlags, INT commonWindow) {
  CAacDecoderChannelInfo *p0 = u10ctx_ci(c, 0), *p1 = u10ctx_ci(c, 1);
  CTns_ReadDataPresentUsac(u10ctx_bs(c), &p0->pDynData->TnsData, &p1->pDynData->TnsData,
                           &p0->pDynData->specificTo.usac.tns_on_lr, &p0->icsInfo, flags, elFlags,
                           commonWindow);
}

void cref_tns_apply(void *c, INT ch, INT nbands, INT igf, UINT flags) {
  CAacDecoderChannelInfo *p = u10ctx_ci(c, ch);
  CTns_Apply(&p->pDynData->TnsData, &p->icsInfo, p->pSpectralCoefficient, u10ctx_sri(c), p->granuleLength,
             (UCHAR)nbands, (UCHAR)igf, flags);
}
}
