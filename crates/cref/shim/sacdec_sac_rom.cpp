// Shims for the differential tests of fdk_aac_rs_sacdec::sac_rom.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacdec_sac_rom (see build.rs).
//
// The tables of sac_rom.cpp (extern) and the file-static tables of sac_calcM1andM2.cpp and
// sac_tsd.cpp (copies of the files in namespaces, compiled with the library's flags), and the
// macro constants of libSACdec. The function-local table chanelIdx of sac_process.cpp is read
// through mapChannel() of a copy of the file in a namespace.

#include "cref_common.h"
#include "sac_rom.h"
#include "sac_calcM1andM2.h"
#include "sac_bitdec.h"
#include "sac_process.h"
#include "sac_smoothing.h"
#include "sac_stp.h"
#include "sac_tsd.h"
#include "FDK_trigFcts.h"
#include "FDK_matrixCalloc.h"

namespace ref_sac_rom {
#include "libSACdec/src/sac_rom.cpp"
}
namespace ref_sac_calc {
#include "libSACdec/src/sac_calcM1andM2.cpp"
}
namespace ref_sac_tsd {
#include "libSACdec/src/sac_tsd.cpp"
}
#include "sac_dec_errorcodes.h"
#include "FDK_decorrelate.h"
namespace ref_sac_process {
#include "libSACdec/src/sac_process.cpp"
}
/* sac_stp.cpp defines struct STP_DEC, which sac_dec.h declares at global scope: the copy is
   included at global scope with its external functions renamed (for its macro constants). */
#define subbandTPCreate ref_rom_subbandTPCreate
#define subbandTPInit ref_rom_subbandTPInit
#define subbandTPApply ref_rom_subbandTPApply
#define subbandTPDestroy ref_rom_subbandTPDestroy
#include "libSACdec/src/sac_stp.cpp"
#undef subbandTPCreate
#undef subbandTPInit
#undef subbandTPApply
#undef subbandTPDestroy

/* Copies n scalars of any integer type to out. */
template <typename T>
static INT copy_vals(const T *p, INT n, INT *out) {
  for (INT i = 0; i < n; i++) out[i] = (INT)p[i];
  return n;
}
#define TAB(sym) copy_vals(&(sym)[0], (INT)(sizeof(sym) / sizeof((sym)[0])), out)
#define TAB2(sym) \
  copy_vals(&(sym)[0][0], (INT)(sizeof(sym) / sizeof((sym)[0][0])), out)
#define TAB3(sym) \
  copy_vals(&(sym)[0][0][0], (INT)(sizeof(sym) / sizeof((sym)[0][0][0])), out)

extern "C" {
/* The tables in the order of the test (id), as INT values; returns the number of values. */
INT cref_sacrom_table(INT id, INT *out) {
  switch (id) {
    case 0: return TAB(ref_sac_rom::dequantCPC__FDK); /* declared without size */
    case 1: return TAB(dequantICC__FDK);
    case 2: return TAB(dequantCLD__FDK);
    case 3: return TAB(dequantIPD__FDK);
    case 4: return TAB3(dequantIPD_CLD_ICC_splitAngle__FDK);
    case 5: return TAB(ref_sac_rom::dequantCLD_c_l); /* not declared in sac_rom.h */
    case 6: return TAB2(H11_nc);
    case 7: return TAB2(H12_nc);
    case 8: return TAB(dequantCLD_c1);
    case 9: return copy_vals(&BP__FDK[0], 3, out);
    case 10: return copy_vals(&BP_GF__FDK[0], 19, out);
    case 11: return TAB(samplingFreqTable);
    case 12: return copy_vals(&freqResTable[0], 8, out);
    case 13: return copy_vals(&freqResTable_LD[0], 8, out);
    case 14: return copy_vals(&tempShapeChanTable[0][0], 16, out);
    case 15: return TAB(kernels_4_to_71);
    case 16: return TAB(kernels_5_to_71);
    case 17: return TAB(kernels_7_to_71);
    case 18: return TAB(kernels_10_to_71);
    case 19: return TAB(kernels_14_to_71);
    case 20: return TAB(kernels_20_to_71);
    case 21: return TAB(kernels_28_to_71);
    case 22: return TAB(kernels_4_to_64);
    case 23: return TAB(kernels_5_to_64);
    case 24: return TAB(kernels_7_to_64);
    case 25: return TAB(kernels_9_to_64);
    case 26: return TAB(kernels_12_to_64);
    case 27: return TAB(kernels_15_to_64);
    case 28: return TAB(kernels_23_to_64);
    case 29: return TAB(mapping_15_to_23);
    case 30: return TAB(mapping_12_to_23);
    case 31: return TAB(mapping_9_to_23);
    case 32: return TAB(mapping_7_to_23);
    case 33: return TAB(mapping_5_to_23);
    case 34: return TAB(mapping_4_to_23);
    case 35: return copy_vals(&clipGainTable__FDK[0], 8, out);
    case 36: return copy_vals(&clipGainSFTable__FDK[0], 8, out);
    case 37: return copy_vals(&pbStrideTable[0], 4, out);
    case 38: return copy_vals(&smgTimeTable[0], 4, out);
    case 39: return TAB2(envShapeDataTable__FDK);
    case 40: return copy_vals(&row2channelSTP[0][0], 16, out);
    case 41: return copy_vals(&row2channelGES[0][0], 16, out);
    case 42: return copy_vals(&row2residual[0][0], 16, out);
    case 43: {
      INT n = 0;
      for (int i = 0; i < 8; i++) {
        out[n++] = treePropertyTable[i].numInputChannels;
        out[n++] = treePropertyTable[i].numOutputChannels;
        out[n++] = treePropertyTable[i].numOttBoxes;
        out[n++] = treePropertyTable[i].numTttBoxes;
        for (int j = 0; j < MAX_NUM_OTT; j++) out[n++] = treePropertyTable[i].ottModeLfe[j];
      }
      return n;
    }
    /* sac_calcM1andM2.cpp */
    case 44: return TAB(ref_sac_calc::sinIpd_tab);
    case 45: return TAB(ref_sac_calc::sqrt_one_minus_ICC2);
    case 46: return TAB(ref_sac_calc::sqrt_CLD_e);
    case 47: return TAB(ref_sac_calc::sqrt_CLD_m);
    case 48: return TAB(ref_sac_calc::CLD_m);
    /* sac_tsd.cpp */
    case 49: return TAB(ref_sac_tsd::nBitsTsdCW_32slots);
    case 50: return TAB(ref_sac_tsd::nBitsTsdCW_64slots);
    case 51: {
      INT n = 0;
      for (int i = 0; i < 8; i++) {
        out[n++] = ref_sac_tsd::phiTsd[i].v.re;
        out[n++] = ref_sac_tsd::phiTsd[i].v.im;
      }
      return n;
    }
    default:
      return -1;
  }
}

/* The macro constants of libSACdec in the order of the test. */
INT cref_sacrom_consts(INT *out) {
  INT n = 0;
  out[n++] = PI__IPD;
  out[n++] = PIx2__IPD;
  out[n++] = SF_SCALE_LD64;
  out[n++] = STP_LPF_COEFF1__FDK;
  out[n++] = ONE_MINUS_STP_LPF_COEFF1__FDK;
  out[n++] = STP_LPF_COEFF2__FDK;
  out[n++] = ONE_MINUS_STP_LPF_COEFF2__FDK;
  out[n++] = STP_SCALE_LIMIT__FDK;
  out[n++] = ONE_DIV_STP_SCALE_LIMIT__FDK;
  out[n++] = ABS_THR__FDK;
  out[n++] = ABS_THR2__FDK;
  out[n++] = STP_SCALE_LIMIT_HI;
  out[n++] = STP_SCALE_LIMIT_LO;
  out[n++] = STP_SCALE_LIMIT_HI_LD64;
  out[n++] = STP_SCALE_LIMIT_LO_LD64;
  return n;
}

void cref_sacrom_get_cld_values(INT index, FIXP_DBL *out) {
  SpatialDequantGetCLDValues(index, &out[0], &out[1]);
}
FIXP_DBL cref_sacrom_get_cld2_values(INT index) {
  FIXP_DBL x;
  SpatialDequantGetCLD2Values(index, &x);
  return x;
}
/* chanelIdx[0][ch] of mapChannel() (sac_process.cpp; the function ignores self) */
INT cref_sacrom_map_channel(UINT ch) { return (INT)ref_sac_process::mapChannel(NULL, ch); }
}
