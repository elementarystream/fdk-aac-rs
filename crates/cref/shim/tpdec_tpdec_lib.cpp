// Shims for the differential tests of fdk_aac_rs_tpdec::tpdec_lib.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::tpdec_tpdec_lib (see build.rs).
//
// The transport decoder of the reference is driven through its API (the functions of the
// library). Its internal state is read through a copy of libMpegTPDec included into a namespace
// (same definition of struct TRANSPORTDEC, same layout; the copy itself is not called). The
// whole library is copied because struct TRANSPORTDEC and HANDLE_TRANSPORTDEC must both live in
// the namespace. The callbacks are the recorders of tpdec_tp_data.cpp.
//
// `cref_tplib_encode()` produces test streams with the reference encoder (synthetic signal), so
// that no test material is needed.

#include "cref_common.h"
#include <stdlib.h>
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "FDK_crc.h"
#include "common_fix.h"
#include "tp_data.h"
#include "aacenc_lib.h"

namespace ref_tplib {
#include "tpdec_lib.h"
#include "libMpegTPDec/src/tp_version.h"
#include "libMpegTPDec/src/tpdec_adts.h"
#include "libMpegTPDec/src/tpdec_adif.h"
#include "libMpegTPDec/src/tpdec_latm.h"
#include "libMpegTPDec/src/tpdec_drm.h"
#include "libMpegTPDec/src/tpdec_lib.cpp"
#include "libMpegTPDec/src/tpdec_asc.cpp"
#include "libMpegTPDec/src/tpdec_adts.cpp"
#include "libMpegTPDec/src/tpdec_adif.cpp"
#include "libMpegTPDec/src/tpdec_latm.cpp"
#include "libMpegTPDec/src/tpdec_drm.cpp"
}

/* The global declarations of the library API (the guard was set inside the namespace). */
#undef TPDEC_LIB_H
#include "tpdec_lib.h"

/* tpdec_tp_data.cpp */
INT tpdec_asc_dump(const CSAudioSpecificConfig *a, INT *out);
UINT tpdec_hash(const INT *v, INT n);
void tpdec_rec_register(void *rec, HANDLE_TRANSPORTDEC hTp, INT has);

static ref_tplib::TRANSPORTDEC *in(void *h) { return (ref_tplib::TRANSPORTDEC *)h; }

static void crc_store(const FDK_CRCINFO *c, INT *out, INT *n) {
  for (int i = 0; i < 3; i++) {
    out[(*n)++] = c->crcRegData[i].isActive;
    out[(*n)++] = c->crcRegData[i].maxBits;
    out[(*n)++] = c->crcRegData[i].bitBufCntBits;
    out[(*n)++] = c->crcRegData[i].validBits;
  }
  out[(*n)++] = (INT)c->regStart;
  out[(*n)++] = (INT)c->regStop;
  out[(*n)++] = c->crcValue;
  out[(*n)++] = c->crcPoly;
  out[(*n)++] = c->crcMask;
  out[(*n)++] = c->startValue;
  out[(*n)++] = c->crcLen;
}

extern "C" {
/* transportDec_Open(); registers the callbacks of `rec` (if not NULL) with `has`. */
void *cref_tplib_open(INT fmt, UINT flags, UINT nrOfLayers, void *rec, INT has) {
  HANDLE_TRANSPORTDEC h = transportDec_Open((TRANSPORT_TYPE)fmt, flags, nrOfLayers);
  if (h != NULL && rec != NULL) tpdec_rec_register(rec, h, has);
  return h;
}
void cref_tplib_close(void *h) {
  HANDLE_TRANSPORTDEC t = (HANDLE_TRANSPORTDEC)h;
  transportDec_Close(&t);
}
INT cref_tplib_fill(void *h, const UCHAR *buf, UINT size, UINT *bytesValid, INT layer) {
  return transportDec_FillData((HANDLE_TRANSPORTDEC)h, (UCHAR *)buf, size, bytesValid, layer);
}
INT cref_tplib_read_au(void *h, UINT layer) {
  return transportDec_ReadAccessUnit((HANDLE_TRANSPORTDEC)h, layer);
}
INT cref_tplib_end_au(void *h) { return transportDec_EndAccessUnit((HANDLE_TRANSPORTDEC)h); }
INT cref_tplib_au_bits_remaining(void *h, UINT layer) {
  return transportDec_GetAuBitsRemaining((HANDLE_TRANSPORTDEC)h, layer);
}
INT cref_tplib_au_bits_total(void *h, UINT layer) {
  return transportDec_GetAuBitsTotal((HANDLE_TRANSPORTDEC)h, layer);
}
INT cref_tplib_buffer_fullness(void *h) {
  return transportDec_GetBufferFullness((HANDLE_TRANSPORTDEC)h);
}
INT cref_tplib_missing_au(void *h, INT *n) {
  return transportDec_GetMissingAccessUnitCount(n, (HANDLE_TRANSPORTDEC)h);
}
UINT cref_tplib_nr_sub_frames(void *h) {
  return transportDec_GetNrOfSubFrames((HANDLE_TRANSPORTDEC)h);
}
INT cref_tplib_get_format(void *h) { return transportDec_GetFormat((HANDLE_TRANSPORTDEC)h); }
INT cref_tplib_set_param(void *h, INT param, INT value) {
  return transportDec_SetParam((HANDLE_TRANSPORTDEC)h, (TPDEC_PARAM)param, value);
}
INT cref_tplib_crc_start_reg(void *h, INT mBits) {
  return transportDec_CrcStartReg((HANDLE_TRANSPORTDEC)h, mBits);
}
void cref_tplib_crc_end_reg(void *h, INT reg) {
  transportDec_CrcEndReg((HANDLE_TRANSPORTDEC)h, reg);
}
INT cref_tplib_crc_check(void *h) { return transportDec_CrcCheck((HANDLE_TRANSPORTDEC)h); }
INT cref_tplib_out_of_band_config(void *h, const UCHAR *conf, UINT length, UINT layer) {
  return transportDec_OutOfBandConfig((HANDLE_TRANSPORTDEC)h, (UCHAR *)conf, length, layer);
}
/* `cfg` must hold TP_USAC_MAX_CONFIG_LEN bytes. */
INT cref_tplib_in_band_config(void *h, UCHAR *cfg, UINT length, UCHAR buildUpStatus, UCHAR *configChanged, UINT layer, UCHAR *diff) {
  return transportDec_InBandConfig((HANDLE_TRANSPORTDEC)h, cfg, length, buildUpStatus,
                                   configChanged, layer, diff);
}
/* Reads n (0..32) bits from the bitstream of the decoder. */
UINT cref_tplib_bs_read(void *h, UINT n) {
  return FDKreadBits(transportDec_GetBitstream((HANDLE_TRANSPORTDEC)h, 0), n);
}
void cref_tplib_bs_push(void *h, INT n) {
  FDKpushBiDirectional(transportDec_GetBitstream((HANDLE_TRANSPORTDEC)h, 0), n);
}
INT cref_tplib_bs_valid_bits(void *h) {
  return (INT)FDKgetValidBits(transportDec_GetBitstream((HANDLE_TRANSPORTDEC)h, 0));
}
/* transportDec_GetAsc() as a dump. */
INT cref_tplib_get_asc(void *h, UINT layer, INT *dump, INT *dumpLen) {
  CSAudioSpecificConfig asc;
  INT r = transportDec_GetAsc((HANDLE_TRANSPORTDEC)h, layer, &asc);
  *dumpLen = tpdec_asc_dump(&asc, dump);
  return r;
}
/* The dump of hTp->asc[idx]. */
INT cref_tplib_asc(void *h, INT idx, INT *dump) { return tpdec_asc_dump(&in(h)->asc[idx], dump); }
/* Hash of the internal bitstream buffer. */
UINT cref_tplib_buf_hash(void *h) {
  INT v[(8192 * 4) / 4];
  FDKmemcpy(v, in(h)->bsBuffer, sizeof(v));
  return tpdec_hash(v, (8192 * 4) / 4);
}
/* The internal state (the parser member of the transport format only); returns its length. */
INT cref_tplib_state(void *h, INT *out) {
  ref_tplib::TRANSPORTDEC *t = in(h);
  INT n = 0;
  out[n++] = t->transportFmt;
  out[n++] = (INT)t->flags;
  out[n++] = (INT)t->globalFramePos;
  out[n++] = (INT)t->accessUnitAnchor[0];
  out[n++] = t->auLength[0];
  out[n++] = t->numberOfRawDataBlocks;
  out[n++] = (INT)t->avgBitRate;
  out[n++] = (INT)t->lastValidBufferFullness;
  out[n++] = t->remainder;
  out[n++] = t->missingAccessUnits;
  out[n++] = (INT)t->burstPeriod;
  out[n++] = (INT)t->holdOffFrames;
  out[n++] = t->targetLayout;
  const CCtrlCFGChange *c = &t->ctrlCFGChange[0];
  out[n++] = c->flushCnt;
  out[n++] = c->flushStatus;
  out[n++] = c->buildUpCnt;
  out[n++] = c->buildUpStatus;
  out[n++] = c->cfgChanged;
  out[n++] = c->contentChanged;
  out[n++] = c->forceCfgChange;
  const FDK_BITSTREAM *bs = &t->bitStream[0];
  out[n++] = (INT)bs->CacheWord;
  out[n++] = (INT)bs->BitsInCache;
  out[n++] = (INT)bs->ConfigCache;
  out[n++] = (INT)bs->hBitBuf.ValidBits;
  out[n++] = (INT)bs->hBitBuf.ReadOffset;
  out[n++] = (INT)bs->hBitBuf.WriteOffset;
  out[n++] = (INT)bs->hBitBuf.BitNdx;
  out[n++] = (INT)bs->hBitBuf.bufSize;
  switch (t->transportFmt) {
    case TT_MP4_ADTS: {
      const ref_tplib::STRUCT_ADTS *a = &t->parser.adts;
      out[n++] = a->bs.mpeg_id;
      out[n++] = a->bs.layer;
      out[n++] = a->bs.protection_absent;
      out[n++] = a->bs.profile;
      out[n++] = a->bs.sample_freq_index;
      out[n++] = a->bs.private_bit;
      out[n++] = a->bs.channel_config;
      out[n++] = a->bs.original;
      out[n++] = a->bs.home;
      out[n++] = a->bs.copyright_id;
      out[n++] = a->bs.copyright_start;
      out[n++] = a->bs.frame_length;
      out[n++] = a->bs.adts_fullness;
      out[n++] = a->bs.num_raw_blocks;
      out[n++] = a->bs.num_pce_bits;
      out[n++] = a->decoderCanDoMpeg4;
      out[n++] = a->BufferFullnesStartFlag;
      out[n++] = a->crcReadValue;
      for (int i = 0; i < 4; i++) out[n++] = a->rawDataBlockDist[i];
      crc_store(&a->crcInfo, out, &n);
    } break;
    case TT_MP4_ADIF: {
      const ref_tplib::CAdifHeader *a = &t->parser.adif;
      out[n++] = a->NumProgramConfigElements;
      out[n++] = (INT)a->BitRate;
      out[n++] = a->CopyrightIdPresent;
      out[n++] = a->OriginalCopy;
      out[n++] = a->Home;
      out[n++] = a->BitstreamType;
    } break;
    case TT_MP4_LATM_MCP0:
    case TT_MP4_LATM_MCP1:
    case TT_MP4_LOAS: {
      const ref_tplib::CLatmDemux *d = &t->parser.latm;
      out[n++] = (INT)d->m_linfo[0][0].m_frameLengthType;
      out[n++] = (INT)d->m_linfo[0][0].m_bufferFullness;
      out[n++] = (INT)d->m_linfo[0][0].m_streamID;
      out[n++] = (INT)d->m_linfo[0][0].m_frameLengthInBits;
      out[n++] = (INT)d->m_taraBufferFullness;
      out[n++] = (INT)d->m_otherDataLength;
      out[n++] = (INT)d->m_audioMuxLengthBytes;
      out[n++] = d->m_useSameStreamMux;
      out[n++] = d->m_AudioMuxVersion;
      out[n++] = d->m_AudioMuxVersionA;
      out[n++] = d->m_allStreamsSameTimeFraming;
      out[n++] = d->m_noSubFrames;
      out[n++] = d->m_numProgram;
      out[n++] = d->m_numLayer[0];
      out[n++] = d->m_otherDataPresent;
      out[n++] = d->m_crcCheckPresent;
      out[n++] = d->BufferFullnessAchieved;
      out[n++] = d->usacExplicitCfgChanged;
      out[n++] = d->applyAsc;
      out[n++] = d->newCfgHasAudioPreRoll;
    } break;
    case TT_DRM: {
      const ref_tplib::STRUCT_DRM *d = &t->parser.drm;
      crc_store(&d->crcInfo, out, &n);
      out[n++] = d->crcReadValue;
    } break;
    default:
      break;
  }
  return n;
}

/* transportDec_GetLibInfo() on an array of FDK_MODULE_LAST entries with the given module ids
   (null_info: NULL). Returns the error code; writes the module id, version, flags, the title
   (32 bytes, NUL terminated) and versionStr (32 bytes) of every entry. */
INT cref_tplib_get_lib_info(INT null_info, const INT *ids_in, INT *ids, INT *version, UINT *flags, char *title, char *vstr) {
  LIB_INFO info[FDK_MODULE_LAST];
  FDKmemclear(info, sizeof(info));
  for (int i = 0; i < FDK_MODULE_LAST; i++) {
    info[i].module_id = (FDK_MODULE_ID)ids_in[i];
    FDKmemset(info[i].versionStr, 'x', sizeof(info[i].versionStr));
  }
  INT r = transportDec_GetLibInfo(null_info ? NULL : info);
  for (int i = 0; i < FDK_MODULE_LAST; i++) {
    ids[i] = info[i].module_id;
    version[i] = info[i].version;
    flags[i] = info[i].flags;
    FDKmemclear(&title[32 * i], 32);
    if (info[i].title != NULL) {
      for (int k = 0; k < 31 && info[i].title[k] != 0; k++) title[32 * i + k] = info[i].title[k];
    }
    FDKmemcpy(&vstr[32 * i], info[i].versionStr, 32);
  }
  return r;
}
INT cref_tplib_drm_check(const UCHAR *conf, UINT length) {
  return transportDec_DrmRawSdcAudioConfig_Check((UCHAR *)conf, length);
}

/* Encodes `frames` frames of a synthetic signal with the reference encoder. `params` holds
   `nparams` pairs {AACENC_PARAM, value} (set after AACENC_SAMPLERATE and AACENC_CHANNELMODE).
   Writes the bitstream (outLen bytes), the size of every non-empty output (nAu entries) and the
   configuration of aacEncInfo (confLen bytes). Returns 0 or the first failing AACENC_ERROR. */
INT cref_tplib_encode(const INT *params, INT nparams, INT channels, INT sampleRate, INT frames, INT seed, UCHAR *out, INT outCap, INT *outLen, INT *auSizes, INT maxAu, INT *nAu, UCHAR *conf, INT *confLen) {
  HANDLE_AACENCODER enc = NULL;
  AACENC_ERROR e = aacEncOpen(&enc, 0, (UINT)channels);
  *outLen = 0;
  *nAu = 0;
  *confLen = 0;
  if (e != AACENC_OK) return e;
  static const CHANNEL_MODE modes[] = {MODE_INVALID, MODE_1, MODE_2, MODE_1_2,
                                       MODE_1_2_1, MODE_1_2_2, MODE_1_2_2_1};
  e = aacEncoder_SetParam(enc, AACENC_SAMPLERATE, (UINT)sampleRate);
  if (e == AACENC_OK) e = aacEncoder_SetParam(enc, AACENC_CHANNELMODE, modes[channels]);
  for (INT i = 0; i < nparams && e == AACENC_OK; i++) {
    e = aacEncoder_SetParam(enc, (AACENC_PARAM)params[2 * i], (UINT)params[2 * i + 1]);
  }
  if (e == AACENC_OK) e = aacEncEncode(enc, NULL, NULL, NULL, NULL);
  AACENC_InfoStruct info;
  if (e == AACENC_OK) e = aacEncInfo(enc, &info);
  if (e != AACENC_OK) {
    aacEncClose(&enc);
    return e;
  }
  FDKmemcpy(conf, info.confBuf, info.confSize);
  *confLen = (INT)info.confSize;

  INT frameLength = (INT)info.frameLength;
  INT total = frameLength * frames;
  INT_PCM *pcm = (INT_PCM *)calloc((size_t)frameLength * channels, sizeof(INT_PCM));
  UCHAR *obuf = (UCHAR *)calloc(20480, 1);
  UINT lcg = (UINT)seed * 2654435761u + 1;
  INT pos = 0;
  for (;;) {
    INT n = total - pos;
    if (n > frameLength) n = frameLength;
    for (INT i = 0; i < n; i++) {
      for (INT c = 0; c < channels; c++) {
        lcg = lcg * 1664525u + 1013904223u;
        INT t = pos + i;
        INT saw = ((t * (c + 3) * 37) & 0x3fff) - 0x2000;
        INT noise = (INT)(lcg >> 20) - 2048;
        pcm[i * channels + c] = (INT_PCM)(saw + noise);
      }
    }
    AACENC_BufDesc inDesc, outDesc;
    AACENC_InArgs inArgs;
    AACENC_OutArgs outArgs;
    void *inPtr = pcm, *outPtr = obuf;
    INT inId = IN_AUDIO_DATA, inSize = n * channels * (INT)sizeof(INT_PCM),
        inEl = (INT)sizeof(INT_PCM);
    INT outId = OUT_BITSTREAM_DATA, outSize = 20480, outEl = 1;
    inDesc.numBufs = 1;
    inDesc.bufs = &inPtr;
    inDesc.bufferIdentifiers = &inId;
    inDesc.bufSizes = &inSize;
    inDesc.bufElSizes = &inEl;
    outDesc.numBufs = 1;
    outDesc.bufs = &outPtr;
    outDesc.bufferIdentifiers = &outId;
    outDesc.bufSizes = &outSize;
    outDesc.bufElSizes = &outEl;
    FDKmemclear(&inArgs, sizeof(inArgs));
    FDKmemclear(&outArgs, sizeof(outArgs));
    inArgs.numInSamples = n > 0 ? n * channels : -1;
    e = aacEncEncode(enc, &inDesc, &outDesc, &inArgs, &outArgs);
    if (e != AACENC_OK) break;
    if (outArgs.numOutBytes > 0) {
      if (*outLen + outArgs.numOutBytes <= outCap && *nAu < maxAu) {
        FDKmemcpy(out + *outLen, obuf, outArgs.numOutBytes);
        *outLen += outArgs.numOutBytes;
        auSizes[(*nAu)++] = outArgs.numOutBytes;
      }
    }
    pos += outArgs.numInSamples / channels;
    if (n == 0 && outArgs.numOutBytes == 0) break;
  }
  free(pcm);
  free(obuf);
  aacEncClose(&enc);
  return (e == AACENC_ENCODE_EOF || e == AACENC_OK) ? 0 : e;
}
}
