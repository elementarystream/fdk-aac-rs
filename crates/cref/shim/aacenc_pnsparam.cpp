// Shims for the differential tests of fdk_aac_rs_aacenc::pnsparam.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_pnsparam (see build.rs).
//
// The public functions are called in the library; the file-static tables of pnsparam.cpp are
// reached by including the source file into a namespace (PORTING.md, "Differential tests").
// NOISEPARAMS is passed as its memory image (void*).

#include "cref_common.h"
#include "libAACenc/src/pnsparam.h"
#include "libAACenc/src/psy_configuration.h"

namespace ref_pnsparam {
#include "libAACenc/src/pnsparam.cpp"
}

struct PnsTable {
  const char *name;
  const void *ptr;
  INT bytes;
};

#define T(x) {#x, (const void *)&ref_pnsparam::x, (INT)sizeof(ref_pnsparam::x)}

static const PnsTable pns_tables[] = {
    T(levelTable_mono), T(levelTable_stereo),       T(levelTable_lowComplexity),
    T(pnsInfoTab),      T(pnsInfoTab_lowComplexity),
};

extern "C" {
/* Number of tables. */
INT cref_pnsparam_table_count() { return (INT)(sizeof(pns_tables) / sizeof(pns_tables[0])); }
/* Name, memory and size in bytes of table i. */
const char *cref_pnsparam_table(INT i, const void **ptr, INT *bytes) {
  *ptr = pns_tables[i].ptr;
  *bytes = pns_tables[i].bytes;
  return pns_tables[i].name;
}
INT cref_pnsparam_lookUpPnsUse(INT bitRate, INT sampleRate, INT numChan, INT isLC) {
  return FDKaacEnc_lookUpPnsUse(bitRate, sampleRate, numChan, isLC);
}
INT cref_pnsparam_GetPnsParam(void *np, INT bitRate, INT sampleRate, INT sfbCnt, const INT *sfbOffset, INT *usePns, INT numChan, INT isLC) {
  return (INT)FDKaacEnc_GetPnsParam((NOISEPARAMS *)np, bitRate, sampleRate, sfbCnt, sfbOffset, usePns, numChan, isLC);
}
}
