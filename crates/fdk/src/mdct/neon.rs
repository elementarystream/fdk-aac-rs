// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized loops of libFDK/src/mdct.cpp (imlt_block) and libFDK/src/scale.cpp
// (scaleValuesSaturate).

//! NEON versions of the element-wise loops of `imlt_block`.
//!
//! - [`scale_values_saturate`]: `scaleValueSaturate()` per lane (the same on every variant).
//!   For a scale factor `s` in `1..=31` (the function limits it to -31..=31) the scalar code
//!   saturates to `MAXVAL_DBL` / `MINVAL_DBL + 1` exactly when `value << s` overflows (the
//!   headroom test), otherwise it computes `max(value << s, MINVAL_DBL + 1)`: that is
//!   `max(sqshl(value, s), MINVAL_DBL + 1)`. For `s` in `-31..=-1` it computes
//!   `value >> -s`, except that the values whose significant bits are all shifted out give 0
//!   also when negative (`value >> -s == -1` becomes 0); the `max` with `MINVAL_DBL + 1` has no
//!   effect after a right shift.
//! - [`window_crossing`]: the window crossing loop with the `aarch64` `cplxMultDiv2` (the 64-bit
//!   sum of products narrowed by 16, see `crate::dct::neon`) and `SATURATE_LEFT_SHIFT_ALT(x, 1,
//!   32)`, which is `max(sqshl(x, 1), -MAXVAL_DBL)`: `x > 0x3FFFFFFF` saturates to `MAXVAL_DBL`
//!   in both, `x <= -0x40000000` gives `-MAXVAL_DBL` in both, other values are doubled.

use crate::archdef::FixpWtp;
use crate::dct::neon::{madd16, msub16, rev};
use core::arch::aarch64::*;

/// `scaleValuesSaturate(vector, len, scalefactor)` (see [`crate::scale::scale_values_saturate`]).
pub(crate) fn scale_values_saturate(vector: &mut [i32], scalefactor: i32) {
    if scalefactor == 0 {
        return;
    }
    let s = scalefactor.clamp(-31, 31);
    let mut chunks = vector.chunks_exact_mut(4);
    // SAFETY: every chunk holds 4 values.
    unsafe {
        let min1 = vdupq_n_s32(i32::MIN + 1);
        let sh = vdupq_n_s32(s);
        if s > 0 {
            for c in &mut chunks {
                let v = vld1q_s32(c.as_ptr());
                vst1q_s32(c.as_mut_ptr(), vmaxq_s32(vqshlq_s32(v, sh), min1));
            }
        } else {
            for c in &mut chunks {
                let t = vshlq_s32(vld1q_s32(c.as_ptr()), sh);
                // t == -1: the mask is -1, t - (-1) = 0
                let m = vreinterpretq_s32_u32(vceqq_s32(t, vdupq_n_s32(-1)));
                vst1q_s32(c.as_mut_ptr(), vsubq_s32(t, m));
            }
        }
    }
    for v in chunks.into_remainder() {
        *v = crate::scale::scale_value_saturate(*v, s);
    }
}

/// `scaleValuesSaturate(dst, src, len, scalefactor)` (see
/// [`crate::scale::scale_values_saturate_to`]): `dst[..src.len()]` from `src`.
pub(crate) fn scale_values_saturate_to(dst: &mut [i32], src: &[i32], scalefactor: i32) {
    let dst = &mut dst[..src.len()];
    if scalefactor == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let s = scalefactor.clamp(-31, 31);
    let mut d = dst.chunks_exact_mut(4);
    let mut x = src.chunks_exact(4);
    // SAFETY: every chunk holds 4 values.
    unsafe {
        let min1 = vdupq_n_s32(i32::MIN + 1);
        let sh = vdupq_n_s32(s);
        if s > 0 {
            for (d, x) in (&mut d).zip(&mut x) {
                let v = vld1q_s32(x.as_ptr());
                vst1q_s32(d.as_mut_ptr(), vmaxq_s32(vqshlq_s32(v, sh), min1));
            }
        } else {
            for (d, x) in (&mut d).zip(&mut x) {
                let t = vshlq_s32(vld1q_s32(x.as_ptr()), sh);
                let m = vreinterpretq_s32_u32(vceqq_s32(t, vdupq_n_s32(-1)));
                vst1q_s32(d.as_mut_ptr(), vsubq_s32(t, m));
            }
        }
    }
    for (d, &x) in d.into_remainder().iter_mut().zip(x.remainder()) {
        *d = crate::scale::scale_value_saturate(x, s);
    }
}

/// `SATURATE_LEFT_SHIFT_ALT(x, 1, DFRACT_BITS)` per lane.
#[inline(always)]
unsafe fn lsh1_alt(x: int32x4_t) -> int32x4_t {
    vmaxq_s32(vqshlq_n_s32::<1>(x), vdupq_n_s32(-i32::MAX))
}

/// The first `n / 4 * 4` iterations `i` of the window crossing loop of `imlt_block` with the
/// `aarch64` arithmetic: `(x1, x0) = cplxMultDiv2(curr[i], o, w[i])` with `o = -ovl[n-1-i]`
/// (`NEG_OVL`) or `ovl[n-1-i]`, `out0[i] = SATURATE_LEFT_SHIFT_ALT(x0, 1)`,
/// `out1[n-1-i] = SATURATE_LEFT_SHIFT_ALT(-x1, 1)` (`NEG_X1`) or `(x1, 1)`. Returns the number of
/// iterations done.
///
/// # Safety
///
/// `curr`, `ovl`, `out0`, `out1` and `w` must be valid for `n` values; the output ranges must not
/// overlap each other or the input ranges.
pub(crate) unsafe fn window_crossing<const NEG_OVL: bool, const NEG_X1: bool>(
    curr: *const i32,
    ovl: *const i32,
    out0: *mut i32,
    out1: *mut i32,
    w: *const FixpWtp,
    n: usize,
) -> usize {
    let mut i = 0;
    while i + 4 <= n {
        let c = vld1q_s32(curr.add(i));
        let mut o = rev(vld1q_s32(ovl.add(n - 4 - i)));
        if NEG_OVL {
            o = vnegq_s32(o);
        }
        let t = vld2_s16(w.add(i) as *const i16);
        let (wr, wi) = (vmovl_s16(t.0), vmovl_s16(t.1));
        let x1 = msub16(c, wr, o, wi);
        let x0 = madd16(c, wi, o, wr);
        vst1q_s32(out0.add(i), lsh1_alt(x0));
        let x1 = if NEG_X1 { vnegq_s32(x1) } else { x1 };
        vst1q_s32(out1.add(n - 4 - i), rev(lsh1_alt(x1)));
        i += 4;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The NEON scaling against `scaleValueSaturate` for every scale factor (also beyond the
    /// limits) on edge values and pseudo-random values of every magnitude.
    #[test]
    fn scale_values_saturate_equals_scalar() {
        let mut vals: Vec<i32> =
            vec![0, 1, -1, 2, -2, i32::MAX, i32::MIN, i32::MIN + 1, i32::MAX - 1];
        for b in 0..31 {
            for d in [-1i32, 0, 1] {
                vals.push((1i32 << b).wrapping_add(d));
                vals.push((-(1i32 << b)).wrapping_add(d));
            }
        }
        let mut x = 0x1234_5678_9abc_def1u64;
        for _ in 0..4000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            vals.push((x as i32) >> (x >> 59));
        }
        // a length that is not a multiple of 4
        vals.push(12345);
        for sf in -40..=40 {
            let mut v = vals.clone();
            scale_values_saturate(&mut v, sf);
            let mut w = vals.clone();
            crate::scale::scale_values_saturate(&mut w, sf);
            assert_eq!(v, w, "scalefactor {sf}");
            for len in [0, 1, 3, 4, 7, vals.len()] {
                let mut d = vec![7; vals.len()];
                scale_values_saturate_to(&mut d, &vals[..len], sf);
                let mut e = vec![7; vals.len()];
                crate::scale::scale_values_saturate_to(&mut e, &vals[..len], sf);
                assert_eq!(d, e, "scalefactor {sf} length {len}");
            }
        }
    }

    /// `lsh1_alt` against `SATURATE_LEFT_SHIFT_ALT(x, 1, 32)` around its thresholds.
    #[test]
    fn lsh1_alt_equals_scalar() {
        let mut vals = vec![0, 1, -1, i32::MAX, i32::MIN, i32::MIN + 1];
        for c in [0x3FFF_FFFFi32, -0x4000_0000] {
            for d in -3..=3 {
                vals.push(c + d);
            }
        }
        vals.resize(vals.len().div_ceil(4) * 4, 7);
        for c in vals.chunks_exact(4) {
            let mut out = [0i32; 4];
            // SAFETY: 4 values each.
            unsafe { vst1q_s32(out.as_mut_ptr(), lsh1_alt(vld1q_s32(c.as_ptr()))) };
            for (o, &v) in out.iter().zip(c) {
                assert_eq!(*o, crate::scale::saturate_left_shift_alt(v, 1, 32), "{v:#x}");
            }
        }
    }
}
