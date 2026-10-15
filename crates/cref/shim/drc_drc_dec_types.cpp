// Shims for the differential tests of fdk_aac_rs_drc::drc_dec_types.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::drc_drc_dec_types (see build.rs).

#include <stddef.h>
#include "cref_common.h"
#include "drcDec_types.h"

#define S(t) out[n++] = (INT)sizeof(t)
#define O(t, f) out[n++] = (INT)offsetof(t, f)

extern "C" {
/* Sizes and field offsets of the structures of drcDec_types.h, in the order of the list in
   tests/drc/drc_dec_types.rs. Returns the number of values written. */
INT cref_drc_types_layout(INT *out) {
  INT n = 0;
  S(GAIN_NODE); O(GAIN_NODE, gainDb); O(GAIN_NODE, time);
  S(UNI_DRC_GAIN_EXTENSION); O(UNI_DRC_GAIN_EXTENSION, uniDrcGainExtType); O(UNI_DRC_GAIN_EXTENSION, extBitSize);
  S(UNI_DRC_GAIN); O(UNI_DRC_GAIN, nNodes); O(UNI_DRC_GAIN, gainNode); O(UNI_DRC_GAIN, uniDrcGainExtPresent); O(UNI_DRC_GAIN, uniDrcGainExtension); O(UNI_DRC_GAIN, status);
  S(DUCKING_MODIFICATION); O(DUCKING_MODIFICATION, duckingScalingPresent); O(DUCKING_MODIFICATION, duckingScaling);
  S(GAIN_MODIFICATION); O(GAIN_MODIFICATION, targetCharacteristicLeftPresent); O(GAIN_MODIFICATION, targetCharacteristicLeftIndex); O(GAIN_MODIFICATION, targetCharacteristicRightPresent); O(GAIN_MODIFICATION, targetCharacteristicRightIndex); O(GAIN_MODIFICATION, gainScalingPresent); O(GAIN_MODIFICATION, attenuationScaling); O(GAIN_MODIFICATION, amplificationScaling); O(GAIN_MODIFICATION, gainOffsetPresent); O(GAIN_MODIFICATION, gainOffset);
  S(BAND_BORDER); S(CUSTOM_INDEX);
  S(DRC_CHARACTERISTIC); O(DRC_CHARACTERISTIC, present); O(DRC_CHARACTERISTIC, isCICP); O(DRC_CHARACTERISTIC, cicpIndex); O(DRC_CHARACTERISTIC, custom);
  S(GAIN_SET); O(GAIN_SET, gainCodingProfile); O(GAIN_SET, gainInterpolationType); O(GAIN_SET, fullFrame); O(GAIN_SET, timeAlignment); O(GAIN_SET, timeDeltaMinPresent); O(GAIN_SET, timeDeltaMin); O(GAIN_SET, bandCount); O(GAIN_SET, drcBandType); O(GAIN_SET, gainSequenceIndex); O(GAIN_SET, drcCharacteristic); O(GAIN_SET, bandBorder);
  S(CUSTOM_DRC_CHAR_SIGMOID); O(CUSTOM_DRC_CHAR_SIGMOID, gain); O(CUSTOM_DRC_CHAR_SIGMOID, ioRatio); O(CUSTOM_DRC_CHAR_SIGMOID, exp); O(CUSTOM_DRC_CHAR_SIGMOID, flipSign);
  S(CUSTOM_DRC_CHAR_NODES); O(CUSTOM_DRC_CHAR_NODES, characteristicNodeCount); O(CUSTOM_DRC_CHAR_NODES, nodeLevel); O(CUSTOM_DRC_CHAR_NODES, nodeGain);
  S(CUSTOM_DRC_CHAR);
  S(DRC_COEFFICIENTS_UNI_DRC); O(DRC_COEFFICIENTS_UNI_DRC, drcLocation); O(DRC_COEFFICIENTS_UNI_DRC, drcFrameSizePresent); O(DRC_COEFFICIENTS_UNI_DRC, drcFrameSize); O(DRC_COEFFICIENTS_UNI_DRC, characteristicLeftCount); O(DRC_COEFFICIENTS_UNI_DRC, characteristicLeftFormat); O(DRC_COEFFICIENTS_UNI_DRC, customCharacteristicLeft); O(DRC_COEFFICIENTS_UNI_DRC, characteristicRightCount); O(DRC_COEFFICIENTS_UNI_DRC, characteristicRightFormat); O(DRC_COEFFICIENTS_UNI_DRC, customCharacteristicRight); O(DRC_COEFFICIENTS_UNI_DRC, gainSequenceCount); O(DRC_COEFFICIENTS_UNI_DRC, gainSetCount); O(DRC_COEFFICIENTS_UNI_DRC, gainSet); O(DRC_COEFFICIENTS_UNI_DRC, gainSetIndexForGainSequence);
  S(DRC_INSTRUCTIONS_UNI_DRC); O(DRC_INSTRUCTIONS_UNI_DRC, drcSetId); O(DRC_INSTRUCTIONS_UNI_DRC, drcSetComplexityLevel); O(DRC_INSTRUCTIONS_UNI_DRC, drcLocation); O(DRC_INSTRUCTIONS_UNI_DRC, drcApplyToDownmix); O(DRC_INSTRUCTIONS_UNI_DRC, downmixIdCount); O(DRC_INSTRUCTIONS_UNI_DRC, downmixId); O(DRC_INSTRUCTIONS_UNI_DRC, drcSetEffect); O(DRC_INSTRUCTIONS_UNI_DRC, limiterPeakTargetPresent); O(DRC_INSTRUCTIONS_UNI_DRC, limiterPeakTarget); O(DRC_INSTRUCTIONS_UNI_DRC, drcSetTargetLoudnessPresent); O(DRC_INSTRUCTIONS_UNI_DRC, drcSetTargetLoudnessValueUpper); O(DRC_INSTRUCTIONS_UNI_DRC, drcSetTargetLoudnessValueLower); O(DRC_INSTRUCTIONS_UNI_DRC, dependsOnDrcSetPresent); O(DRC_INSTRUCTIONS_UNI_DRC, dependsOnDrcSet); O(DRC_INSTRUCTIONS_UNI_DRC, noIndependentUse); O(DRC_INSTRUCTIONS_UNI_DRC, requiresEq); O(DRC_INSTRUCTIONS_UNI_DRC, gainModificationForChannelGroup); O(DRC_INSTRUCTIONS_UNI_DRC, duckingModificationForChannel); O(DRC_INSTRUCTIONS_UNI_DRC, gainSetIndex); O(DRC_INSTRUCTIONS_UNI_DRC, drcChannelCount); O(DRC_INSTRUCTIONS_UNI_DRC, nDrcChannelGroups); O(DRC_INSTRUCTIONS_UNI_DRC, gainSetIndexForChannelGroup);
  S(CHANNEL_LAYOUT); O(CHANNEL_LAYOUT, baseChannelCount); O(CHANNEL_LAYOUT, layoutSignalingPresent); O(CHANNEL_LAYOUT, definedLayout); O(CHANNEL_LAYOUT, speakerPosition);
  S(DOWNMIX_INSTRUCTIONS); O(DOWNMIX_INSTRUCTIONS, downmixId); O(DOWNMIX_INSTRUCTIONS, targetChannelCount); O(DOWNMIX_INSTRUCTIONS, targetLayout); O(DOWNMIX_INSTRUCTIONS, downmixCoefficientsPresent); O(DOWNMIX_INSTRUCTIONS, bsDownmixOffset); O(DOWNMIX_INSTRUCTIONS, downmixCoefficient);
  S(UNI_DRC_CONFIG_EXTENSION); O(UNI_DRC_CONFIG_EXTENSION, uniDrcConfigExtType); O(UNI_DRC_CONFIG_EXTENSION, extBitSize);
  S(UNI_DRC_CONFIG); O(UNI_DRC_CONFIG, sampleRatePresent); O(UNI_DRC_CONFIG, sampleRate); O(UNI_DRC_CONFIG, downmixInstructionsCountV0); O(UNI_DRC_CONFIG, downmixInstructionsCountV1); O(UNI_DRC_CONFIG, downmixInstructionsCount); O(UNI_DRC_CONFIG, drcCoefficientsUniDrcCountV0); O(UNI_DRC_CONFIG, drcCoefficientsUniDrcCountV1); O(UNI_DRC_CONFIG, drcCoefficientsUniDrcCount); O(UNI_DRC_CONFIG, drcInstructionsUniDrcCountV0); O(UNI_DRC_CONFIG, drcInstructionsUniDrcCountV1); O(UNI_DRC_CONFIG, drcInstructionsUniDrcCount); O(UNI_DRC_CONFIG, channelLayout); O(UNI_DRC_CONFIG, downmixInstructions); O(UNI_DRC_CONFIG, drcCoefficientsUniDrc); O(UNI_DRC_CONFIG, drcInstructionsUniDrc); O(UNI_DRC_CONFIG, uniDrcConfigExtPresent); O(UNI_DRC_CONFIG, uniDrcConfigExt); O(UNI_DRC_CONFIG, drcInstructionsCountInclVirtual); O(UNI_DRC_CONFIG, diff);
  S(LOUDNESS_MEASUREMENT); O(LOUDNESS_MEASUREMENT, methodDefinition); O(LOUDNESS_MEASUREMENT, methodValue); O(LOUDNESS_MEASUREMENT, measurementSystem); O(LOUDNESS_MEASUREMENT, reliability);
  S(LOUDNESS_INFO); O(LOUDNESS_INFO, drcSetId); O(LOUDNESS_INFO, eqSetId); O(LOUDNESS_INFO, downmixId); O(LOUDNESS_INFO, samplePeakLevelPresent); O(LOUDNESS_INFO, samplePeakLevel); O(LOUDNESS_INFO, truePeakLevelPresent); O(LOUDNESS_INFO, truePeakLevel); O(LOUDNESS_INFO, truePeakLevelMeasurementSystem); O(LOUDNESS_INFO, truePeakLevelReliability); O(LOUDNESS_INFO, measurementCount); O(LOUDNESS_INFO, loudnessMeasurement);
  S(LOUDNESS_INFO_SET_EXTENSION); O(LOUDNESS_INFO_SET_EXTENSION, loudnessInfoSetExtType); O(LOUDNESS_INFO_SET_EXTENSION, extBitSize);
  S(LOUDNESS_INFO_SET); O(LOUDNESS_INFO_SET, loudnessInfoAlbumCountV0); O(LOUDNESS_INFO_SET, loudnessInfoAlbumCountV1); O(LOUDNESS_INFO_SET, loudnessInfoAlbumCount); O(LOUDNESS_INFO_SET, loudnessInfoCountV0); O(LOUDNESS_INFO_SET, loudnessInfoCountV1); O(LOUDNESS_INFO_SET, loudnessInfoCount); O(LOUDNESS_INFO_SET, loudnessInfoAlbum); O(LOUDNESS_INFO_SET, loudnessInfo); O(LOUDNESS_INFO_SET, loudnessInfoSetExtPresent); O(LOUDNESS_INFO_SET, loudnessInfoSetExt); O(LOUDNESS_INFO_SET, diff);
  return n;
}
}
