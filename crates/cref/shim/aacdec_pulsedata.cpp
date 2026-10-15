// Shims for the differential tests of fdk_aac_rs_aacdec::pulsedata.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacdec_pulsedata (see build.rs).

#include "cref_common.h"
#include "libAACdec/src/channelinfo.h"
#include "libAACdec/src/pulsedata.h"

/* CPulseData <-> INT[11]: PulseDataPresent, NumberPulse, PulseStartBand, PulseOffset[4],
   PulseAmp[4]. */
static void pd_from(CPulseData *p, const INT *v) {
  p->PulseDataPresent = (UCHAR)v[0];
  p->NumberPulse = (UCHAR)v[1];
  p->PulseStartBand = (UCHAR)v[2];
  for (int i = 0; i < 4; i++) p->PulseOffset[i] = (UCHAR)v[3 + i];
  for (int i = 0; i < 4; i++) p->PulseAmp[i] = (UCHAR)v[7 + i];
}
static void pd_to(const CPulseData *p, INT *v) {
  v[0] = p->PulseDataPresent;
  v[1] = p->NumberPulse;
  v[2] = p->PulseStartBand;
  for (int i = 0; i < 4; i++) v[3 + i] = p->PulseOffset[i];
  for (int i = 0; i < 4; i++) v[7 + i] = p->PulseAmp[i];
}

extern "C" {
/* CPulseData_Read on a bitstream over buf, with MaxSfBands and the window sequence of a
   CIcsInfo. */
INT cref_pulsedata_read(UCHAR *buf, INT bufBytes, INT *pd, const SHORT *sfb, INT maxSfBands, INT windowSequence, INT frameLength, INT *bitsLeft) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, (UINT)bufBytes, (UINT)bufBytes * 8, BS_READER);
  CPulseData p;
  pd_from(&p, pd);
  CIcsInfo ics;
  FDKmemclear(&ics, sizeof(ics));
  ics.MaxSfBands = (UCHAR)maxSfBands;
  ics.WindowSequence = (BLOCK_TYPE)windowSequence;
  INT r = CPulseData_Read(&bs, &p, sfb, &ics, (SHORT)frameLength);
  pd_to(&p, pd);
  *bitsLeft = (INT)FDKgetValidBits(&bs);
  return r;
}

void cref_pulsedata_apply(const INT *pd, const SHORT *offsets, FIXP_DBL *coef) {
  CPulseData p;
  pd_from(&p, pd);
  CPulseData_Apply(&p, offsets, coef);
}
}
