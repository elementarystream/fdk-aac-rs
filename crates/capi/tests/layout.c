/* fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
 * A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
 *
 * Layout of the API structures as the installed headers define them (tests/layout.rs compares
 * it with the Rust definitions). One line per item: "<name> <value>". */

#include <stddef.h>
#include <stdio.h>

#include "aacenc_lib.h"
#include "aacdecoder_lib.h"

#define SIZE(T) printf("sizeof(" #T ") %zu\nalignof(" #T ") %zu\n", sizeof(T), _Alignof(T))
#define OFF(T, f) printf("offsetof(" #T ", " #f ") %zu\n", offsetof(T, f))

int main(void) {
  SIZE(AACENC_BufDesc);
  OFF(AACENC_BufDesc, numBufs);
  OFF(AACENC_BufDesc, bufs);
  OFF(AACENC_BufDesc, bufferIdentifiers);
  OFF(AACENC_BufDesc, bufSizes);
  OFF(AACENC_BufDesc, bufElSizes);

  SIZE(AACENC_InArgs);
  OFF(AACENC_InArgs, numInSamples);
  OFF(AACENC_InArgs, numAncBytes);

  SIZE(AACENC_OutArgs);
  OFF(AACENC_OutArgs, numOutBytes);
  OFF(AACENC_OutArgs, numInSamples);
  OFF(AACENC_OutArgs, numAncBytes);
  OFF(AACENC_OutArgs, bitResState);

  SIZE(AACENC_InfoStruct);
  OFF(AACENC_InfoStruct, maxOutBufBytes);
  OFF(AACENC_InfoStruct, maxAncBytes);
  OFF(AACENC_InfoStruct, inBufFillLevel);
  OFF(AACENC_InfoStruct, inputChannels);
  OFF(AACENC_InfoStruct, frameLength);
  OFF(AACENC_InfoStruct, nDelay);
  OFF(AACENC_InfoStruct, nDelayCore);
  OFF(AACENC_InfoStruct, confBuf);
  OFF(AACENC_InfoStruct, confSize);

  SIZE(AACENC_MetaData);
  OFF(AACENC_MetaData, drc_profile);
  OFF(AACENC_MetaData, comp_profile);
  OFF(AACENC_MetaData, drc_TargetRefLevel);
  OFF(AACENC_MetaData, comp_TargetRefLevel);
  OFF(AACENC_MetaData, prog_ref_level_present);
  OFF(AACENC_MetaData, prog_ref_level);
  OFF(AACENC_MetaData, PCE_mixdown_idx_present);
  OFF(AACENC_MetaData, ETSI_DmxLvl_present);
  OFF(AACENC_MetaData, centerMixLevel);
  OFF(AACENC_MetaData, surroundMixLevel);
  OFF(AACENC_MetaData, dolbySurroundMode);
  OFF(AACENC_MetaData, drcPresentationMode);
  OFF(AACENC_MetaData, ExtMetaData);
  OFF(AACENC_MetaData, ExtMetaData.extAncDataEnable);
  OFF(AACENC_MetaData, ExtMetaData.extDownmixLevelEnable);
  OFF(AACENC_MetaData, ExtMetaData.extDownmixLevel_A);
  OFF(AACENC_MetaData, ExtMetaData.extDownmixLevel_B);
  OFF(AACENC_MetaData, ExtMetaData.dmxGainEnable);
  OFF(AACENC_MetaData, ExtMetaData.dmxGain5);
  OFF(AACENC_MetaData, ExtMetaData.dmxGain2);
  OFF(AACENC_MetaData, ExtMetaData.lfeDmxEnable);
  OFF(AACENC_MetaData, ExtMetaData.lfeDmxLevel);
  printf("sizeof(AACENC_MetaData.ExtMetaData) %zu\n",
         sizeof(((AACENC_MetaData *)0)->ExtMetaData));

  SIZE(LIB_INFO);
  OFF(LIB_INFO, title);
  OFF(LIB_INFO, build_date);
  OFF(LIB_INFO, build_time);
  OFF(LIB_INFO, module_id);
  OFF(LIB_INFO, version);
  OFF(LIB_INFO, flags);
  OFF(LIB_INFO, versionStr);

  SIZE(HANDLE_AACENCODER);
  SIZE(AACENC_ERROR);
  SIZE(AACENC_PARAM);
  SIZE(FDK_MODULE_ID);
  printf("FDK_MODULE_LAST %d\n", (int)FDK_MODULE_LAST);

  SIZE(CStreamInfo);
  OFF(CStreamInfo, sampleRate);
  OFF(CStreamInfo, frameSize);
  OFF(CStreamInfo, numChannels);
  OFF(CStreamInfo, pChannelType);
  OFF(CStreamInfo, pChannelIndices);
  OFF(CStreamInfo, aacSampleRate);
  OFF(CStreamInfo, profile);
  OFF(CStreamInfo, aot);
  OFF(CStreamInfo, channelConfig);
  OFF(CStreamInfo, bitRate);
  OFF(CStreamInfo, aacSamplesPerFrame);
  OFF(CStreamInfo, aacNumChannels);
  OFF(CStreamInfo, extAot);
  OFF(CStreamInfo, extSamplingRate);
  OFF(CStreamInfo, outputDelay);
  OFF(CStreamInfo, flags);
  OFF(CStreamInfo, epConfig);
  OFF(CStreamInfo, numLostAccessUnits);
  OFF(CStreamInfo, numTotalBytes);
  OFF(CStreamInfo, numBadBytes);
  OFF(CStreamInfo, numTotalAccessUnits);
  OFF(CStreamInfo, numBadAccessUnits);
  OFF(CStreamInfo, drcProgRefLev);
  OFF(CStreamInfo, drcPresMode);
  OFF(CStreamInfo, outputLoudness);

  SIZE(HANDLE_AACDECODER);
  SIZE(AAC_DECODER_ERROR);
  SIZE(AACDEC_PARAM);
  SIZE(TRANSPORT_TYPE);
  SIZE(AUDIO_OBJECT_TYPE);
  SIZE(AUDIO_CHANNEL_TYPE);
  SIZE(INT_PCM);
  return 0;
}
