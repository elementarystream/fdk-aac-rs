// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSYS/include/FDK_audio.h (LIB_INFO).

//! `LIB_INFO` with the C layout, shared by `aacEncGetLibInfo()` and `aacDecoder_GetLibInfo()`.
//!
//! The port's [`LibInfo`] holds `&'static str` where C has `const char *`. [`with_lib_info`]
//! converts the caller's array to the port's type, runs the straight-port function, and writes
//! back exactly the fields the function wrote, so that every field the reference leaves
//! untouched (for example the `flags` of the DRC decoder's entry; PORTING.md, "Definition of
//! bit-exact") keeps the caller's value:
//!
//! - The string fields start as a sentinel; a field whose value is no longer the sentinel was
//!   written, and receives a pointer to a static NUL-terminated copy of the string (the same
//!   pointer for the same string, as with the string literals of C).
//! - The integer fields and the bytes of `versionStr` are written back where they changed.
//!   Writing an unchanged value is not observable.

use core::ffi::{c_char, c_int, c_uint, CStr};
use fdk_aac_rs_fdk::audio::{LibInfo, FDK_MODULE_LAST};
use std::sync::Mutex;

/// `LIB_INFO` (C layout).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LIB_INFO {
    /// `const char *title`
    pub title: *const c_char,
    /// `const char *build_date`
    pub build_date: *const c_char,
    /// `const char *build_time`
    pub build_time: *const c_char,
    /// `FDK_MODULE_ID module_id`
    pub module_id: c_int,
    /// `INT version`
    pub version: c_int,
    /// `UINT flags`
    pub flags: c_uint,
    /// `char versionStr[32]`
    pub versionStr: [c_char; 32],
}

/// The value of a string field that the straight-port function did not write. Its content
/// differs from every string of the library, so its address is unique.
static UNSET: &str = "\u{0}fdk-aac-rs: LIB_INFO string field not written";

fn is_unset(s: &str) -> bool {
    core::ptr::eq(s.as_ptr(), UNSET.as_ptr()) && s.len() == UNSET.len()
}

/// The static NUL-terminated copies of the strings returned so far.
static STRINGS: Mutex<Vec<(&'static str, &'static CStr)>> = Mutex::new(Vec::new());

/// A static NUL-terminated copy of `s` (up to its first NUL), the same pointer for equal
/// strings.
fn c_string(s: &'static str) -> *const c_char {
    // A poisoned lock still holds a consistent list (entries are pushed complete).
    let mut strings = STRINGS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, c)) = strings.iter().find(|(k, _)| *k == s) {
        return c.as_ptr();
    }
    let bytes: Vec<u8> = s.bytes().take_while(|&b| b != 0).chain([0]).collect();
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    let c = CStr::from_bytes_until_nul(leaked).unwrap_or(c"");
    strings.push((s, c));
    c.as_ptr()
}

/// Runs `f` on the `FDK_MODULE_LAST` entries at `info`, converted to [`LibInfo`], and writes
/// back the fields `f` wrote (see the module documentation).
///
/// # Safety
/// `info` must be valid for reads and writes of `FDK_MODULE_LAST` entries, the size of the
/// array the C API requires.
pub unsafe fn with_lib_info<R>(info: *mut LIB_INFO, f: impl FnOnce(&mut [LibInfo]) -> R) -> R {
    const N: usize = FDK_MODULE_LAST as usize;
    let mut before = [LibInfo::default(); N];
    for (i, e) in before.iter_mut().enumerate() {
        // SAFETY: the caller's guarantee.
        let c = unsafe { info.add(i).read() };
        *e = LibInfo {
            title: UNSET,
            build_date: UNSET,
            build_time: UNSET,
            module_id: c.module_id,
            version: c.version,
            flags: c.flags,
            version_str: c.versionStr.map(|b| b as u8),
        };
    }
    let mut after = before;
    let r = f(&mut after);
    for (i, (a, b)) in after.iter().zip(before.iter()).enumerate() {
        // SAFETY: the caller's guarantee.
        let c = unsafe { &mut *info.add(i) };
        if !is_unset(a.title) {
            c.title = c_string(a.title);
        }
        if !is_unset(a.build_date) {
            c.build_date = c_string(a.build_date);
        }
        if !is_unset(a.build_time) {
            c.build_time = c_string(a.build_time);
        }
        if a.module_id != b.module_id {
            c.module_id = a.module_id;
        }
        if a.version != b.version {
            c.version = a.version;
        }
        if a.flags != b.flags {
            c.flags = a.flags;
        }
        for (k, (&x, &y)) in a.version_str.iter().zip(b.version_str.iter()).enumerate() {
            if x != y {
                c.versionStr[k] = x as c_char;
            }
        }
    }
    r
}
