// A C-side frame stepper around the reference encoder core, for realistic stateful tests of the
// encoder stages (threshold adjustment, quantization, psychoacoustics). Every `cref_*` function
// defined at the start of a line is bound in fdk_aac_rs_cref::aacenc_adj_thr_enc (see build.rs);
// the Rust side is crates/cref/tests/aacenc/adj_thr_enc.rs (module `crate::adj_thr_enc`).
//
// API
// ---
// * `cref_adj_thr_enc_open(aot, sampleRate, channelMode, bitrate, bitrateMode, afterburner,
//   frameLength, transport, &err)` opens a reference encoder through the library API
//   (aacEncOpen with the AAC module only, aacEncoder_SetParam: AACENC_AOT, AACENC_SAMPLERATE,
//   AACENC_CHANNELMODE, AACENC_BITRATEMODE, AACENC_BITRATE (CBR only), AACENC_AFTERBURNER,
//   AACENC_GRANULE_LENGTH (if frameLength != 0; ELD 256/240/128/120 select the downscaled
//   modes), AACENC_SBR_MODE 0 (ELD), AACENC_TRANSMUX) and initializes it with
//   aacEncEncode(h, NULL, NULL, NULL, NULL), i.e. with exactly the configuration that
//   aacenc_lib.cpp derives (aacEncInit, FDKaacEnc_AdjustEncSettings, FDKaacEnc_Initialize,
//   transportEnc_Init). Returns a handle or NULL (`err` = AACENC_ERROR).
//   `cref_adj_thr_enc_close(h)` releases it.
// * `cref_adj_thr_enc_info(h, out)`: out[0..8] = nChannels, nElements, frameLength (input
//   samples per channel and frame), inputBufferSizePerChannel, aot, syntaxFlags, epConfig,
//   bitrateMode.
// * Frame stepping. `cref_adj_thr_enc_begin_frame(h, pcm)` copies one frame of planar 16-bit PCM
//   (`pcm[ch * frameLength + i]`, MPEG channel order) into the encoder's input buffer (as
//   aacEncEncode does, `inputBuffer + ch * inputBufferSizePerChannel`), resets the extension
//   payloads like aacEncEncode (no payloads) and sets the next stage to STAGE_BEGIN.
//   `cref_adj_thr_enc_next(h, &el)` returns the next stage (and its element), and
//   `cref_adj_thr_enc_run(h)` runs it and advances (returns the AAC_ENCODER_ERROR of the stage;
//   on an error the frame ends, as FDKaacEnc_EncodeFrame returns). The stages replicate
//   FDKaacEnc_EncodeFrame (aacenc.cpp) and FDKaacEnc_QCMain (qc_main.cpp) statement by
//   statement (the static helpers of qc_main.cpp are reached through a namespace include):
//      0 BEGIN          clear extPayloadUsed; qcOut->elementExtBits/staticBits/totalNoRedPe = 0
//      1 PSY(el)        PSY_OUT_CHANNEL pointer update + FDKaacEnc_psyMain
//      2 FORM_FACTOR(el) FDKaacEnc_CalcFormFactor          (first part of FDKaacEnc_QCMainPrepare)
//      3 PE(el)         FDKaacEnc_peCalculation
//      4 EL_BITS(el)    FDKaacEnc_ChannelElementWrite (static bits), extension payloads, sums
//      5 GLOBAL         global extension payloads, ID_END, FDKaacEnc_AdjustBitrate, globHdrBits
//      6 BITRES_REDIST  (QCMain) isCBRAdjustment, FDKaacEnc_BitResRedistribution, qcElement[]
//      7 PREPARE_BITS   (prepareBitDistribution) granted/max dyn bits, distributeElementDynBits
//      8 DISTRIBUTE(el) FDKaacEnc_DistributeBits + sums     (prepareBitDistribution)
//      9 PREPARE_END    totalAvailableBits = min(maxBitsPerFrame, totalAvailableBits)
//     10 ADJUST_THR     FDKaacEnc_AdjustThresholds
//     11 QUANT          the rest of FDKaacEnc_QCMain (scalefactors, quantization loop)
//     12 FINALIZE       updateFillBits, FinalizeBitConsumption, updateBitres,
//                       transportEnc_WriteAccessUnit, FDKaacEnc_WriteBitstream, GetFrame
//     13 DONE
//   Stages with an element run for every audio element (SCE, CPE, LFE) in order.
//   `cref_adj_thr_enc_frame(h, pcm, out, outSize, &nBytes)` runs a whole frame and copies the
//   access unit; `cref_adj_thr_enc_lib_frame(h, pcmInterleaved, out, outSize, &nBytes)` encodes a
//   frame with the library's aacEncEncode instead (for an independent second instance: both
//   must give the same bytes, which checks the replication).
//   `cref_adj_thr_enc_qc_vars(h, vars)` reads/`cref_adj_thr_enc_set_qc_vars` writes the locals
//   of the replicated QCMain/EncodeFrame: avgTotalBits, totalAvailableBits, avgTotalDynBits,
//   isCBRAdjustment.
//   `cref_adj_thr_enc_set_stack_poison(h, v)` fills 64 KiB of stack with the byte v before every
//   stage (-1: off), to check whether a result depends on earlier stack contents.
// * State access (between stages). `cref_adj_thr_enc_ptr(h, what, idx)` returns the C structure
//   (see enum Ptr below); structures with the port's layout (QC_OUT_CHANNEL, PE_DATA,
//   ATS_ELEMENT, ELEMENT_BITS, CHANNEL_MAPPING, AACENC_CONFIG, PSY_CONFIGURATION, TOOLSINFO,
//   BLOCK_SWITCHING_CONTROL, TNS_DATA, PNS_DATA, the PSY_OUT_CHANNEL prefix, the PSY_DATA part
//   after its pointer) are copied as bytes by the Rust side, in both directions.
//   `cref_adj_thr_enc_offset(i, &value)` lists offsets/sizes of members that the Rust side
//   needs (name, value). The other structures are exchanged as INT arrays:
//   `cref_adj_thr_enc_get_ints(h, what, idx, out)` / `cref_adj_thr_enc_set_ints(h, what, idx,
//   in)` with what = INTS_* (the field order is documented at each case).
// * `cref_adj_thr_enc_window_slope(ptr, &length, &shape)` identifies a window slope pointer
//   (mdct_t::prev_wrs) as FDKgetWindowSlope(length, shape).

#include "cref_common.h"
#include "aacenc_lib.h"
#include "FDK_audio.h"
#include "FDK_core.h"
#include "FDK_tools_rom.h"
#include "pcm_utils.h"
#include "sbr_encoder.h"
#include "libSBRenc/src/sbrenc_ram.h"
#include "tpenc_lib.h"
#include "sacenc_lib.h"
#include "libAACenc/src/aacenc.h"
#include "libAACenc/src/aacEnc_ram.h"
#include "libAACenc/src/channel_map.h"
#include "libAACenc/src/psy_const.h"
#include "libAACenc/src/bitenc.h"
#include "libAACenc/src/metadata_main.h"
#include "libAACenc/src/mps_main.h"
#include "libAACenc/src/qc_main.h"
#include "libAACenc/src/quantize.h"
#include "libAACenc/src/interface.h"
#include "libAACenc/src/adj_thr.h"
#include "libAACenc/src/sf_estim.h"
#include "libAACenc/src/bit_cnt.h"
#include "libAACenc/src/dyn_bits.h"
#include "libAACenc/src/psy_main.h"

#include <stddef.h>
#include <stdlib.h>

/* qc_main.cpp, for its static helpers. Its local re-declaration of FDKaacEnc_QCClose would be
   ambiguous with the declaration of qc_main.h; the copy gets another name. */
#define FDKaacEnc_QCClose ref_qc_FDKaacEnc_QCClose
namespace ref_qc {
#include "libAACenc/src/qc_main.cpp"
}
#undef FDKaacEnc_QCClose

/* The definitions of USER_PARAM, SBRENC_EXT_PAYLOAD and struct AACENCODER, copied verbatim
   from libAACenc/src/aacenc_lib.cpp of fdk-aac v2.0.3 (the struct is opaque outside of that file;
   this completes the type declared in aacenc_lib.h with the identical definition). */
typedef struct {
  AUDIO_OBJECT_TYPE userAOT; /*!< Audio Object Type.             */
  UINT userSamplerate;       /*!< Sampling frequency.            */
  UINT nChannels;            /*!< will be set via channelMode.   */
  CHANNEL_MODE userChannelMode;
  UINT userBitrate;
  UINT userBitrateMode;
  UINT userBandwidth;
  UINT userAfterburner;
  UINT userFramelength;
  UINT userAncDataRate;
  UINT userPeakBitrate;

  UCHAR userTns;       /*!< Use TNS coding. */
  UCHAR userPns;       /*!< Use PNS coding. */
  UCHAR userIntensity; /*!< Use Intensity coding. */

  TRANSPORT_TYPE userTpType; /*!< Transport type */
  UCHAR userTpSignaling;     /*!< Extension AOT signaling mode. */
  UCHAR userTpNsubFrames;    /*!< Number of sub frames in a transport frame for
                                LOAS/LATM or ADTS (default 1). */
  UCHAR userTpAmxv; /*!< AudioMuxVersion to be used for LATM (default 0). */
  UCHAR userTpProtection;
  UCHAR userTpHeaderPeriod; /*!< Parameter used to configure LATM/LOAS SMC rate.
                               Moreover this parameters is used to configure
                               repetition rate of PCE in raw_data_block. */

  UCHAR userErTools;     /*!< Use VCB11, HCR and/or RVLC ER tool. */
  UINT userPceAdditions; /*!< Configure additional bits in PCE. */

  UCHAR userMetaDataMode; /*!< Meta data library configuration. */

  UCHAR userSbrEnabled; /*!< Enable SBR for ELD. */
  UINT userSbrRatio;    /*!< SBR sampling rate ratio. Dual- or single-rate. */

  UINT userDownscaleFactor;

} USER_PARAM;

/**
 *  SBR extenxion payload struct provides buffers to be filled in SBR encoder
 * library.
 */
typedef struct {
  UCHAR data[(1)][(8)][MAX_PAYLOAD_SIZE]; /*!< extension payload data buffer */
  UINT dataSize[(1)][(8)]; /*!< extension payload data size in bits */
} SBRENC_EXT_PAYLOAD;

////////////////////////////////////////////////////////////////////////////////////

/****************************************************************************
                           Structure Definitions
****************************************************************************/

typedef struct AACENC_CONFIG *HANDLE_AACENC_CONFIG;

struct AACENCODER {
  USER_PARAM extParam;
  CODER_CONFIG coderConfig;

  /* AAC */
  AACENC_CONFIG aacConfig;
  HANDLE_AAC_ENC hAacEnc;

  /* SBR */
  HANDLE_SBR_ENCODER hEnvEnc;      /* SBR encoder */
  SBRENC_EXT_PAYLOAD *pSbrPayload; /* SBR extension payload */

  /* Meta Data */
  HANDLE_FDK_METADATA_ENCODER hMetadataEnc;
  INT metaDataAllowed; /* Signal whether chosen configuration allows metadata.
                          Necessary for delay compensation. Metadata mode is a
                          separate parameter. */

  HANDLE_MPS_ENCODER hMpsEnc;

  /* Transport */
  HANDLE_TRANSPORTENC hTpEnc;

  INT_PCM
  *inputBuffer;     /* Internal input buffer. Input source for AAC encoder */
  UCHAR *outBuffer; /* Internal bitstream buffer */

  INT inputBufferSize;           /* Size of internal input buffer */
  INT inputBufferSizePerChannel; /* Size of internal input buffer per channel */
  INT outBufferInBytes;          /* Size of internal bitstream buffer*/

  INT inputBufferOffset; /* Where to write new input samples. */

  INT nSamplesToRead; /* number of input samples neeeded for encoding one frame
                       */
  INT nSamplesRead;   /* number of input samples already in input buffer */
  INT nZerosAppended; /* appended zeros at end of file*/
  INT nDelay;         /* codec delay */
  INT nDelayCore;     /* codec delay, w/o the SBR decoder delay */

  AACENC_EXT_PAYLOAD extPayload[MAX_TOTAL_EXT_PAYLOADS];

  ULONG InitFlags; /* internal status to treggier re-initialization */

  /* Memory allocation info. */
  INT nMaxAacElements;
  INT nMaxAacChannels;
  INT nMaxSbrElements;
  INT nMaxSbrChannels;

  UINT encoder_modis;

  /* Capability flags */
  UINT CAPF_tpEnc;
};

enum Stage {
  STAGE_BEGIN = 0,
  STAGE_PSY = 1,
  STAGE_FORM_FACTOR = 2,
  STAGE_PE = 3,
  STAGE_EL_BITS = 4,
  STAGE_GLOBAL = 5,
  STAGE_BITRES_REDIST = 6,
  STAGE_PREPARE_BITS = 7,
  STAGE_DISTRIBUTE = 8,
  STAGE_PREPARE_END = 9,
  STAGE_ADJUST_THR = 10,
  STAGE_QUANT = 11,
  STAGE_FINALIZE = 12,
  STAGE_DONE = 13
};

struct CrefEnc {
  HANDLE_AACENCODER h;
  AACENCODER *e; /* the same encoder */
  INT stage, el;
  INT avgTotalBits, totalAvailableBits, avgTotalDynBits, isCBRAdjustment;
  UCHAR extPayloadUsed[MAX_TOTAL_EXT_PAYLOADS];
  QC_OUT_ELEMENT *qcElement[(1)][((8))];
  INT nOutBytes;
  INT stackPoison; /* -1: off, else the byte written below the stack pointer before a stage */
};

/* Fills 64 KiB of the stack below the caller with v (for checking whether the scratch buffers
   of the stages, C_ALLOC_SCRATCH_START, carry earlier contents into a result). */
static void __attribute__((noinline)) poisonStack(INT v) {
  volatile UCHAR buf[64 * 1024];
  for (INT i = 0; i < (INT)sizeof(buf); i++) buf[i] = (UCHAR)v;
}

static HANDLE_AAC_ENC core(CrefEnc *c) { return c->e->hAacEnc; }

static INT isAudio(MP4_ELEMENT_ID t) { return (t == ID_SCE) || (t == ID_CPE) || (t == ID_LFE); }

/* First audio element >= el, or -1. */
static INT nextAudio(CrefEnc *c, INT el) {
  CHANNEL_MAPPING *cm = &core(c)->channelMapping;
  for (; el < cm->nElements; el++)
    if (isAudio(cm->elInfo[el].elType)) return el;
  return -1;
}

/* Advances to the stage after (stage, el). */
static void advance(CrefEnc *c) {
  INT s = c->stage;
  if (s == STAGE_PSY || s == STAGE_FORM_FACTOR || s == STAGE_PE || s == STAGE_EL_BITS) {
    /* per element: PSY, FORM_FACTOR, PE, EL_BITS for each element in turn */
    if (s != STAGE_EL_BITS) {
      c->stage = s + 1;
      return;
    }
    INT n = nextAudio(c, c->el + 1);
    if (n >= 0) {
      c->stage = STAGE_PSY;
      c->el = n;
    } else {
      c->stage = STAGE_GLOBAL;
      c->el = 0;
    }
    return;
  }
  if (s == STAGE_DISTRIBUTE) {
    INT n = nextAudio(c, c->el + 1);
    if (n >= 0) {
      c->el = n;
    } else {
      c->stage = STAGE_PREPARE_END;
      c->el = 0;
    }
    return;
  }
  if (s == STAGE_BEGIN || s == STAGE_PREPARE_BITS) {
    INT n = nextAudio(c, 0);
    c->stage = (n >= 0) ? s + 1 : (s == STAGE_BEGIN ? STAGE_GLOBAL : STAGE_PREPARE_END);
    c->el = (n >= 0) ? n : 0;
    return;
  }
  if (s < STAGE_DONE) c->stage = s + 1;
  c->el = 0;
}

/* The rest of FDKaacEnc_QCMain after FDKaacEnc_AdjustThresholds (qc_main.cpp), verbatim. */
static AAC_ENCODER_ERROR qcMainQuant(CrefEnc *c) {
  using namespace ref_qc;
  HANDLE_AAC_ENC enc = core(c);
  QC_STATE *hQC = enc->qcKernel;
  PSY_OUT **psyOut = enc->psyOut;
  QC_OUT **qcOut = enc->qcOut;
  CHANNEL_MAPPING *cm = &enc->channelMapping;
  const AUDIO_OBJECT_TYPE aot = enc->aot;
  UINT syntaxFlags = enc->config->syntaxFlags;
  SCHAR epConfig = enc->config->epConfig;
  QC_OUT_ELEMENT *(*qcElement)[((8))] = c->qcElement;
  INT totalAvailableBits = c->totalAvailableBits;
  INT avgTotalDynBits = c->avgTotalDynBits;
  int i, c_;
  AAC_ENCODER_ERROR ErrorStatus = AAC_ENC_OK;
  INT nSubFrames = 1;
#define c c_
  /*-------------------------------------------- */
  INT iterations[(1)][((8))];
  INT chConstraintsFulfilled[(1)][((8))][(2)];
  INT calculateQuant[(1)][((8))][(2)];
  INT constraintsFulfilled[(1)][((8))];
  /*-------------------------------------------- */

  /* for ( all sub frames ) ... */
  for (c = 0; c < nSubFrames; c++) {
    for (i = 0; i < cm->nElements; i++) {
      ELEMENT_INFO elInfo = cm->elInfo[i];
      INT ch, nChannels = elInfo.nChannelsInEl;

      if ((elInfo.elType == ID_SCE) || (elInfo.elType == ID_CPE) ||
          (elInfo.elType == ID_LFE)) {
        /* Turn thresholds into scalefactors, optimize bit consumption and
         * verify conformance */
        FDKaacEnc_EstimateScaleFactors(
            psyOut[c]->psyOutElement[i]->psyOutChannel,
            qcElement[c][i]->qcOutChannel, hQC->invQuant, hQC->dZoneQuantEnable,
            cm->elInfo[i].nChannelsInEl);

        /*-------------------------------------------- */
        constraintsFulfilled[c][i] = 1;
        iterations[c][i] = 0;

        for (ch = 0; ch < nChannels; ch++) {
          chConstraintsFulfilled[c][i][ch] = 1;
          calculateQuant[c][i][ch] = 1;
        }

        /*-------------------------------------------- */

      } /*  -end- if(ID_SCE || ID_CPE || ID_LFE) */

    } /* -end- element loop */

    qcOut[c]->usedDynBits = -1;

  } /* -end- sub frame counter */

  INT quantizationDone = 0;
  INT sumDynBitsConsumedTotal = 0;
  INT decreaseBitConsumption = -1; /* no direction yet! */

  /*-------------------------------------------- */
  /* -start- Quantization loop ...               */
  /*-------------------------------------------- */
  do /* until max allowed bits per frame and maxDynBits!=-1*/
  {
    quantizationDone = 0;

    c = 0; /* get frame to process */

    for (i = 0; i < cm->nElements; i++) {
      ELEMENT_INFO elInfo = cm->elInfo[i];
      INT ch, nChannels = elInfo.nChannelsInEl;

      if ((elInfo.elType == ID_SCE) || (elInfo.elType == ID_CPE) ||
          (elInfo.elType == ID_LFE)) {
        do /* until element bits < nChannels*MIN_BUFSIZE_PER_EFF_CHAN */
        {
          do /* until spectral values < MAX_QUANT */
          {
            /*-------------------------------------------- */
            if (!constraintsFulfilled[c][i]) {
              if ((ErrorStatus = FDKaacEnc_reduceBitConsumption(
                       &iterations[c][i], hQC->maxIterations,
                       (decreaseBitConsumption) ? 1 : -1,
                       chConstraintsFulfilled[c][i], calculateQuant[c][i],
                       nChannels, psyOut[c]->psyOutElement[i], qcOut[c],
                       qcElement[c][i], hQC->elementBits[i], aot, syntaxFlags,
                       epConfig)) != AAC_ENC_OK) {
                return ErrorStatus;
              }
            }

            /*-------------------------------------------- */
            /*-------------------------------------------- */
            constraintsFulfilled[c][i] = 1;

            /*-------------------------------------------- */
            /* quantize spectrum (per each channel) */
            for (ch = 0; ch < nChannels; ch++) {
              /*-------------------------------------------- */
              chConstraintsFulfilled[c][i][ch] = 1;

              /*-------------------------------------------- */

              if (calculateQuant[c][i][ch]) {
                QC_OUT_CHANNEL *qcOutCh = qcElement[c][i]->qcOutChannel[ch];
                PSY_OUT_CHANNEL *psyOutCh =
                    psyOut[c]->psyOutElement[i]->psyOutChannel[ch];

                calculateQuant[c][i][ch] =
                    0; /* calculate quantization only if necessary */

                /*-------------------------------------------- */
                FDKaacEnc_QuantizeSpectrum(
                    psyOutCh->sfbCnt, psyOutCh->maxSfbPerGroup,
                    psyOutCh->sfbPerGroup, psyOutCh->sfbOffsets,
                    qcOutCh->mdctSpectrum, qcOutCh->globalGain, qcOutCh->scf,
                    qcOutCh->quantSpec, hQC->dZoneQuantEnable);

                /*-------------------------------------------- */
                if (FDKaacEnc_calcMaxValueInSfb(
                        psyOutCh->sfbCnt, psyOutCh->maxSfbPerGroup,
                        psyOutCh->sfbPerGroup, psyOutCh->sfbOffsets,
                        qcOutCh->quantSpec,
                        qcOutCh->maxValueInSfb) > MAX_QUANT) {
                  chConstraintsFulfilled[c][i][ch] = 0;
                  constraintsFulfilled[c][i] = 0;
                  /* if quanizted value out of range; increase global gain! */
                  decreaseBitConsumption = 1;
                }

                /*-------------------------------------------- */

              } /* if calculateQuant[c][i][ch] */

            } /* channel loop */

            /*-------------------------------------------- */
            /* quantize spectrum (per each channel) */

            /*-------------------------------------------- */

          } while (!constraintsFulfilled[c][i]); /* does not regard bit
                                                    consumption */

          /*-------------------------------------------- */
          /*-------------------------------------------- */
          qcElement[c][i]->dynBitsUsed = 0; /* reset dynamic bits */

          /* quantization valid in current channel! */
          for (ch = 0; ch < nChannels; ch++) {
            QC_OUT_CHANNEL *qcOutCh = qcElement[c][i]->qcOutChannel[ch];
            PSY_OUT_CHANNEL *psyOutCh =
                psyOut[c]->psyOutElement[i]->psyOutChannel[ch];

            /* count dynamic bits */
            INT chDynBits = FDKaacEnc_dynBitCount(
                hQC->hBitCounter, qcOutCh->quantSpec, qcOutCh->maxValueInSfb,
                qcOutCh->scf, psyOutCh->lastWindowSequence, psyOutCh->sfbCnt,
                psyOutCh->maxSfbPerGroup, psyOutCh->sfbPerGroup,
                psyOutCh->sfbOffsets, &qcOutCh->sectionData, psyOutCh->noiseNrg,
                psyOutCh->isBook, psyOutCh->isScale, syntaxFlags);

            /* sum up dynamic channel bits */
            qcElement[c][i]->dynBitsUsed += chDynBits;
          }

          /* save dynBitsUsed for correction of bits2pe relation */
          if (hQC->hAdjThr->adjThrStateElem[i]->dynBitsLast == -1) {
            hQC->hAdjThr->adjThrStateElem[i]->dynBitsLast =
                qcElement[c][i]->dynBitsUsed;
          }

          /* hold total bit consumption in present element below maximum allowed
           */
          if (qcElement[c][i]->dynBitsUsed >
              ((nChannels * MIN_BUFSIZE_PER_EFF_CHAN) -
               qcElement[c][i]->staticBitsUsed -
               qcElement[c][i]->extBitsUsed)) {
            constraintsFulfilled[c][i] = 0;
          }

        } while (!constraintsFulfilled[c][i]);

      } /*  -end- if(ID_SCE || ID_CPE || ID_LFE) */

    } /* -end- element loop */

    /* update dynBits of current subFrame */
    FDKaacEnc_updateUsedDynBits(&qcOut[c]->usedDynBits, qcElement[c], cm);

    /* get total consumed bits, dyn bits in all sub frames have to be valid */
    sumDynBitsConsumedTotal =
        FDKaacEnc_getTotalConsumedDynBits(qcOut, nSubFrames);

    if (sumDynBitsConsumedTotal == -1) {
      quantizationDone = 0; /* bit consumption not valid in all sub frames */
    } else {
      int sumBitsConsumedTotal = FDKaacEnc_getTotalConsumedBits(
          qcOut, qcElement, cm, hQC->globHdrBits, nSubFrames);

      /* in all frames are valid dynamic bits */
      if (((sumBitsConsumedTotal < totalAvailableBits) ||
           sumDynBitsConsumedTotal == 0) &&
          (decreaseBitConsumption == 1) &&
          checkMinFrameBitsDemand(qcOut, hQC->minBitsPerFrame, nSubFrames)
          /*()*/) {
        quantizationDone = 1; /* exit bit adjustment */
      }
      if (sumBitsConsumedTotal > totalAvailableBits &&
          (decreaseBitConsumption == 0)) {
        quantizationDone = 0; /* reset! */
      }
    }

    /*-------------------------------------------- */

    int emergencyIterations = 1;
    int dynBitsOvershoot = 0;

    for (c = 0; c < nSubFrames; c++) {
      for (i = 0; i < cm->nElements; i++) {
        ELEMENT_INFO elInfo = cm->elInfo[i];

        if ((elInfo.elType == ID_SCE) || (elInfo.elType == ID_CPE) ||
            (elInfo.elType == ID_LFE)) {
          /* iteration limitation */
          emergencyIterations &=
              ((iterations[c][i] < hQC->maxIterations) ? 0 : 1);
        }
      }
      /* detection if used dyn bits exceeds the maximal allowed criterion */
      dynBitsOvershoot |=
          ((qcOut[c]->usedDynBits > qcOut[c]->maxDynBits) ? 1 : 0);
    }

    if (quantizationDone == 0 || dynBitsOvershoot) {
      int sumBitsConsumedTotal = FDKaacEnc_getTotalConsumedBits(
          qcOut, qcElement, cm, hQC->globHdrBits, nSubFrames);

      if ((sumDynBitsConsumedTotal >= avgTotalDynBits) ||
          (sumDynBitsConsumedTotal == 0)) {
        quantizationDone = 1;
      }
      if (emergencyIterations && (sumBitsConsumedTotal < totalAvailableBits)) {
        quantizationDone = 1;
      }
      if ((sumBitsConsumedTotal > totalAvailableBits) ||
          !checkMinFrameBitsDemand(qcOut, hQC->minBitsPerFrame, nSubFrames)) {
        quantizationDone = 0;
      }
      if ((sumBitsConsumedTotal < totalAvailableBits) &&
          checkMinFrameBitsDemand(qcOut, hQC->minBitsPerFrame, nSubFrames)) {
        decreaseBitConsumption = 0;
      } else {
        decreaseBitConsumption = 1;
      }

      if (dynBitsOvershoot) {
        quantizationDone = 0;
        decreaseBitConsumption = 1;
      }

      /* reset constraints fullfilled flags */
      FDKmemclear(constraintsFulfilled, sizeof(constraintsFulfilled));
      FDKmemclear(chConstraintsFulfilled, sizeof(chConstraintsFulfilled));

    } /* quantizationDone */

  } while (!quantizationDone);
#undef c

  /*-------------------------------------------- */
  /* ... -end- Quantization loop                 */
  /*-------------------------------------------- */

  return AAC_ENC_OK;
}

/* Runs the current stage (replicating FDKaacEnc_EncodeFrame and FDKaacEnc_QCMain). */
static AAC_ENCODER_ERROR runStage(CrefEnc *c) {
  HANDLE_AAC_ENC hAacEnc = core(c);
  HANDLE_TRANSPORTENC hTpEnc = c->e->hTpEnc;
  CHANNEL_MAPPING *cm = &hAacEnc->channelMapping;
  PSY_OUT *psyOut = hAacEnc->psyOut[0];
  QC_OUT *qcOut = hAacEnc->qcOut[0];
  QC_STATE *hQC = hAacEnc->qcKernel;
  AACENC_EXT_PAYLOAD *extPayload = c->e->extPayload;
  AAC_ENCODER_ERROR ErrorStatus = AAC_ENC_OK;
  const INT el = c->el;
  int n, i, ch;

  switch (c->stage) {
    case STAGE_BEGIN:
      FDKmemclear(c->extPayloadUsed, MAX_TOTAL_EXT_PAYLOADS * sizeof(UCHAR));
      qcOut->elementExtBits = 0; /* sum up all extended bit of each element */
      qcOut->staticBits = 0;     /* sum up side info bits of each element */
      qcOut->totalNoRedPe = 0;   /* sum up PE */
      break;

    case STAGE_PSY: {
      ELEMENT_INFO elInfo = cm->elInfo[el];
      /* update pointer!*/
      for (ch = 0; ch < elInfo.nChannelsInEl; ch++) {
        PSY_OUT_CHANNEL *psyOutChan = psyOut->psyOutElement[el]->psyOutChannel[ch];
        QC_OUT_CHANNEL *qcOutChan = qcOut->qcElement[el]->qcOutChannel[ch];

        psyOutChan->mdctSpectrum = qcOutChan->mdctSpectrum;
        psyOutChan->sfbSpreadEnergy = qcOutChan->sfbSpreadEnergy;
        psyOutChan->sfbEnergy = qcOutChan->sfbEnergy;
        psyOutChan->sfbEnergyLdData = qcOutChan->sfbEnergyLdData;
        psyOutChan->sfbMinSnrLdData = qcOutChan->sfbMinSnrLdData;
        psyOutChan->sfbThresholdLdData = qcOutChan->sfbThresholdLdData;
      }

      ErrorStatus = FDKaacEnc_psyMain(
          elInfo.nChannelsInEl, hAacEnc->psyKernel->psyElement[el],
          hAacEnc->psyKernel->psyDynamic, hAacEnc->psyKernel->psyConf,
          psyOut->psyOutElement[el], c->e->inputBuffer, c->e->inputBufferSizePerChannel,
          cm->elInfo[el].ChannelIndex, cm->nChannels);
      break;
    }

    case STAGE_FORM_FACTOR:
      /* FDKaacEnc_QCMainPrepare, part 1 */
      FDKaacEnc_CalcFormFactor(qcOut->qcElement[el]->qcOutChannel,
                               psyOut->psyOutElement[el]->psyOutChannel,
                               cm->elInfo[el].nChannelsInEl);
      break;

    case STAGE_PE:
      /* FDKaacEnc_QCMainPrepare, part 2: prepare and calculate PE without reduction */
      FDKaacEnc_peCalculation(&qcOut->qcElement[el]->peData,
                              psyOut->psyOutElement[el]->psyOutChannel,
                              qcOut->qcElement[el]->qcOutChannel,
                              &psyOut->psyOutElement[el]->toolsInfo,
                              hQC->hAdjThr->adjThrStateElem[el], cm->elInfo[el].nChannelsInEl);
      break;

    case STAGE_EL_BITS: {
      ELEMENT_INFO elInfo = cm->elInfo[el];
      /* FDKaacEnc_QCMainPrepare, part 3 */
      ErrorStatus = FDKaacEnc_ChannelElementWrite(
          NULL, &elInfo, NULL, psyOut->psyOutElement[el],
          psyOut->psyOutElement[el]->psyOutChannel, hAacEnc->config->syntaxFlags, hAacEnc->aot,
          hAacEnc->config->epConfig, &qcOut->qcElement[el]->staticBitsUsed, 0);
      if (ErrorStatus != AAC_ENC_OK) break;

      qcOut->qcElement[el]->extBitsUsed = 0;
      qcOut->qcElement[el]->nExtensions = 0;
      /* reset extension payload */
      FDKmemclear(&qcOut->qcElement[el]->extension, (1) * sizeof(QC_OUT_EXTENSION));

      for (n = 0; n < MAX_TOTAL_EXT_PAYLOADS; n++) {
        if (!c->extPayloadUsed[n] && (extPayload[n].associatedChElement == el) &&
            (extPayload[n].dataSize > 0) && (extPayload[n].pData != NULL)) {
          int idx = qcOut->qcElement[el]->nExtensions++;

          qcOut->qcElement[el]->extension[idx].type = extPayload[n].dataType;
          qcOut->qcElement[el]->extension[idx].nPayloadBits = extPayload[n].dataSize;
          qcOut->qcElement[el]->extension[idx].pPayload = extPayload[n].pData;
          qcOut->qcElement[el]->extBitsUsed += FDKaacEnc_writeExtensionData(
              NULL, &qcOut->qcElement[el]->extension[idx], 0, 0,
              hAacEnc->config->syntaxFlags, hAacEnc->aot, hAacEnc->config->epConfig);
          c->extPayloadUsed[n] = 1;
        }
      }

      /* sum up extension and static bits for all channel elements */
      qcOut->elementExtBits += qcOut->qcElement[el]->extBitsUsed;
      qcOut->staticBits += qcOut->qcElement[el]->staticBitsUsed;

      /* sum up pe */
      qcOut->totalNoRedPe += qcOut->qcElement[el]->peData.pe;
      break;
    }

    case STAGE_GLOBAL:
      qcOut->nExtensions = 0;
      qcOut->globalExtBits = 0;

      /* reset extension payload */
      FDKmemclear(&qcOut->extension, (2 + 2) * sizeof(QC_OUT_EXTENSION));

      /* Add extension payload not assigned to an channel element
        (Ancillary data is the only supported type up to now) */
      for (n = 0; n < MAX_TOTAL_EXT_PAYLOADS; n++) {
        if (!c->extPayloadUsed[n] && (extPayload[n].associatedChElement == -1) &&
            (extPayload[n].pData != NULL)) {
          UINT payloadBits = 0;

          if (extPayload[n].dataType == EXT_DATA_ELEMENT) {
            if (hAacEnc->ancillaryBitsPerFrame) {
              /* granted frame dse bitrate */
              payloadBits = hAacEnc->ancillaryBitsPerFrame;
            } else {
              /* write anc data if bitrate constraint fulfilled */
              if ((extPayload[n].dataSize >> 3) <= hAacEnc->config->maxAncBytesPerAU) {
                payloadBits = extPayload[n].dataSize;
              }
            }
            payloadBits = fixMin(extPayload[n].dataSize, payloadBits);
          } else {
            payloadBits = extPayload[n].dataSize;
          }

          if (payloadBits > 0) {
            int idx = qcOut->nExtensions++;

            qcOut->extension[idx].type = extPayload[n].dataType;
            qcOut->extension[idx].nPayloadBits = payloadBits;
            qcOut->extension[idx].pPayload = extPayload[n].pData;
            qcOut->globalExtBits += FDKaacEnc_writeExtensionData(
                NULL, &qcOut->extension[idx], 0, 0, hAacEnc->config->syntaxFlags, hAacEnc->aot,
                hAacEnc->config->epConfig);
            if (extPayload[n].dataType == EXT_DATA_ELEMENT) {
              /* substract the processed bits */
              extPayload[n].dataSize -= payloadBits;
            }
            c->extPayloadUsed[n] = 1;
          }
        }
      }

      if (!(hAacEnc->config->syntaxFlags & (AC_SCALABLE | AC_ER))) {
        qcOut->globalExtBits += EL_ID_BITS; /* add bits for ID_END */
      }

      /* frame wise bitrate adaption */
      c->avgTotalBits = 0;
      FDKaacEnc_AdjustBitrate(hQC, cm, &c->avgTotalBits, hAacEnc->config->bitRate,
                              hAacEnc->config->sampleRate, hAacEnc->config->framelength);

      /* adjust super frame bitrate */
      c->avgTotalBits *= hAacEnc->config->nSubFrames;

      /* Make first estimate of transport header overhead.
         Take maximum possible frame size into account to prevent bitreservoir
         underrun. */
      hQC->globHdrBits = transportEnc_GetStaticBits(hTpEnc, c->avgTotalBits + hQC->bitResTot);
      break;

    case STAGE_BITRES_REDIST:
      /* FDKaacEnc_QCMain */
      c->avgTotalDynBits = 0;
      c->totalAvailableBits = 0;
      c->isCBRAdjustment = (ref_qc::isConstantBitrateMode(hQC->bitrateMode) ||
                            (hQC->bitResMode != AACENC_BR_MODE_FULL))
                               ? 1
                               : 0;

      /* redistribute total bitreservoir to elements */
      ErrorStatus = ref_qc::FDKaacEnc_BitResRedistribution(
          hQC, cm, (c->isCBRAdjustment == 0) ? hQC->maxBitsPerFrame : c->avgTotalBits);
      if (ErrorStatus != AAC_ENC_OK) break;

      /* work on a copy of qcChannel and qcElement */
      for (i = 0; i < cm->nElements; i++) {
        ELEMENT_INFO elInfo = cm->elInfo[i];
        if (isAudio(elInfo.elType)) {
          c->qcElement[0][i] = hAacEnc->qcOut[0]->qcElement[i];
        }
      }
      break;

    case STAGE_PREPARE_BITS: {
      /* FDKaacEnc_prepareBitDistribution, part 1 */
      const INT avgTotalBits =
          (c->isCBRAdjustment == 0) ? hQC->maxBitsPerFrame : c->avgTotalBits;
      QC_OUT **qcOutA = hAacEnc->qcOut;
      /* get maximal allowed dynamic bits */
      qcOutA[0]->grantedDynBits =
          (fixMin(hQC->maxBitsPerFrame, avgTotalBits) - hQC->globHdrBits) & ~7;
      qcOutA[0]->grantedDynBits -=
          (qcOutA[0]->globalExtBits + qcOutA[0]->staticBits + qcOutA[0]->elementExtBits);
      qcOutA[0]->maxDynBits =
          ((hQC->maxBitsPerFrame) & ~7) -
          (qcOutA[0]->globalExtBits + qcOutA[0]->staticBits + qcOutA[0]->elementExtBits);
      /* assure that enough bits are available */
      if ((qcOutA[0]->grantedDynBits + hQC->bitResTot) < 0) {
        /* crash recovery allows to reduce static bits to a minimum */
        if ((qcOutA[0]->grantedDynBits + hQC->bitResTot) <
            (ref_qc::FDKaacEnc_getMinimalStaticBitdemand(cm, hAacEnc->psyOut) -
             qcOutA[0]->staticBits)) {
          ErrorStatus = AAC_ENC_BITRES_TOO_LOW;
          break;
        }
      }

      /* distribute dynamic bits to each element */
      ref_qc::FDKaacEnc_distributeElementDynBits(hQC, c->qcElement[0], cm,
                                                 qcOutA[0]->grantedDynBits);

      c->avgTotalDynBits = 0; /*frameDynBits;*/

      c->totalAvailableBits = avgTotalBits;

      /* sum up corrected granted PE */
      qcOutA[0]->totalGrantedPeCorr = 0;
      break;
    }

    case STAGE_DISTRIBUTE: {
      /* FDKaacEnc_prepareBitDistribution, element loop */
      PSY_OUT **psyOutA = hAacEnc->psyOut;
      QC_OUT_ELEMENT *(*qcElement)[((8))] = c->qcElement;
      int nChannels = cm->elInfo[el].nChannelsInEl;
      FDKaacEnc_DistributeBits(
          hQC->hAdjThr, hQC->hAdjThr->adjThrStateElem[el], psyOutA[0]->psyOutElement[el]->psyOutChannel,
          &qcElement[0][el]->peData, &qcElement[0][el]->grantedPe,
          &qcElement[0][el]->grantedPeCorr, nChannels, psyOutA[0]->psyOutElement[el]->commonWindow,
          qcElement[0][el]->grantedDynBits, hQC->elementBits[el]->bitResLevelEl,
          hQC->elementBits[el]->maxBitResBitsEl, hQC->maxBitFac, hQC->bitResMode);

      c->totalAvailableBits += hQC->elementBits[el]->bitResLevelEl;
      /* get total corrected granted PE */
      hAacEnc->qcOut[0]->totalGrantedPeCorr += qcElement[0][el]->grantedPeCorr;
      break;
    }

    case STAGE_PREPARE_END:
      c->totalAvailableBits = fMin(hQC->maxBitsPerFrame, c->totalAvailableBits);
      break;

    case STAGE_ADJUST_THR:
      /* for CBR and VBR mode */
      FDKaacEnc_AdjustThresholds(hQC->hAdjThr, c->qcElement[0], hAacEnc->qcOut[0],
                                 hAacEnc->psyOut[0]->psyOutElement, c->isCBRAdjustment, cm);
      break;

    case STAGE_QUANT:
      ErrorStatus = qcMainQuant(c);
      break;

    case STAGE_FINALIZE: {
      INT totalBits = 0;
      ErrorStatus = FDKaacEnc_updateFillBits(cm, hQC, hQC->elementBits, hAacEnc->qcOut);
      if (ErrorStatus != AAC_ENC_OK) break;

      ErrorStatus = FDKaacEnc_FinalizeBitConsumption(
          cm, hQC, qcOut, qcOut->qcElement, hTpEnc, hAacEnc->aot, hAacEnc->config->syntaxFlags,
          hAacEnc->config->epConfig);
      if (ErrorStatus != AAC_ENC_OK) break;
      totalBits += qcOut->totalBits;

      FDKaacEnc_updateBitres(cm, hQC, hAacEnc->qcOut);

      /* write bitstream header */
      if (TRANSPORTENC_OK != transportEnc_WriteAccessUnit(hTpEnc, totalBits,
                                                          FDKaacEnc_GetBitReservoirState(hAacEnc),
                                                          cm->nChannelsEff)) {
        ErrorStatus = AAC_ENC_UNKNOWN;
        break;
      }

      /* write bitstream */
      ErrorStatus = FDKaacEnc_WriteBitstream(hTpEnc, cm, qcOut, psyOut, hQC, hAacEnc->aot,
                                             hAacEnc->config->syntaxFlags,
                                             hAacEnc->config->epConfig);
      if (ErrorStatus != AAC_ENC_OK) break;

      if (TRANSPORTENC_OK != transportEnc_GetFrame(hTpEnc, &c->nOutBytes)) {
        ErrorStatus = AAC_ENC_UNKNOWN;
      }
      break;
    }

    default:
      break;
  }
  return ErrorStatus;
}

enum Ptr {
  PTR_AAC_ENC = 0,         /* AAC_ENC */
  PTR_CONFIG = 1,          /* AACENC_CONFIG */
  PTR_CHANNEL_MAPPING = 2, /* CHANNEL_MAPPING */
  PTR_QC_STATE = 3,        /* QC_STATE */
  PTR_ADJ_THR = 4,         /* ADJ_THR_STATE */
  PTR_ATS_ELEMENT = 5,     /* ATS_ELEMENT idx */
  PTR_ELEMENT_BITS = 6,    /* ELEMENT_BITS idx */
  PTR_QC_OUT = 7,          /* QC_OUT */
  PTR_QC_ELEMENT = 8,      /* QC_OUT_ELEMENT idx */
  PTR_QC_CHANNEL = 9,      /* QC_OUT_CHANNEL idx (pQcOutChannels) */
  PTR_PSY_OUT = 10,        /* PSY_OUT */
  PTR_PSY_ELEMENT_OUT = 11, /* PSY_OUT_ELEMENT idx */
  PTR_PSY_CHANNEL = 12,    /* PSY_OUT_CHANNEL idx (pPsyOutChannels) */
  PTR_PSY_INTERNAL = 13,   /* PSY_INTERNAL */
  PTR_PSY_ELEMENT = 14,    /* PSY_ELEMENT idx */
  PTR_PSY_STATIC = 15,     /* PSY_STATIC idx (pStaticChannels) */
  PTR_PSY_DYNAMIC = 16,    /* PSY_DYNAMIC */
  PTR_BIT_COUNTER = 17,    /* BITCNTR_STATE */
  PTR_TP_ENC = 18,         /* transport encoder */
  PTR_INPUT = 19,          /* INT_PCM input buffer */
  PTR_DYN_RAM = 20,        /* dynamic RAM (P_BUF_0, P_BUF_1) */
  PTR_PSY_CONF = 21        /* PSY_CONFIGURATION idx (0 long, 1 short) */
};

/* Index of p in arr[0..n), or -1. */
static INT findPtr(void *const *arr, INT n, const void *p) {
  for (INT i = 0; i < n; i++)
    if (arr[i] == p) return i;
  return -1;
}

struct OffsetEntry {
  const char *name;
  INT value;
};
#define S(x) {"sizeof " #x, (INT)sizeof(x)}
#define O(t, f) {#t "." #f, (INT)offsetof(t, f)}
static const OffsetEntry offsets[] = {
    S(AAC_ENC), S(PSY_DATA), S(PSY_DYNAMIC), S(PSY_STATIC), S(QC_OUT_CHANNEL), S(PSY_OUT_CHANNEL),
    S(mdct_t), S(QC_STATE), S(QC_OUT), S(QC_OUT_ELEMENT), S(PSY_OUT_ELEMENT), S(TNS_DATA),
    S(PNS_DATA), S(PSY_CONFIGURATION), S(BLOCK_SWITCHING_CONTROL),
    O(PSY_DATA, sfbThreshold), O(PSY_DYNAMIC, psyData), O(PSY_DYNAMIC, tnsData),
    O(PSY_DYNAMIC, pnsData), O(PSY_STATIC, overlapAddBuffer), O(PSY_STATIC, mdctPers),
    O(PSY_STATIC, blockSwitchingControl), O(PSY_STATIC, sfbThresholdnm1),
    O(PSY_STATIC, mdctScalenm1), O(PSY_STATIC, calcPreEcho), O(PSY_STATIC, isLFE),
    O(PSY_OUT_CHANNEL, mdctSpectrum), O(PSY_OUT_ELEMENT, toolsInfo),
    O(QC_OUT_ELEMENT, peData), O(PSY_INTERNAL, psyConf), O(PSY_INTERNAL, granuleLength),
    {"P_BUF_1", (INT)(P_BUF_1)}, {"BUF_SIZE_1", (INT)(BUF_SIZE_1)},
    {"ADJ_THR_SIZE", (INT)(ADJ_THR_SIZE)},
};

/* INT array exchange (cref_adj_thr_enc_get_ints / set_ints) */
enum Ints {
  /* QC_OUT: nExtensions, maxDynBits, grantedDynBits, totFillBits, elementExtBits,
     globalExtBits, staticBits, totalNoRedPe, totalGrantedPeCorr, usedDynBits, alignBits,
     totalBits, then extension[0..4]: type, nPayloadBits (pPayload not exchanged) */
  INTS_QC_OUT = 0,
  /* QC_OUT_ELEMENT idx: staticBitsUsed, dynBitsUsed, extBitsUsed, nExtensions,
     extension[0].type, extension[0].nPayloadBits, grantedDynBits, grantedPe, grantedPeCorr,
     index of qcOutChannel[0], index of qcOutChannel[1] (in pQcOutChannels, -1 if none; read
     only) */
  INTS_QC_ELEMENT = 1,
  /* QC_STATE: globHdrBits, maxBitsPerFrame, minBitsPerFrame, nElements, bitrateMode,
     bitResMode, bitResTot, bitResTotMax, maxIterations, invQuant, vbrQualFactor, maxBitFac,
     padding.paddingRest, dZoneQuantEnable */
  INTS_QC_STATE = 2,
  /* ADJ_THR_STATE: bresParamLong (8), bresParamShort (8), bitDistributionMode,
     maxIter2ndGuess */
  INTS_ADJ_THR = 3,
  /* PSY_OUT_ELEMENT idx: index of psyOutChannel[0], [1] (in pPsyOutChannels; read only),
     commonWindow */
  INTS_PSY_OUT_ELEMENT = 4,
  /* PSY_ELEMENT idx: index of psyStatic[0], [1] (in pStaticChannels; read only) */
  INTS_PSY_ELEMENT = 5,
  /* AAC_ENC: ancillaryBitsPerFrame, encoderMode, bandwidth90dB, bitrateMode, dontWriteAdif,
     maxChannels, maxElements, maxFrames, aot */
  INTS_AAC_ENC = 6,
  /* mdct_t of PSY_STATIC idx: prev_tl, prev_nr, prev_fr, ov_offset, ov_size,
     prevAliasSymmetry, prevPrevAliasSymmetry, prev_wrs window length, prev_wrs shape (-1 if
     NULL, -2 if not identified; read only), overlap / pFacZir / pAsymOvlp != NULL (read only) */
  INTS_MDCT = 7
};

extern "C" {
/* Opens and initializes an encoder (see the file header). */
void *cref_adj_thr_enc_open(INT aot, INT sampleRate, INT channelMode, INT bitrate, INT bitrateMode, INT afterburner, INT frameLength, INT transport, INT *err) {
  HANDLE_AACENCODER h = NULL;
  AACENC_ERROR e = aacEncOpen(&h, 0x01, 8);
  if (e == AACENC_OK) e = aacEncoder_SetParam(h, AACENC_AOT, aot);
  if (e == AACENC_OK && aot == AOT_ER_AAC_ELD) e = aacEncoder_SetParam(h, AACENC_SBR_MODE, 0);
  if (e == AACENC_OK) e = aacEncoder_SetParam(h, AACENC_SAMPLERATE, sampleRate);
  if (e == AACENC_OK) e = aacEncoder_SetParam(h, AACENC_CHANNELMODE, channelMode);
  if (e == AACENC_OK) e = aacEncoder_SetParam(h, AACENC_BITRATEMODE, bitrateMode);
  if (e == AACENC_OK && bitrateMode == 0) e = aacEncoder_SetParam(h, AACENC_BITRATE, bitrate);
  if (e == AACENC_OK) e = aacEncoder_SetParam(h, AACENC_AFTERBURNER, afterburner);
  if (e == AACENC_OK && frameLength != 0)
    e = aacEncoder_SetParam(h, AACENC_GRANULE_LENGTH, frameLength);
  if (e == AACENC_OK) e = aacEncoder_SetParam(h, AACENC_TRANSMUX, transport);
  if (e == AACENC_OK) e = aacEncEncode(h, NULL, NULL, NULL, NULL);
  *err = (INT)e;
  if (e != AACENC_OK) {
    if (h != NULL) aacEncClose(&h);
    return NULL;
  }
  CrefEnc *c = (CrefEnc *)calloc(1, sizeof(CrefEnc));
  c->h = h;
  c->e = (AACENCODER *)h;
  c->stage = STAGE_DONE;
  c->stackPoison = -1;
  return c;
}
/* Stack poisoning before every stage (-1: off). */
void cref_adj_thr_enc_set_stack_poison(void *h, INT v) { ((CrefEnc *)h)->stackPoison = v; }
/* Closes an encoder of cref_adj_thr_enc_open. */
void cref_adj_thr_enc_close(void *h) {
  CrefEnc *c = (CrefEnc *)h;
  aacEncClose(&c->h);
  free(c);
}
/* out[0..8] (see the file header). */
void cref_adj_thr_enc_info(void *h, INT *out) {
  CrefEnc *c = (CrefEnc *)h;
  HANDLE_AAC_ENC enc = core(c);
  out[0] = enc->channelMapping.nChannels;
  out[1] = enc->channelMapping.nElements;
  out[2] = c->e->nSamplesToRead / (INT)c->e->extParam.nChannels;
  out[3] = c->e->inputBufferSizePerChannel;
  out[4] = enc->aot;
  out[5] = (INT)enc->config->syntaxFlags;
  out[6] = enc->config->epConfig;
  out[7] = enc->config->bitrateMode;
}
/* Copies one frame of planar PCM and starts the frame. */
void cref_adj_thr_enc_begin_frame(void *h, const SHORT *pcm) {
  CrefEnc *c = (CrefEnc *)h;
  AACENCODER *e = c->e;
  const INT nCh = (INT)e->extParam.nChannels;
  const INT len = e->nSamplesToRead / nCh;
  INT_PCM *pIn = e->inputBuffer + e->inputBufferOffset / e->aacConfig.nChannels;
  for (INT ch = 0; ch < nCh; ch++)
    for (INT i = 0; i < len; i++) pIn[ch * e->inputBufferSizePerChannel + i] = pcm[ch * len + i];
  /* init payload (aacEncEncode) */
  FDKmemclear(e->extPayload, sizeof(AACENC_EXT_PAYLOAD) * MAX_TOTAL_EXT_PAYLOADS);
  for (INT i = 0; i < MAX_TOTAL_EXT_PAYLOADS; i++) e->extPayload[i].associatedChElement = -1;
  c->stage = STAGE_BEGIN;
  c->el = 0;
  c->nOutBytes = 0;
}
/* The next stage (STAGE_DONE when the frame is complete); *el its element. */
INT cref_adj_thr_enc_next(void *h, INT *el) {
  CrefEnc *c = (CrefEnc *)h;
  *el = c->el;
  return c->stage;
}
/* Runs the next stage; returns its AAC_ENCODER_ERROR (an error ends the frame). */
INT cref_adj_thr_enc_run(void *h) {
  CrefEnc *c = (CrefEnc *)h;
  if (c->stage >= STAGE_DONE) return 0;
  if (c->stackPoison >= 0) poisonStack(c->stackPoison);
  AAC_ENCODER_ERROR err = runStage(c);
  if (err != AAC_ENC_OK) {
    c->stage = STAGE_DONE;
    return (INT)err;
  }
  advance(c);
  return 0;
}
/* Number of bytes of the last access unit (after STAGE_FINALIZE) and its data. */
INT cref_adj_thr_enc_output(void *h, UCHAR *out, INT outSize) {
  CrefEnc *c = (CrefEnc *)h;
  INT n = fMin(c->nOutBytes, outSize);
  FDKmemcpy(out, c->e->outBuffer, n);
  return c->nOutBytes;
}
/* A whole frame through the stages; returns the error, *nBytes the access unit size. */
INT cref_adj_thr_enc_frame(void *h, const SHORT *pcm, UCHAR *out, INT outSize, INT *nBytes) {
  cref_adj_thr_enc_begin_frame(h, pcm);
  CrefEnc *c = (CrefEnc *)h;
  while (c->stage < STAGE_DONE) {
    INT err = cref_adj_thr_enc_run(h);
    if (err != 0) {
      *nBytes = 0;
      return err;
    }
  }
  *nBytes = cref_adj_thr_enc_output(h, out, outSize);
  return 0;
}
/* A whole frame through the library's aacEncEncode (interleaved PCM of one frame). */
INT cref_adj_thr_enc_lib_frame(void *h, const SHORT *pcmInterleaved, UCHAR *out, INT outSize, INT *nBytes) {
  CrefEnc *c = (CrefEnc *)h;
  AACENC_BufDesc in = {0}, outDesc = {0};
  AACENC_InArgs inArgs = {0};
  AACENC_OutArgs outArgs = {0};
  void *inBuf = (void *)pcmInterleaved;
  INT inId = IN_AUDIO_DATA, inSize = c->e->nSamplesToRead * (INT)sizeof(SHORT),
      inEl = sizeof(SHORT);
  void *outBuf = out;
  INT outId = OUT_BITSTREAM_DATA, outEl = 1;
  in.numBufs = 1;
  in.bufs = &inBuf;
  in.bufferIdentifiers = &inId;
  in.bufSizes = &inSize;
  in.bufElSizes = &inEl;
  outDesc.numBufs = 1;
  outDesc.bufs = &outBuf;
  outDesc.bufferIdentifiers = &outId;
  outDesc.bufSizes = &outSize;
  outDesc.bufElSizes = &outEl;
  inArgs.numInSamples = c->e->nSamplesToRead;
  AACENC_ERROR err = aacEncEncode(c->h, &in, &outDesc, &inArgs, &outArgs);
  *nBytes = outArgs.numOutBytes;
  return (INT)err;
}
/* vars = {avgTotalBits, totalAvailableBits, avgTotalDynBits, isCBRAdjustment} */
void cref_adj_thr_enc_qc_vars(void *h, INT *vars) {
  CrefEnc *c = (CrefEnc *)h;
  vars[0] = c->avgTotalBits;
  vars[1] = c->totalAvailableBits;
  vars[2] = c->avgTotalDynBits;
  vars[3] = c->isCBRAdjustment;
}
/* Sets the variables of cref_adj_thr_enc_qc_vars. */
void cref_adj_thr_enc_set_qc_vars(void *h, const INT *vars) {
  CrefEnc *c = (CrefEnc *)h;
  c->avgTotalBits = vars[0];
  c->totalAvailableBits = vars[1];
  c->avgTotalDynBits = vars[2];
  c->isCBRAdjustment = vars[3];
}
/* Pointer to a C structure (enum Ptr), NULL for an invalid index. */
void *cref_adj_thr_enc_ptr(void *h, INT what, INT idx) {
  CrefEnc *c = (CrefEnc *)h;
  HANDLE_AAC_ENC enc = core(c);
  switch (what) {
    case PTR_AAC_ENC: return enc;
    case PTR_CONFIG: return enc->config;
    case PTR_CHANNEL_MAPPING: return &enc->channelMapping;
    case PTR_QC_STATE: return enc->qcKernel;
    case PTR_ADJ_THR: return enc->qcKernel->hAdjThr;
    case PTR_ATS_ELEMENT: return (idx >= 0 && idx < 8) ? enc->qcKernel->hAdjThr->adjThrStateElem[idx] : NULL;
    case PTR_ELEMENT_BITS: return (idx >= 0 && idx < 8) ? enc->qcKernel->elementBits[idx] : NULL;
    case PTR_QC_OUT: return enc->qcOut[0];
    case PTR_QC_ELEMENT: return (idx >= 0 && idx < 8) ? enc->qcOut[0]->qcElement[idx] : NULL;
    case PTR_QC_CHANNEL: return (idx >= 0 && idx < 8) ? enc->qcOut[0]->pQcOutChannels[idx] : NULL;
    case PTR_PSY_OUT: return enc->psyOut[0];
    case PTR_PSY_ELEMENT_OUT: return (idx >= 0 && idx < 8) ? enc->psyOut[0]->psyOutElement[idx] : NULL;
    case PTR_PSY_CHANNEL: return (idx >= 0 && idx < 8) ? enc->psyOut[0]->pPsyOutChannels[idx] : NULL;
    case PTR_PSY_INTERNAL: return enc->psyKernel;
    case PTR_PSY_ELEMENT: return (idx >= 0 && idx < 8) ? enc->psyKernel->psyElement[idx] : NULL;
    case PTR_PSY_STATIC: return (idx >= 0 && idx < 8) ? enc->psyKernel->pStaticChannels[idx] : NULL;
    case PTR_PSY_DYNAMIC: return enc->psyKernel->psyDynamic;
    case PTR_BIT_COUNTER: return enc->qcKernel->hBitCounter;
    case PTR_TP_ENC: return c->e->hTpEnc;
    case PTR_INPUT: return c->e->inputBuffer;
    case PTR_DYN_RAM: return enc->dynamic_RAM;
    case PTR_PSY_CONF: return (idx >= 0 && idx < 2) ? &enc->psyKernel->psyConf[idx] : NULL;
    default: return NULL;
  }
}
/* Offset/size entry i (name, *value); NULL after the last entry. */
const char *cref_adj_thr_enc_offset(INT i, INT *value) {
  if (i < 0 || i >= (INT)(sizeof(offsets) / sizeof(offsets[0]))) return NULL;
  *value = offsets[i].value;
  return offsets[i].name;
}
/* Reads an INT array (enum Ints); returns the number of values. */
INT cref_adj_thr_enc_get_ints(void *h, INT what, INT idx, INT *v) {
  CrefEnc *c = (CrefEnc *)h;
  HANDLE_AAC_ENC enc = core(c);
  INT n = 0;
  switch (what) {
    case INTS_QC_OUT: {
      QC_OUT *q = enc->qcOut[0];
      v[n++] = q->nExtensions; v[n++] = q->maxDynBits; v[n++] = q->grantedDynBits;
      v[n++] = q->totFillBits; v[n++] = q->elementExtBits; v[n++] = q->globalExtBits;
      v[n++] = q->staticBits; v[n++] = q->totalNoRedPe; v[n++] = q->totalGrantedPeCorr;
      v[n++] = q->usedDynBits; v[n++] = q->alignBits; v[n++] = q->totalBits;
      for (int k = 0; k < (2 + 2); k++) {
        v[n++] = q->extension[k].type;
        v[n++] = q->extension[k].nPayloadBits;
      }
      break;
    }
    case INTS_QC_ELEMENT: {
      QC_OUT_ELEMENT *q = enc->qcOut[0]->qcElement[idx];
      v[n++] = q->staticBitsUsed; v[n++] = q->dynBitsUsed; v[n++] = q->extBitsUsed;
      v[n++] = q->nExtensions; v[n++] = q->extension[0].type;
      v[n++] = q->extension[0].nPayloadBits; v[n++] = q->grantedDynBits; v[n++] = q->grantedPe;
      v[n++] = q->grantedPeCorr;
      for (int ch = 0; ch < 2; ch++)
        v[n++] = (ch < enc->channelMapping.elInfo[idx].nChannelsInEl)
                     ? findPtr((void *const *)enc->qcOut[0]->pQcOutChannels, 8, q->qcOutChannel[ch])
                     : -1;
      break;
    }
    case INTS_QC_STATE: {
      QC_STATE *q = enc->qcKernel;
      v[n++] = q->globHdrBits; v[n++] = q->maxBitsPerFrame; v[n++] = q->minBitsPerFrame;
      v[n++] = q->nElements; v[n++] = q->bitrateMode; v[n++] = q->bitResMode;
      v[n++] = q->bitResTot; v[n++] = q->bitResTotMax; v[n++] = q->maxIterations;
      v[n++] = q->invQuant; v[n++] = q->vbrQualFactor; v[n++] = q->maxBitFac;
      v[n++] = q->padding.paddingRest; v[n++] = q->dZoneQuantEnable;
      break;
    }
    case INTS_ADJ_THR: {
      ADJ_THR_STATE *a = enc->qcKernel->hAdjThr;
      FDKmemcpy(v, &a->bresParamLong, sizeof(BRES_PARAM));
      FDKmemcpy(v + 8, &a->bresParamShort, sizeof(BRES_PARAM));
      n = 16;
      v[n++] = a->bitDistributionMode;
      v[n++] = a->maxIter2ndGuess;
      break;
    }
    case INTS_PSY_OUT_ELEMENT: {
      PSY_OUT_ELEMENT *p = enc->psyOut[0]->psyOutElement[idx];
      for (int ch = 0; ch < 2; ch++)
        v[n++] = (ch < enc->channelMapping.elInfo[idx].nChannelsInEl)
                     ? findPtr((void *const *)enc->psyOut[0]->pPsyOutChannels, 8, p->psyOutChannel[ch])
                     : -1;
      v[n++] = p->commonWindow;
      break;
    }
    case INTS_PSY_ELEMENT: {
      PSY_ELEMENT *p = enc->psyKernel->psyElement[idx];
      for (int ch = 0; ch < 2; ch++)
        v[n++] = (ch < enc->channelMapping.elInfo[idx].nChannelsInEl)
                     ? findPtr((void *const *)enc->psyKernel->pStaticChannels, 8, p->psyStatic[ch])
                     : -1;
      break;
    }
    case INTS_AAC_ENC:
      v[n++] = enc->ancillaryBitsPerFrame; v[n++] = enc->encoderMode;
      v[n++] = enc->bandwidth90dB; v[n++] = enc->bitrateMode; v[n++] = enc->dontWriteAdif;
      v[n++] = enc->maxChannels; v[n++] = enc->maxElements; v[n++] = enc->maxFrames;
      v[n++] = enc->aot;
      break;
    case INTS_MDCT: {
      mdct_t *m = &enc->psyKernel->pStaticChannels[idx]->mdctPers;
      v[n++] = m->prev_tl; v[n++] = m->prev_nr; v[n++] = m->prev_fr; v[n++] = m->ov_offset;
      v[n++] = m->ov_size; v[n++] = m->prevAliasSymmetry; v[n++] = m->prevPrevAliasSymmetry;
      INT len = -1, shape = -1;
      if (m->prev_wrs != NULL) {
        static const INT lengths[] = {4, 8, 15, 16, 30, 32, 60, 64, 120, 128, 240, 256, 480, 512, 960, 1024, 2048};
        len = -2;
        shape = -2;
        for (int k = 0; k < (int)(sizeof(lengths) / sizeof(lengths[0])) && len == -2; k++)
          for (int s = 0; s < 2; s++)
            if (FDKgetWindowSlope(lengths[k], s) == m->prev_wrs) {
              len = lengths[k];
              shape = s;
              break;
            }
      }
      v[n++] = len; v[n++] = shape;
      v[n++] = m->overlap.freq != NULL; v[n++] = m->pFacZir != NULL; v[n++] = m->pAsymOvlp != NULL;
      break;
    }
    default:
      break;
  }
  return n;
}
/* Writes an INT array of cref_adj_thr_enc_get_ints (read-only entries are ignored). */
void cref_adj_thr_enc_set_ints(void *h, INT what, INT idx, const INT *v) {
  CrefEnc *c = (CrefEnc *)h;
  HANDLE_AAC_ENC enc = core(c);
  INT n = 0;
  switch (what) {
    case INTS_QC_OUT: {
      QC_OUT *q = enc->qcOut[0];
      q->nExtensions = v[n++]; q->maxDynBits = v[n++]; q->grantedDynBits = v[n++];
      q->totFillBits = v[n++]; q->elementExtBits = v[n++]; q->globalExtBits = v[n++];
      q->staticBits = v[n++]; q->totalNoRedPe = v[n++]; q->totalGrantedPeCorr = v[n++];
      q->usedDynBits = v[n++]; q->alignBits = v[n++]; q->totalBits = v[n++];
      for (int k = 0; k < (2 + 2); k++) {
        q->extension[k].type = (EXT_PAYLOAD_TYPE)v[n++];
        q->extension[k].nPayloadBits = v[n++];
      }
      break;
    }
    case INTS_QC_ELEMENT: {
      QC_OUT_ELEMENT *q = enc->qcOut[0]->qcElement[idx];
      q->staticBitsUsed = v[n++]; q->dynBitsUsed = v[n++]; q->extBitsUsed = v[n++];
      q->nExtensions = v[n++]; q->extension[0].type = (EXT_PAYLOAD_TYPE)v[n++];
      q->extension[0].nPayloadBits = v[n++]; q->grantedDynBits = v[n++];
      q->grantedPe = v[n++]; q->grantedPeCorr = v[n++];
      break;
    }
    case INTS_QC_STATE: {
      QC_STATE *q = enc->qcKernel;
      q->globHdrBits = v[n++]; q->maxBitsPerFrame = v[n++]; q->minBitsPerFrame = v[n++];
      q->nElements = v[n++]; q->bitrateMode = (QCDATA_BR_MODE)v[n++];
      q->bitResMode = (AACENC_BITRES_MODE)v[n++]; q->bitResTot = v[n++];
      q->bitResTotMax = v[n++]; q->maxIterations = v[n++]; q->invQuant = v[n++];
      q->vbrQualFactor = v[n++]; q->maxBitFac = v[n++]; q->padding.paddingRest = v[n++];
      q->dZoneQuantEnable = v[n++];
      break;
    }
    case INTS_ADJ_THR: {
      ADJ_THR_STATE *a = enc->qcKernel->hAdjThr;
      FDKmemcpy(&a->bresParamLong, v, sizeof(BRES_PARAM));
      FDKmemcpy(&a->bresParamShort, v + 8, sizeof(BRES_PARAM));
      a->bitDistributionMode = (AACENC_BIT_DISTRIBUTION_MODE)v[16];
      a->maxIter2ndGuess = v[17];
      break;
    }
    case INTS_PSY_OUT_ELEMENT:
      enc->psyOut[0]->psyOutElement[idx]->commonWindow = v[2];
      break;
    case INTS_AAC_ENC:
      enc->ancillaryBitsPerFrame = v[0]; enc->encoderMode = (CHANNEL_MODE)v[1];
      enc->bandwidth90dB = v[2]; enc->bitrateMode = (AACENC_BITRATE_MODE)v[3];
      enc->dontWriteAdif = v[4]; enc->maxChannels = v[5]; enc->maxElements = v[6];
      enc->maxFrames = v[7]; enc->aot = (AUDIO_OBJECT_TYPE)v[8];
      break;
    case INTS_MDCT: {
      mdct_t *m = &enc->psyKernel->pStaticChannels[idx]->mdctPers;
      m->prev_tl = v[0]; m->prev_nr = v[1]; m->prev_fr = v[2]; m->ov_offset = v[3];
      m->ov_size = v[4]; m->prevAliasSymmetry = v[5]; m->prevPrevAliasSymmetry = v[6];
      break;
    }
    default:
      break;
  }
}
}
