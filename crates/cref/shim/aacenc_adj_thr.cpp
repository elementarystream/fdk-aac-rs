// Shims for the differential tests of fdk_aac_rs_aacenc::adj_thr.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_adj_thr (see build.rs).
//
// The static functions of adj_thr.cpp are reached by including the file into the namespace
// ref_adj_thr (PORTING.md, "Differential tests"); the public functions are called in the
// library.
//
// Data exchange: QC_OUT_CHANNEL, PE_DATA, ATS_ELEMENT, TOOLSINFO, AH_PARAM, MINSNR_ADAPT_PARAM,
// BRES_PARAM and CHANNEL_MAPPING have the same layout in the port (checked by the qc_data layout
// test) and are passed as pointers. A PSY_OUT_CHANNEL is passed as the port's PsyOutChannel (the C
// struct up to its six pointers); the shim builds the C struct and points the six pointers to the
// QC_OUT_CHANNEL of the same channel, as FDKaacEnc_EncodeFrame() does.
//
// Multi-element functions get the channels of all elements as one array (assigned to the
// elements consecutively, as FDKaacEnc_QCOutInit() / FDKaacEnc_psyInit() do), PE_DATA[nElements],
// TOOLSINFO[nElements] and per element the scalars staticBitsUsed, extBitsUsed, grantedPe,
// grantedPeCorr, commonWindow (INT[5 * nElements]). The dynamic RAM buffers dynMem_* are
// allocated per call and filled with the byte `poison` (0 where there is no poison parameter),
// so that a test can check that the result does not depend on their earlier contents; their
// contents after the call are copied to `dynOut` (ADJ_THR_SIZE bytes) unless it is NULL.

#include "cref_common.h"
#include "libAACenc/src/adj_thr.h"
#include "libAACenc/src/sf_estim.h"
#include "libAACenc/src/aacEnc_ram.h"
#include "libAACenc/src/channel_map.h"

#include <stddef.h>
#include <stdlib.h>

namespace ref_adj_thr {
#include "libAACenc/src/adj_thr.cpp"
}

/* The library function (after the namespace include, so that the copy does not see it);
   defined in adj_thr.cpp without a declaration in adj_thr.h. */
void FDKaacEnc_bitresCalcBitFac(const INT bitresBits, const INT maxBitresBits, const INT pe,
                                const INT lastWindowSequence, const INT avgBits,
                                const FIXP_DBL maxBitFac, const ADJ_THR_STATE *const AdjThr,
                                ATS_ELEMENT *const adjThrChan, FIXP_DBL *const pBitresFac,
                                INT *const pBitresFac_e);

#define PSY_PREFIX offsetof(PSY_OUT_CHANNEL, mdctSpectrum)

static void mkPsy(PSY_OUT_CHANNEL *dst, const void *prefix, QC_OUT_CHANNEL *qc) {
  FDKmemcpy(dst, prefix, PSY_PREFIX);
  if (qc != NULL) {
    dst->mdctSpectrum = qc->mdctSpectrum;
    dst->sfbSpreadEnergy = qc->sfbSpreadEnergy;
    dst->sfbEnergy = qc->sfbEnergy;
    dst->sfbEnergyLdData = qc->sfbEnergyLdData;
    dst->sfbMinSnrLdData = qc->sfbMinSnrLdData;
    dst->sfbThresholdLdData = qc->sfbThresholdLdData;
  } else {
    dst->mdctSpectrum = NULL;
    dst->sfbSpreadEnergy = NULL;
    dst->sfbEnergy = NULL;
    dst->sfbEnergyLdData = NULL;
    dst->sfbMinSnrLdData = NULL;
    dst->sfbThresholdLdData = NULL;
  }
}

/* The channels of one element. */
struct El {
  PSY_OUT_CHANNEL psy[2];
  PSY_OUT_CHANNEL *pp[2];
  QC_OUT_CHANNEL *qp[2];
};

static void mkEl(El *e, const void *psy, void *qc, INT n) {
  for (int ch = 0; ch < 2; ch++) {
    e->pp[ch] = NULL;
    e->qp[ch] = NULL;
  }
  for (int ch = 0; ch < n; ch++) {
    e->qp[ch] = (qc != NULL) ? (QC_OUT_CHANNEL *)qc + ch : NULL;
    mkPsy(&e->psy[ch], (const UCHAR *)psy + ch * PSY_PREFIX, e->qp[ch]);
    e->pp[ch] = &e->psy[ch];
  }
}

/* Several elements (see the file header). */
struct Multi {
  QC_OUT qcOut;
  QC_OUT_ELEMENT qcEl[8];
  QC_OUT_ELEMENT *qcElP[8];
  PSY_OUT_ELEMENT psyEl[8];
  const PSY_OUT_ELEMENT *psyElP[8];
  PSY_OUT_CHANNEL psyCh[8];
  UCHAR *dyn;
};

static void mkMulti(Multi *m, const CHANNEL_MAPPING *cm, void *qcCh, const void *psy,
                    const void *peData, const void *tools, const INT *elScal, INT poison) {
  FDKmemclear(m, sizeof(*m));
  m->dyn = (UCHAR *)malloc(ADJ_THR_SIZE);
  memset(m->dyn, poison, ADJ_THR_SIZE);
  INT chInc = 0;
  for (int i = 0; i < cm->nElements; i++) {
    QC_OUT_ELEMENT *q = &m->qcEl[i];
    PSY_OUT_ELEMENT *p = &m->psyEl[i];
    m->qcElP[i] = q;
    m->psyElP[i] = p;
    m->qcOut.qcElement[i] = q;
    q->dynMem_Ah_Flag = m->dyn;
    q->dynMem_Thr_Exp = m->dyn + ADJ_THR_AH_FLAG_SIZE;
    q->dynMem_SfbNActiveLinesLdData = m->dyn + ADJ_THR_AH_FLAG_SIZE + ADJ_THR_THR_EXP_SIZE;
    FDKmemcpy(&q->peData, (const PE_DATA *)peData + i, sizeof(PE_DATA));
    q->staticBitsUsed = elScal[5 * i + 0];
    q->extBitsUsed = elScal[5 * i + 1];
    q->grantedPe = elScal[5 * i + 2];
    q->grantedPeCorr = elScal[5 * i + 3];
    p->commonWindow = elScal[5 * i + 4];
    FDKmemcpy(&p->toolsInfo, (const struct TOOLSINFO *)tools + i, sizeof(struct TOOLSINFO));
    for (int ch = 0; ch < cm->elInfo[i].nChannelsInEl; ch++) {
      QC_OUT_CHANNEL *qc = (QC_OUT_CHANNEL *)qcCh + chInc;
      m->qcOut.pQcOutChannels[chInc] = qc;
      q->qcOutChannel[ch] = qc;
      mkPsy(&m->psyCh[chInc], (const UCHAR *)psy + chInc * PSY_PREFIX, qc);
      p->psyOutChannel[ch] = &m->psyCh[chInc];
      chInc++;
    }
  }
}

static void unMulti(Multi *m, const CHANNEL_MAPPING *cm, void *peData, INT *elScal,
                    UCHAR *dynOut) {
  if (dynOut != NULL) FDKmemcpy(dynOut, m->dyn, ADJ_THR_SIZE);
  for (int i = 0; i < cm->nElements; i++) {
    FDKmemcpy((PE_DATA *)peData + i, &m->qcEl[i].peData, sizeof(PE_DATA));
    elScal[5 * i + 0] = m->qcEl[i].staticBitsUsed;
    elScal[5 * i + 1] = m->qcEl[i].extBitsUsed;
    elScal[5 * i + 2] = m->qcEl[i].grantedPe;
    elScal[5 * i + 3] = m->qcEl[i].grantedPeCorr;
    elScal[5 * i + 4] = m->psyEl[i].commonWindow;
  }
  free(m->dyn);
}

static void mkAdjThr(ADJ_THR_STATE *s, const void *bres, void *ats, INT nAts, INT bdm,
                     INT maxIter) {
  FDKmemclear(s, sizeof(*s));
  FDKmemcpy(&s->bresParamLong, (const BRES_PARAM *)bres, sizeof(BRES_PARAM));
  FDKmemcpy(&s->bresParamShort, (const BRES_PARAM *)bres + 1, sizeof(BRES_PARAM));
  for (int i = 0; i < nAts; i++) s->adjThrStateElem[i] = (ATS_ELEMENT *)ats + i;
  s->bitDistributionMode = (AACENC_BIT_DISTRIBUTION_MODE)bdm;
  s->maxIter2ndGuess = maxIter;
}

extern "C" {
/* FDKaacEnc_InitChannelMapping(mode, CH_ORDER_MPEG) into a CHANNEL_MAPPING. */
INT cref_adj_thr_channel_mapping(INT mode, void *cm) {
  return FDKaacEnc_InitChannelMapping((CHANNEL_MODE)mode, CH_ORDER_MPEG, (CHANNEL_MAPPING *)cm);
}
/* The file-local tables of adj_thr.cpp: 0 invInt, 1 invSqrt4, 2..8 SnrLdMin1, SnrLdMin2,
   SnrLdFac, SnrLdMin3, SnrLdMin4, SnrLdMin5, limitThrReducedLdData, 9..14 the pPeTab of
   bits2PeConfigTab[0..5] (BIT_PE_SFAC, 5 words each), 15 the sampleRate of bits2PeConfigTab[] (6
   words, copied). Returns the address, *nWords the number of 32-bit words. */
const INT *cref_adj_thr_table(INT i, INT *nWords) {
  using namespace ref_adj_thr;
  static INT rates[6];
  const INT n = (INT)(sizeof(bits2PeConfigTab) / sizeof(bits2PeConfigTab[0]));
  switch (i) {
    case 0: *nWords = INV_INT_TAB_SIZE; return (const INT *)invInt;
    case 1: *nWords = INV_SQRT4_TAB_SIZE; return (const INT *)invSqrt4;
    case 2: *nWords = 1; return (const INT *)&SnrLdMin1;
    case 3: *nWords = 1; return (const INT *)&SnrLdMin2;
    case 4: *nWords = 1; return (const INT *)&SnrLdFac;
    case 5: *nWords = 1; return (const INT *)&SnrLdMin3;
    case 6: *nWords = 1; return (const INT *)&SnrLdMin4;
    case 7: *nWords = 1; return (const INT *)&SnrLdMin5;
    case 8: *nWords = 1; return (const INT *)&limitThrReducedLdData;
    case 15:
      for (int k = 0; k < n; k++) rates[k] = bits2PeConfigTab[k].sampleRate;
      *nWords = n;
      return rates;
    default:
      if (i >= 9 && i < 9 + n) {
        *nWords = bits2PeConfigTab[i - 9].nEntries * (INT)(sizeof(BIT_PE_SFAC) / sizeof(INT));
        return (const INT *)bits2PeConfigTab[i - 9].pPeTab;
      }
      *nWords = 0;
      return NULL;
  }
}
/* ADJ_THR_SIZE (the dynMem_* buffers). */
INT cref_adj_thr_dyn_size() { return (INT)ADJ_THR_SIZE; }
/* Size of the PSY_OUT_CHANNEL prefix (up to the pointers). */
INT cref_adj_thr_psy_prefix_size() { return (INT)PSY_PREFIX; }

/* FDKaacEnc_InitBits2PeFactor */
void cref_adj_thr_InitBits2PeFactor(FIXP_DBL *m, INT *e, INT bitRate, INT nChannels, INT sampleRate, INT advancedBitsToPe, INT dZoneQuantEnable, INT invQuant) {
  ref_adj_thr::FDKaacEnc_InitBits2PeFactor(m, e, bitRate, nChannels, sampleRate, advancedBitsToPe,
                                           dZoneQuantEnable, invQuant);
}
/* FDKaacEnc_bits2pe2 */
INT cref_adj_thr_bits2pe2(INT bits, FIXP_DBL factor_m, INT factor_e) {
  return ref_adj_thr::FDKaacEnc_bits2pe2(bits, factor_m, factor_e);
}
/* FDKaacEnc_calcThreshExp: thrExp[2][60] */
void cref_adj_thr_calcThreshExp(FIXP_DBL *thrExp, void *qc, const void *psy, INT n) {
  El e;
  mkEl(&e, psy, qc, n);
  ref_adj_thr::FDKaacEnc_calcThreshExp((FIXP_DBL(*)[MAX_GROUPED_SFB])thrExp, e.qp, e.pp, n);
}
/* FDKaacEnc_adaptMinSnr */
void cref_adj_thr_adaptMinSnr(void *qc, const void *psy, const void *msaParam, INT n) {
  El e;
  mkEl(&e, psy, qc, n);
  ref_adj_thr::FDKaacEnc_adaptMinSnr(e.qp, e.pp, (const MINSNR_ADAPT_PARAM *)msaParam, n);
}
/* FDKaacEnc_initAvoidHoleFlag: ahFlag[2][60] */
void cref_adj_thr_initAvoidHoleFlag(void *qc, const void *psy, UCHAR *ahFlag, const void *toolsInfo, INT n, const void *ahParam) {
  El e;
  mkEl(&e, psy, qc, n);
  ref_adj_thr::FDKaacEnc_initAvoidHoleFlag(e.qp, e.pp, (UCHAR(*)[MAX_GROUPED_SFB])ahFlag,
                                           (const struct TOOLSINFO *)toolsInfo, n,
                                           (const AH_PARAM *)ahParam);
}
/* FDKaacEnc_preparePe */
void cref_adj_thr_preparePe(void *peData, const void *psy, void *qc, INT n, INT peOffset) {
  El e;
  mkEl(&e, psy, qc, n);
  ref_adj_thr::FDKaacEnc_preparePe((PE_DATA *)peData, e.pp, e.qp, n, peOffset);
}
/* FDKaacEnc_calcWeighting */
void cref_adj_thr_calcWeighting(const void *peData, const void *psy, void *qc, const void *toolsInfo, void *ats, INT n, INT usePatchTool) {
  El e;
  mkEl(&e, psy, qc, n);
  ref_adj_thr::FDKaacEnc_calcWeighting((const PE_DATA *)peData, e.pp, e.qp,
                                       (const struct TOOLSINFO *)toolsInfo, (ATS_ELEMENT *)ats,
                                       n, usePatchTool);
}
/* FDKaacEnc_calcPe */
void cref_adj_thr_calcPe(const void *psy, void *qc, void *peData, INT n) {
  El e;
  mkEl(&e, psy, qc, n);
  ref_adj_thr::FDKaacEnc_calcPe(e.pp, e.qp, (PE_DATA *)peData, n);
}
/* FDKaacEnc_peCalculation (library) */
void cref_adj_thr_peCalculation(void *peData, const void *psy, void *qc, const void *toolsInfo, void *ats, INT n) {
  El e;
  mkEl(&e, psy, qc, n);
  FDKaacEnc_peCalculation((PE_DATA *)peData, e.pp, e.qp, (const struct TOOLSINFO *)toolsInfo,
                          (ATS_ELEMENT *)ats, n);
}
/* FDKaacEnc_FDKaacEnc_calcPeNoAH: out = {pe, constPart, nActiveLines} */
void cref_adj_thr_calcPeNoAH(INT *out, const void *peData, const UCHAR *ahFlag, const void *psy, INT n) {
  El e;
  mkEl(&e, psy, NULL, n);
  ref_adj_thr::FDKaacEnc_FDKaacEnc_calcPeNoAH(&out[0], &out[1], &out[2], (const PE_DATA *)peData,
                                              (const UCHAR(*)[MAX_GROUPED_SFB])ahFlag, e.pp, n);
}
/* FDKaacEnc_reduceThresholdsCBR */
void cref_adj_thr_reduceThresholdsCBR(void *qc, const void *psy, UCHAR *ahFlag, const FIXP_DBL *thrExp, INT n, FIXP_DBL redVal_m, INT redVal_e) {
  El e;
  mkEl(&e, psy, qc, n);
  ref_adj_thr::FDKaacEnc_reduceThresholdsCBR(
      e.qp, e.pp, (UCHAR(*)[MAX_GROUPED_SFB])ahFlag, (const FIXP_DBL(*)[MAX_GROUPED_SFB])thrExp,
      n, redVal_m, (SCHAR)redVal_e);
}
/* FDKaacEnc_calcChaosMeasure */
FIXP_DBL cref_adj_thr_calcChaosMeasure(const void *psy, void *qc) {
  El e;
  mkEl(&e, psy, qc, 1);
  return ref_adj_thr::FDKaacEnc_calcChaosMeasure(e.pp[0], e.qp[0]->sfbFormFactorLdData);
}
/* FDKaacEnc_reduceThresholdsVBR */
void cref_adj_thr_reduceThresholdsVBR(void *qc, const void *psy, UCHAR *ahFlag, const FIXP_DBL *thrExp, INT n, FIXP_DBL vbrQualFactor, FIXP_DBL *chaosMeasureOld) {
  El e;
  mkEl(&e, psy, qc, n);
  ref_adj_thr::FDKaacEnc_reduceThresholdsVBR(
      e.qp, e.pp, (UCHAR(*)[MAX_GROUPED_SFB])ahFlag, (const FIXP_DBL(*)[MAX_GROUPED_SFB])thrExp,
      n, vbrQualFactor, chaosMeasureOld);
}
/* FDKaacEnc_resetAHFlags */
void cref_adj_thr_resetAHFlags(UCHAR *ahFlag, INT n, const void *psy) {
  El e;
  mkEl(&e, psy, NULL, n);
  ref_adj_thr::FDKaacEnc_resetAHFlags((UCHAR(*)[MAX_GROUPED_SFB])ahFlag, n, e.pp);
}
/* CalcRedValPower */
FIXP_DBL cref_adj_thr_CalcRedValPower(FIXP_DBL num, FIXP_DBL denum, INT *scaling) {
  return ref_adj_thr::CalcRedValPower(num, denum, scaling);
}
/* FDKaacEnc_AdaptThresholdsVBR */
void cref_adj_thr_AdaptThresholdsVBR(void *qc, const void *psy, void *ats, const void *toolsInfo, INT n) {
  El e;
  mkEl(&e, psy, qc, n);
  ref_adj_thr::FDKaacEnc_AdaptThresholdsVBR(e.qp, e.pp, (ATS_ELEMENT *)ats,
                                            (const struct TOOLSINFO *)toolsInfo, n);
}
/* FDKaacEnc_calcBitSave */
FIXP_DBL cref_adj_thr_calcBitSave(FIXP_DBL fillLevel, FIXP_DBL clipLow, FIXP_DBL clipHigh, FIXP_DBL minBitSave, FIXP_DBL maxBitSave, FIXP_DBL slope) {
  return ref_adj_thr::FDKaacEnc_calcBitSave(fillLevel, clipLow, clipHigh, minBitSave, maxBitSave,
                                            slope);
}
/* FDKaacEnc_calcBitSpend */
FIXP_DBL cref_adj_thr_calcBitSpend(FIXP_DBL fillLevel, FIXP_DBL clipLow, FIXP_DBL clipHigh, FIXP_DBL minBitSpend, FIXP_DBL maxBitSpend, FIXP_DBL slope) {
  return ref_adj_thr::FDKaacEnc_calcBitSpend(fillLevel, clipLow, clipHigh, minBitSpend,
                                             maxBitSpend, slope);
}
/* FDKaacEnc_adjustPeMinMax */
void cref_adj_thr_adjustPeMinMax(INT currPe, INT *peMin, INT *peMax) {
  ref_adj_thr::FDKaacEnc_adjustPeMinMax(currPe, peMin, peMax);
}
/* FDKaacEnc_bitresCalcBitFac (library); bres = BRES_PARAM[2] (long, short) */
void cref_adj_thr_bitresCalcBitFac(INT bitresBits, INT maxBitresBits, INT pe, INT lastWindowSequence, INT avgBits, FIXP_DBL maxBitFac, const void *bres, void *ats, FIXP_DBL *fac, INT *fac_e) {
  ADJ_THR_STATE s;
  mkAdjThr(&s, bres, ats, 1, 0, 0);
  ::FDKaacEnc_bitresCalcBitFac(bitresBits, maxBitresBits, pe, lastWindowSequence, avgBits, maxBitFac,
                             &s, (ATS_ELEMENT *)ats, fac, fac_e);
}
/* FDKaacEnc_AdjThrNew + FDKaacEnc_AdjThrInit + FDKaacEnc_AdjThrClose (library): out =
   BRES_PARAM[2], ATS_ELEMENT[nElements], modeIter = {bitDistributionMode, maxIter2ndGuess} */
INT cref_adj_thr_AdjThrInit(void *bresOut, void *atsOut, INT *modeIter, INT nElements, INT meanPe, INT invQuant, const void *cm, INT sampleRate, INT totalBitrate, INT isLowDelay, INT bitResMode, INT dZoneQuantEnable, INT bitDistributionMode, FIXP_DBL vbrQualFactor) {
  ADJ_THR_STATE *h = NULL;
  INT err = FDKaacEnc_AdjThrNew(&h, nElements);
  if (err == 0) {
    FDKaacEnc_AdjThrInit(h, meanPe, invQuant, (const CHANNEL_MAPPING *)cm, sampleRate,
                         totalBitrate, isLowDelay, (AACENC_BITRES_MODE)bitResMode,
                         dZoneQuantEnable, bitDistributionMode, vbrQualFactor);
    FDKmemcpy(bresOut, &h->bresParamLong, sizeof(BRES_PARAM));
    FDKmemcpy((BRES_PARAM *)bresOut + 1, &h->bresParamShort, sizeof(BRES_PARAM));
    for (int i = 0; i < nElements; i++)
      FDKmemcpy((ATS_ELEMENT *)atsOut + i, h->adjThrStateElem[i], sizeof(ATS_ELEMENT));
    modeIter[0] = h->bitDistributionMode;
    modeIter[1] = h->maxIter2ndGuess;
  }
  FDKaacEnc_AdjThrClose(&h);
  return err;
}
/* FDKaacEnc_FDKaacEnc_calcPeCorrection */
void cref_adj_thr_calcPeCorrection(FIXP_DBL *m, INT *e, INT peAct, INT peLast, INT bitsLast, FIXP_DBL bits2PeFactor_m, INT bits2PeFactor_e) {
  ref_adj_thr::FDKaacEnc_FDKaacEnc_calcPeCorrection(m, e, peAct, peLast, bitsLast,
                                                    bits2PeFactor_m, bits2PeFactor_e);
}
/* FDKaacEnc_calcPeCorrectionLowBitRes */
void cref_adj_thr_calcPeCorrectionLowBitRes(FIXP_DBL *m, INT *e, INT peLast, INT bitsLast, INT bitresLevel, INT nChannels, FIXP_DBL bits2PeFactor_m, INT bits2PeFactor_e) {
  ref_adj_thr::FDKaacEnc_calcPeCorrectionLowBitRes(m, e, peLast, bitsLast, bitresLevel, nChannels,
                                                   bits2PeFactor_m, bits2PeFactor_e);
}
/* FDKaacEnc_DistributeBits (library); bres = BRES_PARAM[2]; out = {grantedPe, grantedPeCorr} */
void cref_adj_thr_DistributeBits(const void *bres, void *ats, const void *psy, void *peData, INT *out, INT n, INT commonWindow, INT avgBits, INT bitresBits, INT maxBitresBits, FIXP_DBL maxBitFac, INT bitResMode) {
  ADJ_THR_STATE s;
  El e;
  mkAdjThr(&s, bres, ats, 1, 0, 0);
  mkEl(&e, psy, NULL, n);
  FDKaacEnc_DistributeBits(&s, (ATS_ELEMENT *)ats, e.pp, (PE_DATA *)peData, &out[0], &out[1], n,
                           commonWindow, avgBits, bitresBits, maxBitresBits, maxBitFac,
                           (AACENC_BITRES_MODE)bitResMode);
}
/* FDKaacEnc_correctThresh: ahFlag[8][2][60], thrExp[8][2][60] */
void cref_adj_thr_correctThresh(const void *cm, void *qcCh, const void *psy, void *peData, const void *tools, INT *elScal, UCHAR *ahFlag, const FIXP_DBL *thrExp, FIXP_DBL redVal_m, INT redVal_e, INT deltaPe, INT processElements, INT elementOffset, INT poison, UCHAR *dynOut) {
  const CHANNEL_MAPPING *c = (const CHANNEL_MAPPING *)cm;
  Multi *m = (Multi *)malloc(sizeof(Multi));
  mkMulti(m, c, qcCh, psy, peData, tools, elScal, poison);
  ref_adj_thr::FDKaacEnc_correctThresh(
      c, m->qcElP, m->psyElP, (UCHAR(*)[2][MAX_GROUPED_SFB])ahFlag,
      (const FIXP_DBL(*)[2][MAX_GROUPED_SFB])thrExp, redVal_m, (SCHAR)redVal_e, deltaPe,
      processElements, elementOffset);
  unMulti(m, c, peData, elScal, dynOut);
  free(m);
}
/* FDKaacEnc_reduceMinSnr: ahFlag[8][2][60] */
void cref_adj_thr_reduceMinSnr(const void *cm, void *qcCh, const void *psy, void *peData, const void *tools, INT *elScal, const UCHAR *ahFlag, INT desiredPe, INT *redPeGlobal, INT processElements, INT elementOffset, UCHAR *dynOut) {
  const CHANNEL_MAPPING *c = (const CHANNEL_MAPPING *)cm;
  Multi *m = (Multi *)malloc(sizeof(Multi));
  mkMulti(m, c, qcCh, psy, peData, tools, elScal, 0);
  ref_adj_thr::FDKaacEnc_reduceMinSnr(c, m->qcElP, m->psyElP,
                                      (const UCHAR(*)[2][MAX_GROUPED_SFB])ahFlag, desiredPe,
                                      redPeGlobal, processElements, elementOffset);
  unMulti(m, c, peData, elScal, dynOut);
  free(m);
}
/* FDKaacEnc_allowMoreHoles: ats = ATS_ELEMENT[nElements], ahFlag[8][2][60] */
void cref_adj_thr_allowMoreHoles(const void *cm, void *qcCh, const void *psy, void *peData, const void *tools, INT *elScal, const void *ats, UCHAR *ahFlag, INT desiredPe, INT currentPe, INT processElements, INT elementOffset, UCHAR *dynOut) {
  const CHANNEL_MAPPING *c = (const CHANNEL_MAPPING *)cm;
  Multi *m = (Multi *)malloc(sizeof(Multi));
  const ATS_ELEMENT *a[8];
  for (int i = 0; i < 8; i++) a[i] = (i < c->nElements) ? (const ATS_ELEMENT *)ats + i : NULL;
  mkMulti(m, c, qcCh, psy, peData, tools, elScal, 0);
  ref_adj_thr::FDKaacEnc_allowMoreHoles(c, m->qcElP, m->psyElP, a,
                                        (UCHAR(*)[2][MAX_GROUPED_SFB])ahFlag, desiredPe,
                                        currentPe, processElements, elementOffset);
  unMulti(m, c, peData, elScal, dynOut);
  free(m);
}
/* FDKaacEnc_adaptThresholdsToPe: ats = ATS_ELEMENT[nElements] */
void cref_adj_thr_adaptThresholdsToPe(const void *cm, void *qcCh, const void *psy, void *peData, const void *tools, INT *elScal, void *ats, INT desiredPe, INT maxIter2ndGuess, INT processElements, INT elementOffset, INT poison, UCHAR *dynOut) {
  const CHANNEL_MAPPING *c = (const CHANNEL_MAPPING *)cm;
  Multi *m = (Multi *)malloc(sizeof(Multi));
  ATS_ELEMENT *a[8];
  for (int i = 0; i < 8; i++) a[i] = (i < c->nElements) ? (ATS_ELEMENT *)ats + i : NULL;
  mkMulti(m, c, qcCh, psy, peData, tools, elScal, poison);
  ref_adj_thr::FDKaacEnc_adaptThresholdsToPe(c, a, m->qcElP, m->psyElP, desiredPe,
                                             maxIter2ndGuess, processElements, elementOffset);
  unMulti(m, c, peData, elScal, dynOut);
  free(m);
}
/* FDKaacEnc_AdjustThresholds (library): bres = BRES_PARAM[2], ats = ATS_ELEMENT[nElements],
   qcOutTot = {totalGrantedPeCorr, totalNoRedPe} */
void cref_adj_thr_AdjustThresholds(const void *bres, void *ats, INT bitDistributionMode, INT maxIter2ndGuess, const void *cm, void *qcCh, const void *psy, void *peData, const void *tools, INT *elScal, const INT *qcOutTot, INT cbr, INT poison, UCHAR *dynOut) {
  const CHANNEL_MAPPING *c = (const CHANNEL_MAPPING *)cm;
  Multi *m = (Multi *)malloc(sizeof(Multi));
  ADJ_THR_STATE s;
  mkAdjThr(&s, bres, ats, c->nElements, bitDistributionMode, maxIter2ndGuess);
  mkMulti(m, c, qcCh, psy, peData, tools, elScal, poison);
  m->qcOut.totalGrantedPeCorr = qcOutTot[0];
  m->qcOut.totalNoRedPe = qcOutTot[1];
  FDKaacEnc_AdjustThresholds(&s, m->qcElP, &m->qcOut, m->psyElP, cbr, c);
  unMulti(m, c, peData, elScal, dynOut);
  free(m);
}
}
