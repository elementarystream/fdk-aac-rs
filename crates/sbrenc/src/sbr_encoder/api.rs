// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/sbr_encoder.cpp.

//! The functions of `sbr_encoder.cpp` (see the [parent module](super) for the API notes).

use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::crc::FdkCrcInfo;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::qmf::*;
use fdk_aac_rs_fdk::qmf_pcm::{qmf_analysis_filtering, qmf_init_analysis_filter_bank};
use fdk_aac_rs_fdk::tools_rom::QMF_FILTER_PROTOTYPE_SIZE;

use super::*;
use crate::bit_sbr::*;
use crate::code_env::*;
use crate::env_bit::*;
use crate::env_est::*;
use crate::fram_gen::*;
use crate::ps_main::*;
use crate::resampler::*;
use crate::sbr::*;
use crate::sbr_def::*;
use crate::sbrenc_freq_sca::*;
use crate::sbrenc_ram::*;
use crate::sbrenc_rom::*;
use crate::ton_corr::*;
use crate::tran_det::*;

/// `SBRENCODER_LIB_VL0`
pub const SBRENCODER_LIB_VL0: i32 = 4;
/// `SBRENCODER_LIB_VL1`
pub const SBRENCODER_LIB_VL1: i32 = 0;
/// `SBRENCODER_LIB_VL2`
pub const SBRENCODER_LIB_VL2: i32 = 0;

/// `SFB(dwnsmp)`: SBR Frequency bands: 64 for dual-rate, 32 for single-rate
const fn sfb(dwnsmp: i32) -> i32 {
    32 << (dwnsmp - 1)
}
/// `STS(fl)`: SBR Time Slots: 32 for core frame length 1024, 30 for core frame length 960
const fn sts(fl: i32) -> i32 {
    if fl == 1024 {
        32
    } else {
        30
    }
}
/// `DELAY_QMF_ANA(dwnsmp)`: Full bandwidth
const fn delay_qmf_ana(dwnsmp: i32) -> i32 {
    (320 << (dwnsmp - 1)) - (32 << (dwnsmp - 1))
}
/// `DELAY_HYB_ANA`
const DELAY_HYB_ANA: i32 = 10 * 64;
/// `DELAY_HYB_SYN`
const DELAY_HYB_SYN: i32 = 6 * 64 - 32;
/// `DELAY_QMF_POSTPROC(dwnsmp)`: QMF postprocessing delay
const fn delay_qmf_postproc(dwnsmp: i32) -> i32 {
    32 * dwnsmp
}
/// `DELAY_DEC_QMF(dwnsmp)`: Decoder QMF overlap
const fn delay_dec_qmf(dwnsmp: i32) -> i32 {
    6 * sfb(dwnsmp)
}
/// `DELAY_QMF_SYN(dwnsmp)`: QMF_NO_POLY/2=2.5, rounded down to 2, half for single-rate
const fn delay_qmf_syn(dwnsmp: i32) -> i32 {
    1 << (dwnsmp - 1)
}
/// `DELAY_QMF_DS`: QMF synthesis for downsampled time signal
const DELAY_QMF_DS: i32 = 32;

/// `DELAY_SBR(fl, dwnsmp)`: Delay in QMF paths
const fn delay_sbr(fl: i32, dwnsmp: i32) -> i32 {
    delay_qmf_ana(dwnsmp) + (sfb(dwnsmp) * sts(fl) - 1) + delay_qmf_syn(dwnsmp)
}
/// `DELAY_PS(fl, dwnsmp)`
const fn delay_ps(fl: i32, dwnsmp: i32) -> i32 {
    delay_qmf_ana(dwnsmp)
        + DELAY_HYB_ANA
        + delay_dec_qmf(dwnsmp)
        + (sfb(dwnsmp) * sts(fl) - 1)
        + DELAY_HYB_SYN
        + delay_qmf_syn(dwnsmp)
}
/// `DELAY_ELDSBR(fl, dwnsmp)`
const fn delay_eldsbr(fl: i32, dwnsmp: i32) -> i32 {
    ((fl / 2) * dwnsmp) - 1 + delay_qmf_postproc(dwnsmp)
}
/// `DELAY_ELDv2SBR(fl, dwnsmp)`: 80 is the delay caused by the sum of the CLD analysis and the
/// MPSLD synthesis filterbank
const fn delay_eldv2sbr(fl: i32, dwnsmp: i32) -> i32 {
    ((fl / 2) * dwnsmp) - 1 + 80 * dwnsmp
}

/// `DELAY_COREPATH_SBR(fl, dwnsmp)`: Delay in core path (core and downsampler not taken into
/// account)
const fn delay_corepath_sbr(_fl: i32, dwnsmp: i32) -> i32 {
    delay_qmf_ana(dwnsmp) + delay_dec_qmf(dwnsmp) + delay_qmf_syn(dwnsmp)
}
/// `DELAY_COREPATH_ELDSBR(fl, dwnsmp)`
const fn delay_corepath_eldsbr(_fl: i32, dwnsmp: i32) -> i32 {
    delay_qmf_postproc(dwnsmp)
}
/// `DELAY_COREPATH_ELDv2SBR(fl, dwnsmp)`: 4 slots
const fn delay_corepath_eldv2sbr(_fl: i32, dwnsmp: i32) -> i32 {
    128 * dwnsmp
}
/// `DELAY_COREPATH_PS(fl, dwnsmp)`: 2048 - 463*2
const fn delay_corepath_ps(_fl: i32, dwnsmp: i32) -> i32 {
    delay_qmf_ana(dwnsmp)
        + DELAY_QMF_DS
        + delay_qmf_ana(dwnsmp)
        + delay_dec_qmf(dwnsmp)
        + DELAY_HYB_SYN
        + delay_qmf_syn(dwnsmp)
}

/// `DELAY_AAC2SBR(fl, dwnsmp)`: Delay differences between SBR- and downsampled path for SBR
const fn delay_aac2sbr(fl: i32, dwnsmp: i32) -> i32 {
    delay_corepath_sbr(fl, dwnsmp) - delay_sbr(fl, dwnsmp)
}

/// `MAX_DS_FILTER_DELAY`: the additional max downsampler filter delay (source fs)
pub const MAX_DS_FILTER_DELAY: i32 = 5;
/// `MAX_SAMPLE_DELAY`: maximum delay: frame length of 1024 and dual-rate sbr
pub const MAX_SAMPLE_DELAY: i32 = delay_aac2sbr(1024, 2) + MAX_DS_FILTER_DELAY;

/// `DELAY_PARAM`: Delay parameters for sbrEncoder_Init_delay()
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DelayParam {
    /// `dsDelay`: the delay of the (time-domain) downsampler itself
    ds_delay: i32,
    /// `delay`: overall delay / samples
    delay: i32,
    /// `sbrDecDelay`: SBR decoder's delay
    sbr_dec_delay: i32,
    /// `corePathOffset`: core path offset / samples; added by sbrEncoder_Init_delay()
    core_path_offset: i32,
    /// `sbrPathOffset`: SBR path offset / samples; added by sbrEncoder_Init_delay()
    sbr_path_offset: i32,
    /// `bitstrDelay`: bitstream delay / frames; added by sbrEncoder_Init_delay()
    bitstr_delay: i32,
    /// `delayInput2Core`: the delay of the input to the core / samples
    delay_input2_core: i32,
}

/// `INVALID_TABLE_IDX`
const INVALID_TABLE_IDX: i32 = -1;

/// `DISTANCE_CEIL_VALUE`
const DISTANCE_CEIL_VALUE: u32 = 5_000_000;

/// `getSbrTuningTableIndex(bitrate, numChannels, sampleRate, core, pBitRateClosest)`: selects
/// the SBR tuning settings dependent on number of channels, bitrate, sample rate and core coder.
/// Returns the index into [`SBR_TUNING_TABLE`] or -1.
fn get_sbr_tuning_table_index(
    bitrate: u32,
    num_channels: u32,
    sample_rate: u32,
    core: AudioObjectType,
    p_bit_rate_closest: Option<&mut u32>,
) -> i32 {
    let mut bit_rate_closest_lower_index = -1i32;
    let mut bit_rate_closest_upper_index = -1i32;
    let mut found = 0;
    let mut bit_rate_closest_upper: u32 = 0;
    let mut bit_rate_closest_lower: u32 = DISTANCE_CEIL_VALUE;

    let is_for_this_core = |t: &SbrTuningTable| {
        (t.core_coder == CODEC_AACLD && core == AOT_ER_AAC_ELD)
            || (t.core_coder == CODEC_AAC && core != AOT_ER_AAC_ELD)
    };

    for (i, t) in SBR_TUNING_TABLE.iter().enumerate().take(SBR_TUNING_TABLE_SIZE as usize) {
        // tuning table is for this core codec
        if is_for_this_core(t)
            && num_channels == t.num_channels as u32
            && sample_rate == t.sample_rate
        {
            found = 1;
            if bitrate >= t.bitrate_from && bitrate < t.bitrate_to {
                return i as i32;
            } else {
                if t.bitrate_from > bitrate && t.bitrate_from < bit_rate_closest_lower {
                    bit_rate_closest_lower = t.bitrate_from;
                    bit_rate_closest_lower_index = i as i32;
                }
                if t.bitrate_to <= bitrate && t.bitrate_to > bit_rate_closest_upper {
                    bit_rate_closest_upper = t.bitrate_to - 1;
                    bit_rate_closest_upper_index = i as i32;
                }
            }
        }
    }

    if bit_rate_closest_upper_index >= 0 {
        return bit_rate_closest_upper_index;
    }

    if let Some(p) = p_bit_rate_closest {
        // If there was at least one matching tuning entry pick the least distance bit rate
        if found != 0 {
            let mut distance_upper = DISTANCE_CEIL_VALUE as i32;
            let mut distance_lower = DISTANCE_CEIL_VALUE as i32;
            if bit_rate_closest_lower_index >= 0 {
                // C: UINT - UINT assigned to int
                distance_lower = SBR_TUNING_TABLE[bit_rate_closest_lower_index as usize]
                    .bitrate_from
                    .wrapping_sub(bitrate) as i32;
            }
            if bit_rate_closest_upper_index >= 0 {
                distance_upper = bitrate.wrapping_sub(
                    SBR_TUNING_TABLE[bit_rate_closest_upper_index as usize].bitrate_to,
                ) as i32;
            }
            if distance_upper < distance_lower {
                *p = bit_rate_closest_upper;
            } else {
                *p = bit_rate_closest_lower;
            }
        } else {
            *p = 0;
        }
    }

    INVALID_TABLE_IDX
}

/// `getPsTuningTableIndex(bitrate, pBitRateClosest)`: selects the PS tuning settings dependent
/// on bitrate. Returns the index into [`PS_TUNING_TABLE`] or -1.
///
/// Quirk (kept): the distances to the closest bitrates are computed with the entries of
/// `sbrTuningTable` at the indices found in `psTuningTable`.
fn get_ps_tuning_table_index(bitrate: u32, p_bit_rate_closest: Option<&mut u32>) -> i32 {
    let mut bit_rate_closest_lower_index = -1i32;
    let mut bit_rate_closest_upper_index = -1i32;
    let mut bit_rate_closest_upper: u32 = 0;
    let mut bit_rate_closest_lower: u32 = DISTANCE_CEIL_VALUE;

    for (i, t) in PS_TUNING_TABLE.iter().enumerate() {
        if bitrate >= t.bitrate_from && bitrate < t.bitrate_to {
            return i as i32;
        } else {
            if t.bitrate_from > bitrate && t.bitrate_from < bit_rate_closest_lower {
                bit_rate_closest_lower = t.bitrate_from;
                bit_rate_closest_lower_index = i as i32;
            }
            if t.bitrate_to <= bitrate && t.bitrate_to > bit_rate_closest_upper {
                bit_rate_closest_upper = t.bitrate_to - 1;
                bit_rate_closest_upper_index = i as i32;
            }
        }
    }

    if bit_rate_closest_upper_index >= 0 {
        return bit_rate_closest_upper_index;
    }

    if let Some(p) = p_bit_rate_closest {
        let mut distance_upper = DISTANCE_CEIL_VALUE as i32;
        let mut distance_lower = DISTANCE_CEIL_VALUE as i32;
        if bit_rate_closest_lower_index >= 0 {
            distance_lower = SBR_TUNING_TABLE[bit_rate_closest_lower_index as usize]
                .bitrate_from
                .wrapping_sub(bitrate) as i32;
        }
        if bit_rate_closest_upper_index >= 0 {
            distance_upper = bitrate
                .wrapping_sub(SBR_TUNING_TABLE[bit_rate_closest_upper_index as usize].bitrate_to)
                as i32;
        }
        if distance_upper < distance_lower {
            *p = bit_rate_closest_upper;
        } else {
            *p = bit_rate_closest_lower;
        }
    }

    INVALID_TABLE_IDX
}

/// `FDKsbrEnc_GetDownsampledStopFreq(sampleRateCore, startFreq, stopFreq, downSampleFactor)`:
/// the stop frequency index lowered to fit into the lower half of the spectrum, -1 on error.
fn fdk_sbr_enc_get_downsampled_stop_freq(
    sample_rate_core: i32,
    start_freq: i32,
    mut stop_freq: i32,
    down_sample_factor: i32,
) -> i32 {
    let max_stop_freq_raw = sample_rate_core / 2;
    let mut start_band = 0;
    let mut stop_band = 0;

    while stop_freq > 0
        && fdk_sbr_enc_get_sbr_stop_freq_raw(stop_freq, sample_rate_core) > max_stop_freq_raw
    {
        stop_freq -= 1;
    }

    if fdk_sbr_enc_get_sbr_stop_freq_raw(stop_freq, sample_rate_core) > max_stop_freq_raw {
        return -1;
    }

    let err = fdk_sbr_enc_find_start_and_stop_band(
        sample_rate_core.wrapping_shl((down_sample_factor - 1) as u32),
        sample_rate_core,
        32i32.wrapping_shl((down_sample_factor - 1) as u32),
        start_freq,
        stop_freq,
        &mut start_band,
        &mut stop_band,
    );
    if err != 0 {
        return -1;
    }

    stop_freq
}

/// `FDKsbrEnc_IsSbrSettingAvail(bitrate, vbrMode, numOutputChannels, sampleRateInput,
/// sampleRateCore, core)`: 1 if an SBR setting is available.
fn fdk_sbr_enc_is_sbr_setting_avail(
    mut bitrate: u32,
    vbr_mode: u32,
    num_output_channels: u32,
    sample_rate_input: u32,
    sample_rate_core: u32,
    core: AudioObjectType,
) -> u32 {
    if sample_rate_input < 16000 {
        return 0;
    }

    if bitrate == 0 {
        // map vbr quality to bitrate
        bitrate = if vbr_mode < 30 {
            24000
        } else if vbr_mode < 40 {
            28000
        } else if vbr_mode < 60 {
            32000
        } else if vbr_mode < 75 {
            40000
        } else {
            48000
        };
        bitrate = bitrate.wrapping_mul(num_output_channels);
    }

    let idx =
        get_sbr_tuning_table_index(bitrate, num_output_channels, sample_rate_core, core, None);

    if idx == INVALID_TABLE_IDX {
        0
    } else {
        1
    }
}

/// `FDKsbrEnc_AdjustSbrSettings(config, bitRate, numChannels, sampleRateCore, sampleRateSbr,
/// transFac, standardBitrate, vbrMode, useSpeechConfig, lcsMode, bParametricStereo, core)`:
/// adjusts the SBR settings to the core coder settings. Returns 1 on success.
#[allow(clippy::too_many_arguments)]
fn fdk_sbr_enc_adjust_sbr_settings(
    config: &mut SbrConfiguration,
    mut bit_rate: u32,
    num_channels: u32,
    sample_rate_core: u32,
    sample_rate_sbr: u32,
    trans_fac: u32,
    standard_bitrate: u32,
    vbr_mode: u32,
    use_speech_config: u32,
    _lcs_mode: u32,
    b_parametric_stereo: u32,
    core: AudioObjectType,
) -> u32 {
    // set the core codec settings
    config.codec_settings.bit_rate = bit_rate as i32;
    config.codec_settings.n_channels = num_channels as i32;
    config.codec_settings.sample_freq = sample_rate_core as i32;
    config.codec_settings.trans_fac = trans_fac as i32;
    config.codec_settings.standard_bitrate = standard_bitrate as i32;

    if bit_rate < 28000 {
        config.threshold_amp_res_ff_m = MAXVAL_DBL;
        config.threshold_amp_res_ff_e = 7;
    } else if (28000..=48000).contains(&bit_rate) {
        // The float threshold is 75. 0.524288f is fractional part of RELAXATION, the
        // quotaMatrix and therefore tonality are scaled by this 2/3 is because the original
        // implementation divides the tonality values by 3, here it's divided by 2 128
        // compensates the necessary shiftfactor of 7
        config.threshold_amp_res_ff_m =
            fl2fxconst_dbl((75.0f32 * 0.524288f32 / (2.0f32 / 3.0f32) / 128.0f32) as f64);
        config.threshold_amp_res_ff_e = 7;
    } else if bit_rate > 48000 {
        config.threshold_amp_res_ff_m = 0;
        config.threshold_amp_res_ff_e = 0;
    }

    if bit_rate == 0 {
        // map vbr quality to bitrate
        bit_rate = if vbr_mode < 30 {
            24000
        } else if vbr_mode < 40 {
            28000
        } else if vbr_mode < 60 {
            32000
        } else if vbr_mode < 75 {
            40000
        } else {
            48000
        };
        bit_rate = bit_rate.wrapping_mul(num_channels);
        // fix to enable mono vbrMode<40 @ 44.1 of 48kHz
        if num_channels == 1
            && (sample_rate_sbr == 44100 || sample_rate_sbr == 48000)
            && vbr_mode < 40
        {
            bit_rate = 32000;
        }
    }

    let idx = get_sbr_tuning_table_index(bit_rate, num_channels, sample_rate_core, core, None);

    if idx == INVALID_TABLE_IDX {
        return 0;
    }
    let t = &SBR_TUNING_TABLE[idx as usize];
    config.start_freq = t.start_freq as i32;
    config.stop_freq = t.stop_freq as i32;
    if use_speech_config != 0 {
        config.start_freq = t.start_freq_speech as i32;
        config.stop_freq = t.stop_freq_speech as i32;
    }

    // Adapt stop frequency in case of downsampled SBR - only 32 bands then
    if 1 == config.down_sample_factor {
        let ds_stop_freq = fdk_sbr_enc_get_downsampled_stop_freq(
            sample_rate_core as i32,
            config.start_freq,
            config.stop_freq,
            config.down_sample_factor,
        );
        if ds_stop_freq < 0 {
            return 0;
        }

        config.stop_freq = ds_stop_freq;
    }

    config.sbr_noise_bands = t.num_noise_bands as i32;
    if core == AOT_ER_AAC_ELD {
        config.init_amp_res_ff = SBR_AMP_RES_1_5 as u8;
    }
    config.noise_floor_offset = t.noise_floor_offset as i32;

    config.ana_max_level = t.noise_max_level as i32;
    config.stereo_mode = t.stereo_mode;
    config.freq_scale = t.freq_scale;

    if num_channels == 1 {
        // stereo case
        match core {
            AOT_AAC_LC => {
                if bit_rate <= (if use_speech_config != 0 { 24000 } else { 20000 }) {
                    config.freq_res_fixfix[0] = FREQ_RES_LOW; // set low frequency resolution for non-split frames
                    config.freq_res_fixfix[1] = FREQ_RES_LOW; // set low frequency resolution for split frames
                }
            }
            AOT_ER_AAC_ELD => {
                if bit_rate < 36000 {
                    config.freq_res_fixfix[1] = FREQ_RES_LOW; // set low frequency resolution for split frames
                }
                if bit_rate < 26000 {
                    config.freq_res_fixfix[0] = FREQ_RES_LOW; // set low frequency resolution for non-split frames
                    config.f_res_trans_is_low = 1; // for transient frames, set low frequency resolution
                }
            }
            _ => {}
        }
    } else {
        // stereo case
        match core {
            AOT_AAC_LC => {
                if bit_rate <= 28000 {
                    config.freq_res_fixfix[0] = FREQ_RES_LOW; // set low frequency resolution for non-split frames
                    config.freq_res_fixfix[1] = FREQ_RES_LOW; // set low frequency resolution for split frames
                }
            }
            AOT_ER_AAC_ELD => {
                if bit_rate < 72000 {
                    config.freq_res_fixfix[1] = FREQ_RES_LOW; // set low frequency resolution for split frames
                }
                if bit_rate < 52000 {
                    config.freq_res_fixfix[0] = FREQ_RES_LOW; // set low frequency resolution for non-split frames
                    config.f_res_trans_is_low = 1; // for transient frames, set low frequency resolution
                }
            }
            _ => {}
        }
        if bit_rate <= 28000 {
            // additionally restrict frequency resolution in FIXFIX frames to further reduce SBR
            // payload size
            config.freq_res_fixfix[0] = FREQ_RES_LOW;
            config.freq_res_fixfix[1] = FREQ_RES_LOW;
        }
    }

    // adjust usage of parametric coding dependent on bitrate and speech config flag
    if use_speech_config != 0 {
        config.parametric_coding = 0;
    }

    if core == AOT_ER_AAC_ELD {
        if bit_rate < 28000 {
            config.init_amp_res_ff = SBR_AMP_RES_3_0 as u8;
        }
        config.send_header_data_time = -1;
    }

    if num_channels == 1 {
        if bit_rate < 16000 {
            config.parametric_coding = 0;
        }
    } else if bit_rate < 20000 {
        config.parametric_coding = 0;
    }

    config.use_speech_config = use_speech_config;

    // PS settings
    config.b_parametric_stereo = b_parametric_stereo as i32;

    1
}

/// `FDKsbrEnc_InitializeSbrDefaults(config, downSampleFactor, codecGranuleLen, isLowDelay)`:
/// initializes the SBR configuration. Returns 1 on success.
fn fdk_sbr_enc_initialize_sbr_defaults(
    config: &mut SbrConfiguration,
    down_sample_factor: i32,
    codec_granule_len: u32,
    is_low_delay: i32,
) -> u32 {
    if !(1..=2).contains(&down_sample_factor)
        || codec_granule_len.wrapping_mul(down_sample_factor as u32) > 64 * 32
    {
        return 0; // error
    }

    config.send_header_data_time = 1000;
    config.use_wave_coding = 0;
    config.crc_sbr = 0;
    config.dyn_bw_supported = 1;
    config.tran_thr = if is_low_delay != 0 { 6000 } else { 13000 };

    config.parametric_coding = 1;

    config.sbr_frame_size = (codec_granule_len as i32).wrapping_mul(down_sample_factor);
    config.down_sample_factor = down_sample_factor;

    // sbr default parameters
    config.sbr_data_extra = 0;
    config.amp_res = SBR_AMP_RES_3_0;
    config.tran_fc = 0;
    config.tran_det_mode = 1;
    config.spread = 1;
    config.stat = 0;
    config.e = 1;
    config.delta_t_across_frames = 1;
    config.d_f_edge_1st_env = fl2fxconst_dbl(0.3f32 as f64);
    config.d_f_edge_incr = fl2fxconst_dbl(0.3f32 as f64);

    config.sbr_invf_mode = INVF_SWITCHED;
    config.sbr_xpos_mode = XPOS_LC;
    config.sbr_xpos_ctrl = SBR_XPOS_CTRL_DEFAULT;
    config.sbr_xpos_level = 0;
    config.use_sa_pan = 0;
    config.dyn_bw_enabled = 0;

    // the following parameters are overwritten by the FDKsbrEnc_AdjustSbrSettings() function
    // since they are included in the tuning table
    config.stereo_mode = SBR_SWITCH_LRC;
    config.ana_max_level = 6;
    config.noise_floor_offset = 0;
    config.start_freq = 5; // 5.9 respectively 6.0 kHz at fs = 44.1/48 kHz
    config.stop_freq = 9; // 16.2 respectively 16.8 kHz at fs = 44.1/48 kHz
    config.freq_res_fixfix[0] = FREQ_RES_HIGH; // non-split case
    config.freq_res_fixfix[1] = FREQ_RES_HIGH; // split case
    config.f_res_trans_is_low = 0; // for transient frames, set variable frequency resolution according to freqResTable

    // header_extra_1
    config.freq_scale = SBR_FREQ_SCALE_DEFAULT as u8;
    config.alter_scale = SBR_ALTER_SCALE_DEFAULT;
    config.sbr_noise_bands = SBR_NOISE_BANDS_DEFAULT;

    // header_extra_2
    config.sbr_limiter_bands = SBR_LIMITER_BANDS_DEFAULT;
    config.sbr_limiter_gains = SBR_LIMITER_GAINS_DEFAULT;
    config.sbr_interpol_freq = SBR_INTERPOL_FREQ_DEFAULT;
    config.sbr_smoothing_length = SBR_SMOOTHING_LENGTH_DEFAULT;

    1
}

/// `deleteEnvChannel(hEnvCut)`: frees the memory of one SBR channel.
fn delete_env_channel(h_env_cut: Option<&mut EnvChannel>) {
    if let Some(h) = h_env_cut {
        fdk_sbr_enc_delete_ton_corr_param_extr(&mut h.ton_corr);

        fdk_sbr_enc_delete_extract_sbr_envelope(Some(&mut h.sbr_extract_envelope));
    }
}

/// `sbrEncoder_ChannelClose(hSbrChannel)`
fn sbr_encoder_channel_close(h_sbr_channel: Option<&mut SbrChannel>) {
    if let Some(h) = h_sbr_channel {
        delete_env_channel(Some(&mut h.h_env_channel));
    }
}

/// `sbrEncoder_Close(phSbrEncoder)`: closes and frees the encoder instance.
pub fn sbr_encoder_close(ph_sbr_encoder: &mut Option<Box<SbrEncoder>>) {
    if let Some(h) = ph_sbr_encoder.as_deref_mut() {
        for el in h.sbr_element.iter_mut() {
            // sbrEncoder_ElementClose(): frees the tables and the element
            *el = None;
        }

        // Close sbr Channels
        for ch in 0..MAX_SBR_ELEMENTS {
            if let Some(c) = h.p_sbr_channel[ch].as_deref_mut() {
                sbr_encoder_channel_close(Some(c));
            }
            h.p_sbr_channel[ch] = None;

            h.qmf_analysis_states[ch] = Vec::new();
            h.qmf_analysis[ch].filter_states = false;
        }

        if h.h_parametric_stereo.is_some() {
            ps_enc_destroy(&mut h.h_parametric_stereo);
        }
        h.qmf_synthesis_ps_states = Vec::new();
        h.qmf_synthesis_ps.filter_states = false;

        // Release Overlay
        h.dynamic_ram = Vec::new();
    }
    *ph_sbr_encoder = None;
}

/// `updateFreqBandTable(sbrConfigData, sbrHeaderData, downSampleFactor)`: updates vk_master
/// and the frequency band tables. Returns 0 on success.
fn update_freq_band_table(
    sbr_config_data: &mut SbrConfigData,
    sbr_header_data: &mut SbrHeaderData,
    down_sample_factor: i32,
) -> i32 {
    let mut k0 = 0;
    let mut k2 = 0;

    if fdk_sbr_enc_find_start_and_stop_band(
        sbr_config_data.sample_freq,
        sbr_config_data.sample_freq.wrapping_shr((down_sample_factor - 1) as u32),
        sbr_config_data.no_qmf_bands,
        sbr_header_data.sbr_start_frequency,
        sbr_header_data.sbr_stop_frequency,
        &mut k0,
        &mut k2,
    ) != 0
    {
        return 1;
    }

    if fdk_sbr_enc_update_freq_scale(
        &mut sbr_config_data.v_k_master,
        &mut sbr_config_data.num_master,
        k0,
        k2,
        sbr_header_data.freq_scale,
        sbr_header_data.alter_scale,
    ) != 0
    {
        return 1;
    }

    sbr_header_data.sbr_xover_band = 0;

    if fdk_sbr_enc_update_hi_res(
        &mut sbr_config_data.freq_band_table_hi,
        &mut sbr_config_data.n_sfb[HI],
        &sbr_config_data.v_k_master,
        sbr_config_data.num_master,
        &mut sbr_header_data.sbr_xover_band,
    ) != 0
    {
        return 1;
    }

    let n_hi = sbr_config_data.n_sfb[HI];
    fdk_sbr_enc_update_lo_res(
        &mut sbr_config_data.freq_band_table_lo,
        &mut sbr_config_data.n_sfb[LO],
        &sbr_config_data.freq_band_table_hi,
        n_hi,
    );

    sbr_config_data.x_over_freq = ((sbr_config_data.freq_band_table_lo[0] as i32)
        .wrapping_mul(sbr_config_data.sample_freq)
        / sbr_config_data.no_qmf_bands
        + 1)
        >> 1;

    0
}

/// `resetEnvChannel(sbrConfigData, sbrHeaderData, hEnv)`: resets the channel to new frequency
/// tables. Returns 0 on success.
fn reset_env_channel(
    sbr_config_data: &SbrConfigData,
    sbr_header_data: &SbrHeaderData,
    h_env: &mut EnvChannel,
) -> i32 {
    // note !!! hEnv->encEnvData.noOfnoisebands will be updated later in function
    // FDKsbrEnc_extractSbrEnvelope !!!
    h_env.ton_corr.sbr_noise_floor_estimate.noise_bands = sbr_header_data.sbr_noise_bands;

    if fdk_sbr_enc_reset_ton_corr_param_extr(
        &mut h_env.ton_corr,
        sbr_config_data.xpos_ctrl_switch,
        sbr_config_data.freq_band_table_hi[0] as i32,
        &sbr_config_data.v_k_master,
        sbr_config_data.num_master,
        sbr_config_data.sample_freq,
        &sbr_config_data.freq_band_table_lo,
        &sbr_config_data.n_sfb,
        sbr_config_data.no_qmf_bands,
    ) != 0
    {
        return 1;
    }

    h_env.sbr_code_noise_floor.n_sfb[LO] = h_env.ton_corr.sbr_noise_floor_estimate.no_noise_bands;
    h_env.sbr_code_noise_floor.n_sfb[HI] = h_env.ton_corr.sbr_noise_floor_estimate.no_noise_bands;

    h_env.sbr_code_envelope.n_sfb[LO] = sbr_config_data.n_sfb[LO];
    h_env.sbr_code_envelope.n_sfb[HI] = sbr_config_data.n_sfb[HI];

    h_env.enc_env_data.no_harmonics = sbr_config_data.n_sfb[HI];

    h_env.sbr_code_envelope.up_date = 0;
    h_env.sbr_code_noise_floor.up_date = 0;

    0
}

/// `FDKsbrEnc_SbrGetXOverFreq(hEnv, xoverFreq)`: the closest crossover frequency SBR accepts.
fn fdk_sbr_enc_sbr_get_x_over_freq(h_env: &SbrElement, xover_freq: i32) -> i32 {
    let cfg = &h_env.sbr_config_data;
    let p_vk_master = &cfg.v_k_master;

    // Check if there is a matching cutoff frequency in the master table
    let cutoff_sb =
        (4i32.wrapping_mul(xover_freq).wrapping_mul(cfg.no_qmf_bands) / cfg.sample_freq + 1) >> 1;
    let mut last_diff = cutoff_sb;
    let mut band = 0i32;
    while band < cfg.num_master {
        let new_diff = fixp_abs(p_vk_master[band as usize] as i32 - cutoff_sb);

        if new_diff >= last_diff {
            band -= 1;
            break;
        }

        last_diff = new_diff;
        band += 1;
    }

    // Bounded: where the reference has no defined result, the port reads 0.
    let v = if band >= 0 { p_vk_master.get(band as usize).copied().unwrap_or(0) as i32 } else { 0 };
    (v.wrapping_mul(cfg.sample_freq) / cfg.no_qmf_bands + 1) >> 1
}

/// The channels of an element (`hSbrElement->sbrChannel[0..n]`), mutable. A channel index that
/// repeats an earlier one (aliasing in C) or a missing channel is skipped.
fn pick_channels<'a>(
    chans: &'a mut [Option<Box<SbrChannel>>; MAX_SBR_ELEMENTS],
    idx: &[Option<usize>; 2],
    n: usize,
) -> Vec<&'a mut SbrChannel> {
    let mut all: [Option<&mut SbrChannel>; MAX_SBR_ELEMENTS] =
        chans.each_mut().map(|c| c.as_deref_mut());
    idx.iter()
        .take(n)
        .filter_map(|i| i.and_then(|i| all.get_mut(i).and_then(|c| c.take())))
        .collect()
}

/// `FDKsbrEnc_EnvEncodeFrame(hEnvEncoder, iElement, samples, samplesBufSize, sbrDataBits,
/// sbrData, clearOutput)`: the SBR envelope calculation of one element. `samples` (always
/// deinterleaved, channel stride `samplesBufSize`) is `input[samples..]`. Returns 0 on
/// success.
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_env_encode_frame(
    h_env_encoder: Option<&mut SbrEncoder>,
    i_element: i32,
    input: &mut [IntPcm],
    samples: usize,
    samples_buf_size: u32,
    sbr_data_bits: Option<&mut u32>,
    sbr_data: Option<&mut [u8; MAX_PAYLOAD_SIZE]>,
    clear_output: i32,
) -> i32 {
    let Some(enc) = h_env_encoder else {
        return -1;
    };

    let SbrEncoder {
        sbr_element,
        p_sbr_channel,
        qmf_analysis,
        qmf_analysis_states,
        dynamic_ram,
        h_parametric_stereo,
        qmf_synthesis_ps,
        qmf_synthesis_ps_states,
        down_sample_factor,
        n_bitstr_delay,
        ..
    } = enc;
    let n_bitstr_delay = (*n_bitstr_delay).clamp(0, MAX_DELAY_FRAMES as i32) as usize;

    let Some(h_sbr_element) =
        sbr_element.get_mut(i_element.max(0) as usize).and_then(|e| e.as_deref_mut())
    else {
        return -1;
    };

    // header bitstream handling
    let sbr_bitstream_data = &mut h_sbr_element.sbr_bitstream_data;

    let mut ps_header_active = 0;
    sbr_bitstream_data.header_active = 0;

    // Anticipate PS header because of internal PS bitstream delay in order to be in sync with
    // SBR header.
    if sbr_bitstream_data.count_send_header_data == sbr_bitstream_data.nr_send_header_data - 1 {
        ps_header_active = 1;
    }

    // Signal SBR header to be written into bitstream
    if sbr_bitstream_data.count_send_header_data == 0 {
        sbr_bitstream_data.header_active = 1;
    }

    // Increment header interval counter
    if sbr_bitstream_data.nr_send_header_data == 0 {
        sbr_bitstream_data.count_send_header_data = 1;
    } else if sbr_bitstream_data.count_send_header_data >= 0 {
        sbr_bitstream_data.count_send_header_data += 1;
        // C: % NrSendHeaderData (NrSendHeaderData is -1 after sbrEncoder_GetHeader(), where
        // CountSendHeaderData is -1, so this is not reached with -1)
        sbr_bitstream_data.count_send_header_data = sbr_bitstream_data
            .count_send_header_data
            .wrapping_rem(sbr_bitstream_data.nr_send_header_data);
    }

    let n_ch = h_sbr_element.sbr_config_data.n_channels.clamp(0, 2) as usize;

    if h_sbr_element.cmon_data.dyn_bw_enabled != 0 {
        for i in (1..=4).rev() {
            h_sbr_element.dyn_x_over_freq_delay[i] = h_sbr_element.dyn_x_over_freq_delay[i - 1];
        }

        h_sbr_element.dyn_x_over_freq_delay[0] = h_sbr_element.cmon_data.dyn_x_over_freq_enc;
        let new_x_over =
            if h_sbr_element.dyn_x_over_freq_delay[1] > h_sbr_element.dyn_x_over_freq_delay[2] {
                h_sbr_element.dyn_x_over_freq_delay[2]
            } else {
                h_sbr_element.dyn_x_over_freq_delay[1]
            };

        // has the crossover frequency changed?
        if h_sbr_element.sbr_config_data.dyn_x_over_freq != new_x_over {
            let cfg = &h_sbr_element.sbr_config_data;
            // get corresponding master band
            let cutoff_sb = (4i32.wrapping_mul(new_x_over).wrapping_mul(cfg.no_qmf_bands)
                / cfg.sample_freq
                + 1)
                >> 1;

            let mut band = 0;
            while band < cfg.num_master {
                if cutoff_sb == cfg.v_k_master[band as usize] as i32 {
                    break;
                }
                band += 1;
            }
            fdk_assert!(band < cfg.num_master);

            h_sbr_element.sbr_config_data.dyn_x_over_freq = new_x_over;
            h_sbr_element.sbr_header_data.sbr_xover_band = band;
            h_sbr_element.sbr_bitstream_data.header_active = 1;
            ps_header_active = 1; // ps header is one frame delayed

            // update vk_master table
            if update_freq_band_table(
                &mut h_sbr_element.sbr_config_data,
                &mut h_sbr_element.sbr_header_data,
                *down_sample_factor,
            ) != 0
            {
                return 1;
            }

            // reset SBR channels
            let mut chans = pick_channels(p_sbr_channel, &h_sbr_element.sbr_channel, n_ch);
            for c in chans.iter_mut() {
                if reset_env_channel(
                    &h_sbr_element.sbr_config_data,
                    &h_sbr_element.sbr_header_data,
                    &mut c.h_env_channel,
                ) != 0
                {
                    return 1;
                }
            }
        }
    }

    // allocate space for dummy header and crc. crcInfo is only used with the DRM CRC, which
    // FDKsbrEnc_InitSbrBitstream() initializes.
    let mut crc_info = FdkCrcInfo::default();
    let crc_reg = fdk_sbr_enc_init_sbr_bitstream(
        &mut h_sbr_element.cmon_data,
        &h_sbr_element.payload_delay_line,
        (n_bitstr_delay * MAX_PAYLOAD_SIZE) as u32,
        MAX_PAYLOAD_SIZE as i32,
        &mut crc_info,
        h_sbr_element.sbr_config_data.sbr_syntax_flags,
    );

    // Temporal Envelope Data, Init Temporal Envelope Data
    let mut e_data = [SbrEnvTempData::default(); MAX_NUM_CHANNELS];
    let mut f_data =
        SbrFrameTempData { res: [FREQ_RES_HIGH; MAX_NUM_NOISE_VALUES], max_quant_error: 0 };

    let mut chans = pick_channels(p_sbr_channel, &h_sbr_element.sbr_channel, n_ch);
    let buf = samples_buf_size as usize;

    if clear_output == 0 {
        // Transform audio data into QMF domain
        for ch in 0..chans.len() {
            if h_sbr_element.el_info.f_parametric_stereo == 0 {
                let mut tmp_scale = QmfScaleFactor::default();
                let mut qmf_work_buffer = [0 as FixpDbl; 64 * 2];

                // Obtain pointers to QMF buffers.
                let ex = &chans[ch].h_env_channel.sbr_extract_envelope;
                let (r_off, i_off) = (ex.r_buffer, ex.i_buffer);
                let (lo, hi) = dynamic_ram.split_at_mut(i_off);
                // (arrays of the 32 row slices, so that no frame allocates)
                let mut rows_re = lo[r_off..r_off + 32 * 64].chunks_exact_mut(64);
                let mut rows_im = hi[..32 * 64].chunks_exact_mut(64);
                let mut p_qmf_real: [&mut [FixpDbl]; 32] =
                    core::array::from_fn(|_| rows_re.next().unwrap());
                let mut p_qmf_imag: [&mut [FixpDbl]; 32] =
                    core::array::from_fn(|_| rows_im.next().unwrap());

                if let Some(q) = h_sbr_element.h_qmf_analysis[ch] {
                    let pos = samples + h_sbr_element.el_info.channel_index[ch] as usize * buf;
                    let time_in: &[IntPcm] = input.get(pos..).unwrap_or(&[]);
                    qmf_analysis_filtering::<IntPcm, &mut [FixpDbl]>(
                        &qmf_analysis[q],
                        &mut qmf_analysis_states[q],
                        &mut p_qmf_real,
                        &mut p_qmf_imag,
                        &mut tmp_scale,
                        time_in,
                        0,
                        1,
                        &mut qmf_work_buffer,
                    );
                }

                chans[ch].h_env_channel.qmf_scale = tmp_scale.lb_scale + 7;
            } // fParametricStereo == 0

            // Parametric Stereo processing
            if h_sbr_element.el_info.f_parametric_stereo != 0 {
                // Limit Parametric Stereo to one instance
                fdk_assert!(ch == 0);

                // parametric stereo processing:
                // - input: left and right time domain samples
                // - processing: stereo qmf analysis, stereo hybrid analysis, ps parameter
                //   extraction, downmix + hybrid synthesis
                // - output: downmixed qmf data is written to sbrExtrEnv->rBuffer and
                //   sbrExtrEnv->iBuffer
                let mut qmf_scale: i8 = 0;
                let p_samples = [
                    samples + h_sbr_element.el_info.channel_index[0] as usize * buf,
                    samples + h_sbr_element.el_info.channel_index[1] as usize * buf,
                ];
                let ex = &chans[ch].h_env_channel.sbr_extract_envelope;
                let (r_off, i_off) = (ex.r_buffer, ex.i_buffer);
                if let (Some(ps), Some(q0), Some(q1)) = (
                    h_parametric_stereo.as_deref_mut(),
                    h_sbr_element.h_qmf_analysis[0],
                    h_sbr_element.h_qmf_analysis[1],
                ) {
                    if q0 != q1 {
                        let (s_lo, s_hi) = qmf_analysis_states.split_at_mut(q0.max(q1));
                        let (st0, st1) = if q0 < q1 {
                            (&mut s_lo[q0][..], &mut s_hi[0][..])
                        } else {
                            (&mut s_hi[0][..], &mut s_lo[q1][..])
                        };
                        let mut qa = PsQmfAnalysis {
                            bank: [&qmf_analysis[q0], &qmf_analysis[q1]],
                            states: [st0, st1],
                        };
                        // C: the error is not used
                        let _error = fdk_sbr_enc_ps_enc_parametric_stereo_processing(
                            ps,
                            input,
                            p_samples,
                            samples_buf_size,
                            &mut qa,
                            dynamic_ram,
                            r_off,
                            i_off,
                            samples + h_sbr_element.el_info.channel_index[ch] as usize * buf,
                            qmf_synthesis_ps,
                            qmf_synthesis_ps_states,
                            &mut qmf_scale,
                            ps_header_active,
                        );
                    }
                }
                chans[ch].h_env_channel.qmf_scale = qmf_scale as i32;
            } // if (hEnvEncoder->hParametricStereo)

            // Extract Envelope relevant things from QMF data
            fdk_sbr_enc_extract_sbr_envelope1(
                &h_sbr_element.sbr_config_data,
                &mut chans[ch].h_env_channel,
                &mut e_data[ch],
                dynamic_ram,
            );
        } // hEnvEncoder->sbrConfigData.nChannels
    }

    // Process Envelope relevant things and calculate envelope data and write payload
    {
        let ps = if h_sbr_element.el_info.f_parametric_stereo != 0 {
            h_parametric_stereo.as_deref()
        } else {
            None
        };
        // C passes sbrChannel[1] only for stereo modes other than SBR_MONO
        let n_env = if h_sbr_element.sbr_config_data.stereo_mode != SBR_MONO {
            chans.len()
        } else {
            chans.len().min(1)
        };
        let mut env_chans: Vec<&mut EnvChannel> =
            chans.iter_mut().take(n_env).map(|c| &mut c.h_env_channel).collect();
        if !env_chans.is_empty() {
            fdk_sbr_enc_extract_sbr_envelope2(
                &h_sbr_element.sbr_config_data,
                &mut h_sbr_element.sbr_header_data,
                ps,
                &h_sbr_element.sbr_bitstream_data,
                &mut env_chans,
                &mut h_sbr_element.cmon_data,
                &mut e_data,
                &mut f_data,
                clear_output,
                dynamic_ram,
            );
        }
    }

    h_sbr_element.sbr_bitstream_data.right_border_fix = 0;

    // format payload, calculate crc
    fdk_sbr_enc_assemble_sbr_bitstream(
        Some(&mut h_sbr_element.cmon_data),
        &mut crc_info,
        crc_reg,
        h_sbr_element.sbr_config_data.sbr_syntax_flags,
    );

    // save new payload, set to zero length if greater than MAX_PAYLOAD_SIZE
    h_sbr_element.payload_delay_line_size[n_bitstr_delay] =
        fdk_get_valid_bits(&mut h_sbr_element.cmon_data.sbr_bitbuf);

    if h_sbr_element.payload_delay_line_size[n_bitstr_delay] > (MAX_PAYLOAD_SIZE << 3) as u32 {
        h_sbr_element.payload_delay_line_size[n_bitstr_delay] = 0;
    }

    // While filling the Delay lines, sbrData is NULL
    if let Some(d) = sbr_data {
        if let Some(bits) = sbr_data_bits {
            *bits = h_sbr_element.payload_delay_line_size[0];
        }
        let n = ((h_sbr_element.payload_delay_line_size[0] + 7) >> 3) as usize;
        h_sbr_element.payload_delay_line.read(0, &mut d[..n.min(MAX_PAYLOAD_SIZE)]);
    }

    // delay header active flag
    if h_sbr_element.sbr_bitstream_data.header_active == 1 {
        h_sbr_element.sbr_bitstream_data.header_active_delay = 1 + n_bitstr_delay as i32;
    } else if h_sbr_element.sbr_bitstream_data.header_active_delay > 0 {
        h_sbr_element.sbr_bitstream_data.header_active_delay -= 1;
    }

    0
}

/// `FDKsbrEnc_Downsample(hSbrEncoder, samples, samplesBufSize, numChannels, sbrDataBits,
/// sbrData, clearOutput)`: the downsampling and delay compensation of the core path.
/// `samples` is `input[samples..]`. Returns 0.
fn fdk_sbr_enc_downsample(
    h_sbr_encoder: &mut SbrEncoder,
    input: &mut [IntPcm],
    samples: usize,
    samples_buf_size: u32,
    num_channels: u32,
) -> i32 {
    let buf = samples_buf_size as usize;
    let mut n_out_samples = 0;
    // C: hSbrEncoder->bufferOffset / numChannels
    let buffer_offset_per_ch = if num_channels != 0 {
        (h_sbr_encoder.buffer_offset / num_channels as i32).max(0) as usize
    } else {
        0
    };
    if h_sbr_encoder.down_sample_factor > 1 {
        // Do downsampling

        // Loop over elements (LFE is handled later)
        for el in 0..h_sbr_encoder.no_elements.clamp(0, MAX_SBR_ELEMENTS as i32) as usize {
            let SbrEncoder { sbr_element, p_sbr_channel, downsampling_method, .. } =
                &mut *h_sbr_encoder;
            if let Some(h_sbr_element) = sbr_element[el].as_deref() {
                if *downsampling_method == SBRENC_DS_TIME {
                    let n_channels = h_sbr_element.sbr_config_data.n_channels.clamp(0, 2) as usize;

                    for ch in 0..n_channels {
                        let Some(c) = h_sbr_element.sbr_channel[ch]
                            .and_then(|i| p_sbr_channel[i].as_deref_mut())
                        else {
                            continue;
                        };
                        let base = samples + h_sbr_element.el_info.channel_index[ch] as usize * buf;
                        fdk_aac_enc_downsample(
                            &mut c.down_sampler,
                            input,
                            base + buffer_offset_per_ch,
                            h_sbr_element.sbr_config_data.frame_size,
                            base,
                            &mut n_out_samples,
                        );
                    }
                }
            }
        }

        // Handle LFE (if existing)
        if h_sbr_encoder.lfe_ch_idx != -1 {
            // lfe downsampler
            let base = samples + h_sbr_encoder.lfe_ch_idx.max(0) as usize * buf;
            fdk_aac_enc_downsample(
                &mut h_sbr_encoder.lfe_down_sampler,
                input,
                base + buffer_offset_per_ch,
                h_sbr_encoder.frame_size,
                base,
                &mut n_out_samples,
            );
        }
    } else {
        // No downsampling. Still, some buffer shifting for correct delay. FDKmemmove() and
        // FDKmemcpy() (for non-overlapping areas) give the same result as copy_within.
        let samples2_copy = h_sbr_encoder.frame_size.max(0) as usize;
        for c in 0..num_channels as usize {
            let dst = samples + c * buf;
            let src = dst + buffer_offset_per_ch;
            if src + samples2_copy <= input.len() {
                input.copy_within(src..src + samples2_copy, dst);
            }
        }
    }

    0
}

/// `createEnvChannel(hEnv, channel, dynamic_RAM)`: initializes the parameters and allocates
/// the memory of a channel. Returns 0 on success.
fn create_env_channel(h_env: &mut EnvChannel, channel: i32) -> i32 {
    *h_env = EnvChannel::default();

    if fdk_sbr_enc_create_ton_corr_param_extr(&mut h_env.ton_corr, channel) != 0 {
        return 1;
    }

    if fdk_sbr_enc_create_extract_sbr_envelope(
        &mut h_env.sbr_extract_envelope,
        channel,
        0, /*chan*/
    ) != 0
    {
        return 1;
    }

    0
}

/// `initEnvChannel(sbrConfigData, sbrHeaderData, hEnv, params, statesInitFlag, chanInEl,
/// dynamic_RAM)`: initializes the parameters of a channel. Returns 0 on success.
fn init_env_channel(
    sbr_config_data: &mut SbrConfigData,
    sbr_header_data: &SbrHeaderData,
    h_env: &mut EnvChannel,
    params: &SbrConfiguration,
    states_init_flag: u32,
    chan_in_el: i32,
    dyn_ram: &mut [FixpDbl],
) -> i32 {
    let tran_off;
    let mut noise_bands = [3i32; 2];

    // C: e = 1 << params->e
    let e = 1i32.wrapping_shl(params.e as u32);

    fdk_assert!(params.e >= 0);

    h_env.enc_env_data.freq_res_fixfix = params.freq_res_fixfix;
    h_env.enc_env_data.f_res_trans_is_low = params.f_res_trans_is_low;

    h_env.f_level_protect = 0;

    h_env.enc_env_data.ld_grid =
        if sbr_config_data.sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0 { 1 } else { 0 };

    h_env.enc_env_data.sbr_xpos_mode = params.sbr_xpos_mode;

    if h_env.enc_env_data.sbr_xpos_mode == XPOS_SWITCHED {
        // no other type than XPOS_MDCT or XPOS_SPEECH allowed, but enable switching
        sbr_config_data.switch_transposers = TRUE;
        h_env.enc_env_data.sbr_xpos_mode = XPOS_MDCT;
    } else {
        sbr_config_data.switch_transposers = FALSE;
    }

    h_env.enc_env_data.sbr_xpos_ctrl = params.sbr_xpos_ctrl;

    // extended data
    h_env.enc_env_data.extended_data = if params.parametric_coding != 0 { 1 } else { 0 };

    h_env.enc_env_data.extension_size = 0;

    let start_index = QMF_FILTER_PROTOTYPE_SIZE - sbr_config_data.no_qmf_bands;

    let time_slots = match params.sbr_frame_size {
        2304 => 18,
        2048 | 1024 | 512 => 16,
        1920 | 960 | 480 => 15,
        1152 => 9,
        _ => return 1, // Illegal frame size
    };

    let time_step = sbr_config_data.no_qmf_slots / time_slots;

    if fdk_sbr_enc_init_ton_corr_param_extr(
        params.sbr_frame_size,
        &mut h_env.ton_corr,
        sbr_config_data,
        time_slots,
        params.sbr_xpos_ctrl,
        params.ana_max_level,
        sbr_header_data.sbr_noise_bands,
        params.noise_floor_offset,
        params.use_speech_config,
    ) != 0
    {
        return 1;
    }

    h_env.enc_env_data.no_ofnoisebands = h_env.ton_corr.sbr_noise_floor_estimate.no_noise_bands;

    noise_bands[0] = h_env.enc_env_data.no_ofnoisebands;
    noise_bands[1] = h_env.enc_env_data.no_ofnoisebands;

    h_env.enc_env_data.sbr_invf_mode = params.sbr_invf_mode;

    if h_env.enc_env_data.sbr_invf_mode == INVF_SWITCHED {
        h_env.enc_env_data.sbr_invf_mode = INVF_MID_LEVEL;
        h_env.ton_corr.switch_inverse_filt = TRUE;
    } else {
        h_env.ton_corr.switch_inverse_filt = FALSE;
    }

    let mut tran_fc = params.tran_fc;

    if tran_fc == 0 {
        tran_fc = fix_min(
            5000,
            fdk_sbr_enc_get_sbr_start_freq_raw(
                sbr_header_data.sbr_start_frequency,
                params.codec_settings.sample_freq,
            ),
        );
    }

    tran_fc = (tran_fc.wrapping_mul(4).wrapping_mul(sbr_config_data.no_qmf_bands)
        / sbr_config_data.sample_freq
        + 1)
        >> 1;

    let frame_shift;
    if sbr_config_data.sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0 {
        frame_shift = LD_PRETRAN_OFF;
        tran_off = LD_PRETRAN_OFF + FRAME_MIDDLE_SLOT_512LD * time_step;
    } else {
        frame_shift = 0;
        tran_off = match time_slots {
            // The factor of 2 is by definition.
            NUMBER_TIME_SLOTS_2048 => 8 + FRAME_MIDDLE_SLOT_2048 * time_step,
            NUMBER_TIME_SLOTS_1920 => 7 + FRAME_MIDDLE_SLOT_1920 * time_step,
            _ => return 1,
        };
    }
    if fdk_sbr_enc_init_extract_sbr_envelope(
        &mut h_env.sbr_extract_envelope,
        sbr_config_data.no_qmf_slots,
        sbr_config_data.no_qmf_bands,
        start_index,
        time_slots,
        time_step,
        tran_off,
        states_init_flag,
        chan_in_el,
        dyn_ram,
        sbr_config_data.sbr_syntax_flags,
    ) != 0
    {
        return 1;
    }

    if fdk_sbr_enc_init_sbr_code_envelope(
        &mut h_env.sbr_code_envelope,
        &sbr_config_data.n_sfb,
        params.delta_t_across_frames,
        params.d_f_edge_1st_env,
        params.d_f_edge_incr,
    ) != 0
    {
        return 1;
    }

    if fdk_sbr_enc_init_sbr_code_envelope(
        &mut h_env.sbr_code_noise_floor,
        &noise_bands,
        params.delta_t_across_frames,
        0,
        0,
    ) != 0
    {
        return 1;
    }

    if fdk_sbr_enc_init_sbr_huffman_tables(
        &mut h_env.enc_env_data,
        &mut h_env.sbr_code_envelope,
        &mut h_env.sbr_code_noise_floor,
        sbr_header_data.sbr_amp_res,
    ) != 0
    {
        return 1;
    }

    let freq_res_fixfix = h_env.enc_env_data.freq_res_fixfix;
    fdk_sbr_enc_init_frame_info_generator(
        &mut h_env.sbr_env_frame,
        params.spread,
        e,
        params.stat,
        time_slots,
        &freq_res_fixfix,
        h_env.enc_env_data.f_res_trans_is_low,
        h_env.enc_env_data.ld_grid as i32,
    );

    if sbr_config_data.sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0 {
        let bandwidth_qmf_slot = (sbr_config_data.sample_freq >> 1) / sbr_config_data.no_qmf_bands;
        if fdk_sbr_enc_init_sbr_fast_transient_detector(
            &mut h_env.sbr_fast_transient_detector,
            sbr_config_data.no_qmf_slots,
            bandwidth_qmf_slot,
            sbr_config_data.no_qmf_bands,
            sbr_config_data.freq_band_table_lo[0] as i32,
        ) != 0
        {
            return 1;
        }
    }

    // The transient detector has to be initialized also if the fast transient detector was
    // active, because the values from the transient detector structure are used.
    if fdk_sbr_enc_init_sbr_transient_detector(
        &mut h_env.sbr_transient_detector,
        sbr_config_data.sbr_syntax_flags,
        sbr_config_data.frame_size,
        sbr_config_data.sample_freq,
        params,
        tran_fc,
        sbr_config_data.no_qmf_slots,
        sbr_config_data.no_qmf_bands,
        h_env.sbr_extract_envelope.y_buffer_write_offset,
        h_env.sbr_extract_envelope.y_buffer_sz_shift,
        frame_shift,
        tran_off,
    ) != 0
    {
        return 1;
    }

    sbr_config_data.xpos_ctrl_switch = params.sbr_xpos_ctrl;

    h_env.enc_env_data.no_harmonics = sbr_config_data.n_sfb[HI];
    h_env.enc_env_data.add_harmonic_flag = 0;

    0
}

/// `sbrEncoder_Open(phSbrEncoder, nElements, nChannels, supportPS)`: allocates an encoder
/// instance for `n_elements` elements and `n_channels` channels (at most 8 each). Returns 0 on
/// success.
pub fn sbr_encoder_open(
    ph_sbr_encoder: Option<&mut Option<Box<SbrEncoder>>>,
    n_elements: i32,
    n_channels: i32,
    support_ps: i32,
) -> i32 {
    let Some(ph) = ph_sbr_encoder else {
        return 1;
    };

    let mut h = Box::<SbrEncoder>::default();

    h.dynamic_ram = get_ram_sbr_dynamic_ram();

    // Deviation: the AAC encoder opens at most 8 elements and channels; where the reference has
    // no defined result, the port fails.
    let fail = |h: Box<SbrEncoder>| {
        let mut b = Some(h);
        sbr_encoder_close(&mut b);
        1
    };

    // Create SBR elements
    for i in 0..n_elements.max(0) as usize {
        if i >= MAX_SBR_ELEMENTS {
            return fail(h);
        }
        h.sbr_element[i] = Some(Box::default());
    }

    // Create SBR channels
    for i in 0..n_channels.max(0) as usize {
        if i >= MAX_SBR_ELEMENTS {
            return fail(h);
        }
        let mut c = Box::<SbrChannel>::default();
        if create_env_channel(&mut c.h_env_channel, i as i32) != 0 {
            return fail(h);
        }
        h.p_sbr_channel[i] = Some(c);
    }

    // Create QMF States
    for i in 0..fix_max(n_channels, if support_ps != 0 { 2 } else { 0 }).max(0) as usize {
        if i >= MAX_SBR_ELEMENTS {
            return fail(h);
        }
        h.qmf_analysis_states[i] = vec![0; QMF_STATES_ANALYSIS_SIZE];
        h.qmf_analysis[i].filter_states = true;
    }

    // Create Parametric Stereo handle
    if support_ps != 0 {
        h.h_parametric_stereo = Some(ps_enc_create());

        h.qmf_synthesis_ps_states = vec![0; PS_QMF_STATES_SYNTHESIS_SIZE];
        h.qmf_synthesis_ps.filter_states = true;
    } // supportPS

    *ph = Some(h);

    0
}

/// `FDKsbrEnc_Reallocate(hSbrEncoder, elInfo, noElements)`: assigns the channels and QMF banks
/// to the elements. Returns 0.
fn fdk_sbr_enc_reallocate(
    h_sbr_encoder: &mut SbrEncoder,
    el_info: &[SbrElementInfo; 8],
    no_elements: i32,
) -> i32 {
    let mut total_ch = 0usize;
    let mut total_qmf = 0usize;
    let mut el: i32 = -1;

    h_sbr_encoder.lfe_ch_idx = -1; // default value, until lfe found

    for core_el in 0..no_elements.clamp(0, 8) as usize {
        // SBR only handles SCE and CPE's
        if el_info[core_el].el_type == ID_SCE || el_info[core_el].el_type == ID_CPE {
            el += 1;
        } else {
            if el_info[core_el].el_type == ID_LFE {
                h_sbr_encoder.lfe_ch_idx = el_info[core_el].channel_index[0] as i32;
            }
            continue;
        }

        let pel_info = &el_info[core_el];
        let has_channel =
            |i: usize| i < MAX_SBR_ELEMENTS && h_sbr_encoder.p_sbr_channel[i].is_some();
        let ch_idx: Vec<Option<usize>> = (0..pel_info.n_channels_in_el as usize)
            .map(|k| Some(total_ch + k).filter(|&i| has_channel(i)))
            .collect();
        // Guarded: skips an element that was not opened.
        let Some(h_sbr_element) =
            h_sbr_encoder.sbr_element.get_mut(el as usize).and_then(|e| e.as_deref_mut())
        else {
            continue;
        };

        for (ch, idx) in ch_idx.iter().enumerate() {
            // C: hSbrEncoder->pSbrChannel[totalCh], NULL beyond the opened channels
            if ch < 2 {
                h_sbr_element.sbr_channel[ch] = *idx;
            }
            total_ch += 1;
        }
        // analysis QMF
        let n_qmf =
            if pel_info.f_parametric_stereo != 0 { 2 } else { pel_info.n_channels_in_el as usize };
        for ch in 0..n_qmf {
            if ch < 2 {
                h_sbr_element.el_info.channel_index[ch] = pel_info.channel_index[ch];
                h_sbr_element.h_qmf_analysis[ch] =
                    if total_qmf < MAX_SBR_ELEMENTS { Some(total_qmf) } else { None };
            }
            total_qmf += 1;
        }

        // Copy Element info
        h_sbr_element.el_info.el_type = pel_info.el_type;
        h_sbr_element.el_info.instance_tag = pel_info.instance_tag;
        h_sbr_element.el_info.n_channels_in_el = pel_info.n_channels_in_el;
        h_sbr_element.el_info.f_parametric_stereo = pel_info.f_parametric_stereo;
        h_sbr_element.el_info.f_dual_mono = pel_info.f_dual_mono;
    } // coreEl

    0
}

/// `FDKsbrEnc_bsBufInit(hSbrElement, nBitstrDelay)`: initializes the bitstream buffer.
fn fdk_sbr_enc_bs_buf_init(h_sbr_element: &mut SbrElement, n_bitstr_delay: i32) -> i32 {
    // initialize the bitstream buffer
    let row = n_bitstr_delay.clamp(0, MAX_DELAY_FRAMES as i32) as usize;
    fdk_init_bit_stream_at(
        &mut h_sbr_element.cmon_data.sbr_bitbuf,
        &h_sbr_element.payload_delay_line,
        (row * MAX_PAYLOAD_SIZE) as u32,
        MAX_PAYLOAD_SIZE as u32,
        0,
        BS_WRITER,
    );

    0
}

/// `FDKsbrEnc_EnvInit(hSbrElement, params, coreBandWith, aot, nElement, headerPeriod,
/// statesInitFlag, downsamplingMethod, dynamic_RAM)`: initializes an element (with the
/// encoder's channels, QMF banks and dynamic RAM). Returns 0 on success.
#[allow(clippy::too_many_arguments)]
fn fdk_sbr_enc_env_init(
    h_sbr_element: &mut SbrElement,
    p_sbr_channel: &mut [Option<Box<SbrChannel>>; MAX_SBR_ELEMENTS],
    qmf_analysis: &mut [QmfFilterBank; MAX_SBR_ELEMENTS],
    qmf_analysis_states: &mut [Vec<i16>; MAX_SBR_ELEMENTS],
    params: &mut SbrConfiguration,
    core_band_with: &mut i32,
    aot: AudioObjectType,
    header_period: i32,
    states_init_flag: u32,
    dyn_ram: &mut [FixpDbl],
) -> i32 {
    if params.codec_settings.n_channels < 1
        || params.codec_settings.n_channels > MAX_NUM_CHANNELS as i32
    {
        return 1;
    }

    // init and set syntax flags
    h_sbr_element.sbr_config_data.sbr_syntax_flags = 0;

    if aot == AOT_ER_AAC_ELD {
        h_sbr_element.sbr_config_data.sbr_syntax_flags |= SBR_SYNTAX_LOW_DELAY;
    }
    if params.crc_sbr != 0 {
        h_sbr_element.sbr_config_data.sbr_syntax_flags |= SBR_SYNTAX_CRC;
    }

    let SbrElement {
        sbr_config_data: cfg,
        sbr_header_data: hdr,
        sbr_bitstream_data: bsd,
        el_info,
        sbr_channel,
        h_qmf_analysis,
        ..
    } = h_sbr_element;

    // C: 64 >> (2 - params->downSampleFactor)
    cfg.no_qmf_bands = 64i32.wrapping_shr((2 - params.down_sample_factor) as u32);
    match cfg.no_qmf_bands {
        64 => cfg.no_qmf_slots = params.sbr_frame_size >> 6,
        32 => cfg.no_qmf_slots = params.sbr_frame_size >> 5,
        _ => {
            cfg.no_qmf_slots = params.sbr_frame_size >> 6;
            return 2;
        }
    }

    // now initialize sbrConfigData, sbrHeaderData and sbrBitstreamData,
    cfg.n_channels = params.codec_settings.n_channels;

    if params.codec_settings.n_channels == 2 {
        if el_info.el_type == ID_CPE && el_info.f_dual_mono == 1 {
            cfg.stereo_mode = SBR_LEFT_RIGHT;
        } else {
            cfg.stereo_mode = params.stereo_mode;
        }
    } else {
        cfg.stereo_mode = SBR_MONO;
    }

    cfg.frame_size = params.sbr_frame_size;

    cfg.sample_freq = params.down_sample_factor.wrapping_mul(params.codec_settings.sample_freq);

    bsd.count_send_header_data = 0;
    if params.send_header_data_time > 0 {
        if header_period == -1 {
            bsd.nr_send_header_data = params.send_header_data_time.wrapping_mul(cfg.sample_freq)
                / (1000i32.wrapping_mul(cfg.frame_size));
            bsd.nr_send_header_data = fix_max(bsd.nr_send_header_data, 1);
        } else {
            // assure header period at least once per second
            bsd.nr_send_header_data =
                fix_min(fix_max(header_period, 1), cfg.sample_freq / cfg.frame_size);
        }
    } else {
        bsd.nr_send_header_data = 0;
    }

    hdr.sbr_data_extra = params.sbr_data_extra;
    bsd.header_active = 0;
    bsd.right_border_fix = 0;
    hdr.sbr_start_frequency = params.start_freq;
    hdr.sbr_stop_frequency = params.stop_freq;
    hdr.sbr_xover_band = 0;
    hdr.sbr_lc_stereo_mode = 0;

    // data_extra
    if params.sbr_xpos_ctrl != SBR_XPOS_CTRL_DEFAULT {
        hdr.sbr_data_extra = 1;
    }

    hdr.sbr_amp_res = params.amp_res;
    cfg.init_amp_res_ff = params.init_amp_res_ff;

    // header_extra_1
    hdr.freq_scale = params.freq_scale as i32;
    hdr.alter_scale = params.alter_scale;
    hdr.sbr_noise_bands = params.sbr_noise_bands;
    hdr.header_extra_1 = 0;

    if params.freq_scale as i32 != SBR_FREQ_SCALE_DEFAULT
        || params.alter_scale != SBR_ALTER_SCALE_DEFAULT
        || params.sbr_noise_bands != SBR_NOISE_BANDS_DEFAULT
    {
        hdr.header_extra_1 = 1;
    }

    // header_extra_2
    hdr.sbr_limiter_bands = params.sbr_limiter_bands;
    hdr.sbr_limiter_gains = params.sbr_limiter_gains;

    if cfg.sample_freq > 48000 && hdr.sbr_start_frequency >= 9 {
        hdr.sbr_limiter_gains = SBR_LIMITER_GAINS_INFINITE;
    }

    hdr.sbr_interpol_freq = params.sbr_interpol_freq;
    hdr.sbr_smoothing_length = params.sbr_smoothing_length;
    hdr.header_extra_2 = 0;

    if params.sbr_limiter_bands != SBR_LIMITER_BANDS_DEFAULT
        || params.sbr_limiter_gains != SBR_LIMITER_GAINS_DEFAULT
        || params.sbr_interpol_freq != SBR_INTERPOL_FREQ_DEFAULT
        || params.sbr_smoothing_length != SBR_SMOOTHING_LENGTH_DEFAULT
    {
        hdr.header_extra_2 = 1;
    }

    // other switches
    cfg.use_wave_coding = params.use_wave_coding;
    cfg.use_parametric_coding = params.parametric_coding;
    cfg.threshold_amp_res_ff_m = params.threshold_amp_res_ff_m;
    cfg.threshold_amp_res_ff_e = params.threshold_amp_res_ff_e;

    // init freq band table
    if update_freq_band_table(cfg, hdr, params.down_sample_factor) != 0 {
        return 1;
    }

    // now create envelope ext and QMF for each available channel
    for ch in 0..cfg.n_channels.max(0) as usize {
        // Guarded: an error for a channel that was not opened.
        let Some(c) =
            sbr_channel.get(ch).copied().flatten().and_then(|i| p_sbr_channel[i].as_deref_mut())
        else {
            return 1;
        };
        if init_env_channel(
            cfg,
            hdr,
            &mut c.h_env_channel,
            params,
            states_init_flag,
            ch as i32,
            dyn_ram,
        ) != 0
        {
            return 1;
        }
    } // nChannels

    // reset and intialize analysis qmf
    let n_qmf = if el_info.f_parametric_stereo != 0 { 2 } else { cfg.n_channels.max(0) as usize };
    for ch in 0..n_qmf.min(2) {
        let mut qmf_flags =
            if cfg.sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0 { QMF_FLAG_CLDFB } else { 0 };
        if states_init_flag != 0 {
            qmf_flags &= !QMF_FLAG_KEEP_STATES;
        } else {
            qmf_flags |= QMF_FLAG_KEEP_STATES;
        }

        let Some(q) = h_qmf_analysis[ch] else {
            return 1;
        };
        let states = &mut qmf_analysis_states[q];
        let err = qmf_init_analysis_filter_bank::<i16>(
            &mut qmf_analysis[q],
            if states.is_empty() { None } else { Some(&mut states[..]) },
            cfg.no_qmf_slots,
            cfg.no_qmf_bands,
            cfg.no_qmf_bands,
            cfg.no_qmf_bands,
            qmf_flags,
        );
        if err != 0 {
            return err;
        }
    }

    h_sbr_element.cmon_data.x_over_freq = h_sbr_element.sbr_config_data.x_over_freq;
    h_sbr_element.cmon_data.dyn_bw_enabled =
        (params.dyn_bw_supported != 0 && params.dyn_bw_enabled != 0) as i32;
    h_sbr_element.cmon_data.dyn_x_over_freq_enc =
        fdk_sbr_enc_sbr_get_x_over_freq(h_sbr_element, h_sbr_element.cmon_data.x_over_freq);
    for i in 0..5 {
        h_sbr_element.dyn_x_over_freq_delay[i] = h_sbr_element.cmon_data.dyn_x_over_freq_enc;
    }
    h_sbr_element.cmon_data.sbr_num_channels = h_sbr_element.sbr_config_data.n_channels;
    h_sbr_element.sbr_config_data.dyn_x_over_freq = h_sbr_element.cmon_data.x_over_freq;

    // Update Bandwith to be passed to the core encoder
    *core_band_with = h_sbr_element.cmon_data.x_over_freq;

    0
}

/// `sbrEncoder_GetInBufferSize(noChannels)`: the size of the input buffer in bytes.
pub fn sbr_encoder_get_in_buffer_size(no_channels: i32) -> i32 {
    let mut temp = 2048;
    temp += 1024 + MAX_SAMPLE_DELAY;
    temp *= no_channels;
    temp *= core::mem::size_of::<IntPcm>() as i32;
    temp
}

/// `FDKsbrEnc_DelayCompensation(hEnvEnc, timeBuffer, timeBufferBufSize)`: encodes dummy SBR
/// payload frames to fill the delay lines. Returns 0 on success.
fn fdk_sbr_enc_delay_compensation(
    h_env_enc: &mut SbrEncoder,
    time_buffer: &mut [IntPcm],
    time_buffer_buf_size: u32,
) -> i32 {
    let mut n = h_env_enc.n_bitstr_delay;
    while n > 0 {
        for el in 0..h_env_enc.no_elements {
            // C: timeBuffer + hEnvEnc->downsampledOffset / hEnvEnc->nChannels
            let off = (h_env_enc.downsampled_offset / h_env_enc.n_channels).max(0) as usize;
            if fdk_sbr_enc_env_encode_frame(
                Some(h_env_enc),
                el,
                time_buffer,
                off,
                time_buffer_buf_size,
                None,
                None,
                1,
            ) != 0
            {
                return -1;
            }
        }
        sbr_encoder_update_buffers(h_env_enc, time_buffer, time_buffer_buf_size);
        n -= 1;
    }
    0
}

/// `sbrEncoder_LimitBitRate(bitRate, numChannels, coreSampleRate, aot)`: the closest bitrate
/// supported by SBR (0 if there is none).
pub fn sbr_encoder_limit_bit_rate(
    mut bit_rate: u32,
    num_channels: u32,
    core_sample_rate: u32,
    aot: AudioObjectType,
) -> u32 {
    let mut new_bit_rate = bit_rate;

    fdk_assert!(num_channels > 0 && num_channels <= 2);
    if aot == AOT_PS {
        if num_channels == 1 {
            let index = get_ps_tuning_table_index(bit_rate, Some(&mut new_bit_rate));
            if index == INVALID_TABLE_IDX {
                bit_rate = new_bit_rate;
            }
        } else {
            return 0;
        }
    }
    let index = get_sbr_tuning_table_index(
        bit_rate,
        num_channels,
        core_sample_rate,
        aot,
        Some(&mut new_bit_rate),
    );
    if index != INVALID_TABLE_IDX {
        new_bit_rate = bit_rate;
    }

    new_bit_rate
}

/// `sbrEncoder_IsSingleRatePossible(aot)`
pub fn sbr_encoder_is_single_rate_possible(aot: AudioObjectType) -> u32 {
    if AOT_PS == aot {
        0
    } else {
        1
    }
}

/// `sbrEncoder_Init_delay(coreFrameLength, numChannels, downSampleFactor, lowDelay, usePs,
/// is212, downsamplingMethod, hDelayParam)`: the delay balancing and the new encoder delay.
#[allow(clippy::too_many_arguments)]
fn sbr_encoder_init_delay(
    core_frame_length: i32,
    num_channels: i32,
    down_sample_factor: i32,
    low_delay: i32,
    use_ps: i32,
    is212: i32,
    downsampling_method: SbrencDsType,
    h_delay_param: &mut DelayParam,
) -> i32 {
    let mut delay_core_path; // delay in core path
    let mut delay_sbr_path; // delay difference in QMF aka SBR path
    let delay_input2_core; // delay from the input to the core
    let delay_sbr_dec; // delay of the decoder's SBR module

    let delay_core = h_delay_param.delay; // delay of the core

    // Added delay by the SBR delay initialization
    let core_path_offset; // core path
    let sbr_path_offset; // sbr path
    let mut bitstream_delay = 0; // sbr path, framewise

    let fl_core = core_frame_length; // core frame length

    let return_value = 0; // return value - 0 means: no error

    // 1) Calculate actual delay for core and SBR path
    if is212 != 0 {
        delay_core_path = delay_corepath_eldv2sbr(fl_core, down_sample_factor);
        delay_sbr_path = delay_eldv2sbr(fl_core, down_sample_factor);
        delay_sbr_dec = (fl_core / 2) * down_sample_factor;
    } else if low_delay != 0 {
        delay_core_path = delay_corepath_eldsbr(fl_core, down_sample_factor);
        delay_sbr_path = delay_eldsbr(fl_core, down_sample_factor);
        delay_sbr_dec = delay_qmf_postproc(down_sample_factor);
    } else if use_ps != 0 {
        delay_core_path = delay_corepath_ps(fl_core, down_sample_factor);
        delay_sbr_path = delay_ps(fl_core, down_sample_factor);
        delay_sbr_dec = delay_corepath_sbr(fl_core, down_sample_factor);
    } else {
        delay_core_path = delay_corepath_sbr(fl_core, down_sample_factor);
        delay_sbr_path = delay_sbr(fl_core, down_sample_factor);
        delay_sbr_dec = delay_corepath_sbr(fl_core, down_sample_factor);
    }
    delay_core_path += delay_core * down_sample_factor;
    delay_core_path +=
        if downsampling_method == SBRENC_DS_TIME { h_delay_param.ds_delay } else { 0 };

    // 2) Manage coupling of paths
    if downsampling_method == SBRENC_DS_QMF && delay_core_path > delay_sbr_path {
        // In case of QMF downsampling, both paths are coupled, i.e. the SBR path offset would
        // be added to both the SBR path and to the core path as well, thus making it
        // impossible to achieve delay balancing. To overcome that problem, a framewise delay is
        // added to the SBR path first, until the overall delay of the core path is shorter than
        // the delay of the SBR path. When this is achieved, the missing delay difference can be
        // added as downsampled offset to the core path.
        while delay_core_path > delay_sbr_path {
            // Add one frame delay to SBR path
            delay_sbr_path += fl_core * down_sample_factor;
            bitstream_delay += 1;
        }
    }

    // 3) Calculate necessary additional delay to balance the paths
    if delay_core_path > delay_sbr_path {
        // Delay QMF input
        while delay_core_path > delay_sbr_path + fl_core * down_sample_factor {
            // Do bitstream frame-wise delay balancing if there are more than SBR framelength
            // samples delay difference
            delay_sbr_path += fl_core * down_sample_factor;
            bitstream_delay += 1;
        }
        // Multiply input offset by input channels
        core_path_offset = 0;
        sbr_path_offset = (delay_core_path - delay_sbr_path) * num_channels;
    } else {
        // Delay AAC data. Multiply downsampled offset by AAC core channels. Divide by 2 because
        // of half samplerate of downsampled data.
        core_path_offset =
            ((delay_sbr_path - delay_core_path) * num_channels) >> (down_sample_factor - 1);
        sbr_path_offset = 0;
    }

    // 4) Calculate delay from input to core
    if use_ps != 0 {
        delay_input2_core = (delay_qmf_ana(down_sample_factor) + DELAY_QMF_DS + DELAY_HYB_SYN)
            + (down_sample_factor * core_path_offset)
            + 1;
    } else if downsampling_method == SBRENC_DS_TIME {
        delay_input2_core = core_path_offset + h_delay_param.ds_delay;
    } else {
        delay_input2_core = core_path_offset;
    }

    // 6) Set output parameters
    h_delay_param.delay = fix_max(delay_core_path, delay_sbr_path); // overall delay
    h_delay_param.sbr_dec_delay = delay_sbr_dec; // SBR decoder delay
    h_delay_param.delay_input2_core = delay_input2_core; // delay input - core
    h_delay_param.bitstr_delay = bitstream_delay; // bitstream delay, in frames
    h_delay_param.core_path_offset = core_path_offset; // offset added to core path
    h_delay_param.sbr_path_offset = sbr_path_offset; // offset added to SBR path

    return_value
}

/// `sbrEncoder_Init(hSbrEncoder, elInfo, noElements, inputBuffer, inputBufferBufSize,
/// coreBandwidth, inputBufferOffset, numChannels, syntaxFlags, coreSampleRate,
/// downSampleFactor, frameLength, aot, delay, transformFactor, headerPeriod,
/// statesInitFlag)`: initializes the SBR encoder (see the [module documentation](super)).
/// Returns 0 on success.
#[allow(clippy::too_many_arguments)]
pub fn sbr_encoder_init(
    h_sbr_encoder: &mut SbrEncoder,
    el_info: &mut [SbrElementInfo; 8],
    no_elements: i32,
    input_buffer: &mut [IntPcm],
    input_buffer_buf_size: u32,
    core_bandwidth: &mut i32,
    input_buffer_offset: &mut i32,
    num_channels: &mut i32,
    syntax_flags: u32,
    core_sample_rate: &mut i32,
    down_sample_factor: &mut u32,
    frame_length: &mut i32,
    aot: AudioObjectType,
    delay: &mut i32,
    transform_factor: i32,
    header_period: i32,
    states_init_flag: u32,
) -> i32 {
    let error_info; // HANDLE_ERROR_INFO errorInfo = noError
                    // C: the defaults and the tuning set every member of sbrConfig[] but
                    // init_amp_res_FF, which is only set for ELD (the port uses 0; only the state
                    // of non-ELD encoders sees the value).
    let mut sbr_config = [SbrConfiguration::default(); 8];
    let mut error: i32;
    let mut lowest_bandwidth: i32;
    // Save input parameters
    let input_sample_rate = *core_sample_rate;
    let core_frame_length = *frame_length;
    let input_band_width = *core_bandwidth;
    let input_channels = *num_channels;

    let downsampling_method;
    let mut highest_sbr_start_freq;
    let mut highest_sbr_stop_freq;
    let mut low_delay = 0;
    let mut use_ps = 0;

    let mut delay_param = DelayParam::default();

    // The `bail` label: restores the input settings.
    let bail = |core_sample_rate: &mut i32,
                frame_length: &mut i32,
                num_channels: &mut i32,
                core_bandwidth: &mut i32| {
        *core_sample_rate = input_sample_rate;
        *frame_length = core_frame_length;
        *num_channels = input_channels;
        *core_bandwidth = input_band_width;
    };

    // check whether SBR setting is available for the current encoder configuration (bitrate,
    // samplerate)
    if sbr_encoder_is_single_rate_possible(aot) == 0 {
        *down_sample_factor = 2;
    }

    if aot == AOT_PS {
        use_ps = 1;
    }
    if aot == AOT_ER_AAC_ELD {
        low_delay = 1;
    } else if aot == AOT_ER_AAC_LD {
        bail(core_sample_rate, frame_length, num_channels, core_bandwidth);
        return 1;
    }

    // Parametric Stereo
    if use_ps != 0 {
        if *num_channels == 2 && no_elements == 1 {
            // Override Element type in case of Parametric stereo
            el_info[0].el_type = ID_SCE;
            el_info[0].f_parametric_stereo = 1;
            el_info[0].n_channels_in_el = 1;
            // core encoder gets downmixed mono signal
            *num_channels = 1;
        } else {
            bail(core_sample_rate, frame_length, num_channels, core_bandwidth);
            return 1;
        }
    } // usePs

    // set the core's sample rate
    match *down_sample_factor {
        1 => {
            *core_sample_rate = input_sample_rate;
            downsampling_method = SBRENC_DS_NONE;
        }
        2 => {
            *core_sample_rate = input_sample_rate >> 1;
            downsampling_method = if use_ps != 0 { SBRENC_DS_QMF } else { SBRENC_DS_TIME };
        }
        _ => {
            *core_sample_rate = input_sample_rate >> 1;
            return 0; // return error
        }
    }

    // check whether SBR setting is available for the current encoder configuration (bitrate,
    // coreSampleRate)
    let n_el = no_elements.clamp(0, 8) as usize;

    // Check if every element config is feasible
    for info in el_info.iter().take(n_el) {
        // SBR only handles SCE and CPE's
        if info.el_type != ID_SCE && info.el_type != ID_CPE {
            continue;
        }
        // check if desired configuration is available
        if fdk_sbr_enc_is_sbr_setting_avail(
            info.bit_rate as u32,
            0,
            info.n_channels_in_el as u32,
            input_sample_rate as u32,
            *core_sample_rate as u32,
            aot,
        ) == 0
        {
            bail(core_sample_rate, frame_length, num_channels, core_bandwidth);
            return 1;
        }
    }

    h_sbr_encoder.n_channels = *num_channels;
    h_sbr_encoder.frame_size = core_frame_length.wrapping_mul(*down_sample_factor as i32);
    h_sbr_encoder.downsampling_method = downsampling_method;
    h_sbr_encoder.down_sample_factor = *down_sample_factor as i32;
    h_sbr_encoder.estimate_bitrate = 0;
    h_sbr_encoder.input_data_delay = 0;
    let is212 = if aot == AOT_ER_AAC_ELD && syntax_flags & AC_LD_MPS != 0 { 1 } else { 0 };

    // Open SBR elements
    let mut el: i32 = -1;
    highest_sbr_start_freq = 0;
    highest_sbr_stop_freq = 0;
    lowest_bandwidth = 99999;

    // Loop through each core encoder element and get a matching SBR element config
    for info in el_info.iter_mut().take(n_el) {
        // SBR only handles SCE and CPE's
        if info.el_type == ID_SCE || info.el_type == ID_CPE {
            el += 1;
        } else {
            continue;
        }
        let e = el as usize;

        // Set parametric Stereo Flag.
        info.f_parametric_stereo = if use_ps != 0 { 1 } else { 0 };

        // Init sbrConfig structure
        if fdk_sbr_enc_initialize_sbr_defaults(
            &mut sbr_config[e],
            *down_sample_factor as i32,
            core_frame_length as u32,
            is_lowdelay(aot) as i32,
        ) == 0
        {
            bail(core_sample_rate, frame_length, num_channels, core_bandwidth);
            return 1;
        }

        // Modify sbrConfig structure according to Element parameters
        if fdk_sbr_enc_adjust_sbr_settings(
            &mut sbr_config[e],
            info.bit_rate as u32,
            info.n_channels_in_el as u32,
            *core_sample_rate as u32,
            input_sample_rate as u32,
            transform_factor as u32,
            24000,
            0,
            0,             // useSpeechConfig
            0,             // lcsMode
            use_ps as u32, // bParametricStereo
            aot,
        ) == 0
        {
            bail(core_sample_rate, frame_length, num_channels, core_bandwidth);
            return 1;
        }

        // Find common frequency border for all SBR elements
        highest_sbr_start_freq = fix_max(highest_sbr_start_freq, sbr_config[e].start_freq);
        highest_sbr_stop_freq = fix_max(highest_sbr_stop_freq, sbr_config[e].stop_freq);
    } // first element loop

    // Set element count (can be less than core encoder element count)
    h_sbr_encoder.no_elements = el + 1;

    fdk_sbr_enc_reallocate(h_sbr_encoder, el_info, no_elements);

    for el in 0..h_sbr_encoder.no_elements.clamp(0, 8) as usize {
        let mut bandwidth = *core_bandwidth;

        // Use lowest common bandwidth
        sbr_config[el].start_freq = highest_sbr_start_freq;
        sbr_config[el].stop_freq = highest_sbr_stop_freq;

        // initialize SBR element, and get core bandwidth
        let SbrEncoder {
            sbr_element,
            p_sbr_channel,
            qmf_analysis,
            qmf_analysis_states,
            dynamic_ram,
            ..
        } = &mut *h_sbr_encoder;
        error = match sbr_element[el].as_deref_mut() {
            Some(h_el) => fdk_sbr_enc_env_init(
                h_el,
                p_sbr_channel,
                qmf_analysis,
                qmf_analysis_states,
                &mut sbr_config[el],
                &mut bandwidth,
                aot,
                header_period,
                states_init_flag,
                dynamic_ram,
            ),
            // Guarded: an error for an element that was not opened.
            None => 1,
        };

        if error != 0 {
            bail(core_sample_rate, frame_length, num_channels, core_bandwidth);
            return 2;
        }

        // Get lowest core encoder bandwidth to be returned later.
        lowest_bandwidth = fix_min(lowest_bandwidth, bandwidth);
    } // second element loop

    // Initialize a downsampler for each channel in each SBR element
    if h_sbr_encoder.downsampling_method == SBRENC_DS_TIME {
        for el in 0..h_sbr_encoder.no_elements.clamp(0, 8) as usize {
            let SbrEncoder { sbr_element, p_sbr_channel, .. } = &mut *h_sbr_encoder;
            let Some(h_sbr_el) = sbr_element[el].as_deref() else {
                continue;
            };

            let wc = 500; // Cutoff frequency with full bandwidth

            for ch in 0..(h_sbr_el.el_info.n_channels_in_el as usize).min(2) {
                if let Some(c) =
                    h_sbr_el.sbr_channel[ch].and_then(|i| p_sbr_channel[i].as_deref_mut())
                {
                    fdk_aac_enc_init_downsampler(
                        &mut c.down_sampler,
                        wc,
                        *down_sample_factor as i32,
                    );
                    fdk_assert!(c.down_sampler.delay <= MAX_DS_FILTER_DELAY);
                }
            }
        } // third element loop

        // lfe
        fdk_aac_enc_init_downsampler(
            &mut h_sbr_encoder.lfe_down_sampler,
            0,
            *down_sample_factor as i32,
        );
    }

    // Get delay information
    delay_param.ds_delay = h_sbr_encoder.sbr_element[0]
        .as_deref()
        .and_then(|e| e.sbr_channel[0])
        .and_then(|i| h_sbr_encoder.p_sbr_channel[i].as_deref())
        .map(|c| c.down_sampler.delay)
        .unwrap_or(0);
    delay_param.delay = *delay;

    error = sbr_encoder_init_delay(
        core_frame_length,
        *num_channels,
        *down_sample_factor as i32,
        low_delay,
        use_ps,
        is212,
        downsampling_method,
        &mut delay_param,
    );

    if error != 0 {
        bail(core_sample_rate, frame_length, num_channels, core_bandwidth);
        return 3;
    }

    h_sbr_encoder.n_bitstr_delay = delay_param.bitstr_delay;
    h_sbr_encoder.sbr_dec_delay = delay_param.sbr_dec_delay;
    h_sbr_encoder.input_data_delay = delay_param.delay_input2_core;

    // Assign core encoder Bandwidth
    *core_bandwidth = lowest_bandwidth;

    // Estimate sbr bitrate, 2.5 kBit/s per sbr channel
    h_sbr_encoder.estimate_bitrate += 2500 * *num_channels;

    // Initialize bitstream buffer for each element
    for el in 0..h_sbr_encoder.no_elements.clamp(0, 8) as usize {
        if let Some(e) = h_sbr_encoder.sbr_element[el].as_deref_mut() {
            fdk_sbr_enc_bs_buf_init(e, delay_param.bitstr_delay);
        }
    }

    // initialize parametric stereo
    if use_ps != 0 {
        let mut ps_enc_config = PsencConfig::default();
        fdk_assert!(h_sbr_encoder.no_elements == 1);
        let ps_tuning_table_idx = get_ps_tuning_table_index(el_info[0].bit_rate as u32, None);

        ps_enc_config.frame_size = core_frame_length; // sbrConfig.sbrFrameSize;
        ps_enc_config.qmf_filter_mode = 0;
        ps_enc_config.sbr_ps_delay = 0;

        // tuning parameters
        if ps_tuning_table_idx != INVALID_TABLE_IDX {
            let t = &PS_TUNING_TABLE[ps_tuning_table_idx as usize];
            ps_enc_config.n_stereo_bands = t.n_stereo_bands;
            ps_enc_config.max_envelopes = t.n_envelopes;
            ps_enc_config.iid_quant_error_threshold = t.iid_quant_error_threshold;

            // calculation is not quite linear, increased number of envelopes causes more bits;
            // assume avg. 50 bits per frame for 10 stereo bands / 1 envelope configuration
            h_sbr_encoder.estimate_bitrate += (*core_sample_rate)
                .wrapping_mul(5)
                .wrapping_mul(ps_enc_config.n_stereo_bands)
                .wrapping_mul(ps_enc_config.max_envelopes)
                / h_sbr_encoder.frame_size;
        } else {
            // error = ERROR(CDI, "Invalid ps tuning table index.")
            bail(core_sample_rate, frame_length, num_channels, core_bandwidth);
            return 1;
        }

        let (slots, bands) = h_sbr_encoder.sbr_element[0]
            .as_deref()
            .map(|e| (e.sbr_config_data.no_qmf_slots, e.sbr_config_data.no_qmf_bands))
            .unwrap_or((0, 0));
        {
            let SbrEncoder { qmf_synthesis_ps, qmf_synthesis_ps_states, .. } = &mut *h_sbr_encoder;
            qmf_init_synthesis_filter_bank(
                qmf_synthesis_ps,
                if qmf_synthesis_ps_states.is_empty() {
                    None
                } else {
                    Some(&mut qmf_synthesis_ps_states[..])
                },
                slots,
                bands >> 1,
                bands >> 1,
                bands >> 1,
                if states_init_flag != 0 { 0 } else { QMF_FLAG_KEEP_STATES },
            );
        }

        // update delay
        ps_enc_config.sbr_ps_delay = h_sbr_encoder.sbr_element[0]
            .as_deref()
            .and_then(|e| e.sbr_channel[0])
            .and_then(|i| h_sbr_encoder.p_sbr_channel[i].as_deref())
            .map(|c| fdk_sbr_enc_get_env_est_delay(&c.h_env_channel.sbr_extract_envelope))
            .unwrap_or(0);

        // C: the result goes to errorInfo, which is not returned
        error_info = ps_enc_init(
            h_sbr_encoder.h_parametric_stereo.as_deref_mut(),
            Some(&mut ps_enc_config),
            slots,
            bands,
        );
        let _ = error_info;
    }

    h_sbr_encoder.downsampled_offset = delay_param.core_path_offset;
    h_sbr_encoder.buffer_offset = delay_param.sbr_path_offset;
    *delay = delay_param.delay;

    h_sbr_encoder.downmix_size = core_frame_length * *num_channels;

    // Delay Compensation: fill bitstream delay buffer with zero input signal
    if h_sbr_encoder.n_bitstr_delay > 0 {
        error = fdk_sbr_enc_delay_compensation(h_sbr_encoder, input_buffer, input_buffer_buf_size);
        if error != 0 {
            bail(core_sample_rate, frame_length, num_channels, core_bandwidth);
            return error;
        }
    }

    // Set Output frame length
    *frame_length = core_frame_length * *down_sample_factor as i32;
    // Input buffer offset
    *input_buffer_offset = fix_max(delay_param.sbr_path_offset, delay_param.core_path_offset);

    error
}

/// `sbrEncoder_EncodeFrame(hSbrEncoder, samples, samplesBufSize, sbrDataBits, sbrData)`:
/// encodes the SBR data of one frame of all elements and downsamples the core input in
/// `samples` (the encoder's input buffer, channel stride `samplesBufSize`). Returns 0 on
/// success.
pub fn sbr_encoder_encode_frame(
    h_sbr_encoder: &mut SbrEncoder,
    samples: &mut [IntPcm],
    samples_buf_size: u32,
    sbr_data_bits: &mut [u32; 8],
    sbr_data: &mut [[u8; MAX_PAYLOAD_SIZE]; 8],
) -> i32 {
    // C: samples + hSbrEncoder->downsampledOffset / hSbrEncoder->nChannels (0 channels: see
    // div_channels())
    let off =
        div_channels(h_sbr_encoder.downsampled_offset, h_sbr_encoder.n_channels).max(0) as usize;
    for el in 0..h_sbr_encoder.no_elements.clamp(0, 8) {
        let e = el as usize;
        if h_sbr_encoder.sbr_element[e].is_some() {
            let error = fdk_sbr_enc_env_encode_frame(
                Some(h_sbr_encoder),
                el,
                samples,
                off,
                samples_buf_size,
                Some(&mut sbr_data_bits[e]),
                Some(&mut sbr_data[e]),
                0,
            );
            if error != 0 {
                return error;
            }
        }
    }

    // C passes &sbrDataBits[el] and sbrData[el] (unused by FDKsbrEnc_Downsample())
    let n_channels = h_sbr_encoder.n_channels as u32;
    let error = fdk_sbr_enc_downsample(h_sbr_encoder, samples, off, samples_buf_size, n_channels);
    if error != 0 {
        return error;
    }

    0
}

/// `a / b` for the divisions by `hSbrEncoder->nChannels` in `sbrEncoder_EncodeFrame()` and
/// `sbrEncoder_UpdateBuffers()`: x86 division by zero gives the arm64 result (0) on both
/// variants.
fn div_channels(a: i32, b: i32) -> i32 {
    if b == 0 {
        0
    } else {
        a.wrapping_div(b)
    }
}

/// `sbrEncoder_UpdateBuffers(hSbrEncoder, timeBuffer, timeBufferBufSize)`: the delay line
/// housekeeping after each frame. Returns 0.
pub fn sbr_encoder_update_buffers(
    h_sbr_encoder: &mut SbrEncoder,
    time_buffer: &mut [IntPcm],
    time_buffer_buf_size: u32,
) -> i32 {
    let buf = time_buffer_buf_size as usize;
    let nch = h_sbr_encoder.n_channels;
    // FDKmemcpy() of overlapping areas copies front to back on both targets (the source is
    // behind the destination); copy_within gives the same result.
    let copy = |tb: &mut [IntPcm], dst: usize, src: usize, n: usize| {
        if src + n <= tb.len() && dst + n <= tb.len() {
            tb.copy_within(src..src + n, dst);
        }
    };
    if h_sbr_encoder.downsampled_offset > 0 {
        let nd = div_channels(h_sbr_encoder.downmix_size, nch).max(0) as usize;

        for c in 0..nch.max(0) as usize {
            // Move delayed downsampled data
            copy(
                time_buffer,
                buf * c,
                buf * c + nd,
                div_channels(h_sbr_encoder.downsampled_offset, nch).max(0) as usize,
            );
        }
    } else {
        for c in 0..nch.max(0) as usize {
            // Move delayed input data
            copy(
                time_buffer,
                buf * c,
                buf * c + h_sbr_encoder.frame_size.max(0) as usize,
                div_channels(h_sbr_encoder.buffer_offset, nch).max(0) as usize,
            );
        }
    }
    if h_sbr_encoder.n_bitstr_delay > 0 {
        let d = (h_sbr_encoder.n_bitstr_delay as usize).min(MAX_DELAY_FRAMES);
        for el in 0..h_sbr_encoder.no_elements.clamp(0, 8) as usize {
            if let Some(e) = h_sbr_encoder.sbr_element[el].as_deref_mut() {
                e.payload_delay_line.copy_within(MAX_PAYLOAD_SIZE, 0, d * MAX_PAYLOAD_SIZE);
                e.payload_delay_line_size.copy_within(1..1 + d, 0);
            }
        }
    }
    0
}

/// `sbrEncoder_SendHeader(hSbrEncoder)`: requests an SBR header in the next frame. Returns 0, or
/// -1 without an encoder.
pub fn sbr_encoder_send_header(h_sbr_encoder: Option<&mut SbrEncoder>) -> i32 {
    let mut error = -1;
    if let Some(h) = h_sbr_encoder {
        let ps = h.no_elements == 1
            && h.sbr_element[0].as_ref().is_some_and(|e| e.el_info.f_parametric_stereo == 1);
        for el in 0..h.no_elements.clamp(0, 8) as usize {
            if let Some(e) = h.sbr_element[el].as_deref_mut() {
                if ps {
                    e.sbr_bitstream_data.count_send_header_data =
                        e.sbr_bitstream_data.nr_send_header_data - 1;
                } else {
                    e.sbr_bitstream_data.count_send_header_data = 0;
                }
            }
        }
        error = 0;
    }
    error
}

/// `sbrEncoder_ContainsHeader(hSbrEncoder)`: 1 if the current frame contains SBR headers.
pub fn sbr_encoder_contains_header(h_sbr_encoder: Option<&SbrEncoder>) -> i32 {
    let mut sbr_header = 1;
    if let Some(h) = h_sbr_encoder {
        for el in 0..h.no_elements.clamp(0, 8) as usize {
            let d = h.sbr_element[el]
                .as_ref()
                .map(|e| e.sbr_bitstream_data.header_active_delay)
                .unwrap_or(0);
            sbr_header &= if d == 1 { 1 } else { 0 };
        }
    }
    sbr_header
}

/// `sbrEncoder_GetHeaderDelay(hSbrEncoder)`: the SBR header delay in frames, -1 without an
/// encoder.
pub fn sbr_encoder_get_header_delay(h_sbr_encoder: Option<&SbrEncoder>) -> i32 {
    let mut delay = -1;

    if let Some(h) = h_sbr_encoder {
        if h.no_elements == 1
            && h.sbr_element[0].as_ref().is_some_and(|e| e.el_info.f_parametric_stereo == 1)
        {
            delay = h.n_bitstr_delay + 1;
        } else {
            delay = h.n_bitstr_delay;
        }
    }
    delay
}

/// `sbrEncoder_GetBsDelay(hSbrEncoder)`: the bitstream delay in frames, -1 without an encoder.
pub fn sbr_encoder_get_bs_delay(h_sbr_encoder: Option<&SbrEncoder>) -> i32 {
    match h_sbr_encoder {
        Some(h) => h.n_bitstr_delay,
        None => -1,
    }
}

/// `sbrEncoder_SAPPrepare(hSbrEncoder)`: forces a frame that starts at the frame border in the
/// next frame (stream access point). Returns 0, or -1 without an encoder.
pub fn sbr_encoder_sap_prepare(h_sbr_encoder: Option<&mut SbrEncoder>) -> i32 {
    let mut error = -1;
    if let Some(h) = h_sbr_encoder {
        for el in 0..h.no_elements.clamp(0, 8) as usize {
            if let Some(e) = h.sbr_element[el].as_deref_mut() {
                e.sbr_bitstream_data.right_border_fix = 1;
            }
        }
        error = 0;
    }
    error
}

/// `sbrEncoder_GetEstimateBitrate(hSbrEncoder)`: the estimated bitrate of the SBR (and PS)
/// data.
pub fn sbr_encoder_get_estimate_bitrate(h_sbr_encoder: Option<&SbrEncoder>) -> i32 {
    let mut estimate_bitrate = 0;

    if let Some(h) = h_sbr_encoder {
        estimate_bitrate += h.estimate_bitrate;
    }

    estimate_bitrate
}

/// `sbrEncoder_GetInputDataDelay(hSbrEncoder)`: the delay from the input to the core, -1
/// without an encoder.
pub fn sbr_encoder_get_input_data_delay(h_sbr_encoder: Option<&SbrEncoder>) -> i32 {
    match h_sbr_encoder {
        Some(h) => h.input_data_delay,
        None => -1,
    }
}

/// `sbrEncoder_GetSbrDecDelay(hSbrEncoder)`: the delay of the SBR decoder, -1 without an
/// encoder.
pub fn sbr_encoder_get_sbr_dec_delay(h_sbr_encoder: Option<&SbrEncoder>) -> i32 {
    match h_sbr_encoder {
        Some(h) => h.sbr_dec_delay,
        None => -1,
    }
}

/// `__DATE__` in C, which the port cannot reproduce; the value of `SUPPRESS_BUILD_DATE_INFO` is
/// used (deviation).
pub const SBRENCODER_LIB_BUILD_DATE: &str = "";
/// `__TIME__` in C, see [`SBRENCODER_LIB_BUILD_DATE`].
pub const SBRENCODER_LIB_BUILD_TIME: &str = "";
/// The title of the library information.
pub const SBRENCODER_LIB_TITLE: &str = "SBR Encoder";

/// `sbrEncoder_GetLibInfo(info)`: enters the SBR encoder library information into the next
/// free entry of `info` (an array of `FDK_MODULE_LAST` entries). Returns 0 on success, -1 for
/// `None` or without a free entry.
pub fn sbr_encoder_get_lib_info(info: Option<&mut [LibInfo]>) -> i32 {
    let Some(info) = info else {
        return -1;
    };
    // search for next free tab
    let mut i = 0usize;
    while i < FDK_MODULE_LAST as usize {
        // C reads FDK_MODULE_LAST entries; a shorter slice ends the search
        let Some(e) = info.get(i) else {
            return -1;
        };
        if e.module_id == FDK_NONE {
            break;
        }
        i += 1;
    }
    if i == FDK_MODULE_LAST as usize {
        return -1;
    }
    let info = &mut info[i];

    info.module_id = FDK_SBRENC;
    info.version = lib_version(SBRENCODER_LIB_VL0, SBRENCODER_LIB_VL1, SBRENCODER_LIB_VL2);
    lib_version_string(info);
    info.build_date = SBRENCODER_LIB_BUILD_DATE;
    info.build_time = SBRENCODER_LIB_BUILD_TIME;
    info.title = SBRENCODER_LIB_TITLE;

    // Set flags
    info.flags = CAPF_SBR_HQ | CAPF_SBR_PS_MPEG;
    // End of flags

    0
}

/// `sbrEncoder_GetHeader(sbrEncoder, hBs, element_index, fSendHeaders)` (in `bit_sbr.cpp`):
/// writes the SBR header of an element into `h_bs`; without `f_send_headers`, the element no
/// longer embeds headers into its SBR payload.
pub fn sbr_encoder_get_header(
    sbr_encoder: &mut SbrEncoder,
    h_bs: &mut FdkBitStream,
    element_index: i32,
    f_send_headers: i32,
) {
    // Guarded: where the reference has no defined result, the port writes nothing.
    let Some(e) = sbr_encoder
        .sbr_element
        .get_mut(element_index.max(0) as usize)
        .and_then(|e| e.as_deref_mut())
    else {
        return;
    };
    encode_sbr_header_data(Some(&e.sbr_header_data), h_bs);

    if f_send_headers == 0 {
        // Prevent header being embedded into the SBR payload.
        e.sbr_bitstream_data.nr_send_header_data = -1;
        e.sbr_bitstream_data.header_active = 0;
        e.sbr_bitstream_data.count_send_header_data = -1;
    }
}
