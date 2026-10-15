// Shims for the differential tests of fdk_aac_rs_aacdec::aacdec_pns.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_aacdec_pns (see build.rs).
//
// The functions on a context use the block test context of aacdec_block_ctx.cpp.

#include "cref_common.h"
#include "libAACdec/src/aacdec_pns.h"
#include "libAACdec/src/aac_ram.h"
#include "libAACdec/src/aac_rom.h"
#include "libAACdec/src/channelinfo.h"
#include "libAACdec/src/block.h"
#include "FDK_bitstream.h"

/* The file-static functions GenerateRandomVector() and ScaleBand(). CPns_IsPnsUsed is renamed
   in the copy (argument-dependent lookup would also find the library's declaration). */
#define CPns_IsPnsUsed ref_CPns_IsPnsUsed
namespace ref_pns {
#include "libAACdec/src/aacdec_pns.cpp"
}
#undef CPns_IsPnsUsed

CAacDecoderChannelInfo *u10ctx_ci(void *c, int ch);
CAacDecoderStaticChannelInfo *u10ctx_sci(void *c, int ch);
CAacDecoderCommonData *u10ctx_com(void *c);
SamplingRateInfo *u10ctx_sri(void *c);
HANDLE_FDK_BITSTREAM u10ctx_bs(void *c);

/* The pointers of the PNS data of channel ch, as CChannelElement_Decode() sets them. */
static void set_pointers(void *c, int ch) {
  CPnsData *p = &u10ctx_ci(c, ch)->data.aac.PnsData;
  p->pPnsInterChannelData = &u10ctx_com(c)->pnsInterChannelData;
  CPns_UpdateNoiseState(p, &u10ctx_sci(c, 0)->pnsCurrentSeed, u10ctx_com(c)->pnsRandomSeed);
}

extern "C" {
INT cref_pns_generate_random_vector(FIXP_DBL *spec, INT size, INT *state) { return ref_pns::GenerateRandomVector(spec, size, state); }

void cref_pns_scale_band(FIXP_DBL *spec, INT size, INT scaleFactor, INT specScale, INT noise_e, INT outOfPhase) { ref_pns::ScaleBand(spec, size, scaleFactor, specScale, noise_e, outOfPhase); }

void cref_pns_reset_data(void *c, INT ch) {
  CAacDecoderChannelInfo *p = u10ctx_ci(c, ch);
  CPns_ResetData(&p->data.aac.PnsData, &u10ctx_com(c)->pnsInterChannelData);
}

INT cref_pns_is_pns_used(void *c, INT ch, INT group, INT band) { return CPns_IsPnsUsed(&u10ctx_ci(c, ch)->data.aac.PnsData, group, band); }

void cref_pns_set_correlation(void *c, INT ch, INT group, INT band, INT outofphase) {
  set_pointers(c, ch);
  CPns_SetCorrelation(&u10ctx_ci(c, ch)->data.aac.PnsData, group, band, outofphase);
}

void cref_pns_read(void *c, INT ch, INT globalGain, INT band, INT group) {
  CAacDecoderChannelInfo *p = u10ctx_ci(c, ch);
  CPns_Read(&p->data.aac.PnsData, u10ctx_bs(c), &AACcodeBookDescriptionTable[BOOKSCL],
            p->pDynData->aScaleFactor, (UCHAR)globalGain, band, group);
}

void cref_pns_apply(void *c, INT ch) {
  CAacDecoderChannelInfo *p = u10ctx_ci(c, ch);
  set_pointers(c, ch);
  CPns_Apply(&p->data.aac.PnsData, &p->icsInfo, p->pSpectralCoefficient, p->specScale,
             p->pDynData->aScaleFactor, u10ctx_sri(c), p->granuleLength, ch);
}
}
