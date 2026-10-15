// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Lane-parallel kernels of libFDK/src/fft.cpp.

//! Four independent transforms in the lanes of NEON vectors.
//!
//! [`V4`] implements [`Lane`] lane by lane with exactly the arithmetic of the scalar
//! `FIXP_DBL` ([`super::W`]): wrapping additions, subtractions, negations and left shifts,
//! arithmetic right shifts, `fMultDiv2(FIXP_DBL, FIXP_SGL)` as `vqdmulh(a, b << 15)` (the exact
//! `(a * b) >> 16`, which cannot saturate), `fMult` as that `<< 1`, and `cplxMultDiv2` of the
//! selected variant: `aarch64` sums the two 64-bit products and keeps bits 16..47 of the sum
//! with the coefficients (bits 32..63 with the coefficients `<< 16`, see
//! [`crate::cplx_mul`]), the other variants subtract or add the individually truncated
//! `fMultDiv2` products. The generic kernels of `super` therefore compute in every lane what
//! they compute for one scalar transform.

use super::{Lane, W};
use crate::common_fix::{FixpSgl, ARCH_ARM};
use core::arch::aarch64::*;
use core::ops::{Add, AddAssign, Neg, Shl, Shr, Sub, SubAssign};

/// Four `FIXP_DBL` values of four independent transforms.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub(crate) struct V4(pub int32x4_t);

impl Add for V4 {
    type Output = V4;
    #[inline(always)]
    fn add(self, b: V4) -> V4 {
        // SAFETY: NEON is available on aarch64.
        V4(unsafe { vaddq_s32(self.0, b.0) })
    }
}

impl Sub for V4 {
    type Output = V4;
    #[inline(always)]
    fn sub(self, b: V4) -> V4 {
        // SAFETY: NEON is available on aarch64.
        V4(unsafe { vsubq_s32(self.0, b.0) })
    }
}

impl Neg for V4 {
    type Output = V4;
    #[inline(always)]
    fn neg(self) -> V4 {
        // SAFETY: NEON is available on aarch64.
        V4(unsafe { vnegq_s32(self.0) })
    }
}

impl Shr<usize> for V4 {
    type Output = V4;
    /// Arithmetic shift right (the kernels shift by 1 to 3).
    #[inline(always)]
    fn shr(self, n: usize) -> V4 {
        // SAFETY: NEON is available on aarch64.
        V4(unsafe { vshlq_s32(self.0, vdupq_n_s32(-(n as i32))) })
    }
}

impl Shl<usize> for V4 {
    type Output = V4;
    /// Shift left, wrapping (the kernels shift by 1 to 2).
    #[inline(always)]
    fn shl(self, n: usize) -> V4 {
        // SAFETY: NEON is available on aarch64.
        V4(unsafe { vshlq_s32(self.0, vdupq_n_s32(n as i32)) })
    }
}

impl AddAssign for V4 {
    #[inline(always)]
    fn add_assign(&mut self, b: V4) {
        *self = *self + b;
    }
}

impl SubAssign for V4 {
    #[inline(always)]
    fn sub_assign(&mut self, b: V4) {
        *self = *self - b;
    }
}

/// `(a * p + b * q) >> 16` (`SUB`: `a * p - b * q`) per lane, in 64 bits, truncated to 32 bits.
#[inline(always)]
unsafe fn mac16<const SUB: bool>(
    a: int32x4_t,
    p: int32x4_t,
    b: int32x4_t,
    q: int32x4_t,
) -> int32x4_t {
    let lo = vmull_s32(vget_low_s32(a), vget_low_s32(p));
    let hi = vmull_high_s32(a, p);
    let (lo, hi) = if SUB {
        (vmlsl_s32(lo, vget_low_s32(b), vget_low_s32(q)), vmlsl_high_s32(hi, b, q))
    } else {
        (vmlal_s32(lo, vget_low_s32(b), vget_low_s32(q)), vmlal_high_s32(hi, b, q))
    };
    vshrn_high_n_s64::<16>(vshrn_n_s64::<16>(lo), hi)
}

impl Lane for V4 {
    // SAFETY: `int32x4_t` is 16 bytes of 4 `i32` lanes.
    const ZERO: V4 = V4(unsafe { core::mem::transmute::<[i32; 4], int32x4_t>([0; 4]) });

    #[inline(always)]
    fn fmd2(self, b: FixpSgl) -> V4 {
        // SAFETY: NEON is available on aarch64.
        V4(unsafe { vqdmulhq_s32(self.0, vdupq_n_s32((b as i32) << 15)) })
    }

    #[inline(always)]
    fn fm(self, b: FixpSgl) -> V4 {
        // SAFETY: NEON is available on aarch64.
        V4(unsafe { vshlq_n_s32::<1>(self.fmd2(b).0) })
    }

    #[inline(always)]
    fn cmd2_s(a_re: V4, a_im: V4, b_re: FixpSgl, b_im: FixpSgl) -> (V4, V4) {
        if ARCH_ARM {
            // SAFETY: NEON is available on aarch64.
            unsafe {
                let (c, s) = (vdupq_n_s32(b_re as i32), vdupq_n_s32(b_im as i32));
                (V4(mac16::<true>(a_re.0, c, a_im.0, s)), V4(mac16::<false>(a_re.0, s, a_im.0, c)))
            }
        } else {
            (a_re.fmd2(b_re) - a_im.fmd2(b_im), a_re.fmd2(b_im) + a_im.fmd2(b_re))
        }
    }
}

/// The 4x4 transpose of `(q0, q1, q2, q3)`.
#[inline(always)]
unsafe fn transpose(
    q0: int32x4_t,
    q1: int32x4_t,
    q2: int32x4_t,
    q3: int32x4_t,
) -> (int32x4_t, int32x4_t, int32x4_t, int32x4_t) {
    let t0 = vtrn1q_s32(q0, q1);
    let t1 = vtrn2q_s32(q0, q1);
    let t2 = vtrn1q_s32(q2, q3);
    let t3 = vtrn2q_s32(q2, q3);
    let f = |a: int32x4_t| vreinterpretq_s64_s32(a);
    let g = |a: int64x2_t| vreinterpretq_s32_s64(a);
    (
        g(vtrn1q_s64(f(t0), f(t2))),
        g(vtrn1q_s64(f(t1), f(t3))),
        g(vtrn2q_s64(f(t0), f(t2))),
        g(vtrn2q_s64(f(t1), f(t3))),
    )
}

/// `fftN2` (see `super::fftN2`) with the `dim2` transforms of length `dim1` and the `dim1`
/// transforms of length `dim2` computed four at a time ([`V4`]); the rest of them, and the
/// rotation, as in the straight port.
///
/// The transforms of a phase are independent of each other (the first phase reads `p_input`
/// and writes rows of `a_dst`, the second reads columns of `a_dst` and writes `p_input`), so
/// the order in which they are computed does not matter.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(super) fn fft_n2_x4(
    p_input: &mut [W],
    length: usize,
    dim1: usize,
    dim2: usize,
    fft1_v: fn(&mut [V4]),
    fft2_v: fn(&mut [V4]),
    fft1: fn(&mut [W]),
    fft2: fn(&mut [W]),
    rot_vector_real: &[FixpSgl],
    rot_vector_imag: &[FixpSgl],
) {
    assert!(length == dim1 * dim2 && length <= 480 && dim1 <= 32 && dim2 <= 32);
    let p_input = &mut p_input[..2 * length];
    let mut a_dst_buf = [W::ZERO; 2 * 480];
    let a_dst = &mut a_dst_buf[..2 * length];
    let mut xv = [V4::ZERO; 2 * 32];

    // Phase 1: row r (of dim2) is the transform of p_input[r + j * dim2], j < dim1, stored to
    // a_dst[r * dim1 + j].
    let mut r = 0usize;
    while r + 4 <= dim2 {
        let x = &mut xv[..2 * dim1];
        for j in 0..dim1 {
            // SAFETY: the complex values r + j * dim2 .. + 4 are inside p_input (r + 3 < dim2).
            let v = unsafe { vld2q_s32(p_input.as_ptr().add(2 * (r + j * dim2)) as *const i32) };
            x[2 * j] = V4(v.0);
            x[2 * j + 1] = V4(v.1);
        }
        fft1_v(x);
        let mut j = 0usize;
        while j + 2 <= dim1 {
            // SAFETY: the rows r .. r + 4 hold the complex values j, j + 1 (j + 1 < dim1).
            unsafe {
                let pa = a_dst.as_mut_ptr() as *mut i32;
                let (q0, q1, q2, q3) =
                    transpose(x[2 * j].0, x[2 * j + 1].0, x[2 * j + 2].0, x[2 * j + 3].0);
                for (l, q) in [q0, q1, q2, q3].into_iter().enumerate() {
                    vst1q_s32(pa.add(2 * ((r + l) * dim1 + j)), q);
                }
            }
            j += 2;
        }
        if j < dim1 {
            let mut re = [0i32; 4];
            let mut im = [0i32; 4];
            // SAFETY: local arrays of 4 values.
            unsafe {
                vst1q_s32(re.as_mut_ptr(), x[2 * j].0);
                vst1q_s32(im.as_mut_ptr(), x[2 * j + 1].0);
            }
            for l in 0..4 {
                a_dst[2 * ((r + l) * dim1 + j)] = core::num::Wrapping(re[l]);
                a_dst[2 * ((r + l) * dim1 + j) + 1] = core::num::Wrapping(im[l]);
            }
        }
        r += 4;
    }
    while r < dim2 {
        let dst = &mut a_dst[2 * r * dim1..2 * (r + 1) * dim1];
        for j in 0..dim1 {
            dst[2 * j] = p_input[2 * (r + j * dim2)];
            dst[2 * j + 1] = p_input[2 * (r + j * dim2) + 1];
        }
        fft1(dst);
        r += 1;
    }

    // Perform the modulation of the output of the fft of length dim1
    super::fft_apply_rot_vector(a_dst, dim1, length, rot_vector_real, rot_vector_imag);

    // Phase 2: column c (of dim1) is the transform of a_dst[c + j * dim1], j < dim2, stored to
    // p_input[c + j * dim1].
    let mut c = 0usize;
    while c + 4 <= dim1 {
        let x = &mut xv[..2 * dim2];
        for j in 0..dim2 {
            // SAFETY: the complex values c + j * dim1 .. + 4 are inside a_dst (c + 3 < dim1).
            let v = unsafe { vld2q_s32(a_dst.as_ptr().add(2 * (c + j * dim1)) as *const i32) };
            x[2 * j] = V4(v.0);
            x[2 * j + 1] = V4(v.1);
        }
        fft2_v(x);
        for j in 0..dim2 {
            // SAFETY: as above, for p_input.
            unsafe {
                let pi = p_input.as_mut_ptr() as *mut i32;
                vst2q_s32(pi.add(2 * (c + j * dim1)), int32x4x2_t(x[2 * j].0, x[2 * j + 1].0))
            };
        }
        c += 4;
    }
    let mut a_dst2 = [W::ZERO; 2 * 32];
    while c < dim1 {
        let a_dst2 = &mut a_dst2[..2 * dim2];
        for j in 0..dim2 {
            a_dst2[2 * j] = a_dst[2 * (c + j * dim1)];
            a_dst2[2 * j + 1] = a_dst[2 * (c + j * dim1) + 1];
        }
        fft2(a_dst2);
        for j in 0..dim2 {
            p_input[2 * (c + j * dim1)] = a_dst2[2 * j];
            p_input[2 * (c + j * dim1) + 1] = a_dst2[2 * j + 1];
        }
        c += 1;
    }
}
