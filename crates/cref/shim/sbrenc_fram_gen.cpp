// Shims for the differential tests of fdk_aac_rs_sbrenc::fram_gen.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_fram_gen (see build.rs).
//
// The frame generator of the library on a heap allocated SBR_ENVELOPE_FRAME; its state is
// dumped with sbrdump_env_frame() (sbrenc_sbr_encoder_dump.cpp). The tables (file-static) are
// reached through a copy of fram_gen.cpp in a namespace.

#include "cref_common.h"
#include <stdlib.h>
#include "libSBRenc/src/fram_gen.h"
#include "libSBRenc/src/sbr_misc.h"
#include "genericStds.h"

namespace ref_fram_gen {
#include "libSBRenc/src/fram_gen.cpp"
}

struct SbrDump {
  INT *buf;
  INT cap;
  INT n;
};
void sbrdump_env_frame(SbrDump *d, const SBR_ENVELOPE_FRAME *h);
void sbrdump_frame_info(SbrDump *d, const SBR_FRAME_INFO *f);

extern "C" {
void *cref_fram_gen_new(void) { return calloc(1, sizeof(SBR_ENVELOPE_FRAME)); }
void cref_fram_gen_free(void *h) { free(h); }
void cref_fram_gen_init(void *h, INT allowSpread, INT numEnvStatic, INT staticFraming, INT timeSlots, INT fr0, INT fr1, INT fResTransIsLow, INT ldGrid) {
  FREQ_RES fr[2] = {(FREQ_RES)fr0, (FREQ_RES)fr1};
  FDKsbrEnc_initFrameInfoGenerator((SBR_ENVELOPE_FRAME *)h, allowSpread, numEnvStatic, staticFraming, timeSlots, fr, (UCHAR)fResTransIsLow, ldGrid);
}
/* Returns 1 if the result is &h->SbrFrameInfo. */
INT cref_fram_gen_generate(void *h, UCHAR *tran, INT rightBorderFIX, UCHAR *pre, INT ldGrid, const INT *v_tuning) {
  SBR_ENVELOPE_FRAME *f = (SBR_ENVELOPE_FRAME *)h;
  HANDLE_SBR_FRAME_INFO r = FDKsbrEnc_frameInfoGenerator(f, tran, rightBorderFIX, pre, ldGrid, v_tuning);
  return r == &f->SbrFrameInfo;
}
INT cref_fram_gen_dump(void *h, INT *buf, INT cap) {
  SbrDump d = {buf, cap, 0};
  sbrdump_env_frame(&d, (const SBR_ENVELOPE_FRAME *)h);
  return d.n;
}
/* The frame info tables (15, in the order of the generator) and the other tables of
   fram_gen.cpp: 15 envelopeTable_8, 16 envelopeTable_16, 17 envelopeTable_15,
   18 freqRes_table_8, 19 freqRes_table_16, 20 minFrameTranDistance. */
INT cref_fram_gen_table(INT id, INT *out) {
  const SBR_FRAME_INFO *infos[15] = {
      &ref_fram_gen::frameInfo1_2048,  &ref_fram_gen::frameInfo2_2048,  &ref_fram_gen::frameInfo4_2048,
      &ref_fram_gen::frameInfo1_2304,  &ref_fram_gen::frameInfo2_2304,  &ref_fram_gen::frameInfo4_2304,
      &ref_fram_gen::frameInfo1_1920,  &ref_fram_gen::frameInfo2_1920,  &ref_fram_gen::frameInfo4_1920,
      &ref_fram_gen::frameInfo1_1152,  &ref_fram_gen::frameInfo2_1152,  &ref_fram_gen::frameInfo4_1152,
      &ref_fram_gen::frameInfo1_512LD, &ref_fram_gen::frameInfo2_512LD, &ref_fram_gen::frameInfo4_512LD};
  if (id >= 0 && id < 15) {
    SbrDump d = {out, 64, 0};
    sbrdump_frame_info(&d, infos[id]);
    return d.n;
  }
  INT n = 0;
  switch (id) {
    case 15:
      for (INT i = 0; i < 8; i++)
        for (INT j = 0; j < 5; j++) out[n++] = ref_fram_gen::envelopeTable_8[i][j];
      return n;
    case 16:
      for (INT i = 0; i < 16; i++)
        for (INT j = 0; j < 6; j++) out[n++] = ref_fram_gen::envelopeTable_16[i][j];
      return n;
    case 17:
      for (INT i = 0; i < 15; i++)
        for (INT j = 0; j < 6; j++) out[n++] = ref_fram_gen::envelopeTable_15[i][j];
      return n;
    case 18:
      for (INT i = 0; i < (INT)(sizeof(ref_fram_gen::freqRes_table_8) / sizeof(FREQ_RES)); i++)
        out[n++] = ref_fram_gen::freqRes_table_8[i];
      return n;
    case 19:
      for (INT i = 0; i < 16; i++) out[n++] = ref_fram_gen::freqRes_table_16[i];
      return n;
    case 20:
      out[0] = ref_fram_gen::minFrameTranDistance;
      return 1;
  }
  return -1;
}
}
