// Shims for the differential tests of fdk_aac_rs_aacenc::qc_data (and the layouts of the other
// encoder structs that the port declares with the layout of C).
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_qc_data (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/qc_data.h"
#include "libAACenc/src/psy_main.h"
#include "libAACenc/src/psy_configuration.h"
#include "libAACenc/src/channel_map.h"

#include <stddef.h>

struct LayoutEntry {
  const char *name;
  INT size;
};

#define S(x) {#x, (INT)sizeof(x)}
#define O(t, f) {#t "." #f, (INT)offsetof(t, f)}

static const LayoutEntry layouts[] = {
    S(BLOCK_SWITCHING_CONTROL), S(TNS_PARAMETER_TABULATED), S(TNS_CONFIG), S(TNS_SUBBLOCK_INFO),
    S(TNS_DATA_RAW), S(TNS_DATA), S(TNS_INFO), S(NOISEPARAMS), S(PNS_CONFIG), S(PNS_DATA),
    S(PSY_DATA), S(TOOLSINFO), S(PSY_OUT_CHANNEL), S(SECTION_INFO), S(SECTION_DATA),
    S(PE_CHANNEL_DATA), S(PE_DATA), S(BRES_PARAM), S(AH_PARAM), S(MINSNR_ADAPT_PARAM),
    S(ATS_ELEMENT), S(AACENC_CONFIG), S(ELEMENT_INFO), S(CHANNEL_MAPPING), S(PADDING),
    S(QC_OUT_CHANNEL), S(ELEMENT_BITS), S(PSY_CONFIGURATION), S(SFB_PARAM_LONG),
    S(SFB_PARAM_SHORT), S(CHANNEL_MODE_CONFIG_TAB),
    O(PSY_DATA, sfbThreshold), O(PSY_OUT_CHANNEL, mdctSpectrum), O(NOISEPARAMS, gapFillThr),
    O(PSY_CONFIGURATION, minRemainingThresholdFactor), O(PSY_CONFIGURATION, tnsConf),
    O(PSY_CONFIGURATION, pnsConf), O(AACENC_CONFIG, epConfig), O(AACENC_CONFIG, anc_Rate),
    O(AACENC_CONFIG, downscaleFactor), O(QC_OUT_CHANNEL, sectionData),
};

extern "C" {
/* Number of layout entries. */
INT cref_qc_data_layout_count() { return (INT)(sizeof(layouts) / sizeof(layouts[0])); }
/* Name and value (sizeof, or offsetof for "TYPE.field") of layout entry i. */
const char *cref_qc_data_layout(INT i, INT *value) {
  *value = layouts[i].size;
  return layouts[i].name;
}
}
