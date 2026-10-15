// Shims for the differential tests of the file-static helpers, the tables and the instance
// layout of fdk_aac_rs_pcmutils::pcmdmx_lib.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::pcmutils_pcmdmx_lib_static (see build.rs).
//
// A copy of pcmdmx_lib.cpp is compiled into the namespace ref_dmx (with the flags of the
// library). pcmdmx_lib.h is included for the first time inside the namespace, so that
// HANDLE_PCM_DOWNMIX refers to the namespaced struct PCM_DMX_INSTANCE; the extern "C" API
// functions of the copy are renamed, so that they cannot clash with the library's. The
// library's instances have the same layout and are read through the namespaced struct.

#include "cref_common.h"
#include "common_fix.h"
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "fixpoint_math.h"
#include "FDK_core.h"
#include "syslib_channelMapDescr.h"

#define pcmDmx_Open cref_dmxcopy_Open
#define pcmDmx_SetParam cref_dmxcopy_SetParam
#define pcmDmx_GetParam cref_dmxcopy_GetParam
#define pcmDmx_Parse cref_dmxcopy_Parse
#define pcmDmx_ReadDvbAncData cref_dmxcopy_ReadDvbAncData
#define pcmDmx_SetMatrixMixdownFromPce cref_dmxcopy_SetMatrixMixdownFromPce
#define pcmDmx_Reset cref_dmxcopy_Reset
#define pcmDmx_ApplyFrame cref_dmxcopy_ApplyFrame
#define pcmDmx_Close cref_dmxcopy_Close
#define pcmDmx_GetLibInfo cref_dmxcopy_GetLibInfo

namespace ref_dmx {
#include "libPCMutils/src/pcmdmx_lib.cpp"
}

using namespace ref_dmx;

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

static void meta_store(const DMX_BS_META_DATA *m, INT *s) {
  s[0] = (INT)m->typeFlags;
  s[1] = m->cLevIdx;
  s[2] = m->sLevIdx;
  s[3] = m->dmixIdxA;
  s[4] = m->dmixIdxB;
  s[5] = m->dmixIdxLfe;
  s[6] = m->dmxGainIdx2;
  s[7] = m->dmxGainIdx5;
  s[8] = m->matrixMixdownIdx;
  s[9] = m->pseudoSurround;
  s[10] = (INT)m->expiryCount;
}

static void meta_load(DMX_BS_META_DATA *m, const INT *s) {
  m->typeFlags = (UINT)s[0];
  m->cLevIdx = (UCHAR)s[1];
  m->sLevIdx = (UCHAR)s[2];
  m->dmixIdxA = (UCHAR)s[3];
  m->dmixIdxB = (UCHAR)s[4];
  m->dmixIdxLfe = (UCHAR)s[5];
  m->dmxGainIdx2 = (UCHAR)s[6];
  m->dmxGainIdx5 = (UCHAR)s[7];
  m->matrixMixdownIdx = (UCHAR)s[8];
  m->pseudoSurround = (SCHAR)s[9];
  m->expiryCount = (UINT)s[10];
}

extern "C" {
/* The state of a library instance (30 values): the two metadata slots (11 values each), the
   user parameters (7 values) and applyProcessing. */
void cref_pcmdmx_static_state(void *h, INT *s) {
  struct PCM_DMX_INSTANCE *p = (struct PCM_DMX_INSTANCE *)h;
  meta_store(&p->bsMetaData[0], s);
  meta_store(&p->bsMetaData[1], s + 11);
  s[22] = (INT)p->userParams.dmxProfile;
  s[23] = (INT)p->userParams.expiryFrame;
  s[24] = (INT)p->userParams.dualChannelMode;
  s[25] = (INT)p->userParams.pseudoSurrMode;
  s[26] = p->userParams.numOutChannelsMin;
  s[27] = p->userParams.numOutChannelsMax;
  s[28] = p->userParams.frameDelay;
  s[29] = p->applyProcessing;
}
/* Sets the metadata of a library instance (2 * 11 values, layout of cref_pcmdmx_static_state). */
void cref_pcmdmx_static_set_meta(void *h, const INT *s) {
  struct PCM_DMX_INSTANCE *p = (struct PCM_DMX_INSTANCE *)h;
  meta_load(&p->bsMetaData[0], s);
  meta_load(&p->bsMetaData[1], s + 11);
}
/* The tables: abMixLvlValueTab (8), lfeMixLvlValueTab (16), mpegMixDownIdx2Coef (4),
   mpegMixDownIdx2PreFact (12); spkrSlotPos (24); outChModeTable (9); dfltMetaData (11). */
void cref_pcmdmx_static_tables(SHORT *sgl, SCHAR *spkr, INT *modes, INT *dflt) {
  INT n = 0;
  for (int i = 0; i < 8; i++) sgl[n++] = abMixLvlValueTab[i];
  for (int i = 0; i < 16; i++) sgl[n++] = lfeMixLvlValueTab[i];
  for (int i = 0; i < 4; i++) sgl[n++] = mpegMixDownIdx2Coef[i];
  for (int i = 0; i < 3; i++)
    for (int j = 0; j < 4; j++) sgl[n++] = mpegMixDownIdx2PreFact[i][j];
  for (int i = 0; i < 8; i++) {
    spkr[3 * i] = spkrSlotPos[i].x;
    spkr[3 * i + 1] = spkrSlotPos[i].y;
    spkr[3 * i + 2] = spkrSlotPos[i].z;
  }
  for (int i = 0; i < 9; i++) modes[i] = outChModeTable[i];
  meta_store(&dfltMetaData, dflt);
}
void cref_pcmdmx_static_get_speaker_pos(INT chType, UCHAR chIndex, UCHAR numChInGrp, SCHAR *xyz) {
  PCM_DMX_SPEAKER_POSITION p = getSpeakerPos((AUDIO_CHANNEL_TYPE)chType, chIndex, numChInGrp);
  xyz[0] = p.x;
  xyz[1] = p.y;
  xyz[2] = p.z;
}
UINT cref_pcmdmx_static_get_speaker_distance(const SCHAR *a, const SCHAR *b) {
  PCM_DMX_SPEAKER_POSITION pa = {a[0], a[1], a[2]};
  PCM_DMX_SPEAKER_POSITION pb = {b[0], b[1], b[2]};
  return getSpeakerDistance(pa, pb);
}
INT cref_pcmdmx_static_get_ch_mode4_plain(INT plainIndex, INT totChMode, INT chCfg) {
  return getChMode4Plain(plainIndex, (PCM_DMX_CHANNEL_MODE)totChMode, chCfg);
}
/* getChannelMode(): channelIndices is updated in place. */
INT cref_pcmdmx_static_get_channel_mode(UINT numChannels, const INT *chType, UCHAR *chIdx, UCHAR *offsetTable, INT *chMode) {
  AUDIO_CHANNEL_TYPE types[8];
  for (int i = 0; i < 8; i++) types[i] = (AUDIO_CHANNEL_TYPE)chType[i];
  PCM_DMX_CHANNEL_MODE m = CH_MODE_UNDEFINED;
  INT r = getChannelMode(numChannels, types, chIdx, offsetTable, &m);
  *chMode = (INT)m;
  return r;
}
void cref_pcmdmx_static_get_channel_description(INT chMode, INT map_kind, INT pass, const UCHAR *maps, const UCHAR *nch, UINT tab_len, INT *chType, UCHAR *chIdx, UCHAR *offsetTable) {
  CHANNEL_MAP_INFO tab[32];
  FDK_channelMapDescr d;
  dmx_map_descr(&d, tab, map_kind, pass, maps, nch, tab_len);
  AUDIO_CHANNEL_TYPE types[8];
  for (int i = 0; i < 8; i++) types[i] = (AUDIO_CHANNEL_TYPE)chType[i];
  getChannelDescription((PCM_DMX_CHANNEL_MODE)chMode, &d, types, chIdx, offsetTable);
  for (int i = 0; i < 8; i++) chType[i] = (INT)types[i];
}
/* getMixFactors(): params are the 7 user parameters (layout of cref_pcmdmx_static_state), meta
   the 11 metadata values; mix receives the 64 factors. */
INT cref_pcmdmx_static_get_mix_factors(UCHAR inModeIsCfg, INT inChMode, INT outChMode, const INT *params, const INT *meta, SHORT *mix, INT *outScale) {
  PCM_DMX_USER_PARAMS p;
  FDKmemclear(&p, sizeof(p));
  p.dmxProfile = (DMX_PROFILE_TYPE)params[0];
  p.expiryFrame = (UINT)params[1];
  p.dualChannelMode = (DUAL_CHANNEL_MODE)params[2];
  p.pseudoSurrMode = (PSEUDO_SURROUND_MODE)params[3];
  p.numOutChannelsMin = (SHORT)params[4];
  p.numOutChannelsMax = (SHORT)params[5];
  p.frameDelay = (UCHAR)params[6];
  DMX_BS_META_DATA m;
  meta_load(&m, meta);
  FIXP_DMX f[8][8];
  INT r = getMixFactors(inModeIsCfg, (PCM_DMX_CHANNEL_MODE)inChMode, (PCM_DMX_CHANNEL_MODE)outChMode, &p, &m, f, outScale);
  for (int i = 0; i < 8; i++)
    for (int j = 0; j < 8; j++) mix[8 * i + j] = f[i][j];
  return r;
}
/* dmxSetChannel() (op 0) or dmxAddChannel() (op 1) on the matrix mix/scales (64 values each). */
void cref_pcmdmx_static_dmx_channel_op(INT op, SHORT *mix, INT *scales, UINT dstCh, UINT srcCh, SHORT factor, INT scale) {
  FIXP_DMX f[8][8];
  INT s[8][8];
  for (int i = 0; i < 8; i++)
    for (int j = 0; j < 8; j++) {
      f[i][j] = mix[8 * i + j];
      s[i][j] = scales[8 * i + j];
    }
  if (op == 0)
    dmxSetChannel(f, s, dstCh, srcCh, factor, scale);
  else
    dmxAddChannel(f, s, dstCh, srcCh, factor, scale);
  for (int i = 0; i < 8; i++)
    for (int j = 0; j < 8; j++) {
      mix[8 * i + j] = f[i][j];
      scales[8 * i + j] = s[i][j];
    }
}
}
