// Shims for the differential tests of fdk_aac_rs_fdk::lpc.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_lpc (see build.rs).

#include "cref_common.h"
#include "FDK_lpc.h"

/* A copy of FDK_lpc.cpp, for its file-static table. */
namespace ref_lpc {
#include "libFDK/src/FDK_lpc.cpp"
}

extern "C" {
const SCHAR *cref_lpc_order_ld(INT *len) {
  *len = (INT)sizeof(ref_lpc::order_ld);
  return ref_lpc::order_ld;
}
void cref_lpc_synthesis_lattice_sgl(FIXP_DBL *signal, INT signal_size, INT signal_e, INT signal_e_out, INT inc, const FIXP_SGL *coeff, INT order, FIXP_DBL *state) {
  CLpc_SynthesisLattice(signal, signal_size, signal_e, signal_e_out, inc, coeff, order, state);
}
void cref_lpc_synthesis_lattice(FIXP_DBL *signal, INT signal_size, INT signal_e, INT signal_e_out, INT inc, const FIXP_DBL *coeff, INT order, FIXP_DBL *state) {
  CLpc_SynthesisLattice(signal, signal_size, signal_e, signal_e_out, inc, coeff, order, state);
}
void cref_lpc_synthesis(FIXP_DBL *signal, INT signal_size, INT signal_e, INT inc, const FIXP_DBL *coeff, INT coeff_e, INT order, FIXP_DBL *state, INT *state_index) {
  CLpc_Synthesis(signal, signal_size, signal_e, inc, (const FIXP_LPC_TNS *)coeff, coeff_e, order, state, state_index);
}
void cref_lpc_synthesis_sgl(FIXP_DBL *signal, INT signal_size, INT signal_e, INT inc, const FIXP_SGL *coeff, INT coeff_e, INT order, FIXP_DBL *state, INT *state_index) {
  CLpc_Synthesis(signal, signal_size, signal_e, inc, (const FIXP_LPC *)coeff, coeff_e, order, state, state_index);
}
void cref_lpc_analysis(FIXP_DBL *signal, INT signal_size, const FIXP_SGL *coeff, INT coeff_e, INT order, FIXP_DBL *state, INT *state_index) {
  CLpc_Analysis(signal, signal_size, coeff, coeff_e, order, state, state_index);
}
INT cref_lpc_parcor_to_lpc(const FIXP_DBL *refl, FIXP_DBL *lpc, INT n, FIXP_DBL *work) {
  return CLpc_ParcorToLpc((const FIXP_LPC_TNS *)refl, (FIXP_LPC_TNS *)lpc, n, work);
}
INT cref_lpc_parcor_to_lpc_sgl(const FIXP_SGL *refl, FIXP_SGL *lpc, INT n, FIXP_DBL *work) {
  return CLpc_ParcorToLpc((const FIXP_LPC *)refl, (FIXP_LPC *)lpc, n, work);
}
void cref_lpc_auto_to_parcor(FIXP_DBL *acorr, INT acorr_e, FIXP_SGL *refl, INT n, FIXP_DBL *gain_m, INT *gain_e) {
  CLpc_AutoToParcor(acorr, acorr_e, refl, n, gain_m, gain_e);
}
}
