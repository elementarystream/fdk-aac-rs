// Differential tests of fdk_aac_rs_sacdec::sac_dec_interface against the C++ reference: the
// enumeration and macro values of sac_dec_interface.h, sac_dec_lib.h and sac_dec.h.

use fdk_aac_rs_cref::sacdec_sac_dec_interface as c;
use fdk_aac_rs_sacdec::sac_dec as d;
use fdk_aac_rs_sacdec::sac_dec_interface::*;
use fdk_aac_rs_sacdec::sac_dec_lib as l;

#[test]
fn sac_dec_interface_values() {
    let rs: Vec<i32> = vec![
        DECODER_LEVEL_0,
        DECODER_LEVEL_6,
        OUTPUT_CHANNELS_DEFAULT,
        OUTPUT_CHANNELS_2_0,
        OUTPUT_CHANNELS_5_1,
        EXT_HQ_ONLY,
        EXT_LP_ONLY,
        EXT_HQ_AND_LP,
        BINAURAL_NONE,
        INPUTMODE_QMF,
        INPUTMODE_QMF_SBR,
        INPUTMODE_TIME,
        UPMIX_TYPE_BYPASS,
        UPMIX_TYPE_NORMAL,
        MPEGS_INIT_NONE as i32,
        MPEGS_INIT_CONFIG as i32,
        MPEGS_INIT_STATES_ANA_QMF_FILTER as i32,
        MPEGS_INIT_STATES_SYN_QMF_FILTER as i32,
        MPEGS_INIT_STATES_ANA_HYB_FILTER as i32,
        MPEGS_INIT_STATES_DECORRELATOR as i32,
        MPEGS_INIT_STATES_M1M2 as i32,
        MPEGS_INIT_STATES_GES as i32,
        MPEGS_INIT_STATES_REVERB as i32,
        MPEGS_INIT_STATES_PARAM as i32,
        MPEGS_INIT_STATES_ERROR_CONCEALMENT as i32,
        MPEGS_INIT_PARAMS_ERROR_CONCEALMENT as i32,
        MASK_MPEGS_INIT_ALL_STATES as i32,
        MASK_MPEGS_INIT_ALL_PARAMS as i32,
        l::SAC_INTERFACE_QMF,
        l::SAC_INTERFACE_TIME,
        l::SAC_INTERFACE_AUTO,
        l::SACDEC_OUT_MODE_NORMAL,
        l::SACDEC_OUT_MODE_BINAURAL,
        l::SACDEC_OUT_MODE_STEREO,
        l::SACDEC_OUT_MODE_6CHANNEL,
        l::SAC_INSTANCE_NOT_FULL_AVAILABLE,
        l::SAC_INSTANCE_FULL_AVAILABLE,
        l::SACDEC_OUTPUT_MODE,
        l::SACDEC_BLIND_ENABLE,
        l::SACDEC_PARTIALLY_COMPLEX,
        l::SACDEC_INTERFACE,
        l::SACDEC_BS_DELAY,
        l::SACDEC_BINAURAL_QUALITY,
        l::SACDEC_BINAURAL_DISTANCE,
        l::SACDEC_BINAURAL_DIALOG_CLARITY,
        l::SACDEC_BINAURAL_FRONT_ANGLE,
        l::SACDEC_BINAURAL_BACK_ANGLE,
        l::SACDEC_BINAURAL_PRESET,
        l::SACDEC_BS_INTERRUPTION,
        l::SACDEC_CLEAR_HISTORY,
        l::SACDEC_CONCEAL_NUM_KEEP_FRAMES,
        l::SACDEC_CONCEAL_FADE_OUT_SLOPE_LENGTH,
        l::SACDEC_CONCEAL_FADE_IN_SLOPE_LENGTH,
        l::SACDEC_CONCEAL_NUM_RELEASE_FRAMES,
        d::MAX_HYBRID_BANDS as i32,
        d::MAX_NUM_PARAMETERS as i32,
        d::MAX_PARAMETER_SETS as i32,
        d::PC_NUM_HYB_BANDS,
        d::BUFFER_LEN_HF,
        d::SACDEC_SYNTAX_LD as i32,
        d::SACDEC_SYNTAX_USAC as i32,
    ];
    let mut out = [0i32; 128];
    let n = unsafe { c::cref_sacdec_interface_values(out.as_mut_ptr()) } as usize;
    assert_eq!(&out[..n], &rs[..]);
}
