// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/include/aacdecoder_lib.h, libAACdec/src/aacdecoder_lib.cpp.

//! Port of `aacdecoder_lib.h`: the public decoder API (`aacDecoder_*`), the straight-port layer
//! (PORTING.md, "Straight port"), which the C ABI and the differential tests use.
//!
//! - `HANDLE_AACDECODER` is `Box<AacDecoderInstance>` ([`aac_decoder_open`] returns
//!   `Option<Box<...>>`, `NULL` is `None`); the functions take `&mut AacDecoderInstance`, or
//!   `Option<&mut ...>` where the C function checks for `NULL`.
//! - Buffers are slices; the sizes of the C API are kept as separate arguments and keep their
//!   meaning (`timeDataSize` of [`aac_decoder_decode_frame`] is the number of samples the
//!   decoder may use and clears; a slice shorter than that only limits the writes).
//! - `CStreamInfo::pChannelType` and `pChannelIndices` always point to the decoder's channel
//!   arrays; the port keeps these arrays inside [`CStreamInfo`].
//! - The ancillary data buffer of `aacDecoder_AncDataInit()` is either given to the decoder
//!   ([`aac_decoder_anc_data_init`]) or registered as a raw pointer
//!   ([`aac_decoder_anc_data_init_raw`], for the C ABI). [`aac_decoder_anc_data_get`] returns the
//!   offset of an element in that buffer.
//!
//! The transport decoder calls the callbacks of `aacDecoder_Open()` through
//! [`AacDecoderCallbacks`], the [`TpDecCallbacks`] implementation of the decoder (see
//! `fdk_aac_rs_tpdec::tpdec_lib`).

pub use crate::channelinfo::aacdecoder_lib_types::*;

use fdk_aac_rs_drc::drc_dec_lib::{
    fdk_drc_dec_apply_downmix, fdk_drc_dec_close, fdk_drc_dec_get_lib_info, fdk_drc_dec_get_param,
    fdk_drc_dec_open, fdk_drc_dec_preprocess, fdk_drc_dec_process_time,
    fdk_drc_dec_read_downmix_instructions_box, fdk_drc_dec_read_loudness_box,
    fdk_drc_dec_read_loudness_info_set, fdk_drc_dec_read_uni_drc_coefficients_box,
    fdk_drc_dec_read_uni_drc_config, fdk_drc_dec_read_uni_drc_instructions_box,
    fdk_drc_dec_set_channel_gains, fdk_drc_dec_set_codec_mode, fdk_drc_dec_set_param, DrcDecError,
    DRC_DEC_ALBUM_MODE, DRC_DEC_ALL, DRC_DEC_BOOST, DRC_DEC_CODEC_MODE_UNDEFINED, DRC_DEC_COMPRESS,
    DRC_DEC_DRC1, DRC_DEC_DRC2_DRC3, DRC_DEC_EFFECT_TYPE, DRC_DEC_FRAME_SIZE, DRC_DEC_IS_ACTIVE,
    DRC_DEC_LOUDNESS_NORMALIZATION_ON, DRC_DEC_LOUDNESS_NOT_PRESENT, DRC_DEC_MPEG_D_USAC,
    DRC_DEC_OK, DRC_DEC_OUTPUT_LOUDNESS, DRC_DEC_SAMPLE_RATE,
    DRC_DEC_TARGET_CHANNEL_COUNT_REQUESTED, DRC_DEC_TARGET_LOUDNESS,
};
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::{
    fdk_get_free_bits, fdk_get_valid_bits, fdk_init_bit_stream, fdk_push_back, fdk_push_for,
    FdkBitStream, BS_READER,
};
use fdk_aac_rs_fdk::common_fix::{fl2fxconst_dbl, FixpDbl, DFRACT_BITS, MAXVAL_DBL};
use fdk_aac_rs_fdk::core::fdk_tools_get_lib_info;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::fixpoint_math::{f_div_norm, f_div_norm_noexp, f_mult_i_ceil, f_mult_norm_to};
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::qmf_domain::{
    fdk_qmf_domain_clear_persistent_memory, fdk_qmf_domain_configure, fdk_qmf_domain_free_mem,
    FdkQmfDomain, QMF_DOMAIN_INIT_ERROR, QMF_DOMAIN_OK, QMF_DOMAIN_OUT_OF_MEMORY,
};
use fdk_aac_rs_fdk::scale::scale_values_saturate_dbl_to_sgl;
use fdk_aac_rs_fdk::syslib_channel_map_descr::{
    fdk_ch_map_descr_init, fdk_ch_map_descr_is_valid, fdk_ch_map_descr_set_pass_through,
};
use fdk_aac_rs_pcmutils::limiter::{
    pcm_limiter_apply, pcm_limiter_create, pcm_limiter_destroy, pcm_limiter_get_delay,
    pcm_limiter_get_lib_info, pcm_limiter_reset, pcm_limiter_set_attack,
    pcm_limiter_set_n_channels, pcm_limiter_set_release, pcm_limiter_set_sample_rate,
    TDLIMIT_INVALID_HANDLE, TDLIMIT_OK, TDL_ATTACK_DEFAULT_MS, TDL_RELEASE_DEFAULT_MS,
};
use fdk_aac_rs_pcmutils::pcm_utils::{fdk_deinterleave, fdk_interleave, fdk_interleave_sgl};
use fdk_aac_rs_pcmutils::pcmdmx_lib::{
    pcm_dmx_apply_frame, pcm_dmx_close, pcm_dmx_get_lib_info, pcm_dmx_get_param, pcm_dmx_open,
    pcm_dmx_reset, pcm_dmx_set_param, PcmdmxError, DMX_BS_DATA_DELAY, DMX_BS_DATA_EXPIRY_FRAME,
    DMX_DUAL_CHANNEL_MODE, DMX_PRFL_ARIB_JAPAN, DMX_PRFL_FORCE_MATRIX_MIX, DMX_PRFL_MATRIX_MIX,
    DMX_PRFL_STANDARD, DMX_PROFILE_SETTING, MAX_NUMBER_OF_OUTPUT_CHANNELS,
    MIN_NUMBER_OF_OUTPUT_CHANNELS, PCMDMX_INVALID_HANDLE, PCMDMX_INVALID_MODE, PCMDMX_OK,
    PCMDMX_OUTPUT_BUFFER_TOO_SMALL, PCMDMX_RESET_BS_DATA,
};
use fdk_aac_rs_tpdec::tpdec_lib::{
    transport_dec_close, transport_dec_end_access_unit, transport_dec_fill_data,
    transport_dec_get_asc, transport_dec_get_lib_info, transport_dec_get_missing_access_unit_count,
    transport_dec_open, transport_dec_out_of_band_config_cb, transport_dec_read_access_unit_cb,
    transport_dec_set_param, CCtrlCFGChange, CSAudioSpecificConfig, TpDecCallbacks, TransportDec,
    TransportDecError, TPDEC_PARAM_IGNORE_BUFFERFULLNESS, TPDEC_PARAM_RESET, TP_FLAG_MPEG4,
    TP_USAC_MAX_CONFIG_LEN, TRANSPORTDEC_CRC_ERROR, TRANSPORTDEC_NEED_TO_RESTART,
    TRANSPORTDEC_NOT_ENOUGH_BITS, TRANSPORTDEC_OK, TRANSPORTDEC_PARSE_ERROR,
    TRANSPORTDEC_SYNC_ERROR, TRANSPORTDEC_UNKOWN_ERROR, TRANSPORTDEC_UNSUPPORTED_FORMAT,
};

use crate::aac_ram::get_required_mem_work_buffer_core1;
use crate::aac_rom::PcmDec;
use crate::aacdec_drc::{
    aac_decoder_drc_reset, aac_decoder_drc_set_param, apply_drc_level_normalization,
    AACDEC_DRC_GAIN_INIT_VALUE, APPLY_HEAVY_COMPRESSION, DEFAULT_PRESENTATION_MODE,
    DRC_BOOST_SCALE, DRC_BS_DELAY, DRC_CUT_SCALE, DRC_DATA_EXPIRY_FRAME, ENCODER_TARGET_LEVEL,
    MAX_OUTPUT_CHANNELS, TARGET_REF_LEVEL,
};
use crate::aacdecoder::*;
use crate::conceal::{
    cconcealment_get_delay, cconcealment_get_method, cconcealment_set_params,
    AACDEC_CONCEAL_PARAM_NOT_SPECIFIED,
};
use crate::conceal_types::{ConcealMethodNone, ConcealState_FadeIn};
use crate::delay::fdk_delay_apply;
use fdk_aac_rs_sacdec::sac_dec_errorcodes::{
    MPS_OK, MPS_OUTPUT_BUFFER_TOO_SMALL, MPS_PARSE_ERROR, MPS_UNSUPPORTED_CONFIG,
};
use fdk_aac_rs_sacdec::sac_dec_lib::{
    mpeg_surround_decoder_apply, mpeg_surround_decoder_close, mpeg_surround_decoder_config,
    mpeg_surround_decoder_configure_qmf_domain, mpeg_surround_decoder_free_mem,
    mpeg_surround_decoder_get_delay, mpeg_surround_decoder_get_lib_info,
    mpeg_surround_decoder_open, mpeg_surround_decoder_set_param, SACDEC_BS_INTERRUPTION,
    SACDEC_CLEAR_HISTORY, SACDEC_INTERFACE, SACDEC_OUT_MODE_NORMAL,
    SAC_INSTANCE_NOT_FULL_AVAILABLE, SAC_INTERFACE_QMF, SAC_INTERFACE_TIME,
};
use fdk_aac_rs_sbrdec::sbrdecoder::{
    sbr_decoder_apply, sbr_decoder_apply_pool, sbr_decoder_close, sbr_decoder_free_mem,
    sbr_decoder_get_delay, sbr_decoder_get_lib_info, sbr_decoder_header, sbr_decoder_open,
    sbr_decoder_set_param, SbrDecoderInstance, SBRDEC_INVALID_ARGUMENT, SBRDEC_NOT_INITIALIZED,
    SBRDEC_OK, SBRDEC_OUTPUT_BUFFER_TOO_SMALL, SBR_BS_INTERRUPTION, SBR_CLEAR_HISTORY,
    SBR_FLUSH_DATA, SBR_LD_QMF_TIME_ALIGN, SBR_SKIP_QMF, SBR_SYSTEM_BITSTREAM_DELAY,
};

// ---------------------------------------------------------------------------------------------
// aacdecoder_lib.h
// ---------------------------------------------------------------------------------------------

/// `AAC_MD_PROFILE`: the metadata profile of the decoder (`AAC_METADATA_PROFILE`).
pub type AacMdProfile = i32;
/// The standard profile creates a mixdown signal based on the advanced downmix metadata (from a
/// DSE).
pub const AAC_MD_PROFILE_MPEG_STANDARD: AacMdProfile = 0;
/// This profile behaves identical to the standard profile if advanced downmix metadata (from a
/// DSE) is available. If not, the matrix_mixdown information embedded in the program
/// configuration element (PCE) will be applied.
pub const AAC_MD_PROFILE_MPEG_LEGACY: AacMdProfile = 1;
/// Similar to the `AAC_MD_PROFILE_MPEG_LEGACY` profile but if both the advanced (DSE) and the
/// legacy (PCE) MPEG downmix metadata are available the latter will be applied.
pub const AAC_MD_PROFILE_MPEG_LEGACY_PRIO: AacMdProfile = 2;
/// Downmix creation as described in ABNT NBR 15602-2.
pub const AAC_MD_PROFILE_ARIB_JAPAN: AacMdProfile = 3;

/// `AAC_DRC_DEFAULT_PRESENTATION_MODE_OPTIONS`: DRC parameter handling disabled, all parameters
/// are applied as requested.
pub const AAC_DRC_PARAMETER_HANDLING_DISABLED: i32 = -1;
/// Apply changes to requested DRC parameters to prevent clipping.
pub const AAC_DRC_PARAMETER_HANDLING_ENABLED: i32 = 0;
/// Use DRC presentation mode 1 as default (e.g. for Nordig).
pub const AAC_DRC_PRESENTATION_MODE_1_DEFAULT: i32 = 1;
/// Use DRC presentation mode 2 as default (e.g. for DTG DBook).
pub const AAC_DRC_PRESENTATION_MODE_2_DEFAULT: i32 = 2;

/// `AACDEC_PARAM`: decoder parameter identifiers of `aacDecoder_SetParam()`.
pub type AacdecParam = i32;
/// Defines how the decoder processes two channel signals.
pub const AAC_PCM_DUAL_CHANNEL_OUTPUT_MODE: AacdecParam = 0x0002;
/// Output buffer channel ordering. 0: MPEG PCE style order, 1: WAV file channel order.
pub const AAC_PCM_OUTPUT_CHANNEL_MAPPING: AacdecParam = 0x0003;
/// Enable signal level limiting.
pub const AAC_PCM_LIMITER_ENABLE: AacdecParam = 0x0004;
/// Signal level limiting attack time in ms.
pub const AAC_PCM_LIMITER_ATTACK_TIME: AacdecParam = 0x0005;
/// Signal level limiting release time in ms.
pub const AAC_PCM_LIMITER_RELEAS_TIME: AacdecParam = 0x0006;
/// Minimum number of PCM output channels.
pub const AAC_PCM_MIN_OUTPUT_CHANNELS: AacdecParam = 0x0011;
/// Maximum number of PCM output channels.
pub const AAC_PCM_MAX_OUTPUT_CHANNELS: AacdecParam = 0x0012;
/// See `AAC_MD_PROFILE` for all available values.
pub const AAC_METADATA_PROFILE: AacdecParam = 0x0020;
/// Defines the time in ms after which all the bitstream associated meta-data (DRC, downmix
/// coefficients, ...) will be reset to default if no update has been received.
pub const AAC_METADATA_EXPIRY_TIME: AacdecParam = 0x0021;
/// Error concealment: processing method.
pub const AAC_CONCEAL_METHOD: AacdecParam = 0x0100;
/// MPEG-4 / MPEG-D Dynamic Range Control (DRC): scaling factor for boosting gain values.
pub const AAC_DRC_BOOST_FACTOR: AacdecParam = 0x0200;
/// MPEG-4 / MPEG-D DRC: scaling factor for attenuating gain values.
pub const AAC_DRC_ATTENUATION_FACTOR: AacdecParam = 0x0201;
/// MPEG-4 / MPEG-D DRC: target reference level / decoder target loudness.
pub const AAC_DRC_REFERENCE_LEVEL: AacdecParam = 0x0202;
/// MPEG-4 DRC: en-/disable DVB specific heavy compression (aka RF mode).
pub const AAC_DRC_HEAVY_COMPRESSION: AacdecParam = 0x0203;
/// MPEG-4 DRC: default presentation mode (DRC parameter handling).
pub const AAC_DRC_DEFAULT_PRESENTATION_MODE: AacdecParam = 0x0204;
/// MPEG-4 DRC: encoder target level for light (i.e. not heavy) compression.
pub const AAC_DRC_ENC_TARGET_LEVEL: AacdecParam = 0x0205;
/// MPEG-D DRC: request a DRC effect type for selection of a DRC set.
pub const AAC_UNIDRC_SET_EFFECT: AacdecParam = 0x0206;
/// MPEG-D DRC: enable album mode.
pub const AAC_UNIDRC_ALBUM_MODE: AacdecParam = 0x0207;
/// Quadrature Mirror Filter (QMF) Bank processing mode.
pub const AAC_QMF_LOWPOWER: AacdecParam = 0x0300;
/// Clear internal bit stream buffer of transport layers.
pub const AAC_TPDEC_CLEAR_BUFFER: AacdecParam = 0x0603;

/// `AACDEC_CONCEAL`: flag for `aacDecoder_DecodeFrame()`: trigger the built-in error
/// concealment module to generate a substitute signal for one lost frame.
pub const AACDEC_CONCEAL: u32 = 1;
/// `AACDEC_FLUSH`: flag for `aacDecoder_DecodeFrame()`: flush all filterbanks to get all delayed
/// audio without having new input data.
pub const AACDEC_FLUSH: u32 = 2;
/// `AACDEC_INTR`: flag for `aacDecoder_DecodeFrame()`: signal an input bit stream data
/// discontinuity.
pub const AACDEC_INTR: u32 = 4;
/// `AACDEC_CLRHIST`: flag for `aacDecoder_DecodeFrame()`: clear all signal delay lines and
/// history buffers.
pub const AACDEC_CLRHIST: u32 = 8;

/// `CStreamInfo`: this structure gives information about the currently decoded audio data. All
/// fields are read-only.
///
/// `pChannelType` and `pChannelIndices` always point to the decoder's `channelType` and
/// `channelIndices`; the port keeps these arrays here ([`CStreamInfo::p_channel_type`],
/// [`CStreamInfo::p_channel_indices`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CStreamInfo {
    /* These five members are the only really relevant ones for the user. */
    /// `sampleRate`: the sample rate in Hz of the decoded PCM audio signal.
    pub sample_rate: i32,
    /// `frameSize`: the frame size of the decoded PCM audio signal.
    pub frame_size: i32,
    /// `numChannels`: the number of output audio channels before the rendering module, i.e. the
    /// original channel configuration.
    pub num_channels: i32,
    /// `pChannelType` (the decoder's `channelType`): audio channel type of each output audio
    /// channel.
    pub p_channel_type: [AudioChannelType; 8],
    /// `pChannelIndices` (the decoder's `channelIndices`): audio channel index for each output
    /// audio channel.
    pub p_channel_indices: [u8; 8],
    /* Decoder internal members. */
    /// `aacSampleRate`: sampling rate in Hz without SBR (from configuration info) divided by a
    /// (ELD) downscale factor if present.
    pub aac_sample_rate: i32,
    /// `profile`: MPEG-2 profile (from file header) (-1: not applicable (e. g. MPEG-4)).
    pub profile: i32,
    /// `aot`: Audio Object Type (from ASC).
    pub aot: AudioObjectType,
    /// `channelConfig`: channel configuration (0: PCE defined, 1: mono, 2: stereo, ...
    pub channel_config: i32,
    /// `bitRate`: instantaneous bit rate.
    pub bit_rate: i32,
    /// `aacSamplesPerFrame`: samples per frame for the AAC core (from ASC) divided by a (ELD)
    /// downscale factor if present.
    pub aac_samples_per_frame: i32,
    /// `aacNumChannels`: the number of audio channels after AAC core processing (before PS or
    /// MPS processing).
    pub aac_num_channels: i32,
    /// `extAot`: extension Audio Object Type (from ASC)
    pub ext_aot: AudioObjectType,
    /// `extSamplingRate`: extension sampling rate in Hz (from ASC) divided by a (ELD) downscale
    /// factor if present.
    pub ext_sampling_rate: i32,
    /// `outputDelay`: the number of samples the output is additionally delayed by the decoder.
    pub output_delay: u32,
    /// `flags`: copy of internal flags. Only to be written by the decoder, and only to be read
    /// externally.
    pub flags: u32,
    /// `epConfig`: epConfig level (from ASC): only level 0 supported, -1 means no ER.
    pub ep_config: i8,
    /* Statistics */
    /// `numLostAccessUnits`: the estimated amount of lost access units in case
    /// `aacDecoder_DecodeFrame()` returns `AAC_DEC_TRANSPORT_SYNC_ERROR`.
    pub num_lost_access_units: i32,
    /// `numTotalBytes`: the number of total bytes that have passed through the decoder.
    pub num_total_bytes: i64,
    /// `numBadBytes`: the number of total bytes that were considered with errors from
    /// numTotalBytes.
    pub num_bad_bytes: i64,
    /// `numTotalAccessUnits`: the number of total access units that have passed through the
    /// decoder.
    pub num_total_access_units: i64,
    /// `numBadAccessUnits`: the number of total access units that were considered with errors
    /// from numTotalBytes.
    pub num_bad_access_units: i64,
    /* Metadata */
    /// `drcProgRefLev`: DRC program reference level.
    pub drc_prog_ref_lev: i8,
    /// `drcPresMode`: DRC presentation mode.
    pub drc_pres_mode: i8,
    /// `outputLoudness`: audio output loudness in steps of -0.25 dB.
    pub output_loudness: i32,
}

/// The callbacks of the decoder for the transport decoder (`aacDecoder_ConfigCallback`,
/// `aacDecoder_FreeMemCallback`, `aacDecoder_CtrlCFGChangeCallback`, `aacDecoder_SbrCallback`,
/// `aacDecoder_SscCallback`, `aacDecoder_UniDrcCallback`, registered in `aacDecoder_Open()`).
///
/// The transport decoder is taken out of the decoder instance while it calls them
/// (`AacDecoderInstance::with_tp_cb`); none of the callbacks uses it.
pub struct AacDecoderCallbacks<'a> {
    /// The decoder instance (`void *handle`).
    pub dec: &'a mut AacDecoderInstance,
}

impl TpDecCallbacks for AacDecoderCallbacks<'_> {
    fn update_config(
        &mut self,
        asc: &CSAudioSpecificConfig,
        config_mode: u8,
        config_changed: &mut u8,
    ) -> i32 {
        if self.dec.pipe.is_some() {
            return crate::pipeline::state::config_callback(
                self.dec,
                asc,
                config_mode,
                config_changed,
            );
        }
        aac_decoder_config_callback(self.dec, asc, config_mode, config_changed)
    }

    fn free_mem(&mut self, asc: &CSAudioSpecificConfig) -> i32 {
        crate::pipeline::state::callback_join(self.dec);
        aac_decoder_free_mem_callback(self.dec, asc)
    }

    fn ctrl_cfg_change(&mut self, ctrl: &CCtrlCFGChange) -> i32 {
        crate::pipeline::state::callback_join(self.dec);
        aac_decoder_ctrl_cfg_change_callback(Some(self.dec), ctrl)
    }

    fn has_ssc(&self) -> bool {
        true
    }

    fn ssc(
        &mut self,
        hbs: &mut FdkBitStream,
        core_codec: AudioObjectType,
        sampling_rate: i32,
        frame_size: i32,
        num_channels: i32,
        stereo_config_index: i32,
        core_sbr_frame_length_index: i32,
        config_bytes: i32,
        config_mode: u8,
        config_changed: &mut u8,
    ) -> i32 {
        crate::pipeline::state::callback_join(self.dec);
        aac_decoder_ssc_callback(
            self.dec,
            hbs,
            core_codec,
            sampling_rate,
            frame_size,
            num_channels,
            stereo_config_index,
            core_sbr_frame_length_index,
            config_bytes,
            config_mode,
            config_changed,
        )
    }

    fn has_sbr(&self) -> bool {
        true
    }

    fn sbr(
        &mut self,
        hbs: &mut FdkBitStream,
        sample_rate_in: i32,
        sample_rate_out: i32,
        samples_per_frame: i32,
        core_codec: AudioObjectType,
        element_id: Mp4ElementId,
        element_index: i32,
        harmonic_sbr: u8,
        stereo_config_index: u8,
        config_mode: u8,
        config_changed: &mut u8,
        downscale_factor: i32,
    ) -> i32 {
        // The callback's data pointer is hSbrDecoder (registered in aacDecoder_Open()); the SBR
        // decoder's QMF domain is the decoder's.
        crate::pipeline::state::callback_join(self.dec);
        aac_decoder_sbr_callback(
            self.dec.h_sbr_decoder.as_deref_mut(),
            &mut self.dec.qmf_domain,
            hbs,
            sample_rate_in,
            sample_rate_out,
            samples_per_frame,
            core_codec,
            element_id,
            element_index,
            harmonic_sbr,
            stereo_config_index,
            config_mode,
            config_changed,
            downscale_factor,
        )
    }

    fn has_usac(&self) -> bool {
        false
    }

    fn usac(&mut self, _hbs: &mut FdkBitStream) -> i32 {
        0
    }

    fn has_uni_drc(&self) -> bool {
        true
    }

    fn uni_drc(
        &mut self,
        hbs: Option<&mut FdkBitStream>,
        full_payload_length: i32,
        payload_type: i32,
        sub_stream_index: i32,
        payload_start: i32,
        aot: AudioObjectType,
    ) -> i32 {
        crate::pipeline::state::callback_join(self.dec);
        aac_decoder_uni_drc_callback(
            self.dec,
            hbs,
            full_payload_length,
            payload_type,
            sub_stream_index,
            payload_start,
            aot,
        )
    }
}

impl AacDecoderInstance {
    /// Calls `f` with the transport decoder and the decoder's callbacks (the transport decoder
    /// is taken out of the instance meanwhile). `None` without a transport decoder.
    pub(crate) fn with_tp_cb<T>(
        &mut self,
        f: impl FnOnce(&mut TransportDec, &mut dyn TpDecCallbacks) -> T,
    ) -> Option<T> {
        let mut tp = self.h_input.take()?;
        let r = {
            let mut cb = AacDecoderCallbacks { dec: self };
            f(&mut tp, &mut cb)
        };
        self.h_input = Some(tp);
        Some(r)
    }
}

// ---------------------------------------------------------------------------------------------
// aacdecoder_lib.cpp
// ---------------------------------------------------------------------------------------------

/// `AACDECODER_LIB_VL0`
const AACDECODER_LIB_VL0: i32 = 3;
/// `AACDECODER_LIB_VL1`
const AACDECODER_LIB_VL1: i32 = 2;
/// `AACDECODER_LIB_VL2`
const AACDECODER_LIB_VL2: i32 = 0;
/// `AACDECODER_LIB_TITLE`
const AACDECODER_LIB_TITLE: &str = "AAC Decoder Lib";

/// `aacDecoder_setMetadataExpiry(self, value)` (static in C)
pub fn aac_decoder_set_metadata_expiry(self_: &mut AacDecoderInstance, value: i32) {
    let md_exp_frame = metadata_expiry_frames(self_, value);
    set_metadata_expiry_frames(self_, md_exp_frame);
}

/// The number of frames `aacDecoder_setMetadataExpiry(self, value)` sets.
pub(crate) fn metadata_expiry_frames(self_: &AacDecoderInstance, value: i32) -> i32 {
    let mut md_exp_frame: i32 = 0; /* default: disable */

    if (value > 0) && (self_.stream_info.aac_samples_per_frame > 0) {
        /* Determine the corresponding number of frames: */
        let frame_time = f_div_norm_noexp(
            self_.stream_info.aac_sample_rate,
            self_.stream_info.aac_samples_per_frame.wrapping_mul(1000),
        );
        md_exp_frame = f_mult_i_ceil(frame_time, value);
    }
    md_exp_frame
}

/// The settings of `aacDecoder_setMetadataExpiry()` with `md_exp_frame` frames.
pub(crate) fn set_metadata_expiry_frames(self_: &mut AacDecoderInstance, md_exp_frame: i32) {
    /* Configure DRC module */
    aac_decoder_drc_set_param(self_.h_drc_info.as_deref_mut(), DRC_DATA_EXPIRY_FRAME, md_exp_frame);

    /* Configure PCM downmix module */
    pcm_dmx_set_param(self_.h_pcm_utils.as_deref_mut(), DMX_BS_DATA_EXPIRY_FRAME, md_exp_frame);
}

/// `aacDecoder_GetFreeBytes(self, pFreeBytes)`: the number of free bytes in the internal input
/// buffer.
pub fn aac_decoder_get_free_bytes(
    self_: Option<&mut AacDecoderInstance>,
    p_free_bytes: &mut u32,
) -> AacDecoderError {
    /* reset free bytes */
    *p_free_bytes = 0;

    /* check handle */
    let Some(self_) = self_ else {
        return AAC_DEC_INVALID_HANDLE;
    };

    /* return nr of free bytes */
    if let Some(tp) = self_.h_input.as_deref_mut() {
        *p_free_bytes = (fdk_get_free_bits(&tp.bit_stream[0]) >> 3) as u32;
    }

    /* success */
    AAC_DEC_OK
}

/// `aacDecoder_Config(self, pAscStruct, configMode, configChanged)` (static in C): configure
/// the decoder using a `CSAudioSpecificConfig` struct.
pub fn aac_decoder_config(
    self_: &mut AacDecoderInstance,
    p_asc_struct: &CSAudioSpecificConfig,
    config_mode: u8,
    config_changed: &mut u8,
) -> AacDecoderError {
    /* Initialize AAC core decoder, and update self->streaminfo */
    let err = caac_decoder_init(self_, p_asc_struct, config_mode, config_changed);

    if fdk_ch_map_descr_is_valid(Some(&self_.map_descr)) == 0 {
        return AAC_DEC_UNSUPPORTED_CHANNELCONFIG;
    }

    err
}

/// `aacDecoder_ConfigRaw(self, conf[], length[])`: explicitly configure the decoder by passing a
/// raw AudioSpecificConfig (ASC) or a StreamMuxConfig (SMC), contained in a binary buffer, one
/// per layer.
pub fn aac_decoder_config_raw(
    self_: &mut AacDecoderInstance,
    conf: &[&[u8]],
    length: &[u32],
) -> AacDecoderError {
    crate::pipeline::sync_for_api(self_);
    let mut err = AAC_DEC_OK;
    let nr_of_layers = self_.nr_of_layers as u32;

    for layer in 0..nr_of_layers {
        let len = length.get(layer as usize).copied().unwrap_or(0);
        if len > 0 {
            let c: &[u8] = conf.get(layer as usize).copied().unwrap_or(&[]);
            let err_tp = self_
                .with_tp_cb(|tp, cb| transport_dec_out_of_band_config_cb(tp, c, len, layer, cb))
                .unwrap_or(TRANSPORTDEC_UNKOWN_ERROR);
            if err_tp != TRANSPORTDEC_OK {
                err = match err_tp {
                    TRANSPORTDEC_NEED_TO_RESTART => AAC_DEC_NEED_TO_RESTART,
                    TRANSPORTDEC_UNSUPPORTED_FORMAT => AAC_DEC_UNSUPPORTED_FORMAT,
                    _ => AAC_DEC_UNKNOWN,
                };
                /* if baselayer is OK we continue decoding */
                if layer >= 1 {
                    self_.nr_of_layers = layer as i32;
                    err = AAC_DEC_OK;
                }
                break;
            }
        }
    }

    err
}

/// `aacDecoder_RawISOBMFFData(self, buffer, length)`: submit raw ISO base media file format
/// boxes (`ludt`, `dmix`, `udi2`, `udc2`) to the MPEG-D DRC decoder.
///
/// The port reads each box from the rest of `buffer` from the box payload on, followed by zeros
/// (beyond the buffer the reference has no defined result). It copies that rest for every box,
/// so the time grows with the number of boxes times the buffer size; callers pass the few boxes
/// of a configuration.
pub fn aac_decoder_raw_isobmff_data(
    self_: &mut AacDecoderInstance,
    buffer: &[u8],
    length: u32,
) -> AacDecoderError {
    crate::pipeline::sync_for_api(self_);
    let mut err = AAC_DEC_OK;
    let mut length = length;
    let mut pos: usize = 0;

    if length < 8 {
        return AAC_DEC_UNKNOWN;
    }

    let byte = |i: usize| buffer.get(i).copied().unwrap_or(0);

    while length >= 8 {
        let size: u32 = ((byte(pos) as u32) << 24)
            | ((byte(pos + 1) as u32) << 16)
            | ((byte(pos + 2) as u32) << 8)
            | byte(pos + 3) as u32;
        let mut uni_drc_err: DrcDecError = DRC_DEC_OK;

        if length < size {
            return AAC_DEC_UNKNOWN;
        }
        if size <= 8 {
            return AAC_DEC_UNKNOWN;
        }

        // The payload and everything after it in the caller's buffer, padded with zeros.
        let start = (pos + 8).min(buffer.len());
        let data = BitMem::new(buffer.len() - start + 4096);
        data.write(0, &buffer[start..]);
        let mut bs = FdkBitStream::default();
        fdk_init_bit_stream(
            &mut bs,
            &data,
            data.len() as u32,
            (size - 8).wrapping_mul(8),
            BS_READER,
        );

        let tag = [byte(pos + 4), byte(pos + 5), byte(pos + 6), byte(pos + 7)];
        let h = self_.h_uni_drc_decoder.as_deref_mut();
        if &tag == b"ludt" {
            uni_drc_err = fdk_drc_dec_read_loudness_box(h, &mut bs);
        } else if &tag == b"dmix" {
            uni_drc_err = fdk_drc_dec_read_downmix_instructions_box(h, &mut bs);
        } else if &tag == b"udi2" {
            uni_drc_err = fdk_drc_dec_read_uni_drc_instructions_box(h, &mut bs);
        } else if &tag == b"udc2" {
            uni_drc_err = fdk_drc_dec_read_uni_drc_coefficients_box(h, &mut bs);
        }

        if uni_drc_err != DRC_DEC_OK {
            err = AAC_DEC_UNKNOWN;
        }

        pos += size as usize;
        length -= size;
    }

    err
}

/// `aacDecoder_ConfigCallback(handle, pAscStruct, configMode, configChanged)` (static in C)
pub fn aac_decoder_config_callback(
    self_: &mut AacDecoderInstance,
    p_asc_struct: &CSAudioSpecificConfig,
    config_mode: u8,
    config_changed: &mut u8,
) -> i32 {
    let err_tp: TransportDecError;

    let err = aac_decoder_config(self_, p_asc_struct, config_mode, config_changed);
    if err == AAC_DEC_OK {
        /*
        revert concealment method if either
           - Interpolation concealment might not be meaningful
           - Interpolation concealment is not implemented
        */
        let delay = cconcealment_get_delay(Some(&self_.conceal_common_data));
        if ((self_.flags[0] & (AC_LD | AC_ELD)) != 0
            && (self_.conceal_method_user == ConcealMethodNone)
            && delay > 0) /* might not be meaningful but allow if user has set it expicitly */
            || ((self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_RSV603DA)) != 0 && delay > 0)
        /* not implemented */
        {
            /* Revert to error concealment method Noise Substitution. Because interpolation is
            not implemented for USAC or the additional delay is unwanted for low delay codecs. */
            set_conceal_method(self_, 1);
        }
        aac_decoder_set_metadata_expiry(self_, self_.metadata_expiry);
        err_tp = TRANSPORTDEC_OK;
    } else if err == AAC_DEC_NEED_TO_RESTART {
        err_tp = TRANSPORTDEC_NEED_TO_RESTART;
    } else if is_init_error(err) != 0 {
        err_tp = TRANSPORTDEC_UNSUPPORTED_FORMAT;
    }
    /* Fatal errors */
    else {
        err_tp = TRANSPORTDEC_UNKOWN_ERROR;
    }

    err_tp
}

/// `aacDecoder_FreeMemCallback(handle, pAscStruct)` (static in C)
pub fn aac_decoder_free_mem_callback(
    self_: &mut AacDecoderInstance,
    _p_asc_struct: &CSAudioSpecificConfig,
) -> i32 {
    let mut err_tp = TRANSPORTDEC_OK;

    let sub_stream_index = 0;

    if caac_decoder_free_mem(self_, sub_stream_index) != AAC_DEC_OK {
        err_tp = TRANSPORTDEC_UNKOWN_ERROR;
    }

    /* free Ram_SbrDecoder and Ram_SbrDecChannel */
    if self_.h_sbr_decoder.is_some()
        && sbr_decoder_free_mem(self_.h_sbr_decoder.as_deref_mut()) != SBRDEC_OK
    {
        err_tp = TRANSPORTDEC_UNKOWN_ERROR;
    }

    /* free pSpatialDec and mpsData */
    if self_.p_mpeg_surround_decoder.is_some()
        && mpeg_surround_decoder_free_mem(self_.p_mpeg_surround_decoder.as_deref_mut()) != MPS_OK
    {
        err_tp = TRANSPORTDEC_UNKOWN_ERROR;
    }

    /* free persistent qmf domain buffer, QmfWorkBufferCore3, QmfWorkBufferCore4,
     * QmfWorkBufferCore5 and configuration variables */
    fdk_qmf_domain_free_mem(&mut self_.qmf_domain);

    err_tp
}

/// `aacDecoder_CtrlCFGChangeCallback(handle, pCtrlCFGChangeStruct)` (static in C)
pub fn aac_decoder_ctrl_cfg_change_callback(
    self_: Option<&mut AacDecoderInstance>,
    p_ctrl_cfg_change_struct: &CCtrlCFGChange,
) -> i32 {
    let mut err_tp = TRANSPORTDEC_OK;

    if let Some(self_) = self_ {
        caac_decoder_ctrl_cfg_change(
            self_,
            p_ctrl_cfg_change_struct.flush_status,
            p_ctrl_cfg_change_struct.flush_cnt,
            p_ctrl_cfg_change_struct.build_up_status,
            p_ctrl_cfg_change_struct.build_up_cnt,
        );
    } else {
        err_tp = TRANSPORTDEC_UNKOWN_ERROR;
    }

    err_tp
}

/// `aacDecoder_SbrCallback(handle, hBs, sampleRateIn, sampleRateOut, samplesPerFrame,
/// coreCodec, elementID, elementIndex, harmonicSBR, stereoConfigIndex, configMode,
/// configChanged, downscaleFactor)` (static in C): the handle is the SBR decoder; `qd` is the
/// QMF domain it was opened with (the decoder's).
#[allow(clippy::too_many_arguments)]
pub fn aac_decoder_sbr_callback(
    self_: Option<&mut SbrDecoderInstance>,
    qd: &mut FdkQmfDomain,
    h_bs: &mut FdkBitStream,
    sample_rate_in: i32,
    sample_rate_out: i32,
    samples_per_frame: i32,
    core_codec: AudioObjectType,
    element_id: Mp4ElementId,
    element_index: i32,
    harmonic_sbr: u8,
    stereo_config_index: u8,
    config_mode: u8,
    config_changed: &mut u8,
    downscale_factor: i32,
) -> i32 {
    let Some(self_) = self_ else {
        // (sbrDecoder_Header() with a NULL handle)
        return SBRDEC_NOT_INITIALIZED;
    };
    sbr_decoder_header(
        self_,
        qd,
        h_bs,
        sample_rate_in,
        sample_rate_out,
        samples_per_frame,
        core_codec,
        element_id,
        element_index,
        harmonic_sbr,
        stereo_config_index,
        config_mode,
        config_changed,
        downscale_factor,
    )
}

/// `aacDecoder_SscCallback(handle, hBs, coreCodec, samplingRate, frameSize, numChannels,
/// stereoConfigIndex, coreSbrFrameLengthIndex, configBytes, configMode, configChanged)` (static
/// in C)
#[allow(clippy::too_many_arguments)]
pub fn aac_decoder_ssc_callback(
    h_aac_decoder: &mut AacDecoderInstance,
    h_bs: &mut FdkBitStream,
    core_codec: AudioObjectType,
    sampling_rate: i32,
    frame_size: i32,
    num_channels: i32,
    stereo_config_index: i32,
    core_sbr_frame_length_index: i32,
    config_bytes: i32,
    config_mode: u8,
    config_changed: &mut u8,
) -> i32 {
    let Some(mps) = h_aac_decoder.p_mpeg_surround_decoder.as_deref_mut() else {
        // (None: the reference has no defined result here)
        h_aac_decoder.mps_applicable = 0;
        return TRANSPORTDEC_UNKOWN_ERROR;
    };
    let err = mpeg_surround_decoder_config(
        mps,
        h_bs,
        core_codec,
        sampling_rate,
        frame_size,
        num_channels,
        stereo_config_index,
        core_sbr_frame_length_index,
        config_bytes,
        config_mode,
        config_changed,
    );

    match err {
        MPS_UNSUPPORTED_CONFIG => {
            /* MPS found but invalid or not decodable by this instance */
            /* We switch off MPS and keep going */
            h_aac_decoder.mps_enable_curr = 0;
            h_aac_decoder.mps_applicable = 0;
            TRANSPORTDEC_OK
        }
        MPS_PARSE_ERROR => {
            /* MPS found but invalid or not decodable by this instance */
            h_aac_decoder.mps_enable_curr = 0;
            h_aac_decoder.mps_applicable = 0;
            if (core_codec == AOT_USAC) || (core_codec == AOT_DRM_USAC) || is_lowdelay(core_codec) {
                TRANSPORTDEC_PARSE_ERROR
            } else {
                TRANSPORTDEC_OK
            }
        }
        MPS_OK => {
            h_aac_decoder.mps_applicable = 1;
            TRANSPORTDEC_OK
        }
        _ => {
            /* especially Parsing error is critical for transport layer */
            h_aac_decoder.mps_applicable = 0;
            TRANSPORTDEC_UNKOWN_ERROR
        }
    }
}

/// `aacDecoder_UniDrcCallback(handle, hBs, fullPayloadLength, payloadType, subStreamIndex,
/// payloadStart, aot)` (static in C)
pub fn aac_decoder_uni_drc_callback(
    h_aac_decoder: &mut AacDecoderInstance,
    h_bs: Option<&mut FdkBitStream>,
    full_payload_length: i32,
    payload_type: i32,
    sub_stream_index: i32,
    payload_start: i32,
    aot: AudioObjectType,
) -> i32 {
    let mut drc_dec_codec_mode = DRC_DEC_CODEC_MODE_UNDEFINED;
    let mut dummy_bs = FdkBitStream::default();

    if sub_stream_index != 0 {
        return TRANSPORTDEC_OK;
    }

    let h_read_bs: &mut FdkBitStream = match h_bs {
        None => {
            /* use dummy zero payload to clear memory */
            fdk_init_bit_stream(&mut dummy_bs, &BitMem::new(4), 4, 24, BS_READER);
            &mut dummy_bs
        }
        Some(b) => b,
    };

    if aot == AOT_USAC {
        drc_dec_codec_mode = DRC_DEC_MPEG_D_USAC;
    }

    let mut err = fdk_drc_dec_set_codec_mode(
        h_aac_decoder.h_uni_drc_decoder.as_deref_mut(),
        drc_dec_codec_mode,
    );
    if err != 0 {
        return TRANSPORTDEC_UNKOWN_ERROR;
    }

    if payload_type == 0
    /* uniDrcConfig */
    {
        err = fdk_drc_dec_read_uni_drc_config(
            h_aac_decoder.h_uni_drc_decoder.as_deref_mut(),
            h_read_bs,
        );
    } else
    /* loudnessInfoSet */
    {
        err = fdk_drc_dec_read_loudness_info_set(
            h_aac_decoder.h_uni_drc_decoder.as_deref_mut(),
            h_read_bs,
        );
        h_aac_decoder.loudness_info_set_position[1] = payload_start as u32;
        h_aac_decoder.loudness_info_set_position[2] = full_payload_length as u32;
    }

    if err == DRC_DEC_OK {
        TRANSPORTDEC_OK
    } else {
        TRANSPORTDEC_UNKOWN_ERROR
    }
}

/// `aacDecoder_AncDataInit(self, buffer, size)`: initialize the ancillary data buffer. The
/// buffer is given to the decoder (`None` for `NULL`); `size` is the C `size` argument (the
/// number of bytes the decoder may write).
pub fn aac_decoder_anc_data_init(
    self_: &mut AacDecoderInstance,
    buffer: Option<Box<[u8]>>,
    size: i32,
) -> AacDecoderError {
    crate::pipeline::sync_for_api(self_);
    let b = match buffer {
        Some(b) => AncBuffer::Owned(b),
        None => AncBuffer::None,
    };
    caac_decoder_anc_data_init(&mut self_.anc_data, b, size)
}

/// `aacDecoder_AncDataInit(self, buffer, size)` with a caller's buffer (the C ABI).
///
/// # Safety
///
/// `buffer` must be `NULL` or valid for writes of `size` bytes as long as it is registered
/// (until the next `AncDataInit` call or the decoder is closed), and it must not be accessed
/// otherwise while the decoder is used.
pub unsafe fn aac_decoder_anc_data_init_raw(
    self_: &mut AacDecoderInstance,
    buffer: *mut u8,
    size: i32,
) -> AacDecoderError {
    crate::pipeline::sync_for_api(self_);
    // SAFETY: the function's contract.
    let raw = unsafe { RawAncBuffer::new(buffer, size.max(0) as usize) };
    let b = raw.map_or(AncBuffer::None, AncBuffer::Raw);
    caac_decoder_anc_data_init(&mut self_.anc_data, b, size)
}

/// `aacDecoder_AncDataGet(self, index, ptr, size)`: get one ancillary data element. `ptr`
/// receives the offset of the element in the ancillary data buffer (`None` for `NULL`).
pub fn aac_decoder_anc_data_get(
    self_: &AacDecoderInstance,
    index: i32,
    ptr: &mut Option<usize>,
    size: &mut i32,
) -> AacDecoderError {
    caac_decoder_anc_data_get(&self_.anc_data, index, ptr, size)
}

/// The ancillary data buffer given to [`aac_decoder_anc_data_init`] (`None` for a raw or no
/// buffer).
pub fn aac_decoder_anc_data_buffer(self_: &AacDecoderInstance) -> Option<&[u8]> {
    match &self_.anc_data.buffer {
        AncBuffer::Owned(b) => Some(b),
        _ => None,
    }
}

/// `isSupportedMpsConfig(aot, numInChannels, fMpsPresent)` (static in C): if MPS is present in
/// the stream but not supported by this instance, MPS has to be switched off.
pub fn is_supported_mps_config(
    aot: AudioObjectType,
    num_in_channels: u32,
    f_mps_present: u32,
) -> i32 {
    let mut lib_info = [LibInfo::default(); FDK_MODULE_LAST as usize];
    let mut is_supported_cfg = 1;

    fdk_init_lib_info(&mut lib_info);

    mpeg_surround_decoder_get_lib_info(Some(&mut lib_info));

    let mps_caps = fdk_lib_info_get_capabilities(&lib_info, FDK_MPSDEC);

    if (mps_caps & CAPF_MPS_LD) == 0 && is_lowdelay(aot) {
        /* We got an LD AOT but MPS decoder does not support LD. */
        is_supported_cfg = 0;
    }
    if (mps_caps & CAPF_MPS_LD) != 0 && is_lowdelay(aot) && f_mps_present == 0 {
        /* We got an LD AOT and the MPS decoder supports it. But LD-MPS is not explicitly
         * signaled. */
        is_supported_cfg = 0;
    }
    if (mps_caps & CAPF_MPS_USAC) == 0 && is_usac(aot) {
        /* We got an USAC AOT but MPS decoder does not support USAC. */
        is_supported_cfg = 0;
    }
    if (mps_caps & CAPF_MPS_STD) == 0 && !is_lowdelay(aot) && !is_usac(aot) {
        /* We got an GA AOT but MPS decoder does not support it. */
        is_supported_cfg = 0;
    }
    /* Check whether the MPS modul supports the given number of input channels: */
    match num_in_channels {
        1 => {
            if (mps_caps & CAPF_MPS_1CH_IN) == 0 {
                /* We got a one channel input to MPS decoder but it does not support it. */
                is_supported_cfg = 0;
            }
        }
        2 => {
            if (mps_caps & CAPF_MPS_2CH_IN) == 0 {
                /* We got a two channel input to MPS decoder but it does not support it. */
                is_supported_cfg = 0;
            }
        }
        5 | 6 => {
            if (mps_caps & CAPF_MPS_6CH_IN) == 0 {
                /* We got a six channel input to MPS decoder but it does not support it. */
                is_supported_cfg = 0;
            }
        }
        _ => {
            is_supported_cfg = 0;
        }
    }

    is_supported_cfg
}

/// `setConcealMethod(self, method)` (static in C): changes the concealment method and the
/// bitstream delay of all modules consistently.
pub fn set_conceal_method(self_: &mut AacDecoderInstance, method: i32) -> AacDecoderError {
    let mut error_status: AacDecoderError;
    let mut method_revert = 0;
    let mut backup_method = ConcealMethodNone;
    let mut backup_delay: i32 = 0;

    'bail: {
        /* check decoder handle */
        if (self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_RSV603DA)) != 0 && method >= 2 {
            /* Interpolation concealment is not implemented for USAC/RSVD50 */
            /* errorStatus = AAC_DEC_SET_PARAM_FAIL;
            goto bail; */
            method_revert = 1;
        }
        if (self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_RSV603DA)) != 0 && method >= 2 {
            /* Interpolation concealment is not implemented for USAC/RSVD50 */
            error_status = AAC_DEC_SET_PARAM_FAIL;
            break 'bail;
        }

        /* Get current method/delay */
        backup_method = cconcealment_get_method(Some(&self_.conceal_common_data));
        backup_delay = cconcealment_get_delay(Some(&self_.conceal_common_data)) as i32;

        /* Be sure to set AAC and SBR concealment method simultaneously! */
        error_status = cconcealment_set_params(
            Some(&mut self_.conceal_common_data),
            if method_revert == 0 { method } else { 1 }, // concealMethod
            AACDEC_CONCEAL_PARAM_NOT_SPECIFIED,          // concealFadeOutSlope
            AACDEC_CONCEAL_PARAM_NOT_SPECIFIED,          // concealFadeInSlope
            AACDEC_CONCEAL_PARAM_NOT_SPECIFIED,          // concealMuteRelease
            AACDEC_CONCEAL_PARAM_NOT_SPECIFIED as FixpDbl, // concealComfNoiseLevel
        );
        if (error_status != AAC_DEC_OK) && (error_status != AAC_DEC_INVALID_HANDLE) {
            break 'bail;
        }

        /* Get new delay */
        let bs_delay = cconcealment_get_delay(Some(&self_.conceal_common_data)) as i32;

        {
            /* set SBR bitstream delay */
            let sbr_err = sbr_decoder_set_param(
                self_.h_sbr_decoder.as_deref_mut(),
                SBR_SYSTEM_BITSTREAM_DELAY,
                bs_delay,
            );

            match sbr_err {
                SBRDEC_OK | SBRDEC_NOT_INITIALIZED => {
                    /* save the param value and set later (when SBR has been initialized) */
                    self_.sbr_params.bs_delay = bs_delay;
                }
                _ => {
                    error_status = AAC_DEC_SET_PARAM_FAIL;
                    break 'bail;
                }
            }
        }

        error_status =
            aac_decoder_drc_set_param(self_.h_drc_info.as_deref_mut(), DRC_BS_DELAY, bs_delay);
        if (error_status != AAC_DEC_OK) && (error_status != AAC_DEC_INVALID_HANDLE) {
            break 'bail;
        }

        if error_status == AAC_DEC_OK {
            let err =
                pcm_dmx_set_param(self_.h_pcm_utils.as_deref_mut(), DMX_BS_DATA_DELAY, bs_delay);
            match err {
                PCMDMX_INVALID_HANDLE => {
                    error_status = AAC_DEC_INVALID_HANDLE;
                }
                PCMDMX_OK => {}
                _ => {
                    error_status = AAC_DEC_SET_PARAM_FAIL;
                    break 'bail;
                }
            }
        }
    }

    /* bail: */
    if (error_status != AAC_DEC_OK) && (error_status != AAC_DEC_INVALID_HANDLE) {
        /* Revert to the initial state */
        cconcealment_set_params(
            Some(&mut self_.conceal_common_data),
            backup_method,
            AACDEC_CONCEAL_PARAM_NOT_SPECIFIED,
            AACDEC_CONCEAL_PARAM_NOT_SPECIFIED,
            AACDEC_CONCEAL_PARAM_NOT_SPECIFIED,
            AACDEC_CONCEAL_PARAM_NOT_SPECIFIED as FixpDbl,
        );
        /* Revert SBR bitstream delay */
        sbr_decoder_set_param(
            self_.h_sbr_decoder.as_deref_mut(),
            SBR_SYSTEM_BITSTREAM_DELAY,
            backup_delay,
        );
        /* Revert DRC bitstream delay */
        aac_decoder_drc_set_param(self_.h_drc_info.as_deref_mut(), DRC_BS_DELAY, backup_delay);
        /* Revert PCM mixdown bitstream delay */
        pcm_dmx_set_param(self_.h_pcm_utils.as_deref_mut(), DMX_BS_DATA_DELAY, backup_delay);
    }

    error_status
}

/// `aacDecoder_SetParam(self, param, value)`: set one decoder parameter.
pub fn aac_decoder_set_param(
    self_: Option<&mut AacDecoderInstance>,
    param: AacdecParam,
    value: i32,
) -> AacDecoderError {
    let mut error_status = AAC_DEC_OK;
    let mut err_tp: TransportDecError = TRANSPORTDEC_OK;
    let mut dmx_err: PcmdmxError = PCMDMX_OK;
    let mut uni_drc_err: DrcDecError = DRC_DEC_OK;

    /* check decoder handle */
    let Some(self_) = self_ else {
        return AAC_DEC_INVALID_HANDLE;
    };
    // (a parameter acts between the frames of the surrounding calls, see crate::pipeline)
    crate::pipeline::sync_for_api(self_);

    'bail: {
        /* configure the subsystems */
        match param {
            AAC_PCM_MIN_OUTPUT_CHANNELS => {
                if !(-1..=8).contains(&value) {
                    return AAC_DEC_SET_PARAM_FAIL;
                }
                dmx_err = pcm_dmx_set_param(
                    self_.h_pcm_utils.as_deref_mut(),
                    MIN_NUMBER_OF_OUTPUT_CHANNELS,
                    value,
                );
            }

            AAC_PCM_MAX_OUTPUT_CHANNELS => {
                if !(-1..=8).contains(&value) {
                    return AAC_DEC_SET_PARAM_FAIL;
                }
                dmx_err = pcm_dmx_set_param(
                    self_.h_pcm_utils.as_deref_mut(),
                    MAX_NUMBER_OF_OUTPUT_CHANNELS,
                    value,
                );

                if dmx_err != PCMDMX_OK {
                    break 'bail;
                }
                error_status = aac_decoder_drc_set_param(
                    self_.h_drc_info.as_deref_mut(),
                    MAX_OUTPUT_CHANNELS,
                    value,
                );
                if value > 0 {
                    uni_drc_err = fdk_drc_dec_set_param(
                        self_.h_uni_drc_decoder.as_deref_mut(),
                        DRC_DEC_TARGET_CHANNEL_COUNT_REQUESTED,
                        value as FixpDbl,
                    );
                }
            }

            AAC_PCM_DUAL_CHANNEL_OUTPUT_MODE => {
                dmx_err = pcm_dmx_set_param(
                    self_.h_pcm_utils.as_deref_mut(),
                    DMX_DUAL_CHANNEL_MODE,
                    value,
                );
            }

            AAC_PCM_LIMITER_ENABLE => {
                if !(-2..=1).contains(&value) {
                    return AAC_DEC_SET_PARAM_FAIL;
                }
                self_.limiter_enable_user = value as u8;
            }

            AAC_PCM_LIMITER_ATTACK_TIME => {
                if value <= 0 {
                    /* module function converts value to unsigned */
                    return AAC_DEC_SET_PARAM_FAIL;
                }
                match pcm_limiter_set_attack(self_.h_limiter.as_deref_mut(), value as u32) {
                    TDLIMIT_OK => {}
                    TDLIMIT_INVALID_HANDLE => return AAC_DEC_INVALID_HANDLE,
                    _ => return AAC_DEC_SET_PARAM_FAIL,
                }
            }

            AAC_PCM_LIMITER_RELEAS_TIME => {
                if value <= 0 {
                    /* module function converts value to unsigned */
                    return AAC_DEC_SET_PARAM_FAIL;
                }
                match pcm_limiter_set_release(self_.h_limiter.as_deref_mut(), value as u32) {
                    TDLIMIT_OK => {}
                    TDLIMIT_INVALID_HANDLE => return AAC_DEC_INVALID_HANDLE,
                    _ => return AAC_DEC_SET_PARAM_FAIL,
                }
            }

            AAC_METADATA_PROFILE => {
                let mut md_expiry: i32 = -1; /* in ms (-1: don't change) */

                let dmx_profile = match value {
                    AAC_MD_PROFILE_MPEG_STANDARD => DMX_PRFL_STANDARD,
                    AAC_MD_PROFILE_MPEG_LEGACY => DMX_PRFL_MATRIX_MIX,
                    AAC_MD_PROFILE_MPEG_LEGACY_PRIO => DMX_PRFL_FORCE_MATRIX_MIX,
                    AAC_MD_PROFILE_ARIB_JAPAN => {
                        md_expiry = 550; /* ms */
                        DMX_PRFL_ARIB_JAPAN
                    }
                    _ => return AAC_DEC_SET_PARAM_FAIL,
                };
                dmx_err = pcm_dmx_set_param(
                    self_.h_pcm_utils.as_deref_mut(),
                    DMX_PROFILE_SETTING,
                    dmx_profile as i32,
                );
                if dmx_err != PCMDMX_OK {
                    break 'bail;
                }
                if md_expiry >= 0 {
                    self_.metadata_expiry = md_expiry;
                    /* Determine the corresponding number of frames and configure all related
                     * modules. */
                    aac_decoder_set_metadata_expiry(self_, md_expiry);
                }
            }

            AAC_METADATA_EXPIRY_TIME => {
                if value < 0 {
                    return AAC_DEC_SET_PARAM_FAIL;
                }
                self_.metadata_expiry = value;
                /* Determine the corresponding number of frames and configure all related
                 * modules. */
                aac_decoder_set_metadata_expiry(self_, value);
            }

            AAC_PCM_OUTPUT_CHANNEL_MAPPING => {
                if !(0..=1).contains(&value) {
                    return AAC_DEC_SET_PARAM_FAIL;
                }
                /* CAUTION: The given value must be inverted to match the logic! */
                fdk_ch_map_descr_set_pass_through(&mut self_.map_descr, (value == 0) as u32);
            }

            AAC_QMF_LOWPOWER => {
                if !(-1..=1).contains(&value) {
                    return AAC_DEC_SET_PARAM_FAIL;
                }

                /*
                 * Set QMF mode (might be overriden)
                 *  0:HQ (complex)
                 *  1:LP (partially complex)
                 */
                self_.qmf_mode_user = value as QmfMode;
            }

            AAC_DRC_ATTENUATION_FACTOR => {
                /* DRC compression factor (where 0 is no and 127 is max compression) */
                if !(0..=127).contains(&value) {
                    return AAC_DEC_SET_PARAM_FAIL;
                }
                error_status = aac_decoder_drc_set_param(
                    self_.h_drc_info.as_deref_mut(),
                    DRC_CUT_SCALE,
                    value,
                );
                uni_drc_err = fdk_drc_dec_set_param(
                    self_.h_uni_drc_decoder.as_deref_mut(),
                    DRC_DEC_COMPRESS,
                    value.wrapping_mul(fl2fxconst_dbl((0.5f32 / 127.0f32) as f64)),
                );
            }

            AAC_DRC_BOOST_FACTOR => {
                /* DRC boost factor (where 0 is no and 127 is max boost) */
                if !(0..=127).contains(&value) {
                    return AAC_DEC_SET_PARAM_FAIL;
                }
                error_status = aac_decoder_drc_set_param(
                    self_.h_drc_info.as_deref_mut(),
                    DRC_BOOST_SCALE,
                    value,
                );
                uni_drc_err = fdk_drc_dec_set_param(
                    self_.h_uni_drc_decoder.as_deref_mut(),
                    DRC_DEC_BOOST,
                    value.wrapping_mul(fl2fxconst_dbl((0.5f32 / 127.0f32) as f64)),
                );
            }

            AAC_DRC_REFERENCE_LEVEL => {
                if (value >= 0) && !(40..=127).contains(&value) {
                    /* allowed range: -10 to -31.75 dB */
                    return AAC_DEC_SET_PARAM_FAIL;
                }
                /* DRC target reference level quantized in 0.25dB steps using values [40..127].
                Negative values switch off loudness normalisation. Negative values also switch
                off MPEG-4 DRC, while MPEG-D DRC can be separately switched on/off with
                AAC_UNIDRC_SET_EFFECT */
                error_status = aac_decoder_drc_set_param(
                    self_.h_drc_info.as_deref_mut(),
                    TARGET_REF_LEVEL,
                    value,
                );
                uni_drc_err = fdk_drc_dec_set_param(
                    self_.h_uni_drc_decoder.as_deref_mut(),
                    DRC_DEC_LOUDNESS_NORMALIZATION_ON,
                    (value >= 0) as FixpDbl,
                );
                /* set target loudness also for MPEG-D DRC */
                self_.default_target_loudness = value as i8;
            }

            AAC_DRC_HEAVY_COMPRESSION => {
                /* Don't need to overwrite cut/boost values */
                error_status = aac_decoder_drc_set_param(
                    self_.h_drc_info.as_deref_mut(),
                    APPLY_HEAVY_COMPRESSION,
                    value,
                );
            }

            AAC_DRC_DEFAULT_PRESENTATION_MODE => {
                /* DRC default presentation mode */
                error_status = aac_decoder_drc_set_param(
                    self_.h_drc_info.as_deref_mut(),
                    DEFAULT_PRESENTATION_MODE,
                    value,
                );
            }

            AAC_DRC_ENC_TARGET_LEVEL => {
                /* Encoder target level for light (i.e. not heavy) compression: Target reference
                level assumed at encoder for deriving limiting gains */
                error_status = aac_decoder_drc_set_param(
                    self_.h_drc_info.as_deref_mut(),
                    ENCODER_TARGET_LEVEL,
                    value,
                );
            }

            AAC_UNIDRC_SET_EFFECT => {
                if !(-1..=6).contains(&value) {
                    return AAC_DEC_SET_PARAM_FAIL;
                }
                uni_drc_err = fdk_drc_dec_set_param(
                    self_.h_uni_drc_decoder.as_deref_mut(),
                    DRC_DEC_EFFECT_TYPE,
                    value as FixpDbl,
                );
            }
            AAC_UNIDRC_ALBUM_MODE => {
                uni_drc_err = fdk_drc_dec_set_param(
                    self_.h_uni_drc_decoder.as_deref_mut(),
                    DRC_DEC_ALBUM_MODE,
                    value as FixpDbl,
                );
            }

            AAC_TPDEC_CLEAR_BUFFER => {
                if let Some(tp) = self_.h_input.as_deref_mut() {
                    err_tp = transport_dec_set_param(tp, TPDEC_PARAM_RESET, 1);
                }
                self_.stream_info.num_lost_access_units = 0;
                self_.stream_info.num_bad_bytes = 0;
                self_.stream_info.num_total_bytes = 0;
                /* aacDecoder_SignalInterruption(self); */
            }
            AAC_CONCEAL_METHOD => {
                /* Changing the concealment method can introduce additional bitstream delay.
                And that in turn affects sub libraries and modules which makes the whole thing
                quite complex.  So the complete changing routine is packed into a helper function
                which keeps all modules and libs in a consistent state even in the case an error
                occures. */
                error_status = set_conceal_method(self_, value);
                if error_status == AAC_DEC_OK {
                    self_.conceal_method_user = value;
                }
            }

            _ => return AAC_DEC_SET_PARAM_FAIL,
        } /* switch(param) */
    }

    /* bail: */

    if error_status == AAC_DEC_OK {
        /* Check error code returned by DMX module library: */
        match dmx_err {
            PCMDMX_OK => {}
            PCMDMX_INVALID_HANDLE => error_status = AAC_DEC_INVALID_HANDLE,
            _ => error_status = AAC_DEC_SET_PARAM_FAIL,
        }
    }

    if err_tp != TRANSPORTDEC_OK && error_status == AAC_DEC_OK {
        error_status = AAC_DEC_SET_PARAM_FAIL;
    }

    if error_status == AAC_DEC_OK {
        /* Check error code returned by MPEG-D DRC decoder library: */
        match uni_drc_err {
            0 => {}
            -9998 => error_status = AAC_DEC_INVALID_HANDLE,
            _ => error_status = AAC_DEC_SET_PARAM_FAIL,
        }
    }

    error_status
}

/// `aacDecoder_Open(transportFmt, nrOfLayers)`: open an AAC decoder instance. Returns `None`
/// (`NULL`) on failure.
pub fn aac_decoder_open(
    transport_fmt: TransportType,
    nr_of_layers: u32,
) -> Option<Box<AacDecoderInstance>> {
    let stereo_config_index: i32 = -1;

    let nr_of_layers_min = nr_of_layers.min(1);

    /* Allocate transport layer struct. */
    let mut p_in = transport_dec_open(transport_fmt, TP_FLAG_MPEG4, nr_of_layers_min)?;

    transport_dec_set_param(&mut p_in, TPDEC_PARAM_IGNORE_BUFFERFULLNESS, 1);

    /* Allocate AAC decoder core struct. */
    let Some(mut aac_dec) = caac_decoder_open(transport_fmt) else {
        let mut p = Some(p_in);
        transport_dec_close(&mut p);
        return None;
    };
    aac_dec.h_input = Some(p_in);

    aac_dec.nr_of_layers = nr_of_layers_min as i32;

    /* Setup channel mapping descriptor. */
    fdk_ch_map_descr_init(&mut aac_dec.map_descr, None, 0, 0);

    /* Register Config Update callback, Free Memory callback, config switch control callback:
     * the transport decoder calls them through AacDecoderCallbacks. */

    aac_dec.qmf_domain = FdkQmfDomain::default();
    /* open SBR decoder */
    let mut err = 0;
    'bail: {
        if SBRDEC_OK != sbr_decoder_open(&mut aac_dec.h_sbr_decoder) {
            err = -1;
            break 'bail;
        }
        aac_dec.qmf_mode_user = NOT_DEFINED;

        if mpeg_surround_decoder_open(
            &mut aac_dec.p_mpeg_surround_decoder,
            stereo_config_index,
            Some(&mut aac_dec.qmf_domain),
        ) != 0
        {
            err = -1;
            break 'bail;
        }
        /* Set MPEG Surround defaults */
        aac_dec.mps_enable_user = 0;
        aac_dec.mps_enable_curr = 0;
        aac_dec.mps_applicable = 0;
        aac_dec.mps_output_mode = SACDEC_OUT_MODE_NORMAL as i8;

        if fdk_drc_dec_open(&mut aac_dec.h_uni_drc_decoder, DRC_DEC_ALL) != 0 {
            err = -1;
            break 'bail;
        }

        aac_dec.default_target_loudness = 96;

        pcm_dmx_open(Some(&mut aac_dec.h_pcm_utils));
        if aac_dec.h_pcm_utils.is_none() {
            err = -1;
            break 'bail;
        }

        aac_dec.h_limiter =
            pcm_limiter_create(TDL_ATTACK_DEFAULT_MS, TDL_RELEASE_DEFAULT_MS, MAXVAL_DBL, 8, 96000);
        if aac_dec.h_limiter.is_none() {
            err = -1;
            break 'bail;
        }
        aac_dec.limiter_enable_user = -1i8 as u8;
        aac_dec.limiter_enable_curr = 0;

        /* Assure that all modules have same delay */
        let method = cconcealment_get_method(Some(&aac_dec.conceal_common_data));
        if set_conceal_method(&mut aac_dec, method) != 0 {
            err = -1;
            break 'bail;
        }
    }

    /* bail: */
    if err == -1 {
        aac_decoder_close(Some(aac_dec));
        return None;
    }
    Some(aac_dec)
}

/// `aacDecoder_Fill(self, pBuffer[], bufferSize[], pBytesValid[])`: fill the internal input
/// buffer with bitstream data, one buffer per layer. `p_bytes_valid` holds the number of bytes
/// not yet consumed of each buffer (in and out, as in C).
pub fn aac_decoder_fill(
    self_: &mut AacDecoderInstance,
    p_buffer: &[&[u8]],
    buffer_size: &[u32],
    p_bytes_valid: &mut [u32],
) -> AacDecoderError {
    /* loop counter for layers; if not TT_MP4_RAWPACKETS used as index for only available layer */
    let nr_of_layers = self_.nr_of_layers;
    crate::pipeline::note_fill(self_);

    for layer in 0..nr_of_layers.max(0) as usize {
        let buf: &[u8] = p_buffer.get(layer).copied().unwrap_or(&[]);
        let size = buffer_size.get(layer).copied().unwrap_or(0);
        let mut valid = p_bytes_valid.get(layer).copied().unwrap_or(0);
        let tp_err = match self_.h_input.as_deref_mut() {
            Some(tp) => transport_dec_fill_data(tp, buf, size, &mut valid, layer as i32),
            None => TRANSPORTDEC_UNKOWN_ERROR,
        };
        if let Some(v) = p_bytes_valid.get_mut(layer) {
            *v = valid;
        }
        if tp_err != TRANSPORTDEC_OK {
            return AAC_DEC_UNKNOWN; /* Must be an internal error */
        }
    }

    AAC_DEC_OK
}

/// `aacDecoder_SignalInterruption(self)` (static in C)
pub fn aac_decoder_signal_interruption(self_: &mut AacDecoderInstance) {
    caac_decoder_signal_interruption(self_);

    if self_.h_sbr_decoder.is_some() {
        sbr_decoder_set_param(self_.h_sbr_decoder.as_deref_mut(), SBR_BS_INTERRUPTION, 1);
    }
    if self_.mps_enable_user != 0 {
        mpeg_surround_decoder_set_param(
            self_.p_mpeg_surround_decoder.as_deref_mut(),
            SACDEC_BS_INTERRUPTION,
            1,
        );
    }
}

/// `aacDecoder_UpdateBitStreamCounters(pSi, hBs, nBits, ErrorStatus)` (static in C)
pub fn aac_decoder_update_bit_stream_counters(
    p_si: &mut CStreamInfo,
    h_bs: &mut FdkBitStream,
    n_bits: i32,
    error_status: AacDecoderError,
) {
    /* calculate bit difference (amount of bits moved forward) */
    let n_bits = n_bits.wrapping_sub(fdk_get_valid_bits(h_bs) as i32);

    /* Note: The amount of bits consumed might become negative when parsing a bit stream with
    several sub frames, and we find out at the last sub frame that the total frame length does
    not match the sum of sub frame length. If this happens, the transport decoder might want to
    rewind to the supposed ending of the transport frame, and this position might be before the
    last access unit beginning. */

    /* Calc bitrate. */
    if p_si.frame_size > 0 {
        /* bitRate = nBits * sampleRate / frameSize */
        let mut ratio_e: i32 = 0;
        let ratio_m = f_div_norm(p_si.sample_rate, p_si.frame_size, &mut ratio_e);
        p_si.bit_rate = f_mult_norm_to(n_bits, DFRACT_BITS - 1, ratio_m, ratio_e, DFRACT_BITS - 1);
    }

    /* bit/byte counters */
    {
        let n_bytes = n_bits >> 3;
        p_si.num_total_bytes = p_si.num_total_bytes.wrapping_add(n_bytes as i64);
        if is_output_valid(error_status) != 0 {
            p_si.num_total_access_units += 1;
        }
        if is_decode_error(error_status) != 0 {
            p_si.num_bad_bytes = p_si.num_bad_bytes.wrapping_add(n_bytes as i64);
            p_si.num_bad_access_units += 1;
        }
    }
}

/// `aacDecoder_EstimateNumberOfLostFrames(self)` (static in C)
pub fn aac_decoder_estimate_number_of_lost_frames(self_: &AacDecoderInstance) -> i32 {
    let mut n: i32 = 0;

    if let Some(tp) = self_.h_input.as_deref() {
        transport_dec_get_missing_access_unit_count(&mut n, tp);
    }

    n
}

/// `(INT_PCM *)workBufferCore2`: the decoder's `workBufferCore2` (`FIXP_DBL`) seen as 16-bit
/// PCM samples, in the memory order of the (little-endian) reference.
pub(crate) fn as_int_pcm(buf: &mut [FixpDbl]) -> &mut [IntPcm] {
    // SAFETY: `buf` is a slice of `i32`; every byte pattern is a valid `i16`, `i16` has a smaller
    // alignment than `i32`, and the returned slice (twice the length) covers exactly the same
    // memory and borrows `buf` mutably, so no other access can overlap it.
    unsafe { core::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut IntPcm, buf.len() * 2) }
}

/// `aacDecoder_DecodeFrame(self, pTimeData, timeDataSize, flags)`: decode one audio frame into
/// `p_time_data` (interleaved 16-bit PCM). `time_data_size` is the size of the output buffer in
/// samples; when the call returns no valid output, the first `time_data_size` samples are set
/// to 0 (as far as `p_time_data` reaches).
pub fn aac_decoder_decode_frame(
    self_: Option<&mut AacDecoderInstance>,
    p_time_data: &mut [IntPcm],
    time_data_size: i32,
    flags: u32,
) -> AacDecoderError {
    let Some(self_) = self_ else {
        return AAC_DEC_INVALID_HANDLE;
    };

    if self_.pipe.is_some() {
        // More than one thread: frame pipelining (crate::pipeline, opt-in).
        return crate::pipeline::decode_frame(self_, p_time_data, time_data_size, flags);
    }

    // pTimeData2 is a member of the instance; it is taken out while the frame is decoded so
    // that it can be passed next to the instance (CAacDecoder_DecodeFrame() gets it as an
    // argument and never reads self->pTimeData2).
    let mut td2 = self_.p_time_data2.take().unwrap_or_default();
    let error_status = decode_frame_lib(self_, p_time_data, time_data_size, flags, &mut td2);
    self_.p_time_data2 = Some(td2);

    if is_output_valid(error_status) == 0 {
        let n = (time_data_size.max(0) as usize).min(p_time_data.len());
        p_time_data[..n].fill(0);
    }

    error_status
}

/// The body of `aacDecoder_DecodeFrame()` up to the final buffer clearing.
#[allow(clippy::cognitive_complexity)]
pub(crate) fn decode_frame_lib(
    self_: &mut AacDecoderInstance,
    p_time_data: &mut [IntPcm],
    time_data_size: i32,
    flags: u32,
    td2: &mut [PcmDec],
) -> AacDecoderError {
    let mut f_tp_interruption = 0; /* Transport originated interruption detection. */
    let mut f_tp_conceal = 0; /* Transport originated concealment. */

    if flags & AACDEC_INTR != 0 {
        self_.stream_info.num_lost_access_units = 0;
    }

    /* Get current bits position for bitrate calculation. */
    let n_bits: i32 = match self_.h_input.as_deref_mut() {
        Some(tp) => fdk_get_valid_bits(&mut tp.bit_stream[0]) as i32,
        None => 0,
    };

    if flags & AACDEC_CLRHIST != 0 && self_.flags[0] & AC_USAC != 0 {
        /* 1) store AudioSpecificConfig always in AudioSpecificConfig_Parse() */
        /* 2) free memory of dynamic allocated data */
        let mut asc = CSAudioSpecificConfig::zeroed();
        if let Some(tp) = self_.h_input.as_deref() {
            transport_dec_get_asc(tp, 0, &mut asc);
        }
        aac_decoder_free_mem_callback(self_, &asc);
        self_.stream_info.num_channels = 0;
        /* 3) restore AudioSpecificConfig */
        if asc.config_bits <= (TP_USAC_MAX_CONFIG_LEN as u32) << 3 {
            let cfg = asc.config;
            let len = (asc.config_bits + 7) >> 3;
            self_.with_tp_cb(|tp, cb| transport_dec_out_of_band_config_cb(tp, &cfg, len, 0, cb));
        }
    }

    if let Err(error_status) =
        lib_read_access_unit(self_, flags, &mut f_tp_interruption, &mut f_tp_conceal)
    {
        return lib_tail(self_, n_bits, error_status, 0, 0, 0);
    }
    decode_frame_lib_after_read(
        self_,
        p_time_data,
        time_data_size,
        flags,
        td2,
        n_bits,
        f_tp_interruption,
        f_tp_conceal,
    )
}

/// The transport decoder part of `aacDecoder_DecodeFrame()`: reads the access unit (or counts
/// down the lost access units). `Err` is the return value of a call that ends here.
pub(crate) fn lib_read_access_unit(
    self_: &mut AacDecoderInstance,
    flags: u32,
    f_tp_interruption: &mut i32,
    f_tp_conceal: &mut i32,
) -> Result<(), AacDecoderError> {
    if !((flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) != 0
        || (self_.flush_status == AACDEC_RSV60_DASH_IPF_ATSC_FLUSH_ON)
        || (self_.flush_status == AACDEC_USAC_DASH_IPF_FLUSH_ON)
        || (self_.build_up_status == AACDEC_RSV60_BUILD_UP_IDLE_IN_BAND))
    {
        for layer in 0..self_.nr_of_layers.max(0) as u32 {
            let err = self_
                .with_tp_cb(|tp, cb| transport_dec_read_access_unit_cb(tp, layer, cb))
                .unwrap_or(TRANSPORTDEC_UNKOWN_ERROR);
            if err != TRANSPORTDEC_OK {
                match err {
                    TRANSPORTDEC_NOT_ENOUGH_BITS => {
                        return Err(AAC_DEC_NOT_ENOUGH_BITS);
                    }
                    TRANSPORTDEC_SYNC_ERROR => {
                        self_.stream_info.num_lost_access_units =
                            aac_decoder_estimate_number_of_lost_frames(self_);
                        *f_tp_interruption = 1;
                    }
                    TRANSPORTDEC_NEED_TO_RESTART => {
                        return Err(AAC_DEC_NEED_TO_RESTART);
                    }
                    TRANSPORTDEC_CRC_ERROR => {
                        *f_tp_conceal = 1;
                    }
                    TRANSPORTDEC_UNSUPPORTED_FORMAT => {
                        return Err(AAC_DEC_UNSUPPORTED_FORMAT);
                    }
                    _ => {
                        return Err(AAC_DEC_UNKNOWN);
                    }
                }
            }
        }
    } else if self_.stream_info.num_lost_access_units > 0 {
        self_.stream_info.num_lost_access_units -= 1;
    }
    Ok(())
}

/// `aacDecoder_DecodeFrame()` after the transport decoder part ([`lib_read_access_unit`]).
#[allow(clippy::cognitive_complexity, clippy::too_many_arguments)]
pub(crate) fn decode_frame_lib_after_read(
    self_: &mut AacDecoderInstance,
    p_time_data: &mut [IntPcm],
    time_data_size: i32,
    flags: u32,
    td2: &mut [PcmDec],
    n_bits: i32,
    f_tp_interruption: i32,
    f_tp_conceal: i32,
) -> AacDecoderError {
    let mut error_status: AacDecoderError;
    let mut access_unit: u32 = 0;
    let mut num_access_units: u32 = 1;
    let mut num_preroll_au: u32 = 0;
    let mut f_end_au_not_adjusted = 0; /* The end of the access unit was not adjusted */
    let mut apply_crossfade = 1; /* flag indicates if flushing was possible */
    let mut pcm_limiter_scale: i32 = 0;
    let mut interleaved: i32 = 0;

    let mut preroll_au_offset = [0u32; AACDEC_MAX_NUM_PREROLL_AU];
    let mut preroll_au_length = [0u32; AACDEC_MAX_NUM_PREROLL_AU];

    'bail: {
        self_.frame_ok = 1;

        for i in 0..AACDEC_MAX_NUM_PREROLL_AU + 1 {
            self_.preroll_au_length[i] = 0;
        }

        /* Process preroll frames and current frame */
        loop {
            if (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0
                && (self_.flush_status != AACDEC_RSV60_CFG_CHANGE_ATSC_FLUSH_ON)
                && (access_unit == 0)
                && (self_.has_audio_pre_roll != 0
                    || (self_.build_up_status == AACDEC_RSV60_BUILD_UP_IDLE_IN_BAND))
                && f_tp_interruption == 0
                && f_tp_conceal == 0
            /* Bit stream pointer needs to be at the beginning of a (valid) AU. */
            {
                error_status = caac_decoder_pre_roll_extension_payload_parse(
                    self_,
                    &mut num_preroll_au,
                    &mut preroll_au_offset,
                    &mut preroll_au_length,
                );

                if error_status != AAC_DEC_OK {
                    match error_status {
                        AAC_DEC_NOT_ENOUGH_BITS => break 'bail,
                        AAC_DEC_PARSE_ERROR => {
                            self_.frame_ok = 0;
                        }
                        _ => {}
                    }
                }

                num_access_units = num_access_units.wrapping_add(num_preroll_au);
            }

            let au_start_anchor: i32 = match self_.h_input.as_deref_mut() {
                Some(tp) => fdk_get_valid_bits(&mut tp.bit_stream[0]) as i32,
                None => 0,
            };

            self_.access_unit = access_unit as i32;
            if access_unit < num_preroll_au {
                if let Some(tp) = self_.h_input.as_deref_mut() {
                    fdk_push_for(&mut tp.bit_stream[0], preroll_au_offset[access_unit as usize]);
                }
            }

            /* Signal bit stream interruption to other modules if required. */
            if f_tp_interruption != 0 || ((flags & AACDEC_INTR) != 0 && (access_unit == 0)) {
                aac_decoder_signal_interruption(self_);
                if !((flags & AACDEC_INTR) != 0 && (access_unit == 0)) {
                    error_status = AAC_DEC_TRANSPORT_SYNC_ERROR;
                    break 'bail;
                }
            }

            /* Clearing core data will be done in CAacDecoder_DecodeFrame() below. Tell other
            modules to clear states if required. */
            if flags & AACDEC_CLRHIST != 0 && self_.flags[0] & AC_USAC == 0 {
                sbr_decoder_set_param(self_.h_sbr_decoder.as_deref_mut(), SBR_CLEAR_HISTORY, 1);
                mpeg_surround_decoder_set_param(
                    self_.p_mpeg_surround_decoder.as_deref_mut(),
                    SACDEC_CLEAR_HISTORY,
                    1,
                );
                if fdk_qmf_domain_clear_persistent_memory(Some(&mut self_.qmf_domain)) != 0 {
                    error_status = AAC_DEC_UNKNOWN;
                    break 'bail;
                }
            }

            /* Empty bit buffer in case of flush request. */
            if (flags & AACDEC_FLUSH) != 0
                && (flags & AACDEC_CONCEAL) == 0
                && self_.flush_status == 0
            {
                if let Some(tp) = self_.h_input.as_deref_mut() {
                    transport_dec_set_param(tp, TPDEC_PARAM_RESET, 1);
                }
                self_.stream_info.num_lost_access_units = 0;
                self_.stream_info.num_bad_bytes = 0;
                self_.stream_info.num_total_bytes = 0;
            }
            /* Reset the output delay field. The modules will add their figures one after
             * another. */
            self_.stream_info.output_delay = 0;

            if self_.limiter_enable_user == -2i8 as u8 {
                /* Enable limiter only for RSVD60. */
                self_.limiter_enable_curr = ((self_.flags[0] & AC_RSV603DA) != 0) as u8;
            } else if self_.limiter_enable_user == -1i8 as u8 {
                /* Enable limiter for all non-lowdelay AOT's. */
                self_.limiter_enable_curr = ((self_.flags[0] & (AC_LD | AC_ELD)) == 0) as u8;
            } else {
                /* Use limiter configuration as requested. */
                self_.limiter_enable_curr = self_.limiter_enable_user;
            }

            /* reset DRC level normalization gain on a per frame basis */
            self_.ext_gain[0] = AACDEC_DRC_GAIN_INIT_VALUE;

            let time_data2_size: i32 =
                self_.time_data2_size / core::mem::size_of::<PcmDec>() as i32;
            let time_data3_size: i32 = self_.time_data2_size / core::mem::size_of::<i32>() as i32;

            let core_flags = flags
                | (if f_tp_conceal != 0 { AACDEC_CONCEAL } else { 0 })
                | (if self_.flush_status != 0 && (flags & AACDEC_CONCEAL) == 0 {
                    AACDEC_FLUSH
                } else {
                    0
                });
            let spf = self_.stream_info.aac_samples_per_frame;
            error_status = caac_decoder_decode_frame(self_, core_flags, td2, time_data2_size, spf);

            let mut time_data_headroom = self_.aac_out_data_headroom;

            /* if flushing for USAC DASH IPF was not possible go on with decoding preroll */
            if (self_.flags[0] & AC_USAC) != 0
                && (self_.flush_status == AACDEC_USAC_DASH_IPF_FLUSH_ON)
                && (flags & AACDEC_CONCEAL) == 0
                && (error_status != AAC_DEC_OK)
            {
                apply_crossfade = 0;
            } else
            /* USAC DASH IPF flushing possible begin */
            {
                if !((flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) != 0
                    || f_tp_conceal != 0
                    || self_.flush_status != 0)
                    && (is_output_valid(error_status) == 0 || access_unit >= num_preroll_au)
                {
                    let tp_err = match self_.h_input.as_deref_mut() {
                        Some(tp) => transport_dec_end_access_unit(tp),
                        None => TRANSPORTDEC_UNKOWN_ERROR,
                    };
                    if tp_err != TRANSPORTDEC_OK {
                        self_.frame_ok = 0;
                    }
                } else {
                    /* while preroll processing later possibly an error in the renderer part
                     * occurrs */
                    if is_output_valid(error_status) != 0 {
                        f_end_au_not_adjusted = 1;
                    }
                }

                /* If the current pTimeData2 does not contain a valid signal, there nothing else
                 * we can do, so bail. */
                if is_output_valid(error_status) == 0 {
                    break 'bail;
                }

                {
                    self_.stream_info.sample_rate = self_.stream_info.aac_sample_rate;
                    self_.stream_info.frame_size = self_.stream_info.aac_samples_per_frame;
                }

                self_.stream_info.num_channels = self_.stream_info.aac_num_channels;

                {
                    let off =
                        (self_.stream_info.aac_samples_per_frame.max(0) as usize).min(td2.len());
                    fdk_delay_apply(
                        &mut self_.usac_residual_delay,
                        &mut td2[off..],
                        self_.stream_info.frame_size as u32,
                        0,
                    );
                }

                /* Setting of internal MPS state; may be reset in CAacDecoder_SyncQmfMode if
                decoder is unable to decode with user defined qmfMode */
                if self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_RSV603DA | AC_ELD) == 0 {
                    self_.mps_enable_curr = (self_.mps_enable_user != 0
                        && is_supported_mps_config(
                            self_.stream_info.aot,
                            self_.stream_info.num_channels as u32,
                            ((self_.flags[0] & AC_MPS_PRESENT) != 0) as u32,
                        ) != 0) as u8;
                }

                if self_.qmf_domain.global_conf.qmf_domain_explicit_config == 0
                    && self_.mps_enable_curr != 0
                {
                    /* if not done yet, allocate full MPEG Surround decoder instance */
                    if mps_is_full_instance_available(self_.p_mpeg_surround_decoder.as_deref())
                        == SAC_INSTANCE_NOT_FULL_AVAILABLE
                        && mpeg_surround_decoder_open(
                            &mut self_.p_mpeg_surround_decoder,
                            -1,
                            Some(&mut self_.qmf_domain),
                        ) != 0
                    {
                        return AAC_DEC_OUT_OF_MEMORY;
                    }
                }

                caac_decoder_sync_qmf_mode(self_);

                if self_.qmf_domain.global_conf.qmf_domain_explicit_config == 0
                    && self_.mps_enable_curr != 0
                {
                    let sac_interface = if self_.sbr_enabled != 0 && self_.h_sbr_decoder.is_some() {
                        SAC_INTERFACE_QMF
                    } else {
                        SAC_INTERFACE_TIME
                    };
                    /* needs to be done before first SBR apply. */
                    mpeg_surround_decoder_configure_qmf_domain(
                        self_.p_mpeg_surround_decoder.as_deref_mut(),
                        &mut self_.qmf_domain,
                        sac_interface,
                        self_.stream_info.aac_sample_rate as u32,
                        self_.stream_info.aot,
                    );
                    let gc = &mut self_.qmf_domain.global_conf;
                    if gc.n_bands_analysis_requested > 0 {
                        gc.n_qmf_time_slots_requested = (self_.stream_info.aac_samples_per_frame
                            / gc.n_bands_analysis_requested as i32)
                            as u8;
                    } else {
                        gc.n_qmf_time_slots_requested = 0;
                    }
                }

                match fdk_qmf_domain_configure(&mut self_.qmf_domain) {
                    QMF_DOMAIN_OK => {}
                    QMF_DOMAIN_OUT_OF_MEMORY => {
                        error_status = AAC_DEC_OUT_OF_MEMORY;
                        break 'bail;
                    }
                    /* QMF_DOMAIN_INIT_ERROR and default */
                    _ => {
                        let _ = QMF_DOMAIN_INIT_ERROR;
                        error_status = AAC_DEC_UNKNOWN;
                        break 'bail;
                    }
                }

                /* sbr decoder */

                if (error_status != AAC_DEC_OK)
                    || (flags & AACDEC_CONCEAL) != 0
                    || self_.p_aac_decoder_static_channel_info[0]
                        .as_deref()
                        .map_or(0, |s| s.concealment_info.conceal_state)
                        > ConcealState_FadeIn
                {
                    self_.frame_ok = 0; /* if an error has occured do concealment in the SBR
                                        decoder too */
                }

                if self_.sbr_enabled != 0 && (self_.flags[0] & AC_USAC_SCFGI3) == 0 {
                    let num_core_channel = self_.stream_info.num_channels;

                    /* set params */
                    sbr_decoder_set_param(
                        self_.h_sbr_decoder.as_deref_mut(),
                        SBR_SYSTEM_BITSTREAM_DELAY,
                        self_.sbr_params.bs_delay,
                    );
                    sbr_decoder_set_param(
                        self_.h_sbr_decoder.as_deref_mut(),
                        SBR_FLUSH_DATA,
                        ((flags & AACDEC_FLUSH)
                            | if self_.flush_status != 0 && (flags & AACDEC_CONCEAL) == 0 {
                                AACDEC_FLUSH
                            } else {
                                0
                            }) as i32,
                    );

                    if self_.stream_info.aot == AOT_ER_AAC_ELD {
                        /* Configure QMF */
                        sbr_decoder_set_param(
                            self_.h_sbr_decoder.as_deref_mut(),
                            SBR_LD_QMF_TIME_ALIGN,
                            ((self_.flags[0] & AC_MPS_PRESENT) != 0) as i32,
                        );
                    }

                    {
                        let mut max_out_ch: i32 = 0;

                        let dmx_err = pcm_dmx_get_param(
                            self_.h_pcm_utils.as_deref(),
                            MAX_NUMBER_OF_OUTPUT_CHANNELS,
                            Some(&mut max_out_ch),
                        );
                        if (dmx_err == PCMDMX_OK) && (max_out_ch == 1) {
                            /* Disable PS processing if we have to create a mono output signal. */
                            self_.ps_possible = 0;
                        }
                    }

                    sbr_decoder_set_param(
                        self_.h_sbr_decoder.as_deref_mut(),
                        SBR_SKIP_QMF,
                        if self_.mps_enable_curr != 0 { 2 } else { 0 },
                    );

                    let n =
                        (self_.stream_info.num_channels.wrapping_mul(self_.stream_info.frame_size))
                            .max(0) as usize;
                    let wb2 = self_.work_buffer_core2.get_or_insert_with(Vec::new);
                    let n = n.min(wb2.len()).min(td2.len());
                    wb2[..n].copy_from_slice(&td2[..n]);

                    /* apply SBR processing */
                    // (not in C: a decoder with more than one thread decodes the channels of a
                    // stereo element on two threads, see crate::pipeline::chan)
                    let cap = self_.pipe.as_deref().map_or(1, |p| p.inline_cap as usize);
                    let par = if cap > 1 {
                        crate::pipeline::chan::sbr_par(&mut self_.chan_par, cap)
                    } else {
                        None
                    };
                    let sbr_error = match self_.h_sbr_decoder.as_deref_mut() {
                        Some(sbr) => sbr_decoder_apply_pool(
                            sbr,
                            &mut self_.qmf_domain,
                            wb2,
                            td2,
                            time_data3_size,
                            &mut self_.stream_info.num_channels,
                            &mut self_.stream_info.sample_rate,
                            Some(&self_.map_descr),
                            self_.ch_map_index as i32,
                            self_.frame_ok as i32,
                            &mut self_.ps_possible,
                            self_.aac_out_data_headroom,
                            &mut time_data_headroom,
                            crate::pipeline::chan::as_sbr_par(&par),
                        ),
                        // (sbrDecoder_Apply() with a NULL handle)
                        None => SBRDEC_INVALID_ARGUMENT,
                    };

                    if sbr_error == SBRDEC_OK {
                        /* Update data in streaminfo structure. Assume that the SBR upsampling
                        factor is either 1, 2, 8/3 or 4. Maximum upsampling factor is 4 (CELP+SBR
                        or USAC 4:1 SBR) */
                        self_.flags[0] |= AC_SBR_PRESENT;
                        let si = &mut self_.stream_info;
                        if si.aac_sample_rate != si.sample_rate {
                            if si.aac_sample_rate >> 2 == si.sample_rate {
                                si.frame_size = si.aac_samples_per_frame >> 2;
                                si.output_delay >>= 2;
                            } else if si.aac_sample_rate >> 1 == si.sample_rate {
                                si.frame_size = si.aac_samples_per_frame >> 1;
                                si.output_delay >>= 1;
                            } else if si.aac_sample_rate.wrapping_shl(1) == si.sample_rate {
                                si.frame_size = si.aac_samples_per_frame << 1;
                                si.output_delay <<= 1;
                            } else if si.aac_sample_rate.wrapping_shl(2) == si.sample_rate {
                                si.frame_size = si.aac_samples_per_frame << 2;
                                si.output_delay <<= 2;
                            } else if si.frame_size == 768 {
                                si.frame_size = (si.aac_samples_per_frame << 3) / 3;
                                si.output_delay = (si.output_delay << 3) / 3;
                            } else {
                                error_status = AAC_DEC_SET_PARAM_FAIL;
                                break 'bail;
                            }
                        } else {
                            si.frame_size = si.aac_samples_per_frame;
                        }
                        self_.stream_info.output_delay = self_
                            .stream_info
                            .output_delay
                            .wrapping_add(sbr_decoder_get_delay(self_.h_sbr_decoder.as_deref()));

                        if self_.ps_possible != 0 {
                            self_.flags[0] |= AC_PS_PRESENT;
                        }
                        for ch_idx in num_core_channel..self_.stream_info.num_channels {
                            if (0..8).contains(&ch_idx) {
                                self_.stream_info.p_channel_type[ch_idx as usize] = ACT_FRONT;
                                self_.stream_info.p_channel_indices[ch_idx as usize] = ch_idx as u8;
                            }
                        }
                    }
                    if sbr_error == SBRDEC_OUTPUT_BUFFER_TOO_SMALL {
                        error_status = AAC_DEC_OUTPUT_BUFFER_TOO_SMALL;
                        break 'bail;
                    }
                }

                if self_.mps_enable_curr != 0 {
                    let mut n_channels = self_.stream_info.num_channels;
                    let mut frame_size = self_.stream_info.frame_size;
                    let mut sac_interface = SAC_INTERFACE_TIME;

                    if self_.sbr_enabled != 0 && self_.h_sbr_decoder.is_some() {
                        sac_interface = SAC_INTERFACE_QMF;
                    }
                    if self_.stream_info.aot == AOT_USAC && (self_.flags[0] & AC_USAC_SCFGI3) != 0 {
                        sac_interface = SAC_INTERFACE_TIME;
                    }
                    let mut err = mpeg_surround_decoder_set_param(
                        self_.p_mpeg_surround_decoder.as_deref_mut(),
                        SACDEC_INTERFACE,
                        sac_interface,
                    );

                    if err == 0 {
                        let wb2 = self_.work_buffer_core2.get_or_insert_with(Vec::new);
                        err = mpeg_surround_decoder_apply(
                            self_.p_mpeg_surround_decoder.as_deref_mut(),
                            &mut self_.qmf_domain,
                            wb2,
                            td2,
                            time_data3_size,
                            self_.stream_info.aac_samples_per_frame,
                            &mut n_channels,
                            &mut frame_size,
                            self_.stream_info.sample_rate,
                            self_.stream_info.aot,
                            Some(&mut self_.stream_info.p_channel_type),
                            Some(&mut self_.stream_info.p_channel_indices),
                            Some(&self_.map_descr),
                            self_.aac_out_data_headroom,
                            &mut time_data_headroom,
                        );
                    }

                    if err == MPS_OUTPUT_BUFFER_TOO_SMALL {
                        error_status = AAC_DEC_OUTPUT_BUFFER_TOO_SMALL;
                        break 'bail;
                    }
                    if err == 0 {
                        /* Update output parameter */
                        self_.stream_info.num_channels = n_channels;
                        self_.stream_info.frame_size = frame_size;
                        self_.stream_info.output_delay = self_
                            .stream_info
                            .output_delay
                            .wrapping_add(mpeg_surround_decoder_get_delay(
                                self_.p_mpeg_surround_decoder.as_deref(),
                            ));
                        /* Save current parameter for possible concealment of next frame */
                        self_.mps_out_channels_last = n_channels;
                        self_.mps_frame_size_last = frame_size;
                    } else if (self_.mps_out_channels_last > 0) && (self_.mps_frame_size_last > 0) {
                        /* Restore parameters of last frame ... */
                        self_.stream_info.num_channels = self_.mps_out_channels_last;
                        self_.stream_info.frame_size = self_.mps_frame_size_last;
                        /* ... and clear output buffer so that potentially corrupted data does
                         * not reach the framework. */
                        let n =
                            (self_.mps_out_channels_last.wrapping_mul(self_.mps_frame_size_last))
                                .max(0) as usize;
                        let n = n.min(td2.len());
                        td2[..n].fill(0);
                        /* Additionally proclaim that this frame had errors during decoding. */
                        error_status = AAC_DEC_DECODE_FRAME_ERROR;
                    } else {
                        error_status = AAC_DEC_UNKNOWN; /* no output */
                    }
                }

                /* SBR decoder for Unified Stereo Config (stereoConfigIndex == 3) */

                if self_.sbr_enabled != 0 && (self_.flags[0] & AC_USAC_SCFGI3) != 0 {
                    /* set params */
                    sbr_decoder_set_param(
                        self_.h_sbr_decoder.as_deref_mut(),
                        SBR_SYSTEM_BITSTREAM_DELAY,
                        self_.sbr_params.bs_delay,
                    );
                    sbr_decoder_set_param(
                        self_.h_sbr_decoder.as_deref_mut(),
                        SBR_FLUSH_DATA,
                        ((flags & AACDEC_FLUSH)
                            | if self_.flush_status != 0 && (flags & AACDEC_CONCEAL) == 0 {
                                AACDEC_FLUSH
                            } else {
                                0
                            }) as i32,
                    );

                    sbr_decoder_set_param(self_.h_sbr_decoder.as_deref_mut(), SBR_SKIP_QMF, 1);

                    /* apply SBR processing (input == timeData == pTimeData3 in C; with
                    SBR_SKIP_QMF the SBR decoder does not read the input: an empty slice) */
                    let sbr_error = match self_.h_sbr_decoder.as_deref_mut() {
                        Some(sbr) => sbr_decoder_apply(
                            sbr,
                            &mut self_.qmf_domain,
                            &[],
                            td2,
                            time_data3_size,
                            &mut self_.stream_info.num_channels,
                            &mut self_.stream_info.sample_rate,
                            Some(&self_.map_descr),
                            self_.ch_map_index as i32,
                            self_.frame_ok as i32,
                            &mut self_.ps_possible,
                            self_.aac_out_data_headroom,
                            &mut time_data_headroom,
                        ),
                        // (sbrDecoder_Apply() with a NULL handle)
                        None => SBRDEC_INVALID_ARGUMENT,
                    };

                    if sbr_error == SBRDEC_OK {
                        /* Update data in streaminfo structure. Assume that the SBR upsampling
                         * factor is either 1,2 or 4 */
                        self_.flags[0] |= AC_SBR_PRESENT;
                        let si = &mut self_.stream_info;
                        if si.aac_sample_rate != si.sample_rate {
                            if si.frame_size == 768 {
                                si.frame_size = (si.aac_samples_per_frame * 8) / 3;
                            } else if si.aac_sample_rate.wrapping_shl(2) == si.sample_rate {
                                si.frame_size = si.aac_samples_per_frame << 2;
                            } else {
                                si.frame_size = si.aac_samples_per_frame << 1;
                            }
                        }

                        self_.flags[0] &= !AC_PS_PRESENT;
                    }
                    if sbr_error == SBRDEC_OUTPUT_BUFFER_TOO_SMALL {
                        error_status = AAC_DEC_OUTPUT_BUFFER_TOO_SMALL;
                        break 'bail;
                    }
                }

                if PCM_OUT_HEADROOM != time_data_headroom {
                    let n =
                        self_.stream_info.frame_size.wrapping_mul(self_.stream_info.num_channels);
                    // for (i = n - 1; i >= 0; i--) pTimeData3[i] >>= ...: the elements 0..n of
                    // the buffer, each shifted once (the order does not matter); `n - 1` wraps
                    // for n == INT_MIN, where C runs over the whole buffer.
                    let end =
                        if n == i32::MIN { td2.len() } else { (n.max(0) as usize).min(td2.len()) };
                    // (PCM_DEC)pTimeData3[i] >> (PCM_OUT_HEADROOM - timeDataHeadroom): the
                    // amount is 5 (headroom 3) in the library.
                    let sh = (PCM_OUT_HEADROOM - time_data_headroom) & 31;
                    for x in &mut td2[..end] {
                        *x >>= sh;
                    }
                }

                {
                    if fdk_drc_dec_get_param(self_.h_uni_drc_decoder.as_deref(), DRC_DEC_IS_ACTIVE)
                        != 0
                        && (self_.flags[0] & AC_USAC) != 0
                    {
                        /* Apply DRC gains*/
                        let mut drc_delay: i32 = 0;
                        let mut needs_deinterleaving = 0;
                        let channel_gain = [0 as FixpDbl; 8];
                        let mut reverse_in_channel_map = [0i32; 8];
                        let mut reverse_out_channel_map = [0i32; 8];
                        for ch in 0..8 {
                            reverse_in_channel_map[ch] = ch as i32;
                            reverse_out_channel_map[ch] = ch as i32;
                        }

                        /* Update sampleRate and frameSize. This may be necessary in case of
                         * implicit SBR signaling */
                        fdk_drc_dec_set_param(
                            self_.h_uni_drc_decoder.as_deref_mut(),
                            DRC_DEC_SAMPLE_RATE,
                            self_.stream_info.sample_rate,
                        );
                        fdk_drc_dec_set_param(
                            self_.h_uni_drc_decoder.as_deref_mut(),
                            DRC_DEC_FRAME_SIZE,
                            self_.stream_info.frame_size,
                        );

                        /* If SBR and/or MPS is active, the DRC gains are aligned to the QMF
                        domain signal before the QMF synthesis. Therefore the DRC gains need to
                        be delayed by the QMF synthesis delay. */
                        if self_.sbr_enabled != 0 {
                            drc_delay = 257;
                        }
                        if self_.mps_enable_curr != 0 {
                            drc_delay = 257;
                        }
                        /* Take into account concealment delay */
                        drc_delay = drc_delay.wrapping_add(
                            (cconcealment_get_delay(Some(&self_.conceal_common_data)) as i32)
                                .wrapping_mul(self_.stream_info.frame_size),
                        );

                        let nch = self_.stream_info.num_channels;
                        let fs = self_.stream_info.frame_size;
                        /* The output of SBR and MPS is interleaved. Deinterleaving may be
                         * necessary for FDK_drcDec_ProcessTime, which accepts deinterleaved
                         * audio only. */
                        let work_off: usize;
                        if (nch > 1) && (self_.sbr_enabled != 0 || self_.mps_enable_curr != 0) {
                            /* interleaving/deinterleaving is performed on upper part of
                             * pTimeData2. Check if this buffer is large enough. */
                            if time_data2_size < 2 * nch * fs {
                                error_status = AAC_DEC_UNKNOWN;
                                break 'bail;
                            }
                            needs_deinterleaving = 1;
                            work_off = (nch * fs) as usize;
                            let (lo, hi) = td2.split_at_mut(work_off.min(td2.len()));
                            fdk_deinterleave(lo, hi, nch as u32, fs as u32, fs as u32);
                        } else {
                            work_off = 0;
                        }
                        let drc_work_buffer = tail_mut_dbl(td2, work_off);

                        /* prepare Loudness Normalisation gain */
                        fdk_drc_dec_set_param(
                            self_.h_uni_drc_decoder.as_deref_mut(),
                            DRC_DEC_TARGET_LOUDNESS,
                            (-(self_.default_target_loudness as i32))
                                .wrapping_mul(fl2fxconst_dbl(1.0 / (1 << 9) as f64)),
                        );
                        fdk_drc_dec_set_channel_gains(
                            self_.h_uni_drc_decoder.as_deref_mut(),
                            nch,
                            fs,
                            &channel_gain,
                            drc_work_buffer,
                            fs,
                        );
                        fdk_drc_dec_preprocess(self_.h_uni_drc_decoder.as_deref_mut());

                        /* apply DRC1 gain sequence */
                        fdk_drc_dec_process_time(
                            self_.h_uni_drc_decoder.as_deref_mut(),
                            drc_delay,
                            DRC_DEC_DRC1,
                            0,
                            0,
                            nch,
                            drc_work_buffer,
                            fs,
                        );
                        /* apply downmix */
                        fdk_drc_dec_apply_downmix(
                            self_.h_uni_drc_decoder.as_deref_mut(),
                            Some(&reverse_in_channel_map),
                            Some(&reverse_out_channel_map),
                            Some(drc_work_buffer),
                            &mut self_.stream_info.num_channels,
                        ); /* self->streamInfo.numChannels may change here */
                        /* apply DRC2/3 gain sequence */
                        fdk_drc_dec_process_time(
                            self_.h_uni_drc_decoder.as_deref_mut(),
                            drc_delay,
                            DRC_DEC_DRC2_DRC3,
                            0,
                            0,
                            self_.stream_info.num_channels,
                            drc_work_buffer,
                            fs,
                        );

                        if needs_deinterleaving != 0 {
                            let (lo, hi) = td2.split_at_mut(work_off.min(td2.len()));
                            fdk_interleave(
                                hi,
                                lo,
                                self_.stream_info.num_channels as u32,
                                fs as u32,
                                fs as u32,
                            );
                        }
                    }
                }
                if fdk_drc_dec_get_param(self_.h_uni_drc_decoder.as_deref(), DRC_DEC_IS_ACTIVE) != 0
                {
                    /* return output loudness information for MPEG-D DRC */
                    let output_loudness = fdk_drc_dec_get_param(
                        self_.h_uni_drc_decoder.as_deref(),
                        DRC_DEC_OUTPUT_LOUDNESS,
                    );
                    if output_loudness == DRC_DEC_LOUDNESS_NOT_PRESENT {
                        /* no valid MPEG-D DRC loudness value contained */
                        self_.stream_info.output_loudness = -1;
                    } else if output_loudness > 0 {
                        /* positive output loudness values (very unusual) are limited to 0 dB */
                        self_.stream_info.output_loudness = 0;
                    } else {
                        self_.stream_info.output_loudness = output_loudness.wrapping_neg() >> 22;
                        /* negate and scale from e = 7 to e = (31-2) */
                    }
                } else {
                    /* return output loudness information for MPEG-4 DRC */
                    if self_.stream_info.drc_prog_ref_lev < 0 {
                        /* no MPEG-4 DRC loudness metadata contained */
                        self_.stream_info.output_loudness = -1;
                    } else if self_.default_target_loudness < 0 {
                        /* loudness normalization is off */
                        self_.stream_info.output_loudness =
                            self_.stream_info.drc_prog_ref_lev as i32;
                    } else {
                        self_.stream_info.output_loudness = self_.default_target_loudness as i32;
                    }
                }

                if self_.stream_info.ext_aot != AOT_AAC_SLS {
                    interleaved = 0;
                    interleaved |= (self_.sbr_enabled != 0) as i32;
                    interleaved |= (self_.mps_enable_curr != 0) as i32;
                    if (flags & AACDEC_INTR) != 0 && (access_unit == 0) {
                        /* delete data from the past (e.g. mixdown coeficients) */
                        pcm_dmx_reset(self_.h_pcm_utils.as_deref_mut(), PCMDMX_RESET_BS_DATA);
                    }
                    if flags & AACDEC_CLRHIST != 0 && self_.flags[0] & AC_USAC == 0 {
                        /* delete data from the past (e.g. mixdown coeficients) */
                        pcm_dmx_reset(self_.h_pcm_utils.as_deref_mut(), PCMDMX_RESET_BS_DATA);
                    }

                    /* do PCM post processing */
                    let dmx_err = pcm_dmx_apply_frame(
                        self_.h_pcm_utils.as_deref_mut(),
                        Some(td2),
                        time_data2_size,
                        self_.stream_info.frame_size as u32,
                        Some(&mut self_.stream_info.num_channels),
                        interleaved,
                        Some(&mut self_.stream_info.p_channel_type),
                        Some(&mut self_.stream_info.p_channel_indices),
                        Some(&self_.map_descr),
                        Some(&mut pcm_limiter_scale),
                    );
                    if dmx_err == PCMDMX_OUTPUT_BUFFER_TOO_SMALL {
                        error_status = AAC_DEC_OUTPUT_BUFFER_TOO_SMALL;
                        break 'bail;
                    }
                    if (error_status == AAC_DEC_OK) && (dmx_err == PCMDMX_INVALID_MODE) {
                        /* Announce the framework that the current combination of channel
                        configuration and downmix settings are not know to produce a predictable
                        behavior and thus maybe produce strange output. */
                        error_status = AAC_DEC_DECODE_FRAME_ERROR;
                    }
                }

                if self_.flags[0] & AC_USAC != 0 {
                    if self_.flush_status == AACDEC_USAC_DASH_IPF_FLUSH_ON
                        && (flags & AACDEC_CONCEAL) == 0
                    {
                        caac_decoder_prepare_cross_fade(
                            td2,
                            &mut self_.p_time_data_flush,
                            self_.stream_info.num_channels,
                            self_.stream_info.frame_size,
                            interleaved,
                        );
                    }

                    /* prepare crossfade buffer for fade in */
                    if apply_crossfade == 0
                        && (self_.apply_crossfade != AACDEC_CROSSFADE_BITMASK_OFF)
                        && (flags & AACDEC_CONCEAL) == 0
                    {
                        for ch in 0..self_.stream_info.num_channels.max(0) as usize {
                            // pTimeDataFlush[ch] is allocated for the flushed channels (C
                            // dereferences it).
                            if let Some(f) =
                                self_.p_time_data_flush.get_mut(ch).and_then(|f| f.as_deref_mut())
                            {
                                for i in 0..(TIME_DATA_FLUSH_SIZE as usize).min(f.len()) {
                                    f[i] = 0;
                                }
                            }
                        }
                        apply_crossfade = 1;
                    }

                    if apply_crossfade != 0
                        && (self_.apply_crossfade != AACDEC_CROSSFADE_BITMASK_OFF)
                        && access_unit >= num_preroll_au
                        && (self_.build_up_status == AACDEC_USAC_BUILD_UP_ON)
                    {
                        caac_decoder_apply_cross_fade(
                            td2,
                            &self_.p_time_data_flush,
                            self_.stream_info.num_channels,
                            self_.stream_info.frame_size,
                            interleaved,
                        );
                        self_.apply_crossfade = AACDEC_CROSSFADE_BITMASK_OFF; /* disable cross-fade
                                                                              between frames at
                                                                              nect config
                                                                              change */
                    }
                }

                /* Signal interruption to take effect in next frame. */
                if ((flags & AACDEC_FLUSH) != 0 || self_.flush_status != 0)
                    && (flags & AACDEC_CONCEAL) == 0
                {
                    aac_decoder_signal_interruption(self_);
                }

                /* Update externally visible copy of flags */
                self_.stream_info.flags = self_.flags[0];
            } /* USAC DASH IPF flushing possible end */
            if access_unit < num_preroll_au {
                if let Some(tp) = self_.h_input.as_deref_mut() {
                    let v = fdk_get_valid_bits(&mut tp.bit_stream[0]) as i32;
                    fdk_push_back(&mut tp.bit_stream[0], au_start_anchor.wrapping_sub(v) as u32);
                }
            } else {
                if (self_.build_up_status == AACDEC_RSV60_BUILD_UP_ON)
                    || (self_.build_up_status == AACDEC_RSV60_BUILD_UP_ON_IN_BAND)
                    || (self_.build_up_status == AACDEC_USAC_BUILD_UP_ON)
                {
                    self_.build_up_cnt = self_.build_up_cnt.wrapping_sub(1);

                    if self_.build_up_cnt < 0 {
                        self_.build_up_status = 0;
                    }
                }

                if self_.flags[0] & AC_USAC != 0
                    && self_.flush_status == AACDEC_USAC_DASH_IPF_FLUSH_ON
                    && (flags & AACDEC_CONCEAL) == 0
                {
                    self_.stream_info.frame_size = 0;
                }
            }

            if self_.flush_status != AACDEC_USAC_DASH_IPF_FLUSH_ON {
                access_unit += 1;
            }

            if !((access_unit < num_access_units)
                || ((self_.flush_status == AACDEC_USAC_DASH_IPF_FLUSH_ON)
                    && (flags & AACDEC_CONCEAL) == 0))
            {
                break;
            }
        }

        if self_.stream_info.ext_aot != AOT_AAC_SLS {
            pcm_limiter_scale += PCM_OUT_HEADROOM;

            if flags & AACDEC_CLRHIST != 0 && self_.flags[0] & AC_USAC == 0 {
                /* Reset DRC data */
                if let Some(d) = self_.h_drc_info.as_deref_mut() {
                    aac_decoder_drc_reset(d);
                }
                /* Delete the delayed signal. */
                pcm_limiter_reset(self_.h_limiter.as_deref_mut());
            }

            /* Set applyExtGain if DRC processing is enabled and if progRefLevelPresent is
            present for the first time. Consequences: The headroom of the output signal can be
            set to AACDEC_DRC_GAIN_SCALING only for audio formats which support legacy DRC Level
            Normalization. For all other audio formats the headroom of the output signal is set
            to PCM_OUT_HEADROOM. */
            if let Some(d) = self_.h_drc_info.as_deref_mut() {
                if d.enable != 0 && (d.prog_ref_level_present == 1) {
                    d.apply_ext_gain |= 1;
                }
            }

            let nch = self_.stream_info.num_channels;
            let fs = self_.stream_info.frame_size;

            /* Check whether time data buffer is large enough. */
            if time_data_size < nch.wrapping_mul(fs) {
                error_status = AAC_DEC_OUTPUT_BUFFER_TOO_SMALL;
                break 'bail;
            }

            let n = (nch.wrapping_mul(fs)).max(0) as usize;
            let drc_ext =
                self_.h_drc_info.as_deref().is_some_and(|d| d.enable != 0 && d.apply_ext_gain != 0);

            if self_.limiter_enable_curr != 0 {
                /* use workBufferCore2 buffer for interleaving */
                let block_length = fs;

                /* Set actual signal parameters */
                pcm_limiter_set_n_channels(self_.h_limiter.as_deref_mut(), nch as u32);
                pcm_limiter_set_sample_rate(
                    self_.h_limiter.as_deref_mut(),
                    self_.stream_info.sample_rate as u32,
                );

                let use_td2 = (nch == 1) || self_.sbr_enabled != 0 || self_.mps_enable_curr != 0;
                if !use_td2 {
                    /* applyLimiter requests for interleaved data */
                    /* Interleave ouput buffer */
                    let wb2 = self_.work_buffer_core2.get_or_insert_with(Vec::new);
                    fdk_interleave(td2, wb2, nch as u32, block_length as u32, fs as u32);
                }

                let mut p_gain_per_sample: Option<&mut [FixpDbl]> = None;

                if drc_ext {
                    if (get_required_mem_work_buffer_core1() as i32)
                        < fs.wrapping_mul(core::mem::size_of::<FixpDbl>() as i32)
                    {
                        error_status = AAC_DEC_UNKNOWN;
                        break 'bail;
                    }
                    let gain = self_.work_buffer_core1.as_deref_mut().map(|w| &mut w.buf[..]);
                    let interleave_buffer: &mut [FixpDbl] = if use_td2 {
                        &mut *td2
                    } else {
                        self_.work_buffer_core2.as_deref_mut().unwrap_or(&mut [])
                    };

                    if let Some(drc) = self_.h_drc_info.as_deref_mut() {
                        pcm_limiter_scale = apply_drc_level_normalization(
                            drc,
                            interleave_buffer,
                            &self_.ext_gain,
                            gain,
                            pcm_limiter_scale,
                            self_.ext_gain_delay,
                            fs as u32,
                            nch as u32,
                            1,
                            1,
                        );
                    }
                    p_gain_per_sample =
                        self_.work_buffer_core1.as_deref_mut().map(|w| &mut w.buf[..]);
                }

                let interleave_buffer: &[FixpDbl] =
                    if use_td2 { &*td2 } else { self_.work_buffer_core2.as_deref().unwrap_or(&[]) };
                pcm_limiter_apply(
                    self_.h_limiter.as_deref_mut(),
                    interleave_buffer,
                    p_time_data,
                    p_gain_per_sample.as_deref(),
                    pcm_limiter_scale,
                    fs as u32,
                );

                {
                    /* Announce the additional limiter output delay */
                    if let Some(l) = self_.h_limiter.as_deref() {
                        self_.stream_info.output_delay =
                            self_.stream_info.output_delay.wrapping_add(pcm_limiter_get_delay(l));
                    }
                }
            } else {
                if drc_ext {
                    let stride = if interleaved != 0 || (nch == 1) { 1 } else { fs as u32 };
                    if let Some(drc) = self_.h_drc_info.as_deref_mut() {
                        pcm_limiter_scale = apply_drc_level_normalization(
                            drc,
                            td2,
                            &self_.ext_gain,
                            None,
                            pcm_limiter_scale,
                            self_.ext_gain_delay,
                            fs as u32,
                            nch as u32,
                            stride,
                            0,
                        );
                    }
                }

                /* If numChannels = 1 we do not need interleaving. The same applies if SBR or MPS
                are used, since their output is interleaved already (resampled or not) */
                if (nch == 1) || self_.sbr_enabled != 0 || self_.mps_enable_curr != 0 {
                    let m = n.min(p_time_data.len()).min(td2.len());
                    scale_values_saturate_dbl_to_sgl(
                        &mut p_time_data[..m],
                        &td2[..m],
                        pcm_limiter_scale,
                    );
                } else {
                    let wb2 = self_.work_buffer_core2.get_or_insert_with(Vec::new);
                    let pcm = as_int_pcm(wb2);
                    let m = n.min(pcm.len()).min(td2.len());
                    scale_values_saturate_dbl_to_sgl(&mut pcm[..m], &td2[..m], pcm_limiter_scale);
                    /* Interleave ouput buffer */
                    fdk_interleave_sgl(&pcm[..m], p_time_data, nch as u32, fs as u32, fs as u32);
                }
            }
        } /* if (self->streamInfo.extAot != AOT_AAC_SLS)*/
    }

    lib_tail(self_, n_bits, error_status, f_end_au_not_adjusted, access_unit, num_preroll_au)
}

/// The end of `aacDecoder_DecodeFrame()` (after the label `bail`).
pub(crate) fn lib_tail(
    self_: &mut AacDecoderInstance,
    n_bits: i32,
    mut error_status: AacDecoderError,
    f_end_au_not_adjusted: i32,
    access_unit: u32,
    num_preroll_au: u32,
) -> AacDecoderError {
    /* error in renderer part occurred, ErrorStatus was set to invalid output */
    if f_end_au_not_adjusted != 0
        && is_output_valid(error_status) == 0
        && access_unit < num_preroll_au
    {
        if let Some(tp) = self_.h_input.as_deref_mut() {
            transport_dec_end_access_unit(tp);
        }
    }

    /* Update Statistics */
    if let Some(tp) = self_.h_input.as_deref_mut() {
        aac_decoder_update_bit_stream_counters(
            &mut self_.stream_info,
            &mut tp.bit_stream[0],
            n_bits,
            error_status,
        );
    }
    if ((self_.stream_info.num_channels <= 0)
        || (self_.stream_info.frame_size <= 0)
        || (self_.stream_info.sample_rate <= 0))
        && is_output_valid(error_status) != 0
    {
        /* Ensure consistency of IS_OUTPUT_VALID() macro. */
        error_status = AAC_DEC_UNKNOWN;
    }

    error_status
}

/// `&mut buf[off..]`, empty if `off` is beyond the buffer.
#[inline]
pub(crate) fn tail_mut_dbl(buf: &mut [FixpDbl], off: usize) -> &mut [FixpDbl] {
    let off = off.min(buf.len());
    &mut buf[off..]
}

/// `PCM_OUT_HEADROOM`
pub(crate) const PCM_OUT_HEADROOM: i32 = fdk_aac_rs_fdk::mdct::PCM_OUT_HEADROOM;

/// `aacDecoder_Close(self)`: de-allocate all resources of an AAC decoder instance.
pub fn aac_decoder_close(self_: Option<Box<AacDecoderInstance>>) {
    let Some(mut self_) = self_ else {
        return;
    };
    crate::pipeline::close(&mut self_);

    if let Some(l) = self_.h_limiter.take() {
        pcm_limiter_destroy(Some(l));
    }

    if self_.h_pcm_utils.is_some() {
        pcm_dmx_close(Some(&mut self_.h_pcm_utils));
    }

    fdk_drc_dec_close(Some(&mut self_.h_uni_drc_decoder));

    if self_.p_mpeg_surround_decoder.is_some() {
        mpeg_surround_decoder_close(self_.p_mpeg_surround_decoder.take());
    }

    if self_.h_sbr_decoder.is_some() {
        sbr_decoder_close(&mut self_.h_sbr_decoder);
    }

    if self_.h_input.is_some() {
        transport_dec_close(&mut self_.h_input);
    }

    caac_decoder_close(self_);
}

/// Not in C: sets the number of threads of the decoder (default 1). With more than one thread
/// the decoder pipelines frames ([`crate::pipeline`], opt-in): each frame is returned one call
/// later, with the content of the sequential decoder (see [`crate::pipeline::decode_frame`] for
/// the contract). Setting 1 again requires that no frame is pending (`AAC_DEC_SET_PARAM_FAIL`
/// otherwise; see [`aac_decoder_drain`]).
pub fn aac_decoder_set_threads(self_: &mut AacDecoderInstance, threads: u32) -> AacDecoderError {
    crate::pipeline::set_threads(self_, threads)
}

/// Not in C: the thread count of the decoder.
pub fn aac_decoder_get_threads(self_: &AacDecoderInstance) -> u32 {
    crate::pipeline::threads(self_)
}

/// Not in C: the number of frames decoded by earlier calls but not returned yet (0 or 1; always
/// 0 with one thread).
pub fn aac_decoder_pending_frames(self_: &AacDecoderInstance) -> u32 {
    crate::pipeline::pending_frames(self_)
}

/// Not in C: with more than one thread, after a call that returned `AAC_DEC_NOT_ENOUGH_BITS`,
/// waits for the frame decoded meanwhile on the worker thread, so that the stream information
/// is that of the sequential decoder after the call (the call does not wait for it). Without
/// effect otherwise. The C ABI calls it after every `aacDecoder_DecodeFrame()`.
pub fn aac_decoder_sync_stream_info(self_: &mut AacDecoderInstance) {
    crate::pipeline::sync_stream_info(self_);
}

/// Not in C: returns the pending frame of a decoder with more than one thread without
/// decoding input, as `aacDecoder_DecodeFrame()` would return it (output samples, stream
/// information, ancillary data); `AAC_DEC_NOT_ENOUGH_BITS` (and the output cleared) if no frame
/// is pending. `time_data_size` as for [`aac_decoder_decode_frame`].
pub fn aac_decoder_drain(
    self_: Option<&mut AacDecoderInstance>,
    p_time_data: &mut [IntPcm],
    time_data_size: i32,
) -> AacDecoderError {
    let Some(self_) = self_ else {
        return AAC_DEC_INVALID_HANDLE;
    };
    crate::pipeline::drain(self_, p_time_data, time_data_size)
}

/// `aacDecoder_GetStreamInfo(self)`: the current stream information (`None` for a `NULL`
/// handle).
///
/// With more than one thread ([`crate::pipeline`]), the stream information of the frame the
/// last `aacDecoder_DecodeFrame()` call returned.
pub fn aac_decoder_get_stream_info(self_: Option<&AacDecoderInstance>) -> Option<&CStreamInfo> {
    match self_ {
        Some(s) if s.pipe.is_some() => Some(crate::pipeline::presented_info(s)),
        _ => caac_decoder_get_stream_info(self_),
    }
}

/// `aacDecoder_GetLibInfo(info)`: get the decoder library information. `info` must hold
/// `FDK_MODULE_LAST` entries, initialized with `FDKinitLibInfo()`. Returns 0 on success, -1 on
/// failure.
///
/// As in C, the `flags` of the MPEG-D DRC decoder's entry are not written (PORTING.md,
/// "Definition of bit-exact"), and `build_date` and `build_time` are empty.
pub fn aac_decoder_get_lib_info(info: Option<&mut [LibInfo]>) -> i32 {
    let Some(info) = info else {
        return -1;
    };

    sbr_decoder_get_lib_info(Some(info));
    mpeg_surround_decoder_get_lib_info(Some(info));
    transport_dec_get_lib_info(Some(info));
    fdk_tools_get_lib_info(Some(info));
    pcm_dmx_get_lib_info(Some(info));
    pcm_limiter_get_lib_info(Some(info));
    fdk_drc_dec_get_lib_info(Some(info));

    /* search for next free tab */
    let n = info.len().min(FDK_MODULE_LAST as usize);
    let Some(i) = info[..n].iter().position(|e| e.module_id == FDK_NONE) else {
        return -1;
    };
    let info = &mut info[i];

    info.module_id = FDK_AACDEC;
    /* build own library info */
    info.version = lib_version(AACDECODER_LIB_VL0, AACDECODER_LIB_VL1, AACDECODER_LIB_VL2);
    lib_version_string(info);
    info.build_date = "";
    info.build_time = "";
    info.title = AACDECODER_LIB_TITLE;

    /* Set flags */
    info.flags = CAPF_AAC_LC
        | CAPF_ER_AAC_LC
        | CAPF_ER_AAC_SCAL
        | CAPF_AAC_VCB11
        | CAPF_AAC_HCR
        | CAPF_AAC_RVLC
        | CAPF_ER_AAC_LD
        | CAPF_ER_AAC_ELD
        | CAPF_AAC_CONCEALMENT
        | CAPF_AAC_DRC
        | CAPF_AAC_MPEG4
        | CAPF_AAC_DRM_BSFORMAT
        | CAPF_AAC_1024
        | CAPF_AAC_960
        | CAPF_AAC_512
        | CAPF_AAC_480
        | CAPF_AAC_ELD_DOWNSCALE
        | CAPF_AAC_USAC
        | CAPF_ER_AAC_ELDV2
        | CAPF_AAC_UNIDRC;
    /* End of flags */

    0
}

/// `fdk_assert!` is used by the parsing helpers of this module.
#[allow(dead_code)]
fn _assert_used() {
    fdk_assert!(true);
}
