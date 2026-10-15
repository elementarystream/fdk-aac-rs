// Shims for the differential tests of fdk_aac_rs_fdk::fft_rad2.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_fft_rad2 (see build.rs).

#include "cref_common.h"
#include "common_fix.h"
#include "fft_rad2.h"
#include "FDK_tools_rom.h"

extern "C" {
/* dit_fft of the library (arm/fft_rad2_arm.cpp on arm64, fft_rad2.cpp on x86) with one of the
   sine tables: 0 SineTable512, 1 SineTable1024. */
void cref_fft_rad2_dit_fft(FIXP_DBL *x, INT ldn, INT table, INT trigDataSize) {
  dit_fft(x, ldn, table == 0 ? SineTable512 : SineTable1024, trigDataSize);
}
}
