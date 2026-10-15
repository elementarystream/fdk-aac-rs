// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Lane-parallel kernels of libFDK/src/fft.cpp (x86, x86_32 and generic variants on x86 targets).

//! Four independent transforms in the lanes of SSE2 vectors (SSE2 is part of the x86-64 and
//! i686 targets, so no run-time selection is needed).
//!
//! [`V4`] implements [`Lane`] lane by lane with exactly the arithmetic of the scalar `FIXP_DBL`
//! ([`super::W`]) of the variants other than `aarch64`: wrapping additions, subtractions,
//! negations and left shifts, arithmetic right shifts, `fMultDiv2(FIXP_DBL, FIXP_SGL)` as the
//! exact `(a * b) >> 16` (see [`mul_div2`]), `fMult` as that `<< 1` (`((a * (b << 16)) >> 32)
//! << 1` of the x86 build), and `cplxMultDiv2` as the difference and sum of the individually
//! truncated `fMultDiv2` products. The generic kernels of `super` therefore compute in every
//! lane what they compute for one scalar transform. Only used for the variants other than
//! `aarch64` (`!ARCH_ARM`).

use super::{Lane, W};
use crate::common_fix::FixpSgl;
#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;
use core::ops::{Add, AddAssign, Neg, Shl, Shr, Sub, SubAssign};

/// Four `FIXP_DBL` values of four independent transforms.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub(crate) struct V4(pub __m128i);

impl Add for V4 {
    type Output = V4;
    #[inline(always)]
    fn add(self, b: V4) -> V4 {
        // SAFETY: SSE2 is enabled on the x86 and x86-64 targets.
        V4(unsafe { _mm_add_epi32(self.0, b.0) })
    }
}

impl Sub for V4 {
    type Output = V4;
    #[inline(always)]
    fn sub(self, b: V4) -> V4 {
        // SAFETY: SSE2 is enabled on the x86 and x86-64 targets.
        V4(unsafe { _mm_sub_epi32(self.0, b.0) })
    }
}

impl Neg for V4 {
    type Output = V4;
    #[inline(always)]
    fn neg(self) -> V4 {
        // SAFETY: SSE2 is enabled on the x86 and x86-64 targets.
        V4(unsafe { _mm_sub_epi32(_mm_setzero_si128(), self.0) })
    }
}

impl Shr<usize> for V4 {
    type Output = V4;
    /// Arithmetic shift right (the kernels shift by 1 to 3).
    #[inline(always)]
    fn shr(self, n: usize) -> V4 {
        // SAFETY: SSE2 is enabled on the x86 and x86-64 targets.
        V4(unsafe { _mm_sra_epi32(self.0, _mm_cvtsi32_si128(n as i32)) })
    }
}

impl Shl<usize> for V4 {
    type Output = V4;
    /// Shift left, wrapping (the kernels shift by 1 to 2).
    #[inline(always)]
    fn shl(self, n: usize) -> V4 {
        // SAFETY: SSE2 is enabled on the x86 and x86-64 targets.
        V4(unsafe { _mm_sll_epi32(self.0, _mm_cvtsi32_si128(n as i32)) })
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

/// `(a * b) >> 16` per lane (`fMultDiv2(FIXP_DBL, FIXP_SGL)`), exactly, with SSE2.
///
/// With `a = ah * 2^16 + al` (`ah` signed, `al` in `0..2^16`), `(a * b) >> 16 = ah * b +
/// floor(al * b / 2^16)`. `ah * b` is `pmaddwd` of the 16-bit halves of `a` with `(0, b)`. The
/// second term lies in `-32768..=32766`: `pmulhw` gives `floor(al' * b / 2^16)` for the signed
/// reading `al' = al - 2^16` (if bit 15 is set), and adding `b` then corrects it (the 16-bit
/// sum wraps to the exact value); its sign extension is added to the first term.
#[inline(always)]
pub(crate) unsafe fn mul_div2(a: __m128i, b: FixpSgl) -> __m128i {
    let b_lo = _mm_set1_epi32(b as u16 as i32);
    let b_hi = _mm_set1_epi32(((b as u16 as u32) << 16) as i32);
    mul_div2_v(a, b_lo, b_hi)
}

/// [`mul_div2`] with a 16-bit coefficient `b` per lane, given as `b_lo = b & 0xFFFF` and
/// `b_hi = b << 16` (32-bit lanes).
#[inline(always)]
pub(crate) unsafe fn mul_div2_v(a: __m128i, b_lo: __m128i, b_hi: __m128i) -> __m128i {
    let hi = _mm_madd_epi16(a, b_hi);
    let lo = _mm_mulhi_epi16(a, b_lo);
    let corr = _mm_and_si128(_mm_srai_epi16::<15>(a), b_lo);
    let lo = _mm_add_epi16(lo, corr);
    _mm_add_epi32(hi, _mm_srai_epi32::<16>(_mm_slli_epi32::<16>(lo)))
}

impl Lane for V4 {
    // SAFETY: `__m128i` is 16 bytes of 4 `i32` lanes.
    const ZERO: V4 = V4(unsafe { core::mem::transmute::<[i32; 4], __m128i>([0; 4]) });

    #[inline(always)]
    fn fmd2(self, b: FixpSgl) -> V4 {
        // SAFETY: SSE2 is enabled on the x86 and x86-64 targets.
        V4(unsafe { mul_div2(self.0, b) })
    }

    #[inline(always)]
    fn fm(self, b: FixpSgl) -> V4 {
        // SAFETY: SSE2 is enabled on the x86 and x86-64 targets.
        V4(unsafe { _mm_slli_epi32::<1>(mul_div2(self.0, b)) })
    }

    #[inline(always)]
    fn cmd2_s(a_re: V4, a_im: V4, b_re: FixpSgl, b_im: FixpSgl) -> (V4, V4) {
        (a_re.fmd2(b_re) - a_im.fmd2(b_im), a_re.fmd2(b_im) + a_im.fmd2(b_re))
    }
}

/// Loads the 4 interleaved complex values at `p` as (re, im).
#[inline(always)]
unsafe fn load2(p: *const i32) -> (__m128i, __m128i) {
    let lo = _mm_loadu_si128(p as *const __m128i);
    let hi = _mm_loadu_si128(p.add(4) as *const __m128i);
    // (r0 r1 i0 i1), (r2 r3 i2 i3)
    let lo = _mm_shuffle_epi32::<0b11_01_10_00>(lo);
    let hi = _mm_shuffle_epi32::<0b11_01_10_00>(hi);
    (_mm_unpacklo_epi64(lo, hi), _mm_unpackhi_epi64(lo, hi))
}

/// Stores (re, im) as the 4 interleaved complex values at `p`.
#[inline(always)]
unsafe fn store2(p: *mut i32, re: __m128i, im: __m128i) {
    _mm_storeu_si128(p as *mut __m128i, _mm_unpacklo_epi32(re, im));
    _mm_storeu_si128(p.add(4) as *mut __m128i, _mm_unpackhi_epi32(re, im));
}

/// The 4x4 transpose of `(q0, q1, q2, q3)`.
#[inline(always)]
unsafe fn transpose(
    q0: __m128i,
    q1: __m128i,
    q2: __m128i,
    q3: __m128i,
) -> (__m128i, __m128i, __m128i, __m128i) {
    let t0 = _mm_unpacklo_epi32(q0, q1);
    let t1 = _mm_unpacklo_epi32(q2, q3);
    let t2 = _mm_unpackhi_epi32(q0, q1);
    let t3 = _mm_unpackhi_epi32(q2, q3);
    (
        _mm_unpacklo_epi64(t0, t1),
        _mm_unpackhi_epi64(t0, t1),
        _mm_unpacklo_epi64(t2, t3),
        _mm_unpackhi_epi64(t2, t3),
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
            let (re, im) = unsafe { load2(p_input.as_ptr().add(2 * (r + j * dim2)) as *const i32) };
            x[2 * j] = V4(re);
            x[2 * j + 1] = V4(im);
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
                    _mm_storeu_si128(pa.add(2 * ((r + l) * dim1 + j)) as *mut __m128i, q);
                }
            }
            j += 2;
        }
        if j < dim1 {
            let mut re = [0i32; 4];
            let mut im = [0i32; 4];
            // SAFETY: local arrays of 4 values.
            unsafe {
                _mm_storeu_si128(re.as_mut_ptr() as *mut __m128i, x[2 * j].0);
                _mm_storeu_si128(im.as_mut_ptr() as *mut __m128i, x[2 * j + 1].0);
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
    if dim1 >= 8 {
        // SAFETY: `W` is `repr(transparent)` over `i32`.
        let a = unsafe { &mut *(a_dst as *mut [W] as *mut [i32]) };
        rot_vector(a, dim1, length, rot_vector_real, rot_vector_imag);
    } else {
        // rows too short for the vectorized loop
        super::fft_apply_rot_vector(a_dst, dim1, length, rot_vector_real, rot_vector_imag);
    }

    // Phase 2: column c (of dim1) is the transform of a_dst[c + j * dim1], j < dim2, stored to
    // p_input[c + j * dim1].
    let mut c = 0usize;
    while c + 4 <= dim1 {
        let x = &mut xv[..2 * dim2];
        for j in 0..dim2 {
            // SAFETY: the complex values c + j * dim1 .. + 4 are inside a_dst (c + 3 < dim1).
            let (re, im) = unsafe { load2(a_dst.as_ptr().add(2 * (c + j * dim1)) as *const i32) };
            x[2 * j] = V4(re);
            x[2 * j + 1] = V4(im);
        }
        fft2_v(x);
        for j in 0..dim2 {
            // SAFETY: as above, for p_input.
            unsafe {
                let pi = p_input.as_mut_ptr() as *mut i32;
                store2(pi.add(2 * (c + j * dim1)), x[2 * j].0, x[2 * j + 1].0)
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

crate::avx2_clone! {
/// `fft_apply_rot_vector(pData, cl, l, pVecRe, pVecIm)` of the variants other than `aarch64`
/// (see `super::fft_apply_rot_vector`) on slices, which the compiler vectorizes (AVX2 if
/// available): the rows of `cl` complex values after the first are independent, and per value
/// `(im', re') = cplxMultDiv2(im >> 1, re >> 1, vre, vim)` is `(M(im, vre) - M(re, vim),
/// M(im, vim) + M(re, vre))` with the individually truncated products `M(x, w) = (x * w) >> 16`,
/// computed exactly as `(x >> 16) * w + (((x & 0xFFFF) * w) >> 16)`.
fn rot_vector(p: &mut [i32], cl: usize, l: usize, vec_re: &[FixpSgl], vec_im: &[FixpSgl]) {
    #[inline(always)]
    fn m(x: i32, w: FixpSgl) -> i32 {
        let w = w as i32;
        (x >> 16).wrapping_mul(w).wrapping_add(((x & 0xFFFF) * w) >> 16)
    }
    let p = &mut p[..2 * l];
    let cl = cl.min(l);
    for x in &mut p[..2 * cl] {
        *x >>= 2; // * 0.25
    }
    let mut v = 0usize;
    let mut i = cl;
    while i < l {
        p[2 * i] >>= 2; // * 0.25
        p[2 * i + 1] >>= 2;
        let n = (i + cl).min(l) - (i + 1);
        let row = &mut p[2 * (i + 1)..2 * (i + 1 + n)];
        let (vr, vi) = (&vec_re[v..v + n], &vec_im[v..v + n]);
        for ((c, &a), &b) in row.chunks_exact_mut(2).zip(vr).zip(vi) {
            let re = c[0] >> 1;
            let im = c[1] >> 1;
            c[1] = m(im, a).wrapping_sub(m(re, b));
            c[0] = m(im, b).wrapping_add(m(re, a));
        }
        v += n;
        i += cl;
    }
}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::testutil::*;

    /// [`mul_div2`] against the scalar `(a * b) >> 16` on edge and random operands.
    #[test]
    fn mul_div2_is_exact() {
        let mut r = Rng(0x5eed_0aa1);
        let edges = [0, 1, -1, i32::MIN, i32::MAX, 0x7fff, 0x8000, 0xffff, -0x8000, -0x8001];
        let bs = [0i16, 1, -1, i16::MIN, i16::MAX, 0x5a82, -0x5a82];
        for k in 0..200000 {
            let a: [i32; 4] = core::array::from_fn(|i| {
                if k % 3 == 0 {
                    edges[(r.next() % edges.len() as u64) as usize] ^ (i as i32 & (k & 1))
                } else {
                    r.i32() >> r.range(0, 20)
                }
            });
            let b =
                if k % 5 == 0 { bs[(r.next() % bs.len() as u64) as usize] } else { r.i32() as i16 };
            let mut got = [0i32; 4];
            // SAFETY: SSE2 is enabled; local arrays of 4 values.
            unsafe {
                let v = mul_div2(_mm_loadu_si128(a.as_ptr() as *const __m128i), b);
                _mm_storeu_si128(got.as_mut_ptr() as *mut __m128i, v);
            }
            let want = a.map(|a| ((a as i64 * b as i64) >> 16) as i32);
            assert_eq!(got, want, "a {a:?} b {b}");
        }
    }
}
