// Shims for the differential tests of fdk_aac_rs_aacdec::delay.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_delay (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/FDK_delay.h"

extern "C" {
void *cref_delay_create(INT delay, INT numChannels, INT *ret) {
  FDK_SignalDelay *d = (FDK_SignalDelay *)FDKcalloc(1, sizeof(FDK_SignalDelay));
  *ret = FDK_Delay_Create(d, (USHORT)delay, (UCHAR)numChannels);
  return d;
}

/* The delay line (delay * num_channels values) and the fields. */
INT cref_delay_state(void *p, FIXP_DBL *line, INT *delay, INT *numChannels) {
  FDK_SignalDelay *d = (FDK_SignalDelay *)p;
  *delay = d->delay;
  *numChannels = d->num_channels;
  INT n = d->delay_line ? d->delay * d->num_channels : 0;
  for (INT i = 0; i < n; i++) line[i] = d->delay_line[i];
  return d->delay_line != NULL;
}

void cref_delay_apply(void *p, FIXP_DBL *time, UINT frameLength, INT channel) { FDK_Delay_Apply((FDK_SignalDelay *)p, time, frameLength, (UCHAR)channel); }

void cref_delay_destroy(void *p) {
  FDK_Delay_Destroy((FDK_SignalDelay *)p);
  FDKfree(p);
}
}
