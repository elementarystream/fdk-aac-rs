// Shims for the differential tests of fdk_aac_rs_pcmutils::limiter.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::pcmutils_limiter (see build.rs).
//
// The limiter lives on the C side behind an opaque handle (NULL is passed through, so that the
// NULL checks can be tested). The functions called are the library's own.

#include "cref_common.h"
#include "limiter.h"

extern "C" {
void *cref_limiter_create(UINT maxAttackMs, UINT releaseMs, FIXP_DBL threshold, UINT maxChannels, UINT maxSampleRate) {
  return pcmLimiter_Create(maxAttackMs, releaseMs, threshold, maxChannels, maxSampleRate);
}
INT cref_limiter_destroy(void *h) { return pcmLimiter_Destroy((TDLimiterPtr)h); }
INT cref_limiter_reset(void *h) { return pcmLimiter_Reset((TDLimiterPtr)h); }
/* pGainPerSample is NULL if gain is NULL. */
INT cref_limiter_apply(void *h, INT *samplesIn, SHORT *samplesOut, INT *gain, INT scaling, UINT nSamples) {
  return pcmLimiter_Apply((TDLimiterPtr)h, (PCM_LIM *)samplesIn, (INT_PCM *)samplesOut, (FIXP_DBL *)gain, scaling, nSamples);
}
INT cref_limiter_set_threshold(void *h, FIXP_DBL threshold) { return pcmLimiter_SetThreshold((TDLimiterPtr)h, threshold); }
UINT cref_limiter_get_delay(void *h) { return pcmLimiter_GetDelay((TDLimiterPtr)h); }
INT cref_limiter_get_max_gain_reduction(void *h) { return pcmLimiter_GetMaxGainReduction((TDLimiterPtr)h); }
INT cref_limiter_set_n_channels(void *h, UINT n) { return pcmLimiter_SetNChannels((TDLimiterPtr)h, n); }
INT cref_limiter_set_sample_rate(void *h, UINT sr) { return pcmLimiter_SetSampleRate((TDLimiterPtr)h, sr); }
INT cref_limiter_set_attack(void *h, UINT ms) { return pcmLimiter_SetAttack((TDLimiterPtr)h, ms); }
INT cref_limiter_set_release(void *h, UINT ms) { return pcmLimiter_SetRelease((TDLimiterPtr)h, ms); }
/* The scalar fields in declaration order (18 values), and the first max_len / delay_len values
   of maxBuf and delayBuf. */
void cref_limiter_state(void *h, INT *s, INT *maxBuf, UINT max_len, INT *delayBuf, UINT delay_len) {
  TDLimiterPtr l = (TDLimiterPtr)h;
  s[0] = (INT)l->attack;
  s[1] = l->attackConst;
  s[2] = l->releaseConst;
  s[3] = (INT)l->attackMs;
  s[4] = (INT)l->releaseMs;
  s[5] = (INT)l->maxAttackMs;
  s[6] = l->threshold;
  s[7] = (INT)l->channels;
  s[8] = (INT)l->maxChannels;
  s[9] = (INT)l->sampleRate;
  s[10] = (INT)l->maxSampleRate;
  s[11] = l->cor;
  s[12] = l->max;
  s[13] = (INT)l->maxBufIdx;
  s[14] = (INT)l->delayBufIdx;
  s[15] = l->smoothState0;
  s[16] = l->minGain;
  s[17] = l->scaling;
  for (UINT i = 0; i < max_len; i++) maxBuf[i] = l->maxBuf[i];
  for (UINT i = 0; i < delay_len; i++) delayBuf[i] = l->delayBuf[i];
}
INT cref_limiter_module_last(void) { return FDK_MODULE_LAST; }
/* pcmLimiter_GetLibInfo() on FDK_MODULE_LAST entries with the given module ids (versionStr filled
   with 'x'), or on NULL. Writes per entry the module id, version, flags, the 32 bytes of
   versionStr and the first 31 characters of the title ("" for NULL). */
INT cref_limiter_get_lib_info(INT null_info, const INT *ids_in, INT *ids, INT *version, UINT *flags, char *version_str, char *title) {
  LIB_INFO info[FDK_MODULE_LAST];
  FDKmemclear(info, sizeof(info));
  for (int i = 0; i < FDK_MODULE_LAST; i++) {
    info[i].module_id = (FDK_MODULE_ID)ids_in[i];
    FDKmemset(info[i].versionStr, 'x', sizeof(info[i].versionStr));
  }
  INT r = pcmLimiter_GetLibInfo(null_info ? NULL : info);
  for (int i = 0; i < FDK_MODULE_LAST; i++) {
    ids[i] = info[i].module_id;
    version[i] = info[i].version;
    flags[i] = info[i].flags;
    FDKmemcpy(version_str + 32 * i, info[i].versionStr, 32);
    FDKmemclear(title + 32 * i, 32);
    if (info[i].title != NULL) strncpy(title + 32 * i, info[i].title, 31);
  }
  return r;
}
}
