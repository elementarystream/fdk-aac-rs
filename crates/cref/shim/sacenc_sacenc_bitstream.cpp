// Shims for the differential tests of fdk_aac_rs_sacenc::sacenc_bitstream.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacenc_sacenc_bitstream (see build.rs).
//
// The non-static functions are those of the library (libfdkaac_ref.a); the file-static ones
// are reached through the copy in namespace rsb. Structures are passed as byte images:
// SPATIALFRAME (199 bytes), PREV_OTTDATA (50 bytes), LOSSLESSDATA (8 bytes), FRAMINGINFO
// (4 bytes), SMGDATA (52 bytes), SPATIALSPECIFICCONFIG (12 INTs). The state of a bitstream is
// returned as in sacenc_sacenc_nlc_enc.cpp.

#include "cref_common.h"
#include <stdlib.h>
#include "genericStds.h"
#include "common_fix.h"
#include "FDK_audio.h"
#include "FDK_bitstream.h"
#include "FDK_matrixCalloc.h"
#include "FDK_trigFcts.h"
#include "fixpoint_math.h"
#include "FDK_core.h"
#include "qmf.h"

/* The whole of libSACenc in namespace rsb (its headers included there first), so that
   the structures of the .cpp files and the handles that point to them live in the namespace
   with the layout of the library. The extern "C" API functions of the copy are renamed. */
#define FDK_sacenc_open rsb_FDK_sacenc_open
#define FDK_sacenc_init rsb_FDK_sacenc_init
#define FDK_sacenc_close rsb_FDK_sacenc_close
#define FDK_sacenc_encode rsb_FDK_sacenc_encode
#define FDK_sacenc_getInfo rsb_FDK_sacenc_getInfo
#define FDK_sacenc_setParam rsb_FDK_sacenc_setParam
#define FDK_sacenc_getLibInfo rsb_FDK_sacenc_getLibInfo
namespace rsb {
#include "sacenc_lib.h"
#include "libSACenc/src/sacenc_const.h"
#include "libSACenc/src/sacenc_bitstream.h"
#include "libSACenc/src/sacenc_nlc_enc.h"
#include "libSACenc/src/sacenc_huff_tab.h"
#include "libSACenc/src/sacenc_vectorfunctions.h"
#include "libSACenc/src/sacenc_paramextract.h"
#include "libSACenc/src/sacenc_framewindowing.h"
#include "libSACenc/src/sacenc_tree.h"
#include "libSACenc/src/sacenc_onsetdetect.h"
#include "libSACenc/src/sacenc_filter.h"
#include "libSACenc/src/sacenc_staticgain.h"
#include "libSACenc/src/sacenc_delay.h"
#include "libSACenc/src/sacenc_dmx_tdom_enh.h"
/* sacenc_dmx_tdom_enh.cpp overloads these functions at file scope; in the namespace its
   overloads would hide the global ones. */
using ::invSqrtNorm2;
using ::sqrtFixp;
#include "libSACenc/src/sacenc_bitstream.cpp"
#include "libSACenc/src/sacenc_delay.cpp"
#include "libSACenc/src/sacenc_dmx_tdom_enh.cpp"
#include "libSACenc/src/sacenc_filter.cpp"
#include "libSACenc/src/sacenc_framewindowing.cpp"
#include "libSACenc/src/sacenc_huff_tab.cpp"
#include "libSACenc/src/sacenc_lib.cpp"
#include "libSACenc/src/sacenc_nlc_enc.cpp"
#include "libSACenc/src/sacenc_onsetdetect.cpp"
#include "libSACenc/src/sacenc_paramextract.cpp"
#include "libSACenc/src/sacenc_staticgain.cpp"
#include "libSACenc/src/sacenc_tree.cpp"
#include "libSACenc/src/sacenc_vectorfunctions.cpp"
}
#undef FDK_sacenc_open
#undef FDK_sacenc_init
#undef FDK_sacenc_close
#undef FDK_sacenc_encode
#undef FDK_sacenc_getInfo
#undef FDK_sacenc_setParam
#undef FDK_sacenc_getLibInfo
/* The global declarations of the library (the guards were set inside the namespace). */
#undef SACENC_LIB_H
#undef SACENC_CONST_H
#undef SACENC_BITSTREAM_H
#undef SACENC_NLC_ENC_H
#undef SACENC_HUFF_TAB_H
#undef SACENC_VECTORFUNCTIONS_H
#undef SACENC_PARAMEXTRACT_H
#undef SACENC_FRAMEWINDOWING_H
#undef SACENC_TREE_H
#undef SACENC_ONSETDETECT_H
#undef SACENC_FILTER_H
#undef SACENC_STATICGAIN_H
#undef SACENC_DELAY_H
#undef SACENC_DMX_TDOM_ENH_H
#include "sacenc_lib.h"
#include "libSACenc/src/sacenc_const.h"
#include "libSACenc/src/sacenc_bitstream.h"
#include "libSACenc/src/sacenc_nlc_enc.h"
#include "libSACenc/src/sacenc_huff_tab.h"
#include "libSACenc/src/sacenc_vectorfunctions.h"
#include "libSACenc/src/sacenc_paramextract.h"
#include "libSACenc/src/sacenc_framewindowing.h"
#include "libSACenc/src/sacenc_tree.h"
#include "libSACenc/src/sacenc_onsetdetect.h"
#include "libSACenc/src/sacenc_filter.h"
#include "libSACenc/src/sacenc_staticgain.h"
#include "libSACenc/src/sacenc_delay.h"
#include "libSACenc/src/sacenc_dmx_tdom_enh.h"

void sacenc_bs_begin(FDK_BITSTREAM *bs, UCHAR *buf, UINT size, INT pre, UINT preval);
void sacenc_bs_state(FDK_BITSTREAM *bs, UINT *out);

static rsb::BSF_INSTANCE *bsf(void *h) { return (rsb::BSF_INSTANCE *)h; }

extern "C" {
/* sizeof and offsetof of the structures, to check the byte images. */
INT cref_sacenc_bitstream_layout(INT *out) {
  out[0] = sizeof(SPATIALFRAME);
  out[1] = offsetof(SPATIALFRAME, bsIndependencyFlag);
  out[2] = offsetof(SPATIALFRAME, ottData);
  out[3] = offsetof(SPATIALFRAME, smgData);
  out[4] = offsetof(SPATIALFRAME, tempShapeData);
  out[5] = offsetof(SPATIALFRAME, CLDLosslessData);
  out[6] = offsetof(SPATIALFRAME, ICCLosslessData);
  out[7] = offsetof(SPATIALFRAME, bUseBBCues);
  out[8] = sizeof(rsb::PREV_OTTDATA);
  out[9] = sizeof(SPATIALSPECIFICCONFIG);
  out[10] = offsetof(rsb::BSF_INSTANCE, frame);
  out[11] = offsetof(rsb::BSF_INSTANCE, prevFrameData);
  out[12] = sizeof(rsb::BSF_INSTANCE);
  return 13;
}
/* fdk_sacenc_duplicateParameterSet of the library on SPATIALFRAME images. */
INT cref_sacenc_bitstream_duplicateParameterSet(const UCHAR *from, INT setFrom, UCHAR *to, INT setTo) {
  SPATIALFRAME f, t;
  FDKmemcpy(&f, from, sizeof(f));
  FDKmemcpy(&t, to, sizeof(t));
  INT r = fdk_sacenc_duplicateParameterSet(&f, setFrom, &t, setTo);
  FDKmemcpy(to, &t, sizeof(t));
  return r;
}
INT cref_sacenc_bitstream_DuplicateLosslessData(INT startBox, INT stopBox, const UCHAR *from, INT setFrom, UCHAR *to, INT setTo) {
  rsb::LOSSLESSDATA f, t;
  FDKmemcpy(&f, from, sizeof(f));
  FDKmemcpy(&t, to, sizeof(t));
  INT r = rsb::DuplicateLosslessData(startBox, stopBox, &f, setFrom, &t, setTo);
  FDKmemcpy(to, &t, sizeof(t));
  return r;
}
void cref_sacenc_bitstream_clearFrame(UCHAR *frame) {
  rsb::SPATIALFRAME f;
  FDKmemcpy(&f, frame, sizeof(f));
  rsb::clearFrame(&f);
  FDKmemcpy(frame, &f, sizeof(f));
}
void cref_sacenc_bitstream_fine2coarse(SCHAR *data, INT dataType, INT startBand, INT numBands) {
  rsb::fine2coarse(data, (rsb::DATA_TYPE)dataType, startBand, numBands);
}
void cref_sacenc_bitstream_coarse2fine(SCHAR *data, INT dataType, INT startBand, INT numBands) {
  rsb::coarse2fine(data, (rsb::DATA_TYPE)dataType, startBand, numBands);
}
UCHAR cref_sacenc_bitstream_getBsFreqResStride(INT index) { return rsb::getBsFreqResStride(index); }
INT cref_sacenc_bitstream_getBsFreqResIndex(INT numBands, INT *idx) { return rsb::getBsFreqResIndex(numBands, idx); }
INT cref_sacenc_bitstream_getSamplingFrequencyIndex(INT fs, INT *idx) { return rsb::getSamplingFrequencyIndex(fs, idx); }
/* ecData: data 2 x 23 SCHAR, oldData 23, quantCoarseXXXprev 2, losslessData image. */
void cref_sacenc_bitstream_ecData(UCHAR *buf, UINT size, INT pre, UINT preval, const SCHAR *data, SCHAR *oldData, UCHAR *qprev, UCHAR *lossless, INT dataType, INT paramIdx, INT numParamSets, INT indep, INT startBand, INT stopBand, INT defaultValue, UINT *state) {
  FDK_BITSTREAM bs;
  SCHAR d[MAX_NUM_PARAMS][MAX_NUM_BINS];
  rsb::LOSSLESSDATA l;
  FDKmemcpy(d, data, sizeof(d));
  FDKmemcpy(&l, lossless, sizeof(l));
  sacenc_bs_begin(&bs, buf, size, pre, preval);
  rsb::ecData(&bs, d, oldData, qprev, &l, (rsb::DATA_TYPE)dataType, paramIdx, numParamSets, indep,
              startBand, stopBand, defaultValue);
  FDKmemcpy(lossless, &l, sizeof(l));
  sacenc_bs_state(&bs, state);
}
INT cref_sacenc_bitstream_writeFramingInfo(UCHAR *buf, UINT size, INT pre, UINT preval, const UCHAR *fi, INT frameLength, UINT *state) {
  FDK_BITSTREAM bs;
  rsb::FRAMINGINFO f;
  FDKmemcpy(&f, fi, sizeof(f));
  sacenc_bs_begin(&bs, buf, size, pre, preval);
  INT r = rsb::writeFramingInfo(&bs, &f, frameLength);
  sacenc_bs_state(&bs, state);
  return r;
}
INT cref_sacenc_bitstream_writeSmgData(UCHAR *buf, UINT size, INT pre, UINT preval, const UCHAR *smg, INT numParamSets, INT dataBands, UINT *state) {
  FDK_BITSTREAM bs;
  rsb::SMGDATA s;
  FDKmemcpy(&s, smg, sizeof(s));
  sacenc_bs_begin(&bs, buf, size, pre, preval);
  INT r = rsb::writeSmgData(&bs, &s, numParamSets, dataBands);
  sacenc_bs_state(&bs, state);
  return r;
}
/* fdk_sacenc_writeSpatialSpecificConfig of the library. */
INT cref_sacenc_bitstream_writeSpatialSpecificConfig(const INT *ssc, UCHAR *buf, INT size, INT *nbits) {
  SPATIALSPECIFICCONFIG c;
  FDKmemcpy(&c, ssc, sizeof(c));
  return fdk_sacenc_writeSpatialSpecificConfig(&c, buf, size, nbits);
}

/* A bitstream formatter of the library. */
void *cref_sacenc_bitstream_create(void) {
  HANDLE_BSF_INSTANCE h = NULL;
  if (fdk_sacenc_createSpatialBitstreamEncoder(&h) != SACENC_OK) return NULL;
  return h;
}
INT cref_sacenc_bitstream_create_null(void) { return fdk_sacenc_createSpatialBitstreamEncoder(NULL); }
INT cref_sacenc_bitstream_destroy(void *h) {
  HANDLE_BSF_INSTANCE p = (HANDLE_BSF_INSTANCE)h;
  return fdk_sacenc_destroySpatialBitstreamEncoder(&p);
}
INT cref_sacenc_bitstream_init(void *h) { return fdk_sacenc_initSpatialBitstreamEncoder((HANDLE_BSF_INSTANCE)h); }
/* Sets the SpatialSpecificConfig, the frame and the previous data of the instance. */
void cref_sacenc_bitstream_set(void *h, const INT *ssc, const UCHAR *frame, const UCHAR *prev) {
  FDKmemcpy(fdk_sacenc_getSpatialSpecificConfig((HANDLE_BSF_INSTANCE)h), ssc, sizeof(SPATIALSPECIFICCONFIG));
  FDKmemcpy(fdk_sacenc_getSpatialFrame((HANDLE_BSF_INSTANCE)h, WRITE_SPATIALFRAME), frame, sizeof(SPATIALFRAME));
  FDKmemcpy(&bsf(h)->prevFrameData.prevOttData, prev, sizeof(rsb::PREV_OTTDATA));
}
void cref_sacenc_bitstream_get(void *h, INT *ssc, UCHAR *frame, UCHAR *prev) {
  FDKmemcpy(ssc, fdk_sacenc_getSpatialSpecificConfig((HANDLE_BSF_INSTANCE)h), sizeof(SPATIALSPECIFICCONFIG));
  FDKmemcpy(frame, fdk_sacenc_getSpatialFrame((HANDLE_BSF_INSTANCE)h, READ_SPATIALFRAME), sizeof(SPATIALFRAME));
  FDKmemcpy(prev, &bsf(h)->prevFrameData.prevOttData, sizeof(rsb::PREV_OTTDATA));
}
/* fdk_sacenc_getSpatialFrame with an arbitrary type: 1 if it returns the frame, 0 for NULL. */
INT cref_sacenc_bitstream_getSpatialFrame(void *h, INT type) {
  SPATIALFRAME *f = fdk_sacenc_getSpatialFrame((HANDLE_BSF_INSTANCE)h, (SPATIALFRAME_TYPE)type);
  return f == NULL ? 0 : ((void *)f == (void *)&bsf(h)->frame ? 1 : 2);
}
INT cref_sacenc_bitstream_writeSpatialFrame(void *h, UCHAR *buf, INT size, INT *nbits) {
  return fdk_sacenc_writeSpatialFrame(buf, size, nbits, (HANDLE_BSF_INSTANCE)h);
}
}

extern "C" {
/* The file-static tables as INTs: 0 SampleRateTable, 1 FreqResBinTable_LD,
   2 FreqResStrideTable_LD. Returns the number of entries. */
INT cref_sacenc_bitstream_table(INT id, INT *out) {
  INT n = 0;
  switch (id) {
    case 0:
      n = sizeof(rsb::SampleRateTable) / sizeof(INT);
      for (INT i = 0; i < n; i++) out[i] = rsb::SampleRateTable[i];
      break;
    case 1:
      n = sizeof(rsb::FreqResBinTable_LD);
      for (INT i = 0; i < n; i++) out[i] = rsb::FreqResBinTable_LD[i];
      break;
    default:
      n = sizeof(rsb::FreqResStrideTable_LD);
      for (INT i = 0; i < n; i++) out[i] = rsb::FreqResStrideTable_LD[i];
      break;
  }
  return n;
}
}
