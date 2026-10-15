// Shims for the differential tests of the functions of fdk_aac_rs_aacdec::usacdec_acelp.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_usacdec_acelp (see build.rs).
//
// The file-static functions are reached through a copy of usacdec_acelp.cpp in a namespace;
// the other functions are the library's own (declared here where no header does).
//
// CAcelpStaticMem is passed as INT[ACELP_MEM_INTS]: old_exc_mem[428], old_syn_mem[16], A[16],
// A_exp, gc_threshold, de_emph_mem, past_gpit, past_gcode, old_T0, old_T0_frac,
// deemph_mem_wsyn, wsyn_rms, seed_ace.
// CAcelpChannelData is passed as INT[ACELP_DATA_INTS]: acelp_core_mode, mean_energy, T0[4],
// T0_frac[4], ltp_filtering_flag[4], icb_index[4][8], gains[4].
// A bitstream is passed as in aacdec_usacdec_lpc.cpp (st[9] + buf).

#include "cref_common.h"
#include "libAACdec/src/usacdec_acelp.h"
#include "libAACdec/src/usacdec_ace_d4t64.h"
#include "libAACdec/src/usacdec_ace_ltp.h"
#include "libAACdec/src/usacdec_rom.h"
#include "libAACdec/src/usacdec_lpc.h"

namespace ref_acelp {
#include "libAACdec/src/usacdec_acelp.cpp"
}

/* Non-static functions of usacdec_acelp.cpp that no header declares: the library's own code. */
void E_UTIL_preemph(const FIXP_DBL *in, FIXP_DBL *out, INT L);
void BuildAdaptiveExcitation(FIXP_COD code[], FIXP_DBL exc[], FIXP_SGL gain_pit, FIXP_DBL gain_code, FIXP_DBL gain_code_smoothed, FIXP_DBL period_fac, FIXP_DBL exc2[]);
void int_lpc_acelp(const FIXP_LPC lsp_old[], const FIXP_LPC lsp_new[], int subfr_nr, int nb_subfr, FIXP_LPC A[], INT *A_exp);
void Syn_filt(const FIXP_LPC a[], const INT a_exp, INT length, FIXP_DBL x[], FIXP_DBL y[]);
void Deemph(FIXP_DBL *x, FIXP_DBL *y, int L, FIXP_DBL *mem);
void E_UTIL_residu(const FIXP_LPC *a, const INT a_exp, FIXP_DBL *x, FIXP_DBL *y, INT l);

#define ACELP_MEM_INTS (PIT_MAX_MAX + L_INTERPOL + 2 * M_LP_FILTER_ORDER + 10)
#define ACELP_DATA_INTS (2 + 3 * NB_SUBFR + 8 * NB_SUBFR + NB_SUBFR)

static void acelp_mem_load(CAcelpStaticMem *m, const INT *v) {
  int n = 0;
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

static void acelp_mem_store(const CAcelpStaticMem *m, INT *v) {
  int n = 0;
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

static void acelp_data_load(CAcelpChannelData *d, const INT *v) {
  int n = 0;
  d->acelp_core_mode = (UCHAR)v[n++];
  d->mean_energy = (UCHAR)v[n++];
  for (int i = 0; i < NB_SUBFR; i++) d->T0[i] = (USHORT)v[n++];
  for (int i = 0; i < NB_SUBFR; i++) d->T0_frac[i] = (UCHAR)v[n++];
  for (int i = 0; i < NB_SUBFR; i++) d->ltp_filtering_flag[i] = (UCHAR)v[n++];
  for (int i = 0; i < NB_SUBFR; i++)
    for (int k = 0; k < 8; k++) d->icb_index[i][k] = (SHORT)v[n++];
  for (int i = 0; i < NB_SUBFR; i++) d->gains[i] = (UCHAR)v[n++];
}

static void acelp_data_store(const CAcelpChannelData *d, INT *v) {
  int n = 0;
  v[n++] = d->acelp_core_mode;
  v[n++] = d->mean_energy;
  for (int i = 0; i < NB_SUBFR; i++) v[n++] = d->T0[i];
  for (int i = 0; i < NB_SUBFR; i++) v[n++] = d->T0_frac[i];
  for (int i = 0; i < NB_SUBFR; i++) v[n++] = d->ltp_filtering_flag[i];
  for (int i = 0; i < NB_SUBFR; i++)
    for (int k = 0; k < 8; k++) v[n++] = d->icb_index[i][k];
  for (int i = 0; i < NB_SUBFR; i++) v[n++] = d->gains[i];
}

static void acelp_bs_load(FDK_BITSTREAM *bs, const UINT *st, UCHAR *buf) {
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

static void acelp_bs_store(const FDK_BITSTREAM *bs, UINT *st) {
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

extern "C" {
INT cref_acelp_mem_ints(void) { return ACELP_MEM_INTS; }
INT cref_acelp_data_ints(void) { return ACELP_DATA_INTS; }
INT cref_acelp_random(SHORT seed) {
  SHORT s = seed;
  E_UTIL_random(&s);
  return ((INT)s << 16) | (USHORT)E_UTIL_random(&s);
}
void cref_acelp_preemph(const FIXP_DBL *buf, INT off, FIXP_DBL *out, INT L) { E_UTIL_preemph(buf + off, out, L); }
void cref_acelp_preemph_code(SHORT *x) { ref_acelp::Preemph_code(x); }
void cref_acelp_pit_shrp(SHORT *x, INT pit_lag) { ref_acelp::Pit_shrp(x, pit_lag); }
/* D_gain2_plus; io = {gain_pit, gain_code, past_gpit, past_gcode, ener_code, ener_code_e} */
void cref_acelp_d_gain2_plus(INT index, SHORT *code, INT mean_ener_bits, INT bfi, INT *io) {
  FIXP_SGL gain_pit = (FIXP_SGL)io[0], past_gpit = (FIXP_SGL)io[2];
  FIXP_DBL gain_code = io[1], past_gcode = io[3], ener = io[4];
  INT ener_e = io[5];
  ref_acelp::D_gain2_plus(index, code, &gain_pit, &gain_code, mean_ener_bits, bfi, &past_gpit, &past_gcode, &ener, &ener_e);
  io[0] = gain_pit;
  io[1] = gain_code;
  io[2] = past_gpit;
  io[3] = past_gcode;
  io[4] = ener;
  io[5] = ener_e;
}
FIXP_DBL cref_acelp_calc_period_factor(FIXP_DBL *exc, SHORT gain_pit, FIXP_DBL gain_code, FIXP_DBL ener_code, INT ener_code_e) {
  return ref_acelp::calc_period_factor(exc, gain_pit, gain_code, ener_code, ener_code_e);
}
FIXP_DBL cref_acelp_noise_enhancer(FIXP_DBL gain_code, FIXP_DBL period_fac, SHORT stab_fac, FIXP_DBL *gc_threshold) {
  return ref_acelp::noise_enhancer(gain_code, period_fac, stab_fac, gc_threshold);
}
void cref_acelp_build_adaptive_excitation(SHORT *code, FIXP_DBL *exc_buf, INT off, SHORT gain_pit, FIXP_DBL gain_code, FIXP_DBL gain_code_smoothed, FIXP_DBL period_fac, FIXP_DBL *exc2) {
  BuildAdaptiveExcitation(code, exc_buf + off, gain_pit, gain_code, gain_code_smoothed, period_fac, exc2);
}
void cref_acelp_int_lpc_acelp(const SHORT *lsp_old, const SHORT *lsp_new, INT subfr_nr, INT nb_subfr, SHORT *A, INT *A_exp) {
  int_lpc_acelp(lsp_old, lsp_new, subfr_nr, nb_subfr, A, A_exp);
}
void cref_acelp_syn_filt(const SHORT *a, INT a_exp, INT length, FIXP_DBL *x, FIXP_DBL *y_buf, INT y_off) {
  Syn_filt(a, a_exp, length, x, y_buf + y_off);
}
void cref_acelp_deemph(FIXP_DBL *x, FIXP_DBL *y, INT L, FIXP_DBL *mem) { Deemph(x, y, L, mem); }
void cref_acelp_residu(const SHORT *a, INT a_exp, FIXP_DBL *x_buf, INT x_off, FIXP_DBL *y, INT l) {
  E_UTIL_residu(a, a_exp, x_buf + x_off, y, l);
}
/* DecodePitchLag; io = {T0, T0_frac, T0_min, T0_max} */
INT cref_acelp_decode_pitch_lag(UINT *st, UCHAR *buf, INT bits, INT pit_min, INT pit_fr2, INT pit_fr1, INT pit_max, INT *io) {
  FDK_BITSTREAM bs;
  acelp_bs_load(&bs, st, buf);
  INT r = ref_acelp::DecodePitchLag(&bs, (UCHAR)bits, pit_min, pit_fr2, pit_fr1, pit_max, &io[0], &io[1], &io[2], &io[3]);
  acelp_bs_store(&bs, st);
  return r;
}
void cref_acelp_conceal_pitch_lag(INT *mem, INT pit_max, INT *t0, INT *t0_frac) {
  CAcelpStaticMem m;
  acelp_mem_load(&m, mem);
  ref_acelp::ConcealPitchLag(&m, pit_max, t0, t0_frac);
  acelp_mem_store(&m, mem);
}
void cref_acelp_decode(INT *mem, INT i_offset, const SHORT *lsp_old, const SHORT *lsp_new, SHORT stab_fac, INT *data, INT numLostSubframes, INT lastLpcLost, INT frameCnt, FIXP_DBL *synth, INT *pT, FIXP_DBL *pit_gain, INT ccfl) {
  CAcelpStaticMem m;
  CAcelpChannelData d;
  acelp_mem_load(&m, mem);
  acelp_data_load(&d, data);
  CLpd_AcelpDecode(&m, i_offset, lsp_old, lsp_new, stab_fac, &d, numLostSubframes, lastLpcLost, frameCnt, synth, pT, pit_gain, ccfl);
  acelp_mem_store(&m, mem);
  acelp_data_store(&d, data);
}
void cref_acelp_reset(INT *mem) {
  CAcelpStaticMem m;
  acelp_mem_load(&m, mem);
  CLpd_AcelpReset(&m);
  acelp_mem_store(&m, mem);
}
void cref_acelp_tcx_td_conceal(INT *mem, SHORT pitch, const SHORT *lsp_old, const SHORT *lsp_new, SHORT stab_fac, INT nLostSf, FIXP_DBL *synth, INT ccfl, UCHAR nf) {
  CAcelpStaticMem m;
  acelp_mem_load(&m, mem);
  SHORT p = pitch;
  CLpd_TcxTDConceal(&m, &p, lsp_old, lsp_new, stab_fac, nLostSf, synth, ccfl, nf);
  acelp_mem_store(&m, mem);
}
void cref_acelp_pre_processing(FIXP_DBL *synth_buf, FIXP_DBL *old_synth, INT *pitch, INT *old_T_pf, FIXP_DBL *pit_gain, FIXP_DBL *old_gain_pf, INT samplingRate, INT *i_offset, INT ccfl, INT synSfd, INT nbSubfrSuperfr) {
  Acelp_PreProcessing(synth_buf, old_synth, pitch, old_T_pf, pit_gain, old_gain_pf, samplingRate, i_offset, ccfl, synSfd, nbSubfrSuperfr);
}
void cref_acelp_post_processing(FIXP_DBL *synth_buf, FIXP_DBL *old_synth, INT *pitch, INT *old_T_pf, INT ccfl, INT synSfd, INT nbSubfrSuperfr) {
  Acelp_PostProcessing(synth_buf, old_synth, pitch, old_T_pf, ccfl, synSfd, nbSubfrSuperfr);
}
void cref_acelp_zir(const SHORT *A, INT A_exp, INT *mem, INT length, FIXP_DBL *zir, INT doDeemph) {
  CAcelpStaticMem m;
  acelp_mem_load(&m, mem);
  CLpd_Acelp_Zir(A, A_exp, &m, length, zir, doDeemph);
  acelp_mem_store(&m, mem);
}
void cref_acelp_prepare_internal_mem(const FIXP_DBL *buf, INT off, UCHAR last_lpd_mode, UCHAR last_last_lpd_mode, const SHORT *A_new, INT A_new_exp, const SHORT *A_old, INT A_old_exp, INT *mem, INT ccfl, INT clearOldExc, UCHAR lpd_mode) {
  CAcelpStaticMem m;
  acelp_mem_load(&m, mem);
  CLpd_AcelpPrepareInternalMem(buf + off, last_lpd_mode, last_last_lpd_mode, A_new, A_new_exp, A_old, A_old_exp, &m, ccfl, clearOldExc, lpd_mode);
  acelp_mem_store(&m, mem);
}
INT cref_acelp_read(UINT *st, UCHAR *buf, INT *data, INT acelp_core_mode, INT ccfl, INT i_offset) {
  FDK_BITSTREAM bs;
  CAcelpChannelData d;
  acelp_bs_load(&bs, st, buf);
  acelp_data_load(&d, data);
  /* The definition takes (coreCoderFrameLength, i_offset); the header declares the opposite
     order, the caller passes the values in the order of the definition. */
  INT r = CLpd_AcelpRead(&bs, &d, acelp_core_mode, ccfl, i_offset);
  acelp_bs_store(&bs, st);
  acelp_data_store(&d, data);
  return r;
}
}

extern "C" {
/* The file-static table `name` of the copy (ref_acelp) as memory: pow_10_mean_energy,
   tab_coremode2nbits. Returns NULL for another name. */
const void *cref_acelp_table(const char *name, INT *bytes) {
  if (strcmp(name, "pow_10_mean_energy") == 0) {
    *bytes = (INT)sizeof(ref_acelp::pow_10_mean_energy);
    return ref_acelp::pow_10_mean_energy;
  }
  if (strcmp(name, "tab_coremode2nbits") == 0) {
    *bytes = (INT)sizeof(ref_acelp::tab_coremode2nbits);
    return ref_acelp::tab_coremode2nbits;
  }
  *bytes = 0;
  return NULL;
}
}
