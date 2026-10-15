// Shims for the differential tests of fdk_aac_rs_drc::drc_gain_dec_preprocess.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::drc_drc_gain_dec_preprocess (see build.rs).
//
// The file-static characteristic functions of drcGainDec_preprocess.cpp are reached by
// including the source file into a namespace (PORTING.md, "Differential tests"). Characteristics
// are passed as the memory images of CUSTOM_DRC_CHAR (22 bytes).

#include "cref_common.h"
#include "fixpoint_math.h"
#include "drcDec_types.h"
#include "drcDecoder.h"
#include "drcDec_gainDecoder.h"
#include "drcGainDec_preprocess.h"
#include "drcDec_tools.h"
#include "FDK_matrixCalloc.h"
#include "drcDec_rom.h"

namespace ref_drc_pre {
#include "drcGainDec_preprocess.cpp"
}

static CUSTOM_DRC_CHAR load(const UCHAR *b) {
  CUSTOM_DRC_CHAR c;
  FDKmemcpy(&c, b, sizeof(c));
  return c;
}

extern "C" {
INT cref_drc_pre_get_slope_sign(INT format, const UCHAR *cchar, INT *slopeSign) {
  CUSTOM_DRC_CHAR c = load(cchar);
  return ref_drc_pre::_getSlopeSign((CHARACTERISTIC_FORMAT)format, &c, slopeSign);
}
INT cref_drc_pre_sigmoid(const UCHAR *cchar, FIXP_DBL inLevelDb, FIXP_DBL *outGainDb) {
  CUSTOM_DRC_CHAR c = load(cchar);
  return ref_drc_pre::_compressorIO_sigmoid(&c.sigmoid, inLevelDb, outGainDb);
}
INT cref_drc_pre_sigmoid_inverse(const UCHAR *cchar, FIXP_SGL gainDb, FIXP_DBL *inLev) {
  CUSTOM_DRC_CHAR c = load(cchar);
  return ref_drc_pre::_compressorIO_sigmoid_inverse(&c.sigmoid, gainDb, inLev);
}
INT cref_drc_pre_nodes(const UCHAR *cchar, FIXP_DBL inLevelDb, FIXP_DBL *outGainDb) {
  CUSTOM_DRC_CHAR c = load(cchar);
  return ref_drc_pre::_compressorIO_nodes(&c.nodes, inLevelDb, outGainDb);
}
INT cref_drc_pre_nodes_inverse(const UCHAR *cchar, FIXP_SGL gainDb, FIXP_DBL *inLev) {
  CUSTOM_DRC_CHAR c = load(cchar);
  return ref_drc_pre::_compressorIO_nodes_inverse(&c.nodes, gainDb, inLev);
}
INT cref_drc_pre_map_gain(INT formatSource, const UCHAR *source, INT formatTarget, const UCHAR *target, FIXP_SGL gainInDb, FIXP_DBL *gainOutDb) {
  CUSTOM_DRC_CHAR s = load(source), t = load(target);
  return ref_drc_pre::_mapGain((CHARACTERISTIC_FORMAT)formatSource, &s, (CHARACTERISTIC_FORMAT)formatTarget, &t, gainInDb, gainOutDb);
}
}
