// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/include/aacdecoder_lib.h (the API, as a safe Rust layer).

//! Safe decoder API (PORTING.md, "Safe API").
//!
//! [`Decoder`] is a thin layer over the straight port of `aacdecoder_lib.h`
//! ([`fdk_aac_rs_aacdec::aacdecoder_lib`]). Every method makes exactly the straight-port calls its
//! documentation names, with the caller's slices as the buffers (no copies), and converts the
//! arguments and results only:
//!
//! | Method | Straight-port calls |
//! |---|---|
//! | [`Decoder::new`], [`Decoder::open`] | `aacDecoder_Open()` |
//! | [`Decoder::with_config`] | `aacDecoder_Open(TT_MP4_RAW, 1)`, `aacDecoder_ConfigRaw()` |
//! | [`Decoder::config_raw`] | `aacDecoder_ConfigRaw()` |
//! | [`Decoder::set`], [`Decoder::set_raw`] | `aacDecoder_SetParam()` |
//! | [`Decoder::fill`] | `aacDecoder_Fill()` |
//! | [`Decoder::free_bytes`] | `aacDecoder_GetFreeBytes()` |
//! | [`Decoder::decode_frame`] | `aacDecoder_DecodeFrame()` with `timeDataSize` = the length of the output slice |
//! | [`Decoder::stream_info`] | `aacDecoder_GetStreamInfo()` |
//! | [`Decoder::set_ancillary_buffer`], [`Decoder::clear_ancillary_buffer`] | `aacDecoder_AncDataInit()` |
//! | [`Decoder::ancillary_data`], [`Decoder::ancillary_elements`] | `aacDecoder_AncDataGet()` |
//! | [`Decoder::raw_isobmff_data`] | `aacDecoder_RawISOBMFFData()` |
//! | `drop` | `aacDecoder_Close()` |
//! | [`lib_info`] | `aacDecoder_GetLibInfo()` |
//!
//! Return codes other than `AAC_DEC_OK` become a [`DecoderError`], which keeps the C code
//! ([`DecoderError::code`]), with one exception: `aacDecoder_DecodeFrame()` returns valid
//! (concealed) output with the decode errors (`IS_DECODE_ERROR()`, 0x4001..0x4FFF), so
//! [`Decoder::decode_frame`] returns them as [`DecodedFrame::error`] together with the samples.
//!
//! As in C, `aacDecoder_DecodeFrame()` clears the whole output slice when it returns no valid
//! output, and never writes past the returned samples when it does (PORTING.md, "Definition of
//! bit-exact").
//!
//! [`Decoder`] is `Send` but not `Sync`; separate decoders are independent.
//!
//! # Threads
//!
//! A decoder uses one thread by default (the calling thread). [`Decoder::set_threads`] with more
//! than one thread lets it decode the end of each frame (the stereo tools, the inverse
//! transform, SBR, PS, MPEG Surround, DRC, the limiter) on a worker thread while the next access
//! unit is parsed (frame pipelining, `docs/optimization.md`); with more than two threads, the
//! channels of a frame are also decoded in parallel. The output is identical, but each
//! frame comes one call later: [`Decoder::decode_frame`] returns the frame of the previous call
//! that decoded one (the first such call returns [`DecoderError::NotEnoughBits`]), together with
//! its stream information ([`Decoder::stream_info`]) and ancillary data. A call that finds not
//! enough input directly after a [`DecoderError::NotEnoughBits`], without [`Decoder::fill`] in
//! between, returns the frame still pending instead; so after the input, calling
//! [`Decoder::decode_frame`] until no frame is pending ([`Decoder::pending_frames`]) returns the
//! frames still in the input buffer and the last one ([`Decoder::drain`] returns the pending
//! frame without decoding input):
//!
#![cfg_attr(feature = "encoder", doc = "```")]
#![cfg_attr(not(feature = "encoder"), doc = "```ignore")]
//! use fdk_aac_rs::decoder::{DecodeFlags, Decoder, DecoderError, Transport};
//! # use fdk_aac_rs::encoder::{ChannelMode, Encoder};
//! # let mut enc = Encoder::builder().sample_rate(44_100).channel_mode(ChannelMode::Stereo)
//! #     .transport(fdk_aac_rs::encoder::Transport::Adts).build()?;
//! # let mut adts = Vec::new();
//! # let mut out = vec![0u8; 8192];
//! # for _ in 0..8 {
//! #     let r = enc.encode(&vec![0i16; 2 * 1024], &mut out)?;
//! #     adts.extend_from_slice(&out[..r.out_bytes()]);
//! # }
//! let mut dec = Decoder::new(Transport::Adts)?;
//! dec.set_threads(2)?;
//! let mut pcm = vec![0i16; 8 * 2048];
//! let mut input = &adts[..];
//! let mut frames = 0;
//! while !input.is_empty() {
//!     let consumed = dec.fill(input)?;
//!     input = &input[consumed..];
//!     loop {
//!         match dec.decode_frame(&mut pcm, DecodeFlags::empty()) {
//!             Ok(_frame) => frames += 1,
//!             Err(DecoderError::NotEnoughBits) => break,
//!             Err(e) => return Err(e.into()),
//!         }
//!     }
//! }
//! // After the input (with one thread, the first call returns NotEnoughBits).
//! loop {
//!     match dec.decode_frame(&mut pcm, DecodeFlags::empty()) {
//!         Ok(_frame) => frames += 1,
//!         Err(DecoderError::NotEnoughBits) if dec.pending_frames() == 0 => break,
//!         Err(DecoderError::NotEnoughBits) => {}
//!         Err(e) => return Err(e.into()),
//!     }
//! }
//! assert_eq!(frames, 8);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! A caller that fills its input whenever a call returns [`DecoderError::NotEnoughBits`] fills at
//! other calls than with one thread. On a damaged stream, the bits that the transport decoder
//! skips to find the next access unit can then be counted in another call, so the bit rate of
//! the stream information (`bit_rate`, the bits of the call) can differ for such frames; the
//! samples and the return values do not.
//!
//! ```compile_fail
//! fn sync<T: Sync>() {}
//! sync::<fdk_aac_rs::decoder::Decoder>();
//! ```
//!
//! # Example
//!
#![cfg_attr(feature = "encoder", doc = "```")]
#![cfg_attr(not(feature = "encoder"), doc = "```ignore")]
//! use fdk_aac_rs::decoder::{DecodeFlags, Decoder, DecoderError, DecoderParam, LimiterMode, Transport};
//! # use fdk_aac_rs::encoder::{ChannelMode, Encoder};
//! # let mut enc = Encoder::builder().sample_rate(44_100).channel_mode(ChannelMode::Stereo)
//! #     .transport(fdk_aac_rs::encoder::Transport::Adts).build()?;
//! # let mut adts = Vec::new();
//! # let mut out = vec![0u8; 8192];
//! # for _ in 0..8 {
//! #     let r = enc.encode(&vec![0i16; 2 * 1024], &mut out)?;
//! #     adts.extend_from_slice(&out[..r.out_bytes()]);
//! # }
//! // `adts`: an AAC-LC stereo ADTS stream.
//! let mut dec = Decoder::new(Transport::Adts)?;
//! dec.set(DecoderParam::Limiter(LimiterMode::Off))?;
//! let mut pcm = vec![0i16; 8 * 2048];
//! let mut input = &adts[..];
//! let mut frames = 0;
//! while !input.is_empty() {
//!     let consumed = dec.fill(input)?;
//!     input = &input[consumed..];
//!     loop {
//!         match dec.decode_frame(&mut pcm, DecodeFlags::empty()) {
//!             Ok(frame) => {
//!                 assert_eq!(frame.samples().len(), 2 * 1024);
//!                 frames += 1;
//!             }
//!             Err(DecoderError::NotEnoughBits) => break,
//!             Err(e) => return Err(e.into()),
//!         }
//!     }
//! }
//! let info = dec.stream_info();
//! assert_eq!((info.sample_rate(), info.num_channels(), frames), (44_100, 2, 8));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod params;
#[cfg(all(test, feature = "encoder"))]
mod tests;

pub use params::*;

use core::cell::Cell;
use core::fmt;
use core::marker::PhantomData;
use fdk_aac_rs_aacdec::aacdecoder::AacDecoderInstance;
use fdk_aac_rs_aacdec::aacdecoder_lib::{self as lib, CStreamInfo};

/// `LIB_INFO`: one entry of [`lib_info`].
pub use fdk_aac_rs_fdk::audio::LibInfo;

/// An `AAC_DECODER_ERROR` other than `AAC_DEC_OK`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecoderError {
    /// `AAC_DEC_OUT_OF_MEMORY` (0x0002): heap returned NULL pointer.
    OutOfMemory,
    /// `AAC_DEC_UNKNOWN` (0x0005): error of unknown reason, or from another module.
    Unknown,
    /// `AAC_DEC_TRANSPORT_SYNC_ERROR` (0x1001): the transport decoder had synchronization
    /// problems.
    TransportSyncError,
    /// `AAC_DEC_NOT_ENOUGH_BITS` (0x1002): the input buffer ran out of bits.
    NotEnoughBits,
    /// `AAC_DEC_INVALID_HANDLE` (0x2001): the handle was invalid (NULL). Also the error of a
    /// failed `aacDecoder_Open()`, which returns NULL without a code.
    InvalidHandle,
    /// `AAC_DEC_UNSUPPORTED_AOT` (0x2002): the AOT of the configuration is not supported.
    UnsupportedAot,
    /// `AAC_DEC_UNSUPPORTED_FORMAT` (0x2003): the bitstream format is not supported.
    UnsupportedFormat,
    /// `AAC_DEC_UNSUPPORTED_ER_FORMAT` (0x2004): the error resilience tool format is not
    /// supported.
    UnsupportedErFormat,
    /// `AAC_DEC_UNSUPPORTED_EPCONFIG` (0x2005): the error protection format is not supported.
    UnsupportedEpConfig,
    /// `AAC_DEC_UNSUPPORTED_MULTILAYER` (0x2006): more than one layer is not supported.
    UnsupportedMultilayer,
    /// `AAC_DEC_UNSUPPORTED_CHANNELCONFIG` (0x2007): the channel configuration is not
    /// supported.
    UnsupportedChannelConfig,
    /// `AAC_DEC_UNSUPPORTED_SAMPLINGRATE` (0x2008): the sample rate is not supported.
    UnsupportedSamplingRate,
    /// `AAC_DEC_INVALID_SBR_CONFIG` (0x2009): the SBR configuration is not supported.
    InvalidSbrConfig,
    /// `AAC_DEC_SET_PARAM_FAIL` (0x200A): the parameter could not be set.
    SetParamFail,
    /// `AAC_DEC_NEED_TO_RESTART` (0x200B): the decoder needs to be restarted.
    NeedToRestart,
    /// `AAC_DEC_OUTPUT_BUFFER_TOO_SMALL` (0x200C): the output buffer is too small.
    OutputBufferTooSmall,
    /// `AAC_DEC_TRANSPORT_ERROR` (0x4001): the transport decoder encountered an unexpected
    /// error.
    TransportError,
    /// `AAC_DEC_PARSE_ERROR` (0x4002): error while parsing the bitstream.
    ParseError,
    /// `AAC_DEC_UNSUPPORTED_EXTENSION_PAYLOAD` (0x4003): unsupported extension payload.
    UnsupportedExtensionPayload,
    /// `AAC_DEC_DECODE_FRAME_ERROR` (0x4004): a parsed bitstream value is out of range.
    DecodeFrameError,
    /// `AAC_DEC_CRC_ERROR` (0x4005): the embedded CRC did not match.
    CrcError,
    /// `AAC_DEC_INVALID_CODE_BOOK` (0x4006): an invalid codebook was signaled.
    InvalidCodeBook,
    /// `AAC_DEC_UNSUPPORTED_PREDICTION` (0x4007): prediction is not supported.
    UnsupportedPrediction,
    /// `AAC_DEC_UNSUPPORTED_CCE` (0x4008): a CCE element is not supported.
    UnsupportedCce,
    /// `AAC_DEC_UNSUPPORTED_LFE` (0x4009): a LFE element is not supported.
    UnsupportedLfe,
    /// `AAC_DEC_UNSUPPORTED_GAIN_CONTROL_DATA` (0x400A): gain control data is not supported.
    UnsupportedGainControlData,
    /// `AAC_DEC_UNSUPPORTED_SBA` (0x400B): SBA is not supported.
    UnsupportedSba,
    /// `AAC_DEC_TNS_READ_ERROR` (0x400C): error while reading TNS data.
    TnsReadError,
    /// `AAC_DEC_RVLC_ERROR` (0x400D): error while decoding error resilient data.
    RvlcError,
    /// `AAC_DEC_ANC_DATA_ERROR` (0x8001): non severe error concerning the ancillary data.
    AncDataError,
    /// `AAC_DEC_TOO_SMALL_ANC_BUFFER` (0x8002): the ancillary data buffer is too small.
    TooSmallAncBuffer,
    /// `AAC_DEC_TOO_MANY_ANC_ELEMENTS` (0x8003): too many ancillary data elements.
    TooManyAncElements,
    /// A code the library does not define (not returned by the decoder).
    Other(i32),
}

/// An alias of [`DecoderError`].
pub type DecodeError = DecoderError;

impl DecoderError {
    /// The error of a C return code; `None` for `AAC_DEC_OK`.
    pub const fn from_code(code: i32) -> Option<DecoderError> {
        use DecoderError as E;
        Some(match code {
            lib::AAC_DEC_OK => return None,
            lib::AAC_DEC_OUT_OF_MEMORY => E::OutOfMemory,
            lib::AAC_DEC_UNKNOWN => E::Unknown,
            lib::AAC_DEC_TRANSPORT_SYNC_ERROR => E::TransportSyncError,
            lib::AAC_DEC_NOT_ENOUGH_BITS => E::NotEnoughBits,
            lib::AAC_DEC_INVALID_HANDLE => E::InvalidHandle,
            lib::AAC_DEC_UNSUPPORTED_AOT => E::UnsupportedAot,
            lib::AAC_DEC_UNSUPPORTED_FORMAT => E::UnsupportedFormat,
            lib::AAC_DEC_UNSUPPORTED_ER_FORMAT => E::UnsupportedErFormat,
            lib::AAC_DEC_UNSUPPORTED_EPCONFIG => E::UnsupportedEpConfig,
            lib::AAC_DEC_UNSUPPORTED_MULTILAYER => E::UnsupportedMultilayer,
            lib::AAC_DEC_UNSUPPORTED_CHANNELCONFIG => E::UnsupportedChannelConfig,
            lib::AAC_DEC_UNSUPPORTED_SAMPLINGRATE => E::UnsupportedSamplingRate,
            lib::AAC_DEC_INVALID_SBR_CONFIG => E::InvalidSbrConfig,
            lib::AAC_DEC_SET_PARAM_FAIL => E::SetParamFail,
            lib::AAC_DEC_NEED_TO_RESTART => E::NeedToRestart,
            lib::AAC_DEC_OUTPUT_BUFFER_TOO_SMALL => E::OutputBufferTooSmall,
            lib::AAC_DEC_TRANSPORT_ERROR => E::TransportError,
            lib::AAC_DEC_PARSE_ERROR => E::ParseError,
            lib::AAC_DEC_UNSUPPORTED_EXTENSION_PAYLOAD => E::UnsupportedExtensionPayload,
            lib::AAC_DEC_DECODE_FRAME_ERROR => E::DecodeFrameError,
            lib::AAC_DEC_CRC_ERROR => E::CrcError,
            lib::AAC_DEC_INVALID_CODE_BOOK => E::InvalidCodeBook,
            lib::AAC_DEC_UNSUPPORTED_PREDICTION => E::UnsupportedPrediction,
            lib::AAC_DEC_UNSUPPORTED_CCE => E::UnsupportedCce,
            lib::AAC_DEC_UNSUPPORTED_LFE => E::UnsupportedLfe,
            lib::AAC_DEC_UNSUPPORTED_GAIN_CONTROL_DATA => E::UnsupportedGainControlData,
            lib::AAC_DEC_UNSUPPORTED_SBA => E::UnsupportedSba,
            lib::AAC_DEC_TNS_READ_ERROR => E::TnsReadError,
            lib::AAC_DEC_RVLC_ERROR => E::RvlcError,
            lib::AAC_DEC_ANC_DATA_ERROR => E::AncDataError,
            lib::AAC_DEC_TOO_SMALL_ANC_BUFFER => E::TooSmallAncBuffer,
            lib::AAC_DEC_TOO_MANY_ANC_ELEMENTS => E::TooManyAncElements,
            c => E::Other(c),
        })
    }

    /// The C code (`AAC_DECODER_ERROR`).
    pub const fn code(self) -> i32 {
        use DecoderError as E;
        match self {
            E::OutOfMemory => lib::AAC_DEC_OUT_OF_MEMORY,
            E::Unknown => lib::AAC_DEC_UNKNOWN,
            E::TransportSyncError => lib::AAC_DEC_TRANSPORT_SYNC_ERROR,
            E::NotEnoughBits => lib::AAC_DEC_NOT_ENOUGH_BITS,
            E::InvalidHandle => lib::AAC_DEC_INVALID_HANDLE,
            E::UnsupportedAot => lib::AAC_DEC_UNSUPPORTED_AOT,
            E::UnsupportedFormat => lib::AAC_DEC_UNSUPPORTED_FORMAT,
            E::UnsupportedErFormat => lib::AAC_DEC_UNSUPPORTED_ER_FORMAT,
            E::UnsupportedEpConfig => lib::AAC_DEC_UNSUPPORTED_EPCONFIG,
            E::UnsupportedMultilayer => lib::AAC_DEC_UNSUPPORTED_MULTILAYER,
            E::UnsupportedChannelConfig => lib::AAC_DEC_UNSUPPORTED_CHANNELCONFIG,
            E::UnsupportedSamplingRate => lib::AAC_DEC_UNSUPPORTED_SAMPLINGRATE,
            E::InvalidSbrConfig => lib::AAC_DEC_INVALID_SBR_CONFIG,
            E::SetParamFail => lib::AAC_DEC_SET_PARAM_FAIL,
            E::NeedToRestart => lib::AAC_DEC_NEED_TO_RESTART,
            E::OutputBufferTooSmall => lib::AAC_DEC_OUTPUT_BUFFER_TOO_SMALL,
            E::TransportError => lib::AAC_DEC_TRANSPORT_ERROR,
            E::ParseError => lib::AAC_DEC_PARSE_ERROR,
            E::UnsupportedExtensionPayload => lib::AAC_DEC_UNSUPPORTED_EXTENSION_PAYLOAD,
            E::DecodeFrameError => lib::AAC_DEC_DECODE_FRAME_ERROR,
            E::CrcError => lib::AAC_DEC_CRC_ERROR,
            E::InvalidCodeBook => lib::AAC_DEC_INVALID_CODE_BOOK,
            E::UnsupportedPrediction => lib::AAC_DEC_UNSUPPORTED_PREDICTION,
            E::UnsupportedCce => lib::AAC_DEC_UNSUPPORTED_CCE,
            E::UnsupportedLfe => lib::AAC_DEC_UNSUPPORTED_LFE,
            E::UnsupportedGainControlData => lib::AAC_DEC_UNSUPPORTED_GAIN_CONTROL_DATA,
            E::UnsupportedSba => lib::AAC_DEC_UNSUPPORTED_SBA,
            E::TnsReadError => lib::AAC_DEC_TNS_READ_ERROR,
            E::RvlcError => lib::AAC_DEC_RVLC_ERROR,
            E::AncDataError => lib::AAC_DEC_ANC_DATA_ERROR,
            E::TooSmallAncBuffer => lib::AAC_DEC_TOO_SMALL_ANC_BUFFER,
            E::TooManyAncElements => lib::AAC_DEC_TOO_MANY_ANC_ELEMENTS,
            E::Other(c) => c,
        }
    }

    /// `IS_INIT_ERROR()`: an initialization error (0x2000..0x2FFF); the output is invalid.
    pub const fn is_init_error(self) -> bool {
        lib::is_init_error(self.code()) != 0
    }

    /// `IS_DECODE_ERROR()`: a decode error (0x4000..0x4FFF); the output is valid but
    /// concealed.
    pub const fn is_decode_error(self) -> bool {
        lib::is_decode_error(self.code()) != 0
    }

    /// `IS_OUTPUT_VALID()`: whether `aacDecoder_DecodeFrame()` returned valid samples with this
    /// code (only the decode errors).
    pub const fn is_output_valid(self) -> bool {
        lib::is_output_valid(self.code()) != 0
    }

    /// A synchronization error (0x1000..0x1FFF); the output is invalid.
    pub const fn is_sync_error(self) -> bool {
        let c = self.code();
        c >= lib::aac_dec_sync_error_start && c <= lib::aac_dec_sync_error_end
    }

    /// `Ok(())` for `AAC_DEC_OK`, the error otherwise.
    #[inline]
    fn check(code: i32) -> Result<(), DecoderError> {
        match DecoderError::from_code(code) {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }
}

impl fmt::Display for DecoderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use DecoderError as E;
        let s = match self {
            E::OutOfMemory => "out of memory",
            E::Unknown => "unknown error",
            E::TransportSyncError => "transport synchronization error",
            E::NotEnoughBits => "not enough bits",
            E::InvalidHandle => "invalid handle",
            E::UnsupportedAot => "unsupported audio object type",
            E::UnsupportedFormat => "unsupported bitstream format",
            E::UnsupportedErFormat => "unsupported error resilience format",
            E::UnsupportedEpConfig => "unsupported error protection configuration",
            E::UnsupportedMultilayer => "unsupported multilayer configuration",
            E::UnsupportedChannelConfig => "unsupported channel configuration",
            E::UnsupportedSamplingRate => "unsupported sample rate",
            E::InvalidSbrConfig => "invalid SBR configuration",
            E::SetParamFail => "parameter could not be set",
            E::NeedToRestart => "decoder needs to be restarted",
            E::OutputBufferTooSmall => "output buffer too small",
            E::TransportError => "transport error",
            E::ParseError => "parse error",
            E::UnsupportedExtensionPayload => "unsupported extension payload",
            E::DecodeFrameError => "decode frame error",
            E::CrcError => "CRC error",
            E::InvalidCodeBook => "invalid codebook",
            E::UnsupportedPrediction => "unsupported prediction",
            E::UnsupportedCce => "unsupported CCE",
            E::UnsupportedLfe => "unsupported LFE",
            E::UnsupportedGainControlData => "unsupported gain control data",
            E::UnsupportedSba => "unsupported SBA",
            E::TnsReadError => "TNS read error",
            E::RvlcError => "RVLC error",
            E::AncDataError => "ancillary data error",
            E::TooSmallAncBuffer => "ancillary data buffer too small",
            E::TooManyAncElements => "too many ancillary data elements",
            E::Other(_) => "unknown error code",
        };
        write!(f, "{s} (AAC_DECODER_ERROR 0x{:04x})", self.code())
    }
}

impl std::error::Error for DecoderError {}

/// The `flags` of `aacDecoder_DecodeFrame()` (`AACDEC_CONCEAL`, `AACDEC_FLUSH`, `AACDEC_INTR`,
/// `AACDEC_CLRHIST`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DecodeFlags(u32);

impl DecodeFlags {
    /// `AACDEC_CONCEAL`: do concealment (no input is consumed).
    pub const CONCEAL: DecodeFlags = DecodeFlags(lib::AACDEC_CONCEAL);
    /// `AACDEC_FLUSH`: discard the input data and flush the filter banks (output the delayed
    /// audio).
    pub const FLUSH: DecodeFlags = DecodeFlags(lib::AACDEC_FLUSH);
    /// `AACDEC_INTR`: input data is discontinuous; resynchronize.
    pub const INTR: DecodeFlags = DecodeFlags(lib::AACDEC_INTR);
    /// `AACDEC_CLRHIST`: clear all signal delay lines and history buffers.
    pub const CLRHIST: DecodeFlags = DecodeFlags(lib::AACDEC_CLRHIST);

    /// No flags (0).
    pub const fn empty() -> DecodeFlags {
        DecodeFlags(0)
    }

    /// The flags of a C value (any bits are kept).
    pub const fn from_bits(bits: u32) -> DecodeFlags {
        DecodeFlags(bits)
    }

    /// The C value.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether all flags of `other` are set.
    pub const fn contains(self, other: DecodeFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for DecodeFlags {
    type Output = DecodeFlags;
    fn bitor(self, rhs: DecodeFlags) -> DecodeFlags {
        DecodeFlags(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for DecodeFlags {
    fn bitor_assign(&mut self, rhs: DecodeFlags) {
        self.0 |= rhs.0;
    }
}

/// `CStreamInfo`, returned by [`Decoder::stream_info`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamInfo(CStreamInfo);

impl StreamInfo {
    /// `sampleRate`: the sample rate in Hz of the decoded PCM signal.
    pub const fn sample_rate(&self) -> i32 {
        self.0.sample_rate
    }

    /// `frameSize`: the number of samples per channel of a decoded frame.
    pub const fn frame_size(&self) -> i32 {
        self.0.frame_size
    }

    /// `numChannels`: the number of output channels.
    pub const fn num_channels(&self) -> i32 {
        self.0.num_channels
    }

    /// `pChannelType[0..numChannels]`: the `AUDIO_CHANNEL_TYPE` of each output channel.
    pub fn channel_types(&self) -> &[i32] {
        let n = (self.0.num_channels.max(0) as usize).min(self.0.p_channel_type.len());
        &self.0.p_channel_type[..n]
    }

    /// `pChannelIndices[0..numChannels]`: the index of each output channel within its type.
    pub fn channel_indices(&self) -> &[u8] {
        let n = (self.0.num_channels.max(0) as usize).min(self.0.p_channel_indices.len());
        &self.0.p_channel_indices[..n]
    }

    /// `aacSampleRate`: the sample rate without SBR (divided by an ELD downscale factor).
    pub const fn aac_sample_rate(&self) -> i32 {
        self.0.aac_sample_rate
    }

    /// `profile`: the MPEG-2 profile (-1: not applicable).
    pub const fn profile(&self) -> i32 {
        self.0.profile
    }

    /// `aot`: the `AUDIO_OBJECT_TYPE` of the configuration.
    pub const fn aot(&self) -> i32 {
        self.0.aot
    }

    /// `channelConfig`: the channel configuration (0: PCE defined).
    pub const fn channel_config(&self) -> i32 {
        self.0.channel_config
    }

    /// `bitRate`: the instantaneous bit rate.
    pub const fn bit_rate(&self) -> i32 {
        self.0.bit_rate
    }

    /// `aacSamplesPerFrame`: samples per frame of the AAC core.
    pub const fn aac_samples_per_frame(&self) -> i32 {
        self.0.aac_samples_per_frame
    }

    /// `aacNumChannels`: channels after the AAC core (before PS or MPS).
    pub const fn aac_num_channels(&self) -> i32 {
        self.0.aac_num_channels
    }

    /// `extAot`: the extension `AUDIO_OBJECT_TYPE`.
    pub const fn ext_aot(&self) -> i32 {
        self.0.ext_aot
    }

    /// `extSamplingRate`: the extension sample rate.
    pub const fn ext_sampling_rate(&self) -> i32 {
        self.0.ext_sampling_rate
    }

    /// `outputDelay`: the additional output delay of the decoder in samples.
    pub const fn output_delay(&self) -> u32 {
        self.0.output_delay
    }

    /// `flags`: a copy of the decoder's internal flags.
    pub const fn flags(&self) -> u32 {
        self.0.flags
    }

    /// `epConfig`: the epConfig level (-1: no ER).
    pub const fn ep_config(&self) -> i8 {
        self.0.ep_config
    }

    /// `numLostAccessUnits`: the estimated number of lost access units.
    pub const fn num_lost_access_units(&self) -> i32 {
        self.0.num_lost_access_units
    }

    /// `numTotalBytes`: the number of bytes that passed through the decoder.
    pub const fn num_total_bytes(&self) -> i64 {
        self.0.num_total_bytes
    }

    /// `numBadBytes`: the number of bytes considered erroneous.
    pub const fn num_bad_bytes(&self) -> i64 {
        self.0.num_bad_bytes
    }

    /// `numTotalAccessUnits`: the number of access units that passed through the decoder.
    pub const fn num_total_access_units(&self) -> i64 {
        self.0.num_total_access_units
    }

    /// `numBadAccessUnits`: the number of access units considered erroneous.
    pub const fn num_bad_access_units(&self) -> i64 {
        self.0.num_bad_access_units
    }

    /// `drcProgRefLev`: the DRC program reference level.
    pub const fn drc_prog_ref_lev(&self) -> i8 {
        self.0.drc_prog_ref_lev
    }

    /// `drcPresMode`: the DRC presentation mode.
    pub const fn drc_pres_mode(&self) -> i8 {
        self.0.drc_pres_mode
    }

    /// `outputLoudness`: the output loudness in steps of -0.25 dB.
    pub const fn output_loudness(&self) -> i32 {
        self.0.output_loudness
    }

    /// The `CStreamInfo` as the straight port holds it (with the channel arrays inline).
    pub const fn raw(&self) -> &CStreamInfo {
        &self.0
    }
}

/// The valid output of [`Decoder::decode_frame`].
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub struct DecodedFrame<'a> {
    samples: &'a [i16],
    error: Option<DecoderError>,
}

impl<'a> DecodedFrame<'a> {
    /// The decoded interleaved samples: `frameSize * numChannels` samples of the stream
    /// information after the call (limited to the output slice).
    pub fn samples(&self) -> &'a [i16] {
        self.samples
    }

    /// `None` for `AAC_DEC_OK`; the decode error (`IS_DECODE_ERROR()`) for concealed output.
    pub fn error(&self) -> Option<DecoderError> {
        self.error
    }
}

/// An AAC decoder instance (`HANDLE_AACDECODER`). Dropping it calls `aacDecoder_Close()`.
pub struct Decoder {
    /// The handle; `Some` from a successful `aacDecoder_Open()` until the drop.
    handle: Option<Box<AacDecoderInstance>>,
    /// `Send` but not `Sync`.
    _not_sync: PhantomData<Cell<()>>,
}

impl fmt::Debug for Decoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Decoder").finish_non_exhaustive()
    }
}

/// A slice length as a C `UINT`. Lengths beyond `UINT_MAX` are reported as `UINT_MAX`, so the
/// library never accesses more than the slice.
fn c_ulen(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A slice length as a C `INT` (at most `INT_MAX`).
fn c_len(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

impl Decoder {
    /// `aacDecoder_Open(transport, 1)`.
    ///
    /// For [`Transport::Raw`] the decoder needs the configuration ([`Decoder::config_raw`], or
    /// [`Decoder::with_config`]).
    pub fn new(transport: Transport) -> Result<Decoder, DecoderError> {
        Decoder::open(transport, 1)
    }

    /// `aacDecoder_Open(transport, layers)`. The decoder uses at most one layer (C:
    /// `nrOfLayers_min = fMin(nrOfLayers, 1)`); with 0 layers it consumes no input.
    ///
    /// `aacDecoder_Open()` returns NULL without an error code on failure; this function returns
    /// [`DecoderError::InvalidHandle`] then (not reachable with the transport types of
    /// [`Transport`]).
    pub fn open(transport: Transport, layers: u32) -> Result<Decoder, DecoderError> {
        match lib::aac_decoder_open(transport as i32, layers) {
            Some(h) => Ok(Decoder { handle: Some(h), _not_sync: PhantomData }),
            None => Err(DecoderError::InvalidHandle),
        }
    }

    /// A raw-access-unit decoder configured with an AudioSpecificConfig:
    /// `aacDecoder_Open(TT_MP4_RAW, 1)` and `aacDecoder_ConfigRaw()`.
    ///
    /// ```
    /// use fdk_aac_rs::decoder::Decoder;
    ///
    /// // AAC-LC, 44.1 kHz, stereo.
    /// let dec = Decoder::with_config(&[0x12, 0x10])?;
    /// let info = dec.stream_info();
    /// assert_eq!((info.aot(), info.aac_sample_rate(), info.channel_config()), (2, 44_100, 2));
    /// # Ok::<(), fdk_aac_rs::decoder::DecoderError>(())
    /// ```
    pub fn with_config(asc: &[u8]) -> Result<Decoder, DecoderError> {
        let mut dec = Decoder::new(Transport::Raw)?;
        dec.config_raw(asc)?;
        Ok(dec)
    }

    /// `aacDecoder_ConfigRaw()` with one configuration (an AudioSpecificConfig, or a
    /// StreamMuxConfig for [`Transport::LatmMcp0`]).
    pub fn config_raw(&mut self, config: &[u8]) -> Result<(), DecoderError> {
        let h = self.handle.as_deref_mut().ok_or(DecoderError::InvalidHandle)?;
        DecoderError::check(lib::aac_decoder_config_raw(h, &[config], &[c_ulen(config.len())]))
    }

    /// `aacDecoder_SetParam()`.
    pub fn set(&mut self, param: DecoderParam) -> Result<(), DecoderError> {
        self.set_raw(param.id().code(), param.value())
    }

    /// `aacDecoder_SetParam()` with any parameter ID and value.
    pub fn set_raw(&mut self, id: i32, value: i32) -> Result<(), DecoderError> {
        DecoderError::check(lib::aac_decoder_set_param(self.handle.as_deref_mut(), id, value))
    }

    /// `aacDecoder_Fill()`: copies as much of `input` into the decoder's input buffer as fits
    /// and returns the number of bytes consumed (`bufferSize - bytesValid`).
    pub fn fill(&mut self, input: &[u8]) -> Result<usize, DecoderError> {
        let h = self.handle.as_deref_mut().ok_or(DecoderError::InvalidHandle)?;
        let size = c_ulen(input.len());
        let mut valid = [size];
        let err = lib::aac_decoder_fill(h, &[input], &[size], &mut valid);
        DecoderError::check(err)?;
        Ok(size.saturating_sub(valid[0]) as usize)
    }

    /// `aacDecoder_GetFreeBytes()`: the number of free bytes in the input buffer.
    pub fn free_bytes(&mut self) -> Result<u32, DecoderError> {
        let mut free = 0;
        DecoderError::check(lib::aac_decoder_get_free_bytes(
            self.handle.as_deref_mut(),
            &mut free,
        ))?;
        Ok(free)
    }

    /// `aacDecoder_DecodeFrame()` into `pcm` (interleaved samples; `timeDataSize` is
    /// `pcm.len()`).
    ///
    /// With valid output (`AAC_DEC_OK` or a decode error) it returns the samples; otherwise the
    /// error, and `pcm` is then all zeros, as in C. [`DecoderError::NotEnoughBits`] asks for more
    /// input ([`Decoder::fill`]).
    pub fn decode_frame<'a>(
        &mut self,
        pcm: &'a mut [i16],
        flags: DecodeFlags,
    ) -> Result<DecodedFrame<'a>, DecoderError> {
        let size = c_len(pcm.len());
        let err = lib::aac_decoder_decode_frame(self.handle.as_deref_mut(), pcm, size, flags.0);
        if lib::is_output_valid(err) == 0 {
            if let Some(h) = self.handle.as_deref_mut() {
                // (with more than one thread: the stream information after the call is complete
                // when the frame decoded meanwhile is)
                lib::aac_decoder_sync_stream_info(h);
            }
            // Not AAC_DEC_OK here.
            return Err(DecoderError::from_code(err).unwrap_or(DecoderError::Unknown));
        }
        let info = self.stream_info();
        let n = info.frame_size() as i64 * info.num_channels() as i64;
        let n = n.clamp(0, pcm.len() as i64) as usize;
        let pcm: &'a [i16] = pcm;
        Ok(DecodedFrame { samples: &pcm[..n], error: DecoderError::from_code(err) })
    }

    /// Sets the number of threads of the decoder (default 1, see the [module
    /// documentation](self#threads)): more than one thread pipelines the decoding of
    /// consecutive frames, and every frame is returned one call later. One worker thread runs
    /// the end of each frame; with more than two threads, the channels of a frame are also
    /// decoded in parallel. Going back to one thread
    /// requires that no frame is pending ([`Decoder::drain`]); [`DecoderError::SetParamFail`]
    /// otherwise.
    ///
    /// The size check of the output buffer (`AAC_DEC_OUTPUT_BUFFER_TOO_SMALL`) applies to the
    /// call that decodes a frame; the call that returns it one call later copies at most its own
    /// buffer's length. Pass output buffers of the same size to consecutive calls.
    pub fn set_threads(&mut self, threads: usize) -> Result<(), DecoderError> {
        let h = self.handle.as_deref_mut().ok_or(DecoderError::InvalidHandle)?;
        let n = u32::try_from(threads.max(1)).unwrap_or(u32::MAX);
        DecoderError::check(lib::aac_decoder_set_threads(h, n))
    }

    /// The number of threads of the decoder.
    pub fn threads(&self) -> usize {
        self.handle.as_deref().map_or(1, |h| lib::aac_decoder_get_threads(h) as usize)
    }

    /// Sets how a decoder with more than one thread chooses its thread mode (tests; see
    /// `fdk_aac_rs_fdk::par::tune`), until it goes back to one thread.
    #[doc(hidden)]
    pub fn set_tuning(&mut self, tuning: fdk_aac_rs_fdk::par::Tuning) {
        if let Some(h) = self.handle.as_deref_mut() {
            fdk_aac_rs_aacdec::pipeline::set_tuning(h, tuning);
        }
    }

    /// The frames decoded in each thread mode (tests).
    #[doc(hidden)]
    pub fn mode_frames(&self) -> [u64; fdk_aac_rs_fdk::par::tune::MAX_MODES] {
        self.handle.as_deref().map_or([0; fdk_aac_rs_fdk::par::tune::MAX_MODES], |h| {
            fdk_aac_rs_aacdec::pipeline::mode_frames(h)
        })
    }

    /// The number of frames decoded by earlier calls of [`Decoder::decode_frame`] and not
    /// returned yet (0 or 1; always 0 with one thread).
    pub fn pending_frames(&self) -> usize {
        self.handle.as_deref().map_or(0, |h| lib::aac_decoder_pending_frames(h) as usize)
    }

    /// Returns the frame still pending with more than one thread (without decoding input), as
    /// [`Decoder::decode_frame`] would return it; `Ok(None)` if no frame is pending.
    pub fn drain<'a>(
        &mut self,
        pcm: &'a mut [i16],
    ) -> Result<Option<DecodedFrame<'a>>, DecoderError> {
        if self.pending_frames() == 0 {
            return Ok(None);
        }
        let size = c_len(pcm.len());
        let err = lib::aac_decoder_drain(self.handle.as_deref_mut(), pcm, size);
        if lib::is_output_valid(err) == 0 {
            return Err(DecoderError::from_code(err).unwrap_or(DecoderError::Unknown));
        }
        let info = self.stream_info();
        let n = info.frame_size() as i64 * info.num_channels() as i64;
        let n = n.clamp(0, pcm.len() as i64) as usize;
        let pcm: &'a [i16] = pcm;
        Ok(Some(DecodedFrame { samples: &pcm[..n], error: DecoderError::from_code(err) }))
    }

    /// `aacDecoder_GetStreamInfo()`: the information about the stream decoded so far.
    pub fn stream_info(&self) -> StreamInfo {
        lib::aac_decoder_get_stream_info(self.handle.as_deref())
            .map(|s| StreamInfo(*s))
            .unwrap_or_default()
    }

    /// `aacDecoder_AncDataInit()` with a new buffer of `size` bytes, which the decoder keeps:
    /// it writes the ancillary data of each frame there ([`Decoder::ancillary_data`]).
    pub fn set_ancillary_buffer(&mut self, size: usize) -> Result<(), DecoderError> {
        let h = self.handle.as_deref_mut().ok_or(DecoderError::InvalidHandle)?;
        let buf = vec![0u8; size].into_boxed_slice();
        DecoderError::check(lib::aac_decoder_anc_data_init(h, Some(buf), c_len(size)))
    }

    /// `aacDecoder_AncDataInit(self, NULL, 0)`: no ancillary data buffer.
    pub fn clear_ancillary_buffer(&mut self) -> Result<(), DecoderError> {
        let h = self.handle.as_deref_mut().ok_or(DecoderError::InvalidHandle)?;
        DecoderError::check(lib::aac_decoder_anc_data_init(h, None, 0))
    }

    /// `aacDecoder_AncDataGet(index)`: ancillary data element `index` of the last decoded
    /// frame; `None` if there is no such element (or no buffer).
    pub fn ancillary_data(&self, index: usize) -> Option<&[u8]> {
        let h = self.handle.as_deref()?;
        let mut off = None;
        let mut size = 0;
        let index = i32::try_from(index).ok()?;
        lib::aac_decoder_anc_data_get(h, index, &mut off, &mut size);
        let buf = lib::aac_decoder_anc_data_buffer(h)?;
        let off = off?;
        buf.get(off..off.checked_add(size.max(0) as usize)?)
    }

    /// The ancillary data elements of the last decoded frame, in order
    /// (`aacDecoder_AncDataGet()` with the indices 0, 1, ... until there is no element).
    pub fn ancillary_elements(&self) -> impl Iterator<Item = &[u8]> + '_ {
        (0..).map_while(move |i| self.ancillary_data(i))
    }

    /// `aacDecoder_RawISOBMFFData()`: MPEG-D DRC boxes (`ludt`, `dmix`, `udi2`, `udc2`) from an
    /// ISO base media file.
    pub fn raw_isobmff_data(&mut self, data: &[u8]) -> Result<(), DecoderError> {
        let h = self.handle.as_deref_mut().ok_or(DecoderError::InvalidHandle)?;
        DecoderError::check(lib::aac_decoder_raw_isobmff_data(h, data, c_ulen(data.len())))
    }

    /// The decoder instance (for the state digest of the chunked decoding).
    pub(crate) fn instance(&self) -> Option<&AacDecoderInstance> {
        self.handle.as_deref()
    }

    /// The decoder instance (for the counters of the chunked decoding).
    pub(crate) fn instance_mut(&mut self) -> Option<&mut AacDecoderInstance> {
        self.handle.as_deref_mut()
    }
}

impl StreamInfo {
    /// The underlying `CStreamInfo` (for the counters of the chunked decoding).
    pub(crate) fn raw_mut(&mut self) -> &mut CStreamInfo {
        &mut self.0
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        lib::aac_decoder_close(self.handle.take());
    }
}

/// `aacDecoder_GetLibInfo()`: the library information of the decoder and its modules, the
/// entries it filled in (those with a module ID other than `FDK_NONE`), in order. The C function
/// returns -1 (not an `AAC_DECODER_ERROR`) when the array has no free entry, which cannot
/// happen here; that result would be [`DecoderError::Other`]`(-1)`.
///
/// ```
/// let info = fdk_aac_rs::decoder::lib_info()?;
/// let aac = info.iter().find(|e| e.title == "AAC Decoder Lib").unwrap();
/// assert_eq!(&aac.version_str[..6], b"3.2.0\0");
/// # Ok::<(), fdk_aac_rs::decoder::DecoderError>(())
/// ```
pub fn lib_info() -> Result<Vec<LibInfo>, DecoderError> {
    use fdk_aac_rs_fdk::audio::{fdk_init_lib_info, FDK_MODULE_LAST, FDK_NONE};
    let mut info = [LibInfo::default(); FDK_MODULE_LAST as usize];
    fdk_init_lib_info(&mut info);
    let r = lib::aac_decoder_get_lib_info(Some(&mut info));
    if r != 0 {
        return Err(DecoderError::Other(r));
    }
    Ok(info.into_iter().filter(|e| e.module_id != FDK_NONE).collect())
}
