// Shims for the table tests of fdk_aac_rs_tpdec::tpdec_asc.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::tpdec_tpdec_asc_tables (see build.rs).
//
// tpdec_asc.cpp is compiled here in a namespace together with its headers (they are not included
// at global scope before), for its file-static tables.

#include "cref_common.h"

namespace ref_tpdec_asc {
#include "libMpegTPDec/src/tpdec_asc.cpp"
}

#define TPDEC_ASC_TABLES(T)                                                              \
  T(channel_configuration_0) T(channel_configuration_1) T(channel_configuration_2)       \
  T(channel_configuration_3) T(channel_configuration_4) T(channel_configuration_5)       \
  T(channel_configuration_6) T(channel_configuration_7) T(channel_configuration_8)       \
  T(channel_configuration_9) T(channel_configuration_10) T(channel_configuration_11)     \
  T(channel_configuration_12) T(channel_configuration_13) T(channel_configuration_14)    \
  T(sc_chan_config_tab) T(usacFrameLength) T(sbrRatioIndex)

extern "C" {
/* The file-static table `name` of the copy (ref_tpdec_asc) as memory: channel_configuration_0
   ... channel_configuration_14, sc_chan_config_tab, usacFrameLength, sbrRatioIndex. Returns
   NULL for another name. */
const void *cref_tpdec_asc_table(const char *name, INT *bytes) {
#define T(x)                                 \
  if (strcmp(name, #x) == 0) {               \
    *bytes = (INT)sizeof(ref_tpdec_asc::x); \
    return ref_tpdec_asc::x;                 \
  }
  TPDEC_ASC_TABLES(T)
#undef T
  *bytes = 0;
  return NULL;
}

/* Entry i of channel_configuration_array (the copy): the name of the table it points to (NULL if
   unknown); *len is the length of the array. */
const char *cref_tpdec_asc_channel_configuration_array(INT i, INT *len) {
  *len = (INT)(sizeof(ref_tpdec_asc::channel_configuration_array) /
               sizeof(ref_tpdec_asc::channel_configuration_array[0]));
  if (i < 0 || i >= *len) return NULL;
#define T(x) \
  if ((const void *)ref_tpdec_asc::channel_configuration_array[i] == (const void *)ref_tpdec_asc::x) return #x;
  TPDEC_ASC_TABLES(T)
#undef T
  return NULL;
}
}
