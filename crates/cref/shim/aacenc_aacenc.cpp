// Shims for the differential tests of fdk_aac_rs_aacenc::aacenc.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_aacenc (see build.rs).
//
// aacenc.cpp is included into a namespace for its file-static functions and tables; its
// non-static functions get other names there (aacenc.h declares them extern "C").

#include "cref_common.h"
#include "libAACenc/src/aacenc.h"
#include "libAACenc/src/bitenc.h"
#include "libAACenc/src/interface.h"
#include "libAACenc/src/psy_configuration.h"
#include "libAACenc/src/psy_main.h"
#include "libAACenc/src/qc_main.h"
#include "libAACenc/src/bandwidth.h"
#include "libAACenc/src/channel_map.h"
#include "libAACenc/src/tns_func.h"
#include "libAACenc/src/aacEnc_ram.h"

#include <stddef.h>

#define FDKaacEnc_CalcBitsPerFrame ref_aacenc_CalcBitsPerFrame
#define FDKaacEnc_CalcBitrate ref_aacenc_CalcBitrate
#define FDKaacEnc_LimitBitrate ref_aacenc_LimitBitrate
#define FDKaacEnc_GetVBRBitrate ref_aacenc_GetVBRBitrate
#define FDKaacEnc_AdjustVBRBitrateMode ref_aacenc_AdjustVBRBitrateMode
#define FDKaacEnc_GetBitReservoirState ref_aacenc_GetBitReservoirState
#define FDKaacEnc_AacInitDefaultConfig ref_aacenc_AacInitDefaultConfig
#define FDKaacEnc_Open ref_aacenc_Open
#define FDKaacEnc_Initialize ref_aacenc_Initialize
#define FDKaacEnc_EncodeFrame ref_aacenc_EncodeFrame
#define FDKaacEnc_Close ref_aacenc_Close
namespace ref_aacenc {
#include "libAACenc/src/aacenc.cpp"
}
#undef FDKaacEnc_CalcBitsPerFrame
#undef FDKaacEnc_CalcBitrate
#undef FDKaacEnc_LimitBitrate
#undef FDKaacEnc_GetVBRBitrate
#undef FDKaacEnc_AdjustVBRBitrateMode
#undef FDKaacEnc_GetBitReservoirState
#undef FDKaacEnc_AacInitDefaultConfig
#undef FDKaacEnc_Open
#undef FDKaacEnc_Initialize
#undef FDKaacEnc_EncodeFrame
#undef FDKaacEnc_Close

extern "C" {
INT cref_aacenc_calc_bits_per_frame(INT bitRate, INT frameLength, INT samplingRate) {
  return FDKaacEnc_CalcBitsPerFrame(bitRate, frameLength, samplingRate);
}
INT cref_aacenc_calc_bitrate(INT bitsPerFrame, INT frameLength, INT samplingRate) {
  return FDKaacEnc_CalcBitrate(bitsPerFrame, frameLength, samplingRate);
}
/* FDKaacEnc_LimitBitrate; hTpEnc is a HANDLE_TRANSPORTENC or NULL; *avg = -12345 before. */
INT cref_aacenc_limit_bitrate(void *hTpEnc, INT aot, INT coreSamplingRate, INT frameLength, INT nChannels, INT nChannelsEff, INT bitRate, INT averageBits, INT *avg, INT bitrateMode, INT nSubFrames) {
  return FDKaacEnc_LimitBitrate((HANDLE_TRANSPORTENC)hTpEnc, (AUDIO_OBJECT_TYPE)aot,
                                coreSamplingRate, frameLength, nChannels, nChannelsEff, bitRate,
                                averageBits, avg, (AACENC_BITRATE_MODE)bitrateMode, nSubFrames);
}
INT cref_aacenc_get_vbr_bitrate(INT bitrateMode, INT channelMode) {
  return FDKaacEnc_GetVBRBitrate((AACENC_BITRATE_MODE)bitrateMode, (CHANNEL_MODE)channelMode);
}
INT cref_aacenc_adjust_vbr_bitrate_mode(INT bitrateMode, INT bitrate, INT channelMode) {
  return FDKaacEnc_AdjustVBRBitrateMode((AACENC_BITRATE_MODE)bitrateMode, bitrate,
                                        (CHANNEL_MODE)channelMode);
}
/* FDKaacEnc_AacInitDefaultConfig into an AACENC_CONFIG (the port's layout) */
void cref_aacenc_init_default_config(void *config) {
  memset(config, 0x5a, sizeof(AACENC_CONFIG));
  FDKaacEnc_AacInitDefaultConfig((AACENC_CONFIG *)config);
}
INT cref_aacenc_config_size(void) {
  return sizeof(AACENC_CONFIG);
}
/* FDKaacEnc_InitCheckAncillary (file-static) */
INT cref_aacenc_init_check_ancillary(INT bitRate, INT framelength, INT ancillaryRate, INT *ancillaryBitsPerFrame, INT sampleRate) {
  return ref_aacenc::FDKaacEnc_InitCheckAncillary(bitRate, framelength, ancillaryRate,
                                                  ancillaryBitsPerFrame, sampleRate);
}
/* configTabVBR as INTs (bitrateMode, chanBitrate[0], chanBitrate[1] per entry); returns the
   number of entries. */
INT cref_aacenc_config_tab_vbr(INT *out) {
  INT n = (INT)(sizeof(ref_aacenc::configTabVBR) / sizeof(ref_aacenc::configTabVBR[0]));
  for (INT i = 0; i < n; i++) {
    out[3 * i] = ref_aacenc::configTabVBR[i].bitrateMode;
    out[3 * i + 1] = ref_aacenc::configTabVBR[i].chanBitrate[0];
    out[3 * i + 2] = ref_aacenc::configTabVBR[i].chanBitrate[1];
  }
  return n;
}
}
