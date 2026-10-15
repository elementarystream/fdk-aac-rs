// Shims for the differential tests of fdk_aac_rs_tpenc::tpenc_asc.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::tpenc_tpenc_asc (see build.rs).
//
// Besides the tpenc_asc shims, this file holds the pieces shared by all tpenc shims (declared
// again in the other tpenc_*.cpp files, which are linked into the same binary):
//
// - `TpEncRec`: recording transport encoder callbacks. Every call appends a record to a log;
//   the Rust tests implement the same behavior (tests/tpenc/tpenc_asc.rs, `Rec`):
//     sbr: 1, sampleRateIn, sampleRateOut, samplesPerFrame, coreCodec, elementID, elementIndex,
//          harmonicSbr, stereoConfigIndex, configMode, configChanged == NULL, downscaleFactor,
//          validBits; writes sbr_bits + elementIndex bits (pattern, seed + elementIndex);
//          returns 0
//     ssc: 2, hBs == NULL, coreCodec, samplingRate, frameSize, numChannels, stereoConfigIndex,
//          coreSbrFrameLengthIndex, configBytes, configMode, configChanged == NULL,
//          validBits or -1; writes ssc_bits bits (pattern, seed) if hBs != NULL; returns
//          ssc_bits
//     usac: 3, validBits; returns 0 (never called by the transport encoder)
// - `tpenc_pat()`: writes n pattern bits (an LCG, see below).
// - `CODER_CONFIG` as an int array (`tpenc_cc_from`, `tpenc_cc_to`): aot, extAOT, channelMode,
//   channelConfigZero, samplingRate, extSamplingRate, downscaleSamplingRate, bitRate,
//   samplesPerFrame, noChannels, bitsFrame, nSubFrames, BSACnumOfSubFrame, BSAClayerLength,
//   flags, matrixMixdownA, headerPeriod, stereoConfigIndex, sbrMode, sbrSignaling,
//   rawConfigBits, sbrPresent, psPresent (23 values) plus the 64 bytes of rawConfig.
// - Writer bitstreams on caller buffers (`cref_tpenc_bs_*`).

#include "cref_common.h"
#include <stdlib.h>
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "FDK_crc.h"
#include "tp_data.h"
#include "tpenc_lib.h"

/* tpenc_asc.h is included only inside the namespace: a global declaration of its functions
   would make the calls of the copy ambiguous (argument-dependent lookup). The shims call the
   copy, which is compiled from the same source with the same flags as the library. */
namespace ref_tpenc_asc {
#include "libMpegTPEnc/src/tpenc_asc.cpp"
}

/* ------------------------------------------------------------------------------------------ */
/* Shared helpers                                                                              */
/* ------------------------------------------------------------------------------------------ */

/* n pattern bits: x = x * 1103515245 + 12345 per chunk of up to 32 bits, value x >> 3. */
void tpenc_pat(HANDLE_FDK_BITSTREAM hBs, INT nbits, UINT seed) {
  UINT x = seed;
  while (nbits > 0) {
    INT n = nbits > 32 ? 32 : nbits;
    x = x * 1103515245u + 12345u;
    FDKwriteBits(hBs, (x >> 3) | (x << 29), n);
    nbits -= n;
  }
}

void tpenc_cc_from(CODER_CONFIG *cc, const INT *v, const UCHAR *raw) {
  FDKmemclear(cc, sizeof(*cc));
  cc->aot = (AUDIO_OBJECT_TYPE)v[0];
  cc->extAOT = (AUDIO_OBJECT_TYPE)v[1];
  cc->channelMode = (CHANNEL_MODE)v[2];
  cc->channelConfigZero = (UCHAR)v[3];
  cc->samplingRate = v[4];
  cc->extSamplingRate = v[5];
  cc->downscaleSamplingRate = v[6];
  cc->bitRate = v[7];
  cc->samplesPerFrame = v[8];
  cc->noChannels = v[9];
  cc->bitsFrame = v[10];
  cc->nSubFrames = v[11];
  cc->BSACnumOfSubFrame = v[12];
  cc->BSAClayerLength = v[13];
  cc->flags = (UINT)v[14];
  cc->matrixMixdownA = (UCHAR)v[15];
  cc->headerPeriod = (UCHAR)v[16];
  cc->stereoConfigIndex = (UCHAR)v[17];
  cc->sbrMode = (UCHAR)v[18];
  cc->sbrSignaling = (SBR_PS_SIGNALING)v[19];
  cc->rawConfigBits = v[20];
  cc->sbrPresent = (UCHAR)v[21];
  cc->psPresent = (UCHAR)v[22];
  FDKmemcpy(cc->rawConfig, raw, 64);
}

void tpenc_cc_to(const CODER_CONFIG *cc, INT *v, UCHAR *raw) {
  v[0] = cc->aot;
  v[1] = cc->extAOT;
  v[2] = cc->channelMode;
  v[3] = cc->channelConfigZero;
  v[4] = cc->samplingRate;
  v[5] = cc->extSamplingRate;
  v[6] = cc->downscaleSamplingRate;
  v[7] = cc->bitRate;
  v[8] = cc->samplesPerFrame;
  v[9] = cc->noChannels;
  v[10] = cc->bitsFrame;
  v[11] = cc->nSubFrames;
  v[12] = cc->BSACnumOfSubFrame;
  v[13] = cc->BSAClayerLength;
  v[14] = (INT)cc->flags;
  v[15] = cc->matrixMixdownA;
  v[16] = cc->headerPeriod;
  v[17] = cc->stereoConfigIndex;
  v[18] = cc->sbrMode;
  v[19] = cc->sbrSignaling;
  v[20] = cc->rawConfigBits;
  v[21] = cc->sbrPresent;
  v[22] = cc->psPresent;
  FDKmemcpy(raw, cc->rawConfig, 64);
}

void tpenc_bs_dump(FDK_BITSTREAM *bs, INT *out) {
  out[0] = (INT)bs->CacheWord;
  out[1] = (INT)bs->BitsInCache;
  out[2] = (INT)bs->hBitBuf.ValidBits;
  out[3] = (INT)bs->hBitBuf.ReadOffset;
  out[4] = (INT)bs->hBitBuf.WriteOffset;
  out[5] = (INT)bs->hBitBuf.BitNdx;
  out[6] = (INT)bs->hBitBuf.bufSize;
  out[7] = (INT)bs->ConfigCache;
}

struct TpEncRec {
  INT *log;
  INT n;
  INT cap;
  INT sbr_bits;
  INT ssc_bits;
  UINT seed;
};

static void rec_put(TpEncRec *r, INT v) {
  if (r->n < r->cap) r->log[r->n] = v;
  r->n++;
}

INT tpenc_rec_sbr(void *self, HANDLE_FDK_BITSTREAM hBs, const INT sampleRateIn,
                  const INT sampleRateOut, const INT samplesPerFrame,
                  const AUDIO_OBJECT_TYPE coreCodec, const MP4_ELEMENT_ID elementID,
                  const INT elementIndex, const UCHAR harmonicSbr,
                  const UCHAR stereoConfigIndex, const UCHAR configMode, UCHAR *configChanged,
                  const INT downscaleFactor) {
  TpEncRec *r = (TpEncRec *)self;
  rec_put(r, 1);
  rec_put(r, sampleRateIn);
  rec_put(r, sampleRateOut);
  rec_put(r, samplesPerFrame);
  rec_put(r, coreCodec);
  rec_put(r, elementID);
  rec_put(r, elementIndex);
  rec_put(r, harmonicSbr);
  rec_put(r, stereoConfigIndex);
  rec_put(r, configMode);
  rec_put(r, configChanged == NULL);
  rec_put(r, downscaleFactor);
  rec_put(r, (INT)FDKgetValidBits(hBs));
  tpenc_pat(hBs, r->sbr_bits + elementIndex, r->seed + (UINT)elementIndex);
  return 0;
}

INT tpenc_rec_ssc(void *self, HANDLE_FDK_BITSTREAM hBs, const AUDIO_OBJECT_TYPE coreCodec,
                  const INT samplingRate, const INT frameSize, const INT numChannels,
                  const INT stereoConfigIndex, const INT coreSbrFrameLengthIndex,
                  const INT configBytes, const UCHAR configMode, UCHAR *configChanged) {
  TpEncRec *r = (TpEncRec *)self;
  rec_put(r, 2);
  rec_put(r, hBs == NULL);
  rec_put(r, coreCodec);
  rec_put(r, samplingRate);
  rec_put(r, frameSize);
  rec_put(r, numChannels);
  rec_put(r, stereoConfigIndex);
  rec_put(r, coreSbrFrameLengthIndex);
  rec_put(r, configBytes);
  rec_put(r, configMode);
  rec_put(r, configChanged == NULL);
  rec_put(r, hBs == NULL ? -1 : (INT)FDKgetValidBits(hBs));
  if (hBs != NULL) tpenc_pat(hBs, r->ssc_bits, r->seed);
  return r->ssc_bits;
}

INT tpenc_rec_usac(void *self, HANDLE_FDK_BITSTREAM hBs) {
  TpEncRec *r = (TpEncRec *)self;
  rec_put(r, 3);
  rec_put(r, (INT)FDKgetValidBits(hBs));
  return 0;
}

/* CSTpCallBacks with the recorder: bit 0 of `has` cbSbr, bit 1 cbSsc, bit 2 cbUsac. */
void tpenc_cb_make(CSTpCallBacks *cb, void *rec, INT has) {
  FDKmemclear(cb, sizeof(*cb));
  if (has & 1) {
    cb->cbSbr = tpenc_rec_sbr;
    cb->cbSbrData = rec;
  }
  if (has & 2) {
    cb->cbSsc = tpenc_rec_ssc;
    cb->cbSscData = rec;
  }
  if (has & 4) {
    cb->cbUsac = tpenc_rec_usac;
    cb->cbUsacData = rec;
  }
}

extern "C" {
void *cref_tpenc_rec_new(INT sbr_bits, INT ssc_bits, UINT seed) {
  TpEncRec *r = (TpEncRec *)calloc(1, sizeof(TpEncRec));
  r->cap = 1 << 16;
  r->log = (INT *)calloc(r->cap, sizeof(INT));
  r->sbr_bits = sbr_bits;
  r->ssc_bits = ssc_bits;
  r->seed = seed;
  return r;
}
void cref_tpenc_rec_set(void *rec, INT sbr_bits, INT ssc_bits, UINT seed) {
  TpEncRec *r = (TpEncRec *)rec;
  r->sbr_bits = sbr_bits;
  r->ssc_bits = ssc_bits;
  r->seed = seed;
}
/* Copies the log (at most `max` entries) to `out`, returns its length and clears it. */
INT cref_tpenc_rec_take(void *rec, INT *out, INT max) {
  TpEncRec *r = (TpEncRec *)rec;
  INT n = r->n;
  for (INT i = 0; i < n && i < max && i < r->cap; i++) out[i] = r->log[i];
  r->n = 0;
  return n;
}
void cref_tpenc_rec_free(void *rec) {
  TpEncRec *r = (TpEncRec *)rec;
  free(r->log);
  free(r);
}

/* A writer bitstream on the caller's buffer (`size` bytes). */
void *cref_tpenc_bs_new(UCHAR *buf, UINT size) {
  FDK_BITSTREAM *bs = (FDK_BITSTREAM *)calloc(1, sizeof(FDK_BITSTREAM));
  FDKinitBitStream(bs, buf, size, 0, BS_WRITER);
  return bs;
}
void cref_tpenc_bs_free(void *bs) { free(bs); }
void cref_tpenc_bs_pat(void *bs, INT nbits, UINT seed) {
  tpenc_pat((HANDLE_FDK_BITSTREAM)bs, nbits, seed);
}
void cref_tpenc_bs_dump(void *bs, INT *out) { tpenc_bs_dump((FDK_BITSTREAM *)bs, out); }
UINT cref_tpenc_bs_valid_bits(void *bs) { return FDKgetValidBits((HANDLE_FDK_BITSTREAM)bs); }
void cref_tpenc_bs_sync(void *bs) { FDKsyncCache((HANDLE_FDK_BITSTREAM)bs); }

/* ---------------------------------------------------------------------------------------- */
/* tpenc_asc                                                                                 */
/* ---------------------------------------------------------------------------------------- */

INT cref_tpenc_asc_get_channel_config(INT mode, INT czero) {
  return ref_tpenc_asc::getChannelConfig((CHANNEL_MODE)mode, (UCHAR)czero);
}
INT cref_tpenc_asc_get_channel_mode(INT n) { return transportEnc_GetChannelMode(n); }
INT cref_tpenc_asc_get_pce_bits(INT mode, INT mmA, INT bits) {
  return ref_tpenc_asc::transportEnc_GetPCEBits((CHANNEL_MODE)mode, mmA, bits);
}
/* getPceEntry(): returns 0 for NULL, else 1 and nf, ns, nb, nl, the element types (up to 8,
   -9 padding) and the 6 height bytes (-1 for NULL) in out[0..18]. */
INT cref_tpenc_asc_get_pce_entry(INT mode, INT *out) {
  const ref_tpenc_asc::PCE_CONFIGURATION *c = ref_tpenc_asc::getPceEntry((CHANNEL_MODE)mode);
  if (c == NULL) return 0;
  out[0] = c->num_front_channel_elements;
  out[1] = c->num_side_channel_elements;
  out[2] = c->num_back_channel_elements;
  out[3] = c->num_lfe_channel_elements;
  INT n = out[0] + out[1] + out[2] + out[3];
  for (INT i = 0; i < 8; i++) out[4 + i] = i < n ? c->pEl_type[i] : -9;
  for (INT i = 0; i < 6; i++) {
    const UCHAR *h = (const UCHAR *)c->pHeight_num;
    out[12 + i] = (h == NULL) ? -1 : h[i];
  }
  return 1;
}
INT cref_tpenc_asc_write_pce(void *bs, INT mode, INT sr, INT tag, INT profile, INT mmA, INT ps, UINT anchor) {
  return ref_tpenc_asc::transportEnc_writePCE((HANDLE_FDK_BITSTREAM)bs, (CHANNEL_MODE)mode, sr,
                                              tag, profile, mmA, ps, anchor);
}
void cref_tpenc_asc_write_aot(void *bs, INT aot) {
  ref_tpenc_asc::writeAot((HANDLE_FDK_BITSTREAM)bs, (AUDIO_OBJECT_TYPE)aot);
}
void cref_tpenc_asc_write_sample_rate(void *bs, INT sr, INT nbits) {
  ref_tpenc_asc::writeSampleRate((HANDLE_FDK_BITSTREAM)bs, sr, nbits);
}
INT cref_tpenc_asc_write_ga(void *bs, INT *ccv, UCHAR *raw, INT extFlg, UINT anchor) {
  CODER_CONFIG cc;
  tpenc_cc_from(&cc, ccv, raw);
  INT r = ref_tpenc_asc::transportEnc_writeGASpecificConfig((HANDLE_FDK_BITSTREAM)bs, &cc,
                                                            extFlg, anchor);
  tpenc_cc_to(&cc, ccv, raw);
  return r;
}
INT cref_tpenc_asc_write_eld(void *bs, INT *ccv, UCHAR *raw, INT epConfig, void *rec, INT has) {
  CODER_CONFIG cc;
  CSTpCallBacks cb;
  tpenc_cc_from(&cc, ccv, raw);
  tpenc_cb_make(&cb, rec, has);
  INT r = ref_tpenc_asc::transportEnc_writeELDSpecificConfig((HANDLE_FDK_BITSTREAM)bs, &cc,
                                                             epConfig, &cb);
  tpenc_cc_to(&cc, ccv, raw);
  return r;
}
INT cref_tpenc_asc_write_usac(void *bs, INT *ccv, UCHAR *raw, INT extFlag, void *rec, INT has) {
  CODER_CONFIG cc;
  CSTpCallBacks cb;
  tpenc_cc_from(&cc, ccv, raw);
  tpenc_cb_make(&cb, rec, has);
  INT r = ref_tpenc_asc::transportEnc_writeUsacSpecificConfig((HANDLE_FDK_BITSTREAM)bs,
                                                              extFlag, &cc, &cb);
  tpenc_cc_to(&cc, ccv, raw);
  return r;
}
/* transportEnc_writeASC() of the library; `ccv`/`raw` receive the configuration afterwards. */
INT cref_tpenc_asc_write_asc(void *bs, INT *ccv, UCHAR *raw, void *rec, INT has) {
  CODER_CONFIG cc;
  CSTpCallBacks cb;
  tpenc_cc_from(&cc, ccv, raw);
  tpenc_cb_make(&cb, rec, has);
  INT r = transportEnc_writeASC((HANDLE_FDK_BITSTREAM)bs, &cc, &cb);
  tpenc_cc_to(&cc, ccv, raw);
  return r;
}
}

#define TPENC_ASC_TABLES(T)                                                                \
  T(elType_1) T(elType_2) T(elType_1_2) T(elType_1_2_1) T(elType_1_2_2) T(elType_1_2_2_1) \
  T(elType_1_2_2_2_1) T(elType_6_1) T(elType_7_1_back) T(elType_7_1_top_front)            \
  T(elType_7_1_rear_surround) T(elType_7_1_front_center)

extern "C" {
/* The file-static table `name` of the copy as memory: elType_1 ... elType_7_1_front_center and
   heightNum_7_1_top_front. Returns NULL for another name. */
const void *cref_tpenc_asc_table(const char *name, INT *bytes) {
#define T(x)                                 \
  if (strcmp(name, #x) == 0) {               \
    *bytes = (INT)sizeof(ref_tpenc_asc::x); \
    return ref_tpenc_asc::x;                 \
  }
  TPENC_ASC_TABLES(T)
#undef T
  if (strcmp(name, "heightNum_7_1_top_front") == 0) {
    *bytes = (INT)sizeof(ref_tpenc_asc::heightNum_7_1_top_front);
    return &ref_tpenc_asc::heightNum_7_1_top_front;
  }
  *bytes = 0;
  return NULL;
}

/* Entry i of pceConfigTab (the copy): out[0..6] = channel_mode, the numbers of front, side, back
   and LFE elements, pHeight_num (0: NULL, 1: &heightNum_7_1_top_front, -1: other); returns the
   name of the elType table pEl_type points to ("?" if unknown), NULL after the last entry. */
const char *cref_tpenc_asc_pce_config_tab(INT i, INT *out) {
  const INT n = (INT)(sizeof(ref_tpenc_asc::pceConfigTab) / sizeof(ref_tpenc_asc::pceConfigTab[0]));
  if (i < 0 || i >= n) return NULL;
  const ref_tpenc_asc::PCE_CONFIGURATION *c = &ref_tpenc_asc::pceConfigTab[i].pce_configuration;
  out[0] = ref_tpenc_asc::pceConfigTab[i].channel_mode;
  out[1] = c->num_front_channel_elements;
  out[2] = c->num_side_channel_elements;
  out[3] = c->num_back_channel_elements;
  out[4] = c->num_lfe_channel_elements;
  out[5] = c->pHeight_num == NULL ? 0 : (c->pHeight_num == &ref_tpenc_asc::heightNum_7_1_top_front ? 1 : -1);
#define T(x) \
  if (c->pEl_type == ref_tpenc_asc::x) return #x;
  TPENC_ASC_TABLES(T)
#undef T
  return "?";
}
}
