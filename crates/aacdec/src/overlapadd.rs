// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/overlapadd.h.

//! Port of `overlapadd.h`: the spectrum types and the spectrum addressing macros.
//!
//! `SPECTRAL_PTR` (`FIXP_DBL *`) is a slice in the port (see the ownership rules in
//! [`crate::channelinfo`]); the macros `SPEC_LONG`, `SPEC` and `SPEC_TCX`, which add an offset to
//! such a pointer, become functions that return the offset ([`spec_long`], [`spec`],
//! [`spec_tcx`]), plus slice helpers that never panic ([`spec_slice`], [`spec_slice_mut`]).

use fdk_aac_rs_fdk::common_fix::FixpDbl;

/// `OverlapBufferSize`: ELD uses different overlap which is twice the frame size.
pub const OverlapBufferSize: i32 = 768;

/// `SPECTRUM`: `FIXP_DBL[1024]`
pub type Spectrum = [FixpDbl; 1024];

/// `SPEC_LONG(ptr)`: the offset of the long window spectrum (0).
#[inline(always)]
pub const fn spec_long() -> usize {
    0
}

/// `SPEC(ptr, w, gl)`: the offset `w * gl` of window `w` with granule length `gl`.
#[inline(always)]
pub const fn spec(w: i32, gl: i32) -> usize {
    (w * gl) as usize
}

/// `SPEC_TCX(ptr, f, gl, fb)`: the offset `f * (gl * 2) * (fb == 0 ? 1 : 2)` of TCX frame `f`.
#[inline(always)]
pub const fn spec_tcx(f: i32, gl: i32, fb: i32) -> usize {
    (f * (gl * 2) * (if fb == 0 { 1 } else { 2 })) as usize
}

/// `ptr + offset` as a slice: the rest of `ptr` from `offset` (empty if `offset` is beyond the
/// end).
#[inline(always)]
pub fn spec_slice(ptr: &[FixpDbl], offset: usize) -> &[FixpDbl] {
    ptr.get(offset..).unwrap_or(&[])
}

/// `ptr + offset` as a mutable slice: the rest of `ptr` from `offset` (empty if `offset` is
/// beyond the end).
#[inline(always)]
pub fn spec_slice_mut(ptr: &mut [FixpDbl], offset: usize) -> &mut [FixpDbl] {
    match ptr.get_mut(offset..) {
        Some(s) => s,
        None => &mut [],
    }
}
