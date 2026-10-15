// Shims for the differential tests of fdk_aac_rs_pcmutils::pcmdmx_lib.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::pcmutils_pcmdmx_lib (see build.rs).
//
// These shims call the library's own pcmDmx_* functions. The instance lives on the C side
// behind an opaque handle (NULL is passed through, so that the NULL checks can be tested); its
// state is read by pcmutils_pcmdmx_lib_static.cpp. The file-static helpers and the tables are
// tested through a copy of pcmdmx_lib.cpp in pcmutils_pcmdmx_lib_static.cpp.

#include "cref_common.h"
#include "pcmdmx_lib.h"
#include "syslib_channelMapDescr.h"

/* Builds a channel map descriptor: kind 0 = default table, 1 = WG4 table (both through
   FDK_chMapDescr_init), 2 = the custom table `tab` of `tab_len` entries with the channel maps
   maps[24 * i] and the channel counts nch[i] (set directly, so that invalid descriptors can be
   tested). */
static void dmx_map_descr(FDK_channelMapDescr *d, CHANNEL_MAP_INFO *tab, INT kind, INT pass, const UCHAR *maps, const UCHAR *nch, UINT tab_len) {
  FDKmemclear(d, sizeof(*d));
  if (kind == 0) {
    FDK_chMapDescr_init(d, NULL, 0, pass);
  } else if (kind == 1) {
    FDK_chMapDescr_init(d, FDK_mapInfoTabWg4, FDK_mapInfoTabLenWg4, pass);
  } else {
    for (UINT i = 0; i < tab_len; i++) {
      tab[i].pChannelMap = maps + 24 * i;
      tab[i].numChannels = nch[i];
    }
    d->pMapInfoTab = tab;
    d->mapInfoTabLen = tab_len;
    d->fPassThrough = pass;
  }
}

/* FDK_bitstream state in an array, with the layout of fdk_bitstream.cpp. */
static void dmx_bs_load(FDK_BITSTREAM *bs, const UINT *st, UCHAR *buf) {
  bs->CacheWord = st[0];
  bs->BitsInCache = st[1];
  bs->ConfigCache = st[2];
  bs->hBitBuf.ValidBits = st[3];
  bs->hBitBuf.ReadOffset = st[4];
  bs->hBitBuf.WriteOffset = st[5];
  bs->hBitBuf.BitNdx = st[6];
  bs->hBitBuf.bufSize = st[7];
  bs->hBitBuf.bufBits = st[8];
  bs->hBitBuf.Buffer = buf;
}

static void dmx_bs_store(const FDK_BITSTREAM *bs, UINT *st) {
  st[0] = bs->CacheWord;
  st[1] = bs->BitsInCache;
  st[2] = bs->ConfigCache;
  st[3] = bs->hBitBuf.ValidBits;
  st[4] = bs->hBitBuf.ReadOffset;
  st[5] = bs->hBitBuf.WriteOffset;
  st[6] = bs->hBitBuf.BitNdx;
  st[7] = bs->hBitBuf.bufSize;
  st[8] = bs->hBitBuf.bufBits;
}

extern "C" {
/* The values of the enums of pcmdmx_lib.h, in declaration order. */
void cref_pcmdmx_consts(INT *c) {
  INT i = 0;
  c[i++] = PCMDMX_OK;
  c[i++] = PCMDMX_UNSUPPORTED;
  c[i++] = pcm_dmx_fatal_error_start;
  c[i++] = PCMDMX_OUT_OF_MEMORY;
  c[i++] = pcm_dmx_fatal_error_end;
  c[i++] = PCMDMX_INVALID_HANDLE;
  c[i++] = PCMDMX_INVALID_ARGUMENT;
  c[i++] = PCMDMX_INVALID_CH_CONFIG;
  c[i++] = PCMDMX_INVALID_MODE;
  c[i++] = PCMDMX_UNKNOWN_PARAM;
  c[i++] = PCMDMX_UNABLE_TO_SET_PARAM;
  c[i++] = PCMDMX_CORRUPT_ANC_DATA;
  c[i++] = PCMDMX_OUTPUT_BUFFER_TOO_SMALL;
  c[i++] = DMX_PROFILE_SETTING;
  c[i++] = DMX_BS_DATA_EXPIRY_FRAME;
  c[i++] = DMX_BS_DATA_DELAY;
  c[i++] = MIN_NUMBER_OF_OUTPUT_CHANNELS;
  c[i++] = MAX_NUMBER_OF_OUTPUT_CHANNELS;
  c[i++] = DMX_DUAL_CHANNEL_MODE;
  c[i++] = DMX_PSEUDO_SURROUND_MODE;
  c[i++] = DMX_PRFL_STANDARD;
  c[i++] = DMX_PRFL_MATRIX_MIX;
  c[i++] = DMX_PRFL_FORCE_MATRIX_MIX;
  c[i++] = DMX_PRFL_ARIB_JAPAN;
  c[i++] = NEVER_DO_PS_DMX;
  c[i++] = AUTO_PS_DMX;
  c[i++] = FORCE_PS_DMX;
  c[i++] = STEREO_MODE;
  c[i++] = CH1_MODE;
  c[i++] = CH2_MODE;
  c[i++] = MIXED_MODE;
  c[i++] = PCMDMX_RESET_PARAMS;
  c[i++] = PCMDMX_RESET_BS_DATA;
  c[i++] = PCMDMX_RESET_FULL;
  for (INT e = 0; e < 16; e++) c[i++] = PCMDMX_IS_FATAL_ERROR((PCMDMX_ERROR)e);
}
/* pcmDmx_Open(): the handle, or NULL with the error in *err. null_pself passes NULL. */
void *cref_pcmdmx_open(INT null_pself, INT *err) {
  HANDLE_PCM_DOWNMIX h = (HANDLE_PCM_DOWNMIX)1;
  *err = pcmDmx_Open(null_pself ? NULL : &h);
  return null_pself ? NULL : (void *)h;
}
INT cref_pcmdmx_close(void *h, INT null_pself) {
  HANDLE_PCM_DOWNMIX hh = (HANDLE_PCM_DOWNMIX)h;
  return pcmDmx_Close(null_pself ? NULL : &hh);
}
INT cref_pcmdmx_reset(void *h, UINT flags) { return pcmDmx_Reset((HANDLE_PCM_DOWNMIX)h, flags); }
INT cref_pcmdmx_set_param(void *h, INT param, INT value) { return pcmDmx_SetParam((HANDLE_PCM_DOWNMIX)h, (PCMDMX_PARAM)param, value); }
INT cref_pcmdmx_get_param(void *h, INT param, INT *value, INT null_value) {
  return pcmDmx_GetParam((HANDLE_PCM_DOWNMIX)h, (PCMDMX_PARAM)param, null_value ? NULL : value);
}
INT cref_pcmdmx_set_matrix_mixdown_from_pce(void *h, INT present, INT idx, INT ps) {
  return pcmDmx_SetMatrixMixdownFromPce((HANDLE_PCM_DOWNMIX)h, present, idx, ps);
}
/* pcmDmx_Parse() on the bitstream `st`/`buf` (NULL if null_bs); the state is written back. */
INT cref_pcmdmx_parse(void *h, UINT *st, UCHAR *buf, UINT ancDataBits, INT isMpeg2, INT null_bs) {
  FDK_BITSTREAM bs;
  dmx_bs_load(&bs, st, buf);
  INT r = pcmDmx_Parse((HANDLE_PCM_DOWNMIX)h, null_bs ? NULL : &bs, ancDataBits, isMpeg2);
  dmx_bs_store(&bs, st);
  return r;
}
INT cref_pcmdmx_read_dvb_anc_data(void *h, UCHAR *buf, UINT bytes, INT isMpeg2) {
  return pcmDmx_ReadDvbAncData((HANDLE_PCM_DOWNMIX)h, buf, bytes, isMpeg2);
}
/* pcmDmx_ApplyFrame(). null_mask: 1 nChannels, 2 channelType, 4 channelIndices, 8 mapDescr,
   16 pPcmBuf are NULL. pDmxOutScale is NULL unless use_out_scale. */
INT cref_pcmdmx_apply_frame(void *h, INT *pcm, INT pcmBufSize, UINT frameSize, INT *nChannels, INT interleaved, INT *chType, UCHAR *chIdx, INT map_kind, INT pass, const UCHAR *maps, const UCHAR *nch, UINT tab_len, INT use_out_scale, INT *outScale, INT null_mask) {
  CHANNEL_MAP_INFO tab[32];
  FDK_channelMapDescr d;
  dmx_map_descr(&d, tab, map_kind, pass, maps, nch, tab_len);
  AUDIO_CHANNEL_TYPE types[8];
  for (int i = 0; i < 8; i++) types[i] = (AUDIO_CHANNEL_TYPE)chType[i];
  INT r = pcmDmx_ApplyFrame((HANDLE_PCM_DOWNMIX)h, (null_mask & 16) ? NULL : (DMX_PCM *)pcm, pcmBufSize, frameSize, (null_mask & 1) ? NULL : nChannels, interleaved, (null_mask & 2) ? NULL : types, (null_mask & 4) ? NULL : chIdx, (null_mask & 8) ? NULL : &d, use_out_scale ? outScale : NULL);
  for (int i = 0; i < 8; i++) chType[i] = (INT)types[i];
  return r;
}
INT cref_pcmdmx_module_last(void) { return FDK_MODULE_LAST; }
/* pcmDmx_GetLibInfo(), as cref_limiter_get_lib_info(). */
INT cref_pcmdmx_get_lib_info(INT null_info, const INT *ids_in, INT *ids, INT *version, UINT *flags, char *version_str, char *title) {
  LIB_INFO info[FDK_MODULE_LAST];
  FDKmemclear(info, sizeof(info));
  for (int i = 0; i < FDK_MODULE_LAST; i++) {
    info[i].module_id = (FDK_MODULE_ID)ids_in[i];
    FDKmemset(info[i].versionStr, 'x', sizeof(info[i].versionStr));
  }
  INT r = pcmDmx_GetLibInfo(null_info ? NULL : info);
  for (int i = 0; i < FDK_MODULE_LAST; i++) {
    ids[i] = info[i].module_id;
    version[i] = info[i].version;
    flags[i] = info[i].flags;
    FDKmemcpy(version_str + 32 * i, info[i].versionStr, 32);
    FDKmemclear(title + 32 * i, 32);
    if (info[i].title != NULL) strncpy(title + 32 * i, info[i].title, 31);
  }
  return r;
}
}
