// Shims for the differential tests of fdk_aac_rs_aacdec::aacdec_drc.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_aacdec_drc (see build.rs).
//
// aacdec_drc.cpp is compiled into a namespace so that its static functions are reachable; its
// calls of the SBR decoder are made with a NULL handle (bSbrPresent == 0), which the library's
// sbrDecoder_drcDisable() accepts.

#include "cref_common.h"
#include "tp_data.h"
#include "aacdec_drc_types.h"
#include "channel.h"
#include "channelinfo.h"
#include "aac_rom.h"
#include "sbrdecoder.h"
#include "FDK_bitstream.h"
#include <stdlib.h>

namespace ref_drc {
#include "libAACdec/src/aacdec_drc.cpp"
}

/* CDrcInfo <-> INT[37] in the order of the Rust test. */
static void drc_to(const CDrcInfo *d, INT *o) {
  INT n = 0;
  o[n++] = d->params.usrCut;
  o[n++] = d->params.usrBoost;
  o[n++] = d->params.usrApplyHeavyCompression;
  o[n++] = d->params.cut;
  o[n++] = d->params.boost;
  o[n++] = d->params.targetRefLevel;
  o[n++] = d->params.applyHeavyCompression;
  o[n++] = (INT)d->params.expiryFrame;
  o[n++] = d->params.bsDelayEnable;
  o[n++] = d->params.defaultPresentationMode;
  o[n++] = d->params.encoderTargetLevel;
  o[n++] = d->enable;
  o[n++] = d->digitalNorm;
  o[n++] = d->update;
  o[n++] = d->numOutChannels;
  o[n++] = d->prevAacNumChannels;
  o[n++] = d->numPayloads;
  o[n++] = d->numThreads;
  o[n++] = d->progRefLevel;
  o[n++] = d->progRefLevelPresent;
  o[n++] = (INT)d->prlExpiryCount;
  o[n++] = d->presMode;
  o[n++] = d->dvbAncDataAvailable;
  o[n++] = (INT)d->dvbAncDataPosition;
  for (int i = 0; i < MAX_DRC_THREADS; i++) o[n++] = (INT)d->drcPayloadPosition[i];
  o[n++] = d->applyExtGain;
  o[n++] = d->additionalGainPrev;
  o[n++] = d->additionalGainFilterState;
  o[n++] = d->additionalGainFilterState1;
}

static void drc_from(CDrcInfo *d, const INT *o) {
  INT n = 0;
  d->params.usrCut = o[n++];
  d->params.usrBoost = o[n++];
  d->params.usrApplyHeavyCompression = (UCHAR)o[n++];
  d->params.cut = o[n++];
  d->params.boost = o[n++];
  d->params.targetRefLevel = (SCHAR)o[n++];
  d->params.applyHeavyCompression = (UCHAR)o[n++];
  d->params.expiryFrame = (UINT)o[n++];
  d->params.bsDelayEnable = (UCHAR)o[n++];
  d->params.defaultPresentationMode = (AACDEC_DRC_PARAMETER_HANDLING)o[n++];
  d->params.encoderTargetLevel = (UCHAR)o[n++];
  d->enable = (UCHAR)o[n++];
  d->digitalNorm = (UCHAR)o[n++];
  d->update = (UCHAR)o[n++];
  d->numOutChannels = o[n++];
  d->prevAacNumChannels = o[n++];
  d->numPayloads = (USHORT)o[n++];
  d->numThreads = (USHORT)o[n++];
  d->progRefLevel = (SCHAR)o[n++];
  d->progRefLevelPresent = (UCHAR)o[n++];
  d->prlExpiryCount = (UINT)o[n++];
  d->presMode = (SCHAR)o[n++];
  d->dvbAncDataAvailable = (UCHAR)o[n++];
  d->dvbAncDataPosition = (UINT)o[n++];
  for (int i = 0; i < MAX_DRC_THREADS; i++) d->drcPayloadPosition[i] = (UINT)o[n++];
  d->applyExtGain = (UCHAR)o[n++];
  d->additionalGainPrev = o[n++];
  d->additionalGainFilterState = o[n++];
  d->additionalGainFilterState1 = o[n++];
}

/* CDrcChannelData <-> INT[37]: expiryCount, numBands, bandTop[16], drcInterpolationScheme,
   drcValue[16], drcDataType. */
static void ch_to(const CDrcChannelData *c, INT *o) {
  INT n = 0;
  o[n++] = (INT)c->expiryCount;
  o[n++] = (INT)c->numBands;
  for (int i = 0; i < MAX_DRC_BANDS; i++) o[n++] = c->bandTop[i];
  o[n++] = c->drcInterpolationScheme;
  for (int i = 0; i < MAX_DRC_BANDS; i++) o[n++] = c->drcValue[i];
  o[n++] = c->drcDataType;
}

static void ch_from(CDrcChannelData *c, const INT *o) {
  INT n = 0;
  c->expiryCount = (UINT)o[n++];
  c->numBands = (UINT)o[n++];
  for (int i = 0; i < MAX_DRC_BANDS; i++) c->bandTop[i] = (USHORT)o[n++];
  c->drcInterpolationScheme = (SHORT)o[n++];
  for (int i = 0; i < MAX_DRC_BANDS; i++) c->drcValue[i] = (UCHAR)o[n++];
  c->drcDataType = (SCHAR)o[n++];
}

extern "C" {

INT cref_drc_convert_param(INT v) { return ref_drc::convert_drcParam(v); }

void cref_drc_init(INT *st) {
  CDrcInfo d;
  memset(&d, 0, sizeof(d));
  drc_from(&d, st);
  ref_drc::aacDecoder_drcInit(&d);
  drc_to(&d, st);
}

void cref_drc_disable_reset(INT *st, INT which) {
  CDrcInfo d;
  drc_from(&d, st);
  if (which) ref_drc::aacDecoder_drcReset(&d);
  else ref_drc::aacDecoder_drcDisable(&d);
  drc_to(&d, st);
}

void cref_drc_init_channel_data(INT *ch) {
  CDrcChannelData c;
  ch_from(&c, ch);
  ref_drc::aacDecoder_drcInitChannelData(&c);
  ch_to(&c, ch);
}

INT cref_drc_set_param(INT *st, INT null_handle, INT param, INT value) {
  CDrcInfo d;
  INT r;
  drc_from(&d, st);
  r = ref_drc::aacDecoder_drcSetParam(null_handle ? NULL : &d, (ref_drc::AACDEC_DRC_PARAM)param, value);
  drc_to(&d, st);
  return r;
}

void cref_drc_parameter_handling(INT *st, INT aacNumChannels, INT prevPRL, INT prevPM) {
  CDrcInfo d;
  drc_from(&d, st);
  ref_drc::aacDecoder_drcParameterHandling(&d, aacNumChannels, (SCHAR)prevPRL, (SCHAR)prevPM);
  drc_to(&d, st);
}

void cref_drc_get_info(INT *st, INT *presMode, INT *progRefLevel) {
  CDrcInfo d;
  SCHAR pm = -5, prl = -5;
  drc_from(&d, st);
  ref_drc::aacDecoder_drcGetInfo(&d, &pm, &prl);
  *presMode = pm;
  *progRefLevel = prl;
}

/* Runs a bitstream function on buf (len bytes, starting at bit offset `skip`): which 0:
   drcMarkPayload(MPEG), 1: drcMarkPayload(DVB), 2: drcParse, 3: drcReadCompression,
   4: parseExcludedChannels. pos is the payload position for 2 and 3 (in valid bits). Returns
   the function's value; *bits receives the valid bits after the call; pl the CDrcPayload as
   excludedChnsMask, progRefLevel, presMode, pceInstanceTag + CDrcChannelData. */
INT cref_drc_bitstream(INT which, UCHAR *buf, INT len, INT skip, UINT pos, INT *st, INT *pl, INT *bits) {
  FDK_BITSTREAM bs;
  CDrcInfo d;
  CDrcPayload p;
  INT r = 0;
  UINT mask = 0;
  drc_from(&d, st);
  memset(&p, 0, sizeof(p));
  p.excludedChnsMask = (UINT)pl[0];
  p.progRefLevel = (SCHAR)pl[1];
  p.presMode = (SCHAR)pl[2];
  p.pceInstanceTag = (SCHAR)pl[3];
  ch_from(&p.channelData, pl + 4);
  FDKinitBitStream(&bs, buf, 64, (UINT)len * 8, BS_READER); /* buf holds 64 bytes */
  FDKpushFor(&bs, skip);
  switch (which) {
    case 0: r = ref_drc::aacDecoder_drcMarkPayload(&d, &bs, MPEG_DRC_EXT_DATA); break;
    case 1: r = ref_drc::aacDecoder_drcMarkPayload(&d, &bs, DVB_DRC_ANC_DATA); break;
    case 2: r = ref_drc::aacDecoder_drcParse(&bs, &p, pos); break;
    case 3: r = ref_drc::aacDecoder_drcReadCompression(&bs, &p, pos); break;
    default:
      r = ref_drc::parseExcludedChannels(&mask, &bs);
      p.excludedChnsMask = mask;
      break;
  }
  *bits = (INT)FDKgetValidBits(&bs);
  drc_to(&d, st);
  pl[0] = (INT)p.excludedChnsMask;
  pl[1] = p.progRefLevel;
  pl[2] = p.presMode;
  pl[3] = p.pceInstanceTag;
  ch_to(&p.channelData, pl + 4);
  return r;
}

/* aacDecoder_drcApply without SBR on a channel with the given spectrum (1024 values), specScale
   (8) and window sequence. extGain: in/out (-1000: NULL). */
void cref_drc_apply(INT *st, INT *ch, FIXP_DBL *spec, SHORT *specScale, INT winSeq, INT *extGain, INT c, INT frameSize) {
  CDrcInfo d;
  CDrcChannelData cd;
  CAacDecoderChannelInfo *ci = (CAacDecoderChannelInfo *)calloc(1, sizeof(*ci));
  FIXP_DBL g = *extGain;
  drc_from(&d, st);
  ch_from(&cd, ch);
  ci->pSpectralCoefficient = (SPECTRAL_PTR)spec;
  memcpy(ci->specScale, specScale, sizeof(ci->specScale));
  ci->icsInfo.WindowSequence = (BLOCK_TYPE)winSeq;
  ref_drc::aacDecoder_drcApply(&d, NULL, ci, &cd, *extGain == -1000 ? NULL : &g, c, frameSize,
                               0);
  if (*extGain != -1000) *extGain = g;
  memcpy(specScale, ci->specScale, sizeof(ci->specScale));
  free(ci);
  drc_to(&d, st);
  ch_to(&cd, ch);
}

INT cref_drc_apply_level_normalization(INT *st, FIXP_DBL *samples, FIXP_DBL gain, FIXP_DBL *gainPerSample, INT gain_scale, UINT gain_delay, UINT nSamples, UINT channels, UINT stride, UINT limiterEnabled) {
  CDrcInfo d;
  INT r;
  FIXP_DBL g[1] = {gain};
  drc_from(&d, st);
  r = ref_drc::applyDrcLevelNormalization(&d, samples, g, limiterEnabled ? gainPerSample : NULL,
                                          gain_scale, gain_delay, nSamples, channels, stride,
                                          limiterEnabled);
  drc_to(&d, st);
  return r;
}
}
