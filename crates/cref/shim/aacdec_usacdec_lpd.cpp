// Shims for the differential tests of fdk_aac_rs_aacdec::usacdec_lpd and of the transforms of
// fdk_aac_rs_aacdec::usacdec_fac.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_usacdec_lpd (see build.rs).
//
// The channel state is exchanged as INT arrays (the layout is mirrored by the Rust tests in
// tests/aacdec/usacdec_lpd_util.rs):
//
// * "sci": the LPD part of CAacDecoderStaticChannelInfo (sci_store()/sci_load() below), with
//   IMdct (window slope pointers as indices of WIN_CAND, pFacZir as the values it points to),
//   hArCo, the LPD/TCX/ACELP memory and concealmentInfo.prevFrameOk.
// * "ci": the USAC part of CAacDecoderChannelInfo (ci_store()/ci_load()), with the FAC data
//   pointers encoded as -1 (NULL), -2 (fac_data0), -3 (another buffer) or the offset into the
//   spectrum.
// * the spectrum (pSpectralCoefficient, 1024 values), the work buffer core 1 (2048 values) and
//   bitstreams (st[9] + buf, as in aacdec_usacdec_lpc.cpp) separately.
//
// The file-static functions are reached through a copy of usacdec_lpd.cpp and usacdec_fac.cpp
// in namespaces; the other functions are the library's own.
//
// The capture decoder (cref_cap_*, at the end) is a second copy of the decoder (block.cpp,
// channel.cpp, aacdecoder.cpp, aacdecoder_lib.cpp) compiled into this file, with every external
// function renamed to cap_* (capdec_* for the API) by the macros below, which are defined before
// any header is included (so that the prototypes of the headers are renamed as well). In the
// copy, the calls of the LPD functions are redirected to hooks that record their inputs and
// outputs; the tests replay the recorded calls with the Rust functions.

#include "cref_common.h"
#include <stddef.h>
#include <vector>

/* The external functions of block.cpp, channel.cpp and aacdecoder.cpp (nm of the library). */
#define ApplyTools cap_ApplyTools
#define CBlock_GetEscape cap_CBlock_GetEscape
#define CBlock_ApplyNoise cap_CBlock_ApplyNoise
#define CBlock_FrequencyToTime cap_CBlock_FrequencyToTime
#define CBlock_ReadSectionData cap_CBlock_ReadSectionData
#define CBlock_ReadSpectralData cap_CBlock_ReadSpectralData
#define CBlock_ScaleSpectralData cap_CBlock_ScaleSpectralData
#define CBlock_ReadAcSpectralData cap_CBlock_ReadAcSpectralData
#define CBlock_ReadScaleFactorData cap_CBlock_ReadScaleFactorData
#define CBlock_FrequencyToTimeLowDelay cap_CBlock_FrequencyToTimeLowDelay
#define CBlock_InverseQuantizeSpectralData cap_CBlock_InverseQuantizeSpectralData
#define get_gain cap_get_gain
#define CChannelElement_Read cap_CChannelElement_Read
#define CChannelElement_Decode cap_CChannelElement_Decode
#define CChannel_CodebookTableInit cap_CChannel_CodebookTableInit
#define CAacDecoder_Init cap_CAacDecoder_Init
#define CAacDecoder_Open cap_CAacDecoder_Open
#define CAacDecoder_Close cap_CAacDecoder_Close
#define CAacDecoder_FreeMem cap_CAacDecoder_FreeMem
#define CAacDecoder_AncDataGet cap_CAacDecoder_AncDataGet
#define CAacDecoder_AncDataInit cap_CAacDecoder_AncDataInit
#define CAacDecoder_DecodeFrame cap_CAacDecoder_DecodeFrame
#define CAacDecoder_SyncQmfMode cap_CAacDecoder_SyncQmfMode
#define CAacDecoder_CtrlCFGChange cap_CAacDecoder_CtrlCFGChange
#define CAacDecoder_GetStreamInfo cap_CAacDecoder_GetStreamInfo
#define CAacDecoder_ApplyCrossFade cap_CAacDecoder_ApplyCrossFade
#define CAacDecoder_PrepareCrossFade cap_CAacDecoder_PrepareCrossFade
#define CAacDecoder_SignalInterruption cap_CAacDecoder_SignalInterruption
#define CAacDecoder_PreRollExtensionPayloadParse cap_CAacDecoder_PreRollExtensionPayloadParse
/* The API of aacdecoder_lib.cpp (renamed to cref_* by the prefix header). */
#undef aacDecoder_AncDataGet
#define aacDecoder_AncDataGet capdec_AncDataGet
#undef aacDecoder_AncDataInit
#define aacDecoder_AncDataInit capdec_AncDataInit
#undef aacDecoder_Close
#define aacDecoder_Close capdec_Close
#undef aacDecoder_ConfigRaw
#define aacDecoder_ConfigRaw capdec_ConfigRaw
#undef aacDecoder_DecodeFrame
#define aacDecoder_DecodeFrame capdec_DecodeFrame
#undef aacDecoder_Fill
#define aacDecoder_Fill capdec_Fill
#undef aacDecoder_GetFreeBytes
#define aacDecoder_GetFreeBytes capdec_GetFreeBytes
#undef aacDecoder_GetLibInfo
#define aacDecoder_GetLibInfo capdec_GetLibInfo
#undef aacDecoder_GetStreamInfo
#define aacDecoder_GetStreamInfo capdec_GetStreamInfo
#undef aacDecoder_Open
#define aacDecoder_Open capdec_Open
#undef aacDecoder_RawISOBMFFData
#define aacDecoder_RawISOBMFFData capdec_RawISOBMFFData
#undef aacDecoder_SetParam
#define aacDecoder_SetParam capdec_SetParam

#include "aacdecoder_lib.h"
#include "libAACdec/src/aac_ram.h"
#include "libAACdec/src/aac_rom.h"
#include "libAACdec/src/aacdecoder.h"
#include "libAACdec/src/channel.h"
#include "libAACdec/src/aacdec_tns.h"
#include "libAACdec/src/aacdec_pns.h"
#include "libAACdec/src/aacdec_hcr.h"
#include "libAACdec/src/rvlc.h"
#include "libAACdec/src/aacdec_drc.h"
#include "libAACdec/src/ldfiltbank.h"
#include "FDK_bitstream.h"
#include "FDK_audio.h"
#include "FDK_core.h"
#include "FDK_crc.h"
#include "FDK_trigFcts.h"
#include "scale.h"
#include "tpdec_lib.h"
#include "sbrdecoder.h"
#include "sac_dec_lib.h"
#include "pcm_utils.h"

#include "libAACdec/src/channelinfo.h"
#include "libAACdec/src/usacdec_lpd.h"
#include "libAACdec/src/usacdec_rom.h"
#include "libAACdec/src/usacdec_fac.h"
#include "libAACdec/src/usacdec_lpc.h"
#include "FDK_tools_rom.h"
#include "fft.h"
#include "mdct.h"
#include "dct.h"
#include "libAACdec/src/usacdec_acelp.h"
#include "libAACdec/src/overlapadd.h"
#include "libAACdec/src/conceal.h"
#include "libAACdec/src/block.h"
#include "ac_arith_coder.h"

/* The hooks of the capture decoder (defined at the end). */
AAC_DECODER_ERROR cap_hook_read(HANDLE_FDK_BITSTREAM hBs, CAacDecoderChannelInfo *ci, CAacDecoderStaticChannelInfo *sci, const SamplingRateInfo *sri, UINT flags);
void cap_hook_decode(CAacDecoderChannelInfo *ci, CAacDecoderStaticChannelInfo *sci, UINT flags);
AAC_DECODER_ERROR cap_hook_render(CAacDecoderStaticChannelInfo *sci, CAacDecoderChannelInfo *ci, PCM_DEC *pTimeData, INT lFrame, SamplingRateInfo *sri, UINT frameOk, const INT aacOutDataHeadroom, UINT flags, UINT strmFlags);
INT cap_hook_acelp2mdct(H_MDCT hMdct, FIXP_DBL *output, FIXP_DBL *pSpec, const SHORT spec_scale[], const int nSpec, FIXP_DBL *pFac_data, const int fac_data_e, const INT fac_length, INT nrSamples, const INT tl, const FIXP_WTP *wrs, const INT fr, FIXP_LPC A[16], INT A_exp, CAcelpStaticMem *acelp_mem, const FIXP_DBL gain, const int last_frame_lost, const int isFdFac, const UCHAR last_lpd, const int k, int currAliasingSymmetry);
int cap_hook_fac_read(HANDLE_FDK_BITSTREAM hBs, FIXP_DBL *pFac, SCHAR *pFacScale, int length, int use_gain, int frame);
void cap_hook_bass_pf(FIXP_DBL syn[], const INT T_sf[], FIXP_DBL *pit_gain, const int frame_length, const INT l_frame, const INT l_next, PCM_DEC *synth_out, const INT aacOutDataHeadroom, FIXP_DBL mem_bpf[]);

#define CLpdChannelStream_Read cap_hook_read
#define CLpdChannelStream_Decode cap_hook_decode
#define CLpd_RenderTimeSignal cap_hook_render
#define CLpd_FAC_Acelp2Mdct cap_hook_acelp2mdct
#define CLpd_FAC_Read cap_hook_fac_read
#define bass_pf_1sf_delay cap_hook_bass_pf

#include "libAACdec/src/block.cpp"
#include "libAACdec/src/channel.cpp"
#include "libAACdec/src/aacdecoder.cpp"
#include "libAACdec/src/aacdecoder_lib.cpp"

#undef CLpdChannelStream_Read
#undef CLpdChannelStream_Decode
#undef CLpd_RenderTimeSignal
#undef CLpd_FAC_Acelp2Mdct
#undef CLpd_FAC_Read
#undef bass_pf_1sf_delay

/* The library's get_gain() (block.h declared the renamed copy). */
#undef get_gain
FIXP_DBL get_gain(const FIXP_DBL *x, const FIXP_DBL *y, int n);

namespace ref_lpd {
#include "libAACdec/src/usacdec_lpd.cpp"
}
namespace ref_lpd_fac {
#include "libAACdec/src/usacdec_fac.cpp"
}

/* Non-static functions that no header declares: the library's own code. */
void CLpd_AdaptLowFreqDeemph(FIXP_DBL x[], int lg, FIXP_DBL alfd_gains[], INT s);
void lpc2mdctAndNoiseShaping(FIXP_DBL *r, SHORT *pScale, const INT lg, const INT fdns_npts, const FIXP_LPC *A1, const INT A1_exp, const FIXP_LPC *A2, const INT A2_exp);
void CLpd_FdnsDecode(CAacDecoderChannelInfo *pAacDecoderChannelInfo, CAacDecoderStaticChannelInfo *pAacDecoderStaticChannelInfo, FIXP_DBL r[], const INT lg, const INT frame, SHORT *pScale, const FIXP_LPC A1[M_LP_FILTER_ORDER], const INT A1_exp, const FIXP_LPC A2[M_LP_FILTER_ORDER], const INT A2_exp, FIXP_DBL pAlfdGains[LFAC / 4], const INT fdns_npts);

/* ---------------------------------------------------------------------------------------------
 * Serialization
 * ------------------------------------------------------------------------------------------- */

#define OVL_MAX 1024
#define ZIR_MAX (PIT_MAX_MAX + L_INTERPOL)
#define ACELP_MEM_INTS (PIT_MAX_MAX + L_INTERPOL + 2 * M_LP_FILTER_ORDER + 10)
#define ACELP_DATA_INTS (2 + 3 * NB_SUBFR + 8 * NB_SUBFR + NB_SUBFR)
#define ARCO_PREV ((1024 / 2) + 4)

/* The window slopes an IMdct can point to: (length, shape) for FDKgetWindowSlope(). */
static const int WIN_CAND[][2] = {
    {8, 0},    {16, 0},  {32, 0},  {64, 0},  {128, 0}, {256, 0},  {512, 0},  {1024, 0},
    {120, 0},  {240, 0}, {480, 0}, {960, 0}, {12, 0},  {24, 0},   {48, 0},   {96, 0},
    {192, 0},  {384, 0}, {768, 0}, {20, 0},  {40, 0},  {160, 0},  {128, 1},  {256, 1},
    {1024, 1}, {120, 1}, {960, 1}, {96, 1},  {768, 1}, {512, 1},  {480, 1},  {192, 1}};
#define N_WIN_CAND ((int)(sizeof(WIN_CAND) / sizeof(WIN_CAND[0])))

static INT win_id(const FIXP_WTP *w) {
  if (w == NULL) return -1;
  for (int i = 0; i < N_WIN_CAND; i++) {
    if (FDKgetWindowSlope(WIN_CAND[i][0], WIN_CAND[i][1]) == w) return i;
  }
  return -2;
}

static const FIXP_WTP *win_ptr(INT id) {
  if (id < 0 || id >= N_WIN_CAND) return NULL;
  return FDKgetWindowSlope(WIN_CAND[id][0], WIN_CAND[id][1]);
}

static void acelp_mem_store(const CAcelpStaticMem *m, INT *v, int &n) {
  for (int i = 0; i < PIT_MAX_MAX + L_INTERPOL; i++) v[n++] = m->old_exc_mem[i];
  for (int i = 0; i < M_LP_FILTER_ORDER; i++) v[n++] = m->old_syn_mem[i];
  for (int i = 0; i < M_LP_FILTER_ORDER; i++) v[n++] = m->A[i];
  v[n++] = m->A_exp;
  v[n++] = m->gc_threshold;
  v[n++] = m->de_emph_mem;
  v[n++] = m->past_gpit;
  v[n++] = m->past_gcode;
  v[n++] = m->old_T0;
  v[n++] = m->old_T0_frac;
  v[n++] = m->deemph_mem_wsyn;
  v[n++] = m->wsyn_rms;
  v[n++] = m->seed_ace;
}

static void acelp_mem_load(CAcelpStaticMem *m, const INT *v, int &n) {
  for (int i = 0; i < PIT_MAX_MAX + L_INTERPOL; i++) m->old_exc_mem[i] = v[n++];
  for (int i = 0; i < M_LP_FILTER_ORDER; i++) m->old_syn_mem[i] = v[n++];
  for (int i = 0; i < M_LP_FILTER_ORDER; i++) m->A[i] = (FIXP_SGL)v[n++];
  m->A_exp = v[n++];
  m->gc_threshold = v[n++];
  m->de_emph_mem = v[n++];
  m->past_gpit = (FIXP_SGL)v[n++];
  m->past_gcode = v[n++];
  m->old_T0 = (USHORT)v[n++];
  m->old_T0_frac = (UCHAR)v[n++];
  m->deemph_mem_wsyn = v[n++];
  m->wsyn_rms = v[n++];
  m->seed_ace = (SHORT)v[n++];
}

/* The memory of a loaded channel: the overlap buffer, the arithmetic decoder state and the
   memory "other" FAC pointers point to. */
struct LpdMem {
  FIXP_DBL ovl[OVL_MAX];
  CArcoData arco;
  FIXP_DBL scratch[LFAC];
};

static int sci_store(const CAacDecoderStaticChannelInfo *s, INT *v) {
  int n = 0;
  const mdct_t *m = &s->IMdct;
  v[n++] = win_id(m->prev_wrs);
  v[n++] = m->prev_tl;
  v[n++] = m->prev_nr;
  v[n++] = m->prev_fr;
  v[n++] = m->ov_offset;
  v[n++] = m->ov_size;
  v[n++] = m->prevAliasSymmetry;
  v[n++] = m->prevPrevAliasSymmetry;
  /* pFacZir: the values from the pointer to the end of the ACELP excitation memory */
  {
    INT nvalid = 0;
    const FIXP_DBL *z = m->pFacZir;
    if (z != NULL) {
      if (z >= s->acelp.old_exc_mem && z < s->acelp.old_exc_mem + ZIR_MAX) {
        nvalid = (INT)(s->acelp.old_exc_mem + ZIR_MAX - z);
      } else {
        nvalid = -1; /* points elsewhere */
      }
    }
    v[n++] = (z != NULL) ? 1 : 0;
    v[n++] = nvalid;
    for (int i = 0; i < ZIR_MAX; i++) v[n++] = (z != NULL && i < nvalid) ? z[i] : 0;
  }
  v[n++] = (m->pAsymOvlp != NULL) ? 1 : 0;
  for (int i = 0; i < OVL_MAX; i++) v[n++] = (i < m->ov_size && m->overlap.freq != NULL) ? m->overlap.freq[i] : 0;
  /* hArCo */
  v[n++] = (s->hArCo != NULL) ? 1 : 0;
  v[n++] = (s->hArCo != NULL) ? s->hArCo->m_numberLinesPrev : 0;
  for (int i = 0; i < ARCO_PREV; i++) v[n++] = (s->hArCo != NULL) ? s->hArCo->c_prev[i] : 0;
  /* LPD memory */
  for (int i = 0; i < PIT_MAX_MAX - L_SUBFR; i++) v[n++] = s->old_synth[i];
  for (int i = 0; i < SYN_SFD; i++) v[n++] = s->old_T_pf[i];
  for (int i = 0; i < SYN_SFD; i++) v[n++] = s->old_gain_pf[i];
  for (int i = 0; i < L_FILT + L_SUBFR; i++) v[n++] = s->mem_bpf[i];
  v[n++] = s->old_bpf_control_info;
  v[n++] = s->last_core_mode;
  v[n++] = s->last_lpd_mode;
  v[n++] = s->last_last_lpd_mode;
  v[n++] = s->last_lpc_lost;
  for (int i = 0; i < M_LP_FILTER_ORDER; i++) v[n++] = s->lpc4_lsf[i];
  for (int i = 0; i < M_LP_FILTER_ORDER; i++) v[n++] = s->lsf_adaptive_mean[i];
  for (int k = 0; k < 2; k++)
    for (int i = 0; i < M_LP_FILTER_ORDER; i++) v[n++] = s->lp_coeff_old[k][i];
  v[n++] = s->lp_coeff_old_exp[0];
  v[n++] = s->lp_coeff_old_exp[1];
  v[n++] = s->oldStability;
  v[n++] = (INT)s->numLostLpdFrames;
  v[n++] = s->last_tcx_gain;
  v[n++] = s->last_tcx_gain_e;
  for (int i = 0; i < 32; i++) v[n++] = s->last_alfd_gains[i];
  v[n++] = s->last_tcx_pitch;
  v[n++] = s->last_tcx_noise_factor;
  acelp_mem_store(&s->acelp, v, n);
  v[n++] = (INT)s->nfRandomSeed;
  v[n++] = s->concealmentInfo.prevFrameOk[0];
  v[n++] = s->concealmentInfo.prevFrameOk[1];
  return n;
}

static int sci_load(CAacDecoderStaticChannelInfo *s, const INT *v, LpdMem *mem) {
  int n = 0;
  FDKmemclear(s, sizeof(*s));
  mdct_t *m = &s->IMdct;
  m->prev_wrs = win_ptr(v[n++]);
  m->prev_tl = v[n++];
  m->prev_nr = v[n++];
  m->prev_fr = v[n++];
  m->ov_offset = v[n++];
  m->ov_size = v[n++];
  m->prevAliasSymmetry = v[n++];
  m->prevPrevAliasSymmetry = v[n++];
  INT zir_flag = v[n++];
  INT zir_valid = v[n++];
  int zir_pos = n;
  n += ZIR_MAX;
  m->pAsymOvlp = NULL;
  n++;
  for (int i = 0; i < OVL_MAX; i++) mem->ovl[i] = v[n++];
  m->overlap.freq = mem->ovl;
  s->pOverlapBuffer = mem->ovl;
  if (v[n++]) {
    s->hArCo = &mem->arco;
    mem->arco.m_numberLinesPrev = (SHORT)v[n++];
    for (int i = 0; i < ARCO_PREV; i++) mem->arco.c_prev[i] = (UCHAR)v[n++];
  } else {
    s->hArCo = NULL;
    n += 1 + ARCO_PREV;
  }
  for (int i = 0; i < PIT_MAX_MAX - L_SUBFR; i++) s->old_synth[i] = v[n++];
  for (int i = 0; i < SYN_SFD; i++) s->old_T_pf[i] = v[n++];
  for (int i = 0; i < SYN_SFD; i++) s->old_gain_pf[i] = v[n++];
  for (int i = 0; i < L_FILT + L_SUBFR; i++) s->mem_bpf[i] = v[n++];
  s->old_bpf_control_info = (UCHAR)v[n++];
  s->last_core_mode = (USAC_COREMODE)v[n++];
  s->last_lpd_mode = (UCHAR)v[n++];
  s->last_last_lpd_mode = (UCHAR)v[n++];
  s->last_lpc_lost = (UCHAR)v[n++];
  for (int i = 0; i < M_LP_FILTER_ORDER; i++) s->lpc4_lsf[i] = (FIXP_LPC)v[n++];
  for (int i = 0; i < M_LP_FILTER_ORDER; i++) s->lsf_adaptive_mean[i] = (FIXP_LPC)v[n++];
  for (int k = 0; k < 2; k++)
    for (int i = 0; i < M_LP_FILTER_ORDER; i++) s->lp_coeff_old[k][i] = (FIXP_LPC)v[n++];
  s->lp_coeff_old_exp[0] = v[n++];
  s->lp_coeff_old_exp[1] = v[n++];
  s->oldStability = (FIXP_SGL)v[n++];
  s->numLostLpdFrames = (UINT)v[n++];
  s->last_tcx_gain = v[n++];
  s->last_tcx_gain_e = v[n++];
  for (int i = 0; i < 32; i++) s->last_alfd_gains[i] = v[n++];
  s->last_tcx_pitch = (SHORT)v[n++];
  s->last_tcx_noise_factor = (UCHAR)v[n++];
  acelp_mem_load(&s->acelp, v, n);
  s->nfRandomSeed = (ULONG)v[n++];
  s->concealmentInfo.prevFrameOk[0] = v[n++];
  s->concealmentInfo.prevFrameOk[1] = v[n++];
  /* pFacZir points into the ACELP memory (zir_valid values from its end) */
  if (zir_flag) {
    if (zir_valid >= 0 && zir_valid <= ZIR_MAX) {
      m->pFacZir = s->acelp.old_exc_mem + ZIR_MAX - zir_valid;
      for (int i = 0; i < zir_valid; i++) m->pFacZir[i] = v[zir_pos + i];
    } else {
      m->pFacZir = mem->scratch;
      for (int i = 0; i < LFAC; i++) mem->scratch[i] = v[zir_pos + i];
    }
  } else {
    m->pFacZir = NULL;
  }
  return n;
}

static INT fac_ptr_code(const CAacDecoderChannelInfo *c, const FIXP_DBL *p, const FIXP_DBL *spec) {
  if (p == NULL) return -1;
  if (p == c->data.usac.fac_data0) return -2;
  if (spec != NULL && p >= spec && p < spec + 1024) return (INT)(p - spec);
  return -3;
}

static int ci_store(const CAacDecoderChannelInfo *c, INT *v, const FIXP_DBL *spec) {
  int n = 0;
  const CAacDecoderChannelInfo *ci = c;
  for (int i = 0; i < LFAC; i++) v[n++] = ci->data.usac.fac_data0[i];
  for (int i = 0; i < 4; i++) v[n++] = ci->data.usac.fac_data_e[i];
  for (int i = 0; i < 4; i++) v[n++] = fac_ptr_code(c, ci->data.usac.fac_data[i], spec);
  v[n++] = ci->data.usac.core_mode;
  v[n++] = ci->data.usac.core_mode_last;
  v[n++] = ci->data.usac.lpd_mode_last;
  for (int i = 0; i < 4; i++) v[n++] = ci->data.usac.mod[i];
  v[n++] = ci->data.usac.bpf_control_info;
  for (int k = 0; k < 5; k++)
    for (int i = 0; i < M_LP_FILTER_ORDER; i++) v[n++] = ci->data.usac.lsp_coeff[k][i];
  for (int k = 0; k < 5; k++)
    for (int i = 0; i < M_LP_FILTER_ORDER; i++) v[n++] = ci->data.usac.lp_coeff[k][i];
  for (int k = 0; k < 5; k++) v[n++] = ci->data.usac.lp_coeff_exp[k];
  for (int i = 0; i < M_LP_FILTER_ORDER; i++) v[n++] = ci->data.usac.lsf_adaptive_mean_cand[i];
  for (int i = 0; i < 4; i++) v[n++] = ci->data.usac.aStability[i];
  for (int k = 0; k < 4; k++) {
    const CAcelpChannelData *d = &ci->data.usac.acelp[k];
    v[n++] = d->acelp_core_mode;
    v[n++] = d->mean_energy;
    for (int i = 0; i < NB_SUBFR; i++) v[n++] = d->T0[i];
    for (int i = 0; i < NB_SUBFR; i++) v[n++] = d->T0_frac[i];
    for (int i = 0; i < NB_SUBFR; i++) v[n++] = d->ltp_filtering_flag[i];
    for (int i = 0; i < NB_SUBFR; i++)
      for (int j = 0; j < 8; j++) v[n++] = d->icb_index[i][j];
    for (int i = 0; i < NB_SUBFR; i++) v[n++] = d->gains[i];
  }
  for (int i = 0; i < 4; i++) v[n++] = ci->data.usac.tcx_gain[i];
  for (int i = 0; i < 4; i++) v[n++] = ci->data.usac.tcx_gain_e[i];
  for (int i = 0; i < 8; i++) v[n++] = ci->specScale[i];
  v[n++] = ci->icsInfo.WindowShape;
  v[n++] = (INT)ci->icsInfo.WindowSequence;
  v[n++] = ci->granuleLength;
  v[n++] = ci->currAliasingSymmetry;
  v[n++] = (INT)ci->renderMode;
  for (int i = 0; i < 4; i++) v[n++] = ci->pDynData->specificTo.usac.tcx_noise_factor[i];
  for (int i = 0; i < 4; i++) v[n++] = ci->pDynData->specificTo.usac.tcx_global_gain[i];
  return n;
}

/* A loaded CAacDecoderChannelInfo with its dynamic data. */
struct CiMem {
  CAacDecoderChannelInfo ci;
  CAacDecoderDynamicData dyn;
};

static int ci_load(CiMem *cm, const INT *v, FIXP_DBL *spec, FIXP_DBL *scratch) {
  int n = 0;
  CAacDecoderChannelInfo *ci = &cm->ci;
  FDKmemclear(cm, sizeof(*cm));
  ci->pDynData = &cm->dyn;
  ci->pSpectralCoefficient = spec;
  for (int i = 0; i < LFAC; i++) ci->data.usac.fac_data0[i] = v[n++];
  for (int i = 0; i < 4; i++) ci->data.usac.fac_data_e[i] = (SCHAR)v[n++];
  for (int i = 0; i < 4; i++) {
    INT code = v[n++];
    FIXP_DBL *p = NULL;
    if (code == -2) p = ci->data.usac.fac_data0;
    else if (code == -3) p = scratch;
    else if (code >= 0) p = spec + code;
    ci->data.usac.fac_data[i] = p;
  }
  ci->data.usac.core_mode = (UCHAR)v[n++];
  ci->data.usac.core_mode_last = (USAC_COREMODE)v[n++];
  ci->data.usac.lpd_mode_last = (UCHAR)v[n++];
  for (int i = 0; i < 4; i++) ci->data.usac.mod[i] = (UCHAR)v[n++];
  ci->data.usac.bpf_control_info = (UCHAR)v[n++];
  for (int k = 0; k < 5; k++)
    for (int i = 0; i < M_LP_FILTER_ORDER; i++) ci->data.usac.lsp_coeff[k][i] = (FIXP_LPC)v[n++];
  for (int k = 0; k < 5; k++)
    for (int i = 0; i < M_LP_FILTER_ORDER; i++) ci->data.usac.lp_coeff[k][i] = (FIXP_LPC)v[n++];
  for (int k = 0; k < 5; k++) ci->data.usac.lp_coeff_exp[k] = v[n++];
  for (int i = 0; i < M_LP_FILTER_ORDER; i++) ci->data.usac.lsf_adaptive_mean_cand[i] = (FIXP_LPC)v[n++];
  for (int i = 0; i < 4; i++) ci->data.usac.aStability[i] = (FIXP_SGL)v[n++];
  for (int k = 0; k < 4; k++) {
    CAcelpChannelData *d = &ci->data.usac.acelp[k];
    d->acelp_core_mode = (UCHAR)v[n++];
    d->mean_energy = (UCHAR)v[n++];
    for (int i = 0; i < NB_SUBFR; i++) d->T0[i] = (USHORT)v[n++];
    for (int i = 0; i < NB_SUBFR; i++) d->T0_frac[i] = (UCHAR)v[n++];
    for (int i = 0; i < NB_SUBFR; i++) d->ltp_filtering_flag[i] = (UCHAR)v[n++];
    for (int i = 0; i < NB_SUBFR; i++)
      for (int j = 0; j < 8; j++) d->icb_index[i][j] = (SHORT)v[n++];
    for (int i = 0; i < NB_SUBFR; i++) d->gains[i] = (UCHAR)v[n++];
  }
  for (int i = 0; i < 4; i++) ci->data.usac.tcx_gain[i] = v[n++];
  for (int i = 0; i < 4; i++) ci->data.usac.tcx_gain_e[i] = (SCHAR)v[n++];
  for (int i = 0; i < 8; i++) ci->specScale[i] = (SHORT)v[n++];
  ci->icsInfo.WindowShape = (UCHAR)v[n++];
  ci->icsInfo.WindowSequence = (BLOCK_TYPE)v[n++];
  ci->granuleLength = v[n++];
  ci->currAliasingSymmetry = v[n++];
  ci->renderMode = (AACDEC_RENDER_MODE)v[n++];
  for (int i = 0; i < 4; i++) ci->pDynData->specificTo.usac.tcx_noise_factor[i] = (UCHAR)v[n++];
  for (int i = 0; i < 4; i++) ci->pDynData->specificTo.usac.tcx_global_gain[i] = (UCHAR)v[n++];
  return n;
}

static void lpd_bs_load(FDK_BITSTREAM *bs, const UINT *st, UCHAR *buf) {
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

static void lpd_bs_store(const FDK_BITSTREAM *bs, UINT *st) {
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

/* The state of one test call: a static channel info, a channel info and their memory. */
struct LpdState {
  CAacDecoderStaticChannelInfo sci;
  LpdMem mem;
  CiMem cm;
};

static LpdState *lpd_state_load(const INT *sci_v, const INT *ci_v, FIXP_DBL *spec) {
  LpdState *s = (LpdState *)FDKcalloc(1, sizeof(LpdState));
  sci_load(&s->sci, sci_v, &s->mem);
  if (ci_v != NULL) ci_load(&s->cm, ci_v, spec, s->mem.scratch);
  return s;
}

static void lpd_state_store(LpdState *s, INT *sci_v, INT *ci_v, const FIXP_DBL *spec) {
  sci_store(&s->sci, sci_v);
  if (ci_v != NULL) ci_store(&s->cm.ci, ci_v, spec);
  FDKfree(s);
}

extern "C" {
INT cref_lpd_sci_ints(void) {
  static INT buf[8192];
  CAacDecoderStaticChannelInfo s;
  FDKmemclear(&s, sizeof(s));
  return sci_store(&s, buf);
}
INT cref_lpd_ci_ints(void) {
  static INT buf[8192];
  CAacDecoderChannelInfo c;
  CAacDecoderDynamicData d;
  FDKmemclear(&c, sizeof(c));
  FDKmemclear(&d, sizeof(d));
  c.pDynData = &d;
  return ci_store(&c, buf, NULL);
}
/* The window slope id of FDKgetWindowSlope(length, shape) (-1 for NULL, -2 for others). */
INT cref_lpd_win_id(INT length, INT shape) { return win_id(FDKgetWindowSlope(length, shape)); }
/* load and store again (checks that the serialization is lossless) */
void cref_lpd_roundtrip(INT *sci_v, INT *ci_v, FIXP_DBL *spec) {
  LpdState *s = lpd_state_load(sci_v, ci_v, spec);
  lpd_state_store(s, sci_v, ci_v, spec);
}

INT cref_lpd_get_gain(const FIXP_DBL *x, const FIXP_DBL *y, INT n) { return get_gain(x, y, n); }
INT cref_lpd_last_frame_ok(INT *sci_v, INT before) {
  LpdState *s = lpd_state_load(sci_v, NULL, NULL);
  INT r = CConcealment_GetLastFrameOk(&s->sci.concealmentInfo, before);
  lpd_state_store(s, sci_v, NULL, NULL);
  return r;
}
void cref_lpd_filt_lp(const FIXP_DBL *syn, FIXP_DBL *syn_out, FIXP_DBL *noise_buf, INT noise_off, INT headroom, INT stop, INT len) {
  filtLP(syn, syn_out, noise_buf + noise_off, fdk_dec_filt_lp, headroom, stop, len);
}
void cref_lpd_bass_pf(FIXP_DBL *syn_buf, INT syn_off, const INT *T_sf, FIXP_DBL *pit_gain, INT frame_length, INT l_frame, INT l_next, FIXP_DBL *synth_out, INT headroom, FIXP_DBL *mem_bpf) {
  bass_pf_1sf_delay(syn_buf + syn_off, T_sf, pit_gain, frame_length, l_frame, l_next, synth_out, headroom, mem_bpf);
}
void cref_lpd_adapt_low_freq_deemph(FIXP_DBL *x, INT lg, FIXP_DBL *alfd_gains, INT s) {
  CLpd_AdaptLowFreqDeemph(x, lg, alfd_gains, s);
}
void cref_lpd_lpc2mdct(FIXP_DBL *r, SHORT *pScale, INT lg, INT fdns_npts, const SHORT *A1, INT A1_exp, const SHORT *A2, INT A2_exp) {
  lpc2mdctAndNoiseShaping(r, pScale, lg, fdns_npts, A1, A1_exp, A2, A2_exp);
}
FIXP_DBL cref_lpd_calc_energy(const FIXP_DBL *r, SHORT rs, INT lg, INT *rms_e) { return ref_lpd::calcEnergy(r, rs, lg, rms_e); }
INT cref_lpd_find_mpitch(FIXP_DBL *xri, INT lg) { return ref_lpd::find_mpitch(xri, lg); }
INT cref_lpd_usac_random_sign(UINT *seed) { return UsacRandomSign((ULONG *)seed); }
INT cref_lpd_read_mod(UINT *st, UCHAR *buf, UCHAR *mod) {
  FDK_BITSTREAM bs;
  lpd_bs_load(&bs, st, buf);
  INT r = (INT)ref_lpd::CLpd_ReadAndMapLpdModeToModArray(mod, &bs, 0);
  lpd_bs_store(&bs, st);
  return r;
}
void cref_lpd_reset(INT *sci_v, INT *ci_v, FIXP_DBL *spec, INT keep) {
  LpdState *s = lpd_state_load(sci_v, ci_v, spec);
  ref_lpd::CLpd_Reset(&s->cm.ci, &s->sci, keep);
  lpd_state_store(s, sci_v, ci_v, spec);
}
void cref_lpd_calc_tcx_gain(INT *ci_v, FIXP_DBL *spec, FIXP_DBL rms_m, INT rms_e, INT frame, INT lg) {
  CiMem *cm = (CiMem *)FDKcalloc(1, sizeof(CiMem));
  FIXP_DBL scratch[LFAC];
  ci_load(cm, ci_v, spec, scratch);
  ref_lpd::calcTCXGain(&cm->ci, spec, rms_m, rms_e, frame, lg);
  ci_store(&cm->ci, ci_v, spec);
  FDKfree(cm);
}
void cref_lpd_fdns_decode(INT *sci_v, INT *ci_v, FIXP_DBL *spec, INT r_off, INT lg, INT frame, const SHORT *A1, INT A1_exp, const SHORT *A2, INT A2_exp, INT fdns_npts) {
  LpdState *s = lpd_state_load(sci_v, ci_v, spec);
  CLpd_FdnsDecode(&s->cm.ci, &s->sci, spec + r_off, lg, frame, s->cm.ci.specScale + frame, A1, A1_exp, A2, A2_exp, s->sci.last_alfd_gains, fdns_npts);
  lpd_state_store(s, sci_v, ci_v, spec);
}
void cref_lpd_tcx_decode(INT *sci_v, INT *ci_v, FIXP_DBL *spec, UINT flags, INT mod, INT last_mod, INT frame, INT frameOk) {
  LpdState *s = lpd_state_load(sci_v, ci_v, spec);
  ref_lpd::CLpd_TcxDecode(&s->cm.ci, &s->sci, flags, mod, last_mod, frame, frameOk);
  lpd_state_store(s, sci_v, ci_v, spec);
}
INT cref_lpd_tcx_read(UINT *st, UCHAR *buf, INT *sci_v, INT *ci_v, FIXP_DBL *spec, INT lg, INT first_tcx_flag, INT frame, UINT flags) {
  FDK_BITSTREAM bs;
  lpd_bs_load(&bs, st, buf);
  LpdState *s = lpd_state_load(sci_v, ci_v, spec);
  INT r = (INT)ref_lpd::CLpd_TCX_Read(&bs, &s->cm.ci, &s->sci, lg, first_tcx_flag, frame, flags);
  lpd_state_store(s, sci_v, ci_v, spec);
  lpd_bs_store(&bs, st);
  return r;
}
INT cref_lpd_read(UINT *st, UCHAR *buf, INT *sci_v, INT *ci_v, FIXP_DBL *spec, UINT samplingRate, UINT flags) {
  FDK_BITSTREAM bs;
  lpd_bs_load(&bs, st, buf);
  LpdState *s = lpd_state_load(sci_v, ci_v, spec);
  SamplingRateInfo sri;
  FDKmemclear(&sri, sizeof(sri));
  sri.samplingRate = samplingRate;
  INT r = (INT)CLpdChannelStream_Read(&bs, &s->cm.ci, &s->sci, &sri, flags);
  lpd_state_store(s, sci_v, ci_v, spec);
  lpd_bs_store(&bs, st);
  return r;
}
void cref_lpd_decode(INT *sci_v, INT *ci_v, FIXP_DBL *spec, UINT flags) {
  LpdState *s = lpd_state_load(sci_v, ci_v, spec);
  CLpdChannelStream_Decode(&s->cm.ci, &s->sci, flags);
  lpd_state_store(s, sci_v, ci_v, spec);
}
INT cref_lpd_render(INT *sci_v, INT *ci_v, FIXP_DBL *spec, FIXP_DBL *wb1, FIXP_DBL *time, INT lFrame, UINT samplingRate, UINT frameOk, INT headroom, UINT flags, UINT strmFlags) {
  LpdState *s = lpd_state_load(sci_v, ci_v, spec);
  CAacDecoderCommonStaticData csd;
  FDKmemclear(&csd, sizeof(csd));
  csd.pWorkBufferCore1 = (CWorkBufferCore1 *)wb1;
  s->cm.ci.pComStaticData = &csd;
  SamplingRateInfo sri;
  FDKmemclear(&sri, sizeof(sri));
  sri.samplingRate = samplingRate;
  INT r = (INT)CLpd_RenderTimeSignal(&s->sci, &s->cm.ci, time, lFrame, &sri, frameOk, headroom, flags, strmFlags);
  lpd_state_store(s, sci_v, ci_v, spec);
  return r;
}

/* usacdec_fac.cpp */
INT cref_fac_get_memory(INT granule_length, const UCHAR *mod, INT *state) {
  CAacDecoderChannelInfo *ci = (CAacDecoderChannelInfo *)FDKcalloc(1, sizeof(CAacDecoderChannelInfo));
  static FIXP_DBL spec[1024 * 2];
  ci->granuleLength = granule_length;
  ci->pSpectralCoefficient = spec;
  UCHAR m[4];
  for (int i = 0; i < 4; i++) m[i] = mod[i];
  FIXP_DBL *p = CLpd_FAC_GetMemory(ci, m, state);
  INT r = (p == ci->data.usac.fac_data0) ? -2 : (INT)(p - spec);
  FDKfree(ci);
  return r;
}
INT cref_fac_read(UINT *st, UCHAR *buf, FIXP_DBL *pFac, SCHAR *pFacScale, INT length, INT use_gain, INT frame) {
  FDK_BITSTREAM bs;
  lpd_bs_load(&bs, st, buf);
  INT r = CLpd_FAC_Read(&bs, pFac, pFacScale, length, use_gain, frame);
  lpd_bs_store(&bs, st);
  return r;
}
void cref_fac_apply_gains(FIXP_DBL *fac_data, INT fac_length, FIXP_DBL tcx_gain, const FIXP_DBL *alfd_gains, INT mod) {
  CFac_ApplyGains(fac_data, fac_length, tcx_gain, alfd_gains, mod);
}
void cref_fac_calc_fac_signal(FIXP_DBL *pOut, FIXP_DBL *pFac, INT fac_scale, INT fac_length, const SHORT *A, INT A_exp, INT fAddZir, INT isFdFac) {
  ref_lpd_fac::CFac_CalcFacSignal(pOut, pFac, fac_scale, fac_length, A, A_exp, fAddZir, isFdFac);
}
void cref_fac_syn_filt_zero(const SHORT *a, INT a_exp, INT length, FIXP_DBL *x) { ref_lpd_fac::Syn_filt_zero(a, a_exp, length, x); }
/* CLpd_FAC_Mdct2Acelp on the IMdct of the static channel info `sci_v` (pFac == NULL if hasFac == 0) */
INT cref_fac_mdct2acelp(INT *sci_v, FIXP_DBL *out_buf, INT out_off, INT hasFac, FIXP_DBL *pFac, INT fac_scale, SHORT *A, INT A_exp, INT nrOutSamples, INT fac_length, INT isFdFac, UCHAR prevWindowShape) {
  LpdState *s = lpd_state_load(sci_v, NULL, NULL);
  INT r = CLpd_FAC_Mdct2Acelp(&s->sci.IMdct, out_buf + out_off, hasFac ? pFac : NULL, fac_scale, A, A_exp, nrOutSamples, fac_length, isFdFac, prevWindowShape);
  lpd_state_store(s, sci_v, NULL, NULL);
  return r;
}
/* CLpd_FAC_Acelp2Mdct on the IMdct and the ACELP memory of `sci_v` */
INT cref_fac_acelp2mdct(INT *sci_v, FIXP_DBL *out_buf, INT out_off, FIXP_DBL *pSpec, const SHORT *spec_scale, INT nSpec, FIXP_DBL *pFac, INT fac_scale, INT fac_length, INT noOutSamples, INT tl, INT wrs_id, INT fr, SHORT *A, INT A_exp, FIXP_DBL gain, INT last_frame_lost, INT isFdFac, UCHAR last_lpd_mode, INT k, INT currAliasingSymmetry) {
  LpdState *s = lpd_state_load(sci_v, NULL, NULL);
  INT r = CLpd_FAC_Acelp2Mdct(&s->sci.IMdct, out_buf + out_off, pSpec, spec_scale, nSpec, pFac, fac_scale, fac_length, noOutSamples, tl, win_ptr(wrs_id), fr, A, A_exp, &s->sci.acelp, gain, last_frame_lost, isFdFac, last_lpd_mode, k, currAliasingSymmetry);
  lpd_state_store(s, sci_v, NULL, NULL);
  return r;
}
}

/* ---------------------------------------------------------------------------------------------
 * Capture decoder
 * ------------------------------------------------------------------------------------------- */

/* Record kinds */
#define CAP_READ 1
#define CAP_DECODE 2
#define CAP_RENDER 3
#define CAP_ACELP2MDCT 4
#define CAP_FAC_READ 5
#define CAP_BASS_PF 6

#define SCI_INTS_MAX 4096
#define CI_INTS_MAX 1024
#define WB1_INTS (WB_SECTION_SIZE)

static std::vector<std::vector<INT> > cap_records;
static int cap_enabled = 0;
/* The work buffer core 1 of the capture decoder (for pointers into it). */
static FIXP_DBL *cap_wb1 = NULL;

static void cap_push(std::vector<INT> &r, const INT *v, int n) { r.insert(r.end(), v, v + n); }
/* ILP32 (x86_32): FIXP_DBL is `long`, a type distinct from INT of the same size. */
#if !defined(__LP64__)
static void cap_push(std::vector<INT> &r, const LONG *v, int n) { cap_push(r, (const INT *)v, n); }
#endif

static void cap_push_sci(std::vector<INT> &r, const CAacDecoderStaticChannelInfo *s) {
  INT tmp[SCI_INTS_MAX];
  int n = sci_store(s, tmp);
  cap_push(r, tmp, n);
}

static void cap_push_ci(std::vector<INT> &r, const CAacDecoderChannelInfo *c) {
  INT tmp[CI_INTS_MAX];
  int n = ci_store(c, tmp, c->pSpectralCoefficient);
  cap_push(r, tmp, n);
}

static void cap_push_bs(std::vector<INT> &r, const FDK_BITSTREAM *bs) {
  UINT st[9];
  lpd_bs_store(bs, st);
  cap_push(r, (const INT *)st, 9);
}

/* The memory of a bit buffer, 4 bytes per INT (little endian). */
static void cap_push_bs_mem(std::vector<INT> &r, const FDK_BITSTREAM *bs) {
  UINT size = bs->hBitBuf.bufSize;
  r.push_back((INT)size);
  for (UINT i = 0; i < size; i += 4) {
    UINT w = 0;
    for (UINT k = 0; k < 4 && i + k < size; k++) w |= (UINT)bs->hBitBuf.Buffer[i + k] << (8 * k);
    r.push_back((INT)w);
  }
}

/* The address of the static channel info (identifies the channel of a record). */
static void cap_push_addr(std::vector<INT> &r, const CAacDecoderStaticChannelInfo *sci) {
  UINT64 a = (UINT64)(size_t)sci;
  r.push_back((INT)(UINT)a);
  r.push_back((INT)(UINT)(a >> 32));
}

static void cap_push_wb1(std::vector<INT> &r) {
  if (cap_wb1 != NULL) {
    cap_push(r, (const INT *)cap_wb1, WB1_INTS);
  } else {
    r.insert(r.end(), WB1_INTS, 0);
  }
}

/* The offset of a pointer into the work buffer core 1 (-1 outside of it). */
static INT cap_wb1_off(const FIXP_DBL *p) {
  if (cap_wb1 != NULL && p >= cap_wb1 && p < cap_wb1 + WB1_INTS) return (INT)(p - cap_wb1);
  return -1;
}

static CAacDecoderStaticChannelInfo *cap_sci_of_mdct(H_MDCT hMdct) {
  return (CAacDecoderStaticChannelInfo *)((char *)hMdct - offsetof(CAacDecoderStaticChannelInfo, IMdct));
}

AAC_DECODER_ERROR cap_hook_read(HANDLE_FDK_BITSTREAM hBs, CAacDecoderChannelInfo *ci, CAacDecoderStaticChannelInfo *sci, const SamplingRateInfo *sri, UINT flags) {
  std::vector<INT> r;
  if (cap_enabled) {
    r.push_back(CAP_READ);
    r.push_back((INT)sri->samplingRate);
    r.push_back((INT)flags);
    cap_push_bs(r, hBs);
    cap_push_bs_mem(r, hBs);
    cap_push_sci(r, sci);
    cap_push_ci(r, ci);
    cap_push(r, ci->pSpectralCoefficient, 1024);
  }
  AAC_DECODER_ERROR err = ::CLpdChannelStream_Read(hBs, ci, sci, sri, flags);
  if (cap_enabled) {
    r.push_back((INT)err);
    cap_push_bs(r, hBs);
    cap_push_sci(r, sci);
    cap_push_ci(r, ci);
    cap_push(r, ci->pSpectralCoefficient, 1024);
    cap_push_addr(r, sci);
    cap_records.push_back(r);
  }
  return err;
}

void cap_hook_decode(CAacDecoderChannelInfo *ci, CAacDecoderStaticChannelInfo *sci, UINT flags) {
  std::vector<INT> r;
  if (cap_enabled) {
    r.push_back(CAP_DECODE);
    r.push_back((INT)flags);
    cap_push_sci(r, sci);
    cap_push_ci(r, ci);
    cap_push(r, ci->pSpectralCoefficient, 1024);
  }
  ::CLpdChannelStream_Decode(ci, sci, flags);
  if (cap_enabled) {
    cap_push_sci(r, sci);
    cap_push_ci(r, ci);
    cap_push(r, ci->pSpectralCoefficient, 1024);
    cap_push_addr(r, sci);
    cap_records.push_back(r);
  }
}

AAC_DECODER_ERROR cap_hook_render(CAacDecoderStaticChannelInfo *sci, CAacDecoderChannelInfo *ci, PCM_DEC *pTimeData, INT lFrame, SamplingRateInfo *sri, UINT frameOk, const INT aacOutDataHeadroom, UINT flags, UINT strmFlags) {
  std::vector<INT> r;
  FIXP_DBL *wb1 = ci->pComStaticData->pWorkBufferCore1->synth_buf;
  if (cap_enabled) {
    cap_wb1 = wb1;
    r.push_back(CAP_RENDER);
    r.push_back(lFrame);
    r.push_back((INT)sri->samplingRate);
    r.push_back((INT)frameOk);
    r.push_back(aacOutDataHeadroom);
    r.push_back((INT)flags);
    r.push_back((INT)strmFlags);
    cap_push_sci(r, sci);
    cap_push_ci(r, ci);
    cap_push(r, ci->pSpectralCoefficient, 1024);
    cap_push_wb1(r);
  }
  AAC_DECODER_ERROR err = ::CLpd_RenderTimeSignal(sci, ci, pTimeData, lFrame, sri, frameOk, aacOutDataHeadroom, flags, strmFlags);
  if (cap_enabled) {
    r.push_back((INT)err);
    cap_push_sci(r, sci);
    cap_push_ci(r, ci);
    cap_push(r, ci->pSpectralCoefficient, 1024);
    cap_push_wb1(r);
    cap_push(r, pTimeData, lFrame);
    cap_push_addr(r, sci);
    cap_records.push_back(r);
  }
  return err;
}

INT cap_hook_acelp2mdct(H_MDCT hMdct, FIXP_DBL *output, FIXP_DBL *pSpec, const SHORT spec_scale[], const int nSpec, FIXP_DBL *pFac_data, const int fac_data_e, const INT fac_length, INT nrSamples, const INT tl, const FIXP_WTP *wrs, const INT fr, FIXP_LPC A[16], INT A_exp, CAcelpStaticMem *acelp_mem, const FIXP_DBL gain, const int last_frame_lost, const int isFdFac, const UCHAR last_lpd, const int k, int currAliasingSymmetry) {
  std::vector<INT> r;
  CAacDecoderStaticChannelInfo *sci = cap_sci_of_mdct(hMdct);
  INT out_off = cap_wb1_off(output);
  if (cap_enabled) {
    r.push_back(CAP_ACELP2MDCT);
    r.push_back(out_off);
    r.push_back(nSpec);
    r.push_back(fac_data_e);
    r.push_back(fac_length);
    r.push_back(nrSamples);
    r.push_back(tl);
    r.push_back(win_id(wrs));
    r.push_back(fr);
    r.push_back(A_exp);
    r.push_back(gain);
    r.push_back(last_frame_lost);
    r.push_back(isFdFac);
    r.push_back(last_lpd);
    r.push_back(k);
    r.push_back(currAliasingSymmetry);
    r.push_back(acelp_mem == &sci->acelp ? 1 : 0);
    for (int i = 0; i < 16; i++) r.push_back(A[i]);
    for (int i = 0; i < 8; i++) r.push_back(i < nSpec ? spec_scale[i] : 0);
    cap_push_sci(r, sci);
    cap_push(r, pSpec, 1024);
    for (int i = 0; i < LFAC; i++) r.push_back(i < fac_length ? pFac_data[i] : 0);
    cap_push_wb1(r);
  }
  INT ret = ::CLpd_FAC_Acelp2Mdct(hMdct, output, pSpec, spec_scale, nSpec, pFac_data, fac_data_e, fac_length, nrSamples, tl, wrs, fr, A, A_exp, acelp_mem, gain, last_frame_lost, isFdFac, last_lpd, k, currAliasingSymmetry);
  if (cap_enabled) {
    r.push_back(ret);
    cap_push_sci(r, sci);
    cap_push(r, pSpec, 1024);
    for (int i = 0; i < LFAC; i++) r.push_back(i < fac_length ? pFac_data[i] : 0);
    cap_push_wb1(r);
    cap_push_addr(r, sci);
    cap_records.push_back(r);
  }
  return ret;
}

int cap_hook_fac_read(HANDLE_FDK_BITSTREAM hBs, FIXP_DBL *pFac, SCHAR *pFacScale, int length, int use_gain, int frame) {
  std::vector<INT> r;
  if (cap_enabled) {
    r.push_back(CAP_FAC_READ);
    r.push_back(length);
    r.push_back(use_gain);
    r.push_back(frame);
    cap_push_bs(r, hBs);
    cap_push_bs_mem(r, hBs);
    for (int i = 0; i < LFAC; i++) r.push_back(i < length ? pFac[i] : 0);
    for (int i = 0; i < 4; i++) r.push_back(pFacScale[i]);
  }
  int err = ::CLpd_FAC_Read(hBs, pFac, pFacScale, length, use_gain, frame);
  if (cap_enabled) {
    r.push_back(err);
    cap_push_bs(r, hBs);
    for (int i = 0; i < LFAC; i++) r.push_back(i < length ? pFac[i] : 0);
    for (int i = 0; i < 4; i++) r.push_back(pFacScale[i]);
    cap_records.push_back(r);
  }
  return err;
}

void cap_hook_bass_pf(FIXP_DBL syn[], const INT T_sf[], FIXP_DBL *pit_gain, const int frame_length, const INT l_frame, const INT l_next, PCM_DEC *synth_out, const INT aacOutDataHeadroom, FIXP_DBL mem_bpf[]) {
  std::vector<INT> r;
  INT syn_off = cap_wb1_off(syn);
  if (cap_enabled) {
    r.push_back(CAP_BASS_PF);
    r.push_back(syn_off);
    r.push_back(frame_length);
    r.push_back(l_frame);
    r.push_back(l_next);
    r.push_back(aacOutDataHeadroom);
    for (int i = 0; i < NB_SUBFR_SUPERFR + SYN_SFD; i++) r.push_back(T_sf[i]);
    for (int i = 0; i < NB_SUBFR_SUPERFR + SYN_SFD; i++) r.push_back(pit_gain[i]);
    cap_push(r, mem_bpf, L_FILT + L_SUBFR);
    cap_push_wb1(r);
  }
  ::bass_pf_1sf_delay(syn, T_sf, pit_gain, frame_length, l_frame, l_next, synth_out, aacOutDataHeadroom, mem_bpf);
  if (cap_enabled) {
    cap_push(r, mem_bpf, L_FILT + L_SUBFR);
    cap_push(r, synth_out, frame_length);
    cap_records.push_back(r);
  }
}

extern "C" {
/* Opens the capture decoder for raw access units with the AudioSpecificConfig `asc`. */
void *cref_cap_open(UCHAR *asc, UINT asc_len) {
  HANDLE_AACDECODER h = capdec_Open(TT_MP4_RAW, 1);
  if (h == NULL) return NULL;
  UCHAR *conf[1] = {asc};
  UINT len[1] = {asc_len};
  if (capdec_ConfigRaw(h, conf, len) != AAC_DEC_OK) {
    capdec_Close(h);
    return NULL;
  }
  return h;
}
void cref_cap_close(void *h) { capdec_Close((HANDLE_AACDECODER)h); }
/* Decodes one access unit (len 0: none) with the decoder flags; returns the decoder error. */
INT cref_cap_decode(void *h, UCHAR *au, UINT len, INT flags, INT *pcm, INT pcm_size) {
  HANDLE_AACDECODER self = (HANDLE_AACDECODER)h;
  if (len > 0) {
    UCHAR *buf[1] = {au};
    UINT size[1] = {len};
    UINT valid = len;
    INT e = capdec_Fill(self, buf, size, &valid);
    if (e != AAC_DEC_OK) return e;
  }
  cap_enabled = 1;
  INT err = capdec_DecodeFrame(self, (INT_PCM *)pcm, pcm_size, flags);
  cap_enabled = 0;
  return err;
}
INT cref_cap_count(void) { return (INT)cap_records.size(); }
/* Copies record i to out (at most max values); returns its length. */
INT cref_cap_get(INT i, INT *out, INT max) {
  const std::vector<INT> &r = cap_records[i];
  INT n = (INT)r.size();
  for (INT k = 0; k < n && k < max; k++) out[k] = r[k];
  return n;
}
void cref_cap_clear(void) { cap_records.clear(); }
}
