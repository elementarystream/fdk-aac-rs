// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/include/aacenc_lib.h (the C API).

//! Encoder part of the C ABI: `aacEnc*`, with the signatures of `aacenc_lib.h`.
//!
//! Each function passes its arguments to the straight port
//! ([`fdk_aac_rs_aacenc::aacenc_lib`]) unchanged: `HANDLE_AACENCODER` is a pointer to the
//! port's [`AacEncoder`] (a `Box` released by `aacEncClose()`), NULL pointers become `None`,
//! and the buffer descriptors and argument structures, which have the C layout in the port, are
//! passed through as they are. The NULL handling is therefore that of the reference:
//!
//! | Function | NULL argument | Result |
//! |---|---|---|
//! | `aacEncOpen` | `phAacEncoder` | `AACENC_INVALID_HANDLE` |
//! | `aacEncClose` | `phAacEncoder` | `AACENC_INVALID_HANDLE`; `*phAacEncoder == NULL`: `AACENC_OK` |
//! | `aacEncEncode` | `hAacEncoder` | `AACENC_INVALID_HANDLE` |
//! | `aacEncEncode` | all four buffer arguments | (re)initialization only, as in C |
//! | `aacEncEncode` | `inargs` or `outargs` (not all four) | `AACENC_UNSUPPORTED_PARAMETER` |
//! | `aacEncInfo` | either argument | `AACENC_INVALID_HANDLE` |
//! | `aacEncoder_SetParam` | `hAacEncoder` | `AACENC_INVALID_HANDLE` |
//! | `aacEncoder_GetParam` | `hAacEncoder` | 0 |
//! | `aacEncGetLibInfo` | `info` | `AACENC_INVALID_HANDLE` |
//!
//! `aacEncGetLibInfo()` converts the `LIB_INFO` array with [`with_lib_info`], which writes back
//! only the fields the library wrote.
//!
//! # Threads
//!
//! `aacEncOpen()` sets the thread count of the port's opt-in multi-threading from the
//! environment variable `FDK_AAC_RS_ENC_THREADS`, or `FDK_AAC_RS_THREADS` if that is not set (a
//! positive integer; unset, invalid or 0: 1, the single-threaded encoder). The output is identical for every thread count (see
//! [`fdk_aac_rs_aacenc::aacenc_lib::aac_enc_set_threads`]). A panic of a task on a worker
//! thread is propagated to the calling thread and caught there as described below.
//!
//! With more than one thread, an encoder with SBR (HE-AAC, HE-AAC v2, ELD with SBR) delays its
//! output by one call, so that it can encode the SBR data of a frame while the core encodes
//! the previous one: from the first `aacEncEncode()` call with SBR active, each call with
//! output arguments returns the return code, `numOutBytes`, `numAncBytes`, `bitResState` and
//! output buffer contents of the previous call (the first one: an empty result), while
//! `numInSamples` is its own. The sequence of results is that of the single-threaded encoder;
//! `AACENC_ENCODE_EOF` comes one call later, so callers that flush until `AACENC_ENCODE_EOF`
//! (`aac-enc`, FFmpeg) get every access unit. Details:
//! [`fdk_aac_rs_aacenc::aacenc_lib::aac_enc_get_output_delay`] and the module documentation
//! of `pipeline` in that crate.
//!
//! # Panics
//!
//! No panic crosses the FFI boundary: every function runs the straight port inside
//! [`std::panic::catch_unwind`]. The straight port has no known panicking path (its policy is
//! "no panics on any input", PORTING.md); the guard turns a panic that would still occur
//! into an error code instead of an abort:
//!
//! | Function | Result after a panic |
//! |---|---|
//! | `aacEncOpen` | `AACENC_MEMORY_ERROR`, `*phAacEncoder` unchanged |
//! | `aacEncClose` | `AACENC_INVALID_HANDLE`, `*phAacEncoder = NULL` |
//! | `aacEncEncode` | `AACENC_ENCODE_ERROR`, and the encoder is marked for a full reinitialization (`AACENC_INIT_ALL`), as after an encoding error in C; after a panic in a task of the SBR pipelining (more than one thread) the encoder's modules are lost and every later call returns an error |
//! | `aacEncInfo` | `AACENC_INIT_ERROR` |
//! | `aacEncoder_SetParam` | `AACENC_INVALID_CONFIG` |
//! | `aacEncoder_GetParam` | 0 |
//! | `aacEncGetLibInfo` | `AACENC_INIT_ERROR` |
//!
//! `catch_unwind` is used instead of proving every path panic-free because the straight port is
//! large and its absence of panics is established by testing, not by construction. The
//! default panic hook still prints the panic message to stderr. In a build with
//! `panic = "abort"` a panic aborts the process instead (it still does not unwind into C).

use crate::lib_info::{with_lib_info, LIB_INFO};
use core::ffi::{c_int, c_uint};
use core::panic::AssertUnwindSafe;
use fdk_aac_rs_aacenc::aacenc_lib::{self as lib, AacEncoder};
use std::panic::catch_unwind;

/// `HANDLE_AACENCODER`
pub type HANDLE_AACENCODER = *mut AacEncoder;
/// `AACENC_ERROR`
pub type AACENC_ERROR = lib::AacencError;
/// `AACENC_PARAM`
pub type AACENC_PARAM = c_int;
/// `AACENC_BufDesc`
pub type AACENC_BufDesc = lib::AacencBufDesc;
/// `AACENC_InArgs`
pub type AACENC_InArgs = lib::AacencInArgs;
/// `AACENC_OutArgs`
pub type AACENC_OutArgs = lib::AacencOutArgs;
/// `AACENC_InfoStruct`
pub type AACENC_InfoStruct = lib::AacencInfoStruct;
/// `AACENC_MetaData`
pub type AACENC_MetaData = lib::AacencMetaData;
/// `AACENC_ExtMetaData`
pub type AACENC_ExtMetaData = lib::AacencExtMetaData;

/// Runs `f`, or returns `on_panic` if it panics.
fn guard<R>(on_panic: R, f: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(on_panic)
}

/// `AACENC_ERROR aacEncOpen(HANDLE_AACENCODER *phAacEncoder, const UINT encModules, const UINT
/// maxChannels)`
///
/// # Safety
/// `phAacEncoder` must be NULL or valid for writes.
#[no_mangle]
pub unsafe extern "C" fn aacEncOpen(
    phAacEncoder: *mut HANDLE_AACENCODER,
    encModules: c_uint,
    maxChannels: c_uint,
) -> AACENC_ERROR {
    guard(lib::AACENC_MEMORY_ERROR, || {
        if phAacEncoder.is_null() {
            return lib::aac_enc_open(None, encModules, maxChannels);
        }
        let mut h = None;
        let err = lib::aac_enc_open(Some(&mut h), encModules, maxChannels);
        // C writes the handle on success only.
        if let Some(mut h) = h {
            // The opt-in multi-threading of the port (not in C): FDK_AAC_RS_ENC_THREADS or
            // FDK_AAC_RS_THREADS, default 1.
            lib::aac_enc_set_threads(&mut h, fdk_aac_rs_fdk::par::env_enc_threads() as u32);
            // SAFETY: the caller's guarantee.
            unsafe { *phAacEncoder = Box::into_raw(h) };
        }
        err
    })
}

/// `AACENC_ERROR aacEncClose(HANDLE_AACENCODER *phAacEncoder)`
///
/// # Safety
/// `phAacEncoder` must be NULL or valid for reads and writes, and `*phAacEncoder` NULL or a
/// handle from [`aacEncOpen`] that was not closed.
#[no_mangle]
pub unsafe extern "C" fn aacEncClose(phAacEncoder: *mut HANDLE_AACENCODER) -> AACENC_ERROR {
    let r = catch_unwind(AssertUnwindSafe(|| {
        if phAacEncoder.is_null() {
            return lib::aac_enc_close(None);
        }
        // SAFETY: the caller's guarantee; the handle came from Box::into_raw in aacEncOpen.
        let p = unsafe { *phAacEncoder };
        let mut slot = if p.is_null() { None } else { Some(unsafe { Box::from_raw(p) }) };
        let err = lib::aac_enc_close(Some(&mut slot));
        // aacEncClose() releases the encoder and sets the handle to NULL (Free_AacEncoder()).
        // SAFETY: the caller's guarantee.
        unsafe { *phAacEncoder = slot.map_or(core::ptr::null_mut(), Box::into_raw) };
        err
    }));
    match r {
        Ok(err) => err,
        Err(_) => {
            // The encoder was released while unwinding; do not leave a dangling handle.
            if !phAacEncoder.is_null() {
                // SAFETY: the caller's guarantee.
                unsafe { *phAacEncoder = core::ptr::null_mut() };
            }
            lib::AACENC_INVALID_HANDLE
        }
    }
}

/// `AACENC_ERROR aacEncEncode(const HANDLE_AACENCODER hAacEncoder, const AACENC_BufDesc
/// *inBufDesc, const AACENC_BufDesc *outBufDesc, const AACENC_InArgs *inargs, AACENC_OutArgs
/// *outargs)`
///
/// # Safety
/// As in C: `hAacEncoder` NULL or an open handle; each pointer NULL or valid; the descriptors'
/// arrays valid for `numBufs` entries and their buffers valid for the sizes the API documents
/// (see [`lib::aac_enc_encode`]).
#[no_mangle]
pub unsafe extern "C" fn aacEncEncode(
    hAacEncoder: HANDLE_AACENCODER,
    inBufDesc: *const AACENC_BufDesc,
    outBufDesc: *const AACENC_BufDesc,
    inargs: *const AACENC_InArgs,
    outargs: *mut AACENC_OutArgs,
) -> AACENC_ERROR {
    let r = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the caller's guarantee.
        unsafe {
            lib::aac_enc_encode(
                hAacEncoder.as_mut(),
                inBufDesc.as_ref(),
                outBufDesc.as_ref(),
                inargs.as_ref(),
                outargs.as_mut(),
            )
        }
    }));
    match r {
        Ok(err) => err,
        Err(_) => {
            // SAFETY: the caller's guarantee.
            if let Some(h) = unsafe { hAacEncoder.as_mut() } {
                h.init_flags = lib::AACENC_INIT_ALL;
            }
            lib::AACENC_ENCODE_ERROR
        }
    }
}

/// `AACENC_ERROR aacEncInfo(const HANDLE_AACENCODER hAacEncoder, AACENC_InfoStruct *pInfo)`
///
/// # Safety
/// `hAacEncoder` NULL or an open handle; `pInfo` NULL or valid for writes.
#[no_mangle]
pub unsafe extern "C" fn aacEncInfo(
    hAacEncoder: HANDLE_AACENCODER,
    pInfo: *mut AACENC_InfoStruct,
) -> AACENC_ERROR {
    guard(lib::AACENC_INIT_ERROR, || {
        // SAFETY: the caller's guarantee.
        unsafe { lib::aac_enc_info(hAacEncoder.as_mut(), pInfo.as_mut()) }
    })
}

/// `AACENC_ERROR aacEncoder_SetParam(const HANDLE_AACENCODER hAacEncoder, const AACENC_PARAM
/// param, const UINT value)`
///
/// # Safety
/// `hAacEncoder` NULL or an open handle.
#[no_mangle]
pub unsafe extern "C" fn aacEncoder_SetParam(
    hAacEncoder: HANDLE_AACENCODER,
    param: AACENC_PARAM,
    value: c_uint,
) -> AACENC_ERROR {
    guard(lib::AACENC_INVALID_CONFIG, || {
        // SAFETY: the caller's guarantee.
        lib::aac_encoder_set_param(unsafe { hAacEncoder.as_mut() }, param, value)
    })
}

/// `UINT aacEncoder_GetParam(const HANDLE_AACENCODER hAacEncoder, const AACENC_PARAM param)`
///
/// # Safety
/// `hAacEncoder` NULL or an open handle.
#[no_mangle]
pub unsafe extern "C" fn aacEncoder_GetParam(
    hAacEncoder: HANDLE_AACENCODER,
    param: AACENC_PARAM,
) -> c_uint {
    guard(0, || {
        // SAFETY: the caller's guarantee.
        lib::aac_encoder_get_param(unsafe { hAacEncoder.as_ref() }, param)
    })
}

/// `AACENC_ERROR aacEncGetLibInfo(LIB_INFO *info)`
///
/// # Safety
/// `info` NULL or an array of `FDK_MODULE_LAST` entries, as the C API requires.
#[no_mangle]
pub unsafe extern "C" fn aacEncGetLibInfo(info: *mut LIB_INFO) -> AACENC_ERROR {
    guard(lib::AACENC_INIT_ERROR, || {
        if info.is_null() {
            return lib::aac_enc_get_lib_info(None);
        }
        // SAFETY: the caller's guarantee.
        unsafe { with_lib_info(info, |s| lib::aac_enc_get_lib_info(Some(s))) }
    })
}
