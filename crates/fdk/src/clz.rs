// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/clz.h, libFDK/include/arm/clz_arm.h, libFDK/include/x86/clz_x86.h.

//! Port of `clz.h`: counting leading bits.
//!
//! `aarch64` uses the `clz` instruction (`arm/clz_arm.h`, which also defines `fixnormz_S` and
//! `fixnorm_S` on the value shifted left by 16), `x86` uses `__builtin_clz` with a test for 0
//! (`x86/clz_x86.h`, with the generic `fixnormz_S`/`fixnorm_S` of `clz.h` on top: 0 for negative
//! values, `fixnormz_D(value) - 16` otherwise), `generic` uses the bit loops of `clz.h`. All
//! variants compute the same values:
//!
//! - `fixnormz_D(0) == 32`, `fixnorm_D(0) == 0`;
//! - `fixnormz_S(x) == 0` for `x < 0`, `fixnormz_S(0) == 16`; `fixnorm_S(0) == 0`.

use crate::common_fix::{FixpDbl, FixpSgl};

/// `fixnormz_D(value)` / `CntLeadingZeros(value)`: number of leading zero bits, 32 for 0.
#[inline(always)]
pub const fn fixnormz_d(value: i32) -> i32 {
    (value as u32).leading_zeros() as i32
}

/// `fixnorm_D(value)` / `CountLeadingBits(value)`: number of redundant sign bits, 0 for 0.
#[inline(always)]
pub const fn fixnorm_d(value: FixpDbl) -> i32 {
    if value == 0 {
        return 0;
    }
    let v = if value < 0 { !value } else { value };
    fixnormz_d(v) - 1
}

/// `fixnormz_S(value)`: leading zero bits of the 16 bit value, 16 for 0, 0 for negative values.
#[inline(always)]
pub const fn fixnormz_s(value: i16) -> i32 {
    let result = (value as i32) << 16;
    if result == 0 {
        16
    } else {
        fixnormz_d(result)
    }
}

/// `fixnorm_S(value)`: redundant sign bits of the 16 bit value, 0 for 0.
#[inline(always)]
pub const fn fixnorm_s(value: FixpSgl) -> i32 {
    let lvalue = (value as i32) << 16;
    if lvalue == 0 {
        return 0;
    }
    let v = if lvalue < 0 { !lvalue } else { lvalue };
    fixnormz_d(v) - 1
}
