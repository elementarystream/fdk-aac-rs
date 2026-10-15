// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_lib.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_lib (see build.rs).
//
// The encoder of the library is driven through its API (FDK_sacenc_*) as the AAC encoder
// (mps_main.cpp) does. Its whole state is read through the copy of struct MP4SPACE_ENCODER in
// namespace rsl (cref_sacenc_lib_state, see there for the order), which also gives the
// file-static functions.

#include "cref_common.h"
#include <stdlib.h>
#include "genericStds.h"
#include "common_fix.h"
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "FDK_matrixCalloc.h"
#include "FDK_trigFcts.h"
#include "fixpoint_math.h"
#include "FDK_core.h"
#include "qmf.h"

/* The whole of libSACenc in namespace rsl (its headers included there first), so that
   the structures of the .cpp files and the handles that point to them live in the namespace
   with the layout of the library. The extern "C" API functions of the copy are renamed. */
#define FDK_sacenc_open rsl_FDK_sacenc_open
#define FDK_sacenc_init rsl_FDK_sacenc_init
#define FDK_sacenc_close rsl_FDK_sacenc_close
#define FDK_sacenc_encode rsl_FDK_sacenc_encode
#define FDK_sacenc_getInfo rsl_FDK_sacenc_getInfo
#define FDK_sacenc_setParam rsl_FDK_sacenc_setParam
#define FDK_sacenc_getLibInfo rsl_FDK_sacenc_getLibInfo
namespace rsl {
#include "sacenc_lib.h"
#include "libSACenc/src/sacenc_const.h"
#include "libSACenc/src/sacenc_bitstream.h"
#include "libSACenc/src/sacenc_nlc_enc.h"
#include "libSACenc/src/sacenc_huff_tab.h"
#include "libSACenc/src/sacenc_vectorfunctions.h"
#include "libSACenc/src/sacenc_paramextract.h"
#include "libSACenc/src/sacenc_framewindowing.h"
#include "libSACenc/src/sacenc_tree.h"
#include "libSACenc/src/sacenc_onsetdetect.h"
#include "libSACenc/src/sacenc_filter.h"
#include "libSACenc/src/sacenc_staticgain.h"
#include "libSACenc/src/sacenc_delay.h"
#include "libSACenc/src/sacenc_dmx_tdom_enh.h"
/* sacenc_dmx_tdom_enh.cpp overloads these functions at file scope; in the namespace its
   overloads would hide the global ones. */
using ::invSqrtNorm2;
using ::sqrtFixp;
#include "libSACenc/src/sacenc_bitstream.cpp"
#include "libSACenc/src/sacenc_delay.cpp"
#include "libSACenc/src/sacenc_dmx_tdom_enh.cpp"
#include "libSACenc/src/sacenc_filter.cpp"
#include "libSACenc/src/sacenc_framewindowing.cpp"
#include "libSACenc/src/sacenc_huff_tab.cpp"
#include "libSACenc/src/sacenc_lib.cpp"
#include "libSACenc/src/sacenc_nlc_enc.cpp"
#include "libSACenc/src/sacenc_onsetdetect.cpp"
#include "libSACenc/src/sacenc_paramextract.cpp"
#include "libSACenc/src/sacenc_staticgain.cpp"
#include "libSACenc/src/sacenc_tree.cpp"
#include "libSACenc/src/sacenc_vectorfunctions.cpp"
}
#undef FDK_sacenc_open
#undef FDK_sacenc_init
#undef FDK_sacenc_close
#undef FDK_sacenc_encode
#undef FDK_sacenc_getInfo
#undef FDK_sacenc_setParam
#undef FDK_sacenc_getLibInfo
/* The global declarations of the library (the guards were set inside the namespace). */
#undef SACENC_LIB_H
#undef SACENC_CONST_H
#undef SACENC_BITSTREAM_H
#undef SACENC_NLC_ENC_H
#undef SACENC_HUFF_TAB_H
#undef SACENC_VECTORFUNCTIONS_H
#undef SACENC_PARAMEXTRACT_H
#undef SACENC_FRAMEWINDOWING_H
#undef SACENC_TREE_H
#undef SACENC_ONSETDETECT_H
#undef SACENC_FILTER_H
#undef SACENC_STATICGAIN_H
#undef SACENC_DELAY_H
#undef SACENC_DMX_TDOM_ENH_H
#include "sacenc_lib.h"
#include "libSACenc/src/sacenc_const.h"
#include "libSACenc/src/sacenc_bitstream.h"
#include "libSACenc/src/sacenc_nlc_enc.h"
#include "libSACenc/src/sacenc_huff_tab.h"
#include "libSACenc/src/sacenc_vectorfunctions.h"
#include "libSACenc/src/sacenc_paramextract.h"
#include "libSACenc/src/sacenc_framewindowing.h"
#include "libSACenc/src/sacenc_tree.h"
#include "libSACenc/src/sacenc_onsetdetect.h"
#include "libSACenc/src/sacenc_filter.h"
#include "libSACenc/src/sacenc_staticgain.h"
#include "libSACenc/src/sacenc_delay.h"
#include "libSACenc/src/sacenc_dmx_tdom_enh.h"

void sacenc_tto_state(const void *box, INT *out);

struct Dump {
  INT *out;
  INT n;
  void push(INT v) { out[n++] = v; }
  void bytes(const void *p, INT len) {
    const UCHAR *b = (const UCHAR *)p;
    for (INT i = 0; i < len; i++) push(b[i]);
  }
  void ints(const void *p, INT len) {
    const INT *b = (const INT *)p;
    for (INT i = 0; i < len; i++) push(b[i]);
  }
};

extern "C" {
void *cref_sacenc_lib_open(void) {
  HANDLE_MP4SPACE_ENCODER h = NULL;
  if (FDK_sacenc_open(&h) != SACENC_OK) return NULL;
  return h;
}
INT cref_sacenc_lib_open_null(void) { return FDK_sacenc_open(NULL); }
INT cref_sacenc_lib_close(void *h) {
  HANDLE_MP4SPACE_ENCODER p = (HANDLE_MP4SPACE_ENCODER)h;
  return FDK_sacenc_close(&p);
}
INT cref_sacenc_lib_close_null(void) {
  HANDLE_MP4SPACE_ENCODER p = NULL;
  return FDK_sacenc_close(NULL) * 2 + FDK_sacenc_close(&p);
}
INT cref_sacenc_lib_init(void *h, INT dmxDelay) { return FDK_sacenc_init((HANDLE_MP4SPACE_ENCODER)h, dmxDelay); }
INT cref_sacenc_lib_setParam(void *h, INT param, UINT value) {
  return FDK_sacenc_setParam((HANDLE_MP4SPACE_ENCODER)h, (SPACEENC_PARAM)param, value);
}
/* out: the 8 INTs of MP4SPACEENC_INFO, then nSscSizeBits and the 64 bytes of pSsc. */
INT cref_sacenc_lib_getInfo(void *h, INT *out) {
  MP4SPACEENC_INFO info;
  FDKmemclear(&info, sizeof(info));
  INT r = FDK_sacenc_getInfo((HANDLE_MP4SPACE_ENCODER)h, &info);
  if (r != SACENC_OK) return r;
  out[0] = info.nSampleRate;
  out[1] = info.nSamplesFrame;
  out[2] = info.nTotalInputChannels;
  out[3] = info.nDmxDelay;
  out[4] = info.nCodecDelay;
  out[5] = info.nDecoderDelay;
  out[6] = info.nPayloadDelay;
  out[7] = info.nDiscardOutFrames;
  out[8] = info.pSscBuf->nSscSizeBits;
  for (INT i = 0; i < 64; i++) out[9 + i] = info.pSscBuf->pSsc[i];
  return r;
}
INT cref_sacenc_lib_getInfo_null(void *h) {
  MP4SPACEENC_INFO info;
  return FDK_sacenc_getInfo(NULL, &info) * 4 + FDK_sacenc_getInfo((HANDLE_MP4SPACE_ENCODER)h, NULL);
}
/* FDK_sacenc_getLibInfo on an array of FDK_MODULE_LAST entries, of which the first `used` are
   taken. Per entry: module_id, version, flags, the version string (32 bytes) and the title
   (64 bytes, as far as it goes) = 99 INTs. */
INT cref_sacenc_lib_getLibInfo(INT used, INT *out) {
  LIB_INFO info[FDK_MODULE_LAST];
  FDKmemclear(info, sizeof(info));
  for (INT i = 0; i < FDK_MODULE_LAST; i++) info[i].module_id = (i < used) ? FDK_AACDEC : FDK_NONE;
  INT r = FDK_sacenc_getLibInfo(info);
  INT k = 0;
  for (INT i = 0; i < FDK_MODULE_LAST; i++) {
    out[k++] = info[i].module_id;
    out[k++] = info[i].version;
    out[k++] = info[i].flags;
    for (INT j = 0; j < 32; j++) out[k++] = (UCHAR)info[i].versionStr[j];
    for (INT j = 0; j < 64; j++) {
      const char *t = info[i].title;
      INT len = t ? (INT)strlen(t) : 0;
      out[k++] = (j < len) ? (UCHAR)t[j] : 0;
    }
  }
  return r;
}
INT cref_sacenc_lib_getLibInfo_null(void) { return FDK_sacenc_getLibInfo(NULL); }

/* FDK_sacenc_encode with the descriptors of mps_main.cpp: the input PCM buffer `pcm` is also
   the output PCM buffer (outPcmBytes bytes declared), `bs` the bitstream buffer (bsSize bytes
   declared). bsType: the type of the bitstream entry; numOut: number of output entries;
   outPcmNull: NULL output PCM pointer. outargs: in/out (nOutputBits, nOutputSamples,
   nSamplesConsumed). */
INT cref_sacenc_lib_encode(void *h, SHORT *pcm, INT nInputSamples, UINT bufSizePerCh, UINT interleaved, UINT outPcmBytes, UCHAR *bs, UINT bsSize, UINT bsType, INT numOut, INT outPcmNull, INT *outargs) {
  void *inBase[1] = {pcm};
  UINT inSize[1] = {0};
  UINT inEle[1] = {sizeof(INT_PCM)};
  UINT inType[1] = {FDK_BUF_TYPE_INPUT | FDK_BUF_TYPE_PCM_DATA};
  void *outBase[2] = {outPcmNull ? NULL : (void *)pcm, bs};
  UINT outSize[2] = {outPcmBytes, bsSize};
  UINT outEle[2] = {sizeof(INT_PCM), sizeof(UCHAR)};
  UINT outType[2] = {FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_PCM_DATA, bsType};
  FDK_bufDescr inDesc, outDesc;
  inDesc.ppBase = inBase;
  inDesc.pBufSize = inSize;
  inDesc.pEleSize = inEle;
  inDesc.pBufType = inType;
  inDesc.numBufs = 1;
  outDesc.ppBase = outBase;
  outDesc.pBufSize = outSize;
  outDesc.pEleSize = outEle;
  outDesc.pBufType = outType;
  outDesc.numBufs = numOut;
  SACENC_InArgs ia;
  ia.nInputSamples = nInputSamples;
  ia.inputBufferSizePerChannel = bufSizePerCh;
  ia.isInputInterleaved = interleaved;
  SACENC_OutArgs oa;
  oa.nOutputBits = outargs[0];
  oa.nOutputSamples = outargs[1];
  oa.nSamplesConsumed = outargs[2];
  INT r = FDK_sacenc_encode((HANDLE_MP4SPACE_ENCODER)h, &inDesc, &outDesc, &ia, &oa);
  outargs[0] = oa.nOutputBits;
  outargs[1] = oa.nOutputSamples;
  outargs[2] = oa.nSamplesConsumed;
  return r;
}

/* The whole state of the encoder; returns the number of INTs. */
INT cref_sacenc_lib_state(void *h, INT *out) {
  rsl::MP4SPACE_ENCODER *e = (rsl::MP4SPACE_ENCODER *)h;
  Dump d = {out, 0};
  const rsl::MP4SPACEENC_SETUP *u = &e->user;
  d.push(u->encMode);
  d.push(u->nParamBands);
  d.push(u->quantMode);
  d.push(u->bUseCoarseQuant);
  d.push(u->bLdMode);
  d.push(u->bTimeDomainDmx);
  d.push(u->sampleRate);
  d.push(u->frameTimeSlots);
  d.push(u->independencyFactor);
  d.push(u->timeAlignment);
  const rsl::ENC_CONFIG_SETUP *s = &e->setup;
  d.push(s->bEncMode_212);
  d.push(s->maxHybridInStaticSlots);
  d.push(s->maxSamplingrate);
  d.push(s->maxAnalysisLengthTimeSlots);
  d.push(s->maxHybridBands);
  d.push(s->maxQmfBands);
  d.push(s->maxChIn);
  d.push(s->maxFrameTimeSlots);
  d.push(s->maxFrameLength);
  d.push(s->maxChOut);
  d.push(s->maxChTotOut);
  d.ints(e->hFrameWindow, sizeof(rsl::FRAMEWINDOW) / sizeof(INT));
  d.push(e->nSamplesValid);
  d.push(e->nParamBands);
  d.push(e->useTimeDomDownmix);
  d.push(e->encMode);
  d.push(e->bEncMode_212_only);
  d.push(e->useFrameKeep);
  d.push(e->independencyFactor);
  d.push(e->nSampleRate);
  d.push(e->nInputChannels);
  d.push(e->nOutputChannels);
  d.push(e->nFrameTimeSlots);
  d.push(e->nQmfBands);
  d.push(e->nHybridBands);
  d.push(e->nFrameLength);
  d.push(e->nSamplesNext);
  d.push(e->nAnalysisLengthTimeSlots);
  d.push(e->nAnalysisLookaheadTimeSlots);
  d.push(e->nUpdateHybridPositionTimeSlots);
  d.push(e->pnOutputBits[0]);
  d.push(e->nInputDelay);
  d.push(e->nOutputBufferDelay);
  d.push(e->nSurroundAnalysisBufferDelay);
  d.push(e->nBitstreamDelayBuffer);
  d.push(e->nBitstreamBufferRead);
  d.push(e->nBitstreamBufferWrite);
  d.push(e->nDiscardOutFrames);
  d.push(e->avoid_keep);
  d.push(e->useCoarseQuantCld);
  d.push(e->useCoarseQuantIcc);
  d.push(e->useCoarseQuantCpc);
  d.push(e->useCoarseQuantArbDmx);
  d.push(e->quantMode);
  d.push(e->coreCoderDelay);
  d.push(e->timeAlignment);
  d.push(e->independencyCount);
  d.push(e->independencyFlag);
  d.push(e->ppTrCurrPos[0][0]);
  d.push(e->ppTrCurrPos[1][0]);
  d.push(e->trPrevPos[0]);
  d.push(e->trPrevPos[1]);
  d.push(e->frameWinList.dat[0].slot);
  d.push(e->frameWinList.dat[0].hold);
  d.push(e->frameWinList.dat[1].slot);
  d.push(e->frameWinList.dat[1].hold);
  d.push(e->frameWinList.n);
  d.bytes(&e->saveFrame, sizeof(e->saveFrame));
  const rsl::SPACE_TREE_SETUP *t = &e->spaceTreeSetup;
  d.push(t->nParamBands);
  d.push(t->bUseCoarseQuantTtoCld);
  d.push(t->bUseCoarseQuantTtoIcc);
  d.push(t->quantMode);
  d.push(t->mode);
  d.push(t->nChannelsInMax);
  d.push(t->nHybridBandsMax);
  d.push(e->sscBuf.nSscSizeBits);
  d.bytes(e->sscBuf.pSsc, 64);
  for (INT i = 0; i < 2; i++) d.ints(e->pFrameWindowAna__FDK[i], 48);
  for (INT ch = 0; ch < 2; ch++) {
    const QMF_FILTER_BANK *q = e->phQmfFiltIn__FDK[ch];
    d.push(q->FilterSize);
    d.push(q->filterScale);
    d.push(q->no_channels);
    d.push(q->no_col);
    d.push(q->lsb);
    d.push(q->usb);
    d.push(q->synScalefactor);
    d.push(q->outScalefactor);
    d.push(q->outGain_m);
    d.push(q->outGain_e);
    d.push(q->flags);
    d.push(q->p_stride);
    for (INT i = 0; i < 4; i++) d.push(q->p_filter ? q->p_filter[i] : -1);
    d.push(q->t_cos != NULL);
    d.push(q->t_sin != NULL);
    d.push(q->FilterStates != NULL);
    const SHORT *st = (const SHORT *)q->FilterStates;
    for (INT i = 0; i < 640; i++) d.push(st[i]);
  }
  for (INT ch = 0; ch < 2; ch++) {
    const rsl::DC_FILTER *f = (const rsl::DC_FILTER *)e->phDCFilterSigIn[ch];
    d.push(f->c__FDK);
    d.push(f->state__FDK);
  }
  for (INT ch = 0; ch < 2; ch++) {
    const rsl::ONSET_DETECT *o = (const rsl::ONSET_DETECT *)e->phOnset[ch];
    d.push(o->maxTimeSlots);
    d.push(o->minTransientDistance);
    d.push(o->avgEnergyDistance);
    d.push(o->lowerBoundOnsetDetection);
    d.push(o->upperBoundOnsetDetection);
    d.push(o->avgEnergyDistanceScale);
    d.ints(o->pEnergyHist__FDK, 32);
    for (INT i = 0; i < 32; i++) d.push(o->pEnergyHistScale[i]);
  }
  {
    const rsl::SPACE_TREE *st = (const rsl::SPACE_TREE *)e->hSpaceTree;
    d.push(st->mode);
    d.push(st->descr.nOttBoxes);
    d.push(st->descr.nInChannels);
    d.push(st->descr.nOutChannels);
    sacenc_tto_state(st->ttoBox[0], d.out + d.n);
    d.n += 156;
    d.push(st->nParamBands);
    d.push(st->bUseCoarseQuantTtoIcc);
    d.push(st->bUseCoarseQuantTtoCld);
    d.push(st->quantMode);
    d.push(st->frameCount);
    d.push(st->bFrameKeep);
    for (INT i = 0; i < 28; i++) d.push(st->pCld_prev[0][i]);
    for (INT i = 0; i < 28; i++) d.push(st->pIcc_prev[0][i]);
    d.push(st->nChannelsInMax);
    d.push(st->nHybridBandsMax);
  }
  {
    const rsl::BSF_INSTANCE *b = (const rsl::BSF_INSTANCE *)e->hBitstreamFormatter;
    d.ints(&b->spatialSpecificConfig, 12);
    d.bytes(&b->frame, sizeof(b->frame));
    d.bytes(&b->prevFrameData.prevOttData, sizeof(b->prevFrameData.prevOttData));
  }
  d.ints(e->hStaticGainConfig, 3);
  d.ints(e->hStaticGain, 6);
  d.ints(e->hDelay, 23);
  {
    const rsl::ENHANCED_TIME_DOMAIN_DMX *x = (const rsl::ENHANCED_TIME_DOMAIN_DMX *)e->hEnhancedTimeDmx;
    d.push(x->maxFramelength);
    d.push(x->framelength);
    d.push(x->prev_gain_m[0]);
    d.push(x->prev_gain_m[1]);
    d.push(x->prev_gain_e);
    d.push(x->prev_H1_m[0]);
    d.push(x->prev_H1_m[1]);
    d.push(x->prev_H1_e);
    d.push(x->sinusWindow_e);
    d.push(x->prev_Left_m);
    d.push(x->prev_Left_e);
    d.push(x->prev_Right_m);
    d.push(x->prev_Right_e);
    d.push(x->prev_XNrg_m);
    d.push(x->prev_XNrg_e);
    d.push(x->lin_bbCld_weight_m);
    d.push(x->lin_bbCld_weight_e);
    d.push(x->gain_weight_m[0]);
    d.push(x->gain_weight_m[1]);
    d.push(x->gain_weight_e);
    d.ints(x->sinusWindow_m, 1 + x->maxFramelength);
  }
  for (INT ch = 0; ch < 2; ch++)
    for (INT i = 0; i < 1029; i++) d.push(e->ppTimeSigIn__FDK[ch][i]);
  for (INT ch = 0; ch < 2; ch++)
    for (INT i = 0; i < 5; i++) d.push(e->ppTimeSigDelayIn__FDK[ch][i]);
  for (INT ch = 0; ch < 2; ch++)
    for (INT ts = 0; ts < 40; ts++) d.ints(e->pppHybridIn__FDK[ch][ts], 2 * 64);
  for (INT ch = 0; ch < 2; ch++)
    for (INT ts = 0; ts < 24; ts++) d.ints(e->pppHybridInStatic__FDK[ch][ts], 2 * 64);
  d.bytes(e->pParameterBand2HybridBandOffset, 28);
  d.push(e->staticGainScale);
  d.push(e->pEncoderInputChScale[0]);
  d.push(e->pEncoderInputChScale[1]);
  d.push(e->staticTimeDomainDmxInScale[0]);
  d.push(e->staticTimeDomainDmxInScale[1]);
  d.push(e->pppProcDataIn__FDK == NULL);
  d.push(e->ppTimeSigOut__FDK == NULL);
  d.push(e->pOutputDelayBuffer__FDK == NULL);
  d.push(e->ppBitstreamDelayBuffer == NULL);
  return d.n;
}

/* File-static functions */
UINT cref_sacenc_lib_GetNumQmfBands(UINT fs) { return rsl::mp4SpaceEnc_GetNumQmfBands(fs); }
UINT cref_sacenc_lib_updateQmfFlags(UINT flags, INT keep) { return rsl::updateQmfFlags(flags, keep); }
INT cref_sacenc_lib_freq2HybridBand(UINT f, UINT fs, UINT bands) { return rsl::freq2HybridBand(f, fs, bands); }
INT cref_sacenc_lib_getAnalysisLengthTimeSlots(INT *win, INT n) { return rsl::getAnalysisLengthTimeSlots((FIXP_DBL *)win, n); }
INT cref_sacenc_lib_getAnalysisStartTimeSlot(INT *win, INT n) { return rsl::getAnalysisStartTimeSlot((FIXP_DBL *)win, n); }
INT cref_sacenc_lib_mapQuantMode(INT m) { return rsl::__mapQuantMode((rsl::MP4SPACEENC_QUANTMODE)m); }
INT cref_sacenc_lib_getBufDescIdx(UINT *types, UINT n, UINT type) {
  FDK_bufDescr d;
  d.ppBase = NULL;
  d.pBufSize = NULL;
  d.pEleSize = NULL;
  d.pBufType = types;
  d.numBufs = n;
  return rsl::getBufDescIdx(&d, type);
}
}

extern "C" {
/* The file-static tables as INTs: 0 pValidBands_Ld, 1 qmf2qmf. Returns the number of
   entries. */
INT cref_sacenc_lib_table(INT id, INT *out) {
  const UCHAR *t = id == 0 ? rsl::pValidBands_Ld : rsl::qmf2qmf;
  INT n = id == 0 ? sizeof(rsl::pValidBands_Ld) : sizeof(rsl::qmf2qmf);
  for (INT i = 0; i < n; i++) out[i] = t[i];
  return n;
}
}
