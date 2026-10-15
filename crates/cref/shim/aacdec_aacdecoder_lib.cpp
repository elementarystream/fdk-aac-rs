// Shims for the differential tests of fdk_aac_rs_aacdec::aacdecoder_lib.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_aacdecoder_lib (see build.rs).
//
// Thin wrappers around the public decoder API of the reference library (renamed to
// cref_aacDecoder_* by the prefix header), plus a stream generator built on the reference
// encoder (for transports and features the conformance material lacks: LATM, ADIF, CRC,
// ancillary data, DRC metadata).

#include "cref_common.h"
#include "aacdecoder_lib.h"
#include "aacenc_lib.h"
#include "libAACdec/src/aacdecoder.h"
#include <stdlib.h>

extern "C" {

void *cref_aacdecoder_lib_open(INT tt, UINT layers) {
  return (void *)aacDecoder_Open((TRANSPORT_TYPE)tt, layers);
}

void cref_aacdecoder_lib_close(void *h) { aacDecoder_Close((HANDLE_AACDECODER)h); }

INT cref_aacdecoder_lib_config_raw(void *h, UCHAR *conf, UINT len) {
  UCHAR *c[1] = {conf};
  UINT l[1] = {len};
  return aacDecoder_ConfigRaw((HANDLE_AACDECODER)h, c, l);
}

INT cref_aacdecoder_lib_fill(void *h, UCHAR *buf, UINT size, UINT *valid) {
  UCHAR *b[1] = {buf};
  UINT s[1] = {size};
  return aacDecoder_Fill((HANDLE_AACDECODER)h, b, s, valid);
}

INT cref_aacdecoder_lib_decode(void *h, SHORT *out, INT size, UINT flags) {
  return aacDecoder_DecodeFrame((HANDLE_AACDECODER)h, out, size, flags);
}

INT cref_aacdecoder_lib_set_param(void *h, INT param, INT value) {
  return aacDecoder_SetParam((HANDLE_AACDECODER)h, (AACDEC_PARAM)param, value);
}

INT cref_aacdecoder_lib_free_bytes(void *h, UINT *free_bytes) {
  return aacDecoder_GetFreeBytes((HANDLE_AACDECODER)h, free_bytes);
}

INT cref_aacdecoder_lib_raw_isobmff(void *h, UCHAR *buf, UINT len) {
  return aacDecoder_RawISOBMFFData((HANDLE_AACDECODER)h, buf, len);
}

/* The CStreamInfo as INT values in the order of the struct (INT64 as two INTs, low first;
   pChannelType/pChannelIndices as 8 values each). Returns the count, 0 for NULL. */
INT cref_aacdecoder_lib_stream_info(void *h, INT *o) {
  CStreamInfo *si = aacDecoder_GetStreamInfo((HANDLE_AACDECODER)h);
  INT n = 0, c;
  if (si == NULL) return 0;
  o[n++] = si->sampleRate;
  o[n++] = si->frameSize;
  o[n++] = si->numChannels;
  for (c = 0; c < 8; c++) o[n++] = si->pChannelType ? si->pChannelType[c] : -99;
  for (c = 0; c < 8; c++) o[n++] = si->pChannelIndices ? si->pChannelIndices[c] : -99;
  o[n++] = si->aacSampleRate;
  o[n++] = si->profile;
  o[n++] = si->aot;
  o[n++] = si->channelConfig;
  o[n++] = si->bitRate;
  o[n++] = si->aacSamplesPerFrame;
  o[n++] = si->aacNumChannels;
  o[n++] = si->extAot;
  o[n++] = si->extSamplingRate;
  o[n++] = (INT)si->outputDelay;
  o[n++] = (INT)si->flags;
  o[n++] = si->epConfig;
  o[n++] = si->numLostAccessUnits;
  o[n++] = (INT)si->numTotalBytes;
  o[n++] = (INT)(si->numTotalBytes >> 32);
  o[n++] = (INT)si->numBadBytes;
  o[n++] = (INT)(si->numBadBytes >> 32);
  o[n++] = (INT)si->numTotalAccessUnits;
  o[n++] = (INT)(si->numTotalAccessUnits >> 32);
  o[n++] = (INT)si->numBadAccessUnits;
  o[n++] = (INT)(si->numBadAccessUnits >> 32);
  o[n++] = si->drcProgRefLev;
  o[n++] = si->drcPresMode;
  o[n++] = si->outputLoudness;
  return n;
}

INT cref_aacdecoder_lib_anc_init(void *h, UCHAR *buf, INT size) {
  return aacDecoder_AncDataInit((HANDLE_AACDECODER)h, buf, size);
}

/* aacDecoder_AncDataGet: *offset = ptr - base (-1 for NULL). */
INT cref_aacdecoder_lib_anc_get(void *h, INT index, UCHAR *base, INT *offset, INT *size) {
  UCHAR *p = NULL;
  INT r = aacDecoder_AncDataGet((HANDLE_AACDECODER)h, index, &p, size);
  *offset = p ? (INT)(p - base) : -1;
  return r;
}

/* aacDecoder_GetLibInfo on an array filled with `fill` bytes and FDKinitLibInfo(): per entry
   module_id, version, flags, title (32 bytes), versionStr (32 bytes) as INT[3] + 64 bytes. */
INT cref_aacdecoder_lib_lib_info(UCHAR fill, INT *ints, UCHAR *strs, INT *n) {
  LIB_INFO info[FDK_MODULE_LAST];
  INT i, r;
  memset(info, fill, sizeof(info));
  FDKinitLibInfo(info);
  r = aacDecoder_GetLibInfo(info);
  for (i = 0; i < FDK_MODULE_LAST && info[i].module_id != FDK_NONE; i++) {
    ints[3 * i] = info[i].module_id;
    ints[3 * i + 1] = info[i].version;
    ints[3 * i + 2] = (INT)info[i].flags;
    memset(strs + 64 * i, 0, 64);
    if (info[i].title) strncpy((char *)strs + 64 * i, info[i].title, 31);
    memcpy(strs + 64 * i + 32, info[i].versionStr, 32);
  }
  *n = i;
  return r;
}

/* Fills the decoder's scratch buffers (workBufferCore1, workBufferCore2, pTimeData2, the
   shared pWorkBufferCore1 of the elements) with a byte pattern (PORTING.md, "Memory"). */
void cref_aacdecoder_lib_poison(void *h, UCHAR pattern) {
  AAC_DECODER_INSTANCE *s = (AAC_DECODER_INSTANCE *)h;
  if (s->workBufferCore1) memset(s->workBufferCore1, pattern, sizeof(CWorkBufferCore1));
  if (s->workBufferCore2) memset(s->workBufferCore2, pattern, (8) * 1024 * sizeof(FIXP_DBL));
  /* timeData2Size (GetRequiredMemWorkBufferCore5()) includes the alignment overhead of the
     allocation: the buffer itself has (8) * (1024 * 4) * 2 samples. */
  if (s->pTimeData2) memset(s->pTimeData2, pattern, (8) * (1024 * 4) * 2 * sizeof(PCM_DEC));
  if (s->pAacDecoderChannelInfo[0] && s->pAacDecoderChannelInfo[0]->pComStaticData &&
      s->pAacDecoderChannelInfo[0]->pComStaticData->pWorkBufferCore1)
    memset(s->pAacDecoderChannelInfo[0]->pComStaticData->pWorkBufferCore1, pattern,
           sizeof(CWorkBufferCore1));
}

/* Encodes `frames` frames of a synthetic signal with the reference encoder. params holds
   (id, value) pairs; meta (if nmeta > 0) sets AACENC_MetaData fields by index:
   drc_profile, comp_profile, drc_TargetRefLevel, comp_TargetRefLevel, prog_ref_level_present,
   prog_ref_level, PCE_mixdown_idx_present, ETSI_DmxLvl_present, centerMixLevel,
   surroundMixLevel, dolbySurroundMode, drcPresentationMode. anc: if non-zero, 3 bytes of
   ancillary data per frame. Writes the access units back to back into out (capacity outcap) and
   their sizes into au (capacity aucap). Returns the number of access units, -1 on error. */
INT cref_aacdecoder_lib_encode(INT channels, INT rate, INT *params, INT nparams, INT *meta, INT nmeta, INT anc, INT frames, UCHAR *out, INT outcap, INT *au, INT aucap, UCHAR *asc, INT *asclen) {
  HANDLE_AACENCODER h = NULL;
  AACENC_InfoStruct info;
  AACENC_MetaData md;
  INT i, nau = 0, pos = 0, f, n_in;
  SHORT *pcm;
  UINT seed = 12345;
  if (aacEncOpen(&h, 0, channels) != AACENC_OK) return -1;
  aacEncoder_SetParam(h, AACENC_SAMPLERATE, rate);
  aacEncoder_SetParam(h, AACENC_CHANNELMODE,
                      channels == 1   ? MODE_1
                      : channels == 2 ? MODE_2
                      : channels == 3 ? MODE_1_2
                      : channels == 4 ? MODE_1_2_1
                      : channels == 5 ? MODE_1_2_2
                                      : MODE_1_2_2_1);
  for (i = 0; i < nparams; i++) aacEncoder_SetParam(h, (AACENC_PARAM)params[2 * i], params[2 * i + 1]);
  if (aacEncEncode(h, NULL, NULL, NULL, NULL) != AACENC_OK) {
    aacEncClose(&h);
    return -1;
  }
  aacEncInfo(h, &info);
  *asclen = (INT)info.confSize;
  memcpy(asc, info.confBuf, info.confSize);
  memset(&md, 0, sizeof(md));
  if (nmeta > 0) {
    md.drc_profile = (AACENC_METADATA_DRC_PROFILE)meta[0];
    md.comp_profile = (AACENC_METADATA_DRC_PROFILE)meta[1];
    md.drc_TargetRefLevel = meta[2];
    md.comp_TargetRefLevel = meta[3];
    md.prog_ref_level_present = meta[4];
    md.prog_ref_level = meta[5];
    md.PCE_mixdown_idx_present = meta[6];
    md.ETSI_DmxLvl_present = meta[7];
    md.centerMixLevel = meta[8];
    md.surroundMixLevel = meta[9];
    md.dolbySurroundMode = meta[10];
    md.drcPresentationMode = meta[11];
  }
  n_in = info.frameLength * channels;
  pcm = (SHORT *)calloc(n_in, sizeof(SHORT));
  for (f = 0; f < frames + 8; f++) {
    AACENC_BufDesc ib, ob;
    AACENC_InArgs ia;
    AACENC_OutArgs oa;
    void *ibufs[3];
    INT iids[3], isz[3], iel[3], nib = 0;
    UCHAR ancb[3];
    void *obufs[1];
    INT oid = OUT_BITSTREAM_DATA, osz, oel = 1;
    UCHAR tmp[20480];
    for (i = 0; i < n_in; i++) {
      INT ch = i % channels, t = f * info.frameLength + i / channels;
      seed = seed * 1103515245u + 12345u;
      pcm[i] = (SHORT)(((t * (37 + 11 * ch)) % 400 - 200) * 60 + (INT)((seed >> 16) & 1023) - 512);
    }
    memset(&ia, 0, sizeof(ia));
    ibufs[nib] = pcm;
    iids[nib] = IN_AUDIO_DATA;
    isz[nib] = n_in * 2;
    iel[nib] = 2;
    nib++;
    ia.numInSamples = f < frames ? n_in : -1;
    if (anc) {
      ancb[0] = (UCHAR)f;
      ancb[1] = 0xA5;
      ancb[2] = (UCHAR)(f * 7);
      ibufs[nib] = ancb;
      iids[nib] = IN_ANCILLRY_DATA;
      isz[nib] = 3;
      iel[nib] = 1;
      nib++;
      ia.numAncBytes = 3;
    }
    if (nmeta > 0) {
      ibufs[nib] = &md;
      iids[nib] = IN_METADATA_SETUP;
      isz[nib] = sizeof(md);
      iel[nib] = sizeof(md);
      nib++;
    }
    ib.numBufs = nib;
    ib.bufs = ibufs;
    ib.bufferIdentifiers = iids;
    ib.bufSizes = isz;
    ib.bufElSizes = iel;
    obufs[0] = tmp;
    osz = sizeof(tmp);
    ob.numBufs = 1;
    ob.bufs = obufs;
    ob.bufferIdentifiers = &oid;
    ob.bufSizes = &osz;
    ob.bufElSizes = &oel;
    if (aacEncEncode(h, &ib, &ob, &ia, &oa) != AACENC_OK) break;
    if (oa.numOutBytes > 0) {
      if (pos + oa.numOutBytes > outcap || nau >= aucap) break;
      memcpy(out + pos, tmp, oa.numOutBytes);
      pos += oa.numOutBytes;
      au[nau++] = oa.numOutBytes;
    }
  }
  free(pcm);
  aacEncClose(&h);
  return nau;
}
}

extern "C" {
/* Copies the first n values of the decoder's pTimeData2 (stage comparison). */
void cref_aacdecoder_lib_time_data2(void *h, INT *out, INT n) {
  AAC_DECODER_INSTANCE *s = (AAC_DECODER_INSTANCE *)h;
  memcpy(out, s->pTimeData2, n * sizeof(INT));
}
}
