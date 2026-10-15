// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/aacdec_drc.h, libAACdec/src/aacdec_drc.cpp.

//! Port of `aacdec_drc.h`: the dynamic range control (DRC) decoder tool of the AAC decoder:
//! MPEG-4 `dynamic_range_info()` (ISO/IEC 14496-3, 4.5.2.7) and DVB heavy compression
//! (ETSI TS 101 154), the DRC parameter and presentation mode handling, and the level
//! normalization gain of the output signal.
//!
//! `HANDLE_AAC_DRC` is `&mut CDrcInfo` (`Option<...>` where the C code checks for `NULL`).
//! `pAacDecoderStaticChannelInfo[]` is the decoder's array of static channel infos; only their
//! `drcData` is used. The SBR decoder of `aacDecoder_drcApply()` is `None` for `NULL`.
//!
//! The `.ii` files of the `aarch64` and `x86` builds are identical for `aacdec_drc.cpp` apart
//! from the shared header primitives.

use fdk_aac_rs_fdk::bitstream::{
    fdk_get_valid_bits, fdk_push_bi_directional, fdk_read_bits, FdkBitStream,
};
use fdk_aac_rs_fdk::common_fix::{
    cnt_leading_zeros, f_mult, f_mult_div2, fl2fxconst_dbl, FixpDbl, DFRACT_BITS,
};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::fixminmax::{fixmax, fixmin};
use fdk_aac_rs_fdk::fixpoint_math::{
    f2_pow, f_div_norm_noexp, f_ld_pow, f_log2, f_mult_norm, f_pow_int,
};
use fdk_aac_rs_fdk::mdct::BLOCK_SHORT;
use fdk_aac_rs_fdk::scale::{scale_value, scale_values_saturate};

use crate::aac_rom::PcmDec;
pub use crate::aacdec_drc_types::*;

pub mod tables;
use crate::channelinfo::{
    AacDecoderError, CAacDecoderChannelInfo, CAacDecoderStaticChannelInfo, AAC_DEC_INVALID_HANDLE,
    AAC_DEC_OK, AAC_DEC_SET_PARAM_FAIL,
};
use fdk_aac_rs_sbrdec::sbrdecoder::{
    sbr_decoder_drc_disable, sbr_decoder_drc_feed_channel, SbrDecoderInstance,
};
use tables::{SMOOTH_A, SMOOTH_B};

// ---------------------------------------------------------------------------------------------
// aacdec_drc.h
// ---------------------------------------------------------------------------------------------

/// `AACDEC_DRC_GAIN_SCALING`: scaling of DRC gains
pub const AACDEC_DRC_GAIN_SCALING: i32 = 11;
/// `AACDEC_DRC_GAIN_INIT_VALUE`: init value for DRC gains (`FL2FXCONST_DBL(1.0f / (1 <<
/// AACDEC_DRC_GAIN_SCALING))`)
pub const AACDEC_DRC_GAIN_INIT_VALUE: FixpDbl =
    fl2fxconst_dbl((1.0f32 / (1 << AACDEC_DRC_GAIN_SCALING) as f32) as f64);

/// `AACDEC_DRC_DFLT_EXPIRY_FRAMES`: default DRC data expiry time in AAC frames
pub const AACDEC_DRC_DFLT_EXPIRY_FRAMES: u32 = 0;

/// `AACDEC_DRC_PARAM`: DRC module setting parameters
pub type AacdecDrcParam = i32;
/// `DRC_CUT_SCALE`
pub const DRC_CUT_SCALE: AacdecDrcParam = 0;
/// `DRC_BOOST_SCALE`
pub const DRC_BOOST_SCALE: AacdecDrcParam = 1;
/// `TARGET_REF_LEVEL`
pub const TARGET_REF_LEVEL: AacdecDrcParam = 2;
/// `DRC_BS_DELAY`
pub const DRC_BS_DELAY: AacdecDrcParam = 3;
/// `DRC_DATA_EXPIRY_FRAME`
pub const DRC_DATA_EXPIRY_FRAME: AacdecDrcParam = 4;
/// `APPLY_HEAVY_COMPRESSION`
pub const APPLY_HEAVY_COMPRESSION: AacdecDrcParam = 5;
/// `DEFAULT_PRESENTATION_MODE`
pub const DEFAULT_PRESENTATION_MODE: AacdecDrcParam = 6;
/// `ENCODER_TARGET_LEVEL`
pub const ENCODER_TARGET_LEVEL: AacdecDrcParam = 7;
/// `MAX_OUTPUT_CHANNELS`
pub const MAX_OUTPUT_CHANNELS: AacdecDrcParam = 8;

// ---------------------------------------------------------------------------------------------
// aacdec_drc.cpp
// ---------------------------------------------------------------------------------------------

/* For parameter conversion */
/// `DRC_PARAMETER_BITS`
const DRC_PARAMETER_BITS: i32 = 7;
/// `DRC_MAX_QUANT_STEPS`
const DRC_MAX_QUANT_STEPS: i32 = 1 << DRC_PARAMETER_BITS;
/// `DRC_MAX_QUANT_FACTOR`
const DRC_MAX_QUANT_FACTOR: i32 = DRC_MAX_QUANT_STEPS - 1;
/// `DRC_PARAM_QUANT_STEP`: `FL2FXCONST_DBL(1.0f / (float)DRC_MAX_QUANT_FACTOR)` (a `float`
/// quotient)
const DRC_PARAM_QUANT_STEP: FixpDbl = fl2fxconst_dbl((1.0f32 / DRC_MAX_QUANT_FACTOR as f32) as f64);
/// `DRC_PARAM_SCALE`
const DRC_PARAM_SCALE: i32 = 1;
/// `DRC_SCALING_MAX`
const DRC_SCALING_MAX: FixpDbl = (DRC_PARAM_QUANT_STEP >> DRC_PARAM_SCALE) * 127;

/// `DRC_BLOCK_LEN`
const DRC_BLOCK_LEN: i32 = 1024;
/// `DRC_BAND_MULT`
const DRC_BAND_MULT: i32 = 4;
/// `DRC_BLOCK_LEN_DIV_BAND_MULT`
const DRC_BLOCK_LEN_DIV_BAND_MULT: i32 = DRC_BLOCK_LEN / DRC_BAND_MULT;

/// `MAX_REFERENCE_LEVEL`
const MAX_REFERENCE_LEVEL: i32 = 127;

/// `DRC_HEAVY_THRESHOLD_DB`
const DRC_HEAVY_THRESHOLD_DB: i32 = 10;

/// `DVB_ANC_DATA_SYNC_BYTE`: DVB ancillary data sync byte.
const DVB_ANC_DATA_SYNC_BYTE: u32 = 0xBC;

/// `DVB_COMPRESSION_SCALE`: 48,164 dB
const DVB_COMPRESSION_SCALE: i32 = 8;

/// `AAC_DRC_PARAMETER_HANDLING_DISABLED` of `aacdecoder_lib.h`
const AAC_DRC_PARAMETER_HANDLING_DISABLED: i32 = -1;
/// `AAC_DRC_PRESENTATION_MODE_2_DEFAULT` of `aacdecoder_lib.h`
const AAC_DRC_PRESENTATION_MODE_2_DEFAULT: i32 = 2;

const OFF: u8 = 0;
const ON: u8 = 1;

/// `convert_drcParam(param_dbl)` (static in C): converts an internal DRC boost/cut scaling
/// factor in `FIXP_DBL` (which is downscaled by `DRC_PARAM_SCALE`) back to an integer value
/// between 0 and 127.
pub fn convert_drc_param(param_dbl: FixpDbl) -> i32 {
    let mut param_long: i64 = (param_dbl >> 7) as i64;
    // LONG is 32 bits on the reference builds (INT); the product of a value below 2^24 with 127
    // fits.
    param_long = (param_long as i32).wrapping_mul(DRC_MAX_QUANT_FACTOR) as i64;
    param_long >>= 31 - 7 - DRC_PARAM_SCALE - 1;
    param_long += 1; /* for rounding */
    param_long >>= 1;

    param_long as i32
}

/// `aacDecoder_drcDisable(self)`: disable DRC.
pub fn aac_decoder_drc_disable(self_: &mut CDrcInfo) {
    self_.enable = 0;
    self_.apply_ext_gain = 0;
    self_.prog_ref_level_present = 0;
}

/// `aacDecoder_drcReset(self)`: reset DRC information.
pub fn aac_decoder_drc_reset(self_: &mut CDrcInfo) {
    self_.apply_ext_gain = 0;
    self_.additional_gain_prev = AACDEC_DRC_GAIN_INIT_VALUE;
    self_.additional_gain_filter_state = AACDEC_DRC_GAIN_INIT_VALUE;
    self_.additional_gain_filter_state1 = AACDEC_DRC_GAIN_INIT_VALUE;
}

/// `aacDecoder_drcInit(self)`: initialize DRC information.
pub fn aac_decoder_drc_init(self_: Option<&mut CDrcInfo>) {
    let Some(self_) = self_ else {
        return;
    };

    /* init control fields */
    self_.enable = OFF;
    self_.num_threads = 0;

    /* init params */
    let p_params = &mut self_.params;
    p_params.bs_delay_enable = 0;
    p_params.cut = 0;
    p_params.usr_cut = 0;
    p_params.boost = 0;
    p_params.usr_boost = 0;
    p_params.target_ref_level = 96;
    p_params.expiry_frame = AACDEC_DRC_DFLT_EXPIRY_FRAMES;
    p_params.apply_heavy_compression = OFF;
    p_params.usr_apply_heavy_compression = OFF;

    p_params.default_presentation_mode = DISABLED_PARAMETER_HANDLING;
    p_params.encoder_target_level = MAX_REFERENCE_LEVEL as u8; /* worst case assumption */

    self_.update = 1;
    self_.num_out_channels = 0;
    self_.prev_aac_num_channels = 0;

    /* initial program ref level = target ref level */
    self_.prog_ref_level = self_.params.target_ref_level;
    self_.prog_ref_level_present = 0;
    self_.pres_mode = -1;

    aac_decoder_drc_reset(self_);
}

/// `aacDecoder_drcInitChannelData(pDrcChData)`: initialize DRC control data for one channel.
pub fn aac_decoder_drc_init_channel_data(p_drc_ch_data: Option<&mut CDrcChannelData>) {
    if let Some(p) = p_drc_ch_data {
        p.expiry_count = 0;
        p.num_bands = 1;
        p.band_top[0] = (DRC_BLOCK_LEN_DIV_BAND_MULT - 1) as u16;
        p.drc_value[0] = 0;
        p.drc_interpolation_scheme = 0;
        p.drc_data_type = UNKNOWN_PAYLOAD as i8;
    }
}

/// `aacDecoder_drcSetParam(self, param, value)`: set one single DRC parameter.
pub fn aac_decoder_drc_set_param(
    self_: Option<&mut CDrcInfo>,
    param: AacdecDrcParam,
    value: i32,
) -> AacDecoderError {
    let error_status = AAC_DEC_OK;

    match param {
        DRC_CUT_SCALE => {
            /* set attenuation scale factor */
            if !(0..=DRC_MAX_QUANT_FACTOR).contains(&value) {
                return AAC_DEC_SET_PARAM_FAIL;
            }
            let Some(s) = self_ else {
                return AAC_DEC_INVALID_HANDLE;
            };
            s.params.usr_cut = (DRC_PARAM_QUANT_STEP >> DRC_PARAM_SCALE).wrapping_mul(value);
            s.update = 1;
        }
        DRC_BOOST_SCALE => {
            /* set boost factor */
            if !(0..=DRC_MAX_QUANT_FACTOR).contains(&value) {
                return AAC_DEC_SET_PARAM_FAIL;
            }
            let Some(s) = self_ else {
                return AAC_DEC_INVALID_HANDLE;
            };
            s.params.usr_boost = (DRC_PARAM_QUANT_STEP >> DRC_PARAM_SCALE).wrapping_mul(value);
            s.update = 1;
        }
        TARGET_REF_LEVEL => {
            if value > MAX_REFERENCE_LEVEL || value < -MAX_REFERENCE_LEVEL {
                return AAC_DEC_SET_PARAM_FAIL;
            }
            let Some(s) = self_ else {
                return AAC_DEC_INVALID_HANDLE;
            };
            if value < 0 {
                s.params.target_ref_level = -1;
            } else {
                if s.params.target_ref_level != value as i8 {
                    s.params.target_ref_level = value as i8;
                    s.prog_ref_level = value as i8; /* Always set the program reference level
                                                    equal to the target level according to
                                                    4.5.2.7.3 of ISO/IEC 14496-3. */
                }
                s.update = 1;
            }
        }
        APPLY_HEAVY_COMPRESSION => {
            if value != OFF as i32 && value != ON as i32 {
                return AAC_DEC_SET_PARAM_FAIL;
            }
            let Some(s) = self_ else {
                return AAC_DEC_INVALID_HANDLE;
            };
            /* Store new parameter value */
            s.params.usr_apply_heavy_compression = value as u8;
            s.update = 1;
        }
        DEFAULT_PRESENTATION_MODE => {
            if !(AAC_DRC_PARAMETER_HANDLING_DISABLED..=AAC_DRC_PRESENTATION_MODE_2_DEFAULT)
                .contains(&value)
            {
                return AAC_DEC_SET_PARAM_FAIL;
            }
            let Some(s) = self_ else {
                return AAC_DEC_INVALID_HANDLE;
            };
            s.params.default_presentation_mode = value;
            s.update = 1;
        }
        ENCODER_TARGET_LEVEL => {
            if !(0..=MAX_REFERENCE_LEVEL).contains(&value) {
                return AAC_DEC_SET_PARAM_FAIL;
            }
            let Some(s) = self_ else {
                return AAC_DEC_INVALID_HANDLE;
            };
            s.params.encoder_target_level = value as u8;
            s.update = 1;
        }
        DRC_BS_DELAY => {
            if !(0..=1).contains(&value) {
                return AAC_DEC_SET_PARAM_FAIL;
            }
            let Some(s) = self_ else {
                return AAC_DEC_INVALID_HANDLE;
            };
            s.params.bs_delay_enable = value as u8;
        }
        DRC_DATA_EXPIRY_FRAME => {
            let Some(s) = self_ else {
                return AAC_DEC_INVALID_HANDLE;
            };
            s.params.expiry_frame = if value > 0 { value as u32 } else { 0 };
        }
        MAX_OUTPUT_CHANNELS => {
            let Some(s) = self_ else {
                return AAC_DEC_INVALID_HANDLE;
            };
            s.num_out_channels = value;
            s.update = 1;
        }
        _ => return AAC_DEC_SET_PARAM_FAIL,
    }

    error_status
}

/// `parseExcludedChannels(excludedChnsMask, bs)` (static in C)
pub fn parse_excluded_channels(excluded_chns_mask: &mut u32, bs: &mut FdkBitStream) -> i32 {
    let mut exclude_mask: u32 = 0;
    let mut bit_cnt: i32 = 9;

    let mut j: u32 = 1;
    for _i in 0..7 {
        if fdk_read_bits(bs, 1) != 0 {
            exclude_mask |= j;
        }
        j = j.wrapping_shl(1);
    }

    /* additional_excluded_chns */
    while fdk_read_bits(bs, 1) != 0 {
        for _i in 0..7 {
            if fdk_read_bits(bs, 1) != 0 {
                exclude_mask |= j;
            }
            // UINT j <<= 1: after 32 shifts j is 0 (the shift is by 1 each time).
            j = j.wrapping_shl(1);
        }
        bit_cnt += 9;
        fdk_assert!(j < u32::MAX);
    }

    *excluded_chns_mask = exclude_mask;

    bit_cnt
}

/// `aacDecoder_drcMarkPayload(self, bs, type)`: save the DRC payload bitstream position.
/// Returns the number of DRC payload bits.
pub fn aac_decoder_drc_mark_payload(
    self_: Option<&mut CDrcInfo>,
    bs: &mut FdkBitStream,
    type_: AacdecDrcPayloadType,
) -> i32 {
    let mut num_bands: i32 = 1;
    let mut bit_cnt: i32 = 0;

    let Some(self_) = self_ else {
        return 0;
    };

    let bs_start_pos: u32 = fdk_get_valid_bits(bs);

    match type_ {
        MPEG_DRC_EXT_DATA => {
            bit_cnt = 4;

            if fdk_read_bits(bs, 1) != 0 {
                /* pce_tag_present */
                fdk_read_bits(bs, 8); /* pce_instance_tag + drc_tag_reserved_bits */
                bit_cnt += 8;
            }

            if fdk_read_bits(bs, 1) != 0 {
                /* excluded_chns_present */
                fdk_read_bits(bs, 7); /* exclude mask [0..7] */
                bit_cnt += 8;
                while fdk_read_bits(bs, 1) != 0 {
                    /* additional_excluded_chns */
                    fdk_read_bits(bs, 7); /* exclude mask [x..y] */
                    bit_cnt += 8;
                }
            }

            if fdk_read_bits(bs, 1) != 0 {
                /* drc_bands_present */
                num_bands += fdk_read_bits(bs, 4) as i32; /* drc_band_incr */
                fdk_read_bits(bs, 4); /* reserved */
                bit_cnt += 8;
                for _i in 0..num_bands {
                    fdk_read_bits(bs, 8); /* drc_band_top[i] */
                    bit_cnt += 8;
                }
            }

            if fdk_read_bits(bs, 1) != 0 {
                /* prog_ref_level_present */
                fdk_read_bits(bs, 8); /* prog_ref_level + prog_ref_level_reserved_bits */
                bit_cnt += 8;
            }

            for _i in 0..num_bands {
                fdk_read_bits(bs, 8); /* dyn_rng_sgn[i] + dyn_rng_ctl[i] */
                bit_cnt += 8;
            }

            if (self_.num_payloads as i32) < MAX_DRC_THREADS && (fdk_get_valid_bits(bs) as i32) >= 0
            {
                self_.drc_payload_position[self_.num_payloads as usize] = bs_start_pos;
                self_.num_payloads += 1;
            }
        }

        DVB_DRC_ANC_DATA => {
            bit_cnt += 8;
            /* check sync word */
            if fdk_read_bits(bs, 8) == DVB_ANC_DATA_SYNC_BYTE {
                /* bs_info field */
                fdk_read_bits(bs, 8); /* mpeg_audio_type, dolby_surround_mode, presentation_mode */
                bit_cnt += 8;

                /* Evaluate ancillary_data_status */
                fdk_read_bits(bs, 3); /* reserved, set to 0 */
                let dmx_levels_present = fdk_read_bits(bs, 1); /* downmixing_levels_MPEG4_status */
                fdk_read_bits(bs, 1); /* reserved, set to 0 */
                let compression_present = fdk_read_bits(bs, 1); /* audio_coding_mode_and_compression status */
                let coarse_grain_tc_present = fdk_read_bits(bs, 1); /* coarse_grain_timecode_status */
                let fine_grain_tc_present = fdk_read_bits(bs, 1); /* fine_grain_timecode_status */
                bit_cnt += 8;

                /* MPEG4 downmixing levels */
                if dmx_levels_present != 0 {
                    fdk_read_bits(bs, 8); /* downmixing_levels_MPEG4 */
                    bit_cnt += 8;
                }
                /* audio coding mode and compression status */
                if compression_present != 0 {
                    fdk_read_bits(bs, 16); /* audio_coding_mode, Compression_value */
                    bit_cnt += 16;
                }
                /* coarse grain timecode */
                if coarse_grain_tc_present != 0 {
                    fdk_read_bits(bs, 16); /* coarse_grain_timecode */
                    bit_cnt += 16;
                }
                /* fine grain timecode */
                if fine_grain_tc_present != 0 {
                    fdk_read_bits(bs, 16); /* fine_grain_timecode */
                    bit_cnt += 16;
                }
                if self_.dvb_anc_data_available == 0 && (fdk_get_valid_bits(bs) as i32) >= 0 {
                    self_.dvb_anc_data_position = bs_start_pos;
                    self_.dvb_anc_data_available = 1;
                }
            }
        }

        _ => {}
    }

    bit_cnt
}

/// `aacDecoder_drcParse(bs, pDrcBs, payloadPosition)` (static in C): parse DRC parameters from
/// the bitstream. Returns whether new DRC data has been found.
pub fn aac_decoder_drc_parse(
    bs: &mut FdkBitStream,
    p_drc_bs: &mut CDrcPayload,
    payload_position: u32,
) -> i32 {
    /* Move to the beginning of the DRC payload field */
    let valid = fdk_get_valid_bits(bs) as i32;
    fdk_push_bi_directional(bs, valid.wrapping_sub(payload_position as i32));

    /* pce_tag_present */
    if fdk_read_bits(bs, 1) != 0 {
        p_drc_bs.pce_instance_tag = fdk_read_bits(bs, 4) as i8; /* pce_instance_tag */
        /* only one program supported */
        fdk_read_bits(bs, 4); /* drc_tag_reserved_bits */
    } else {
        p_drc_bs.pce_instance_tag = -1; /* not present */
    }

    if fdk_read_bits(bs, 1) != 0 {
        /* excluded_chns_present */
        /* get excluded_chn_mask */
        parse_excluded_channels(&mut p_drc_bs.excluded_chns_mask, bs);
    } else {
        p_drc_bs.excluded_chns_mask = 0;
    }

    let mut num_bands: i32 = 1;
    if fdk_read_bits(bs, 1) != 0
    /* drc_bands_present */
    {
        /* get band_incr */
        num_bands += fdk_read_bits(bs, 4) as i32; /* drc_band_incr */
        p_drc_bs.channel_data.drc_interpolation_scheme = fdk_read_bits(bs, 4) as i16; /* drc_interpolation_scheme */
        /* band_top */
        for i in 0..num_bands as usize {
            p_drc_bs.channel_data.band_top[i] = fdk_read_bits(bs, 8) as u16; /* drc_band_top[i] */
        }
    } else {
        p_drc_bs.channel_data.band_top[0] = (DRC_BLOCK_LEN_DIV_BAND_MULT - 1) as u16;
        /* ... comprising the whole spectrum. */
    }

    p_drc_bs.channel_data.num_bands = num_bands as u32;

    if fdk_read_bits(bs, 1) != 0
    /* prog_ref_level_present */
    {
        p_drc_bs.prog_ref_level = fdk_read_bits(bs, 7) as i8; /* prog_ref_level */
        fdk_read_bits(bs, 1); /* prog_ref_level_reserved_bits */
    } else {
        p_drc_bs.prog_ref_level = -1;
    }

    for i in 0..num_bands as usize {
        p_drc_bs.channel_data.drc_value[i] = (fdk_read_bits(bs, 1) << 7) as u8; /* dyn_rng_sgn[i] */
        p_drc_bs.channel_data.drc_value[i] |= (fdk_read_bits(bs, 7) & 0x7F) as u8;
        /* dyn_rng_ctl[i] */
    }

    /* Set DRC payload type */
    p_drc_bs.channel_data.drc_data_type = MPEG_DRC_EXT_DATA as i8;

    1
}

/// `aacDecoder_drcReadCompression(bs, pDrcBs, payloadPosition)` (static in C): parse the heavy
/// compression value transported in DSEs of DVB streams with MPEG-4 content. Returns whether new
/// DRC data has been found.
pub fn aac_decoder_drc_read_compression(
    bs: &mut FdkBitStream,
    p_drc_bs: &mut CDrcPayload,
    payload_position: u32,
) -> i32 {
    let mut found_drc_data = 0;

    /* Move to the beginning of the DRC payload field */
    let valid = fdk_get_valid_bits(bs) as i32;
    fdk_push_bi_directional(bs, valid.wrapping_sub(payload_position as i32));

    /* Sanity checks */
    if fdk_get_valid_bits(bs) < 24 {
        return 0;
    }

    /* Check sync word */
    if fdk_read_bits(bs, 8) != DVB_ANC_DATA_SYNC_BYTE {
        return 0;
    }

    /* Evaluate bs_info field */
    if fdk_read_bits(bs, 2) != 3 {
        /* mpeg_audio_type */
        /* No MPEG-4 audio data */
        return 0;
    }
    fdk_read_bits(bs, 2); /* dolby_surround_mode */
    p_drc_bs.pres_mode = fdk_read_bits(bs, 2) as i8; /* presentation_mode */
    fdk_read_bits(bs, 1); /* stereo_downmix_mode */
    if fdk_read_bits(bs, 1) != 0 {
        /* reserved, set to 0 */
        return 0;
    }

    /* Evaluate ancillary_data_status */
    if fdk_read_bits(bs, 3) != 0 {
        /* reserved, set to 0 */
        return 0;
    }
    let dmx_levels_present = fdk_read_bits(bs, 1); /* downmixing_levels_MPEG4_status */
    /*extensionPresent =*/
    fdk_read_bits(bs, 1); /* ancillary_data_extension_status; */
    let compression_present = fdk_read_bits(bs, 1); /* audio_coding_mode_and_compression status */
    /*coarseGrainTcPresent =*/
    fdk_read_bits(bs, 1); /* coarse_grain_timecode_status */
    /*fineGrainTcPresent   =*/
    fdk_read_bits(bs, 1); /* fine_grain_timecode_status */

    if dmx_levels_present != 0 {
        fdk_read_bits(bs, 8); /* downmixing_levels_MPEG4 */
    }

    /* audio_coding_mode_and_compression_status */
    if compression_present != 0 {
        /* audio_coding_mode */
        if fdk_read_bits(bs, 7) != 0 {
            /* The reserved bits shall be set to "0". */
            return 0;
        }
        let compression_on = fdk_read_bits(bs, 1) as u8; /* compression_on */
        let compression_value = fdk_read_bits(bs, 8) as u8; /* Compression_value */

        if compression_on != 0 {
            /* A compression value is available so store the data just like MPEG DRC data */
            p_drc_bs.channel_data.num_bands = 1; /* One band ... */
            p_drc_bs.channel_data.drc_value[0] = compression_value; /* ... with one value ... */
            p_drc_bs.channel_data.band_top[0] = (DRC_BLOCK_LEN_DIV_BAND_MULT - 1) as u16; /* ... comprising the whole spectrum. */
            p_drc_bs.pce_instance_tag = -1; /* Not present */
            p_drc_bs.prog_ref_level = -1; /* Not present */
            p_drc_bs.channel_data.drc_data_type = DVB_DRC_ANC_DATA as i8; /* Set DRC payload type to DVB. */
            found_drc_data = 1;
        }
    }

    found_drc_data
}

/// `aacDecoder_drcExtractAndMap(self, hBs, pAacDecoderStaticChannelInfo[], pceInstanceTag,
/// channelMapping[], validChannels)` (static in C): extract the DRC payload from the bitstream
/// and map it to channels. Returns -1 on an unexpected error, 0 if no valid DRC data is
/// available, 1 if valid DRC data has been mapped.
///
/// `p_aac_decoder_static_channel_info` is the decoder's array; the entries below
/// `valid_channels` exist in the library (C dereferences them). `channel_mapping` holds at
/// least `valid_channels` entries.
pub fn aac_decoder_drc_extract_and_map(
    self_: &mut CDrcInfo,
    h_bs: &mut FdkBitStream,
    p_aac_decoder_static_channel_info: &mut [Option<Box<CAacDecoderStaticChannelInfo>>],
    pce_instance_tag: u8,
    channel_mapping: &[u8],
    valid_channels: i32,
) -> i32 {
    let mut thread_bs = [CDrcPayload::default(); MAX_DRC_THREADS as usize];
    let mut valid_thread_bs = [0usize; MAX_DRC_THREADS as usize];
    let mut result = 0;
    let mut valid_threads = 0usize;

    self_.num_threads = 0;
    let backup_bs_position: u32 = fdk_get_valid_bits(h_bs);

    let mut i = 0;
    while i < self_.num_payloads as usize && (self_.num_threads as i32) < MAX_DRC_THREADS {
        /* Init payload data chunk. The memclear is very important because it initializes the
        most values. Without it the module wouldn't work properly or crash. */
        let t = self_.num_threads as usize;
        thread_bs[t] = CDrcPayload::default();
        thread_bs[t].channel_data.band_top[0] = (DRC_BLOCK_LEN_DIV_BAND_MULT - 1) as u16;

        /* Extract payload */
        self_.num_threads +=
            aac_decoder_drc_parse(h_bs, &mut thread_bs[t], self_.drc_payload_position[i]) as u16;
        i += 1;
    }
    self_.num_payloads = 0;

    if self_.dvb_anc_data_available != 0 && (self_.num_threads as i32) < MAX_DRC_THREADS {
        /* Append a DVB heavy compression payload thread if available. */

        /* Init payload data chunk. The memclear is very important because it initializes the
        most values. Without it the module wouldn't work properly or crash. */
        let t = self_.num_threads as usize;
        thread_bs[t] = CDrcPayload::default();
        thread_bs[t].channel_data.band_top[0] = (DRC_BLOCK_LEN_DIV_BAND_MULT - 1) as u16;

        /* Extract payload */
        self_.num_threads +=
            aac_decoder_drc_read_compression(h_bs, &mut thread_bs[t], self_.dvb_anc_data_position)
                as u16;
    }
    self_.dvb_anc_data_available = 0;

    /* Reset the bitbufffer */
    let valid = fdk_get_valid_bits(h_bs) as i32;
    fdk_push_bi_directional(h_bs, valid.wrapping_sub(backup_bs_position as i32));

    /* calculate number of valid bits in excl_chn_mask */

    /* coupling channels not supported */

    /* check for valid threads */
    for thread in 0..self_.num_threads as usize {
        let p_thread_bs = &thread_bs[thread];
        let mut num_excl_chns = 0;

        match p_thread_bs.channel_data.drc_data_type as i32 {
            MPEG_DRC_EXT_DATA | DVB_DRC_ANC_DATA => {}
            _ => continue,
        }

        if p_thread_bs.pce_instance_tag >= 0 {
            /* if PCE tag present */
            if p_thread_bs.pce_instance_tag as i32 != pce_instance_tag as i32 {
                continue; /* don't accept */
            }
        }

        /* calculate number of excluded channels */
        if p_thread_bs.excluded_chns_mask > 0 {
            let mut excl_mask = p_thread_bs.excluded_chns_mask as i32;
            for _ch in 0..valid_channels {
                num_excl_chns += excl_mask & 0x1;
                excl_mask >>= 1;
            }
        }
        if num_excl_chns < valid_channels {
            valid_thread_bs[valid_threads] = thread;
            valid_threads += 1;
        }
    }

    /* map DRC bitstream information onto DRC channel information */
    for &thread in valid_thread_bs.iter().take(valid_threads) {
        let p_thread_bs = &thread_bs[thread];
        let excl_mask = p_thread_bs.excluded_chns_mask as i32;
        let drc_payload_type = p_thread_bs.channel_data.drc_data_type as i32;

        /* last progRefLevel transmitted is the one that is used (but it should really only be
         * transmitted once per block!) */
        if p_thread_bs.prog_ref_level >= 0 {
            self_.prog_ref_level = p_thread_bs.prog_ref_level;
            self_.prog_ref_level_present = 1;
            self_.prl_expiry_count = 0; /* Got a new value -> Reset counter */
        }

        if drc_payload_type == DVB_DRC_ANC_DATA {
            /* Announce the presentation mode of this valid thread. */
            self_.pres_mode = p_thread_bs.pres_mode;
        }

        /* SCE, CPE and LFE */
        for ch in 0..valid_channels.max(0) as usize {
            let mut prv_payload_type = UNKNOWN_PAYLOAD;
            let maped_channel = channel_mapping.get(ch).copied().unwrap_or(255) as i32;

            if (maped_channel >= valid_channels) || ((excl_mask & (1i32 << maped_channel)) != 0) {
                continue;
            }

            // The static channel infos below validChannels exist (C dereferences them).
            let Some(sci) =
                p_aac_decoder_static_channel_info.get_mut(ch).and_then(|s| s.as_deref_mut())
            else {
                continue;
            };

            if (self_.params.expiry_frame == 0)
                || (sci.drc_data.expiry_count < self_.params.expiry_frame)
            {
                prv_payload_type = sci.drc_data.drc_data_type as i32;
            }
            if ((drc_payload_type == MPEG_DRC_EXT_DATA) && (prv_payload_type != DVB_DRC_ANC_DATA))
                || ((drc_payload_type == DVB_DRC_ANC_DATA)
                    && (self_.params.apply_heavy_compression == ON))
            {
                /* copy thread to channel */
                sci.drc_data = p_thread_bs.channel_data;
                result = 1;
            }
        }
        /* CCEs not supported by now */
    }

    /* Increment and check expiry counter for the program reference level: */
    if self_.params.expiry_frame > 0 {
        let c = self_.prl_expiry_count;
        self_.prl_expiry_count = c.wrapping_add(1);
        if c > self_.params.expiry_frame {
            /* The program reference level is too old, so set it back to the target level. */
            self_.prog_ref_level_present = 0;
            self_.prog_ref_level = self_.params.target_ref_level;
            self_.prl_expiry_count = 0;
        }
    }

    result
}

/// `aacDecoder_drcApply(self, pSbrDec, pAacDecoderChannelInfo, pDrcChData, extGain, ch,
/// aacFrameSize, bSbrPresent)`: apply DRC. If SBR is present, the DRC data is handed over to the
/// SBR decoder.
///
/// `ext_gain`: at the call it holds the scale (0 <= scale <= `DFRACT_BITS`) to be applied on the
/// gain value; the externally applicable gain is stored into it. `spec` is the channel's
/// spectrum (`pSpectralCoefficient`, at least `aac_frame_size` values).
#[allow(clippy::too_many_arguments)]
pub fn aac_decoder_drc_apply(
    self_: &mut CDrcInfo,
    p_sbr_dec: Option<&mut SbrDecoderInstance>,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_drc_ch_data: &mut CDrcChannelData,
    ext_gain: Option<&mut FixpDbl>,
    ch: i32, /* needed only for SBR */
    aac_frame_size: i32,
    b_sbr_present: i32,
    spec: &mut [FixpDbl],
) {
    let mut bottom: i32;
    let mut modify_bins = 0;

    let mut max_mantissa: FixpDbl;
    let mut max_exponent: i32;

    let mut norm_mantissa: FixpDbl = fl2fxconst_dbl(0.5);
    let mut norm_exponent: i32 = 1;

    let mut fact_mantissa = [0 as FixpDbl; MAX_DRC_BANDS as usize];
    let mut fact_exponent = [0i32; MAX_DRC_BANDS as usize];

    let win_seq = p_aac_decoder_channel_info.ics_info.window_sequence as i32;

    /* Increment and check expiry counter */
    if self_.params.expiry_frame > 0 {
        p_drc_ch_data.expiry_count = p_drc_ch_data.expiry_count.wrapping_add(1);
        if p_drc_ch_data.expiry_count > self_.params.expiry_frame {
            /* The DRC data is too old, so delete it. */
            aac_decoder_drc_init_channel_data(Some(p_drc_ch_data));
        }
    }

    if self_.enable != ON {
        // (sbrDecoder_drcDisable() returns at once for a NULL handle)
        if let Some(sbr) = p_sbr_dec {
            sbr_decoder_drc_disable(sbr, ch);
        }
        if let Some(ext_gain) = ext_gain {
            let gain_scale = *ext_gain;
            /* The gain scaling must be passed to the function in the buffer pointed on by
             * extGain. */
            if (0..=DFRACT_BITS).contains(&gain_scale) {
                *ext_gain = scale_value(norm_mantissa, norm_exponent - gain_scale);
            } else {
                fdk_assert!(false);
            }
        }
        return;
    }

    let mut num_bands = p_drc_ch_data.num_bands as i32;

    /* If program reference normalization is done in the digital domain, modify factor to
    perform normalization. prog_ref_level can alternatively be passed to the system for
    modification of the level in the analog domain. Analog level modification avoids problems
    with reduced DAC SNR (if signal is attenuated) or clipping (if signal is boosted) */

    if self_.params.target_ref_level >= 0 {
        /* 0.5^((targetRefLevel - progRefLevel)/24) */
        norm_mantissa = f_ld_pow(
            fl2fxconst_dbl(-1.0), /* log2(0.5) */
            0,
            (fl2fxconst_dbl(1.0f32 as f64 / 24.0) >> 3)
                .wrapping_mul(self_.params.target_ref_level as i32 - self_.prog_ref_level as i32),
            3,
            &mut norm_exponent,
        );
    }
    /* Always export the normalization gain (if possible). */
    if let Some(ext_gain) = ext_gain {
        let gain_scale = *ext_gain;
        /* The gain scaling must be passed to the function in the buffer pointed on by
         * extGain. */
        if (0..=DFRACT_BITS).contains(&gain_scale) {
            *ext_gain = scale_value(norm_mantissa, norm_exponent - gain_scale);
        } else {
            fdk_assert!(false);
        }
    }
    /* Reset normalization gain since this module must not apply it */
    norm_mantissa = fl2fxconst_dbl(0.5);
    norm_exponent = 1;

    // numBands is at most 16 in the library (1 + a 4-bit increment); the arrays have 16 entries.
    let nb = num_bands.clamp(0, MAX_DRC_BANDS) as usize;

    /* calc scale factors */
    for band in 0..nb {
        let drc_val: u8 = p_drc_ch_data.drc_value[band];

        fact_mantissa[band] = fl2fxconst_dbl(0.5);
        fact_exponent[band] = 1;

        if (self_.params.apply_heavy_compression == ON)
            && (p_drc_ch_data.drc_data_type as i32 == DVB_DRC_ANC_DATA)
        {
            let mut compression_factor_val_e: i32 = 0;

            let val_x = (drc_val >> 4) as i32;
            let val_y = (drc_val & 0x0F) as i32;

            /* calculate the unscaled heavy compression factor. compressionFactor = 48.164 -
            6.0206*valX - 0.4014*valY dB range: -48.166 dB to 48.164 dB */
            if drc_val != 0x7F {
                fact_mantissa[band] = f_pow_int(
                    fl2fxconst_dbl(0.95483867181), /* -0.4014dB = 0.95483867181 */
                    0,
                    val_y,
                    &mut compression_factor_val_e,
                );

                /* -0.0008dB (48.164 - 6.0206*8 = -0.0008) */
                fact_mantissa[band] = f_mult(fl2fxconst_dbl(0.99990790084), fact_mantissa[band]);

                fact_exponent[band] = DVB_COMPRESSION_SCALE - val_x + compression_factor_val_e;
            }
        } else if p_drc_ch_data.drc_data_type as i32 == MPEG_DRC_EXT_DATA {
            /* apply the scaled dynamic range control words to factor. if scaling drc_cut (or
             * drc_boost), or control word drc_mantissa is 0 then there is no dynamic range
             * compression
             *
             * if pDrcChData->drcSgn[band] is
             *  1 then gain is < 1 :  factor = 2^(-self->cut   * pDrcChData->drcMag[band] / 24)
             *  0 then gain is > 1 :  factor = 2^( self->boost * pDrcChData->drcMag[band] / 24)
             */

            if (drc_val & 0x7F) > 0 {
                let t_param_val: FixpDbl = if drc_val & 0x80 != 0 {
                    self_.params.cut.wrapping_neg()
                } else {
                    self_.params.boost
                };

                fact_mantissa[band] = f2_pow(
                    f_mult(fl2fxconst_dbl((1.0f32 / 192.0f32) as f64), t_param_val)
                        .wrapping_mul((drc_val & 0x7F) as i32),
                    3 + DRC_PARAM_SCALE,
                    &mut fact_exponent[band],
                );
            }
        }

        fact_mantissa[band] = f_mult(fact_mantissa[band], norm_mantissa);
        fact_exponent[band] += norm_exponent;
    } /* end loop over bands */

    /* normalizations */
    {
        max_mantissa = 0;
        max_exponent = 0;
        for band in 0..nb {
            max_mantissa = fixmax(max_mantissa, fact_mantissa[band]);
            max_exponent = fixmax(max_exponent, fact_exponent[band]);
        }

        /* left shift factors to gain accurancy */
        let mut res = cnt_leading_zeros(max_mantissa) - 1;

        /* above topmost DRC band gain factor is 1 */
        let top_idx = 0.max(num_bands - 1) as usize;
        let band_top = p_drc_ch_data.band_top.get(top_idx).copied().unwrap_or(0) as i32;
        if ((band_top + 1) << 2) < aac_frame_size {
            res = 0;
        }

        if res > 0 {
            res = fixmin(res, max_exponent);
            max_exponent -= res;

            for band in 0..nb {
                fact_mantissa[band] = fact_mantissa[band].wrapping_shl(res as u32);
                fact_exponent[band] -= res;
            }
        }

        /* normalize magnitudes to one scale factor */
        for band in 0..nb {
            if fact_exponent[band] < max_exponent {
                // The difference is below 32 for the factors of the library.
                fact_mantissa[band] >>= (max_exponent - fact_exponent[band]).min(31);
            }
            if fact_mantissa[band] != fl2fxconst_dbl(0.5) {
                modify_bins = 1;
            }
        }
        if max_exponent != 1 {
            modify_bins = 1;
        }
    }

    /*  apply factor to spectral lines
     *  short blocks must take care that bands fall on
     *  block boundaries!
     */
    if b_sbr_present == 0 {
        bottom = 0;

        if modify_bins == 0 {
            /* We don't have to modify the spectral bins because the fractional part of all
            factors is 0.5. In order to keep accurancy we don't apply the factor but decrease
            the exponent instead. */
            max_exponent -= 1;
        } else {
            for band in 0..nb {
                let top =
                    fixmin(((p_drc_ch_data.band_top[band] as i32 + 1) << 2) as i32, aac_frame_size); /* ... * DRC_BAND_MULT; */

                let mut bin = bottom;
                while bin < top {
                    if let Some(s) = spec.get_mut(bin as usize) {
                        *s = f_mult(*s, fact_mantissa[band]);
                    }
                    bin += 1;
                }

                bottom = top;
            }
        }

        /* above topmost DRC band gain factor is 1 */
        if max_exponent > 0 {
            let mut bin = bottom;
            while bin < aac_frame_size {
                if let Some(s) = spec.get_mut(bin as usize) {
                    // max_exponent is below 32 for the factors of the library.
                    *s >>= max_exponent.min(31);
                }
                bin += 1;
            }
        }

        /* adjust scaling */
        let p_spec_scale = &mut p_aac_decoder_channel_info.spec_scale;
        p_spec_scale[0] = p_spec_scale[0].wrapping_add(max_exponent as i16);

        if win_seq == BLOCK_SHORT as i32 {
            for win in 1..8 {
                p_spec_scale[win] = p_spec_scale[win].wrapping_add(max_exponent as i16);
            }
        }
    } else {
        num_bands = p_drc_ch_data.num_bands as i32;

        /* feed factors into SBR decoder for application in QMF domain. */
        // (sbrDecoder_drcFeedChannel() returns SBRDEC_NOT_INITIALIZED for a NULL handle; the
        // result is not used)
        if let Some(sbr) = p_sbr_dec {
            sbr_decoder_drc_feed_channel(
                sbr,
                ch,
                num_bands as u32,
                Some(&fact_mantissa),
                max_exponent,
                p_drc_ch_data.drc_interpolation_scheme,
                win_seq as u8,
                &p_drc_ch_data.band_top,
            );
        }
    }
}

/// `aacDecoder_drcParameterHandling(self, aacNumChannels, prevDrcProgRefLevel,
/// prevDrcPresMode)` (static in C): DRC parameter and presentation mode handling.
pub fn aac_decoder_drc_parameter_handling(
    self_: &mut CDrcInfo,
    aac_num_channels: i32,
    prev_drc_prog_ref_level: i8,
    prev_drc_pres_mode: i8,
) {
    if self_.prog_ref_level != prev_drc_prog_ref_level {
        self_.update = 1;
    }

    if self_.pres_mode != prev_drc_pres_mode {
        self_.update = 1;
    }

    if self_.prev_aac_num_channels != aac_num_channels {
        self_.update = 1;
    }

    /* return if no relevant parameter has changed */
    if self_.update == 0 {
        return;
    }

    let num_out_channels = self_.num_out_channels;
    let prog_ref_level = self_.prog_ref_level as i32;

    /* derive downmix property. aacNumChannels: number of channels in aac stream,
     * numOutChannels: number of output channels */
    let mut is_downmix = aac_num_channels > num_out_channels;
    is_downmix = is_downmix && (num_out_channels > 0);
    let is_mono_downmix = is_downmix && (num_out_channels == 1);
    let is_stereo_downmix = is_downmix && (num_out_channels == 2);

    let drc_parameter_handling: AacdecDrcParameterHandling =
        if (self_.pres_mode == 1) || (self_.pres_mode == 2) {
            self_.pres_mode as AacdecDrcParameterHandling
        } else {
            /* no presentation mode -> use parameter handling specified by
            AAC_DRC_DEFAULT_PRESENTATION_MODE */
            self_.params.default_presentation_mode
        };

    let p = &mut self_.params;

    /* by default, do as desired */
    p.cut = p.usr_cut;
    p.boost = p.usr_boost;
    p.apply_heavy_compression = p.usr_apply_heavy_compression;

    match drc_parameter_handling {
        ENABLED_PARAMETER_HANDLING => {
            /* dDmx: estimated headroom reduction due to downmix, format: -1/4*dB
            dDmx = floor(-4*20*log10(aacNumChannels/numOutChannels)) */
            let d_dmx: i32 = if is_downmix {
                let mut e_log = 0;
                let mut e_mult = 0;
                /* inverse division -> negative sign after logarithm */
                let mut dmx_tmp = f_div_norm_noexp(num_out_channels, aac_num_channels);
                dmx_tmp = f_log2(dmx_tmp, 0, &mut e_log);
                dmx_tmp = f_mult_norm(
                    dmx_tmp,
                    fl2fxconst_dbl((4.0f32 * 20.0f32 * 0.30103f32 / (1 << 5) as f32) as f64),
                    &mut e_mult,
                ); /* e = e_log + e_mult + 5 */
                scale_value(dmx_tmp, e_log + e_mult + 5 - (DFRACT_BITS - 1))
            } else {
                0
            };

            /* dHr: Full estimated (decoder) headroom reduction due to loudness normalisation
             * (DTL - PRL) and downmix. Format: -1/4*dB */
            let d_hr: i32 = if p.target_ref_level >= 0 {
                /* if target level is provided */
                p.target_ref_level as i32 + d_dmx - prog_ref_level
            } else {
                d_dmx
            };

            if d_hr < 0 {
                /* if headroom is reduced */
                /* Use compression, but as little as possible. */
                /* eHr: Headroom provided by encoder, format: -1/4 dB */
                let e_hr = fixmin(p.encoder_target_level as i32 - prog_ref_level, 0);
                if e_hr < d_hr {
                    /* if encoder provides more headroom than decoder needs */
                    /* derive scaling of light DRC */
                    /* fraction of DRC gains that is minimally needed for clipping prevention */
                    let mut calc_factor_norm = f_div_norm_noexp(-d_hr, -e_hr); /* 0.0 < calcFactor_norm < 1.0 */
                    calc_factor_norm >>= DRC_PARAM_SCALE;
                    /* quantize to 128 steps */
                    let calc_factor = convert_drc_param(calc_factor_norm); /* convert to integer value between 0 and 127 */
                    calc_factor_norm =
                        (DRC_PARAM_QUANT_STEP >> DRC_PARAM_SCALE).wrapping_mul(calc_factor);
                    /* use calcFactor_norm as lower limit */
                    p.cut = if calc_factor_norm > p.cut { calc_factor_norm } else { p.cut };
                } else {
                    /* encoder provides equal or less headroom than decoder needs */
                    /* the time domain limiter must always be active in this case. It is assumed
                     * that the framework activates it by default */
                    p.cut = DRC_SCALING_MAX;
                    if (d_hr - e_hr) <= -4 * DRC_HEAVY_THRESHOLD_DB {
                        /* use heavy compression if headroom deficit is equal or higher than
                         * DRC_HEAVY_THRESHOLD_DB */
                        p.apply_heavy_compression = ON;
                    }
                }
            } else {
                /* dHr >= 0: no restrictions required, as headroom is not reduced. */
                /* p->cut = p->usrCut; */
            }
        }

        /* presentation mode 1 and 2 according to ETSI TS 101 154: Digital Video Broadcasting
        (DVB); Specification for the use of Video and Audio Coding in Broadcasting Applications
        based on the MPEG-2 Transport Stream, section C.5.4., "Decoding", and Table C.33. Also
        according to amendment 4 to ISO/IEC 14496-3, section 4.5.2.14.2.4, and Table AMD4.11.
        ISO DRC            -> applyHeavyCompression = OFF (Use light compression, MPEG-style)
        Compression_value  -> applyHeavyCompression = ON  (Use heavy compression, DVB-style)
        scaling restricted -> p->cut = DRC_SCALING_MAX */
        DRC_PRESENTATION_MODE_1 => {
            /* presentation mode 1, Light:-31/Heavy:-23 */
            if (p.target_ref_level >= 0) && (p.target_ref_level < 124) {
                /* if target level is provided and > -31 dB */
                /* playback up to -23 dB */
                p.apply_heavy_compression = ON;
            } else {
                /* target level <= -31 dB or not provided */
                /* playback -31 dB */
                if is_mono_downmix || is_stereo_downmix {
                    /* stereo or mono downmixing */
                    p.cut = DRC_SCALING_MAX;
                }
            }
        }

        DRC_PRESENTATION_MODE_2 => {
            /* presentation mode 2, Light:-23/Heavy:-23 */
            if (p.target_ref_level >= 0) && (p.target_ref_level < 124) {
                /* if target level is provided and > -31 dB */
                /* playback up to -23 dB */
                if is_mono_downmix {
                    /* if mono downmix */
                    p.apply_heavy_compression = ON;
                } else {
                    p.apply_heavy_compression = OFF;
                    p.cut = DRC_SCALING_MAX;
                }
            } else {
                /* target level <= -31 dB or not provided */
                /* playback -31 dB */
                p.apply_heavy_compression = OFF;
                if is_mono_downmix || is_stereo_downmix {
                    /* stereo or mono downmixing */
                    p.cut = DRC_SCALING_MAX;
                }
            }
        }

        /* DISABLED_PARAMETER_HANDLING and default: use drc parameters as requested */
        _ => {}
    } /*  switch (drcParameterHandling) */

    /* With heavy compression, there is no scaling. Scaling factors are set for notification
    only. */
    if p.apply_heavy_compression == ON {
        p.boost = DRC_SCALING_MAX;
        p.cut = DRC_SCALING_MAX;
    }

    /* switch on/off processing */
    let enable = (p.boost > 0)
        || (p.cut > 0)
        || (p.apply_heavy_compression == ON)
        || (p.target_ref_level >= 0);
    self_.enable = enable as u8;

    self_.prev_aac_num_channels = aac_num_channels;
    self_.update = 0;
}

/// `aacDecoder_drcProlog(self, hBs, pAacDecoderStaticChannelInfo[], pceInstanceTag,
/// channelMapping[], validChannels)`: prepare DRC processing (without bitstream delay). Returns
/// -1 on an unexpected error, 0 if no valid DRC data is available, 1 if valid DRC data has been
/// mapped.
pub fn aac_decoder_drc_prolog(
    self_: Option<&mut CDrcInfo>,
    h_bs: &mut FdkBitStream,
    p_aac_decoder_static_channel_info: &mut [Option<Box<CAacDecoderStaticChannelInfo>>],
    pce_instance_tag: u8,
    channel_mapping: &[u8],
    valid_channels: i32,
) -> i32 {
    let mut result = 0;

    let Some(self_) = self_ else {
        return -1;
    };

    if self_.params.bs_delay_enable == 0 {
        /* keep previous progRefLevel and presMode for update flag in drcParameterHandling */
        let prev_prl = self_.prog_ref_level;
        let prev_pm = self_.pres_mode;

        result = aac_decoder_drc_extract_and_map(
            self_,
            h_bs,
            p_aac_decoder_static_channel_info,
            pce_instance_tag,
            channel_mapping,
            valid_channels,
        );

        if result < 0 {
            return result;
        }

        /* Drc parameter handling */
        aac_decoder_drc_parameter_handling(self_, valid_channels, prev_prl, prev_pm);
    }

    result
}

/// `aacDecoder_drcEpilog(self, hBs, pAacDecoderStaticChannelInfo[], pceInstanceTag,
/// channelMapping[], validChannels)`: finalize DRC processing (with bitstream delay). Returns as
/// [`aac_decoder_drc_prolog`].
pub fn aac_decoder_drc_epilog(
    self_: Option<&mut CDrcInfo>,
    h_bs: &mut FdkBitStream,
    p_aac_decoder_static_channel_info: &mut [Option<Box<CAacDecoderStaticChannelInfo>>],
    pce_instance_tag: u8,
    channel_mapping: &[u8],
    valid_channels: i32,
) -> i32 {
    let mut result = 0;

    let Some(self_) = self_ else {
        return -1;
    };

    if self_.params.bs_delay_enable != 0 {
        /* keep previous progRefLevel and presMode for update flag in drcParameterHandling */
        let prev_prl = self_.prog_ref_level;
        let prev_pm = self_.pres_mode;

        result = aac_decoder_drc_extract_and_map(
            self_,
            h_bs,
            p_aac_decoder_static_channel_info,
            pce_instance_tag,
            channel_mapping,
            valid_channels,
        );

        if result < 0 {
            return result;
        }

        /* Drc parameter handling */
        aac_decoder_drc_parameter_handling(self_, valid_channels, prev_prl, prev_pm);
    }

    result
}

/// `aacDecoder_drcGetInfo(self, pPresMode, pProgRefLevel)`: export the relevant metadata info
/// from the bitstream payload.
pub fn aac_decoder_drc_get_info(
    self_: Option<&CDrcInfo>,
    p_pres_mode: Option<&mut i8>,
    p_prog_ref_level: Option<&mut i8>,
) {
    if let Some(self_) = self_ {
        if let Some(p) = p_pres_mode {
            *p = self_.pres_mode;
        }
        if let Some(p) = p_prog_ref_level {
            if self_.prog_ref_level_present != 0 {
                *p = self_.prog_ref_level;
            } else {
                *p = -1;
            }
        }
    }
}

/// `applyDrcLevelNormalization(hDrcInfo, samplesIn, pGain, pGainPerSample, gain_scale,
/// gain_delay, nSamples, channels, stride, limiterEnabled)`: prepares/applies the gain values
/// for the DRC level normalization and returns the exponent of the time data.
///
/// - Limiter enabled: the input data must be interleaved; one gain per sample is written to
///   `p_gain_per_sample`; if necessary the time data is rescaled.
/// - Limiter disabled: the input data can be interleaved or deinterleaved; the gain values are
///   applied to the time data; if necessary the time data is rescaled.
///
/// `samples_in` holds the time data (`channels * n_samples` values). Accesses outside the slices
/// are skipped (the reference has no defined result there).
#[allow(clippy::too_many_arguments)]
pub fn apply_drc_level_normalization(
    h_drc_info: &mut CDrcInfo,
    samples_in: &mut [PcmDec],
    p_gain: &[FixpDbl],
    mut p_gain_per_sample: Option<&mut [FixpDbl]>,
    gain_scale: i32,
    gain_delay: u32,
    n_samples: u32,
    channels: u32,
    stride: u32,
    limiter_enabled: u32,
) -> i32 {
    let mut additional_gain_scaling: i32;
    let mut additional_gain: FixpDbl;

    fdk_assert!(gain_delay <= n_samples);

    let mut additional_gain_smooth_state = h_drc_info.additional_gain_filter_state;
    let mut additional_gain_smooth_state1 = h_drc_info.additional_gain_filter_state1;

    let gain0 = p_gain.first().copied().unwrap_or(0);

    if gain_delay == 0 {
        additional_gain = gain0;

        /* Apply the additional scaling gain_scale[0] that has no delay and no smoothing */
        additional_gain_scaling = gain_scale.min(cnt_leading_zeros(additional_gain) - 1);
        additional_gain = scale_value(additional_gain, additional_gain_scaling);

        /* if it's not possible to fully apply gain_scale to additionalGain, apply it to the
         * input signal */
        additional_gain_scaling -= gain_scale;

        let total = (channels.wrapping_mul(n_samples) as usize).min(samples_in.len());
        if additional_gain_scaling != 0 {
            scale_values_saturate(&mut samples_in[..total], -additional_gain_scaling);
        }

        if limiter_enabled != 0 {
            fdk_assert!(p_gain_per_sample.is_some());

            if let Some(g) = p_gain_per_sample.as_deref_mut() {
                let n = (n_samples as usize).min(g.len());
                for v in g[..n].iter_mut() {
                    *v = additional_gain;
                }
            }
        } else {
            for s in samples_in[..total].iter_mut() {
                *s = f_mult(*s, additional_gain);
            }
        }
    } else {
        let inc: usize = if stride == 1 { channels as usize } else { 1 };
        let mut pos: usize = 0;

        for i in 0..n_samples {
            let additional_gain_unfiltered: FixpDbl =
                if i < gain_delay { h_drc_info.additional_gain_prev } else { gain0 };

            /* Smooth additionalGain */

            /* [b,a] = butter(1, 0.01) */
            additional_gain = f_mult(additional_gain_smooth_state, SMOOTH_A[1])
                .wrapping_neg()
                .wrapping_add(f_mult_div2(additional_gain_unfiltered, SMOOTH_B[0]))
                .wrapping_add(f_mult_div2(additional_gain_smooth_state1, SMOOTH_B[1]));
            additional_gain_smooth_state1 = additional_gain_unfiltered;
            additional_gain_smooth_state = additional_gain;

            /* Apply the additional scaling gain_scale[0] that has no delay and no smoothing */
            additional_gain_scaling = gain_scale.min(cnt_leading_zeros(additional_gain) - 1);
            additional_gain = scale_value(additional_gain, additional_gain_scaling);

            /* if it's not possible to fully apply gain_scale[0] to additionalGain, apply it to
             * the input signal */
            additional_gain_scaling -= gain_scale;

            if limiter_enabled != 0 {
                fdk_assert!(stride == 1);
                fdk_assert!(p_gain_per_sample.is_some());

                if additional_gain_scaling != 0 {
                    let end = (pos + channels as usize).min(samples_in.len());
                    if pos < end {
                        scale_values_saturate(&mut samples_in[pos..end], -additional_gain_scaling);
                    }
                }

                if let Some(g) =
                    p_gain_per_sample.as_deref_mut().and_then(|g| g.get_mut(i as usize))
                {
                    *g = additional_gain;
                }
            } else {
                if additional_gain_scaling != 0 {
                    for k in 0..channels as usize {
                        let idx = pos + k * stride as usize;
                        if idx < samples_in.len() {
                            scale_values_saturate(
                                &mut samples_in[idx..idx + 1],
                                -additional_gain_scaling,
                            );
                        }
                    }
                }

                for k in 0..channels as usize {
                    if let Some(s) = samples_in.get_mut(pos + k * stride as usize) {
                        *s = f_mult(*s, additional_gain);
                    }
                }
            }

            pos += inc;
        }
    }

    h_drc_info.additional_gain_prev = gain0;
    h_drc_info.additional_gain_filter_state = additional_gain_smooth_state;
    h_drc_info.additional_gain_filter_state1 = additional_gain_smooth_state1;

    AACDEC_DRC_GAIN_SCALING
}
