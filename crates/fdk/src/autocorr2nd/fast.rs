// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized loops of autoCorr2nd_real() and autoCorr2nd_cplx() of libFDK/src/autocorr2nd.cpp.

//! The loops of `autoCorr2nd_real()` and `autoCorr2nd_cplx()` in SIMD (an optimization of the
//! port; the straight ports are `super::real_loop_ref` and `super::cplx_loop_ref`).
//!
//! Every term of the loops is a sum of one or two products `fMultDiv2(a, b) = (a * b) >> 32`
//! (the same on every variant), shifted right by `len_scale`, and the terms are added to the
//! accumulators with wrap-around, so the terms can be added in any order. The terms are computed
//! exactly as in the scalar code, several at a time: the products as the high halves of 64-bit
//! products (NEON `smull`, AVX2 `vpmuldq`), the sums of a term with 32-bit wrapping additions
//! (`autoCorr2nd_real()` adds the products of two neighbouring values `p`, `p + 1` before the
//! shift: pairwise additions of neighbouring lanes), the shifts per lane. The terms that do not
//! fill a vector take the scalar code. Loop counts that the straight loop would not end (`len`
//! below 2 or 1) and buffers shorter than the loop reads take the straight loop.

use super::*;

/// The loop of `autoCorr2nd_real()` (see the module documentation).
#[inline(always)]
pub(super) fn real_loop(
    re: &[FixpDbl],
    len: i32,
    len_scale: i32,
    accu1: FixpDbl,
    accu3: FixpDbl,
    accu5: FixpDbl,
) -> (FixpDbl, FixpDbl, FixpDbl) {
    let pairs = (len - 2) >> 1;
    // the loop reads re[2 .. 2 pairs + 4]
    if pairs <= 0 || re.len() < 2 * pairs as usize + 4 || !(0..32).contains(&len_scale) {
        return real_loop_ref(re, len, len_scale, accu1, accu3, accu5);
    }
    let pairs = pairs as usize;
    let (done, s1, s3, s5) = real_simd(re, pairs, len_scale);
    // the remaining pairs as in the straight loop
    let (mut a1, mut a3, mut a5) =
        (accu1.wrapping_add(s1), accu3.wrapping_add(s3), accu5.wrapping_add(s5));
    for k in done..pairs {
        let p = 2 + 2 * k;
        a1 = a1.wrapping_add(f_pow2_div2(re[p]).wrapping_add(f_pow2_div2(re[p + 1])) >> len_scale);
        a3 = a3.wrapping_add(
            f_mult_div2(re[p], re[p + 1]).wrapping_add(f_mult_div2(re[p + 1], re[p + 2]))
                >> len_scale,
        );
        a5 = a5.wrapping_add(
            f_mult_div2(re[p], re[p + 2]).wrapping_add(f_mult_div2(re[p + 1], re[p + 3]))
                >> len_scale,
        );
    }
    (a1, a3, a5)
}

/// The loop of `autoCorr2nd_cplx()` (see the module documentation).
#[inline(always)]
pub(super) fn cplx_loop(
    re: &[FixpDbl],
    im: &[FixpDbl],
    len: i32,
    len_scale: i32,
    sums: [FixpDbl; 5],
) -> [FixpDbl; 5] {
    // the loop runs p = 1 .. len - 1 and reads re[p .. p + 3], im[p .. p + 3]
    if len < 1 || re.len() < len as usize + 2 || im.len() < len as usize + 2 {
        return cplx_loop_ref(re, im, len, len_scale, sums);
    }
    if !(0..32).contains(&len_scale) {
        return cplx_loop_ref(re, im, len, len_scale, sums);
    }
    let n = len as usize - 1; // terms p = 1 ..= n
    let (done, add) = cplx_simd(re, im, n, len_scale);
    let mut s = [0 as FixpDbl; 5];
    for (s, (&a, &b)) in s.iter_mut().zip(sums.iter().zip(&add)) {
        *s = a.wrapping_add(b);
    }
    let [mut accu1, mut accu3, mut accu5, mut accu7, mut accu8] = s;
    for p in 1 + done..=n {
        accu1 =
            accu1.wrapping_add(f_pow2_div2(re[p]).wrapping_add(f_pow2_div2(im[p])) >> len_scale);
        accu3 = accu3.wrapping_add(
            f_mult_div2(re[p], re[p + 1]).wrapping_add(f_mult_div2(im[p], im[p + 1])) >> len_scale,
        );
        accu5 = accu5.wrapping_add(
            f_mult_div2(im[p + 1], re[p]).wrapping_sub(f_mult_div2(re[p + 1], im[p])) >> len_scale,
        );
        accu7 = accu7.wrapping_add(
            f_mult_div2(re[p + 2], re[p]).wrapping_add(f_mult_div2(im[p + 2], im[p])) >> len_scale,
        );
        accu8 = accu8.wrapping_add(
            f_mult_div2(im[p + 2], re[p]).wrapping_sub(f_mult_div2(re[p + 2], im[p])) >> len_scale,
        );
    }
    [accu1, accu3, accu5, accu7, accu8]
}

/// The pairs of `autoCorr2nd_real()` in SIMD blocks: `(count, sum1, sum3, sum5)` (count 0
/// without SIMD). `re.len() >= 2 pairs + 4`.
#[inline(always)]
fn real_simd(re: &[FixpDbl], pairs: usize, len_scale: i32) -> (usize, FixpDbl, FixpDbl, FixpDbl) {
    debug_assert!(re.len() >= 2 * pairs + 4);
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is part of AArch64; the blocks read re[2 .. 2 count + 4].
        unsafe { neon::real(re, pairs, len_scale) }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if crate::simd::avx2() {
            // SAFETY: the CPU has AVX2; the blocks read re[2 .. 2 count + 4].
            unsafe { x86::real(re, pairs, len_scale) }
        } else {
            (0, 0, 0, 0)
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    {
        let _ = (re, pairs, len_scale);
        (0, 0, 0, 0)
    }
}

/// The terms `p = 1 ..` of `autoCorr2nd_cplx()` in SIMD blocks: `(count, sums)` (count 0 without
/// SIMD). `re.len()`, `im.len() >= n + 3`.
#[inline(always)]
fn cplx_simd(re: &[FixpDbl], im: &[FixpDbl], n: usize, len_scale: i32) -> (usize, [FixpDbl; 5]) {
    debug_assert!(re.len() >= n + 3 && im.len() >= n + 3);
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is part of AArch64; the blocks read re, im [1 .. count + 3].
        unsafe { neon::cplx(re, im, n, len_scale) }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if crate::simd::avx2() {
            // SAFETY: the CPU has AVX2; the blocks read re, im [1 .. count + 3].
            unsafe { x86::cplx(re, im, n, len_scale) }
        } else {
            (0, [0; 5])
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    {
        let _ = (re, im, n, len_scale);
        (0, [0; 5])
    }
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use super::*;
    use core::arch::aarch64::*;

    /// `(a * b) >> 32` per lane (the high halves of the 64-bit products).
    #[inline(always)]
    unsafe fn hi(a: int32x4_t, b: int32x4_t) -> int32x4_t {
        let lo = vreinterpretq_s32_s64(vmull_s32(vget_low_s32(a), vget_low_s32(b)));
        let hi = vreinterpretq_s32_s64(vmull_high_s32(a, b));
        vuzp2q_s32(lo, hi)
    }

    /// The pairs `0 .. 4 (pairs / 4)` of the loop of `autoCorr2nd_real()`, four per block (pair
    /// `k` reads `re[2 + 2k .. 2 + 2k + 4]`): `(count, sum1, sum3, sum5)`.
    ///
    /// # Safety
    /// `re.len() >= 2 pairs + 4`.
    pub(super) unsafe fn real(
        re: &[FixpDbl],
        pairs: usize,
        len_scale: i32,
    ) -> (usize, FixpDbl, FixpDbl, FixpDbl) {
        let sh = vdupq_n_s32(-len_scale);
        let (mut a1, mut a3, mut a5) = (vdupq_n_s32(0), vdupq_n_s32(0), vdupq_n_s32(0));
        let blocks = pairs / 4;
        let mut x = re.as_ptr().add(2);
        for _ in 0..blocks {
            // x[0..10] (block of the pairs (x[0], x[1]) .. (x[6], x[7]))
            let (x0, x0b) = (vld1q_s32(x), vld1q_s32(x.add(4)));
            let (x1, x1b) = (vld1q_s32(x.add(1)), vld1q_s32(x.add(5)));
            let (x2, x2b) = (vld1q_s32(x.add(2)), vld1q_s32(x.add(6)));
            let t1 = vpaddq_s32(hi(x0, x0), hi(x0b, x0b));
            let t3 = vpaddq_s32(hi(x0, x1), hi(x0b, x1b));
            let t5 = vpaddq_s32(hi(x0, x2), hi(x0b, x2b));
            a1 = vaddq_s32(a1, vshlq_s32(t1, sh));
            a3 = vaddq_s32(a3, vshlq_s32(t3, sh));
            a5 = vaddq_s32(a5, vshlq_s32(t5, sh));
            x = x.add(8);
        }
        let mut s = (vaddvq_s32(a1), vaddvq_s32(a3), vaddvq_s32(a5));
        let mut done = 4 * blocks;
        if pairs - done >= 2 {
            // two more pairs: x[0..6]
            let (x0, x1, x2) = (vld1q_s32(x), vld1q_s32(x.add(1)), vld1q_s32(x.add(2)));
            let sh = vget_low_s32(sh);
            let half = |t: int32x4_t| vshl_s32(vpadd_s32(vget_low_s32(t), vget_high_s32(t)), sh);
            s.0 = s.0.wrapping_add(vaddv_s32(half(hi(x0, x0))));
            s.1 = s.1.wrapping_add(vaddv_s32(half(hi(x0, x1))));
            s.2 = s.2.wrapping_add(vaddv_s32(half(hi(x0, x2))));
            done += 2;
        }
        (done, s.0, s.1, s.2)
    }

    /// The terms `p = 1 .. 1 + 4 (n / 4)` of the loop of `autoCorr2nd_cplx()`, four per block:
    /// `(count, sums)`.
    ///
    /// # Safety
    /// `re.len()`, `im.len() >= n + 3`.
    pub(super) unsafe fn cplx(
        re: &[FixpDbl],
        im: &[FixpDbl],
        n: usize,
        len_scale: i32,
    ) -> (usize, [FixpDbl; 5]) {
        let sh = vdupq_n_s32(-len_scale);
        let z = vdupq_n_s32(0);
        let (mut a1, mut a3, mut a5, mut a7, mut a8) = (z, z, z, z, z);
        let blocks = n / 4;
        let (mut r, mut i) = (re.as_ptr().add(1), im.as_ptr().add(1));
        for _ in 0..blocks {
            let (r0, r1, r2) = (vld1q_s32(r), vld1q_s32(r.add(1)), vld1q_s32(r.add(2)));
            let (i0, i1, i2) = (vld1q_s32(i), vld1q_s32(i.add(1)), vld1q_s32(i.add(2)));
            let t1 = vaddq_s32(hi(r0, r0), hi(i0, i0));
            let t3 = vaddq_s32(hi(r0, r1), hi(i0, i1));
            let t5 = vsubq_s32(hi(i1, r0), hi(r1, i0));
            let t7 = vaddq_s32(hi(r2, r0), hi(i2, i0));
            let t8 = vsubq_s32(hi(i2, r0), hi(r2, i0));
            a1 = vaddq_s32(a1, vshlq_s32(t1, sh));
            a3 = vaddq_s32(a3, vshlq_s32(t3, sh));
            a5 = vaddq_s32(a5, vshlq_s32(t5, sh));
            a7 = vaddq_s32(a7, vshlq_s32(t7, sh));
            a8 = vaddq_s32(a8, vshlq_s32(t8, sh));
            r = r.add(4);
            i = i.add(4);
        }
        let sums = [vaddvq_s32(a1), vaddvq_s32(a3), vaddvq_s32(a5), vaddvq_s32(a7), vaddvq_s32(a8)];
        (4 * blocks, sums)
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86 {
    use super::*;
    #[cfg(target_arch = "x86")]
    use core::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;

    /// `(a * b) >> 32` per lane (the high halves of the 64-bit products of `vpmuldq`).
    #[inline(always)]
    unsafe fn hi(a: __m256i, b: __m256i) -> __m256i {
        let even = _mm256_srli_epi64::<32>(_mm256_mul_epi32(a, b));
        let odd = _mm256_mul_epi32(_mm256_srli_epi64::<32>(a), _mm256_srli_epi64::<32>(b));
        _mm256_blend_epi32::<0b1010_1010>(even, odd)
    }

    #[inline(always)]
    unsafe fn ld(p: *const FixpDbl) -> __m256i {
        _mm256_loadu_si256(p as *const __m256i)
    }

    /// The wrapping sum of the lanes.
    #[inline(always)]
    unsafe fn hsum(v: __m256i) -> FixpDbl {
        let mut a = [0 as FixpDbl; 8];
        _mm256_storeu_si256(a.as_mut_ptr() as *mut __m256i, v);
        a.iter().fold(0, |s: FixpDbl, &x| s.wrapping_add(x))
    }

    /// The AVX2 form of `neon::real`: the pairs `0 .. 8 (pairs / 8)`, eight per block
    /// (`vphaddd` adds the neighbouring lanes).
    ///
    /// # Safety
    /// AVX2; `re.len() >= 2 pairs + 4`.
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn real(
        re: &[FixpDbl],
        pairs: usize,
        len_scale: i32,
    ) -> (usize, FixpDbl, FixpDbl, FixpDbl) {
        let sh = _mm_cvtsi32_si128(len_scale);
        let z = _mm256_setzero_si256();
        let (mut a1, mut a3, mut a5) = (z, z, z);
        let blocks = pairs / 8;
        let mut x = re.as_ptr().add(2);
        for _ in 0..blocks {
            // x[0..18] (block of the pairs (x[0], x[1]) .. (x[14], x[15]))
            let (x0, x0b) = (ld(x), ld(x.add(8)));
            let (x1, x1b) = (ld(x.add(1)), ld(x.add(9)));
            let (x2, x2b) = (ld(x.add(2)), ld(x.add(10)));
            let t1 = _mm256_hadd_epi32(hi(x0, x0), hi(x0b, x0b));
            let t3 = _mm256_hadd_epi32(hi(x0, x1), hi(x0b, x1b));
            let t5 = _mm256_hadd_epi32(hi(x0, x2), hi(x0b, x2b));
            a1 = _mm256_add_epi32(a1, _mm256_sra_epi32(t1, sh));
            a3 = _mm256_add_epi32(a3, _mm256_sra_epi32(t3, sh));
            a5 = _mm256_add_epi32(a5, _mm256_sra_epi32(t5, sh));
            x = x.add(16);
        }
        (8 * blocks, hsum(a1), hsum(a3), hsum(a5))
    }

    /// The AVX2 form of `neon::cplx`: the terms `p = 1 .. 1 + 8 (n / 8)`, eight per block.
    ///
    /// # Safety
    /// AVX2; `re.len()`, `im.len() >= n + 3`.
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn cplx(
        re: &[FixpDbl],
        im: &[FixpDbl],
        n: usize,
        len_scale: i32,
    ) -> (usize, [FixpDbl; 5]) {
        let sh = _mm_cvtsi32_si128(len_scale);
        let z = _mm256_setzero_si256();
        let (mut a1, mut a3, mut a5, mut a7, mut a8) = (z, z, z, z, z);
        let blocks = n / 8;
        let (mut r, mut i) = (re.as_ptr().add(1), im.as_ptr().add(1));
        for _ in 0..blocks {
            let (r0, r1, r2) = (ld(r), ld(r.add(1)), ld(r.add(2)));
            let (i0, i1, i2) = (ld(i), ld(i.add(1)), ld(i.add(2)));
            let t1 = _mm256_add_epi32(hi(r0, r0), hi(i0, i0));
            let t3 = _mm256_add_epi32(hi(r0, r1), hi(i0, i1));
            let t5 = _mm256_sub_epi32(hi(i1, r0), hi(r1, i0));
            let t7 = _mm256_add_epi32(hi(r2, r0), hi(i2, i0));
            let t8 = _mm256_sub_epi32(hi(i2, r0), hi(r2, i0));
            a1 = _mm256_add_epi32(a1, _mm256_sra_epi32(t1, sh));
            a3 = _mm256_add_epi32(a3, _mm256_sra_epi32(t3, sh));
            a5 = _mm256_add_epi32(a5, _mm256_sra_epi32(t5, sh));
            a7 = _mm256_add_epi32(a7, _mm256_sra_epi32(t7, sh));
            a8 = _mm256_add_epi32(a8, _mm256_sra_epi32(t8, sh));
            r = r.add(8);
            i = i.add(8);
        }
        (8 * blocks, [hsum(a1), hsum(a3), hsum(a5), hsum(a7), hsum(a8)])
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
        fn val(&mut self, mode: u32) -> i32 {
            match mode {
                0 => [i32::MIN, i32::MIN + 1, i32::MAX, i32::MAX - 1, 0, -1, 1 << 30, -(1 << 30)]
                    [(self.next() % 8) as usize],
                1 => self.next() as i32,
                n => (self.next() as i32) >> (n * 3),
            }
        }
        fn vec(&mut self, n: usize) -> Vec<i32> {
            let mode = self.next() % 8;
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                let m = if mode == 7 { self.next() % 7 } else { mode };
                v.push(self.val(m));
            }
            v
        }
    }

    /// Both autocorrelations against the straight ports: every length up to 80 (the decoder and
    /// the encoder use up to 40), random and extreme values, including `MINVAL_DBL`.
    #[test]
    fn autocorr_matches_straight() {
        let mut r = Rng(0x7777_1234_9876_5432);
        for it in 0..60_000 {
            let len = (it % 80) as i32 + 1;
            let n = len as usize + 2 + (r.next() % 3) as usize;
            let re = r.vec(n);
            let im = r.vec(n);
            if len >= 2 {
                let (mut a, mut b) = (AcorrCoefs::default(), AcorrCoefs::default());
                let x = auto_corr2nd_real(&mut a, &re, len);
                let y = auto_corr2nd_real_ref(&mut b, &re, len);
                assert_eq!((x, a), (y, b), "real it {it} len {len}");
            }
            let (mut a, mut b) = (AcorrCoefs::default(), AcorrCoefs::default());
            let x = auto_corr2nd_cplx(&mut a, &re, &im, len);
            let y = auto_corr2nd_cplx_ref(&mut b, &re, &im, len);
            assert_eq!((x, a), (y, b), "cplx it {it} len {len}");
        }
    }
}
