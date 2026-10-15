// Shims for the differential tests of fdk_aac_rs_sbrenc::ps_bitenc.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_ps_bitenc (see build.rs).
//
// The bitstreams are written into caller buffers of 1024 bytes (or only counted).

#include "cref_common.h"
#include "libSBRenc/src/ps_bitenc.h"

/* A PS_OUT from the INTs of sbrdump_ps_out(). */
static void ps_out_from(PS_OUT *o, const INT *v) {
  INT k = 0;
  o->enablePSHeader = v[k++];
  o->enableIID = v[k++];
  o->iidMode = v[k++];
  o->enableICC = v[k++];
  o->iccMode = v[k++];
  o->enableIpdOpd = v[k++];
  o->frameClass = v[k++];
  o->nEnvelopes = v[k++];
  for (INT i = 0; i < PS_MAX_ENVELOPES; i++) o->frameBorder[i] = v[k++];
  PS_DELTA *delta[4] = {o->deltaIID, o->deltaICC, o->deltaIPD, o->deltaOPD};
  INT(*val[4])[PS_MAX_BANDS] = {o->iid, o->icc, o->ipd, o->opd};
  INT *last[4] = {o->iidLast, o->iccLast, o->ipdLast, o->opdLast};
  for (INT p = 0; p < 4; p++) {
    for (INT i = 0; i < PS_MAX_ENVELOPES; i++) delta[p][i] = (PS_DELTA)v[k++];
    for (INT i = 0; i < PS_MAX_ENVELOPES; i++)
      for (INT b = 0; b < PS_MAX_BANDS; b++) val[p][i][b] = v[k++];
    for (INT b = 0; b < PS_MAX_BANDS; b++) last[p][b] = v[k++];
  }
}

extern "C" {
/* FDKsbrEnc_WritePSBitstream(); count != 0 passes a NULL bitstream. */
INT cref_ps_bitenc_write(INT *psOut, UCHAR *buf, INT count, INT *validBits) {
  PS_OUT o;
  ps_out_from(&o, psOut);
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, 1024, 0, BS_WRITER);
  INT n = FDKsbrEnc_WritePSBitstream(&o, count ? NULL : &bs);
  FDKsyncCache(&bs);
  *validBits = (INT)FDKgetValidBits(&bs);
  return n;
}
/* which: 0 iid, 1 icc, 2 ipd, 3 opd; valLast may be all values (NULL is not tested). */
INT cref_ps_bitenc_encode(INT which, INT count, INT *val, INT *valLast, INT nBands, INT res, INT mode, INT *error, UCHAR *buf, INT *validBits) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, 1024, 0, BS_WRITER);
  HANDLE_FDK_BITSTREAM h = count ? NULL : &bs;
  INT n = 0;
  switch (which) {
    case 0:
      n = FDKsbrEnc_EncodeIid(h, val, valLast, nBands, (PS_IID_RESOLUTION)res, (PS_DELTA)mode, error);
      break;
    case 1:
      n = FDKsbrEnc_EncodeIcc(h, val, valLast, nBands, (PS_DELTA)mode, error);
      break;
    case 2:
      n = FDKsbrEnc_EncodeIpd(h, val, valLast, nBands, (PS_DELTA)mode, error);
      break;
    default:
      n = FDKsbrEnc_EncodeOpd(h, val, valLast, nBands, (PS_DELTA)mode, error);
      break;
  }
  FDKsyncCache(&bs);
  *validBits = (INT)FDKgetValidBits(&bs);
  return n;
}
}
