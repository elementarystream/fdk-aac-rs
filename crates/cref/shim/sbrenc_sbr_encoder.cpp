// Shims for the differential tests of fdk_aac_rs_sbrenc::sbr_encoder (the whole SBR encoder).
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_sbr_encoder (see build.rs).
//
// The SBR encoder API of the library on a handle from sbrEncoder_Open(), driven as the AAC
// encoder drives it (aacenc_lib.cpp): sbrEncoder_Init() on the encoder's input buffer,
// sbrEncoder_EncodeFrame() and sbrEncoder_UpdateBuffers() per frame. The complete state is
// dumped with sbrdump_sbr_encoder() (sbrenc_sbr_encoder_dump.cpp).

#include "cref_common.h"
#include <string.h>
#include "sbr_encoder.h"
#include "libSBRenc/src/sbr.h"

struct SbrDump {
  INT *buf;
  INT cap;
  INT n;
};
void sbrdump_sbr_encoder(SbrDump *d, const SBR_ENCODER *e);

/* The harness clears the stack below its frame before sbrEncoder_Init(), so that values without
   a defined reference value are 0, as in the port. */
static __attribute__((noinline)) void clear_stack(void) {
  volatile UCHAR buf[64 * 1024];
  for (UINT i = 0; i < sizeof(buf); i++) buf[i] = 0;
}

/* ELEMENT_INFO as 8 INTs: elType, bitRate, instanceTag, fParametricStereo, fDualMono,
   nChannelsInEl, ChannelIndex[0], ChannelIndex[1]. */
static void el_from(SBR_ELEMENT_INFO *el, const INT *v) {
  for (INT i = 0; i < 8; i++) {
    const INT *p = v + 8 * i;
    el[i].elType = (MP4_ELEMENT_ID)p[0];
    el[i].bitRate = p[1];
    el[i].instanceTag = p[2];
    el[i].fParametricStereo = (UCHAR)p[3];
    el[i].fDualMono = (UCHAR)p[4];
    el[i].nChannelsInEl = (UCHAR)p[5];
    el[i].ChannelIndex[0] = (UCHAR)p[6];
    el[i].ChannelIndex[1] = (UCHAR)p[7];
  }
}
static void el_to(const SBR_ELEMENT_INFO *el, INT *v) {
  for (INT i = 0; i < 8; i++) {
    INT *p = v + 8 * i;
    p[0] = el[i].elType;
    p[1] = el[i].bitRate;
    p[2] = el[i].instanceTag;
    p[3] = el[i].fParametricStereo;
    p[4] = el[i].fDualMono;
    p[5] = el[i].nChannelsInEl;
    p[6] = el[i].ChannelIndex[0];
    p[7] = el[i].ChannelIndex[1];
  }
}

extern "C" {
/* Returns the handle, or NULL if sbrEncoder_Open() fails. */
void *cref_sbr_encoder_open(INT nElements, INT nChannels, INT supportPS) {
  HANDLE_SBR_ENCODER h = NULL;
  if (sbrEncoder_Open(&h, nElements, nChannels, supportPS) != 0) return NULL;
  return h;
}
void cref_sbr_encoder_close(void *h) {
  HANDLE_SBR_ENCODER e = (HANDLE_SBR_ENCODER)h;
  sbrEncoder_Close(&e);
}
/* io: coreBandwidth, inputBufferOffset, numChannels, coreSampleRate, downSampleFactor,
   frameLength, delay (in and out); elInfo: 8 ELEMENT_INFOs of 8 INTs (in and out). */
INT cref_sbr_encoder_init(void *h, INT *elInfo, INT noElements, INT_PCM *inputBuffer, UINT bufSize, INT *io, UINT syntaxFlags, INT aot, INT transformFactor, INT headerPeriod, UINT statesInitFlag) {
  SBR_ELEMENT_INFO el[8];
  FDKmemclear(el, sizeof(el));
  el_from(el, elInfo);
  UINT dsf = (UINT)io[4];
  clear_stack();
  INT r = sbrEncoder_Init((HANDLE_SBR_ENCODER)h, el, noElements, inputBuffer, bufSize, &io[0], &io[1], &io[2], syntaxFlags, &io[3], &dsf, &io[5], (AUDIO_OBJECT_TYPE)aot, &io[6], transformFactor, headerPeriod, statesInitFlag);
  io[4] = (INT)dsf;
  el_to(el, elInfo);
  return r;
}
/* bits: 8 UINTs; data: 8 * MAX_PAYLOAD_SIZE bytes. */
INT cref_sbr_encoder_encode(void *h, INT_PCM *buf, UINT bufSize, UINT *bits, UCHAR *data) {
  return sbrEncoder_EncodeFrame((HANDLE_SBR_ENCODER)h, buf, bufSize, bits, (UCHAR(*)[MAX_PAYLOAD_SIZE])data);
}
INT cref_sbr_encoder_update(void *h, INT_PCM *buf, UINT bufSize) {
  return sbrEncoder_UpdateBuffers((HANDLE_SBR_ENCODER)h, buf, bufSize);
}
/* out: HeaderDelay, BsDelay, EstimateBitrate, InputDataDelay, SbrDecDelay, ContainsHeader. */
void cref_sbr_encoder_info(void *h, INT *out) {
  HANDLE_SBR_ENCODER e = (HANDLE_SBR_ENCODER)h;
  out[0] = sbrEncoder_GetHeaderDelay(e);
  out[1] = sbrEncoder_GetBsDelay(e);
  out[2] = sbrEncoder_GetEstimateBitrate(e);
  out[3] = sbrEncoder_GetInputDataDelay(e);
  out[4] = sbrEncoder_GetSbrDecDelay(e);
  out[5] = sbrEncoder_ContainsHeader(e);
}
/* The getters with a NULL handle. */
void cref_sbr_encoder_info_null(INT *out) {
  out[0] = sbrEncoder_GetHeaderDelay(NULL);
  out[1] = sbrEncoder_GetBsDelay(NULL);
  out[2] = sbrEncoder_GetEstimateBitrate(NULL);
  out[3] = sbrEncoder_GetInputDataDelay(NULL);
  out[4] = sbrEncoder_GetSbrDecDelay(NULL);
  out[5] = sbrEncoder_ContainsHeader(NULL);
  out[6] = sbrEncoder_SendHeader(NULL);
  out[7] = sbrEncoder_SAPPrepare(NULL);
}
INT cref_sbr_encoder_send_header(void *h) { return sbrEncoder_SendHeader((HANDLE_SBR_ENCODER)h); }
INT cref_sbr_encoder_sap_prepare(void *h) { return sbrEncoder_SAPPrepare((HANDLE_SBR_ENCODER)h); }
/* sbrEncoder_GetHeader() into buf (256 bytes); returns the valid bits. */
INT cref_sbr_encoder_get_header(void *h, INT el, INT fSendHeaders, UCHAR *buf) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, 256, 0, BS_WRITER);
  sbrEncoder_GetHeader((HANDLE_SBR_ENCODER)h, &bs, el, fSendHeaders);
  FDKsyncCache(&bs);
  return (INT)FDKgetValidBits(&bs);
}
INT cref_sbr_encoder_in_buffer_size(INT noChannels) { return sbrEncoder_GetInBufferSize(noChannels); }
UINT cref_sbr_encoder_limit_bit_rate(UINT bitRate, UINT numChannels, UINT coreSampleRate, INT aot) {
  return sbrEncoder_LimitBitRate(bitRate, numChannels, coreSampleRate, (AUDIO_OBJECT_TYPE)aot);
}
UINT cref_sbr_encoder_single_rate_possible(INT aot) { return sbrEncoder_IsSingleRatePossible((AUDIO_OBJECT_TYPE)aot); }
/* sbrEncoder_GetLibInfo() on an array of FDK_MODULE_LAST entries whose first `used` entries
   are taken (module ids 1..used): out = result, index, module_id, version, flags; the version
   string and the title into str (64 bytes). */
void cref_sbr_encoder_lib_info(INT used, INT nullInfo, INT *out, char *str) {
  LIB_INFO info[FDK_MODULE_LAST];
  FDKinitLibInfo(info);
  for (INT i = 0; i < used && i < FDK_MODULE_LAST; i++) info[i].module_id = (FDK_MODULE_ID)(i + 1);
  out[0] = sbrEncoder_GetLibInfo(nullInfo ? NULL : info);
  out[1] = -1;
  for (INT i = 0; i < FDK_MODULE_LAST; i++)
    if (info[i].module_id == FDK_SBRENC) out[1] = i;
  if (out[1] >= 0) {
    LIB_INFO *p = &info[out[1]];
    out[2] = p->module_id;
    out[3] = p->version;
    out[4] = (INT)p->flags;
    memcpy(str, p->versionStr, 32);
    strncpy(str + 32, p->title, 31);
    str[63] = 0;
  }
}
INT cref_sbr_encoder_dump(void *h, INT *buf, INT cap) {
  SbrDump d = {buf, cap, 0};
  sbrdump_sbr_encoder(&d, (const SBR_ENCODER *)h);
  return d.n;
}
}
