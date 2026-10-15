// Shims for the differential tests of fdk_aac_rs_fdk::syslib_channel_map_descr.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_syslib_channel_map_descr (see build.rs).

#include "cref_common.h"
#include "syslib_channelMapDescr.h"

extern "C" {
/* getMapValue of the default table (FDK_chMapDescr_init with NULL). */
UCHAR cref_chmap_default_get(UINT pass, UCHAR ch, UINT mapIdx) {
  FDK_channelMapDescr d;
  FDK_chMapDescr_init(&d, NULL, 0, pass);
  return FDK_chMapDescr_getMapValue(&d, ch, mapIdx);
}

/* getMapValue of FDK_mapInfoTabWg4. */
UCHAR cref_chmap_wg4_get(UINT pass, UCHAR ch, UINT mapIdx) {
  FDK_channelMapDescr d;
  FDK_chMapDescr_init(&d, FDK_mapInfoTabWg4, FDK_mapInfoTabLenWg4, pass);
  return FDK_chMapDescr_getMapValue(&d, ch, mapIdx);
}

UINT cref_chmap_wg4_len(void) { return FDK_mapInfoTabLenWg4; }

/* FDK_chMapDescr_isValid of a descriptor with a single channel map. */
INT cref_chmap_is_valid_single(const UCHAR *map, UCHAR numCh) {
  CHANNEL_MAP_INFO info = {map, numCh};
  FDK_channelMapDescr d = {&info, 1, 0};
  return FDK_chMapDescr_isValid(&d);
}

INT cref_chmap_is_valid_null(void) { return FDK_chMapDescr_isValid(NULL); }

/* FDK_chMapDescr_init with a custom table of one channel map and the length len (0 or 1), then
   FDK_chMapDescr_setPassThrough(pass2) (result in *err) and getMapValue; returns 1 if the
   default table was chosen. */
INT cref_chmap_init_custom(const UCHAR *map, UCHAR numCh, UINT len, UINT pass, UINT pass2, UCHAR ch, UINT mapIdx, UCHAR *value, INT *err) {
  static CHANNEL_MAP_INFO info;
  info.pChannelMap = map;
  info.numChannels = numCh;
  FDK_channelMapDescr d;
  FDK_chMapDescr_init(&d, &info, len, pass);
  *err = FDK_chMapDescr_setPassThrough(&d, pass2);
  *value = FDK_chMapDescr_getMapValue(&d, ch, mapIdx);
  return d.pMapInfoTab != &info;
}

/* setPassThrough on a descriptor without a table. */
INT cref_chmap_set_pass_through_empty(UINT pass) {
  FDK_channelMapDescr d = {NULL, 0, 0};
  return FDK_chMapDescr_setPassThrough(&d, pass);
}
}
