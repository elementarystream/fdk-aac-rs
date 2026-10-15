// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/include/aacdecoder_lib.h (the C API).

//! Decoder part of the C ABI: `aacDecoder_*`, with the signatures of `aacdecoder_lib.h`.
//!
//! Each function passes its arguments to the straight port
//! ([`fdk_aac_rs_aacdec::aacdecoder_lib`]) unchanged. `HANDLE_AACDECODER` points to an
//! [`AAC_DECODER_INSTANCE`] (a `Box` released by `aacDecoder_Close()`), which holds the port's
//! [`AacDecoderInstance`] and the [`CStreamInfo`] with the C layout that
//! `aacDecoder_GetStreamInfo()` returns.
//!
//! **`CStreamInfo`.** C returns `&self->streamInfo`: one structure per decoder, valid until
//! `aacDecoder_Close()`, whose fields the decoder updates in place, and whose `pChannelType` and
//! `pChannelIndices` point to the decoder's channel arrays. The port keeps the C structure and
//! the two arrays in the handle and copies the straight port's stream information into them at
//! the end of every call that takes the handle (`aacDecoder_Open()` included). The pointer
//! returned by `aacDecoder_GetStreamInfo()` is therefore the same for every call on a handle,
//! stays valid until `aacDecoder_Close()`, and shows after each call what C shows after that
//! call.
//!
//! **Threads (not in C).** `aacDecoder_Open()` reads the environment variable
//! `FDK_AAC_RS_DEC_THREADS`, or `FDK_AAC_RS_THREADS` if that is not set (a positive integer;
//! unset, invalid or 0: 1, the default). With more than
//! one thread the decoder pipelines frames ([`fdk_aac_rs_aacdec::pipeline`]): every
//! `aacDecoder_DecodeFrame()` call that decodes a frame returns the frame of the previous such
//! call, with its stream information and ancillary data; a call that finds not enough input
//! returns `AAC_DEC_NOT_ENOUGH_BITS` at once (as does the call that decodes the first frame, as
//! there is nothing to return yet). The frames come in the order and with the content of one
//! thread. After the last `aacDecoder_Fill()`, the caller calls `aacDecoder_DecodeFrame()` with
//! flags 0 until it returns `AAC_DEC_NOT_ENOUGH_BITS` twice in a row: a call that finds not
//! enough input directly after an `AAC_DEC_NOT_ENOUGH_BITS` return, without `aacDecoder_Fill()`
//! in between, returns the pending frame instead. The stream information after an
//! `AAC_DEC_NOT_ENOUGH_BITS` return is that of one thread after the call (the call waits for
//! the frame being decoded on the worker thread for it).
//!
//! **Buffers.** `UCHAR *pBuffer[]`, `const UINT bufferSize[]`, `UINT *pBytesValid` (one entry per
//! layer, as many layers as the decoder has) become slices of the straight port; the output
//! buffer of `aacDecoder_DecodeFrame()` is a slice of `timeDataSize` samples, which the straight
//! port clears completely when the call returns no valid output, as C does (PORTING.md,
//! "Definition of bit-exact"). The ancillary data buffer of `aacDecoder_AncDataInit()` stays the
//! caller's buffer ([`lib::aac_decoder_anc_data_init_raw`]); `aacDecoder_AncDataGet()` returns
//! pointers into it.
//!
//! **NULL arguments.** Where C checks for NULL, the port returns what C returns:
//!
//! | Function | NULL argument | C | Port |
//! |---|---|---|---|
//! | `aacDecoder_Open` | – (unsupported transport) | NULL | NULL |
//! | `aacDecoder_Close` | `self` | returns | returns |
//! | `aacDecoder_GetStreamInfo` | `self` | NULL | NULL |
//! | `aacDecoder_DecodeFrame` | `self` | `AAC_DEC_INVALID_HANDLE` | `AAC_DEC_INVALID_HANDLE` |
//! | `aacDecoder_SetParam` | `self` | `AAC_DEC_INVALID_HANDLE` | `AAC_DEC_INVALID_HANDLE` |
//! | `aacDecoder_GetFreeBytes` | `self` | `*pFreeBytes = 0`, `AAC_DEC_INVALID_HANDLE` | the same |
//! | `aacDecoder_GetLibInfo` | `info` | -1 | -1 |
//!
//! Where C does not define the result of a NULL argument, the port returns:
//!
//! | Function | NULL argument | Port |
//! |---|---|---|
//! | `aacDecoder_GetFreeBytes` | `pFreeBytes` | not written; the result of C otherwise |
//! | `aacDecoder_Fill`, `aacDecoder_ConfigRaw`, `aacDecoder_AncDataInit`, `aacDecoder_AncDataGet`, `aacDecoder_RawISOBMFFData` | `self` | `AAC_DEC_INVALID_HANDLE` |
//! | `aacDecoder_Fill` | `pBuffer`, `bufferSize` or `pBytesValid` (with a layer) | `AAC_DEC_UNKNOWN` |
//! | `aacDecoder_Fill` | `pBuffer[layer]` with a size | an empty buffer |
//! | `aacDecoder_ConfigRaw` | `conf` or `length` (with a layer) | `AAC_DEC_UNKNOWN` |
//! | `aacDecoder_ConfigRaw` | `conf[layer]` with a length | an empty configuration |
//! | `aacDecoder_DecodeFrame` | `pTimeData` | decodes without writing samples |
//! | `aacDecoder_AncDataGet` | `ptr` or `size` | not written |
//! | `aacDecoder_RawISOBMFFData` | `buffer` | an empty buffer |
//!
//! `aacDecoder_GetLibInfo()` converts the `LIB_INFO` array with [`with_lib_info`], which writes
//! back only the fields the library wrote (the `flags` of the DRC decoder's entry stay the
//! caller's; PORTING.md, "Definition of bit-exact").
//!
//! **`aacDecoder_RawISOBMFFData`.** Upstream declares it in `aacdecoder_lib.h` but leaves it
//! out of `fdk-aac.sym`, so the static library has it and the shared library does not. The
//! symbol is a global label defined with `global_asm!` that branches to a Rust `extern "C"`
//! function (PORTING.md, "C ABI"): rustc does not know it as an export, so the shared library's
//! export list (rustc's) does not contain it, while the static library's object defines it
//! globally.
//!
//! # Panics
//!
//! No panic crosses the FFI boundary: every function runs the straight port inside
//! [`std::panic::catch_unwind`]. The straight port has no known panicking path (PORTING.md); the
//! guard turns a panic that would still occur into an error code instead of an abort. A panic
//! can leave the decoder's state inconsistent, so the handle is then marked as failed, and every
//! later call on it (except `aacDecoder_Close()` and `aacDecoder_GetStreamInfo()`) returns the
//! error code of the table below without calling the straight port:
//!
//! | Function | Result after a panic, and on a failed handle |
//! |---|---|
//! | `aacDecoder_Open` | NULL |
//! | `aacDecoder_Close` | returns (the decoder's memory may leak) |
//! | `aacDecoder_DecodeFrame` | `AAC_DEC_UNKNOWN`, and the `timeDataSize` samples are cleared (no valid output) |
//! | `aacDecoder_Fill`, `aacDecoder_ConfigRaw`, `aacDecoder_RawISOBMFFData` | `AAC_DEC_UNKNOWN` |
//! | `aacDecoder_SetParam` | `AAC_DEC_SET_PARAM_FAIL` |
//! | `aacDecoder_AncDataInit`, `aacDecoder_AncDataGet` | `AAC_DEC_ANC_DATA_ERROR` (`*ptr` NULL, `*size` 0) |
//! | `aacDecoder_GetFreeBytes` | `AAC_DEC_UNKNOWN`, `*pFreeBytes = 0` |
//! | `aacDecoder_GetStreamInfo` | the stream information after the last successful call |
//! | `aacDecoder_GetLibInfo` | -1 |
//!
//! In a build with `panic = "abort"` a panic aborts the process instead (it still does not
//! unwind into C).

use crate::lib_info::{with_lib_info, LIB_INFO};
use core::ffi::{c_int, c_schar, c_uchar, c_uint};
use core::panic::AssertUnwindSafe;
use core::ptr::{addr_of_mut, null_mut};
use fdk_aac_rs_aacdec::aacdecoder::{AacDecoderInstance, AncBuffer};
use fdk_aac_rs_aacdec::aacdecoder_lib as lib;
use std::panic::catch_unwind;

/// `AAC_DECODER_ERROR`
pub type AAC_DECODER_ERROR = lib::AacDecoderError;

/// `AACDEC_PARAM`
pub type AACDEC_PARAM = c_int;
/// `TRANSPORT_TYPE`
pub type TRANSPORT_TYPE = c_int;
/// `AUDIO_OBJECT_TYPE`
pub type AUDIO_OBJECT_TYPE = c_int;
/// `AUDIO_CHANNEL_TYPE`
pub type AUDIO_CHANNEL_TYPE = c_int;
/// `INT_PCM`
pub type INT_PCM = i16;
/// `HANDLE_AACDECODER`
pub type HANDLE_AACDECODER = *mut AAC_DECODER_INSTANCE;

/// The number of entries of the decoder's `channelType` and `channelIndices` arrays, to which
/// `pChannelType` and `pChannelIndices` point.
pub const STREAM_INFO_CHANNELS: usize = 8;

/// `CStreamInfo` (C layout): the stream information of `aacDecoder_GetStreamInfo()`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CStreamInfo {
    /// `INT sampleRate`
    pub sampleRate: c_int,
    /// `INT frameSize`
    pub frameSize: c_int,
    /// `INT numChannels`
    pub numChannels: c_int,
    /// `AUDIO_CHANNEL_TYPE *pChannelType`
    pub pChannelType: *mut AUDIO_CHANNEL_TYPE,
    /// `UCHAR *pChannelIndices`
    pub pChannelIndices: *mut c_uchar,
    /// `INT aacSampleRate`
    pub aacSampleRate: c_int,
    /// `INT profile`
    pub profile: c_int,
    /// `AUDIO_OBJECT_TYPE aot`
    pub aot: AUDIO_OBJECT_TYPE,
    /// `INT channelConfig`
    pub channelConfig: c_int,
    /// `INT bitRate`
    pub bitRate: c_int,
    /// `INT aacSamplesPerFrame`
    pub aacSamplesPerFrame: c_int,
    /// `INT aacNumChannels`
    pub aacNumChannels: c_int,
    /// `AUDIO_OBJECT_TYPE extAot`
    pub extAot: AUDIO_OBJECT_TYPE,
    /// `INT extSamplingRate`
    pub extSamplingRate: c_int,
    /// `UINT outputDelay`
    pub outputDelay: c_uint,
    /// `UINT flags`
    pub flags: c_uint,
    /// `SCHAR epConfig`
    pub epConfig: c_schar,
    /// `INT numLostAccessUnits`
    pub numLostAccessUnits: c_int,
    /// `INT64 numTotalBytes`
    pub numTotalBytes: i64,
    /// `INT64 numBadBytes`
    pub numBadBytes: i64,
    /// `INT64 numTotalAccessUnits`
    pub numTotalAccessUnits: i64,
    /// `INT64 numBadAccessUnits`
    pub numBadAccessUnits: i64,
    /// `SCHAR drcProgRefLev`
    pub drcProgRefLev: c_schar,
    /// `SCHAR drcPresMode`
    pub drcPresMode: c_schar,
    /// `INT outputLoudness`
    pub outputLoudness: c_int,
}

/// `struct AAC_DECODER_INSTANCE`, the object `HANDLE_AACDECODER` points to (opaque in C).
pub struct AAC_DECODER_INSTANCE {
    /// The straight port's decoder; `None` only while `aacDecoder_Close()` releases it.
    dec: Option<Box<AacDecoderInstance>>,
    /// Set when a call panicked (see the module documentation).
    failed: bool,
    /// `streamInfo`, as `aacDecoder_GetStreamInfo()` returns it.
    info: CStreamInfo,
    /// `channelType`, to which `info.pChannelType` points.
    channel_type: [AUDIO_CHANNEL_TYPE; STREAM_INFO_CHANNELS],
    /// `channelIndices`, to which `info.pChannelIndices` points.
    channel_indices: [c_uchar; STREAM_INFO_CHANNELS],
}

impl AAC_DECODER_INSTANCE {
    /// The straight port's decoder, unless the handle failed.
    fn decoder(&mut self) -> Option<&mut AacDecoderInstance> {
        if self.failed {
            None
        } else {
            self.dec.as_deref_mut()
        }
    }
}

/// Copies the straight port's stream information into the C structure of the handle.
///
/// # Safety
/// `h` must be an open handle.
unsafe fn sync_stream_info(h: HANDLE_AACDECODER) {
    // SAFETY: the caller's guarantee; the pointers are derived from `h` without a reference to
    // the whole handle, so pointers into it that the caller holds stay valid.
    unsafe {
        let Some(dec) = (*h).dec.as_deref_mut() else { return };
        // With more than one thread: the stream information of the frame the last decode call
        // returned, complete after a call without a frame (see the module documentation).
        lib::aac_decoder_sync_stream_info(dec);
        let Some(s) = lib::aac_decoder_get_stream_info(Some(&*dec)) else { return };
        let ct = addr_of_mut!((*h).channel_type);
        let ci = addr_of_mut!((*h).channel_indices);
        *ct = s.p_channel_type;
        *ci = s.p_channel_indices;
        *addr_of_mut!((*h).info) = CStreamInfo {
            sampleRate: s.sample_rate,
            frameSize: s.frame_size,
            numChannels: s.num_channels,
            pChannelType: ct.cast(),
            pChannelIndices: ci.cast(),
            aacSampleRate: s.aac_sample_rate,
            profile: s.profile,
            aot: s.aot,
            channelConfig: s.channel_config,
            bitRate: s.bit_rate,
            aacSamplesPerFrame: s.aac_samples_per_frame,
            aacNumChannels: s.aac_num_channels,
            extAot: s.ext_aot,
            extSamplingRate: s.ext_sampling_rate,
            outputDelay: s.output_delay,
            flags: s.flags,
            epConfig: s.ep_config,
            numLostAccessUnits: s.num_lost_access_units,
            numTotalBytes: s.num_total_bytes,
            numBadBytes: s.num_bad_bytes,
            numTotalAccessUnits: s.num_total_access_units,
            numBadAccessUnits: s.num_bad_access_units,
            drcProgRefLev: s.drc_prog_ref_lev,
            drcPresMode: s.drc_pres_mode,
            outputLoudness: s.output_loudness,
        };
    }
}

/// Runs `f` on the decoder of the open handle `h` and updates the stream information. Returns
/// `failed` if the handle failed earlier or `f` panics (the handle is then marked as failed).
///
/// # Safety
/// `h` must be an open handle.
unsafe fn with_decoder<R: Copy>(
    h: HANDLE_AACDECODER,
    failed: R,
    f: impl FnOnce(&mut AacDecoderInstance) -> R,
) -> R {
    let r = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the caller's guarantee.
        let inst = unsafe { &mut *h };
        inst.decoder().map(f)
    }));
    // SAFETY: the caller's guarantee.
    unsafe {
        match r {
            Ok(Some(r)) => {
                // (the synchronization joins the frame pipeline's worker, which can propagate
                // a panic of a worker task)
                if catch_unwind(AssertUnwindSafe(|| sync_stream_info(h))).is_err() {
                    (*h).failed = true;
                    return failed;
                }
                r
            }
            Ok(None) => failed,
            Err(_) => {
                (*h).failed = true;
                failed
            }
        }
    }
}

/// `nrOfLayers` of the decoder (the number of entries of the per-layer arrays), 0 for a failed
/// handle.
///
/// # Safety
/// `h` must be an open handle.
unsafe fn layers(h: HANDLE_AACDECODER) -> usize {
    // SAFETY: the caller's guarantee.
    let inst = unsafe { &mut *h };
    inst.decoder().map_or(0, |d| d.nr_of_layers.max(0) as usize)
}

/// A slice of `len` elements at `p`; empty for NULL or a non-positive length.
///
/// # Safety
/// `p` must be NULL or valid for `len` elements for the lifetime `'a`.
unsafe fn slice<'a, T>(p: *const T, len: usize) -> &'a [T] {
    if p.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: the caller's guarantee.
        unsafe { core::slice::from_raw_parts(p, len) }
    }
}

/// `AAC_DECODER_ERROR aacDecoder_AncDataInit(HANDLE_AACDECODER self, UCHAR *buffer, int size)`
///
/// # Safety
/// `self_` NULL or an open handle; `buffer` NULL or valid for writes of `size` bytes while it is
/// registered (until the next `aacDecoder_AncDataInit()` or `aacDecoder_Close()`).
#[no_mangle]
pub unsafe extern "C" fn aacDecoder_AncDataInit(
    self_: HANDLE_AACDECODER,
    buffer: *mut c_uchar,
    size: c_int,
) -> AAC_DECODER_ERROR {
    if self_.is_null() {
        return lib::AAC_DEC_INVALID_HANDLE;
    }
    // SAFETY: the caller's guarantees.
    unsafe {
        with_decoder(self_, lib::AAC_DEC_ANC_DATA_ERROR, |d| {
            lib::aac_decoder_anc_data_init_raw(d, buffer, size)
        })
    }
}

/// `AAC_DECODER_ERROR aacDecoder_AncDataGet(HANDLE_AACDECODER self, int index, UCHAR **ptr, int
/// *size)`
///
/// # Safety
/// `self_` NULL or an open handle; `ptr` and `size` NULL or valid for writes.
#[no_mangle]
pub unsafe extern "C" fn aacDecoder_AncDataGet(
    self_: HANDLE_AACDECODER,
    index: c_int,
    ptr: *mut *mut c_uchar,
    size: *mut c_int,
) -> AAC_DECODER_ERROR {
    if self_.is_null() {
        return lib::AAC_DEC_INVALID_HANDLE;
    }
    // SAFETY: the caller's guarantees.
    unsafe {
        let (err, out, n) =
            with_decoder(self_, (lib::AAC_DEC_ANC_DATA_ERROR, null_mut(), 0), |d| {
                let mut p: Option<usize> = None;
                let mut n: c_int = 0;
                let err = lib::aac_decoder_anc_data_get(d, index, &mut p, &mut n);
                // &ancData->buffer[offset]: an offset into the registered buffer (NULL + 0
                // without one).
                let base = match &d.anc_data.buffer {
                    AncBuffer::Raw(b) => b.as_ptr(),
                    _ => null_mut(),
                };
                (err, p.map_or(null_mut(), |o| base.wrapping_add(o)), n)
            });
        if !ptr.is_null() {
            *ptr = out;
        }
        if !size.is_null() {
            *size = n;
        }
        err
    }
}

/// `AAC_DECODER_ERROR aacDecoder_SetParam(const HANDLE_AACDECODER self, const AACDEC_PARAM
/// param, const INT value)`
///
/// # Safety
/// `self_` NULL or an open handle.
#[no_mangle]
pub unsafe extern "C" fn aacDecoder_SetParam(
    self_: HANDLE_AACDECODER,
    param: AACDEC_PARAM,
    value: c_int,
) -> AAC_DECODER_ERROR {
    if self_.is_null() {
        return catch_unwind(|| lib::aac_decoder_set_param(None, param, value))
            .unwrap_or(lib::AAC_DEC_SET_PARAM_FAIL);
    }
    // SAFETY: the caller's guarantee.
    unsafe {
        with_decoder(self_, lib::AAC_DEC_SET_PARAM_FAIL, |d| {
            lib::aac_decoder_set_param(Some(d), param, value)
        })
    }
}

/// `AAC_DECODER_ERROR aacDecoder_GetFreeBytes(const HANDLE_AACDECODER self, UINT *pFreeBytes)`
///
/// # Safety
/// `self_` NULL or an open handle; `pFreeBytes` NULL or valid for writes.
#[no_mangle]
pub unsafe extern "C" fn aacDecoder_GetFreeBytes(
    self_: HANDLE_AACDECODER,
    pFreeBytes: *mut c_uint,
) -> AAC_DECODER_ERROR {
    let (err, free) = if self_.is_null() {
        let mut free: c_uint = 0;
        (lib::aac_decoder_get_free_bytes(None, &mut free), free)
    } else {
        // SAFETY: the caller's guarantee.
        unsafe {
            with_decoder(self_, (lib::AAC_DEC_UNKNOWN, 0), |d| {
                let mut free: c_uint = 0;
                (lib::aac_decoder_get_free_bytes(Some(d), &mut free), free)
            })
        }
    };
    if !pFreeBytes.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { *pFreeBytes = free };
    }
    err
}

/// `HANDLE_AACDECODER aacDecoder_Open(TRANSPORT_TYPE transportFmt, UINT nrOfLayers)`
#[no_mangle]
pub extern "C" fn aacDecoder_Open(
    transportFmt: TRANSPORT_TYPE,
    nrOfLayers: c_uint,
) -> HANDLE_AACDECODER {
    let r = catch_unwind(|| {
        let mut dec = lib::aac_decoder_open(transportFmt, nrOfLayers)?;
        // The opt-in frame pipelining of the port (not in C): FDK_AAC_RS_DEC_THREADS or
        // FDK_AAC_RS_THREADS, default 1.
        let threads = fdk_aac_rs_fdk::par::env_dec_threads().min(u32::MAX as usize) as u32;
        if threads > 1 {
            lib::aac_decoder_set_threads(&mut dec, threads);
        }
        let h = Box::into_raw(Box::new(AAC_DECODER_INSTANCE {
            dec: Some(dec),
            failed: false,
            info: CStreamInfo {
                sampleRate: 0,
                frameSize: 0,
                numChannels: 0,
                pChannelType: null_mut(),
                pChannelIndices: null_mut(),
                aacSampleRate: 0,
                profile: 0,
                aot: 0,
                channelConfig: 0,
                bitRate: 0,
                aacSamplesPerFrame: 0,
                aacNumChannels: 0,
                extAot: 0,
                extSamplingRate: 0,
                outputDelay: 0,
                flags: 0,
                epConfig: 0,
                numLostAccessUnits: 0,
                numTotalBytes: 0,
                numBadBytes: 0,
                numTotalAccessUnits: 0,
                numBadAccessUnits: 0,
                drcProgRefLev: 0,
                drcPresMode: 0,
                outputLoudness: 0,
            },
            channel_type: [0; STREAM_INFO_CHANNELS],
            channel_indices: [0; STREAM_INFO_CHANNELS],
        }));
        // SAFETY: `h` was just created.
        unsafe { sync_stream_info(h) };
        Some(h)
    });
    match r {
        Ok(Some(h)) => h,
        _ => null_mut(),
    }
}

/// `AAC_DECODER_ERROR aacDecoder_ConfigRaw(HANDLE_AACDECODER self, UCHAR *conf[], const UINT
/// length[])`
///
/// # Safety
/// `self_` NULL or an open handle; `conf` and `length` NULL or arrays with an entry per layer
/// of the decoder; each `conf[layer]` NULL or valid for `length[layer]` bytes.
#[no_mangle]
pub unsafe extern "C" fn aacDecoder_ConfigRaw(
    self_: HANDLE_AACDECODER,
    conf: *mut *mut c_uchar,
    length: *const c_uint,
) -> AAC_DECODER_ERROR {
    if self_.is_null() {
        return lib::AAC_DEC_INVALID_HANDLE;
    }
    // SAFETY: the caller's guarantees.
    unsafe {
        let n = layers(self_);
        if n > 0 && (conf.is_null() || length.is_null()) {
            return lib::AAC_DEC_UNKNOWN;
        }
        let length: &[c_uint] = slice(length, n);
        let conf_ptrs: &[*mut c_uchar] = slice(conf, n);
        let conf: Vec<&[u8]> =
            conf_ptrs.iter().zip(length).map(|(&p, &l)| slice(p, l as usize)).collect();
        with_decoder(self_, lib::AAC_DEC_UNKNOWN, |d| lib::aac_decoder_config_raw(d, &conf, length))
    }
}

/// The implementation of `aacDecoder_RawISOBMFFData()`, whose symbol is defined in assembly
/// (see the module documentation).
///
/// # Safety
/// `self_` NULL or an open handle; `buffer` NULL or valid for `length` bytes.
pub unsafe extern "C" fn raw_isobmff_data(
    self_: HANDLE_AACDECODER,
    buffer: *mut c_uchar,
    length: c_uint,
) -> AAC_DECODER_ERROR {
    if self_.is_null() {
        return lib::AAC_DEC_INVALID_HANDLE;
    }
    // SAFETY: the caller's guarantees.
    unsafe {
        let data: &[u8] = slice(buffer, length as usize);
        with_decoder(self_, lib::AAC_DEC_UNKNOWN, |d| {
            lib::aac_decoder_raw_isobmff_data(d, data, length)
        })
    }
}

// `AAC_DECODER_ERROR aacDecoder_RawISOBMFFData(HANDLE_AACDECODER self, UCHAR *buffer, UINT
// length)`: a global label that branches to `raw_isobmff_data` (tail call, same arguments).
#[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
core::arch::global_asm!(
    ".text",
    ".globl _aacDecoder_RawISOBMFFData",
    ".p2align 2",
    "_aacDecoder_RawISOBMFFData:",
    "b {f}",
    f = sym raw_isobmff_data,
);
#[cfg(all(target_vendor = "apple", target_arch = "x86_64"))]
core::arch::global_asm!(
    ".text",
    ".globl _aacDecoder_RawISOBMFFData",
    ".p2align 4",
    "_aacDecoder_RawISOBMFFData:",
    "jmp {f}",
    f = sym raw_isobmff_data,
);
#[cfg(all(any(target_os = "linux", target_os = "android"), target_arch = "aarch64"))]
core::arch::global_asm!(
    ".text",
    ".globl aacDecoder_RawISOBMFFData",
    ".type aacDecoder_RawISOBMFFData, %function",
    ".p2align 2",
    "aacDecoder_RawISOBMFFData:",
    "b {f}",
    ".size aacDecoder_RawISOBMFFData, . - aacDecoder_RawISOBMFFData",
    f = sym raw_isobmff_data,
);
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    any(target_arch = "x86_64", target_arch = "x86")
))]
core::arch::global_asm!(
    ".text",
    ".globl aacDecoder_RawISOBMFFData",
    ".type aacDecoder_RawISOBMFFData, @function",
    ".p2align 4",
    "aacDecoder_RawISOBMFFData:",
    "jmp {f}",
    ".size aacDecoder_RawISOBMFFData, . - aacDecoder_RawISOBMFFData",
    f = sym raw_isobmff_data,
);

/// `aacDecoder_RawISOBMFFData` on the other targets: an ordinary export, so that the static
/// library has the symbol (the shared library exports it as well, one symbol more than
/// upstream's).
///
/// # Safety
/// See [`raw_isobmff_data`].
#[cfg(not(any(
    all(target_vendor = "apple", any(target_arch = "aarch64", target_arch = "x86_64")),
    all(
        any(target_os = "linux", target_os = "android"),
        any(target_arch = "aarch64", target_arch = "x86_64", target_arch = "x86")
    )
)))]
#[no_mangle]
pub unsafe extern "C" fn aacDecoder_RawISOBMFFData(
    self_: HANDLE_AACDECODER,
    buffer: *mut c_uchar,
    length: c_uint,
) -> AAC_DECODER_ERROR {
    // SAFETY: the caller's guarantee.
    unsafe { raw_isobmff_data(self_, buffer, length) }
}

/// `AAC_DECODER_ERROR aacDecoder_Fill(HANDLE_AACDECODER self, UCHAR *pBuffer[], const UINT
/// bufferSize[], UINT *bytesValid)`
///
/// # Safety
/// `self_` NULL or an open handle; `pBuffer`, `bufferSize` and `bytesValid` NULL or arrays with
/// an entry per layer of the decoder (`bytesValid` writable); each `pBuffer[layer]` NULL or
/// valid for `bufferSize[layer]` bytes.
#[no_mangle]
pub unsafe extern "C" fn aacDecoder_Fill(
    self_: HANDLE_AACDECODER,
    pBuffer: *mut *mut c_uchar,
    bufferSize: *const c_uint,
    bytesValid: *mut c_uint,
) -> AAC_DECODER_ERROR {
    if self_.is_null() {
        return lib::AAC_DEC_INVALID_HANDLE;
    }
    // SAFETY: the caller's guarantees.
    unsafe {
        let n = layers(self_);
        if n > 0 && (pBuffer.is_null() || bufferSize.is_null() || bytesValid.is_null()) {
            return lib::AAC_DEC_UNKNOWN;
        }
        // (copied first: C allows bufferSize and bytesValid to be the same array)
        let size: Vec<c_uint> = slice(bufferSize, n).to_vec();
        let ptrs: &[*mut c_uchar] = slice(pBuffer, n);
        let bufs: Vec<&[u8]> =
            ptrs.iter().zip(&size).map(|(&p, &l)| slice(p, l as usize)).collect();
        let valid: &mut [c_uint] =
            if n == 0 { &mut [] } else { core::slice::from_raw_parts_mut(bytesValid, n) };
        with_decoder(self_, lib::AAC_DEC_UNKNOWN, |d| lib::aac_decoder_fill(d, &bufs, &size, valid))
    }
}

/// `AAC_DECODER_ERROR aacDecoder_DecodeFrame(HANDLE_AACDECODER self, INT_PCM *pTimeData, const
/// INT timeDataSize, const UINT flags)`
///
/// # Safety
/// `self_` NULL or an open handle; `pTimeData` NULL or valid for writes of `timeDataSize`
/// samples.
#[no_mangle]
pub unsafe extern "C" fn aacDecoder_DecodeFrame(
    self_: HANDLE_AACDECODER,
    pTimeData: *mut INT_PCM,
    timeDataSize: c_int,
    flags: c_uint,
) -> AAC_DECODER_ERROR {
    let n = timeDataSize.max(0) as usize;
    // SAFETY: the caller's guarantee.
    let pcm: &mut [INT_PCM] = if pTimeData.is_null() || n == 0 {
        &mut []
    } else {
        unsafe { core::slice::from_raw_parts_mut(pTimeData, n) }
    };
    if self_.is_null() {
        return lib::aac_decoder_decode_frame(None, pcm, timeDataSize, flags);
    }
    let pcm_len = pcm.len();
    let mut returned = false;
    // SAFETY: the caller's guarantee.
    let err = unsafe {
        with_decoder(self_, lib::AAC_DEC_UNKNOWN, |d| {
            let err = lib::aac_decoder_decode_frame(Some(d), pcm, timeDataSize, flags);
            returned = true;
            err
        })
    };
    if !returned && pcm_len > 0 {
        // After a panic or on a failed handle: the result of a call without valid output.
        // SAFETY: `pTimeData` is valid for `pcm_len` samples (the caller's guarantee); the slice
        // given to the straight port is no longer used.
        unsafe { core::slice::from_raw_parts_mut(pTimeData, pcm_len).fill(0) };
    }
    err
}

/// `void aacDecoder_Close(HANDLE_AACDECODER self)`
///
/// # Safety
/// `self_` NULL or an open handle, which is invalid afterwards.
#[no_mangle]
pub unsafe extern "C" fn aacDecoder_Close(self_: HANDLE_AACDECODER) {
    if self_.is_null() {
        return;
    }
    // SAFETY: the caller's guarantee; the handle came from Box::into_raw in aacDecoder_Open.
    let mut inst = unsafe { Box::from_raw(self_) };
    let dec = inst.dec.take();
    let failed = inst.failed;
    drop(inst);
    let _ = catch_unwind(AssertUnwindSafe(move || {
        if failed {
            // The state may be inconsistent: release the memory without the close sequence.
            drop(dec);
        } else {
            lib::aac_decoder_close(dec);
        }
    }));
}

/// `CStreamInfo *aacDecoder_GetStreamInfo(HANDLE_AACDECODER self)`
///
/// # Safety
/// `self_` NULL or an open handle.
#[no_mangle]
pub unsafe extern "C" fn aacDecoder_GetStreamInfo(self_: HANDLE_AACDECODER) -> *mut CStreamInfo {
    if self_.is_null() {
        return null_mut();
    }
    // SAFETY: the caller's guarantee.
    unsafe { addr_of_mut!((*self_).info) }
}

/// `INT aacDecoder_GetLibInfo(LIB_INFO *info)`
///
/// # Safety
/// `info` NULL or an array of `FDK_MODULE_LAST` entries, as the C API requires.
#[no_mangle]
pub unsafe extern "C" fn aacDecoder_GetLibInfo(info: *mut LIB_INFO) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if info.is_null() {
            return lib::aac_decoder_get_lib_info(None);
        }
        // SAFETY: the caller's guarantee.
        unsafe { with_lib_info(info, |s| lib::aac_decoder_get_lib_info(Some(s))) }
    }))
    .unwrap_or(-1)
}
