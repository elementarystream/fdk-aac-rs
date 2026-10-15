// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/fixmadd.h, libFDK/include/arm/fixmadd_arm.h.

//! Port of `fixmadd.h`: fixed point multiply-accumulate.
//!
//! On `aarch64`, `fixmadddiv2_DD` and `fixmsubdiv2_DD` are inline assembly (`smull`,
//! `asr #32`, then a 32 bit `add`/`sub`), which is `x ± fixmuldiv2_DD(a, b)` in wrapping 32 bit
//! arithmetic, the same value as the generic code of `x86`. fdk-aac has no
//! `fixmadd_DD` assembly: `fMultAdd()` is `fixmadddiv2_DD(x, a, b) << 1` on every variant.
//! The `BitExact` variants are the same functions on every variant (macros or generic code).

use crate::common_fix::{FixpDbl, FixpSgl};
use crate::fixmul::*;

/// `fixmadddiv2_DD(x, a, b)`: `x + fMultDiv2(a, b)`.
#[inline(always)]
pub const fn fixmadddiv2_dd(x: FixpDbl, a: FixpDbl, b: FixpDbl) -> FixpDbl {
    x.wrapping_add(fixmuldiv2_dd(a, b))
}

/// `fixmadddiv2_SD(x, a, b)`: `fixmadddiv2_DD(x, FX_SGL2FX_DBL(a), b)`.
#[inline(always)]
pub const fn fixmadddiv2_sd(x: FixpDbl, a: FixpSgl, b: FixpDbl) -> FixpDbl {
    fixmadddiv2_dd(x, (a as i32) << 16, b)
}

/// `fixmadddiv2_DS(x, a, b)`: `fixmadddiv2_DD(x, a, FX_SGL2FX_DBL(b))`.
#[inline(always)]
pub const fn fixmadddiv2_ds(x: FixpDbl, a: FixpDbl, b: FixpSgl) -> FixpDbl {
    fixmadddiv2_dd(x, a, (b as i32) << 16)
}

/// `fixmadddiv2_SS(x, a, b)`: `x + fMultDiv2(a, b)` (`a * b`).
#[inline(always)]
pub const fn fixmadddiv2_ss(x: FixpDbl, a: FixpSgl, b: FixpSgl) -> FixpDbl {
    x.wrapping_add(fixmuldiv2_ss(a, b))
}

/// `fixmsubdiv2_DD(x, a, b)`: `x - fMultDiv2(a, b)`.
#[inline(always)]
pub const fn fixmsubdiv2_dd(x: FixpDbl, a: FixpDbl, b: FixpDbl) -> FixpDbl {
    x.wrapping_sub(fixmuldiv2_dd(a, b))
}

/// `fixmsubdiv2_SD(x, a, b)`
#[inline(always)]
pub const fn fixmsubdiv2_sd(x: FixpDbl, a: FixpSgl, b: FixpDbl) -> FixpDbl {
    fixmsubdiv2_dd(x, (a as i32) << 16, b)
}

/// `fixmsubdiv2_DS(x, a, b)`
#[inline(always)]
pub const fn fixmsubdiv2_ds(x: FixpDbl, a: FixpDbl, b: FixpSgl) -> FixpDbl {
    fixmsubdiv2_dd(x, a, (b as i32) << 16)
}

/// `fixmsubdiv2_SS(x, a, b)`
#[inline(always)]
pub const fn fixmsubdiv2_ss(x: FixpDbl, a: FixpSgl, b: FixpSgl) -> FixpDbl {
    x.wrapping_sub(fixmuldiv2_ss(a, b))
}

/// `fixmadddiv2BitExact_DD(x, a, b)`: `x + fMultDiv2BitExact(a, b)`.
#[inline(always)]
pub const fn fixmadddiv2_bit_exact_dd(x: FixpDbl, a: FixpDbl, b: FixpDbl) -> FixpDbl {
    x.wrapping_add(fixmuldiv2_bit_exact_dd(a, b))
}

/// `fixmadddiv2BitExact_SD(x, a, b)`
#[inline(always)]
pub const fn fixmadddiv2_bit_exact_sd(x: FixpDbl, a: FixpSgl, b: FixpDbl) -> FixpDbl {
    x.wrapping_add(fixmuldiv2_bit_exact_sd(a, b))
}

/// `fixmadddiv2BitExact_DS(x, a, b)`
#[inline(always)]
pub const fn fixmadddiv2_bit_exact_ds(x: FixpDbl, a: FixpDbl, b: FixpSgl) -> FixpDbl {
    x.wrapping_add(fixmuldiv2_bit_exact_ds(a, b))
}

/// `fixmsubdiv2BitExact_DD(x, a, b)`: `x - fMultDiv2BitExact(a, b)`.
#[inline(always)]
pub const fn fixmsubdiv2_bit_exact_dd(x: FixpDbl, a: FixpDbl, b: FixpDbl) -> FixpDbl {
    x.wrapping_sub(fixmuldiv2_bit_exact_dd(a, b))
}

/// `fixmsubdiv2BitExact_SD(x, a, b)`
#[inline(always)]
pub const fn fixmsubdiv2_bit_exact_sd(x: FixpDbl, a: FixpSgl, b: FixpDbl) -> FixpDbl {
    x.wrapping_sub(fixmuldiv2_bit_exact_sd(a, b))
}

/// `fixmsubdiv2BitExact_DS(x, a, b)`
#[inline(always)]
pub const fn fixmsubdiv2_bit_exact_ds(x: FixpDbl, a: FixpDbl, b: FixpSgl) -> FixpDbl {
    x.wrapping_sub(fixmuldiv2_bit_exact_ds(a, b))
}

/// `fixmadd_DD(x, a, b)`: `fixmadddiv2_DD(x, a, b) << 1`.
#[inline(always)]
pub const fn fixmadd_dd(x: FixpDbl, a: FixpDbl, b: FixpDbl) -> FixpDbl {
    fixmadddiv2_dd(x, a, b) << 1
}

/// `fixmadd_SD(x, a, b)`: `fixmadd_DD(x, FX_SGL2FX_DBL(a), b)`.
#[inline(always)]
pub const fn fixmadd_sd(x: FixpDbl, a: FixpSgl, b: FixpDbl) -> FixpDbl {
    fixmadd_dd(x, (a as i32) << 16, b)
}

/// `fixmadd_DS(x, a, b)`: `fixmadd_DD(x, a, FX_SGL2FX_DBL(b))`.
#[inline(always)]
pub const fn fixmadd_ds(x: FixpDbl, a: FixpDbl, b: FixpSgl) -> FixpDbl {
    fixmadd_dd(x, a, (b as i32) << 16)
}

/// `fixmadd_SS(x, a, b)`: `(x + fMultDiv2(a, b)) << 1`.
#[inline(always)]
pub const fn fixmadd_ss(x: FixpDbl, a: FixpSgl, b: FixpSgl) -> FixpDbl {
    x.wrapping_add(fixmuldiv2_ss(a, b)) << 1
}

/// `fixmsub_DD(x, a, b)`: `fixmsubdiv2_DD(x, a, b) << 1`.
#[inline(always)]
pub const fn fixmsub_dd(x: FixpDbl, a: FixpDbl, b: FixpDbl) -> FixpDbl {
    fixmsubdiv2_dd(x, a, b) << 1
}

/// `fixmsub_SD(x, a, b)`
#[inline(always)]
pub const fn fixmsub_sd(x: FixpDbl, a: FixpSgl, b: FixpDbl) -> FixpDbl {
    fixmsub_dd(x, (a as i32) << 16, b)
}

/// `fixmsub_DS(x, a, b)`
#[inline(always)]
pub const fn fixmsub_ds(x: FixpDbl, a: FixpDbl, b: FixpSgl) -> FixpDbl {
    fixmsub_dd(x, a, (b as i32) << 16)
}

/// `fixmsub_SS(x, a, b)`: `(x - fMultDiv2(a, b)) << 1`.
#[inline(always)]
pub const fn fixmsub_ss(x: FixpDbl, a: FixpSgl, b: FixpSgl) -> FixpDbl {
    x.wrapping_sub(fixmuldiv2_ss(a, b)) << 1
}

/// `fixpadddiv2_D(x, a)`: `x + fPow2Div2(a)` (`fixmadddiv2_DD(x, a, a)` on `aarch64`).
#[inline(always)]
pub const fn fixpadddiv2_d(x: FixpDbl, a: FixpDbl) -> FixpDbl {
    x.wrapping_add(fixpow2div2_d(a))
}

/// `fixpadd_D(x, a)`: `x + fPow2(a)`.
#[inline(always)]
pub const fn fixpadd_d(x: FixpDbl, a: FixpDbl) -> FixpDbl {
    x.wrapping_add(fixpow2_d(a))
}

/// `fixpadddiv2_S(x, a)`: `x + fPow2Div2(a)`.
#[inline(always)]
pub const fn fixpadddiv2_s(x: FixpDbl, a: FixpSgl) -> FixpDbl {
    x.wrapping_add(fixpow2div2_s(a))
}

/// `fixpadd_S(x, a)`: `x + fPow2(a)`.
#[inline(always)]
pub const fn fixpadd_s(x: FixpDbl, a: FixpSgl) -> FixpDbl {
    x.wrapping_add(fixpow2_s(a))
}
