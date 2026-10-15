// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/abs.h, libFDK/include/x86/abs_x86.h.

//! Port of `abs.h`: absolute values.
//!
//! The generic code (`aarch64`, `generic`) is `x > 0 ? x : -x`; `x86/abs_x86.h` defines the same
//! expression with `INT` parameters and results. The values are the same on every variant:
//!
//! - `fixabs_D(MINVAL_DBL)` and `fixabs_I(INT_MIN)` negate `INT_MIN`, which is undefined in C and
//!   wraps in the reference builds: the result is `MINVAL_DBL`, not `MAXVAL_DBL`.
//! - `fixabs_S(-32768)` gives `32768` in `int`. The generic version truncates it to `FIXP_SGL`
//!   (`-32768`); the `x86` version returns the `INT` 32768, but its only user, `fAbs(FIXP_SGL)`,
//!   truncates it to `FIXP_SGL` as well. The library calls `fixabs_S` only through `fAbs`.

use crate::common_fix::{FixpDbl, FixpSgl};

/// `fixabs_D(x)`: `x > 0 ? x : -x` (wrapping: `fixabs_D(MINVAL_DBL) == MINVAL_DBL`).
#[inline(always)]
pub const fn fixabs_d(x: FixpDbl) -> FixpDbl {
    if x > 0 {
        x
    } else {
        x.wrapping_neg()
    }
}

/// `fixabs_I(x)`: as [`fixabs_d`].
#[inline(always)]
pub const fn fixabs_i(x: i32) -> i32 {
    fixabs_d(x)
}

/// `fixabs_S(x)` as returned by `fAbs(FIXP_SGL)`: `x > 0 ? x : -x`, truncated to `FIXP_SGL`
/// (`fixabs_S(-32768) == -32768`).
#[inline(always)]
pub const fn fixabs_s(x: FixpSgl) -> FixpSgl {
    if x > 0 {
        x
    } else {
        x.wrapping_neg()
    }
}
