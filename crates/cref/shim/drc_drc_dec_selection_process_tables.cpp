// Shims for the table tests of fdk_aac_rs_drc::drc_dec_selection_process.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::drc_drc_dec_selection_process_tables (see build.rs).
//
// drcDec_selectionProcess.cpp is compiled here in a namespace together with its headers (they
// are not included at global scope before), for its file-static table and for the file-static
// functions that hold the function-local tables secondTry and pOrdering. drcDec_tools.cpp is
// compiled into the namespace as well, for the functions the copy calls.

#include <math.h> /* before the namespace: fixpoint_math_x86.h includes it */
#include <stdlib.h>
#include "cref_common.h"

namespace ref_sel {
#include "libDRCdec/src/drcDec_selectionProcess.cpp"
/* selectDrcCoefficients() and the other helpers the copy calls (declared in the namespace). */
#include "libDRCdec/src/drcDec_tools.cpp"
}

extern "C" {
/* fallbackEffectTypeRequests (file-static) of the copy as memory. */
const void *cref_drc_sel_fallback_effect_type_requests(INT *bytes) {
  *bytes = (INT)sizeof(ref_sel::fallbackEffectTypeRequests);
  return ref_sel::fallbackEffectTypeRequests;
}

/* pOrdering[measurementSystemRequested][measurementSystem] of _getMethodValue() (the copy): the
   order the function stores into a VALUE_ORDER with order -1 (every element is >= 0). Returns
   the return value of the function in *ret. */
INT cref_drc_sel_get_method_value(INT measurementSystemRequested, INT measurementSystem, INT *ret) {
  ref_sel::VALUE_ORDER v;
  v.value = 0;
  v.order = -1;
  *ret = ref_sel::_getMethodValue(&v, (ref_sel::FIXP_DBL)0x12345678, measurementSystem,
                                  measurementSystemRequested);
  return v.order;
}

/* _selectDrcCharacteristic() (the copy) for a configuration with two DRC instruction sets, each
   with one channel group: set 0 uses gain set 0, which has one band of the CICP characteristic
   `cicpIndex`, set 1 uses gain set 1, which has one band without a CICP characteristic (it never
   matches). Both sets are candidates. Returns the number of candidates after the call: 1 if set 0
   matched one of the characteristics the function tries, 2 otherwise; the return value of the
   function in *ret. */
INT cref_drc_sel_select_drc_characteristic(INT requested, INT cicpIndex, INT *ret) {
  ref_sel::UNI_DRC_CONFIG *cfg = (ref_sel::UNI_DRC_CONFIG *)calloc(1, sizeof(ref_sel::UNI_DRC_CONFIG));
  ref_sel::DRCDEC_SELECTION *sel = (ref_sel::DRCDEC_SELECTION *)calloc(2, sizeof(ref_sel::DRCDEC_SELECTION));
  cfg->drcCoefficientsUniDrcCount = 1;
  ref_sel::DRC_COEFFICIENTS_UNI_DRC *coef = &cfg->drcCoefficientsUniDrc[0];
  coef->drcLocation = LOCATION_SELECTED;
  coef->gainSetCount = 2;
  for (int g = 0; g < 2; g++) {
    coef->gainSet[g].bandCount = 1;
    coef->gainSet[g].drcCharacteristic[0].present = 1;
    coef->gainSet[g].drcCharacteristic[0].isCICP = (g == 0);
    coef->gainSet[g].drcCharacteristic[0].cicpIndex = (g == 0) ? (UCHAR)cicpIndex : 0;
  }
  cfg->drcInstructionsUniDrcCount = 2;
  cfg->drcInstructionsCountInclVirtual = 2;
  ref_sel::DRCDEC_SELECTION *pot = &sel[0], *selected = &sel[1];
  for (int i = 0; i < 2; i++) {
    cfg->drcInstructionsUniDrc[i].drcSetId = (SCHAR)(i + 1);
    cfg->drcInstructionsUniDrc[i].nDrcChannelGroups = 1;
    cfg->drcInstructionsUniDrc[i].gainSetIndexForChannelGroup[0] = (SCHAR)i;
    ref_sel::DRCDEC_SELECTION_DATA *d = ref_sel::_drcdec_selection_addNew(pot);
    d->pInst = &cfg->drcInstructionsUniDrc[i];
  }
  *ret = ref_sel::_selectDrcCharacteristic(cfg, requested, &pot, &selected);
  INT n = ref_sel::_drcdec_selection_getNumber(selected);
  free(sel);
  free(cfg);
  return n;
}
}
