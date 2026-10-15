// Shims for the differential tests of fdk_aac_rs_aacdec::channelinfo.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_channelinfo (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/channelinfo.h"
#include "aacdecoder_lib.h"

/* CIcsInfo <-> INT[16]: WindowGroupLength[8], WindowGroups, Valid, WindowShape, WindowSequence,
   MaxSfBands, max_sfb_ste, ScaleFactorGrouping, TotalSfBands. */
static void ics_from(CIcsInfo *p, const INT *v) {
  for (int i = 0; i < 8; i++) p->WindowGroupLength[i] = (UCHAR)v[i];
  p->WindowGroups = (UCHAR)v[8];
  p->Valid = (UCHAR)v[9];
  p->WindowShape = (UCHAR)v[10];
  p->WindowSequence = (BLOCK_TYPE)v[11];
  p->MaxSfBands = (UCHAR)v[12];
  p->max_sfb_ste = (UCHAR)v[13];
  p->ScaleFactorGrouping = (UCHAR)v[14];
  p->TotalSfBands = (UCHAR)v[15];
}
static void ics_to(const CIcsInfo *p, INT *v) {
  for (int i = 0; i < 8; i++) v[i] = p->WindowGroupLength[i];
  v[8] = p->WindowGroups;
  v[9] = p->Valid;
  v[10] = p->WindowShape;
  v[11] = (INT)p->WindowSequence;
  v[12] = p->MaxSfBands;
  v[13] = p->max_sfb_ste;
  v[14] = p->ScaleFactorGrouping;
  v[15] = p->TotalSfBands;
}

/* SamplingRateInfo -> INT[4] (NumberOfScaleFactorBands_Long/Short, samplingRateIndex,
   samplingRate) and the two pointers. */
static void sri_to(const SamplingRateInfo *t, INT *v, const SHORT **lng, const SHORT **sht) {
  v[0] = t->NumberOfScaleFactorBands_Long;
  v[1] = t->NumberOfScaleFactorBands_Short;
  v[2] = (INT)t->samplingRateIndex;
  v[3] = (INT)t->samplingRate;
  *lng = t->ScaleFactorBands_Long;
  *sht = t->ScaleFactorBands_Short;
}

extern "C" {
INT cref_ci_get_sampling_rate_info(UINT samplesPerFrame, UINT samplingRateIndex, UINT samplingRate, INT *fields, const SHORT **lng, const SHORT **sht) {
  SamplingRateInfo t;
  FDKmemclear(&t, sizeof(t));
  INT err = (INT)getSamplingRateInfo(&t, samplesPerFrame, samplingRateIndex, samplingRate);
  sri_to(&t, fields, lng, sht);
  return err;
}

/* IcsRead (which = 0) or IcsReadMaxSfb (which = 1) on a bitstream over buf (bufBytes: a power of
   two, all bits valid), with the SamplingRateInfo of getSamplingRateInfo(spf, sri, sr). */
INT cref_ci_ics_read(INT which, UCHAR *buf, INT bufBytes, UINT flags, UINT spf, UINT sri, UINT sr, INT *ics, INT *bitsLeft) {
  SamplingRateInfo t;
  FDKmemclear(&t, sizeof(t));
  getSamplingRateInfo(&t, spf, sri, sr);
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, (UINT)bufBytes, (UINT)bufBytes * 8, BS_READER);
  CIcsInfo p;
  ics_from(&p, ics);
  INT err = (which == 0) ? (INT)IcsRead(&bs, &p, &t, flags) : (INT)IcsReadMaxSfb(&bs, &p, &t);
  ics_to(&p, ics);
  *bitsLeft = (INT)FDKgetValidBits(&bs);
  return err;
}

/* The inline functions of channelinfo.h on ics0/ics1 and the SamplingRateInfo of
   getSamplingRateInfo(spf, sri, sr). */
INT cref_ci_inline(const INT *ics0, const INT *ics1, UINT spf, UINT sri, UINT sr, INT idx, INT tnsSri, INT *out, const SHORT **offsets) {
  SamplingRateInfo t;
  FDKmemclear(&t, sizeof(t));
  getSamplingRateInfo(&t, spf, sri, sr);
  CIcsInfo a, b;
  ics_from(&a, ics0);
  ics_from(&b, ics1);
  INT n = 0;
  out[n++] = IsValid(&a);
  out[n++] = IsLongBlock(&a);
  out[n++] = GetWindowShape(&a);
  out[n++] = (INT)GetWindowSequence(&a);
  out[n++] = GetNumberOfScaleFactorBands(&a, &t);
  out[n++] = GetWindowsPerFrame(&a);
  out[n++] = GetWindowGroups(&a);
  out[n++] = GetWindowGroupLength(&a, idx);
  out[n++] = GetScaleFactorBandsTransmitted(&a);
  out[n++] = GetScaleMaxFactorBandsTransmitted(&a, &b);
  out[n++] = GetScaleFactorBandsTotal(&a);
  out[n++] = GetMaximumTnsBands(&a, tnsSri);
  out[n++] = (GetWindowGroupLengthTable(&a) == a.WindowGroupLength) ? 1 : 0;
  *offsets = GetScaleFactorBandOffsets(&a, &t);
  return n;
}

#define AL1(s, f) out[n++] = (INT)(sizeof(((s *)0)->f) / sizeof(((s *)0)->f[0]))
#define AL2(s, f) out[n++] = (INT)(sizeof(((s *)0)->f) / sizeof(((s *)0)->f[0][0]))

/* The number of elements of the array fields of the structures, in the order of the Rust test. */
INT cref_ci_array_lengths(INT *out) {
  INT n = 0;
  AL1(CIcsInfo, WindowGroupLength);
  AL1(CAacDecoderStaticChannelInfo, old_synth);
  AL1(CAacDecoderStaticChannelInfo, old_T_pf);
  AL1(CAacDecoderStaticChannelInfo, old_gain_pf);
  AL1(CAacDecoderStaticChannelInfo, mem_bpf);
  AL1(CAacDecoderStaticChannelInfo, lpc4_lsf);
  AL1(CAacDecoderStaticChannelInfo, lsf_adaptive_mean);
  AL2(CAacDecoderStaticChannelInfo, lp_coeff_old);
  AL1(CAacDecoderStaticChannelInfo, lp_coeff_old_exp);
  AL1(CAacDecoderStaticChannelInfo, last_alfd_gains);
  AL1(CAacDecoderDynamicData, aScaleFactor);
  AL1(CAacDecoderDynamicData, aSfbScale);
  AL1(CAacDecoderDynamicData, aCodeBook);
  AL1(CAacDecoderDynamicData, band_is_noise);
  AL1(CAacDecoderDynamicData, specificTo.aac.aNumLineInSec4Hcr);
  AL1(CAacDecoderDynamicData, specificTo.aac.aCodeBooks4Hcr);
  AL1(CAacDecoderDynamicData, specificTo.usac.tcx_noise_factor);
  AL1(CAacDecoderDynamicData, specificTo.usac.tcx_global_gain);
  AL1(CWorkBufferCore1, DrmBsBuffer);
  AL1(CWorkBufferCore1, mdctOutTemp);
  AL1(CWorkBufferCore1, synth_buf);
  AL1(CWorkBufferCore1, workBuffer);
  out[n++] = (INT)sizeof(CWorkBufferCore1);
  AL1(CAacDecoderCommonData, pnsRandomSeed);
  AL1(CAacDecoderCommonData, overlay.aac.aRvlcScfEsc);
  AL1(CAacDecoderCommonData, overlay.aac.aRvlcScfFwd);
  AL1(CAacDecoderCommonData, overlay.aac.aRvlcScfBwd);
  AL1(CAacDecoderCommonData, pAacDecoderDynamicData);
  AL1(CAacDecoderChannelInfo, data.usac.fac_data0);
  AL1(CAacDecoderChannelInfo, data.usac.fac_data_e);
  AL1(CAacDecoderChannelInfo, data.usac.fac_data);
  AL1(CAacDecoderChannelInfo, data.usac.mod);
  AL2(CAacDecoderChannelInfo, data.usac.lsp_coeff);
  AL2(CAacDecoderChannelInfo, data.usac.lp_coeff);
  AL1(CAacDecoderChannelInfo, data.usac.lp_coeff_exp);
  AL1(CAacDecoderChannelInfo, data.usac.lsf_adaptive_mean_cand);
  AL1(CAacDecoderChannelInfo, data.usac.aStability);
  AL1(CAacDecoderChannelInfo, data.usac.acelp);
  AL1(CAacDecoderChannelInfo, data.usac.tcx_gain);
  AL1(CAacDecoderChannelInfo, data.usac.tcx_gain_e);
  AL1(CAacDecoderChannelInfo, specScale);
  AL2(CCplxPredictionData, alpha_q_re);
  AL2(CCplxPredictionData, alpha_q_im);
  AL1(CCplxPredictionData, alpha_q_re);
  AL1(CJointStereoData, MsUsed);
  AL1(CJointStereoData, store_dmx_re_prev);
  AL2(CJointStereoPersistentData, alpha_q_re_prev);
  AL2(CJointStereoPersistentData, alpha_q_im_prev);
  AL1(CJointStereoPersistentData, spectralCoeffs);
  AL1(CJointStereoPersistentData, specScale);
  AL1(CPulseData, PulseOffset);
  AL1(CPulseData, PulseAmp);
  AL1(CFilter, Coeff);
  AL2(CTnsData, Filter);
  AL1(CTnsData, Filter);
  AL1(CTnsData, NumberOfFilters);
  AL1(CPnsInterChannelData, correlated);
  AL1(CPnsData, pnsUsed);
  return n;
}

/* The constants of channelinfo.h and of the headers whose types are defined in channelinfo, in
   the order of the Rust test. */
INT cref_ci_constants(INT *out) {
  INT n = 0;
  out[n++] = WB_SECTION_SIZE;
  out[n++] = DRM_BS_BUFFER_SIZE;
  out[n++] = AACDEC_RENDER_INVALID;
  out[n++] = AACDEC_RENDER_IMDCT;
  out[n++] = AACDEC_RENDER_ELDFB;
  out[n++] = AACDEC_RENDER_LPD;
  out[n++] = AACDEC_RENDER_INTIMDCT;
  out[n++] = MAX_QUANTIZED_VALUE;
  out[n++] = FD_LONG;
  out[n++] = FD_SHORT;
  out[n++] = LPD;
  out[n++] = ZERO_HCB;
  out[n++] = ESCBOOK;
  out[n++] = NSPECBOOKS;
  out[n++] = BOOKSCL;
  out[n++] = NOISE_HCB;
  out[n++] = INTENSITY_HCB2;
  out[n++] = INTENSITY_HCB;
  out[n++] = LAST_HCB;
  /* stereo.h */
  out[n++] = SFB_PER_PRED_BAND;
  out[n++] = SR_FNA_OUT;
  out[n++] = JointStereoMaximumGroups;
  out[n++] = JointStereoMaximumBands;
  /* pulsedata.h */
  out[n++] = N_MAX_LINES;
  /* aacdec_tns.h */
  out[n++] = TNS_MAX_WINDOWS;
  out[n++] = TNS_MAXIMUM_FILTERS;
  out[n++] = TNS_MAXIMUM_ORDER;
  /* aacdec_pns.h */
  out[n++] = NO_OFBANDS;
  return n;
}

/* AAC_DECODER_ERROR: the values in the order of the Rust test. */
INT cref_ci_error_codes(INT *out) {
  INT n = 0;
  out[n++] = AAC_DEC_OK;
  out[n++] = AAC_DEC_OUT_OF_MEMORY;
  out[n++] = AAC_DEC_UNKNOWN;
  out[n++] = aac_dec_sync_error_start;
  out[n++] = AAC_DEC_TRANSPORT_SYNC_ERROR;
  out[n++] = AAC_DEC_NOT_ENOUGH_BITS;
  out[n++] = aac_dec_sync_error_end;
  out[n++] = aac_dec_init_error_start;
  out[n++] = AAC_DEC_INVALID_HANDLE;
  out[n++] = AAC_DEC_UNSUPPORTED_AOT;
  out[n++] = AAC_DEC_UNSUPPORTED_FORMAT;
  out[n++] = AAC_DEC_UNSUPPORTED_ER_FORMAT;
  out[n++] = AAC_DEC_UNSUPPORTED_EPCONFIG;
  out[n++] = AAC_DEC_UNSUPPORTED_MULTILAYER;
  out[n++] = AAC_DEC_UNSUPPORTED_CHANNELCONFIG;
  out[n++] = AAC_DEC_UNSUPPORTED_SAMPLINGRATE;
  out[n++] = AAC_DEC_INVALID_SBR_CONFIG;
  out[n++] = AAC_DEC_SET_PARAM_FAIL;
  out[n++] = AAC_DEC_NEED_TO_RESTART;
  out[n++] = AAC_DEC_OUTPUT_BUFFER_TOO_SMALL;
  out[n++] = aac_dec_init_error_end;
  out[n++] = aac_dec_decode_error_start;
  out[n++] = AAC_DEC_TRANSPORT_ERROR;
  out[n++] = AAC_DEC_PARSE_ERROR;
  out[n++] = AAC_DEC_UNSUPPORTED_EXTENSION_PAYLOAD;
  out[n++] = AAC_DEC_DECODE_FRAME_ERROR;
  out[n++] = AAC_DEC_CRC_ERROR;
  out[n++] = AAC_DEC_INVALID_CODE_BOOK;
  out[n++] = AAC_DEC_UNSUPPORTED_PREDICTION;
  out[n++] = AAC_DEC_UNSUPPORTED_CCE;
  out[n++] = AAC_DEC_UNSUPPORTED_LFE;
  out[n++] = AAC_DEC_UNSUPPORTED_GAIN_CONTROL_DATA;
  out[n++] = AAC_DEC_UNSUPPORTED_SBA;
  out[n++] = AAC_DEC_TNS_READ_ERROR;
  out[n++] = AAC_DEC_RVLC_ERROR;
  out[n++] = aac_dec_decode_error_end;
  out[n++] = aac_dec_anc_data_error_start;
  out[n++] = AAC_DEC_ANC_DATA_ERROR;
  out[n++] = AAC_DEC_TOO_SMALL_ANC_BUFFER;
  out[n++] = AAC_DEC_TOO_MANY_ANC_ELEMENTS;
  out[n++] = aac_dec_anc_data_error_end;
  return n;
}

INT cref_ci_is_init_error(INT err) { return IS_INIT_ERROR(err); }
INT cref_ci_is_decode_error(INT err) { return IS_DECODE_ERROR(err); }
INT cref_ci_is_output_valid(INT err) { return IS_OUTPUT_VALID(err); }

/* Evidence for the union decision of CAacDecoderChannelInfo::data: the offset of
   data.usac.lsp_coeff (read by CLpc_Conceal() for every AACDEC_RENDER_IMDCT channel) within the
   union, and the size of data.aac. */
void cref_ci_data_union_layout(INT *lsp_coeff_offset, INT *aac_size) {
  *lsp_coeff_offset = (INT)(offsetof(CAacDecoderChannelInfo, data.usac.lsp_coeff) -
                            offsetof(CAacDecoderChannelInfo, data));
  *aac_size = (INT)sizeof(((CAacDecoderChannelInfo *)0)->data.aac);
}
}
