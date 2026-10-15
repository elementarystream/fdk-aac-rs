// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libMpegTPEnc/include/tpenc_lib.h, libMpegTPEnc/src/tpenc_lib.cpp,
// libMpegTPEnc/src/tp_version.h.

//! Port of `tpenc_lib.h`: the MPEG transport encoder (raw packets, ADIF, ADTS, LATM MCP0/MCP1,
//! LOAS): headers, access units, CRC regions, configuration buffers.
//!
//! The code of libMpegTPEnc is identical in the `aarch64` and `x86` reference builds: the
//! preprocessed translation units differ only in `FDK_ASSERT` (and the build time string of
//! `transportEnc_GetLibInfo()`, which the port does not reproduce). The one
//! difference of the binaries is the division by `noConsideredChannels` in
//! [`transport_enc_write_access_unit`] (see there).
//!
//! # `tp_data.h`
//!
//! `libMpegTPEnc/include/tp_data.h` is the same file as `libMpegTPDec/include/tp_data.h` apart
//! from the library name in its header comment (checked with `diff` on the v2.0.3 sources). Its
//! types live in `fdk_aac_rs_tpdec::tp_data`; this module re-exports them.
//!
//! # Callbacks
//!
//! In C, the transport encoder stores callbacks of the AAC encoder in a `CSTpCallBacks`
//! (registered with `transportEnc_RegisterSbrCallback()`, `..._RegisterSscCallback()` and
//! `..._RegisterUsacCallback()`: a function pointer plus an opaque `void *` each) and calls them
//! while it writes an AudioSpecificConfig: the SBR callback writes the SBR header of each
//! channel element of an ELD configuration with SBR, the SSC callback the
//! SpatialSpecificConfig of LD MPEG Surround (once without a bitstream, `hBs == NULL`, to get
//! its length in bits, then into the bitstream). The USAC callback is registered but never
//! called by the transport encoder.
//!
//! The encoder calls differ from the decoder's: `hBs` and `configChanged` may be `NULL`. The
//! port therefore has its own callback trait, [`TpEncCallbacks`], with
//! `Option<&mut FdkBitStream>` and `Option<&mut u8>` where C passes `NULL`, and `has_*` methods
//! for the `NULL` checks of the C code (`cbSbr != NULL`, `cbSsc != NULL`). The design follows
//! the transport decoder ([`fdk_aac_rs_tpdec::tpdec_lib`]): every function that may call a
//! callback exists in two forms:
//!
//! - `transport_enc_init_cb(h, ..., cb)`, `transport_enc_write_access_unit_cb(h, ..., cb)` and
//!   `transport_enc_get_conf_cb(h, ..., cb)` take the callbacks as an argument. This is the form
//!   for the AAC encoder: it implements the trait on a structure that borrows the SBR and
//!   MPEG Surround encoders, next to the `&mut TransportEnc` it passes, so that no raw pointers
//!   and no aliasing mutable references are needed (C passes the whole `HANDLE_AACENCODER` as
//!   user data, which owns the transport encoder itself).
//! - `transport_enc_init(h, ...)` (etc.) use the callbacks registered with the
//!   `transport_enc_register_*` functions, stored in [`TransportEnc::callbacks`] as
//!   [`TpEncCallBacks`] (the members of `CSTpCallBacks` the encoder uses: function pointers and
//!   `*mut c_void` user data), exactly as in C.
//!
//! The lower-level writers (`transport_enc_write_asc`, `create_stream_mux_config`,
//! `transport_enc_latm_init`, `transport_enc_latm_write`, ...) take `cb: &mut dyn
//! TpEncCallbacks` where C takes `CSTpCallBacks *cb`.
//!
//! # Writer union
//!
//! `TRANSPORTENC::writer` is a union of the ADTS, ADIF, LATM and raw packet writer states. The
//! members are initialized partially, and some fields are read before the active member writes
//! them, so a member can see what another member left in the same bytes. The port therefore
//! keeps the union as its exact byte image, reproduced from the C layout
//! ([`TransportEncWriter`], 120 bytes, the C
//! layout of the LP64 reference builds; 112 bytes with 4-byte pointers on ref-i686) and
//! loads/stores the active member around each call. Two members of the image are pointers in
//! C: `LATM_STREAM::config[0][0]` (stored as [`crate::tpenc_latm::LATM_CONFIG_SET`]) and
//! `STRUCT_ADTS::crcInfo.pCrcLookup` (stored as the index of the CRC table); see PORTING.md,
//! "Deviations from the reference".

use core::ffi::c_void;

use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::{
    fdk_get_valid_bits, fdk_init_bit_stream, fdk_sync_cache, fdk_write_bits, FdkBitStream,
    BS_WRITER,
};
use fdk_aac_rs_fdk::fdk_assert;

pub use fdk_aac_rs_tpdec::tp_data::*;

pub use crate::tpenc_asc::{transport_enc_get_channel_mode, transport_enc_write_asc};

use crate::tpenc_adif::{adif_write_encode_header, adif_write_get_header_bits, AdifInfo};
use crate::tpenc_adts::{
    adts_write_crc_end_reg, adts_write_crc_start_reg, adts_write_encode_header,
    adts_write_end_raw_data_block, adts_write_get_header_bits, adts_write_init, StructAdts,
};
use crate::tpenc_asc::{get_channel_config, transport_enc_get_pce_bits, transport_enc_write_pce};
use crate::tpenc_latm::{
    create_stream_mux_config, transport_enc_latm_add_other_data_bits,
    transport_enc_latm_adjust_subframe_bits, transport_enc_latm_count_total_bit_demand_header,
    transport_enc_latm_get_frame, transport_enc_latm_init, transport_enc_latm_write, LatmStream,
};

// ---------------------------------------------------------------------------------------------
// tpenc_lib.h
// ---------------------------------------------------------------------------------------------

/// `TRANSPORTENC_INBUF_SIZE`
pub const TRANSPORTENC_INBUF_SIZE: i32 = 8192;

/// `TRANSPORTENC_ERROR`
pub type TransportEncError = i32;
/// All fine.
pub const TRANSPORTENC_OK: TransportEncError = 0;
/// Out of memory.
pub const TRANSPORTENC_NO_MEM: TransportEncError = 1;
/// Unknown error (embarrasing).
pub const TRANSPORTENC_UNKOWN_ERROR: TransportEncError = 1;
/// An invalid parameter was passed to a function.
pub const TRANSPORTENC_INVALID_PARAMETER: TransportEncError = 2;
/// Unsupported transport format.
pub const TRANSPORTENC_UNSUPPORTED_FORMAT: TransportEncError = 3;
/// Out of bits. Provide more bits and try again.
pub const TRANSPORTENC_NOT_ENOUGH_BITS: TransportEncError = 4;
/// Error in configuration.
pub const TRANSPORTENC_INVALID_CONFIG: TransportEncError = 5;
/// LATM: number of subframes out of range.
pub const TRANSPORTENC_LATM_INVALID_NR_OF_SUBFRAMES: TransportEncError = 6;
/// LOAS format not supported.
pub const TRANSPORTENC_LOAS_NOT_AVAILABLE: TransportEncError = 7;
/// AudioMuxElement length not aligned to 1 byte.
pub const TRANSPORTENC_INVALID_LATM_ALIGNMENT: TransportEncError = 8;
/// Invalid transmission frame length (< 0).
pub const TRANSPORTENC_INVALID_TRANSMISSION_FRAME_LENGTH: TransportEncError = 9;
/// Invalid CELP frame length found (>= 62).
pub const TRANSPORTENC_INVALID_CELP_FRAME_LENGTH: TransportEncError = 10;
/// Frame bits is not 40 and not 80.
pub const TRANSPORTENC_INVALID_FRAME_BITS: TransportEncError = 11;
/// Unknown AOT found.
pub const TRANSPORTENC_INVALID_AOT: TransportEncError = 12;
/// Invalid Access Unit length (not byte-aligned).
pub const TRANSPORTENC_INVALID_AU_LENGTH: TransportEncError = 13;

/// `TP_FLAG_MPEG4`: MPEG4 (instead of MPEG2)
pub const TP_FLAG_MPEG4: u32 = 1;
/// `TP_FLAG_LATM_AMV`: LATM AudioMuxVersion
pub const TP_FLAG_LATM_AMV: u32 = 2;
/// `TP_FLAG_LATM_AMVA`: LATM AudioMuxVersionA
pub const TP_FLAG_LATM_AMVA: u32 = 4;

// ---------------------------------------------------------------------------------------------
// tp_version.h (libMpegTPEnc)
// ---------------------------------------------------------------------------------------------

/// `TP_LIB_VL0`
pub const TP_LIB_VL0: i32 = 3;
/// `TP_LIB_VL1`
pub const TP_LIB_VL1: i32 = 0;
/// `TP_LIB_VL2`
pub const TP_LIB_VL2: i32 = 0;
/// `TP_LIB_TITLE`
pub const TP_LIB_TITLE: &str = "MPEG Transport";
/// `TP_LIB_BUILD_DATE`: `__DATE__` in C, which the port cannot reproduce (empty).
pub const TP_LIB_BUILD_DATE: &str = "";
/// `TP_LIB_BUILD_TIME`: `__TIME__` in C, see [`TP_LIB_BUILD_DATE`].
pub const TP_LIB_BUILD_TIME: &str = "";

// ---------------------------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------------------------

/// The transport encoder callbacks (the members of `CSTpCallBacks` the encoder calls),
/// implemented by the AAC encoder. See the module documentation.
pub trait TpEncCallbacks {
    /// `cbSbr != NULL`
    fn has_sbr(&self) -> bool;
    /// `cbSbr(self, hBs, sampleRateIn, sampleRateOut, samplesPerFrame, coreCodec, elementID,
    /// elementIndex, harmonicSbr, stereoConfigIndex, configMode, configChanged,
    /// downscaleFactor)`: write the SBR header of channel element `element_index`. The
    /// transport encoder passes `configChanged == NULL` (`None`).
    #[allow(clippy::too_many_arguments)]
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
        config_changed: Option<&mut u8>,
        downscale_factor: i32,
    ) -> i32;

    /// `cbSsc != NULL`
    fn has_ssc(&self) -> bool;
    /// `cbSsc(self, hBs, coreCodec, samplingRate, frameSize, numChannels, stereoConfigIndex,
    /// coreSbrFrameLengthIndex, configBytes, configMode, configChanged)`: write the
    /// SpatialSpecificConfig into `hbs`, or only return its length in bits for `hbs == None`
    /// (`NULL`). The transport encoder passes `configChanged == NULL` (`None`).
    #[allow(clippy::too_many_arguments)]
    fn ssc(
        &mut self,
        hbs: Option<&mut FdkBitStream>,
        core_codec: AudioObjectType,
        sampling_rate: i32,
        frame_size: i32,
        num_channels: i32,
        stereo_config_index: i32,
        core_sbr_frame_length_index: i32,
        config_bytes: i32,
        config_mode: u8,
        config_changed: Option<&mut u8>,
    ) -> i32;
}

/// `cbSbr_t` as the encoder calls it.
pub type CbSbrEnc = fn(
    *mut c_void,
    &mut FdkBitStream,
    i32,
    i32,
    i32,
    AudioObjectType,
    Mp4ElementId,
    i32,
    u8,
    u8,
    u8,
    Option<&mut u8>,
    i32,
) -> i32;
/// `cbSsc_t` as the encoder calls it.
pub type CbSscEnc = fn(
    *mut c_void,
    Option<&mut FdkBitStream>,
    AudioObjectType,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    u8,
    Option<&mut u8>,
) -> i32;
/// `cbUsac_t`
pub type CbUsacEnc = fn(*mut c_void, &mut FdkBitStream) -> i32;

/// The members of `CSTpCallBacks` that the transport encoder registers and uses: function
/// pointers and their user data pointers, as in C. (`cbUpdateConfig`, `cbFreeMem`,
/// `cbCtrlCFGChange` and `cbUniDrc` stay `NULL` in the encoder and are not kept.)
#[derive(Clone, Copy, Debug)]
pub struct TpEncCallBacks {
    /// Function pointer for SSC writer callback.
    pub cb_ssc: Option<CbSscEnc>,
    /// User data pointer for SSC writer callback.
    pub cb_ssc_data: *mut c_void,
    /// Function pointer for SBR header writer callback.
    pub cb_sbr: Option<CbSbrEnc>,
    /// User data pointer for SBR header writer callback.
    pub cb_sbr_data: *mut c_void,
    /// Function pointer for USAC SC writer callback (never called by the transport encoder).
    pub cb_usac: Option<CbUsacEnc>,
    /// User data pointer for USAC SC writer callback.
    pub cb_usac_data: *mut c_void,
}

impl Default for TpEncCallBacks {
    fn default() -> Self {
        Self {
            cb_ssc: None,
            cb_ssc_data: core::ptr::null_mut(),
            cb_sbr: None,
            cb_sbr_data: core::ptr::null_mut(),
            cb_usac: None,
            cb_usac_data: core::ptr::null_mut(),
        }
    }
}

// SAFETY: the user data pointers are only handed to the callbacks; moving the structure to
// another thread is as safe as moving the objects they point to.
unsafe impl Send for TpEncCallBacks {}

impl TpEncCallbacks for TpEncCallBacks {
    fn has_sbr(&self) -> bool {
        self.cb_sbr.is_some()
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
        config_changed: Option<&mut u8>,
        downscale_factor: i32,
    ) -> i32 {
        match self.cb_sbr {
            Some(f) => f(
                self.cb_sbr_data,
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
            ),
            None => 0,
        }
    }

    fn has_ssc(&self) -> bool {
        self.cb_ssc.is_some()
    }

    fn ssc(
        &mut self,
        hbs: Option<&mut FdkBitStream>,
        core_codec: AudioObjectType,
        sampling_rate: i32,
        frame_size: i32,
        num_channels: i32,
        stereo_config_index: i32,
        core_sbr_frame_length_index: i32,
        config_bytes: i32,
        config_mode: u8,
        config_changed: Option<&mut u8>,
    ) -> i32 {
        match self.cb_ssc {
            Some(f) => f(
                self.cb_ssc_data,
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
            ),
            None => 0,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Byte images of the writer union
// ---------------------------------------------------------------------------------------------

#[inline]
pub(crate) fn rd_u8(b: &[u8], o: usize) -> u8 {
    b[o]
}
#[inline]
pub(crate) fn rd_u16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
#[inline]
pub(crate) fn rd_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
#[inline]
pub(crate) fn rd_i32(b: &[u8], o: usize) -> i32 {
    rd_u32(b, o) as i32
}
#[inline]
pub(crate) fn rd_u64(b: &[u8], o: usize) -> u64 {
    rd_u32(b, o) as u64 | (rd_u32(b, o + 4) as u64) << 32
}
#[inline]
pub(crate) fn wr_u8(b: &mut [u8], o: usize, v: u8) {
    b[o] = v;
}
#[inline]
pub(crate) fn wr_u16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
#[inline]
pub(crate) fn wr_u32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
#[inline]
pub(crate) fn wr_i32(b: &mut [u8], o: usize, v: i32) {
    wr_u32(b, o, v as u32);
}
#[inline]
pub(crate) fn wr_u64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

/// The reference is an ILP32 build (`x86_32`, ref-i686): pointers have 4 bytes, which shifts
/// the fields after the pointers of the writer union's members.
pub(crate) const ILP32: bool = matches!(fdk_aac_rs_fdk::ARCH, fdk_aac_rs_fdk::Arch::X86_32);
/// The size of a C pointer in the reference build.
pub(crate) const PTR_SIZE: usize = if ILP32 { 4 } else { 8 };

/// Reads a pointer-sized value (the port's identifier of a pointer).
#[inline]
pub(crate) fn rd_ptr(b: &[u8], o: usize) -> u64 {
    if ILP32 {
        rd_u32(b, o) as u64
    } else {
        rd_u64(b, o)
    }
}
/// Writes a pointer-sized value (the port's identifier of a pointer).
#[inline]
pub(crate) fn wr_ptr(b: &mut [u8], o: usize, v: u64) {
    if ILP32 {
        wr_u32(b, o, v as u32)
    } else {
        wr_u64(b, o, v)
    }
}

/// `RAWPACKETS_INFO`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RawPacketsInfo {
    pub cur_sub_frame: i32,
    pub n_sub_frames: i32,
    pub prev_bits: i32,
}

impl RawPacketsInfo {
    /// Reads the structure from its C layout (the writer union of the transport encoder).
    pub fn load(b: &[u8]) -> Self {
        RawPacketsInfo {
            cur_sub_frame: rd_i32(b, 0),
            n_sub_frames: rd_i32(b, 4),
            prev_bits: rd_i32(b, 8),
        }
    }

    /// Writes the structure in its C layout.
    pub fn store(&self, b: &mut [u8]) {
        wr_i32(b, 0, self.cur_sub_frame);
        wr_i32(b, 4, self.n_sub_frames);
        wr_i32(b, 8, self.prev_bits);
    }
}

/// Size of the writer union of `struct TRANSPORTENC` in the reference builds (the size of
/// `STRUCT_ADTS`: 120 bytes on the LP64 builds, 112 on ref-i686).
pub const TRANSPORTENC_WRITER_SIZE: usize = crate::tpenc_adts::STRUCT_ADTS_SIZE;

/// The writer union of `struct TRANSPORTENC` (`adts`, `adif`, `latm`, `raw`) as its byte image
/// in the C layout (see the module documentation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportEncWriter {
    pub bytes: [u8; TRANSPORTENC_WRITER_SIZE],
}

impl Default for TransportEncWriter {
    fn default() -> Self {
        Self { bytes: [0; TRANSPORTENC_WRITER_SIZE] }
    }
}

impl TransportEncWriter {
    /// `writer.adts`
    pub fn adts(&self) -> StructAdts {
        StructAdts::load(&self.bytes)
    }
    /// Stores `writer.adts`.
    pub fn set_adts(&mut self, v: &StructAdts) {
        v.store(&mut self.bytes);
    }
    /// `writer.adif`
    pub fn adif(&self) -> AdifInfo {
        AdifInfo::load(&self.bytes)
    }
    /// Stores `writer.adif`.
    pub fn set_adif(&mut self, v: &AdifInfo) {
        v.store(&mut self.bytes);
    }
    /// `writer.latm`
    pub fn latm(&self) -> LatmStream {
        LatmStream::load(&self.bytes)
    }
    /// Stores `writer.latm`.
    pub fn set_latm(&mut self, v: &LatmStream) {
        v.store(&mut self.bytes);
    }
    /// `writer.raw`
    pub fn raw(&self) -> RawPacketsInfo {
        RawPacketsInfo::load(&self.bytes)
    }
    /// Stores `writer.raw`.
    pub fn set_raw(&mut self, v: &RawPacketsInfo) {
        v.store(&mut self.bytes);
    }
}

// ---------------------------------------------------------------------------------------------
// struct TRANSPORTENC
// ---------------------------------------------------------------------------------------------

/// `struct TRANSPORTENC` (`HANDLE_TRANSPORTENC` is `&mut TransportEnc`).
#[derive(Clone, Debug, Default)]
pub struct TransportEnc {
    pub config: CoderConfig,
    /// MPEG4 transport type.
    pub transport_fmt: TransportType,

    pub bit_stream: FdkBitStream,
    /// `bsBuffer`: the output buffer of `transportEnc_Init()` (shared with the caller).
    pub bs_buffer: BitMem,
    pub bs_buffer_size: i32,

    /// Indicates frame period when PCE must be written in raw_data_block. -1 means not to write
    /// a PCE in raw_dat_block.
    pub pce_frame_counter: i32,
    /// The writer union (`adts`, `adif`, `latm`, `raw`).
    pub writer: TransportEncWriter,

    /// The registered callbacks (used by the functions without `_cb` suffix).
    pub callbacks: TpEncCallBacks,
}

/// `HANDLE_TRANSPORTENC`
pub type HandleTransportEnc<'a> = &'a mut TransportEnc;

/// `transportEnc_Open(phTpEnc)`: allocates a transport encoder (all fields zero, like the
/// `calloc()` of C).
///
/// C returns `TRANSPORTENC_INVALID_PARAMETER` for `phTpEnc == NULL` (not representable) and
/// `TRANSPORTENC_NO_MEM` if the allocation fails (the port aborts, as Rust allocations do).
pub fn transport_enc_open(ph_tp_enc: &mut Option<Box<TransportEnc>>) -> TransportEncError {
    *ph_tp_enc = Some(Box::default());
    TRANSPORTENC_OK
}

/// `getPceRepetitionRate(channelMode, channelConfigZero, transportFmt, headerPeriod,
/// matrixMixdownA)`: the frame period of a PCE in the raw_data_block, -1 for none.
pub fn get_pce_repetition_rate(
    channel_mode: ChannelMode,
    channel_config_zero: i32,
    transport_fmt: TransportType,
    header_period: i32,
    matrix_mixdown_a: i32,
) -> i32 {
    let mut pce_frame_counter: i32 = -1; // variable to be returned

    if header_period > 0 {
        match get_channel_config(channel_mode, channel_config_zero as u8) {
            0 => match transport_fmt {
                TT_MP4_ADTS | TT_MP4_LATM_MCP0 | TT_MP4_RAW => {
                    pce_frame_counter = header_period;
                }
                TT_MP4_ADIF if channel_mode == MODE_1_2_2 || channel_mode == MODE_1_2_2_1 => {
                    // ADIF header comprises PCE; repeating pce only meaningful for potential
                    // matrix mixdown
                    pce_frame_counter = header_period;
                }
                // TT_MP4_ADIF (fall through), TT_MP4_LOAS, TT_MP4_LATM_MCP1: PCE in ASC if
                // chChonfig==0
                _ => {
                    pce_frame_counter = -1; // no PCE in raw_data_block
                }
            },
            5 | 6 => {
                // MODE_1_2_2, MODE_1_2_2_1: matrixMixdownCoefficient can only be written if 5.0
                // and 5.1 config present.
                if matrix_mixdown_a != 0 {
                    match transport_fmt {
                        TT_MP4_ADIF | TT_MP4_ADTS | TT_MP4_LOAS | TT_MP4_LATM_MCP1
                        | TT_MP4_LATM_MCP0 | TT_MP4_RAW => {
                            pce_frame_counter = header_period;
                        }
                        _ => {
                            pce_frame_counter = -1; // no PCE in raw_data_block
                        }
                    }
                }
            }
            _ => {
                pce_frame_counter = -1; // no PCE in raw_data_block
            }
        }
    } else {
        pce_frame_counter = -1; // no PCE in raw_data_block
    }

    pce_frame_counter
}

/// `transportEnc_Init(hTpEnc, bsBuffer, bsBufferSize, transportFmt, config, flags)` with the
/// registered callbacks.
pub fn transport_enc_init(
    h_tp_enc: &mut TransportEnc,
    bs_buffer: &BitMem,
    bs_buffer_size: i32,
    transport_fmt: TransportType,
    cconfig: &CoderConfig,
    flags: u32,
) -> TransportEncError {
    let mut cb = h_tp_enc.callbacks;
    transport_enc_init_cb(
        h_tp_enc,
        bs_buffer,
        bs_buffer_size,
        transport_fmt,
        cconfig,
        flags,
        &mut cb,
    )
}

/// `transportEnc_Init(hTpEnc, bsBuffer, bsBufferSize, transportFmt, config, flags)`:
/// initializes the transport encoder for `transport_fmt` and the configuration (copied) on the
/// output buffer `bs_buffer` (`bs_buffer_size` bytes, a power of two, as the bit buffer
/// requires).
pub fn transport_enc_init_cb(
    h_tp_enc: &mut TransportEnc,
    bs_buffer: &BitMem,
    bs_buffer_size: i32,
    transport_fmt: TransportType,
    cconfig: &CoderConfig,
    flags: u32,
    cb: &mut dyn TpEncCallbacks,
) -> TransportEncError {
    // Copy configuration structure
    h_tp_enc.config = *cconfig;

    // Init transportEnc struct.
    h_tp_enc.transport_fmt = transport_fmt;

    h_tp_enc.bs_buffer = bs_buffer.clone();
    h_tp_enc.bs_buffer_size = bs_buffer_size;

    fdk_init_bit_stream(
        &mut h_tp_enc.bit_stream,
        &h_tp_enc.bs_buffer,
        h_tp_enc.bs_buffer_size as u32,
        0,
        BS_WRITER,
    );

    match transport_fmt {
        TT_MP4_ADIF => {
            // Sanity checks
            if h_tp_enc.config.aot != AOT_AAC_LC || h_tp_enc.config.samples_per_frame != 1024 {
                return TRANSPORTENC_INVALID_PARAMETER;
            }
            let mut adif = h_tp_enc.writer.adif();
            adif.header_written = 0;
            adif.sampling_rate = h_tp_enc.config.sampling_rate;
            adif.bit_rate = h_tp_enc.config.bit_rate;
            adif.profile = h_tp_enc.config.aot - 1;
            adif.cm = h_tp_enc.config.channel_mode;
            adif.b_variable_rate = 0;
            adif.instance_tag = 0;
            adif.matrix_mixdown_a = h_tp_enc.config.matrix_mixdown_a as i32;
            adif.pseudo_surround_enable =
                ((h_tp_enc.config.flags & CC_PSEUDO_SURROUND) != 0) as i32;
            h_tp_enc.writer.set_adif(&adif);
        }

        TT_MP4_ADTS => {
            // Sanity checks
            if h_tp_enc.config.aot != AOT_AAC_LC || h_tp_enc.config.samples_per_frame != 1024 {
                return TRANSPORTENC_INVALID_PARAMETER;
            }
            let mut adts = h_tp_enc.writer.adts();
            let r = adts_write_init(&mut adts, &h_tp_enc.config);
            h_tp_enc.writer.set_adts(&adts);
            if r != 0 {
                return TRANSPORTENC_INVALID_PARAMETER;
            }
        }

        TT_MP4_LOAS | TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 => {
            let mut latm = h_tp_enc.writer.latm();
            let error = transport_enc_latm_init(
                &mut latm,
                &mut h_tp_enc.bit_stream,
                &mut h_tp_enc.config,
                flags & TP_FLAG_LATM_AMV,
                transport_fmt,
                cb,
            );
            h_tp_enc.writer.set_latm(&latm);
            if error != TRANSPORTENC_OK {
                return error;
            }
        }

        TT_MP4_RAW => {
            let mut raw = h_tp_enc.writer.raw();
            raw.cur_sub_frame = 0;
            raw.n_sub_frames = h_tp_enc.config.n_sub_frames;
            h_tp_enc.writer.set_raw(&raw);
        }

        _ => return TRANSPORTENC_INVALID_PARAMETER,
    }

    // pceFrameCounter indicates if PCE must be written in raw_data_block.
    h_tp_enc.pce_frame_counter = get_pce_repetition_rate(
        h_tp_enc.config.channel_mode,
        h_tp_enc.config.channel_config_zero as i32,
        transport_fmt,
        h_tp_enc.config.header_period as i32,
        h_tp_enc.config.matrix_mixdown_a as i32,
    );

    TRANSPORTENC_OK
}

/// `transportEnc_AddOtherDataBits(hTpEnc, nBits)`: other data bits for LATM/LOAS
/// (`TRANSPORTENC_UNKOWN_ERROR` for the other formats).
pub fn transport_enc_add_other_data_bits(
    h_tp_enc: &mut TransportEnc,
    n_bits: i32,
) -> TransportEncError {
    match h_tp_enc.transport_fmt {
        TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 | TT_MP4_LOAS => {
            let mut latm = h_tp_enc.writer.latm();
            let tp_err = transport_enc_latm_add_other_data_bits(&mut latm, n_bits);
            h_tp_enc.writer.set_latm(&latm);
            tp_err
        }
        _ => TRANSPORTENC_UNKOWN_ERROR, // TT_MP4_ADTS, TT_MP4_ADIF, TT_MP4_RAW and others
    }
}

/// `transportEnc_GetBitstream(hTp)`: the bitstream the access units are written into.
pub fn transport_enc_get_bitstream(h_tp: &mut TransportEnc) -> &mut FdkBitStream {
    &mut h_tp.bit_stream
}

/// `transportEnc_RegisterSbrCallback(hTpEnc, cbSbr, user_data)`: returns 0, -1 without a
/// handle.
pub fn transport_enc_register_sbr_callback(
    h_tp_enc: Option<&mut TransportEnc>,
    cb_sbr: Option<CbSbrEnc>,
    user_data: *mut c_void,
) -> i32 {
    let Some(h) = h_tp_enc else {
        return -1;
    };
    h.callbacks.cb_sbr = cb_sbr;
    h.callbacks.cb_sbr_data = user_data;
    0
}

/// `transportEnc_RegisterUsacCallback(hTpEnc, cbUsac, user_data)`: returns 0, -1 without a
/// handle.
pub fn transport_enc_register_usac_callback(
    h_tp_enc: Option<&mut TransportEnc>,
    cb_usac: Option<CbUsacEnc>,
    user_data: *mut c_void,
) -> i32 {
    let Some(h) = h_tp_enc else {
        return -1;
    };
    h.callbacks.cb_usac = cb_usac;
    h.callbacks.cb_usac_data = user_data;
    0
}

/// `transportEnc_RegisterSscCallback(hTpEnc, cbSsc, user_data)`: returns 0, -1 without a
/// handle.
pub fn transport_enc_register_ssc_callback(
    h_tp_enc: Option<&mut TransportEnc>,
    cb_ssc: Option<CbSscEnc>,
    user_data: *mut c_void,
) -> i32 {
    let Some(h) = h_tp_enc else {
        return -1;
    };
    h.callbacks.cb_ssc = cb_ssc;
    h.callbacks.cb_ssc_data = user_data;
    0
}

/// `bufferFullness /= ncc` (`sdiv` on AArch64, `idiv` on x86).
///
/// x86 division by zero and `INT_MIN / -1`: the AArch64 result (0, `INT_MIN`) on both variants
/// (deviation).
#[inline]
fn div_ncc(a: i32, ncc: i32) -> i32 {
    if ncc == 0 {
        0
    } else {
        a.wrapping_div(ncc)
    }
}

/// `transportEnc_WriteAccessUnit(hTp, total_bits, bufferFullness, noConsideredChannels)` with
/// the registered callbacks.
pub fn transport_enc_write_access_unit(
    h_tp: Option<&mut TransportEnc>,
    frame_used_bits: i32,
    buffer_fullness: i32,
    ncc: i32,
) -> TransportEncError {
    let Some(h_tp) = h_tp else {
        return TRANSPORTENC_INVALID_PARAMETER;
    };
    let mut cb = h_tp.callbacks;
    transport_enc_write_access_unit_cb(Some(h_tp), frame_used_bits, buffer_fullness, ncc, &mut cb)
}

/// `transportEnc_WriteAccessUnit(hTp, total_bits, bufferFullness, noConsideredChannels)`:
/// writes the transport header of an access unit of `frame_used_bits` bits (and a PCE in the
/// raw_data_block if due).
pub fn transport_enc_write_access_unit_cb(
    h_tp: Option<&mut TransportEnc>,
    mut frame_used_bits: i32,
    mut buffer_fullness: i32,
    ncc: i32,
    cb: &mut dyn TpEncCallbacks,
) -> TransportEncError {
    let mut err = TRANSPORTENC_OK;

    let Some(h_tp) = h_tp else {
        return TRANSPORTENC_INVALID_PARAMETER;
    };

    // In case of writing PCE in raw_data_block frameUsedBits must be adapted.
    if h_tp.pce_frame_counter >= h_tp.config.header_period as i32 {
        frame_used_bits = frame_used_bits.wrapping_add(transport_enc_get_pce_bits(
            h_tp.config.channel_mode,
            h_tp.config.matrix_mixdown_a as i32,
            3,
        )); // Consider 3 bits ID signalling in alignment
    }

    match h_tp.transport_fmt {
        TT_MP4_ADIF => {
            fdk_init_bit_stream(
                &mut h_tp.bit_stream,
                &h_tp.bs_buffer,
                h_tp.bs_buffer_size as u32,
                0,
                BS_WRITER,
            );
            let mut adif = h_tp.writer.adif();
            let r = adif_write_encode_header(&mut adif, &mut h_tp.bit_stream, buffer_fullness);
            h_tp.writer.set_adif(&adif);
            if 0 != r {
                err = TRANSPORTENC_INVALID_CONFIG;
            }
        }
        TT_MP4_ADTS => {
            buffer_fullness = div_ncc(buffer_fullness, ncc); // Number of Considered Channels
            buffer_fullness /= 32;
            buffer_fullness = buffer_fullness.min(0x7FF); // Signal variable rate
            let mut adts = h_tp.writer.adts();
            adts_write_encode_header(
                &mut adts,
                &mut h_tp.bit_stream,
                buffer_fullness,
                frame_used_bits,
            );
            h_tp.writer.set_adts(&adts);
        }
        TT_MP4_LOAS | TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 => {
            buffer_fullness = div_ncc(buffer_fullness, ncc); // Number of Considered Channels
            buffer_fullness /= 32;
            buffer_fullness = buffer_fullness.min(0xFF); // Signal variable rate
            let mut latm = h_tp.writer.latm();
            transport_enc_latm_write(
                &mut latm,
                &mut h_tp.bit_stream,
                frame_used_bits,
                buffer_fullness,
                &mut h_tp.config,
                cb,
            );
            h_tp.writer.set_latm(&latm);
        }
        TT_MP4_RAW => {
            let mut raw = h_tp.writer.raw();
            if raw.cur_sub_frame >= raw.n_sub_frames {
                raw.cur_sub_frame = 0;
                fdk_init_bit_stream(
                    &mut h_tp.bit_stream,
                    &h_tp.bs_buffer,
                    h_tp.bs_buffer_size as u32,
                    0,
                    BS_WRITER,
                );
            }
            raw.prev_bits = fdk_get_valid_bits(&mut h_tp.bit_stream) as i32;
            h_tp.writer.set_raw(&raw);
        }
        _ => {
            err = TRANSPORTENC_UNSUPPORTED_FORMAT;
        }
    }

    // Write PCE in raw_data_block if required
    if h_tp.pce_frame_counter >= h_tp.config.header_period as i32 {
        let mut crc_index: i32 = 0;
        // Align inside PCE with repsect to the first bit of the raw_data_block()
        let align_anchor: u32 = fdk_get_valid_bits(&mut h_tp.bit_stream);

        // Write PCE element ID bits
        fdk_write_bits(&mut h_tp.bit_stream, ID_PCE as u32, 3);

        let adts_crc =
            h_tp.transport_fmt == TT_MP4_ADTS && h_tp.writer.adts().protection_absent == 0;
        if adts_crc {
            let mut adts = h_tp.writer.adts();
            crc_index = adts_write_crc_start_reg(&mut adts, &mut h_tp.bit_stream, 0);
            h_tp.writer.set_adts(&adts);
        }

        // Write PCE as first raw_data_block element
        transport_enc_write_pce(
            &mut h_tp.bit_stream,
            h_tp.config.channel_mode,
            h_tp.config.sampling_rate,
            0,
            1,
            h_tp.config.matrix_mixdown_a as i32,
            ((h_tp.config.flags & CC_PSEUDO_SURROUND) != 0) as i32,
            align_anchor,
        );

        if h_tp.transport_fmt == TT_MP4_ADTS && h_tp.writer.adts().protection_absent == 0 {
            let mut adts = h_tp.writer.adts();
            adts_write_crc_end_reg(&mut adts, &mut h_tp.bit_stream, crc_index);
            h_tp.writer.set_adts(&adts);
        }
        h_tp.pce_frame_counter = 0; // reset pce frame counter
    }

    if h_tp.pce_frame_counter != -1 {
        // Update pceFrameCounter only if PCE writing is active.
        h_tp.pce_frame_counter = h_tp.pce_frame_counter.wrapping_add(1);
    }

    err
}

/// `transportEnc_EndAccessUnit(hTp, pBits)`: bitstream housekeeping after an access unit was
/// written; corrects `*bits` (the frame bits) to the bits of the access unit.
pub fn transport_enc_end_access_unit(h_tp: &mut TransportEnc, bits: &mut i32) -> TransportEncError {
    match h_tp.transport_fmt {
        TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 | TT_MP4_LOAS => {
            let latm = h_tp.writer.latm();
            transport_enc_latm_adjust_subframe_bits(&latm, bits);
        }
        TT_MP4_ADTS => {
            let mut adts = h_tp.writer.adts();
            adts_write_end_raw_data_block(&mut adts, &mut h_tp.bit_stream, bits);
            h_tp.writer.set_adts(&adts);
        }
        TT_MP4_ADIF => {
            // Substract ADIF header from AU bits, not to be considered.
            let mut adif = h_tp.writer.adif();
            *bits = bits.wrapping_sub(adif_write_get_header_bits(&adif));
            adif.header_written = 1;
            h_tp.writer.set_adif(&adif);
        }
        TT_MP4_RAW => {
            *bits = bits.wrapping_sub(h_tp.writer.raw().prev_bits);
        }
        _ => {}
    }

    TRANSPORTENC_OK
}

/// `transportEnc_GetFrame(hTpEnc, nbytes)`: the size of the completed frame in `*nbytes` (0 if
/// the frame is not complete yet).
pub fn transport_enc_get_frame(h_tp_enc: &mut TransportEnc, nbytes: &mut i32) -> TransportEncError {
    let mut tp_err = TRANSPORTENC_OK;
    let h_bs = &mut h_tp_enc.bit_stream;

    match h_tp_enc.transport_fmt {
        TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 | TT_MP4_LOAS => {
            *nbytes = h_tp_enc.bs_buffer_size;
            let mut latm = h_tp_enc.writer.latm();
            tp_err = transport_enc_latm_get_frame(&mut latm, h_bs, nbytes);
            h_tp_enc.writer.set_latm(&latm);
        }
        TT_MP4_ADTS => {
            let mut adts = h_tp_enc.writer.adts();
            #[allow(clippy::int_plus_one)]
            let complete = adts.current_block >= adts.num_raw_blocks as i32 + 1;
            if complete {
                *nbytes = (fdk_get_valid_bits(h_bs).wrapping_add(7) >> 3) as i32;
                adts.current_block = 0;
            } else {
                *nbytes = 0;
            }
            h_tp_enc.writer.set_adts(&adts);
        }
        TT_MP4_ADIF => {
            // FDK_ASSERT((INT)FDKgetValidBits(hBs) >= 0), without the cache synchronization of
            // FDKgetValidBits() (the assertion is compiled out in the reference)
            fdk_assert!(h_bs.h_bit_buf.valid_bits.wrapping_add(h_bs.bits_in_cache) as i32 >= 0);
            *nbytes = (fdk_get_valid_bits(h_bs).wrapping_add(7) >> 3) as i32;
        }
        TT_MP4_RAW => {
            fdk_sync_cache(h_bs);
            let mut raw = h_tp_enc.writer.raw();
            raw.cur_sub_frame = raw.cur_sub_frame.wrapping_add(1);
            *nbytes = (fdk_get_valid_bits(h_bs).wrapping_sub(raw.prev_bits as u32).wrapping_add(7)
                >> 3) as i32;
            h_tp_enc.writer.set_raw(&raw);
        }
        _ => {}
    }

    tp_err
}

/// `transportEnc_GetStaticBits(hTp, auBits)`: the bits of the transport headers (and of a PCE
/// in the raw_data_block) for a payload of `au_bits` bits.
pub fn transport_enc_get_static_bits(h_tp: &mut TransportEnc, mut au_bits: i32) -> i32 {
    let mut nbits: i32;
    let mut n_pce_bits: i32 = 0;

    // Write PCE within raw_data_block in transport lib.
    if h_tp.pce_frame_counter >= h_tp.config.header_period as i32 {
        n_pce_bits = transport_enc_get_pce_bits(
            h_tp.config.channel_mode,
            h_tp.config.matrix_mixdown_a as i32,
            3,
        ); // Consider 3 bits ID signalling in alignment
           // Adapt required raw_data_block bit consumtpion for AU length information e.g. in
           // LATM/LOAS configuration.
        au_bits = au_bits.wrapping_add(n_pce_bits);
    }

    match h_tp.transport_fmt {
        TT_MP4_ADIF | TT_MP4_RAW => {
            nbits = 0; // Do not consider the ADIF header into the total bitrate
        }
        TT_MP4_ADTS => {
            let mut adts = h_tp.writer.adts();
            nbits = adts_write_get_header_bits(&mut adts);
            h_tp.writer.set_adts(&adts);
        }
        TT_MP4_LOAS | TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 => {
            let mut latm = h_tp.writer.latm();
            nbits =
                transport_enc_latm_count_total_bit_demand_header(&mut latm, au_bits as u32) as i32;
            h_tp.writer.set_latm(&latm);
        }
        _ => {
            nbits = 0;
        }
    }

    // PCE is written in the transport library therefore the bit consumption is part of the
    // transport static bits.
    nbits = nbits.wrapping_add(n_pce_bits);

    nbits
}

/// `transportEnc_Close(phTp)`: frees the transport encoder.
pub fn transport_enc_close(ph_tp: &mut Option<Box<TransportEnc>>) {
    *ph_tp = None;
}

/// `transportEnc_CrcStartReg(hTpEnc, mBits)`: starts a CRC region (ADTS only). Returns the
/// region ID (0 for the other formats).
pub fn transport_enc_crc_start_reg(h_tp_enc: &mut TransportEnc, m_bits: i32) -> i32 {
    let mut crc_reg: i32 = 0;

    if h_tp_enc.transport_fmt == TT_MP4_ADTS {
        let mut adts = h_tp_enc.writer.adts();
        crc_reg = adts_write_crc_start_reg(&mut adts, &mut h_tp_enc.bit_stream, m_bits);
        h_tp_enc.writer.set_adts(&adts);
    }

    crc_reg
}

/// `transportEnc_CrcEndReg(hTpEnc, reg)`: ends a CRC region (ADTS only).
pub fn transport_enc_crc_end_reg(h_tp_enc: &mut TransportEnc, reg: i32) {
    if h_tp_enc.transport_fmt == TT_MP4_ADTS {
        let mut adts = h_tp_enc.writer.adts();
        adts_write_crc_end_reg(&mut adts, &mut h_tp_enc.bit_stream, reg);
        h_tp_enc.writer.set_adts(&adts);
    }
}

/// `transportEnc_GetConf(hTpEnc, cc, dataBuffer, confType)` with the registered callbacks.
pub fn transport_enc_get_conf(
    h_tp_enc: &mut TransportEnc,
    cc: &mut CoderConfig,
    data_buffer: &mut FdkBitStream,
    conf_type: &mut u32,
) -> TransportEncError {
    let mut cb = h_tp_enc.callbacks;
    transport_enc_get_conf_cb(h_tp_enc, cc, data_buffer, conf_type, &mut cb)
}

/// `transportEnc_GetConf(hTpEnc, cc, dataBuffer, confType)`: writes the StreamMuxConfig
/// (LATM/LOAS, `*conf_type = 1`, from the configuration of the transport encoder) or the
/// AudioSpecificConfig (the other formats, `*conf_type = 0`, from `cc`) into `data_buffer`.
pub fn transport_enc_get_conf_cb(
    h_tp_enc: &mut TransportEnc,
    cc: &mut CoderConfig,
    data_buffer: &mut FdkBitStream,
    conf_type: &mut u32,
    cb: &mut dyn TpEncCallbacks,
) -> TransportEncError {
    let mut tp_err = TRANSPORTENC_OK;

    *conf_type = 0; // set confType variable to default

    // write StreamMuxConfig or AudioSpecificConfig depending on format used
    match h_tp_enc.transport_fmt {
        TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 | TT_MP4_LOAS => {
            let mut latm = h_tp_enc.writer.latm();
            tp_err = create_stream_mux_config(&mut latm, data_buffer, 0, &mut h_tp_enc.config, cb);
            h_tp_enc.writer.set_latm(&latm);
            *conf_type = 1; // config is SMC
        }
        _ => {
            if transport_enc_write_asc(data_buffer, cc, cb) != 0 {
                tp_err = TRANSPORTENC_UNKOWN_ERROR;
            }
        }
    }

    tp_err
}

/// `transportEnc_GetLibInfo(info)`: enters the library information into the first free entry
/// of `info` (an array of `FDK_MODULE_LAST` entries).
///
/// A slice shorter than `FDK_MODULE_LAST` without a free entry ends the search.
/// `build_date` and `build_time` are empty.
pub fn transport_enc_get_lib_info(info: Option<&mut [LibInfo]>) -> TransportEncError {
    let Some(info) = info else {
        return TRANSPORTENC_INVALID_PARAMETER;
    };
    // search for next free tab
    let mut i = 0usize;
    while i < FDK_MODULE_LAST as usize {
        match info.get(i) {
            Some(e) if e.module_id == FDK_NONE => break,
            Some(_) => {}
            None => return TRANSPORTENC_UNKOWN_ERROR,
        }
        i += 1;
    }
    if i == FDK_MODULE_LAST as usize {
        return TRANSPORTENC_UNKOWN_ERROR;
    }
    let info = &mut info[i];

    info.module_id = FDK_TPENC;
    info.version = lib_version(TP_LIB_VL0, TP_LIB_VL1, TP_LIB_VL2);
    lib_version_string(info);
    info.build_date = TP_LIB_BUILD_DATE;
    info.build_time = TP_LIB_BUILD_TIME;
    info.title = TP_LIB_TITLE;

    // Set flags
    info.flags = CAPF_ADIF | CAPF_ADTS | CAPF_LATM | CAPF_LOAS | CAPF_RAWPACKETS;

    TRANSPORTENC_OK
}
