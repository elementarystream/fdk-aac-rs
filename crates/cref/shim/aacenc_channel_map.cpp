// Shims for the differential tests of fdk_aac_rs_aacenc::channel_map.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_channel_map (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/channel_map.h"
#include "libAACenc/src/qc_data.h"

extern "C" {
/* FDKaacEnc_DetermineEncoderMode(); *mode is input and output. */
INT cref_channel_map_DetermineEncoderMode(INT *mode, INT nChannels) {
  CHANNEL_MODE m = (CHANNEL_MODE)*mode;
  INT err = FDKaacEnc_DetermineEncoderMode(&m, nChannels);
  *mode = (INT)m;
  return err;
}
/* FDKaacEnc_InitChannelMapping() into the CHANNEL_MAPPING at cm (C layout). */
INT cref_channel_map_InitChannelMapping(INT mode, INT co, void *cm) {
  return FDKaacEnc_InitChannelMapping((CHANNEL_MODE)mode, (CHANNEL_ORDER)co,
                                      (CHANNEL_MAPPING *)cm);
}
/* FDKaacEnc_InitElementBits() with hQC->elementBits[i] pointing to eb[i] (8 ELEMENT_BITS, C
   layout, input and output). */
INT cref_channel_map_InitElementBits(void *eb, void *cm, INT bitrateTot, INT averageBitsTot, INT maxChannelBits) {
  QC_STATE qc;
  FDKmemclear(&qc, sizeof(qc));
  for (int i = 0; i < 8; i++) qc.elementBits[i] = &((ELEMENT_BITS *)eb)[i];
  return FDKaacEnc_InitElementBits(&qc, (CHANNEL_MAPPING *)cm, bitrateTot, averageBitsTot,
                                   maxChannelBits);
}
/* FDKaacEnc_GetMonoStereoMode() */
INT cref_channel_map_GetMonoStereoMode(INT mode) {
  return (INT)FDKaacEnc_GetMonoStereoMode((CHANNEL_MODE)mode);
}
/* FDKaacEnc_GetChannelModeConfiguration(): index of the entry in channelModeConfig (relative to
   the entry of MODE_1, the first one) and a copy of it in out[4]; -1 for NULL. */
INT cref_channel_map_GetChannelModeConfiguration(INT mode, INT *out) {
  const CHANNEL_MODE_CONFIG_TAB *p = FDKaacEnc_GetChannelModeConfiguration((CHANNEL_MODE)mode);
  if (p == NULL) return -1;
  FDKmemcpy(out, p, sizeof(*p));
  return (INT)(p - FDKaacEnc_GetChannelModeConfiguration(MODE_1));
}
}
