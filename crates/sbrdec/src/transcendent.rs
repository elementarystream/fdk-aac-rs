// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/transcendent.h.

//! Port of `transcendent.h`: arithmetic on mantissa/exponent pairs (the overloaded
//! `FDK_add_MantExp`, `FDK_divide_MantExp` and `FDK_sqrt_MantExp`, and
//! `FDK_getNumOctavesDiv8`).
//!
//! The functions are inline in C; the arithmetic follows the C expressions (`int` promotion of
//! the `SCHAR` exponents, truncation when the results are stored back into `SCHAR`/`FIXP_SGL`).

use crate::sbr_rom::FDK_SBR_DECODER_INV_TABLE;
use crate::sbr_rom::INV_TABLE_BITS;
use fdk_aac_rs_fdk::common_fix::*;

/// `FDK_getNumOctavesDiv8(a, b)`: `ld(b / a) / 8` as `FIXP_SGL`.
#[inline]
pub fn fdk_get_num_octaves_div8(a: i32, b: i32) -> FixpSgl {
    // (SHORT)((LONG)(CalcLdInt(b) - CalcLdInt(a)) >> (FRACT_BITS - 3)); the table values are
    // ld(i) / 64 >= 0, the difference cannot overflow.
    ((calc_ld_int(b).wrapping_sub(calc_ld_int(a))) >> (FRACT_BITS - 3)) as i16
}

/// `FDK_add_MantExp(FIXP_SGL a_m, SCHAR a_e, FIXP_SGL b_m, SCHAR b_e, FIXP_SGL *ptrSum_m,
/// SCHAR *ptrSum_e)`
#[inline]
pub fn fdk_add_mant_exp_sgl(a_m: FixpSgl, a_e: i8, b_m: FixpSgl, b_e: i8) -> (FixpSgl, i8) {
    // Equalize exponents of the summands. For the smaller summand, the exponent is adapted and
    // for compensation, the mantissa is shifted right.
    let shift = a_e as i32 - b_e as i32;

    let mut shift_abs = if shift > 0 { shift } else { -shift };
    shift_abs = if shift_abs < DFRACT_BITS - 1 { shift_abs } else { DFRACT_BITS - 1 };
    let shifted_mantissa =
        if shift > 0 { fx_sgl2fx_dbl(b_m) >> shift_abs } else { fx_sgl2fx_dbl(a_m) >> shift_abs };
    let other_mantissa = if shift > 0 { fx_sgl2fx_dbl(a_m) } else { fx_sgl2fx_dbl(b_m) };
    let mut sum_e = if shift > 0 { a_e } else { b_e };

    // shift by 1 bit to avoid overflow
    let mut accu = (shifted_mantissa >> 1) + (other_mantissa >> 1);

    if accu >= (fl2fxconst_dbl(0.5) - 1) || accu <= fl2fxconst_dbl(-0.5) {
        // *ptrSum_e += 1 (SCHAR)
        sum_e = sum_e.wrapping_add(1);
    } else {
        // Both mantissas are 16-bit values shifted left by 16; their sum cannot overflow here.
        accu = shifted_mantissa.wrapping_add(other_mantissa);
    }

    (fx_dbl2fx_sgl(accu), sum_e)
}

/// `FDK_add_MantExp(FIXP_DBL a, SCHAR a_e, FIXP_DBL b, SCHAR b_e, FIXP_DBL *ptrSum,
/// SCHAR *ptrSum_e)`
#[inline]
pub fn fdk_add_mant_exp(a: FixpDbl, a_e: i8, b: FixpDbl, b_e: i8) -> (FixpDbl, i8) {
    let shift = a_e as i32 - b_e as i32;

    let mut shift_abs = if shift > 0 { shift } else { -shift };
    shift_abs = if shift_abs < DFRACT_BITS - 1 { shift_abs } else { DFRACT_BITS - 1 };
    let shifted_mantissa = if shift > 0 { b >> shift_abs } else { a >> shift_abs };
    let other_mantissa = if shift > 0 { a } else { b };
    let mut sum_e = if shift > 0 { a_e } else { b_e };

    // shift by 1 bit to avoid overflow
    let mut accu = (shifted_mantissa >> 1) + (other_mantissa >> 1);

    if accu >= (fl2fxconst_dbl(0.5) - 1) || accu <= fl2fxconst_dbl(-0.5) {
        sum_e = sum_e.wrapping_add(1);
    } else {
        // |accu| < 0.5: the sum of the unshifted values cannot overflow.
        accu = shifted_mantissa.wrapping_add(other_mantissa);
    }

    (accu, sum_e)
}

/// The table lookup of `FDK_divide_MantExp()`: `(bInv_m, index < 0)`.
#[inline(always)]
fn inv_lookup(index: i32) -> FixpSgl {
    // index is in -1..=255 (9 valid bits, minus 1, shifted right by 1).
    if index < 0 {
        0
    } else {
        FDK_SBR_DECODER_INV_TABLE[index as usize]
    }
}

/// `FDK_divide_MantExp(FIXP_SGL a_m, SCHAR a_e, FIXP_SGL b_m, SCHAR b_e, FIXP_SGL *ptrResult_m,
/// SCHAR *ptrResult_e)`: division based on a table lookup of `1/b`.
#[inline]
pub fn fdk_divide_mant_exp_sgl(a_m: FixpSgl, a_e: i8, b_m: FixpSgl, b_e: i8) -> (FixpSgl, i8) {
    let pre_shift = cnt_leading_zeros(fx_sgl2fx_dbl(b_m));

    // Shift b into the range from 0..INV_TABLE_SIZE-1 (see transcendent.h).
    let shift = FRACT_BITS - 2 - INV_TABLE_BITS - pre_shift;

    // (shift < 0) ? (LONG)b_m << (-shift) : (LONG)b_m >> shift; pre_shift <= 31 for b_m != 0
    // and 32 for b_m == 0 (shift = -26), the amounts stay below 32.
    let mut index =
        if shift < 0 { (b_m as i32).wrapping_shl((-shift) as u32) } else { (b_m as i32) >> shift };

    // The index has INV_TABLE_BITS +1 valid bits here. Clear the other bits.
    index &= (1 << (INV_TABLE_BITS + 1)) - 1;
    // Remove offset of half an interval
    index -= 1;
    // Now the lowest bit is shifted out
    index >>= 1;

    // Fetch inversed mantissa from table:
    let b_inv_m = inv_lookup(index);

    // Multiply a with the inverse of b:
    let ratio_m = if index < 0 { fx_sgl2fx_dbl(a_m >> 1) } else { f_mult_div2(b_inv_m, a_m) };

    let post_shift = cnt_leading_zeros(ratio_m) - 1;

    // ratio_m << postShift: postShift is -1 for negative ratios (a_m < 0), a left shift by -1
    // (undefined); the AArch64 and x86 builds use lsl/shl with the amount modulo 32.
    let result_m = fx_dbl2fx_sgl(ratio_m.wrapping_shl(post_shift as u32));
    let result_e = (a_e as i32 - b_e as i32 + 1 + pre_shift - post_shift) as i8;
    (result_m, result_e)
}

/// `FDK_divide_MantExp(FIXP_DBL a_m, SCHAR a_e, FIXP_DBL b_m, SCHAR b_e, FIXP_DBL *ptrResult_m,
/// SCHAR *ptrResult_e)`: division based on a table lookup of `1/b`.
#[inline]
pub fn fdk_divide_mant_exp(a_m: FixpDbl, a_e: i8, b_m: FixpDbl, b_e: i8) -> (FixpDbl, i8) {
    let pre_shift = cnt_leading_zeros(b_m);

    let shift = DFRACT_BITS - 2 - INV_TABLE_BITS - pre_shift;

    // (shift < 0) ? (LONG)b_m << (-shift) : (LONG)b_m >> shift; for b_m == 0 pre_shift is 32 and
    // shift -10.
    let mut index = if shift < 0 { b_m.wrapping_shl((-shift) as u32) } else { b_m >> shift };

    index &= (1 << (INV_TABLE_BITS + 1)) - 1;
    index -= 1;
    index >>= 1;

    let b_inv_m = inv_lookup(index);

    let ratio_m = if index < 0 { a_m >> 1 } else { f_mult_div2(b_inv_m, a_m) };

    let post_shift = cnt_leading_zeros(ratio_m) - 1;

    // See the FIXP_SGL variant for postShift == -1.
    let result_m = ratio_m.wrapping_shl(post_shift as u32);
    let result_e = (a_e as i32 - b_e as i32 + 1 + pre_shift - post_shift) as i8;
    (result_m, result_e)
}

/// `FDK_sqrt_MantExp(FIXP_DBL *mantissa, SCHAR *exponent, const SCHAR *destScale)`: square
/// root by table lookup. `dest_scale` is `None` if `destScale` points to `exponent` itself (the
/// result keeps its own exponent), otherwise the target exponent.
#[inline]
pub fn fdk_sqrt_mant_exp(mantissa: &mut FixpDbl, exponent: &mut i8, dest_scale: Option<i8>) {
    let input_m = *mantissa;
    let mut input_e = *exponent as i32;

    // Call lookup square root, which does internally normalization.
    let result = sqrt_fixp_lookup_e(input_m, &mut input_e);
    let result_e = input_e;

    // Write result
    match dest_scale {
        None => {
            *mantissa = result;
            *exponent = result_e as i8;
        }
        Some(dest) => {
            let shift = result_e - dest as i32;
            *mantissa = if shift >= 0 {
                result << fix_min(DFRACT_BITS - 1, shift)
            } else {
                result >> fix_min(DFRACT_BITS - 1, -shift)
            };
            *exponent = dest;
        }
    }
}

/// `fDivNorm(L_num, L_denum, result_e)` (`fixpoint_math.cpp`), optimized for the SBR decoder:
/// the same value as [`f_div_norm`] on every variant.
///
/// On the `aarch64` and `generic` variants `fDivNorm()` divides with the bit-serial
/// `schur_div(num, denum, 16)`: after the normalization `num` lies in `[2^29, 2^30)` and `denum`
/// in `[2^30, 2^31)`, so `l_num = num >> 1 < l_denum = denum >> 1`, the restoring division never
/// overflows and its 15 quotient bits are `(l_num << 15) / l_denum`, computed here with one
/// integer division. Inputs outside the domain of `fDivNorm()` (`L_num < 0`, `L_denum <= 0`) and
/// the `x86` variants (whose `schur_div` already divides) use [`f_div_norm`].
#[inline]
pub fn f_div_norm_fast(l_num: FixpDbl, l_denum: FixpDbl, result_e: &mut i32) -> FixpDbl {
    use fdk_aac_rs_fdk::{Arch, ARCH};
    if matches!(ARCH, Arch::X86 | Arch::X86_32) || l_num < 0 || l_denum <= 0 {
        return f_div_norm(l_num, l_denum, result_e);
    }
    if l_num == 0 {
        *result_e = 0;
        return 0;
    }
    let norm_num = count_leading_bits(l_num);
    let num = (l_num << norm_num) >> 1;
    let norm_den = count_leading_bits(l_denum);
    let den = l_denum << norm_den;
    *result_e = -norm_num + 1 + norm_den;
    let div = (((num >> 1) as u64) << 15) / ((den >> 1) as u64);
    (div as i32) << (DFRACT_BITS - FRACT_BITS)
}
