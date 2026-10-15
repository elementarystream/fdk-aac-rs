// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/aacenc.h, libAACenc/src/aacenc.cpp.

//! Port of `aacenc.h` and `aacenc.cpp`: the AAC encoder core interface (open, initialize,
//! encode a frame, close) and its bitrate helpers.
//!
//! # Memory model
//!
//! `AAC_ENC` is [`AacEnc`](crate::aac_enc_ram::AacEnc) (see [`aac_enc_ram`](crate::aac_enc_ram)).
//! In C, `hAacEnc->config` points to the caller's `AACENC_CONFIG` (the one of `aacenc_lib.cpp`),
//! and `FDKaacEnc_Initialize()` writes `ancDataBitRate`, `maxAncBytesPerAU` and `bandWidth`
//! through it; here [`fdk_aac_enc_initialize`] takes the caller's configuration by reference,
//! writes the same members, and stores a copy in `AacEnc::config` once C would have bound the
//! pointer. The caller keeps the copy up to date where C would see later changes through the
//! pointer (`aacenc_lib` does so before every use).

pub use self::frame::*;

/// The functions of `aacenc.cpp`.
mod frame;
/// The file-static tables of `aacenc.cpp`.
mod tables;

/// `CONFIG_TAB_ENTRY_VBR`
#[derive(Clone, Copy)]
struct ConfigTabEntryVbr {
    bitrate_mode: AacencBitrateMode,
    /// mono/stereo settings
    chan_bitrate: [i32; 2],
}

/// `configTabVBR` as (`bitrateMode`, `chanBitrate`) (for the table tests).
#[doc(hidden)]
pub fn config_tab_vbr() -> Vec<(AacencBitrateMode, [i32; 2])> {
    tables::CONFIG_TAB_VBR.iter().map(|e| (e.bitrate_mode, e.chan_bitrate)).collect()
}

use fdk_aac_rs_fdk::audio::{AudioObjectType, ChannelMode, ExtPayloadType};

/// `MIN_BUFSIZE_PER_EFF_CHAN`
pub const MIN_BUFSIZE_PER_EFF_CHAN: i32 = 6144;

/// `AAC_ENCODER_ERROR`: AAC-LC error codes.
pub type AacEncoderError = i32;
/// All fine.
pub const AAC_ENC_OK: AacEncoderError = 0x0000;
/// Error condition is of unknown reason, or from another module.
pub const AAC_ENC_UNKNOWN: AacEncoderError = 0x0002;
/// `aac_enc_init_error_start`
pub const AAC_ENC_INIT_ERROR_START: AacEncoderError = 0x2000;
/// The handle passed to the function call was invalid (probably NULL).
pub const AAC_ENC_INVALID_HANDLE: AacEncoderError = 0x2020;
/// Invalid frame length (must be 1024 or 960).
pub const AAC_ENC_INVALID_FRAME_LENGTH: AacEncoderError = 0x2080;
/// Invalid amount of audio input channels.
pub const AAC_ENC_INVALID_N_CHANNELS: AacEncoderError = 0x20e0;
/// Internal encoder error.
pub const AAC_ENC_INVALID_SFB_TABLE: AacEncoderError = 0x2140;
/// The Audio Object Type (AOT) is not supported.
pub const AAC_ENC_UNSUPPORTED_AOT: AacEncoderError = 0x3000;
/// Filterbank type is not supported.
pub const AAC_ENC_UNSUPPORTED_FILTERBANK: AacEncoderError = 0x3010;
/// The chosen bitrate is not supported.
pub const AAC_ENC_UNSUPPORTED_BITRATE: AacEncoderError = 0x3020;
/// Unsupported bit rate mode (CBR or VBR).
pub const AAC_ENC_UNSUPPORTED_BITRATE_MODE: AacEncoderError = 0x3028;
/// Unsupported ancillay bitrate.
pub const AAC_ENC_UNSUPPORTED_ANC_BITRATE: AacEncoderError = 0x3040;
/// `AAC_ENC_UNSUPPORTED_ANC_MODE`
pub const AAC_ENC_UNSUPPORTED_ANC_MODE: AacEncoderError = 0x3060;
/// The bitstream format is not supported.
pub const AAC_ENC_UNSUPPORTED_TRANSPORT_TYPE: AacEncoderError = 0x3080;
/// The error resilience tool format is not supported.
pub const AAC_ENC_UNSUPPORTED_ER_FORMAT: AacEncoderError = 0x30a0;
/// The error protection format is not supported.
pub const AAC_ENC_UNSUPPORTED_EPCONFIG: AacEncoderError = 0x30c0;
/// The channel configuration (either number or arrangement) is not supported.
pub const AAC_ENC_UNSUPPORTED_CHANNELCONFIG: AacEncoderError = 0x30e0;
/// Sample rate of audio input is not supported.
pub const AAC_ENC_UNSUPPORTED_SAMPLINGRATE: AacEncoderError = 0x3100;
/// Could not allocate memory.
pub const AAC_ENC_NO_MEMORY: AacEncoderError = 0x3120;
/// Internal encoder error.
pub const AAC_ENC_PE_INIT_TABLE_NOT_FOUND: AacEncoderError = 0x3140;
/// `aac_enc_init_error_end`
pub const AAC_ENC_INIT_ERROR_END: AacEncoderError = 0x3141;
/// `aac_enc_error_start`
pub const AAC_ENC_ERROR_START: AacEncoderError = 0x4000;
/// Too many bits used in quantization.
pub const AAC_ENC_QUANT_ERROR: AacEncoderError = 0x4020;
/// Unexpected number of written bits, differs to calculated number of bits.
pub const AAC_ENC_WRITTEN_BITS_ERROR: AacEncoderError = 0x4040;
/// PNS level out of range.
pub const AAC_ENC_PNS_TABLE_ERROR: AacEncoderError = 0x4060;
/// Internal quantizer error.
pub const AAC_ENC_GLOBAL_GAIN_TOO_HIGH: AacEncoderError = 0x4080;
/// Too few bits in bit reservoir.
pub const AAC_ENC_BITRES_TOO_LOW: AacEncoderError = 0x40a0;
/// Too many bits in bit reservoir.
pub const AAC_ENC_BITRES_TOO_HIGH: AacEncoderError = 0x40a1;
/// `AAC_ENC_INVALID_CHANNEL_BITRATE`
pub const AAC_ENC_INVALID_CHANNEL_BITRATE: AacEncoderError = 0x4100;
/// Internal encoder error.
pub const AAC_ENC_INVALID_ELEMENTINFO_TYPE: AacEncoderError = 0x4120;
/// Error writing scalefacData.
pub const AAC_ENC_WRITE_SCAL_ERROR: AacEncoderError = 0x41e0;
/// Error writing sectionData.
pub const AAC_ENC_WRITE_SEC_ERROR: AacEncoderError = 0x4200;
/// Error writing spectralData.
pub const AAC_ENC_WRITE_SPEC_ERROR: AacEncoderError = 0x4220;
/// `aac_enc_error_end`
pub const AAC_ENC_ERROR_END: AacEncoderError = 0x4221;

/// `ANC_DATA_BUFFERSIZE`: ancBuffer size
pub const ANC_DATA_BUFFERSIZE: usize = 1024;

/// `MAX_TOTAL_EXT_PAYLOADS`
pub const MAX_TOTAL_EXT_PAYLOADS: usize = (8 * 1) + (2 + 2);

/// `AACENC_BITRATE_MODE`
pub type AacencBitrateMode = i32;
/// Invalid bitrate mode.
pub const AACENC_BR_MODE_INVALID: AacencBitrateMode = -1;
/// Constant bitrate mode.
pub const AACENC_BR_MODE_CBR: AacencBitrateMode = 0;
/// Variable bitrate mode, very low bitrate.
pub const AACENC_BR_MODE_VBR_1: AacencBitrateMode = 1;
/// Variable bitrate mode, low bitrate.
pub const AACENC_BR_MODE_VBR_2: AacencBitrateMode = 2;
/// Variable bitrate mode, medium bitrate.
pub const AACENC_BR_MODE_VBR_3: AacencBitrateMode = 3;
/// Variable bitrate mode, high bitrate.
pub const AACENC_BR_MODE_VBR_4: AacencBitrateMode = 4;
/// Variable bitrate mode, very high bitrate.
pub const AACENC_BR_MODE_VBR_5: AacencBitrateMode = 5;
/// Fixed frame mode.
pub const AACENC_BR_MODE_FF: AacencBitrateMode = 6;
/// Superframe mode.
pub const AACENC_BR_MODE_SFR: AacencBitrateMode = 7;

/// `AACENC_BR_MODE_IS_VBR(brMode)`
#[inline]
pub const fn aacenc_br_mode_is_vbr(br_mode: AacencBitrateMode) -> bool {
    (br_mode >= 1) && (br_mode <= 5)
}

/// `CHANNEL_ORDER`
pub type ChannelOrder = i32;
/// MPEG channel ordering (e. g. 5.1: C, L, R, SL, SR, LFE)
pub const CH_ORDER_MPEG: ChannelOrder = 0;
/// WAV fileformat channel ordering (e. g. 5.1: L, R, C, LFE, SL, SR)
pub const CH_ORDER_WAV: ChannelOrder = 1;
/// WG4 fileformat channel ordering (e. g. 5.1: L, R, SL, SR, C, LFE)
pub const CH_ORDER_WG4: ChannelOrder = 2;

/// `struct AACENC_CONFIG` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AacencConfig {
    /// encoder sample rate
    pub sample_rate: i32,
    /// encoder bit rate in bits/sec
    pub bit_rate: i32,
    /// additional bits consumed by anc data or sbr have to be consiedered while configuration
    pub anc_data_bit_rate: i32,
    /// number of frames in super frame (not ADTS/LATM subframes !)
    pub n_sub_frames: i32,
    /// Audio Object Type
    pub audio_object_type: AudioObjectType,
    /// encoder bit rate in bits/superframe
    pub average_bits: i32,
    /// encoder bitrate mode (CBR/VBR)
    pub bitrate_mode: AacencBitrateMode,
    /// number of channels to process
    pub n_channels: i32,
    /// input Channel ordering scheme.
    pub channel_order: ChannelOrder,
    /// targeted audio bandwidth in Hz
    pub band_width: i32,
    /// encoder channel mode configuration
    pub channel_mode: ChannelMode,
    /// used frame size
    pub framelength: i32,
    /// bitstreams syntax configuration
    pub syntax_flags: u32,
    /// error protection configuration
    pub ep_config: i8,
    /// ancillary rate, 0 (disabled), -1 (default) else desired rate
    pub anc_rate: i32,
    pub max_anc_bytes_per_au: u32,
    /// minimum number of bits in AU
    pub min_bits_per_frame: i32,
    /// maximum number of bits in AU
    pub max_bits_per_frame: i32,
    /// audio mux version in loas/latm transport format
    pub audio_mux_version: i32,
    /// sbr sampling rate ratio: dual- or single-rate
    pub sbr_ratio: u32,
    /// flag: use temporal noise shaping
    pub use_tns: u8,
    /// flag: use perceptual noise substitution
    pub use_pns: u8,
    /// flag: use intensity coding
    pub use_is: u8,
    /// flag: use ms stereo tool
    pub use_ms: u8,
    /// flag: use afterburner
    pub use_requant: u8,
    pub downscale_factor: u32,
}

/// `AACENC_EXT_PAYLOAD`
///
/// `pData` points to caller memory in C and stays a raw pointer (null for `NULL`); the encoder
/// core only stores and compares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AacencExtPayload {
    /// pointer to extension payload data
    pub p_data: *const u8,
    /// extension payload data size in bits
    pub data_size: u32,
    /// extension payload data type
    pub data_type: ExtPayloadType,
    /// number of the channel element the data is assigned to
    pub associated_ch_element: i32,
}

impl Default for AacencExtPayload {
    fn default() -> Self {
        AacencExtPayload {
            p_data: core::ptr::null(),
            data_size: 0,
            data_type: 0,
            associated_ch_element: 0,
        }
    }
}
