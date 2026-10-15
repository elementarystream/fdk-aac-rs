// Shims for the differential tests of fdk_aac_rs_aacdec::block.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_block (see build.rs).
//
// The functions on a context use the block test context of aacdec_block_ctx.cpp.

#include "cref_common.h"
#include "libAACdec/src/block.h"
#include "libAACdec/src/aac_rom.h"
#include "FDK_bitstream.h"
#include "scale.h"
#include "FDK_tools_rom.h"
#include "libAACdec/src/usacdec_fac.h"
#include "libAACdec/src/usacdec_lpd.h"
#include "libAACdec/src/usacdec_lpc.h"
#include "FDK_trigFcts.h"
#include "ac_arith_coder.h"
#include "libAACdec/src/aacdec_hcr.h"
#include "libAACdec/src/rvlc.h"
#include "libAACdec/src/ldfiltbank.h"
#include "libAACdec/src/aacdec_pns.h"

/* The file-static functions of block.cpp (InverseQuantizeBand, maxabs_D, getWindow2Nr).
   CBlock_GetEscape is renamed in the copy: argument-dependent lookup would also find the
   library's declaration (same signature) at its call inside the copy. */
#define CBlock_GetEscape ref_CBlock_GetEscape
namespace ref_block {
#include "libAACdec/src/block.cpp"
}
#undef CBlock_GetEscape

/* The ARM scaling loop, also on x86 (it is plain C). */
namespace ref_block_arm {
#include "libAACdec/src/arm/block_arm.cpp"
}
#undef FUNCTION_CBlock_ScaleSpectralData_func1

/* The block test context (aacdec_block_ctx.cpp). */
CAacDecoderChannelInfo *u10ctx_ci(void *c, int ch);
CAacDecoderStaticChannelInfo *u10ctx_sci(void *c, int ch);
CAacDecoderCommonData *u10ctx_com(void *c);
SamplingRateInfo *u10ctx_sri(void *c);
FIXP_DBL *u10ctx_out(void *c, int ch);
HANDLE_FDK_BITSTREAM u10ctx_bs(void *c);

static void bs_init(FDK_BITSTREAM *bs, UCHAR *buf, INT bufBytes) {
  FDKinitBitStream(bs, buf, (UINT)bufBytes, (UINT)bufBytes * 8, BS_READER);
}

extern "C" {
/* ---- standalone functions ---- */

/* CBlock_GetEscape on a bitstream over buf (bufBytes: a power of two). */
INT cref_block_get_escape(UCHAR *buf, INT bufBytes, INT q, INT *bitsLeft) {
  FDK_BITSTREAM bs;
  bs_init(&bs, buf, bufBytes);
  INT r = (INT)CBlock_GetEscape(&bs, q);
  *bitsLeft = (INT)FDKgetValidBits(&bs);
  return r;
}

/* CBlock_DecodeHuffmanWord (which = 0) or CBlock_DecodeHuffmanWordCB (which = 1) with
   AACcodeBookDescriptionTable[cb]. */
INT cref_block_decode_huffman_word(INT which, UCHAR *buf, INT bufBytes, INT cb, INT *bitsLeft) {
  FDK_BITSTREAM bs;
  bs_init(&bs, buf, bufBytes);
  INT r = which == 0 ? CBlock_DecodeHuffmanWord(&bs, &AACcodeBookDescriptionTable[cb])
                     : CBlock_DecodeHuffmanWordCB(&bs, AACcodeBookDescriptionTable[cb].CodeBook);
  *bitsLeft = (INT)FDKgetValidBits(&bs);
  return r;
}

INT cref_block_evaluate_power43(FIXP_DBL *v, UINT lsb) { return EvaluatePower43(v, lsb); }
INT cref_block_get_scale_from_value(FIXP_DBL v, UINT lsb) { return GetScaleFromValue(v, lsb); }
FIXP_DBL cref_block_get_gain(const FIXP_DBL *x, const FIXP_DBL *y, INT n) { return get_gain(x, y, n); }
INT cref_block_get_window2_nr(INT length, INT shape) { return ref_block::getWindow2Nr(length, shape); }

void cref_block_inverse_quantize_band(FIXP_DBL *spectrum, UINT lsb, INT noLines, INT scale) {
  ref_block::InverseQuantizeBand(spectrum, InverseQuantTable, MantissaTable[lsb], ExponentTable[lsb],
                                 noLines, scale);
}
FIXP_DBL cref_block_maxabs_d(const FIXP_DBL *spec, INT noLines) { return ref_block::maxabs_D(spec, noLines); }

/* CBlock_ScaleSpectralData_func1 of arm/block_arm.cpp. */
void cref_block_scale_func1(FIXP_DBL *pSpectrum, INT maxSfbs, const SHORT *BandOffsets, INT SpecScale_window, const SHORT *pSfbScale, INT window) {
  ref_block_arm::CBlock_ScaleSpectralData_func1(pSpectrum, maxSfbs, BandOffsets, SpecScale_window, pSfbScale,
                                                window);
}

/* ---- functions on the context ---- */

INT cref_block_read_section_data(void *c, INT ch, UINT flags) {
  return (INT)CBlock_ReadSectionData(u10ctx_bs(c), u10ctx_ci(c, ch), u10ctx_sri(c), flags);
}

INT cref_block_read_scale_factor_data(void *c, INT ch, UINT flags) {
  /* the PNS pointer of CPnsData is not used by CPns_Read() */
  return (INT)CBlock_ReadScaleFactorData(u10ctx_ci(c, ch), u10ctx_bs(c), flags);
}

INT cref_block_read_spectral_data(void *c, INT ch, UINT flags) {
  return (INT)CBlock_ReadSpectralData(u10ctx_bs(c), u10ctx_ci(c, ch), u10ctx_sri(c), flags);
}

INT cref_block_inverse_quantize(void *c, INT ch, INT active) {
  CAacDecoderChannelInfo *p = u10ctx_ci(c, ch);
  return (INT)CBlock_InverseQuantizeSpectralData(p, u10ctx_sri(c), p->pDynData->band_is_noise,
                                                 (UCHAR)active);
}

void cref_block_scale_spectral_data(void *c, INT ch, INT maxSfbs) {
  CBlock_ScaleSpectralData(u10ctx_ci(c, ch), (UCHAR)maxSfbs, u10ctx_sri(c));
}

void cref_block_apply_noise(void *c, INT ch) {
  CAacDecoderChannelInfo *p = u10ctx_ci(c, ch);
  CBlock_ApplyNoise(p, u10ctx_sri(c), &u10ctx_sci(c, ch)->nfRandomSeed, p->pDynData->band_is_noise);
}

INT cref_block_read_ac_spectral_data(void *c, INT ch, UINT frameLength, UINT flags) {
  return (INT)CBlock_ReadAcSpectralData(u10ctx_bs(c), u10ctx_ci(c, ch), u10ctx_sci(c, ch), u10ctx_sri(c),
                                        frameLength, flags);
}

/* ApplyTools on the two channels of the context; the PNS pointers are set as
   CChannelElement_Decode() sets them (AAC images only: the pointers overlap the USAC members). */
void cref_block_apply_tools(void *c, UINT flags, UINT elFlags, INT channel, INT commonWindow, INT usacImage) {
  CAacDecoderChannelInfo *p[2] = {u10ctx_ci(c, 0), u10ctx_ci(c, 1)};
  if (!usacImage) {
    for (int ch = 0; ch < 2; ch++) {
      p[ch]->data.aac.PnsData.pPnsInterChannelData = &u10ctx_com(c)->pnsInterChannelData;
      CPns_UpdateNoiseState(&p[ch]->data.aac.PnsData, &u10ctx_sci(c, 0)->pnsCurrentSeed,
                            u10ctx_com(c)->pnsRandomSeed);
    }
  }
  ApplyTools(p, u10ctx_sri(c), flags, elFlags, channel, commonWindow);
}

void cref_block_frequency_to_time(void *c, INT ch, INT frameLen, INT frameOk, INT headroom, UINT elFlags, INT elCh) {
  CAacDecoderChannelInfo *p = u10ctx_ci(c, ch);
  CBlock_FrequencyToTime(u10ctx_sci(c, ch), p, u10ctx_out(c, ch), (SHORT)frameLen, frameOk,
                         p->pComStaticData->pWorkBufferCore1->mdctOutTemp, headroom, elFlags, elCh);
}

/* CLpd_RenderTimeSignal() on a channel of the context (the LPD render mode of the decoder; the
   work buffer is pComStaticData->pWorkBufferCore1 of the context). */
INT cref_block_lpd_render_time_signal(void *c, INT ch, INT samplesPerFrame, UINT frameOk, INT headroom, UINT flags, UINT strmFlags) {
  return (INT)CLpd_RenderTimeSignal(u10ctx_sci(c, ch), u10ctx_ci(c, ch), u10ctx_out(c, ch), samplesPerFrame,
                                    u10ctx_sri(c), frameOk, headroom, flags, strmFlags);
}

void cref_block_frequency_to_time_ld(void *c, INT ch, INT frameLen) {
  CBlock_FrequencyToTimeLowDelay(u10ctx_sci(c, ch), u10ctx_ci(c, ch), u10ctx_out(c, ch), (short)frameLen);
}
}

extern "C" {
/* The file-static table `name` of the copy (ref_block) as memory: noise_level_tab.
   Returns NULL for another name. */
const void *cref_block_table(const char *name, INT *bytes) {
  if (strcmp(name, "noise_level_tab") == 0) {
    *bytes = (INT)sizeof(ref_block::noise_level_tab);
    return ref_block::noise_level_tab;
  }
  *bytes = 0;
  return NULL;
}
}
