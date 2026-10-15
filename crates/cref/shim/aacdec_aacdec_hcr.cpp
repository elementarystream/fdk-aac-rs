// Shims for the differential tests of fdk_aac_rs_aacdec::aacdec_hcr, aacdec_hcrs and
// aacdec_hcr_bit. Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_aacdec_hcr (see build.rs).
//
// The shims work on a context: two channels of one element (CAacDecoderChannelInfo with their
// pDynData in the element's CAacDecoderCommonData, as in the decoder), their static channel infos
// and a spectrum of 2 * 1024 values (channel ch at ch * 1024, as in workBufferCore2). The state
// is transferred as flat INT arrays (layouts below; the Rust side visits the fields in the same
// order). aacdec_rvlc.cpp and aacdec_aacdec_hcr_stream.cpp use the context and the transfer
// functions through the exported cref_aacdec_hcr_* functions.
//
// A bitstream is passed as a state array `st` (see fdk_bitstream.cpp) plus the memory `buf` of its
// bit buffer.
//
// The file-local functions of aacdec_hcr.cpp and aacdec_hcrs.cpp are reached by including the
// sources into the namespaces ref_hcr and ref_hcrs; the other functions are the library's.

#include "cref_common.h"
#include "FDK_bitstream.h"
#include "aacdec_hcr_types.h"
#include "aacdec_hcr.h"
#include "aacdec_hcr_bit.h"
#include "aacdec_hcrs.h"
#include "aac_ram.h"
#include "aac_rom.h"
#include "channel.h"
#include "block.h"
#include "aacdecoder.h"
#include <stdlib.h>
#include <stddef.h>

namespace ref_hcr {
#include "libAACdec/src/aacdec_hcr.cpp"
}
namespace ref_hcrs {
#include "libAACdec/src/aacdec_hcrs.cpp"
}

/* ---- flat transfer ------------------------------------------------------------------------ */

struct Fl {
  INT *v;
  int pos;
  int load;
};

template <class T>
static void s(Fl &f, T &x) {
  if (f.load)
    x = (T)f.v[f.pos];
  else
    f.v[f.pos] = (INT)x;
  f.pos++;
}

template <class T>
static void a(Fl &f, T *x, int n) {
  for (int i = 0; i < n; i++) s(f, x[i]);
}

/* CH: the fields of a channel that HCR and RVLC use. */
static void ch_xfer(Fl &f, CAacDecoderChannelInfo *p) {
  CIcsInfo *ics = &p->icsInfo;
  a(f, ics->WindowGroupLength, 8);
  s(f, ics->WindowGroups);
  s(f, ics->Valid);
  s(f, ics->WindowShape);
  s(f, ics->WindowSequence);
  s(f, ics->MaxSfBands);
  s(f, ics->max_sfb_ste);
  s(f, ics->ScaleFactorGrouping);
  s(f, ics->TotalSfBands);
  s(f, p->granuleLength);
  CAacDecoderDynamicData *d = p->pDynData;
  a(f, d->aScaleFactor, 8 * 16);
  a(f, d->aSfbScale, 8 * 16);
  a(f, d->aCodeBook, 8 * 16);
  s(f, d->RawDataInfo.GlobalGain);
  s(f, d->RawDataInfo.CommonWindow);
  a(f, d->specificTo.aac.aNumLineInSec4Hcr, MAX_SFB_HCR);
  a(f, d->specificTo.aac.aCodeBooks4Hcr, MAX_SFB_HCR);
  s(f, d->specificTo.aac.lenOfReorderedSpectralData);
  s(f, d->specificTo.aac.lenOfLongestCodeword);
  s(f, d->specificTo.aac.numberSection);
  s(f, d->specificTo.aac.rvlcCurrentScaleFactorOK);
  s(f, d->specificTo.aac.rvlcIntensityUsed);
  a(f, p->data.aac.PnsData.pnsUsed, NO_OFBANDS);
  s(f, p->data.aac.PnsData.PnsActive);
}

/* HCR: CErHcrInfo without its pointers; pState as its index in aStateConstant2State (0 for
   NULL, -1 for an unknown pointer). */
static void hcr_xfer(Fl &f, CErHcrInfo *h) {
  HCR_INPUT_OUTPUT *io = &h->decInOut;
  s(f, io->errorLog);
  s(f, io->quantizedSpectralCoefficientsIdx);
  s(f, io->lengthOfReorderedSpectralData);
  s(f, io->numSection);
  s(f, io->bitstreamAnchor);
  s(f, io->lengthOfLongestCodeword);
  HCR_SEGMENT_INFO *sg = &h->segmentInfo;
  s(f, sg->numSegment);
  a(f, sg->pSegmentBitfield, (1024 >> 1) / NUMBER_OF_BIT_IN_WORD + 1);
  a(f, sg->pCodewordBitfield, (1024 >> 1) / NUMBER_OF_BIT_IN_WORD + 1);
  s(f, sg->segmentOffset);
  a(f, sg->pLeftStartOfSegment, 1024 >> 1);
  a(f, sg->pRightStartOfSegment, 1024 >> 1);
  a(f, sg->pRemainingBitsInSegment, 1024 >> 1);
  s(f, sg->readDirection);
  s(f, sg->numWordForBitfield);
  s(f, sg->pNumBitValidInLastWord);
  HCR_SECTION_INFO *sc = &h->sectionInfo;
  s(f, sc->numCodeword);
  s(f, sc->numSortedSection);
  a(f, sc->pNumCodewordInSection, MAX_SFB_HCR);
  a(f, sc->pNumSortedCodewordInSection, MAX_SFB_HCR);
  a(f, sc->pNumExtendedSortedCodewordInSection, MAX_SFB_HCR + MAX_HCR_SETS);
  s(f, sc->numExtendedSortedCodewordInSectionIdx);
  a(f, sc->pNumExtendedSortedSectionsInSets, MAX_HCR_SETS);
  s(f, sc->numExtendedSortedSectionsInSetsIdx);
  a(f, sc->pReorderOffset, MAX_SFB_HCR);
  a(f, sc->pSortedCodebook, MAX_SFB_HCR);
  a(f, sc->pExtendedSortedCodebook, MAX_SFB_HCR + MAX_HCR_SETS);
  s(f, sc->extendedSortedCodebookIdx);
  a(f, sc->pMaxLenOfCbInExtSrtSec, MAX_SFB_HCR + MAX_HCR_SETS);
  s(f, sc->maxLenOfCbInExtSrtSecIdx);
  a(f, sc->pCodebookSwitch, MAX_SFB_HCR);
  HCR_NON_PCW_SIDEINFO *np = &h->nonPcwSideinfo;
  a(f, np->iNode, 1024 >> 2);
  a(f, np->iResultPointer, 1024 >> 2);
  a(f, np->pEscapeSequenceInfo, 1024 >> 2);
  s(f, np->codewordOffset);
  if (f.load) {
    INT k = f.v[f.pos];
    np->pState = (k >= 0 && k < 8) ? aStateConstant2State[k] : NULL;
  } else {
    INT k = -1;
    for (int i = 0; i < 8; i++)
      if (np->pState == aStateConstant2State[i]) k = i;
    f.v[f.pos] = k;
  }
  f.pos++;
  a(f, np->pCodebook, 1024 >> 2);
  a(f, np->pCntSign, 1024 >> 2);
  a(f, np->pSta, 1024 >> 2);
}

/* RV: CErRvlcInfo (pointers as selectors), the three scale factor arrays of the overlay and
   jointStereoData.MsMaskPresent of the element. */
static void rv_xfer(Fl &f, CAacDecoderCommonData *c) {
  CErRvlcInfo *r = &c->overlay.aac.erRvlcInfo;
  s(f, r->sf_concealment);
  s(f, r->rev_global_gain);
  s(f, r->length_of_rvlc_sf);
  s(f, r->dpcm_noise_nrg);
  s(f, r->sf_escapes_present);
  s(f, r->length_of_rvlc_escapes);
  s(f, r->dpcm_noise_last_position);
  s(f, r->dpcm_is_last_position);
  s(f, r->length_of_rvlc_sf_fwd);
  s(f, r->length_of_rvlc_sf_bwd);
  if (f.load) {
    r->pRvlBitCnt_RVL = f.v[f.pos] == 1 ? &r->length_of_rvlc_sf_bwd : &r->length_of_rvlc_sf_fwd;
    r->pBitstrIndxRvl_RVL =
        f.v[f.pos + 1] == 1 ? &r->bitstreamIndexRvlBwd : &r->bitstreamIndexRvlFwd;
  } else {
    /* NULL (never dereferenced before it is set) is the forward selector */
    f.v[f.pos] = r->pRvlBitCnt_RVL == &r->length_of_rvlc_sf_bwd
                     ? 1
                     : ((r->pRvlBitCnt_RVL == NULL ||
                         r->pRvlBitCnt_RVL == &r->length_of_rvlc_sf_fwd)
                            ? 0
                            : -1);
    f.v[f.pos + 1] = r->pBitstrIndxRvl_RVL == &r->bitstreamIndexRvlBwd
                         ? 1
                         : ((r->pBitstrIndxRvl_RVL == NULL ||
                             r->pBitstrIndxRvl_RVL == &r->bitstreamIndexRvlFwd)
                                ? 0
                                : -1);
  }
  f.pos += 2;
  s(f, r->numWindowGroups);
  s(f, r->maxSfbTransmitted);
  s(f, r->first_noise_group);
  s(f, r->first_noise_band);
  s(f, r->direction);
  s(f, r->bsAnchor);
  s(f, r->bitstreamIndexRvlFwd);
  s(f, r->bitstreamIndexRvlBwd);
  s(f, r->bitstreamIndexEsc);
  if (f.load) {
    r->pHuffTreeRvlCodewds = f.v[f.pos] ? aHuffTreeRvlCodewds : NULL;
    r->pHuffTreeRvlcEscape = f.v[f.pos + 1] ? aHuffTreeRvlcEscape : NULL;
  } else {
    f.v[f.pos] = r->pHuffTreeRvlCodewds == NULL
                     ? 0
                     : (r->pHuffTreeRvlCodewds == aHuffTreeRvlCodewds ? 1 : -1);
    f.v[f.pos + 1] = r->pHuffTreeRvlcEscape == NULL
                         ? 0
                         : (r->pHuffTreeRvlcEscape == aHuffTreeRvlcEscape ? 1 : -1);
  }
  f.pos += 2;
  s(f, r->numDecodedEscapeWordsFwd);
  s(f, r->numDecodedEscapeWordsBwd);
  s(f, r->numDecodedEscapeWordsEsc);
  s(f, r->noise_used);
  s(f, r->intensity_used);
  s(f, r->sf_used);
  s(f, r->firstScf);
  s(f, r->lastScf);
  s(f, r->firstNrg);
  s(f, r->lastNrg);
  s(f, r->firstIs);
  s(f, r->lastIs);
  s(f, r->errorLogRvlc);
  s(f, r->conceal_min);
  s(f, r->conceal_max);
  s(f, r->conceal_min_esc);
  s(f, r->conceal_max_esc);
  a(f, c->overlay.aac.aRvlcScfEsc, RVLC_MAX_SFB);
  a(f, c->overlay.aac.aRvlcScfFwd, RVLC_MAX_SFB);
  a(f, c->overlay.aac.aRvlcScfBwd, RVLC_MAX_SFB);
  s(f, c->jointStereoData.MsMaskPresent);
}

/* ST: the RVLC fields of the concealment info. */
static void st_xfer(Fl &f, CAacDecoderStaticChannelInfo *p) {
  a(f, p->concealmentInfo.aRvlcPreviousScaleFactor, RVLC_MAX_SFB);
  a(f, p->concealmentInfo.aRvlcPreviousCodebook, RVLC_MAX_SFB);
  s(f, p->concealmentInfo.rvlcPreviousScaleFactorOK);
  s(f, p->concealmentInfo.rvlcPreviousBlockType);
}

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

/* ---- context ------------------------------------------------------------------------------ */

struct Ctx {
  CAacDecoderChannelInfo ch[2];
  CAacDecoderStaticChannelInfo st[2];
  CAacDecoderCommonData com;
  FIXP_DBL spec[2 * 1024];
};

static Ctx *ctx_of(void *p) { return (Ctx *)p; }

/* The pointers HcrInit() sets, for channel ch of the context. */
static CErHcrInfo *hcr_point(Ctx *c, INT ch) {
  CErHcrInfo *h = &c->com.overlay.aac.erHcrInfo;
  ch &= 1;
  h->decInOut.pQuantizedSpectralCoefficientsBase = c->ch[ch].pSpectralCoefficient;
  h->decInOut.pCodebook = c->ch[ch].pDynData->specificTo.aac.aCodeBooks4Hcr;
  h->decInOut.pNumLineInSect = c->ch[ch].pDynData->specificTo.aac.aNumLineInSec4Hcr;
  h->nonPcwSideinfo.pResultBase = SPEC_LONG(c->ch[ch].pSpectralCoefficient);
  return h;
}

static void sri(SamplingRateInfo *t, UINT spf, UINT sri, UINT sr) {
  FDKmemclear(t, sizeof(*t));
  getSamplingRateInfo(t, spf, sri, sr);
}

extern "C" {
/* The lengths of the flat arrays (CH, HCR, RV, ST) and layout facts:
   [4] offsetof(lenOfReorderedSpectralData) - offsetof(aCodeBooks4Hcr) in specificTo.aac. */
INT cref_aacdec_hcr_layout(INT *out) {
  Ctx *c = (Ctx *)calloc(1, sizeof(Ctx));
  c->ch[0].pDynData = &c->com.pAacDecoderDynamicData[0];
  Fl f;
  INT *tmp = (INT *)calloc(16384, sizeof(INT));
  f = {tmp, 0, 0};
  ch_xfer(f, &c->ch[0]);
  out[0] = f.pos;
  f = {tmp, 0, 0};
  hcr_xfer(f, &c->com.overlay.aac.erHcrInfo);
  out[1] = f.pos;
  f = {tmp, 0, 0};
  rv_xfer(f, &c->com);
  out[2] = f.pos;
  f = {tmp, 0, 0};
  st_xfer(f, &c->st[0]);
  out[3] = f.pos;
  out[4] = (INT)(offsetof(CAacDecoderDynamicData, specificTo.aac.lenOfReorderedSpectralData) -
                 offsetof(CAacDecoderDynamicData, specificTo.aac.aCodeBooks4Hcr));
  free(tmp);
  free(c);
  return 5;
}

void *cref_aacdec_hcr_ctx_new(void) {
  Ctx *c = (Ctx *)calloc(1, sizeof(Ctx));
  for (int i = 0; i < 2; i++) {
    c->ch[i].pDynData = &c->com.pAacDecoderDynamicData[i];
    c->ch[i].pComData = &c->com;
    c->ch[i].pSpectralCoefficient = (SPECTRAL_PTR)&c->spec[i * 1024];
  }
  return c;
}

void cref_aacdec_hcr_ctx_free(void *ctx) { free(ctx); }

void *cref_aacdec_hcr_ctx_ch(void *ctx, INT ch) { return &ctx_of(ctx)->ch[ch & 1]; }

void *cref_aacdec_hcr_ctx_st(void *ctx, INT ch) { return &ctx_of(ctx)->st[ch & 1]; }

/* Transfer of the channel `chp` (CAacDecoderChannelInfo *), the HCR info, the RVLC part of the
   common data `comp` and the static channel info `stp`; load != 0 writes the structures. */
void cref_aacdec_hcr_xfer_ch(void *chp, INT *v, INT load) {
  Fl f = {v, 0, load};
  ch_xfer(f, (CAacDecoderChannelInfo *)chp);
}
void cref_aacdec_hcr_xfer_hcr(void *hcrp, INT *v, INT load) {
  Fl f = {v, 0, load};
  hcr_xfer(f, (CErHcrInfo *)hcrp);
}
void cref_aacdec_hcr_xfer_rv(void *comp, INT *v, INT load) {
  Fl f = {v, 0, load};
  rv_xfer(f, (CAacDecoderCommonData *)comp);
}
void cref_aacdec_hcr_xfer_st(void *stp, INT *v, INT load) {
  Fl f = {v, 0, load};
  st_xfer(f, (CAacDecoderStaticChannelInfo *)stp);
}

/* Sets (load = 1) or gets (load = 0) the whole context; NULL arrays are skipped. */
void cref_aacdec_hcr_ctx_xfer(void *ctx, INT *ch0, INT *ch1, INT *hcr, INT *rv, INT *st0, INT *st1, INT *spec, INT load) {
  Ctx *c = ctx_of(ctx);
  if (ch0) cref_aacdec_hcr_xfer_ch(&c->ch[0], ch0, load);
  if (ch1) cref_aacdec_hcr_xfer_ch(&c->ch[1], ch1, load);
  if (hcr) cref_aacdec_hcr_xfer_hcr(&c->com.overlay.aac.erHcrInfo, hcr, load);
  if (rv) cref_aacdec_hcr_xfer_rv(&c->com, rv, load);
  if (st0) cref_aacdec_hcr_xfer_st(&c->st[0], st0, load);
  if (st1) cref_aacdec_hcr_xfer_st(&c->st[1], st1, load);
  if (spec) {
    for (int i = 0; i < 2 * 1024; i++) {
      if (load)
        c->spec[i] = (FIXP_DBL)spec[i];
      else
        spec[i] = (INT)c->spec[i];
    }
  }
}

/* ---- aacdec_hcr.h ------------------------------------------------------------------------- */

void cref_aacdec_hcr_CHcr_Read(void *ctx, INT ch, UINT *bsSt, UCHAR *buf, INT type) {
  BS_BEGIN;
  CHcr_Read(&bs, &ctx_of(ctx)->ch[ch & 1], (MP4_ELEMENT_ID)type);
  BS_END;
}

UINT cref_aacdec_hcr_HcrInit(void *ctx, INT ch, UINT spf, UINT sr_idx, UINT sr, UINT *bsSt, UCHAR *buf) {
  Ctx *c = ctx_of(ctx);
  SamplingRateInfo t;
  sri(&t, spf, sr_idx, sr);
  BS_BEGIN;
  UINT r = HcrInit(hcr_point(c, ch), &c->ch[ch & 1], &t, &bs);
  BS_END;
  return r;
}

UINT cref_aacdec_hcr_HcrDecoder(void *ctx, INT ch, UINT spf, UINT sr_idx, UINT sr, UINT *bsSt, UCHAR *buf) {
  Ctx *c = ctx_of(ctx);
  SamplingRateInfo t;
  sri(&t, spf, sr_idx, sr);
  BS_BEGIN;
  UINT r = HcrDecoder(hcr_point(c, ch), &c->ch[ch & 1], &t, &bs);
  BS_END;
  return r;
}

void cref_aacdec_hcr_HcrMuteErroneousLines(void *ctx, INT ch) {
  HcrMuteErroneousLines(hcr_point(ctx_of(ctx), ch));
}

void cref_aacdec_hcr_CarryBitToBranchValue(UINT carryBit, UINT treeNode, UINT *out) {
  CarryBitToBranchValue((UCHAR)carryBit, treeNode, &out[0], &out[1]);
}

/* The file-local functions of aacdec_hcr.cpp. */
void cref_aacdec_hcr_HcrCalcNumCodeword(void *ctx, INT ch) {
  ref_hcr::HcrCalcNumCodeword(hcr_point(ctx_of(ctx), ch));
}
void cref_aacdec_hcr_HcrSortCodebookAndNumCodewordInSection(void *ctx, INT ch) {
  ref_hcr::HcrSortCodebookAndNumCodewordInSection(hcr_point(ctx_of(ctx), ch));
}
void cref_aacdec_hcr_HcrPrepareSegmentationGrid(void *ctx, INT ch) {
  ref_hcr::HcrPrepareSegmentationGrid(hcr_point(ctx_of(ctx), ch));
}
void cref_aacdec_hcr_HcrExtendedSectionInfo(void *ctx, INT ch) {
  ref_hcr::HcrExtendedSectionInfo(hcr_point(ctx_of(ctx), ch));
}
/* DeriveNumberOfExtendedSortedSectionsInSets with the arguments of HcrDecoder(). */
void cref_aacdec_hcr_DeriveNumberOfExtendedSortedSectionsInSets(void *ctx, INT ch) {
  CErHcrInfo *h = hcr_point(ctx_of(ctx), ch);
  ref_hcr::DeriveNumberOfExtendedSortedSectionsInSets(
      h->segmentInfo.numSegment, h->sectionInfo.pNumExtendedSortedCodewordInSection,
      h->sectionInfo.numExtendedSortedCodewordInSectionIdx,
      h->sectionInfo.pNumExtendedSortedSectionsInSets,
      h->sectionInfo.numExtendedSortedSectionsInSetsIdx);
}
void cref_aacdec_hcr_DecodePCWs(void *ctx, INT ch, UINT *bsSt, UCHAR *buf) {
  BS_BEGIN;
  ref_hcr::DecodePCWs(&bs, hcr_point(ctx_of(ctx), ch));
  BS_END;
}
void cref_aacdec_hcr_HcrReorderQuantizedSpectralCoefficients(void *ctx, INT ch, UINT spf, UINT sr_idx, UINT sr) {
  Ctx *c = ctx_of(ctx);
  SamplingRateInfo t;
  sri(&t, spf, sr_idx, sr);
  ref_hcr::HcrReorderQuantizedSpectralCoefficients(hcr_point(c, ch), &c->ch[ch & 1], &t);
}
void cref_aacdec_hcr_errDetectWithinSegmentationFinal(void *ctx, INT ch) {
  ref_hcr::errDetectWithinSegmentationFinal(hcr_point(ctx_of(ctx), ch));
}
/* errDetectPcwSegmentation(remaining, pHcr, kind, spectrum of ch + offset, dimension) */
UINT cref_aacdec_hcr_errDetectPcwSegmentation(void *ctx, INT ch, INT remaining, INT kind, INT offset, INT dimension) {
  Ctx *c = ctx_of(ctx);
  return ref_hcr::errDetectPcwSegmentation((SCHAR)remaining, hcr_point(c, ch), (PCW_TYPE)kind,
                                           c->ch[ch & 1].pSpectralCoefficient + offset,
                                           (UCHAR)dimension);
}
UINT cref_aacdec_hcr_errDetectorInHcrSideinfoShrt(INT cb, INT numLine, UINT errorWord) {
  ref_hcr::errDetectorInHcrSideinfoShrt((SCHAR)cb, (SHORT)numLine, &errorWord);
  return errorWord;
}
UINT cref_aacdec_hcr_errDetectorInHcrLengths(INT lolc, INT lorsd, UINT errorWord) {
  ref_hcr::errDetectorInHcrLengths((SCHAR)lolc, (SHORT)lorsd, &errorWord);
  return errorWord;
}
/* DecodePCW_Body with the tree of codebook cb; io = {leftStartOfSegment, remainingBits,
   numDecodedBits}; returns the offset of the result in the quantized value table of cb. */
INT cref_aacdec_hcr_DecodePCW_Body(UINT *bsSt, UCHAR *buf, INT bsAnchor, INT cb, INT *io) {
  BS_BEGIN;
  INT left = io[0];
  SCHAR rem = (SCHAR)io[1];
  int ndb = io[2];
  const SCHAR *q = ref_hcr::DecodePCW_Body(&bs, bsAnchor, aHuffTable[cb], aQuantTable[cb], &left,
                                           &rem, &ndb);
  BS_END;
  io[0] = left;
  io[1] = rem;
  io[2] = ndb;
  return (INT)(q - aQuantTable[cb]);
}
/* DecodeEscapeSequence; io = {leftStartOfSegment, remainingBits, numDecodedBits, errorWord} */
INT cref_aacdec_hcr_DecodeEscapeSequence(UINT *bsSt, UCHAR *buf, INT bsAnchor, INT q, INT *io) {
  BS_BEGIN;
  INT left = io[0];
  SCHAR rem = (SCHAR)io[1];
  int ndb = io[2];
  UINT err = (UINT)io[3];
  INT r = ref_hcr::DecodeEscapeSequence(&bs, bsAnchor, q, &left, &rem, &ndb, &err);
  BS_END;
  io[0] = left;
  io[1] = rem;
  io[2] = ndb;
  io[3] = (INT)err;
  return r;
}
/* DecodePCW_Sign on the values qv[dim] into spec[1024]; io = {leftStartOfSegment, remainingBits,
   numDecodedBits, quantSpecCoefIdx} */
INT cref_aacdec_hcr_DecodePCW_Sign(UINT *bsSt, UCHAR *buf, INT bsAnchor, INT dim, const INT *qv, INT *spec, INT *io) {
  SCHAR qvs[4];
  for (int i = 0; i < 4; i++) qvs[i] = (SCHAR)qv[i];
  BS_BEGIN;
  INT left = io[0];
  SCHAR rem = (SCHAR)io[1];
  int ndb = io[2];
  int idx = io[3];
  INT r = ref_hcr::DecodePCW_Sign(&bs, bsAnchor, (UINT)dim, qvs, (FIXP_DBL *)spec, &idx, &left,
                                  &rem, &ndb);
  BS_END;
  io[0] = left;
  io[1] = rem;
  io[2] = ndb;
  io[3] = idx;
  return r;
}
/* GetScaleFromValue of block.h (compiled into this shim) */
INT cref_aacdec_hcr_GetScaleFromValue(INT value, UINT lsb) { return GetScaleFromValue((FIXP_DBL)value, lsb); }

/* ---- aacdec_hcr_bit.h --------------------------------------------------------------------- */

UINT cref_aacdec_hcr_bit_ToggleReadDirection(UINT d) { return ToggleReadDirection((UCHAR)d); }

/* HcrGetABitFromBitstream; lr = {leftStartOfSegment, rightStartOfSegment} */
UINT cref_aacdec_hcr_bit_HcrGetABitFromBitstream(UINT *bsSt, UCHAR *buf, INT bsAnchor, INT *lr, UINT readDirection) {
  BS_BEGIN;
  UINT r = HcrGetABitFromBitstream(&bs, bsAnchor, &lr[0], &lr[1], (UCHAR)readDirection);
  BS_END;
  return r;
}

/* ---- aacdec_hcrs.h ------------------------------------------------------------------------ */

void cref_aacdec_hcrs_DecodeNonPCWs(void *ctx, INT ch, UINT *bsSt, UCHAR *buf) {
  BS_BEGIN;
  DecodeNonPCWs(&bs, hcr_point(ctx_of(ctx), ch));
  BS_END;
}

/* The state function of state constant k (1..7) of aStateConstant2State. */
UINT cref_aacdec_hcrs_state(void *ctx, INT ch, INT k, UINT *bsSt, UCHAR *buf) {
  BS_BEGIN;
  UINT r = aStateConstant2State[k](&bs, hcr_point(ctx_of(ctx), ch));
  BS_END;
  return r;
}

UINT cref_aacdec_hcrs_InitSegmentBitfield(void *ctx) {
  HCR_SEGMENT_INFO *sg = &ctx_of(ctx)->com.overlay.aac.erHcrInfo.segmentInfo;
  return ref_hcrs::InitSegmentBitfield(&sg->numSegment, sg->pRemainingBitsInSegment,
                                       sg->pSegmentBitfield, &sg->numWordForBitfield,
                                       &sg->pNumBitValidInLastWord);
}

void cref_aacdec_hcrs_InitNonPCWSideInformationForCurrentSet(void *ctx) {
  ref_hcrs::InitNonPCWSideInformationForCurrentSet(&ctx_of(ctx)->com.overlay.aac.erHcrInfo);
}

INT cref_aacdec_hcrs_ModuloValue(INT input, INT bufferlength) {
  return ref_hcrs::ModuloValue(input, bufferlength);
}

/* ClearBitFromBitfield on bitfield[17]; returns 1 if the state was set to NULL. */
INT cref_aacdec_hcrs_ClearBitFromBitfield(UINT offset, UINT *bitfield) {
  STATEFUNC st = aStateConstant2State[1];
  ref_hcrs::ClearBitFromBitfield(&st, offset, bitfield);
  return st == NULL;
}
}
