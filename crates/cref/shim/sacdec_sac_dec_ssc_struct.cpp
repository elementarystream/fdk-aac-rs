// Shims for the differential tests of fdk_aac_rs_sacdec::sac_dec_ssc_struct.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacdec_sac_dec_ssc_struct (see build.rs).

#include <stddef.h>
#include "cref_common.h"
#include "sac_dec_interface.h"

#define OFF(f) out[n++] = (INT)offsetof(SPATIAL_SPECIFIC_CONFIG, f)

extern "C" {
/* The offsets of the fields of SPATIAL_SPECIFIC_CONFIG in declaration order, then its size. */
INT cref_ssc_layout(INT *out) {
  INT n = 0;
  OFF(syntaxFlags);
  OFF(samplingFreq);
  OFF(nTimeSlots);
  OFF(freqRes);
  OFF(treeConfig);
  OFF(quantMode);
  OFF(bArbitraryDownmix);
  OFF(bResidualCoding);
  OFF(bsFixedGainDMX);
  OFF(tempShapeConfig);
  OFF(decorrConfig);
  OFF(nInputChannels);
  OFF(nOutputChannels);
  OFF(nOttBoxes);
  OFF(OttConfig);
  OFF(nTttBoxes);
  OFF(ResidualConfig);
  OFF(sacExtCnt);
  OFF(sacExtType);
  OFF(envQuantMode);
  OFF(coreCodec);
  OFF(stereoConfigIndex);
  OFF(coreSbrFrameLengthIndex);
  OFF(bsHighRateMode);
  OFF(bsDecorrType);
  OFF(bsPseudoLr);
  OFF(bsPhaseCoding);
  OFF(bsOttBandsPhasePresent);
  OFF(bsOttBandsPhase);
  OFF(ottCLDdefault);
  OFF(numOttBandsIPD);
  OFF(bitstreamOttBands);
  OFF(numOttBands);
  out[n++] = (INT)sizeof(SPATIAL_SPECIFIC_CONFIG);
  out[n++] = (INT)sizeof(SPATIALDEC_FREQ_RES);
  out[n++] = (INT)sizeof(AUDIO_OBJECT_TYPE);
  return n;
}
}
