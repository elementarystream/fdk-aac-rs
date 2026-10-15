// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACdec/src/sac_bitdec.h, libSACdec/src/sac_bitdec.cpp.

//! Port of `sac_bitdec.h` and `sac_bitdec.cpp`: the bitstream decoder of MPEG Surround (the
//! spatial specific configs of LD MPEG Surround and USAC MPS212, the frame data) and the
//! decoding and mapping of the parameters.
//!
//! Deviations (PORTING.md, "Deviations from the reference"): indices of the per-box and
//! per-channel arrays beyond their sizes, for which the reference has no defined result, are not
//! accessed; the functions return an error code or skip the access, as documented at each
//! function.

use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::nlc_dec::*;

use crate::sac_dec::*;
use crate::sac_dec_conceal::*;
use crate::sac_dec_interface::*;
use crate::sac_rom::*;
use crate::sac_tsd::*;

/// `TREEPROPERTIES`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeProperties {
    /// `numInputChannels`
    pub num_input_channels: i8,
    /// `numOutputChannels`
    pub num_output_channels: i8,
    /// `numOttBoxes`
    pub num_ott_boxes: i8,
    /// `numTttBoxes`
    pub num_ttt_boxes: i8,
    /// `ottModeLfe`
    pub ott_mode_lfe: [i8; MAX_NUM_OTT],
}

/// `TREE_212`
pub const TREE_212: i32 = 7;
/// `TREE_DUMMY`
pub const TREE_DUMMY: i32 = 255;

/// `QUANT_FINE`
pub const QUANT_FINE: i32 = 0;
/// `QUANT_EBQ1`
pub const QUANT_EBQ1: i32 = 1;
/// `QUANT_EBQ2`
pub const QUANT_EBQ2: i32 = 2;

/// `ottVsTotInactiv`
const OTT_VS_TOT_INACTIV: i8 = 0;
/// `ottVsTotDb1Activ`
const OTT_VS_TOT_DB1_ACTIV: i8 = 1;
/// `ottVsTotDb2Activ`
const OTT_VS_TOT_DB2_ACTIV: i8 = 2;

/// `treePropertyTable[treeConfig]` (deviation: a tree configuration outside of the table gives
/// zeros).
pub(crate) fn tree_property(tree_config: i32) -> TreeProperties {
    TREE_PROPERTY_TABLE.get(tree_config as usize).copied().unwrap_or_default()
}

/// `SpatialDecDecodeHelperInfo(pSpatialSpecificConfig, upmixType)`: derives the helper
/// variables of a configuration.
fn spatial_dec_decode_helper_info(
    p_ssc: &mut SpatialSpecificConfig,
    _upmix_type: Upmixtype,
) -> SacdecError {
    use fdk_aac_rs_fdk::audio::*;

    // Determine bit stream syntax
    let mut syntax_flags = 0u32;
    match p_ssc.core_codec {
        AOT_ER_AAC_ELD | AOT_ER_AAC_LD => syntax_flags |= SACDEC_SYNTAX_LD,
        AOT_USAC => syntax_flags |= SACDEC_SYNTAX_USAC,
        _ => return MPS_UNSUPPORTED_FORMAT,
    }

    p_ssc.syntax_flags = syntax_flags;

    match p_ssc.tree_config {
        TREE_212 => {
            p_ssc.ott_cld_default[0] = 0;
        }
        _ => return MPS_INVALID_TREECONFIG,
    }

    if syntax_flags & SACDEC_SYNTAX_USAC != 0 {
        if p_ssc.bs_ott_bands_phase_present != 0 {
            p_ssc.num_ott_bands_ipd = p_ssc.bs_ott_bands_phase as u8;
        } else {
            let num_parameter_bands = p_ssc.freq_res;
            match num_parameter_bands {
                4 | 5 => p_ssc.num_ott_bands_ipd = 2,
                7 => p_ssc.num_ott_bands_ipd = 3,
                10 => p_ssc.num_ott_bands_ipd = 5,
                14 => p_ssc.num_ott_bands_ipd = 7,
                20 | 28 => p_ssc.num_ott_bands_ipd = 10,
                _ => return MPS_INVALID_PARAMETERBANDS,
            }
        }
    } else {
        p_ssc.num_ott_bands_ipd = 0;
    }
    // nOttBoxes <= MAX_NUM_OTT for every tree configuration of treePropertyTable.
    for i in 0..(p_ssc.n_ott_boxes.max(0) as usize).min(MAX_NUM_OTT) {
        p_ssc.bitstream_ott_bands[i] = p_ssc.freq_res as u8;
        p_ssc.num_ott_bands[i] = p_ssc.bitstream_ott_bands[i];
        if syntax_flags & SACDEC_SYNTAX_USAC != 0
            && p_ssc.bs_ott_bands_phase_present == 0
            && p_ssc.b_residual_coding != 0
            && p_ssc.residual_config[i].b_residual_present != 0
            && (p_ssc.num_ott_bands_ipd as i32) < p_ssc.residual_config[i].n_residual_bands
        {
            p_ssc.num_ott_bands_ipd = p_ssc.residual_config[i].n_residual_bands as u8;
        }
    }

    MPS_OK
}

/// `SpatialDecParseExtensionConfig(bitstream, config, numOttBoxes, numTttBoxes, numOutChan,
/// bitsAvailable)`
fn spatial_dec_parse_extension_config(
    bitstream: &mut FdkBitStream,
    config: &mut SpatialSpecificConfig,
    _num_ott_boxes: i32,
    _num_ttt_boxes: i32,
    _num_out_chan: i32,
    bits_available: i32,
) -> SacdecError {
    let mut ba = bits_available;

    config.sac_ext_cnt = 0;
    config.b_residual_coding = 0;

    ba = f_min(fdk_get_valid_bits(bitstream) as i32, ba);

    while ba >= 8 && (config.sac_ext_cnt as usize) < MAX_NUM_EXT_TYPES {
        let cnt = config.sac_ext_cnt as usize;
        config.sac_ext_type[cnt] = fdk_read_bits(bitstream, 4) as i32;
        ba -= 4;

        let mut sac_ext_len = fdk_read_bits(bitstream, 4);
        ba -= 4;

        if sac_ext_len == 15 {
            sac_ext_len += fdk_read_bits(bitstream, 8);
            ba -= 8;
            if sac_ext_len == 15 + 255 {
                sac_ext_len += fdk_read_bits(bitstream, 16);
                ba -= 16;
            }
        }

        let tmp = fdk_get_valid_bits(bitstream) as i32; // Extension config payload start anchor.
        if tmp <= 0 || tmp < (sac_ext_len as i32) * 8 || ba < (sac_ext_len as i32) * 8 {
            return MPS_PARSE_ERROR;
        }

        // unknown extension data => do nothing

        // skip remaining extension data
        let bits_read = tmp - fdk_get_valid_bits(bitstream) as i32;
        let n_fill_bits = 8 * sac_ext_len as i32 - bits_read;

        if n_fill_bits < 0 {
            return MPS_PARSE_ERROR;
        }
        // Skip fill bits or an unknown extension.
        fdk_push_for(bitstream, n_fill_bits as u32);

        ba -= 8 * sac_ext_len as i32;
        config.sac_ext_cnt += 1;
    }

    MPS_OK
}

/// `SpatialDecParseSpecificConfigHeader(bitstream, pSpatialSpecificConfig, coreCodec,
/// upmixType)`: parses the in-band spatial specific config of LD MPEG Surround (with its
/// length header).
pub fn spatial_dec_parse_specific_config_header(
    bitstream: &mut FdkBitStream,
    p_ssc: &mut SpatialSpecificConfig,
    core_codec: i32,
    upmix_type: SpatialDecUpmixType,
) -> SacdecError {
    let sac_time_align_flag = fdk_read_bits(bitstream, 1);
    let mut sac_header_len = fdk_read_bits(bitstream, 7) as i32;

    if sac_header_len == 127 {
        sac_header_len += fdk_read_bits(bitstream, 16) as i32;
    }
    let mut num_fill_bits = fdk_get_valid_bits(bitstream) as i32;

    let mut err = spatial_dec_parse_specific_config(bitstream, p_ssc, sac_header_len, core_codec);

    num_fill_bits -= fdk_get_valid_bits(bitstream) as i32; // the number of read bits (tmpBits)
    num_fill_bits = (8 * sac_header_len) - num_fill_bits;
    if num_fill_bits < 0 {
        // Parsing went wrong
        err = MPS_PARSE_ERROR;
    }
    // Move to the very end of the SSC
    fdk_push_bi_directional(bitstream, num_fill_bits);

    if err == MPS_OK && sac_time_align_flag != 0 {
        // not supported
        fdk_read_bits(bitstream, 16);
        err = MPS_UNSUPPORTED_CONFIG;
    }

    // Derive additional helper variables
    spatial_dec_decode_helper_info(p_ssc, upmix_type);

    err
}

/// `SpatialDecParseMps212Config(bitstream, pSpatialSpecificConfig, samplingRate, coreCodec,
/// stereoConfigIndex, coreSbrFrameLengthIndex)`: parses the USAC MPS212 config.
pub fn spatial_dec_parse_mps212_config(
    bitstream: &mut FdkBitStream,
    p_ssc: &mut SpatialSpecificConfig,
    sampling_rate: i32,
    mut core_codec: i32,
    stereo_config_index: i32,
    core_sbr_frame_length_index: i32,
) -> SacdecError {
    use fdk_aac_rs_fdk::audio::*;

    *p_ssc = SpatialSpecificConfig::default();

    p_ssc.stereo_config_index = stereo_config_index as u8;
    p_ssc.core_sbr_frame_length_index = core_sbr_frame_length_index as u8;
    p_ssc.freq_res = FREQ_RES_TABLE[fdk_read_bits(bitstream, 3) as usize] as i32;
    if p_ssc.freq_res == 0 {
        return MPS_PARSE_ERROR; // reserved value
    }

    match core_codec {
        AOT_DRM_USAC => {
            p_ssc.bs_fixed_gain_dmx = fdk_read_bits(bitstream, 3) as i32;
            // tempShapeConfig = (bsTempShapeConfigDrm == 1) ? 3 : 0
            p_ssc.temp_shape_config = (fdk_read_bits(bitstream, 1) * 3) as i32;
            p_ssc.decorr_config = 0;
            p_ssc.bs_decorr_type = 0;
        }
        AOT_USAC => {
            p_ssc.bs_fixed_gain_dmx = fdk_read_bits(bitstream, 3) as i32;
            p_ssc.temp_shape_config = fdk_read_bits(bitstream, 2) as i32;
            p_ssc.decorr_config = fdk_read_bits(bitstream, 2) as i32;
            if p_ssc.decorr_config > 2 {
                return MPS_PARSE_ERROR; // reserved value
            }
            p_ssc.bs_decorr_type = 0;
        }
        _ => return MPS_UNSUPPORTED_FORMAT,
    }
    p_ssc.n_time_slots = if core_sbr_frame_length_index == 4 { 64 } else { 32 };
    p_ssc.bs_high_rate_mode = fdk_read_bits(bitstream, 1) as u8;

    {
        p_ssc.bs_phase_coding = fdk_read_bits(bitstream, 1) as u8;
        p_ssc.bs_ott_bands_phase_present = fdk_read_bits(bitstream, 1) as u8;
        if p_ssc.bs_ott_bands_phase_present != 0 {
            p_ssc.bs_ott_bands_phase = fdk_read_bits(bitstream, 5) as i32;
            if (MAX_PARAMETER_BANDS as i32) < p_ssc.bs_ott_bands_phase {
                return MPS_PARSE_ERROR;
            }
        } else {
            p_ssc.bs_ott_bands_phase = 0;
        }
    }

    if stereo_config_index > 1 {
        // do residual coding
        p_ssc.b_residual_coding = 1;
        p_ssc.residual_config[0].b_residual_present = 1;
        p_ssc.residual_config[0].n_residual_bands = fdk_read_bits(bitstream, 5) as i32;
        if p_ssc.freq_res < p_ssc.residual_config[0].n_residual_bands {
            return MPS_PARSE_ERROR;
        }
        p_ssc.bs_ott_bands_phase =
            f_max(p_ssc.bs_ott_bands_phase, p_ssc.residual_config[0].n_residual_bands);
        p_ssc.bs_pseudo_lr = fdk_read_bits(bitstream, 1) as u8;

        if p_ssc.bs_phase_coding != 0 {
            p_ssc.bs_phase_coding = 3;
        }
    } else {
        p_ssc.b_residual_coding = 0;
        p_ssc.residual_config[0].b_residual_present = 0;
    }

    if p_ssc.temp_shape_config == 2 {
        if core_codec == AOT_USAC {
            p_ssc.env_quant_mode = fdk_read_bits(bitstream, 1) as i32;
        }
    }

    // Static parameters

    p_ssc.sampling_freq = sampling_rate; // wrong for stereoConfigIndex == 3 but value is unused
    p_ssc.tree_config = SPATIALDEC_MODE_RSVD7;
    let tp = tree_property(p_ssc.tree_config);
    p_ssc.n_ott_boxes = tp.num_ott_boxes as i32;
    p_ssc.n_input_channels = tp.num_input_channels as i32;
    p_ssc.n_output_channels = tp.num_output_channels as i32;

    p_ssc.b_arbitrary_downmix = 0;

    for i in 0..p_ssc.n_ott_boxes as usize {
        p_ssc.ott_config[i].n_ott_bands = 0;
    }

    if core_codec == AOT_DRM_USAC {
        // MPS payload is MPEG conform -> no need for pseudo DRM AOT
        core_codec = AOT_USAC;
    }
    p_ssc.core_codec = core_codec;

    // Derive additional helper variables
    spatial_dec_decode_helper_info(p_ssc, UPMIXTYPE_NORMAL);

    MPS_OK
}

/// `SpatialDecParseSpecificConfig(bitstream, pSpatialSpecificConfig, sacHeaderLen,
/// coreCodec)`: parses the spatial specific config of LD MPEG Surround.
pub fn spatial_dec_parse_specific_config(
    bitstream: &mut FdkBitStream,
    p_ssc: &mut SpatialSpecificConfig,
    sac_header_len: i32,
    core_codec: i32,
) -> SacdecError {
    let err;
    let b3d_audio_mode = 0;

    *p_ssc = SpatialSpecificConfig::default();

    let cfg_start_pos = fdk_get_valid_bits(bitstream) as i32;
    let mut bits_available;
    // It might be that we do not know the SSC length beforehand.
    if sac_header_len == 0 {
        bits_available = cfg_start_pos;
    } else {
        bits_available = 8 * sac_header_len;
        if bits_available > cfg_start_pos {
            err = MPS_PARSE_ERROR;
            return parse_specific_config_bail(bitstream, sac_header_len, cfg_start_pos, err);
        }
    }

    'bail: {
        let bs_sampling_freq_index = fdk_read_bits(bitstream, 4);

        if bs_sampling_freq_index == 15 {
            p_ssc.sampling_freq = fdk_read_bits(bitstream, 24) as i32;
        } else {
            p_ssc.sampling_freq = SAMPLING_FREQ_TABLE[bs_sampling_freq_index as usize];
            if p_ssc.sampling_freq == 0 {
                err = MPS_PARSE_ERROR;
                break 'bail;
            }
        }

        p_ssc.n_time_slots = fdk_read_bits(bitstream, 5) as i32 + 1;
        if p_ssc.n_time_slots < 1 || p_ssc.n_time_slots > MAX_TIME_SLOTS {
            err = MPS_PARSE_ERROR;
            break 'bail;
        }

        let bs_freq_res = fdk_read_bits(bitstream, 3);

        p_ssc.freq_res = FREQ_RES_TABLE_LD[bs_freq_res as usize] as i32;

        {
            let tree_config = fdk_read_bits(bitstream, 4);

            match tree_config as i32 {
                SPATIALDEC_MODE_RSVD7 => p_ssc.tree_config = tree_config as i32,
                _ => {
                    err = MPS_UNSUPPORTED_CONFIG;
                    break 'bail;
                }
            }
        }

        {
            let tp = tree_property(p_ssc.tree_config);
            p_ssc.n_ott_boxes = tp.num_ott_boxes as i32;
            p_ssc.n_ttt_boxes = tp.num_ttt_boxes as i32;
            p_ssc.n_input_channels = tp.num_input_channels as i32;
            p_ssc.n_output_channels = tp.num_output_channels as i32;
        }

        p_ssc.quant_mode = fdk_read_bits(bitstream, 2) as i32;

        p_ssc.b_arbitrary_downmix = fdk_read_bits(bitstream, 1) as i32;

        p_ssc.bs_fixed_gain_dmx = fdk_read_bits(bitstream, 3) as i32;

        p_ssc.temp_shape_config = fdk_read_bits(bitstream, 2) as i32;
        if p_ssc.temp_shape_config > 2 {
            return MPS_PARSE_ERROR; // reserved value
        }

        p_ssc.decorr_config = fdk_read_bits(bitstream, 2) as i32;
        if p_ssc.decorr_config > 2 {
            return MPS_PARSE_ERROR; // reserved value
        }

        for i in 0..p_ssc.n_ott_boxes as usize {
            p_ssc.ott_config[i].n_ott_bands = 0;
        }

        for _ in 0..p_ssc.n_ttt_boxes {
            let b_ttt_dual_mode = fdk_read_bits(bitstream, 1);
            fdk_read_bits(bitstream, 3); // not supported

            if b_ttt_dual_mode != 0 {
                fdk_read_bits(bitstream, 8); // not supported
            }
        }

        if p_ssc.temp_shape_config == 2 {
            p_ssc.env_quant_mode = fdk_read_bits(bitstream, 1) as i32;
        }

        if b3d_audio_mode != 0 {
            if fdk_read_bits(bitstream, 2) == 0 {
                // b3DaudioHRTFset ?
                let hrtf_freq_res = fdk_read_bits(bitstream, 3);
                let hrtf_num_chan = fdk_read_bits(bitstream, 4);
                let hrtf_asymmetric = fdk_read_bits(bitstream, 1);

                let hrtf_num_band = FREQ_RES_TABLE_LD[hrtf_freq_res as usize] as u32;

                for _ in 0..hrtf_num_chan {
                    fdk_push_for(bitstream, hrtf_num_band * 6); // HRTFlevelLeft[hc][hb]
                    if hrtf_asymmetric != 0 {
                        fdk_push_for(bitstream, hrtf_num_band * 6); // HRTFlevelRight[hc][hb]
                    }
                    if fdk_read_bits(bitstream, 1) != 0 {
                        // HRTFphase[hc] ?
                        fdk_push_for(bitstream, hrtf_num_band * 6); // HRTFphaseLR[hc][hb]
                    }
                    if fdk_read_bits(bitstream, 1) != 0 {
                        // HRTFicc[hc] ?
                        fdk_push_for(bitstream, hrtf_num_band * 6); // HRTFiccLR[hc][hb]
                    }
                }
            }
        }

        // ISO/IEC FDIS 23003-1: 5.2. ... byte alignment with respect to the beginning of the
        // syntactic element in which ByteAlign() occurs.
        fdk_byte_align(bitstream, cfg_start_pos as u32);

        let num_header_bits = cfg_start_pos - fdk_get_valid_bits(bitstream) as i32;
        bits_available -= num_header_bits;
        if bits_available < 0 {
            err = MPS_PARSE_ERROR;
            break 'bail;
        }

        p_ssc.sac_ext_cnt = 0;
        p_ssc.b_residual_coding = 0;

        let (n_ott, n_ttt, n_out) = (p_ssc.n_ott_boxes, p_ssc.n_ttt_boxes, p_ssc.n_output_channels);
        err = spatial_dec_parse_extension_config(
            bitstream,
            p_ssc,
            n_ott,
            n_ttt,
            n_out,
            bits_available,
        );

        // Same alignment anchor as above because SpatialExtensionConfig() always reads full
        // bytes
        fdk_byte_align(bitstream, cfg_start_pos as u32);

        p_ssc.core_codec = core_codec;

        spatial_dec_decode_helper_info(p_ssc, UPMIXTYPE_NORMAL);
    }

    parse_specific_config_bail(bitstream, sac_header_len, cfg_start_pos, err)
}

/// The `bail:` label of `SpatialDecParseSpecificConfig()`.
fn parse_specific_config_bail(
    bitstream: &mut FdkBitStream,
    sac_header_len: i32,
    cfg_start_pos: i32,
    err: SacdecError,
) -> SacdecError {
    if sac_header_len > 0 {
        // If the config is of known length then assure that the bitbuffer is exactly at its
        // end when leaving the function.
        let n = (sac_header_len * 8) - (cfg_start_pos - fdk_get_valid_bits(bitstream) as i32);
        fdk_push_bi_directional(bitstream, n);
    }
    err
}

/// `SpatialDecDefaultSpecificConfig(pSpatialSpecificConfig, coreCodec, samplingFreq,
/// nTimeSlots, sacDecoderLevel, isBlind, numCoreChannels)`: the default configuration. The
/// other members keep their values.
pub fn spatial_dec_default_specific_config(
    p_ssc: &mut SpatialSpecificConfig,
    core_codec: i32,
    sampling_freq: i32,
    n_time_slots: i32,
    _sac_decoder_level: i32,
    _is_blind: i32,
    _num_core_channels: i32,
) -> i32 {
    use fdk_aac_rs_fdk::audio::*;
    let err = MPS_OK;

    fdk_assert!(core_codec != AOT_NONE);
    fdk_assert!(n_time_slots > 0);
    fdk_assert!(sampling_freq > 0);

    p_ssc.core_codec = core_codec;
    p_ssc.sampling_freq = sampling_freq;
    p_ssc.n_time_slots = n_time_slots;
    if p_ssc.core_codec == AOT_ER_AAC_ELD || p_ssc.core_codec == AOT_ER_AAC_LD {
        p_ssc.freq_res = SPATIALDEC_FREQ_RES_23;
    } else {
        p_ssc.freq_res = SPATIALDEC_FREQ_RES_28;
    }

    p_ssc.tree_config = SPATIALDEC_MODE_RSVD7; // 212 configuration

    {
        let tp = tree_property(p_ssc.tree_config);
        p_ssc.n_ott_boxes = tp.num_ott_boxes as i32;
        p_ssc.n_input_channels = tp.num_input_channels as i32;
        p_ssc.n_output_channels = tp.num_output_channels as i32;
    }

    p_ssc.quant_mode = SPATIALDEC_QUANT_FINE_DEF;
    p_ssc.b_arbitrary_downmix = 0;
    p_ssc.b_residual_coding = 0;
    if p_ssc.core_codec == AOT_ER_AAC_ELD || p_ssc.core_codec == AOT_ER_AAC_LD {
        p_ssc.bs_fixed_gain_dmx = SPATIALDEC_GAIN_RSVD2;
    } else {
        p_ssc.bs_fixed_gain_dmx = SPATIALDEC_GAIN_MODE0;
    }

    p_ssc.temp_shape_config = SPATIALDEC_TS_TPNOWHITE;
    p_ssc.decorr_config = SPATIALDEC_DECORR_MODE0;

    for i in 0..p_ssc.n_ott_boxes as usize {
        p_ssc.ott_config[i].n_ott_bands = 0;
    }

    err
}

/// `coarse2fine(data, dataType, startBand, numBands)`: parameter index mapping from coarse to
/// fine quantization.
pub fn coarse2fine(
    data: &mut [i8; MAX_PARAMETER_BANDS],
    data_type: DataType,
    start_band: i32,
    num_bands: i32,
) {
    let r = band_range(start_band, start_band + num_bands);
    for i in r.clone() {
        // (SCHAR)(data[i] << 1)
        data[i] = ((data[i] as i32) << 1) as i8;
    }

    if data_type == t_CLD {
        for i in r {
            if data[i] == -14 {
                data[i] = -15;
            } else if data[i] == 14 {
                data[i] = 15;
            }
        }
    }
}

/// `fine2coarse(data, dataType, startBand, numBands)`: parameter index mapping from fine to
/// coarse quantization.
pub fn fine2coarse(
    data: &mut [i8; MAX_PARAMETER_BANDS],
    data_type: DataType,
    start_band: i32,
    num_bands: i32,
) {
    for i in band_range(start_band, start_band + num_bands) {
        // Note: the if cases below actually make a difference (negative values)
        if data_type == t_CLD {
            data[i] /= 2;
        } else {
            data[i] >>= 1;
        }
    }
}

/// The band index range `start..stop` within one row of `MAX_PARAMETER_BANDS` values (the
/// library never uses bands outside of a row).
#[inline]
fn band_range(start: i32, stop: i32) -> core::ops::Range<usize> {
    let s = start.clamp(0, MAX_PARAMETER_BANDS as i32) as usize;
    let e = stop.clamp(0, MAX_PARAMETER_BANDS as i32) as usize;
    s..e.max(s)
}

/// `pbStrideTable[i]` (0..=3 in every use; deviation: other values give the stride 1).
#[inline]
fn pb_stride(i: i32) -> i32 {
    PB_STRIDE_TABLE.get(i as usize).map_or(1, |&v| v as i32)
}

/// `getStrideMap(freqResStride, startBand, stopBand, aStrides)`: index mapping according to
/// the parameter band strides. Returns the number of data bands.
pub fn get_stride_map(
    freq_res_stride: i32,
    start_band: i32,
    stop_band: i32,
    a_strides: &mut [i32; MAX_PARAMETER_BANDS + 1],
) -> i32 {
    let pb_stride = pb_stride(freq_res_stride);
    let data_bands = (stop_band - start_band - 1) / pb_stride + 1;

    // dataBands <= MAX_PARAMETER_BANDS for 0 <= startBand < stopBand <= MAX_PARAMETER_BANDS.
    let n = data_bands.clamp(0, MAX_PARAMETER_BANDS as i32) as usize;
    a_strides[0] = start_band;
    for pb in 1..=n {
        a_strides[pb] = a_strides[pb - 1] + pb_stride;
    }
    let mut str_offset = 0;
    while a_strides[n] > stop_band {
        if str_offset < n {
            str_offset += 1;
        }
        for i in str_offset..=n {
            a_strides[i] -= 1;
        }
    }

    data_bands
}

/// `ecDataDec(frame, syntaxFlags, bitstream, llData, data, lastdata, datatype, boxIdx,
/// startBand, stopBand, defaultValue)`: delta decoding and dequantization of one parameter of
/// one box. `ll_state` is `llData->state`.
///
/// Deviation: a `boxIdx` outside of `data` or `lastdata` returns `MPS_INVALID_BOXIDX`.
fn ec_data_dec(
    frame: &SpatialBsFrame,
    syntax_flags: u32,
    bitstream: &mut FdkBitStream,
    ll_data: &mut LosslessData,
    ll_state: &mut LosslessState,
    data: &mut [ParamSets],
    lastdata: &mut [[i8; MAX_PARAMETER_BANDS]],
    datatype: DataType,
    box_idx: i32,
    start_band: i32,
    stop_band: i32,
    default_value: i8,
) -> SacdecError {
    let mut a_strides = [0i32; MAX_PARAMETER_BANDS + 1];
    let num_parameter_sets = frame.num_parameter_sets as usize;

    let mut _data_sets = 0;
    for i in 0..num_parameter_sets.min(MAX_PARAMETER_SETS) {
        ll_data.bs_xxx_data_mode[i] = fdk_read_bits(bitstream, 2) as i8;

        if frame.bs_independency_flag == 1
            && i == 0
            && (ll_data.bs_xxx_data_mode[i] == 1 || ll_data.bs_xxx_data_mode[i] == 2)
        {
            // This check catches bitstreams generated by older encoder that cause trouble
            return MPS_PARSE_ERROR;
        }
        if i + 1 >= num_parameter_sets && ll_data.bs_xxx_data_mode[i] == 2 {
            // The interpolation mode must not be active for the last parameter set
            return MPS_PARSE_ERROR;
        }

        if ll_data.bs_xxx_data_mode[i] == 3 {
            _data_sets += 1;
        }
    }

    if box_idx < 0 || box_idx as usize >= data.len() || box_idx as usize >= lastdata.len() {
        return MPS_INVALID_BOXIDX;
    }
    let bx = box_idx as usize;

    let mut set_idx: i32 = 0;
    let mut bs_data_pair: i32 = 0;
    let mut old_quant_coarse_xxx = ll_state.bs_quant_coarse_xxx_prev_parse as i32;

    for i in 0..num_parameter_sets.min(MAX_PARAMETER_SETS) {
        if ll_data.bs_xxx_data_mode[i] == 0 {
            for pb in band_range(start_band, stop_band) {
                lastdata[bx][pb] = default_value;
            }

            old_quant_coarse_xxx = 0;
        }

        if ll_data.bs_xxx_data_mode[i] == 3 {
            if bs_data_pair != 0 {
                bs_data_pair = 0;
            } else {
                // setIdx <= i <= 7 here (see the documentation of the port).
                let si = set_idx as usize;
                bs_data_pair = fdk_read_bits(bitstream, 1) as i32;
                ll_data.bs_quant_coarse_xxx[si] = fdk_read_bits(bitstream, 1) as u8 as i8;
                ll_data.bs_freq_res_stride_xxx[si] = fdk_read_bits(bitstream, 2) as u8 as i8;

                if ll_data.bs_quant_coarse_xxx[si] as i32 != old_quant_coarse_xxx {
                    if old_quant_coarse_xxx != 0 {
                        coarse2fine(
                            &mut lastdata[bx],
                            datatype,
                            start_band,
                            stop_band - start_band,
                        );
                    } else {
                        fine2coarse(
                            &mut lastdata[bx],
                            datatype,
                            start_band,
                            stop_band - start_band,
                        );
                    }
                }

                let data_bands = get_stride_map(
                    ll_data.bs_freq_res_stride_xxx[si] as i32,
                    start_band,
                    stop_band,
                    &mut a_strides,
                );

                // startBand + pb and aStrides[pb] are below stopBand <= MAX_PARAMETER_BANDS
                // (getStrideMap() keeps the strides increasing).
                for pb in 0..data_bands.max(0) as usize {
                    let src = lastdata[bx].get(a_strides[pb] as usize).copied().unwrap_or(0);
                    if let Some(d) = lastdata[bx].get_mut(start_band as usize + pb) {
                        *d = src;
                    }
                }

                if box_idx > MAX_NUM_OTT as i32 {
                    return MPS_INVALID_BOXIDX;
                }
                if (set_idx + bs_data_pair) > MAX_PARAMETER_SETS as i32 {
                    return MPS_INVALID_SETIDX;
                }

                // DECODER_TYPE defined in FDK_tools
                let mut this_decoder_type = SAC_DECODER;
                if syntax_flags & (SACDEC_SYNTAX_USAC | SACDEC_SYNTAX_RSVD50) != 0 {
                    this_decoder_type = USAC_DECODER;
                } else if syntax_flags & SACDEC_SYNTAX_LD != 0 {
                    this_decoder_type = SAOC_DECODER;
                }

                let err = {
                    let rows = &mut data[bx];
                    // data[boxIdx][setIdx + 1]: setIdx + 1 <= 8.
                    let (lo, hi) = rows.split_at_mut(si + 1);
                    let mut spare = [0i8; MAX_PARAMETER_BANDS];
                    let out2: &mut [i8] = match hi.first_mut() {
                        Some(r) => r,
                        None => &mut spare,
                    };
                    ec_data_pair_dec(
                        this_decoder_type,
                        bitstream,
                        &mut lo[si],
                        out2,
                        &lastdata[bx],
                        datatype,
                        start_band,
                        data_bands,
                        bs_data_pair,
                        ll_data.bs_quant_coarse_xxx[si] as i32,
                        (!(frame.bs_independency_flag != 0 && i == 0) || set_idx > 0) as i32,
                    )
                };
                if err != MPS_OK {
                    return err;
                }

                let src_row = data[bx][(si + bs_data_pair as usize).min(MAX_PARAMETER_SETS - 1)];
                let mask: i8 = if datatype == t_IPD {
                    if ll_data.bs_quant_coarse_xxx[si] != 0 {
                        7
                    } else {
                        15
                    }
                } else {
                    -1 // no mask
                };
                for pb in 0..data_bands.max(0) as usize {
                    let v = src_row[(start_band as usize + pb).min(MAX_PARAMETER_BANDS - 1)] & mask;
                    for j in band_range(a_strides[pb], a_strides[pb + 1]) {
                        lastdata[bx][j] = v;
                    }
                }

                old_quant_coarse_xxx = ll_data.bs_quant_coarse_xxx[si] as i32;

                if bs_data_pair != 0 {
                    ll_data.bs_quant_coarse_xxx[si + 1] = ll_data.bs_quant_coarse_xxx[si];
                    ll_data.bs_freq_res_stride_xxx[si + 1] = ll_data.bs_freq_res_stride_xxx[si];
                }
                set_idx += bs_data_pair + 1;
            } // !bsDataPair
        } // llData->bsXXXDataMode[i] == 3
    }

    ll_state.bs_quant_coarse_xxx_prev_parse = old_quant_coarse_xxx as i8;

    MPS_OK
}

/// `parseArbitraryDownmixData(self, pSSC, syntaxFlags, frame, bitstream)`
fn parse_arbitrary_downmix_data(
    self_: &mut SpatialDec,
    p_ssc: &SpatialSpecificConfig,
    syntax_flags: u32,
    frame: &mut SpatialBsFrame,
    ll_state: &mut BsLlState,
    bitstream: &mut FdkBitStream,
) -> SacdecError {
    let mut err = MPS_OK;
    let offset = p_ssc.n_ott_boxes;

    // CLD (arbitrary down-mix gains)
    for ch in 0..p_ssc.n_input_channels {
        let idx = (offset + ch) as usize;
        if idx >= MAX_NUM_PARAMETERS {
            // Deviation: guarded index of CLDLosslessData.
            return MPS_INVALID_BOXIDX;
        }
        let mut ll = frame.cld_lossless_data[idx];
        let mut data = frame.cmp_arbdmx_gain_idx;
        err = ec_data_dec(
            frame,
            syntax_flags,
            bitstream,
            &mut ll,
            &mut ll_state.cld_lossless_state[idx],
            &mut data,
            &mut self_.cmp_arbdmx_gain_idx_prev,
            t_CLD,
            ch,
            0,
            p_ssc.freq_res,
            ARBDMX_GAIN_DEFAULT,
        );
        frame.cld_lossless_data[idx] = ll;
        frame.cmp_arbdmx_gain_idx = data;
        if err != MPS_OK {
            return err;
        }
    }
    err
}

/// `nBitsParamSlot(i)`
pub fn n_bits_param_slot(i: i32) -> i32 {
    let mut bits_param_slot = f_max(0, DFRACT_BITS - 1 - f_normz(i));
    if (1i32.wrapping_shl(bits_param_slot as u32)) < i {
        bits_param_slot += 1;
    }
    fdk_assert!(bits_param_slot >= 0 && bits_param_slot <= 32);

    bits_param_slot
}

/// `SpatialDecParseFrameData(self, frame, bitstream, pSpatialSpecificConfig, upmixType,
/// fGlobalIndependencyFlag)`: parses one spatial frame. `ll_state` is the lossless state of
/// the MPEG Surround decoder (the `state` links of the lossless data of `frame`).
pub fn spatial_dec_parse_frame_data(
    self_: &mut SpatialDec,
    frame: &mut SpatialBsFrame,
    ll_state: &mut BsLlState,
    bitstream: &mut FdkBitStream,
    p_ssc: &SpatialSpecificConfig,
    _upmix_type: Upmixtype,
    f_global_independency_flag: i32,
) -> SacdecError {
    let mut err = MPS_OK;
    let bs_framing_type;
    let param_set_err;

    // CAUTION: Maybe different to pSpatialSpecificConfig->treeConfig in some modes!
    let bs_num_output_channels = tree_property(p_ssc.tree_config).num_output_channels as i32;

    // Anchor for ByteAlign() function. See comment below.
    let align_anchor = fdk_get_valid_bits(bitstream);

    let syntax_flags = p_ssc.syntax_flags;

    'bail: {
        if syntax_flags & (SACDEC_SYNTAX_USAC | SACDEC_SYNTAX_RSVD50) != 0
            && p_ssc.bs_high_rate_mode == 0
        {
            bs_framing_type = 0; // fixed framing
            frame.num_parameter_sets = 1;
        } else {
            bs_framing_type = fdk_read_bits(bitstream, 1);
            if syntax_flags & SACDEC_SYNTAX_LD != 0 {
                frame.num_parameter_sets = (fdk_read_bits(bitstream, 1) + 1) as u8;
            } else {
                frame.num_parameter_sets = (fdk_read_bits(bitstream, 3) + 1) as u8;
            }
        }

        // Any error after this line shall trigger parameter invalidation at bail label.
        param_set_err = 1;

        if frame.num_parameter_sets as usize >= MAX_PARAMETER_SETS {
            break 'bail;
        }

        // Basic config check.
        if p_ssc.n_input_channels <= 0 || p_ssc.n_output_channels <= 0 {
            err = MPS_UNSUPPORTED_CONFIG;
            break 'bail;
        }

        let nps = frame.num_parameter_sets as usize;
        if bs_framing_type != 0 {
            let mut prev_param_slot = -1;
            let bits_param_slot = n_bits_param_slot(p_ssc.n_time_slots);

            for i in 0..nps {
                frame.param_slot[i] = fdk_read_bits(bitstream, bits_param_slot as u32) as i32;
                // Sanity check
                if frame.param_slot[i] <= prev_param_slot
                    || frame.param_slot[i] >= p_ssc.n_time_slots
                {
                    err = MPS_PARSE_ERROR;
                    break 'bail;
                }
                prev_param_slot = frame.param_slot[i];
            }
        } else {
            for i in 0..nps {
                frame.param_slot[i] = ((p_ssc.n_time_slots * (i as i32 + 1)) / nps as i32) - 1;
            }
        }

        if syntax_flags & (SACDEC_SYNTAX_USAC | SACDEC_SYNTAX_RSVD50) != 0
            && f_global_independency_flag != 0
        {
            frame.bs_independency_flag = 1;
        } else {
            frame.bs_independency_flag = fdk_read_bits(bitstream, 1) as u8;
        }

        // OttData()
        for i in 0..p_ssc.n_ott_boxes.clamp(0, MAX_NUM_OTT as i32) as usize {
            let mut ll = frame.cld_lossless_data[i];
            let mut data = frame.cmp_ott_cld_idx;
            err = ec_data_dec(
                frame,
                syntax_flags,
                bitstream,
                &mut ll,
                &mut ll_state.cld_lossless_state[i],
                &mut data,
                &mut self_.cmp_ott_cld_idx_prev,
                t_CLD,
                i as i32,
                0,
                p_ssc.bitstream_ott_bands[i] as i32,
                p_ssc.ott_cld_default[i],
            );
            frame.cld_lossless_data[i] = ll;
            frame.cmp_ott_cld_idx = data;
            if err != MPS_OK {
                break 'bail;
            }
        } // i < numOttBoxes

        {
            for i in 0..p_ssc.n_ott_boxes.clamp(0, MAX_NUM_OTT as i32) as usize {
                let mut ll = frame.icc_lossless_data[i];
                let mut data = frame.cmp_ott_icc_idx;
                err = ec_data_dec(
                    frame,
                    syntax_flags,
                    bitstream,
                    &mut ll,
                    &mut ll_state.icc_lossless_state[i],
                    &mut data,
                    &mut self_.cmp_ott_icc_idx_prev,
                    t_ICC,
                    i as i32,
                    0,
                    p_ssc.bitstream_ott_bands[i] as i32,
                    ICC_DEFAULT,
                );
                frame.icc_lossless_data[i] = ll;
                frame.cmp_ott_icc_idx = data;
                if err != MPS_OK {
                    break 'bail;
                }
            } // i < numOttBoxes
        } // !oneICC

        if p_ssc.tree_config == SPATIALDEC_MODE_RSVD7 && p_ssc.bs_phase_coding != 0 {
            frame.phase_mode = fdk_read_bits(bitstream, 1) as i32;

            if frame.phase_mode == 0 {
                for pb in band_range(0, p_ssc.num_ott_bands_ipd as i32) {
                    self_.cmp_ott_ipd_idx_prev[0][pb] = 0;
                    for i in 0..nps {
                        frame.cmp_ott_ipd_idx[0][i][pb] = 0;
                    }
                }
                frame.opd_smoothing_mode = 0;
            } else {
                frame.opd_smoothing_mode = fdk_read_bits(bitstream, 1) as i32;
                let mut ll = frame.ipd_lossless_data[0];
                let mut data = frame.cmp_ott_ipd_idx;
                err = ec_data_dec(
                    frame,
                    syntax_flags,
                    bitstream,
                    &mut ll,
                    &mut ll_state.ipd_lossless_state[0],
                    &mut data,
                    &mut self_.cmp_ott_ipd_idx_prev,
                    t_IPD,
                    0,
                    0,
                    p_ssc.num_ott_bands_ipd as i32,
                    IPD_DEFAULT,
                );
                frame.ipd_lossless_data[0] = ll;
                frame.cmp_ott_ipd_idx = data;
                if err != MPS_OK {
                    break 'bail;
                }
            }
        }

        // SmgData()
        {
            if p_ssc.bs_high_rate_mode == 0 && syntax_flags & SACDEC_SYNTAX_USAC != 0 {
                for ps in 0..nps {
                    frame.bs_smooth_mode[ps] = 0;
                }
            } else {
                for ps in 0..nps {
                    frame.bs_smooth_mode[ps] = fdk_read_bits(bitstream, 2) as u8;
                    if frame.bs_smooth_mode[ps] >= 2 {
                        frame.bs_smooth_time[ps] = fdk_read_bits(bitstream, 2) as u8;
                    }
                    if frame.bs_smooth_mode[ps] == 3 {
                        frame.bs_freq_res_stride_smg[ps] = fdk_read_bits(bitstream, 2) as u8;
                        let data_bands = (p_ssc.freq_res - 1)
                            / pb_stride(frame.bs_freq_res_stride_smg[ps] as i32)
                            + 1;
                        for pg in 0..data_bands.clamp(0, MAX_PARAMETER_BANDS as i32) as usize {
                            frame.bs_smg_data[ps][pg] = fdk_read_bits(bitstream, 1) as u8;
                        }
                    }
                } // ps < numParameterSets
            }
        }

        // TempShapeData()
        if p_ssc.temp_shape_config == 3 && syntax_flags & SACDEC_SYNTAX_USAC != 0 {
            let tsd_err = tsd_read(bitstream, p_ssc.n_time_slots, &mut frame.tsd_data[0]);
            if tsd_err != 0 {
                err = MPS_PARSE_ERROR;
                break 'bail;
            }
        } else {
            frame.tsd_data[0].bs_tsd_enable = 0;
        }

        // bsNumOutputChannels is 2 for the 212 tree (deviation: entries beyond the arrays,
        // which have no defined result in C, are not written).
        for i in 0..(bs_num_output_channels.max(0) as usize).min(MAX_OUTPUT_CHANNELS) {
            frame.temp_shape_enable_channel_stp[i] = 0;
            frame.temp_shape_enable_channel_ges[i] = 0;
        }

        if p_ssc.temp_shape_config == 1 || p_ssc.temp_shape_config == 2 {
            let bs_temp_shape_enable = fdk_read_bits(bitstream, 1);
            if bs_temp_shape_enable != 0 {
                let num_temp_shape_chan = TEMP_SHAPE_CHAN_TABLE
                    [(p_ssc.temp_shape_config - 1) as usize]
                    .get(p_ssc.tree_config as usize)
                    .copied()
                    .unwrap_or(0) as usize;
                // numTempShapeChan is 2 for the 212 tree.
                let n = num_temp_shape_chan.min(MAX_OUTPUT_CHANNELS);
                match p_ssc.temp_shape_config {
                    1 => {
                        // STP
                        for i in 0..n {
                            let stp_enable = fdk_read_bits(bitstream, 1);
                            frame.temp_shape_enable_channel_stp[i] = stp_enable as u8;
                        }
                    }
                    2 => {
                        // GES
                        let mut ges_channel_enable = [0u8; MAX_OUTPUT_CHANNELS];

                        for i in 0..n {
                            ges_channel_enable[i] = fdk_read_bits(bitstream, 1) as u8;
                            frame.temp_shape_enable_channel_ges[i] = ges_channel_enable[i];
                        }
                        for i in 0..n {
                            if ges_channel_enable[i] != 0 {
                                let mut env_shape_data_tmp = [0i32; MAX_TIME_SLOTS as usize];
                                // nTimeSlots <= MAX_TIME_SLOTS (checked by the parsers).
                                if huff_dec_reshape(
                                    bitstream,
                                    &mut env_shape_data_tmp,
                                    p_ssc.n_time_slots.min(MAX_TIME_SLOTS),
                                ) != 0
                                {
                                    err = MPS_PARSE_ERROR;
                                    break 'bail;
                                }
                                for ts in 0..p_ssc.n_time_slots.clamp(0, MAX_TIME_SLOTS) as usize {
                                    // Quirk: `!(x >= 0) && (x <= 4)` rejects only negative
                                    // values.
                                    if env_shape_data_tmp[ts] < 0 {
                                        err = MPS_PARSE_ERROR;
                                        break 'bail;
                                    }
                                    frame.bs_env_shape_data[i][ts] = env_shape_data_tmp[ts] as u8;
                                }
                            }
                        }
                    }
                    _ => {
                        err = MPS_INVALID_TEMPSHAPE;
                        break 'bail;
                    }
                }
            } // bsTempShapeEnable
        } // pSpatialSpecificConfig->tempShapeConfig != 0

        if p_ssc.b_arbitrary_downmix != 0 {
            err = parse_arbitrary_downmix_data(
                self_,
                p_ssc,
                syntax_flags,
                frame,
                ll_state,
                bitstream,
            );
            if err != MPS_OK {
                break 'bail;
            }
        }

        if syntax_flags & SACDEC_SYNTAX_USAC == 0 {
            // ISO/IEC FDIS 23003-1: 5.2. ... byte alignment with respect to the beginning of
            // the syntactic element in which ByteAlign() occurs.
            fdk_byte_align(bitstream, align_anchor);
        }
    }

    if err != MPS_OK && param_set_err != 0 {
        // Since the parameter set data has already been written to the instance we need to ...
        frame.num_parameter_sets = 0; // ... signal that it is corrupt ...
    }

    err
}

/// `createMapping(aMap, startBand, stopBand, stride)`
pub fn create_mapping(
    a_map: &mut [i32; MAX_PARAMETER_BANDS + 1],
    start_band: i32,
    stop_band: i32,
    stride: i32,
) {
    let mut v_dk = [0i32; MAX_PARAMETER_BANDS + 1];

    let in_bands = stop_band - start_band;
    let mut out_bands = (in_bands - 1) / stride + 1;

    if out_bands < 1 {
        out_bands = 1;
    }
    // outBands <= MAX_PARAMETER_BANDS for the bands of the library.
    let ob = out_bands.min(MAX_PARAMETER_BANDS as i32);

    let bands_achived = out_bands * stride;
    let mut bands_diff = in_bands - bands_achived;
    for i in 0..ob as usize {
        v_dk[i] = stride;
    }

    let incr;
    let mut k;
    if bands_diff > 0 {
        incr = -1;
        k = ob - 1;
    } else {
        incr = 1;
        k = 0;
    }

    while bands_diff != 0 {
        if let Some(v) = v_dk.get_mut(k as usize) {
            *v -= incr;
        }
        k += incr;
        bands_diff += incr;
        if k >= ob {
            if bands_diff > 0 {
                k = ob - 1;
            } else if bands_diff < 0 {
                k = 0;
            }
        }
    }

    a_map[0] = start_band;
    for i in 0..ob as usize {
        a_map[i + 1] = a_map[i] + v_dk[i];
    }
}

/// `mapFrequency(pInput, pOutput, pMap, dataBands)`
pub fn map_frequency(
    p_input: &[i8; MAX_PARAMETER_BANDS],
    p_output: &mut [i8; MAX_PARAMETER_BANDS],
    p_map: &[i32; MAX_PARAMETER_BANDS + 1],
    data_bands: i32,
) {
    let start_band0 = p_map[0];

    for i in 0..data_bands.clamp(0, MAX_PARAMETER_BANDS as i32) as usize {
        let value = p_input[((i as i32 + start_band0) as usize).min(MAX_PARAMETER_BANDS - 1)];
        let start_band = p_map[i];
        let stop_band = p_map[i + 1];
        for j in band_range(start_band, stop_band) {
            p_output[j] = value;
        }
    }
}

/// `deqIdx(value, paramType)`
pub fn deq_idx(value: i32, param_type: i32) -> i32 {
    let mut idx = -1;

    match param_type {
        t_CLD => {
            // `(value + 15) >= 0 && (value + 15) < 31`: clang folds the signed (no-overflow)
            // range check to -15 <= value < 16, which the 64-bit sum reproduces for all values.
            if (value as i64 + 15) >= 0 && (value as i64 + 15) < 31 {
                idx = value + 15;
            }
        }
        t_ICC => {
            if value >= 0 && value < 8 {
                idx = value;
            }
        }
        t_IPD => {
            // (+/-)15 * MAX_PARAMETER_BANDS for differential coding in frequency domain
            // (according to rbl)
            if value >= -420 && value <= 420 {
                idx = value & 0xf;
            }
        }
        _ => {
            fdk_assert!(false);
        }
    }

    idx
}

/// `SF_IDX`
const SF_IDX: i32 = 7;
/// `SF_FACTOR`
const SF_FACTOR: i32 = 3;
/// `SCALE_FACTOR`
const SCALE_FACTOR: i32 = 1 << SF_FACTOR;
/// `SCALE_CLD_C1C2`
const SCALE_CLD_C1C2: i32 = 1 << SF_CLD_C1C2;

/// `factorFunct(ottVsTotDb, quantMode)`
pub fn factor_funct(mut ott_vs_tot_db: FixpDbl, quant_mode: i32) -> FixpDbl {
    let factor;

    if ott_vs_tot_db > 0 {
        ott_vs_tot_db = 0;
    }

    // `ottVsTotDb = -ottVsTotDb` overflows for MINVAL_DBL (undefined behavior). Both builds
    // compare the mathematical value (clang folds `-x >= c` into `x <= -c`), measured with the
    // cref test; the comparisons below use 64 bits. The products are only reached for values
    // between the thresholds, where the negation does not overflow.
    let neg = -(ott_vs_tot_db as i64);
    ott_vs_tot_db = ott_vs_tot_db.wrapping_neg();

    match quant_mode {
        0 => {
            factor = fl2fxconst_dbl((1.0f32 / SCALE_FACTOR as f32) as f64);
        }
        1 => {
            if neg >= fl2fxconst_dbl((21.0f32 / SCALE_CLD_C1C2 as f32) as f64) as i64 {
                factor = fl2fxconst_dbl((5.0f32 / SCALE_FACTOR as f32) as f64);
            } else if neg <= fl2fxconst_dbl((1.0f32 / SCALE_CLD_C1C2 as f32) as f64) as i64 {
                factor = fl2fxconst_dbl((1.0f32 / SCALE_FACTOR as f32) as f64);
            } else {
                factor = (f_mult(fl2fxconst_dbl(0.2f32 as f64), ott_vs_tot_db)
                    .wrapping_add(fl2fxconst_dbl((0.8f32 / SCALE_CLD_C1C2 as f32) as f64)))
                .wrapping_shl((SF_CLD_C1C2 - SF_FACTOR) as u32);
            }
        }
        2 => {
            if neg >= fl2fxconst_dbl((25.0f32 / SCALE_CLD_C1C2 as f32) as f64) as i64 {
                factor = MAXVAL_DBL; // avoid warning: FL2FXCONST_DBL(8.0f/SCALE_FACTOR)
            } else if neg <= fl2fxconst_dbl((1.0f32 / SCALE_CLD_C1C2 as f32) as f64) as i64 {
                factor = fl2fxconst_dbl((1.0f32 / SCALE_FACTOR as f32) as f64);
            } else {
                factor = (f_mult(fl2fxconst_dbl((7.0f32 / 24.0f32) as f64), ott_vs_tot_db)
                    .wrapping_add(fl2fxconst_dbl(
                        ((17.0f32 / 24.0f32) / SCALE_CLD_C1C2 as f32) as f64,
                    )))
                .wrapping_shl((SF_CLD_C1C2 - SF_FACTOR) as u32);
            }
        }
        _ => {
            factor = 0;
        }
    }

    factor
}

/// `factorCLD(idx, ottVsTotDb, ottVsTotDb1, ottVsTotDb2, ottVsTotDbMode, quantMode)`
pub fn factor_cld(
    idx: &mut i8,
    ott_vs_tot_db: FixpDbl,
    ott_vs_tot_db1: Option<&mut FixpDbl>,
    ott_vs_tot_db2: Option<&mut FixpDbl>,
    ott_vs_tot_db_mode: i8,
    quant_mode: i32,
) {
    let factor = factor_funct(ott_vs_tot_db, quant_mode);

    // (*idx) << ((DFRACT_BITS - 1) - SF_IDX): a left shift of a negative value, compiled as a
    // plain shift.
    let mut cld_idx_fract =
        f_mult((*idx as i32).wrapping_shl(((DFRACT_BITS - 1) - SF_IDX) as u32), factor);
    cld_idx_fract = cld_idx_fract
        .wrapping_add(fl2fxconst_dbl((15.5f32 / (1 << (SF_FACTOR + SF_IDX)) as f32) as f64));
    let mut cld_idx = fixp_truncate_to_int(cld_idx_fract, SF_FACTOR + SF_IDX);

    cld_idx = f_min(cld_idx, 30);
    cld_idx = f_max(cld_idx, 0);

    *idx = (cld_idx - 15) as i8;

    if ott_vs_tot_db_mode & OTT_VS_TOT_DB1_ACTIV != 0 {
        if let Some(d) = ott_vs_tot_db1 {
            *d = ott_vs_tot_db.wrapping_add(DEQUANT_CLD_C1[cld_idx as usize]);
        }
    }

    if ott_vs_tot_db_mode & OTT_VS_TOT_DB2_ACTIV != 0 {
        if let Some(d) = ott_vs_tot_db2 {
            *d = ott_vs_tot_db.wrapping_add(DEQUANT_CLD_C1[(30 - cld_idx) as usize]);
        }
    }
}

/// `FIXP_DBL (*)[MAX_PARAMETER_SETS][MAX_PARAMETER_BANDS]` of `mapIndexData()`.
pub(crate) type OttVsTotDb = [[FixpDbl; MAX_PARAMETER_BANDS]; MAX_PARAMETER_SETS];

/// `mapIndexData(llData, outputDataIdx, outputIdxData, cmpIdxData, diffIdxData, xttIdx,
/// idxPrev, paramIdx, paramType, startBand, stopBand, defaultValue, numParameterSets,
/// paramSlot, extendFrame, quantMode, concealmentInfo, ottVsTotDbMode, pOttVsTotDbIn,
/// pOttVsTotDb1, pOttVsTotDb2)`. `ll_state` is `llData->state`.
///
/// Deviation: an `xttIdx` outside of the arrays returns `MPS_WRONG_OTT`.
pub(crate) fn map_index_data(
    ll_data: &mut LosslessData,
    ll_state: &mut LosslessState,
    output_data_idx: &mut [ParamSets],
    output_idx_data: &mut [ParamSets],
    cmp_idx_data: &[ParamSets],
    diff_idx_data: Option<&mut [ParamSets]>,
    xtt_idx: i32,
    idx_prev: &mut [[i8; MAX_PARAMETER_BANDS]],
    _param_idx: i32,
    param_type: i32,
    start_band: i32,
    stop_band: i32,
    default_value: i8,
    num_parameter_sets: i32,
    param_slot: &[i32; MAX_PARAMETER_SETS],
    extend_frame: i32,
    quant_mode: i32,
    concealment_info: &SpatialDecConcealmentInfo,
    ott_vs_tot_db_mode: i8,
    p_ott_vs_tot_db_in: Option<&OttVsTotDb>,
    mut p_ott_vs_tot_db1: Option<&mut OttVsTotDb>,
    mut p_ott_vs_tot_db2: Option<&mut OttVsTotDb>,
) -> SacdecError {
    let mut a_param_slots = [0i32; MAX_PARAMETER_SETS];
    let mut a_interpolate = [0i32; MAX_PARAMETER_SETS];
    let mut a_map = [0i32; MAX_PARAMETER_BANDS + 1];

    if num_parameter_sets > MAX_PARAMETER_SETS as i32 {
        return MPS_WRONG_PARAMETERSETS;
    }
    let nps = num_parameter_sets.max(0) as usize;

    let x = xtt_idx as usize;
    if xtt_idx < 0
        || x >= output_data_idx.len()
        || x >= output_idx_data.len()
        || x >= cmp_idx_data.len()
        || x >= idx_prev.len()
    {
        return MPS_WRONG_OTT;
    }
    let mut diff_idx_data = diff_idx_data.and_then(|d| d.get_mut(x));

    let mut data_sets = 0usize;
    for i in 0..nps {
        if ll_data.bs_xxx_data_mode[i] == 3 {
            a_param_slots[data_sets] = i as i32;
            data_sets += 1;
        }
    }

    let mut set_idx = 0usize;

    // Main concealment stage is here:
    spatial_dec_concealment_apply(
        concealment_info,
        &cmp_idx_data[x],
        diff_idx_data.as_deref_mut().map(|d| &mut d[..]),
        &mut idx_prev[x],
        &mut ll_data.bs_xxx_data_mode,
        start_band,
        stop_band,
        default_value,
        param_type,
        num_parameter_sets,
    );

    let bands = band_range(start_band, stop_band);
    let out = &mut output_idx_data[x];

    // Prepare data
    for i in 0..nps {
        if ll_data.bs_xxx_data_mode[i] == 0 {
            ll_data.nocmp_quant_coarse_xxx[i] = 0;
            for band in bands.clone() {
                out[i][band] = default_value;
            }
            for band in bands.clone() {
                idx_prev[x][band] = out[i][band];
            }
            // Because the idxPrev are also set to the defaultValue -> signalize fine
            ll_state.bs_quant_coarse_xxx_prev = 0;
        }

        if ll_data.bs_xxx_data_mode[i] == 1 {
            for band in bands.clone() {
                out[i][band] = idx_prev[x][band];
            }
            ll_data.nocmp_quant_coarse_xxx[i] = ll_state.bs_quant_coarse_xxx_prev;
        }

        if ll_data.bs_xxx_data_mode[i] == 2 {
            for band in bands.clone() {
                out[i][band] = idx_prev[x][band];
            }
            ll_data.nocmp_quant_coarse_xxx[i] = ll_state.bs_quant_coarse_xxx_prev;
            a_interpolate[i] = 1;
        } else {
            a_interpolate[i] = 0;
        }

        if ll_data.bs_xxx_data_mode[i] == 3 {
            // setIdx < dataSets: the modes 3 counted above are still the modes 3 here (the
            // concealment replaces either all or none of them).
            let si = set_idx.min(MAX_PARAMETER_SETS - 1);
            let parm_slot = a_param_slots[si] as usize;
            let stride = pb_stride(ll_data.bs_freq_res_stride_xxx[si] as i32);
            let data_bands = (stop_band - start_band - 1) / stride + 1;
            create_mapping(&mut a_map, start_band, stop_band, stride);
            map_frequency(&cmp_idx_data[x][si], &mut out[parm_slot], &a_map, data_bands);

            for band in bands.clone() {
                idx_prev[x][band] = out[parm_slot][band];
            }

            ll_state.bs_quant_coarse_xxx_prev = ll_data.bs_quant_coarse_xxx[si];
            ll_data.nocmp_quant_coarse_xxx[i] = ll_data.bs_quant_coarse_xxx[si];

            set_idx += 1;
        }
        if let Some(diff) = diff_idx_data.as_deref() {
            for band in bands.clone() {
                out[i][band] = out[i][band].wrapping_add(diff[i][band]);
            }
        }
    } // for( i = 0 ; i < numParameterSets; i++ )

    // Map all coarse data to fine
    for i in 0..nps {
        if ll_data.nocmp_quant_coarse_xxx[i] == 1 {
            coarse2fine(&mut out[i], param_type, start_band, stop_band - start_band);
            ll_data.nocmp_quant_coarse_xxx[i] = 0;
        }
    }

    // Interpolate
    let mut i1 = 0usize;
    for i in 0..nps {
        if a_interpolate[i] != 1 {
            i1 = i;
        } else {
            let mut i2 = i;
            while i2 < nps {
                if a_interpolate[i2] != 1 {
                    break;
                }
                i2 += 1;
            }
            if i2 >= nps {
                return MPS_WRONG_PARAMETERSETS;
            }

            let x1 = param_slot[i1];
            let xi = param_slot[i];
            let x2 = param_slot[i2];

            for band in bands.clone() {
                let y1 = out[i1][band] as i32;
                let y2 = out[i2][band] as i32;
                let yi = if x1 != x2 {
                    y1.wrapping_add(
                        (xi.wrapping_sub(x1)).wrapping_mul(y2 - y1) / (x2.wrapping_sub(x1)),
                    )
                } else {
                    y1 // + (xi-x1)*(y2-y1)/1e-12
                };
                out[i][band] = yi as i8;
            }
        }
    } // for( i = 0 ; i < numParameterSets; i++ )

    // Dequantize data and apply factorCLD if necessary
    for ps in 0..nps {
        if quant_mode != 0 && param_type == t_CLD {
            let Some(db_in) = p_ott_vs_tot_db_in else {
                return MPS_WRONG_OTT;
            };
            if p_ott_vs_tot_db1.is_none() && (ott_vs_tot_db_mode & OTT_VS_TOT_DB1_ACTIV != 0) {
                return MPS_WRONG_OTT;
            }
            if p_ott_vs_tot_db2.is_none() && (ott_vs_tot_db_mode & OTT_VS_TOT_DB2_ACTIV != 0) {
                return MPS_WRONG_OTT;
            }
            for pb in bands.clone() {
                factor_cld(
                    &mut out[ps][pb],
                    db_in[ps][pb],
                    p_ott_vs_tot_db1.as_deref_mut().map(|d| &mut d[ps][pb]),
                    p_ott_vs_tot_db2.as_deref_mut().map(|d| &mut d[ps][pb]),
                    ott_vs_tot_db_mode,
                    quant_mode,
                );
            }
        }

        // Dequantize data
        for band in bands.clone() {
            let v = deq_idx(out[ps][band] as i32, param_type) as i8;
            output_data_idx[x][ps][band] = v;
            if v == -1 {
                output_data_idx[x][ps][band] = default_value;
            }
        }
    } // for( i = 0 ; i < numParameterSets; i++ )

    if extend_frame != 0 && nps >= 1 && nps < MAX_PARAMETER_SETS {
        if param_type == t_IPD {
            ll_data.bs_quant_coarse_xxx[nps] = ll_data.bs_quant_coarse_xxx[nps - 1];
        }
        for band in bands.clone() {
            output_data_idx[x][nps][band] = output_data_idx[x][nps - 1][band];
        }
    }

    MPS_OK
}

/// `decodeAndMapFrameOtt(self, pCurBs)`. Quirk: always returns `MPS_OK` (the errors of
/// `mapIndexData()` only end the decoding).
fn decode_and_map_frame_ott(
    self_: &mut SpatialDec,
    cfg: &SpatialSpecificConfig,
    p_cur_bs: &mut SpatialBsFrame,
    ll_state: &mut BsLlState,
) -> SacdecError {
    let num_ott_boxes = self_.num_ott_boxes;
    let nps = p_cur_bs.num_parameter_sets as i32;

    'bail: {
        {
            if self_.quant_mode != 0 {
                break 'bail;
            }
            for i in 0..num_ott_boxes.clamp(0, MAX_NUM_OTT as i32) as usize {
                let err = map_index_data(
                    &mut p_cur_bs.cld_lossless_data[i], // LOSSLESSDATA *llData
                    &mut ll_state.cld_lossless_state[i],
                    &mut self_.ott_cld__fdk,
                    &mut self_.out_idx_data,
                    &p_cur_bs.cmp_ott_cld_idx,
                    None,     // no differential data
                    i as i32, // Which ott/ttt index to use for input and output buffers
                    &mut self_.ott_cld_idx_prev,
                    i as i32,
                    t_CLD,
                    0,                                 // startBand
                    cfg.bitstream_ott_bands[i] as i32, // stopBand
                    cfg.ott_cld_default[i],            // defaultValue
                    nps,                               // numParameterSets
                    &p_cur_bs.param_slot,
                    self_.extend_frame as i32,
                    self_.quant_mode,
                    &self_.conceal_info,
                    OTT_VS_TOT_INACTIV,
                    None,
                    None,
                    None,
                );
                if err != MPS_OK {
                    break 'bail;
                }
            } // for(i = 0; i < numOttBoxes ; i++ )
        }

        for ott_idx in 0..num_ott_boxes.clamp(0, MAX_NUM_OTT as i32) as usize {
            // Read ICC
            let err = map_index_data(
                &mut p_cur_bs.icc_lossless_data[ott_idx],
                &mut ll_state.icc_lossless_state[ott_idx],
                &mut self_.ott_icc__fdk,
                &mut self_.out_idx_data,
                &p_cur_bs.cmp_ott_icc_idx,
                Some(&mut self_.ott_icc_diffidx), // differential data
                ott_idx as i32,
                &mut self_.ott_icc_idx_prev,
                ott_idx as i32,
                t_ICC,
                0,
                cfg.bitstream_ott_bands[ott_idx] as i32,
                ICC_DEFAULT,
                nps,
                &p_cur_bs.param_slot,
                self_.extend_frame as i32,
                self_.quant_mode,
                &self_.conceal_info,
                OTT_VS_TOT_INACTIV,
                None,
                None,
                None,
            );
            if err != MPS_OK {
                break 'bail;
            }
        } // ottIdx

        if self_.tree_config == TREE_212 && self_.phase_coding != 0 {
            if p_cur_bs.phase_mode == 0 {
                for pb in band_range(0, cfg.num_ott_bands_ipd as i32) {
                    self_.ott_ipd_idx_prev[0][pb] = 0;
                }
            }
            for ott_idx in 0..num_ott_boxes.clamp(0, MAX_NUM_OTT as i32) as usize {
                let _err = map_index_data(
                    &mut p_cur_bs.ipd_lossless_data[ott_idx],
                    &mut ll_state.ipd_lossless_state[ott_idx],
                    &mut self_.ott_ipd__fdk,
                    &mut self_.out_idx_data,
                    &p_cur_bs.cmp_ott_ipd_idx,
                    None,
                    ott_idx as i32,
                    &mut self_.ott_ipd_idx_prev,
                    ott_idx as i32,
                    t_IPD,
                    0,
                    self_.num_ott_bands_ipd,
                    IPD_DEFAULT,
                    nps,
                    &p_cur_bs.param_slot,
                    self_.extend_frame as i32,
                    self_.quant_mode,
                    &self_.conceal_info,
                    OTT_VS_TOT_INACTIV,
                    None,
                    None,
                    None,
                );
            }
        }
    }

    MPS_OK
}

/// `decodeAndMapFrameSmg(self, frame)`: decodes the smoothing flags.
fn decode_and_map_frame_smg(self_: &mut SpatialDec, frame: &SpatialBsFrame) -> SacdecError {
    let mut a_group_to_band = [0i32; MAX_PARAMETER_BANDS + 1];

    if frame.num_parameter_sets as usize > MAX_PARAMETER_SETS {
        return MPS_WRONG_PARAMETERSETS;
    }
    if self_.bitstream_parameter_bands > MAX_PARAMETER_BANDS as i32 {
        return MPS_WRONG_PARAMETERBANDS;
    }
    // A negative number of bands copies or clears nothing (deviation).
    let nb = self_.bitstream_parameter_bands.max(0) as usize;
    let nps = frame.num_parameter_sets as usize;

    for ps in 0..nps {
        match frame.bs_smooth_mode[ps] {
            0 => {
                self_.smg_time[ps] = 256;
                self_.smg_data[ps][..nb].fill(0);
            }
            1 => {
                if ps > 0 {
                    self_.smg_time[ps] = self_.smg_time[ps - 1];
                    let src = self_.smg_data[ps - 1];
                    self_.smg_data[ps][..nb].copy_from_slice(&src[..nb]);
                } else {
                    self_.smg_time[ps] = self_.smooth_state.prev_smg_time;
                    let src = self_.smooth_state.prev_smg_data;
                    self_.smg_data[ps][..nb].copy_from_slice(&src[..nb]);
                }
            }
            2 => {
                self_.smg_time[ps] = SMG_TIME_TABLE[(frame.bs_smooth_time[ps] & 3) as usize];
                for pb in 0..nb {
                    self_.smg_data[ps][pb] = 1;
                }
            }
            3 => {
                self_.smg_time[ps] = SMG_TIME_TABLE[(frame.bs_smooth_time[ps] & 3) as usize];
                let pb_stride = pb_stride(frame.bs_freq_res_stride_smg[ps] as i32);
                let data_bands = (self_.bitstream_parameter_bands - 1) / pb_stride + 1;
                create_mapping(&mut a_group_to_band, 0, self_.bitstream_parameter_bands, pb_stride);
                for pg in 0..data_bands.clamp(0, MAX_PARAMETER_BANDS as i32) as usize {
                    let pb_start = a_group_to_band[pg];
                    let pb_stop = a_group_to_band[pg + 1];
                    for pb in band_range(pb_start, pb_stop) {
                        self_.smg_data[ps][pb] = frame.bs_smg_data[ps][pg];
                    }
                }
            }
            _ => {}
        }
    }

    // numParameterSets >= 1 here (see the documentation of the port; deviation: 0 is treated
    // as 1).
    let last = nps.max(1) - 1;
    self_.smooth_state.prev_smg_time = self_.smg_time[last];
    let src = self_.smg_data[last];
    self_.smooth_state.prev_smg_data[..nb].copy_from_slice(&src[..nb]);

    if self_.extend_frame != 0 && nps < MAX_PARAMETER_SETS {
        self_.smg_time[nps] = self_.smg_time[last];
        let src = self_.smg_data[last];
        self_.smg_data[nps][..nb].copy_from_slice(&src[..nb]);
    }

    MPS_OK
}

/// `decodeAndMapFrameArbdmx(self, frame)`
fn decode_and_map_frame_arbdmx(
    self_: &mut SpatialDec,
    frame: &mut SpatialBsFrame,
    ll_state: &mut BsLlState,
) -> SacdecError {
    let mut err = MPS_OK;
    let offset = self_.num_ott_boxes;

    for ch in 0..self_.num_input_channels {
        let idx = (offset + ch) as usize;
        if idx >= MAX_NUM_PARAMETERS {
            return MPS_WRONG_OTT;
        }
        err = map_index_data(
            &mut frame.cld_lossless_data[idx],
            &mut ll_state.cld_lossless_state[idx],
            &mut self_.arbdmx_gain__fdk,
            &mut self_.out_idx_data,
            &frame.cmp_arbdmx_gain_idx,
            None, // no differential data
            ch,
            &mut self_.arbdmx_gain_idx_prev,
            offset + ch,
            t_CLD,
            0,
            self_.bitstream_parameter_bands,
            0, // self->arbdmxGainDefault
            frame.num_parameter_sets as i32,
            &frame.param_slot,
            self_.extend_frame as i32,
            0,
            &self_.conceal_info,
            OTT_VS_TOT_INACTIV,
            None,
            None,
            None,
        );
        if err != MPS_OK {
            return err;
        }
    }

    err
}

/// `SpatialDecDecodeFrame(self, frame)`: decodes and maps the parameters of the frame. `cfg`
/// is `self->pConfigCurrent`.
pub fn spatial_dec_decode_frame(
    self_: &mut SpatialDec,
    cfg: &SpatialSpecificConfig,
    frame: &mut SpatialBsFrame,
    ll_state: &mut BsLlState,
) -> SacdecError {
    let mut err;

    self_.extend_frame = 0;
    // numParameterSets >= 1 here (deviation: 0 reads paramSlot[0]).
    let last = (frame.num_parameter_sets as usize).max(1) - 1;
    if frame.param_slot[last.min(MAX_PARAMETER_SETS - 1)] != self_.time_slots - 1 {
        self_.extend_frame = 1;
    }

    self_.tsd_ts = 0;

    // DTDF and MAP DATA
    err = decode_and_map_frame_ott(self_, cfg, frame, ll_state);
    if err != MPS_OK {
        return err;
    }
    err = decode_and_map_frame_smg(self_, frame);
    if err != MPS_OK {
        return err;
    }
    if self_.arbitrary_downmix != 0 {
        err = decode_and_map_frame_arbdmx(self_, frame, ll_state);
        if err != MPS_OK {
            return err;
        }
    }

    if self_.extend_frame != 0 {
        frame.num_parameter_sets =
            fix_min(MAX_PARAMETER_SETS as i32, frame.num_parameter_sets as i32 + 1) as u8;
        let n = frame.num_parameter_sets as usize;
        frame.param_slot[n - 1] = self_.time_slots - 1;

        for p in 0..n {
            if frame.param_slot[p] > self_.time_slots - 1 {
                frame.param_slot[p] = self_.time_slots - 1;
                err = MPS_PARSE_ERROR;
            }
        }
        if err != MPS_OK {
            return err;
        }
    }

    err
}

/// The kernel table of a number of parameter bands (`None`: unsupported).
fn kernels_of(ld: bool, num_parameter_bands: i32) -> Option<&'static [i8; MAX_HYBRID_BANDS]> {
    if ld {
        match num_parameter_bands {
            4 => Some(&KERNELS_4_TO_64),
            5 => Some(&KERNELS_5_TO_64),
            7 => Some(&KERNELS_7_TO_64),
            9 => Some(&KERNELS_9_TO_64),
            12 => Some(&KERNELS_12_TO_64),
            15 => Some(&KERNELS_15_TO_64),
            23 => Some(&KERNELS_23_TO_64),
            _ => None,
        }
    } else {
        match num_parameter_bands {
            4 => Some(&KERNELS_4_TO_71),
            5 => Some(&KERNELS_5_TO_71),
            7 => Some(&KERNELS_7_TO_71),
            10 => Some(&KERNELS_10_TO_71),
            14 => Some(&KERNELS_14_TO_71),
            20 => Some(&KERNELS_20_TO_71),
            28 => Some(&KERNELS_28_TO_71),
            _ => None,
        }
    }
}

/// The kernel tables `kernels_{4,5,7,10,14,20,28}_to_71`, `kernels_{4,5,7,9,12,15,23}_to_64`
/// (the identification of a table in the tests).
fn all_kernel_tables() -> [&'static [i8; MAX_HYBRID_BANDS]; 14] {
    [
        &KERNELS_4_TO_71,
        &KERNELS_5_TO_71,
        &KERNELS_7_TO_71,
        &KERNELS_10_TO_71,
        &KERNELS_14_TO_71,
        &KERNELS_20_TO_71,
        &KERNELS_28_TO_71,
        &KERNELS_4_TO_64,
        &KERNELS_5_TO_64,
        &KERNELS_7_TO_64,
        &KERNELS_9_TO_64,
        &KERNELS_12_TO_64,
        &KERNELS_15_TO_64,
        &KERNELS_23_TO_64,
    ]
}

/// The index of a kernel table in `all_kernel_tables` (0..=13; -1 for `None`). Used by the
/// tests.
pub fn kernels_table_index(k: Option<&'static [i8; MAX_HYBRID_BANDS]>) -> i32 {
    match k {
        None => -1,
        Some(k) => {
            all_kernel_tables().iter().position(|t| core::ptr::eq(*t, k)).map_or(-2, |i| i as i32)
        }
    }
}

/// The kernel table with the index of [`kernels_table_index`].
pub fn kernels_table(i: i32) -> Option<&'static [i8; MAX_HYBRID_BANDS]> {
    all_kernel_tables().get(usize::try_from(i).ok()?).copied()
}

/// `SpatialDecDecodeHeader(self, pSpatialSpecificConfig)`: sets up the decoder from the
/// configuration. `p_ssc` is also `self->pConfigCurrent` (see the
/// [`crate::sac_dec`] module documentation).
pub fn spatial_dec_decode_header(
    self_: &mut SpatialDec,
    p_ssc: &mut SpatialSpecificConfig,
) -> SacdecError {
    let err = MPS_OK;

    self_.sampling_freq = p_ssc.sampling_freq;
    self_.time_slots = p_ssc.n_time_slots;
    self_.frame_length = self_.time_slots.wrapping_mul(self_.qmf_bands);
    self_.bitstream_parameter_bands = p_ssc.freq_res;

    let ld = p_ssc.syntax_flags & SACDEC_SYNTAX_LD != 0;
    if ld {
        self_.hybrid_bands = self_.qmf_bands;
    } else {
        self_.hybrid_bands = sac_get_hybrid_subbands(self_.qmf_bands);
    }
    self_.tp_hyb_band_border = 12;

    self_.num_parameter_bands = self_.bitstream_parameter_bands;

    match kernels_of(ld, self_.num_parameter_bands) {
        Some(k) => self_.kernels = Some(k),
        None => return MPS_INVALID_PARAMETERBANDS, // unsupported numParameterBands
    }
    let kernels = self_.kernels.unwrap_or(&KERNELS_4_TO_71);

    // create param to hyb band table
    self_.param2hyb = [0; MAX_PARAMETER_BANDS + 1];
    // hybridBands <= 71 (qmfBands <= 64) for the configurations of the library.
    let hb = self_.hybrid_bands.clamp(0, MAX_HYBRID_BANDS as i32) as usize;
    let mut i = 0usize;
    while i < hb {
        self_.param2hyb[kernels[i] as usize + 1] = i as i32 + 1;
        i += 1;
    }
    {
        // kernels[i - 1] with i = hybridBands >= 1 (deviation: 0 bands read kernels[0]).
        let mut pb = kernels[i.max(1) - 1] as usize + 2;
        while pb < MAX_PARAMETER_BANDS + 1 {
            self_.param2hyb[pb] = i as i32;
            pb += 1;
        }
        for pb in 0..MAX_PARAMETER_BANDS {
            self_.kernels_width[pb] = self_.param2hyb[pb + 1] - self_.param2hyb[pb];
        }
    }

    self_.tree_config = p_ssc.tree_config;

    self_.num_ott_boxes = p_ssc.n_ott_boxes;

    self_.num_input_channels = p_ssc.n_input_channels;

    self_.num_output_channels = p_ssc.n_output_channels;

    self_.quant_mode = p_ssc.quant_mode;

    self_.arbitrary_downmix = p_ssc.b_arbitrary_downmix;

    self_.num_m2rows = self_.num_output_channels;

    {
        self_.residual_coding = 0;
        if self_.arbitrary_downmix == 2 {
            self_.arbitrary_downmix = 1; // no arbitrary downmix residuals
        }
    }
    if p_ssc.syntax_flags & SACDEC_SYNTAX_USAC != 0 {
        self_.residual_coding = p_ssc.b_residual_coding;
    }

    // bsFixedGainDMX is a 3 bit value.
    let g = (p_ssc.bs_fixed_gain_dmx & 7) as usize;
    self_.clip_protect_gain__fdk = fx_cfg2fx_dbl(CLIP_GAIN_TABLE__FDK[g]);
    self_.clip_protect_gain_sf__fdk = CLIP_GAIN_SF_TABLE__FDK[g] as i8;

    self_.temp_shape_config = p_ssc.temp_shape_config;

    self_.decorr_config = p_ssc.decorr_config;

    if self_.upmix_type == UPMIXTYPE_BYPASS {
        self_.num_output_channels = self_.num_input_channels;
    }

    self_.num_output_channels_at = self_.num_output_channels;

    self_.num_ott_bands_ipd = p_ssc.num_ott_bands_ipd as i32;

    self_.phase_coding = p_ssc.bs_phase_coding as i32;
    for i in 0..(self_.num_ott_boxes.max(0) as usize).min(NUM_OTT) {
        {
            p_ssc.bitstream_ott_bands[i] = self_.bitstream_parameter_bands as u8;
        }
        self_.num_ott_bands[i] = p_ssc.bitstream_ott_bands[i] as i32;
    } // i

    if self_.residual_coding != 0 {
        let num_boxes = (self_.num_ott_boxes.max(0) as usize).min(MAX_NUM_OTT);

        for i in 0..num_boxes {
            self_.residual_present[i] = p_ssc.residual_config[i].b_residual_present as u8;

            if self_.residual_present[i] != 0 {
                self_.residual_bands[i] = p_ssc.residual_config[i].n_residual_bands as u8;
                // conversion from hybrid bands to qmf bands
                self_.residual_qmf_bands[i] = f_max(
                    self_.param2hyb[(self_.residual_bands[i] as usize).min(MAX_PARAMETER_BANDS)]
                        + 3
                        - 10,
                    3,
                ) as u8; // simplification for the lowest 10 hybrid bands
            } else {
                self_.residual_bands[i] = 0;
                self_.residual_qmf_bands[i] = 0;
            }
        }
    } else {
        let boxes = (self_.num_ott_boxes.max(0) as usize).min(MAX_NUM_OTT);
        for i in 0..boxes {
            self_.residual_present[i] = 0;
            self_.residual_bands[i] = 0;
        }
    }

    match self_.tree_config {
        TREE_212 => {
            self_.num_direkt_signals = 1;
            self_.num_decor_signals = 1;
            self_.num_x_channels = 1;
            if self_.arbitrary_downmix == 2 {
                self_.num_x_channels += 1;
            }
            self_.num_v_channels = self_.num_direkt_signals + self_.num_decor_signals;
        }
        _ => return MPS_INVALID_TREECONFIG,
    }

    self_.high_rate_mode = p_ssc.bs_high_rate_mode as i32;
    self_.decorr_type = p_ssc.bs_decorr_type as i32;

    spatial_dec_decode_helper_info(p_ssc, UPMIXTYPE_NORMAL);

    err
}

/// `SpatialDecCreateBsFrame(bsFrame, llState)`: initializes a spatial bitstream structure (the
/// C function allocates its arrays and links the lossless states; see the
/// [`crate::sac_dec`] module documentation).
pub fn spatial_dec_create_bs_frame(bs_frame: &mut SpatialBsFrame) -> SacdecError {
    bs_frame.new_bs_data = 0;
    bs_frame.num_parameter_sets = 1;
    MPS_OK
}

/// `SpatialDecCloseBsFrame(bsFrame)`: frees the arrays of the structure in C; nothing to do in
/// the port.
pub fn spatial_dec_close_bs_frame(_bs_frame: Option<&mut SpatialBsFrame>) {}
