// Shims for the differential tests of fdk_aac_rs_aacenc::bandwidth.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_bandwidth (see build.rs).

#include "cref_common.h"
#include "libAACenc/src/channel_map.h"
#include "libAACenc/src/bandwidth.h"
#include "libAACenc/src/aacEnc_ram.h"

/* A private copy of bandwidth.cpp for the file-static function and tables. */
namespace ref_bw {
#include "libAACenc/src/bandwidth.cpp"
}

extern "C" {
/* FDKaacEnc_DetermineBandWidth() with the CHANNEL_MAPPING at cm (C layout). */
INT cref_bandwidth_DetermineBandWidth(INT proposedBandWidth, INT bitrate, INT bitrateMode, INT sampleRate, INT frameLength, void *cm, INT encoderMode, INT *bandWidth) {
  return FDKaacEnc_DetermineBandWidth(proposedBandWidth, bitrate, (AACENC_BITRATE_MODE)bitrateMode,
                                      sampleRate, frameLength, (const CHANNEL_MAPPING *)cm,
                                      (CHANNEL_MODE)encoderMode, bandWidth);
}
/* GetBandwidthEntry() (file-static, from the private copy). */
INT cref_bandwidth_GetBandwidthEntry(INT frameLength, INT sampleRate, INT chanBitRate, INT entryNo) {
  return ref_bw::GetBandwidthEntry(frameLength, sampleRate, chanBitRate, entryNo);
}
/* Table i (0: bandWidthTable, 1..5: the LD tables, 6: bandWidthTableVBR) as INTs into out
   (at least 33 entries); returns the number of INTs. */
INT cref_bandwidth_table(INT i, INT *out) {
  const void *p;
  INT n;
  switch (i) {
    case 0: p = ref_bw::bandWidthTable; n = sizeof(ref_bw::bandWidthTable); break;
    case 1: p = ref_bw::bandWidthTable_LD_22050; n = sizeof(ref_bw::bandWidthTable_LD_22050); break;
    case 2: p = ref_bw::bandWidthTable_LD_24000; n = sizeof(ref_bw::bandWidthTable_LD_24000); break;
    case 3: p = ref_bw::bandWidthTable_LD_32000; n = sizeof(ref_bw::bandWidthTable_LD_32000); break;
    case 4: p = ref_bw::bandWidthTable_LD_44100; n = sizeof(ref_bw::bandWidthTable_LD_44100); break;
    case 5: p = ref_bw::bandWidthTable_LD_48000; n = sizeof(ref_bw::bandWidthTable_LD_48000); break;
    default: p = ref_bw::bandWidthTableVBR; n = sizeof(ref_bw::bandWidthTableVBR); break;
  }
  FDKmemcpy(out, p, n);
  return n / (INT)sizeof(INT);
}
}
