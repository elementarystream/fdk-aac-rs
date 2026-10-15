// Shims for the differential tests of fdk_aac_rs_sbrdec::psdec and psbitdec
// (tests/sbrdec/psdec.rs). One PS decoder instance lives in the shim; its state is compared
// with the state dump of shim/sbrdec_e2e.cpp (cref_sbrdec_e2e_dump_ps). PS_DEC_COEFFICIENTS is
// passed as memory (PsDecCoefficients has its layout); QMF data as flat arrays of slots of 64
// values.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrdec_psdec (see build.rs).

#include "cref_common.h"
#include "libSBRdec/src/psdec.h"
#include "libSBRdec/src/psbitdec.h"

SBR_ERROR ResetPsDec(HANDLE_PS_DEC h_ps_d); /* (declared in psbitdec.cpp, not in a header) */

static HANDLE_PS_DEC g_ps;

extern "C" {

void *cref_sbrdec_ps_handle(void) { return g_ps; }

INT cref_sbrdec_ps_create(INT aacSamplesPerFrame) { return CreatePsDec(&g_ps, aacSamplesPerFrame); }

INT cref_sbrdec_ps_delete(void) { return DeletePsDec(&g_ps); }

INT cref_sbrdec_ps_reset(void) { return ResetPsDec(g_ps); }

/* The slot indices and flags that sbrdecoder.cpp and sbr_dec.cpp set directly. */
void cref_sbrdec_ps_set(INT bsLastSlot, INT bsReadSlot, INT processSlot, INT psDecodedPrv, INT procFrameBased) {
  g_ps->bsLastSlot = (UCHAR)bsLastSlot;
  g_ps->bsReadSlot = (UCHAR)bsReadSlot;
  g_ps->processSlot = (UCHAR)processSlot;
  g_ps->psDecodedPrv = (UCHAR)psDecodedPrv;
  g_ps->procFrameBased = (SCHAR)procFrameBased;
}

/* ReadPsData (with the shim's instance, or NULL); *bitsLeft: valid bits after the call */
UINT cref_sbrdec_ps_read(INT withInstance, UCHAR *buf, INT bufSize, INT validBits, INT nBitsLeft, INT *bitsLeft) {
  FDK_BITSTREAM bs;
  FDKinitBitStream(&bs, buf, bufSize, validBits, BS_READER);
  UINT r = ReadPsData(withInstance ? g_ps : NULL, &bs, nBitsLeft);
  *bitsLeft = (INT)FDKgetValidBits(&bs);
  return r;
}

INT cref_sbrdec_ps_decode(INT frameError, void *coef) { return DecodePs(g_ps, (UCHAR)frameError, (PS_DEC_COEFFICIENTS *)coef); }

/* The PS part of sbr_dec(): PreparePsProcessing, then for every slot initSlotBasedRotation at
   the envelope borders and ApplyPsSlot. re/im: the left channel (noCols + 6 slots, modified),
   outRe/outIm: the right channel (noCols slots). */
void cref_sbrdec_ps_frame(FIXP_DBL *re, FIXP_DBL *im, INT noCols, INT sfOv, INT sfNoOv, INT sfHigh, INT highSubband, INT lsb, INT usb, INT ovLen, FIXP_DBL *outRe, FIXP_DBL *outIm) {
  FIXP_DBL *r[64 + 6], *i[64 + 6];
  for (int k = 0; k < noCols + 6 && k < 70; k++) {
    r[k] = re + 64 * k;
    i[k] = im + 64 * k;
  }
  PreparePsProcessing(g_ps, r, i, sfOv);
  int env = 0;
  for (int k = 0; k < noCols; k++) {
    if (k == g_ps->bsData[g_ps->processSlot].mpeg.aEnvStartStop[env]) {
      initSlotBasedRotation(g_ps, env, highSubband);
      env++;
    }
    ApplyPsSlot(g_ps, r + k, i + k, outRe + 64 * k, outIm + 64 * k, sfNoOv, (k < ovLen) ? sfOv : sfNoOv, sfHigh, lsb, usb);
  }
}

}  // extern "C"
