// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/scale.h, libFDK/src/scale.cpp, libFDK/include/arm/scale_arm.h, libFDK/src/arm/scale_arm.cpp.

//! Port of `scale.h` and `scale.cpp`: scaling operations.
//!
//! Variants:
//!
//! - `scaleValuesWithFactor`: `aarch64` compiles `arm/scale_arm.cpp`, which limits the shift
//!   `scalefactor + 1` to 31 before it is negated, so right shifts are not limited; `x86` and
//!   `generic` compile the generic `scale.cpp`, which limits both shift directions to 31.
//! - `SATURATE_RIGHT_SHIFT` and `SATURATE_LEFT_SHIFT`: `arm/scale_arm.h` defines them with an
//!   XOR of the sign instead of two comparisons. Both forms give the same value for every shift
//!   amount in 0..=31.
//! - `scaleValueInPlace`: `arm/scale_arm.h` has its own copy of the generic code.
//!
//! The vector functions take slices; the C length is the length of the slice (of `src` for the
//! `dst`/`src` functions, of which `dst` must hold at least as many elements).
//!
//! Shift amounts of 32 or more are undefined in C. Where the C code can shift by such amounts,
//! the port uses `wrapping_shl`/`wrapping_shr`, which take the amount modulo 32 like the `lsl`,
//! `asr` (AArch64) and `shl`, `sar` (x86) instructions with a register amount that the reference
//! builds use.
//!
//! `scaleCplxValues` is declared in `scale.h` but not defined anywhere in fdk-aac, so it is not
//! ported. The second `getScalefactorShort` of `scale.cpp` (with a stride) is compiled out by
//! `FUNCTION_getScalefactorShort`.

use crate::common_fix::*;
use crate::machine_type::{IntPcm, SAMPLE_BITS, SHORT_BITS};

/// `scaleValue(FIXP_DBL value, INT scalefactor)`: `value << scalefactor` for `scalefactor > 0`,
/// `value >> -scalefactor` otherwise. Amounts outside of 0..=31 are taken modulo 32.
#[inline(always)]
pub const fn scale_value(value: FixpDbl, scalefactor: i32) -> FixpDbl {
    if scalefactor > 0 {
        value.wrapping_shl(scalefactor as u32)
    } else {
        value.wrapping_shr(scalefactor.wrapping_neg() as u32)
    }
}

/// `scaleValue(FIXP_SGL value, INT scalefactor)`: computed in `int`, truncated to `FIXP_SGL`.
#[inline(always)]
pub const fn scale_value_sgl(value: FixpSgl, scalefactor: i32) -> FixpSgl {
    if scalefactor > 0 {
        (value as i32).wrapping_shl(scalefactor as u32) as i16
    } else {
        (value as i32).wrapping_shr(scalefactor.wrapping_neg() as u32) as i16
    }
}

/// `fixmin_I(-scalefactor, limit)` for `scalefactor <= 0` as the reference builds compute it:
/// clang assumes that `-scalefactor` does not overflow and limits it with an unsigned comparison
/// (`neg`, `cmp`, `csneg` on arm64), so `INT_MIN` gives `limit`.
#[inline(always)]
const fn neg_limit(scalefactor: i32, limit: i32) -> i32 {
    let n = -(scalefactor as i64);
    if n < limit as i64 {
        n as i32
    } else {
        limit
    }
}

/// `scaleValueSaturate(FIXP_DBL value, INT scalefactor)` (the same on every variant): multiply by
/// `2^scalefactor` and saturate to `MINVAL_DBL + 1 ..= MAXVAL_DBL`. Right shifts that shift out
/// all significant bits give 0, also for negative values.
///
/// Quirk: for `scalefactor >= 32`, the headroom test saturates every value, also 0, which gives
/// `MINVAL_DBL + 1`.
#[inline(always)]
pub const fn scale_value_saturate(value: FixpDbl, scalefactor: i32) -> FixpDbl {
    // headroom in range 1...32
    let headroom = fixnormz_d(value ^ (value >> 31));
    if scalefactor >= 0 {
        // shift left: saturate in case of headroom less/equal scalefactor
        if headroom <= scalefactor {
            if value > 0 {
                MAXVAL_DBL
            } else {
                MINVAL_DBL + 1
            }
        } else {
            fixmax_d(value << scalefactor, MINVAL_DBL + 1)
        }
    } else {
        // -scalefactor is computed without overflow: the reference builds compare
        // `32 - headroom + scalefactor` with 0 (clang assumes that the negation does not
        // overflow), so INT_MIN clears the value. The shift below is always in 1..=31.
        let scalefactor = -(scalefactor as i64);
        // shift right: clear in case of 32-headroom greater/equal -scalefactor
        if (DFRACT_BITS - headroom) as i64 <= scalefactor {
            0
        } else {
            fixmax_d(value >> scalefactor, MINVAL_DBL + 1)
        }
    }
}

/// `scaleValueInPlace(FIXP_DBL* value, INT scalefactor)` (the generic code; `arm/scale_arm.h`
/// has an identical copy). Amounts outside of 0..=31 are taken modulo 32.
#[inline(always)]
pub fn scale_value_in_place(value: &mut FixpDbl, scalefactor: i32) {
    if scalefactor >= 0 {
        *value = value.wrapping_shl(scalefactor as u32);
    } else {
        *value = value.wrapping_shr(scalefactor.wrapping_neg() as u32);
    }
}

/// `((1U << (dBits - 1)) - 1)` as `LONG`
#[inline(always)]
const fn sat_max(dbits: i32) -> i32 {
    (1u32.wrapping_shl((dbits - 1) as u32).wrapping_sub(1)) as i32
}

/// `SATURATE_RIGHT_SHIFT(src, scale, dBits)`: `src >> scale`, saturated to `dBits` bits.
#[inline(always)]
pub const fn saturate_right_shift(src: i32, scale: i32, dbits: i32) -> i32 {
    let max = sat_max(dbits);
    if ARCH_ARM {
        // arm/scale_arm.h
        if (src ^ (src >> (DFRACT_BITS - 1))).wrapping_shr(scale as u32) > max {
            (src >> (DFRACT_BITS - 1)) ^ max
        } else {
            src.wrapping_shr(scale as u32)
        }
    } else {
        let v = src.wrapping_shr(scale as u32);
        if v > max {
            max
        } else if v < !max {
            !max
        } else {
            v
        }
    }
}

/// `SATURATE_LEFT_SHIFT(src, scale, dBits)`: `src << scale`, saturated to `dBits` bits.
#[inline(always)]
pub const fn saturate_left_shift(src: i32, scale: i32, dbits: i32) -> i32 {
    let max = sat_max(dbits);
    if ARCH_ARM {
        // arm/scale_arm.h
        if (src ^ (src >> (DFRACT_BITS - 1))) > max.wrapping_shr(scale as u32) {
            (src >> (DFRACT_BITS - 1)) ^ max
        } else {
            src.wrapping_shl(scale as u32)
        }
    } else if src > max.wrapping_shr(scale as u32) {
        max
    } else if src < !max.wrapping_shr(scale as u32) {
        !max
    } else {
        src.wrapping_shl(scale as u32)
    }
}

/// `SATURATE_SHIFT(src, scale, dBits)`: a left shift by `-scale` for `scale < 0`, a right shift
/// by `scale` otherwise, saturated to `dBits` bits.
#[inline(always)]
pub const fn saturate_shift(src: i32, scale: i32, dbits: i32) -> i32 {
    if scale < 0 {
        saturate_left_shift(src, scale.wrapping_neg(), dbits)
    } else {
        saturate_right_shift(src, scale, dbits)
    }
}

/// `SATURATE_LEFT_SHIFT_ALT(src, scale, dBits)`: saturates to `-max` instead of `-max - 1`
/// (the same on every variant).
#[inline(always)]
pub const fn saturate_left_shift_alt(src: i32, scale: i32, dbits: i32) -> i32 {
    let max = sat_max(dbits);
    let max2 = (1u32.wrapping_shl((dbits - 1) as u32).wrapping_sub(2)) as i32;
    if src > max.wrapping_shr(scale as u32) {
        max
    } else if src <= !max.wrapping_shr(scale as u32) {
        !max2
    } else {
        src.wrapping_shl(scale as u32)
    }
}

/// `SATURATE_RIGHT_SHIFT_ALT(src, scale, dBits)`: saturates to `-max` instead of `-max - 1`
/// (the same on every variant).
#[inline(always)]
pub const fn saturate_right_shift_alt(src: i32, scale: i32, dbits: i32) -> i32 {
    let max = sat_max(dbits);
    let max2 = (1u32.wrapping_shl((dbits - 1) as u32).wrapping_sub(2)) as i32;
    let v = src.wrapping_shr(scale as u32);
    if v > max {
        max
    } else if v < !max2 {
        !max2
    } else {
        v
    }
}

/// `SATURATE_INT_PCM_RIGHT_SHIFT(src, scale)`: `SATURATE_RIGHT_SHIFT(src, scale, SAMPLE_BITS)`
#[inline(always)]
pub const fn saturate_int_pcm_right_shift(src: i32, scale: i32) -> i32 {
    saturate_right_shift(src, scale, SAMPLE_BITS)
}

/// `SATURATE_INT_PCM_LEFT_SHIFT(src, scale)`: `SATURATE_LEFT_SHIFT(src, scale, SAMPLE_BITS)`
#[inline(always)]
pub const fn saturate_int_pcm_left_shift(src: i32, scale: i32) -> i32 {
    saturate_left_shift(src, scale, SAMPLE_BITS)
}

/// `scaleValues(FIXP_SGL* vector, INT len, INT scalefactor)`: shifts limited to 15, computed in
/// `int` and truncated.
pub fn scale_values_sgl(vector: &mut [FixpSgl], scalefactor: i32) {
    // Return if scalefactor is Zero
    if scalefactor == 0 {
        return;
    }
    if scalefactor > 0 {
        let s = fixmin_i(scalefactor, FRACT_BITS - 1);
        for v in vector.iter_mut() {
            *v = ((*v as i32) << s) as i16;
        }
    } else {
        let s = neg_limit(scalefactor, FRACT_BITS - 1);
        for v in vector.iter_mut() {
            *v = (*v as i32).wrapping_shr(s as u32) as i16;
        }
    }
}

/// `scaleValues(FIXP_DBL* vector, INT len, INT scalefactor)`: shifts limited to 31.
pub fn scale_values(vector: &mut [FixpDbl], scalefactor: i32) {
    // Return if scalefactor is Zero
    if scalefactor == 0 {
        return;
    }
    if scalefactor > 0 {
        let s = fixmin_i(scalefactor, DFRACT_BITS - 1);
        for v in vector.iter_mut() {
            *v <<= s;
        }
    } else {
        let s = neg_limit(scalefactor, DFRACT_BITS - 1);
        for v in vector.iter_mut() {
            *v = v.wrapping_shr(s as u32);
        }
    }
}

/// `scaleValues(FIXP_DBL* dst, const FIXP_DBL* src, INT len, INT scalefactor)` for
/// `len = src.len()`. For `dst == src` use [`scale_values`].
pub fn scale_values_to(dst: &mut [FixpDbl], src: &[FixpDbl], scalefactor: i32) {
    let dst = &mut dst[..src.len()];
    // Return if scalefactor is Zero
    if scalefactor == 0 {
        dst.copy_from_slice(src);
    } else if scalefactor > 0 {
        let s = fixmin_i(scalefactor, DFRACT_BITS - 1);
        for (d, &x) in dst.iter_mut().zip(src) {
            *d = x << s;
        }
    } else {
        let s = neg_limit(scalefactor, DFRACT_BITS - 1);
        for (d, &x) in dst.iter_mut().zip(src) {
            *d = x.wrapping_shr(s as u32);
        }
    }
}

/// `scaleValues(FIXP_PCM* dst, const FIXP_DBL* src, INT len, INT scalefactor)`
/// (`SAMPLE_BITS == 16`) for `len = src.len()`: shift by `scalefactor - 16`, limited to 31 in
/// both directions, and truncate to `INT_PCM`.
pub fn scale_values_pcm_to(dst: &mut [IntPcm], src: &[FixpDbl], scalefactor: i32) {
    let dst = &mut dst[..src.len()];
    // `scalefactor - 16` does not overflow in the reference builds (clang assumes so and limits
    // the mathematical difference), so it is computed in 64 bits.
    let scalefactor = scalefactor as i64 - (DFRACT_BITS - SAMPLE_BITS) as i64;
    if scalefactor > 0 {
        let s = scalefactor.min((DFRACT_BITS - 1) as i64) as i32;
        for (d, &x) in dst.iter_mut().zip(src) {
            *d = (x << s) as IntPcm;
        }
    } else {
        let s = (-scalefactor).min((DFRACT_BITS - 1) as i64) as i32;
        for (d, &x) in dst.iter_mut().zip(src) {
            *d = x.wrapping_shr(s as u32) as IntPcm;
        }
    }
}

/// `scaleValues(FIXP_SGL* dst, const FIXP_SGL* src, INT len, INT scalefactor)`: shifts limited
/// to 31 (not 15), computed in `int` and truncated.
///
/// Quirk: for `scalefactor == 0`, the reference copies `len * sizeof(FIXP_DBL)` bytes, i.e.
/// `2 * len` values, from `src` to `dst`. The port copies as many of these `2 * len` values as
/// both slices hold (shorter slices have no defined result in C).
pub fn scale_values_sgl_to(dst: &mut [FixpSgl], src: &[FixpSgl], len: usize, scalefactor: i32) {
    if scalefactor == 0 {
        let n = (2 * len).min(dst.len()).min(src.len());
        dst[..n].copy_from_slice(&src[..n]);
    } else if scalefactor > 0 {
        let s = fixmin_i(scalefactor, DFRACT_BITS - 1);
        for (d, &x) in dst[..len].iter_mut().zip(&src[..len]) {
            *d = ((x as i32) << s) as i16;
        }
    } else {
        let s = neg_limit(scalefactor, DFRACT_BITS - 1);
        for (d, &x) in dst[..len].iter_mut().zip(&src[..len]) {
            *d = (x as i32).wrapping_shr(s as u32) as i16;
        }
    }
}

/// `scaleValuesWithFactor(FIXP_DBL* vector, FIXP_DBL factor, INT len, INT scalefactor)`:
/// `vector[i] = fMultDiv2(vector[i], factor) * 2^(scalefactor + 1)`.
///
/// `aarch64` (`arm/scale_arm.cpp`): `shift = fixmin_I(scalefactor + 1, 31)`; a negative `shift`
/// is negated without a limit, so right shifts can reach 32 or more for scalefactors below -32.
/// The arm64 reference compiles this to `asr` with a register amount (modulo 32), which
/// `wrapping_shr` reproduces. The binary computes `fixmin(scalefactor, 30) + 1`, which is what
/// the port does too (it differs from the C source only for `scalefactor == INT_MAX`).
///
/// `x86`, `generic` (`scale.cpp`): both shift directions are limited to 31.
pub fn scale_values_with_factor(vector: &mut [FixpDbl], factor: FixpDbl, scalefactor: i32) {
    if ARCH_ARM {
        // This code combines the fMult with the scaling. It performs a fMultDiv2 and increments
        // shift by 1.
        let shift = fixmin_i(scalefactor, DFRACT_BITS - 2) + 1;
        if shift >= 0 {
            for v in vector.iter_mut() {
                *v = fixmuldiv2_dd(*v, factor).wrapping_shl(shift as u32);
            }
        } else {
            let shift = shift.wrapping_neg();
            for v in vector.iter_mut() {
                *v = fixmuldiv2_dd(*v, factor).wrapping_shr(shift as u32);
            }
        }
    } else {
        // Compensate fMultDiv2. `scalefactor + 1` does not overflow in the reference builds
        // (clang assumes so), so it is computed in 64 bits.
        let scalefactor = scalefactor as i64 + 1;
        if scalefactor > 0 {
            let s = scalefactor.min((DFRACT_BITS - 1) as i64) as i32;
            for v in vector.iter_mut() {
                *v = fixmuldiv2_dd(*v, factor) << s;
            }
        } else {
            let s = (-scalefactor).min((DFRACT_BITS - 1) as i64) as i32;
            for v in vector.iter_mut() {
                *v = fixmuldiv2_dd(*v, factor).wrapping_shr(s as u32);
            }
        }
    }
}

/// The scalefactor of the `scaleValuesSaturate` functions, limited to -31..=31.
#[inline(always)]
const fn saturate_scalefactor(scalefactor: i32) -> i32 {
    fixmax_i(fixmin_i(scalefactor, DFRACT_BITS - 1), -(DFRACT_BITS - 1))
}

/// `scaleValueSaturate(x, s)` for `s` in `0..=31` without the headroom test: the values above
/// `MAXVAL_DBL >> s` saturate to `MAXVAL_DBL`, the others give
/// `max(max(x, MINVAL_DBL >> s) << s, MINVAL_DBL + 1)`.
///
/// The same value: `headroom <= s` holds exactly for `x > MAXVAL_DBL >> s` (`x >= 0`, `clz(x) <=
/// s`) and for `x < MINVAL_DBL >> s` (`x < 0`, `clz(!x) <= s`), which saturate to `MAXVAL_DBL`
/// and `MINVAL_DBL + 1`; the latter is also `max(x, MINVAL_DBL >> s) << s = MINVAL_DBL` raised by
/// the `max`. The other values are shifted without overflow.
#[inline(always)]
const fn sat_shl(x: FixpDbl, s: u32) -> FixpDbl {
    let y = fixmax_d(fixmax_d(x, MINVAL_DBL >> s) << s, MINVAL_DBL + 1);
    if x > MAXVAL_DBL >> s {
        MAXVAL_DBL
    } else {
        y
    }
}

/// `scaleValueSaturate(x, -k)` for `k` in `1..=31` without the headroom test: `x >> k`, but 0
/// for `x >> k == -1`.
///
/// The same value: `32 - headroom <= k` holds exactly when all significant bits are shifted out,
/// that is `x >> k` is 0 or -1, and the result is then 0; otherwise `x >> k` lies in
/// `MINVAL_DBL >> k ..= MAXVAL_DBL >> k`, so the `max` with `MINVAL_DBL + 1` has no effect.
#[inline(always)]
const fn sat_shr(x: FixpDbl, k: u32) -> FixpDbl {
    let y = x >> k;
    if y == -1 {
        0
    } else {
        y
    }
}

crate::avx2_clone! {
/// `x = scaleValueSaturate(x, s)` for every value, `s` in `-31..=31` (branch-free, vectorized).
fn sat_in_place(v: &mut [FixpDbl], s: i32) {
    if s >= 0 {
        let s = s as u32;
        v.iter_mut().for_each(|x| *x = sat_shl(*x, s));
    } else {
        let k = s.unsigned_abs();
        v.iter_mut().for_each(|x| *x = sat_shr(*x, k));
    }
}
}

crate::avx2_clone! {
/// `dst[i] = scaleValueSaturate(src[i], s)`, `s` in `-31..=31` (branch-free, vectorized).
fn sat_copy(dst: &mut [FixpDbl], src: &[FixpDbl], s: i32) {
    let dst = &mut dst[..src.len()];
    if s >= 0 {
        let s = s as u32;
        dst.iter_mut().zip(src).for_each(|(d, &x)| *d = sat_shl(x, s));
    } else {
        let k = s.unsigned_abs();
        dst.iter_mut().zip(src).for_each(|(d, &x)| *d = sat_shr(x, k));
    }
}
}

/// `FX_DBL2FX_SGL(fAddSaturate(x, 0x8000))`: `(x >> 1) + 0x4000` cannot fall below
/// `MINVAL_DBL >> 1`, so only the upper limit of `fAddSaturate` applies.
#[inline(always)]
const fn round_sgl(x: FixpDbl) -> FixpSgl {
    (fixmin_i((x >> 1) + 0x4000, MAXVAL_DBL >> 1) >> 15) as FixpSgl
}

crate::avx2_clone! {
/// [`scale_values_saturate_dbl_to_sgl`] for `s` in `-31..=31` (branch-free, vectorized).
fn sat_dbl_to_sgl(dst: &mut [FixpSgl], src: &[FixpDbl], s: i32) {
    let dst = &mut dst[..src.len()];
    if s >= 0 {
        let s = s as u32;
        dst.iter_mut().zip(src).for_each(|(d, &x)| *d = round_sgl(sat_shl(x, s)));
    } else {
        // For k >= 1 the shifted value has at least one bit of headroom, so the upper limit of
        // round_sgl does not apply, and sat_shr's -1 -> 0 makes no difference after the
        // rounding: (-1 >> 1) + 0x4000 and 0x4000 both give 0 after >> 15.
        let k = s.unsigned_abs();
        dst.iter_mut().zip(src).for_each(|(d, &x)| *d = ((((x >> k) >> 1) + 0x4000) >> 15) as FixpSgl);
    }
}
}

/// `scaleValuesSaturate(FIXP_DBL* vector, INT len, INT scalefactor)`
///
/// Branch-free per value (see `sat_shl`, `sat_shr`), vectorized by the compiler, AVX2 if
/// available ([`crate::simd`]); the straight port is [`scale_values_saturate_ref`].
pub fn scale_values_saturate(vector: &mut [FixpDbl], scalefactor: i32) {
    // Return if scalefactor is Zero
    if scalefactor == 0 {
        return;
    }
    sat_in_place(vector, saturate_scalefactor(scalefactor));
}

/// `scaleValuesSaturate(FIXP_DBL* vector, INT len, INT scalefactor)`: the straight port of
/// [`scale_values_saturate`].
pub fn scale_values_saturate_ref(vector: &mut [FixpDbl], scalefactor: i32) {
    // Return if scalefactor is Zero
    if scalefactor == 0 {
        return;
    }
    let s = saturate_scalefactor(scalefactor);
    for v in vector.iter_mut() {
        *v = scale_value_saturate(*v, s);
    }
}

/// `scaleValuesSaturate(FIXP_DBL* dst, const FIXP_DBL* src, INT len, INT scalefactor)` for
/// `len = src.len()`.
///
/// Branch-free per value, see [`scale_values_saturate`]; the straight port is
/// [`scale_values_saturate_to_ref`].
pub fn scale_values_saturate_to(dst: &mut [FixpDbl], src: &[FixpDbl], scalefactor: i32) {
    let dst = &mut dst[..src.len()];
    // Return if scalefactor is Zero
    if scalefactor == 0 {
        dst.copy_from_slice(src);
        return;
    }
    sat_copy(dst, src, saturate_scalefactor(scalefactor));
}

/// `scaleValuesSaturate(FIXP_DBL* dst, const FIXP_DBL* src, INT len, INT scalefactor)`: the
/// straight port of [`scale_values_saturate_to`].
pub fn scale_values_saturate_to_ref(dst: &mut [FixpDbl], src: &[FixpDbl], scalefactor: i32) {
    let dst = &mut dst[..src.len()];
    // Return if scalefactor is Zero
    if scalefactor == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let s = saturate_scalefactor(scalefactor);
    for (d, &x) in dst.iter_mut().zip(src) {
        *d = scale_value_saturate(x, s);
    }
}

/// `scaleValuesSaturate(FIXP_SGL* dst, const FIXP_DBL* src, INT len, INT scalefactor)` for
/// `len = src.len()`: rounded to `FIXP_SGL` with `fAddSaturate(x, 0x8000)`.
///
/// Branch-free per value, see [`scale_values_saturate`]; the straight port is
/// [`scale_values_saturate_dbl_to_sgl_ref`].
pub fn scale_values_saturate_dbl_to_sgl(dst: &mut [FixpSgl], src: &[FixpDbl], scalefactor: i32) {
    sat_dbl_to_sgl(dst, src, saturate_scalefactor(scalefactor));
}

/// `scaleValuesSaturate(FIXP_SGL* dst, const FIXP_DBL* src, INT len, INT scalefactor)`: the
/// straight port of [`scale_values_saturate_dbl_to_sgl`].
pub fn scale_values_saturate_dbl_to_sgl_ref(
    dst: &mut [FixpSgl],
    src: &[FixpDbl],
    scalefactor: i32,
) {
    let dst = &mut dst[..src.len()];
    let s = saturate_scalefactor(scalefactor);
    for (d, &x) in dst.iter_mut().zip(src) {
        *d = fx_dbl2fx_sgl(f_add_saturate(scale_value_saturate(x, s), 0x8000));
    }
}

/// `scaleValuesSaturate(FIXP_SGL* vector, INT len, INT scalefactor)`
pub fn scale_values_saturate_sgl(vector: &mut [FixpSgl], scalefactor: i32) {
    // Return if scalefactor is Zero
    if scalefactor == 0 {
        return;
    }
    let s = saturate_scalefactor(scalefactor);
    for v in vector.iter_mut() {
        *v = fx_dbl2fx_sgl(scale_value_saturate(fx_sgl2fx_dbl(*v), s));
    }
}

/// `scaleValuesSaturate(FIXP_SGL* dst, const FIXP_SGL* src, INT len, INT scalefactor)` for
/// `len = src.len()`.
pub fn scale_values_saturate_sgl_to(dst: &mut [FixpSgl], src: &[FixpSgl], scalefactor: i32) {
    let dst = &mut dst[..src.len()];
    // Return if scalefactor is Zero
    if scalefactor == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let s = saturate_scalefactor(scalefactor);
    for (d, &x) in dst.iter_mut().zip(src) {
        *d = fx_dbl2fx_sgl(scale_value_saturate(fx_sgl2fx_dbl(x), s));
    }
}

/// `getScalefactorShort(const SHORT* vector, INT len)`: headroom of a vector of 16 bit values.
pub fn get_scalefactor_short(vector: &[i16]) -> i32 {
    let mut max_val: i16 = 0;
    for &temp in vector {
        max_val |= temp ^ (temp >> (SHORT_BITS - 1));
    }
    fixmax_i(0, fixnormz_d(max_val as i32) - 1 - (DFRACT_BITS - SHORT_BITS))
}

/// `getScalefactorPCM(const INT_PCM* vector, INT len, INT stride)`: headroom of every
/// `stride`-th of the first `len` values. `vector` must hold `(len - 1) * stride + 1` values.
pub fn get_scalefactor_pcm(vector: &[IntPcm], len: usize, stride: usize) -> i32 {
    let mut max_val: IntPcm = 0;
    for i in 0..len {
        let temp = vector[i * stride];
        max_val |= temp ^ (temp >> (SAMPLE_BITS - 1));
    }
    fixmax_i(0, fixnormz_d(max_val as i32) - 1 - (DFRACT_BITS - SAMPLE_BITS))
}

/// `getScalefactor(const FIXP_DBL* vector, INT len)`: headroom of the vector, 31 if all values
/// are 0 or -1.
///
/// Note (from the C++ code): if the vector contains negative values, the result may be the
/// headroom of the actual maximum minus 1.
pub fn get_scalefactor(vector: &[FixpDbl]) -> i32 {
    let mut max_val: FixpDbl = 0;
    for &temp in vector {
        max_val |= temp ^ (temp >> (DFRACT_BITS - 1));
    }
    fixmax_i(0, fixnormz_d(max_val) - 1)
}

/// `getScalefactor(const FIXP_SGL* vector, INT len)`
pub fn get_scalefactor_sgl(vector: &[FixpSgl]) -> i32 {
    let mut max_val: i16 = 0;
    for &temp in vector {
        max_val |= temp ^ (temp >> (FRACT_BITS - 1));
    }
    fixmax_i(0, fixnormz_s(max_val) - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::testutil::*;

    /// Values around the saturation limits of every shift, the extremes and random values.
    fn values(r: &mut Rng, n: usize) -> Vec<i32> {
        (0..n)
            .map(|_| match r.next() % 4 {
                0 => {
                    let k = r.range(0, 31) as u32;
                    // (wrapping for k = 31: the same values in every profile)
                    let p = 1i32 << k;
                    let b = [MAXVAL_DBL >> k, MINVAL_DBL >> k, p.wrapping_sub(1), p.wrapping_neg()]
                        [r.range(0, 3) as usize];
                    b.wrapping_add(r.range(-2, 2))
                }
                1 => [0, -1, 1, i32::MIN, i32::MAX, i32::MIN + 1][r.range(0, 5) as usize],
                _ => r.i32() >> r.range(0, 31),
            })
            .collect()
    }

    #[test]
    fn scale_values_saturate_equals_straight_port() {
        let mut r = Rng(0x5eed_5ca1);
        for case in 0..30000 {
            let n = r.range(0, 70) as usize;
            let src = values(&mut r, n);
            let s = match r.next() % 8 {
                0 => r.range(-100, 100),
                1 => [i32::MIN, i32::MAX, -32, 32, -31, 31, 0][r.range(0, 6) as usize],
                _ => r.range(-31, 31),
            };
            let (mut a, mut b) = (src.clone(), src.clone());
            scale_values_saturate(&mut a, s);
            scale_values_saturate_ref(&mut b, s);
            assert_eq!(a, b, "in place, case {case}, s {s}");
            let g: Vec<i32> = (0..n + 3).map(|_| r.i32()).collect();
            let (mut a, mut b) = (g.clone(), g);
            scale_values_saturate_to(&mut a, &src, s);
            scale_values_saturate_to_ref(&mut b, &src, s);
            assert_eq!(a, b, "copy, case {case}, s {s}");
            let g: Vec<i16> = (0..n + 3).map(|_| r.i32() as i16).collect();
            let (mut a, mut b) = (g.clone(), g);
            scale_values_saturate_dbl_to_sgl(&mut a, &src, s);
            scale_values_saturate_dbl_to_sgl_ref(&mut b, &src, s);
            assert_eq!(a, b, "to FIXP_SGL, case {case}, s {s}");
        }
    }
}
