// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/sbr_misc.h, libSBRenc/src/sbr_misc.cpp.

//! Port of `sbr_misc.h` and `sbr_misc.cpp`: sorting and vector helpers, and a division with
//! scaling.

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fixpoint_math::schur_div;
use fdk_aac_rs_fdk::{Arch, ARCH};

/// `schur_div(num, denum, count)` ([`fdk_aac_rs_fdk::fixpoint_math::schur_div`]) with the
/// bit-serial division of the `aarch64` and `generic` variants replaced by one integer division
/// where it gives the same quotient: for `0 < a < b` (`a = num >> 1`, `b = denum >> 1`) and
/// `1 <= count <= 32`, the `count - 1` steps of the restoring division keep the remainder below
/// `b` (and `2 * remainder` below 2^31) and produce `floor(a * 2^(count - 1) / b)`. The other
/// inputs (including `a == b`, where every step sets its bit) and the `x86` variants (already a
/// division) call `schur_div`. The bit-serial loop has a data-dependent branch per bit.
#[inline]
pub fn schur_div_fast(num: FixpDbl, denum: FixpDbl, count: i32) -> FixpDbl {
    if matches!(ARCH, Arch::Aarch64 | Arch::Generic) {
        let (a, b) = (num >> 1, denum >> 1);
        if a > 0 && a < b && (1..=DFRACT_BITS).contains(&count) {
            let div = (((a as i64) << (count - 1)) / b as i64) as i32;
            return div.wrapping_shl((DFRACT_BITS - count) as u32);
        }
    }
    schur_div(num, denum, count)
}

/// `FDKsbrEnc_Shellsort_fract(in, n)`: sorts `in[0..n]` ascending.
pub fn fdk_sbr_enc_shellsort_fract(input: &mut [FixpDbl], n: i32) {
    let mut inc: i32 = 1;
    loop {
        inc = 3 * inc + 1;
        if inc > n {
            break;
        }
    }
    loop {
        inc /= 3;
        for i in inc + 1..=n {
            let v = input[(i - 1) as usize];
            let mut j = i;
            while input[(j - inc - 1) as usize] > v {
                input[(j - 1) as usize] = input[(j - inc - 1) as usize];
                j -= inc;
                if j <= inc {
                    break;
                }
            }
            input[(j - 1) as usize] = v;
        }
        if inc <= 1 {
            break;
        }
    }
}

/// `FDKsbrEnc_Shellsort_int(in, n)`: sorts `in[0..n]` ascending.
pub fn fdk_sbr_enc_shellsort_int(input: &mut [i32], n: i32) {
    let mut inc: i32 = 1;
    loop {
        inc = 3 * inc + 1;
        if inc > n {
            break;
        }
    }
    loop {
        inc /= 3;
        for i in inc + 1..=n {
            let v = input[(i - 1) as usize];
            let mut j = i;
            while input[(j - inc - 1) as usize] > v {
                input[(j - 1) as usize] = input[(j - inc - 1) as usize];
                j -= inc;
                if j <= inc {
                    break;
                }
            }
            input[(j - 1) as usize] = v;
        }
        if inc <= 1 {
            break;
        }
    }
}

/// `FDKsbrEnc_AddVecLeft(dst, length_dst, src, length_src)`
pub fn fdk_sbr_enc_add_vec_left(
    dst: &mut [i32],
    length_dst: &mut i32,
    src: &[i32],
    length_src: i32,
) {
    let mut i = length_src - 1;
    while i >= 0 {
        fdk_sbr_enc_add_left(dst, length_dst, src[i as usize]);
        i -= 1;
    }
}

/// `FDKsbrEnc_AddLeft(vector, length_vector, value)`
pub fn fdk_sbr_enc_add_left(vector: &mut [i32], length_vector: &mut i32, value: i32) {
    let mut i = *length_vector;
    while i > 0 {
        vector[i as usize] = vector[(i - 1) as usize];
        i -= 1;
    }
    vector[0] = value;
    *length_vector += 1;
}

/// `FDKsbrEnc_AddRight(vector, length_vector, value)`
pub fn fdk_sbr_enc_add_right(vector: &mut [i32], length_vector: &mut i32, value: i32) {
    vector[*length_vector as usize] = value;
    *length_vector += 1;
}

/// `FDKsbrEnc_AddVecRight(dst, length_dst, src, length_src)`
pub fn fdk_sbr_enc_add_vec_right(
    dst: &mut [i32],
    length_dst: &mut i32,
    src: &[i32],
    length_src: i32,
) {
    for i in 0..length_src.max(0) as usize {
        fdk_sbr_enc_add_right(dst, length_dst, src[i]);
    }
}

/// `FDKsbrEnc_LSI_divide_scale_fract(num, denom, scale)`: `num * scale / denom` with best
/// precision.
///
/// The shifts by `shiftNum + shiftScale - 1` (-1 for `|num|` and `|scale|` of at least 0.5) and
/// by `-shiftCommon` (32 for some denominators) are out of range in C; both reference builds
/// shift by the amount modulo 32 (`asr`/`lsl` with a register amount on AArch64, `sar`/`shl` on
/// x86), which `wrapping_shr`/`wrapping_shl` reproduce.
pub fn fdk_sbr_enc_lsi_divide_scale_fract(
    mut num: FixpDbl,
    mut denom: FixpDbl,
    mut scale: FixpDbl,
) -> FixpDbl {
    let mut tmp: FixpDbl = 0;

    if num != 0 {
        let shift_num = count_leading_bits(num);
        let shift_denom = count_leading_bits(denom);
        let shift_scale = count_leading_bits(scale);

        num <<= shift_num;
        scale <<= shift_scale;

        tmp = f_mult_div2(num, scale);

        // C: tmp >> fixMin(shiftNum + shiftScale - 1, (DFRACT_BITS - 1))
        if denom > tmp.wrapping_shr(fix_min(shift_num + shift_scale - 1, DFRACT_BITS - 1) as u32) {
            denom <<= shift_denom;
            tmp = schur_div_fast(tmp, denom, 15);

            let shift_common = fix_min(shift_num - shift_denom + shift_scale - 1, DFRACT_BITS - 1);
            if shift_common < 0 {
                // C: tmp <<= -shiftCommon
                tmp = tmp.wrapping_shl((-shift_common) as u32);
            } else {
                tmp >>= shift_common;
            }
        } else {
            tmp = MAXVAL_DBL;
        }
    }

    tmp
}
