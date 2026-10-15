// Shims for the differential tests of fdk_aac_rs_aacenc::metadata_main.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_metadata_main (see build.rs).
//
// metadata_main.cpp is included at file scope for its file-static functions and tables (the
// structure FDK_METADATA_ENCODER is defined there, so a namespace copy would not match the
// handle type of metadata_main.h); its non-static functions get other names in the copy. The
// stateful tests call the library's own FDK_MetadataEnc_* functions.

#include "cref_common.h"
#include "aacenc_lib.h"
#include "libAACenc/src/aacenc.h"
#include "libAACenc/src/metadata_main.h"
#include "libAACenc/src/metadata_compressor.h"
#include "FDK_bitstream.h"
#include "FDK_audio.h"

#include <stddef.h>
#include <stdlib.h>

/* the library's functions, declared with their own names by metadata_main.h above */
static FDK_METADATA_ERROR (*const lib_Open)(HANDLE_FDK_METADATA_ENCODER *, const UINT) =
    FDK_MetadataEnc_Open;
static FDK_METADATA_ERROR (*const lib_Close)(HANDLE_FDK_METADATA_ENCODER *) =
    FDK_MetadataEnc_Close;
static FDK_METADATA_ERROR (*const lib_Init)(HANDLE_FDK_METADATA_ENCODER, const INT, const INT,
                                            const INT, const UINT, const UINT, const UINT,
                                            const CHANNEL_MODE, const CHANNEL_ORDER) =
    FDK_MetadataEnc_Init;
static FDK_METADATA_ERROR (*const lib_Process)(HANDLE_FDK_METADATA_ENCODER, INT_PCM *const,
                                               const UINT, const INT,
                                               const AACENC_MetaData *const,
                                               AACENC_EXT_PAYLOAD **, UINT *, INT *) =
    FDK_MetadataEnc_Process;
static INT (*const lib_GetDelay)(HANDLE_FDK_METADATA_ENCODER) = FDK_MetadataEnc_GetDelay;

#define FDK_MetadataEnc_Open copy_FDK_MetadataEnc_Open
#define FDK_MetadataEnc_Close copy_FDK_MetadataEnc_Close
#define FDK_MetadataEnc_Init copy_FDK_MetadataEnc_Init
#define FDK_MetadataEnc_Process copy_FDK_MetadataEnc_Process
#define FDK_MetadataEnc_GetDelay copy_FDK_MetadataEnc_GetDelay
/* the copy calls its Close before defining it */
FDK_METADATA_ERROR copy_FDK_MetadataEnc_Close(HANDLE_FDK_METADATA_ENCODER *phMetaData);
#include "libAACenc/src/metadata_main.cpp"
#undef FDK_MetadataEnc_Open
#undef FDK_MetadataEnc_Close
#undef FDK_MetadataEnc_Init
#undef FDK_MetadataEnc_Process
#undef FDK_MetadataEnc_GetDelay

/* AAC_METADATA <-> INT[82] in the order of the port's test (see metadata_main.rs). */
static void md_to_ints(const AAC_METADATA *m, INT *v) {
  INT k = 0, i;
  v[k++] = m->mpegDrc.prog_ref_level_present;
  v[k++] = m->mpegDrc.prog_ref_level;
  for (i = 0; i < MAX_DRC_BANDS; i++) v[k++] = m->mpegDrc.dyn_rng_sgn[i];
  for (i = 0; i < MAX_DRC_BANDS; i++) v[k++] = m->mpegDrc.dyn_rng_ctl[i];
  v[k++] = m->mpegDrc.drc_bands_present;
  v[k++] = m->mpegDrc.drc_band_incr;
  for (i = 0; i < MAX_DRC_BANDS; i++) v[k++] = m->mpegDrc.drc_band_top[i];
  v[k++] = m->mpegDrc.drc_interpolation_scheme;
  v[k++] = m->mpegDrc.drc_profile;
  v[k++] = m->mpegDrc.drc_TargetRefLevel;
  v[k++] = m->mpegDrc.excluded_chns_present;
  v[k++] = m->mpegDrc.exclude_mask[0];
  v[k++] = m->mpegDrc.exclude_mask[1];
  v[k++] = m->etsiAncData.compression_on;
  v[k++] = m->etsiAncData.compression_value;
  v[k++] = m->etsiAncData.comp_profile;
  v[k++] = m->etsiAncData.comp_TargetRefLevel;
  v[k++] = m->etsiAncData.timecode_coarse_status;
  v[k++] = m->etsiAncData.timecode_fine_status;
  v[k++] = m->etsiAncData.extAncDataStatus;
  v[k++] = m->etsiAncData.extAncData.ext_downmix_lvl_status;
  v[k++] = m->etsiAncData.extAncData.ext_downmix_gain_status;
  v[k++] = m->etsiAncData.extAncData.ext_lfe_downmix_status;
  v[k++] = m->etsiAncData.extAncData.ext_dmix_a_idx;
  v[k++] = m->etsiAncData.extAncData.ext_dmix_b_idx;
  v[k++] = m->etsiAncData.extAncData.dmx_gain_5_sgn;
  v[k++] = m->etsiAncData.extAncData.dmx_gain_5_idx;
  v[k++] = m->etsiAncData.extAncData.dmx_gain_2_sgn;
  v[k++] = m->etsiAncData.extAncData.dmx_gain_2_idx;
  v[k++] = m->etsiAncData.extAncData.ext_dmix_lfe_idx;
  v[k++] = m->centerMixLevel;
  v[k++] = m->surroundMixLevel;
  v[k++] = m->WritePCEMixDwnIdx;
  v[k++] = m->DmxLvl_On;
  v[k++] = m->dolbySurroundMode;
  v[k++] = m->drcPresentationMode;
  v[k++] = m->metadataMode;
}
static void md_from_ints(AAC_METADATA *m, const INT *v) {
  INT k = 0, i;
  memset(m, 0, sizeof(*m));
  m->mpegDrc.prog_ref_level_present = (UCHAR)v[k++];
  m->mpegDrc.prog_ref_level = (SCHAR)v[k++];
  for (i = 0; i < MAX_DRC_BANDS; i++) m->mpegDrc.dyn_rng_sgn[i] = (UCHAR)v[k++];
  for (i = 0; i < MAX_DRC_BANDS; i++) m->mpegDrc.dyn_rng_ctl[i] = (UCHAR)v[k++];
  m->mpegDrc.drc_bands_present = (UCHAR)v[k++];
  m->mpegDrc.drc_band_incr = (UCHAR)v[k++];
  for (i = 0; i < MAX_DRC_BANDS; i++) m->mpegDrc.drc_band_top[i] = (UCHAR)v[k++];
  m->mpegDrc.drc_interpolation_scheme = (UCHAR)v[k++];
  m->mpegDrc.drc_profile = (AACENC_METADATA_DRC_PROFILE)v[k++];
  m->mpegDrc.drc_TargetRefLevel = v[k++];
  m->mpegDrc.excluded_chns_present = (UCHAR)v[k++];
  m->mpegDrc.exclude_mask[0] = (UCHAR)v[k++];
  m->mpegDrc.exclude_mask[1] = (UCHAR)v[k++];
  m->etsiAncData.compression_on = (UCHAR)v[k++];
  m->etsiAncData.compression_value = (UCHAR)v[k++];
  m->etsiAncData.comp_profile = (AACENC_METADATA_DRC_PROFILE)v[k++];
  m->etsiAncData.comp_TargetRefLevel = v[k++];
  m->etsiAncData.timecode_coarse_status = v[k++];
  m->etsiAncData.timecode_fine_status = v[k++];
  m->etsiAncData.extAncDataStatus = (UCHAR)v[k++];
  m->etsiAncData.extAncData.ext_downmix_lvl_status = (UCHAR)v[k++];
  m->etsiAncData.extAncData.ext_downmix_gain_status = (UCHAR)v[k++];
  m->etsiAncData.extAncData.ext_lfe_downmix_status = (UCHAR)v[k++];
  m->etsiAncData.extAncData.ext_dmix_a_idx = (UCHAR)v[k++];
  m->etsiAncData.extAncData.ext_dmix_b_idx = (UCHAR)v[k++];
  m->etsiAncData.extAncData.dmx_gain_5_sgn = (UCHAR)v[k++];
  m->etsiAncData.extAncData.dmx_gain_5_idx = (UCHAR)v[k++];
  m->etsiAncData.extAncData.dmx_gain_2_sgn = (UCHAR)v[k++];
  m->etsiAncData.extAncData.dmx_gain_2_idx = (UCHAR)v[k++];
  m->etsiAncData.extAncData.ext_dmix_lfe_idx = (UCHAR)v[k++];
  m->centerMixLevel = (SCHAR)v[k++];
  m->surroundMixLevel = (SCHAR)v[k++];
  m->WritePCEMixDwnIdx = (UCHAR)v[k++];
  m->DmxLvl_On = (UCHAR)v[k++];
  m->dolbySurroundMode = (UCHAR)v[k++];
  m->drcPresentationMode = (UCHAR)v[k++];
  m->metadataMode = (UCHAR)v[k++];
}

extern "C" {
/* The small file-static helpers: which 0 convertProfile(a), 1 dialnorm2progreflvl(a),
   2 progreflvl2dialnorm((UCHAR)a), 3 encodeDmxLvls((SCHAR)a, (SCHAR)b), 4 encodeDynrng(a,
   &ctl, &sgn) -> ctl | sgn << 8, 5 decodeDynrng((UCHAR)a, (UCHAR)b), 6 encodeCompr(a),
   7 decodeCompr((UCHAR)a). */
INT cref_metadata_main_helper(INT which, INT a, INT b) {
  switch (which) {
    case 0:
      return convertProfile((AACENC_METADATA_DRC_PROFILE)a);
    case 1:
      return dialnorm2progreflvl(a);
    case 2:
      return progreflvl2dialnorm((UCHAR)a);
    case 3:
      return encodeDmxLvls((SCHAR)a, (SCHAR)b);
    case 4: {
      UCHAR ctl = 0, sgn = 0;
      encodeDynrng(a, &ctl, &sgn);
      return ctl | (sgn << 8);
    }
    case 5:
      return decodeDynrng((UCHAR)a, (UCHAR)b);
    case 6:
      return encodeCompr(a);
    default:
      return decodeCompr((UCHAR)a);
  }
}
/* LoadSubmittedMetadata(meta or NULL, nChannels, metadataMode, &out) -> out as INT[82] */
INT cref_metadata_main_load(const void *meta, INT nChannels, INT metadataMode, INT *out) {
  AAC_METADATA m;
  memset(&m, 0x5a, sizeof(m));
  INT e = LoadSubmittedMetadata((const AACENC_MetaData *)meta, nChannels, metadataMode, &m);
  md_to_ints(&m, out);
  return e;
}
/* WriteDynamicRangeInfoPayload / WriteEtsiAncillaryDataPayload(metadata from INT[82]) into a
   payload buffer of 16 bytes (plus 16 guard bytes, all initialized to fill); returns the bits. */
INT cref_metadata_main_write_payload(INT which, const INT *md, UCHAR *buf, INT fill) {
  AAC_METADATA m;
  md_from_ints(&m, md);
  memset(buf, fill, 32);
  return which == 0 ? WriteDynamicRangeInfoPayload(&m, buf)
                    : WriteEtsiAncillaryDataPayload(&m, buf);
}
/* The tables: dmxTable (8), dmxLfeTable (16), surmix2matrix_mixdown_idx (8 as INT), and
   defaultMetaDataSetup (as its bytes, 84). */
void cref_metadata_main_tables(INT *dmx, INT *lfe, INT *surmix, UCHAR *dflt) {
  for (INT i = 0; i < 8; i++) dmx[i] = dmxTable[i];
  for (INT i = 0; i < 16; i++) lfe[i] = dmxLfeTable[i];
  for (INT i = 0; i < 8; i++) surmix[i] = surmix2matrix_mixdown_idx[i];
  memcpy(dflt, &defaultMetaDataSetup, sizeof(defaultMetaDataSetup));
}
/* The layout of the members of FDK_METADATA_ENCODER the port keeps as one byte image:
   offsets of drcInfoPayload, drcDsePayload, matrix_mixdown_idx, exPayload. */
void cref_metadata_main_layout(INT *v) {
  v[0] = offsetof(FDK_METADATA_ENCODER, drcInfoPayload);
  v[1] = offsetof(FDK_METADATA_ENCODER, drcDsePayload);
  v[2] = offsetof(FDK_METADATA_ENCODER, matrix_mixdown_idx);
  v[3] = offsetof(FDK_METADATA_ENCODER, exPayload);
}
/* The library's FDK_MetadataEnc_Open(&h, maxChannels) */
INT cref_metadata_main_open(void **ph, UINT maxChannels) {
  HANDLE_FDK_METADATA_ENCODER h = NULL;
  INT e = lib_Open(&h, maxChannels);
  *ph = h;
  return e;
}
INT cref_metadata_main_close(void *h) {
  HANDLE_FDK_METADATA_ENCODER hh = (HANDLE_FDK_METADATA_ENCODER)h;
  return lib_Close(&hh);
}
INT cref_metadata_main_init(void *h, INT resetStates, INT metadataMode, INT audioDelay, UINT frameLength, UINT sampleRate, UINT nChannels, INT channelMode, INT channelOrder) {
  return lib_Init((HANDLE_FDK_METADATA_ENCODER)h, resetStates, metadataMode, audioDelay,
                  frameLength, sampleRate, nChannels, (CHANNEL_MODE)channelMode,
                  (CHANNEL_ORDER)channelOrder);
}
/* FDK_MetadataEnc_Process(h, audio, bufSize, nSamples, meta or NULL, ...). Writes the
   extension payloads as {type, size in bits, associatedChElement} into ext[3 * i] and their
   bytes (16 per payload) into bytes; *n = nMetaDataExtensions, *mmidx = matrix_mixdown_idx
   (both initialized to -77 here). */
INT cref_metadata_main_process(void *h, SHORT *audio, UINT bufSize, INT nSamples, const void *meta, INT *n, INT *ext, UCHAR *bytes, INT *mmidx) {
  AACENC_EXT_PAYLOAD *p = NULL;
  UINT nExt = (UINT)-77;
  *mmidx = -77;
  INT e = lib_Process((HANDLE_FDK_METADATA_ENCODER)h, audio, bufSize, nSamples,
                      (const AACENC_MetaData *)meta, &p, &nExt, mmidx);
  *n = (INT)nExt;
  if (p != NULL) {
    for (UINT i = 0; i < nExt && i < 2; i++) {
      ext[3 * i] = p[i].dataType;
      ext[3 * i + 1] = (INT)p[i].dataSize;
      ext[3 * i + 2] = p[i].associatedChElement;
      memcpy(bytes + 16 * i, p[i].pData, 16);
    }
  }
  return e;
}
INT cref_metadata_main_get_delay(void *h) {
  return lib_GetDelay((HANDLE_FDK_METADATA_ENCODER)h);
}
/* The delay line and state of the encoder: nAudioDataDelay, nMetaDataDelay, nChannels,
   channelMode, metadataMode, metaDataDelayIdx, finalizeMetaData, initializeMetaData; the
   audio delay buffer (maxChannels * 2048 samples) into delay; metaDataBuffer[i] as INT[82]
   each into md. */
void cref_metadata_main_state(void *h, INT *v, SHORT *delay, INT *md) {
  HANDLE_FDK_METADATA_ENCODER m = (HANDLE_FDK_METADATA_ENCODER)h;
  v[0] = m->nAudioDataDelay;
  v[1] = m->nMetaDataDelay;
  v[2] = m->nChannels;
  v[3] = m->channelMode;
  v[4] = m->metadataMode;
  v[5] = m->metaDataDelayIdx;
  v[6] = m->finalizeMetaData;
  v[7] = m->initializeMetaData;
  memcpy(delay, m->pAudioDelayBuffer, m->maxChannels * MAX_DRC_FRAMELEN * sizeof(INT_PCM));
  for (INT i = 0; i < MAX_DELAY_FRAMES; i++) md_to_ints(&m->metaDataBuffer[i], md + 82 * i);
}
}
