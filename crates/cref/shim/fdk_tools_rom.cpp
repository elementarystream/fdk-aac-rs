// Shims for the differential tests of fdk_aac_rs_fdk::tools_rom.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_tools_rom (see build.rs).

#include "cref_common.h"
#include "FDK_tools_rom.h"
#include "fixpoint_math.h"

/* A copy of FDK_tools_rom.cpp, for the tables that are file-local in the library (they are
   defined without a declaration in a header, so `const` gives them internal linkage) and for
   the size of the tables that the header declares without a size. */
namespace ref_tr {
#include "libFDK/src/FDK_tools_rom.cpp"
}

struct TableEntry {
  const char *name;
  const void *ptr;
  INT bytes;
  INT elem_bytes;
};

/* The library's table, whose header declaration has a size. */
#define TG(x) {#x, (const void *)::x, (INT)sizeof(::x), (INT)sizeof(::x[0])}
/* The library's table, whose header declaration has no size (the size of the definition). */
#define TGN(x) {#x, (const void *)::x, (INT)sizeof(ref_tr::x), (INT)sizeof(ref_tr::x[0])}
/* A table that is file-local in the library (the copy). */
#define TN(x) {#x, (const void *)ref_tr::x, (INT)sizeof(ref_tr::x), (INT)sizeof(ref_tr::x[0])}

static const TableEntry tables[] = {
    TGN(SineTable80), TGN(SineTable384), TGN(SineTable480), TGN(SineTable512),
    TGN(SineTable1024),
    TG(RotVectorReal6), TG(RotVectorImag6), TG(RotVectorReal12), TG(RotVectorImag12),
    TG(RotVectorReal24), TG(RotVectorImag24), TG(RotVectorReal48), TG(RotVectorImag48),
    TG(RotVectorReal80), TG(RotVectorImag80), TG(RotVectorReal96), TG(RotVectorImag96),
    TG(RotVectorReal384), TG(RotVectorImag384), TG(RotVectorReal60), TG(RotVectorImag60),
    TG(RotVectorReal120), TG(RotVectorImag120), TG(RotVectorReal192), TG(RotVectorImag192),
    TG(RotVectorReal240), TG(RotVectorImag240), TG(RotVectorReal480), TG(RotVectorImag480),
    TG(RotVectorReal20), TG(RotVectorImag20),
    TGN(SineWindow8), TN(SineWindow12), TGN(SineWindow16), TN(SineWindow20),
    TGN(SineWindow24), TGN(SineWindow32), TN(SineWindow40), TGN(SineWindow48),
    TGN(SineWindow64), TGN(SineWindow96), TGN(SineWindow120), TGN(SineWindow128),
    TN(SineWindow160), TGN(SineWindow192), TN(SineWindow240), TG(SineWindow256),
    TN(SineWindow384), TGN(SineWindow480), TGN(SineWindow512), TGN(SineWindow768),
    TGN(SineWindow960), TGN(SineWindow1024),
    TGN(KBDWindow96), TGN(KBDWindow120), TGN(KBDWindow128), TN(KBDWindow256),
    TGN(KBDWindow768), TGN(KBDWindow960), TGN(KBDWindow1024),
    TGN(qmf_pfilt120), TGN(qmf_pfilt200), TG(qmf_phaseshift_cos40), TG(qmf_phaseshift_sin40),
    TGN(qmf_pfilt400), TG(qmf_phaseshift_cos16), TG(qmf_phaseshift_sin16), TGN(qmf_pfilt240),
    TG(qmf_phaseshift_cos24), TG(qmf_phaseshift_sin24), TG(qmf_phaseshift_cos32),
    TG(qmf_phaseshift_sin32), TG(qmf_phaseshift_cos_downsamp32),
    TG(qmf_phaseshift_sin_downsamp32), TG(qmf_pfilt640), TG(qmf_pfilt640_vector),
    TG(qmf_phaseshift_cos64), TG(qmf_phaseshift_sin64),
    TG(qmf_phaseshift_cos32_cldfb_ana), TG(qmf_phaseshift_cos32_cldfb_syn),
    TG(qmf_phaseshift_sin32_cldfb), TG(qmf_phaseshift_cos16_cldfb_ana),
    TG(qmf_phaseshift_cos16_cldfb_syn), TG(qmf_phaseshift_sin16_cldfb),
    TG(qmf_phaseshift_cos8_cldfb_ana), TG(qmf_phaseshift_cos8_cldfb_syn),
    TG(qmf_phaseshift_sin8_cldfb), TG(qmf_phaseshift_cos64_cldfb),
    TG(qmf_phaseshift_sin64_cldfb),
    TG(qmf_cldfb_640), TG(qmf_cldfb_320), TG(qmf_cldfb_160), TG(qmf_cldfb_80),
    TG(qmf_mpsldfb_320), TG(qmf_mpsldfb_640),
    TGN(sin_twiddle_L64), TG(sqrt_tab), TG(invCount), TG(invSqrtTab), TG(format_nchan),
};

/* The window slope tables of the copy, to identify the entries of ref_tr::windowSlopes. */
#define W(x) {#x, (const void *)ref_tr::x, (INT)sizeof(ref_tr::x), (INT)sizeof(ref_tr::x[0])}
static const TableEntry windows[] = {
    W(SineWindow8),    W(SineWindow12),   W(SineWindow16),  W(SineWindow20),
    W(SineWindow24),   W(SineWindow32),   W(SineWindow40),  W(SineWindow48),
    W(SineWindow64),   W(SineWindow96),   W(SineWindow120), W(SineWindow128),
    W(SineWindow160),  W(SineWindow192),  W(SineWindow240), W(SineWindow256),
    W(SineWindow384),  W(SineWindow480),  W(SineWindow512), W(SineWindow768),
    W(SineWindow960),  W(SineWindow1024), W(KBDWindow96),   W(KBDWindow120),
    W(KBDWindow128),   W(KBDWindow256),   W(KBDWindow768),  W(KBDWindow960),
    W(KBDWindow1024),
};

static INT window_index(const FIXP_WTP *w) {
  if (w == NULL) return -1;
  for (INT i = 0; i < (INT)(sizeof(windows) / sizeof(windows[0])); i++) {
    if (windows[i].ptr == (const void *)w) return i;
  }
  return -2;
}

struct RbdEntry {
  const char *name;
  INT value;
};
#define R(x) {#x, (INT)x}
static const RbdEntry rbd_ids[] = {
    R(element_instance_tag), R(common_window), R(global_gain), R(ics_info), R(max_sfb), R(ms),
    R(ltp_data_present), R(ltp_data), R(section_data), R(scale_factor_data), R(pulse),
    R(tns_data_present), R(tns_data), R(gain_control_data_present), R(gain_control_data),
    R(esc1_hcr), R(esc2_rvlc), R(spectral_data), R(scale_factor_data_usac), R(core_mode),
    R(common_tw), R(lpd_channel_stream), R(tw_data), R(noise), R(ac_spectral_data), R(fac_data),
    R(tns_active), R(tns_data_present_usac), R(common_max_sfb), R(coupled_elements),
    R(gain_element_lists), R(adtscrc_start_reg1), R(adtscrc_start_reg2), R(adtscrc_end_reg1),
    R(adtscrc_end_reg2), R(drmcrc_start_reg), R(drmcrc_end_reg), R(next_channel),
    R(next_channel_loop), R(link_sequence), R(end_of_sequence),
};

extern "C" {
INT cref_tr_num_tables(void) { return (INT)(sizeof(tables) / sizeof(tables[0])); }
const void *cref_tr_table(INT i, INT *bytes, INT *elem_bytes) {
  *bytes = tables[i].bytes;
  *elem_bytes = tables[i].elem_bytes;
  return tables[i].ptr;
}
const char *cref_tr_table_name(INT i) { return tables[i].name; }

INT cref_tr_num_windows(void) { return (INT)(sizeof(windows) / sizeof(windows[0])); }
const char *cref_tr_window_name(INT i) { return windows[i].name; }
/* The entry windowSlopes[s][r][i]: an index of windows[], or -1 for NULL. */
INT cref_tr_window_slopes(INT s, INT r, INT i) { return window_index(ref_tr::windowSlopes[s][r][i]); }
/* FDKgetWindowSlope(): the result of the copy as an index of windows[] (-1: NULL, -2: unknown),
   and the result of the library in *lib. */
INT cref_tr_get_window_slope(INT length, INT shape, const SHORT **lib) {
  *lib = (const SHORT *)::FDKgetWindowSlope(length, shape);
  return window_index(ref_tr::FDKgetWindowSlope(length, shape));
}

const void *cref_tr_get_bitstream_element_list(INT aot, INT epConfig, INT nChannels, INT layer, UINT elFlags) {
  return (const void *)getBitstreamElementList((AUDIO_OBJECT_TYPE)aot, (SCHAR)epConfig, (UCHAR)nChannels,
                                               (UCHAR)layer, elFlags);
}
const INT *cref_tr_element_list_ids(const void *node) {
  return (const INT *)((const element_list_t *)node)->id;
}
const void *cref_tr_element_list_next(const void *node, INT i) {
  return (const void *)((const element_list_t *)node)->next[i];
}
/* The element lists of FDK_tools_rom.cpp (file-static; the copy), in the order of the source. */
struct ElEntry {
  const char *name;
  const rbd_id_t *ids;
  INT n;
};
#define EL(x) {#x, ref_tr::x, (INT)(sizeof(ref_tr::x) / sizeof(ref_tr::x[0]))}
static const ElEntry el_lists[] = {EL(el_aac_sce), EL(el_aac_cce), EL(el_aac_cpe), EL(el_aac_cpe0), EL(el_aac_cpe1), EL(el_aac_sce_epc0), EL(el_aac_sce_epc1), EL(el_aac_cpe_epc0), EL(el_aac_cpe0_epc0), EL(el_aac_cpe1_epc0), EL(el_aac_cpe0_epc1), EL(el_aac_cpe1_epc1), EL(el_scal_sce_epc0), EL(el_scal_cpe_epc0), EL(el_scal_sce_epc1), EL(el_scal_cpe_epc1), EL(el_drm_sce), EL(el_drm_cpe), EL(el_eld_sce_epc0), EL(el_eld_cpe_epc0), EL(el_eld_cpe_epc1), EL(el_usac_coremode), EL(el_usac_sce0_epc0), EL(el_usac_lfe_epc0), EL(el_usac_lpd_epc0), EL(list_usac_cpe00_epc0), EL(el_usac_common_tw), EL(list_usac_cpe0000_epc0), EL(list_usac_cpe0001_epc0), EL(list_usac_cpe001_epc0), EL(list_usac_cpe0010_epc0), EL(list_usac_cpe0011_epc0), EL(list_usac_cpe10_epc0), EL(list_usac_cpe01_epc0), EL(list_usac_cpe11_epc0)};
#undef EL
/* Element list i of el_lists[]: its name, its ids (*ids) and its length; NULL after the last. */
const char *cref_tr_element_list(INT i, const INT **ids, INT *n) {
  if (i < 0 || i >= (INT)(sizeof(el_lists) / sizeof(el_lists[0]))) return NULL;
  *ids = (const INT *)el_lists[i].ids;
  *n = el_lists[i].n;
  return el_lists[i].name;
}
INT cref_tr_num_rbd_ids(void) { return (INT)(sizeof(rbd_ids) / sizeof(rbd_ids[0])); }
INT cref_tr_rbd_id(INT i, const char **name) {
  *name = rbd_ids[i].name;
  return rbd_ids[i].value;
}

SHORT cref_tr_fx_dbl2fx_qtw(FIXP_DBL x) { return FX_DBL2FX_QTW(x); }

/* The constants of FDK_tools_rom.h, in the order of the Rust test. */
INT cref_tr_constants(INT *out) {
  INT n = 0;
  out[n++] = QMF_FILTER_PROTOTYPE_SIZE;
  out[n++] = QMF_NO_POLY;
  out[n++] = QMF640_PFT_TABLE_SIZE;
  out[n++] = QMF640_CLDFB_PFT_TABLE_SIZE;
  out[n++] = QMF320_CLDFB_PFT_TABLE_SIZE;
  out[n++] = QMF_CLDFB_PFT_SCALE;
  out[n++] = QMF160_CLDFB_PFT_TABLE_SIZE;
  out[n++] = QMF80_CLDFB_PFT_TABLE_SIZE;
  out[n++] = QMF320_MPSLDFB_PFT_TABLE_SIZE;
  out[n++] = QMF640_MPSLDFB_PFT_TABLE_SIZE;
  out[n++] = QMF_MPSLDFB_PFT_SCALE;
  out[n++] = AC_EL_GA_CCE;
  out[n++] = FDK_FORMAT_1_0;
  out[n++] = FDK_FORMAT_2_0;
  out[n++] = FDK_FORMAT_3_0_FC;
  out[n++] = FDK_FORMAT_3_1_0;
  out[n++] = FDK_FORMAT_5_0;
  out[n++] = FDK_FORMAT_5_1;
  out[n++] = FDK_FORMAT_7_1_ALT;
  out[n++] = FDK_FORMAT_3_0_RC;
  out[n++] = FDK_FORMAT_2_2_0;
  out[n++] = FDK_FORMAT_6_1;
  out[n++] = FDK_FORMAT_7_1;
  out[n++] = FDK_FORMAT_22_2;
  out[n++] = FDK_FORMAT_5_2_1;
  out[n++] = FDK_FORMAT_5_5_2;
  out[n++] = FDK_FORMAT_9_1;
  out[n++] = FDK_FORMAT_6_5_1;
  out[n++] = FDK_FORMAT_6_7_1;
  out[n++] = FDK_FORMAT_5_6_1;
  out[n++] = FDK_FORMAT_7_6_1;
  out[n++] = FDK_FORMAT_IN_LISTOFCHANNELS;
  out[n++] = FDK_FORMAT_OUT_LISTOFCHANNELS;
  out[n++] = FDK_NFORMATS;
  out[n++] = FDK_FORMAT_FAIL;
  out[n++] = SQRT_VALUES;
  return n;
}
}
