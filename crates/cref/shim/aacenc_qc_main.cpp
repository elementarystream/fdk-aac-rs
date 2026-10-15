// Shims for the differential tests of fdk_aac_rs_aacenc::qc_main.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_qc_main (see build.rs).
//
// The stateful tests of FDKaacEnc_QCMain use the encoder harness (aacenc_adj_thr_enc.cpp);
// this file wraps the functions that are tested on their own.

#include "cref_common.h"
#include "libAACenc/src/aacenc.h"
#include "libAACenc/src/aacEnc_ram.h"
#include "libAACenc/src/qc_main.h"

/* QC_STATE scalars in the order of the port's tests. */
static void qc_state_get(const QC_STATE *h, INT *v) {
  v[0] = h->globHdrBits;
  v[1] = h->maxBitsPerFrame;
  v[2] = h->minBitsPerFrame;
  v[3] = h->nElements;
  v[4] = h->bitrateMode;
  v[5] = h->bitResMode;
  v[6] = h->bitResTot;
  v[7] = h->bitResTotMax;
  v[8] = h->maxIterations;
  v[9] = h->invQuant;
  v[10] = h->vbrQualFactor;
  v[11] = h->maxBitFac;
  v[12] = h->padding.paddingRest;
  v[13] = h->dZoneQuantEnable;
}

extern "C" {
/* FDKaacEnc_AdjustBitrate on a QC_STATE whose paddingRest is *paddingRest. */
INT cref_qc_main_adjust_bitrate(INT *paddingRest, INT bitRate, INT sampleRate, INT granuleLength, INT *avgTotalBits) {
  QC_STATE qc;
  FDKmemclear(&qc, sizeof(qc));
  CHANNEL_MAPPING cm;
  FDKmemclear(&cm, sizeof(cm));
  qc.padding.paddingRest = *paddingRest;
  INT e = FDKaacEnc_AdjustBitrate(&qc, &cm, avgTotalBits, bitRate, sampleRate, granuleLength);
  *paddingRest = qc.padding.paddingRest;
  return e;
}
/* FDKaacEnc_updateFillBits and FDKaacEnc_updateBitres on a QC_STATE with
   qs = {bitrateMode, bitResTotMax, bitResTot, minBitsPerFrame, maxBitsPerFrame} and a QC_OUT
   with qo = {grantedDynBits, usedDynBits, staticBits, elementExtBits, globalExtBits,
   totFillBits, totalBits, alignBits}; both are written back. which: 0 updateFillBits,
   1 updateBitres. */
INT cref_qc_main_update(INT which, INT *qs, INT *qo) {
  QC_STATE qc;
  FDKmemclear(&qc, sizeof(qc));
  QC_OUT out;
  FDKmemclear(&out, sizeof(out));
  QC_OUT *pOut[1] = {&out};
  CHANNEL_MAPPING cm;
  FDKmemclear(&cm, sizeof(cm));
  qc.bitrateMode = (QCDATA_BR_MODE)qs[0];
  qc.bitResTotMax = qs[1];
  qc.bitResTot = qs[2];
  qc.minBitsPerFrame = qs[3];
  qc.maxBitsPerFrame = qs[4];
  out.grantedDynBits = qo[0];
  out.usedDynBits = qo[1];
  out.staticBits = qo[2];
  out.elementExtBits = qo[3];
  out.globalExtBits = qo[4];
  out.totFillBits = qo[5];
  out.totalBits = qo[6];
  out.alignBits = qo[7];
  INT e = 0;
  if (which == 0)
    e = FDKaacEnc_updateFillBits(&cm, &qc, qc.elementBits, pOut);
  else
    FDKaacEnc_updateBitres(&cm, &qc, pOut);
  qs[0] = qc.bitrateMode;
  qs[1] = qc.bitResTotMax;
  qs[2] = qc.bitResTot;
  qs[3] = qc.minBitsPerFrame;
  qs[4] = qc.maxBitsPerFrame;
  qo[0] = out.grantedDynBits;
  qo[1] = out.usedDynBits;
  qo[2] = out.staticBits;
  qo[3] = out.elementExtBits;
  qo[4] = out.globalExtBits;
  qo[5] = out.totFillBits;
  qo[6] = out.totalBits;
  qo[7] = out.alignBits;
  return e;
}
/* FDKaacEnc_QCNew(nElements) + bitResTot/bitResTotMax = prev[0..2] + FDKaacEnc_QCInit with
   init = {sceCpe, maxBits, averageBits, bitRes, sampleRate, isLowDelay, staticBits,
   bitrateMode, meanPe, chBitrate, invQuant, maxIterations, maxBitFac, bitrate, nSubFrames,
   minBits, bitResMode, bitDistributionMode, paddingRest} and the channel mapping cm. Writes the
   QC_STATE scalars (14), elementBits[0..nElements], bresParamLong, bresParamShort,
   adjThrStateElem[0..nElements], adjMisc = {bitDistributionMode, maxIter2ndGuess}. Returns
   the error of FDKaacEnc_QCInit. */
INT cref_qc_main_qc_init(INT nElements, const INT *iv, const void *cm, UINT initFlags, const INT *prev, INT *scalars, void *elementBits, void *bresLong, void *bresShort, void *ats, INT *adjMisc) {
  FIXP_DBL *ram = GetAACdynamic_RAM();
  QC_STATE *h = NULL;
  INT e = FDKaacEnc_QCNew(&h, nElements, (UCHAR *)ram);
  if (e != AAC_ENC_OK) return -1;
  h->bitResTot = prev[0];
  h->bitResTotMax = prev[1];
  struct QC_INIT init;
  FDKmemclear(&init, sizeof(init));
  init.channelMapping = (CHANNEL_MAPPING *)cm;
  init.sceCpe = iv[0];
  init.maxBits = iv[1];
  init.averageBits = iv[2];
  init.bitRes = iv[3];
  init.sampleRate = iv[4];
  init.isLowDelay = iv[5];
  init.staticBits = iv[6];
  init.bitrateMode = (QCDATA_BR_MODE)iv[7];
  init.meanPe = iv[8];
  init.chBitrate = iv[9];
  init.invQuant = iv[10];
  init.maxIterations = iv[11];
  init.maxBitFac = iv[12];
  init.bitrate = iv[13];
  init.nSubFrames = iv[14];
  init.minBits = iv[15];
  init.bitResMode = (AACENC_BITRES_MODE)iv[16];
  init.bitDistributionMode = iv[17];
  init.padding.paddingRest = iv[18];
  e = FDKaacEnc_QCInit(h, &init, initFlags);
  qc_state_get(h, scalars);
  for (INT i = 0; i < nElements; i++) {
    memcpy((ELEMENT_BITS *)elementBits + i, h->elementBits[i], sizeof(ELEMENT_BITS));
    memcpy((ATS_ELEMENT *)ats + i, h->hAdjThr->adjThrStateElem[i], sizeof(ATS_ELEMENT));
  }
  memcpy(bresLong, &h->hAdjThr->bresParamLong, sizeof(BRES_PARAM));
  memcpy(bresShort, &h->hAdjThr->bresParamShort, sizeof(BRES_PARAM));
  adjMisc[0] = h->hAdjThr->bitDistributionMode;
  adjMisc[1] = h->hAdjThr->maxIter2ndGuess;
  FDKaacEnc_QCClose(&h, NULL);
  FreeAACdynamic_RAM(&ram);
  return e;
}
}
