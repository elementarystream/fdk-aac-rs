// Shims for the differential tests of fdk_aac_rs_aacenc::psy_main.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_psy_main (see build.rs).
//
// A reference psychoacoustic model as FDKaacEnc_Open/FDKaacEnc_Initialize/FDKaacEnc_EncodeFrame
// (aacenc.cpp) set it up and run it: PSY_INTERNAL, PSY_OUT and QC_OUT allocated by the library
// (FDKaacEnc_PsyNew, FDKaacEnc_PsyOutNew, FDKaacEnc_QCOutNew) with the shared dynamic RAM, the
// channel mapping, FDKaacEnc_psyInit, FDKaacEnc_psyMainInit, FDKaacEnc_QCOutInit, and per frame
// the pointer updates of FDKaacEnc_EncodeFrame and FDKaacEnc_psyMain for every audio element.
// The quantizer does not run, so the dynamic RAM holds only what the psychoacoustic model wrote.

#include "cref_common.h"
#include "libAACenc/src/aacenc.h"
#include "libAACenc/src/aacEnc_ram.h"
#include "libAACenc/src/psy_main.h"
#include "libAACenc/src/qc_main.h"
#include "libAACenc/src/channel_map.h"
#include "libAACenc/src/bandwidth.h"
#include "FDK_tools_rom.h"

#include <stddef.h>

struct PsyHarness {
  PSY_INTERNAL *hPsy;
  PSY_OUT *psyOut[1];
  QC_OUT *qcOut[1];
  FIXP_DBL *dynRam;
  CHANNEL_MAPPING cm;
  INT nElements;
  INT nChannels;
};

static INT idx_of(void *const *arr, INT n, const void *p) {
  for (INT i = 0; i < n; i++)
    if (arr[i] == p) return i;
  return -1;
}

extern "C" {
/* Allocates the structures for nElements elements and nChannels channels. */
void *cref_psy_main_open(INT nElements, INT nChannels) {
  PsyHarness *h = new PsyHarness;
  memset(h, 0, sizeof(*h));
  h->nElements = nElements;
  h->nChannels = nChannels;
  h->dynRam = GetAACdynamic_RAM();
  UCHAR *dyn = (UCHAR *)h->dynRam;
  FDKmemclear(dyn, AAC_ENC_DYN_RAM_SIZE);
  if (FDKaacEnc_PsyNew(&h->hPsy, nElements, nChannels, dyn) != AAC_ENC_OK) return NULL;
  if (FDKaacEnc_PsyOutNew(h->psyOut, nElements, nChannels, 1, dyn) != AAC_ENC_OK) return NULL;
  if (FDKaacEnc_QCOutNew(h->qcOut, nElements, nChannels, 1, dyn) != AAC_ENC_OK) return NULL;
  return h;
}
void cref_psy_main_close(void *p) {
  PsyHarness *h = (PsyHarness *)p;
  FDKaacEnc_PsyClose(&h->hPsy, h->psyOut);
  FDKaacEnc_QCClose(NULL, h->qcOut);
  FreeAACdynamic_RAM(&h->dynRam);
  delete h;
}
/* Channel mapping, bandwidth (FDKaacEnc_DetermineBandWidth with the proposed bandwidth),
   FDKaacEnc_psyInit (if doPsyInit), FDKaacEnc_psyMainInit and FDKaacEnc_QCOutInit as in
   FDKaacEnc_Initialize. Returns the first error; *bandwidth receives the bandwidth. */
INT cref_psy_main_init(void *p, INT aot, INT mode, INT order, INT sampleRate, INT frameLength, INT bitRate, INT bitrateMode, INT useTns, INT usePns, INT useIS, INT useMS, UINT syntaxFlags, UINT initFlags, INT doPsyInit, INT *bandwidth) {
  PsyHarness *h = (PsyHarness *)p;
  AAC_ENCODER_ERROR e;
  e = FDKaacEnc_InitChannelMapping((CHANNEL_MODE)mode, (CHANNEL_ORDER)order, &h->cm);
  if (e != AAC_ENC_OK) return e;
  INT bw = *bandwidth;
  e = FDKaacEnc_DetermineBandWidth(bw, bitRate, (AACENC_BITRATE_MODE)bitrateMode, sampleRate,
                                   frameLength, &h->cm, (CHANNEL_MODE)mode, &bw);
  if (e != AAC_ENC_OK) return e;
  *bandwidth = bw;
  if (doPsyInit) {
    e = FDKaacEnc_psyInit(h->hPsy, h->psyOut, 1, h->nChannels, (AUDIO_OBJECT_TYPE)aot, &h->cm);
    if (e != AAC_ENC_OK) return e;
  }
  e = FDKaacEnc_psyMainInit(h->hPsy, (AUDIO_OBJECT_TYPE)aot, &h->cm, sampleRate, frameLength,
                            bitRate, useTns ? TNS_ENABLE_MASK : 0, bw, usePns, useIS, useMS,
                            syntaxFlags, initFlags);
  if (e != AAC_ENC_OK) return e;
  return FDKaacEnc_QCOutInit(h->qcOut, 1, &h->cm);
}
/* One frame: `fn` (a function with the signature of FDKaacEnc_psyMain, such as the copy of
   cref_psy_main_amplified_fn(), or NULL for the library's FDKaacEnc_psyMain) for every audio element
   (planar input, inputBufSize samples per channel). Returns the first error. */
INT cref_psy_main_frame_fn(void *p, SHORT *input, UINT inputBufSize, void *fn) {
  typedef AAC_ENCODER_ERROR (*PsyMainFn)(INT, PSY_ELEMENT *, PSY_DYNAMIC *, PSY_CONFIGURATION *,
                                         PSY_OUT_ELEMENT *, INT_PCM *, const UINT, INT *, INT);
  PsyMainFn psyMain = fn ? (PsyMainFn)fn : FDKaacEnc_psyMain;
  PsyHarness *h = (PsyHarness *)p;
  CHANNEL_MAPPING *cm = &h->cm;
  PSY_OUT *psyOut = h->psyOut[0];
  QC_OUT *qcOut = h->qcOut[0];
  for (INT el = 0; el < cm->nElements; el++) {
    ELEMENT_INFO elInfo = cm->elInfo[el];
    if ((elInfo.elType == ID_SCE) || (elInfo.elType == ID_CPE) || (elInfo.elType == ID_LFE)) {
      for (INT ch = 0; ch < elInfo.nChannelsInEl; ch++) {
        PSY_OUT_CHANNEL *psyOutChan = psyOut->psyOutElement[el]->psyOutChannel[ch];
        QC_OUT_CHANNEL *qcOutChan = qcOut->qcElement[el]->qcOutChannel[ch];
        psyOutChan->mdctSpectrum = qcOutChan->mdctSpectrum;
        psyOutChan->sfbSpreadEnergy = qcOutChan->sfbSpreadEnergy;
        psyOutChan->sfbEnergy = qcOutChan->sfbEnergy;
        psyOutChan->sfbEnergyLdData = qcOutChan->sfbEnergyLdData;
        psyOutChan->sfbMinSnrLdData = qcOutChan->sfbMinSnrLdData;
        psyOutChan->sfbThresholdLdData = qcOutChan->sfbThresholdLdData;
      }
      AAC_ENCODER_ERROR e = psyMain(
          elInfo.nChannelsInEl, h->hPsy->psyElement[el], h->hPsy->psyDynamic,
          h->hPsy->psyConf, psyOut->psyOutElement[el], (INT_PCM *)input, inputBufSize,
          cm->elInfo[el].ChannelIndex, cm->nChannels);
      if (e != AAC_ENC_OK) return e;
    }
  }
  return AAC_ENC_OK;
}
/* One frame of the library's FDKaacEnc_psyMain. */
INT cref_psy_main_frame(void *p, SHORT *input, UINT inputBufSize) {
  return cref_psy_main_frame_fn(p, input, inputBufSize, NULL);
}
/* CHANNEL_MAPPING (C layout). */
void cref_psy_main_get_cm(void *p, void *out) {
  memcpy(out, &((PsyHarness *)p)->cm, sizeof(CHANNEL_MAPPING));
}
/* Sets psyConf[i].lowpassLine and .lowpassLineLFE. */
void cref_psy_main_set_lowpass(void *p, INT i, INT lowpassLine, INT lowpassLineLFE) {
  PsyHarness *h = (PsyHarness *)p;
  h->hPsy->psyConf[i].lowpassLine = lowpassLine;
  h->hPsy->psyConf[i].lowpassLineLFE = lowpassLineLFE;
}
/* psyConf[i] (C layout); granuleLength of PSY_INTERNAL in *granule. */
void cref_psy_main_get_conf(void *p, INT i, void *out, INT *granule) {
  PsyHarness *h = (PsyHarness *)p;
  memcpy(out, &h->hPsy->psyConf[i], sizeof(PSY_CONFIGURATION));
  *granule = h->hPsy->granuleLength;
}
/* Indexes of the statics, psy out channels and qc out channels of element el (-1: none). */
void cref_psy_main_get_element(void *p, INT el, INT *st, INT *po, INT *qo) {
  PsyHarness *h = (PsyHarness *)p;
  for (INT ch = 0; ch < 2; ch++) {
    st[ch] = idx_of((void *const *)h->hPsy->pStaticChannels, h->nChannels,
                    h->hPsy->psyElement[el]->psyStatic[ch]);
    po[ch] = idx_of((void *const *)h->psyOut[0]->pPsyOutChannels, h->nChannels,
                    h->psyOut[0]->psyOutElement[el]->psyOutChannel[ch]);
    qo[ch] = idx_of((void *const *)h->qcOut[0]->pQcOutChannels, h->nChannels,
                    h->qcOut[0]->qcElement[el]->qcOutChannel[ch]);
  }
}
/* PSY_STATIC i: the block switching control (C layout), the input buffer (2048), the overlap
   add buffer (768), sfbThresholdnm1 (51) and misc[0..8] = mdctScalenm1, calcPreEcho, isLFE,
   mdctPers.prev_tl, prev_nr, prev_fr, ov_offset, ov_size, prevAliasSymmetry,
   prevPrevAliasSymmetry, the index in lens/shapes of mdctPers.prev_wrs (-1: NULL, -2: other). */
void cref_psy_main_get_static(void *p, INT i, void *bsc, SHORT *inbuf, FIXP_DBL *ola, FIXP_DBL *thr, INT *misc, const INT *lens, const INT *shapes, INT nSlopes) {
  PsyHarness *h = (PsyHarness *)p;
  PSY_STATIC *s = h->hPsy->pStaticChannels[i];
  memcpy(bsc, &s->blockSwitchingControl, sizeof(BLOCK_SWITCHING_CONTROL));
  memcpy(inbuf, s->psyInputBuffer, MAX_INPUT_BUFFER_SIZE * sizeof(INT_PCM));
  memcpy(ola, s->overlapAddBuffer, sizeof(s->overlapAddBuffer));
  memcpy(thr, s->sfbThresholdnm1, sizeof(s->sfbThresholdnm1));
  misc[0] = s->mdctScalenm1;
  misc[1] = s->calcPreEcho;
  misc[2] = s->isLFE;
  misc[3] = s->mdctPers.prev_tl;
  misc[4] = s->mdctPers.prev_nr;
  misc[5] = s->mdctPers.prev_fr;
  misc[6] = s->mdctPers.ov_offset;
  misc[7] = s->mdctPers.ov_size;
  misc[8] = s->mdctPers.prevAliasSymmetry;
  misc[9] = s->mdctPers.prevPrevAliasSymmetry;
  misc[10] = -2;
  if (s->mdctPers.prev_wrs == NULL) misc[10] = -1;
  for (INT k = 0; k < nSlopes && misc[10] == -2; k++)
    if (FDKgetWindowSlope(lens[k], shapes[k]) == s->mdctPers.prev_wrs) misc[10] = k;
}
/* psyDynamic: psyData[ch] from sfbThreshold on (sizeof(PSY_DATA) - offsetof(sfbThreshold)
   bytes), tnsData[ch], pnsData[ch] (C layout). */
void cref_psy_main_get_dynamic(void *p, INT ch, void *psyData, void *tnsData, void *pnsData) {
  PsyHarness *h = (PsyHarness *)p;
  PSY_DYNAMIC *d = h->hPsy->psyDynamic;
  memcpy(psyData, &d->psyData[ch].sfbThreshold,
         sizeof(PSY_DATA) - offsetof(PSY_DATA, sfbThreshold));
  memcpy(tnsData, &d->tnsData[ch], sizeof(TNS_DATA));
  memcpy(pnsData, &d->pnsData[ch], sizeof(PNS_DATA));
}
/* pPsyOutChannels[i] up to the pointers (offsetof(PSY_OUT_CHANNEL, mdctSpectrum) bytes). */
void cref_psy_main_get_psy_out_channel(void *p, INT i, void *out) {
  PsyHarness *h = (PsyHarness *)p;
  memcpy(out, h->psyOut[0]->pPsyOutChannels[i], offsetof(PSY_OUT_CHANNEL, mdctSpectrum));
}
/* psyOutElement[el]: commonWindow and toolsInfo (C layout). */
void cref_psy_main_get_psy_out_element(void *p, INT el, INT *commonWindow, void *toolsInfo) {
  PsyHarness *h = (PsyHarness *)p;
  *commonWindow = h->psyOut[0]->psyOutElement[el]->commonWindow;
  memcpy(toolsInfo, &h->psyOut[0]->psyOutElement[el]->toolsInfo, sizeof(TOOLSINFO));
}
/* pQcOutChannels[i] (C layout). */
void cref_psy_main_get_qc_out_channel(void *p, INT i, void *out) {
  PsyHarness *h = (PsyHarness *)p;
  memcpy(out, h->qcOut[0]->pQcOutChannels[i], sizeof(QC_OUT_CHANNEL));
}
}
