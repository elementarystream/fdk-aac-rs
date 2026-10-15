// Stream harness for the differential tests of fdk_aac_rs_aacdec::aacdec_hcr and rvlc. Every
// `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_aacdec_hcr_stream (see build.rs).
//
// It decodes a stream with a copy of the reference decoder whose calls of the HCR and RVLC
// functions are intercepted: block.cpp, channel.cpp, aacdecoder.cpp and aacdecoder_lib.cpp are
// compiled into this file with every global function they define renamed to hcrt_* (so that they
// do not clash with the library), and the calls of HcrInit, HcrDecoder, HcrMuteErroneousLines,
// CHcr_Read, CRvlc_Read, CRvlc_Decode and CRvlc_ElementCheck are renamed to wrappers that record
// the state before and after the library's function (with the transfer functions of
// aacdec_aacdec_hcr.cpp). The tests replay every recorded call on the C++ function and on the port.
// Everything else (transport, concealment, SBR, ...) is the library's code.
//
// A record is [kind, number of blocks, blocks...], a block [tag, length, values...]; the tags are
// REC_* (inputs) and REC_* + REC_OUT (outputs). The memory of the bit buffer is stored once per
// change (snapshots).

#define ApplyTools hcrt_ApplyTools
#define CBlock_ApplyNoise hcrt_CBlock_ApplyNoise
#define CBlock_FrequencyToTime hcrt_CBlock_FrequencyToTime
#define CBlock_FrequencyToTimeLowDelay hcrt_CBlock_FrequencyToTimeLowDelay
#define CBlock_GetEscape hcrt_CBlock_GetEscape
#define CBlock_InverseQuantizeSpectralData hcrt_CBlock_InverseQuantizeSpectralData
#define CBlock_ReadAcSpectralData hcrt_CBlock_ReadAcSpectralData
#define CBlock_ReadScaleFactorData hcrt_CBlock_ReadScaleFactorData
#define CBlock_ReadSectionData hcrt_CBlock_ReadSectionData
#define CBlock_ReadSpectralData hcrt_CBlock_ReadSpectralData
#define CBlock_ScaleSpectralData hcrt_CBlock_ScaleSpectralData
#define get_gain hcrt_get_gain
#define CChannel_CodebookTableInit hcrt_CChannel_CodebookTableInit
#define CChannelElement_Decode hcrt_CChannelElement_Decode
#define CChannelElement_Read hcrt_CChannelElement_Read
#define CAacDecoder_AncDataGet hcrt_CAacDecoder_AncDataGet
#define CAacDecoder_AncDataInit hcrt_CAacDecoder_AncDataInit
#define CAacDecoder_ApplyCrossFade hcrt_CAacDecoder_ApplyCrossFade
#define CAacDecoder_Close hcrt_CAacDecoder_Close
#define CAacDecoder_CtrlCFGChange hcrt_CAacDecoder_CtrlCFGChange
#define CAacDecoder_DecodeFrame hcrt_CAacDecoder_DecodeFrame
#define CAacDecoder_FreeMem hcrt_CAacDecoder_FreeMem
#define CAacDecoder_GetStreamInfo hcrt_CAacDecoder_GetStreamInfo
#define CAacDecoder_Init hcrt_CAacDecoder_Init
#define CAacDecoder_Open hcrt_CAacDecoder_Open
#define CAacDecoder_PrepareCrossFade hcrt_CAacDecoder_PrepareCrossFade
#define CAacDecoder_PreRollExtensionPayloadParse hcrt_CAacDecoder_PreRollExtensionPayloadParse
#define CAacDecoder_SignalInterruption hcrt_CAacDecoder_SignalInterruption
#define CAacDecoder_SyncQmfMode hcrt_CAacDecoder_SyncQmfMode
#undef aacDecoder_AncDataGet
#undef aacDecoder_AncDataInit
#undef aacDecoder_Close
#undef aacDecoder_ConfigRaw
#undef aacDecoder_DecodeFrame
#undef aacDecoder_Fill
#undef aacDecoder_GetFreeBytes
#undef aacDecoder_GetLibInfo
#undef aacDecoder_GetStreamInfo
#undef aacDecoder_Open
#undef aacDecoder_RawISOBMFFData
#undef aacDecoder_SetParam
#define aacDecoder_AncDataGet hcrt_aacDecoder_AncDataGet
#define aacDecoder_AncDataInit hcrt_aacDecoder_AncDataInit
#define aacDecoder_Close hcrt_aacDecoder_Close
#define aacDecoder_ConfigRaw hcrt_aacDecoder_ConfigRaw
#define aacDecoder_DecodeFrame hcrt_aacDecoder_DecodeFrame
#define aacDecoder_Fill hcrt_aacDecoder_Fill
#define aacDecoder_GetFreeBytes hcrt_aacDecoder_GetFreeBytes
#define aacDecoder_GetLibInfo hcrt_aacDecoder_GetLibInfo
#define aacDecoder_GetStreamInfo hcrt_aacDecoder_GetStreamInfo
#define aacDecoder_Open hcrt_aacDecoder_Open
#define aacDecoder_RawISOBMFFData hcrt_aacDecoder_RawISOBMFFData
#define aacDecoder_SetParam hcrt_aacDecoder_SetParam
#define HcrInit hcrt_HcrInit
#define HcrDecoder hcrt_HcrDecoder
#define HcrMuteErroneousLines hcrt_HcrMuteErroneousLines
#define CHcr_Read hcrt_CHcr_Read
#define CRvlc_Read hcrt_CRvlc_Read
#define CRvlc_Decode hcrt_CRvlc_Decode
#define CRvlc_ElementCheck hcrt_CRvlc_ElementCheck

#include "cref_common.h"
#include "aac_ram.h"
#include "aac_rom.h"
#include "aacdec_drc.h"
#include "aacdec_hcr.h"
#include "aacdec_pns.h"
#include "aacdec_tns.h"
#include "aacdecoder_lib.h"
#include "aacdecoder.h"
#include "ac_arith_coder.h"
#include "block.h"
#include "channel.h"
#include "conceal.h"
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "FDK_core.h"
#include "FDK_crc.h"
#include "FDK_tools_rom.h"
#include "FDK_trigFcts.h"
#include "ldfiltbank.h"
#include "pcm_utils.h"
#include "rvlc.h"
#include "sac_dec_lib.h"
#include "sbrdecoder.h"
#include "scale.h"
#include "tpdec_lib.h"
#include "usacdec_fac.h"
#include "usacdec_lpc.h"
#include "usacdec_lpd.h"
#include <stdlib.h>

#include "libAACdec/src/block.cpp"
#include "libAACdec/src/channel.cpp"
#include "libAACdec/src/aacdecoder.cpp"
#include "libAACdec/src/aacdecoder_lib.cpp"

#undef HcrInit
#undef HcrDecoder
#undef HcrMuteErroneousLines
#undef CHcr_Read
#undef CRvlc_Read
#undef CRvlc_Decode
#undef CRvlc_ElementCheck

/* The library's functions (their declarations above were renamed). */
UINT HcrInit(H_HCR_INFO pHcr, CAacDecoderChannelInfo *pAacDecoderChannelInfo,
             const SamplingRateInfo *pSamplingRateInfo, HANDLE_FDK_BITSTREAM bs);
UINT HcrDecoder(H_HCR_INFO hHcr, CAacDecoderChannelInfo *pAacDecoderChannelInfo,
                const SamplingRateInfo *pSamplingRateInfo, HANDLE_FDK_BITSTREAM bs);
void HcrMuteErroneousLines(H_HCR_INFO hHcr);
void CHcr_Read(HANDLE_FDK_BITSTREAM bs, CAacDecoderChannelInfo *pAacDecoderChannelInfo,
               const MP4_ELEMENT_ID globalHcrType);
void CRvlc_Read(CAacDecoderChannelInfo *pAacDecoderChannelInfo, HANDLE_FDK_BITSTREAM bs);
void CRvlc_Decode(CAacDecoderChannelInfo *pAacDecoderChannelInfo,
                  CAacDecoderStaticChannelInfo *pAacDecoderStaticChannelInfo,
                  HANDLE_FDK_BITSTREAM bs);
void CRvlc_ElementCheck(CAacDecoderChannelInfo *pAacDecoderChannelInfo[],
                        CAacDecoderStaticChannelInfo *pAacDecoderStaticChannelInfo[],
                        const UINT flags, const INT elChannels);

extern "C" {
INT cref_aacdec_hcr_layout(INT *out);
void cref_aacdec_hcr_xfer_ch(void *chp, INT *v, INT load);
void cref_aacdec_hcr_xfer_hcr(void *hcrp, INT *v, INT load);
void cref_aacdec_hcr_xfer_rv(void *comp, INT *v, INT load);
void cref_aacdec_hcr_xfer_st(void *stp, INT *v, INT load);
}

/* ---- recorder ----------------------------------------------------------------------------- */

enum {
  KIND_CHCR_READ = 1,
  KIND_HCR_INIT = 2,
  KIND_HCR_DECODER = 3,
  KIND_HCR_MUTE = 4,
  KIND_RVLC_READ = 5,
  KIND_RVLC_DECODE = 6,
  KIND_RVLC_ELEMENT_CHECK = 7
};
enum {
  REC_ARGS = 1,
  REC_BS = 2,
  REC_SNAP = 3,
  REC_CH0 = 4,
  REC_CH1 = 5,
  REC_HCR = 6,
  REC_RV = 7,
  REC_ST0 = 8,
  REC_ST1 = 9,
  REC_SPEC = 10,
  REC_RET = 11,
  REC_OUT = 100
};

struct Rec {
  INT *d;
  size_t len, cap;
  size_t recStart;
  INT nBlocks;
  UCHAR **snap;
  INT *snapLen;
  INT nSnap, capSnap;
  INT lens[5];
};

static thread_local Rec *g_rec;

static INT *rec_block(Rec *r, INT tag, INT n) {
  if (r->len + n + 2 > r->cap) {
    size_t c = (r->cap ? r->cap * 2 : 1 << 20);
    while (c < r->len + n + 2) c *= 2;
    r->d = (INT *)realloc(r->d, c * sizeof(INT));
    r->cap = c;
  }
  r->d[r->len] = tag;
  r->d[r->len + 1] = n;
  INT *p = &r->d[r->len + 2];
  r->len += n + 2;
  r->nBlocks++;
  return p;
}

static void rec_begin(Rec *r, INT kind) {
  r->nBlocks = 0;
  rec_block(r, kind, 0);
  r->recStart = r->len - 2;
  r->nBlocks = 0;
}

static void rec_end(Rec *r) { r->d[r->recStart + 1] = r->nBlocks; }

static void rec_bs(Rec *r, HANDLE_FDK_BITSTREAM bs, INT out) {
  INT *p = rec_block(r, REC_BS + out, 9);
  p[0] = (INT)bs->CacheWord;
  p[1] = (INT)bs->BitsInCache;
  p[2] = (INT)bs->ConfigCache;
  p[3] = (INT)bs->hBitBuf.ValidBits;
  p[4] = (INT)bs->hBitBuf.ReadOffset;
  p[5] = (INT)bs->hBitBuf.WriteOffset;
  p[6] = (INT)bs->hBitBuf.BitNdx;
  p[7] = (INT)bs->hBitBuf.bufSize;
  p[8] = (INT)bs->hBitBuf.bufBits;
  if (out) return;
  /* the memory of the bit buffer, stored once per change */
  INT n = (INT)bs->hBitBuf.bufSize;
  INT k = r->nSnap - 1;
  if (k < 0 || r->snapLen[k] != n || FDKmemcmp(r->snap[k], bs->hBitBuf.Buffer, n) != 0) {
    if (r->nSnap == r->capSnap) {
      r->capSnap = r->capSnap ? 2 * r->capSnap : 64;
      r->snap = (UCHAR **)realloc(r->snap, r->capSnap * sizeof(UCHAR *));
      r->snapLen = (INT *)realloc(r->snapLen, r->capSnap * sizeof(INT));
    }
    r->snap[r->nSnap] = (UCHAR *)malloc(n);
    FDKmemcpy(r->snap[r->nSnap], bs->hBitBuf.Buffer, n);
    r->snapLen[r->nSnap] = n;
    r->nSnap++;
    k = r->nSnap - 1;
  }
  rec_block(r, REC_SNAP, 1)[0] = k;
}

static void rec_ch(Rec *r, CAacDecoderChannelInfo *p, INT tag) {
  cref_aacdec_hcr_xfer_ch(p, rec_block(r, tag, r->lens[0]), 0);
}
static void rec_hcr(Rec *r, CErHcrInfo *h, INT tag) {
  cref_aacdec_hcr_xfer_hcr(h, rec_block(r, tag, r->lens[1]), 0);
}
static void rec_rv(Rec *r, CAacDecoderCommonData *c, INT tag) {
  cref_aacdec_hcr_xfer_rv(c, rec_block(r, tag, r->lens[2]), 0);
}
static void rec_st(Rec *r, CAacDecoderStaticChannelInfo *p, INT tag) {
  cref_aacdec_hcr_xfer_st(p, rec_block(r, tag, r->lens[3]), 0);
}
static void rec_spec(Rec *r, const FIXP_DBL *spec, INT tag) {
  INT *p = rec_block(r, tag, 1024);
  for (int i = 0; i < 1024; i++) p[i] = (INT)spec[i];
}
static void rec_sri(Rec *r, const SamplingRateInfo *s) {
  INT *p = rec_block(r, REC_ARGS, 3);
  p[0] = s->ScaleFactorBands_Long[s->NumberOfScaleFactorBands_Long];
  p[1] = (INT)s->samplingRateIndex;
  p[2] = (INT)s->samplingRate;
}

/* ---- wrappers ----------------------------------------------------------------------------- */

UINT hcrt_HcrInit(H_HCR_INFO pHcr, CAacDecoderChannelInfo *p, const SamplingRateInfo *s, HANDLE_FDK_BITSTREAM bs) {
  Rec *r = g_rec;
  if (r) {
    rec_begin(r, KIND_HCR_INIT);
    rec_sri(r, s);
    rec_bs(r, bs, 0);
    rec_ch(r, p, REC_CH0);
    rec_hcr(r, pHcr, REC_HCR);
  }
  UINT ret = HcrInit(pHcr, p, s, bs);
  if (r) {
    rec_bs(r, bs, REC_OUT);
    rec_ch(r, p, REC_CH0 + REC_OUT);
    rec_hcr(r, pHcr, REC_HCR + REC_OUT);
    rec_block(r, REC_RET, 1)[0] = (INT)ret;
    rec_end(r);
  }
  return ret;
}

UINT hcrt_HcrDecoder(H_HCR_INFO pHcr, CAacDecoderChannelInfo *p, const SamplingRateInfo *s, HANDLE_FDK_BITSTREAM bs) {
  Rec *r = g_rec;
  if (r) {
    rec_begin(r, KIND_HCR_DECODER);
    rec_sri(r, s);
    rec_bs(r, bs, 0);
    rec_ch(r, p, REC_CH0);
    rec_hcr(r, pHcr, REC_HCR);
    rec_spec(r, p->pSpectralCoefficient, REC_SPEC);
  }
  UINT ret = HcrDecoder(pHcr, p, s, bs);
  if (r) {
    rec_bs(r, bs, REC_OUT);
    rec_ch(r, p, REC_CH0 + REC_OUT);
    rec_hcr(r, pHcr, REC_HCR + REC_OUT);
    rec_spec(r, p->pSpectralCoefficient, REC_SPEC + REC_OUT);
    rec_block(r, REC_RET, 1)[0] = (INT)ret;
    rec_end(r);
  }
  return ret;
}

void hcrt_HcrMuteErroneousLines(H_HCR_INFO pHcr) {
  Rec *r = g_rec;
  if (r) {
    rec_begin(r, KIND_HCR_MUTE);
    rec_hcr(r, pHcr, REC_HCR);
    rec_spec(r, pHcr->decInOut.pQuantizedSpectralCoefficientsBase, REC_SPEC);
  }
  HcrMuteErroneousLines(pHcr);
  if (r) {
    rec_hcr(r, pHcr, REC_HCR + REC_OUT);
    rec_spec(r, pHcr->decInOut.pQuantizedSpectralCoefficientsBase, REC_SPEC + REC_OUT);
    rec_end(r);
  }
}

void hcrt_CHcr_Read(HANDLE_FDK_BITSTREAM bs, CAacDecoderChannelInfo *p, const MP4_ELEMENT_ID type) {
  Rec *r = g_rec;
  if (r) {
    rec_begin(r, KIND_CHCR_READ);
    rec_block(r, REC_ARGS, 1)[0] = (INT)type;
    rec_bs(r, bs, 0);
    rec_ch(r, p, REC_CH0);
  }
  CHcr_Read(bs, p, type);
  if (r) {
    rec_bs(r, bs, REC_OUT);
    rec_ch(r, p, REC_CH0 + REC_OUT);
    rec_end(r);
  }
}

void hcrt_CRvlc_Read(CAacDecoderChannelInfo *p, HANDLE_FDK_BITSTREAM bs) {
  Rec *r = g_rec;
  if (r) {
    rec_begin(r, KIND_RVLC_READ);
    rec_bs(r, bs, 0);
    rec_ch(r, p, REC_CH0);
    rec_rv(r, p->pComData, REC_RV);
  }
  CRvlc_Read(p, bs);
  if (r) {
    rec_bs(r, bs, REC_OUT);
    rec_ch(r, p, REC_CH0 + REC_OUT);
    rec_rv(r, p->pComData, REC_RV + REC_OUT);
    rec_end(r);
  }
}

void hcrt_CRvlc_Decode(CAacDecoderChannelInfo *p, CAacDecoderStaticChannelInfo *st, HANDLE_FDK_BITSTREAM bs) {
  Rec *r = g_rec;
  if (r) {
    rec_begin(r, KIND_RVLC_DECODE);
    rec_bs(r, bs, 0);
    rec_ch(r, p, REC_CH0);
    rec_rv(r, p->pComData, REC_RV);
    rec_st(r, st, REC_ST0);
  }
  CRvlc_Decode(p, st, bs);
  if (r) {
    rec_bs(r, bs, REC_OUT);
    rec_ch(r, p, REC_CH0 + REC_OUT);
    rec_rv(r, p->pComData, REC_RV + REC_OUT);
    rec_st(r, st, REC_ST0 + REC_OUT);
    rec_end(r);
  }
}

void hcrt_CRvlc_ElementCheck(CAacDecoderChannelInfo *p[], CAacDecoderStaticChannelInfo *st[], const UINT flags, const INT elChannels) {
  Rec *r = g_rec;
  /* only elements of one or two channels with static channel infos (the decoder's AAC
     elements) are recorded */
  INT rec = r && st != NULL && (elChannels == 1 || elChannels == 2);
  if (rec) {
    rec_begin(r, KIND_RVLC_ELEMENT_CHECK);
    INT *a = rec_block(r, REC_ARGS, 3);
    a[0] = 1;
    a[1] = (INT)flags;
    a[2] = elChannels;
    rec_rv(r, p[0]->pComData, REC_RV);
    for (int ch = 0; ch < elChannels; ch++) {
      rec_ch(r, p[ch], REC_CH0 + ch);
      rec_st(r, st[ch], REC_ST0 + ch);
    }
  }
  CRvlc_ElementCheck(p, st, flags, elChannels);
  if (rec) {
    for (int ch = 0; ch < elChannels; ch++) {
      rec_ch(r, p[ch], REC_CH0 + ch + REC_OUT);
      rec_st(r, st[ch], REC_ST0 + ch + REC_OUT);
    }
    rec_end(r);
  }
}

extern "C" {
/* Decodes `data` (transport type `tt`, e.g. TT_MP4_LOAS) for at most maxFrames calls of
   aacDecoder_DecodeFrame that do not return AAC_DEC_NOT_ENOUGH_BITS, and returns the recording
   (free it with cref_aacdec_hcr_stream_free). info = {frames, calls with errors, last error}. */
void *cref_aacdec_hcr_stream_decode(const UCHAR *data, INT len, INT tt, INT maxFrames, INT *info) {
  Rec *r = (Rec *)calloc(1, sizeof(Rec));
  cref_aacdec_hcr_layout(r->lens);
  g_rec = r;
  HANDLE_AACDECODER h = hcrt_aacDecoder_Open((TRANSPORT_TYPE)tt, 1);
  INT frames = 0, errors = 0, lastErr = 0;
  if (h != NULL) {
    const INT outLen = 8 * 4096;
    INT_PCM *out = (INT_PCM *)calloc(outLen, sizeof(INT_PCM));
    UINT left = (UINT)len;
    const UCHAR *p = data;
    for (int it = 0; it < 4 * maxFrames + 64 && frames < maxFrames; it++) {
      if (left > 0) {
        UCHAR *pp = (UCHAR *)p;
        UINT size = left, valid = left;
        hcrt_aacDecoder_Fill(h, &pp, &size, &valid);
        p += left - valid;
        left = valid;
      }
      AAC_DECODER_ERROR err = hcrt_aacDecoder_DecodeFrame(h, out, outLen, 0);
      if (err == AAC_DEC_NOT_ENOUGH_BITS) {
        if (left == 0) break;
        continue;
      }
      frames++;
      if (err != AAC_DEC_OK) {
        errors++;
        lastErr = (INT)err;
      }
    }
    free(out);
    hcrt_aacDecoder_Close(h);
  }
  g_rec = NULL;
  info[0] = frames;
  info[1] = errors;
  info[2] = lastErr;
  return r;
}

INT cref_aacdec_hcr_stream_len(void *rec) { return (INT)((Rec *)rec)->len; }

void cref_aacdec_hcr_stream_copy(void *rec, INT *dst) {
  Rec *r = (Rec *)rec;
  if (r->len) FDKmemcpy(dst, r->d, r->len * sizeof(INT));
}

INT cref_aacdec_hcr_stream_snap_count(void *rec) { return ((Rec *)rec)->nSnap; }

INT cref_aacdec_hcr_stream_snap_len(void *rec, INT i) { return ((Rec *)rec)->snapLen[i]; }

void cref_aacdec_hcr_stream_snap_copy(void *rec, INT i, UCHAR *dst) {
  Rec *r = (Rec *)rec;
  FDKmemcpy(dst, r->snap[i], r->snapLen[i]);
}

void cref_aacdec_hcr_stream_free(void *rec) {
  Rec *r = (Rec *)rec;
  for (int i = 0; i < r->nSnap; i++) free(r->snap[i]);
  free(r->snap);
  free(r->snapLen);
  free(r->d);
  free(r);
}
}
