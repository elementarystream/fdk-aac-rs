// Shims for the differential tests of fdk_aac_rs_fdk::core.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_core (see build.rs).

#include "cref_common.h"
#include "FDK_core.h"

extern "C" {
INT cref_core_fdk_module_last(void) { return FDK_MODULE_LAST; }
INT cref_core_fdk_tools(void) { return FDK_TOOLS; }
/* FDK_toolsGetLibInfo() on FDK_MODULE_LAST entries with the given module ids (versionStr filled
   with 'x'), or on NULL. Returns the result; writes per entry the module id, version, flags,
   the 32 bytes of versionStr and the first 31 characters of the title ("" for NULL). */
INT cref_core_get_lib_info(INT null_info, const INT *ids_in, INT *ids, INT *version, UINT *flags, char *version_str, char *title) {
  LIB_INFO info[FDK_MODULE_LAST];
  FDKmemclear(info, sizeof(info));
  for (int i = 0; i < FDK_MODULE_LAST; i++) {
    info[i].module_id = (FDK_MODULE_ID)ids_in[i];
    FDKmemset(info[i].versionStr, 'x', sizeof(info[i].versionStr));
  }
  INT r = FDK_toolsGetLibInfo(null_info ? NULL : info);
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
