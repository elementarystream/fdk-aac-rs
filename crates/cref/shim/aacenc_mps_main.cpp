// Shims for the differential tests of fdk_aac_rs_aacenc::mps_main.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_mps_main (see build.rs).
//
// mps_main.cpp is included at file scope for its table (the structure MPS_ENCODER is defined
// there, so a namespace copy would not match the handle type of mps_main.h); its non-static
// functions get other names in the copy. The tests call the library's FDK_MpegsEnc_*
// functions.

#include "cref_common.h"
#include "libAACenc/src/aacenc.h"
#include "libAACenc/src/mps_main.h"
#include "sacenc_lib.h"
#include "FDK_bitstream.h"

#include <stddef.h>

static MPS_ENCODER_ERROR (*const lib_Open)(HANDLE_MPS_ENCODER *) = FDK_MpegsEnc_Open;
static MPS_ENCODER_ERROR (*const lib_Close)(HANDLE_MPS_ENCODER *) = FDK_MpegsEnc_Close;
static MPS_ENCODER_ERROR (*const lib_Init)(HANDLE_MPS_ENCODER, const AUDIO_OBJECT_TYPE,
                                           const UINT, const UINT, const UINT, const UINT,
                                           const UINT, const UINT) = FDK_MpegsEnc_Init;
static MPS_ENCODER_ERROR (*const lib_Process)(HANDLE_MPS_ENCODER, INT_PCM *const, const INT,
                                              AACENC_EXT_PAYLOAD *) = FDK_MpegsEnc_Process;
static INT (*const lib_WriteSsc)(HANDLE_MPS_ENCODER, HANDLE_FDK_BITSTREAM) =
    FDK_MpegsEnc_WriteSpatialSpecificConfig;
static INT (*const lib_Closest)(const AUDIO_OBJECT_TYPE, const CHANNEL_MODE, const UINT,
                                const UINT, const UINT) = FDK_MpegsEnc_GetClosestBitRate;
static INT (*const lib_GetDelay)(HANDLE_MPS_ENCODER) = FDK_MpegsEnc_GetDelay;
static INT (*const lib_GetDecDelay)(HANDLE_MPS_ENCODER) = FDK_MpegsEnc_GetDecDelay;

#define FDK_MpegsEnc_Open copy_FDK_MpegsEnc_Open
#define FDK_MpegsEnc_Close copy_FDK_MpegsEnc_Close
#define FDK_MpegsEnc_Init copy_FDK_MpegsEnc_Init
#define FDK_MpegsEnc_Process copy_FDK_MpegsEnc_Process
#define FDK_MpegsEnc_WriteSpatialSpecificConfig copy_FDK_MpegsEnc_WriteSpatialSpecificConfig
#define FDK_MpegsEnc_GetClosestBitRate copy_FDK_MpegsEnc_GetClosestBitRate
#define FDK_MpegsEnc_GetDelay copy_FDK_MpegsEnc_GetDelay
#define FDK_MpegsEnc_GetDecDelay copy_FDK_MpegsEnc_GetDecDelay
#define FDK_MpegsEnc_GetLibInfo copy_FDK_MpegsEnc_GetLibInfo
/* the copy calls these before defining them */
MPS_ENCODER_ERROR copy_FDK_MpegsEnc_Close(HANDLE_MPS_ENCODER *phMpsEnc);
INT copy_FDK_MpegsEnc_GetClosestBitRate(const AUDIO_OBJECT_TYPE audioObjectType,
                                        const CHANNEL_MODE channelMode,
                                        const UINT samplingrate, const UINT sbrRatio,
                                        const UINT bitrate);
INT copy_FDK_MpegsEnc_WriteSpatialSpecificConfig(HANDLE_MPS_ENCODER hMpsEnc,
                                                 HANDLE_FDK_BITSTREAM hBs);
#include "libAACenc/src/mps_main.cpp"
#undef FDK_MpegsEnc_Open
#undef FDK_MpegsEnc_Close
#undef FDK_MpegsEnc_Init
#undef FDK_MpegsEnc_Process
#undef FDK_MpegsEnc_WriteSpatialSpecificConfig
#undef FDK_MpegsEnc_GetClosestBitRate
#undef FDK_MpegsEnc_GetDelay
#undef FDK_MpegsEnc_GetDecDelay
#undef FDK_MpegsEnc_GetLibInfo

extern "C" {
/* mpsConfigTab as UINTs (6 per entry); returns the number of entries */
INT cref_mps_main_tab(UINT *out) {
  INT n = (INT)(sizeof(mpsConfigTab) / sizeof(mpsConfigTab[0]));
  for (INT i = 0; i < n; i++) {
    out[6 * i] = mpsConfigTab[i].audio_object_type;
    out[6 * i + 1] = mpsConfigTab[i].channel_mode;
    out[6 * i + 2] = mpsConfigTab[i].sbr_ratio;
    out[6 * i + 3] = mpsConfigTab[i].sampling_rate;
    out[6 * i + 4] = mpsConfigTab[i].bitrate_min;
    out[6 * i + 5] = mpsConfigTab[i].bitrate_max;
  }
  return n;
}
INT cref_mps_main_closest(INT aot, INT channelMode, UINT samplingrate, UINT sbrRatio, UINT bitrate) {
  return lib_Closest((AUDIO_OBJECT_TYPE)aot, (CHANNEL_MODE)channelMode, samplingrate, sbrRatio,
                     bitrate);
}
INT cref_mps_main_open(void **ph) {
  HANDLE_MPS_ENCODER h = NULL;
  INT e = lib_Open(&h);
  *ph = h;
  return e;
}
INT cref_mps_main_close(void *h) {
  HANDLE_MPS_ENCODER hh = (HANDLE_MPS_ENCODER)h;
  return lib_Close(&hh);
}
INT cref_mps_main_init(void *h, INT aot, UINT samplingrate, UINT bitrate, UINT sbrRatio, UINT framelength, UINT inputBufferSizePerChannel, UINT coreCoderDelay) {
  return lib_Init((HANDLE_MPS_ENCODER)h, (AUDIO_OBJECT_TYPE)aot, samplingrate, bitrate, sbrRatio,
                  framelength, inputBufferSizePerChannel, coreCoderDelay);
}
/* FDK_MpegsEnc_Process; the payload as {pData == sacOutBuffer, dataSize, dataType,
   associatedChElement} in pl[0..4] and its bytes (128) in bytes. */
INT cref_mps_main_process(void *h, SHORT *audio, INT n, INT *pl, UCHAR *bytes) {
  AACENC_EXT_PAYLOAD p;
  memset(&p, 0, sizeof(p));
  p.associatedChElement = -77;
  INT e = lib_Process((HANDLE_MPS_ENCODER)h, audio, n, &p);
  pl[0] = p.pData == ((HANDLE_MPS_ENCODER)h)->sacOutBuffer;
  pl[1] = (INT)p.dataSize;
  pl[2] = p.dataType;
  pl[3] = p.associatedChElement;
  if (p.pData) memcpy(bytes, p.pData, 128);
  return e;
}
/* FDK_MpegsEnc_WriteSpatialSpecificConfig(h, bs ? bitstream on buf (128 bytes) : NULL);
   *bits = valid bits of the bitstream */
INT cref_mps_main_write_ssc(void *h, INT useBs, UCHAR *buf, INT *bits) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, 128, 0, BS_WRITER);
  INT r = lib_WriteSsc((HANDLE_MPS_ENCODER)h, useBs ? &bs : NULL);
  *bits = (INT)FDKgetValidBits(&bs);
  return r;
}
INT cref_mps_main_get_delay(void *h, INT dec) {
  return dec ? lib_GetDecDelay((HANDLE_MPS_ENCODER)h) : lib_GetDelay((HANDLE_MPS_ENCODER)h);
}
}
