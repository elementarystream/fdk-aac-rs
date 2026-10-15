// Shims for the differential tests of fdk_aac_rs_aacdec::usacdec_rom.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_usacdec_rom (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/usacdec_rom.h"

/* A copy of usacdec_rom.cpp, for the size of the tables that the header declares without a
   size. */
namespace ref_urom {
#include "libAACdec/src/usacdec_rom.cpp"
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
#define TGN(x) {#x, (const void *)::x, (INT)sizeof(ref_urom::x), (INT)sizeof(ref_urom::x[0])}

static const TableEntry tables[] = {
    TG(fdk_dec_tab_factorial), TGN(fdk_dec_Da), TG(fdk_dec_Ds), TGN(fdk_dec_Ns),
    TG(fdk_dec_Ia), TG(fdk_dec_Is), TGN(fdk_dec_A3), TGN(fdk_dec_A4), TGN(fdk_dec_I3),
    TGN(fdk_dec_I4), TG(fdk_dec_lsf_init), TG(fdk_dec_dico_lsf_abs_8b), TG(fdk_t_qua_gain7b),
    TG(lsp_interpol_factor), TG(fdk_dec_filt_lp), TG(FacWindowSynth128), TG(FacWindowZir128),
    TG(FacWindowSynth64), TG(FacWindowZir64), TG(FacWindowSynth32), TG(FacWindowZir32),
    TG(FacWindowSynth96), TG(FacWindowZir96), TG(FacWindowSynth48), TG(FacWindowZir48),
};

extern "C" {
INT cref_urom_num_tables(void) { return (INT)(sizeof(tables) / sizeof(tables[0])); }
const void *cref_urom_table(INT i, INT *bytes, INT *elem_bytes) {
  *bytes = tables[i].bytes;
  *elem_bytes = tables[i].elem_bytes;
  return tables[i].ptr;
}
const char *cref_urom_table_name(INT i) { return tables[i].name; }

/* The constants of usacdec_rom.h, in the order of the Rust test. */
INT cref_urom_constants(INT *out) {
  INT n = 0;
  out[n++] = NB_SPHERE;
  out[n++] = NB_LEADER;
  out[n++] = NB_LDSIGN;
  out[n++] = NB_LDQ3;
  out[n++] = NB_LDQ4;
  out[n++] = LSF_SCALE;
  out[n++] = SF_QUA_GAIN7B;
  out[n++] = L_FILT;
  return n;
}
}
