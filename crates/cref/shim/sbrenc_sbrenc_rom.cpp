// Shims for the differential tests of fdk_aac_rs_sbrenc::sbrenc_rom.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sbrenc_sbrenc_rom (see build.rs).

#include "cref_common.h"
#include "libSBRenc/src/sbrenc_rom.h"

#define T(x) {#x, (const void *)x, (INT)(sizeof(x) / sizeof(x[0])), (INT)sizeof(x[0])}
struct Tab {
  const char *name;
  const void *p;
  INT n;
  INT elem;
};
static const Tab tabs[] = {
    T(v_Huff_envelopeLevelC10T), T(v_Huff_envelopeLevelL10T), T(v_Huff_envelopeLevelC10F),
    T(v_Huff_envelopeLevelL10F), T(bookSbrEnvBalanceC10T),    T(bookSbrEnvBalanceL10T),
    T(bookSbrEnvBalanceC10F),    T(bookSbrEnvBalanceL10F),    T(v_Huff_envelopeLevelC11T),
    T(v_Huff_envelopeLevelL11T), T(v_Huff_envelopeLevelC11F), T(v_Huff_envelopeLevelL11F),
    T(bookSbrEnvBalanceC11T),    T(bookSbrEnvBalanceL11T),    T(bookSbrEnvBalanceC11F),
    T(bookSbrEnvBalanceL11F),    T(v_Huff_NoiseLevelC11T),    T(v_Huff_NoiseLevelL11T),
    T(bookSbrNoiseBalanceC11T),  T(bookSbrNoiseBalanceL11T),
};

extern "C" {
/* Huffman table `id` (in the order of tabs[]) as INT values; returns the count, -1 at the end. */
INT cref_sbrenc_rom_huff(INT id, INT *out) {
  if (id < 0 || id >= (INT)(sizeof(tabs) / sizeof(tabs[0]))) return -1;
  const Tab *t = &tabs[id];
  for (INT i = 0; i < t->n; i++)
    out[i] = (t->elem == 4) ? ((const INT *)t->p)[i] : ((const UCHAR *)t->p)[i];
  return t->n;
}
INT cref_sbrenc_rom_tuning_size(void) { return sbrTuningTableSize; }
/* Row i of sbrTuningTable, 14 fields in declaration order. */
void cref_sbrenc_rom_tuning_row(INT i, INT *v) {
  const sbrTuningTable_t *t = &sbrTuningTable[i];
  INT k = 0;
  v[k++] = t->coreCoder;
  v[k++] = (INT)t->bitrateFrom;
  v[k++] = (INT)t->bitrateTo;
  v[k++] = (INT)t->sampleRate;
  v[k++] = t->numChannels;
  v[k++] = t->startFreq;
  v[k++] = t->startFreqSpeech;
  v[k++] = t->stopFreq;
  v[k++] = t->stopFreqSpeech;
  v[k++] = t->numNoiseBands;
  v[k++] = t->noiseFloorOffset;
  v[k++] = t->noiseMaxLevel;
  v[k++] = t->stereoMode;
  v[k++] = t->freqScale;
}
/* Row i of psTuningTable, 5 fields. */
void cref_sbrenc_rom_ps_tuning_row(INT i, INT *v) {
  const psTuningTable_t *t = &psTuningTable[i];
  v[0] = (INT)t->bitrateFrom;
  v[1] = (INT)t->bitrateTo;
  v[2] = t->nStereoBands;
  v[3] = t->nEnvelopes;
  v[4] = (INT)t->iidQuantErrorThreshold;
}
}
