// Shims for the differential tests of fdk_aac_rs_sacdec::sac_dec_lib (the mpegSurroundDecoder_*
// API) and for the end-to-end tests of shim/sacdec_sac_dec_lib_e2e.cpp.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacdec_sac_dec_lib (see build.rs).
//
// sac_dec_lib.cpp is included here with its external functions renamed to ref_lib_* (the same
// code as the library's, compiled with the same flags), which makes struct MpegSurroundDecoder
// visible for the dumps. The capture functions sacx_mps_* have the signatures of the API; they
// call ref_lib_* and, while a capture is on (per thread), append a record of the call to the
// log of the thread: the inputs (scalars, buffers, bitstream states and bytes, the QMF domain
// before the call) and the outputs (return values, buffers, the complete decoder state and the
// QMF domain after the call). The test replays the records on the port.
//
// Record: 0x5ACD, kind, number of arrays, then each array as its length and its values.

#define mpegSurroundDecoder_IsFullMpegSurroundDecoderInstanceAvailable ref_lib_IsFull
#define mpegSurroundDecoder_Open ref_lib_Open
#define mpegSurroundDecoder_Init ref_lib_Init
#define mpegSurroundDecoder_Config ref_lib_Config
#define mpegSurroundDecoder_ConfigureQmfDomain ref_lib_ConfigureQmfDomain
#define mpegSurroundDecoder_ParseNoHeader ref_lib_ParseNoHeader
#define mpegSurroundDecoder_Parse ref_lib_Parse
#define mpegSurroundDecoder_Apply ref_lib_Apply
#define mpegSurroundDecoder_Close ref_lib_Close
#define mpegSurroundDecoder_FreeMem ref_lib_FreeMem
#define mpegSurroundDecoder_SetParam ref_lib_SetParam
#define mpegSurroundDecoder_GetLibInfo ref_lib_GetLibInfo
#define mpegSurroundDecoder_GetDelay ref_lib_GetDelay
#define mpegSurroundDecoder_IsPseudoLR ref_lib_IsPseudoLR
#define checkTimeSlots ref_lib_checkTimeSlots

#include <stdlib.h>
#include "cref_common.h"
#include "libSACdec/src/sac_dec_lib.cpp"

#undef mpegSurroundDecoder_IsFullMpegSurroundDecoderInstanceAvailable
#undef mpegSurroundDecoder_Open
#undef mpegSurroundDecoder_Init
#undef mpegSurroundDecoder_Config
#undef mpegSurroundDecoder_ConfigureQmfDomain
#undef mpegSurroundDecoder_ParseNoHeader
#undef mpegSurroundDecoder_Parse
#undef mpegSurroundDecoder_Apply
#undef mpegSurroundDecoder_Close
#undef mpegSurroundDecoder_FreeMem
#undef mpegSurroundDecoder_SetParam
#undef mpegSurroundDecoder_GetLibInfo
#undef mpegSurroundDecoder_GetDelay
#undef mpegSurroundDecoder_IsPseudoLR
#undef checkTimeSlots

/* The dumps of shim/sacdec_sac_dec.cpp. */
INT sacshim_dump_self(spatialDec *s, INT *out);
INT sacshim_dump_frame(SPATIAL_BS_FRAME *f, BS_LL_STATE *ll, INT *out);
INT sacshim_dump_qd(const FDK_QMF_DOMAIN *qd, INT *out);
void sacshim_load_gc(FDK_QMF_DOMAIN_GC *g, const INT *v);

/* The size of the AAC decoder's workBufferCore2 (aac_ram.cpp), the input of
   mpegSurroundDecoder_Apply(). */
#define SACX_INPUT_SIZE ((8) * 1024)
/* The size of the arrays channelType and channelIndices of the AAC decoder. */
#define SACX_CH (8)
/* The offset of the padding byte of SPATIAL_SPECIFIC_CONFIG (its value is not compared). */
#define SSC_PAD_OFFSET 171

struct LW {
  INT *o;
  INT n;
  void put(INT v) { o[n++] = v; }
  template <typename T>
  void arr(const T *p, INT cnt) {
    for (INT i = 0; i < cnt; i++) o[n++] = (INT)p[i];
  }
};

static INT *scratch(int k) {
  static thread_local INT *s[2] = {NULL, NULL};
  if (s[k] == NULL) s[k] = (INT *)malloc((1 << 21) * sizeof(INT));
  return s[k];
}

static void put_ssc(LW &w, const SPATIAL_SPECIFIC_CONFIG *ssc) {
  UCHAR b[sizeof(SPATIAL_SPECIFIC_CONFIG)];
  FDKmemcpy(b, ssc, sizeof(b));
  b[SSC_PAD_OFFSET] = 0;
  w.arr(b, (INT)sizeof(b));
}

/* The complete state of a decoder instance (see dump_lib() of the test). */
static INT dump_lib(CMpegSurroundDecoder *d, INT *out) {
  LW w = {out, 0};
  if (d == NULL) {
    w.put(0);
    return w.n;
  }
  w.put(1);
  w.arr(d->mpsData, MPS_DATA_BUFFER_SIZE);
  w.put(d->mpsDataBits);
  put_ssc(w, &d->spatialSpecificConfig[0]);
  put_ssc(w, &d->spatialSpecificConfigBackup);
  w.put(d->mpegSurroundDecoderLevel);
  w.put(d->mpegSurroundSscIsGlobalCfg);
  w.put(d->mpegSurroundUseTimeInterface);
  w.n += sacshim_dump_frame(&d->bsFrames[0], &d->llState, w.o + w.n);
  w.put(d->bsFrameParse);
  w.put(d->bsFrameDecode);
  w.put(d->bsFrameDelay);
  const SPATIALDEC_PARAM *p = &d->mpegSurroundUserParams;
  w.put(p->outputMode);
  w.put(p->blindEnable);
  w.put(p->bypassMode);
  w.put(p->concealMethod);
  w.put((INT)p->concealNumKeepFrames);
  w.put((INT)p->concealFadeOutSlopeLength);
  w.put((INT)p->concealFadeInSlopeLength);
  w.put((INT)p->concealNumReleaseFrames);
  w.put(d->upmixType);
  w.put(d->initFlags[0]);
  w.put(d->ancStartStopPrev);
  w.put(d->fOnSync[0]);
  w.put(d->decConfig.decoderLevel);
  w.put(d->decConfig.decoderMode);
  w.put(d->decConfig.maxNumOutputChannels);
  w.put(d->decConfig.binauralMode);
  w.put(d->pSpatialDec != NULL);
  if (d->pSpatialDec != NULL) {
    const SPATIAL_SPECIFIC_CONFIG *c = d->pSpatialDec->pConfigCurrent;
    w.put(c == NULL ? 0 : (c == &d->spatialSpecificConfig[0] ? 1 : -1));
    w.n += sacshim_dump_self(d->pSpatialDec, w.o + w.n);
  }
  return w.n;
}

/* --- the log of the thread ----------------------------------------------------------------- */

struct SacLog {
  INT *d;
  INT n, cap;
  int on;
};
static thread_local SacLog g_log = {NULL, 0, 0, 0};

static void log_reserve(INT more) {
  if (g_log.n + more > g_log.cap) {
    INT cap = g_log.cap ? g_log.cap : (1 << 16);
    while (g_log.n + more > cap) cap *= 2;
    g_log.d = (INT *)realloc(g_log.d, (size_t)cap * sizeof(INT));
    g_log.cap = cap;
  }
}
static void log_begin(INT kind, INT narr) {
  log_reserve(3);
  g_log.d[g_log.n++] = 0x5ACD;
  g_log.d[g_log.n++] = kind;
  g_log.d[g_log.n++] = narr;
}
template <typename T>
static void log_arr(const T *p, INT n) {
  log_reserve(n + 1);
  g_log.d[g_log.n++] = n;
  for (INT i = 0; i < n; i++) g_log.d[g_log.n++] = (INT)p[i];
}
static void log_vals(INT n, const INT *v) { log_arr(v, n); }
static void log_lib(CMpegSurroundDecoder *d) {
  INT *s = scratch(0);
  log_arr(s, dump_lib(d, s));
}
/* The QMF domain dump (an empty array for NULL). */
static void log_qd(const FDK_QMF_DOMAIN *qd) {
  if (qd == NULL) {
    log_arr((INT *)NULL, 0);
    return;
  }
  INT *s = scratch(1);
  log_arr(s, sacshim_dump_qd(qd, s));
}
/* CacheWord, BitsInCache, ValidBits, ReadOffset, WriteOffset, BitNdx, bufSize, bufBits,
   ConfigCache */
static void log_bs_state(HANDLE_FDK_BITSTREAM bs) {
  INT s[9] = {0};
  if (bs != NULL) {
    s[0] = (INT)bs->CacheWord;
    s[1] = (INT)bs->BitsInCache;
    s[2] = (INT)bs->hBitBuf.ValidBits;
    s[3] = (INT)bs->hBitBuf.ReadOffset;
    s[4] = (INT)bs->hBitBuf.WriteOffset;
    s[5] = (INT)bs->hBitBuf.BitNdx;
    s[6] = (INT)bs->hBitBuf.bufSize;
    s[7] = (INT)bs->hBitBuf.bufBits;
    s[8] = (INT)bs->ConfigCache;
  }
  log_arr(s, 9);
}
static void log_bs_bytes(HANDLE_FDK_BITSTREAM bs) {
  if (bs == NULL) {
    log_arr((UCHAR *)NULL, 0);
    return;
  }
  log_arr(bs->hBitBuf.Buffer, (INT)bs->hBitBuf.bufSize);
}

/* The QMF domain of the decoder (pQmfDomain; NULL before the open). */
static FDK_QMF_DOMAIN *qd_of(CMpegSurroundDecoder *d) { return d == NULL ? NULL : d->pQmfDomain; }

enum {
  K_ISFULL = 1,
  K_OPEN = 2,
  K_CONFIG = 3,
  K_CONFQMF = 4,
  K_PARSE = 5,
  K_PARSENH = 6,
  K_APPLY = 7,
  K_SETPARAM = 8,
  K_GETDELAY = 9,
  K_FREEMEM = 10,
  K_CLOSE = 11,
  K_PSEUDOLR = 12,
  K_INIT = 13
};

/* --- capture functions (called by the AAC decoder copy of sacdec_sac_dec_lib_e2e.cpp) ----- */

extern "C" {

SAC_INSTANCE_AVAIL sacx_mps_IsFull(CMpegSurroundDecoder *d) {
  SAC_INSTANCE_AVAIL r = ref_lib_IsFull(d);
  if (g_log.on) {
    INT v[1] = {(INT)r};
    log_begin(K_ISFULL, 1);
    log_vals(1, v);
  }
  return r;
}

SACDEC_ERROR sacx_mps_Open(CMpegSurroundDecoder **pd, int stereoConfigIndex,
                           HANDLE_FDK_QMF_DOMAIN pQmfDomain) {
  INT in[2] = {*pd != NULL, stereoConfigIndex};
  SACDEC_ERROR r = ref_lib_Open(pd, stereoConfigIndex, pQmfDomain);
  if (g_log.on) {
    INT out[1] = {(INT)r};
    log_begin(K_OPEN, 3);
    log_vals(2, in);
    log_vals(1, out);
    log_lib(*pd);
  }
  return r;
}

SACDEC_ERROR sacx_mps_Init(CMpegSurroundDecoder *d) {
  if (g_log.on) {
    log_begin(K_INIT, 4);
    log_qd(qd_of(d));
  }
  SACDEC_ERROR r = ref_lib_Init(d);
  if (g_log.on) {
    INT out[1] = {(INT)r};
    log_vals(1, out);
    log_lib(d);
    log_qd(qd_of(d));
  }
  return r;
}

SACDEC_ERROR sacx_mps_Config(CMpegSurroundDecoder *d, HANDLE_FDK_BITSTREAM hBs,
                             AUDIO_OBJECT_TYPE coreCodec, INT samplingRate, INT frameSize,
                             INT numChannels, INT stereoConfigIndex,
                             INT coreSbrFrameLengthIndex, INT configBytes,
                             const UCHAR configMode, UCHAR *configChanged) {
  if (g_log.on) {
    INT in[10] = {(INT)coreCodec, samplingRate, frameSize, numChannels, stereoConfigIndex,
                  coreSbrFrameLengthIndex, configBytes, configMode, *configChanged, 0};
    log_begin(K_CONFIG, 8);
    log_vals(10, in);
    log_bs_state(hBs);
    log_bs_bytes(hBs);
    log_qd(qd_of(d));
  }
  SACDEC_ERROR r =
      ref_lib_Config(d, hBs, coreCodec, samplingRate, frameSize, numChannels, stereoConfigIndex,
                     coreSbrFrameLengthIndex, configBytes, configMode, configChanged);
  if (g_log.on) {
    INT out[2] = {(INT)r, *configChanged};
    log_vals(2, out);
    log_bs_state(hBs);
    log_lib(d);
    log_qd(qd_of(d));
  }
  return r;
}

SACDEC_ERROR sacx_mps_ConfigureQmfDomain(CMpegSurroundDecoder *d,
                                         SAC_INPUT_CONFIG sac_dec_interface,
                                         UINT coreSamplingRate, AUDIO_OBJECT_TYPE coreCodec) {
  if (g_log.on) {
    INT in[3] = {(INT)sac_dec_interface, (INT)coreSamplingRate, (INT)coreCodec};
    log_begin(K_CONFQMF, 5);
    log_vals(3, in);
    log_qd(qd_of(d));
  }
  SACDEC_ERROR r = ref_lib_ConfigureQmfDomain(d, sac_dec_interface, coreSamplingRate, coreCodec);
  if (g_log.on) {
    INT out[1] = {(INT)r};
    log_vals(1, out);
    log_lib(d);
    log_qd(qd_of(d));
  }
  return r;
}

int sacx_mps_ParseNoHeader(CMpegSurroundDecoder *d, HANDLE_FDK_BITSTREAM hBs, int *pMpsDataBits,
                           int fGlobalIndependencyFlag) {
  if (g_log.on) {
    INT in[2] = {*pMpsDataBits, fGlobalIndependencyFlag};
    log_begin(K_PARSENH, 6);
    log_vals(2, in);
    log_bs_state(hBs);
    log_bs_bytes(hBs);
  }
  int r = ref_lib_ParseNoHeader(d, hBs, pMpsDataBits, fGlobalIndependencyFlag);
  if (g_log.on) {
    INT out[2] = {r, *pMpsDataBits};
    log_vals(2, out);
    log_bs_state(hBs);
    log_lib(d);
  }
  return r;
}

int sacx_mps_Parse(CMpegSurroundDecoder *d, HANDLE_FDK_BITSTREAM hBs, int *pMpsDataBits,
                   AUDIO_OBJECT_TYPE coreCodec, int sampleRate, int frameSize,
                   int fGlobalIndependencyFlag) {
  if (g_log.on) {
    INT in[5] = {*pMpsDataBits, (INT)coreCodec, sampleRate, frameSize, fGlobalIndependencyFlag};
    log_begin(K_PARSE, 6);
    log_vals(5, in);
    log_bs_state(hBs);
    log_bs_bytes(hBs);
  }
  int r = ref_lib_Parse(d, hBs, pMpsDataBits, coreCodec, sampleRate, frameSize,
                        fGlobalIndependencyFlag);
  if (g_log.on) {
    INT out[2] = {r, *pMpsDataBits};
    log_vals(2, out);
    log_bs_state(hBs);
    log_lib(d);
  }
  return r;
}

int sacx_mps_Apply(CMpegSurroundDecoder *d, PCM_MPS *input, PCM_MPS *pTimeData,
                   const int timeDataSize, int timeDataFrameSize, int *nChannels, int *frameSize,
                   int sampleRate, AUDIO_OBJECT_TYPE coreCodec,
                   AUDIO_CHANNEL_TYPE channelType[], UCHAR channelIndices[],
                   const FDK_channelMapDescr *const mapDescr, const INT inDataHeadroom,
                   INT *outDataHeadroom) {
  if (g_log.on) {
    INT in[11] = {timeDataSize,
                  timeDataFrameSize,
                  *nChannels,
                  *frameSize,
                  sampleRate,
                  (INT)coreCodec,
                  inDataHeadroom,
                  mapDescr == NULL ? -1 : (INT)mapDescr->fPassThrough,
                  mapDescr == NULL ? -1 : (INT)mapDescr->mapInfoTabLen,
                  channelType == NULL,
                  channelIndices == NULL};
    log_begin(K_APPLY, 13);
    log_vals(11, in);
    log_arr(input, input == NULL ? 0 : SACX_INPUT_SIZE);
    log_arr(pTimeData, timeDataSize);
    log_arr(channelType, channelType == NULL ? 0 : SACX_CH);
    log_arr(channelIndices, channelIndices == NULL ? 0 : SACX_CH);
    log_qd(qd_of(d));
  }
  int r = ref_lib_Apply(d, input, pTimeData, timeDataSize, timeDataFrameSize, nChannels,
                        frameSize, sampleRate, coreCodec, channelType, channelIndices, mapDescr,
                        inDataHeadroom, outDataHeadroom);
  if (g_log.on) {
    INT out[4] = {r, *nChannels, *frameSize, *outDataHeadroom};
    log_vals(4, out);
    log_arr(input, input == NULL ? 0 : SACX_INPUT_SIZE);
    log_arr(pTimeData, timeDataSize);
    log_arr(channelType, channelType == NULL ? 0 : SACX_CH);
    log_arr(channelIndices, channelIndices == NULL ? 0 : SACX_CH);
    log_lib(d);
    log_qd(qd_of(d));
  }
  return r;
}

void sacx_mps_Close(CMpegSurroundDecoder *d) {
  if (g_log.on) log_begin(K_CLOSE, 0);
  ref_lib_Close(d);
}

SACDEC_ERROR sacx_mps_FreeMem(CMpegSurroundDecoder *d) {
  SACDEC_ERROR r = ref_lib_FreeMem(d);
  if (g_log.on) {
    INT out[1] = {(INT)r};
    log_begin(K_FREEMEM, 2);
    log_vals(1, out);
    log_lib(d);
  }
  return r;
}

SACDEC_ERROR sacx_mps_SetParam(CMpegSurroundDecoder *d, const SACDEC_PARAM param,
                               const INT value) {
  SACDEC_ERROR r = ref_lib_SetParam(d, param, value);
  if (g_log.on) {
    INT in[3] = {(INT)param, value, d != NULL};
    INT out[1] = {(INT)r};
    log_begin(K_SETPARAM, 3);
    log_vals(3, in);
    log_vals(1, out);
    log_lib(d);
  }
  return r;
}

int sacx_mps_GetLibInfo(LIB_INFO *libInfo) { return ref_lib_GetLibInfo(libInfo); }

UINT sacx_mps_GetDelay(const CMpegSurroundDecoder *d) {
  UINT r = ref_lib_GetDelay(d);
  if (g_log.on) {
    INT out[2] = {(INT)r, d != NULL};
    log_begin(K_GETDELAY, 1);
    log_vals(2, out);
  }
  return r;
}

SACDEC_ERROR sacx_mps_IsPseudoLR(CMpegSurroundDecoder *d, int *bsPseudoLr) {
  SACDEC_ERROR r = ref_lib_IsPseudoLR(d, bsPseudoLr);
  if (g_log.on) {
    INT out[3] = {(INT)r, *bsPseudoLr, d != NULL};
    log_begin(K_PSEUDOLR, 1);
    log_vals(3, out);
  }
  return r;
}

/* --- the log ---------------------------------------------------------------------------- */

void cref_saclib_capture(INT on) { g_log.on = on; }
INT cref_saclib_log_len(void) { return g_log.n; }
void cref_saclib_log_read(INT *out) {
  if (g_log.n > 0) FDKmemcpy(out, g_log.d, (size_t)g_log.n * sizeof(INT));
}
void cref_saclib_log_clear(void) { g_log.n = 0; }

/* --- a decoder instance of the API tests (with its QMF domain) --------------------------- */

struct LibInst {
  CMpegSurroundDecoder *d;
  FDK_QMF_DOMAIN qd;
};

void *cref_saclib_new(void) { return calloc(1, sizeof(LibInst)); }
void cref_saclib_free(void *h) {
  LibInst *x = (LibInst *)h;
  if (x->d != NULL) sacx_mps_Close(x->d);
  FDK_QmfDomain_Close(&x->qd);
  free(x);
}
INT cref_saclib_open(void *h, INT stereoConfigIndex) {
  LibInst *x = (LibInst *)h;
  return sacx_mps_Open(&x->d, stereoConfigIndex, &x->qd);
}
INT cref_saclib_is_full(void *h) { return sacx_mps_IsFull(((LibInst *)h)->d); }
INT cref_saclib_config(void *h, UCHAR *buf, UINT bufSize, UINT validBits, INT coreCodec, INT samplingRate, INT frameSize, INT numChannels, INT stereoConfigIndex, INT coreSbrFrameLengthIndex, INT configBytes, INT configMode, INT configChanged) {
  LibInst *x = (LibInst *)h;
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, validBits, BS_READER);
  UCHAR cc = (UCHAR)configChanged;
  return sacx_mps_Config(x->d, &bs, (AUDIO_OBJECT_TYPE)coreCodec, samplingRate, frameSize,
                         numChannels, stereoConfigIndex, coreSbrFrameLengthIndex, configBytes,
                         (UCHAR)configMode, &cc);
}
INT cref_saclib_init(void *h) { return sacx_mps_Init(((LibInst *)h)->d); }
/* ConfigureQmfDomain, then FDK_QmfDomain_Configure() as the AAC decoder does (with the time
   slots of the core frame); returns the result of ConfigureQmfDomain, or 1000 + the error
   of FDK_QmfDomain_Configure(). */
INT cref_saclib_configure_qmf_domain(void *h, INT sacInterface, UINT coreSamplingRate, INT coreCodec, INT coreFrameSize) {
  LibInst *x = (LibInst *)h;
  INT r = sacx_mps_ConfigureQmfDomain(x->d, (SAC_INPUT_CONFIG)sacInterface, coreSamplingRate,
                                      (AUDIO_OBJECT_TYPE)coreCodec);
  FDK_QMF_DOMAIN_GC *g = &x->qd.globalConf;
  g->nQmfTimeSlots_requested =
      g->nBandsAnalysis_requested > 0 ? coreFrameSize / g->nBandsAnalysis_requested : 0;
  if (g->nQmfProcChannels_requested == 0) g->nQmfProcChannels_requested = 1;
  if (g->nQmfProcBands_requested == 0) g->nQmfProcBands_requested = 64;
  if (g->nOutputChannels_requested == 0) g->nOutputChannels_requested = 2;
  if (g->nInputChannels_requested == 0) g->nInputChannels_requested = 1;
  INT qe = FDK_QmfDomain_Configure(&x->qd);
  return r != MPS_OK ? r : (qe != QMF_DOMAIN_OK ? 1000 + qe : 0);
}
/* Configures the QMF domain explicitly (as the AAC decoder does for USAC): gc as in
   sacdec_sac_dec.cpp (21 values); returns the result of FDK_QmfDomain_Configure(). */
INT cref_saclib_configure_qd(void *h, const INT *gc) {
  LibInst *x = (LibInst *)h;
  sacshim_load_gc(&x->qd.globalConf, gc);
  return FDK_QmfDomain_Configure(&x->qd);
}
/* The QMF domain as the dump of sacdec_sac_dec.cpp. */
INT cref_saclib_dump_qd(void *h, INT *out) { return sacshim_dump_qd(&((LibInst *)h)->qd, out); }
INT cref_saclib_parse(void *h, UCHAR *buf, UINT bufSize, UINT validBits, INT *mpsDataBits, INT coreCodec, INT sampleRate, INT frameSize, INT fGlobalIndependencyFlag) {
  LibInst *x = (LibInst *)h;
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, validBits, BS_READER);
  return sacx_mps_Parse(x->d, &bs, mpsDataBits, (AUDIO_OBJECT_TYPE)coreCodec, sampleRate,
                        frameSize, fGlobalIndependencyFlag);
}
INT cref_saclib_parse_no_header(void *h, UCHAR *buf, UINT bufSize, UINT validBits, INT *mpsDataBits, INT fGlobalIndependencyFlag) {
  LibInst *x = (LibInst *)h;
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, validBits, BS_READER);
  return sacx_mps_ParseNoHeader(x->d, &bs, mpsDataBits, fGlobalIndependencyFlag);
}
/* io: nChannels, frameSize (in/out), outDataHeadroom (out); channelType/channelIndices: 8. */
INT cref_saclib_apply(void *h, INT *input, INT *pTimeData, INT timeDataSize, INT timeDataFrameSize, INT *io, INT sampleRate, INT coreCodec, INT *channelType, UCHAR *channelIndices, INT passThrough, INT inDataHeadroom) {
  LibInst *x = (LibInst *)h;
  FDK_channelMapDescr md;
  FDK_chMapDescr_init(&md, NULL, 0, passThrough);
  AUDIO_CHANNEL_TYPE t[SACX_CH];
  for (int i = 0; i < SACX_CH; i++) t[i] = (AUDIO_CHANNEL_TYPE)channelType[i];
  INT r = sacx_mps_Apply(x->d, (PCM_MPS *)input, (PCM_MPS *)pTimeData, timeDataSize,
                         timeDataFrameSize, &io[0], &io[1], sampleRate,
                         (AUDIO_OBJECT_TYPE)coreCodec, t, channelIndices, &md, inDataHeadroom,
                         &io[2]);
  for (int i = 0; i < SACX_CH; i++) channelType[i] = t[i];
  return r;
}
INT cref_saclib_set_param(void *h, INT param, INT value, INT nullHandle) {
  LibInst *x = (LibInst *)h;
  return sacx_mps_SetParam(nullHandle ? NULL : x->d, (SACDEC_PARAM)param, value);
}
UINT cref_saclib_get_delay(void *h) { return sacx_mps_GetDelay(((LibInst *)h)->d); }
INT cref_saclib_free_mem(void *h) { return sacx_mps_FreeMem(((LibInst *)h)->d); }
INT cref_saclib_is_pseudo_lr(void *h, INT *out) { return sacx_mps_IsPseudoLR(((LibInst *)h)->d, out); }
/* Closes the decoder (the instance keeps its QMF domain). */
void cref_saclib_close(void *h) {
  LibInst *x = (LibInst *)h;
  sacx_mps_Close(x->d);
  x->d = NULL;
}
INT cref_saclib_dump(void *h, INT *out) { return dump_lib(((LibInst *)h)->d, out); }

/* --- functions without an instance ------------------------------------------------------- */

/* mpegSurroundDecoder_GetNrOfQmfBands() for USAC with stereoConfigIndex 3 (the function-local
   table mapIdx2QmfBands[coreSbrFrameLengthIndex - 2]). */
INT cref_saclib_nr_of_qmf_bands_sci3(INT coreSbrFrameLengthIndex) {
  SPATIAL_SPECIFIC_CONFIG ssc;
  FDKmemclear(&ssc, sizeof(ssc));
  ssc.coreCodec = AOT_USAC;
  ssc.stereoConfigIndex = 3;
  ssc.coreSbrFrameLengthIndex = coreSbrFrameLengthIndex;
  return mpegSurroundDecoder_GetNrOfQmfBands(&ssc, 48000);
}
/* The function-local decConfig of mpegSurroundDecoder_Create(), as a fully opened decoder holds it:
   decoderLevel, decoderMode, maxNumOutputChannels, binauralMode. */
void cref_saclib_dec_config(INT *out) {
  CMpegSurroundDecoder *d = NULL;
  FDK_QMF_DOMAIN qd;
  FDKmemclear(&qd, sizeof(qd));
  ref_lib_Open(&d, -1, &qd); /* the structure */
  ref_lib_Open(&d, -1, &qd); /* the spatial decoder and decConfig */
  out[0] = d->decConfig.decoderLevel;
  out[1] = d->decConfig.decoderMode;
  out[2] = d->decConfig.maxNumOutputChannels;
  out[3] = d->decConfig.binauralMode;
  ref_lib_Close(d);
}

INT cref_saclib_check_time_slots(INT frameLength, INT qmfBands, INT timeSlots) {
  return ref_lib_checkTimeSlots(frameLength, qmfBands, timeSlots);
}
/* LIB_INFO of the entries after `used` filled ones (FDK_MODULE_LAST entries): returns the
   result; out per entry: module_id, version, flags, then 3 strings of 32 bytes (title, build
   date, build time; "" for NULL). */
INT cref_saclib_get_lib_info(INT used, INT nullInfo, INT *out, UCHAR *strs) {
  LIB_INFO info[FDK_MODULE_LAST];
  FDKmemclear(info, sizeof(info));
  FDKinitLibInfo(info);
  for (int i = 0; i < used && i < FDK_MODULE_LAST; i++) info[i].module_id = (FDK_MODULE_ID)(100 + i);
  INT r = ref_lib_GetLibInfo(nullInfo ? NULL : info);
  for (int i = 0; i < FDK_MODULE_LAST; i++) {
    out[3 * i] = info[i].module_id;
    out[3 * i + 1] = info[i].version;
    out[3 * i + 2] = (INT)info[i].flags;
    const char *s[3] = {info[i].title, info[i].build_date, info[i].build_time};
    for (int k = 0; k < 3; k++) {
      UCHAR *dst = strs + (3 * i + k) * 32;
      FDKmemclear(dst, 32);
      if (s[k] != NULL) strncpy((char *)dst, s[k], 31);
    }
  }
  return r;
}
}
