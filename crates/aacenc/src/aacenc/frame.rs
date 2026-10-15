// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/aacenc.cpp.

//! The functions of `aacenc.cpp` (see the [parent module](super)).

use super::*;
use crate::aac_enc_ram::AacEnc;
use crate::aacenc_tns::TNS_ENABLE_MASK;
use crate::adj_thr_data::{AACENC_BR_MODE_DISABLED, AACENC_BR_MODE_FULL, AACENC_BR_MODE_REDUCED};
use crate::bandwidth::fdk_aac_enc_determine_band_width;
use crate::bitenc::{fdk_aac_enc_write_bitstream, fdk_aac_enc_write_extension_data, EL_ID_BITS};
use crate::channel_map::{
    fdk_aac_enc_get_channel_mode_configuration, fdk_aac_enc_get_mono_stereo_mode,
    fdk_aac_enc_init_channel_mapping, ChannelModeConfigTab, EL_MODE_STEREO,
};
use crate::interface::is_low_delay;
use crate::psy_main::{
    fdk_aac_enc_psy_close, fdk_aac_enc_psy_init, fdk_aac_enc_psy_main_init,
    fdk_aac_enc_psy_main_pool, fdk_aac_enc_psy_new, fdk_aac_enc_psy_out_new,
};
use crate::qc_data::*;
use crate::qc_main::*;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::fixpoint_math::{f_div_norm, f_div_norm_noexp, f_mult_i};
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::scale::scale_value;
use fdk_aac_rs_tpenc::tpenc_lib::{
    transport_enc_get_frame, transport_enc_get_static_bits, transport_enc_write_access_unit_cb,
    TpEncCallbacks, TransportEnc, TRANSPORTENC_OK,
};

/// `BITRES_MIN`: default threshold for using reduced/disabled bitres mode
const BITRES_MIN: i32 = 300;
/// `BITRES_MAX_LD`
const BITRES_MAX_LD: i32 = 4000;
/// `BITRES_MIN_LD`
const BITRES_MIN_LD: i32 = 500;
/// `BITRATE_MAX_LD`: Max assumed bitrate for bitres calculation
const BITRATE_MAX_LD: i32 = 70000;
/// `BITRATE_MIN_LD`: Min assumed bitrate for bitres calculation
const BITRATE_MIN_LD: i32 = 12000;

/// `FDKaacEnc_GetChannelModeConfiguration(mode)->...`: where the reference has no defined
/// result, the port reads a zero entry (PORTING.md, "Deviations from the reference").
pub fn channel_mode_config_or_zero(mode: ChannelMode) -> ChannelModeConfigTab {
    fdk_aac_enc_get_channel_mode_configuration(mode).copied().unwrap_or(ChannelModeConfigTab {
        enc_mode: 0,
        n_channels: 0,
        n_channels_eff: 0,
        n_elements: 0,
    })
}

/// The `shift` loop of `FDKaacEnc_CalcBitsPerFrame()`/`FDKaacEnc_CalcBitrate()`: the number of
/// trailing zero bits common to `frameLength` and `samplingRate`.
///
/// The port stops at 31, so that the shift amount `shift + 1` stays below 32.
fn common_shift(frame_length: i32, sampling_rate: i32) -> i32 {
    let mut shift = 0;
    while shift < 31 {
        let mask = !((1i32 << (shift + 1)).wrapping_sub(1));
        if (frame_length & mask) == frame_length && (sampling_rate & mask) == sampling_rate {
            shift += 1;
        } else {
            break;
        }
    }
    shift
}

/// `a / b` as the arm64 build computes it (`sdiv`: `x / 0 == 0`, `INT_MIN / -1 == INT_MIN`).
/// x86 division by zero: the arm64 result on both variants.
#[inline]
pub(crate) fn sdiv(a: i32, b: i32) -> i32 {
    if b == 0 {
        0
    } else {
        a.wrapping_div(b)
    }
}

/// `FDKaacEnc_CalcBitsPerFrame(bitRate, frameLength, samplingRate)`: framesize in bits for the
/// given bit rate, frame length and sampling rate.
pub fn fdk_aac_enc_calc_bits_per_frame(
    bit_rate: i32,
    frame_length: i32,
    sampling_rate: i32,
) -> i32 {
    let shift = common_shift(frame_length, sampling_rate);
    sdiv(bit_rate.wrapping_mul(frame_length >> shift), sampling_rate >> shift)
}

/// `FDKaacEnc_CalcBitrate(bitsPerFrame, frameLength, samplingRate)`: bitrate in bits per second
/// for the given framesize, frame length and sampling rate.
pub fn fdk_aac_enc_calc_bitrate(bits_per_frame: i32, frame_length: i32, sampling_rate: i32) -> i32 {
    let shift = common_shift(frame_length, sampling_rate);
    sdiv(bits_per_frame.wrapping_mul(sampling_rate >> shift), frame_length >> shift)
}

/// `FDKaacEnc_LimitBitrate(hTpEnc, aot, coreSamplingRate, frameLength, nChannels, nChannelsEff,
/// bitRate, averageBits, pAverageBitsPerFrame, bitrateMode, nSubFrames)`: limit given bit rate
/// to a valid value.
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_limit_bitrate(
    mut h_tp_enc: Option<&mut TransportEnc>,
    aot: AudioObjectType,
    core_sampling_rate: i32,
    frame_length: i32,
    n_channels: i32,
    n_channels_eff: i32,
    mut bit_rate: i32,
    _average_bits: i32,
    mut p_average_bits_per_frame: Option<&mut i32>,
    _bitrate_mode: AacencBitrateMode,
    n_sub_frames: i32,
) -> i32 {
    let mut min_bitrate = 0;
    let mut iter = 0;
    let min_bits_per_frame = 40i32.wrapping_mul(n_channels);
    if is_low_delay(aot) != 0 {
        min_bitrate = 8000i32.wrapping_mul(n_channels_eff);
    }

    loop {
        let prev_bit_rate = bit_rate;
        let average_bits_per_frame = sdiv(
            fdk_aac_enc_calc_bits_per_frame(bit_rate, frame_length, core_sampling_rate),
            n_sub_frames,
        );

        if let Some(p) = p_average_bits_per_frame.as_deref_mut() {
            *p = average_bits_per_frame;
        }

        let transport_bits = match h_tp_enc.as_deref_mut() {
            Some(tp) => transport_enc_get_static_bits(tp, average_bits_per_frame),
            // Assume some worst case
            None => 208,
        };

        bit_rate = f_max(
            bit_rate,
            f_max(
                min_bitrate,
                fdk_aac_enc_calc_bitrate(
                    min_bits_per_frame.wrapping_add(transport_bits),
                    frame_length,
                    core_sampling_rate,
                ),
            ),
        );
        fdk_assert!(bit_rate >= 0);

        bit_rate = f_min(
            bit_rate,
            fdk_aac_enc_calc_bitrate(
                n_channels_eff.wrapping_mul(MIN_BUFSIZE_PER_EFF_CHAN),
                frame_length,
                core_sampling_rate,
            ),
        );
        fdk_assert!(bit_rate >= 0);

        // while (prevBitRate != bitRate && iter++ < 3)
        if prev_bit_rate == bit_rate {
            break;
        }
        let i = iter;
        iter += 1;
        if i >= 3 {
            break;
        }
    }

    bit_rate
}

use super::tables::CONFIG_TAB_VBR;

/// `FDKaacEnc_GetVBRBitrate(bitrateMode, channelMode)`: get VBR bitrate from vbr quality.
pub fn fdk_aac_enc_get_vbr_bitrate(
    bitrate_mode: AacencBitrateMode,
    channel_mode: ChannelMode,
) -> i32 {
    let mut mono_stereo_mode = 0; // default mono

    if fdk_aac_enc_get_mono_stereo_mode(channel_mode) == EL_MODE_STEREO {
        mono_stereo_mode = 1;
    }

    let bitrate = match bitrate_mode {
        AACENC_BR_MODE_VBR_1 | AACENC_BR_MODE_VBR_2 | AACENC_BR_MODE_VBR_3
        | AACENC_BR_MODE_VBR_4 | AACENC_BR_MODE_VBR_5 => {
            CONFIG_TAB_VBR[bitrate_mode as usize].chan_bitrate[mono_stereo_mode]
        }
        _ => 0,
    };

    // convert channel bitrate to overall bitrate
    bitrate.wrapping_mul(channel_mode_config_or_zero(channel_mode).n_channels_eff)
}

/// `FDKaacEnc_AdjustVBRBitrateMode(bitrateMode, bitrate, channelMode)`: adjust bitrate mode to
/// given bitrate parameter.
pub fn fdk_aac_enc_adjust_vbr_bitrate_mode(
    bitrate_mode: AacencBitrateMode,
    bitrate: i32,
    channel_mode: ChannelMode,
) -> AacencBitrateMode {
    let mut new_bitrate_mode = bitrate_mode;

    if bitrate != -1 {
        let mono_stereo_mode =
            if fdk_aac_enc_get_mono_stereo_mode(channel_mode) == EL_MODE_STEREO { 1 } else { 0 };
        let n_channels_eff = channel_mode_config_or_zero(channel_mode).n_channels_eff;
        new_bitrate_mode = AACENC_BR_MODE_INVALID;

        for entry in CONFIG_TAB_VBR.iter().rev() {
            let tab = entry.chan_bitrate[mono_stereo_mode].wrapping_mul(n_channels_eff);
            if bitrate >= tab {
                if tab < fdk_aac_enc_get_vbr_bitrate(bitrate_mode, channel_mode) {
                    new_bitrate_mode = entry.bitrate_mode;
                } else {
                    new_bitrate_mode = bitrate_mode;
                }
                break;
            }
        }
    }

    if aacenc_br_mode_is_vbr(new_bitrate_mode) {
        new_bitrate_mode
    } else {
        AACENC_BR_MODE_INVALID
    }
}

/// `FDKaacEnc_EncBitresToTpBitres(hAacEnc)` on its members: convert encoder bitreservoir value
/// for transport library.
fn enc_bitres_to_tp_bitres(
    bitrate_mode: AacencBitrateMode,
    bit_res_tot: i32,
    audio_mux_version: i32,
    n_channels_eff: i32,
) -> i32 {
    let mut transport_bitreservoir = match bitrate_mode {
        AACENC_BR_MODE_CBR => bit_res_tot, // encoder bitreservoir level
        AACENC_BR_MODE_VBR_1 | AACENC_BR_MODE_VBR_2 | AACENC_BR_MODE_VBR_3
        | AACENC_BR_MODE_VBR_4 | AACENC_BR_MODE_VBR_5 => i32::MAX, // signal variable bitrate
        AACENC_BR_MODE_SFR => 0, // super framing and fixed framing without bitreservoir signaling
        _ => 0,                  // invalid configuration
    };

    if audio_mux_version == 2 {
        transport_bitreservoir = MIN_BUFSIZE_PER_EFF_CHAN.wrapping_mul(n_channels_eff);
    }

    transport_bitreservoir
}

/// `FDKaacEnc_GetBitReservoirState(hAacEncoder)`: current state of the bit reservoir in bits.
pub fn fdk_aac_enc_get_bit_reservoir_state(h_aac_encoder: &AacEnc) -> i32 {
    enc_bitres_to_tp_bitres(
        h_aac_encoder.bitrate_mode,
        h_aac_encoder.qc_kernel.as_ref().map_or(0, |q| q.bit_res_tot),
        h_aac_encoder.config.audio_mux_version,
        h_aac_encoder.channel_mapping.n_channels_eff,
    )
}

/// `FDKaacEnc_AacInitDefaultConfig(config)`: gives reasonable default configuration.
pub fn fdk_aac_enc_aac_init_default_config(config: &mut AacencConfig) {
    // make the preinitialization of the structs flexible
    *config = AacencConfig::default();

    // default ancillary
    config.anc_rate = 0; // no ancillary data
    config.anc_data_bit_rate = 0; // no additional consumed bitrate

    // default configurations
    config.bit_rate = -1; // bitrate must be set
    config.average_bits = -1; // instead of bitrate/s we can configure bits/superframe
    config.bitrate_mode = AACENC_BR_MODE_CBR; // set bitrate mode to constant bitrate
    config.band_width = 0; // get bandwidth from table
    config.use_tns = TNS_ENABLE_MASK as u8; // tns enabled completly
    config.use_pns = 1; // depending on channelBitrate this might be set to 0 later
    config.use_is = 1; // Intensity Stereo Configuration
    config.use_ms = 1; // MS Stereo tool
    config.framelength = -1; // Framesize not configured
    config.syntax_flags = 0; // default syntax with no specialities
    config.ep_config = -1; // no ER syntax -> no additional error protection
    config.n_sub_frames = 1; // default, no sub frames
    config.channel_order = CH_ORDER_MPEG; // Use MPEG channel ordering.
    config.channel_mode = MODE_UNKNOWN;
    config.min_bits_per_frame = -1; // minum number of bits in each AU
    config.max_bits_per_frame = -1; // minum number of bits in each AU
    config.audio_mux_version = -1; // audio mux version not configured
    config.downscale_factor = 1; // downscale factor for ELD reduced delay mode, 1 is normal ELD
}

/// `FDKaacEnc_Open(phAacEnc, nElements, nChannels, nSubFrames)`: allocate and initialize a new
/// encoder instance. The allocations do not fail in the port (Rust aborts instead).
pub fn fdk_aac_enc_open(
    ph_aac_enc: &mut Option<Box<AacEnc>>,
    n_elements: i32,
    n_channels: i32,
    n_sub_frames: i32,
) -> AacEncoderError {
    // allocate encoder structure
    let mut h_aac_enc = Box::<AacEnc>::default();
    let mut error_status;

    'bail: {
        // allocate the Psy aud Psy Out structure
        error_status = fdk_aac_enc_psy_new(&mut h_aac_enc.psy_kernel, n_elements, n_channels);
        if error_status != AAC_ENC_OK {
            break 'bail;
        }

        error_status =
            fdk_aac_enc_psy_out_new(&mut h_aac_enc.psy_out, n_elements, n_channels, n_sub_frames);
        if error_status != AAC_ENC_OK {
            break 'bail;
        }

        // allocate the Q&C Out structure
        error_status =
            fdk_aac_enc_qc_out_new(&mut h_aac_enc.qc_out, n_elements, n_channels, n_sub_frames);
        if error_status != AAC_ENC_OK {
            break 'bail;
        }

        // allocate the Q&C kernel
        error_status = fdk_aac_enc_qc_new(&mut h_aac_enc.qc_kernel, n_elements);
        if error_status != AAC_ENC_OK {
            break 'bail;
        }

        h_aac_enc.max_channels = n_channels;
        h_aac_enc.max_elements = n_elements;
        h_aac_enc.max_frames = n_sub_frames;
    }

    *ph_aac_enc = Some(h_aac_enc);
    error_status
}

/// `FDKaacEnc_Initialize(hAacEnc, config, hTpEnc, initFlags)`: initializes the encoder core
/// with the (pre-initialized) configuration `config`, which it updates (see the
/// [module documentation](super)).
pub fn fdk_aac_enc_initialize(
    h_aac_enc: &mut AacEnc,
    config: Option<&mut AacencConfig>,
    h_tp_enc: &mut TransportEnc,
    init_flags: u32,
) -> AacEncoderError {
    let Some(config) = config else {
        return AAC_ENC_INVALID_HANDLE;
    };
    let mut bound = false;
    fdk_aac_enc_initialize_bound(h_aac_enc, config, h_tp_enc, init_flags, &mut bound)
}

/// [`fdk_aac_enc_initialize`], which also tells whether C bound `hAacEnc->config` to `config`
/// (`*bound` is set to true if it did; C leaves `hAacEnc->config` unchanged otherwise).
pub fn fdk_aac_enc_initialize_bound(
    h_aac_enc: &mut AacEnc,
    config: &mut AacencConfig,
    h_tp_enc: &mut TransportEnc,
    init_flags: u32,
    bound: &mut bool,
) -> AacEncoderError {
    let mut b = false;
    let r = initialize(h_aac_enc, config, h_tp_enc, init_flags, &mut b);
    if b {
        // hAacEnc->config = config, with the writes through the pointer after the binding
        h_aac_enc.config = *config;
        *bound = true;
    }
    r
}

fn initialize(
    h_aac_enc: &mut AacEnc,
    config: &mut AacencConfig,
    h_tp_enc: &mut TransportEnc,
    init_flags: u32,
    bound: &mut bool,
) -> AacEncoderError {
    let mut error_status: AacEncoderError;
    let mut average_bits_per_frame: i32 = 0;
    let prev_channel_mode = h_aac_enc.encoder_mode;
    let mut qc_init = QcInit::default();

    // ******************* sanity checks *******************

    // check config structure
    if config.n_channels < 1 || config.n_channels > 8 {
        return AAC_ENC_UNSUPPORTED_CHANNELCONFIG;
    }

    // check sample rate
    match config.sample_rate {
        8000 | 11025 | 12000 | 16000 | 22050 | 24000 | 32000 | 44100 | 48000 | 64000 | 88200
        | 96000 => {}
        _ => return AAC_ENC_UNSUPPORTED_SAMPLINGRATE,
    }

    // bitrate has to be set
    if config.bit_rate == -1 {
        return AAC_ENC_UNSUPPORTED_BITRATE;
    }

    // check bit rate
    if fdk_aac_enc_limit_bitrate(
        Some(&mut *h_tp_enc),
        config.audio_object_type,
        config.sample_rate,
        config.framelength,
        config.n_channels,
        channel_mode_config_or_zero(config.channel_mode).n_channels_eff,
        config.bit_rate,
        config.average_bits,
        Some(&mut average_bits_per_frame),
        config.bitrate_mode,
        config.n_sub_frames,
    ) != config.bit_rate
        && !aacenc_br_mode_is_vbr(config.bitrate_mode)
    {
        return AAC_ENC_UNSUPPORTED_BITRATE;
    }

    if config.syntax_flags & AC_ER_VCB11 != 0 {
        return AAC_ENC_UNSUPPORTED_ER_FORMAT;
    }
    if config.syntax_flags & AC_ER_HCR != 0 {
        return AAC_ENC_UNSUPPORTED_ER_FORMAT;
    }

    // check frame length
    match config.framelength {
        1024 => {
            if is_low_delay(config.audio_object_type) != 0 {
                return AAC_ENC_INVALID_FRAME_LENGTH;
            }
        }
        128 | 256 | 512 | 120 | 240 | 480 => {
            if is_low_delay(config.audio_object_type) == 0 {
                return AAC_ENC_INVALID_FRAME_LENGTH;
            }
        }
        _ => return AAC_ENC_INVALID_FRAME_LENGTH,
    }

    if config.anc_rate != 0 {
        error_status = fdk_aac_enc_init_check_ancillary(
            config.bit_rate,
            config.framelength,
            config.anc_rate,
            &mut h_aac_enc.ancillary_bits_per_frame,
            config.sample_rate,
        );
        if error_status != AAC_ENC_OK {
            return error_status;
        }

        // update estimated consumed bitrate
        config.anc_data_bit_rate = config.anc_data_bit_rate.wrapping_add(fdk_aac_enc_calc_bitrate(
            h_aac_enc.ancillary_bits_per_frame,
            config.framelength,
            config.sample_rate,
        ));
    }

    // maximal allowed DSE bytes in frame
    config.max_anc_bytes_per_au = f_min(
        256,
        f_max(
            0,
            fdk_aac_enc_calc_bits_per_frame(
                config.bit_rate.wrapping_sub(config.n_channels.wrapping_mul(8000)),
                config.framelength,
                config.sample_rate,
            ) >> 3,
        ),
    ) as u32;

    // bind config to hAacEnc->config
    *bound = true;

    // set hAacEnc->bitrateMode
    h_aac_enc.bitrate_mode = config.bitrate_mode;

    h_aac_enc.encoder_mode = config.channel_mode;

    error_status = fdk_aac_enc_init_channel_mapping(
        h_aac_enc.encoder_mode,
        config.channel_order,
        &mut h_aac_enc.channel_mapping,
    );
    if error_status != AAC_ENC_OK {
        return error_status;
    }

    let cm = h_aac_enc.channel_mapping;

    let mut band_width = config.band_width;
    error_status = fdk_aac_enc_determine_band_width(
        config.band_width,
        config.bit_rate.wrapping_sub(config.anc_data_bit_rate),
        h_aac_enc.bitrate_mode,
        config.sample_rate,
        config.framelength,
        &cm,
        h_aac_enc.encoder_mode,
        &mut band_width,
    );
    config.band_width = band_width;
    if error_status != AAC_ENC_OK {
        return error_status;
    }

    h_aac_enc.bandwidth90d_b = config.band_width;

    let tns_mask = if config.use_tns != 0 { TNS_ENABLE_MASK } else { 0x0 };
    let psy_bitrate = config.bit_rate.wrapping_sub(config.anc_data_bit_rate);

    let (Some(psy_kernel), Some(qc_kernel)) =
        (h_aac_enc.psy_kernel.as_deref_mut(), h_aac_enc.qc_kernel.as_deref_mut())
    else {
        // Guarded (the port's allocations do not fail).
        return AAC_ENC_INVALID_HANDLE;
    };

    if (h_aac_enc.encoder_mode != prev_channel_mode) || (init_flags != 0) {
        // Reinitialize psych states in case of channel configuration change ore if full reset
        // requested.
        error_status = fdk_aac_enc_psy_init(
            psy_kernel,
            &mut h_aac_enc.psy_out,
            h_aac_enc.max_frames,
            h_aac_enc.max_channels,
            config.audio_object_type,
            &cm,
        );
        if error_status != AAC_ENC_OK {
            return error_status;
        }
    }

    error_status = fdk_aac_enc_psy_main_init(
        psy_kernel,
        config.audio_object_type,
        &cm,
        config.sample_rate,
        config.framelength,
        psy_bitrate,
        tns_mask,
        h_aac_enc.bandwidth90d_b,
        config.use_pns as i32,
        config.use_is as i32,
        config.use_ms as i32,
        config.syntax_flags,
        init_flags,
    );
    if error_status != AAC_ENC_OK {
        return error_status;
    }

    error_status = fdk_aac_enc_qc_out_init(&mut h_aac_enc.qc_out, h_aac_enc.max_frames, &cm);
    if error_status != AAC_ENC_OK {
        return error_status;
    }

    qc_init.channel_mapping = cm;
    qc_init.sce_cpe = 0;

    let avg_ceil = average_bits_per_frame.wrapping_add(7) & !7;
    let max_buf = MIN_BUFSIZE_PER_EFF_CHAN.wrapping_mul(cm.n_channels_eff);
    if aacenc_br_mode_is_vbr(config.bitrate_mode) {
        qc_init.average_bits = avg_ceil;
        qc_init.bit_res = max_buf;
        qc_init.max_bits = max_buf;
        qc_init.max_bits = if config.max_bits_per_frame != -1 {
            fix_min(qc_init.max_bits, config.max_bits_per_frame)
        } else {
            qc_init.max_bits
        };
        qc_init.max_bits = fix_max(qc_init.max_bits, avg_ceil);
        qc_init.min_bits =
            if config.min_bits_per_frame != -1 { config.min_bits_per_frame } else { 0 };
        qc_init.min_bits = fix_min(qc_init.min_bits, average_bits_per_frame & !7);
    } else {
        let mut bitreservoir: i32 = -1; // default bitreservoir size
        if is_low_delay(config.audio_object_type) != 0 {
            let mut br_per_channel = sdiv(config.bit_rate, config.n_channels);
            br_per_channel = f_min(BITRATE_MAX_LD, f_max(BITRATE_MIN_LD, br_per_channel));

            // bitreservoir =
            // (maxBitRes-minBitRes)/(maxBitRate-minBitrate)*(bitRate-minBitrate)+minBitRes;
            // calc slope for interpolation
            let slope =
                f_div_norm_noexp(br_per_channel - BITRATE_MIN_LD, BITRATE_MAX_LD - BITRATE_MIN_LD);
            // interpolate
            bitreservoir = f_mult_i(slope, BITRES_MAX_LD - BITRES_MIN_LD) + BITRES_MIN_LD;
            bitreservoir &= !7; // align to bytes
        }

        qc_init.average_bits = avg_ceil;
        let max_bitres = max_buf.wrapping_sub(qc_init.average_bits);
        qc_init.bit_res =
            if bitreservoir != -1 { f_min(bitreservoir, max_bitres) } else { max_bitres };

        qc_init.max_bits = fix_min(max_buf, avg_ceil.wrapping_add(qc_init.bit_res));
        qc_init.max_bits = if config.max_bits_per_frame != -1 {
            fix_min(qc_init.max_bits, config.max_bits_per_frame)
        } else {
            qc_init.max_bits
        };
        qc_init.max_bits = fix_min(
            max_buf,
            fix_max(qc_init.max_bits, average_bits_per_frame.wrapping_add(7 + 8) & !7),
        );

        let static_bits =
            transport_enc_get_static_bits(h_tp_enc, avg_ceil.wrapping_add(qc_init.bit_res));
        qc_init.min_bits = fix_max(
            0,
            (average_bits_per_frame.wrapping_sub(1) & !7)
                .wrapping_sub(qc_init.bit_res)
                .wrapping_sub(static_bits),
        );
        qc_init.min_bits = if config.min_bits_per_frame != -1 {
            fix_max(qc_init.min_bits, config.min_bits_per_frame)
        } else {
            qc_init.min_bits
        };
        let static_bits = transport_enc_get_static_bits(h_tp_enc, qc_init.max_bits);
        qc_init.min_bits =
            fix_min(qc_init.min_bits, average_bits_per_frame.wrapping_sub(static_bits) & !7);
    }

    qc_init.sample_rate = config.sample_rate;
    qc_init.is_low_delay = is_low_delay(config.audio_object_type);
    qc_init.n_sub_frames = config.n_sub_frames;
    qc_init.padding.padding_rest = config.sample_rate;

    if qc_init.max_bits.wrapping_sub(qc_init.average_bits)
        >= (if qc_init.is_low_delay != 0 { BITRES_MIN_LD } else { BITRES_MIN })
            .wrapping_mul(config.n_channels)
    {
        qc_init.bit_res_mode = AACENC_BR_MODE_FULL; // full bitreservoir
    } else if qc_init.max_bits > qc_init.average_bits {
        qc_init.bit_res_mode = AACENC_BR_MODE_REDUCED; // reduced bitreservoir
    } else {
        qc_init.bit_res_mode = AACENC_BR_MODE_DISABLED; // disabled bitreservoir
    }

    // Configure bitrate distribution strategy.
    qc_init.bit_distribution_mode = match config.channel_mode {
        MODE_1_2
        | MODE_1_2_1
        | MODE_1_2_2
        | MODE_1_2_2_1
        | MODE_6_1
        | MODE_1_2_2_2_1
        | MODE_7_1_BACK
        | MODE_7_1_TOP_FRONT
        | MODE_7_1_REAR_SURROUND
        | MODE_7_1_FRONT_CENTER => {
            0 // over all elements bitrate estimation
        }
        _ => 1, // MODE_1, MODE_2, all non mpeg defined channel modes: element-wise
    };

    // Calc meanPe: qcInit.meanPe = 10.0f * FRAME_LEN_LONG *
    // hAacEnc->bandwidth90dB/(config->sampleRate/2.0f);
    let mut qbw: i32 = 0;
    let bw_ratio = f_div_norm(
        10i32.wrapping_mul(config.framelength).wrapping_mul(h_aac_enc.bandwidth90d_b),
        config.sample_rate,
        &mut qbw,
    );
    qc_init.mean_pe = f_max(scale_value(bw_ratio, qbw + 1 - (DFRACT_BITS - 1)), 1);

    // Calc maxBitFac, scale it to 24 bit accuracy
    let mut mbfac_e: i32 = 0;
    let mbfac = f_div_norm(
        qc_init.max_bits,
        sdiv(qc_init.average_bits, qc_init.n_sub_frames),
        &mut mbfac_e,
    );
    qc_init.max_bit_fac = scale_value(mbfac, -(DFRACT_BITS - 1 - 24 - mbfac_e));

    qc_init.bitrate_mode = match config.bitrate_mode {
        AACENC_BR_MODE_CBR => QCDATA_BR_MODE_CBR,
        AACENC_BR_MODE_VBR_1 => QCDATA_BR_MODE_VBR_1,
        AACENC_BR_MODE_VBR_2 => QCDATA_BR_MODE_VBR_2,
        AACENC_BR_MODE_VBR_3 => QCDATA_BR_MODE_VBR_3,
        AACENC_BR_MODE_VBR_4 => QCDATA_BR_MODE_VBR_4,
        AACENC_BR_MODE_VBR_5 => QCDATA_BR_MODE_VBR_5,
        AACENC_BR_MODE_SFR => QCDATA_BR_MODE_SFR,
        AACENC_BR_MODE_FF => QCDATA_BR_MODE_FF,
        _ => return AAC_ENC_UNSUPPORTED_BITRATE_MODE,
    };

    qc_init.inv_quant = if config.use_requant != 0 { 2 } else { 0 };

    // maxIterations should be set to the maximum number of requantization iterations that are
    // allowed before the crash recovery functionality is activated.
    qc_init.max_iterations = if is_low_delay(config.audio_object_type) != 0 { 2 } else { 5 };

    qc_init.bitrate = config.bit_rate.wrapping_sub(config.anc_data_bit_rate);

    qc_init.static_bits =
        transport_enc_get_static_bits(h_tp_enc, sdiv(qc_init.average_bits, qc_init.n_sub_frames));

    error_status = fdk_aac_enc_qc_init(qc_kernel, &qc_init, init_flags);
    if error_status != AAC_ENC_OK {
        return error_status;
    }

    // Map virtual aot's to intern aot used in bitstream writer.
    h_aac_enc.aot = match config.audio_object_type {
        AOT_MP2_AAC_LC => AOT_AAC_LC,
        AOT_MP2_SBR => AOT_SBR,
        a => a,
    };

    // common things

    AAC_ENC_OK
}

/// The psychoacoustics of an element in [`fdk_aac_enc_encode_frame_pool`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ElState {
    /// Not started: the sequential code runs.
    Fresh,
    /// `psy_main_start()` ran; `psy_main_finish()` and `FDKaacEnc_QCMainPrepare()` run in the
    /// sequential loop.
    Started(crate::psy_main::PsyFrame),
    /// `FDKaacEnc_psyMain()` and `FDKaacEnc_QCMainPrepare()` ran.
    Done,
    /// The element fails with this error (the sequential loop returns it there).
    Failed(AacEncoderError),
}

/// The first static, PSY_OUT_CHANNEL and QC_OUT_CHANNEL of an element and its channel count.
type ElMap = (usize, usize, usize, usize);

/// The masks of the lines `from..to` (at most 1024) in the 16 words of a `QcOut::mdct_undefined`
/// entry.
fn undefined_lines_masks(from: usize, to: usize) -> impl Iterator<Item = (usize, u64)> {
    let to = to.min(1024);
    (from >> 6..to.div_ceil(64)).filter_map(move |w| {
        let lo = from.max(w * 64) - w * 64;
        let hi = to.min(w * 64 + 64) - w * 64;
        (lo < hi).then(|| (w, (u64::MAX >> (64 - (hi - lo))) << lo))
    })
}

/// Whether a line in `from..to` of a `QcOut::mdct_undefined` entry is set.
fn undefined_lines_any(b: &[u64; 16], from: usize, to: usize) -> bool {
    undefined_lines_masks(from, to).any(|(w, m)| b[w] & m != 0)
}

/// Sets or clears the lines `from..to` of a `QcOut::mdct_undefined` entry.
fn undefined_lines_set(b: &mut [u64; 16], from: usize, to: usize, v: bool) {
    for (w, m) in undefined_lines_masks(from, to) {
        if v {
            b[w] |= m;
        } else {
            b[w] &= !m;
        }
    }
}

/// `sfbActive` of a channel (`sfbActiveLFE` for a static with `isLFE`).
fn sfb_active_of(
    conf: &crate::psy_configuration::PsyConfiguration,
    st: &crate::psy_data::PsyStatic,
) -> usize {
    let a = if st.is_lfe != 0 { conf.sfb_active_lfe } else { conf.sfb_active };
    a.clamp(0, crate::psy_const::MAX_SFB as i32) as usize
}

/// Before `FDKaacEnc_psyMain()` of an element with statics `st` and QC_OUT_CHANNELs from `q0`:
/// updates the counters of calls without a defined reference result
/// (`psy_main_undefined_spectrum_reads`, `psy_main_lfe_flag_pairs`).
fn undefined_spectrum_check(
    psy_kernel: &crate::psy_main::PsyInternal,
    qc_out: &QcOut,
    st: &[usize],
    q0: usize,
) {
    let conf = &psy_kernel.psy_conf;
    let statics = &psy_kernel.p_static_channels;
    let nts = conf[0].granule_length.clamp(0, 1024) as usize;
    let active = st.iter().map(|&s| sfb_active_of(&conf[0], &statics[s])).max().unwrap_or(0);
    let top = conf[0].sfb_offset[active].clamp(0, 1024) as usize;
    if top > nts
        && (0..st.len()).any(|ch| {
            qc_out.mdct_undefined.get(q0 + ch).is_some_and(|b| undefined_lines_any(b, nts, top))
        })
    {
        crate::psy_main::add_psy_main_undefined_spectrum_reads(1);
    }
    // (the other case is counted in fdk_aac_enc_psy_main)
    if st.len() == 2
        && statics[st[0]].is_lfe == 0
        && statics[st[1]].is_lfe != 0
        && conf[0].pns_conf.use_pns != 0
    {
        crate::psy_main::add_psy_main_lfe_flag_pairs(1);
    }
}

/// After `FDKaacEnc_psyMain()` of an element: records the lines of the spectra that the
/// reference does not define (`QcOut::mdct_undefined`; the port holds 0 there) and updates
/// `psy_main_lfe_flag_pairs`.
fn undefined_spectrum_update(
    psy_kernel: &crate::psy_main::PsyInternal,
    qc_out: &mut QcOut,
    st: &[usize],
    q0: usize,
) {
    let conf = &psy_kernel.psy_conf;
    let statics = &psy_kernel.p_static_channels;
    let n_qc = qc_out.p_qc_out_channels.len();
    if qc_out.mdct_undefined.len() < n_qc {
        qc_out.mdct_undefined.resize(n_qc, [0; 16]);
    }
    let nts = conf[0].granule_length.clamp(0, 1024) as usize;
    let mut short = false;
    for (ch, &s) in st.iter().enumerate() {
        let Some(b) = qc_out.mdct_undefined.get_mut(q0 + ch) else { continue };
        undefined_lines_set(b, 0, nts, false);
        let bsc = &statics[s].block_switching_control;
        if bsc.last_window_sequence != crate::psy_const::SHORT_WINDOW {
            continue;
        }
        short = true;
        // FDKaacEnc_groupShortData(): tmpSpectrum[i] for the bands below sfbActive of every
        // group, the rest of psyConf[1].granuleLength lines is the scratch buffer
        let c1 = &conf[1];
        let gl = c1.granule_length.clamp(0, 1024) as usize;
        let so = |k: usize| c1.sfb_offset[k.min(crate::psy_const::MAX_SFB)].max(0) as usize;
        let active = sfb_active_of(c1, &statics[s]);
        let cnt = (c1.sfb_cnt.max(0) as usize).min(crate::psy_const::MAX_SFB);
        undefined_lines_set(b, 0, gl, true);
        let mut i = 0usize;
        for grp in 0..(bsc.no_of_groups.clamp(0, 8) as usize) {
            let len = bsc.group_len[grp].max(0) as usize;
            let written = len * so(active);
            undefined_lines_set(b, i, i + written, false);
            i += written + len * so(cnt).saturating_sub(so(active));
        }
    }
    if short && st.len() == 2 && statics[st[0]].is_lfe != statics[st[1]].is_lfe {
        crate::psy_main::add_psy_main_lfe_flag_pairs(1);
    }
}

/// Splits `s` into the disjoint ranges `ranges[i] = (start, len)`, returned in the order of
/// `ranges`; `None` if two ranges overlap or a range is out of bounds.
fn split_ranges<'a, T>(s: &'a mut [T], ranges: &[(usize, usize)]) -> Option<Vec<&'a mut [T]>> {
    let mut order: Vec<usize> = (0..ranges.len()).collect();
    order.sort_by_key(|&i| ranges[i].0);
    let mut out: Vec<Option<&'a mut [T]>> = (0..ranges.len()).map(|_| None).collect();
    let mut rest = s;
    let mut offset = 0;
    for i in order {
        let (start, len) = ranges[i];
        let skip = start.checked_sub(offset)?;
        if skip.checked_add(len)? > rest.len() {
            return None;
        }
        let (_, tail) = core::mem::take(&mut rest).split_at_mut(skip);
        let (part, tail) = tail.split_at_mut(len);
        out[i] = Some(part);
        rest = tail;
        offset = start + len;
    }
    out.into_iter().collect()
}

/// The psychoacoustics of the elements in parallel (opt-in multi-threading, configurations with
/// more than one audio element): `FDKaacEnc_psyMain()` and `FDKaacEnc_QCMainPrepare()` of every
/// element as one task, with its own `PSY_DYNAMIC`, and the results (`state`) consumed by the
/// sequential element loop of [`fdk_aac_enc_encode_frame_pool`].
///
/// Why the result is the sequential one:
///
/// - The two functions of element `el` read and write the element's statics, `PSY_OUT_ELEMENT`,
///   `PSY_OUT_CHANNEL`s, `QC_OUT_ELEMENT`, `QC_OUT_CHANNEL`s and `ATS_ELEMENT`, which belong to no
///   other element (checked: the ranges are disjoint), read the configuration and the input, and
///   use `PSY_DYNAMIC` as scratch. The sequential loop does nothing else between the elements
///   that they read (it sums the static and extension bits of the elements, which it still does
///   in element order).
/// - `PSY_DYNAMIC`: the encoder outputs do not depend on its contents before the call (see
///   [`crate::psy_main`]); with the feature `verify-poison` every task's `PSY_DYNAMIC` is filled
///   with pseudo-random values, and all encoder outputs of the corpus stay identical
///   (docs/optimization.md, "Encoder multi-threading").
/// - Errors: the sequential loop stops at the first failing element. The checks that can fail
///   (the element mapping, the block switching of `psy_main_start()`) run here in element order
///   first and stop at the first failure, which `state` records for the sequential loop. An
///   element whose transform could fail (`psy_main_finish_cannot_fail()`) is left to the
///   sequential loop (state `Started`), and so are all elements after it. The remaining code
///   cannot fail: `FDKaacEnc_QCMainPrepare()` only fails for an element without a bitstream
///   element list (checked here) or a broken list (the lists are constant).
/// - Mappings the parallel code cannot represent (statics of a pair that are not consecutive,
///   shared or out-of-range channels) leave the whole frame to the sequential code.
#[allow(clippy::too_many_arguments)]
fn psy_elements_parallel(
    state: &mut [ElState; 8],
    par: &mut crate::par::EncPar<'_>,
    psy_kernel: &mut crate::psy_main::PsyInternal,
    psy_out: &mut crate::interface::PsyOut,
    qc_out: &mut QcOut,
    adj_thr_state_elem: &mut [crate::adj_thr_data::AtsElement],
    cm: &ChannelMapping,
    input_buffer: &[IntPcm],
    input_buffer_buf_size: u32,
    aot: AudioObjectType,
    syntax_flags: u32,
    ep_config: i8,
) {
    let n_el = cm.n_elements.clamp(0, 8) as usize;
    // (statics, psy channels, qc channels, channels) of every audio element
    let mut map: [Option<ElMap>; 8] = [None; 8];
    let mut n_audio = 0;
    for el in 0..n_el {
        let el_info = cm.el_info[el];
        if !((el_info.el_type == ID_SCE)
            || (el_info.el_type == ID_CPE)
            || (el_info.el_type == ID_LFE))
        {
            continue;
        }
        let n = el_info.n_channels_in_el.clamp(0, 2) as usize;
        let (Some(st), Some(p0)) = (
            crate::psy_main::psy_element_statics(
                &psy_kernel.psy_element[el].psy_static,
                el_info.n_channels_in_el,
                psy_kernel.p_static_channels.len(),
            ),
            crate::psy_main::psy_element_channels(
                &psy_out.psy_out_element[el].psy_out_channel,
                el_info.n_channels_in_el,
                psy_out.p_psy_out_channels.len(),
            ),
        ) else {
            return;
        };
        let q0 = qc_out.qc_element[el].qc_out_channel[0];
        let nch = if (el_info.el_type == ID_SCE) || (el_info.el_type == ID_LFE) { 1 } else { 2 };
        if n == 0
            || (n == 2 && st[1] != st[0] + 1)
            || q0.checked_add(n).is_none_or(|e| e > qc_out.p_qc_out_channels.len())
            || el >= adj_thr_state_elem.len()
            || fdk_aac_rs_fdk::tools_rom::get_bitstream_element_list(aot, ep_config, nch, 0, 0)
                .is_none()
        {
            return;
        }
        map[el] = Some((st[0], p0, q0, n));
        n_audio += 1;
    }
    if n_audio < 2 {
        return;
    }
    let els: Vec<usize> = (0..n_el).filter(|&el| map[el].is_some()).collect();
    let range = |f: fn(&ElMap) -> (usize, usize)| -> Vec<(usize, usize)> {
        els.iter().map(|&el| map[el].as_ref().map(f).unwrap_or((0, 0))).collect()
    };
    let one = els.iter().map(|&el| (el, 1)).collect::<Vec<_>>();
    let crate::psy_main::PsyInternal { psy_conf, p_static_channels, .. } = psy_kernel;
    let (Some(statics), Some(psy_chs), Some(qc_chs), Some(psy_els), Some(qc_els), Some(ats)) = (
        split_ranges(p_static_channels, &range(|m| (m.0, m.3))),
        split_ranges(&mut psy_out.p_psy_out_channels, &range(|m| (m.1, m.3))),
        split_ranges(&mut qc_out.p_qc_out_channels, &range(|m| (m.2, m.3))),
        split_ranges(&mut psy_out.psy_out_element, &one),
        split_ranges(&mut qc_out.qc_element, &one),
        split_ranges(adj_thr_state_elem, &one),
    ) else {
        return;
    };

    struct Task<'a> {
        el: usize,
        frame: crate::psy_main::PsyFrame,
        statics: &'a mut [crate::psy_data::PsyStatic],
        psy_el: &'a mut crate::interface::PsyOutElement,
        psy_chs: &'a mut [crate::interface::PsyOutChannel],
        /// `QC_OUT_ELEMENT::peData` and `staticBitsUsed`, the fields `FDKaacEnc_QCMainPrepare()`
        /// reads and writes (the element itself is not `Send`: its extension payload pointers)
        pe_data: &'a mut crate::line_pe::PeData,
        static_bits_used: &'a mut i32,
        qc_chs: &'a mut [QcOutChannel],
        ats: &'a mut crate::adj_thr_data::AtsElement,
        dynamic: &'a mut crate::psy_main::PsyDynamic,
        error: AacEncoderError,
    }

    // psy_main_start() in element order, up to the first element that fails or whose
    // psy_main_finish() could fail
    if par.psy_dynamic.len() < els.len() {
        par.psy_dynamic.resize_with(els.len(), Default::default);
    }
    let mut tasks: Vec<Task<'_>> = Vec::with_capacity(els.len());
    let parts = statics
        .into_iter()
        .zip(psy_chs)
        .zip(qc_chs)
        .zip(psy_els.into_iter().zip(qc_els).zip(ats))
        .zip(par.psy_dynamic.iter_mut());
    for (k, ((((statics, psy_chs), qc_chs), ((psy_el, qc_el), ats)), dynamic)) in parts.enumerate()
    {
        let el = els[k];
        let el_info = cm.el_info[el];
        let frame = match crate::psy_main::psy_main_start(
            el_info.n_channels_in_el,
            statics,
            psy_conf,
            input_buffer,
            input_buffer_buf_size,
            &el_info.channel_index,
        ) {
            Ok(frame) => frame,
            Err(error) => {
                state[el] = ElState::Failed(error);
                break;
            }
        };
        if !crate::psy_main::psy_main_finish_cannot_fail(&frame, statics, psy_conf) {
            state[el] = ElState::Started(frame);
            break;
        }
        tasks.push(Task {
            el,
            frame,
            statics,
            psy_el: &mut psy_el[0],
            psy_chs,
            pe_data: &mut qc_el[0].pe_data,
            static_bits_used: &mut qc_el[0].static_bits_used,
            qc_chs,
            ats: &mut ats[0],
            dynamic,
            error: AAC_ENC_OK,
        });
    }

    let psy_conf: &[crate::psy_configuration::PsyConfiguration; 2] = psy_conf;
    crate::par::for_each_mut(Some(par.pool), &mut tasks, |_, t| {
        let el_info = cm.el_info[t.el];
        #[cfg(feature = "verify-poison")]
        crate::psy_main::poison_psy_dynamic(
            t.dynamic,
            t.el as u32 ^ t.qc_chs[0].global_gain as u32,
        );
        t.error = crate::psy_main::psy_main_finish(
            &t.frame,
            el_info.n_channels_in_el,
            t.statics,
            t.dynamic,
            psy_conf,
            t.psy_el,
            t.psy_chs,
            t.qc_chs,
            input_buffer,
            input_buffer_buf_size,
            &el_info.channel_index,
            None,
        );
        if t.error == AAC_ENC_OK {
            // FormFactor, Pe and staticBitDemand calculation (on an element with the two
            // fields it uses)
            let mut qc_el = QcOutElement {
                pe_data: *t.pe_data,
                static_bits_used: *t.static_bits_used,
                ..Default::default()
            };
            t.error = fdk_aac_enc_qc_main_prepare(
                &el_info,
                t.ats,
                t.psy_el,
                t.psy_chs,
                &mut qc_el,
                t.qc_chs,
                aot,
                syntax_flags,
                ep_config,
            );
            *t.pe_data = qc_el.pe_data;
            *t.static_bits_used = qc_el.static_bits_used;
        }
    });
    for t in &tasks {
        state[t.el] = if t.error == AAC_ENC_OK { ElState::Done } else { ElState::Failed(t.error) };
    }
}

/// `FDKaacEnc_EncodeFrame(hAacEnc, hTpEnc, inputBuffer, inputBufferBufSize, nOutBytes,
/// extPayload)`: encodes one frame. `input_buffer` is the whole input buffer of the encoder
/// (`inputBufferBufSize` samples per channel); `cb` are the callbacks of the transport encoder
/// (registered with the handle of `aacenc_lib.cpp` in C).
///
/// # Safety
/// The extension payloads (`ext_payload[].p_data`) must be valid for reads of their payload
/// bytes (see [`fdk_aac_enc_write_extension_data`]).
pub unsafe fn fdk_aac_enc_encode_frame(
    h_aac_enc: &mut AacEnc,
    h_tp_enc: &mut TransportEnc,
    input_buffer: &[IntPcm],
    input_buffer_buf_size: u32,
    n_out_bytes: &mut [i32],
    ext_payload: &mut [AacencExtPayload; MAX_TOTAL_EXT_PAYLOADS],
    cb: &mut dyn TpEncCallbacks,
) -> AacEncoderError {
    // SAFETY: the caller's guarantee.
    unsafe {
        fdk_aac_enc_encode_frame_pool(
            h_aac_enc,
            h_tp_enc,
            input_buffer,
            input_buffer_buf_size,
            n_out_bytes,
            ext_payload,
            cb,
            None,
        )
    }
}

/// [`fdk_aac_enc_encode_frame`] with the opt-in multi-threading (`par`, see [`crate::par`]):
/// the same frame, with the parallel variants of the psychoacoustic model and the quantization.
///
/// # Safety
/// As [`fdk_aac_enc_encode_frame`].
#[allow(clippy::too_many_arguments)]
pub unsafe fn fdk_aac_enc_encode_frame_pool(
    h_aac_enc: &mut AacEnc,
    h_tp_enc: &mut TransportEnc,
    input_buffer: &[IntPcm],
    input_buffer_buf_size: u32,
    n_out_bytes: &mut [i32],
    ext_payload: &mut [AacencExtPayload; MAX_TOTAL_EXT_PAYLOADS],
    cb: &mut dyn TpEncCallbacks,
    mut par: Option<crate::par::EncPar<'_>>,
) -> AacEncoderError {
    let mut error_status: AacEncoderError;
    let mut ext_payload_used = [0u8; MAX_TOTAL_EXT_PAYLOADS];

    let cm = h_aac_enc.channel_mapping;
    let syntax_flags = h_aac_enc.config.syntax_flags;
    let ep_config = h_aac_enc.config.ep_config;
    let aot = h_aac_enc.aot;
    let ancillary_bits_per_frame = h_aac_enc.ancillary_bits_per_frame;
    let max_anc_bytes_per_au = h_aac_enc.config.max_anc_bytes_per_au;
    let config = h_aac_enc.config;
    let bitrate_mode = h_aac_enc.bitrate_mode;

    let (Some(psy_kernel), Some(qc_kernel), Some(psy_out), Some(qc_out)) = (
        h_aac_enc.psy_kernel.as_deref_mut(),
        h_aac_enc.qc_kernel.as_deref_mut(),
        h_aac_enc.psy_out[0].as_deref_mut(),
        h_aac_enc.qc_out[0].as_deref_mut(),
    ) else {
        return AAC_ENC_INVALID_HANDLE;
    };

    qc_out.element_ext_bits = 0; // sum up all extended bit of each element
    qc_out.static_bits = 0; // sum up side info bits of each element
    qc_out.total_no_red_pe = 0; // sum up PE

    // Where the reference has no defined result, the port returns an error (PORTING.md,
    // "Deviations from the reference").
    {
        let gl = psy_kernel.granule_length;
        let long = &psy_kernel.psy_conf[0];
        let short = &psy_kernel.psy_conf[1];
        // (short blocks only exist with the LC filterbank)
        if !(0..=gl).contains(&long.lowpass_line)
            || (long.filterbank == crate::psy_const::FB_LC
                && !(0..=gl / 8).contains(&short.lowpass_line))
            || !(0..=gl).contains(&long.lowpass_line_lfe)
        {
            return AAC_ENC_UNKNOWN;
        }
    }

    // "update pointer!": the six array pointers of every PSY_OUT_CHANNEL of an audio element point
    // to the arrays of the element's QC_OUT_CHANNEL. C sets them in the element loop below, just
    // before the element's psyMain; the port reads the element's own QC_OUT_CHANNEL there and
    // records the pointers for the stages after the loop. PSY_OUT_CHANNELs can be shared
    // between elements; the port follows the same array pointers (see the interface module
    // documentation).
    {
        let n_psy = psy_out.p_psy_out_channels.len();
        psy_out.p_array_qc.resize(n_psy, crate::psy_main::UNASSIGNED);
        for el in 0..cm.n_elements.clamp(0, 8) as usize {
            let el_info = cm.el_info[el];
            if (el_info.el_type == ID_SCE)
                || (el_info.el_type == ID_CPE)
                || (el_info.el_type == ID_LFE)
            {
                for ch in 0..el_info.n_channels_in_el.clamp(0, 2) as usize {
                    let p = psy_out.psy_out_element[el].psy_out_channel[ch];
                    if p < n_psy {
                        psy_out.p_array_qc[p] = qc_out.qc_element[el].qc_out_channel[0] + ch;
                    }
                }
            }
        }
    }

    // (opt-in multi-threading: the psychoacoustics of the elements in parallel)
    let mut el_state = [ElState::Fresh; 8];
    if let Some(par) = par.as_mut() {
        psy_elements_parallel(
            &mut el_state,
            par,
            psy_kernel,
            psy_out,
            qc_out,
            &mut qc_kernel.h_adj_thr.adj_thr_state_elem,
            &cm,
            input_buffer,
            input_buffer_buf_size,
            aot,
            syntax_flags,
            ep_config,
        );
    }

    // advance psychoacoustics
    for el in 0..cm.n_elements.clamp(0, 8) as usize {
        let el_info = cm.el_info[el];

        // (not in C: the spectral lines that the reference does not define; see
        // psy_main_undefined_spectrum_reads)
        let el_tracked = if (el_info.el_type == ID_SCE)
            || (el_info.el_type == ID_CPE)
            || (el_info.el_type == ID_LFE)
        {
            crate::psy_main::psy_element_statics(
                &psy_kernel.psy_element[el].psy_static,
                el_info.n_channels_in_el,
                psy_kernel.p_static_channels.len(),
            )
            .map(|st| (st, el_info.n_channels_in_el.clamp(0, 2) as usize))
        } else {
            None
        };
        let el_q0 = qc_out.qc_element[el].qc_out_channel[0];
        if let Some((st, n)) = el_tracked {
            undefined_spectrum_check(psy_kernel, qc_out, &st[..n], el_q0);
        }

        if ((el_info.el_type == ID_SCE)
            || (el_info.el_type == ID_CPE)
            || (el_info.el_type == ID_LFE))
            && el_state[el] != ElState::Done
        {
            if let ElState::Failed(error) = el_state[el] {
                return error;
            }
            let n = el_info.n_channels_in_el.clamp(0, 2) as usize;
            // The reference has no defined result for an element without statics or output
            // channels for this channel mode; the port refuses the frame
            // (psy_main_stale_element_mappings).
            let (Some(st), Some(p0)) = (
                crate::psy_main::psy_element_statics(
                    &psy_kernel.psy_element[el].psy_static,
                    el_info.n_channels_in_el,
                    psy_kernel.p_static_channels.len(),
                ),
                crate::psy_main::psy_element_channels(
                    &psy_out.psy_out_element[el].psy_out_channel,
                    el_info.n_channels_in_el,
                    psy_out.p_psy_out_channels.len(),
                ),
            ) else {
                crate::psy_main::stale_element_mapping();
                return AAC_ENC_UNKNOWN;
            };
            // The statics of a pair need not be consecutive (see
            // psy_main_stale_element_mappings); C uses them as they are. The port swaps the
            // second one next to the first for the call and back afterwards.
            let s0 = st[0];
            let swap = (n == 2 && st[1] != s0 + 1).then_some((s0 + 1, st[1]));
            if let Some((x, y)) = swap {
                if x >= psy_kernel.p_static_channels.len() {
                    // (not reached: the first static of such a pair is 0)
                    crate::psy_main::stale_element_mapping();
                    return AAC_ENC_UNKNOWN;
                }
                psy_kernel.p_static_channels.swap(x, y);
            }
            let q0 = qc_out.qc_element[el].qc_out_channel[0];

            // update pointer! (the six arrays of PSY_OUT_CHANNEL are those of the
            // QC_OUT_CHANNELs of the element, passed with it)

            // (verification: the output must not depend on what PSY_DYNAMIC holds before)
            #[cfg(feature = "verify-poison")]
            crate::psy_main::poison_psy_dynamic(
                &mut psy_kernel.psy_dynamic,
                (el as u32) ^ (input_buffer.first().copied().unwrap_or(0) as u32),
            );
            error_status = if let ElState::Started(frame) = el_state[el] {
                // (psy_main_start() ran in psy_elements_parallel())
                crate::psy_main::psy_main_finish(
                    &frame,
                    el_info.n_channels_in_el,
                    &mut psy_kernel.p_static_channels[s0..s0 + n],
                    &mut psy_kernel.psy_dynamic,
                    &psy_kernel.psy_conf,
                    &mut psy_out.psy_out_element[el],
                    &mut psy_out.p_psy_out_channels[p0..p0 + n],
                    &mut qc_out.p_qc_out_channels[q0..q0 + n],
                    input_buffer,
                    input_buffer_buf_size,
                    &cm.el_info[el].channel_index,
                    par.as_ref().map(|p| p.pool),
                )
            } else {
                fdk_aac_enc_psy_main_pool(
                    el_info.n_channels_in_el,
                    &mut psy_kernel.p_static_channels[s0..s0 + n],
                    &mut psy_kernel.psy_dynamic,
                    &mut psy_kernel.psy_conf,
                    &mut psy_out.psy_out_element[el],
                    &mut psy_out.p_psy_out_channels[p0..p0 + n],
                    &mut qc_out.p_qc_out_channels[q0..q0 + n],
                    input_buffer,
                    input_buffer_buf_size,
                    &cm.el_info[el].channel_index,
                    cm.n_channels,
                    par.as_ref().map(|p| p.pool),
                )
            };
            if let Some((x, y)) = swap {
                psy_kernel.p_static_channels.swap(x, y);
            }

            if error_status != AAC_ENC_OK {
                return error_status;
            }

            // FormFactor, Pe and staticBitDemand calculation
            error_status = fdk_aac_enc_qc_main_prepare(
                &el_info,
                &mut qc_kernel.h_adj_thr.adj_thr_state_elem[el],
                &mut psy_out.psy_out_element[el],
                &mut psy_out.p_psy_out_channels[p0..p0 + n],
                &mut qc_out.qc_element[el],
                &mut qc_out.p_qc_out_channels[q0..q0 + n],
                aot,
                syntax_flags,
                ep_config,
            );

            if error_status != AAC_ENC_OK {
                return error_status;
            }
        }
        if let Some((st, n)) = el_tracked {
            undefined_spectrum_update(psy_kernel, qc_out, &st[..n], el_q0);
        }

        if (el_info.el_type == ID_SCE) || (el_info.el_type == ID_CPE) || (el_info.el_type == ID_LFE)
        {
            // --------------------------------------------

            let qe = &mut qc_out.qc_element[el];
            qe.ext_bits_used = 0;
            qe.n_extensions = 0;
            // reset extension payload
            qe.extension = [QcOutExtension::default(); 1];

            for n in 0..MAX_TOTAL_EXT_PAYLOADS {
                let ep = ext_payload[n];
                if ext_payload_used[n] == 0
                    && (ep.associated_ch_element == el as i32)
                    && (ep.data_size > 0)
                    && !ep.p_data.is_null()
                {
                    let idx = qe.n_extensions as usize;
                    qe.n_extensions += 1;

                    let ext = QcOutExtension {
                        type_: ep.data_type, // Perform a sanity check on the type?
                        n_payload_bits: ep.data_size as i32,
                        p_payload: ep.p_data,
                    };
                    // Bounded to the 1-entry array; no element gets more than one payload (one
                    // SBR payload per SBR element).
                    if idx < 1 {
                        qe.extension[idx] = ext;
                    }
                    // Now ask the bitstream encoder how many bits we need to encode the data
                    // with the current bitstream syntax:
                    // SAFETY: without a transport encoder nothing is dereferenced.
                    qe.ext_bits_used += unsafe {
                        fdk_aac_enc_write_extension_data(
                            None,
                            &ext,
                            0,
                            0,
                            syntax_flags,
                            aot,
                            ep_config,
                        )
                    };
                    ext_payload_used[n] = 1;
                }
            }

            // sum up extension and static bits for all channel elements
            qc_out.element_ext_bits += qc_out.qc_element[el].ext_bits_used;
            qc_out.static_bits += qc_out.qc_element[el].static_bits_used;

            // sum up pe
            qc_out.total_no_red_pe += qc_out.qc_element[el].pe_data.pe;
        }
    }

    qc_out.n_extensions = 0;
    qc_out.global_ext_bits = 0;

    // reset extension payload
    qc_out.extension = [QcOutExtension::default(); 2 + 2];

    // Add extension payload not assigned to an channel element (Ancillary data is the only
    // supported type up to now)
    for n in 0..MAX_TOTAL_EXT_PAYLOADS {
        let ep = ext_payload[n];
        if ext_payload_used[n] == 0 && (ep.associated_ch_element == -1) && !ep.p_data.is_null() {
            let mut payload_bits: u32 = 0;

            if ep.data_type == EXT_DATA_ELEMENT {
                if ancillary_bits_per_frame != 0 {
                    // granted frame dse bitrate
                    payload_bits = ancillary_bits_per_frame as u32;
                } else {
                    // write anc data if bitrate constraint fulfilled
                    if (ep.data_size >> 3) <= max_anc_bytes_per_au {
                        payload_bits = ep.data_size;
                    }
                }
                payload_bits = fix_min(ep.data_size, payload_bits);
            } else {
                payload_bits = ep.data_size;
            }

            if payload_bits > 0 {
                let idx = qc_out.n_extensions as usize;
                qc_out.n_extensions += 1;

                let ext = QcOutExtension {
                    type_: ep.data_type, // Perform a sanity check on the type?
                    n_payload_bits: payload_bits as i32,
                    p_payload: ep.p_data,
                };
                // At most three global payloads exist (MPS or DRC metadata, ETSI metadata,
                // ancillary data); bounded to the 4-entry array.
                if idx < 4 {
                    qc_out.extension[idx] = ext;
                }
                // Now ask the bitstream encoder how many bits we need to encode the data with
                // the current bitstream syntax:
                // SAFETY: without a transport encoder nothing is dereferenced.
                qc_out.global_ext_bits += unsafe {
                    fdk_aac_enc_write_extension_data(None, &ext, 0, 0, syntax_flags, aot, ep_config)
                };
                if ep.data_type == EXT_DATA_ELEMENT {
                    // substract the processed bits
                    ext_payload[n].data_size = ext_payload[n].data_size.wrapping_sub(payload_bits);
                }
                ext_payload_used[n] = 1;
            }
        }
    }

    if syntax_flags & (AC_SCALABLE | AC_ER) == 0 {
        qc_out.global_ext_bits += EL_ID_BITS; // add bits for ID_END
    }

    // build bitstream all nSubFrames
    {
        let mut total_bits: i32 = 0; // Total AU bits
        let mut avg_total_bits: i32 = 0;

        // --------------------------------------------
        // Get average total bits
        // --------------------------------------------
        {
            // frame wise bitrate adaption
            fdk_aac_enc_adjust_bitrate(
                qc_kernel,
                &cm,
                &mut avg_total_bits,
                config.bit_rate,
                config.sample_rate,
                config.framelength,
            );

            // adjust super frame bitrate
            avg_total_bits = avg_total_bits.wrapping_mul(config.n_sub_frames);
        }

        // Make first estimate of transport header overhead. Take maximum possible frame size
        // into account to prevent bitreservoir underrun.
        qc_kernel.glob_hdr_bits = transport_enc_get_static_bits(
            h_tp_enc,
            avg_total_bits.wrapping_add(qc_kernel.bit_res_tot),
        );

        // --------------------------------------------

        error_status = fdk_aac_enc_qc_main_pool(
            qc_kernel,
            psy_out,
            qc_out,
            avg_total_bits,
            &cm,
            aot,
            syntax_flags,
            ep_config,
            par.as_mut(),
        );

        if error_status != AAC_ENC_OK {
            return error_status;
        }

        // --------------------------------------------
        error_status = fdk_aac_enc_update_fill_bits(&cm, qc_kernel, qc_out);
        if error_status != AAC_ENC_OK {
            return error_status;
        }

        // --------------------------------------------
        error_status = fdk_aac_enc_finalize_bit_consumption(
            &cm,
            qc_kernel,
            qc_out,
            h_tp_enc,
            aot,
            syntax_flags,
            ep_config,
        );
        if error_status != AAC_ENC_OK {
            return error_status;
        }
        // --------------------------------------------
        total_bits += qc_out.total_bits;

        // --------------------------------------------
        fdk_aac_enc_update_bitres(&cm, qc_kernel, qc_out);

        // --------------------------------------------

        // for ( all sub frames ) ...
        // write bitstream header
        let tp_bitres = enc_bitres_to_tp_bitres(
            bitrate_mode,
            qc_kernel.bit_res_tot,
            config.audio_mux_version,
            cm.n_channels_eff,
        );
        if TRANSPORTENC_OK
            != transport_enc_write_access_unit_cb(
                Some(&mut *h_tp_enc),
                total_bits,
                tp_bitres,
                cm.n_channels_eff,
                cb,
            )
        {
            return AAC_ENC_UNKNOWN;
        }

        // write bitstream
        // SAFETY: the caller's guarantee for the payloads.
        error_status = unsafe {
            fdk_aac_enc_write_bitstream(
                h_tp_enc,
                &cm,
                qc_out,
                psy_out,
                qc_kernel,
                aot,
                syntax_flags,
                ep_config,
            )
        };

        if error_status != AAC_ENC_OK {
            return error_status;
        }

        // transportEnc_EndAccessUnit() is being called inside FDKaacEnc_WriteBitstream()
        if TRANSPORTENC_OK != transport_enc_get_frame(h_tp_enc, &mut n_out_bytes[0]) {
            return AAC_ENC_UNKNOWN;
        }
    } // -end- if (curFrame==hAacEnc->qcKernel->nSubFrames)

    // --------------------------------------------
    AAC_ENC_OK
}

/// `FDKaacEnc_Close(phAacEnc)`: delete encoder instance.
pub fn fdk_aac_enc_close(ph_aac_enc: &mut Option<Box<AacEnc>>) {
    let Some(h_aac_enc) = ph_aac_enc.as_deref_mut() else {
        return;
    };

    fdk_aac_enc_psy_close(Some(&mut h_aac_enc.psy_kernel), Some(&mut h_aac_enc.psy_out));

    fdk_aac_enc_qc_close(Some(&mut h_aac_enc.qc_kernel), Some(&mut h_aac_enc.qc_out));

    *ph_aac_enc = None;
}

/// `MAX_ANCRATE`: ancillary rate >= 19200 isn't valid
const MAX_ANCRATE: i32 = 19200;

/// `FDKaacEnc_InitCheckAncillary(bitRate, framelength, ancillaryRate, ancillaryBitsPerFrame,
/// sampleRate)`: initialize and check ancillary data struct.
pub fn fdk_aac_enc_init_check_ancillary(
    bit_rate: i32,
    framelength: i32,
    mut ancillary_rate: i32,
    ancillary_bits_per_frame: &mut i32,
    sample_rate: i32,
) -> AacEncoderError {
    // don't use negative ancillary rates
    if ancillary_rate < -1 {
        return AAC_ENC_UNSUPPORTED_ANC_BITRATE;
    }

    // check if ancillary rate is ok
    if (ancillary_rate != -1) && (ancillary_rate != 0) {
        // ancRate <= 15% of bitrate && ancRate < 19200
        if (ancillary_rate >= MAX_ANCRATE)
            || (ancillary_rate.wrapping_mul(20) > bit_rate.wrapping_mul(3))
        {
            return AAC_ENC_UNSUPPORTED_ANC_BITRATE;
        }
    } else if ancillary_rate == -1 {
        // if no special ancRate is requested but a ancillary file is stated, then generate a
        // ancillary rate matching to the bitrate
        if bit_rate >= (MAX_ANCRATE * 10) {
            // ancillary rate is 19199
            ancillary_rate = MAX_ANCRATE - 1;
        } else {
            // 10% of bitrate
            ancillary_rate = bit_rate / 10;
        }
    }

    // make ancillaryBitsPerFrame byte align
    *ancillary_bits_per_frame =
        fdk_aac_enc_calc_bits_per_frame(ancillary_rate, framelength, sample_rate) & !0x7;

    AAC_ENC_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The word masks of `QcOut::mdct_undefined` against a line-by-line model.
    #[test]
    fn undefined_lines_masks_model() {
        let mut seed = 12345u32;
        let mut next = |m: usize| {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            (seed >> 8) as usize % m
        };
        let mut b = [0u64; 16];
        let mut model = [false; 1024];
        for _ in 0..20000 {
            let (from, to, v) = (next(1100), next(1100), next(2) == 1);
            undefined_lines_set(&mut b, from, to, v);
            for l in from..to.min(1024) {
                model[l] = v;
            }
            let (a, c) = (next(1100), next(1100));
            let any = (a..c.min(1024)).any(|l| model[l]);
            assert_eq!(undefined_lines_any(&b, a, c), any, "{a}..{c}");
        }
        for l in 0..1024 {
            assert_eq!(b[l >> 6] >> (l & 63) & 1 == 1, model[l]);
        }
    }
}
