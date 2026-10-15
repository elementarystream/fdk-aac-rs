// Shims for the differential tests of fdk_aac_rs_aacdec::rvlc, rvlcbit and rvlcconceal.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_rvlc (see build.rs).
//
// The shims work on the context of aacdec_aacdec_hcr.cpp (two channels of an element, their
// static channel infos and the element's common data; the state is transferred with
// cref_aacdec_hcr_ctx_xfer). A bitstream is passed as a state array `st` (see fdk_bitstream.cpp)
// plus the memory `buf` of its bit buffer.
//
// The file-local functions of rvlc.cpp and rvlcconceal.cpp are reached by including the sources
// into the namespaces ref_rvlc and ref_rvlcc; the other functions are the library's.

#include "cref_common.h"
#include "FDK_bitstream.h"
#include "aacdecoder.h"
#include "channel.h"
#include "block.h"
#include "aac_rom.h"
#include "aacdec_hcr.h"
#include "rvlc_info.h"
#include "rvlc.h"
#include "rvlcbit.h"
#include "rvlcconceal.h"

namespace ref_rvlc {
#include "libAACdec/src/rvlc.cpp"
}
namespace ref_rvlcc {
#include "libAACdec/src/rvlcconceal.cpp"
}

extern "C" {
void *cref_aacdec_hcr_ctx_ch(void *ctx, INT ch);
void *cref_aacdec_hcr_ctx_st(void *ctx, INT ch);
}

static CAacDecoderChannelInfo *CH(void *ctx, INT ch) {
  return (CAacDecoderChannelInfo *)cref_aacdec_hcr_ctx_ch(ctx, ch);
}
static CAacDecoderStaticChannelInfo *ST(void *ctx, INT ch) {
  return (CAacDecoderStaticChannelInfo *)cref_aacdec_hcr_ctx_st(ctx, ch);
}
static CErRvlcInfo *RV(void *ctx) { return &CH(ctx, 0)->pComData->overlay.aac.erRvlcInfo; }

static void bs_load(FDK_BITSTREAM *bs, const UINT *st, UCHAR *buf) {
  bs->CacheWord = st[0];
  bs->BitsInCache = st[1];
  bs->ConfigCache = st[2];
  bs->hBitBuf.ValidBits = st[3];
  bs->hBitBuf.ReadOffset = st[4];
  bs->hBitBuf.WriteOffset = st[5];
  bs->hBitBuf.BitNdx = st[6];
  bs->hBitBuf.bufSize = st[7];
  bs->hBitBuf.bufBits = st[8];
  bs->hBitBuf.Buffer = buf;
}

static void bs_store(const FDK_BITSTREAM *bs, UINT *st) {
  st[0] = bs->CacheWord;
  st[1] = bs->BitsInCache;
  st[2] = bs->ConfigCache;
  st[3] = bs->hBitBuf.ValidBits;
  st[4] = bs->hBitBuf.ReadOffset;
  st[5] = bs->hBitBuf.WriteOffset;
  st[6] = bs->hBitBuf.BitNdx;
  st[7] = bs->hBitBuf.bufSize;
  st[8] = bs->hBitBuf.bufBits;
}

#define BS_BEGIN    \
  FDK_BITSTREAM bs; \
  bs_load(&bs, bsSt, buf)
#define BS_END bs_store(&bs, bsSt)

extern "C" {
/* ---- rvlc.h ------------------------------------------------------------------------------- */

void cref_rvlc_CRvlc_Read(void *ctx, INT ch, UINT *bsSt, UCHAR *buf) {
  BS_BEGIN;
  CRvlc_Read(CH(ctx, ch), &bs);
  BS_END;
}

void cref_rvlc_CRvlc_Decode(void *ctx, INT ch, UINT *bsSt, UCHAR *buf) {
  BS_BEGIN;
  CRvlc_Decode(CH(ctx, ch), ST(ctx, ch), &bs);
  BS_END;
}

/* CRvlc_ElementCheck on the two channels of the context (withStatic = 0: NULL static infos) */
void cref_rvlc_CRvlc_ElementCheck(void *ctx, INT withStatic, UINT flags, INT elChannels) {
  CAacDecoderChannelInfo *ch[2] = {CH(ctx, 0), CH(ctx, 1)};
  CAacDecoderStaticChannelInfo *st[2] = {ST(ctx, 0), ST(ctx, 1)};
  CRvlc_ElementCheck(ch, withStatic ? st : NULL, flags, elChannels);
}

/* The functions of rvlc.cpp that rvlc.h does not declare. */
void cref_rvlc_rvlcInit(void *ctx, INT ch, UINT *bsSt, UCHAR *buf) {
  BS_BEGIN;
  ref_rvlc::rvlcInit(RV(ctx), CH(ctx, ch), &bs);
  BS_END;
}
void cref_rvlc_rvlcCheckIntensityCb(void *ctx, INT ch) {
  ref_rvlc::rvlcCheckIntensityCb(RV(ctx), CH(ctx, ch));
}
INT cref_rvlc_rvlcDecodeEscapeWord(void *ctx, UINT *bsSt, UCHAR *buf) {
  BS_BEGIN;
  INT r = ref_rvlc::rvlcDecodeEscapeWord(RV(ctx), &bs);
  BS_END;
  return r;
}
void cref_rvlc_rvlcDecodeEscapes(void *ctx, UINT *bsSt, UCHAR *buf) {
  BS_BEGIN;
  ref_rvlc::rvlcDecodeEscapes(RV(ctx), CH(ctx, 0)->pComData->overlay.aac.aRvlcScfEsc, &bs);
  BS_END;
}
INT cref_rvlc_decodeRVLCodeword(void *ctx, UINT *bsSt, UCHAR *buf) {
  BS_BEGIN;
  INT r = ref_rvlc::decodeRVLCodeword(&bs, RV(ctx));
  BS_END;
  return r;
}
void cref_rvlc_rvlcDecodeForward(void *ctx, INT ch, UINT *bsSt, UCHAR *buf) {
  BS_BEGIN;
  ref_rvlc::rvlcDecodeForward(RV(ctx), CH(ctx, ch), &bs);
  BS_END;
}
void cref_rvlc_rvlcDecodeBackward(void *ctx, INT ch, UINT *bsSt, UCHAR *buf) {
  BS_BEGIN;
  ref_rvlc::rvlcDecodeBackward(RV(ctx), CH(ctx, ch), &bs);
  BS_END;
}
void cref_rvlc_rvlcFinalErrorDetection(void *ctx, INT ch) {
  ref_rvlc::rvlcFinalErrorDetection(CH(ctx, ch), ST(ctx, ch));
}

/* ---- rvlcbit.h ---------------------------------------------------------------------------- */

UINT cref_rvlcbit_rvlcReadBitFromBitstream(UINT *bsSt, UCHAR *buf, INT bsAnchor, INT *pos, UINT readDirection) {
  BS_BEGIN;
  UINT r = rvlcReadBitFromBitstream(&bs, bsAnchor, pos, (UCHAR)readDirection);
  BS_END;
  return r;
}

/* ---- rvlcconceal.h ------------------------------------------------------------------------ */

void cref_rvlcconceal_BidirectionalEstimation_UseLowerScfOfCurrentFrame(void *ctx, INT ch) {
  BidirectionalEstimation_UseLowerScfOfCurrentFrame(CH(ctx, ch));
}
void cref_rvlcconceal_BidirectionalEstimation_UseScfOfPrevFrameAsReference(void *ctx, INT ch) {
  BidirectionalEstimation_UseScfOfPrevFrameAsReference(CH(ctx, ch), ST(ctx, ch));
}
void cref_rvlcconceal_StatisticalEstimation(void *ctx, INT ch) {
  StatisticalEstimation(CH(ctx, ch));
}
void cref_rvlcconceal_PredictiveInterpolation(void *ctx, INT ch) {
  PredictiveInterpolation(CH(ctx, ch), ST(ctx, ch));
}
/* calcRefValFwd (dir = 0) or calcRefValBwd (dir = 1); out = {refIs, refNrg, refScf} */
void cref_rvlcconceal_calcRefVal(void *ctx, INT ch, INT dir, INT *out) {
  if (dir == 0)
    ref_rvlcc::calcRefValFwd(RV(ctx), CH(ctx, ch), &out[0], &out[1], &out[2]);
  else
    ref_rvlcc::calcRefValBwd(RV(ctx), CH(ctx, ch), &out[0], &out[1], &out[2]);
}
}
