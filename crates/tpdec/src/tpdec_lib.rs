// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libMpegTPDec/include/tpdec_lib.h, libMpegTPDec/src/tpdec_lib.cpp.

//! Port of `tpdec_lib.h`: the MPEG transport decoder (raw, ADIF, ADTS, LATM MCP0/MCP1, LOAS,
//! DRM): synchronization, access unit handling, configuration and CRC.
//!
//! The code of this file is identical in the `aarch64` and `x86` reference builds, except for
//! the build time string of `transportDec_GetLibInfo()` (not reproduced, see
//! [`transport_dec_get_lib_info`]).
//!
//! # Callbacks
//!
//! In C, the transport decoder stores the callbacks of the AAC decoder (`CSTpCallBacks`: a
//! function pointer and an opaque `void *` per callback, registered with
//! `transportDec_Register*Callback()`) and calls them while it parses configurations.
//!
//! The port expresses the callbacks as the trait [`TpDecCallbacks`] (in [`crate::tp_data`]).
//! Every function that may call a callback exists in two forms:
//!
//! - `transport_dec_read_access_unit_cb(h_tp, layer, cb)` (and `..._out_of_band_config_cb`,
//!   `..._in_band_config_cb`) take the callbacks as an argument. This is the form for the AAC
//!   decoder: it can implement the trait on a structure that borrows the parts of the decoder
//!   the callbacks need, next to the `&mut TransportDec` it passes, so that no raw pointers and
//!   no aliasing mutable references are needed.
//! - `transport_dec_read_access_unit(h_tp, layer)` (etc.) use the callbacks registered with
//!   the `transport_dec_register_*` functions, stored in [`TransportDec::callbacks`] as the C
//!   structure [`CSTpCallBacks`] (function pointers and `*mut c_void` user data), exactly as in
//!   C.
//!
//! The trait has `has_*` methods for the four callbacks the C code checks for `NULL`
//! (`cbSsc`, `cbSbr`, `cbUsac`, `cbUniDrc`).
//!
//! # Parser union
//!
//! `transportdec_parser_t` is a union of the ADTS, ADIF, LATM and DRM parser states in C; the
//! port keeps them as separate members of [`TransportDecParser`]. Only the member of the
//! transport format is used, with one exception: `transportDec_InBandConfig()` reads and writes
//! `parser.latm.usacExplicitCfgChanged` for any format with USAC. For raw packets the union is
//! never written, so the byte is 0 in C, as in the port. For DRM (xHE-AAC) it is byte 38 of the
//! union, inside `parser.drm.crcInfo.crcRegData[2].maxBits`, which DRM never writes either
//! (`drmRead_CrcStartReg()` resets the CRC, so only region 0 is used), so it is 0 as well.
//!
//! # Layers
//!
//! The transport decoder supports one layer (`transportDec_Open()` rejects more). Arrays with
//! one entry per layer (`bitStream`, `accessUnitAnchor`, `auLength`, `ctrlCFGChange`) have no
//! defined reference result for a layer other than 0. The port uses
//! entry 0 for such layers (`asc[]`, which has two entries, accepts layers 0 and 1) (PORTING.md,
//! "Deviations from the reference").

use core::ffi::c_void;

use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::{
    fdk_byte_align, fdk_feed_buffer, fdk_get_valid_bits, fdk_init_bit_stream, fdk_push_back,
    fdk_push_bi_directional, fdk_push_for, fdk_read_bits, fdk_reset_bitbuffer, FdkBitStream,
    BS_READER, BUFSIZE_DUMMY_VALUE,
};
use fdk_aac_rs_fdk::crc::fdk_crc_reset;

pub use crate::tp_data::*;
pub use crate::tpdec_asc::{
    audio_specific_config_init, audio_specific_config_parse, cprogram_config_compare,
    cprogram_config_get_channel_description, cprogram_config_get_default,
    cprogram_config_get_element_table, cprogram_config_get_pce_ch_map, cprogram_config_init,
    cprogram_config_is_valid, cprogram_config_lookup_element, cprogram_config_read,
    cprogram_config_reset,
};

use crate::tpdec_adif::{adif_read_decode_header, CAdifHeader};
use crate::tpdec_adts::*;
use crate::tpdec_asc::drm_raw_sdc_audio_config_parse;
use crate::tpdec_drm::*;
use crate::tpdec_latm::*;

// ---------------------------------------------------------------------------------------------
// tpdec_lib.h
// ---------------------------------------------------------------------------------------------

/// `TRANSPORTDEC_ERROR`
pub type TransportDecError = i32;
/// All fine.
pub const TRANSPORTDEC_OK: TransportDecError = 0;

// Synchronization errors. Wait for new input data and try again.
pub const TPDEC_SYNC_ERROR_START: TransportDecError = 0x100;
/// Out of bits. Provide more bits and try again.
pub const TRANSPORTDEC_NOT_ENOUGH_BITS: TransportDecError = 0x101;
/// No sync was found or sync got lost. Keep trying.
pub const TRANSPORTDEC_SYNC_ERROR: TransportDecError = 0x102;
pub const TPDEC_SYNC_ERROR_END: TransportDecError = 0x103;

// Decode errors. Mostly caused due to bit errors.
pub const TPDEC_DECODE_ERROR_START: TransportDecError = 0x400;
/// Bitstream data showed inconsistencies (wrong syntax).
pub const TRANSPORTDEC_PARSE_ERROR: TransportDecError = 0x401;
/// Unsupported format or feature found in the bitstream data.
pub const TRANSPORTDEC_UNSUPPORTED_FORMAT: TransportDecError = 0x402;
/// CRC error encountered in bitstream data.
pub const TRANSPORTDEC_CRC_ERROR: TransportDecError = 0x403;
pub const TPDEC_DECODE_ERROR_END: TransportDecError = 0x404;

// Fatal errors. Stop immediately on one of these errors!
pub const TPDEC_FATAL_ERROR_START: TransportDecError = 0x200;
/// An unknown error occured.
pub const TRANSPORTDEC_UNKOWN_ERROR: TransportDecError = 0x201;
/// An invalid parameter was passed to a function.
pub const TRANSPORTDEC_INVALID_PARAMETER: TransportDecError = 0x202;
/// The decoder needs to be restarted, since the requiered configuration change cannot be
/// performed.
pub const TRANSPORTDEC_NEED_TO_RESTART: TransportDecError = 0x203;
/// In case of packet based formats: Supplied number of bits exceed the size of the internal bit
/// buffer.
pub const TRANSPORTDEC_TOO_MANY_BITS: TransportDecError = 0x204;
pub const TPDEC_FATAL_ERROR_END: TransportDecError = 0x205;

/// `TPDEC_IS_DECODE_ERROR(err)`
#[inline]
pub fn tpdec_is_decode_error(err: TransportDecError) -> i32 {
    (err >= TPDEC_DECODE_ERROR_START && err <= TPDEC_DECODE_ERROR_END) as i32
}

/// `TPDEC_IS_FATAL_ERROR(err)`
#[inline]
pub fn tpdec_is_fatal_error(err: TransportDecError) -> i32 {
    (err >= TPDEC_FATAL_ERROR_START && err <= TPDEC_FATAL_ERROR_END) as i32
}

/// `TPDEC_PARAM`
pub type TpdecParam = i32;
/// Delay minimization strategy. 0: none, 1: discard as many frames as possible.
pub const TPDEC_PARAM_MINIMIZE_DELAY: TpdecParam = 1;
/// Enable early config discovery.
pub const TPDEC_PARAM_EARLY_CONFIG: TpdecParam = 2;
/// Ignore buffer fullness.
pub const TPDEC_PARAM_IGNORE_BUFFERFULLNESS: TpdecParam = 3;
/// Set average bit rate for bit stream interruption frame misses estimation.
pub const TPDEC_PARAM_SET_BITRATE: TpdecParam = 4;
/// Reset transport decoder instance status.
pub const TPDEC_PARAM_RESET: TpdecParam = 5;
/// Set data reception burst period in mili seconds.
pub const TPDEC_PARAM_BURST_PERIOD: TpdecParam = 6;
/// Set CICP target layout.
pub const TPDEC_PARAM_TARGETLAYOUT: TpdecParam = 7;
/// Force config change for next received config.
pub const TPDEC_PARAM_FORCE_CONFIG_CHANGE: TpdecParam = 8;
pub const TPDEC_PARAM_USE_ELEM_SKIPPING: TpdecParam = 9;

// CELP stuff
pub const MPE: i32 = 0;
pub const RPE: i32 = 1;
pub const FS8KHZ: i32 = 0;
pub const FS16KHZ: i32 = 1;

/// `TP_FLAG_MPEG4`: flag that can be passed to `transportDecOpen()`.
pub const TP_FLAG_MPEG4: u32 = 1;

// Capability flags
/// Flag indicating support for ADIF transport format.
pub const CAPF_TPDEC_ADIF: u32 = 0x00001000;
/// Flag indicating support for ADTS transport format.
pub const CAPF_TPDEC_ADTS: u32 = 0x00002000;
/// Flag indicating support for LOAS transport format.
pub const CAPF_TPDEC_LOAS: u32 = 0x00004000;
/// Flag indicating support for LATM transport format.
pub const CAPF_TPDEC_LATM: u32 = 0x00008000;
/// Flag indicating support for raw packets transport format.
pub const CAPF_TPDEC_RAWPACKETS: u32 = 0x00010000;

// ---------------------------------------------------------------------------------------------
// tp_version.h
// ---------------------------------------------------------------------------------------------

/// `TP_LIB_VL0`
pub const TP_LIB_VL0: i32 = 3;
/// `TP_LIB_VL1`
pub const TP_LIB_VL1: i32 = 0;
/// `TP_LIB_VL2`
pub const TP_LIB_VL2: i32 = 0;
/// `TP_LIB_TITLE`
pub const TP_LIB_TITLE: &str = "MPEG Transport";
/// `TP_LIB_BUILD_DATE`: `__DATE__` in C, which the port cannot reproduce.
pub const TP_LIB_BUILD_DATE: &str = "";
/// `TP_LIB_BUILD_TIME`: `__TIME__` in C, see [`TP_LIB_BUILD_DATE`].
pub const TP_LIB_BUILD_TIME: &str = "";

// ---------------------------------------------------------------------------------------------
// struct TRANSPORTDEC
// ---------------------------------------------------------------------------------------------

/// Size of the internal bitstream buffer (`Ram_TransportDecoderBuffer`, `8192 * 4`).
pub const TRANSPORTDEC_BUFFER_SIZE: usize = 8192 * 4;

/// `transportdec_parser_t`: format specific parser structs (a union in C, see the module
/// documentation).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransportDecParser {
    pub adts: StructAdts,
    pub adif: CAdifHeader,
    pub latm: CLatmDemux,
    pub drm: StructDrm,
}

// Flag bitmasks for "flags" member of struct TRANSPORTDEC
pub const TPDEC_SYNCOK: u32 = 1;
pub const TPDEC_MINIMIZE_DELAY: u32 = 2;
pub const TPDEC_IGNORE_BUFFERFULLNESS: u32 = 4;
pub const TPDEC_EARLY_CONFIG: u32 = 8;
pub const TPDEC_LOST_FRAMES_PENDING: u32 = 16;
pub const TPDEC_CONFIG_FOUND: u32 = 32;
pub const TPDEC_USE_ELEM_SKIPPING: u32 = 64;

// force config/content change
pub const TPDEC_FORCE_CONFIG_CHANGE: u8 = 1;
pub const TPDEC_FORCE_CONTENT_CHANGE: u8 = 2;

// skip packet
pub const TPDEC_SKIP_PACKET: i32 = 1;

/// How many bits to advance for synchronization search.
const TPDEC_SYNCSKIP: i32 = 8;

/// `struct TRANSPORTDEC` (`HANDLE_TRANSPORTDEC` is `&mut TransportDec`).
pub struct TransportDec {
    /// MPEG4 transportDec type.
    pub transport_fmt: TransportType,

    /// Struct holding callback and its data (used by the functions without `_cb` suffix).
    pub callbacks: CSTpCallBacks,

    /// Bitstream reader
    pub bit_stream: [FdkBitStream; 1],
    /// Internal bitstream data buffer (`bsBuffer`) of TRANSPORTDEC_BUFFER_SIZE bytes, the memory
    /// of `bit_stream[0]`.
    bs_buffer: BitMem,

    /// Format specific parser structs.
    pub parser: TransportDecParser,

    /// Audio specific config from the last config found. One additional CSAudioSpecificConfig
    /// is used temporarily for parsing.
    pub asc: [CSAudioSpecificConfig; 2],
    /// Controls config change
    pub ctrl_cfg_change: [CCtrlCFGChange; 1],

    /// Global transport frame reference bit position.
    pub global_frame_pos: u32,
    /// Current access unit start bit position.
    pub access_unit_anchor: [u32; 1],
    /// Length of current access unit.
    pub au_length: [i32; 1],
    /// Current number of raw data blocks contained remaining from the current transport frame.
    pub number_of_raw_data_blocks: i32,
    /// Average bit rate used for frame loss estimation.
    pub avg_bit_rate: u32,
    /// Last valid buffer fullness value for frame loss estimation
    pub last_valid_buffer_fullness: u32,
    /// Reminder in division during lost access unit estimation.
    pub remainder: i32,
    /// Estimated missing access units.
    pub missing_access_units: i32,
    /// Data burst period in mili seconds.
    pub burst_period: u32,
    /// Amount of frames that were already hold off due to buffer fullness condition not being
    /// met.
    pub hold_off_frames: u32,
    /// Flags.
    pub flags: u32,
    /// CICP target layout.
    pub target_layout: i32,
    /// Reference and start position (bits) and length (bytes) of loudnessInfoSet within
    /// rsv603daConfig (`pLoudnessInfoSetPosition`). Only stored, never dereferenced.
    pub p_loudness_info_set_position: *mut u32,
}

// SAFETY: `p_loudness_info_set_position` is only stored and compared, never dereferenced; the
// callback user data pointers are only passed to the callbacks (see `CSTpCallBacks`).
unsafe impl Send for TransportDec {}

impl TransportDec {
    /// The internal bitstream buffer (`bsBuffer`).
    pub fn bs_buffer(&self) -> &BitMem {
        &self.bs_buffer
    }
}

/// `HANDLE_TRANSPORTDEC`
pub type HandleTransportDec<'a> = &'a mut TransportDec;

/// Index of a per-layer array of one entry (see the module documentation).
#[inline]
fn l1(_layer: u32) -> usize {
    0
}

/// Index into `asc[2]` for a layer (layers 0 and 1 are valid C memory).
#[inline]
fn la(layer: u32) -> usize {
    if layer < 2 {
        layer as usize
    } else {
        0
    }
}

/// `cbUpdateConfig(data, &hTp->asc[i], configMode, &hTp->asc[i].AacConfigChanged)`: the
/// changed flag points into the ASC; the port passes a copy (see
/// [`TpDecCallbacks::update_config`]).
fn update_config_in_asc(cb: &mut dyn TpDecCallbacks, asc: &mut CSAudioSpecificConfig) -> i32 {
    let mut cc = asc.aac_config_changed;
    let mode = asc.config_mode;
    let r = cb.update_config(asc, mode, &mut cc);
    asc.aac_config_changed = cc;
    r
}

// ---------------------------------------------------------------------------------------------
// Open, configuration, callbacks
// ---------------------------------------------------------------------------------------------

/// `transportDec_Open(transportFmt, flags, nrOfLayers)`: opens a transport decoder. Returns
/// `None` for unsupported formats and more than one layer.
pub fn transport_dec_open(
    transport_fmt: TransportType,
    flags: u32,
    nr_of_layers: u32,
) -> Option<Box<TransportDec>> {
    // GetRam_TransportDecoder(): calloc()
    let mut h_input = Box::new(TransportDec {
        transport_fmt,
        callbacks: CSTpCallBacks::default(),
        bit_stream: [FdkBitStream::default(); 1],
        bs_buffer: BitMem::default(),
        parser: TransportDecParser::default(),
        asc: [CSAudioSpecificConfig::zeroed(); 2],
        ctrl_cfg_change: [CCtrlCFGChange::default(); 1],
        global_frame_pos: 0,
        access_unit_anchor: [0; 1],
        au_length: [0; 1],
        number_of_raw_data_blocks: 0,
        avg_bit_rate: 0,
        last_valid_buffer_fullness: 0,
        remainder: 0,
        missing_access_units: 0,
        burst_period: 0,
        hold_off_frames: 0,
        flags: 0,
        target_layout: 0,
        p_loudness_info_set_position: core::ptr::null_mut(),
    });

    // Init transportDec struct.
    match transport_fmt {
        TT_MP4_ADIF => {}

        TT_MP4_ADTS => {
            if flags & TP_FLAG_MPEG4 != 0 {
                h_input.parser.adts.decoder_can_do_mpeg4 = 1;
            } else {
                h_input.parser.adts.decoder_can_do_mpeg4 = 0;
            }
            adts_read_crc_init(&mut h_input.parser.adts);
            h_input.parser.adts.buffer_fullnes_start_flag = 1;
            h_input.number_of_raw_data_blocks = 0;
        }

        TT_DRM => {
            drm_read_crc_init(&mut h_input.parser.drm);
        }

        TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 | TT_MP4_LOAS => {
            h_input.parser.latm.usac_explicit_cfg_changed = 0;
            h_input.parser.latm.apply_asc = 1;
        }
        TT_MP4_RAW => {}

        _ => return None,
    }

    // Create bitstream
    h_input.bs_buffer = BitMem::new(TRANSPORTDEC_BUFFER_SIZE);
    if nr_of_layers > 1 {
        return None;
    }
    for i in 0..nr_of_layers as usize {
        fdk_init_bit_stream(
            &mut h_input.bit_stream[i],
            &h_input.bs_buffer,
            TRANSPORTDEC_BUFFER_SIZE as u32,
            0,
            BS_READER,
        );
    }
    h_input.burst_period = 0;

    Some(h_input)
}

/// `transportDec_OutOfBandConfig(hTp, conf, length, layer)` with the registered callbacks.
pub fn transport_dec_out_of_band_config(
    h_tp: &mut TransportDec,
    conf: &[u8],
    length: u32,
    layer: u32,
) -> TransportDecError {
    let mut cb = h_tp.callbacks;
    transport_dec_out_of_band_config_cb(h_tp, conf, length, layer, &mut cb)
}

/// `transportDec_OutOfBandConfig(hTp, conf, length, layer)`: configures the transport decoder
/// with an out-of-band configuration (an AudioSpecificConfig, a StreamMuxConfig for LATM/LOAS,
/// or a DRM SDC audio config).
///
/// `conf` holds the configuration; C copies `length` bytes of it (bytes beyond the slice are
/// read as 0).
pub fn transport_dec_out_of_band_config_cb(
    h_tp: &mut TransportDec,
    conf: &[u8],
    length: u32,
    layer: u32,
    cb: &mut dyn TpDecCallbacks,
) -> TransportDecError {
    let mut err = TRANSPORTDEC_OK;

    let mut bs = FdkBitStream::default();

    let mut f_config_found = 0;

    let mut config_changed: u8 = 0;
    let mut config_mode: u8 = AC_CM_DET_CFG_CHANGE;

    let mut tmp_conf = [0u8; 1024];
    if length > 1024 {
        return TRANSPORTDEC_UNSUPPORTED_FORMAT;
    }
    let n = (length as usize).min(conf.len());
    tmp_conf[..n].copy_from_slice(&conf[..n]);
    fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&tmp_conf), 1024, length << 3, BS_READER);
    let hbs = &mut bs;
    let li = la(layer);

    for i in 0..2 {
        if i > 0 {
            let n = (length as i32 * 8).wrapping_sub(fdk_get_valid_bits(hbs) as i32);
            fdk_push_back(hbs, n as u32);
            config_mode = AC_CM_ALLOC_MEM;
        }

        // config transport decoder
        match h_tp.transport_fmt {
            TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 | TT_MP4_LOAS => {
                if layer != 0 {
                    return TRANSPORTDEC_INVALID_PARAMETER;
                }
                err = clatm_demux_read_stream_mux_config(
                    hbs,
                    &mut h_tp.parser.latm,
                    cb,
                    &mut h_tp.asc,
                    &mut f_config_found,
                    config_mode,
                    config_changed,
                );
                if err != TRANSPORTDEC_OK {
                    return err;
                }
            }
            TT_DRM => {
                f_config_found = 1;
                err = drm_raw_sdc_audio_config_parse(
                    &mut h_tp.asc[li],
                    hbs,
                    Some(&mut *cb),
                    config_mode,
                    config_changed,
                );
                if err == TRANSPORTDEC_OK {
                    let err_c = update_config_in_asc(cb, &mut h_tp.asc[li]);
                    if err_c != 0 {
                        err = TRANSPORTDEC_PARSE_ERROR;
                    }
                }
            }
            _ => {
                f_config_found = 1;
                err = audio_specific_config_parse(
                    &mut h_tp.asc[1],
                    hbs,
                    1,
                    cb,
                    config_mode,
                    config_changed,
                    AOT_NULL_OBJECT,
                );
                if err == TRANSPORTDEC_OK {
                    h_tp.asc[li] = h_tp.asc[1];
                    let err_c = update_config_in_asc(cb, &mut h_tp.asc[li]);
                    if err_c != 0 {
                        err = TRANSPORTDEC_PARSE_ERROR;
                    }
                }
            }
        }

        if err == TRANSPORTDEC_OK
            && i == 0
            && (h_tp.asc[li].aac_config_changed != 0
                || h_tp.asc[li].sbr_config_changed != 0
                || h_tp.asc[li].sac_config_changed != 0)
        {
            config_changed = 1;
            let err_c = cb.free_mem(&h_tp.asc[li]);
            if err_c != 0 {
                err = TRANSPORTDEC_PARSE_ERROR;
            }
        }

        // if an error is detected terminate config parsing to avoid that an invalid config is
        // accepted in the second pass
        if err != TRANSPORTDEC_OK {
            break;
        }
    }

    if err == TRANSPORTDEC_OK && f_config_found != 0 {
        h_tp.flags |= TPDEC_CONFIG_FOUND;
    }

    err
}

/// `transportDec_InBandConfig(hTp, newConfig, newConfigLength, buildUpStatus, configChanged,
/// layer, implicitExplicitCfgDiff)` with the registered callbacks.
#[allow(clippy::too_many_arguments)]
pub fn transport_dec_in_band_config(
    h_tp: &mut TransportDec,
    new_config: &[u8],
    new_config_length: u32,
    build_up_status: u8,
    config_changed: &mut u8,
    layer: u32,
    implicit_explicit_cfg_diff: &mut u8,
) -> TransportDecError {
    let mut cb = h_tp.callbacks;
    transport_dec_in_band_config_cb(
        h_tp,
        new_config,
        new_config_length,
        build_up_status,
        config_changed,
        layer,
        implicit_explicit_cfg_diff,
        &mut cb,
    )
}

/// `transportDec_InBandConfig(hTp, newConfig, newConfigLength, buildUpStatus, configChanged,
/// layer, implicitExplicitCfgDiff)`: applies an in-band (AudioPreRoll) USAC configuration.
///
/// The port reads the configuration from a copy of up to `TP_USAC_MAX_CONFIG_LEN` bytes of
/// `new_config` (bytes beyond the slice are read as 0). Lengths above
/// `TP_USAC_MAX_CONFIG_LEN` are stored truncated.
#[allow(clippy::too_many_arguments)]
pub fn transport_dec_in_band_config_cb(
    h_tp: &mut TransportDec,
    new_config: &[u8],
    new_config_length: u32,
    _build_up_status: u8,
    config_changed: &mut u8,
    layer: u32,
    implicit_explicit_cfg_diff: &mut u8,
    cb: &mut dyn TpDecCallbacks,
) -> TransportDecError {
    let mut bs = FdkBitStream::default();
    let mut err = TRANSPORTDEC_OK;
    let mut f_config_found = 0;
    let mut config_mode: u8;
    *implicit_explicit_cfg_diff = 0;
    let lc = l1(layer);
    let li = la(layer);

    fdk_aac_rs_fdk::fdk_assert!(h_tp.asc[0].m_aot == AOT_USAC);

    let mut cfg = [0u8; TP_USAC_MAX_CONFIG_LEN];
    let n = new_config.len().min(TP_USAC_MAX_CONFIG_LEN);
    cfg[..n].copy_from_slice(&new_config[..n]);
    fdk_init_bit_stream(
        &mut bs,
        &BitMem::from_slice(&cfg),
        TP_USAC_MAX_CONFIG_LEN as u32,
        new_config_length << 3,
        BS_READER,
    );
    let hbs = &mut bs;
    // The bytes of newConfig that C compares and copies.
    let ncl = (new_config_length as usize).min(TP_USAC_MAX_CONFIG_LEN);

    'bail: {
        if h_tp.ctrl_cfg_change[lc].flush_status == TPDEC_FLUSH_OFF
            && h_tp.ctrl_cfg_change[lc].build_up_status != TPDEC_RSV60_BUILD_UP_IDLE_IN_BAND
            && h_tp.asc[0].m_aot == AOT_USAC
        {
            let usac = &h_tp.asc[0].m_sc.m_usac_config;
            if ((usac.usac_config_bits as u32 + 7) >> 3) == new_config_length {
                if cfg[..ncl] == usac.usac_config[..ncl] {
                    if h_tp.parser.latm.usac_explicit_cfg_changed != 0 {
                        // configChange from LOAS/LATM parser
                        h_tp.parser.latm.usac_explicit_cfg_changed = 0;
                        h_tp.ctrl_cfg_change[lc].flush_cnt = 0;
                        h_tp.ctrl_cfg_change[lc].flush_status = TPDEC_USAC_DASH_IPF_FLUSH_ON;
                        h_tp.ctrl_cfg_change[lc].build_up_cnt = 0;
                        h_tp.ctrl_cfg_change[lc].build_up_status = TPDEC_BUILD_UP_OFF;
                    } else {
                        *config_changed = 0;
                        return err;
                    }
                } else {
                    *implicit_explicit_cfg_diff = 1;
                }
            } else {
                *implicit_explicit_cfg_diff = 1;
            }
            // ISO/IEC 23003-3:2012/FDAM 3:2016(E) Annex F.2: explicit and implicit config shall
            // be identical.
            if *implicit_explicit_cfg_diff != 0 {
                match h_tp.transport_fmt {
                    TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 | TT_MP4_LOAS => {
                        // reset decoder to initial state to achieve definite behavior after error
                        // in config
                        cb.free_mem(&h_tp.asc[li]);
                        h_tp.parser.latm.usac_explicit_cfg_changed = 0;
                        h_tp.parser.latm.apply_asc = 1;
                        err = TRANSPORTDEC_PARSE_ERROR;
                        break 'bail;
                    }
                    _ => {}
                }
            }
        }

        if h_tp.ctrl_cfg_change[lc].flush_status == TPDEC_FLUSH_OFF
            && h_tp.ctrl_cfg_change[lc].build_up_status != TPDEC_RSV60_BUILD_UP_IDLE_IN_BAND
        {
            h_tp.ctrl_cfg_change[lc].flush_cnt = 0;
            h_tp.ctrl_cfg_change[lc].build_up_cnt = 0;
            h_tp.ctrl_cfg_change[lc].build_up_status = TPDEC_BUILD_UP_OFF;
            if h_tp.asc[0].m_aot == AOT_USAC {
                h_tp.ctrl_cfg_change[lc].flush_status = TPDEC_USAC_DASH_IPF_FLUSH_ON;
            }
        }

        if h_tp.ctrl_cfg_change[lc].flush_status == TPDEC_RSV60_DASH_IPF_ATSC_FLUSH_ON
            || h_tp.ctrl_cfg_change[lc].flush_status == TPDEC_USAC_DASH_IPF_FLUSH_ON
        {
            let mut counter: i8 = 0;
            if h_tp.asc[0].m_aot == AOT_USAC {
                counter = TPDEC_USAC_NUM_CONFIG_CHANGE_FRAMES;
            }
            if h_tp.ctrl_cfg_change[lc].flush_cnt >= counter {
                h_tp.ctrl_cfg_change[lc].flush_cnt = 0;
                h_tp.ctrl_cfg_change[lc].flush_status = TPDEC_FLUSH_OFF;
                h_tp.ctrl_cfg_change[lc].force_cfg_change = 0;
                if h_tp.asc[0].m_aot == AOT_USAC {
                    h_tp.ctrl_cfg_change[lc].build_up_cnt = TPDEC_USAC_NUM_CONFIG_CHANGE_FRAMES - 1;
                    h_tp.ctrl_cfg_change[lc].build_up_status = TPDEC_USAC_BUILD_UP_ON;
                }
            }

            // Activate flush mode. After that continue with build up mode in core
            if cb.ctrl_cfg_change(&h_tp.ctrl_cfg_change[lc]) != 0 {
                err = TRANSPORTDEC_PARSE_ERROR;
            }

            if h_tp.ctrl_cfg_change[lc].flush_status == TPDEC_RSV60_DASH_IPF_ATSC_FLUSH_ON
                || h_tp.ctrl_cfg_change[lc].flush_status == TPDEC_USAC_DASH_IPF_FLUSH_ON
            {
                h_tp.ctrl_cfg_change[lc].flush_cnt =
                    h_tp.ctrl_cfg_change[lc].flush_cnt.wrapping_add(1);
                return err;
            }
        }

        if h_tp.asc[0].m_aot == AOT_USAC {
            f_config_found = 1;

            if err == TRANSPORTDEC_OK {
                *config_changed = 0;
                config_mode = AC_CM_DET_CFG_CHANGE;

                for i in 0..2 {
                    if i > 0 {
                        let n = (new_config_length as i32)
                            .wrapping_mul(8)
                            .wrapping_sub(fdk_get_valid_bits(hbs) as i32);
                        fdk_push_back(hbs, n as u32);
                        config_mode = AC_CM_ALLOC_MEM;
                    }
                    // config transport decoder
                    let aot = h_tp.asc[li].m_aot;
                    err = audio_specific_config_parse(
                        &mut h_tp.asc[1],
                        hbs,
                        0,
                        cb,
                        config_mode,
                        *config_changed,
                        aot,
                    );
                    if err == TRANSPORTDEC_OK {
                        h_tp.asc[li] = h_tp.asc[1];
                        let err_c = update_config_in_asc(cb, &mut h_tp.asc[li]);
                        if err_c != 0 {
                            err = TRANSPORTDEC_PARSE_ERROR;
                        }
                    }

                    if err == TRANSPORTDEC_OK
                        && i == 0
                        && (h_tp.asc[li].aac_config_changed != 0
                            || h_tp.asc[li].sbr_config_changed != 0
                            || h_tp.asc[li].sac_config_changed != 0)
                    {
                        *config_changed = 1;
                        let err_c = cb.free_mem(&h_tp.asc[li]);
                        if err_c != 0 {
                            err = TRANSPORTDEC_PARSE_ERROR;
                        }
                    }

                    // if an error is detected terminate config parsing to avoid that an invalid
                    // config is accepted in the second pass
                    if err != TRANSPORTDEC_OK {
                        break;
                    }
                }
            }
        }
    }

    // bail:
    // save new config
    if err == TRANSPORTDEC_OK {
        if h_tp.asc[0].m_aot == AOT_USAC {
            let usac = &mut h_tp.asc[0].m_sc.m_usac_config;
            usac.usac_config_bits = (new_config_length << 3) as u16;
            usac.usac_config[..ncl].copy_from_slice(&cfg[..ncl]);
            // in case of USAC reset transportDecoder variables here because otherwise without
            // IPF they are not reset
            h_tp.ctrl_cfg_change[lc].flush_cnt = 0;
            h_tp.ctrl_cfg_change[lc].flush_status = TPDEC_FLUSH_OFF;
            h_tp.ctrl_cfg_change[lc].build_up_cnt = 0;
            h_tp.ctrl_cfg_change[lc].build_up_status = TPDEC_BUILD_UP_OFF;
        }
    } else {
        h_tp.number_of_raw_data_blocks = 0;

        // If parsing error while config found, clear ctrlCFGChange-struct
        let c = &mut h_tp.ctrl_cfg_change[lc];
        c.flush_cnt = 0;
        c.flush_status = TPDEC_FLUSH_OFF;
        c.build_up_cnt = 0;
        c.build_up_status = TPDEC_BUILD_UP_OFF;
        c.cfg_changed = 0;
        c.content_changed = 0;
        c.force_cfg_change = 0;

        cb.ctrl_cfg_change(&h_tp.ctrl_cfg_change[lc]);
    }

    if err == TRANSPORTDEC_OK && f_config_found != 0 {
        h_tp.flags |= TPDEC_CONFIG_FOUND;
    }

    err
}

/// `transportDec_RegisterAscCallback(hTp, cbUpdateConfig, user_data)`
pub fn transport_dec_register_asc_callback(
    h_tp_dec: &mut TransportDec,
    cb_update_config: Option<CbUpdateConfig>,
    user_data: *mut c_void,
) -> i32 {
    h_tp_dec.callbacks.cb_update_config = cb_update_config;
    h_tp_dec.callbacks.cb_update_config_data = user_data;
    0
}

/// `transportDec_RegisterFreeMemCallback(hTp, cbFreeMem, user_data)`
pub fn transport_dec_register_free_mem_callback(
    h_tp_dec: &mut TransportDec,
    cb_free_mem: Option<CbFreeMem>,
    user_data: *mut c_void,
) -> i32 {
    h_tp_dec.callbacks.cb_free_mem = cb_free_mem;
    h_tp_dec.callbacks.cb_free_mem_data = user_data;
    0
}

/// `transportDec_RegisterCtrlCFGChangeCallback(hTp, cbCtrlCFGChange, user_data)`
pub fn transport_dec_register_ctrl_cfg_change_callback(
    h_tp_dec: &mut TransportDec,
    cb_ctrl_cfg_change: Option<CbCtrlCfgChange>,
    user_data: *mut c_void,
) -> i32 {
    h_tp_dec.callbacks.cb_ctrl_cfg_change = cb_ctrl_cfg_change;
    h_tp_dec.callbacks.cb_ctrl_cfg_change_data = user_data;
    0
}

/// `transportDec_RegisterSscCallback(hTp, cbSsc, user_data)`
pub fn transport_dec_register_ssc_callback(
    h_tp_dec: &mut TransportDec,
    cb_ssc: Option<CbSsc>,
    user_data: *mut c_void,
) -> i32 {
    h_tp_dec.callbacks.cb_ssc = cb_ssc;
    h_tp_dec.callbacks.cb_ssc_data = user_data;
    0
}

/// `transportDec_RegisterSbrCallback(hTp, cbSbr, user_data)`
pub fn transport_dec_register_sbr_callback(
    h_tp_dec: &mut TransportDec,
    cb_sbr: Option<CbSbr>,
    user_data: *mut c_void,
) -> i32 {
    h_tp_dec.callbacks.cb_sbr = cb_sbr;
    h_tp_dec.callbacks.cb_sbr_data = user_data;
    0
}

/// `transportDec_RegisterUsacCallback(hTp, cbUsac, user_data)`
pub fn transport_dec_register_usac_callback(
    h_tp_dec: &mut TransportDec,
    cb_usac: Option<CbUsac>,
    user_data: *mut c_void,
) -> i32 {
    h_tp_dec.callbacks.cb_usac = cb_usac;
    h_tp_dec.callbacks.cb_usac_data = user_data;
    0
}

/// `transportDec_RegisterUniDrcConfigCallback(hTp, cbUniDrc, user_data,
/// pLoudnessInfoSetPosition)`
pub fn transport_dec_register_uni_drc_config_callback(
    h_tp_dec: &mut TransportDec,
    cb_uni_drc: Option<CbUniDrc>,
    user_data: *mut c_void,
    p_loudness_info_set_position: *mut u32,
) -> i32 {
    h_tp_dec.callbacks.cb_uni_drc = cb_uni_drc;
    h_tp_dec.callbacks.cb_uni_drc_data = user_data;

    h_tp_dec.p_loudness_info_set_position = p_loudness_info_set_position;
    0
}

// ---------------------------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------------------------

/// `transportDec_FillData(hTp, pBuffer, bufferSize, pBytesValid, layer)`: feeds the last
/// `*p_bytes_valid` bytes of the first `buffer_size` bytes of `p_buffer` into the internal
/// buffer.
///
/// For a negative `layer` the port returns `TRANSPORTDEC_INVALID_PARAMETER`, as for layers of 1
/// or more.
pub fn transport_dec_fill_data(
    h_tp: &mut TransportDec,
    p_buffer: &[u8],
    buffer_size: u32,
    p_bytes_valid: &mut u32,
    layer: i32,
) -> TransportDecError {
    if layer >= 1 || layer < 0 {
        return TRANSPORTDEC_INVALID_PARAMETER;
    }
    let l = layer as usize;

    // set bitbuffer shortcut
    if tt_is_packet(h_tp.transport_fmt) {
        if h_tp.number_of_raw_data_blocks == 0 {
            let h_bs = &mut h_tp.bit_stream[l];
            fdk_reset_bitbuffer(h_bs, BS_READER);
            fdk_feed_buffer(h_bs, p_buffer, buffer_size, p_bytes_valid);
            if *p_bytes_valid != 0 {
                return TRANSPORTDEC_TOO_MANY_BITS;
            }
        }
    } else {
        // ... else feed bitbuffer with new stream data (append).

        if *p_bytes_valid == 0 {
            // nothing to do
            return TRANSPORTDEC_OK;
        } else {
            let bytes_valid = *p_bytes_valid as i32;
            fdk_feed_buffer(&mut h_tp.bit_stream[l], p_buffer, buffer_size, p_bytes_valid);

            if h_tp.number_of_raw_data_blocks > 0 {
                // (int - UINT) * 8 in UINT arithmetic
                h_tp.global_frame_pos = h_tp.global_frame_pos.wrapping_add(
                    (bytes_valid as u32).wrapping_sub(*p_bytes_valid).wrapping_mul(8),
                );
                h_tp.access_unit_anchor[l] = fdk_get_valid_bits(&mut h_tp.bit_stream[l]);
            }
        }
    }

    TRANSPORTDEC_OK
}

/// `transportDec_GetBitstream(hTp, layer)`
pub fn transport_dec_get_bitstream(h_tp: &mut TransportDec, layer: u32) -> &mut FdkBitStream {
    &mut h_tp.bit_stream[l1(layer)]
}

/// `transportDec_GetFormat(hTp)`
pub fn transport_dec_get_format(h_tp: &TransportDec) -> TransportType {
    h_tp.transport_fmt
}

/// `transportDec_GetBufferFullness(hTp)`: the buffer fullness of the current frame in bits, -1
/// if unknown or variable.
pub fn transport_dec_get_buffer_fullness(h_tp: &TransportDec) -> i32 {
    let mut buffer_fullness = -1;

    match h_tp.transport_fmt {
        TT_MP4_ADTS => {
            let b = &h_tp.parser.adts.bs;
            if b.adts_fullness != 0x7ff {
                buffer_fullness = b.frame_length as i32 * 8
                    + b.adts_fullness as i32
                        * 32
                        * get_number_of_effective_channels(b.channel_config as i32);
            }
        }
        TT_MP4_LOAS | TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 => {
            if h_tp.parser.latm.m_linfo[0][0].m_buffer_fullness != 0xff {
                buffer_fullness = h_tp.parser.latm.m_linfo[0][0].m_buffer_fullness as i32;
            }
        }
        _ => {}
    }

    buffer_fullness
}

/// `transportDec_AdjustEndOfAccessUnit(hTp)`: adjusts the bitstream position at the end of an
/// access unit.
fn transport_dec_adjust_end_of_access_unit(h_tp: &mut TransportDec) -> TransportDecError {
    let mut err = TRANSPORTDEC_OK;

    match h_tp.transport_fmt {
        TT_MP4_ADIF => {
            // Do byte align at the end of raw_data_block() because UsacFrame() is not byte
            // aligned.
            fdk_byte_align(&mut h_tp.bit_stream[0], h_tp.access_unit_anchor[0]);
        }
        TT_MP4_LOAS | TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 => {
            if h_tp.number_of_raw_data_blocks == 0 {
                let h_bs = &mut h_tp.bit_stream[0];
                // Do byte align at the end of AudioMuxElement.
                fdk_byte_align(h_bs, h_tp.global_frame_pos);

                // Check global frame length
                if h_tp.transport_fmt == TT_MP4_LOAS
                    && h_tp.parser.latm.m_audio_mux_length_bytes > 0
                {
                    let loas_offset = ((h_tp.parser.latm.m_audio_mux_length_bytes as i32)
                        .wrapping_mul(8)
                        .wrapping_add(fdk_get_valid_bits(h_bs) as i32))
                    .wrapping_sub(h_tp.global_frame_pos as i32);
                    if loas_offset != 0 {
                        fdk_push_bi_directional(h_bs, loas_offset);
                        // For ELD and other payloads there is an unknown amount of padding, so
                        // ignore unread bits, but throw an error only if too many bits where
                        // read.
                        if loas_offset < 0 {
                            err = TRANSPORTDEC_PARSE_ERROR;
                        }
                    }
                }
            }
        }

        TT_MP4_ADTS => {
            let adts = &mut h_tp.parser.adts;
            let h_bs = &mut h_tp.bit_stream[0];
            if adts.bs.protection_absent == 0 {
                // Calculate offset to end of AU.
                //
                // The index is num_raw_blocks - numberOfRawDataBlocks, in -1..=3. Index -1 is
                // the USHORT before rawDataBlockDist in STRUCT_ADTS, crcReadValue, reproduced
                // from the C layout; other values give 0.
                let idx = adts.bs.num_raw_blocks as i32 - h_tp.number_of_raw_data_blocks;
                let dist = if idx == -1 {
                    adts.crc_read_value as i32
                } else {
                    match usize::try_from(idx).ok().and_then(|i| adts.raw_data_block_dist.get(i)) {
                        Some(&d) => d as i32,
                        None => 0,
                    }
                };
                let mut offset = dist << 3;
                // CAUTION: The PCE (if available) is declared to be a part of the header!
                offset = offset.wrapping_sub(
                    (h_tp.access_unit_anchor[0] as i32)
                        .wrapping_sub(fdk_get_valid_bits(h_bs) as i32)
                        .wrapping_add(16)
                        .wrapping_add(adts.bs.num_pce_bits as i32),
                );
                fdk_push_bi_directional(h_bs, offset);
            }
            if adts.bs.num_raw_blocks > 0 && adts.bs.protection_absent == 0 {
                // Note this CRC read currently happens twice because of transportDec_CrcCheck()
                adts.crc_read_value = fdk_read_bits(h_bs, 16) as u16;
            }
            if h_tp.number_of_raw_data_blocks == 0 {
                // Check global frame length
                if adts.bs.protection_absent == 0 {
                    let offset = ((adts.bs.frame_length as i32) * 8 - ADTS_SYNCLENGTH)
                        .wrapping_add(fdk_get_valid_bits(h_bs) as i32)
                        .wrapping_sub(h_tp.global_frame_pos as i32);
                    if offset != 0 {
                        fdk_push_bi_directional(h_bs, offset);
                    }
                }
            }
        }

        _ => {}
    }

    err
}

/// `additionalHoldOffNeeded(hTp, bufferFullness, bitsAvail)`: additional buffer fullness
/// constraint due to burst data reception (`TPDEC_PARAM_BURST_PERIOD`).
///
/// The signed products can overflow for large parameter values (undefined in C); the port wraps,
/// as the `mul`/`madd`/`imul` instructions of both reference builds do.
fn additional_hold_off_needed(
    h_tp: &TransportDec,
    mut buffer_fullness: i32,
    bits_avail: i32,
) -> TransportDecError {
    let samples_per_frame = h_tp.asc[0].m_samples_per_frame as i32;
    let sampling_frequency = h_tp.asc[0].m_sampling_frequency as i32;

    if h_tp.avg_bit_rate == 0 || h_tp.burst_period == 0 {
        return TRANSPORTDEC_OK;
    }
    if samples_per_frame == 0 || sampling_frequency == 0 {
        return TRANSPORTDEC_NOT_ENOUGH_BITS;
    }

    // One Master Frame is sent every hTp->burstPeriod ms
    // UINT * INT + INT in UINT arithmetic, stored as INT.
    let mut max_au = h_tp
        .burst_period
        .wrapping_mul(sampling_frequency as u32)
        .wrapping_add(samples_per_frame.wrapping_mul(1000).wrapping_sub(1) as u32)
        as i32;
    max_au = div_c(max_au, samples_per_frame.wrapping_mul(1000));
    // Subtract number of frames which were already held off.
    max_au = (max_au as u32).wrapping_sub(h_tp.hold_off_frames) as i32;

    let mut avg_bits_per_frame =
        h_tp.avg_bit_rate
            .wrapping_mul(samples_per_frame as u32)
            .wrapping_add(sampling_frequency.wrapping_sub(1) as u32) as i32;
    avg_bits_per_frame = div_c(avg_bits_per_frame, sampling_frequency);

    // Consider worst case of bufferFullness quantization.
    match h_tp.transport_fmt {
        TT_MP4_ADIF | TT_MP4_ADTS | TT_MP4_LOAS | TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 => {
            buffer_fullness = buffer_fullness.wrapping_add(31);
        }
        _ => {}
    }

    let check_length_bits =
        buffer_fullness.wrapping_add(max_au.wrapping_sub(1).wrapping_mul(avg_bits_per_frame));

    // Check if buffer is big enough to fullfill buffer fullness condition
    if check_length_bits > ((8192 * 4) << 3) - 7 {
        return TRANSPORTDEC_SYNC_ERROR;
    }

    if bits_avail < check_length_bits {
        TRANSPORTDEC_NOT_ENOUGH_BITS
    } else {
        TRANSPORTDEC_OK
    }
}

/// Signed `int` division as compiled by the reference builds: `sdiv` (AArch64) and `idiv`
/// (x86) agree except for a divisor of 0, for which the port uses the AArch64 result (0) on
/// every variant, and `INT_MIN / -1` (wraps).
#[inline]
fn div_c(a: i32, b: i32) -> i32 {
    if b == 0 {
        0
    } else {
        a.wrapping_div(b)
    }
}

/// `a % b` to match [`div_c`]: AArch64 computes `a - (a / b) * b` (`msub`), which is `a` for
/// `b == 0`.
#[inline]
fn rem_c(a: i32, b: i32) -> i32 {
    if b == 0 {
        a
    } else {
        a.wrapping_rem(b)
    }
}

/// `transportDec_readHeader(hTp, hBs, syncLength, ignoreBufferFullness, pRawDataBlockLength,
/// pfTraverseMoreFrames, pSyncLayerFrameBits, pfConfigFound, pHeaderBits)` on `bitStream[0]`.
#[allow(clippy::too_many_arguments)]
fn transport_dec_read_header(
    h_tp: &mut TransportDec,
    cb: &mut dyn TpDecCallbacks,
    sync_length: i32,
    ignore_buffer_fullness: i32,
    p_raw_data_block_length: &mut i32,
    pf_traverse_more_frames: Option<&mut i32>,
    p_sync_layer_frame_bits: Option<&mut i32>,
    pf_config_found: Option<&mut i32>,
    p_header_bits: Option<&mut i32>,
) -> TransportDecError {
    let mut err = TRANSPORTDEC_OK;
    let mut raw_data_block_length = *p_raw_data_block_length;
    let mut f_traverse_more_frames = match &pf_traverse_more_frames {
        Some(p) => **p,
        None => 0,
    };
    let mut sync_layer_frame_bits = match &p_sync_layer_frame_bits {
        Some(p) => **p,
        None => 0,
    };
    let mut f_config_found = match &pf_config_found {
        Some(p) => **p,
        None => 0,
    };

    let start_pos = fdk_get_valid_bits(&mut h_tp.bit_stream[0]) as i32;

    'bail: {
        match h_tp.transport_fmt {
            TT_MP4_ADTS => {
                if h_tp.number_of_raw_data_blocks <= 0 {
                    h_tp.global_frame_pos = fdk_get_valid_bits(&mut h_tp.bit_stream[0]);

                    let mut config_changed: u8 = 0;
                    let mut config_mode: u8 = AC_CM_DET_CFG_CHANGE;

                    for i in 0..2 {
                        if i > 0 {
                            let h_bs = &mut h_tp.bit_stream[0];
                            let n = (h_tp.global_frame_pos as i32)
                                .wrapping_sub(fdk_get_valid_bits(h_bs) as i32);
                            fdk_push_back(h_bs, n as u32);
                            config_mode = AC_CM_ALLOC_MEM;
                        }

                        // Parse ADTS header
                        err = adts_read_decode_header(
                            &mut h_tp.parser.adts,
                            &mut h_tp.asc[0],
                            &mut h_tp.bit_stream[0],
                            ignore_buffer_fullness,
                        );
                        if err != TRANSPORTDEC_OK {
                            if err != TRANSPORTDEC_NOT_ENOUGH_BITS {
                                err = TRANSPORTDEC_SYNC_ERROR;
                            }
                        } else {
                            let err_c =
                                cb.update_config(&h_tp.asc[0], config_mode, &mut config_changed);
                            if err_c != 0 {
                                if err_c == TRANSPORTDEC_NEED_TO_RESTART {
                                    err = TRANSPORTDEC_NEED_TO_RESTART;
                                    break 'bail;
                                } else {
                                    err = TRANSPORTDEC_SYNC_ERROR;
                                }
                            } else {
                                f_config_found = 1;
                                h_tp.number_of_raw_data_blocks =
                                    h_tp.parser.adts.bs.num_raw_blocks as i32 + 1;
                            }
                        }

                        if err == TRANSPORTDEC_OK && i == 0 && config_changed != 0 {
                            let err_c = cb.free_mem(&h_tp.asc[0]);
                            if err_c != 0 {
                                err = TRANSPORTDEC_PARSE_ERROR;
                            }
                        }
                        // if an error is detected terminate config parsing to avoid that an
                        // invalid config is accepted in the second pass
                        if err != TRANSPORTDEC_OK {
                            break;
                        }
                    }
                } else {
                    // Reset CRC because the next bits are the beginning of a raw_data_block()
                    fdk_crc_reset(&mut h_tp.parser.adts.crc_info);
                    h_tp.parser.adts.bs.num_pce_bits = 0;
                }
                if err == TRANSPORTDEC_OK {
                    h_tp.number_of_raw_data_blocks -= 1;
                    raw_data_block_length = adts_read_get_raw_data_block_length(
                        &h_tp.parser.adts,
                        h_tp.parser.adts.bs.num_raw_blocks as i32 - h_tp.number_of_raw_data_blocks,
                    );
                    if raw_data_block_length <= 0 {
                        // No further frame traversal possible.
                        f_traverse_more_frames = 0;
                    }
                    sync_layer_frame_bits = ((h_tp.parser.adts.bs.frame_length as i32) << 3)
                        .wrapping_sub(
                            start_pos
                                .wrapping_sub(fdk_get_valid_bits(&mut h_tp.bit_stream[0]) as i32),
                        )
                        .wrapping_sub(sync_length);
                    if sync_layer_frame_bits <= 0 {
                        err = TRANSPORTDEC_SYNC_ERROR;
                    }
                } else {
                    h_tp.number_of_raw_data_blocks = 0;
                }
            }
            TT_MP4_LOAS | TT_MP4_LATM_MCP1 | TT_MP4_LATM_MCP0 => {
                if h_tp.transport_fmt == TT_MP4_LOAS && h_tp.number_of_raw_data_blocks <= 0 {
                    sync_layer_frame_bits = fdk_read_bits(&mut h_tp.bit_stream[0], 13) as i32;
                    h_tp.parser.latm.m_audio_mux_length_bytes = sync_layer_frame_bits as u32;
                    sync_layer_frame_bits <<= 3;
                }
                // FDK_FALLTHROUGH
                if h_tp.number_of_raw_data_blocks <= 0 {
                    h_tp.global_frame_pos = fdk_get_valid_bits(&mut h_tp.bit_stream[0]);

                    err = clatm_demux_read(
                        &mut h_tp.bit_stream[0],
                        &mut h_tp.parser.latm,
                        h_tp.transport_fmt,
                        cb,
                        &mut h_tp.asc,
                        &mut f_config_found,
                        ignore_buffer_fullness,
                    );

                    if err != TRANSPORTDEC_OK {
                        if err != TRANSPORTDEC_NOT_ENOUGH_BITS && tpdec_is_fatal_error(err) == 0 {
                            err = TRANSPORTDEC_SYNC_ERROR;
                        }
                    } else {
                        h_tp.number_of_raw_data_blocks =
                            clatm_demux_get_nr_of_sub_frames(&h_tp.parser.latm) as i32;
                        if h_tp.transport_fmt == TT_MP4_LOAS {
                            sync_layer_frame_bits =
                                sync_layer_frame_bits.wrapping_sub(
                                    start_pos
                                        .wrapping_sub(
                                            fdk_get_valid_bits(&mut h_tp.bit_stream[0]) as i32
                                        )
                                        .wrapping_sub(13),
                                );
                            if sync_layer_frame_bits <= 0 {
                                err = TRANSPORTDEC_SYNC_ERROR;
                            }
                        }
                    }
                } else {
                    err = clatm_demux_read_payload_length_info(
                        &mut h_tp.bit_stream[0],
                        &mut h_tp.parser.latm,
                    );
                    if err != TRANSPORTDEC_OK {
                        err = TRANSPORTDEC_SYNC_ERROR;
                    }
                }
                if err == TRANSPORTDEC_OK {
                    raw_data_block_length = 0;
                    let n = clatm_demux_get_nr_of_layers(&h_tp.parser.latm, 0) as i32;
                    for layer in 0..n {
                        raw_data_block_length = raw_data_block_length.wrapping_add(
                            clatm_demux_get_frame_length_in_bits(&h_tp.parser.latm, 0, layer as u32)
                                as i32,
                        );
                    }
                    h_tp.number_of_raw_data_blocks -= 1;
                } else {
                    h_tp.number_of_raw_data_blocks = 0;
                }
            }
            _ => {
                sync_layer_frame_bits = 0;
            }
        }
    }

    // bail:

    *p_raw_data_block_length = raw_data_block_length;

    if let Some(p) = p_header_bits {
        *p = p
            .wrapping_add(start_pos)
            .wrapping_sub(fdk_get_valid_bits(&mut h_tp.bit_stream[0]) as i32);
    }

    for i in 0..1 {
        // If parsing error while config found, clear ctrlCFGChange-struct
        if h_tp.ctrl_cfg_change[i].cfg_changed != 0 && err != TRANSPORTDEC_OK {
            h_tp.number_of_raw_data_blocks = 0;
            let c = &mut h_tp.ctrl_cfg_change[i];
            c.flush_cnt = 0;
            c.flush_status = TPDEC_FLUSH_OFF;
            c.build_up_cnt = 0;
            c.build_up_status = TPDEC_BUILD_UP_OFF;
            c.cfg_changed = 0;
            c.content_changed = 0;
            c.force_cfg_change = 0;

            cb.ctrl_cfg_change(&h_tp.ctrl_cfg_change[i]);
        }
    }

    if let Some(p) = pf_config_found {
        *p = f_config_found;
    }

    if let Some(p) = pf_traverse_more_frames {
        *p = f_traverse_more_frames;
    }
    if let Some(p) = p_sync_layer_frame_bits {
        *p = sync_layer_frame_bits;
    }

    err
}

thread_local! {
    static UNALIGNED_SYNC_SEARCHES: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// The sync word searches of `synchronization` on this thread that started at a bit position
/// that is not a multiple of `TPDEC_SYNCSKIP` (`FDK_ASSERT((bitsAvail % TPDEC_SYNCSKIP) == 0)`
/// violated). The reference continues without the assertion (`NDEBUG`) and so does the port:
/// the search steps by `TPDEC_SYNCSKIP` bits from that position. Counted in every profile.
pub fn synchronization_unaligned_searches() -> u64 {
    UNALIGNED_SYNC_SEARCHES.with(|c| c.get())
}

/// `synchronization(hTp, pHeaderBits)`: searches the next sync word (ADTS, LOAS), parses the
/// transport header and checks that the frame is complete in the buffer.
fn synchronization(
    h_tp: &mut TransportDec,
    cb: &mut dyn TpDecCallbacks,
    p_header_bits: Option<&mut i32>,
) -> TransportDecError {
    let mut err = TRANSPORTDEC_OK;
    let mut err_first_frame = TRANSPORTDEC_OK;

    let mut sync_layer_frame_bits: i32 = 0; // Length of sync layer frame (i.e. LOAS)
    let mut raw_data_block_length: i32 = 0;
    let mut raw_data_block_length_previous: i32;
    let mut header_bits: i32 = 0;
    let mut header_bits_first_frame: i32 = 0;
    let mut header_bits_previous: i32;
    let mut num_frames_traversed: i32 = 0;
    let mut f_traverse_more_frames: i32;
    let mut f_config_found: i32 = (h_tp.flags & TPDEC_CONFIG_FOUND) as i32;
    let mut start_pos_first_frame: i32 = -1;
    let mut num_raw_data_blocks_first_frame: i32 = 0;
    let mut num_raw_data_blocks_previous: i32;
    let mut global_frame_pos_first_frame: i32 = 0;
    let mut raw_data_block_length_first_frame: i32 = 0;
    let mut ignore_buffer_fullness: i32 = (h_tp.flags
        & (TPDEC_LOST_FRAMES_PENDING | TPDEC_IGNORE_BUFFERFULLNESS | TPDEC_SYNCOK))
        as i32;
    let mut end_tp_frame_bits_previous: u32 = 0;

    // C_ALLOC_SCRATCH_START(contextFirstFrame, transportdec_parser_t, 1)
    let mut context_first_frame = TransportDecParser::default();

    let total_bits = fdk_get_valid_bits(&mut h_tp.bit_stream[0]) as i32;

    'bail: {
        if total_bits <= 0 {
            err = TRANSPORTDEC_NOT_ENOUGH_BITS;
            break 'bail;
        }

        f_traverse_more_frames = ((h_tp.flags & (TPDEC_MINIMIZE_DELAY | TPDEC_EARLY_CONFIG) != 0)
            && (h_tp.flags & TPDEC_SYNCOK == 0)) as i32;

        // Set transport specific sync parameters
        let (sync_word, sync_length): (u32, i32) = match h_tp.transport_fmt {
            TT_MP4_ADTS => (ADTS_SYNCWORD, ADTS_SYNCLENGTH),
            TT_MP4_LOAS => (0x2B7, 11),
            _ => (0, 0),
        };

        // Mask for sync word (for adding one bit, so comprising one bit less)
        let sync_mask: u32 = (1u32 << sync_length).wrapping_sub(1);

        loop {
            let mut bits_avail: i32; // Bits available in bitstream buffer
            let check_length_bits: i32; // Helper to check remaining bits and buffer boundaries
            let mut synch: u32; // Current sync word read from bitstream

            header_bits_previous = header_bits;

            bits_avail = fdk_get_valid_bits(&mut h_tp.bit_stream[0]) as i32;

            if h_tp.number_of_raw_data_blocks == 0 {
                // search synchword

                fdk_aac_rs_fdk::fdk_assert!((bits_avail % TPDEC_SYNCSKIP) == 0);
                if bits_avail % TPDEC_SYNCSKIP != 0 {
                    UNALIGNED_SYNC_SEARCHES.with(|c| c.set(c.get() + 1));
                }

                if bits_avail.wrapping_sub(sync_length) < TPDEC_SYNCSKIP {
                    err = TRANSPORTDEC_NOT_ENOUGH_BITS;
                    header_bits = 0;
                } else {
                    let h_bs = &mut h_tp.bit_stream[0];
                    synch = fdk_read_bits(h_bs, sync_length as u32);

                    if h_tp.flags & TPDEC_SYNCOK == 0 {
                        while bits_avail.wrapping_sub(sync_length) >= TPDEC_SYNCSKIP {
                            if synch == sync_word {
                                break;
                            }
                            synch = ((synch << TPDEC_SYNCSKIP) & sync_mask)
                                | fdk_read_bits(h_bs, TPDEC_SYNCSKIP as u32);
                            bits_avail -= TPDEC_SYNCSKIP;
                        }
                    }
                    if synch != sync_word {
                        // No correct syncword found.
                        err = TRANSPORTDEC_SYNC_ERROR;
                    } else {
                        err = TRANSPORTDEC_OK;
                    }
                    header_bits = sync_length;
                }
            } else {
                header_bits = 0;
            }

            // Save previous raw data block data
            raw_data_block_length_previous = raw_data_block_length;
            num_raw_data_blocks_previous = h_tp.number_of_raw_data_blocks;

            // Parse transport header (raw data block granularity)

            if err == TRANSPORTDEC_OK {
                err = transport_dec_read_header(
                    h_tp,
                    cb,
                    sync_length,
                    ignore_buffer_fullness,
                    &mut raw_data_block_length,
                    Some(&mut f_traverse_more_frames),
                    Some(&mut sync_layer_frame_bits),
                    Some(&mut f_config_found),
                    Some(&mut header_bits),
                );
                if header_bits > bits_avail {
                    err = if header_bits < h_tp.bit_stream[0].h_bit_buf.buf_bits() as i32 {
                        TRANSPORTDEC_NOT_ENOUGH_BITS
                    } else {
                        TRANSPORTDEC_SYNC_ERROR
                    };
                }
                if tpdec_is_fatal_error(err) != 0 {
                    // Rewind - TPDEC_SYNCSKIP, in order to look for a synch one bit ahead next
                    // time. Ensure that the bit amount lands at a multiple of TPDEC_SYNCSKIP.
                    fdk_push_bi_directional(
                        &mut h_tp.bit_stream[0],
                        header_bits
                            .wrapping_neg()
                            .wrapping_add(TPDEC_SYNCSKIP)
                            .wrapping_add(bits_avail % TPDEC_SYNCSKIP),
                    );

                    break 'bail;
                }
            }

            bits_avail = bits_avail.wrapping_sub(header_bits);

            check_length_bits = sync_layer_frame_bits;

            // Check if the whole frame would fit the bitstream buffer
            if err == TRANSPORTDEC_OK {
                if check_length_bits.wrapping_add(header_bits) > ((8192 * 4) << 3) - 7 {
                    // We assume that the size of the transport bit buffer has been chosen to meet
                    // all system requirements, thus this condition is considered a
                    // synchronisation error.
                    err = TRANSPORTDEC_SYNC_ERROR;
                } else if bits_avail < check_length_bits {
                    err = TRANSPORTDEC_NOT_ENOUGH_BITS;
                }
            }

            if err == TRANSPORTDEC_NOT_ENOUGH_BITS {
                break;
            }

            if err == TRANSPORTDEC_SYNC_ERROR {
                // Enforce re-sync of transport headers.
                h_tp.number_of_raw_data_blocks = 0;

                // Ensure that the bit amount lands at a multiple of TPDEC_SYNCSKIP
                let bits = bits_avail.wrapping_add(header_bits) % TPDEC_SYNCSKIP;
                // Rewind - TPDEC_SYNCSKIP, in order to look for a synch one bit ahead next
                // time.
                fdk_push_bi_directional(
                    &mut h_tp.bit_stream[0],
                    header_bits.wrapping_sub(TPDEC_SYNCSKIP).wrapping_neg().wrapping_add(bits),
                );
                header_bits = 0;
            }

            // Frame traversal
            if f_traverse_more_frames != 0 {
                // Save parser context for early config discovery "rewind all frames"
                if h_tp.flags & TPDEC_EARLY_CONFIG != 0 && h_tp.flags & TPDEC_MINIMIZE_DELAY == 0 {
                    // ignore buffer fullness if just traversing additional frames for ECD
                    ignore_buffer_fullness = 1;

                    // Save context in order to return later
                    if err == TRANSPORTDEC_OK && start_pos_first_frame == -1 {
                        start_pos_first_frame = fdk_get_valid_bits(&mut h_tp.bit_stream[0]) as i32;
                        num_raw_data_blocks_first_frame = h_tp.number_of_raw_data_blocks;
                        global_frame_pos_first_frame = h_tp.global_frame_pos as i32;
                        raw_data_block_length_first_frame = raw_data_block_length;
                        header_bits_first_frame = header_bits;
                        err_first_frame = err;
                        context_first_frame = h_tp.parser;
                    }

                    // Break when config was found or it is not possible anymore to find a config
                    if start_pos_first_frame != -1
                        && (f_config_found != 0 || err != TRANSPORTDEC_OK)
                    {
                        // In case of ECD and sync error, do not rewind anywhere.
                        if err == TRANSPORTDEC_SYNC_ERROR {
                            start_pos_first_frame = -1;
                            f_config_found = 0;
                            num_frames_traversed = 0;
                        }
                        break;
                    }
                }

                if err == TRANSPORTDEC_OK {
                    fdk_push_for(&mut h_tp.bit_stream[0], raw_data_block_length as u32);
                    num_frames_traversed += 1;
                    end_tp_frame_bits_previous = fdk_get_valid_bits(&mut h_tp.bit_stream[0]);
                    // Ignore error here itentionally.
                    transport_dec_adjust_end_of_access_unit(h_tp);
                    end_tp_frame_bits_previous = end_tp_frame_bits_previous
                        .wrapping_sub(fdk_get_valid_bits(&mut h_tp.bit_stream[0]));
                }
            }

            if !(f_traverse_more_frames != 0
                || (err == TRANSPORTDEC_SYNC_ERROR && h_tp.flags & TPDEC_SYNCOK == 0))
            {
                break;
            }
        }

        // Restore context in case of ECD frame traversal
        if start_pos_first_frame != -1 && (f_config_found != 0 || err != TRANSPORTDEC_OK) {
            let h_bs = &mut h_tp.bit_stream[0];
            let n = fdk_get_valid_bits(h_bs).wrapping_sub(start_pos_first_frame as u32);
            fdk_push_bi_directional(h_bs, n as i32);
            h_tp.parser = context_first_frame;
            h_tp.number_of_raw_data_blocks = num_raw_data_blocks_first_frame;
            h_tp.global_frame_pos = global_frame_pos_first_frame as u32;
            raw_data_block_length = raw_data_block_length_first_frame;
            header_bits = header_bits_first_frame;
            err = err_first_frame;
            num_frames_traversed = 0;
        }

        // Additional burst data mode buffer fullness check.
        if h_tp.flags & (TPDEC_LOST_FRAMES_PENDING | TPDEC_IGNORE_BUFFERFULLNESS | TPDEC_SYNCOK)
            == 0
            && err == TRANSPORTDEC_OK
        {
            let bf = transport_dec_get_buffer_fullness(h_tp);
            let avail = (fdk_get_valid_bits(&mut h_tp.bit_stream[0]) as i32)
                .wrapping_sub(sync_layer_frame_bits);
            err = additional_hold_off_needed(h_tp, bf, avail);
            if err == TRANSPORTDEC_NOT_ENOUGH_BITS {
                h_tp.hold_off_frames = h_tp.hold_off_frames.wrapping_add(1);
            }
        }

        // Rewind for retry because of not enough bits
        if err == TRANSPORTDEC_NOT_ENOUGH_BITS {
            fdk_push_back(&mut h_tp.bit_stream[0], header_bits as u32);
            h_tp.number_of_raw_data_blocks = num_raw_data_blocks_previous;
            header_bits = 0;
            raw_data_block_length = raw_data_block_length_previous;
        } else {
            // reset hold off frame counter
            h_tp.hold_off_frames = 0;
        }

        // Return to last good frame in case of frame traversal but not ECD.
        if num_frames_traversed > 0 {
            fdk_push_back(
                &mut h_tp.bit_stream[0],
                (raw_data_block_length_previous as u32).wrapping_add(end_tp_frame_bits_previous),
            );
            if err != TRANSPORTDEC_OK {
                h_tp.number_of_raw_data_blocks = num_raw_data_blocks_previous;
                header_bits = header_bits_previous;
                raw_data_block_length = raw_data_block_length_previous;
            }
            err = TRANSPORTDEC_OK;
        }
    }

    // bail:
    h_tp.au_length[0] = raw_data_block_length;

    // Detect pointless TRANSPORTDEC_NOT_ENOUGH_BITS error case, where the bit buffer is already
    // full, or no new burst packet fits. Recover by advancing the bit buffer.
    if total_bits > 0
        && TRANSPORTDEC_NOT_ENOUGH_BITS == err
        && fdk_get_valid_bits(&mut h_tp.bit_stream[0])
            >= ((8192u32 * 4) * 8)
                .wrapping_sub(h_tp.avg_bit_rate.wrapping_mul(h_tp.burst_period) / 1000)
                .wrapping_sub(7)
    {
        fdk_push_for(&mut h_tp.bit_stream[0], TPDEC_SYNCSKIP as u32);
        err = TRANSPORTDEC_SYNC_ERROR;
    }

    if err == TRANSPORTDEC_OK {
        h_tp.flags |= TPDEC_SYNCOK;
    }

    if f_config_found != 0 {
        h_tp.flags |= TPDEC_CONFIG_FOUND;
    }

    if let Some(p) = p_header_bits {
        *p = header_bits;
    }

    if err == TRANSPORTDEC_SYNC_ERROR {
        h_tp.flags &= !TPDEC_SYNCOK;
    }

    err
}

/// `transportDec_readStream(hTp, layer)`: synchronizes to the stream and estimates the number of
/// missing access units due to a synchronization error in case of a constant average bit rate.
///
/// The estimate divides by `avgBitRate * m_samplesPerFrame`, which can wrap to 0 in `UINT`; the
/// port then computes the AArch64 result (0, `sdiv`) on every variant.
fn transport_dec_read_stream(
    h_tp: &mut TransportDec,
    cb: &mut dyn TpDecCallbacks,
    _layer: u32,
) -> TransportDecError {
    let mut header_bits: i32 = 0;
    let bf_delta: i32;

    // Obtain distance to next synch word
    let mut bit_distance = fdk_get_valid_bits(&mut h_tp.bit_stream[0]) as i32;
    let mut error = synchronization(h_tp, cb, Some(&mut header_bits));
    bit_distance = bit_distance.wrapping_sub(fdk_get_valid_bits(&mut h_tp.bit_stream[0]) as i32);

    fdk_aac_rs_fdk::fdk_assert!(bit_distance >= 0);

    let mut n_au: i32 = -1;

    if (error == TRANSPORTDEC_SYNC_ERROR || h_tp.flags & TPDEC_LOST_FRAMES_PENDING != 0)
        && h_tp.avg_bit_rate > 0
        && h_tp.asc[0].m_samples_per_frame > 0
        && h_tp.asc[0].m_sampling_frequency > 0
    {
        // Check if estimating lost access units is feasible.
        if error == TRANSPORTDEC_OK {
            let aj = transport_dec_get_buffer_fullness(h_tp);
            bf_delta = if aj > 0 { aj } else { 0 };
            // sync was ok: last of a series of bad access units.
            h_tp.flags &= !TPDEC_LOST_FRAMES_PENDING;
            // Add up bitDistance until end of the current frame. Later we substract this frame
            // from the grand total, since this current successfully synchronized frame should
            // not be skipped of course; but it must be accounted into the bufferfulness math.
            bit_distance = bit_distance.wrapping_add(h_tp.au_length[0]);
        } else if h_tp.flags & TPDEC_LOST_FRAMES_PENDING == 0 {
            // sync not ok: one of many bad access units.
            h_tp.flags |= TPDEC_LOST_FRAMES_PENDING;
            bf_delta = (h_tp.last_valid_buffer_fullness as i32).wrapping_neg();
        } else {
            bf_delta = 0;
        }

        {
            // Obtain estimate of number of lost frames
            let num = (h_tp.asc[0].m_sampling_frequency as i32)
                .wrapping_mul(bf_delta.wrapping_add(bit_distance))
                .wrapping_add(h_tp.remainder);
            let denom = h_tp.avg_bit_rate.wrapping_mul(h_tp.asc[0].m_samples_per_frame) as i32;
            if num > 0 {
                n_au = div_c(num, denom);
                h_tp.remainder = rem_c(num, denom);
            } else {
                h_tp.remainder = num;
            }

            if error == TRANSPORTDEC_OK {
                // Final adjustment of remainder, taken -1 into account because current frame
                // should not be skipped, thus substract -1 or do nothing instead of +1-1
                // accordingly.
                if denom.wrapping_sub(h_tp.remainder) >= h_tp.remainder {
                    n_au = n_au.wrapping_sub(1);
                }

                if n_au < 0 {
                    // There was one frame too much concealed, so unfortunately we will have to
                    // skip one good frame.
                    transport_dec_end_access_unit(h_tp);
                    error = synchronization(h_tp, cb, Some(&mut header_bits));
                    n_au = -1;
                }
                h_tp.remainder = 0;
                // Enforce last missed frames to be concealed.
                if n_au > 0 {
                    fdk_push_back(&mut h_tp.bit_stream[0], header_bits as u32);
                }
            }
        }
    }

    // Be sure that lost frames are handled correctly. This is necessary due to some sync error
    // sequences where later it turns out that there is not enough data, but the bits upto the
    // sync word are discarded, thus causing a value of nAU > 0
    if n_au > 0 {
        error = TRANSPORTDEC_SYNC_ERROR;
    }

    h_tp.missing_access_units = n_au;

    error
}

/// `transportDec_ReadAccessUnit(hTp, layer)` with the registered callbacks.
pub fn transport_dec_read_access_unit(h_tp: &mut TransportDec, layer: u32) -> TransportDecError {
    let mut cb = h_tp.callbacks;
    transport_dec_read_access_unit_cb(h_tp, layer, &mut cb)
}

/// `transportDec_ReadAccessUnit(hTp, layer)`: synchronizes to the next access unit and parses
/// its transport header (and the configuration, if one is found).
pub fn transport_dec_read_access_unit_cb(
    h_tp: &mut TransportDec,
    layer: u32,
    cb: &mut dyn TpDecCallbacks,
) -> TransportDecError {
    let mut err = TRANSPORTDEC_OK;
    let l = l1(layer);

    if (fdk_get_valid_bits(&mut h_tp.bit_stream[l]) as i32) <= 0 {
        // This is only relevant for RAW and ADIF cases. For streaming formats err will get
        // overwritten.
        err = TRANSPORTDEC_NOT_ENOUGH_BITS;
        h_tp.number_of_raw_data_blocks = 0;
    }

    match h_tp.transport_fmt {
        TT_MP4_ADIF => {
            // Read header if not already done
            if h_tp.flags & TPDEC_CONFIG_FOUND == 0 {
                let bs_start = fdk_get_valid_bits(&mut h_tp.bit_stream[l]) as i32;
                let mut config_changed: u8 = 0;
                let mut config_mode: u8 = AC_CM_DET_CFG_CHANGE;

                for i in 0..2 {
                    if i > 0 {
                        let h_bs = &mut h_tp.bit_stream[l];
                        let n = bs_start.wrapping_sub(fdk_get_valid_bits(h_bs) as i32);
                        fdk_push_back(h_bs, n as u32);
                        config_mode = AC_CM_ALLOC_MEM;
                    }

                    audio_specific_config_init(&mut h_tp.asc[0]);
                    err = adif_read_decode_header(
                        &mut h_tp.parser.adif,
                        &mut h_tp.asc[0].m_progr_config_element,
                        &mut h_tp.bit_stream[l],
                    );
                    if err != 0 {
                        return err; // goto bail
                    }

                    // Map adif header to ASC
                    let pce = h_tp.asc[0].m_progr_config_element;
                    h_tp.asc[0].m_aot = pce.profile as i32 + 1;
                    h_tp.asc[0].m_sampling_frequency_index = pce.sampling_frequency_index;
                    h_tp.asc[0].m_sampling_frequency =
                        sampling_rate_table(pce.sampling_frequency_index as usize);
                    h_tp.asc[0].m_channel_configuration = 0;
                    h_tp.asc[0].m_samples_per_frame = 1024;
                    h_tp.avg_bit_rate = h_tp.parser.adif.bit_rate;

                    // Call callback to decoder.
                    let err_c = cb.update_config(&h_tp.asc[0], config_mode, &mut config_changed);
                    if err_c == 0 {
                        h_tp.flags |= TPDEC_CONFIG_FOUND;
                    } else {
                        return TRANSPORTDEC_PARSE_ERROR; // goto bail
                    }

                    if err == TRANSPORTDEC_OK && i == 0 && config_changed != 0 {
                        let err_c = cb.free_mem(&h_tp.asc[0]);
                        if err_c != 0 {
                            err = TRANSPORTDEC_PARSE_ERROR;
                        }
                    }
                }
            }
            h_tp.au_length[l] = -1; // Access Unit data length is unknown.
        }

        TT_MP4_RAW | TT_DRM => {
            // One Access Unit was filled into buffer. So get the length out of the buffer.
            h_tp.au_length[l] = fdk_get_valid_bits(&mut h_tp.bit_stream[l]) as i32;
            h_tp.flags |= TPDEC_SYNCOK;
        }

        TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 => {
            if err == TRANSPORTDEC_OK {
                let mut f_config_found = (h_tp.flags & TPDEC_CONFIG_FOUND) as i32;
                let mut au_length = h_tp.au_length[l];
                err = transport_dec_read_header(
                    h_tp,
                    cb,
                    0,
                    1,
                    &mut au_length,
                    None,
                    None,
                    Some(&mut f_config_found),
                    None,
                );
                h_tp.au_length[l] = au_length;
                if f_config_found != 0 {
                    h_tp.flags |= TPDEC_CONFIG_FOUND;
                }
            }
        }

        TT_MP4_ADTS | TT_MP4_LOAS => {
            err = transport_dec_read_stream(h_tp, cb, layer);
        }

        _ => {
            err = TRANSPORTDEC_UNSUPPORTED_FORMAT;
        }
    }

    if err == TRANSPORTDEC_OK {
        h_tp.access_unit_anchor[l] = fdk_get_valid_bits(&mut h_tp.bit_stream[l]);
    } else {
        h_tp.access_unit_anchor[l] = 0;
    }

    err
}

/// `transportDec_GetAsc(hTp, layer, asc)`: copies the current configuration.
pub fn transport_dec_get_asc(
    h_tp: &TransportDec,
    layer: u32,
    asc: &mut CSAudioSpecificConfig,
) -> TransportDecError {
    *asc = h_tp.asc[la(layer)];
    TRANSPORTDEC_OK
}

/// `transportDec_GetAuBitsRemaining(hTp, layer)`: the number of bits of the current access unit
/// that have not been read yet.
pub fn transport_dec_get_au_bits_remaining(h_tp: &mut TransportDec, layer: u32) -> i32 {
    let l = l1(layer);
    let mut bits: i32;

    if h_tp.access_unit_anchor[l] > 0 && h_tp.au_length[l] > 0 {
        bits = fdk_get_valid_bits(&mut h_tp.bit_stream[l]) as i32;
        if bits >= 0 {
            bits = h_tp.au_length[l]
                .wrapping_sub((h_tp.access_unit_anchor[l] as i32).wrapping_sub(bits));
        }
    } else {
        bits = fdk_get_valid_bits(&mut h_tp.bit_stream[l]) as i32;
    }

    bits
}

/// `transportDec_GetAuBitsTotal(hTp, layer)`: the length of the current access unit in bits.
pub fn transport_dec_get_au_bits_total(h_tp: &TransportDec, layer: u32) -> i32 {
    h_tp.au_length[l1(layer)]
}

/// `transportDec_GetMissingAccessUnitCount(pNAccessUnits, hTp)`
pub fn transport_dec_get_missing_access_unit_count(
    p_n_access_units: &mut i32,
    h_tp: &TransportDec,
) -> TransportDecError {
    *p_n_access_units = h_tp.missing_access_units;

    TRANSPORTDEC_OK
}

/// `transportDec_EndAccessUnit(hTp)`: informs the transport decoder that reading of the access
/// unit has finished.
pub fn transport_dec_end_access_unit(h_tp: &mut TransportDec) -> TransportDecError {
    match h_tp.transport_fmt {
        TT_MP4_LOAS | TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 => {
            if h_tp.number_of_raw_data_blocks == 0 {
                // Read other data if available.
                if clatm_demux_get_other_data_present_flag(&h_tp.parser.latm) != 0 {
                    let other_data_len =
                        clatm_demux_get_other_data_length(&h_tp.parser.latm) as i32;

                    let h_bs = &mut h_tp.bit_stream[0];
                    if fdk_get_valid_bits(h_bs) as i32 >= other_data_len {
                        fdk_push_for(h_bs, other_data_len as u32);
                    } else {
                        // Do byte align at the end of AudioMuxElement.
                        if h_tp.number_of_raw_data_blocks == 0 {
                            fdk_byte_align(h_bs, h_tp.global_frame_pos);
                        }
                        return TRANSPORTDEC_NOT_ENOUGH_BITS;
                    }
                }
            } else {
                // If bit buffer has not more bits but hTp->numberOfRawDataBlocks > 0 then too
                // many bits were read and obviously no more RawDataBlocks can be read. Set
                // numberOfRawDataBlocks to zero to attempt a new sync attempt.
                if fdk_get_valid_bits(&mut h_tp.bit_stream[0]) as i32 <= 0 {
                    h_tp.number_of_raw_data_blocks = 0;
                }
            }
        }
        _ => {}
    }

    transport_dec_adjust_end_of_access_unit(h_tp)
}

/// `transportDec_SetParam(hTp, param, value)`
///
/// `TPDEC_PARAM_FORCE_CONFIG_CHANGE` has no defined reference result for values other than 0;
/// the port ignores them (the AAC decoder never sets this parameter).
pub fn transport_dec_set_param(
    h_tp: &mut TransportDec,
    param: TpdecParam,
    value: i32,
) -> TransportDecError {
    let error = TRANSPORTDEC_OK;

    match param {
        TPDEC_PARAM_MINIMIZE_DELAY => {
            if value != 0 {
                h_tp.flags |= TPDEC_MINIMIZE_DELAY;
            } else {
                h_tp.flags &= !TPDEC_MINIMIZE_DELAY;
            }
        }
        TPDEC_PARAM_EARLY_CONFIG => {
            if value != 0 {
                h_tp.flags |= TPDEC_EARLY_CONFIG;
            } else {
                h_tp.flags &= !TPDEC_EARLY_CONFIG;
            }
        }
        TPDEC_PARAM_IGNORE_BUFFERFULLNESS => {
            if value != 0 {
                h_tp.flags |= TPDEC_IGNORE_BUFFERFULLNESS;
            } else {
                h_tp.flags &= !TPDEC_IGNORE_BUFFERFULLNESS;
            }
        }
        TPDEC_PARAM_SET_BITRATE => {
            h_tp.avg_bit_rate = value as u32;
        }
        TPDEC_PARAM_BURST_PERIOD => {
            h_tp.burst_period = value as u32;
        }
        TPDEC_PARAM_RESET => {
            for i in 0..1 {
                fdk_reset_bitbuffer(&mut h_tp.bit_stream[i], BS_READER);
                h_tp.au_length[i] = 0;
                h_tp.access_unit_anchor[i] = 0;
            }
            h_tp.flags &= !(TPDEC_SYNCOK | TPDEC_LOST_FRAMES_PENDING);
            if h_tp.transport_fmt != TT_MP4_ADIF {
                h_tp.flags &= !TPDEC_CONFIG_FOUND;
            }
            h_tp.remainder = 0;
            h_tp.avg_bit_rate = 0;
            h_tp.missing_access_units = 0;
            h_tp.number_of_raw_data_blocks = 0;
            h_tp.global_frame_pos = 0;
            h_tp.hold_off_frames = 0;
        }
        TPDEC_PARAM_TARGETLAYOUT => {
            h_tp.target_layout = value;
        }
        TPDEC_PARAM_FORCE_CONFIG_CHANGE => {
            if value == 0 {
                h_tp.ctrl_cfg_change[0].force_cfg_change = TPDEC_FORCE_CONFIG_CHANGE;
            }
        }
        TPDEC_PARAM_USE_ELEM_SKIPPING => {
            if value != 0 {
                h_tp.flags |= TPDEC_USE_ELEM_SKIPPING;
            } else {
                h_tp.flags &= !TPDEC_USE_ELEM_SKIPPING;
            }
        }
        _ => {}
    }

    error
}

/// `transportDec_GetNrOfSubFrames(hTp)`
pub fn transport_dec_get_nr_of_sub_frames(h_tp: &TransportDec) -> u32 {
    let mut n_sub_frames = 0;

    if h_tp.transport_fmt == TT_MP4_LATM_MCP1
        || h_tp.transport_fmt == TT_MP4_LATM_MCP0
        || h_tp.transport_fmt == TT_MP4_LOAS
    {
        n_sub_frames = clatm_demux_get_nr_of_sub_frames(&h_tp.parser.latm);
    } else if h_tp.transport_fmt == TT_MP4_ADTS {
        n_sub_frames = h_tp.parser.adts.bs.num_raw_blocks as u32;
    }

    n_sub_frames
}

/// `transportDec_Close(phTp)`
pub fn transport_dec_close(ph_tp: &mut Option<Box<TransportDec>>) {
    *ph_tp = None;
}

/// `transportDec_GetLibInfo(info)`: enters the library information into the first free entry
/// of `info` (an array of `FDK_MODULE_LAST` entries).
///
/// A slice shorter than `FDK_MODULE_LAST` without a free entry ends the search. `build_date`
/// and `build_time` are empty.
pub fn transport_dec_get_lib_info(info: Option<&mut [LibInfo]>) -> TransportDecError {
    let Some(info) = info else {
        return TRANSPORTDEC_UNKOWN_ERROR;
    };

    // search for next free tab
    let mut i = 0usize;
    while i < FDK_MODULE_LAST as usize {
        match info.get(i) {
            Some(e) if e.module_id == FDK_NONE => break,
            Some(_) => {}
            None => return TRANSPORTDEC_UNKOWN_ERROR,
        }
        i += 1;
    }
    if i == FDK_MODULE_LAST as usize {
        return TRANSPORTDEC_UNKOWN_ERROR;
    }
    let info = &mut info[i];

    info.module_id = FDK_TPDEC;
    info.build_date = TP_LIB_BUILD_DATE;
    info.build_time = TP_LIB_BUILD_TIME;
    info.title = TP_LIB_TITLE;
    info.version = lib_version(TP_LIB_VL0, TP_LIB_VL1, TP_LIB_VL2);
    lib_version_string(info);
    info.flags = CAPF_ADIF | CAPF_ADTS | CAPF_LATM | CAPF_LOAS | CAPF_RAWPACKETS | CAPF_DRM;

    TRANSPORTDEC_OK // FDKERR_NOERROR;
}

// ---------------------------------------------------------------------------------------------
// CRC
// ---------------------------------------------------------------------------------------------

/// `transportDec_CrcStartReg(hTp, mBits)`: starts a CRC region (ADTS, DRM). Returns the region
/// ID, -1 for other formats.
pub fn transport_dec_crc_start_reg(p_tp: &mut TransportDec, m_bits: i32) -> i32 {
    match p_tp.transport_fmt {
        TT_MP4_ADTS => {
            adts_read_crc_start_reg(&mut p_tp.parser.adts, &mut p_tp.bit_stream[0], m_bits)
        }
        TT_DRM => drm_read_crc_start_reg(&mut p_tp.parser.drm, &mut p_tp.bit_stream[0], m_bits),
        _ => -1,
    }
}

/// `transportDec_CrcEndReg(hTp, reg)`: ends a CRC region.
pub fn transport_dec_crc_end_reg(p_tp: &mut TransportDec, reg: i32) {
    match p_tp.transport_fmt {
        TT_MP4_ADTS => adts_read_crc_end_reg(&mut p_tp.parser.adts, &mut p_tp.bit_stream[0], reg),
        TT_DRM => drm_read_crc_end_reg(&mut p_tp.parser.drm, &mut p_tp.bit_stream[0], reg),
        _ => {}
    }
}

/// `transportDec_CrcCheck(hTp)`: checks the CRC of the access unit.
pub fn transport_dec_crc_check(p_tp: &mut TransportDec) -> TransportDecError {
    match p_tp.transport_fmt {
        TT_MP4_ADTS => {
            if p_tp.parser.adts.bs.num_raw_blocks > 0 && p_tp.parser.adts.bs.protection_absent == 0
            {
                transport_dec_adjust_end_of_access_unit(p_tp);
            }
            adts_read_crc_check(&mut p_tp.parser.adts)
        }
        TT_DRM => drm_read_crc_check(&mut p_tp.parser.drm),
        _ => TRANSPORTDEC_OK,
    }
}

/// `transportDec_DrmRawSdcAudioConfig_Check(conf, length)`: checks a DRM SDC audio config
/// without configuring anything.
///
/// The port reads a zero-padded copy of the slice `conf` (which may extend beyond `length`
/// bytes, as the caller's buffer does in C); bits beyond the slice read as 0 (no defined
/// reference result).
pub fn transport_dec_drm_raw_sdc_audio_config_check(conf: &[u8], length: u32) -> TransportDecError {
    let mut asc = CSAudioSpecificConfig::zeroed();
    let mut bs = FdkBitStream::default();

    // A power of two with room for the at most ~10 bytes the parser reads.
    let size = (conf.len() + 64).next_power_of_two().min(BUFSIZE_DUMMY_VALUE as usize);
    let buf = BitMem::new(size);
    buf.write(0, &conf[..conf.len().min(size)]);

    fdk_init_bit_stream(&mut bs, &buf, size as u32, length << 3, BS_READER);

    drm_raw_sdc_audio_config_parse(&mut asc, &mut bs, None, AC_CM_ALLOC_MEM, 0)
}

/// Not in C: the state digest of the chunked decoding (`fdk_aac_rs_fdk::digest`; here because of
/// the private field). The input buffer `bs_buffer` is the memory of `bit_stream[0]`, fed there
/// relative to the read position; `p_loudness_info_set_position` points to the decoder that owns
/// the transport decoder (`loudnessInfoSetPosition`, part of that decoder's digest): only whether
/// it is set is fed.
impl fdk_aac_rs_fdk::digest::StateDigest for TransportDec {
    fn digest(&self, d: &mut fdk_aac_rs_fdk::digest::Digester) {
        let TransportDec {
            transport_fmt,
            callbacks,
            bit_stream,
            bs_buffer,
            parser,
            asc,
            ctrl_cfg_change,
            global_frame_pos,
            access_unit_anchor,
            au_length,
            number_of_raw_data_blocks,
            avg_bit_rate,
            last_valid_buffer_fullness,
            remainder,
            missing_access_units,
            burst_period,
            hold_off_frames,
            flags,
            target_layout,
            p_loudness_info_set_position,
        } = self;
        d.field("transport_fmt", transport_fmt);
        d.field("callbacks", callbacks);
        d.field("bit_stream", bit_stream);
        d.field("bs_buffer_is_bit_stream", &bs_buffer.ptr_eq(bit_stream[0].h_bit_buf.mem()));
        d.field("parser", parser);
        d.field("asc", asc);
        d.field("ctrl_cfg_change", ctrl_cfg_change);
        d.field("global_frame_pos", global_frame_pos);
        d.field("access_unit_anchor", access_unit_anchor);
        d.field("au_length", au_length);
        d.field("number_of_raw_data_blocks", number_of_raw_data_blocks);
        d.field("avg_bit_rate", avg_bit_rate);
        d.field("last_valid_buffer_fullness", last_valid_buffer_fullness);
        d.field("remainder", remainder);
        d.field("missing_access_units", missing_access_units);
        d.field("burst_period", burst_period);
        d.field("hold_off_frames", hold_off_frames);
        d.field("flags", flags);
        d.field("target_layout", target_layout);
        d.field("p_loudness_info_set_position", &p_loudness_info_set_position.is_null());
    }
}
