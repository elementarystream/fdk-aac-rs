// Shims for the differential tests of fdk_aac_rs_aacenc::metadata_compressor.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::aacenc_metadata_compressor (see build.rs).
//
// DRC_COMP is defined in metadata_compressor.cpp, so the namespace-include trick does not work
// (the functions of the copy would use the global, incomplete DRC_COMP of the header). Instead,
// a private copy of the .cpp is included at global scope with its public functions renamed;
// it provides the complete DRC_COMP type (the same type as the library's) and the file-static
// functions and tables. The public functions under test are the library's.

#include "cref_common.h"
#include <stddef.h>

#define FDK_DRC_Generator_Open mccopy_FDK_DRC_Generator_Open
#define FDK_DRC_Generator_Close mccopy_FDK_DRC_Generator_Close
#define FDK_DRC_Generator_Initialize mccopy_FDK_DRC_Generator_Initialize
#define FDK_DRC_Generator_Calc mccopy_FDK_DRC_Generator_Calc
#define FDK_DRC_Generator_setDrcProfile mccopy_FDK_DRC_Generator_setDrcProfile
#define FDK_DRC_Generator_getDrcProfile mccopy_FDK_DRC_Generator_getDrcProfile
#define FDK_DRC_Generator_getCompProfile mccopy_FDK_DRC_Generator_getCompProfile
#include "libAACenc/src/metadata_compressor.cpp"
#undef FDK_DRC_Generator_Open
#undef FDK_DRC_Generator_Close
#undef FDK_DRC_Generator_Initialize
#undef FDK_DRC_Generator_Calc
#undef FDK_DRC_Generator_setDrcProfile
#undef FDK_DRC_Generator_getDrcProfile
#undef FDK_DRC_Generator_getCompProfile

/* The library's functions (declared here because the header was included with the renames). */
INT FDK_DRC_Generator_Open(HDRC_COMP *phDrcComp);
INT FDK_DRC_Generator_Close(HDRC_COMP *phDrcComp);
INT FDK_DRC_Generator_Initialize(HDRC_COMP drcComp, const DRC_PROFILE profileLine,
                                 const DRC_PROFILE profileRF, const INT blockLength,
                                 const UINT sampleRate, const CHANNEL_MODE channelMode,
                                 const CHANNEL_ORDER channelOrder, const UCHAR useWeighting);
INT FDK_DRC_Generator_Calc(HDRC_COMP drcComp, const INT_PCM *const inSamples,
                           const UINT inSamplesBufSize, const INT dialnorm,
                           const INT drc_TargetRefLevel, const INT comp_TargetRefLevel,
                           const FIXP_DBL clev, const FIXP_DBL slev, const FIXP_DBL ext_leva,
                           const FIXP_DBL ext_levb, const FIXP_DBL lfe_lev, const INT dmxGain5,
                           const INT dmxGain2, INT *const dynrng, INT *const compr);
INT FDK_DRC_Generator_setDrcProfile(HDRC_COMP drcComp, const DRC_PROFILE profileLine,
                                    const DRC_PROFILE profileRF);
DRC_PROFILE FDK_DRC_Generator_getDrcProfile(const HDRC_COMP drcComp);
DRC_PROFILE FDK_DRC_Generator_getCompProfile(const HDRC_COMP drcComp);

extern "C" {
/* FDK_DRC_Generator_Open(); returns the handle (NULL on failure). */
void *cref_metadata_compressor_Open() {
  HDRC_COMP h = NULL;
  if (FDK_DRC_Generator_Open(&h) != 0) return NULL;
  return h;
}
/* FDK_DRC_Generator_Close() */
INT cref_metadata_compressor_Close(void *h) {
  HDRC_COMP p = (HDRC_COMP)h;
  return FDK_DRC_Generator_Close(&p);
}
/* FDK_DRC_Generator_Initialize() */
INT cref_metadata_compressor_Initialize(void *h, INT profileLine, INT profileRF, INT blockLength, UINT sampleRate, INT channelMode, INT channelOrder, UCHAR useWeighting) {
  return FDK_DRC_Generator_Initialize((HDRC_COMP)h, (DRC_PROFILE)profileLine,
                                      (DRC_PROFILE)profileRF, blockLength, sampleRate,
                                      (CHANNEL_MODE)channelMode, (CHANNEL_ORDER)channelOrder,
                                      useWeighting);
}
/* FDK_DRC_Generator_setDrcProfile() */
INT cref_metadata_compressor_setDrcProfile(void *h, INT profileLine, INT profileRF) {
  return FDK_DRC_Generator_setDrcProfile((HDRC_COMP)h, (DRC_PROFILE)profileLine,
                                         (DRC_PROFILE)profileRF);
}
/* FDK_DRC_Generator_Calc() */
INT cref_metadata_compressor_Calc(void *h, const INT_PCM *inSamples, UINT inSamplesBufSize, INT dialnorm, INT drc_TargetRefLevel, INT comp_TargetRefLevel, FIXP_DBL clev, FIXP_DBL slev, FIXP_DBL ext_leva, FIXP_DBL ext_levb, FIXP_DBL lfe_lev, INT dmxGain5, INT dmxGain2, INT *dynrng, INT *compr) {
  return FDK_DRC_Generator_Calc((HDRC_COMP)h, inSamples, inSamplesBufSize, dialnorm,
                                drc_TargetRefLevel, comp_TargetRefLevel, clev, slev, ext_leva,
                                ext_levb, lfe_lev, dmxGain5, dmxGain2, dynrng, compr);
}
/* FDK_DRC_Generator_Calc() of the private copy (diagnostics). */
INT cref_metadata_compressor_CalcCopy(void *h, const INT_PCM *inSamples, UINT inSamplesBufSize, INT dialnorm, INT drc_TargetRefLevel, INT comp_TargetRefLevel, FIXP_DBL clev, FIXP_DBL slev, FIXP_DBL ext_leva, FIXP_DBL ext_levb, FIXP_DBL lfe_lev, INT dmxGain5, INT dmxGain2, INT *dynrng, INT *compr) {
  return mccopy_FDK_DRC_Generator_Calc((HDRC_COMP)h, inSamples, inSamplesBufSize, dialnorm,
                                       drc_TargetRefLevel, comp_TargetRefLevel, clev, slev,
                                       ext_leva, ext_levb, lfe_lev, dmxGain5, dmxGain2, dynrng,
                                       compr);
}
/* FDK_DRC_Generator_getDrcProfile() */
INT cref_metadata_compressor_getDrcProfile(void *h) {
  return (INT)FDK_DRC_Generator_getDrcProfile((HDRC_COMP)h);
}
/* FDK_DRC_Generator_getCompProfile() */
INT cref_metadata_compressor_getCompProfile(void *h) {
  return (INT)FDK_DRC_Generator_getCompProfile((HDRC_COMP)h);
}
/* Copies the DRC_COMP of h to out; returns sizeof(DRC_COMP). */
INT cref_metadata_compressor_state(void *h, void *out) {
  FDKmemcpy(out, h, sizeof(DRC_COMP));
  return (INT)sizeof(DRC_COMP);
}
/* Overwrites the DRC_COMP of h with in. */
void cref_metadata_compressor_set_state(void *h, const void *in) {
  FDKmemcpy(h, in, sizeof(DRC_COMP));
}
/* Byte offset of DRC_COMP member i (0: useWeighting, 1: channels, 2: filter, 3: limDecay). */
INT cref_metadata_compressor_offset(INT i) {
  switch (i) {
    case 0: return (INT)offsetof(DRC_COMP, useWeighting);
    case 1: return (INT)offsetof(DRC_COMP, channels);
    case 2: return (INT)offsetof(DRC_COMP, filter);
    default: return (INT)offsetof(DRC_COMP, limDecay);
  }
}
/* getShiftFactor() (file-static, from the private copy). */
UINT cref_metadata_compressor_getShiftFactor(UINT length) { return getShiftFactor(length); }
/* fixpAdd() (file-static, from the private copy). */
void cref_metadata_compressor_fixpAdd(FIXP_DBL value1, INT q1, FIXP_DBL *pValue2, INT *pQ2) {
  fixpAdd(value1, q1, pValue2, pQ2);
}
/* tc2Coeff() (file-static, from the private copy). */
FIXP_DBL cref_metadata_compressor_tc2Coeff(FIXP_DBL t, INT sampleRate, INT blockLength) {
  return tc2Coeff(t, sampleRate, blockLength);
}
/* findPeakLevels() (file-static, from the private copy) on the DRC_COMP of h. */
void cref_metadata_compressor_findPeakLevels(void *h, const INT_PCM *inSamples, FIXP_DBL clev, FIXP_DBL slev, FIXP_DBL ext_leva, FIXP_DBL ext_levb, FIXP_DBL lfe_lev, FIXP_DBL dmxGain5, FIXP_DBL dmxGain2, FIXP_DBL *peak) {
  findPeakLevels((HDRC_COMP)h, inSamples, clev, slev, ext_leva, ext_levb, lfe_lev, dmxGain5,
                 dmxGain2, peak);
}
/* Profile table i (in the order of the file, 17: b0, a1, a2) as INTs into out; returns the
   number of INTs. */
INT cref_metadata_compressor_table(INT i, INT *out) {
  const void *tabs[] = {tabMaxBoostThr, tabBoostThr,   tabEarlyCutThr, tabCutThr,
                        tabMaxCutThr,   tabBoostRatio, tabEarlyCutRatio, tabCutRatio,
                        tabMaxBoost,    tabMaxCut,     tabFastAttack,  tabFastDecay,
                        tabSlowAttack,  tabSlowDecay,  tabHoldOff,     tabAttackThr,
                        tabDecayThr};
  if (i < 17) {
    FDKmemcpy(out, tabs[i], 6 * sizeof(INT));
    return 6;
  }
  out[0] = b0;
  out[1] = a1;
  out[2] = a2;
  return 3;
}
}
