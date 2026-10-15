// Shims for the differential tests of fdk_aac_rs_sacdec::sac_dec_interface.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::sacdec_sac_dec_interface (see build.rs).

#include "cref_common.h"
#include "sac_dec_interface.h"
#include "sac_dec_lib.h"
#include "sac_dec.h"

extern "C" {
/* The enumeration and macro values of sac_dec_interface.h, sac_dec_lib.h and sac_dec.h in the
   order of the test. */
INT cref_sacdec_interface_values(INT *out) {
  INT n = 0;
  out[n++] = DECODER_LEVEL_0;
  out[n++] = DECODER_LEVEL_6;
  out[n++] = OUTPUT_CHANNELS_DEFAULT;
  out[n++] = OUTPUT_CHANNELS_2_0;
  out[n++] = OUTPUT_CHANNELS_5_1;
  out[n++] = EXT_HQ_ONLY;
  out[n++] = EXT_LP_ONLY;
  out[n++] = EXT_HQ_AND_LP;
  out[n++] = BINAURAL_NONE;
  out[n++] = INPUTMODE_QMF;
  out[n++] = INPUTMODE_QMF_SBR;
  out[n++] = INPUTMODE_TIME;
  out[n++] = UPMIX_TYPE_BYPASS;
  out[n++] = UPMIX_TYPE_NORMAL;
  out[n++] = MPEGS_INIT_NONE;
  out[n++] = MPEGS_INIT_CONFIG;
  out[n++] = MPEGS_INIT_STATES_ANA_QMF_FILTER;
  out[n++] = MPEGS_INIT_STATES_SYN_QMF_FILTER;
  out[n++] = MPEGS_INIT_STATES_ANA_HYB_FILTER;
  out[n++] = MPEGS_INIT_STATES_DECORRELATOR;
  out[n++] = MPEGS_INIT_STATES_M1M2;
  out[n++] = MPEGS_INIT_STATES_GES;
  out[n++] = MPEGS_INIT_STATES_REVERB;
  out[n++] = MPEGS_INIT_STATES_PARAM;
  out[n++] = MPEGS_INIT_STATES_ERROR_CONCEALMENT;
  out[n++] = MPEGS_INIT_PARAMS_ERROR_CONCEALMENT;
  out[n++] = MASK_MPEGS_INIT_ALL_STATES;
  out[n++] = MASK_MPEGS_INIT_ALL_PARAMS;
  /* sac_dec_lib.h */
  out[n++] = SAC_INTERFACE_QMF;
  out[n++] = SAC_INTERFACE_TIME;
  out[n++] = SAC_INTERFACE_AUTO;
  out[n++] = SACDEC_OUT_MODE_NORMAL;
  out[n++] = SACDEC_OUT_MODE_BINAURAL;
  out[n++] = SACDEC_OUT_MODE_STEREO;
  out[n++] = SACDEC_OUT_MODE_6CHANNEL;
  out[n++] = SAC_INSTANCE_NOT_FULL_AVAILABLE;
  out[n++] = SAC_INSTANCE_FULL_AVAILABLE;
  out[n++] = SACDEC_OUTPUT_MODE;
  out[n++] = SACDEC_BLIND_ENABLE;
  out[n++] = SACDEC_PARTIALLY_COMPLEX;
  out[n++] = SACDEC_INTERFACE;
  out[n++] = SACDEC_BS_DELAY;
  out[n++] = SACDEC_BINAURAL_QUALITY;
  out[n++] = SACDEC_BINAURAL_DISTANCE;
  out[n++] = SACDEC_BINAURAL_DIALOG_CLARITY;
  out[n++] = SACDEC_BINAURAL_FRONT_ANGLE;
  out[n++] = SACDEC_BINAURAL_BACK_ANGLE;
  out[n++] = SACDEC_BINAURAL_PRESET;
  out[n++] = SACDEC_BS_INTERRUPTION;
  out[n++] = SACDEC_CLEAR_HISTORY;
  out[n++] = SACDEC_CONCEAL_NUM_KEEP_FRAMES;
  out[n++] = SACDEC_CONCEAL_FADE_OUT_SLOPE_LENGTH;
  out[n++] = SACDEC_CONCEAL_FADE_IN_SLOPE_LENGTH;
  out[n++] = SACDEC_CONCEAL_NUM_RELEASE_FRAMES;
  /* sac_dec.h */
  out[n++] = MAX_HYBRID_BANDS;
  out[n++] = MAX_NUM_PARAMETERS;
  out[n++] = MAX_PARAMETER_SETS;
  out[n++] = PC_NUM_HYB_BANDS;
  out[n++] = BUFFER_LEN_HF;
  out[n++] = SACDEC_SYNTAX_LD;
  out[n++] = SACDEC_SYNTAX_USAC;
  return n;
}
}
