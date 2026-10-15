// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized version of multE2_DinvF_fdk() of libAACdec/src/ldfiltbank.cpp.

//! `multE2_DinvF_fdk()` vectorized (an optimization of the port; the straight port is
//! [`super::mult_e2_dinv_f_fdk_straight`]).
//!
//! Iteration `i` of the first two loops reads `x[N/2 + i]`, `x[N/2 - 1 - i]`, `z[i]`,
//! `z[N/2 + i]`, `z[N + i]` and writes `z[i]`, `z[N/2 + i]`, `z[N + i]` and its outputs: the
//! iterations are independent (the third loop reads the `z[i]` that the first one wrote). They
//! run 4 at a time with NEON (AArch64, every variant), 8 at a time with AVX2 (x86, selected at
//! run time), otherwise as a loop over slices. The operations are the same on every variant:
//! `fMultDiv2(FIXP_DBL, FIXP_SGL)` is `(a * b) >> 16`, `SATURATE_LEFT_SHIFT(x, 0, 32)` (the
//! `WTS1` outputs) is `x`, and `SATURATE_LEFT_SHIFT(x, 1, 32)` (the `WTS0` outputs) is the
//! saturating doubling in both the `arm/scale_arm.h` and the generic form.
//!
//! The buffers passed have exactly the lengths the C code accesses (`N` values of `output` and
//! `x`, `3 N` of `fb`, `3 N / 2` of `z`), and `N` is a multiple of 4.

use super::*;

/// The output shift of the `WTS0` outputs (`SATURATE_LEFT_SHIFT(x, WTS0 + 1 - scale, 32)`).
const SH0: i32 = WTS0 + 1 - ((DFRACT_BITS - PCM_OUT_BITS) - LDFB_HEADROOM + 3);
/// The output shift of the `WTS1` outputs.
const SH1: i32 = WTS1 + 1 - ((DFRACT_BITS - PCM_OUT_BITS) - LDFB_HEADROOM + 3);
/// The right shift of the `WTS2` products.
const SH2: i32 = -WTS2 - 1;
const _: () = assert!(SH0 == 1 && SH1 == 0 && SH2 == 1 && PCM_OUT_BITS == DFRACT_BITS);

/// See the module documentation (`n` a multiple of 4, the buffers exactly as long as the C code
/// accesses).
pub(super) fn mult_e2(
    output: &mut [PcmDec],
    x: &[FixpDbl],
    fb: &[FixpWtb],
    z: &mut [FixpDbl],
    n: usize,
) {
    assert!(
        n % 4 == 0
            && output.len() == n
            && x.len() == n
            && fb.len() == 3 * n
            && z.len() == n * 3 / 2
    );
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is part of AArch64; the lengths are checked above.
        unsafe { neon::mult_e2(output, x, fb, z, n) };
        return;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if fdk_aac_rs_fdk::simd_x86::avx2() {
        // SAFETY: the CPU has AVX2; the lengths are checked above.
        unsafe { x86::mult_e2(output, x, fb, z, n) };
        return;
    }
    #[allow(unreachable_code)]
    scalar(output, x, fb, z, n, 0);
}

/// `fMultDiv2(FIXP_DBL, FIXP_SGL)`
#[inline(always)]
fn md2(a: FixpDbl, b: FixpWtb) -> FixpDbl {
    f_mult_div2(a, b)
}

/// The iterations `i0..N/4` and `N/4 + i0..N/2` of the first two loops and `i0..N/4` of the third
/// (all of them for `i0 = 0`; the remainder of the vector forms otherwise), on slices.
fn scalar(
    output: &mut [PcmDec],
    x: &[FixpDbl],
    fb: &[FixpWtb],
    z: &mut [FixpDbl],
    n: usize,
    i0: usize,
) {
    let (h, q) = (n / 2, n / 4);
    let (z0s, rest) = z.split_at_mut(h);
    let (z1s, z2s) = rest.split_at_mut(h);
    let (x_lo, x_hi) = x.split_at(h);
    let (fb0, rest) = fb.split_at(n);
    let (fb1, fb2) = rest.split_at(n);
    let (out_a, out_b) = output.split_at_mut(3 * q);
    for i in (i0..q).chain(q + i0..h) {
        let z2 = x_hi[i];
        let z0 = f_add_saturate(z2, md2(z1s[i], fb2[i]) >> SH2);
        let zn = f_add_saturate(x_lo[h - 1 - i], md2(z2s[i], fb2[h + i]) >> SH2);
        z1s[i] = zn;
        let tmp1 = md2(zn, fb1[h - 1 - i]).wrapping_add(md2(z0s[i], fb1[h + i]));
        out_a[3 * q - 1 - i] = saturate_left_shift(tmp1, SH1, PCM_OUT_BITS);
        if i >= q {
            let tmp0 = md2(zn, fb0[h - 1 - i]).wrapping_add(md2(z0s[i], fb0[h + i]));
            out_a[i - q] = saturate_left_shift(tmp0, SH0, PCM_OUT_BITS);
        }
        z0s[i] = z0;
        z2s[i] = z2;
    }
    for i in i0..q {
        out_b[i] = saturate_left_shift(md2(z0s[i], fb0[h + i]), SH0, PCM_OUT_BITS);
    }
}

/// NEON form: `fMultDiv2(a, b)` is `sqdmulh(a, b << 15)` (`b << 15` is never `INT_MIN`, so it
/// cannot saturate), `fAddSaturate(a, b)` is `min(max((a >> 1) + (b >> 1), MINVAL_DBL >> 1),
/// MAXVAL_DBL >> 1) << 1`, the saturating doubling is `sqshl(x, 1)`. Reversed index ranges are
/// loaded or stored as vectors and reversed.
#[cfg(target_arch = "aarch64")]
mod neon {
    use super::*;
    use core::arch::aarch64::*;
    use fdk_aac_rs_fdk::common_fix::{MAXVAL_DBL, MINVAL_DBL};

    #[inline(always)]
    unsafe fn rev(v: int32x4_t) -> int32x4_t {
        let r = vrev64q_s32(v);
        vextq_s32::<2>(r, r)
    }

    /// `p[0..4] << 15` in 32 bit lanes.
    #[inline(always)]
    unsafe fn w(p: *const i16) -> int32x4_t {
        vshll_n_s16::<15>(vld1_s16(p))
    }

    /// `p[3], p[2], p[1], p[0]`, each `<< 15`.
    #[inline(always)]
    unsafe fn wr(p: *const i16) -> int32x4_t {
        vshll_n_s16::<15>(vrev64_s16(vld1_s16(p)))
    }

    #[inline(always)]
    unsafe fn add_sat(a: int32x4_t, b: int32x4_t) -> int32x4_t {
        let s = vaddq_s32(vshrq_n_s32::<1>(a), vshrq_n_s32::<1>(b));
        let s = vminq_s32(vmaxq_s32(s, vdupq_n_s32(MINVAL_DBL >> 1)), vdupq_n_s32(MAXVAL_DBL >> 1));
        vshlq_n_s32::<1>(s)
    }

    /// # Safety
    /// The buffers have the lengths of [`super::mult_e2`].
    pub(super) unsafe fn mult_e2(
        output: &mut [PcmDec],
        x: &[FixpDbl],
        fb: &[FixpWtb],
        z: &mut [FixpDbl],
        n: usize,
    ) {
        let (h, q) = (n / 2, n / 4);
        let q4 = q & !3;
        {
            let (xp, fp, zp, op) = (x.as_ptr(), fb.as_ptr(), z.as_mut_ptr(), output.as_mut_ptr());
            // blocks i..i + 4 inside [0, q) and [q, h)
            for i in (0..q4).step_by(4).chain((q..q + q4).step_by(4)) {
                let z2 = vld1q_s32(xp.add(h + i));
                let p = vqdmulhq_s32(vld1q_s32(zp.add(h + i)), w(fp.add(2 * n + i)));
                let z0 = add_sat(z2, vshrq_n_s32::<1>(p));
                let xl = rev(vld1q_s32(xp.add(h - 4 - i)));
                let p = vqdmulhq_s32(vld1q_s32(zp.add(n + i)), w(fp.add(2 * n + h + i)));
                let zn = add_sat(xl, vshrq_n_s32::<1>(p));
                vst1q_s32(zp.add(h + i), zn);
                let zi = vld1q_s32(zp.add(i));
                let tmp1 = vaddq_s32(
                    vqdmulhq_s32(zn, wr(fp.add(n + h - 4 - i))),
                    vqdmulhq_s32(zi, w(fp.add(n + h + i))),
                );
                vst1q_s32(op.add(3 * q - 4 - i), rev(tmp1));
                if i >= q {
                    let tmp0 = vaddq_s32(
                        vqdmulhq_s32(zn, wr(fp.add(h - 4 - i))),
                        vqdmulhq_s32(zi, w(fp.add(h + i))),
                    );
                    vst1q_s32(op.add(i - q), vqshlq_n_s32::<1>(tmp0));
                }
                vst1q_s32(zp.add(i), z0);
                vst1q_s32(zp.add(n + i), z2);
            }
        }
        // the remaining iterations of the first two loops and of the third
        if q4 != q {
            scalar(output, x, fb, z, n, q4);
        }
        let (fp, zp, op) = (fb.as_ptr(), z.as_ptr(), output.as_mut_ptr());
        for i in (0..q4).step_by(4) {
            let tmp0 = vqdmulhq_s32(vld1q_s32(zp.add(i)), w(fp.add(h + i)));
            vst1q_s32(op.add(3 * q + i), vqshlq_n_s32::<1>(tmp0));
        }
    }
}

/// AVX2 form (8 iterations at a time): `fMultDiv2(a, b)` for 16 bit `b` is computed exactly in
/// 32 bit lanes as `(a >> 16) * b + (((a & 0xFFFF) * b) >> 16)` (no intermediate value exceeds
/// 31 bits), `fAddSaturate` with `min`/`max`, the saturating doubling with two comparisons.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86 {
    use super::*;
    #[cfg(target_arch = "x86")]
    use core::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;
    use fdk_aac_rs_fdk::common_fix::{MAXVAL_DBL, MINVAL_DBL};

    #[inline(always)]
    unsafe fn md2v(a: __m256i, b: __m256i) -> __m256i {
        let hi = _mm256_mullo_epi32(_mm256_srai_epi32::<16>(a), b);
        let lo = _mm256_and_si256(a, _mm256_set1_epi32(0xFFFF));
        _mm256_add_epi32(hi, _mm256_srai_epi32::<16>(_mm256_mullo_epi32(lo, b)))
    }

    #[inline(always)]
    unsafe fn rev(v: __m256i) -> __m256i {
        _mm256_permutevar8x32_epi32(v, _mm256_setr_epi32(7, 6, 5, 4, 3, 2, 1, 0))
    }

    #[inline(always)]
    unsafe fn ld(p: *const i32) -> __m256i {
        _mm256_loadu_si256(p as *const __m256i)
    }

    #[inline(always)]
    unsafe fn st(p: *mut i32, v: __m256i) {
        _mm256_storeu_si256(p as *mut __m256i, v)
    }

    /// `p[0..8]` sign-extended to 32 bit lanes.
    #[inline(always)]
    unsafe fn w(p: *const i16) -> __m256i {
        _mm256_cvtepi16_epi32(_mm_loadu_si128(p as *const __m128i))
    }

    #[inline(always)]
    unsafe fn add_sat(a: __m256i, b: __m256i) -> __m256i {
        let s = _mm256_add_epi32(_mm256_srai_epi32::<1>(a), _mm256_srai_epi32::<1>(b));
        let s = _mm256_min_epi32(
            _mm256_max_epi32(s, _mm256_set1_epi32(MINVAL_DBL >> 1)),
            _mm256_set1_epi32(MAXVAL_DBL >> 1),
        );
        _mm256_slli_epi32::<1>(s)
    }

    /// `SATURATE_LEFT_SHIFT(v, 1, 32)`: `MAXVAL_DBL` above `MAXVAL_DBL >> 1`, `MINVAL_DBL` below
    /// `!(MAXVAL_DBL >> 1)`, otherwise `v << 1`.
    #[inline(always)]
    unsafe fn sat_dbl(v: __m256i) -> __m256i {
        let over = _mm256_cmpgt_epi32(v, _mm256_set1_epi32(MAXVAL_DBL >> 1));
        let under = _mm256_cmpgt_epi32(_mm256_set1_epi32(!(MAXVAL_DBL >> 1)), v);
        let r = _mm256_blendv_epi8(_mm256_slli_epi32::<1>(v), _mm256_set1_epi32(MAXVAL_DBL), over);
        _mm256_blendv_epi8(r, _mm256_set1_epi32(MINVAL_DBL), under)
    }

    /// # Safety
    /// AVX2 must be available; the buffers have the lengths of [`super::mult_e2`].
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn mult_e2(
        output: &mut [PcmDec],
        x: &[FixpDbl],
        fb: &[FixpWtb],
        z: &mut [FixpDbl],
        n: usize,
    ) {
        let (h, q) = (n / 2, n / 4);
        let q8 = q & !7;
        {
            let (xp, fp, zp, op) = (x.as_ptr(), fb.as_ptr(), z.as_mut_ptr(), output.as_mut_ptr());
            for i in (0..q8).step_by(8).chain((q..q + q8).step_by(8)) {
                let z2 = ld(xp.add(h + i));
                let p = md2v(ld(zp.add(h + i)), w(fp.add(2 * n + i)));
                let z0 = add_sat(z2, _mm256_srai_epi32::<1>(p));
                let xl = rev(ld(xp.add(h - 8 - i)));
                let p = md2v(ld(zp.add(n + i)), w(fp.add(2 * n + h + i)));
                let zn = add_sat(xl, _mm256_srai_epi32::<1>(p));
                st(zp.add(h + i), zn);
                let zi = ld(zp.add(i));
                let tmp1 = _mm256_add_epi32(
                    md2v(zn, rev(w(fp.add(n + h - 8 - i)))),
                    md2v(zi, w(fp.add(n + h + i))),
                );
                st(op.add(3 * q - 8 - i), rev(tmp1));
                if i >= q {
                    let tmp0 = _mm256_add_epi32(
                        md2v(zn, rev(w(fp.add(h - 8 - i)))),
                        md2v(zi, w(fp.add(h + i))),
                    );
                    st(op.add(i - q), sat_dbl(tmp0));
                }
                st(zp.add(i), z0);
                st(zp.add(n + i), z2);
            }
        }
        if q8 != q {
            scalar(output, x, fb, z, n, q8);
        }
        let (fp, zp, op) = (fb.as_ptr(), z.as_ptr(), output.as_mut_ptr());
        for i in (0..q8).step_by(8) {
            st(op.add(3 * q + i), sat_dbl(md2v(ld(zp.add(i)), w(fp.add(h + i)))));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
        fn val(&mut self) -> i32 {
            match self.next() % 8 {
                0 => [
                    i32::MIN,
                    i32::MIN + 1,
                    i32::MAX,
                    0,
                    -1,
                    0x3FFF_FFFF,
                    0x4000_0000,
                    -0x4000_0000,
                ][(self.next() % 8) as usize],
                1 => self.next() as i32,
                n => (self.next() as i32) >> (n * 3),
            }
        }
    }

    /// The vector and slice forms against the straight port, with random windows (any 16 bit
    /// values) and extreme data, for every length of the library and other multiples of 4.
    #[test]
    fn mult_e2_matches_straight() {
        let mut r = Rng(0x0123_4567_89ab_cdef);
        for iter in 0..4000 {
            let n = [512, 480, 256, 240, 160, 128, 120, 4, 8, 12, 20, 36][iter % 12];
            let x: Vec<i32> = (0..n).map(|_| r.val()).collect();
            let fb: Vec<i16> = (0..3 * n).map(|_| r.next() as i16).collect();
            let z: Vec<i32> = (0..n * 3 / 2).map(|_| r.val()).collect();
            let (mut oa, mut ob, mut oc) = (vec![0; n], vec![0; n], vec![0; n]);
            let (mut za, mut zb, mut zc) = (z.clone(), z.clone(), z);
            mult_e2(&mut oa, &x, &fb, &mut za, n);
            mult_e2_dinv_f_fdk_straight(&mut ob, &x, &fb, &mut zb, n as i32);
            scalar(&mut oc, &x, &fb, &mut zc, n, 0);
            assert_eq!((&oa, &za), (&ob, &zb), "n {n} iter {iter}");
            assert_eq!((&oc, &zc), (&ob, &zb), "scalar n {n} iter {iter}");
        }
    }
}
