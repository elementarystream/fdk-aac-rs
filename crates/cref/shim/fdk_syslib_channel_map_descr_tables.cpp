// Shims for the table tests of fdk_aac_rs_fdk::syslib_channel_map_descr.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_syslib_channel_map_descr_tables (see build.rs).
//
// syslib_channelMapDescr.cpp is compiled here in a namespace together with its header (it is not
// included at global scope before), for its file-static tables.

#include "cref_common.h"

namespace ref_chmap {
#include "libSYS/src/syslib_channelMapDescr.cpp"
}

#define CHMAP_TABLES(T)                                                                     \
  T(mapFallback) T(mapCfg1) T(mapCfg2) T(mapCfg3) T(mapCfg4) T(mapCfg5) T(mapCfg6) T(mapCfg7) \
  T(mapCfg11) T(mapCfg12) T(mapCfg13) T(mapCfg14) T(mapWg4Cfg1) T(mapWg4Cfg2) T(mapWg4Cfg3)  \
  T(mapWg4Cfg4) T(mapWg4Cfg5) T(mapWg4Cfg6) T(mapWg4Cfg7) T(mapWg4Cfg14)

extern "C" {
/* The file-static channel map `name` of syslib_channelMapDescr.cpp (the copy) as memory.
   Returns NULL for another name. */
const UCHAR *cref_chmap_table(const char *name, INT *bytes) {
#define T(x)                                                  \
  if (strcmp(name, #x) == 0) {                                \
    *bytes = (INT)sizeof(ref_chmap::x);                       \
    return ref_chmap::x;                                      \
  }
  CHMAP_TABLES(T)
#undef T
  *bytes = 0;
  return NULL;
}

/* Entry i of mapInfoTabDflt (wg4 == 0) or FDK_mapInfoTabWg4 (wg4 != 0) of the copy: returns the
   name of the channel map it points to (NULL if unknown) and its numChannels; *len is the length
   of the table. */
const char *cref_chmap_info(INT wg4, INT i, INT *numChannels, INT *len) {
  const ref_chmap::CHANNEL_MAP_INFO *t = wg4 ? ref_chmap::FDK_mapInfoTabWg4 : ref_chmap::mapInfoTabDflt;
  *len = wg4 ? (INT)(sizeof(ref_chmap::FDK_mapInfoTabWg4) / sizeof(ref_chmap::CHANNEL_MAP_INFO))
             : (INT)(sizeof(ref_chmap::mapInfoTabDflt) / sizeof(ref_chmap::CHANNEL_MAP_INFO));
  *numChannels = t[i].numChannels;
#define T(x) \
  if (t[i].pChannelMap == ref_chmap::x) return #x;
  CHMAP_TABLES(T)
#undef T
  return NULL;
}
}
