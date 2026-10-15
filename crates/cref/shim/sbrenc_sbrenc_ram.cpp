// Shims for the differential tests of fdk_aac_rs_sbrenc::sbrenc_ram.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_sbrenc_ram (see build.rs).

#include "cref_common.h"
#include "libSBRenc/src/sbrenc_ram.h"

extern "C" {
/* The layout of the dynamic RAM in FIXP_DBL values: its size, OFFSET_NRG, the buffer sizes and
   the offsets of GetRam_Sbr_env{R,I,Y}Buffer(n) for n = 0, 1. */
void cref_sbrenc_ram_layout(INT *out) {
  static FIXP_DBL ram[SBR_ENC_DYN_RAM_SIZE / sizeof(FIXP_DBL)];
  UCHAR *base = (UCHAR *)ram;
  out[0] = (INT)(SBR_ENC_DYN_RAM_SIZE / sizeof(FIXP_DBL));
  out[1] = (INT)(OFFSET_NRG / sizeof(FIXP_DBL));
  out[2] = (INT)(ENV_R_BUFF_BYTE / sizeof(FIXP_DBL));
  out[3] = (INT)(ENV_I_BUFF_BYTE / sizeof(FIXP_DBL));
  out[4] = (INT)(Y_2_BUF_BYTE / sizeof(FIXP_DBL));
  for (INT n = 0; n < 2; n++) {
    out[5 + 3 * n] = (INT)(GetRam_Sbr_envRBuffer(n, base) - ram);
    out[6 + 3 * n] = (INT)(GetRam_Sbr_envIBuffer(n, base) - ram);
    out[7 + 3 * n] = (INT)(GetRam_Sbr_envYBuffer(n, base) - ram);
  }
}
}
