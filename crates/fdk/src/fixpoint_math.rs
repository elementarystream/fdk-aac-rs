// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/fixpoint_math.h, libFDK/src/fixpoint_math.cpp, libFDK/include/x86/fixpoint_math_x86.h.

//! Port of `fixpoint_math.h` and `fixpoint_math.cpp`: fixed point mathematical functions.
//!
//! Variants:
//!
//! - `x86` (`x86/fixpoint_math_x86.h`) replaces `sqrtFixp`, `invSqrtNorm2`, both `invFixp` and
//!   `schur_div`. The first four compute in single precision (`cvtsi2ss`, `sqrtss`, `divss`,
//!   `mulss`, calls of `frexpf` and `ldexpf`, `cvttss2si` in the `ref-x86_64-mac` and
//!   `ref-x86_64` disassembly); `schur_div` is the 64 bit division `((INT64)num << 31) / denum`.
//! - `aarch64` and `generic` use the integer code of `fixpoint_math.h` and the bit-serial
//!   `schur_div` of `fixpoint_math.cpp` (fdk-aac has no `fixpoint_math_arm.h`).
//! - Every function that calls `fMult(FIXP_DBL, FIXP_DBL)` (`CalcInvLdData`, `fMultNorm`,
//!   `fLog2`, `f2Pow`, `fPow`, `fLdPow`, `fPowInt`, ...) inherits the variant of `fixmul_DD`.
//! - `fLdPow` without a result exponent uses `SATURATE_SHIFT`, whose `aarch64` form differs
//!   textually (see [`crate::scale`]).
//!
//! - `x86_32` (`ref-i686`, clang `-m32`, no SSE) compiles the same `x86/fixpoint_math_x86.h`
//!   code to x87 instructions (measured in the disassembly of every inlined copy): `fild` loads
//!   `op` exactly (the `(float)(INT)op` conversion is not rounded to 24 bits), `fmuls`, `fsqrt`,
//!   `fdivp` and `fidivl` round to the 64-bit significand of the extended format, and only the
//!   explicit `(float)` conversions of `invSqrtNorm2` and `invFixp` round to `float` (`fstps` before
//!   the call of `frexpf`). `sqrtFixp` keeps the root in extended precision and truncates the
//!   product with `46340.9492f` directly (`fistpl`). The model is [`crate::x87::F80`]; the
//!   expressions are computed by [`crate::x87`] (the x87 instructions on x86 hosts).
//!
//! `mul_dbl_sgl_rnd` and `InitInvSqrtTab` are declared in `fixpoint_math.h` but not defined in
//! fdk-aac, so they are not ported.

use crate::common_fix::*;

#[cfg(target_arch = "aarch64")]
mod neon;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86;

/// `LD_DATA_SCALING`
pub const LD_DATA_SCALING: f32 = 64.0;
/// `LD_DATA_SHIFT`: `pow(2, LD_DATA_SHIFT) = LD_DATA_SCALING`
pub const LD_DATA_SHIFT: i32 = 6;
/// `MAX_LD_PRECISION`
pub const MAX_LD_PRECISION: usize = 10;
/// `LD_PRECISION`
pub const LD_PRECISION: usize = 10;

/// `SQRT_BITS`
pub const SQRT_BITS: i32 = 7;
/// `SQRT_VALUES`
pub const SQRT_VALUES: usize = 128 + 2;
/// `SQRT_BITS_MASK`
pub const SQRT_BITS_MASK: i32 = 0x7f;
/// `SQRT_FRACT_BITS_MASK`
pub const SQRT_FRACT_BITS_MASK: i32 = 0x007F_FFFF;

/// `MAX_POW2_PRECISION`
pub const MAX_POW2_PRECISION: usize = 8;
/// `POW2_PRECISION`
pub const POW2_PRECISION: usize = 5;

/// `LD_INT_TAB_LEN`
pub const LD_INT_TAB_LEN: i32 = 193;

// ---------------------------------------------------------------------------------------------
// Floating point helpers of the x86 variant
// ---------------------------------------------------------------------------------------------

/// `(INT)x` for a `float` on x86 (`cvttss2si`): truncation toward zero; `INT_MIN` for NaN and
/// values out of range (where Rust's `as` saturates).
#[inline(always)]
fn x86_f32_to_i32(x: f32) -> i32 {
    if x >= -2147483648.0 && x < 2147483648.0 {
        x as i32
    } else {
        i32::MIN
    }
}

/// `(float)(1.0 / (float)(INT)op)` of `x86/fixpoint_math_x86.h`. `x86`: `cvtsi2ss`, `divss`
/// (the correctly rounded `float` quotient of the rounded operand). `x86_32`: `fld1`, `fidivl`
/// (the exact operand, the quotient rounded to 64 bits) and `fstps` (rounded to `float`).
#[inline(always)]
fn x86_inv(op: i32) -> f32 {
    if ARCH_X86_32 {
        crate::x87::inv_fixp_f32(op)
    } else {
        1.0f32 / op as f32
    }
}

/// `2^n` as `f64` for `-1022 <= n <= 1023`.
#[inline(always)]
fn pow2_f64(n: i32) -> f64 {
    f64::from_bits(((n + 1023) as u64) << 52)
}

/// `frexpf(x, &e)`: `x = m * 2^e` with `0.5 <= |m| < 1`. For 0, infinities and NaN, `x` is
/// returned unchanged and `e` is 0 (as in Apple's libm and glibc).
#[inline(always)]
fn frexpf(x: f32) -> (f32, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let bits = x.to_bits();
    let biased = ((bits >> 23) & 0xff) as i32;
    if biased == 0 {
        // subnormal: normalize first
        let (m, e) = frexpf(x * 16777216.0); // 2^24
        return (m, e - 24);
    }
    (f32::from_bits((bits & 0x807f_ffff) | (126 << 23)), biased - 126)
}

/// `ldexpf(x, n)`: `x * 2^n`, correctly rounded (the product is exact in `f64` for the
/// mantissas and exponents used here and is rounded once to `f32`).
#[inline(always)]
fn ldexpf(x: f32, n: i32) -> f32 {
    let n = n.clamp(-1000, 1000);
    (x as f64 * pow2_f64(n)) as f32
}

// ---------------------------------------------------------------------------------------------
// fixpoint_math.h
// ---------------------------------------------------------------------------------------------

/// `fIsLessThan(FIXP_DBL a_m, INT a_e, FIXP_DBL b_m, INT b_e)`: non-zero if
/// `a_m * 2^a_e < b_m * 2^b_e`.
#[inline]
pub fn f_is_less_than(mut a_m: FixpDbl, mut a_e: i32, mut b_m: FixpDbl, mut b_e: i32) -> i32 {
    let n = fixnorm_d(a_m);
    a_m <<= n;
    a_e = a_e.wrapping_sub(n);

    let n = fixnorm_d(b_m);
    b_m <<= n;
    b_e = b_e.wrapping_sub(n);

    if a_m == 0 {
        a_e = b_e;
    }
    if b_m == 0 {
        b_e = a_e;
    }

    // The exponent differences can overflow; the shifts then take the amount modulo 32.
    if a_e > b_e {
        (b_m.wrapping_shr(f_min(a_e.wrapping_sub(b_e), DFRACT_BITS - 1) as u32) > a_m) as i32
    } else {
        (a_m.wrapping_shr(f_min(b_e.wrapping_sub(a_e), DFRACT_BITS - 1) as u32) < b_m) as i32
    }
}

/// `fIsLessThan(FIXP_SGL a_m, INT a_e, FIXP_SGL b_m, INT b_e)`
#[inline]
pub fn f_is_less_than_sgl(mut a_m: FixpSgl, mut a_e: i32, mut b_m: FixpSgl, mut b_e: i32) -> i32 {
    let n = fixnorm_s(a_m);
    a_m = ((a_m as i32) << n) as i16;
    a_e = a_e.wrapping_sub(n);

    let n = fixnorm_s(b_m);
    b_m = ((b_m as i32) << n) as i16;
    b_e = b_e.wrapping_sub(n);

    if a_m == 0 {
        a_e = b_e;
    }
    if b_m == 0 {
        b_e = a_e;
    }

    // fMin(INT, INT) is the FIXP_DBL overload.
    if a_e > b_e {
        let s = f_min(a_e.wrapping_sub(b_e), FRACT_BITS - 1) as u32;
        ((b_m as i32).wrapping_shr(s) > a_m as i32) as i32
    } else {
        let s = f_min(b_e.wrapping_sub(a_e), FRACT_BITS - 1) as u32;
        ((a_m as i32).wrapping_shr(s) < b_m as i32) as i32
    }
}

/// `CalcLdData(op)`: deprecated alias of `fLog2(op, 0)`.
#[inline]
pub fn calc_ld_data(op: FixpDbl) -> FixpDbl {
    f_log2_noexp(op, 0)
}

/// `CalcInvLdData(x)`: delivers `2^(x * LD_DATA_SCALING)`, the inverse of `CalcLdData()`.
///
/// For `x <= 0` the result is fractional positive, for `x > 0` it is an integer in
/// `1..=0x7FFFFFFF`. `x < -31/64` gives 0, `x == 0` and `x >= 31/64` give `MAXVAL_DBL`.
#[inline]
pub fn calc_inv_ld_data(x: FixpDbl) -> FixpDbl {
    let set_zero: i32 = if x < fl2fxconst_dbl(-31.0 / 64.0) { 0 } else { 1 };
    let set_max = (x >= fl2fxconst_dbl(31.0 / 64.0)) | (x == fl2fxconst_dbl(0.0));

    let frac = (x & 0x3FF) as FixpSgl;
    let index3 = (x >> 10) as u32 & 0x1F;
    let index2 = (x >> 15) as u32 & 0x1F;
    let index1 = (x >> 20) as u32 & 0x1F;
    let exp = f_min(31, if x > 0 { 31 - (x >> 25) } else { -(x >> 25) });

    let lookup1 = EXP2_TAB_LONG[index1 as usize].wrapping_mul(set_zero as u32);
    let lookup2 = EXP2W_TAB_LONG[index2 as usize];
    let lookup3 = EXP2X_TAB_LONG[index3 as usize];
    let lookup3f = lookup3.wrapping_add(f_mult_div2(0x0016302F_i32, frac) as u32);

    let lookup12 = f_mult(lookup1 as i32, lookup2 as i32) as u32;
    let lookup = f_mult(lookup12 as i32, lookup3f as i32) as u32;

    // exp is negative for x >= 1.0 (undefined shift), where the result is replaced by
    // MAXVAL_DBL anyway.
    let ret_val = (lookup << 3).wrapping_shr(exp as u32) as FixpDbl;

    if set_max {
        MAXVAL_DBL
    } else {
        ret_val
    }
}

/// `InitLdInt()`: nothing to do, the table is precomputed.
#[inline]
pub fn init_ld_int() {}

/// `CalcLdInt(INT i)`: `ld(i) / LD_DATA_SCALING` for `0 < i < LD_INT_TAB_LEN`, 0 otherwise.
#[inline]
pub fn calc_ld_int(i: i32) -> FixpDbl {
    if i > 0 && i < LD_INT_TAB_LEN {
        LD_INT_COEFF[i as usize]
    } else {
        0
    }
}

/// The table interpolation of `sqrtFixp_lookup()`.
#[inline(always)]
fn sqrt_tab_interpolate(y: u32) -> u32 {
    let idx = ((y >> 26) - 16) as usize;
    let frac = ((y >> 10) & 0xffff) as u16;
    let nfrac = 0xffff ^ frac;
    (nfrac as u32)
        .wrapping_mul(SQRT_TAB[idx] as u32)
        .wrapping_add((frac as u32).wrapping_mul(SQRT_TAB[idx + 1] as u32))
}

/// `sqrtFixp_lookup(FIXP_DBL x)`: square root by table interpolation, 0 for 0.
///
/// For `x == 0` the reference has no defined table read; the port returns 0 without reading
/// the table (the C result is also 0).
#[inline]
pub fn sqrt_fixp_lookup(x: FixpDbl) -> FixpDbl {
    let mut y = x as u32;
    if y == 0 {
        return 0;
    }
    let zeros = fixnormz_d(y as i32) & 0x1e;
    y <<= zeros;
    let t = sqrt_tab_interpolate(y);
    (t >> (zeros >> 1)) as FixpDbl
}

/// `sqrtFixp_lookup(FIXP_DBL x, INT* x_e)`: square root of `x * 2^x_e` by table interpolation.
#[inline]
pub fn sqrt_fixp_lookup_e(x: FixpDbl, x_e: &mut i32) -> FixpDbl {
    let mut y = x as u32;
    if x == 0 {
        return x;
    }
    // Normalize
    let mut e = fixnormz_d(y as i32);
    y <<= e;
    e = x_e.wrapping_sub(e).wrapping_add(2);
    // Correct odd exponent.
    if e & 1 != 0 {
        y >>= 1;
        e = e.wrapping_add(1);
    }
    // Get square root
    let t = sqrt_tab_interpolate(y);
    // Write back exponent
    *x_e = e >> 1;
    (t >> 1) as FixpDbl
}

/// `invSqrtNorm2(FIXP_DBL op, INT* shift)`: `1/sqrt(op)` normalized to 0.5..1, and the
/// exponent of the result: `1/sqrt(op) = result * 2^shift`. `op` must be positive.
///
/// `aarch64`, `generic`: table interpolation (`invSqrtTab`). For negative `op` the C++ code
/// shifts by -1 (undefined); the port takes the amount modulo 32 like `lsl`.
///
/// `x86`: `(float)(1.0 / sqrt(0.5f * (float)op))` in single precision, normalized with
/// `frexpf()` and `ldexpf()`; negative `op` gives `INT_MIN` (NaN converted by `cvttss2si`).
#[inline]
pub fn inv_sqrt_norm2(op: FixpDbl, shift: &mut i32) -> FixpDbl {
    if ARCH_X86_32 {
        if op == 0 {
            *shift = 16;
            return 0x7fffffff;
        }
        // x87: fildl op, fmuls 0.5f, fsqrt, fld1, fdivp (extended precision), fstps for the
        // (float) conversion passed to frexpf. A negative op gives NaN (the inline fsqrt is
        // skipped for sqrtf, which returns NaN as well), frexpf leaves the exponent 0 and fistp
        // gives INT_MIN.
        let result = crate::x87::inv_sqrt_norm2_f32(op);
        let (m, e) = frexpf(result);
        let result = ldexpf(m, DFRACT_BITS - 1);
        *shift = e.wrapping_add(15);
        fdk_assert!(result >= 0.0, "FDK_ASSERT(result >= 0)");
        return x86_f32_to_i32(result);
    }
    if ARCH_X86 {
        if op == 0 {
            *shift = 16;
            return 0x7fffffff;
        }
        // sqrt(float) and the division compile to sqrtss and divss: (float)(1.0 / (double)s)
        // is the correctly rounded float quotient.
        let result = 1.0f32 / (0.5f32 * op as f32).sqrt();
        let (m, e) = frexpf(result);
        let result = ldexpf(m, DFRACT_BITS - 1);
        *shift = e.wrapping_add(15);
        fdk_assert!(result >= 0.0, "FDK_ASSERT(result >= 0)");
        return x86_f32_to_i32(result);
    }

    let mut val = op;
    if val == 0 {
        *shift = 16;
        return MAXVAL_DBL; // maximum positive value
    }

    // normalize input, calculate shift value
    fdk_assert!(val > 0, "FDK_ASSERT(val > FL2FXCONST_DBL(0.0))");
    *shift = fixnormz_d(val) - 1;
    val = val.wrapping_shl(*shift as u32); // normalized input V
    *shift += 2; // bias for exponent

    let index = (val >> (DFRACT_BITS - 1 - (SQRT_BITS + 1))) & SQRT_BITS_MASK;
    let mut fract = (val & SQRT_FRACT_BITS_MASK) << (SQRT_BITS + 1);
    let index = index as usize;
    let mut diff = INV_SQRT_TAB[index + 1].wrapping_sub(INV_SQRT_TAB[index]);
    let mut reg1 = INV_SQRT_TAB[index].wrapping_add(fixmuldiv2_dd(diff, fract) << 1);
    // reg1 = t[i] + (t[i+1]-t[i])*fract ... already computed ...
    //                                  + (1-fract)fract*(t[i+2]-t[i+1])/2
    if fract != 0 {
        // fract = fract * (1 - fract)
        fract = fixmuldiv2_dd(fract, 0x8000_0000u32.wrapping_sub(fract as u32) as i32) << 1;
        diff = diff.wrapping_sub(INV_SQRT_TAB[index + 2].wrapping_sub(INV_SQRT_TAB[index + 1]));
        reg1 = fixmadddiv2_dd(reg1, fract, diff);
    }
    // calculate the output exponent = input exp/2
    if *shift & 0x00000001 != 0 {
        // odd shift values ?
        // Note: Do not use rounded value 0x5A82799A to avoid overflow with shift-by-2
        let reg2: FixpDbl = 0x5A827999; // 1/sqrt(2)
        reg1 = fixmuldiv2_dd(reg1, reg2) << 2;
    }

    *shift >>= 1;

    reg1
}

/// `sqrtFixp(FIXP_DBL op)`: square root of `op` (`0 <= op`).
///
/// `aarch64`, `generic`: `fMultDiv2(op << (e - 1), invSqrtNorm2(op, &e)) << 2`. For negative
/// `op`, `e` is 0 and the C++ code shifts by -1 (undefined); the port takes the amount modulo
/// 32 like `lsl`.
///
/// `x86`: `(INT)(sqrt((float)op) * 46340.9492f)` in single precision (`sqrtss`, `mulss`,
/// `cvttss2si`); negative `op` gives `INT_MIN`.
///
/// `x86_32`: the same expression in extended precision (`fildl`, `fsqrt`, `fmuls`, `fistpl`):
/// neither the operand nor the root is rounded to `float`.
#[inline]
pub fn sqrt_fixp(op: FixpDbl) -> FixpDbl {
    if ARCH_X86_32 {
        let result = crate::x87::sqrt_fixp(op);
        fdk_assert!(result >= 0, "FDK_ASSERT(result >= (FIXP_DBL)0)");
        return result;
    }
    if ARCH_X86 {
        let result = x86_f32_to_i32((op as f32).sqrt() * 46340.9492f32);
        fdk_assert!(result >= 0, "FDK_ASSERT(result >= (FIXP_DBL)0)");
        return result;
    }
    let mut tmp_exp = 0;
    let tmp_inv = inv_sqrt_norm2(op, &mut tmp_exp);
    fdk_assert!(tmp_exp > 0, "FDK_ASSERT(tmp_exp > 0)");
    fixmuldiv2_dd(op.wrapping_shl(tmp_exp.wrapping_sub(1) as u32), tmp_inv) << 2
}

/// `invFixp(FIXP_DBL op)`: `1.0 / op` for an integer `op`; the result has an implicit exponent
/// of 31. 0 and 1 give `MAXVAL_DBL`.
///
/// `aarch64`, `generic`: `fPow2Div2(invSqrtNorm2(op))`, rounded to the exponent 31. `x86`:
/// `(float)(1.0 / (float)op)` in single precision, scaled by `2^31` with `frexpf()` and
/// `ldexpf()`. `x86_32`: `fld1`, `fidivl op` (the exact `op`, not rounded to `float`), rounded to
/// `float` once (`fstps`).
#[inline]
pub fn inv_fixp(op: FixpDbl) -> FixpDbl {
    if op == 0 || op == 1 {
        return MAXVAL_DBL;
    }
    if ARCH_X86 {
        let result = x86_inv(op);
        let (m, e) = frexpf(result);
        return x86_f32_to_i32(ldexpf(m, 31i32.wrapping_add(e)));
    }
    let mut tmp_exp = 0;
    let mut tmp_inv = inv_sqrt_norm2(op, &mut tmp_exp);
    fdk_assert!((31 - (2 * tmp_exp + 1)) >= 0, "FDK_ASSERT((31 - (2 * tmp_exp + 1)) >= 0)");
    let shift = 31 - (2 * tmp_exp + 1);
    tmp_inv = fixpow2div2_d(tmp_inv);
    if shift != 0 {
        tmp_inv = (tmp_inv.wrapping_shr((shift - 1) as u32).wrapping_add(1)) >> 1;
    }
    tmp_inv
}

/// `invFixp(FIXP_DBL op_m, int* op_e)`: `1.0 / (op_m * 2^op_e)`; the exponent of the result is
/// written to `op_e`.
///
/// `aarch64`, `generic`: `fPow2Div2(invSqrtNorm2(op_m, &e))` with the exponent
/// `2 * e - op_e + 1`. `x86`: single precision division normalized with `frexpf()`, exponent
/// `e - op_e + 31`. `x86_32`: as `invFixp(op)` (`fld1`, `fidivl`, `fstps`).
#[inline]
pub fn inv_fixp_e(op_m: FixpDbl, op_e: &mut i32) -> FixpDbl {
    if op_m == 0 || op_m == 1 {
        *op_e = 31i32.wrapping_sub(*op_e);
        return MAXVAL_DBL;
    }
    if ARCH_X86 {
        let result = x86_inv(op_m);
        let (m, e) = frexpf(result);
        let result = ldexpf(m, DFRACT_BITS - 1);
        *op_e = e.wrapping_sub(*op_e).wrapping_add(31);
        return x86_f32_to_i32(result);
    }
    let mut tmp_exp = 0;
    let tmp_inv = inv_sqrt_norm2(op_m, &mut tmp_exp);
    *op_e = (tmp_exp << 1).wrapping_sub(*op_e).wrapping_add(1);
    fixpow2div2_d(tmp_inv)
}

/// `schur_div(FIXP_DBL num, FIXP_DBL denum, INT count)`: `num / denum` for `0 <= num <= denum`,
/// `denum > 0`.
///
/// `aarch64`, `generic` (`fixpoint_math.cpp`): bit-serial division with `count` significant
/// bits (`count >= 1`). `x86` (`x86/fixpoint_math_x86.h`): `((INT64)num << 31) / denum`
/// truncated to `INT`, `MAXVAL_DBL` for `num == denum`; `count` is ignored.
///
/// Deviation: for `denum == 0` and `num != 0` the x86 reference has no defined result; the port
/// returns 0 instead of panicking.
pub fn schur_div(num: FixpDbl, denum: FixpDbl, count: i32) -> FixpDbl {
    fdk_assert!(num >= 0, "FDK_ASSERT(num >= (FIXP_DBL)0)");
    fdk_assert!(denum > 0, "FDK_ASSERT(denum > (FIXP_DBL)0)");
    fdk_assert!(num <= denum, "FDK_ASSERT(num <= denum)");
    if ARCH_X86 {
        return if num == denum {
            MAXVAL_DBL
        } else {
            // |num << 31| < 2^63, so only the division by zero can fail.
            ((num as i64) << 31).checked_div(denum as i64).unwrap_or(0) as i32
        };
    }
    let mut l_num = num >> 1;
    let l_denum = denum >> 1;
    let mut div: i32 = 0;
    let mut k = count;

    if l_num != 0 {
        loop {
            k = k.wrapping_sub(1);
            if k == 0 {
                break;
            }
            div = div.wrapping_shl(1);
            l_num = l_num.wrapping_shl(1);
            if l_num >= l_denum {
                l_num = l_num.wrapping_sub(l_denum);
                div = div.wrapping_add(1);
            }
        }
    }
    div.wrapping_shl((DFRACT_BITS - count) as u32)
}

/// `fMultNorm(FIXP_DBL f1, FIXP_DBL f2, INT* result_e)`: product with normalization of the
/// factors (maximum precision).
pub fn f_mult_norm(mut f1: FixpDbl, mut f2: FixpDbl, result_e: &mut i32) -> FixpDbl {
    if f1 == 0 || f2 == 0 {
        *result_e = 0;
        return 0;
    }
    let norm_f1 = count_leading_bits(f1);
    f1 <<= norm_f1;
    let norm_f2 = count_leading_bits(f2);
    f2 <<= norm_f2;

    if f1 == MINVAL_DBL && f2 == MINVAL_DBL {
        *result_e = -(norm_f1 + norm_f2 - 1);
        -(MINVAL_DBL >> 1)
    } else {
        *result_e = -(norm_f1 + norm_f2);
        f_mult(f1, f2)
    }
}

/// `fMultNorm(FIXP_DBL f1, FIXP_DBL f2)`: product with exponent 0 (saturated).
#[inline]
pub fn f_mult_norm_noexp(f1: FixpDbl, f2: FixpDbl) -> FixpDbl {
    let mut e = 0;
    let m = f_mult_norm(f1, f2, &mut e);
    scale_value_saturate(m, e)
}

/// `fMultNorm(FIXP_DBL f1_m, INT f1_e, FIXP_DBL f2_m, INT f2_e, INT result_e)`: product with the
/// exponent `result_e` (saturated).
#[inline]
pub fn f_mult_norm_to(
    f1_m: FixpDbl,
    f1_e: i32,
    f2_m: FixpDbl,
    f2_e: i32,
    result_e: i32,
) -> FixpDbl {
    let mut e = 0;
    let m = f_mult_norm(f1_m, f2_m, &mut e);
    scale_value_saturate(m, e.wrapping_add(f1_e).wrapping_add(f2_e).wrapping_sub(result_e))
}

/// `fMultI(FIXP_DBL a, INT b)`: fractional times integer, rounded to nearest.
#[inline]
pub fn f_mult_i(a: FixpDbl, b: i32) -> i32 {
    let mut m_e = 0;
    let mut m = f_mult_norm(a, b, &mut m_e);
    if m_e < 0 {
        if m_e > -DFRACT_BITS {
            m >>= (-m_e) - 1;
            m.wrapping_add(1) >> 1
        } else {
            0
        }
    } else {
        scale_value_saturate(m, m_e)
    }
}

/// `fMultIfloor(FIXP_DBL a, INT b)`: fractional times integer, rounded down.
#[inline]
pub fn f_mult_i_floor(a: FixpDbl, b: i32) -> i32 {
    let mut m_e = 0;
    let m = f_mult_norm(a, b, &mut m_e);
    if m_e < 0 {
        if m_e > -DFRACT_BITS {
            m >> (-m_e)
        } else if m < 0 {
            -1
        } else {
            0
        }
    } else {
        scale_value_saturate(m, m_e)
    }
}

/// `fMultIceil(FIXP_DBL a, INT b)`: fractional times integer, rounded up.
#[inline]
pub fn f_mult_i_ceil(a: FixpDbl, b: i32) -> i32 {
    let mut m_e = 0;
    let m = f_mult_norm(a, b, &mut m_e);
    if m_e < 0 {
        if m_e > -(DFRACT_BITS - 1) {
            let mut mi = m >> (-m_e);
            if m & ((1 << (-m_e)) - 1) != 0 {
                mi = mi.wrapping_add(1);
            }
            mi
        } else if m > 0 {
            1
        } else if m_e == -(DFRACT_BITS - 1) && m == MINVAL_DBL {
            -1
        } else {
            0
        }
    } else {
        scale_value_saturate(m, m_e)
    }
}

/// `fDivNorm(FIXP_DBL num, FIXP_DBL denom, INT* result_e)`: `num / denom` for `num >= 0`,
/// `denom > 0` with normalization of the inputs.
///
/// On the `aarch64` and `generic` variants the bit-serial `schur_div(num, denum, 16)` is
/// computed with one integer division (the straight port is [`f_div_norm_ref`]): for
/// `L_num > 0`, `L_denum > 0` the normalized `num` lies in `[2^29, 2^30)` and `denum` in
/// `[2^30, 2^31)`, so `num >> 1 < denum >> 1`, the restoring division never overflows and its
/// 15 quotient bits are `((num >> 1) << 15) / (denum >> 1)`. The `x86` variants keep their
/// `schur_div` (one division of 64-bit values); inputs outside the domain (`L_num < 0`,
/// `L_denum <= 0`) take the straight code.
pub fn f_div_norm(l_num: FixpDbl, l_denum: FixpDbl, result_e: &mut i32) -> FixpDbl {
    if ARCH_X86 || l_num <= 0 || l_denum <= 0 {
        return f_div_norm_ref(l_num, l_denum, result_e);
    }
    let norm_num = count_leading_bits(l_num);
    let num = (l_num << norm_num) >> 1;
    let norm_den = count_leading_bits(l_denum);
    let den = l_denum << norm_den;
    *result_e = -norm_num + 1 + norm_den;
    let div = (((num >> 1) as u64) << 15) / ((den >> 1) as u64);
    (div as i32) << (DFRACT_BITS - FRACT_BITS)
}

/// `fDivNorm(FIXP_DBL num, FIXP_DBL denom, INT* result_e)`: the straight port of
/// [`f_div_norm`].
pub fn f_div_norm_ref(mut l_num: FixpDbl, mut l_denum: FixpDbl, result_e: &mut i32) -> FixpDbl {
    fdk_assert!(l_num >= 0, "FDK_ASSERT(L_num >= (FIXP_DBL)0)");
    fdk_assert!(l_denum > 0, "FDK_ASSERT(L_denum > (FIXP_DBL)0)");
    if l_num == 0 {
        *result_e = 0;
        return 0;
    }
    let norm_num = count_leading_bits(l_num);
    l_num <<= norm_num;
    l_num >>= 1;
    *result_e = -norm_num + 1;

    let norm_den = count_leading_bits(l_denum);
    l_denum <<= norm_den;
    *result_e -= -norm_den;

    schur_div(l_num, l_denum, FRACT_BITS)
}

/// `fDivNorm(FIXP_DBL num, FIXP_DBL denom)`: `num / denom` with exponent 0 for `denom >= num`.
pub fn f_div_norm_noexp(num: FixpDbl, denom: FixpDbl) -> FixpDbl {
    fdk_assert!(denom >= num, "FDK_ASSERT(denom >= num)");
    let mut e = 0;
    let res = f_div_norm(num, denom, &mut e);
    // Avoid overflow since we must output a value with exponent 0; there is no other choice
    // than saturating to almost 1.0f.
    if res == (1 << (DFRACT_BITS - 2)) && e == 1 {
        MAXVAL_DBL
    } else {
        scale_value(res, e)
    }
}

/// `fDivNormSigned(FIXP_DBL num, FIXP_DBL denom)`: signed division with exponent 0
/// (saturated).
pub fn f_div_norm_signed_noexp(num: FixpDbl, denom: FixpDbl) -> FixpDbl {
    if denom == 0 {
        return MAXVAL_DBL;
    }
    let sign = (num >= 0) != (denom >= 0);
    let mut e = 0;
    let res = f_div_norm_signed(num, denom, &mut e);
    // Saturate since we must output a value with exponent 0
    if e > 0 && f_abs(res) >= fl2fxconst_dbl(0.5) {
        if sign {
            MINVAL_DBL
        } else {
            MAXVAL_DBL
        }
    } else {
        scale_value(res, e)
    }
}

/// `fDivNormSigned(FIXP_DBL L_num, FIXP_DBL L_denum, INT* result_e)`: signed division with
/// normalization of the inputs.
pub fn f_div_norm_signed(mut l_num: FixpDbl, mut l_denum: FixpDbl, result_e: &mut i32) -> FixpDbl {
    let sign = (l_num >= 0) != (l_denum >= 0);

    if l_num == 0 {
        *result_e = 0;
        return 0;
    }
    if l_denum == 0 {
        *result_e = 14;
        return MAXVAL_DBL;
    }

    let norm_num = count_leading_bits(l_num);
    l_num <<= norm_num;
    l_num >>= 2;
    l_num = f_abs(l_num);
    *result_e = -norm_num + 1;

    let norm_den = count_leading_bits(l_denum);
    l_denum <<= norm_den;
    l_denum >>= 1;
    l_denum = f_abs(l_denum);
    *result_e -= -norm_den;

    let div = schur_div(l_num, l_denum, FRACT_BITS);

    if sign {
        div.wrapping_neg()
    } else {
        div
    }
}

/// `fDivNormHighPrec(FIXP_DBL num, FIXP_DBL denom, INT* result_e)`: as [`f_div_norm`] with 31
/// significant bits.
pub fn f_div_norm_high_prec(mut num: FixpDbl, mut denom: FixpDbl, result_e: &mut i32) -> FixpDbl {
    fdk_assert!(num >= 0, "FDK_ASSERT(num >= (FIXP_DBL)0)");
    fdk_assert!(denom > 0, "FDK_ASSERT(denom > (FIXP_DBL)0)");
    if num == 0 {
        *result_e = 0;
        return 0;
    }
    let norm_num = count_leading_bits(num);
    num <<= norm_num;
    num >>= 1;
    *result_e = -norm_num + 1;

    let norm_den = count_leading_bits(denom);
    denom <<= norm_den;
    *result_e -= -norm_den;

    schur_div(num, denom, 31)
}

/// `fAdjust(FIXP_DBL a_m, INT* pA_e)`: adjust the mantissa to the exponent -1.
#[inline]
pub fn f_adjust(a_m: FixpDbl, p_a_e: &mut i32) -> FixpDbl {
    let shift = fixnorm_d(a_m) - 1;
    *p_a_e = p_a_e.wrapping_sub(shift);
    scale_value(a_m, shift)
}

/// `fAddNorm(FIXP_DBL a_m, INT a_e, FIXP_DBL b_m, INT b_e, INT* pResult_e)`: sum with
/// normalization.
#[inline]
pub fn f_add_norm(
    mut a_m: FixpDbl,
    mut a_e: i32,
    mut b_m: FixpDbl,
    mut b_e: i32,
    p_result_e: &mut i32,
) -> FixpDbl {
    // If one of the summands is zero, return the other. This is necessary for the summation of a
    // very small number to zero.
    if a_m == 0 {
        *p_result_e = b_e;
        return b_m;
    }
    if b_m == 0 {
        *p_result_e = a_e;
        return a_m;
    }

    a_m = f_adjust(a_m, &mut a_e);
    b_m = f_adjust(b_m, &mut b_e);

    // The exponent differences can overflow; the shifts then take the amount modulo 32.
    let (result_m, result_e) = if a_e > b_e {
        let s = f_min(a_e.wrapping_sub(b_e), DFRACT_BITS - 1) as u32;
        (a_m.wrapping_add(b_m.wrapping_shr(s)), a_e)
    } else {
        let s = f_min(b_e.wrapping_sub(a_e), DFRACT_BITS - 1) as u32;
        (a_m.wrapping_shr(s).wrapping_add(b_m), b_e)
    };

    *p_result_e = result_e;
    result_m
}

/// `fAddNorm(FIXP_DBL a_m, INT a_e, FIXP_DBL b_m, INT b_e, INT result_e)`: sum with the exponent
/// `result_e`.
#[inline]
///
/// The exponent differences can overflow (undefined). Measured on the reference builds (the
/// inline function compiled into the `cref` shim): the arm64 code decides the shift direction
/// with `subs` + `csel gt`, i.e. by the mathematical difference, the x86-64 code by the sign of
/// the wrapped 32 bit difference (`subl` + `testl`); both shift by the 32 bit difference modulo
/// 32. The port reproduces both. Other inlined call sites may compile differently.
pub fn f_add_norm_to(a_m: FixpDbl, a_e: i32, b_m: FixpDbl, b_e: i32, result_e: i32) -> FixpDbl {
    let (a_m, b_m) = if ARCH_ARM {
        (
            scale_value_wide(a_m, a_e as i64 - result_e as i64),
            scale_value_wide(b_m, b_e as i64 - result_e as i64),
        )
    } else {
        (scale_value(a_m, a_e.wrapping_sub(result_e)), scale_value(b_m, b_e.wrapping_sub(result_e)))
    };
    a_m.wrapping_add(b_m)
}

/// `scaleValue(value, a - b)` as the arm64 reference compiles it where the scalefactor is a
/// difference of two `INT`s: the direction is decided by the comparison `a > b` (clang assumes
/// that the subtraction does not overflow), the amount is the 32 bit difference modulo 32.
#[inline(always)]
fn scale_value_wide(value: FixpDbl, scalefactor: i64) -> FixpDbl {
    if scalefactor > 0 {
        value.wrapping_shl(scalefactor as u32)
    } else {
        value.wrapping_shr(scalefactor.wrapping_neg() as u32)
    }
}

/// The Taylor polynomial of `ln(1-x)` of `fLog2()`, multiplied by `1/ln(2)` (straight port, kept
/// as the reference of [`ld_poly`]).
#[inline(always)]
fn ld_poly_ref(x2_m: FixpDbl) -> FixpDbl {
    // Taylor polynomial approximation of ln(1-x)
    let mut result_m = fl2fxconst_dbl(0.0);
    let mut px2_m = x2_m;
    for &coeff in LD_COEFF.iter().take(LD_PRECISION) {
        result_m = f_mult_add_div2(result_m, coeff, px2_m);
        px2_m = f_mult(px2_m, x2_m);
    }
    // Multiply result with 1/ln(2) = 1.0 + 0.442695040888 (get log2(x) from ln(x) result).
    f_mult_add_div2(result_m, result_m, fl2fxconst_dbl(2.0 * 0.4426950408889634073599246810019))
}

/// [`ld_poly_ref`] with a shorter dependency chain.
///
/// The powers `px2_m` form a chain of 10 dependent `fMult()`s, which is the latency of `fLog2()`
/// (a multiplication plus a shift per step). Here every step is one high-half multiplication.
///
/// Exactness: `fLog2()` calls the polynomial with `0 < x <= 2^30` (`x = 2^31 - (x_m << b_norm)`,
/// `x_m << b_norm` in `[2^30, 2^31)`), so every power is non-negative and below `2^30`.
/// - `aarch64` (`fixmul_DD` = `(a * b) >> 31`): `p' = floor(p * x / 2^31)
///   = floor(p * (x << 33) / 2^64)`, the high half of an unsigned 64-bit product (`x << 33 <=
///   2^63`). On 32-bit targets: `floor(p * 2x / 2^32)` (`2x <= 2^31`).
/// - `x86`, `x86_32`, `generic` (`fixmul_DD` = `((a * b) >> 32) << 1`): every power from the
///   second one on is `2q` with `q = floor(p * x / 2^32)`; the next `q` is `floor(2q * x / 2^32)
///   = floor(q * x / 2^31)`, the same high-half product as above; the term of `2q` is
///   `floor(c * 2q / 2^16) = floor((c << 17) * q / 2^32)` (`c << 17` fits 32 bits for
///   `|c| <= 2^14`, which holds for every coefficient but the first).
/// - The terms `fMultAddDiv2(r, FIXP_SGL c, p)` are `r + floor((c << 16) * p / 2^32)` on every
///   variant, added in the same order with wrap-around.
///
/// The test `ld_poly_equals_straight_port` compares the two for every `x` in `(0, 2^30]` (in
/// strides; `FDK_AAC_RS_LD_POLY_EXHAUSTIVE=N` runs all of them) with the variant of the build.
#[inline(always)]
fn ld_poly(x: FixpDbl) -> FixpDbl {
    debug_assert!(0 < x && x <= 1 << 30);
    // r + floor((c << shift) * p / 2^32), with `c << shift` in 32 bits
    #[inline(always)]
    fn term(r: FixpDbl, c: i16, shift: u32, p: u32) -> FixpDbl {
        r.wrapping_add(fixmuldiv2_dd((c as i32) << shift, p as i32))
    }
    const _: () = assert!(LD_PRECISION == 10 && LD_COEFF.len() == LD_PRECISION);
    const _: () = {
        let mut i = 1;
        while i < LD_PRECISION {
            assert!(LD_COEFF[i] >= -(1 << 14) && LD_COEFF[i] <= 1 << 14);
            i += 1;
        }
    };
    let xu = x as u32;
    let mut r: FixpDbl = 0;
    #[cfg(target_pointer_width = "64")]
    let next = {
        let xs = (xu as u64) << 33;
        move |p: u32| ((p as u64 as u128 * xs as u128) >> 64) as u32
    };
    #[cfg(not(target_pointer_width = "64"))]
    let next = {
        let x2 = xu << 1;
        move |p: u32| ((p as u64 * x2 as u64) >> 32) as u32
    };
    if ARCH_ARM {
        let mut p = xu;
        for &c in LD_COEFF.iter() {
            r = term(r, c, 16, p);
            p = next(p);
        }
    } else {
        r = term(r, LD_COEFF[0], 16, xu);
        let mut q = ((xu as u64 * xu as u64) >> 32) as u32;
        for &c in LD_COEFF[1..].iter() {
            r = term(r, c, 17, q);
            q = next(q);
        }
    }
    f_mult_add_div2(r, r, fl2fxconst_dbl(2.0 * 0.4426950408889634073599246810019))
}

/// `fLog2(FIXP_DBL x_m, INT x_e, INT* result_e)`: logarithm of base 2 of `x_m * 2^x_e`
/// (Taylor polynomial of `ln(1-x)` on every variant).
#[inline]
pub fn f_log2(x_m: FixpDbl, x_e: i32, result_e: &mut i32) -> FixpDbl {
    f_log2_with(x_m, x_e, result_e, ld_poly)
}

/// [`f_log2`] with the straight port of the polynomial ([`ld_poly_ref`]).
#[doc(hidden)]
#[inline]
pub fn f_log2_ref(x_m: FixpDbl, x_e: i32, result_e: &mut i32) -> FixpDbl {
    f_log2_with(x_m, x_e, result_e, ld_poly_ref)
}

/// The Taylor polynomial of `fLog2()` multiplied by `1/ln(2)` (`fMultAddDiv2(r, r, 2 *
/// 0.4426950408889634)` included), for the argument `0 < x2_m <= 2^30` that `fLog2()` computes
/// from a positive mantissa; other arguments take the straight port.
#[inline]
pub fn f_log2_poly(x2_m: FixpDbl) -> FixpDbl {
    if 0 < x2_m && x2_m <= 1 << 30 {
        ld_poly(x2_m)
    } else {
        ld_poly_ref(x2_m)
    }
}

/// The polynomial of `fLog2()` for `0 < x <= 2^30`: the optimized and the straight version
/// (for tests).
#[doc(hidden)]
pub fn ld_poly_pair(x: FixpDbl) -> (FixpDbl, FixpDbl) {
    (ld_poly(x), ld_poly_ref(x))
}

#[inline(always)]
fn f_log2_with(
    x_m: FixpDbl,
    mut x_e: i32,
    result_e: &mut i32,
    poly: impl Fn(FixpDbl) -> FixpDbl,
) -> FixpDbl {
    // Short cut for zero and negative numbers.
    if x_m <= fl2fxconst_dbl(0.0) {
        *result_e = DFRACT_BITS - 1;
        return fl2fxconst_dbl(-1.0);
    }

    // Move input value x_m * 2^x_e toward 1.0, where the taylor approximation of the function
    // log(1-x) centered at 0 is most accurate.
    let b_norm = fixnormz_d(x_m) - 1;
    let mut x2_m = x_m << b_norm;
    x_e = x_e.wrapping_sub(b_norm);

    // map x from log(x) domain to log(1-x) domain.
    x2_m = x2_m.wrapping_add(fl2fxconst_dbl(-1.0)).wrapping_neg();

    // Taylor polynomial approximation of ln(1-x), multiplied with 1/ln(2)
    let mut result_m = poly(x2_m);

    // Add exponent part. log2(x_m * 2^x_e) = log2(x_m) + x_e
    if x_e != 0 {
        let enorm = DFRACT_BITS - fixnorm_d(x_e);
        // The -1 in the right shift of result_m compensates the fMultDiv2() above in the taylor
        // polynomial evaluation loop.
        // For |x_e| >= 2^30, enorm is 32 and the C++ code shifts x_e by -1 (undefined); the
        // reference builds take the amount modulo 32.
        result_m = (result_m >> (enorm - 1))
            .wrapping_add(x_e.wrapping_shl((DFRACT_BITS - 1 - enorm) as u32));
        *result_e = enorm;
    } else {
        // 1 compensates the fMultDiv2() above in the taylor polynomial evaluation loop.
        *result_e = 1;
    }
    result_m
}

/// `fLog2(FIXP_DBL x_m, INT x_e)`: the result has an implicit exponent of `LD_DATA_SHIFT`.
#[inline]
pub fn f_log2_noexp(mut x_m: FixpDbl, x_e: i32) -> FixpDbl {
    if x_m <= fl2fxconst_dbl(0.0) {
        x_m = fl2fxconst_dbl(-1.0);
    } else {
        let mut result_e = 0;
        x_m = f_log2(x_m, x_e, &mut result_e);
        x_m = scale_value(x_m, result_e - LD_DATA_SHIFT);
    }
    x_m
}

/// [`f_log2_noexp`] with the straight port of the polynomial.
#[doc(hidden)]
#[inline]
pub fn f_log2_noexp_ref(mut x_m: FixpDbl, x_e: i32) -> FixpDbl {
    if x_m <= fl2fxconst_dbl(0.0) {
        x_m = fl2fxconst_dbl(-1.0);
    } else {
        let mut result_e = 0;
        x_m = f_log2_ref(x_m, x_e, &mut result_e);
        x_m = scale_value(x_m, result_e - LD_DATA_SHIFT);
    }
    x_m
}

/// `fAddSaturate(const FIXP_SGL a, const FIXP_SGL b)`: saturated sum.
#[inline]
pub fn f_add_saturate_sgl(a: FixpSgl, b: FixpSgl) -> FixpSgl {
    let sum = a as i32 + b as i32;
    fixmax_i(fixmin_i(sum, MAXVAL_SGL), MINVAL_SGL) as i16
}

/// `fAddSaturate(const FIXP_DBL a, const FIXP_DBL b)`: the sum of `a >> 1` and `b >> 1`,
/// saturated and shifted back: the least significant bit of the result is always 0.
#[inline]
pub fn f_add_saturate(a: FixpDbl, b: FixpDbl) -> FixpDbl {
    let sum = (a >> 1) + (b >> 1);
    let sum = fixmax_i(fixmin_i(sum, MAXVAL_DBL >> 1), MINVAL_DBL >> 1);
    sum << 1
}

/// `InitInvInt()`: nothing to do, the table is precomputed.
#[inline]
pub fn init_inv_int() {}

/// `GetInvInt(int intValue)`: `1 / intValue` for `1 <= intValue < 80` (the index is limited to
/// `0..=79`).
#[inline]
pub fn get_inv_int(int_value: i32) -> FixpDbl {
    INV_COUNT[f_min(f_max(int_value, 0), 80 - 1) as usize]
}

// ---------------------------------------------------------------------------------------------
// fixpoint_math.cpp
// ---------------------------------------------------------------------------------------------

/// `LdDataVector(FIXP_DBL* srcVector, FIXP_DBL* destVector, INT n)`: `fLog2(src[i], 0)` for
/// `n = src.len()` values.
///
/// Four values at a time in NEON (AArch64), eight in AVX2 (x86 with [`crate::simd::avx2`]),
/// each lane computing exactly [`f_log2_noexp`] (see `fixpoint_math/neon.rs` and `x86.rs`; the
/// test `ld_data_vector_equals_straight_port` covers every 32-bit input with
/// `FDK_AAC_RS_LD_DATA_EXHAUSTIVE=N`); the remaining values and other targets take the scalar
/// code ([`ld_data_vector_ref`]).
pub fn ld_data_vector(src: &[FixpDbl], dst: &mut [FixpDbl]) {
    let dst = &mut dst[..src.len()];
    #[cfg(target_arch = "aarch64")]
    {
        let groups = src.len() / 4;
        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            // SAFETY: NEON is part of the AArch64 baseline; the chunks hold 4 values.
            unsafe {
                core::arch::aarch64::vst1q_s32(
                    d.as_mut_ptr(),
                    neon::ld_data4(core::arch::aarch64::vld1q_s32(s.as_ptr())),
                )
            };
        }
        ld_data_vector_ref(&src[groups * 4..], &mut dst[groups * 4..]);
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if crate::simd::avx2() {
            // SAFETY: the CPU supports AVX2.
            unsafe { ld_data_vector_avx2(src, dst) };
        } else {
            ld_data_vector_ref(src, dst);
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    ld_data_vector_ref(src, dst);
}

/// [`ld_data_vector`] with AVX2.
///
/// # Safety
///
/// The CPU must support AVX2.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn ld_data_vector_avx2(src: &[FixpDbl], dst: &mut [FixpDbl]) {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;
    let groups = src.len() / 8;
    for (d, s) in dst.chunks_exact_mut(8).zip(src.chunks_exact(8)) {
        let v = x86::ld_data8(_mm256_loadu_si256(s.as_ptr() as *const __m256i));
        _mm256_storeu_si256(d.as_mut_ptr() as *mut __m256i, v);
    }
    ld_data_vector_ref(&src[groups * 8..], &mut dst[groups * 8..]);
}

/// `LdDataVector()`: straight port, kept as the reference of [`ld_data_vector`].
#[doc(hidden)]
pub fn ld_data_vector_ref(src: &[FixpDbl], dst: &mut [FixpDbl]) {
    for (d, &s) in dst[..src.len()].iter_mut().zip(src) {
        *d = f_log2_noexp(s, 0);
    }
}

/// `FL2FXCONST_DBL(2.0 * 0.4426950408889634073599246810019)`, the factor of the last step of
/// the `fLog2()` polynomial.
#[allow(dead_code)]
const LD_POLY_FINAL: FixpDbl = fl2fxconst_dbl(2.0 * 0.4426950408889634073599246810019);

/// `f2Pow(const FIXP_DBL exp_m, const INT exp_e, INT* result_e)`: `2 ^ (exp_m * 2^exp_e)`.
///
/// For `exp_e >= 32` (or `exp_e < -31`) the C++ code shifts by amounts outside of 0..=31
/// (undefined); the port takes them modulo 32.
pub fn f2_pow(exp_m: FixpDbl, exp_e: i32, result_e: &mut i32) -> FixpDbl {
    let mut int_part: i32;
    let mut frac_part: FixpDbl;

    if exp_e > 0 {
        let exp_bits = (DFRACT_BITS - 1).wrapping_sub(exp_e);
        int_part = exp_m.wrapping_shr(exp_bits as u32);
        frac_part = exp_m.wrapping_sub(int_part.wrapping_shl(exp_bits as u32));
        frac_part = frac_part.wrapping_shl(exp_e as u32);
    } else {
        int_part = 0;
        frac_part = exp_m.wrapping_shr(exp_e.wrapping_neg() as u32);
    }

    // Best accuracy is around 0, so try to get there with the fractional part.
    if frac_part > fl2fxconst_dbl(0.5) {
        int_part = int_part.wrapping_add(1);
        frac_part = frac_part.wrapping_add(fl2fxconst_dbl(-1.0));
    }
    if frac_part < fl2fxconst_dbl(-0.5) {
        int_part = int_part.wrapping_sub(1);
        frac_part = fl2fxconst_dbl(-1.0).wrapping_sub(frac_part).wrapping_neg();
    }

    // "+ 1" compensates fMultAddDiv2() of the polynomial evaluation below.
    *result_e = int_part.wrapping_add(1);

    // Evaluate taylor polynomial which approximates 2^x
    // result_m ~= 2^frac_part
    let mut p = frac_part;
    // First taylor series coefficient a_0 = 1.0, scaled by 0.5 due to fMultDiv2().
    let mut result_m = fl2fxconst_dbl(1.0 / 2.0);
    for &coeff in POW2_COEFF.iter().take(POW2_PRECISION) {
        // next taylor series term: a_i * x^i, x=0
        result_m = f_mult_add_div2(result_m, coeff, p);
        p = f_mult(p, frac_part);
    }
    result_m
}

/// `f2Pow(const FIXP_DBL exp_m, const INT exp_e)`: mantissa with the implicit exponent 0.
pub fn f2_pow_noexp(exp_m: FixpDbl, exp_e: i32) -> FixpDbl {
    let mut result_e = 0;
    let result_m = f2_pow(exp_m, exp_e, &mut result_e);
    let result_e = fix_min(DFRACT_BITS - 1, fix_max(-(DFRACT_BITS - 1), result_e));
    scale_value(result_m, result_e)
}

/// `CountLeadingBits(fAbs(x))` as the reference builds compute it in `fPow` and `fLdPow`: clang
/// assumes that `fAbs()` is not negative (the negation of `MINVAL_DBL` is undefined) and
/// computes `clz(fAbs(x)) - 1` (`cneg`, `clz`, `sub` on arm64), which is -1 for `MINVAL_DBL`
/// (`fixnorm_D` would give 0). The following shift by -1 is taken modulo 32.
#[inline(always)]
fn count_leading_bits_of_abs(x: FixpDbl) -> i32 {
    if x == 0 {
        0
    } else {
        fixnormz_d(f_abs(x)) - 1
    }
}

/// `fPow(FIXP_DBL base_m, INT base_e, FIXP_DBL exp_m, INT exp_e, INT* result_e)`:
/// `(base_m * 2^base_e) ^ (exp_m * 2^exp_e)`; 0 for `base_m <= 0`.
pub fn f_pow(
    base_m: FixpDbl,
    base_e: i32,
    mut exp_m: FixpDbl,
    mut exp_e: i32,
    result_e: &mut i32,
) -> FixpDbl {
    if base_m <= 0 {
        *result_e = 0;
        return 0;
    }

    // Calc log2 of base
    let mut baselg2_e = 0;
    let base_lg2 = f_log2(base_m, base_e, &mut baselg2_e);

    // Prepare exp
    let leading_bits = count_leading_bits_of_abs(exp_m);
    exp_m = exp_m.wrapping_shl(leading_bits as u32);
    exp_e = exp_e.wrapping_sub(leading_bits);

    // Calc base pow exp
    let ans_lg2 = f_mult(base_lg2, exp_m);
    let ans_lg2_e = exp_e.wrapping_add(baselg2_e);

    // Calc antilog
    f2_pow(ans_lg2, ans_lg2_e, result_e)
}

/// `fLdPow(FIXP_DBL baseLd_m, INT baseLd_e, FIXP_DBL exp_m, INT exp_e, INT* result_e)`:
/// `x ^ (exp_m * 2^exp_e)` where `log2(x) = baseLd_m * 2^baseLd_e`.
pub fn f_ld_pow(
    base_ld_m: FixpDbl,
    base_ld_e: i32,
    mut exp_m: FixpDbl,
    mut exp_e: i32,
    result_e: &mut i32,
) -> FixpDbl {
    // Prepare exp
    let leading_bits = count_leading_bits_of_abs(exp_m);
    exp_m = exp_m.wrapping_shl(leading_bits as u32);
    exp_e = exp_e.wrapping_sub(leading_bits);

    // Calc base pow exp
    let ans_lg2 = f_mult(base_ld_m, exp_m);
    let ans_lg2_e = exp_e.wrapping_add(base_ld_e);

    // Calc antilog
    f2_pow(ans_lg2, ans_lg2_e, result_e)
}

/// `fLdPow(FIXP_DBL baseLd_m, INT baseLd_e, FIXP_DBL exp_m, INT exp_e)`: mantissa with the
/// implicit exponent 0, saturated with `SATURATE_SHIFT`.
pub fn f_ld_pow_noexp(base_ld_m: FixpDbl, base_ld_e: i32, exp_m: FixpDbl, exp_e: i32) -> FixpDbl {
    let mut result_e = 0;
    let result_m = f_ld_pow(base_ld_m, base_ld_e, exp_m, exp_e, &mut result_e);
    // SATURATE_SHIFT(result_m, -result_e, DFRACT_BITS): the reference builds test
    // `result_e > 0` (`-result_e < 0` without overflow) for the direction.
    if result_e > 0 {
        saturate_left_shift(result_m, result_e, DFRACT_BITS)
    } else {
        saturate_right_shift(result_m, result_e.wrapping_neg(), DFRACT_BITS)
    }
}

/// `fPowInt(FIXP_DBL base_m, INT base_e, INT N, INT* result_e)`: `(base_m * 2^base_e) ^ N`
/// (`base_m >= 0`).
pub fn f_pow_int(mut base_m: FixpDbl, mut base_e: i32, exp: i32, p_result_e: &mut i32) -> FixpDbl {
    if exp != 0 {
        let mut result_e = 0;
        let mut result: FixpDbl;

        if base_m != 0 {
            let leading_bits = count_leading_bits(base_m);
            base_m <<= leading_bits;
            base_e = base_e.wrapping_sub(leading_bits);

            result = base_m;

            // `i < fAbs(exp)` compiles to an unsigned comparison with |exp| (clang assumes that
            // fAbs() is not negative), so exp == INT_MIN loops 2^31 - 1 times.
            let mut i: u32 = 1;
            while i < exp.unsigned_abs() {
                result = f_mult(result, base_m);
                i += 1;
            }

            if exp < 0 {
                // 1.0 / ans
                result = f_div_norm(fl2fxconst_dbl(0.5), result, &mut result_e);
                result_e += 1;
            } else {
                let ans_scale = count_leading_bits(result);
                result <<= ans_scale;
                result_e -= ans_scale;
            }

            result_e = result_e.wrapping_add(exp.wrapping_mul(base_e));
        } else {
            result = 0;
        }
        *p_result_e = result_e;
        result
    } else {
        *p_result_e = 1;
        fl2fxconst_dbl(0.5)
    }
}

/// `CalcLog2(FIXP_DBL arg, INT arg_e, INT* result_e)`: deprecated alias of `fLog2`.
#[inline]
pub fn calc_log2(base_m: FixpDbl, base_e: i32, result_e: &mut i32) -> FixpDbl {
    f_log2(base_m, base_e, result_e)
}

/// `fixp_floorToInt(FIXP_DBL f_inp, INT sf)`: `sf` mantissa bits left of the dot (`sf >= 0`).
pub fn fixp_floor_to_int(f_inp: FixpDbl, sf: i32) -> i32 {
    fdk_assert!(sf >= 0, "FDK_ASSERT(sf >= 0)");
    f_inp.wrapping_shr(((DFRACT_BITS - 1).wrapping_sub(sf)) as u32)
}

/// `fixp_floor(FIXP_DBL f_inp, INT sf)`
pub fn fixp_floor(f_inp: FixpDbl, sf: i32) -> FixpDbl {
    fdk_assert!(sf >= 0, "FDK_ASSERT(sf >= 0)");
    let floor_int = fixp_floor_to_int(f_inp, sf);
    floor_int.wrapping_shl(((DFRACT_BITS - 1).wrapping_sub(sf)) as u32)
}

/// `fixp_ceilToInt(FIXP_DBL f_inp, INT sf)`: `sf` mantissa bits left of the dot.
pub fn fixp_ceil_to_int(f_inp: FixpDbl, sf: i32) -> i32 {
    fdk_assert!(sf >= 0, "FDK_ASSERT(sf >= 0)");
    let sx = (DFRACT_BITS - 1).wrapping_sub(sf); // mantissa bits right of dot
    let inp_int = f_inp;
    let mask = 1i32.wrapping_shl(sx as u32).wrapping_sub(1);
    let mut ceil_int = f_inp.wrapping_shr(sx as u32);
    if inp_int & mask != 0 {
        // increment only, if there is at least one set mantissa bit right of dot
        ceil_int = ceil_int.wrapping_add(1);
    }
    ceil_int
}

/// `fixp_ceil(FIXP_DBL f_inp, INT sf)`
pub fn fixp_ceil(f_inp: FixpDbl, sf: i32) -> FixpDbl {
    fdk_assert!(sf >= 0, "FDK_ASSERT(sf >= 0)");
    let sx = (DFRACT_BITS - 1).wrapping_sub(sf);
    let mut ceil_int = fixp_ceil_to_int(f_inp, sf);
    let mask = 1u32 << (DFRACT_BITS - 1); // 0x80000000
                                          // no fract warn bec. shift into saturation done on int side
    ceil_int = ceil_int.wrapping_shl(sx as u32);
    if f_inp > 0 && (ceil_int as u32 & mask) != 0 {
        ceil_int = ceil_int.wrapping_sub(1);
    }
    ceil_int
}

/// `fixp_truncateToInt(FIXP_DBL f_inp, INT sf)`: remove the fractional part, like a cast of a
/// float to `INT`.
pub fn fixp_truncate_to_int(f_inp: FixpDbl, sf: i32) -> i32 {
    fdk_assert!(sf >= 0, "FDK_ASSERT(sf >= 0)");
    let sx = (DFRACT_BITS - 1).wrapping_sub(sf); // mantissa bits right of dot
    let mut fbaccu = f_inp;
    let mask = 1i32.wrapping_shl(sx as u32);
    if fbaccu < 0 && (fbaccu & mask.wrapping_sub(1)) != 0 {
        fbaccu = fbaccu.wrapping_add(mask);
    }
    fbaccu.wrapping_shr(sx as u32)
}

/// `fixp_truncate(FIXP_DBL f_inp, INT sf)`
pub fn fixp_truncate(f_inp: FixpDbl, sf: i32) -> FixpDbl {
    fdk_assert!(sf >= 0, "FDK_ASSERT(sf >= 0)");
    let truncate_int = fixp_truncate_to_int(f_inp, sf);
    truncate_int.wrapping_shl(((DFRACT_BITS - 1).wrapping_sub(sf)) as u32)
}

/// `fixp_roundToInt(FIXP_DBL f_inp, INT sf)`: round to nearest (`0 <= sf <= 30`; for
/// `sf == 31` the C++ code shifts by -1, which the port takes modulo 32).
pub fn fixp_round_to_int(f_inp: FixpDbl, sf: i32) -> i32 {
    fdk_assert!(sf >= 0, "FDK_ASSERT(sf >= 0)");
    let sx = (DFRACT_BITS - 1).wrapping_sub(sf);
    let inp = f_inp;
    let mask1 = 1i32.wrapping_shl(sx.wrapping_sub(1) as u32);
    let mask2 = 1i32.wrapping_shl(sx as u32).wrapping_sub(1);
    let mask3 = 0x7FFF_FFFF;
    let iam = inp & mask2;

    let mut rnd = if inp < 0 && iam != mask1 {
        inp.wrapping_add(mask1)
    } else if inp > 0 && inp != mask3 {
        inp.wrapping_add(mask1)
    } else {
        inp
    };

    rnd = rnd.wrapping_shr(sx as u32);

    if inp == mask3 {
        rnd = rnd.wrapping_add(1);
    }
    rnd
}

/// `fixp_round(FIXP_DBL f_inp, INT sf)`
pub fn fixp_round(f_inp: FixpDbl, sf: i32) -> FixpDbl {
    fdk_assert!(sf >= 0, "FDK_ASSERT(sf >= 0)");
    let sx = (DFRACT_BITS - 1).wrapping_sub(sf);
    let mut r = fixp_round_to_int(f_inp, sf);
    let mask = 1u32 << (DFRACT_BITS - 1); // 0x80000000
    r = r.wrapping_shl(sx as u32);
    if f_inp > 0 && (r as u32 & mask) != 0 {
        r = r.wrapping_sub(1);
    }
    r
}

// ---------------------------------------------------------------------------------------------
// Tables (generated with scripts/dump_table.py)
// ---------------------------------------------------------------------------------------------

// `ldCoeff` (fixpoint_math.h, file-static): Taylor series coefficients for ln(1-x), centered at
// 0 (MacLaurin polynomial).
// `pow2Coeff` (fixpoint_math.cpp, file-static): Taylor series coefficients of the function 2^x;
// `pow2Coeff[i-1] = ln(2)^i / i!`.
// `exp2_tab_long`, `exp2w_tab_long`, `exp2x_tab_long` (fixpoint_math.cpp): 2^x for x in
// [0, 1) in steps of 1/32, [0, 1/32) in steps of 1/1024, [0, 1/1024) in steps of 1/32768.
// `ldIntCoeff` (fixpoint_math.cpp, file-static, `LD_INT_TAB_LEN == 193`).
// `invSqrtTab`, `sqrt_tab` and `invCount` are defined in FDK_tools_rom.cpp (module tools_rom);
// they are re-exported here.

/// `ldCoeff`
pub static LD_COEFF: [i16; 10] =
    [-32768, -16384, -10923, -8192, -6554, -5461, -4681, -4096, -3641, -3277];

/// `pow2Coeff` (`POW2COEFF_16BIT` is defined for both variants)
///
/// `scripts/dump_table.py libFDK/src/fixpoint_math.cpp pow2Coeff SHORT`
pub static POW2_COEFF: [i16; 8] = [22713, 7872, 1819, 315, 44, 5, 0, 0];

/// `exp2_tab_long`
pub static EXP2_TAB_LONG: [u32; 32] = [
    0x40000000, 0x4166C34C, 0x42D561B4, 0x444C0740, 0x45CAE0F2, 0x47521CC6, 0x48E1E9BA, 0x4A7A77D4,
    0x4C1BF829, 0x4DC69CDD, 0x4F7A9930, 0x51382182, 0x52FF6B55, 0x54D0AD5A, 0x56AC1F75, 0x5891FAC1,
    0x5A82799A, 0x5C7DD7A4, 0x5E8451D0, 0x60962665, 0x62B39509, 0x64DCDEC3, 0x6712460B, 0x69540EC9,
    0x6BA27E65, 0x6DFDDBCC, 0x70666F76, 0x72DC8374, 0x75606374, 0x77F25CCE, 0x7A92BE8B, 0x7D41D96E,
];

/// `exp2w_tab_long`
pub static EXP2W_TAB_LONG: [u32; 32] = [
    0x40000000, 0x400B1818, 0x4016321B, 0x40214E0C, 0x402C6BE9, 0x40378BB4, 0x4042AD6D, 0x404DD113,
    0x4058F6A8, 0x40641E2B, 0x406F479E, 0x407A7300, 0x4085A051, 0x4090CF92, 0x409C00C4, 0x40A733E6,
    0x40B268FA, 0x40BD9FFF, 0x40C8D8F5, 0x40D413DD, 0x40DF50B8, 0x40EA8F86, 0x40F5D046, 0x410112FA,
    0x410C57A2, 0x41179E3D, 0x4122E6CD, 0x412E3152, 0x41397DCC, 0x4144CC3B, 0x41501CA0, 0x415B6EFB,
];

/// `exp2x_tab_long`
pub static EXP2X_TAB_LONG: [u32; 32] = [
    0x40000000, 0x400058B9, 0x4000B173, 0x40010A2D, 0x400162E8, 0x4001BBA3, 0x4002145F, 0x40026D1B,
    0x4002C5D8, 0x40031E95, 0x40037752, 0x4003D011, 0x400428CF, 0x4004818E, 0x4004DA4E, 0x4005330E,
    0x40058BCE, 0x4005E48F, 0x40063D51, 0x40069613, 0x4006EED5, 0x40074798, 0x4007A05B, 0x4007F91F,
    0x400851E4, 0x4008AAA8, 0x4009036E, 0x40095C33, 0x4009B4FA, 0x400A0DC0, 0x400A6688, 0x400ABF4F,
];

/// `ldIntCoeff`
///
/// `scripts/dump_table.py libFDK/src/fixpoint_math.cpp ldIntCoeff LONG --hex`
pub static LD_INT_COEFF: [i32; 193] = [
    0x80000001u32 as i32,
    0x00000000,
    0x02000000,
    0x032B8034,
    0x04000000,
    0x04A4D3C2,
    0x052B8034,
    0x059D5DA0,
    0x06000000,
    0x06570069,
    0x06A4D3C2,
    0x06EB3A9F,
    0x072B8034,
    0x0766A009,
    0x079D5DA0,
    0x07D053F7,
    0x08000000,
    0x082CC7EE,
    0x08570069,
    0x087EF05B,
    0x08A4D3C2,
    0x08C8DDD4,
    0x08EB3A9F,
    0x090C1050,
    0x092B8034,
    0x0949A785,
    0x0966A009,
    0x0982809D,
    0x099D5DA0,
    0x09B74949,
    0x09D053F7,
    0x09E88C6B,
    0x0A000000,
    0x0A16BAD3,
    0x0A2CC7EE,
    0x0A423162,
    0x0A570069,
    0x0A6B3D79,
    0x0A7EF05B,
    0x0A92203D,
    0x0AA4D3C2,
    0x0AB7110E,
    0x0AC8DDD4,
    0x0ADA3F60,
    0x0AEB3A9F,
    0x0AFBD42B,
    0x0B0C1050,
    0x0B1BF312,
    0x0B2B8034,
    0x0B3ABB40,
    0x0B49A785,
    0x0B584822,
    0x0B66A009,
    0x0B74B1FD,
    0x0B82809D,
    0x0B900E61,
    0x0B9D5DA0,
    0x0BAA708F,
    0x0BB74949,
    0x0BC3E9CA,
    0x0BD053F7,
    0x0BDC899B,
    0x0BE88C6B,
    0x0BF45E09,
    0x0C000000,
    0x0C0B73CB,
    0x0C16BAD3,
    0x0C21D671,
    0x0C2CC7EE,
    0x0C379085,
    0x0C423162,
    0x0C4CABA8,
    0x0C570069,
    0x0C6130AF,
    0x0C6B3D79,
    0x0C7527B9,
    0x0C7EF05B,
    0x0C88983F,
    0x0C92203D,
    0x0C9B8926,
    0x0CA4D3C2,
    0x0CAE00D2,
    0x0CB7110E,
    0x0CC0052B,
    0x0CC8DDD4,
    0x0CD19BB0,
    0x0CDA3F60,
    0x0CE2C97D,
    0x0CEB3A9F,
    0x0CF39355,
    0x0CFBD42B,
    0x0D03FDA9,
    0x0D0C1050,
    0x0D140CA0,
    0x0D1BF312,
    0x0D23C41D,
    0x0D2B8034,
    0x0D3327C7,
    0x0D3ABB40,
    0x0D423B08,
    0x0D49A785,
    0x0D510118,
    0x0D584822,
    0x0D5F7CFF,
    0x0D66A009,
    0x0D6DB197,
    0x0D74B1FD,
    0x0D7BA190,
    0x0D82809D,
    0x0D894F75,
    0x0D900E61,
    0x0D96BDAD,
    0x0D9D5DA0,
    0x0DA3EE7F,
    0x0DAA708F,
    0x0DB0E412,
    0x0DB74949,
    0x0DBDA072,
    0x0DC3E9CA,
    0x0DCA258E,
    0x0DD053F7,
    0x0DD6753E,
    0x0DDC899B,
    0x0DE29143,
    0x0DE88C6B,
    0x0DEE7B47,
    0x0DF45E09,
    0x0DFA34E1,
    0x0E000000,
    0x0E05BF94,
    0x0E0B73CB,
    0x0E111CD2,
    0x0E16BAD3,
    0x0E1C4DFB,
    0x0E21D671,
    0x0E275460,
    0x0E2CC7EE,
    0x0E323143,
    0x0E379085,
    0x0E3CE5D8,
    0x0E423162,
    0x0E477346,
    0x0E4CABA8,
    0x0E51DAA8,
    0x0E570069,
    0x0E5C1D0B,
    0x0E6130AF,
    0x0E663B74,
    0x0E6B3D79,
    0x0E7036DB,
    0x0E7527B9,
    0x0E7A1030,
    0x0E7EF05B,
    0x0E83C857,
    0x0E88983F,
    0x0E8D602E,
    0x0E92203D,
    0x0E96D888,
    0x0E9B8926,
    0x0EA03232,
    0x0EA4D3C2,
    0x0EA96DF0,
    0x0EAE00D2,
    0x0EB28C7F,
    0x0EB7110E,
    0x0EBB8E96,
    0x0EC0052B,
    0x0EC474E4,
    0x0EC8DDD4,
    0x0ECD4012,
    0x0ED19BB0,
    0x0ED5F0C4,
    0x0EDA3F60,
    0x0EDE8797,
    0x0EE2C97D,
    0x0EE70525,
    0x0EEB3A9F,
    0x0EEF69FF,
    0x0EF39355,
    0x0EF7B6B4,
    0x0EFBD42B,
    0x0EFFEBCD,
    0x0F03FDA9,
    0x0F0809CF,
    0x0F0C1050,
    0x0F10113B,
    0x0F140CA0,
    0x0F18028D,
    0x0F1BF312,
    0x0F1FDE3D,
    0x0F23C41D,
    0x0F27A4C0,
    0x0F2B8034,
];

pub use crate::tools_rom::{INV_COUNT, INV_SQRT_TAB, SQRT_TAB};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::testutil::Rng;

    #[test]
    fn f_div_norm_equals_straight_port() {
        let mut r = Rng(0x5eed_d1f0);
        let edge = [1, 2, 3, i32::MAX, i32::MAX - 1, 0x4000_0000, 0x3fff_ffff, 0x4000_0001];
        for k in 0..2_000_000 {
            let mut v = || match r.next() % 4 {
                0 => edge[(r.next() % edge.len() as u64) as usize],
                _ => ((r.next() >> 33) as i32 >> r.range(0, 30)).max(1),
            };
            let (n, d) = (if k % 50 == 0 { 0 } else { v() }, v());
            let (mut ea, mut eb) = (7, 7);
            let a = f_div_norm(n, d, &mut ea);
            let b = f_div_norm_ref(n, d, &mut eb);
            assert_eq!((a, ea), (b, eb), "fDivNorm({n}, {d})");
        }
    }

    /// `ld_poly` against the straight port for `x` in `(0, 2^30]`: every power of two and its
    /// neighbours, and a stride through the range (all values with
    /// `FDK_AAC_RS_LD_POLY_EXHAUSTIVE=N`, on N threads; 0: all cores).
    #[test]
    fn ld_poly_equals_straight_port() {
        let check = |x: i32| {
            let (a, b) = ld_poly_pair(x);
            assert_eq!(a, b, "ld_poly({x:#x})");
        };
        for s in 0..=30 {
            for d in -2..=2 {
                let x = (1i32 << s) + d;
                if 0 < x && x <= 1 << 30 {
                    check(x);
                }
            }
        }
        let exhaustive = std::env::var("FDK_AAC_RS_LD_POLY_EXHAUSTIVE").ok();
        let stride: i64 = if exhaustive.is_some() { 1 } else { 1021 };
        let threads = match exhaustive.map(|s| s.parse::<i64>().unwrap_or(0)) {
            None => 1,
            Some(n) if n > 0 => n,
            Some(_) => std::thread::available_parallelism().map_or(4, |n| n.get()) as i64,
        };
        let end: i64 = 1 << 30;
        std::thread::scope(|sc| {
            for t in 0..threads {
                sc.spawn(move || {
                    let mut x = 1 + t * stride;
                    while x <= end {
                        check(x as i32);
                        x += threads * stride;
                    }
                });
            }
        });
    }

    /// `ld_data_vector` against the straight port: edge values, random values of every
    /// magnitude and every length up to 40 (tails), and every 32-bit input with
    /// `FDK_AAC_RS_LD_DATA_EXHAUSTIVE=N` (N threads; 0: all cores).
    #[test]
    fn ld_data_vector_equals_straight_port() {
        let check = |src: &[i32]| {
            let mut a = vec![0x5555_5555; src.len()];
            ld_data_vector(src, &mut a);
            for (i, (&x, &y)) in src.iter().zip(&a).enumerate() {
                assert_eq!(y, f_log2_noexp_ref(x, 0), "LdDataVector {x:#x} (lane {i})");
            }
        };
        let mut edge = vec![0, -1, 1, 2, 3, i32::MIN, i32::MIN + 1, i32::MAX, i32::MAX - 1];
        for s in 0..31 {
            for d in -1..=1 {
                edge.push((1i32 << s).wrapping_add(d));
            }
        }
        check(&edge);
        let mut r = Rng(0x5eed_1d4a);
        for _ in 0..20_000 {
            let n = r.range(0, 40) as usize;
            let v: Vec<i32> = (0..n).map(|_| (r.next() >> 32) as i32 >> r.range(0, 31)).collect();
            check(&v);
        }
        let Ok(t) = std::env::var("FDK_AAC_RS_LD_DATA_EXHAUSTIVE") else { return };
        let threads = match t.parse::<u64>() {
            Ok(n) if n > 0 => n,
            _ => std::thread::available_parallelism().map_or(4, |n| n.get()) as u64,
        };
        std::thread::scope(|sc| {
            for t in 0..threads {
                sc.spawn(move || {
                    let mut src = [0i32; 4096];
                    let mut dst = [0i32; 4096];
                    let mut base = t << 12;
                    while base < 1 << 32 {
                        for (k, s) in src.iter_mut().enumerate() {
                            *s = (base + k as u64) as u32 as i32;
                        }
                        ld_data_vector(&src, &mut dst);
                        for (&x, &y) in src.iter().zip(&dst) {
                            assert_eq!(y, f_log2_noexp_ref(x, 0), "LdDataVector {x:#x}");
                        }
                        base += threads << 12;
                    }
                });
            }
        });
    }

    #[test]
    fn f_log2_equals_straight_port() {
        let mut r = Rng(0x5eed_1062);
        for k in 0..2_000_000 {
            let x_m = match k % 8 {
                0 => [0, -1, 1, i32::MIN, i32::MAX, 0x4000_0000, 0x3fff_ffff]
                    [(r.next() % 7) as usize],
                _ => (r.next() >> 32) as i32 >> r.range(0, 31),
            };
            let x_e = match k % 3 {
                0 => 0,
                _ => (r.next() >> 32) as i32 >> r.range(0, 31),
            };
            let (mut ea, mut eb) = (7, 7);
            let a = f_log2(x_m, x_e, &mut ea);
            let b = f_log2_ref(x_m, x_e, &mut eb);
            assert_eq!((a, ea), (b, eb), "fLog2({x_m:#x}, {x_e})");
            assert_eq!(
                f_log2_noexp(x_m, x_e),
                f_log2_noexp_ref(x_m, x_e),
                "fLog2({x_m:#x}, {x_e})"
            );
        }
    }
}
