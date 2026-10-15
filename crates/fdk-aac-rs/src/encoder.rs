// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/include/aacenc_lib.h (the API, as a safe Rust layer).

//! Safe encoder API (PORTING.md, "Safe API").
//!
//! [`Encoder`] is a thin layer over the straight port of `aacenc_lib.h`
//! ([`fdk_aac_rs_aacenc::aacenc_lib`]). Every method makes exactly the straight-port calls its
//! documentation names, with the caller's slices as the buffers (no copies), and converts the
//! arguments and results only:
//!
//! | Method | Straight-port calls |
//! |---|---|
//! | [`Encoder::open`] | `aacEncOpen()` |
//! | [`EncoderBuilder::build`] | `aacEncOpen()`, `aacEncoder_SetParam()` for each parameter in the order given, `aacEncEncode(h, NULL, NULL, NULL, NULL)` |
//! | [`Encoder::set`], [`Encoder::set_raw`] | `aacEncoder_SetParam()` |
//! | [`Encoder::get`], [`Encoder::get_raw`] | `aacEncoder_GetParam()` |
//! | [`Encoder::initialize`] | `aacEncEncode(h, NULL, NULL, NULL, NULL)` |
//! | [`Encoder::encode`], [`Encoder::encode_with`] | `aacEncEncode()` with the samples (and ancillary data and metadata) as input buffers and `out` as `OUT_BITSTREAM_DATA` |
//! | [`Encoder::flush`] | `aacEncEncode()` with `numInSamples = -1` |
//! | [`Encoder::info`] | `aacEncInfo()` |
//! | `drop` | `aacEncClose()` |
//! | [`lib_info`] | `aacEncGetLibInfo()` |
//!
//! Return codes other than `AACENC_OK` become an [`EncoderError`], which keeps the C code
//! ([`EncoderError::code`]). The end of a flush is [`EncoderError::EncodeEof`], as in C.
//!
//! [`Encoder`] is `Send` but not `Sync`; separate encoders are independent.
//!
//! # Threads and the output delay
//!
//! With more than one thread ([`EncoderBuilder::threads`], [`Encoder::set_threads`]) the
//! encoder runs parts of each frame in parallel and, for HE-AAC, HE-AAC v2 and ELD with SBR,
//! the SBR encoder of a frame at the same time as the core of the previous one. For this an
//! encoder with SBR *delays its output by one call*: from the first [`Encoder::encode`] or
//! [`Encoder::flush`] made with more than one thread and SBR (see [`Encoder::output_delay`]),
//! every call returns the result of the previous call (its access unit in `out`,
//! [`EncodeOutput::out_bytes`], [`EncodeOutput::anc_bytes`], [`EncodeOutput::bit_res_state`],
//! or its error), while [`EncodeOutput::in_samples`] is that of the call itself. The first such
//! call returns no access unit. The access units, and every other result, are those of the
//! single-threaded encoder, one call later; [`Encoder::flush`] returns
//! [`EncoderError::EncodeEof`] one call later too, so the flush loop below returns every
//! access unit. An error returned for the previous call hides the `in_samples` of the call.
//!
//! ```compile_fail
//! fn sync<T: Sync>() {}
//! sync::<fdk_aac_rs::encoder::Encoder>();
//! ```
//!
//! # Example
//!
//! ```
//! use fdk_aac_rs::encoder::{Aot, Bitrate, ChannelMode, Encoder, EncoderError, Transport};
//!
//! let mut enc = Encoder::builder()
//!     .aot(Aot::AacLc)
//!     .sample_rate(44_100)
//!     .channel_mode(ChannelMode::Stereo)
//!     .bitrate(Bitrate::Cbr(128_000))
//!     .transport(Transport::Adts)
//!     .build()?;
//! let info = enc.info()?;
//! assert_eq!(info.frame_length(), 1024);
//!
//! // One second of interleaved stereo silence.
//! let pcm = vec![0i16; 2 * 44_100];
//! let mut out = vec![0u8; info.max_out_buf_bytes() as usize];
//! let mut adts = Vec::new();
//! let mut input = &pcm[..];
//! while !input.is_empty() {
//!     let r = enc.encode(input, &mut out)?;
//!     adts.extend_from_slice(&out[..r.out_bytes()]);
//!     input = &input[r.in_samples()..];
//! }
//! loop {
//!     match enc.flush(&mut out) {
//!         Ok(r) => adts.extend_from_slice(&out[..r.out_bytes()]),
//!         Err(EncoderError::EncodeEof) => break,
//!         Err(e) => return Err(e.into()),
//!     }
//! }
//! assert_eq!(&adts[..2], &[0xff, 0xf1]); // ADTS sync word, MPEG-4, no CRC
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod params;
#[cfg(test)]
mod tests;

pub use params::*;

use core::cell::Cell;
use core::ffi::c_void;
use core::fmt;
use core::marker::PhantomData;
use fdk_aac_rs_aacenc::aacenc_lib::{
    self as lib, AacEncoder, AacencBufDesc, AacencInArgs, AacencInfoStruct, AacencOutArgs,
};

/// `AACENC_ExtMetaData`: the extended metadata inside [`MetaData`].
pub use fdk_aac_rs_aacenc::aacenc_lib::AacencExtMetaData as ExtMetaData;
/// `AACENC_MetaData`: metadata for `IN_METADATA_SETUP` (see [`EncodeInput::metadata`]).
pub use fdk_aac_rs_aacenc::aacenc_lib::AacencMetaData as MetaData;
/// `LIB_INFO`: one entry of [`lib_info`].
pub use fdk_aac_rs_fdk::audio::LibInfo;

/// An `AACENC_ERROR` other than `AACENC_OK`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EncoderError {
    /// `AACENC_INVALID_HANDLE` (0x20): handle passed to function call was invalid.
    InvalidHandle,
    /// `AACENC_MEMORY_ERROR` (0x21): memory allocation failed.
    MemoryError,
    /// `AACENC_UNSUPPORTED_PARAMETER` (0x22): parameter not available.
    UnsupportedParameter,
    /// `AACENC_INVALID_CONFIG` (0x23): configuration not provided.
    InvalidConfig,
    /// `AACENC_INIT_ERROR` (0x40): general initialization error.
    InitError,
    /// `AACENC_INIT_AAC_ERROR` (0x41): AAC library initialization error.
    InitAacError,
    /// `AACENC_INIT_SBR_ERROR` (0x42): SBR library initialization error.
    InitSbrError,
    /// `AACENC_INIT_TP_ERROR` (0x43): transport library initialization error.
    InitTpError,
    /// `AACENC_INIT_META_ERROR` (0x44): metadata library initialization error.
    InitMetaError,
    /// `AACENC_INIT_MPS_ERROR` (0x45): MPS library initialization error.
    InitMpsError,
    /// `AACENC_ENCODE_ERROR` (0x60): the encoding process was interrupted by an unexpected
    /// error.
    EncodeError,
    /// `AACENC_ENCODE_EOF` (0x80): end of file reached (the flush is complete).
    EncodeEof,
    /// A code the library does not define (not returned by the encoder).
    Other(i32),
}

impl EncoderError {
    /// The error of a C return code; `None` for `AACENC_OK`.
    pub const fn from_code(code: i32) -> Option<EncoderError> {
        Some(match code {
            lib::AACENC_OK => return None,
            lib::AACENC_INVALID_HANDLE => EncoderError::InvalidHandle,
            lib::AACENC_MEMORY_ERROR => EncoderError::MemoryError,
            lib::AACENC_UNSUPPORTED_PARAMETER => EncoderError::UnsupportedParameter,
            lib::AACENC_INVALID_CONFIG => EncoderError::InvalidConfig,
            lib::AACENC_INIT_ERROR => EncoderError::InitError,
            lib::AACENC_INIT_AAC_ERROR => EncoderError::InitAacError,
            lib::AACENC_INIT_SBR_ERROR => EncoderError::InitSbrError,
            lib::AACENC_INIT_TP_ERROR => EncoderError::InitTpError,
            lib::AACENC_INIT_META_ERROR => EncoderError::InitMetaError,
            lib::AACENC_INIT_MPS_ERROR => EncoderError::InitMpsError,
            lib::AACENC_ENCODE_ERROR => EncoderError::EncodeError,
            lib::AACENC_ENCODE_EOF => EncoderError::EncodeEof,
            c => EncoderError::Other(c),
        })
    }

    /// The C code (`AACENC_ERROR`).
    pub const fn code(self) -> i32 {
        match self {
            EncoderError::InvalidHandle => lib::AACENC_INVALID_HANDLE,
            EncoderError::MemoryError => lib::AACENC_MEMORY_ERROR,
            EncoderError::UnsupportedParameter => lib::AACENC_UNSUPPORTED_PARAMETER,
            EncoderError::InvalidConfig => lib::AACENC_INVALID_CONFIG,
            EncoderError::InitError => lib::AACENC_INIT_ERROR,
            EncoderError::InitAacError => lib::AACENC_INIT_AAC_ERROR,
            EncoderError::InitSbrError => lib::AACENC_INIT_SBR_ERROR,
            EncoderError::InitTpError => lib::AACENC_INIT_TP_ERROR,
            EncoderError::InitMetaError => lib::AACENC_INIT_META_ERROR,
            EncoderError::InitMpsError => lib::AACENC_INIT_MPS_ERROR,
            EncoderError::EncodeError => lib::AACENC_ENCODE_ERROR,
            EncoderError::EncodeEof => lib::AACENC_ENCODE_EOF,
            EncoderError::Other(c) => c,
        }
    }

    /// `Ok(())` for `AACENC_OK`, the error otherwise.
    #[inline]
    fn check(code: i32) -> Result<(), EncoderError> {
        match EncoderError::from_code(code) {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }
}

impl fmt::Display for EncoderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            EncoderError::InvalidHandle => "invalid handle",
            EncoderError::MemoryError => "memory allocation failed",
            EncoderError::UnsupportedParameter => "parameter not available",
            EncoderError::InvalidConfig => "configuration not provided",
            EncoderError::InitError => "general initialization error",
            EncoderError::InitAacError => "AAC library initialization error",
            EncoderError::InitSbrError => "SBR library initialization error",
            EncoderError::InitTpError => "transport library initialization error",
            EncoderError::InitMetaError => "metadata library initialization error",
            EncoderError::InitMpsError => "MPS library initialization error",
            EncoderError::EncodeError => "encoding error",
            EncoderError::EncodeEof => "end of file reached",
            EncoderError::Other(_) => "unknown error",
        };
        write!(f, "{s} (AACENC_ERROR 0x{:04x})", self.code())
    }
}

impl std::error::Error for EncoderError {}

/// The failed step of [`EncoderBuilder::build`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuildError {
    /// `aacEncOpen()` failed.
    Open(EncoderError),
    /// `aacEncoder_SetParam(h, id, value)` failed.
    SetParam {
        /// The parameter ID.
        id: u32,
        /// The value.
        value: u32,
        /// The error.
        error: EncoderError,
    },
    /// The initialization (`aacEncEncode(h, NULL, NULL, NULL, NULL)`) failed.
    Initialize(EncoderError),
}

impl BuildError {
    /// The error of the failed call.
    pub const fn error(self) -> EncoderError {
        match self {
            BuildError::Open(e) | BuildError::Initialize(e) => e,
            BuildError::SetParam { error, .. } => error,
        }
    }

    /// The C code of the failed call.
    pub const fn code(self) -> i32 {
        self.error().code()
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::Open(e) => write!(f, "aacEncOpen: {e}"),
            BuildError::SetParam { id, value, error } => {
                write!(f, "aacEncoder_SetParam(0x{id:04x}, {value}): {error}")
            }
            BuildError::Initialize(e) => write!(f, "encoder initialization: {e}"),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<BuildError> for EncoderError {
    fn from(e: BuildError) -> Self {
        e.error()
    }
}

/// Encoder modules to allocate (`encModules` of `aacEncOpen()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Modules(u32);

impl Modules {
    /// 0: every module the library provides (AAC, SBR, PS, MPEG Surround, metadata).
    pub const ALL: Modules = Modules(0);
    /// `0x01`: AAC core encoder.
    pub const AAC: Modules = Modules(lib::ENC_MODE_FLAG_AAC);
    /// `0x02`: SBR.
    pub const SBR: Modules = Modules(lib::ENC_MODE_FLAG_SBR);
    /// `0x04`: parametric stereo.
    pub const PS: Modules = Modules(lib::ENC_MODE_FLAG_PS);
    /// `0x08`: MPEG Surround (ELDv2).
    pub const MPS: Modules = Modules(lib::ENC_MODE_FLAG_SAC);
    /// `0x10`: metadata.
    pub const META: Modules = Modules(lib::ENC_MODE_FLAG_META);

    /// The modules of a C `encModules` value.
    pub const fn from_bits(bits: u32) -> Modules {
        Modules(bits)
    }

    /// The C `encModules` value.
    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl core::ops::BitOr for Modules {
    type Output = Modules;
    fn bitor(self, rhs: Modules) -> Modules {
        Modules(self.0 | rhs.0)
    }
}

/// `AACENC_CTRLFLAGS`: the values of [`Param::ControlState`] and [`ParamId::ControlState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ControlFlags(u32);

impl ControlFlags {
    /// `AACENC_INIT_NONE`: do not trigger initialization.
    pub const INIT_NONE: ControlFlags = ControlFlags(lib::AACENC_INIT_NONE);
    /// `AACENC_INIT_CONFIG`: initialize the configuration of all encoder modules.
    pub const INIT_CONFIG: ControlFlags = ControlFlags(lib::AACENC_INIT_CONFIG);
    /// `AACENC_INIT_STATES`: reset the history buffers of all encoder modules.
    pub const INIT_STATES: ControlFlags = ControlFlags(lib::AACENC_INIT_STATES);
    /// `AACENC_INIT_TRANSPORT`: initialize the transport library with new parameters.
    pub const INIT_TRANSPORT: ControlFlags = ControlFlags(lib::AACENC_INIT_TRANSPORT);
    /// `AACENC_RESET_INBUFFER`: reset the fill level of the internal input buffer.
    pub const RESET_INBUFFER: ControlFlags = ControlFlags(lib::AACENC_RESET_INBUFFER);
    /// `AACENC_INIT_ALL`: initialize all.
    pub const INIT_ALL: ControlFlags = ControlFlags(lib::AACENC_INIT_ALL);

    /// The flags of a C value.
    pub const fn from_bits(bits: u32) -> ControlFlags {
        ControlFlags(bits)
    }

    /// The C value.
    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl core::ops::BitOr for ControlFlags {
    type Output = ControlFlags;
    fn bitor(self, rhs: ControlFlags) -> ControlFlags {
        ControlFlags(self.0 | rhs.0)
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for i16 {}
    impl Sealed for i32 {}
}

/// An input sample type: `i16` (`INT_PCM`, element size 2) or `i32` (element size 4, of which
/// the encoder uses the upper 16 bits, `(SHORT)(x >> 16)`, as in C).
pub trait Sample: sealed::Sealed + Copy {
    /// `bufElSizes` of the input buffer.
    const EL_SIZE: i32;
}

impl Sample for i16 {
    const EL_SIZE: i32 = 2;
}

impl Sample for i32 {
    const EL_SIZE: i32 = 4;
}

/// The input of [`Encoder::encode_with`].
#[derive(Clone, Copy, Debug)]
pub struct EncodeInput<'a, S: Sample = i16> {
    /// Interleaved samples (`IN_AUDIO_DATA`); `numInSamples` is their number.
    pub pcm: &'a [S],
    /// Ancillary data (`IN_ANCILLRY_DATA`); `numAncBytes` is its length. Passed only if not
    /// empty.
    pub ancillary: &'a [u8],
    /// Metadata (`IN_METADATA_SETUP`), passed if present.
    pub metadata: Option<&'a MetaData>,
}

impl<'a, S: Sample> EncodeInput<'a, S> {
    /// Samples only.
    pub const fn new(pcm: &'a [S]) -> Self {
        EncodeInput { pcm, ancillary: &[], metadata: None }
    }
}

/// The result of a successful `aacEncEncode()` (`AACENC_OutArgs`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[must_use]
pub struct EncodeOutput(AacencOutArgs);

impl EncodeOutput {
    /// `numOutBytes`: bytes written to the output buffer.
    pub const fn out_bytes(&self) -> usize {
        clamp_len(self.0.num_out_bytes)
    }

    /// `numInSamples`: input samples consumed (all channels).
    pub const fn in_samples(&self) -> usize {
        clamp_len(self.0.num_in_samples)
    }

    /// `numAncBytes`: ancillary data bytes consumed.
    pub const fn anc_bytes(&self) -> usize {
        clamp_len(self.0.num_anc_bytes)
    }

    /// `bitResState`: state of the bit reservoir in bits.
    pub const fn bit_res_state(&self) -> i32 {
        self.0.bit_res_state
    }

    /// The `AACENC_OutArgs` as returned.
    pub const fn raw(&self) -> &AacencOutArgs {
        &self.0
    }
}

/// `AACENC_InfoStruct`, returned by [`Encoder::info`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncoderInfo(AacencInfoStruct);

impl EncoderInfo {
    /// `maxOutBufBytes`: maximum number of bitstream bytes of one frame.
    pub const fn max_out_buf_bytes(&self) -> u32 {
        self.0.max_out_buf_bytes
    }

    /// `maxAncBytes`: maximum number of ancillary data bytes per frame.
    pub const fn max_anc_bytes(&self) -> u32 {
        self.0.max_anc_bytes
    }

    /// `inBufFillLevel`: fill level of the internal input buffer in samples per channel.
    pub const fn in_buf_fill_level(&self) -> u32 {
        self.0.in_buf_fill_level
    }

    /// `inputChannels`: number of input channels.
    pub const fn input_channels(&self) -> u32 {
        self.0.input_channels
    }

    /// `frameLength`: input samples per channel consumed by each frame.
    pub const fn frame_length(&self) -> u32 {
        self.0.frame_length
    }

    /// `nDelay`: codec delay in samples per channel.
    pub const fn delay(&self) -> u32 {
        self.0.n_delay
    }

    /// `nDelayCore`: codec delay in samples per channel without the delay of the decoder's SBR.
    pub const fn delay_core(&self) -> u32 {
        self.0.n_delay_core
    }

    /// The configuration (AudioSpecificConfig or StreamMuxConfig): `confBuf[..confSize]`.
    pub fn config(&self) -> &[u8] {
        let n = (self.0.conf_size as usize).min(self.0.conf_buf.len());
        &self.0.conf_buf[..n]
    }

    /// The `AACENC_InfoStruct` as returned.
    pub const fn raw(&self) -> &AacencInfoStruct {
        &self.0
    }
}

/// A length from a C `INT` count: negative counts (not returned by the encoder) give 0.
const fn clamp_len(n: i32) -> usize {
    if n < 0 {
        0
    } else {
        n as usize
    }
}

/// A slice length as a C `INT`. Lengths beyond `INT_MAX` are reported as `INT_MAX`, so the
/// library never accesses more than the slice.
fn c_len(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// One step of [`EncoderBuilder`].
#[derive(Clone, Copy, Debug)]
struct SetOp {
    id: u32,
    value: u32,
}

/// Builder of an [`Encoder`]: `aacEncOpen()`, the parameters, and the initialization.
#[derive(Clone, Debug, Default)]
#[must_use = "an EncoderBuilder does nothing until `build()` is called"]
pub struct EncoderBuilder {
    modules: u32,
    max_channels: u32,
    ops: Vec<SetOp>,
    threads: u32,
}

impl EncoderBuilder {
    /// `encModules` of `aacEncOpen()` (default [`Modules::ALL`]).
    pub fn modules(mut self, modules: Modules) -> Self {
        self.modules = modules.bits();
        self
    }

    /// `maxChannels` of `aacEncOpen()` (default 0: 8 channels). The low byte is the maximum
    /// number of AAC channels, the second byte that of SBR channels (0: as AAC).
    pub fn max_channels(mut self, max_channels: u32) -> Self {
        self.max_channels = max_channels;
        self
    }

    /// The thread count of the opt-in multi-threading (default 1; see [`Encoder::set_threads`]).
    pub fn threads(mut self, threads: u32) -> Self {
        self.threads = threads;
        self
    }

    /// Adds `aacEncoder_SetParam()` of a parameter. Parameters are set in the order they are
    /// added.
    pub fn param(mut self, param: Param) -> Self {
        self.ops.push(SetOp { id: param.id() as u32, value: param.value() });
        self
    }

    /// Adds `aacEncoder_SetParam(id, value)` with any parameter ID and value.
    pub fn param_raw(mut self, id: u32, value: u32) -> Self {
        self.ops.push(SetOp { id, value });
        self
    }

    /// [`Param::Aot`]
    pub fn aot(self, aot: Aot) -> Self {
        self.param(Param::Aot(aot))
    }

    /// [`Param::SampleRate`]
    pub fn sample_rate(self, hz: u32) -> Self {
        self.param(Param::SampleRate(hz))
    }

    /// [`Param::ChannelMode`]
    pub fn channel_mode(self, mode: ChannelMode) -> Self {
        self.param(Param::ChannelMode(mode))
    }

    /// [`Param::ChannelOrder`]
    pub fn channel_order(self, order: ChannelOrder) -> Self {
        self.param(Param::ChannelOrder(order))
    }

    /// [`Bitrate::Cbr`] adds [`Param::BitrateMode`] (CBR) and [`Param::Bitrate`];
    /// [`Bitrate::Vbr`] adds [`Param::BitrateMode`].
    pub fn bitrate(self, bitrate: Bitrate) -> Self {
        match bitrate {
            Bitrate::Cbr(bps) => {
                self.param(Param::BitrateMode(BitrateMode::Cbr)).param(Param::Bitrate(bps))
            }
            Bitrate::Vbr(mode) => self.param(Param::BitrateMode(mode)),
        }
    }

    /// [`Param::Transmux`]
    pub fn transport(self, transport: Transport) -> Self {
        self.param(Param::Transmux(transport))
    }

    /// [`Param::Afterburner`]
    pub fn afterburner(self, on: bool) -> Self {
        self.param(Param::Afterburner(on))
    }

    /// [`Param::SbrMode`]
    pub fn sbr_mode(self, mode: SbrMode) -> Self {
        self.param(Param::SbrMode(mode))
    }

    /// [`Param::SbrRatio`]
    pub fn sbr_ratio(self, ratio: SbrRatio) -> Self {
        self.param(Param::SbrRatio(ratio))
    }

    /// [`Param::GranuleLength`]
    pub fn granule_length(self, samples: u32) -> Self {
        self.param(Param::GranuleLength(samples))
    }

    /// [`Param::Bandwidth`]
    pub fn bandwidth(self, hz: u32) -> Self {
        self.param(Param::Bandwidth(hz))
    }

    /// [`Param::PeakBitrate`]
    pub fn peak_bitrate(self, bps: u32) -> Self {
        self.param(Param::PeakBitrate(bps))
    }

    /// [`Param::HeaderPeriod`]
    pub fn header_period(self, frames: u32) -> Self {
        self.param(Param::HeaderPeriod(frames))
    }

    /// [`Param::SignalingMode`]
    pub fn signaling_mode(self, mode: SignalingMode) -> Self {
        self.param(Param::SignalingMode(mode))
    }

    /// [`Param::TpSubframes`]
    pub fn tp_subframes(self, n: u32) -> Self {
        self.param(Param::TpSubframes(n))
    }

    /// [`Param::AudioMuxVersion`]
    pub fn audio_mux_version(self, v: u32) -> Self {
        self.param(Param::AudioMuxVersion(v))
    }

    /// [`Param::Protection`]
    pub fn protection(self, crc: bool) -> Self {
        self.param(Param::Protection(crc))
    }

    /// [`Param::AncillaryBitrate`]
    pub fn ancillary_bitrate(self, bps: u32) -> Self {
        self.param(Param::AncillaryBitrate(bps))
    }

    /// [`Param::MetadataMode`]
    pub fn metadata_mode(self, mode: MetadataMode) -> Self {
        self.param(Param::MetadataMode(mode))
    }

    /// Opens the encoder (`aacEncOpen()`), sets the parameters in the order they were added
    /// (`aacEncoder_SetParam()`), and initializes it (`aacEncEncode(h, NULL, NULL, NULL,
    /// NULL)`). The first failing call ends the build; the encoder is then closed.
    pub fn build(self) -> Result<Encoder, BuildError> {
        let mut enc = Encoder::open(Modules::from_bits(self.modules), self.max_channels)
            .map_err(BuildError::Open)?;
        enc.set_threads(self.threads);
        for op in &self.ops {
            enc.set_raw(op.id, op.value).map_err(|error| BuildError::SetParam {
                id: op.id,
                value: op.value,
                error,
            })?;
        }
        enc.initialize().map_err(BuildError::Initialize)?;
        Ok(enc)
    }
}

/// An AAC encoder instance (`HANDLE_AACENCODER`). Dropping it calls `aacEncClose()`.
pub struct Encoder {
    /// The handle; `Some` from a successful `aacEncOpen()` until the drop.
    handle: Option<Box<AacEncoder>>,
    /// `Send` but not `Sync`.
    _not_sync: PhantomData<Cell<()>>,
}

impl fmt::Debug for Encoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Encoder").finish_non_exhaustive()
    }
}

impl Encoder {
    /// A builder with the defaults of `aacEncOpen(&h, 0, 0)`.
    pub fn builder() -> EncoderBuilder {
        EncoderBuilder::default()
    }

    /// `aacEncOpen(&h, modules, maxChannels)` without setting parameters or initializing.
    ///
    /// ```
    /// use fdk_aac_rs::encoder::{Aot, ChannelMode, Encoder, EncoderError, Modules, Param, ParamId};
    ///
    /// let mut enc = Encoder::open(Modules::AAC, 2)?;
    /// // HE-AAC needs the SBR module, which was not allocated.
    /// assert_eq!(enc.set(Param::Aot(Aot::HeAac)), Err(EncoderError::InvalidConfig));
    /// enc.set(Param::ChannelMode(ChannelMode::Stereo))?;
    /// enc.set(Param::SampleRate(48_000))?;
    /// enc.initialize()?;
    /// assert_eq!(enc.get(ParamId::Aot), Aot::AacLc as u32);
    /// # Ok::<(), EncoderError>(())
    /// ```
    pub fn open(modules: Modules, max_channels: u32) -> Result<Encoder, EncoderError> {
        let mut handle = None;
        EncoderError::check(lib::aac_enc_open(Some(&mut handle), modules.bits(), max_channels))?;
        if handle.is_none() {
            // Not reachable: aacEncOpen() returns the handle with AACENC_OK.
            return Err(EncoderError::InvalidHandle);
        }
        Ok(Encoder { handle, _not_sync: PhantomData })
    }

    /// Sets the thread count of the opt-in multi-threading (not a function of the C library).
    ///
    /// With 1 (the default; 0 is the same) the encoder is single-threaded. With more, it owns a
    /// pool of `min(threads, channels)` threads (created with the next frame) and runs the
    /// independent per-channel and per-element work of each frame on it; with SBR, the SBR
    /// encoder of a frame runs on a further thread at the same time as the core of the
    /// previous frame (the core then gets `min(threads - 1, channels)` threads). Idle workers
    /// park after a short spin. The output is identical for every thread count; with SBR it is
    /// delayed by one call (see [the module documentation](self#threads-and-the-output-delay)
    /// and [`Encoder::output_delay`]). A process that encodes several streams at once usually gets
    /// more from one thread per encoder.
    ///
    /// ```
    /// use fdk_aac_rs::encoder::{ChannelMode, Encoder};
    ///
    /// let mut enc = Encoder::builder()
    ///     .sample_rate(48_000)
    ///     .channel_mode(ChannelMode::Stereo)
    ///     .threads(2)
    ///     .build()?;
    /// assert_eq!(enc.threads(), 2);
    /// enc.set_threads(1);
    /// # Ok::<(), fdk_aac_rs::encoder::BuildError>(())
    /// ```
    pub fn set_threads(&mut self, threads: u32) {
        if let Some(h) = self.handle.as_deref_mut() {
            lib::aac_enc_set_threads(h, threads);
        }
    }

    /// The thread count of [`Encoder::set_threads`] (at least 1).
    pub fn threads(&self) -> u32 {
        self.handle.as_deref().map_or(1, lib::aac_enc_get_threads)
    }

    /// Sets how an encoder with more than one thread chooses its thread mode (tests; see
    /// `fdk_aac_rs_fdk::par::tune`).
    #[doc(hidden)]
    pub fn set_tuning(&mut self, tuning: fdk_aac_rs_fdk::par::Tuning) {
        if let Some(h) = self.handle.as_deref_mut() {
            lib::aac_enc_set_tuning(h, tuning);
        }
    }

    /// The frames encoded in each thread mode (tests).
    #[doc(hidden)]
    pub fn mode_frames(&self) -> [u64; fdk_aac_rs_fdk::par::tune::MAX_MODES] {
        self.handle
            .as_deref()
            .map_or([0; fdk_aac_rs_fdk::par::tune::MAX_MODES], lib::aac_enc_get_mode_frames)
    }

    /// The output delay in calls (not a function of the C library): 1 once the encoder returns
    /// the result of each [`Encoder::encode`] or [`Encoder::flush`] with the next call (from
    /// the first such call made with more than one thread and SBR, for the rest of the
    /// encoder's life), otherwise 0. See
    /// [the module documentation](self#threads-and-the-output-delay).
    pub fn output_delay(&self) -> u32 {
        self.handle.as_deref().map_or(0, lib::aac_enc_get_output_delay)
    }

    /// `aacEncoder_SetParam()`.
    pub fn set(&mut self, param: Param) -> Result<(), EncoderError> {
        self.set_raw(param.id() as u32, param.value())
    }

    /// `aacEncoder_SetParam()` with any parameter ID and value.
    pub fn set_raw(&mut self, id: u32, value: u32) -> Result<(), EncoderError> {
        EncoderError::check(lib::aac_encoder_set_param(
            self.handle.as_deref_mut(),
            id as lib::AacencParam,
            value,
        ))
    }

    /// `aacEncoder_GetParam()`.
    pub fn get(&self, id: ParamId) -> u32 {
        self.get_raw(id as u32)
    }

    /// `aacEncoder_GetParam()` with any parameter ID (0 for unknown IDs, as in C).
    pub fn get_raw(&self, id: u32) -> u32 {
        lib::aac_encoder_get_param(self.handle.as_deref(), id as lib::AacencParam)
    }

    /// `aacEncEncode(h, NULL, NULL, NULL, NULL)`: applies the pending parameter changes.
    pub fn initialize(&mut self) -> Result<(), EncoderError> {
        // SAFETY: no buffers are passed.
        EncoderError::check(unsafe {
            lib::aac_enc_encode(self.handle.as_deref_mut(), None, None, None, None)
        })
    }

    /// `aacEncInfo()`.
    pub fn info(&mut self) -> Result<EncoderInfo, EncoderError> {
        let mut info = AacencInfoStruct::default();
        EncoderError::check(lib::aac_enc_info(self.handle.as_deref_mut(), Some(&mut info)))?;
        Ok(EncoderInfo(info))
    }

    /// `aacEncEncode()` of interleaved samples into `out` (`OUT_BITSTREAM_DATA`).
    ///
    /// The encoder consumes [`EncodeOutput::in_samples`] samples; it returns a frame
    /// ([`EncodeOutput::out_bytes`] > 0) once it has the samples of a frame. As in C, it clears
    /// `out` first, and fails with [`EncoderError::EncodeError`] if `out` is shorter than the
    /// frame (an output of [`EncoderInfo::max_out_buf_bytes`] bytes is always large enough).
    pub fn encode<S: Sample>(
        &mut self,
        pcm: &[S],
        out: &mut [u8],
    ) -> Result<EncodeOutput, EncoderError> {
        self.encode_with(&EncodeInput::new(pcm), out)
    }

    /// `aacEncEncode()` with ancillary data and metadata.
    ///
    /// ```
    /// use fdk_aac_rs::encoder::{ChannelMode, EncodeInput, Encoder, MetaData, MetadataMode};
    ///
    /// let mut enc = Encoder::builder()
    ///     .sample_rate(48_000)
    ///     .channel_mode(ChannelMode::Stereo)
    ///     .metadata_mode(MetadataMode::DynamicRangeAndAncillary)
    ///     .build()?;
    /// let pcm = vec![0i16; 2 * 1024];
    /// let metadata = MetaData {
    ///     prog_ref_level_present: 1,
    ///     prog_ref_level: -(23 << 16),
    ///     ..Default::default()
    /// };
    /// let input = EncodeInput { pcm: &pcm, ancillary: b"hello", metadata: Some(&metadata) };
    /// let mut out = vec![0u8; 2048];
    /// let r = enc.encode_with(&input, &mut out)?;
    /// assert_eq!(r.in_samples(), pcm.len());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn encode_with<S: Sample>(
        &mut self,
        input: &EncodeInput<'_, S>,
        out: &mut [u8],
    ) -> Result<EncodeOutput, EncoderError> {
        let n = c_len(input.pcm.len());
        self.call(input.pcm.as_ptr() as *mut c_void, n, S::EL_SIZE, n, input, out)
    }

    /// `aacEncEncode()` with `numInSamples = -1`: flushes the encoder's delay. Returns
    /// [`EncoderError::EncodeEof`] when the flush is complete.
    pub fn flush(&mut self, out: &mut [u8]) -> Result<EncodeOutput, EncoderError> {
        let none: EncodeInput<'_, i16> = EncodeInput::new(&[]);
        self.call(none.pcm.as_ptr() as *mut c_void, 0, i16::EL_SIZE, -1, &none, out)
    }

    /// The `aacEncEncode()` call: the audio buffer (`samples` elements of `el_size` bytes at
    /// `pcm`), `num_in_samples`, and the ancillary data and metadata of `input`.
    fn call<S: Sample>(
        &mut self,
        pcm: *mut c_void,
        samples: i32,
        el_size: i32,
        num_in_samples: i32,
        input: &EncodeInput<'_, S>,
        out: &mut [u8],
    ) -> Result<EncodeOutput, EncoderError> {
        let mut in_bufs: [*mut c_void; 3] = [pcm, core::ptr::null_mut(), core::ptr::null_mut()];
        let mut in_ids = [lib::IN_AUDIO_DATA, 0, 0];
        let mut in_sizes = [samples.saturating_mul(el_size), 0, 0];
        let mut in_el_sizes = [el_size, 0, 0];
        let mut n = 1;
        if !input.ancillary.is_empty() {
            in_bufs[n] = input.ancillary.as_ptr() as *mut c_void;
            in_ids[n] = lib::IN_ANCILLRY_DATA;
            in_sizes[n] = c_len(input.ancillary.len());
            in_el_sizes[n] = 1;
            n += 1;
        }
        if let Some(md) = input.metadata {
            in_bufs[n] = md as *const MetaData as *mut c_void;
            in_ids[n] = lib::IN_METADATA_SETUP;
            in_sizes[n] = core::mem::size_of::<MetaData>() as i32;
            in_el_sizes[n] = core::mem::size_of::<MetaData>() as i32;
            n += 1;
        }
        let in_desc = AacencBufDesc {
            num_bufs: n as i32,
            bufs: in_bufs.as_mut_ptr(),
            buffer_identifiers: in_ids.as_mut_ptr(),
            buf_sizes: in_sizes.as_mut_ptr(),
            buf_el_sizes: in_el_sizes.as_mut_ptr(),
        };
        let mut out_bufs: [*mut c_void; 1] = [out.as_mut_ptr() as *mut c_void];
        let mut out_ids = [lib::OUT_BITSTREAM_DATA];
        let mut out_sizes = [c_len(out.len())];
        let mut out_el_sizes = [1i32];
        let out_desc = AacencBufDesc {
            num_bufs: 1,
            bufs: out_bufs.as_mut_ptr(),
            buffer_identifiers: out_ids.as_mut_ptr(),
            buf_sizes: out_sizes.as_mut_ptr(),
            buf_el_sizes: out_el_sizes.as_mut_ptr(),
        };
        let in_args = AacencInArgs { num_in_samples, num_anc_bytes: c_len(input.ancillary.len()) };
        let mut out_args = AacencOutArgs::default();
        // SAFETY: the descriptors point to arrays of `num_bufs` entries that live until the
        // call returns. The audio buffer holds `samples >= max(num_in_samples, 0)` elements of
        // `el_size` bytes (the encoder reads at most `numInSamples` of them), the ancillary
        // buffer `numAncBytes` bytes, the metadata buffer one `AACENC_MetaData`, and the output
        // buffer `bufSizes[0]` writable bytes (the encoder writes at most that many). The
        // encoder does not keep any of the pointers beyond the call.
        let err = unsafe {
            lib::aac_enc_encode(
                self.handle.as_deref_mut(),
                Some(&in_desc),
                Some(&out_desc),
                Some(&in_args),
                Some(&mut out_args),
            )
        };
        EncoderError::check(err)?;
        Ok(EncodeOutput(out_args))
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // aacEncClose() always returns AACENC_OK with a handle pointer.
        let _ = lib::aac_enc_close(Some(&mut self.handle));
    }
}

/// `aacEncGetLibInfo()`: the library information of the encoder and its modules, the entries
/// it filled in (those with a module ID other than `FDK_NONE`), in order.
///
/// ```
/// let info = fdk_aac_rs::encoder::lib_info()?;
/// let aac = info.iter().find(|e| e.title == "AAC Encoder").unwrap();
/// assert_eq!(&aac.version_str[..6], b"4.0.1\0");
/// # Ok::<(), fdk_aac_rs::encoder::EncoderError>(())
/// ```
pub fn lib_info() -> Result<Vec<LibInfo>, EncoderError> {
    use fdk_aac_rs_fdk::audio::{fdk_init_lib_info, FDK_MODULE_LAST, FDK_NONE};
    let mut info = [LibInfo::default(); FDK_MODULE_LAST as usize];
    fdk_init_lib_info(&mut info);
    EncoderError::check(lib::aac_enc_get_lib_info(Some(&mut info)))?;
    Ok(info.into_iter().filter(|e| e.module_id != FDK_NONE).collect())
}
