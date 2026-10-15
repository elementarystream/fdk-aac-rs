// Shims for the differential tests of fdk_aac_rs_aacenc::block_switch.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_block_switch (see build.rs).
//
// BLOCK_SWITCHING_CONTROL is passed as void*: the port's BlockSwitchingControl has the C layout,
// so the tests let the C functions work directly on a Rust struct.

#include "cref_common.h"
#include "libAACenc/src/block_switch.h"

#include <stddef.h>

/* The static functions and tables of block_switch.cpp (namespace-include trick). */
namespace ref_block_switch {
#include "libAACenc/src/block_switch.cpp"
}

extern "C" {
void cref_block_switch_InitBlockSwitching(void *bsc, INT isLowDelay) {
  FDKaacEnc_InitBlockSwitching((BLOCK_SWITCHING_CONTROL *)bsc, isLowDelay);
}
INT cref_block_switch_BlockSwitching(void *bsc, INT granuleLength, INT isLFE, const SHORT *pTimeSignal) {
  return FDKaacEnc_BlockSwitching((BLOCK_SWITCHING_CONTROL *)bsc, granuleLength, isLFE,
                                  (const INT_PCM *)pTimeSignal);
}
INT cref_block_switch_SyncBlockSwitching(void *left, void *right, INT nChannels, INT commonWindow) {
  return FDKaacEnc_SyncBlockSwitching((BLOCK_SWITCHING_CONTROL *)left,
                                      (BLOCK_SWITCHING_CONTROL *)right, nChannels, commonWindow);
}
void cref_block_switch_CalcWindowEnergy(void *bsc, INT windowLen, const SHORT *pTimeSignal) {
  ref_block_switch::FDKaacEnc_CalcWindowEnergy((BLOCK_SWITCHING_CONTROL *)bsc, windowLen,
                                               (const INT_PCM *)pTimeSignal);
}
FIXP_DBL cref_block_switch_GetWindowEnergy(const FIXP_DBL *in, INT blSwWndIdx) {
  return ref_block_switch::FDKaacEnc_GetWindowEnergy(in, blSwWndIdx);
}
/* The tables and constants of block_switch.cpp: pointer and size in bytes of table `id`
   (NULL past the last one). */
const void *cref_block_switch_table(INT id, INT *size) {
  using namespace ref_block_switch;
#define T(i, x)             \
  case i:                   \
    *size = (INT)sizeof(x); \
    return &x;
  switch (id) {
    T(0, blockType2windowShape)
    T(1, hiPassCoeff)
    T(2, accWindowNrgFac)
    T(3, oneMinusAccWindowNrgFac)
    T(4, invAttackRatio)
    T(5, minAttackNrg)
    T(6, suggestedGroupingTable)
    T(7, chgWndSq)
    T(8, chgWndSqLkAhd)
    T(9, synchronizedBlockTypeTable)
  }
#undef T
  *size = 0;
  return NULL;
}
/* sizeof(BLOCK_SWITCHING_CONTROL) and the offsets of its members, in declaration order. */
INT cref_block_switch_layout(INT *offsets) {
  typedef BLOCK_SWITCHING_CONTROL B;
  const INT o[] = {
      (INT)offsetof(B, lastWindowSequence), (INT)offsetof(B, windowShape),
      (INT)offsetof(B, lastWindowShape),    (INT)offsetof(B, nBlockSwitchWindows),
      (INT)offsetof(B, attack),             (INT)offsetof(B, lastattack),
      (INT)offsetof(B, attackIndex),        (INT)offsetof(B, lastAttackIndex),
      (INT)offsetof(B, allowShortFrames),   (INT)offsetof(B, allowLookAhead),
      (INT)offsetof(B, noOfGroups),         (INT)offsetof(B, groupLen),
      (INT)offsetof(B, maxWindowNrg),       (INT)offsetof(B, windowNrg),
      (INT)offsetof(B, windowNrgF),         (INT)offsetof(B, accWindowNrg),
      (INT)offsetof(B, iirStates),
  };
  memcpy(offsets, o, sizeof(o));
  return (INT)sizeof(B);
}
}
