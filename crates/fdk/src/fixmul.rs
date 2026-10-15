// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/fixmul.h, libFDK/include/arm/fixmul_arm.h, libFDK/include/x86/fixmul_x86.h.

//! Port of `fixmul.h`: fixed point multiplications.
//!
//! The variants, as compiled in the reference builds (`.ii` files of `ref-arm64` and
//! `ref-x86_64-mac` / `ref-x86_64`):
//!
//! | Function | `aarch64` (`arm/fixmul_arm.h`) | `x86` (`x86/fixmul_x86.h`) | `generic` |
//! |---|---|---|---|
//! | `fixmuldiv2_DD` | `smull`, `asr #32` | `imul` (high half) | `(a*b) >> 32` |
//! | `fixmul_DD` | `smull`, `asr #31` | `imul`, `shl $1` of the high half | `fixmuldiv2_DD << 1` |
//! | `fixmuldiv2_SD` | `fixmuldiv2_DD((INT)(a << 16), b)` | generic | `fixmuldiv2_DD(FX_SGL2FX_DBL(a), b)` |
//!
//! Only `fixmul_DD` differs in value: `(a*b) >> 31` keeps the bit 31 of the product, which the
//! other variants clear. All other functions, including `fixpow2_D` (which is
//! `fixmuldiv2_DD(a, a) << 1` on every variant, not `fixmul_DD(a, a)`), compute the same values.

use crate::common_fix::{FixpDbl, FixpSgl, ARCH_ARM};

/// `fixmuldiv2_DD(a, b)`: `(a * b) >> 32` (high half of the 64 bit product).
#[inline(always)]
pub const fn fixmuldiv2_dd(a: FixpDbl, b: FixpDbl) -> FixpDbl {
    ((a as i64 * b as i64) >> 32) as i32
}

/// `fixmuldiv2BitExact_DD(a, b)`: the same as [`fixmuldiv2_dd`] on every variant.
#[inline(always)]
pub const fn fixmuldiv2_bit_exact_dd(a: FixpDbl, b: FixpDbl) -> FixpDbl {
    fixmuldiv2_dd(a, b)
}

/// `fixmul_DD(a, b)`.
///
/// `aarch64`: `smull` + `asr #31`, i.e. `(INT)((a * b) >> 31)`. `x86` (`imul` + `shl $1`) and
/// `generic`: `fixmuldiv2_DD(a, b) << 1`, whose bit 0 is always 0.
///
/// `MINVAL_DBL * MINVAL_DBL` gives `MINVAL_DBL` on all variants (the result wraps).
#[inline(always)]
pub const fn fixmul_dd(a: FixpDbl, b: FixpDbl) -> FixpDbl {
    if ARCH_ARM {
        ((a as i64 * b as i64) >> 31) as i32
    } else {
        fixmuldiv2_dd(a, b) << 1
    }
}

/// `fixmulBitExact_DD(a, b)`: `fixmuldiv2_DD(a, b) << 1` on every variant (on `aarch64` this is
/// not `fixmul_DD`).
#[inline(always)]
pub const fn fixmul_bit_exact_dd(a: FixpDbl, b: FixpDbl) -> FixpDbl {
    fixmuldiv2_bit_exact_dd(a, b) << 1
}

/// `fixmuldiv2_SS(a, b)`: `(LONG)a * b`.
#[inline(always)]
pub const fn fixmuldiv2_ss(a: FixpSgl, b: FixpSgl) -> FixpDbl {
    a as i32 * b as i32
}

/// `fixmul_SS(a, b)`: `(a * b) << 1` (`0x8000 * 0x8000` wraps to `MINVAL_DBL`).
#[inline(always)]
pub const fn fixmul_ss(a: FixpSgl, b: FixpSgl) -> FixpDbl {
    (a as i32 * b as i32) << 1
}

/// `fixmuldiv2_SD(a, b)`: `fixmuldiv2_DD(a << 16, b)` on every variant.
#[inline(always)]
pub const fn fixmuldiv2_sd(a: FixpSgl, b: FixpDbl) -> FixpDbl {
    fixmuldiv2_dd((a as i32) << 16, b)
}

/// `fixmuldiv2_DS(a, b)`: `fixmuldiv2_SD(b, a)`.
#[inline(always)]
pub const fn fixmuldiv2_ds(a: FixpDbl, b: FixpSgl) -> FixpDbl {
    fixmuldiv2_sd(b, a)
}

/// `fixmuldiv2BitExact_SD(a, b)`: `fixmuldiv2_DS(b, a)` (`aarch64`, `x86`); the generic code
/// computes `(a * b) >> 16` in 64 bits, which is the same value.
#[inline(always)]
pub const fn fixmuldiv2_bit_exact_sd(a: FixpSgl, b: FixpDbl) -> FixpDbl {
    fixmuldiv2_ds(b, a)
}

/// `fixmuldiv2BitExact_DS(a, b)`: `fixmuldiv2_DS(a, b)` (macro on `aarch64` and `x86`).
#[inline(always)]
pub const fn fixmuldiv2_bit_exact_ds(a: FixpDbl, b: FixpSgl) -> FixpDbl {
    fixmuldiv2_ds(a, b)
}

/// `fixmul_SD(a, b)`: `fixmuldiv2_SD(a, b) << 1` on every variant (there is no `fixmul_SD`
/// assembly in fdk-aac).
#[inline(always)]
pub const fn fixmul_sd(a: FixpSgl, b: FixpDbl) -> FixpDbl {
    fixmuldiv2_sd(a, b) << 1
}

/// `fixmul_DS(a, b)`: `fixmul_SD(b, a)`.
#[inline(always)]
pub const fn fixmul_ds(a: FixpDbl, b: FixpSgl) -> FixpDbl {
    fixmul_sd(b, a)
}

/// `fixmulBitExact_SD(a, b)`: `fixmul_DS(b, a)` (`aarch64`, `x86`); the generic code computes
/// `((a * b) >> 16) << 1` in 64 bits, which is the same value.
#[inline(always)]
pub const fn fixmul_bit_exact_sd(a: FixpSgl, b: FixpDbl) -> FixpDbl {
    fixmul_ds(b, a)
}

/// `fixmulBitExact_DS(a, b)`: `fixmul_DS(a, b)` (macro on `aarch64` and `x86`).
#[inline(always)]
pub const fn fixmul_bit_exact_ds(a: FixpDbl, b: FixpSgl) -> FixpDbl {
    fixmul_ds(a, b)
}

/// `fixpow2div2_D(a)`: `fixmuldiv2_DD(a, a)`.
#[inline(always)]
pub const fn fixpow2div2_d(a: FixpDbl) -> FixpDbl {
    fixmuldiv2_dd(a, a)
}

/// `fixpow2_D(a)`: `fixpow2div2_D(a) << 1` on every variant.
#[inline(always)]
pub const fn fixpow2_d(a: FixpDbl) -> FixpDbl {
    fixpow2div2_d(a) << 1
}

/// `fixpow2div2_S(a)`: `fixmuldiv2_SS(a, a)`.
#[inline(always)]
pub const fn fixpow2div2_s(a: FixpSgl) -> FixpDbl {
    fixmuldiv2_ss(a, a)
}

/// `fixpow2_S(a)`: `(a * a) << 1`, with `0x8000 * 0x8000` saturated to `MAXVAL_DBL` by
/// `result ^ (result >> 31)`.
#[inline(always)]
pub const fn fixpow2_s(a: FixpSgl) -> FixpDbl {
    let result = fixpow2div2_s(a) << 1;
    result ^ (result >> 31)
}
