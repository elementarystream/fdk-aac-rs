// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/include/aacenc_lib.h, libAACenc/src/aacenc_lib.cpp.

//! Port of `aacenc_lib.h`: the encoder API (PORTING.md, "Straight port").
//!
//! The functions keep the parameters of the C API. `HANDLE_AACENCODER *` is
//! `Option<&mut Option<Box<AacEncoder>>>`, `HANDLE_AACENCODER` is `Option<&mut AacEncoder>`
//! (`None` for `NULL`). The buffer descriptors ([`AacencBufDesc`]) and the argument structures
//! have the C layout and hold raw pointers, so [`aac_enc_encode`] is `unsafe`: the C ABI layer
//! and the command line tool pass the caller's descriptors through unchanged.
//!
//! # Memory model
//!
//! `struct AACENCODER` is [`AacEncoder`]. Its sub-encoders are owned (`Option<Box<..>>` for the
//! pointers that may be `NULL`). The transport encoder writes into `outBuffer` through a raw
//! pointer (as in C); the buffer is a `Vec` owned by the encoder, allocated once in
//! [`aac_enc_open`].
//!
//! The transport encoder calls back into the AAC encoder while it writes a configuration (the
//! SBR header of ELD and the SpatialSpecificConfig of ELDv2); C registers the callbacks with
//! the handle as user data. The port passes them explicitly (the `_cb` functions of
//! `tpenc_lib`) as a structure that borrows the SBR and MPEG Surround encoders.
//!
//! # Undefined behavior and deviations
//!
//! See PORTING.md, "Deviations from the reference".
//!
//! - Divisions in `aacEncInfo()`, `aacEncEncode()` and the bitrate helpers: x86 division by
//!   zero gives the arm64 result (0) on both variants.
//! - Where `FDKaacEnc_GetChannelModeConfiguration()` has no defined result, the port reads a
//!   zero entry.
//! - Where the reference has no defined result, `aacEncEncode()` returns
//!   `AACENC_ENCODE_ERROR` and `aacEncoder_SetParam()` returns `AACENC_INVALID_CONFIG`.
//! - Writes of input samples outside the internal input buffer are dropped.

use crate::aac_enc_ram::AacEnc;
use crate::aacenc::*;
use crate::channel_map::{
    fdk_aac_enc_determine_encoder_mode, fdk_aac_enc_get_channel_mode_configuration,
    fdk_aac_enc_init_channel_mapping,
};
use crate::metadata_main::*;
use crate::mps_main::*;
use crate::qc_data::ChannelMapping;
use core::ffi::c_void;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::{
    fdk_byte_align, fdk_fetch_buffer, fdk_get_valid_bits, fdk_init_bit_stream, FdkBitStream,
    BS_WRITER,
};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::core::fdk_tools_get_lib_info;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::fixpoint_math::{f_div_norm_noexp, f_mult_i_floor};
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_sbrenc::sbr::SbrEncoder;
use fdk_aac_rs_sbrenc::sbr_encoder::*;
use fdk_aac_rs_tpenc::tpenc_lib::*;

mod pipeline;
mod tables;
pub use pipeline::EncPipe;
use tables::ELD_SBR_AUTO_CONFIG_TAB;

/// `eldSbrAutoConfigTab` (for the table tests).
#[doc(hidden)]
pub fn eld_sbr_auto_config_tab() -> &'static [EldSbrConfigurator] {
    &ELD_SBR_AUTO_CONFIG_TAB
}

/// `AACENCODER_LIB_VL0`
pub const AACENCODER_LIB_VL0: i32 = 4;
/// `AACENCODER_LIB_VL1`
pub const AACENCODER_LIB_VL1: i32 = 0;
/// `AACENCODER_LIB_VL2`
pub const AACENCODER_LIB_VL2: i32 = 1;
/// `AACENCODER_LIB_TITLE`
pub const AACENCODER_LIB_TITLE: &str = "AAC Encoder";
/// `AACENCODER_LIB_BUILD_DATE`: `__DATE__` in C (empty in the port).
pub const AACENCODER_LIB_BUILD_DATE: &str = "";
/// `AACENCODER_LIB_BUILD_TIME`: `__TIME__` in C (empty in the port).
pub const AACENCODER_LIB_BUILD_TIME: &str = "";

// ---------------------------------------------------------------------------------------------
// aacenc_lib.h
// ---------------------------------------------------------------------------------------------

/// `AACENC_ERROR`
pub type AacencError = i32;
/// No error happened. All fine.
pub const AACENC_OK: AacencError = 0x0000;
/// Handle passed to function call was invalid.
pub const AACENC_INVALID_HANDLE: AacencError = 0x0020;
/// Memory allocation failed.
pub const AACENC_MEMORY_ERROR: AacencError = 0x0021;
/// Parameter not available.
pub const AACENC_UNSUPPORTED_PARAMETER: AacencError = 0x0022;
/// Configuration not provided.
pub const AACENC_INVALID_CONFIG: AacencError = 0x0023;
/// General initialization error.
pub const AACENC_INIT_ERROR: AacencError = 0x0040;
/// AAC library initialization error.
pub const AACENC_INIT_AAC_ERROR: AacencError = 0x0041;
/// SBR library initialization error.
pub const AACENC_INIT_SBR_ERROR: AacencError = 0x0042;
/// Transport library initialization error.
pub const AACENC_INIT_TP_ERROR: AacencError = 0x0043;
/// Meta data library initialization error.
pub const AACENC_INIT_META_ERROR: AacencError = 0x0044;
/// MPS library initialization error.
pub const AACENC_INIT_MPS_ERROR: AacencError = 0x0045;
/// The encoding process was interrupted by an unexpected error.
pub const AACENC_ENCODE_ERROR: AacencError = 0x0060;
/// End of file reached.
pub const AACENC_ENCODE_EOF: AacencError = 0x0080;

/// `AACENC_BufferIdentifier`
pub type AacencBufferIdentifier = i32;
/// Audio input buffer, interleaved INT_PCM samples.
pub const IN_AUDIO_DATA: AacencBufferIdentifier = 0;
/// Ancillary data to be embedded into bitstream.
pub const IN_ANCILLRY_DATA: AacencBufferIdentifier = 1;
/// Setup structure for embedding meta data.
pub const IN_METADATA_SETUP: AacencBufferIdentifier = 2;
/// Buffer holds bitstream output data.
pub const OUT_BITSTREAM_DATA: AacencBufferIdentifier = 3;
/// Buffer contains sizes of each access unit.
pub const OUT_AU_SIZES: AacencBufferIdentifier = 4;

/// `AACENC_InfoStruct` (C layout)
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AacencInfoStruct {
    /// Maximum number of encoder bitstream bytes within one frame.
    pub max_out_buf_bytes: u32,
    /// Maximum number of ancillary data bytes which can be inserted into bitstream within one
    /// frame.
    pub max_anc_bytes: u32,
    /// Internal input buffer fill level in samples per channel.
    pub in_buf_fill_level: u32,
    /// Number of input channels expected in encoding process.
    pub input_channels: u32,
    /// Amount of input audio samples consumed each frame per channel.
    pub frame_length: u32,
    /// Codec delay in PCM samples/channel.
    pub n_delay: u32,
    /// Codec delay in PCM samples/channel, w/o delay caused by the decoder SBR module.
    pub n_delay_core: u32,
    /// Configuration buffer in binary format as an AudioSpecificConfig or StreamMuxConfig.
    pub conf_buf: [u8; 64],
    /// Number of valid bytes in confBuf.
    pub conf_size: u32,
}

impl Default for AacencInfoStruct {
    fn default() -> Self {
        Self {
            max_out_buf_bytes: 0,
            max_anc_bytes: 0,
            in_buf_fill_level: 0,
            input_channels: 0,
            frame_length: 0,
            n_delay: 0,
            n_delay_core: 0,
            conf_buf: [0; 64],
            conf_size: 0,
        }
    }
}

/// `AACENC_BufDesc` (C layout, raw pointers)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AacencBufDesc {
    /// Number of buffers.
    pub num_bufs: i32,
    /// Pointer to vector containing buffer addresses.
    pub bufs: *mut *mut c_void,
    /// Identifier of each buffer element.
    pub buffer_identifiers: *mut i32,
    /// Size of each buffer in 8-bit bytes.
    pub buf_sizes: *mut i32,
    /// Size of each buffer element in bytes.
    pub buf_el_sizes: *mut i32,
}

/// `AACENC_InArgs` (C layout)
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AacencInArgs {
    /// Number of valid input audio samples (multiple of input channels).
    pub num_in_samples: i32,
    /// Number of ancillary data bytes to be encoded.
    pub num_anc_bytes: i32,
}

/// `AACENC_OutArgs` (C layout)
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AacencOutArgs {
    /// Number of valid bitstream bytes generated during aacEncEncode().
    pub num_out_bytes: i32,
    /// Number of input audio samples consumed by the encoder.
    pub num_in_samples: i32,
    /// Number of ancillary data bytes consumed by the encoder.
    pub num_anc_bytes: i32,
    /// State of the bit reservoir in bits.
    pub bit_res_state: i32,
}

/// `AACENC_METADATA_DRC_PROFILE`
pub type AacencMetadataDrcProfile = i32;
/// None.
pub const AACENC_METADATA_DRC_NONE: AacencMetadataDrcProfile = 0;
/// Film standard.
pub const AACENC_METADATA_DRC_FILMSTANDARD: AacencMetadataDrcProfile = 1;
/// Film light.
pub const AACENC_METADATA_DRC_FILMLIGHT: AacencMetadataDrcProfile = 2;
/// Music standard.
pub const AACENC_METADATA_DRC_MUSICSTANDARD: AacencMetadataDrcProfile = 3;
/// Music light.
pub const AACENC_METADATA_DRC_MUSICLIGHT: AacencMetadataDrcProfile = 4;
/// Speech.
pub const AACENC_METADATA_DRC_SPEECH: AacencMetadataDrcProfile = 5;
/// Disable writing gain factor (used for comp_profile only).
pub const AACENC_METADATA_DRC_NOT_PRESENT: AacencMetadataDrcProfile = 256;

/// `AACENC_MetaData::ExtMetaData` (C layout)
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AacencExtMetaData {
    /// Indicates if MPEG4_ext_ancillary_data() exists.
    pub ext_anc_data_enable: u8,
    /// Indicates if ext_downmixing_levels() exists.
    pub ext_downmix_level_enable: u8,
    /// Downmix level index A (0...7, according to table)
    pub ext_downmix_level_a: u8,
    /// Downmix level index B (0...7, according to table)
    pub ext_downmix_level_b: u8,
    /// Indicates if ext_downmixing_global_gains() exists.
    pub dmx_gain_enable: u8,
    pub _pad0: [u8; 3],
    /// Gain factor for downmix to 5 channels. Scaled with 16 bit. x*2^16.
    pub dmx_gain5: i32,
    /// Gain factor for downmix to 2 channels. Scaled with 16 bit. x*2^16.
    pub dmx_gain2: i32,
    /// Indicates if ext_downmixing_lfe_level() exists.
    pub lfe_dmx_enable: u8,
    /// Downmix level index for LFE (0..15, according to table)
    pub lfe_dmx_level: u8,
    pub _pad1: [u8; 2],
}

/// `AACENC_MetaData` (C layout, 52 bytes)
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AacencMetaData {
    /// MPEG DRC compression profile.
    pub drc_profile: AacencMetadataDrcProfile,
    /// ETSI heavy compression profile.
    pub comp_profile: AacencMetadataDrcProfile,
    /// Used to define expected level. Scaled with 16 bit. x*2^16.
    pub drc_target_ref_level: i32,
    /// Adjust limiter to avoid overload. Scaled with 16 bit. x*2^16.
    pub comp_target_ref_level: i32,
    /// Flag, if prog_ref_level is present
    pub prog_ref_level_present: i32,
    /// Programme Reference Level = Dialogue Level. Scaled with 16 bit. x*2^16.
    pub prog_ref_level: i32,
    /// Flag, if dmx-idx should be written in programme config element
    pub pce_mixdown_idx_present: u8,
    /// Flag, if dmx-lvl should be written in ETSI-ancData
    pub etsi_dmx_lvl_present: u8,
    /// Center downmix level (0...7, according to table)
    pub center_mix_level: i8,
    /// Surround downmix level (0...7, according to table)
    pub surround_mix_level: i8,
    /// Indication for Dolby Surround Encoding Mode.
    pub dolby_surround_mode: u8,
    /// Indicatin for DRC Presentation Mode.
    pub drc_presentation_mode: u8,
    pub _pad0: [u8; 2],
    /// extended ancillary data
    pub ext_meta_data: AacencExtMetaData,
}

/// `AACENC_CTRLFLAGS`
pub type AacencCtrlFlags = u32;
/// Do not trigger initialization.
pub const AACENC_INIT_NONE: AacencCtrlFlags = 0x0000;
/// Initialize all encoder modules configuration.
pub const AACENC_INIT_CONFIG: AacencCtrlFlags = 0x0001;
/// Reset all encoder modules history buffer.
pub const AACENC_INIT_STATES: AacencCtrlFlags = 0x0002;
/// Initialize transport lib with new parameters.
pub const AACENC_INIT_TRANSPORT: AacencCtrlFlags = 0x1000;
/// Reset fill level of internal input buffer.
pub const AACENC_RESET_INBUFFER: AacencCtrlFlags = 0x2000;
/// Initialize all.
pub const AACENC_INIT_ALL: AacencCtrlFlags = 0xFFFF;

/// `AACENC_PARAM`
pub type AacencParam = i32;
/// Audio object type.
pub const AACENC_AOT: AacencParam = 0x0100;
/// Total encoder bitrate.
pub const AACENC_BITRATE: AacencParam = 0x0101;
/// Bitrate mode.
pub const AACENC_BITRATEMODE: AacencParam = 0x0102;
/// Audio input data sampling rate.
pub const AACENC_SAMPLERATE: AacencParam = 0x0103;
/// Configure SBR independently of the chosen Audio Object Type.
pub const AACENC_SBR_MODE: AacencParam = 0x0104;
/// Core encoder (AAC) audio frame length in samples.
pub const AACENC_GRANULE_LENGTH: AacencParam = 0x0105;
/// Set explicit channel mode.
pub const AACENC_CHANNELMODE: AacencParam = 0x0106;
/// Input audio data channel ordering scheme.
pub const AACENC_CHANNELORDER: AacencParam = 0x0107;
/// Controls activation of downsampled SBR.
pub const AACENC_SBR_RATIO: AacencParam = 0x0108;
/// Use of the afterburner feature.
pub const AACENC_AFTERBURNER: AacencParam = 0x0200;
/// Core encoder audio bandwidth.
pub const AACENC_BANDWIDTH: AacencParam = 0x0203;
/// Peak bitrate configuration parameter to adjust maximum bits per audio frame.
pub const AACENC_PEAK_BITRATE: AacencParam = 0x0207;
/// Transport type to be used.
pub const AACENC_TRANSMUX: AacencParam = 0x0300;
/// Frame count period for sending in-band configuration buffers.
pub const AACENC_HEADER_PERIOD: AacencParam = 0x0301;
/// Signaling mode of the extension AOT.
pub const AACENC_SIGNALING_MODE: AacencParam = 0x0302;
/// Number of sub frames in a transport frame for LOAS/LATM or ADTS.
pub const AACENC_TPSUBFRAMES: AacencParam = 0x0303;
/// AudioMuxVersion to be used for LATM.
pub const AACENC_AUDIOMUXVER: AacencParam = 0x0304;
/// Configure protection in transport layer.
pub const AACENC_PROTECTION: AacencParam = 0x0306;
/// Constant ancillary data bitrate in bits/second.
pub const AACENC_ANCILLARY_BITRATE: AacencParam = 0x0500;
/// Configure Meta Data.
pub const AACENC_METADATA_MODE: AacencParam = 0x0600;
/// Control status of the internal reconfiguration process.
pub const AACENC_CONTROL_STATE: AacencParam = 0xFF00;
/// ------
pub const AACENC_NONE: AacencParam = 0xFFFF;

// ---------------------------------------------------------------------------------------------
// aacenc_lib.cpp
// ---------------------------------------------------------------------------------------------

/// `SBL(fl)`: Short block length (hardcoded to 8 short blocks per long block)
const fn sbl(fl: i32) -> i32 {
    fl / 8
}
/// `BSLA(fl)`: AAC block switching look-ahead
const fn bsla(fl: i32) -> i32 {
    4 * sbl(fl) + sbl(fl) / 2
}
/// `DELAY_AAC(fl)`: MDCT + blockswitching
const fn delay_aac(fl: i32) -> i32 {
    fl + bsla(fl)
}
/// `DELAY_AACLD(fl)`: MDCT delay (no framing delay included)
const fn delay_aacld(fl: i32) -> i32 {
    fl
}
/// `DELAY_AACELD(fl)`: ELD FB delay (no framing delay included)
const fn delay_aaceld(fl: i32) -> i32 {
    fl / 2
}

/// `MAX_DS_DELAY`: Maximum downsampler delay in SBR.
const MAX_DS_DELAY: i32 = 100;
/// `INPUTBUFFER_SIZE`: Audio input samples + downsampler delay + sbr/aac delay compensation
pub const INPUTBUFFER_SIZE: i32 = 2 * 1024 + MAX_DS_DELAY + 1537;

/// `DEFAULT_HEADER_PERIOD_REPETITION_RATE`: Default header repetition rate used in transport
/// library and for SBR header.
const DEFAULT_HEADER_PERIOD_REPETITION_RATE: i32 = 10;

/// Flags to characterize encoder modules to be supported in present instance.
pub const ENC_MODE_FLAG_AAC: u32 = 0x0001;
/// `ENC_MODE_FLAG_SBR`
pub const ENC_MODE_FLAG_SBR: u32 = 0x0002;
/// `ENC_MODE_FLAG_PS`
pub const ENC_MODE_FLAG_PS: u32 = 0x0004;
/// `ENC_MODE_FLAG_SAC`
pub const ENC_MODE_FLAG_SAC: u32 = 0x0008;
/// `ENC_MODE_FLAG_META`
pub const ENC_MODE_FLAG_META: u32 = 0x0010;

/// `USER_PARAM`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UserParam {
    /// Audio Object Type.
    pub user_aot: AudioObjectType,
    /// Sampling frequency.
    pub user_samplerate: u32,
    /// will be set via channelMode.
    pub n_channels: u32,
    pub user_channel_mode: ChannelMode,
    pub user_bitrate: u32,
    pub user_bitrate_mode: u32,
    pub user_bandwidth: u32,
    pub user_afterburner: u32,
    pub user_framelength: u32,
    pub user_anc_data_rate: u32,
    pub user_peak_bitrate: u32,

    /// Use TNS coding.
    pub user_tns: u8,
    /// Use PNS coding.
    pub user_pns: u8,
    /// Use Intensity coding.
    pub user_intensity: u8,

    /// Transport type
    pub user_tp_type: TransportType,
    /// Extension AOT signaling mode.
    pub user_tp_signaling: u8,
    /// Number of sub frames in a transport frame for LOAS/LATM or ADTS (default 1).
    pub user_tp_nsub_frames: u8,
    /// AudioMuxVersion to be used for LATM (default 0).
    pub user_tp_amxv: u8,
    pub user_tp_protection: u8,
    /// Parameter used to configure LATM/LOAS SMC rate. Moreover this parameters is used to
    /// configure repetition rate of PCE in raw_data_block.
    pub user_tp_header_period: u8,

    /// Use VCB11, HCR and/or RVLC ER tool.
    pub user_er_tools: u8,
    /// Configure additional bits in PCE.
    pub user_pce_additions: u32,

    /// Meta data library configuration.
    pub user_meta_data_mode: u8,

    /// Enable SBR for ELD.
    pub user_sbr_enabled: u8,
    /// SBR sampling rate ratio. Dual- or single-rate.
    pub user_sbr_ratio: u32,

    pub user_downscale_factor: u32,
}

/// `SBRENC_EXT_PAYLOAD`: SBR extension payload struct provides buffers to be filled in SBR
/// encoder library.
#[derive(Clone, Debug)]
pub struct SbrencExtPayload {
    /// extension payload data buffer
    pub data: [[[u8; MAX_PAYLOAD_SIZE]; 8]; 1],
    /// extension payload data size in bits
    pub data_size: [[u32; 8]; 1],
}

impl Default for SbrencExtPayload {
    fn default() -> Self {
        Self { data: [[[0; MAX_PAYLOAD_SIZE]; 8]; 1], data_size: [[0; 8]; 1] }
    }
}

/// `struct AACENCODER` (`HANDLE_AACENCODER` is `&mut AacEncoder`)
#[derive(Debug)]
pub struct AacEncoder {
    pub ext_param: UserParam,
    pub coder_config: CoderConfig,

    // AAC
    pub aac_config: AacencConfig,
    pub h_aac_enc: Option<Box<AacEnc>>,
    /// `hAacEnc->config != NULL`: the core configuration pointer was bound by
    /// `FDKaacEnc_Initialize()` (see [`crate::aacenc`]).
    pub aac_config_bound: bool,

    // SBR
    /// SBR encoder
    pub h_env_enc: Option<Box<SbrEncoder>>,
    /// SBR extension payload
    pub p_sbr_payload: Option<Box<SbrencExtPayload>>,

    // Meta Data
    pub h_metadata_enc: Option<Box<FdkMetadataEncoder>>,
    /// Signal whether chosen configuration allows metadata. Necessary for delay compensation.
    /// Metadata mode is a separate parameter.
    pub meta_data_allowed: i32,

    pub h_mps_enc: Option<Box<MpsEncoder>>,

    // Transport
    pub h_tp_enc: Option<Box<TransportEnc>>,

    /// Internal input buffer. Input source for AAC encoder
    pub input_buffer: Vec<IntPcm>,
    /// Internal bitstream buffer
    pub out_buffer: BitMem,

    /// Size of internal input buffer
    pub input_buffer_size: i32,
    /// Size of internal input buffer per channel
    pub input_buffer_size_per_channel: i32,
    /// Size of internal bitstream buffer
    pub out_buffer_in_bytes: i32,

    /// Where to write new input samples.
    pub input_buffer_offset: i32,

    /// number of input samples neeeded for encoding one frame
    pub n_samples_to_read: i32,
    /// number of input samples already in input buffer
    pub n_samples_read: i32,
    /// appended zeros at end of file
    pub n_zeros_appended: i32,
    /// codec delay
    pub n_delay: i32,
    /// codec delay, w/o the SBR decoder delay
    pub n_delay_core: i32,

    pub ext_payload: [AacencExtPayload; MAX_TOTAL_EXT_PAYLOADS],

    /// internal status to treggier re-initialization
    pub init_flags: u32,

    // Memory allocation info.
    pub n_max_aac_elements: i32,
    pub n_max_aac_channels: i32,
    pub n_max_sbr_elements: i32,
    pub n_max_sbr_channels: i32,

    pub encoder_modis: u32,

    // Capability flags
    pub capf_tp_enc: u32,

    /// The opt-in multi-threading (not in C): the thread count (default 1) and the pool
    /// ([`aac_enc_set_threads`]).
    pub threads: crate::par::EncThreads,
    /// The output delay and SBR frame pipelining of the multi-threading (not in C, see
    /// [`aac_enc_get_output_delay`]).
    pub pipe: EncPipe,
}

// SAFETY: the raw pointers (extension payloads, the transport encoder's buffer pointer) point
// into memory owned by the encoder or are only used during a call.
unsafe impl Send for AacEncoder {}

impl AacEncoder {
    /// The cleared structure (`FDKmemclear(hAacEncoder, sizeof(AACENCODER))`).
    fn cleared() -> Self {
        AacEncoder {
            ext_param: UserParam::default(),
            coder_config: CoderConfig::default(),
            aac_config: AacencConfig::default(),
            h_aac_enc: None,
            aac_config_bound: false,
            h_env_enc: None,
            p_sbr_payload: None,
            h_metadata_enc: None,
            meta_data_allowed: 0,
            h_mps_enc: None,
            h_tp_enc: None,
            input_buffer: Vec::new(),
            out_buffer: BitMem::default(),
            input_buffer_size: 0,
            input_buffer_size_per_channel: 0,
            out_buffer_in_bytes: 0,
            input_buffer_offset: 0,
            n_samples_to_read: 0,
            n_samples_read: 0,
            n_zeros_appended: 0,
            n_delay: 0,
            n_delay_core: 0,
            ext_payload: [AacencExtPayload::default(); MAX_TOTAL_EXT_PAYLOADS],
            init_flags: 0,
            n_max_aac_elements: 0,
            n_max_aac_channels: 0,
            n_max_sbr_elements: 0,
            n_max_sbr_channels: 0,
            encoder_modis: 0,
            capf_tp_enc: 0,
            threads: Default::default(),
            pipe: Default::default(),
        }
    }
}

/// Sets the thread count of the opt-in multi-threading (not in C; PORTING.md, "Optimization
/// rules"). With 1 (the default; 0 is the same) the encoder runs the sequential code. With
/// more, it runs independent per-channel work of each frame on a pool of `min(threads,
/// channels)` threads, created with the next frame; with SBR, it runs the SBR encoder of a
/// frame on a further thread at the same time as the core of the previous frame (whose pool
/// then has `min(threads - 1, channels)` threads) and delays its output by one call (see
/// [`aac_enc_get_output_delay`]). The results are those of the single-threaded encoder for
/// every thread count. Takes effect with the next frame.
pub fn aac_enc_set_threads(h_aac_encoder: &mut AacEncoder, threads: u32) {
    h_aac_encoder.threads.set(threads);
}

/// The thread count of [`aac_enc_set_threads`] (at least 1).
pub fn aac_enc_get_threads(h_aac_encoder: &AacEncoder) -> u32 {
    h_aac_encoder.threads.get()
}

/// Sets how an encoder with more than one thread chooses its thread mode (not in C; tests: the
/// default is [`fdk_aac_rs_fdk::par::tune::TUNE_ENV`] or adaptive; see `crate::par`).
#[doc(hidden)]
pub fn aac_enc_set_tuning(h_aac_encoder: &mut AacEncoder, tuning: fdk_aac_rs_fdk::par::Tuning) {
    h_aac_encoder.threads.set_tuning(tuning);
}

/// The frames an encoder encoded in each thread mode (not in C; tests).
#[doc(hidden)]
pub fn aac_enc_get_mode_frames(
    h_aac_encoder: &AacEncoder,
) -> [u64; fdk_aac_rs_fdk::par::tune::MAX_MODES] {
    h_aac_encoder.threads.mode_frames()
}

/// The output delay of the multi-threading (not in C): 1 if every `aacEncEncode()` call with
/// output arguments returns the result of the previous one, otherwise 0. The encoder switches
/// to the delay with the first such call made while the thread count is above 1 and SBR is
/// active after the call's initialization (that call returns an empty result), and keeps it
/// until it is closed; see [`aac_enc_set_threads`] and the module `pipeline`
/// (docs/optimization.md, "SBR encoder pipelining").
pub fn aac_enc_get_output_delay(h_aac_encoder: &AacEncoder) -> u32 {
    h_aac_encoder.pipe.delayed as u32
}

/// Statistics of the SBR frame pipelining (not in C, for tests): the frames whose core ran at
/// the same time as the next frame's SBR encoder, and the pipelined calls whose speculative
/// part was undone.
pub fn aac_enc_get_pipeline_stats(h_aac_encoder: &AacEncoder) -> (u64, u64) {
    (h_aac_encoder.pipe.overlapped, h_aac_encoder.pipe.undone)
}

/// `ELD_SBR_CONFIGURATOR`
#[derive(Clone, Copy, Debug)]
pub struct EldSbrConfigurator {
    // input
    /// Number of audio channels.
    pub n_channels: u32,
    /// Encoder output sampling rate.
    pub sampling_rate: u32,
    /// Lower bitrate range for config entry.
    pub bitrate_range: u32,
    // output
    /// 0: ELD sbr off, 1: ELD with downsampled sbr, 2: ELD with dualrate sbr.
    pub sbr_mode: u8,
    /// Channel mode.
    pub ch_mode: ChannelMode,
}

/// `eldSbrConfigurator(nChannels, samplingRate, bitrate)`: finds default SBR configuration for
/// ELD based on number of channels, sampling rate and bitrate.
#[doc(hidden)]
pub fn eld_sbr_configurator(
    n_channels: u32,
    sampling_rate: u32,
    bitrate: u32,
) -> Option<&'static EldSbrConfigurator> {
    let mut p_setup = None;

    for t in ELD_SBR_AUTO_CONFIG_TAB.iter() {
        if (n_channels == t.n_channels)
            && (sampling_rate <= t.sampling_rate)
            && (bitrate >= t.bitrate_range)
        {
            p_setup = Some(t);
        }
    }

    p_setup
}

/// `isSbrActive(hAacConfig)`
#[doc(hidden)]
pub fn is_sbr_active(h_aac_config: &AacencConfig) -> i32 {
    let mut sbr_used = 0;

    // Note: Even if implicit signalling was selected, The AOT itself here is not AOT_AAC_LC
    if (h_aac_config.audio_object_type == AOT_SBR)
        || (h_aac_config.audio_object_type == AOT_PS)
        || (h_aac_config.audio_object_type == AOT_MP2_SBR)
    {
        sbr_used = 1;
    }
    if h_aac_config.audio_object_type == AOT_ER_AAC_ELD
        && (h_aac_config.syntax_flags & AC_SBR_PRESENT != 0)
    {
        sbr_used = 1;
    }

    sbr_used
}

/// `isPsActive(audioObjectType)`
#[doc(hidden)]
pub fn is_ps_active(audio_object_type: AudioObjectType) -> i32 {
    (audio_object_type == AOT_PS) as i32
}

/// `GetCoreChannelMode(channelMode, audioObjectType)`
#[doc(hidden)]
pub fn get_core_channel_mode(
    channel_mode: ChannelMode,
    audio_object_type: AudioObjectType,
) -> ChannelMode {
    let mut mapped_channel_mode = channel_mode;
    if (is_ps_active(audio_object_type) != 0 && (channel_mode == MODE_2))
        || (channel_mode == MODE_212)
    {
        mapped_channel_mode = MODE_1;
    }
    mapped_channel_mode
}

/// `getSbrSignalingMode(audioObjectType, transportType, transportSignaling, sbrRatio)`
#[doc(hidden)]
pub fn get_sbr_signaling_mode(
    audio_object_type: AudioObjectType,
    transport_type: TransportType,
    transport_signaling: u8,
    sbr_ratio: u32,
) -> SbrPsSignaling {
    let mut sbr_signaling;

    if transport_type == TT_UNKNOWN || sbr_ratio == 0 {
        return SIG_UNKNOWN; // Needed parameters have not been set
    } else {
        sbr_signaling = SIG_EXPLICIT_HIERARCHICAL; // default: explicit hierarchical signaling
    }

    if (audio_object_type == AOT_AAC_LC)
        || (audio_object_type == AOT_SBR)
        || (audio_object_type == AOT_PS)
        || (audio_object_type == AOT_MP2_AAC_LC)
        || (audio_object_type == AOT_MP2_SBR)
    {
        match transport_type {
            TT_MP4_ADIF | TT_MP4_ADTS => {
                // For MPEG-2 transport types, only implicit signaling is possible
                sbr_signaling = SIG_IMPLICIT;
            }
            _ => {
                // TT_MP4_RAW, TT_MP4_LATM_MCP1, TT_MP4_LATM_MCP0, TT_MP4_LOAS, default
                if transport_signaling == 0xFF {
                    // Defaults
                    sbr_signaling = SIG_EXPLICIT_HIERARCHICAL;
                } else {
                    // User set parameters
                    // Attention: Backward compatible explicit signaling does only work with
                    // AMV1 for LATM/LOAS
                    sbr_signaling = transport_signaling as SbrPsSignaling;
                }
            }
        }
    }

    sbr_signaling
}

/// `getAssociatedChElement(elInfoSbr, channelMapping)`
#[doc(hidden)]
pub fn get_associated_ch_element(
    el_info_sbr: &SbrElementInfo,
    channel_mapping: &ChannelMapping,
) -> i32 {
    let n_elements = channel_mapping.n_elements;
    let mut associated_ch_element = -1;

    for i in 0..n_elements.clamp(0, 8) as usize {
        let e = &channel_mapping.el_info[i];
        if el_info_sbr.el_type == e.el_type && el_info_sbr.instance_tag == e.instance_tag {
            associated_ch_element = i as i32;
            break;
        }
    }

    associated_ch_element
}

/// `FDKaacEnc_MapConfig(cc, extCfg, sbrSignaling, hAacConfig)`: map encoder specific config
/// structures to CODER_CONFIG.
#[doc(hidden)]
pub fn fdk_aac_enc_map_config(
    cc: &mut CoderConfig,
    ext_cfg: &UserParam,
    sbr_signaling: SbrPsSignaling,
    h_aac_config: &AacencConfig,
) {
    let transport_aot: AudioObjectType;
    *cc = CoderConfig::default();

    cc.flags = 0;

    cc.samples_per_frame = h_aac_config.framelength;
    cc.sampling_rate = h_aac_config.sample_rate;
    cc.ext_sampling_rate = ext_cfg.user_samplerate as i32;

    // Map virtual aot to transport aot.
    match h_aac_config.audio_object_type {
        AOT_MP2_AAC_LC => transport_aot = AOT_AAC_LC,
        AOT_MP2_SBR => {
            transport_aot = AOT_SBR;
            cc.flags |= CC_SBR;
        }
        a => transport_aot = a,
    }

    if h_aac_config.audio_object_type == AOT_ER_AAC_ELD {
        cc.flags |= if h_aac_config.syntax_flags & AC_SBR_PRESENT != 0 { CC_SBR } else { 0 };
        cc.flags |= if h_aac_config.syntax_flags & AC_LD_MPS != 0 { CC_SAC } else { 0 };
    }

    // transport type is usually AAC-LC.
    if (transport_aot == AOT_SBR) || (transport_aot == AOT_PS) {
        cc.aot = AOT_AAC_LC;
    } else {
        cc.aot = transport_aot;
    }

    // Configure extension aot.
    if sbr_signaling == SIG_IMPLICIT {
        cc.ext_aot = AOT_NULL_OBJECT; // implicit
    } else if (sbr_signaling == SIG_EXPLICIT_BW_COMPATIBLE)
        && ((transport_aot == AOT_SBR) || (transport_aot == AOT_PS))
    {
        cc.ext_aot = AOT_SBR; // explicit backward compatible
    } else {
        cc.ext_aot = transport_aot; // explicit hierarchical
    }

    if (transport_aot == AOT_SBR) || (transport_aot == AOT_PS) {
        cc.sbr_present = 1;
        if transport_aot == AOT_PS {
            cc.ps_present = 1;
        }
    }
    cc.sbr_signaling = sbr_signaling;

    if h_aac_config.downscale_factor > 1 {
        cc.downscale_sampling_rate = cc.sampling_rate;
        cc.sampling_rate = cc.sampling_rate.wrapping_mul(h_aac_config.downscale_factor as i32);
        cc.ext_sampling_rate =
            cc.ext_sampling_rate.wrapping_mul(h_aac_config.downscale_factor as i32);
    }

    cc.bit_rate = h_aac_config.bit_rate;
    cc.no_channels = h_aac_config.n_channels;
    cc.flags |= CC_IS_BASELAYER;
    cc.channel_mode = h_aac_config.channel_mode;

    cc.n_sub_frames = if h_aac_config.n_sub_frames > 1 && ext_cfg.user_tp_nsub_frames == 1 {
        h_aac_config.n_sub_frames
    } else {
        ext_cfg.user_tp_nsub_frames as i32
    };

    cc.flags |= if ext_cfg.user_tp_protection != 0 { CC_PROTECTION } else { 0 };

    if ext_cfg.user_tp_header_period != 0xFF {
        cc.header_period = ext_cfg.user_tp_header_period;
    } else {
        // auto-mode
        cc.header_period = match ext_cfg.user_tp_type {
            TT_MP4_ADTS | TT_MP4_LOAS | TT_MP4_LATM_MCP1 => {
                DEFAULT_HEADER_PERIOD_REPETITION_RATE as u8
            }
            _ => 0,
        };
    }

    // Mpeg-4 signaling for transport library.
    match h_aac_config.audio_object_type {
        AOT_MP2_AAC_LC | AOT_MP2_SBR => {
            cc.flags &= !CC_MPEG_ID; // Required for ADTS.
            cc.ext_aot = AOT_NULL_OBJECT;
        }
        _ => cc.flags |= CC_MPEG_ID,
    }

    // ER-tools signaling.
    cc.flags |= if h_aac_config.syntax_flags & AC_ER_VCB11 != 0 { CC_VCB11 } else { 0 };
    cc.flags |= if h_aac_config.syntax_flags & AC_ER_HCR != 0 { CC_HCR } else { 0 };
    cc.flags |= if h_aac_config.syntax_flags & AC_ER_RVLC != 0 { CC_RVLC } else { 0 };

    // Matrix mixdown coefficient configuration.
    if (ext_cfg.user_pce_additions & 0x1 != 0)
        && (h_aac_config.ep_config == -1)
        && ((cc.channel_mode == MODE_1_2_2) || (cc.channel_mode == MODE_1_2_2_1))
    {
        cc.matrix_mixdown_a = (((ext_cfg.user_pce_additions >> 1) & 0x3) + 1) as u8;
        cc.flags |=
            if (ext_cfg.user_pce_additions >> 3) & 0x1 != 0 { CC_PSEUDO_SURROUND } else { 0 };
    } else {
        cc.matrix_mixdown_a = 0;
    }

    cc.channel_config_zero = 0;
}

/// `validateBufDesc(pBufDesc)`: validate prefilled pointers within buffer descriptor.
///
/// # Safety
/// `bufs` must be valid for reads of `num_bufs` entries if it is not null.
unsafe fn validate_buf_desc(p_buf_desc: Option<&AacencBufDesc>) -> AacencError {
    let Some(d) = p_buf_desc else {
        return AACENC_INVALID_HANDLE;
    };
    if d.buffer_identifiers.is_null()
        || d.buf_sizes.is_null()
        || d.buf_el_sizes.is_null()
        || d.bufs.is_null()
    {
        return AACENC_UNSUPPORTED_PARAMETER;
    }
    for i in 0..d.num_bufs.max(0) as usize {
        // SAFETY: the caller's guarantee.
        if unsafe { *d.bufs.add(i) }.is_null() {
            return AACENC_UNSUPPORTED_PARAMETER;
        }
    }
    AACENC_OK
}

/// `getBufDescIdx(pBufDesc, identifier)`: buffer descriptor index of `identifier`, -1 if there
/// is no entry (also where the reference has no defined result).
///
/// # Safety
/// `buffer_identifiers` must be valid for reads of `num_bufs` entries if it is not null.
unsafe fn get_buf_desc_idx(
    p_buf_desc: Option<&AacencBufDesc>,
    identifier: AacencBufferIdentifier,
) -> i32 {
    let mut idx = -1;

    if let Some(d) = p_buf_desc {
        if d.buffer_identifiers.is_null() {
            return -1;
        }
        for i in 0..d.num_bufs.max(0) {
            // SAFETY: the caller's guarantee.
            if unsafe { *d.buffer_identifiers.add(i as usize) } == identifier {
                idx = i;
                break;
            }
        }
    }
    idx
}

/// `aacEncDefaultConfig(hAacConfig, config)`
fn aac_enc_default_config(
    h_aac_config: &mut AacencConfig,
    config: &mut UserParam,
) -> AacEncoderError {
    // make reasonable default settings
    fdk_aac_enc_aac_init_default_config(h_aac_config);

    // clear configuration structure and copy default settings
    *config = UserParam::default();

    // copy encoder configuration settings
    config.n_channels = h_aac_config.n_channels as u32;
    h_aac_config.audio_object_type = AOT_AAC_LC;
    config.user_aot = AOT_AAC_LC;
    config.user_samplerate = h_aac_config.sample_rate as u32;
    config.user_channel_mode = h_aac_config.channel_mode;
    config.user_bitrate = h_aac_config.bit_rate as u32;
    config.user_bitrate_mode = h_aac_config.bitrate_mode as u32;
    config.user_peak_bitrate = u32::MAX;
    config.user_bandwidth = h_aac_config.band_width as u32;
    config.user_tns = h_aac_config.use_tns;
    config.user_pns = h_aac_config.use_pns;
    config.user_intensity = h_aac_config.use_is;
    config.user_afterburner = h_aac_config.use_requant as u32;
    config.user_framelength = u32::MAX;

    config.user_downscale_factor = 1;

    // initialize transport parameters
    config.user_tp_type = TT_UNKNOWN;
    config.user_tp_amxv = 0;
    config.user_tp_signaling = 0xFF; // choose signaling automatically
    config.user_tp_nsub_frames = 1;
    config.user_tp_protection = 0; // not crc protected
    config.user_tp_header_period = 0xFF; // header period in auto mode
    config.user_pce_additions = 0; // no matrix mixdown coefficient
    config.user_meta_data_mode = 0; // do not embed any meta data info

    config.user_anc_data_rate = 0;

    // SBR rate is set to 0 here, which means it should be set automatically in
    // FDKaacEnc_AdjustEncSettings() if the user did not set a rate expilicitely.
    config.user_sbr_ratio = 0;

    // SBR enable set to -1 means to inquire ELD audio configurator for reasonable
    // configuration.
    config.user_sbr_enabled = 0xFF;

    AAC_ENC_OK
}

/// `aacEncDistributeSbrBits(channelMapping, sbrElInfo, bitRate)`
#[doc(hidden)]
pub fn aac_enc_distribute_sbr_bits(
    channel_mapping: &ChannelMapping,
    sbr_el_info: &mut [SbrElementInfo; 8],
    bit_rate: i32,
) {
    let mut codebits = bit_rate;

    // Copy Element info
    for el in 0..channel_mapping.n_elements.clamp(0, 8) as usize {
        let e = &channel_mapping.el_info[el];
        sbr_el_info[el].channel_index[0] = e.channel_index[0] as u8;
        sbr_el_info[el].channel_index[1] = e.channel_index[1] as u8;
        sbr_el_info[el].el_type = e.el_type;
        sbr_el_info[el].bit_rate = f_mult_i_floor(e.relative_bits, bit_rate);
        sbr_el_info[el].instance_tag = e.instance_tag;
        sbr_el_info[el].n_channels_in_el = e.n_channels_in_el as u8;
        sbr_el_info[el].f_parametric_stereo = 0;
        sbr_el_info[el].f_dual_mono = 0;

        codebits = codebits.wrapping_sub(sbr_el_info[el].bit_rate);
    }
    sbr_el_info[0].bit_rate = sbr_el_info[0].bit_rate.wrapping_add(codebits);
}

/// `aacEncoder_LimitBitrate(hTpEnc, samplingRate, frameLength, nChannels, channelMode, bitRate,
/// nSubFrames, sbrActive, sbrDownSampleRate, syntaxFlags, aot)`
#[allow(clippy::too_many_arguments)]
#[doc(hidden)]
pub fn aac_encoder_limit_bitrate(
    h_tp_enc: Option<&mut TransportEnc>,
    sampling_rate: i32,
    frame_length: i32,
    n_channels: i32,
    channel_mode: ChannelMode,
    mut bit_rate: i32,
    n_sub_frames: i32,
    sbr_active: i32,
    sbr_down_sample_rate: i32,
    syntax_flags: u32,
    aot: AudioObjectType,
) -> i32 {
    let mut cm = ChannelMapping::default();

    fdk_aac_enc_init_channel_mapping(channel_mode, CH_ORDER_MPEG, &mut cm);

    let core_sampling_rate = if sbr_active != 0 {
        let shift = if sbr_encoder_is_single_rate_possible(aot) != 0 {
            sbr_down_sample_rate - 1
        } else {
            1
        };
        // A shift amount outside 0..31 is undefined; the encoder passes 1 or 2.
        sampling_rate.wrapping_shr(shift as u32)
    } else {
        sampling_rate
    };

    // Limit bit rate in respect to the core coder
    bit_rate = fdk_aac_enc_limit_bitrate(
        h_tp_enc,
        aot,
        core_sampling_rate,
        frame_length,
        n_channels,
        cm.n_channels_eff,
        bit_rate,
        -1,
        None,
        AACENC_BR_MODE_INVALID,
        n_sub_frames,
    );

    // Limit bit rate in respect to available SBR modes if active
    if sbr_active != 0 {
        let mut num_iterations = 0;
        let mut initial_bitrate;
        let mut adjusted_bitrate = bit_rate;

        // Find total bitrate which provides valid configuration for each SBR element.
        loop {
            let mut sbr_el_info = [SbrElementInfo::default(); 8];
            fdk_assert!(cm.n_elements <= 8);

            initial_bitrate = adjusted_bitrate;

            // Get bit rate for each SBR element
            aac_enc_distribute_sbr_bits(&cm, &mut sbr_el_info, initial_bitrate);

            for e in 0..cm.n_elements.clamp(0, 8) as usize {
                if cm.el_info[e].el_type != ID_SCE && cm.el_info[e].el_type != ID_CPE {
                    continue;
                }
                let sbr_element_bit_rate_in = sbr_el_info[e].bit_rate;

                let sbr_bit_rate_out = sbr_encoder_limit_bit_rate(
                    sbr_element_bit_rate_in as u32,
                    cm.el_info[e].n_channels_in_el as u32,
                    core_sampling_rate as u32,
                    aot,
                ) as i32;

                if sbr_bit_rate_out == 0 {
                    return 0;
                }

                // If bitrates don't match, distribution and limiting needs to be determined
                // again. Abort element loop and restart with adapted bitrate.
                if sbr_element_bit_rate_in != sbr_bit_rate_out {
                    if sbr_element_bit_rate_in < sbr_bit_rate_out {
                        adjusted_bitrate = f_max(
                            initial_bitrate,
                            f_div_norm_noexp(
                                sbr_bit_rate_out.wrapping_add(8),
                                cm.el_info[e].relative_bits,
                            ),
                        );
                        break;
                    }

                    if sbr_element_bit_rate_in > sbr_bit_rate_out {
                        adjusted_bitrate = f_min(
                            initial_bitrate,
                            f_div_norm_noexp(
                                sbr_bit_rate_out.wrapping_sub(8),
                                cm.el_info[e].relative_bits,
                            ),
                        );
                        break;
                    }
                } // sbrElementBitRateIn != sbrBitRateOut
            } // elements

            num_iterations += 1; // restrict iteration to worst case of num elements

            if !((initial_bitrate != adjusted_bitrate) && (num_iterations <= cm.n_elements)) {
                break;
            }
        }

        // Unequal bitrates mean that no reasonable bitrate configuration found.
        bit_rate = if initial_bitrate == adjusted_bitrate { adjusted_bitrate } else { 0 };
    }

    // Limit bit rate in respect to available MPS modes if active
    if (aot == AOT_ER_AAC_ELD) && (syntax_flags & AC_LD_MPS != 0) && (channel_mode == MODE_1) {
        bit_rate = fdk_mpegs_enc_get_closest_bit_rate(
            aot,
            MODE_212,
            sampling_rate as u32,
            if sbr_active != 0 { sbr_down_sample_rate as u32 } else { 0 },
            bit_rate as u32,
        );
    }

    bit_rate
}

/// `FDKaacEnc_GetCBRBitrate(hAacConfig, userSbrRatio)`
#[doc(hidden)]
pub fn fdk_aac_enc_get_cbr_bitrate(h_aac_config: &AacencConfig, user_sbr_ratio: i32) -> i32 {
    let mut bitrate = channel_mode_config_or_zero(h_aac_config.channel_mode)
        .n_channels_eff
        .wrapping_mul(h_aac_config.sample_rate);

    if is_ps_active(h_aac_config.audio_object_type) != 0 {
        // 0.5 bit per sample (bitrate = 1 * bitrate)
    } else if is_sbr_active(h_aac_config) != 0 {
        if (user_sbr_ratio == 2)
            || ((user_sbr_ratio == 0) && (h_aac_config.audio_object_type != AOT_ER_AAC_ELD))
        {
            bitrate = bitrate.wrapping_add(bitrate >> 2) >> 1; // 0.625 bits per sample
        }
        if (user_sbr_ratio == 1)
            || ((user_sbr_ratio == 0) && (h_aac_config.audio_object_type == AOT_ER_AAC_ELD))
        {
            bitrate = bitrate.wrapping_add(bitrate >> 3); // 1.125 bits per sample
        }
    } else {
        bitrate = bitrate.wrapping_add(bitrate >> 1); // 1.5 bits per sample
    }

    bitrate
}

/// `FDKaacEnc_AdjustEncSettings(hAacEncoder, config)`: consistency check of the given
/// USER_PARAM struct and copy back of the configuration from the public struct into the
/// internal encoder configuration struct.
fn fdk_aac_enc_adjust_enc_settings(
    h_aac_encoder: &mut AacEncoder,
    config: &mut UserParam,
) -> AacencError {
    let err = AACENC_OK;
    let out_buffer_in_bytes = h_aac_encoder.out_buffer_in_bytes;
    let n_max_aac_channels = h_aac_encoder.n_max_aac_channels;
    let n_max_sbr_channels = h_aac_encoder.n_max_sbr_channels;

    // Get struct pointers.
    let h_aac_config = &mut h_aac_encoder.aac_config;

    // Encoder settings update.
    h_aac_config.sample_rate = config.user_samplerate as i32;
    if config.user_downscale_factor > 1 {
        h_aac_config.use_tns = 0;
        h_aac_config.use_pns = 0;
        h_aac_config.use_is = 0;
    } else {
        h_aac_config.use_tns = config.user_tns;
        h_aac_config.use_pns = config.user_pns;
        h_aac_config.use_is = config.user_intensity;
    }

    h_aac_config.audio_object_type = config.user_aot;
    h_aac_config.channel_mode =
        get_core_channel_mode(config.user_channel_mode, h_aac_config.audio_object_type);
    h_aac_config.n_channels = channel_mode_config_or_zero(h_aac_config.channel_mode).n_channels;
    h_aac_config.bitrate_mode = config.user_bitrate_mode as AacencBitrateMode;
    h_aac_config.band_width = config.user_bandwidth as i32;
    h_aac_config.use_requant = config.user_afterburner as u8;

    h_aac_config.anc_rate = config.user_anc_data_rate as i32;
    h_aac_config.syntax_flags = 0;
    h_aac_config.ep_config = -1;

    if h_aac_config.audio_object_type != AOT_ER_AAC_ELD && config.user_downscale_factor > 1 {
        return AACENC_INVALID_CONFIG; // downscaling only allowed for AOT_ER_AAC_ELD
    }
    if config.user_downscale_factor > 1 && config.user_sbr_enabled == 1 {
        return AACENC_INVALID_CONFIG; // downscaling only allowed for AOT_ER_AAC_ELD w/o SBR
    }
    if config.user_downscale_factor > 1 && config.user_channel_mode == 128 {
        return AACENC_INVALID_CONFIG; // disallow downscaling for AAC-ELDv2
    }

    if config.user_tp_type == TT_MP4_LATM_MCP1
        || config.user_tp_type == TT_MP4_LATM_MCP0
        || config.user_tp_type == TT_MP4_LOAS
    {
        h_aac_config.audio_mux_version = config.user_tp_amxv as i32;
    } else {
        h_aac_config.audio_mux_version = -1;
    }

    // Adapt internal AOT when necessary.
    match config.user_aot {
        AOT_MP2_AAC_LC | AOT_MP2_SBR | AOT_AAC_LC | AOT_SBR | AOT_PS => {
            if config.user_aot == AOT_MP2_AAC_LC || config.user_aot == AOT_MP2_SBR {
                h_aac_config.use_pns = 0;
                // FDK_FALLTHROUGH
            }
            config.user_tp_type =
                if config.user_tp_type != TT_UNKNOWN { config.user_tp_type } else { TT_MP4_ADTS };
            h_aac_config.framelength = if config.user_framelength != u32::MAX {
                config.user_framelength as i32
            } else {
                1024
            };
            if h_aac_config.framelength != 1024 && h_aac_config.framelength != 960 {
                return AACENC_INVALID_CONFIG;
            }
        }
        AOT_ER_AAC_LD => {
            h_aac_config.ep_config = 0;
            h_aac_config.syntax_flags |= AC_ER | AC_LD;
            h_aac_config.syntax_flags |=
                if config.user_er_tools & 0x1 != 0 { AC_ER_VCB11 } else { 0 };
            h_aac_config.syntax_flags |=
                if config.user_er_tools & 0x2 != 0 { AC_ER_HCR } else { 0 };
            h_aac_config.syntax_flags |=
                if config.user_er_tools & 0x4 != 0 { AC_ER_RVLC } else { 0 };
            config.user_tp_type =
                if config.user_tp_type != TT_UNKNOWN { config.user_tp_type } else { TT_MP4_LOAS };
            h_aac_config.framelength = if config.user_framelength != u32::MAX {
                config.user_framelength as i32
            } else {
                512
            };
            if h_aac_config.framelength != 512 && h_aac_config.framelength != 480 {
                return AACENC_INVALID_CONFIG;
            }
        }
        AOT_ER_AAC_ELD => {
            h_aac_config.ep_config = 0;
            h_aac_config.syntax_flags |= AC_ER | AC_ELD;
            h_aac_config.syntax_flags |=
                if config.user_er_tools & 0x1 != 0 { AC_ER_VCB11 } else { 0 };
            h_aac_config.syntax_flags |=
                if config.user_er_tools & 0x2 != 0 { AC_ER_HCR } else { 0 };
            h_aac_config.syntax_flags |=
                if config.user_er_tools & 0x4 != 0 { AC_ER_RVLC } else { 0 };
            h_aac_config.syntax_flags |=
                if config.user_sbr_enabled == 1 { AC_SBR_PRESENT } else { 0 };
            h_aac_config.syntax_flags |=
                if config.user_channel_mode == MODE_212 { AC_LD_MPS } else { 0 };
            config.user_tp_type =
                if config.user_tp_type != TT_UNKNOWN { config.user_tp_type } else { TT_MP4_LOAS };
            h_aac_config.framelength = if config.user_framelength != u32::MAX {
                config.user_framelength as i32
            } else {
                512
            };

            h_aac_config.downscale_factor = config.user_downscale_factor;

            match config.user_downscale_factor {
                1 => {}
                2 | 4 => h_aac_config.syntax_flags |= AC_ELD_DOWNSCALE,
                _ => return AACENC_INVALID_CONFIG,
            }

            if !matches!(h_aac_config.framelength, 512 | 480 | 256 | 240 | 128 | 120) {
                return AACENC_INVALID_CONFIG;
            }
        }
        _ => {}
    }

    // Initialize SBR parameters
    if (config.user_sbr_ratio == 0) && (is_sbr_active(h_aac_config) != 0) {
        // Automatic SBR ratio configuration
        // - downsampled SBR for ELD
        // - otherwise always dualrate SBR
        if h_aac_config.audio_object_type == AOT_ER_AAC_ELD {
            h_aac_config.sbr_ratio = if (h_aac_config.syntax_flags & AC_LD_MPS != 0)
                && (h_aac_config.sample_rate >= 27713)
            {
                2
            } else {
                1
            };
        } else {
            h_aac_config.sbr_ratio = 2;
        }
    } else {
        // SBR ratio has been set by the user, so use it.
        h_aac_config.sbr_ratio =
            if is_sbr_active(h_aac_config) != 0 { config.user_sbr_ratio } else { 0 };
    }

    // Set default bitrate
    h_aac_config.bit_rate = config.user_bitrate as i32;

    match h_aac_config.bitrate_mode {
        AACENC_BR_MODE_CBR => {
            // Set default bitrate if no external bitrate declared.
            if config.user_bitrate == u32::MAX {
                h_aac_config.bit_rate =
                    fdk_aac_enc_get_cbr_bitrate(h_aac_config, config.user_sbr_ratio as i32);
            }
            h_aac_config.average_bits = -1;
        }
        AACENC_BR_MODE_VBR_1 | AACENC_BR_MODE_VBR_2 | AACENC_BR_MODE_VBR_3
        | AACENC_BR_MODE_VBR_4 | AACENC_BR_MODE_VBR_5 => {
            // Adjust bitrate mode in case given peak bitrate is lower than expected VBR
            // bitrate.
            if config.user_peak_bitrate as i32 != -1 {
                h_aac_config.bitrate_mode = fdk_aac_enc_adjust_vbr_bitrate_mode(
                    h_aac_config.bitrate_mode,
                    config.user_peak_bitrate as i32,
                    h_aac_config.channel_mode,
                );
            }
            // Get bitrate in VBR configuration
            // In VBR mode; SBR-modul depends on bitrate, core encoder on bitrateMode.
            h_aac_config.bit_rate =
                fdk_aac_enc_get_vbr_bitrate(h_aac_config.bitrate_mode, h_aac_config.channel_mode);
        }
        _ => return AACENC_INVALID_CONFIG,
    }

    // set bitreservoir size
    match h_aac_config.bitrate_mode {
        AACENC_BR_MODE_VBR_1 | AACENC_BR_MODE_VBR_2 | AACENC_BR_MODE_VBR_3
        | AACENC_BR_MODE_VBR_4 | AACENC_BR_MODE_VBR_5 | AACENC_BR_MODE_CBR => {
            if config.user_peak_bitrate as i32 != -1 {
                h_aac_config.max_bits_per_frame = (fdk_aac_enc_calc_bits_per_frame(
                    f_max(h_aac_config.bit_rate, config.user_peak_bitrate as i32),
                    h_aac_config.framelength,
                    h_aac_config.sample_rate,
                )
                .wrapping_add(7))
                    & !7;
            } else {
                h_aac_config.max_bits_per_frame = -1;
            }
            if h_aac_config.audio_mux_version == 2 {
                h_aac_config.min_bits_per_frame = f_min(
                    32 * 8,
                    fdk_aac_enc_calc_bits_per_frame(
                        h_aac_config.bit_rate,
                        h_aac_config.framelength,
                        h_aac_config.sample_rate,
                    ),
                ) & !7;
            }
        }
        _ => return AACENC_INVALID_CONFIG,
    }

    // Max bits per frame limitation depending on transport format.
    if config.user_tp_nsub_frames > 1 {
        let mut max_frame_length = 8i32.wrapping_mul(out_buffer_in_bytes);
        match config.user_tp_type {
            TT_MP4_LOAS => {
                max_frame_length =
                    f_min(max_frame_length, 8 * (1 << 13)) / config.user_tp_nsub_frames as i32;
            }
            TT_MP4_ADTS => {
                max_frame_length = f_min(max_frame_length, 8 * ((1 << 13) - 1))
                    / config.user_tp_nsub_frames as i32;
            }
            _ => max_frame_length = -1,
        }
        if max_frame_length != -1 {
            if h_aac_config.max_bits_per_frame > max_frame_length {
                return AACENC_INVALID_CONFIG;
            } else if h_aac_config.max_bits_per_frame == -1 {
                h_aac_config.max_bits_per_frame = max_frame_length;
            }
        }
    }

    if (h_aac_config.audio_object_type == AOT_ER_AAC_ELD)
        && (h_aac_config.syntax_flags & AC_ELD_DOWNSCALE == 0)
        && (config.user_sbr_enabled == 0xFF)
        && (config.user_sbr_ratio == 0)
        && ((h_aac_config.syntax_flags & AC_LD_MPS) == 0)
    {
        if let Some(p_config) = eld_sbr_configurator(
            channel_mode_config_or_zero(h_aac_config.channel_mode).n_channels as u32,
            h_aac_config.sample_rate as u32,
            h_aac_config.bit_rate as u32,
        ) {
            h_aac_config.syntax_flags |= if p_config.sbr_mode == 0 { 0 } else { AC_SBR_PRESENT };
            h_aac_config.syntax_flags |= if p_config.ch_mode == MODE_212 { AC_LD_MPS } else { 0 };
            h_aac_config.channel_mode =
                get_core_channel_mode(p_config.ch_mode, h_aac_config.audio_object_type);
            h_aac_config.n_channels =
                channel_mode_config_or_zero(h_aac_config.channel_mode).n_channels;
            h_aac_config.sbr_ratio = match p_config.sbr_mode {
                0 => 0,
                1 => 1,
                _ => 2,
            };
        }
    }

    {
        let tp_signaling = get_sbr_signaling_mode(
            h_aac_config.audio_object_type,
            config.user_tp_type,
            config.user_tp_signaling,
            h_aac_config.sbr_ratio,
        ) as u8;

        if (h_aac_config.audio_object_type == AOT_AAC_LC
            || h_aac_config.audio_object_type == AOT_SBR
            || h_aac_config.audio_object_type == AOT_PS)
            && (config.user_tp_type == TT_MP4_LATM_MCP1
                || config.user_tp_type == TT_MP4_LATM_MCP0
                || config.user_tp_type == TT_MP4_LOAS)
            && (tp_signaling == 1)
            && (config.user_tp_amxv == 0)
        {
            // For backward compatible explicit signaling, AMV1 has to be active
            return AACENC_INVALID_CONFIG;
        }

        if (h_aac_config.audio_object_type == AOT_AAC_LC
            || h_aac_config.audio_object_type == AOT_SBR
            || h_aac_config.audio_object_type == AOT_PS)
            && (tp_signaling == 0)
            && (h_aac_config.sbr_ratio == 1)
        {
            // Downsampled SBR has to be signaled explicitely (for transmission of SBR sampling
            // fequency)
            return AACENC_INVALID_CONFIG;
        }
    }

    match h_aac_config.bitrate_mode {
        AACENC_BR_MODE_CBR | AACENC_BR_MODE_VBR_1 | AACENC_BR_MODE_VBR_2 | AACENC_BR_MODE_VBR_3
        | AACENC_BR_MODE_VBR_4 | AACENC_BR_MODE_VBR_5 => {
            // We need the frame length to call aacEncoder_LimitBitrate()
            h_aac_config.bit_rate = aac_encoder_limit_bitrate(
                None,
                h_aac_config.sample_rate,
                h_aac_config.framelength,
                h_aac_config.n_channels,
                h_aac_config.channel_mode,
                h_aac_config.bit_rate,
                h_aac_config.n_sub_frames,
                is_sbr_active(h_aac_config),
                h_aac_config.sbr_ratio as i32,
                h_aac_config.syntax_flags,
                h_aac_config.audio_object_type,
            );
            if 0 >= h_aac_config.bit_rate {
                return AACENC_INVALID_CONFIG;
            }
        }
        _ => {}
    }

    // Configure PNS
    if aacenc_br_mode_is_vbr(h_aac_config.bitrate_mode) // VBR without PNS.
        || (h_aac_config.use_tns == 0)
    // TNS required.
    {
        h_aac_config.use_pns = 0;
    }

    if h_aac_config.ep_config >= 0 {
        h_aac_config.syntax_flags |= AC_ER;
        if (h_aac_config.channel_mode < 1) || (h_aac_config.channel_mode > 14) {
            return AACENC_INVALID_CONFIG; // Channel config 0 not supported.
        }
    }

    if (h_aac_config.syntax_flags & AC_LD_MPS) == 0 {
        let n_channels = h_aac_config.n_channels;
        if fdk_aac_enc_determine_encoder_mode(&mut h_aac_config.channel_mode, n_channels)
            != AAC_ENC_OK
        {
            // nChannels doesn't match chMode, this is just a check-up
            return AACENC_INVALID_CONFIG;
        }
    }

    if (h_aac_config.n_channels > n_max_aac_channels)
        || ((channel_mode_config_or_zero(h_aac_config.channel_mode).n_channels_eff
            > n_max_sbr_channels)
            && is_sbr_active(h_aac_config) != 0)
    {
        return AACENC_INVALID_CONFIG; // not enough channels allocated
    }

    // Meta data restriction.
    match h_aac_config.audio_object_type {
        // Allow metadata support
        AOT_AAC_LC | AOT_SBR | AOT_PS | AOT_MP2_AAC_LC | AOT_MP2_SBR => {
            h_aac_encoder.meta_data_allowed = 1;
            let cm = h_aac_encoder.aac_config.channel_mode;
            if !(((1..=14).contains(&cm))
                || (MODE_7_1_REAR_SURROUND == cm)
                || (MODE_7_1_FRONT_CENTER == cm))
            {
                config.user_meta_data_mode = 0;
            }
        }
        // Prohibit metadata support
        _ => h_aac_encoder.meta_data_allowed = 0,
    }

    err
}

/// The transport encoder callbacks of the AAC encoder (`aacenc_SbrCallback`,
/// `aacenc_SscCallback`): C registers them with the encoder handle as user data; here they
/// borrow the SBR and MPEG Surround encoders.
struct EncCallbacks<'a> {
    env_enc: Option<&'a mut SbrEncoder>,
    mps_enc: Option<&'a MpsEncoder>,
}

impl TpEncCallbacks for EncCallbacks<'_> {
    fn has_sbr(&self) -> bool {
        true // registered in aacEncOpen()
    }

    /// `aacenc_SbrCallback(self, hBs, ...)`
    fn sbr(
        &mut self,
        hbs: &mut FdkBitStream,
        _sample_rate_in: i32,
        _sample_rate_out: i32,
        _samples_per_frame: i32,
        _core_codec: AudioObjectType,
        _element_id: Mp4ElementId,
        element_index: i32,
        _harmonic_sbr: u8,
        _stereo_config_index: u8,
        _config_mode: u8,
        _config_changed: Option<&mut u8>,
        _downscale_factor: i32,
    ) -> i32 {
        // C dereferences hEnvEnc unchecked; it exists whenever the configuration has SBR.
        if let Some(e) = self.env_enc.as_deref_mut() {
            sbr_encoder_get_header(e, hbs, element_index, 0);
        }
        0
    }

    fn has_ssc(&self) -> bool {
        true // registered in aacEncOpen()
    }

    /// `aacenc_SscCallback(self, hBs, ...)`
    fn ssc(
        &mut self,
        hbs: Option<&mut FdkBitStream>,
        _core_codec: AudioObjectType,
        _sampling_rate: i32,
        _frame_size: i32,
        _num_channels: i32,
        _stereo_config_index: i32,
        _core_sbr_frame_length_index: i32,
        _config_bytes: i32,
        _config_mode: u8,
        _config_changed: Option<&mut u8>,
    ) -> i32 {
        fdk_mpegs_enc_write_spatial_specific_config(self.mps_enc, hbs)
    }
}

/// `aacEncInit(hAacEncoder, InitFlags, config)` (`config` is `&hAacEncoder->extParam`, passed
/// in and stored back by the caller).
fn aac_enc_init(
    h_aac_encoder: &mut AacEncoder,
    mut init_flags: u32,
    config: &mut UserParam,
) -> AacencError {
    let mut aac_buffer_offset: i32 = 0;

    h_aac_encoder.n_zeros_appended = 0; // count appended zeros

    let mut frame_length = h_aac_encoder.aac_config.framelength;

    if init_flags & AACENC_INIT_CONFIG != 0 {
        let prev_ch_mode = h_aac_encoder.aac_config.channel_mode;

        // Verify settings and update: config -> heAacEncoder
        let err = fdk_aac_enc_adjust_enc_settings(h_aac_encoder, config);
        if err != AACENC_OK {
            return err;
        }
        frame_length = h_aac_encoder.aac_config.framelength; // adapt temporal framelength

        // Seamless channel reconfiguration in sbr not fully implemented
        if (prev_ch_mode != h_aac_encoder.aac_config.channel_mode)
            && is_sbr_active(&h_aac_encoder.aac_config) != 0
        {
            init_flags |= AACENC_INIT_STATES;
        }
    }

    // Clear input buffer
    if init_flags == AACENC_INIT_ALL {
        h_aac_encoder.input_buffer.fill(0);
    }

    let h_aac_config = &mut h_aac_encoder.aac_config;

    if init_flags & AACENC_INIT_CONFIG != 0 {
        aac_buffer_offset = 0;
        h_aac_encoder.n_delay = match h_aac_config.audio_object_type {
            AOT_ER_AAC_LD => delay_aacld(h_aac_config.framelength),
            AOT_ER_AAC_ELD => delay_aaceld(h_aac_config.framelength),
            _ => delay_aac(h_aac_config.framelength), // AAC encoder delay
        };

        h_aac_config.anc_data_bit_rate = 0;
    }

    if h_aac_encoder.h_env_enc.is_some()
        && is_sbr_active(h_aac_config) != 0
        && ((init_flags & AACENC_INIT_CONFIG != 0) || (init_flags & AACENC_INIT_STATES != 0))
    {
        let mut init_flag: u32 = 0;
        let mut sbr_el_info = [SbrElementInfo::default(); 8];
        let mut channel_mapping = ChannelMapping::default();
        let channel_mode = if is_ps_active(h_aac_config.audio_object_type) != 0 {
            config.user_channel_mode
        } else {
            h_aac_config.channel_mode
        };
        let mut num_channels = if is_ps_active(h_aac_config.audio_object_type) != 0 {
            config.n_channels as i32
        } else {
            h_aac_config.n_channels
        };

        if fdk_aac_enc_init_channel_mapping(
            channel_mode,
            h_aac_config.channel_order,
            &mut channel_mapping,
        ) != AAC_ENC_OK
        {
            return AACENC_INIT_ERROR;
        }

        // Check return value and if the SBR encoder can handle enough elements
        if channel_mapping.n_elements > 8 {
            return AACENC_INIT_ERROR;
        }

        aac_enc_distribute_sbr_bits(&channel_mapping, &mut sbr_el_info, h_aac_config.bit_rate);

        init_flag += if init_flags & AACENC_INIT_STATES != 0 { 1 } else { 0 };

        // Let the SBR encoder take a look at the configuration and change if required.
        let Some(h_sbr) = h_aac_encoder.h_env_enc.as_deref_mut() else {
            return AACENC_INIT_SBR_ERROR;
        };
        let sbr_error = sbr_encoder_init(
            h_sbr,
            &mut sbr_el_info,
            channel_mapping.n_elements,
            &mut h_aac_encoder.input_buffer,
            h_aac_encoder.input_buffer_size_per_channel as u32,
            &mut h_aac_config.band_width,
            &mut aac_buffer_offset,
            &mut num_channels,
            h_aac_config.syntax_flags,
            &mut h_aac_config.sample_rate,
            &mut h_aac_config.sbr_ratio,
            &mut frame_length,
            h_aac_config.audio_object_type,
            &mut h_aac_encoder.n_delay,
            if h_aac_config.audio_object_type == AOT_ER_AAC_ELD {
                1
            } else {
                crate::psy_const::TRANS_FAC
            },
            if config.user_tp_header_period != 0xFF {
                config.user_tp_header_period as i32
            } else {
                DEFAULT_HEADER_PERIOD_REPETITION_RATE
            },
            init_flag,
        );

        // Suppress AOT reconfiguration and check error status.
        if (sbr_error != 0) || (num_channels != h_aac_config.n_channels) {
            return AACENC_INIT_SBR_ERROR;
        }

        if num_channels == 1 {
            h_aac_config.channel_mode = MODE_1;
        }

        // Never use PNS if SBR is active
        if h_aac_config.use_pns != 0 {
            h_aac_config.use_pns = 0;
        }

        // estimated bitrate consumed by SBR or PS
        h_aac_config.anc_data_bit_rate = sbr_encoder_get_estimate_bitrate(Some(h_sbr));
    } // sbr initialization

    if h_aac_encoder.h_mps_enc.is_some() && (h_aac_config.syntax_flags & AC_LD_MPS != 0) {
        let mut core_coder_delay = delay_aaceld(h_aac_config.framelength);

        if is_sbr_active(h_aac_config) != 0 {
            core_coder_delay = (h_aac_config.sbr_ratio as i32)
                .wrapping_mul(core_coder_delay)
                .wrapping_add(sbr_encoder_get_input_data_delay(h_aac_encoder.h_env_enc.as_deref()));
        }

        if MPS_ENCODER_OK
            != fdk_mpegs_enc_init(
                h_aac_encoder.h_mps_enc.as_deref_mut(),
                h_aac_config.audio_object_type,
                config.user_samplerate,
                h_aac_config.bit_rate as u32,
                if is_sbr_active(h_aac_config) != 0 { h_aac_config.sbr_ratio } else { 0 },
                // for dual rate sbr this value is already multiplied by 2
                frame_length as u32,
                h_aac_encoder.input_buffer_size_per_channel as u32,
                core_coder_delay as u32,
            )
        {
            return AACENC_INIT_MPS_ERROR;
        }
    }
    h_aac_encoder.n_delay =
        f_max(fdk_mpegs_enc_get_delay(h_aac_encoder.h_mps_enc.as_deref()), h_aac_encoder.n_delay);

    // Initialize Transport - Module.
    if init_flags & AACENC_INIT_TRANSPORT != 0 {
        let mut flags: u32 = 0;

        fdk_aac_enc_map_config(
            &mut h_aac_encoder.coder_config,
            config,
            get_sbr_signaling_mode(
                h_aac_config.audio_object_type,
                config.user_tp_type,
                config.user_tp_signaling,
                h_aac_config.sbr_ratio,
            ),
            h_aac_config,
        );

        // create flags for transport encoder
        if config.user_tp_amxv != 0 {
            flags |= TP_FLAG_LATM_AMV;
        }
        // Clear output buffer
        let n = h_aac_encoder.out_buffer.len();
        h_aac_encoder.out_buffer.fill(0, n, 0);

        // Initialize Bitstream encoder
        let Some(tp) = h_aac_encoder.h_tp_enc.as_deref_mut() else {
            return AACENC_INIT_TP_ERROR;
        };
        let mut cb = EncCallbacks {
            env_enc: h_aac_encoder.h_env_enc.as_deref_mut(),
            mps_enc: h_aac_encoder.h_mps_enc.as_deref(),
        };
        let r = transport_enc_init_cb(
            tp,
            &h_aac_encoder.out_buffer,
            h_aac_encoder.out_buffer_in_bytes,
            config.user_tp_type,
            &h_aac_encoder.coder_config,
            flags,
            &mut cb,
        );
        if r != 0 {
            return AACENC_INIT_TP_ERROR;
        }
    } // transport initialization

    // Initialize AAC - Core.
    if (init_flags & AACENC_INIT_CONFIG != 0) || (init_flags & AACENC_INIT_STATES != 0) {
        let (Some(aac), Some(tp)) =
            (h_aac_encoder.h_aac_enc.as_deref_mut(), h_aac_encoder.h_tp_enc.as_deref_mut())
        else {
            return AACENC_INIT_AAC_ERROR;
        };
        let mut bound = false;
        let r = fdk_aac_enc_initialize_bound(
            aac,
            h_aac_config,
            tp,
            if init_flags & AACENC_INIT_STATES != 0 { 1 } else { 0 },
            &mut bound,
        );
        h_aac_encoder.aac_config_bound |= bound;
        if r != AAC_ENC_OK {
            return AACENC_INIT_AAC_ERROR;
        }
    } // aac initialization

    // Initialize Meta Data - Encoder.
    if h_aac_encoder.h_metadata_enc.is_some()
        && (h_aac_encoder.meta_data_allowed != 0)
        && ((init_flags & AACENC_INIT_CONFIG != 0) || (init_flags & AACENC_INIT_STATES != 0))
    {
        let mut input_data_delay = delay_aac(h_aac_config.framelength);

        if is_sbr_active(h_aac_config) != 0 {
            // hSbrEncoder (the address of hEnvEnc) is never NULL
            input_data_delay = (h_aac_config.sbr_ratio as i32)
                .wrapping_mul(input_data_delay)
                .wrapping_add(sbr_encoder_get_input_data_delay(h_aac_encoder.h_env_enc.as_deref()));
        }

        if fdk_metadata_enc_init(
            h_aac_encoder.h_metadata_enc.as_deref_mut(),
            if init_flags & AACENC_INIT_STATES != 0 { 1 } else { 0 },
            config.user_meta_data_mode as i32,
            input_data_delay,
            frame_length as u32,
            config.user_samplerate,
            config.n_channels,
            config.user_channel_mode,
            h_aac_config.channel_order,
        ) != 0
        {
            return AACENC_INIT_META_ERROR;
        }

        h_aac_encoder.n_delay = h_aac_encoder
            .n_delay
            .wrapping_add(fdk_metadata_enc_get_delay(h_aac_encoder.h_metadata_enc.as_deref()));
    }

    // Get custom delay, i.e. the codec delay w/o the decoder's SBR- or MPS delay
    if h_aac_encoder.h_mps_enc.is_some() && (h_aac_config.syntax_flags & AC_LD_MPS != 0) {
        h_aac_encoder.n_delay_core = h_aac_encoder.n_delay.wrapping_sub(f_max(
            0,
            fdk_mpegs_enc_get_dec_delay(h_aac_encoder.h_mps_enc.as_deref()),
        ));
    } else if is_sbr_active(h_aac_config) != 0 {
        // hSbrEncoder (the address of hEnvEnc) is never NULL
        h_aac_encoder.n_delay_core = h_aac_encoder.n_delay.wrapping_sub(f_max(
            0,
            sbr_encoder_get_sbr_dec_delay(h_aac_encoder.h_env_enc.as_deref()),
        ));
    } else {
        h_aac_encoder.n_delay_core = h_aac_encoder.n_delay;
    }

    // Update pointer to working buffer.
    if init_flags & AACENC_INIT_CONFIG != 0 {
        h_aac_encoder.input_buffer_offset = aac_buffer_offset;

        h_aac_encoder.n_samples_to_read = frame_length.wrapping_mul(config.n_channels as i32);
    } // parameter changed

    AACENC_OK
}

/// `aacEncOpen(phAacEncoder, encModules, maxChannels)`
pub fn aac_enc_open(
    ph_aac_encoder: Option<&mut Option<Box<AacEncoder>>>,
    enc_modules: u32,
    max_channels: u32,
) -> AacencError {
    let mut err = AACENC_OK;

    let Some(ph) = ph_aac_encoder else {
        return AACENC_INVALID_HANDLE;
    };

    // allocate memory
    let mut h = Box::new(AacEncoder::cleared());

    'bail: {
        // Specify encoder modules to be allocated.
        if enc_modules == 0 {
            let mut lib_info = [LibInfo::default(); FDK_MODULE_LAST as usize];
            fdk_init_lib_info(&mut lib_info);
            aac_enc_get_lib_info(Some(&mut lib_info));

            h.encoder_modis = ENC_MODE_FLAG_AAC;
            if fdk_lib_info_get_capabilities(&lib_info, FDK_SBRENC) & CAPF_SBR_HQ != 0 {
                h.encoder_modis |= ENC_MODE_FLAG_SBR;
            }
            if fdk_lib_info_get_capabilities(&lib_info, FDK_SBRENC) & CAPF_SBR_PS_MPEG != 0 {
                h.encoder_modis |= ENC_MODE_FLAG_PS;
            }
            if fdk_lib_info_get_capabilities(&lib_info, FDK_AACENC) & CAPF_AAC_DRC != 0 {
                h.encoder_modis |= ENC_MODE_FLAG_META;
            }
            h.encoder_modis |= ENC_MODE_FLAG_SAC;
        } else {
            h.encoder_modis = enc_modules;
        }

        // Determine max channel configuration.
        if max_channels == 0 {
            h.n_max_aac_channels = 8;
            h.n_max_sbr_channels = 8;
        } else {
            h.n_max_aac_channels = (max_channels & 0x00FF) as i32;
            if h.encoder_modis & ENC_MODE_FLAG_SBR != 0 {
                h.n_max_sbr_channels = if max_channels & 0xFF00 != 0 {
                    (max_channels >> 8) as i32
                } else {
                    h.n_max_aac_channels
                };
            }

            if (h.n_max_aac_channels > 8) || (h.n_max_sbr_channels > 8) {
                err = AACENC_INVALID_CONFIG;
                break 'bail;
            }
        } // maxChannels==0

        // Max number of elements could be tuned any more.
        h.n_max_aac_elements = fix_min(8, h.n_max_aac_channels);
        h.n_max_sbr_elements = fix_min(8, h.n_max_sbr_channels);

        // In case of memory overlay, allocate memory out of libraries

        if h.encoder_modis & (ENC_MODE_FLAG_SBR | ENC_MODE_FLAG_PS) != 0 {
            h.input_buffer_size_per_channel = INPUTBUFFER_SIZE;
        } else {
            h.input_buffer_size_per_channel = 1024;
        }

        h.input_buffer_size = h.n_max_aac_channels * h.input_buffer_size_per_channel;

        h.input_buffer = vec![0; h.input_buffer_size.max(0) as usize];

        // Open SBR Encoder
        if h.encoder_modis & ENC_MODE_FLAG_SBR != 0 {
            if sbr_encoder_open(
                Some(&mut h.h_env_enc),
                h.n_max_sbr_elements,
                h.n_max_sbr_channels,
                if h.encoder_modis & ENC_MODE_FLAG_PS != 0 { 1 } else { 0 },
            ) != 0
            {
                err = AACENC_MEMORY_ERROR;
                break 'bail;
            }

            h.p_sbr_payload = Some(Box::default());
        } // (encoder_modis&ENC_MODE_FLAG_SBR)

        // Open Aac Encoder
        if fdk_aac_enc_open(&mut h.h_aac_enc, h.n_max_aac_elements, h.n_max_aac_channels, 1)
            != AAC_ENC_OK
        {
            err = AACENC_MEMORY_ERROR;
            break 'bail;
        }

        // Bitstream output buffer: buffer has to be 2^n
        h.out_buffer_in_bytes =
            1 << (DFRACT_BITS - cnt_leading_zeros(fix_max(1, (h.n_max_aac_channels * 6144) >> 3)));
        h.out_buffer = BitMem::new(h.out_buffer_in_bytes as usize);

        // Open Meta Data Encoder
        if h.encoder_modis & ENC_MODE_FLAG_META != 0
            && fdk_metadata_enc_open(Some(&mut h.h_metadata_enc), h.n_max_aac_channels as u32) != 0
        {
            err = AACENC_MEMORY_ERROR;
            break 'bail;
        } // (encoder_modis&ENC_MODE_FLAG_META)

        // Open MPEG Surround Encoder
        if h.encoder_modis & ENC_MODE_FLAG_SAC != 0
            && MPS_ENCODER_OK != fdk_mpegs_enc_open(Some(&mut h.h_mps_enc))
        {
            err = AACENC_MEMORY_ERROR;
            break 'bail;
        } // (hAacEncoder->encoder_modis&ENC_MODE_FLAG_SAC)

        // Open Transport Encoder
        if transport_enc_open(&mut h.h_tp_enc) != 0 {
            err = AACENC_MEMORY_ERROR;
            break 'bail;
        } else {
            let mut lib_info = [LibInfo::default(); FDK_MODULE_LAST as usize];
            fdk_init_lib_info(&mut lib_info);
            transport_enc_get_lib_info(Some(&mut lib_info));

            // Get capabilty flag for transport encoder.
            h.capf_tp_enc = fdk_lib_info_get_capabilities(&lib_info, FDK_TPENC);
        }
        // transportEnc_RegisterSbrCallback() and transportEnc_RegisterSscCallback() register
        // aacenc_SbrCallback/aacenc_SscCallback with the handle as user data; they cannot fail
        // with a handle. The port passes the callbacks to the transport encoder on each call
        // (EncCallbacks).

        // Initialize encoder instance with default parameters.
        let mut ext = h.ext_param;
        aac_enc_default_config(&mut h.aac_config, &mut ext);
        h.ext_param = ext;

        // Initialize headerPeriod in coderConfig for aacEncoder_GetParam().
        h.coder_config.header_period = h.ext_param.user_tp_header_period;

        // All encoder modules have to be initialized
        h.init_flags = AACENC_INIT_ALL;

        // Return encoder instance
        *ph = Some(h);

        return err;
    }

    // bail:
    let mut hh = Some(h);
    aac_enc_close(Some(&mut hh));

    err
}

/// `aacEncClose(phAacEncoder)`
pub fn aac_enc_close(ph_aac_encoder: Option<&mut Option<Box<AacEncoder>>>) -> AacencError {
    let Some(ph) = ph_aac_encoder else {
        return AACENC_INVALID_HANDLE;
    };

    if let Some(h) = ph.as_deref_mut() {
        h.input_buffer = Vec::new();
        h.out_buffer = BitMem::default();

        if h.h_env_enc.is_some() {
            sbr_encoder_close(&mut h.h_env_enc);
        }
        h.p_sbr_payload = None;
        if h.h_aac_enc.is_some() {
            fdk_aac_enc_close(&mut h.h_aac_enc);
        }

        transport_enc_close(&mut h.h_tp_enc);

        if h.h_metadata_enc.is_some() {
            fdk_metadata_enc_close(Some(&mut h.h_metadata_enc));
        }
        if h.h_mps_enc.is_some() {
            fdk_mpegs_enc_close(Some(&mut h.h_mps_enc));
        }
    }
    *ph = None;

    AACENC_OK
}

/// `FDK_deinterleave()` of the caller's input samples into the internal input buffer: `frames`
/// samples of each of `channels` channels from `p_in` (interleaved, element size `el_size`) to
/// `out[base + ch * stride + i]`. Writes outside the buffer are dropped.
///
/// # Safety
/// `p_in` must be valid for reads of `channels * frames` elements of the element type.
unsafe fn deinterleave_input(
    p_in: *const c_void,
    el_size: i32,
    out: &mut [IntPcm],
    base: usize,
    channels: u32,
    frames: u32,
    stride: u32,
) {
    let (n_ch, n, step) = (channels as usize, frames as usize, stride as usize);
    // Every write inside the buffer: the same loops with the element size decided once and
    // without the per-sample bounds check (the channels are written in the same order, so
    // overlapping channel regions end the same).
    if n_ch == 0 || n == 0 {
        return;
    }
    let fits = (n_ch - 1)
        .checked_mul(step)
        .and_then(|o| o.checked_add(base))
        .and_then(|o| o.checked_add(n))
        .is_some_and(|end| end <= out.len());
    if !fits {
        // SAFETY: the caller's guarantee.
        return unsafe {
            deinterleave_input_straight(p_in, el_size, out, base, channels, frames, stride)
        };
    }
    for ch in 0..n_ch {
        let o = &mut out[base + ch * step..base + ch * step + n];
        // SAFETY: the caller's guarantee (sample k = sample * channels + ch < channels * frames).
        unsafe {
            if el_size > core::mem::size_of::<IntPcm>() as i32 {
                // FDK_deinterleave(const LONG *, SHORT *): (SHORT)(In[0] >> 16)
                let p = (p_in as *const i32).add(ch);
                for (sample, v) in o.iter_mut().enumerate() {
                    *v = (p.add(sample * n_ch).read_unaligned() >> 16) as i16;
                }
            } else {
                // FDK_deinterleave(const SHORT *, SHORT *)
                let p = (p_in as *const i16).add(ch);
                for (sample, v) in o.iter_mut().enumerate() {
                    *v = p.add(sample * n_ch).read_unaligned();
                }
            }
        }
    }
}

/// [`deinterleave_input`] (straight port, kept as its reference and for the inputs with writes
/// outside the buffer).
///
/// # Safety
/// As [`deinterleave_input`].
unsafe fn deinterleave_input_straight(
    p_in: *const c_void,
    el_size: i32,
    out: &mut [IntPcm],
    base: usize,
    channels: u32,
    frames: u32,
    stride: u32,
) {
    for ch in 0..channels as usize {
        for sample in 0..frames as usize {
            let k = sample * channels as usize + ch;
            // SAFETY: the caller's guarantee.
            let v: IntPcm = unsafe {
                if el_size == core::mem::size_of::<IntPcm>() as i32 {
                    // FDK_deinterleave(const SHORT *, SHORT *)
                    (p_in as *const i16).add(k).read_unaligned()
                } else if el_size > core::mem::size_of::<IntPcm>() as i32 {
                    // FDK_deinterleave(const LONG *, SHORT *): (SHORT)(In[0] >> 16)
                    ((p_in as *const i32).add(k).read_unaligned() >> 16) as i16
                } else {
                    // FDK_deinterleave(const SHORT *, SHORT *)
                    (p_in as *const i16).add(k).read_unaligned()
                }
            };
            if let Some(o) = out.get_mut(base + ch * stride as usize + sample) {
                *o = v;
            }
        }
    }
}

/// `aacEncEncode(hAacEncoder, inBufDesc, outBufDesc, inargs, outargs)`
///
/// # Safety
/// The descriptors must be valid as in C: their arrays valid for `numBufs` entries, the input
/// buffers valid for reads of the samples and bytes announced in `inargs` (`numInSamples`
/// elements of the audio buffer, `numAncBytes` bytes of the ancillary data, an
/// `AACENC_MetaData` for `IN_METADATA_SETUP`), and the output buffers valid for writes of
/// their `bufSizes` (and of one `INT` per access unit for `OUT_AU_SIZES`).
pub unsafe fn aac_enc_encode(
    h_aac_encoder: Option<&mut AacEncoder>,
    in_buf_desc: Option<&AacencBufDesc>,
    out_buf_desc: Option<&AacencBufDesc>,
    inargs: Option<&AacencInArgs>,
    outargs: Option<&mut AacencOutArgs>,
) -> AacencError {
    // deal with valid encoder handle
    let Some(h) = h_aac_encoder else {
        return AACENC_INVALID_HANDLE;
    };
    // (the output delay of the multi-threading, not in C; it starts in encode())
    if h.pipe.delayed {
        // SAFETY: the caller's guarantee.
        return unsafe { pipeline::encode_delayed(h, in_buf_desc, out_buf_desc, inargs, outargs) };
    }
    // SAFETY: the caller's guarantee.
    let err = unsafe { encode(h, in_buf_desc, out_buf_desc, inargs, outargs) };
    if err == AACENC_ENCODE_ERROR {
        // All encoder modules have to be initialized
        h.init_flags = AACENC_INIT_ALL;
    }
    err
}

/// The front part of `aacEncEncode()` (up to the core): the initialization, the input samples,
/// the metadata, MPEG Surround and SBR encoders and the extension payloads of the frame, in
/// `h.ext_payload`. `opts` gives the channel mapping of the core (if it is not in `h`) and
/// whether the output buffers are cleared here, and receives what C did to the output
/// arguments and buffers.
///
/// # Safety
/// As [`aac_enc_encode`].
unsafe fn front(
    h: &mut AacEncoder,
    in_buf_desc: Option<&AacencBufDesc>,
    out_buf_desc: Option<&AacencBufDesc>,
    inargs: Option<&AacencInArgs>,
    mut outargs: Option<&mut AacencOutArgs>,
    opts: &mut FrontOpts,
) -> Front {
    let mut n_extensions: usize = 0;
    let mut anc_data_ext_idx: i32 = -1;

    // Adjust user settings and trigger reinitialization.
    if h.init_flags != 0 {
        let mut ext = h.ext_param;
        let err = aac_enc_init(h, h.init_flags, &mut ext);
        h.ext_param = ext;

        if err != AACENC_OK {
            // keep init flags alive!
            return Front::Return(err);
        }
        h.init_flags = AACENC_INIT_NONE;
    }

    if let Some(o) = outargs.as_deref_mut() {
        *o = AacencOutArgs::default();
        opts.cleared = true;
    }

    if let Some(d) = out_buf_desc {
        if !d.bufs.is_null() && !d.buf_sizes.is_null() {
            opts.zeroed = true;
        }
        if !d.bufs.is_null() && !d.buf_sizes.is_null() && !opts.defer_zero {
            for i in 0..d.num_bufs.max(0) as usize {
                // SAFETY: the caller's guarantee.
                unsafe {
                    let b = *d.bufs.add(i);
                    if !b.is_null() {
                        let n = *d.buf_sizes.add(i);
                        core::ptr::write_bytes(b as *mut u8, 0, n.max(0) as usize);
                    }
                }
            }
        }
    }

    // If only encoder handle given, independent (re)initialization can be triggered.
    if in_buf_desc.is_none() && out_buf_desc.is_none() && inargs.is_none() && outargs.is_none() {
        return Front::Return(AACENC_OK);
    }

    // check if buffer descriptors are filled out properly.
    let (Some(inargs), Some(outargs)) = (inargs, outargs) else {
        return Front::Return(AACENC_UNSUPPORTED_PARAMETER);
    };
    // SAFETY: the caller's guarantee.
    if ((AACENC_OK != unsafe { validate_buf_desc(in_buf_desc) }) && (inargs.num_in_samples > 0))
        || (AACENC_OK != unsafe { validate_buf_desc(out_buf_desc) })
    {
        return Front::Return(AACENC_UNSUPPORTED_PARAMETER);
    }

    // reset buffer wich signals number of valid bytes in output bitstream buffer (outBytes[0] =
    // 0: the core writes it, see encode())

    let ext_n_channels = h.ext_param.n_channels as i32;

    // Manage incoming audio samples.
    // SAFETY: the caller's guarantee.
    let idx = unsafe { get_buf_desc_idx(in_buf_desc, IN_AUDIO_DATA) };
    if let Some(d) = in_buf_desc {
        if inargs.num_in_samples > 0 && idx != -1 {
            // Fetch data until nSamplesToRead reached
            let mut new_samples = fix_max(
                0,
                fix_min(inargs.num_in_samples, h.n_samples_to_read.wrapping_sub(h.n_samples_read)),
            );
            let base = sdiv_i(h.input_buffer_offset, h.aac_config.n_channels)
                .wrapping_add(sdiv_i(h.n_samples_read, ext_n_channels));
            // process multiple samples of input channels
            new_samples -= srem_i(new_samples, ext_n_channels);

            // Copy new input samples to internal buffer
            // SAFETY: the caller's guarantee (the descriptor was validated above).
            unsafe {
                let el_size = *d.buf_el_sizes.add(idx as usize);
                let p = *d.bufs.add(idx as usize);
                deinterleave_input(
                    p,
                    el_size,
                    &mut h.input_buffer,
                    base.max(0) as usize,
                    ext_n_channels as u32,
                    sdiv_i(new_samples, ext_n_channels) as u32,
                    h.input_buffer_size_per_channel as u32,
                );
            }
            h.n_samples_read += new_samples;

            // Number of fetched input buffer samples.
            outargs.num_in_samples = new_samples;
        }
    }

    // input buffer completely filled ?
    if h.n_samples_read < h.n_samples_to_read {
        // - eof reached and flushing enabled, or
        // - return to main and wait for further incoming audio samples
        if inargs.num_in_samples == -1 {
            if h.n_zeros_appended < h.n_delay {
                let n_zeros =
                    sdiv_i(h.n_samples_to_read.wrapping_sub(h.n_samples_read), ext_n_channels);

                fdk_assert!(n_zeros >= 0);

                // clear out until end-of-buffer
                if n_zeros != 0 {
                    let base = sdiv_i(h.input_buffer_offset, h.aac_config.n_channels)
                        .wrapping_add(sdiv_i(h.n_samples_read, ext_n_channels));
                    for i in 0..ext_n_channels.max(0) {
                        let s = base.wrapping_add(i.wrapping_mul(h.input_buffer_size_per_channel));
                        for k in 0..n_zeros.max(0) {
                            if let Some(v) = usize::try_from(s.wrapping_add(k))
                                .ok()
                                .and_then(|k| h.input_buffer.get_mut(k))
                            {
                                *v = 0;
                            }
                        }
                    }
                    h.n_zeros_appended += n_zeros;
                    h.n_samples_read = h.n_samples_to_read;
                }
            } else {
                // flushing completed
                return Front::Return(AACENC_ENCODE_EOF); // eof reached
            }
        } else {
            // inargs->numInSamples!= -1: not enough samples in input buffer and no flushing
            // enabled
            return Front::Return(AACENC_OK);
        }
    }

    // The reference has no defined result for a core that was never initialized; the port
    // returns an error (PORTING.md, "Deviations from the reference").
    if !h.aac_config_bound {
        return Front::Return(AACENC_ENCODE_ERROR);
    }

    // init payload
    h.ext_payload = [AacencExtPayload::default(); MAX_TOTAL_EXT_PAYLOADS];
    for p in h.ext_payload.iter_mut() {
        p.associated_ch_element = -1;
    }
    if let Some(p) = h.p_sbr_payload.as_deref_mut() {
        p.data_size = [[0; 8]; 1];
        p.data = [[[0; MAX_PAYLOAD_SIZE]; 8]; 1];
    }

    let core_offset = sdiv_i(h.input_buffer_offset, h.coder_config.no_channels).max(0) as usize;

    // Calculate Meta Data info.
    if h.h_metadata_enc.is_some() && (h.meta_data_allowed != 0) {
        let mut p_meta_data: Option<AacencMetaData> = None;
        let mut have_payload = false;
        let mut n_meta_data_extensions: u32 = 0;
        let mut matrix_mixdown_idx: i32 = 0;

        // New meta data info available ?
        // SAFETY: the caller's guarantee.
        let mi = unsafe { get_buf_desc_idx(in_buf_desc, IN_METADATA_SETUP) };
        if mi != -1 {
            if let Some(d) = in_buf_desc {
                // SAFETY: the caller's guarantee (an AACENC_MetaData).
                p_meta_data = unsafe {
                    let p = *d.bufs.add(mi as usize) as *const AacencMetaData;
                    if p.is_null() {
                        None
                    } else {
                        Some(p.read_unaligned())
                    }
                };
            }
        }

        let size = h.input_buffer_size_per_channel as u32;
        let n_read = h.n_samples_read;
        if let Some(md) = h.h_metadata_enc.as_deref_mut() {
            let samples = h.input_buffer.get_mut(core_offset..).unwrap_or(&mut []);
            fdk_metadata_enc_process(
                md,
                samples,
                size,
                n_read,
                p_meta_data.as_ref(),
                &mut have_payload,
                &mut n_meta_data_extensions,
                &mut matrix_mixdown_idx,
            );

            // Get meta data extension payload.
            for i in 0..n_meta_data_extensions as usize {
                if have_payload && i < 2 && n_extensions < MAX_TOTAL_EXT_PAYLOADS {
                    h.ext_payload[n_extensions] = md.ex_payload[i];
                }
                n_extensions += 1;
            }
        }

        if (matrix_mixdown_idx != -1)
            && ((h.ext_param.user_channel_mode == MODE_1_2_2)
                || (h.ext_param.user_channel_mode == MODE_1_2_2_1))
        {
            // Set matrix mixdown coefficient.
            let pce_value = ((0 << 3) | ((matrix_mixdown_idx & 0x3) << 1) | 1) as u32;
            if h.ext_param.user_pce_additions != pce_value {
                h.ext_param.user_pce_additions = pce_value;
                h.init_flags |= AACENC_INIT_TRANSPORT;
            }
        }
    }

    // Encode MPS data.
    if h.h_mps_enc.is_some() && (h.aac_config.syntax_flags & AC_LD_MPS != 0) {
        let mut mps_extension_payload = AacencExtPayload::default();

        let p = h.input_buffer.as_mut_ptr().wrapping_add(core_offset);
        // SAFETY: the MPEG Surround encoder reads the two input channels and writes the downmix
        // inside the input buffer (sizes set up by aacEncInit()).
        if MPS_ENCODER_OK
            != unsafe {
                fdk_mpegs_enc_process(
                    h.h_mps_enc.as_deref_mut(),
                    p,
                    h.n_samples_read,
                    &mut mps_extension_payload,
                )
            }
        {
            return Front::Return(AACENC_ENCODE_ERROR);
        }

        if !mps_extension_payload.p_data.is_null() && (mps_extension_payload.data_size != 0) {
            if n_extensions < MAX_TOTAL_EXT_PAYLOADS {
                h.ext_payload[n_extensions] = mps_extension_payload;
            }
            n_extensions += 1;
        }
    }

    if h.h_env_enc.is_some() && h.p_sbr_payload.is_some() && is_sbr_active(&h.aac_config) != 0 {
        let n_payload = 0usize;
        // the core's channel mapping
        let cm = opts.cm.or_else(|| h.h_aac_enc.as_deref().map(|a| a.channel_mapping));

        // Encode SBR data.
        let (Some(env), Some(pl)) = (h.h_env_enc.as_deref_mut(), h.p_sbr_payload.as_deref_mut())
        else {
            return Front::Return(AACENC_ENCODE_ERROR); // checked above
        };
        // An SBR encoder whose initialization failed has no channels, for which
        // sbrEncoder_EncodeFrame() has no defined result; the port returns an error instead.
        if env.n_channels == 0 {
            return Front::Return(AACENC_ENCODE_ERROR);
        }
        if sbr_encoder_encode_frame(
            env,
            &mut h.input_buffer,
            h.input_buffer_size_per_channel as u32,
            &mut pl.data_size[n_payload],
            &mut pl.data[n_payload],
        ) != 0
        {
            return Front::Return(AACENC_ENCODE_ERROR);
        } else {
            // Add SBR extension payload
            for i in 0..8 {
                if pl.data_size[n_payload][i] > 0 {
                    let associated = match (env.sbr_element[i].as_deref(), cm.as_ref()) {
                        (Some(e), Some(cm)) => get_associated_ch_element(&e.el_info, cm),
                        // (never `None` for an element with data)
                        _ => -1,
                    };
                    if n_extensions < MAX_TOTAL_EXT_PAYLOADS {
                        let x = &mut h.ext_payload[n_extensions];
                        x.p_data = pl.data[n_payload][i].as_ptr();
                        x.data_size = pl.data_size[n_payload][i];
                        x.associated_ch_element = associated;
                    }
                    if associated == -1 {
                        return Front::Return(AACENC_ENCODE_ERROR);
                    }
                    if n_extensions < MAX_TOTAL_EXT_PAYLOADS {
                        // Once SBR Encoder supports SBR CRC set EXT_SBR_DATA_CRC
                        h.ext_payload[n_extensions].data_type = EXT_SBR_DATA;
                    }
                    n_extensions += 1; // or EXT_SBR_DATA according to configuration.
                    fdk_assert!(n_extensions <= MAX_TOTAL_EXT_PAYLOADS);
                }
            }
        }
    } // sbrEnabled

    // SAFETY: the caller's guarantee.
    let ai = unsafe { get_buf_desc_idx(in_buf_desc, IN_ANCILLRY_DATA) };
    if (inargs.num_anc_bytes > 0) && ai != -1 {
        if let Some(d) = in_buf_desc {
            if n_extensions < MAX_TOTAL_EXT_PAYLOADS {
                let x = &mut h.ext_payload[n_extensions];
                x.data_size = (inargs.num_anc_bytes as u32).wrapping_mul(8);
                // SAFETY: the caller's guarantee.
                x.p_data = unsafe { *d.bufs.add(ai as usize) } as *const u8;
                x.data_type = EXT_DATA_ELEMENT;
                x.associated_ch_element = -1;
            }
            anc_data_ext_idx = n_extensions as i32; // store index
                                                    // nExtensions++ (not read afterwards)
        }
    }

    Front::Frame { anc_data_ext_idx }
}

/// What [`front`] produced.
enum Front {
    /// `aacEncEncode()` returns this before the core.
    Return(AacencError),
    /// A frame for the core; its extension payloads are in `AacEncoder::ext_payload`, the
    /// ancillary data at `anc_data_ext_idx` (-1: none).
    Frame { anc_data_ext_idx: i32 },
}

/// Options and results of [`front`].
#[derive(Default)]
struct FrontOpts {
    /// The channel mapping of the core (`None`: the one of `h.h_aac_enc`).
    cm: Option<ChannelMapping>,
    /// Leave the output buffers as they are (the output delay clears them when it returns the
    /// result of the call).
    defer_zero: bool,
    /// Set if C cleared the output arguments (and numInSamples is set).
    cleared: bool,
    /// Set if C cleared the output buffers (or would have, with `defer_zero`).
    zeroed: bool,
}

/// The body of `aacEncEncode()` up to `bail`, without output delay: the front part, the core
/// and the output.
unsafe fn encode(
    h: &mut AacEncoder,
    in_buf_desc: Option<&AacencBufDesc>,
    out_buf_desc: Option<&AacencBufDesc>,
    inargs: Option<&AacencInArgs>,
    mut outargs: Option<&mut AacencOutArgs>,
) -> AacencError {
    let mut opts = FrontOpts::default();
    // SAFETY: the caller's guarantee.
    let f =
        unsafe { front(h, in_buf_desc, out_buf_desc, inargs, outargs.as_deref_mut(), &mut opts) };
    // (the opt-in multi-threading, not in C: with SBR, the output delay starts here)
    if outargs.is_some() && h.threads.get() > 1 && is_sbr_active(&h.aac_config) != 0 {
        // SAFETY: the caller's guarantee.
        return unsafe { pipeline::enter(h, f, &opts, inargs, out_buf_desc) };
    }
    let anc_data_ext_idx = match f {
        Front::Return(err) => return err,
        Front::Frame { anc_data_ext_idx } => anc_data_ext_idx,
    };
    // (front() returns a frame only with both)
    let (Some(inargs), Some(outargs)) = (inargs, outargs) else {
        return AACENC_UNSUPPORTED_PARAMETER;
    };
    let mut out_bytes = [0i32; 1];

    // Encode AAC - Core.
    {
        // hAacEnc->config points to aacConfig (see crate::aacenc)
        let (Some(aac), Some(tp)) = (h.h_aac_enc.as_deref_mut(), h.h_tp_enc.as_deref_mut()) else {
            return AACENC_ENCODE_ERROR;
        };
        aac.config = h.aac_config;
        let mut cb =
            EncCallbacks { env_enc: h.h_env_enc.as_deref_mut(), mps_enc: h.h_mps_enc.as_deref() };
        // SAFETY: the payloads point to the caller's ancillary data (the caller's guarantee)
        // and to buffers of the encoder.
        // (the opt-in multi-threading, not in C: `None` with one thread)
        let par = h.threads.par(aac.channel_mapping.n_channels);
        let r = unsafe {
            fdk_aac_enc_encode_frame_pool(
                aac,
                tp,
                &h.input_buffer,
                h.input_buffer_size_per_channel as u32,
                &mut out_bytes,
                &mut h.ext_payload,
                &mut cb,
                par,
            )
        };
        if r != AAC_ENC_OK {
            return AACENC_ENCODE_ERROR;
        }
    }

    if anc_data_ext_idx >= 0 && (anc_data_ext_idx as usize) < MAX_TOTAL_EXT_PAYLOADS {
        outargs.num_anc_bytes = inargs
            .num_anc_bytes
            .wrapping_sub((h.ext_payload[anc_data_ext_idx as usize].data_size >> 3) as i32);
    }

    // samples exhausted
    h.n_samples_read -= h.n_samples_to_read;

    // Delay balancing buffer handling
    if is_sbr_active(&h.aac_config) != 0 {
        if let Some(env) = h.h_env_enc.as_deref_mut() {
            sbr_encoder_update_buffers(
                env,
                &mut h.input_buffer,
                h.input_buffer_size_per_channel as u32,
            );
        }
    }

    // Make bitstream public
    if let Some(d) = out_buf_desc {
        if d.num_bufs >= 1 {
            // SAFETY: the caller's guarantee (validated above).
            let bs_idx = unsafe { get_buf_desc_idx(out_buf_desc, OUT_BITSTREAM_DATA) };
            let au_idx = unsafe { get_buf_desc_idx(out_buf_desc, OUT_AU_SIZES) };

            let mut n_bs_bytes = 0;
            for &ob in out_bytes.iter().take(h.aac_config.n_sub_frames.clamp(0, 1) as usize) {
                n_bs_bytes += ob;

                if au_idx != -1 {
                    // SAFETY: the caller's guarantee.
                    unsafe { (*d.bufs.add(au_idx as usize) as *mut i32).write_unaligned(ob) };
                }
            }

            // SAFETY: the caller's guarantee.
            if (bs_idx != -1) && (unsafe { *d.buf_sizes.add(bs_idx as usize) } >= n_bs_bytes) {
                let n = (n_bs_bytes.max(0) as usize).min(h.out_buffer.len());
                // SAFETY: the caller's guarantee (bufSizes[bsIdx] >= nBsBytes).
                let dst = unsafe {
                    core::slice::from_raw_parts_mut(*d.bufs.add(bs_idx as usize) as *mut u8, n)
                };
                h.out_buffer.read(0, dst);
                outargs.num_out_bytes = n_bs_bytes;
                outargs.bit_res_state =
                    h.h_aac_enc.as_deref().map_or(0, fdk_aac_enc_get_bit_reservoir_state);
            } else {
                // output buffer too small, can't write valid bitstream
                return AACENC_ENCODE_ERROR;
            }
        }
    }

    AACENC_OK
}

/// `a / b` with the arm64 result for `b == 0` (see the module documentation).
#[inline]
fn sdiv_i(a: i32, b: i32) -> i32 {
    if b == 0 {
        0
    } else {
        a.wrapping_div(b)
    }
}

/// `a % b` with the arm64 result for `b == 0` (`msub` after `sdiv`: `a`).
#[inline]
fn srem_i(a: i32, b: i32) -> i32 {
    if b == 0 {
        a
    } else {
        a.wrapping_rem(b)
    }
}

/// `aacEncGetConf(hAacEncoder, size, confBuffer)`
fn aac_enc_get_conf(
    h: &mut AacEncoder,
    size: &mut u32,
    conf_buffer: &mut [u8; 64],
) -> AacEncoderError {
    let mut tmp_conf = FdkBitStream::default();
    let mut conf_type: u32 = 0;
    // Init bit buffer
    fdk_init_bit_stream(&mut tmp_conf, &BitMem::new(64), 64, 0, BS_WRITER);

    // write conf in tmp buffer
    let Some(tp) = h.h_tp_enc.as_deref_mut() else {
        return AAC_ENC_UNKNOWN;
    };
    let mut cb =
        EncCallbacks { env_enc: h.h_env_enc.as_deref_mut(), mps_enc: h.h_mps_enc.as_deref() };
    let err =
        transport_enc_get_conf_cb(tp, &mut h.coder_config, &mut tmp_conf, &mut conf_type, &mut cb);

    // copy data to outbuffer: length in bytes
    fdk_byte_align(&mut tmp_conf, 0);

    // Check buffer size
    if fdk_get_valid_bits(&mut tmp_conf) > ((*size) << 3) {
        return AAC_ENC_UNKNOWN;
    }

    fdk_fetch_buffer(&mut tmp_conf, conf_buffer, size);

    if err != 0 {
        AAC_ENC_UNKNOWN
    } else {
        AAC_ENC_OK
    }
}

/// `aacEncGetLibInfo(info)`: enters the library information of the encoder and its modules
/// into `info` (an array of `FDK_MODULE_LAST` entries).
pub fn aac_enc_get_lib_info(info: Option<&mut [LibInfo]>) -> AacencError {
    let Some(info) = info else {
        return AACENC_INVALID_HANDLE;
    };

    fdk_tools_get_lib_info(Some(info));
    transport_enc_get_lib_info(Some(info));
    sbr_encoder_get_lib_info(Some(info));
    fdk_mpegs_enc_get_lib_info(Some(info));

    // search for next free tab
    let mut i = 0usize;
    while i < FDK_MODULE_LAST as usize {
        match info.get(i) {
            Some(e) if e.module_id == FDK_NONE => break,
            Some(_) => {}
            // C reads FDK_MODULE_LAST entries; a shorter slice ends the search
            None => return AACENC_INIT_ERROR,
        }
        i += 1;
    }
    if i == FDK_MODULE_LAST as usize {
        return AACENC_INIT_ERROR;
    }

    let e = &mut info[i];
    e.module_id = FDK_AACENC;
    e.build_date = AACENCODER_LIB_BUILD_DATE;
    e.build_time = AACENCODER_LIB_BUILD_TIME;
    e.title = AACENCODER_LIB_TITLE;
    e.version = lib_version(AACENCODER_LIB_VL0, AACENCODER_LIB_VL1, AACENCODER_LIB_VL2);
    lib_version_string(e);

    // Capability flags
    e.flags = CAPF_AAC_1024
        | CAPF_AAC_LC
        | CAPF_AAC_512
        | CAPF_AAC_480
        | CAPF_AAC_DRC
        | CAPF_AAC_ELD_DOWNSCALE;
    // End of flags

    AACENC_OK
}

/// `aacEncoder_SetParam(hAacEncoder, param, value)`
pub fn aac_encoder_set_param(
    h_aac_encoder: Option<&mut AacEncoder>,
    param: AacencParam,
    value: u32,
) -> AacencError {
    let mut err = AACENC_OK;

    // check encoder handle
    let Some(h) = h_aac_encoder else {
        return AACENC_INVALID_HANDLE;
    };
    // (output delay, not in C: the core of the pending frame comes first)
    pipeline::drain(h);
    let settings = &mut h.ext_param;

    // apply param value
    match param {
        AACENC_AOT => {
            if settings.user_aot != value as AudioObjectType {
                // check if AOT matches the allocated modules
                let modis = h.encoder_modis;
                let ok = match value as AudioObjectType {
                    AOT_PS => {
                        modis & ENC_MODE_FLAG_PS != 0
                            && modis & ENC_MODE_FLAG_SBR != 0
                            && modis & ENC_MODE_FLAG_AAC != 0
                    }
                    AOT_SBR | AOT_MP2_SBR => {
                        modis & ENC_MODE_FLAG_SBR != 0 && modis & ENC_MODE_FLAG_AAC != 0
                    }
                    AOT_AAC_LC | AOT_MP2_AAC_LC | AOT_ER_AAC_LD | AOT_ER_AAC_ELD => {
                        modis & ENC_MODE_FLAG_AAC != 0
                    }
                    _ => false,
                };
                if !ok {
                    return AACENC_INVALID_CONFIG;
                }
                settings.user_aot = value as AudioObjectType;
                h.init_flags |= AACENC_INIT_CONFIG | AACENC_INIT_STATES | AACENC_INIT_TRANSPORT;
            }
        }
        AACENC_BITRATE => {
            if settings.user_bitrate != value {
                settings.user_bitrate = value;
                h.init_flags |= AACENC_INIT_CONFIG | AACENC_INIT_TRANSPORT;
            }
        }
        AACENC_BITRATEMODE => {
            if settings.user_bitrate_mode != value {
                match value {
                    0..=5 => {
                        settings.user_bitrate_mode = value;
                        h.init_flags |= AACENC_INIT_CONFIG | AACENC_INIT_TRANSPORT;
                    }
                    _ => err = AACENC_INVALID_CONFIG,
                }
            }
        }
        AACENC_SAMPLERATE => {
            if settings.user_samplerate != value {
                if !matches!(
                    value,
                    8000 | 11025
                        | 12000
                        | 16000
                        | 22050
                        | 24000
                        | 32000
                        | 44100
                        | 48000
                        | 64000
                        | 88200
                        | 96000
                ) {
                    return AACENC_INVALID_CONFIG;
                }
                settings.user_samplerate = value;
                h.n_samples_read = 0; // reset internal inputbuffer
                h.init_flags |= AACENC_INIT_CONFIG | AACENC_INIT_STATES | AACENC_INIT_TRANSPORT;
            }
        }
        AACENC_CHANNELMODE => {
            if settings.user_channel_mode != value as ChannelMode {
                if (value as ChannelMode == MODE_212) && h.h_mps_enc.is_some() {
                    // Deviation: the port refuses MODE_212 where the reference has no defined
                    // result.
                    if h.n_max_aac_channels < 2 {
                        return AACENC_INVALID_CONFIG;
                    }
                    settings.user_channel_mode = value as ChannelMode;
                    settings.n_channels = 2;
                } else {
                    let Some(p_config) =
                        fdk_aac_enc_get_channel_mode_configuration(value as ChannelMode)
                    else {
                        return AACENC_INVALID_CONFIG;
                    };
                    if (p_config.n_elements > h.n_max_aac_elements)
                        || (p_config.n_channels_eff > h.n_max_aac_channels)
                    {
                        return AACENC_INVALID_CONFIG;
                    }

                    settings.user_channel_mode = value as ChannelMode;
                    settings.n_channels = p_config.n_channels as u32;
                }
                h.n_samples_read = 0; // reset internal inputbuffer
                h.init_flags |= AACENC_INIT_CONFIG | AACENC_INIT_TRANSPORT;
                if !((1..=6).contains(&value)) {
                    h.init_flags |= AACENC_INIT_STATES;
                }
            }
        }
        AACENC_BANDWIDTH => {
            if settings.user_bandwidth != value {
                settings.user_bandwidth = value;
                h.init_flags |= AACENC_INIT_CONFIG;
            }
        }
        AACENC_CHANNELORDER => {
            if h.aac_config.channel_order != value as ChannelOrder {
                if !(value == 0 || value == 1 || value == 2) {
                    return AACENC_INVALID_CONFIG;
                }
                h.aac_config.channel_order = value as ChannelOrder;
                h.n_samples_read = 0; // reset internal inputbuffer
                h.init_flags |= AACENC_INIT_CONFIG | AACENC_INIT_STATES | AACENC_INIT_TRANSPORT;
            }
        }
        AACENC_AFTERBURNER => {
            if settings.user_afterburner != value {
                if !(value == 0 || value == 1) {
                    return AACENC_INVALID_CONFIG;
                }
                settings.user_afterburner = value;
                h.init_flags |= AACENC_INIT_CONFIG;
            }
        }
        AACENC_GRANULE_LENGTH => {
            if settings.user_framelength != value {
                match value {
                    1024 | 512 | 480 | 256 | 240 | 128 | 120 => {
                        if (value << 1) == 480 || (value << 1) == 512 {
                            settings.user_downscale_factor = 2;
                        } else if (value << 2) == 480 || (value << 2) == 512 {
                            settings.user_downscale_factor = 4;
                        }
                        settings.user_framelength = value;
                        h.init_flags |= AACENC_INIT_CONFIG | AACENC_INIT_TRANSPORT;
                    }
                    _ => err = AACENC_INVALID_CONFIG,
                }
            }
        }
        AACENC_SBR_RATIO => {
            if settings.user_sbr_ratio != value {
                if !(value == 0 || value == 1 || value == 2) {
                    return AACENC_INVALID_CONFIG;
                }
                settings.user_sbr_ratio = value;
                h.init_flags |= AACENC_INIT_CONFIG | AACENC_INIT_STATES | AACENC_INIT_TRANSPORT;
            }
        }
        AACENC_SBR_MODE => {
            // UCHAR userSbrEnabled != UINT value: compared after the promotion to UINT
            if (settings.user_sbr_enabled as u32 != value) && h.h_env_enc.is_some() {
                settings.user_sbr_enabled = value as u8;
                h.init_flags |= AACENC_INIT_CONFIG | AACENC_INIT_STATES | AACENC_INIT_TRANSPORT;
            }
        }
        AACENC_TRANSMUX => {
            if settings.user_tp_type != value as TransportType {
                let type_ = value as TransportType;
                let flags = h.capf_tp_enc;

                if !(((type_ == TT_MP4_ADIF) && (flags & CAPF_ADIF != 0))
                    || ((type_ == TT_MP4_ADTS) && (flags & CAPF_ADTS != 0))
                    || ((type_ == TT_MP4_LATM_MCP0)
                        && ((flags & CAPF_LATM != 0) && (flags & CAPF_RAWPACKETS != 0)))
                    || ((type_ == TT_MP4_LATM_MCP1)
                        && ((flags & CAPF_LATM != 0) && (flags & CAPF_RAWPACKETS != 0)))
                    || ((type_ == TT_MP4_LOAS) && (flags & CAPF_LOAS != 0))
                    || ((type_ == TT_MP4_RAW) && (flags & CAPF_RAWPACKETS != 0)))
                {
                    return AACENC_INVALID_CONFIG;
                }
                settings.user_tp_type = value as TransportType;
                h.init_flags |= AACENC_INIT_TRANSPORT;
            }
        }
        AACENC_SIGNALING_MODE => {
            if settings.user_tp_signaling as u32 != value {
                if !(value == 0 || value == 1 || value == 2) {
                    return AACENC_INVALID_CONFIG;
                }
                settings.user_tp_signaling = value as u8;
                h.init_flags |= AACENC_INIT_TRANSPORT;
            }
        }
        AACENC_PROTECTION => {
            if settings.user_tp_protection as u32 != value {
                if !(value == 0 || value == 1) {
                    return AACENC_INVALID_CONFIG;
                }
                settings.user_tp_protection = value as u8;
                h.init_flags |= AACENC_INIT_TRANSPORT;
            }
        }
        AACENC_HEADER_PERIOD => {
            if settings.user_tp_header_period as u32 != value {
                if !((value as i32 >= 0) && (value <= 255)) {
                    return AACENC_INVALID_CONFIG;
                }
                settings.user_tp_header_period = value as u8;
                h.init_flags |= AACENC_INIT_TRANSPORT;
            }
        }
        AACENC_AUDIOMUXVER => {
            if settings.user_tp_amxv as u32 != value {
                if !(value == 0 || value == 1 || value == 2) {
                    return AACENC_INVALID_CONFIG;
                }
                settings.user_tp_amxv = value as u8;
                h.init_flags |= AACENC_INIT_TRANSPORT;
            }
        }
        AACENC_TPSUBFRAMES => {
            if settings.user_tp_nsub_frames as u32 != value {
                if !((1..=4).contains(&value)) {
                    return AACENC_INVALID_CONFIG;
                }
                settings.user_tp_nsub_frames = value as u8;
                h.init_flags |= AACENC_INIT_TRANSPORT;
            }
        }
        AACENC_ANCILLARY_BITRATE => {
            if settings.user_anc_data_rate != value {
                settings.user_anc_data_rate = value;
            }
        }
        AACENC_CONTROL_STATE => {
            if h.init_flags != value {
                if value & AACENC_RESET_INBUFFER != 0 {
                    h.n_samples_read = 0;
                }
                h.init_flags = value;
            }
        }
        AACENC_METADATA_MODE => {
            if settings.user_meta_data_mode as u32 != value {
                if !((value as i32 >= 0) && (value as i32 <= 3)) {
                    return AACENC_INVALID_CONFIG;
                }
                settings.user_meta_data_mode = value as u8;
                h.init_flags |= AACENC_INIT_CONFIG;
            }
        }
        AACENC_PEAK_BITRATE => {
            if settings.user_peak_bitrate != value {
                settings.user_peak_bitrate = value;
                h.init_flags |= AACENC_INIT_CONFIG | AACENC_INIT_TRANSPORT;
            }
        }
        _ => err = AACENC_UNSUPPORTED_PARAMETER,
    } // switch(param)

    err
}

/// `aacEncoder_GetParam(hAacEncoder, param)`
pub fn aac_encoder_get_param(h_aac_encoder: Option<&AacEncoder>, param: AacencParam) -> u32 {
    // check encoder handle
    let Some(h) = h_aac_encoder else {
        return 0;
    };
    let settings = &h.ext_param;
    let c = &h.aac_config;

    // apply param value
    match param {
        AACENC_AOT => c.audio_object_type as u32,
        AACENC_BITRATE => match c.bitrate_mode {
            AACENC_BR_MODE_CBR => c.bit_rate as u32,
            _ => u32::MAX,
        },
        AACENC_BITRATEMODE => {
            (if c.bitrate_mode != AACENC_BR_MODE_FF { c.bitrate_mode } else { AACENC_BR_MODE_CBR })
                as u32
        }
        AACENC_SAMPLERATE => h.coder_config.ext_sampling_rate as u32,
        AACENC_CHANNELMODE => {
            if (MODE_1 == c.channel_mode) && (c.syntax_flags & AC_LD_MPS != 0) {
                MODE_212 as u32
            } else {
                c.channel_mode as u32
            }
        }
        AACENC_BANDWIDTH => c.band_width as u32,
        AACENC_CHANNELORDER => c.channel_order as u32,
        AACENC_AFTERBURNER => c.use_requant as u32,
        AACENC_GRANULE_LENGTH => c.framelength as u32,
        AACENC_SBR_RATIO => {
            if is_sbr_active(c) != 0 {
                c.sbr_ratio
            } else {
                0
            }
        }
        AACENC_SBR_MODE => (c.syntax_flags & AC_SBR_PRESENT != 0) as u32,
        AACENC_TRANSMUX => settings.user_tp_type as u32,
        AACENC_SIGNALING_MODE => get_sbr_signaling_mode(
            c.audio_object_type,
            settings.user_tp_type,
            settings.user_tp_signaling,
            c.sbr_ratio,
        ) as u32,
        AACENC_PROTECTION => settings.user_tp_protection as u32,
        AACENC_HEADER_PERIOD => h.coder_config.header_period as u32,
        AACENC_AUDIOMUXVER => c.audio_mux_version as u32,
        AACENC_TPSUBFRAMES => settings.user_tp_nsub_frames as u32,
        AACENC_ANCILLARY_BITRATE => c.anc_rate as u32,
        AACENC_CONTROL_STATE => h.init_flags,
        AACENC_METADATA_MODE => {
            if h.meta_data_allowed == 0 {
                0
            } else {
                settings.user_meta_data_mode as u32
            }
        }
        AACENC_PEAK_BITRATE => {
            // peak bitrate parameter is meaningless
            let mut value = u32::MAX;
            if h.ext_param.user_peak_bitrate as i32 != -1 {
                // peak bitrate parameter is in use
                value = f_max(h.ext_param.user_peak_bitrate as i32, c.bit_rate) as u32;
            }
            value
        }
        _ => 0,
    }
}

/// `aacEncInfo(hAacEncoder, pInfo)`
pub fn aac_enc_info(
    h_aac_encoder: Option<&mut AacEncoder>,
    p_info: Option<&mut AacencInfoStruct>,
) -> AacencError {
    let (Some(h), Some(p_info)) = (h_aac_encoder, p_info) else {
        return AACENC_INVALID_HANDLE;
    };
    // (output delay, not in C: the core of the pending frame comes first)
    pipeline::drain(h);

    *p_info = AacencInfoStruct::default();
    p_info.conf_size = 64; // pre-initialize

    p_info.max_out_buf_bytes = ((h.n_max_aac_channels.wrapping_mul(6144) + 7) >> 3) as u32;
    p_info.max_anc_bytes = h.aac_config.max_anc_bytes_per_au;
    let n_ch = h.ext_param.n_channels as i32;
    p_info.in_buf_fill_level = sdiv_i(h.n_samples_read, n_ch) as u32;
    p_info.input_channels = h.ext_param.n_channels;
    p_info.frame_length = sdiv_i(h.n_samples_to_read, n_ch) as u32;
    p_info.n_delay = h.n_delay as u32;
    p_info.n_delay_core = h.n_delay_core as u32;

    // Get encoder configuration
    let mut size = p_info.conf_size;
    let r = aac_enc_get_conf(h, &mut size, &mut p_info.conf_buf);
    p_info.conf_size = size;
    if r != AAC_ENC_OK {
        return AACENC_INIT_ERROR;
    }
    AACENC_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The optimized `deinterleave_input` against the straight port: both element sizes,
    /// unaligned input, 1 to 8 channels, strides shorter than the frame (overlapping channel
    /// regions) and writes past the buffer.
    #[test]
    fn deinterleave_input_vs_straight() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 32) as u32
        };
        for _ in 0..20_000 {
            let channels = 1 + next() % 8;
            let frames = next() % 70;
            let stride = match next() % 4 {
                0 => next() % 80,
                _ => frames + next() % 8,
            };
            let base = (next() % 50) as usize;
            let len = (next() % 700) as usize;
            let el_size = [1, 2, 4, 3][(next() % 4) as usize];
            let n = (channels * frames) as usize;
            // input bytes with a random misalignment
            let off = (next() % 4) as usize;
            let mut bytes = vec![0u8; off + n * 4 + 4];
            for b in bytes.iter_mut() {
                *b = next() as u8;
            }
            let p = bytes[off..].as_ptr() as *const c_void;
            let fill: Vec<i16> = (0..len).map(|_| next() as i16).collect();
            let (mut a, mut b) = (fill.clone(), fill);
            // SAFETY: `p` is valid for reads of n elements of 4 bytes.
            unsafe {
                deinterleave_input_straight(p, el_size, &mut a, base, channels, frames, stride);
                deinterleave_input(p, el_size, &mut b, base, channels, frames, stride);
            }
            assert_eq!(a, b, "channels {channels} frames {frames} stride {stride} base {base}");
        }
    }
}
