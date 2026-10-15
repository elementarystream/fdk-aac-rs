// Shims for the differential tests of fdk_aac_rs_tpenc::tpenc_latm.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::tpenc_tpenc_latm (see build.rs).
//
// The functions run on a copy of tpenc_latm.cpp in a namespace (same source and flags as the
// library; the static functions are reachable there). LATM_STREAM is passed as its byte image
// (88 bytes). Its pointer config[0][0] is stored in the image as 1 (not NULL) or 0 (NULL), as the
// port does; a set pointer refers to the configuration passed with the call (`ccv`, `raw`),
// which is returned afterwards.

#include "cref_common.h"
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "tp_data.h"
#include "tpenc_lib.h"

namespace ref_tpenc_latm {
#include "libMpegTPEnc/src/tpenc_latm.cpp"
}

typedef ref_tpenc_latm::LATM_STREAM LATM;

/* tpenc_tpenc_asc.cpp */
void tpenc_cc_from(CODER_CONFIG *cc, const INT *v, const UCHAR *raw);
void tpenc_cc_to(const CODER_CONFIG *cc, INT *v, UCHAR *raw);
void tpenc_cb_make(CSTpCallBacks *cb, void *rec, INT has);

struct LatmCall {
  LATM l;
  CODER_CONFIG cc;
  CSTpCallBacks cb;
  UCHAR *img;
  INT *ccv;
  UCHAR *raw;

  LatmCall(UCHAR *img_, INT *ccv_, UCHAR *raw_, void *rec, INT has)
      : img(img_), ccv(ccv_), raw(raw_) {
    FDKmemcpy(&l, img, sizeof(l));
    if (ccv != NULL) tpenc_cc_from(&cc, ccv, raw);
    /* config[0][0] (pointer-sized: 8 bytes at 32 on LP64, 4 bytes at 28 on ILP32) */
    size_t p;
    FDKmemcpy(&p, img + offsetof(LATM, config), sizeof(p));
    l.config[0][0] = (p != 0) ? &cc : NULL;
    tpenc_cb_make(&cb, rec, has);
  }
  ~LatmCall() {
    size_t p = (l.config[0][0] != NULL) ? 1 : 0;
    FDKmemcpy(img, &l, sizeof(l));
    FDKmemcpy(img + offsetof(LATM, config), &p, sizeof(p));
    if (ccv != NULL) tpenc_cc_to(&cc, ccv, raw);
  }
};

extern "C" {
INT cref_tpenc_latm_size(void) { return (INT)sizeof(LATM); }
UINT cref_tpenc_latm_write_value(void *bs, INT value) {
  return ref_tpenc_latm::transportEnc_LatmWriteValue((HANDLE_FDK_BITSTREAM)bs, value);
}
UINT cref_tpenc_latm_count_fix(UCHAR *img) {
  LatmCall c(img, NULL, NULL, NULL, 0);
  return ref_tpenc_latm::transportEnc_LatmCountFixBitDemandHeader(&c.l);
}
UINT cref_tpenc_latm_count_var(UCHAR *img, UINT len) {
  LatmCall c(img, NULL, NULL, NULL, 0);
  return ref_tpenc_latm::transportEnc_LatmCountVarBitDemandHeader(&c.l, len);
}
UINT cref_tpenc_latm_count_total(UCHAR *img, UINT len) {
  LatmCall c(img, NULL, NULL, NULL, 0);
  return ref_tpenc_latm::transportEnc_LatmCountTotalBitDemandHeader(&c.l, len);
}
INT cref_tpenc_latm_create_smc(UCHAR *img, void *bs, INT bf, INT *ccv, UCHAR *raw, void *rec, INT has) {
  LatmCall c(img, ccv, raw, rec, has);
  return ref_tpenc_latm::CreateStreamMuxConfig(&c.l, (HANDLE_FDK_BITSTREAM)bs, bf, &c.cb);
}
INT cref_tpenc_latm_payload_len(void *bs, INT bits) {
  return ref_tpenc_latm::WriteAuPayloadLengthInfo((HANDLE_FDK_BITSTREAM)bs, bits);
}
INT cref_tpenc_latm_set_nr_sub(UCHAR *img, INT n) {
  LatmCall c(img, NULL, NULL, NULL, 0);
  return ref_tpenc_latm::transportEnc_LatmSetNrOfSubframes(&c.l, n);
}
INT cref_tpenc_latm_all_same(UCHAR *img, INT noProgram, INT *ccv, UCHAR *raw) {
  LatmCall c(img, ccv, raw, NULL, 0);
  return ref_tpenc_latm::allStreamsSameTimeFraming(&c.l, (UCHAR)noProgram, c.l.noLayer);
}
INT cref_tpenc_latm_init_stream(UCHAR *img, INT fdp, INT mcp, UINT amv, INT tt, INT *ccv, UCHAR *raw) {
  LatmCall c(img, ccv, raw, NULL, 0);
  return ref_tpenc_latm::transportEnc_InitLatmStream(&c.l, fdp, mcp, amv, (TRANSPORT_TYPE)tt);
}
INT cref_tpenc_latm_advance(UCHAR *img, void *bs, INT au, INT bf, INT *ccv, UCHAR *raw, void *rec, INT has) {
  LatmCall c(img, ccv, raw, rec, has);
  return ref_tpenc_latm::AdvanceAudioMuxElement(&c.l, (HANDLE_FDK_BITSTREAM)bs, au, bf, &c.cb);
}
/* transportEnc_Latm_Init(): the configuration pointer is set to `ccv`/`raw`. */
INT cref_tpenc_latm_init(UCHAR *img, void *bs, INT *ccv, UCHAR *raw, UINT amv, INT tt, void *rec, INT has) {
  LatmCall c(img, ccv, raw, rec, has);
  return ref_tpenc_latm::transportEnc_Latm_Init(&c.l, (HANDLE_FDK_BITSTREAM)bs, &c.cc, amv,
                                                (TRANSPORT_TYPE)tt, &c.cb);
}
INT cref_tpenc_latm_write(UCHAR *img, void *bs, INT au, INT bf, INT *ccv, UCHAR *raw, void *rec, INT has) {
  LatmCall c(img, ccv, raw, rec, has);
  return ref_tpenc_latm::transportEnc_LatmWrite(&c.l, (HANDLE_FDK_BITSTREAM)bs, au, bf, &c.cb);
}
void cref_tpenc_latm_adjust(UCHAR *img, INT *bits) {
  LatmCall c(img, NULL, NULL, NULL, 0);
  ref_tpenc_latm::transportEnc_LatmAdjustSubframeBits(&c.l, bits);
}
INT cref_tpenc_latm_get_frame(UCHAR *img, void *bs, INT *bytes) {
  LatmCall c(img, NULL, NULL, NULL, 0);
  return ref_tpenc_latm::transportEnc_LatmGetFrame(&c.l, (HANDLE_FDK_BITSTREAM)bs, bytes);
}
INT cref_tpenc_latm_add_other(UCHAR *img, INT n) {
  LatmCall c(img, NULL, NULL, NULL, 0);
  return ref_tpenc_latm::transportEnc_LatmAddOtherDataBits(&c.l, n);
}
}

extern "C" {
/* celpFrameLengthTable (file-static) of the copy as memory. */
const SHORT *cref_tpenc_latm_celp_frame_length_table(INT *bytes) {
  *bytes = (INT)sizeof(ref_tpenc_latm::celpFrameLengthTable);
  return ref_tpenc_latm::celpFrameLengthTable;
}
}
