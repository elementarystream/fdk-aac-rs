// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/fixminmax.h.

//! Port of `fixminmax.h`: the `fixmin`/`fixmax` templates and their `_D`, `_S`, `_I` and `_UI`
//! macros. The same on every variant.

/// `fixmin(a, b)` (template): `a < b ? a : b`.
#[inline(always)]
pub fn fixmin<T: PartialOrd>(a: T, b: T) -> T {
    if a < b {
        a
    } else {
        b
    }
}

/// `fixmax(a, b)` (template): `a > b ? a : b`.
#[inline(always)]
pub fn fixmax<T: PartialOrd>(a: T, b: T) -> T {
    if a > b {
        a
    } else {
        b
    }
}

/// `fixmin_D(a, b)`
#[inline(always)]
pub const fn fixmin_d(a: i32, b: i32) -> i32 {
    if a < b {
        a
    } else {
        b
    }
}

/// `fixmax_D(a, b)`
#[inline(always)]
pub const fn fixmax_d(a: i32, b: i32) -> i32 {
    if a > b {
        a
    } else {
        b
    }
}

/// `fixmin_S(a, b)`
#[inline(always)]
pub const fn fixmin_s(a: i16, b: i16) -> i16 {
    if a < b {
        a
    } else {
        b
    }
}

/// `fixmax_S(a, b)`
#[inline(always)]
pub const fn fixmax_s(a: i16, b: i16) -> i16 {
    if a > b {
        a
    } else {
        b
    }
}

/// `fixmin_I(a, b)`
#[inline(always)]
pub const fn fixmin_i(a: i32, b: i32) -> i32 {
    fixmin_d(a, b)
}

/// `fixmax_I(a, b)`
#[inline(always)]
pub const fn fixmax_i(a: i32, b: i32) -> i32 {
    fixmax_d(a, b)
}

/// `fixmin_UI(a, b)`
#[inline(always)]
pub const fn fixmin_ui(a: u32, b: u32) -> u32 {
    if a < b {
        a
    } else {
        b
    }
}

/// `fixmax_UI(a, b)`
#[inline(always)]
pub const fn fixmax_ui(a: u32, b: u32) -> u32 {
    if a > b {
        a
    } else {
        b
    }
}
