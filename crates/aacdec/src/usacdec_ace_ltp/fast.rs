// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized version of Pred_lt4() of libAACdec/src/usacdec_ace_ltp.cpp.

//! `Pred_lt4()` with several outputs at once (an optimization of the port; the straight port is
//! [`super::pred_lt4_ref`]).
//!
//! Output `j` (`0 <= j <= L_SUBFR`) is `fAddSaturate(sumt << 1, sumb << 1)` of the two sums
//! `sumt = sum_k fMultDiv2(exc_buf[x + j + 2k], hi(tab[k]))` and
//! `sumb = sum_k fMultDiv2(exc_buf[x + j + 2k + 1], lo(tab[k]))` (`k < 16`, wrapping sums, so
//! their order does not matter), written to `exc_buf[exc + j]`. The outputs depend on each other
//! only through the buffer: output `j` reads `exc_buf[x + j .. x + j + 32]`, which contains
//! earlier outputs when the pitch lag is short. A block of `LANES` consecutive outputs
//! `j0 .. j0 + LANES` therefore gives the values of the sequential loop if every value it reads
//! precedes the block's first output, i.e. `x + j0 + LANES - 1 + 31 < exc + j0`, which is
//! `x + LANES + 30 < exc` for every block (pitch lags of at least 20 or 24, the decoder's minimum
//! is 34). The blocks run in order, so all earlier outputs are stored before a block reads them;
//! the last output (`j = L_SUBFR`) is computed after the blocks with the scalar code.
//!
//! `fMultDiv2(FIXP_DBL a, FIXP_SGL b)` is `(a * b) >> 16` on every variant: NEON computes it as
//! `sqdmulh(a, b << 15)` (`b << 15` is never `INT_MIN`, so it does not saturate), AVX2 exactly in
//! 32-bit lanes as `(a >> 16) * b + (((a & 0xFFFF) * b) >> 16)` (no intermediate value exceeds 31
//! bits). `fAddSaturate(a, b)` is `min(max((a >> 1) + (b >> 1), MINVAL_DBL >> 1), MAXVAL_DBL >> 1)
//! << 1`; the shifts `<< 1` wrap as in C.

use super::*;
#[allow(unused_imports)]
use fdk_aac_rs_fdk::common_fix::{MAXVAL_DBL, MINVAL_DBL};

/// The number of values one output reads (`2 * L_INTERPOL2`).
const TAPS: usize = 2 * L_INTERPOL2 as usize;
/// The number of outputs (`L_SUBFR + 1`).
const OUTPUTS: usize = L_SUBFR as usize + 1;

/// `Pred_lt4()` in SIMD blocks; returns `false` (nothing done) where the blocks do not apply: an
/// access outside of `exc_buf` (the straight port reads 0 there), a pitch lag too short for the
/// block width, or no SIMD on this target or CPU.
pub(super) fn pred_lt4(exc_buf: &mut [FixpDbl], exc_off: usize, t0: i32, frac: i32) -> bool {
    let (x, row) = pred_lt4_setup(exc_off, t0, frac);
    let n = exc_buf.len();
    if x < 0 || x as usize + OUTPUTS - 1 + TAPS > n || exc_off + OUTPUTS > n {
        return false;
    }
    let x = x as usize;
    let tab = &PRED_LT4_INTER4_2[row];
    #[cfg(target_arch = "aarch64")]
    if x + 4 + 30 < exc_off {
        // SAFETY: NEON is part of AArch64; the accesses are inside exc_buf (checked above).
        unsafe { neon::pred_lt4(exc_buf, x, exc_off, tab) };
        return true;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if x + 8 + 30 < exc_off && fdk_aac_rs_fdk::simd::avx2() {
        // SAFETY: the CPU has AVX2; the accesses are inside exc_buf (checked above).
        unsafe { x86::pred_lt4(exc_buf, x, exc_off, tab) };
        return true;
    }
    let _ = (x, tab);
    false
}

/// One output from its `TAPS` input values (the scalar code of the straight port).
#[inline(always)]
#[allow(dead_code)]
fn one(w: &[FixpDbl], tab: &[i32; L_INTERPOL2 as usize]) -> FixpDbl {
    let mut l_sumt = f_mult_div2(w[0], (tab[0] >> 16) as i16);
    let mut l_sumb = f_mult_div2(w[1], tab[0] as i16);
    for k in 1..L_INTERPOL2 as usize {
        l_sumt = f_mult_add_div2(l_sumt, w[2 * k], (tab[k] >> 16) as i16);
        l_sumb = f_mult_add_div2(l_sumb, w[2 * k + 1], tab[k] as i16);
    }
    f_add_saturate(l_sumt << 1, l_sumb << 1)
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use super::*;
    use core::arch::aarch64::*;

    /// Adds the products of the taps `2k` (top halves) and `2k + 1` (bottom halves) of 4 outputs.
    macro_rules! taps {
        ($q:ident, $st:ident, $sb:ident, $ct:ident, $cb:ident; $($k:literal $v:literal $l:literal),*) => {
            $(
                $st = vaddq_s32($st, vqdmulhq_laneq_s32::<$l>(vld1q_s32($q.add(2 * $k)), $ct[$v]));
                $sb = vaddq_s32($sb, vqdmulhq_laneq_s32::<$l>(vld1q_s32($q.add(2 * $k + 1)), $cb[$v]));
            )*
        };
    }

    /// # Safety
    /// `x + OUTPUTS - 1 + TAPS <= buf.len()`, `exc + OUTPUTS <= buf.len()`, `x + 34 < exc`.
    pub(super) unsafe fn pred_lt4(
        buf: &mut [FixpDbl],
        x: usize,
        exc: usize,
        tab: &[i32; L_INTERPOL2 as usize],
    ) {
        let mut ct = [0i32; 16];
        let mut cb = [0i32; 16];
        for k in 0..16 {
            ct[k] = (((tab[k] >> 16) as i16) as i32) << 15;
            cb[k] = ((tab[k] as i16) as i32) << 15;
        }
        let ct = [
            vld1q_s32(ct.as_ptr()),
            vld1q_s32(ct.as_ptr().add(4)),
            vld1q_s32(ct.as_ptr().add(8)),
            vld1q_s32(ct.as_ptr().add(12)),
        ];
        let cb = [
            vld1q_s32(cb.as_ptr()),
            vld1q_s32(cb.as_ptr().add(4)),
            vld1q_s32(cb.as_ptr().add(8)),
            vld1q_s32(cb.as_ptr().add(12)),
        ];
        let lo = vdupq_n_s32(MINVAL_DBL >> 1);
        let hi = vdupq_n_s32(MAXVAL_DBL >> 1);
        let p = buf.as_mut_ptr();
        let mut j0 = 0;
        while j0 < OUTPUTS - 1 {
            let q = p.add(x + j0) as *const i32;
            let mut st = vdupq_n_s32(0);
            let mut sb = vdupq_n_s32(0);
            taps!(q, st, sb, ct, cb;
                0 0 0, 1 0 1, 2 0 2, 3 0 3, 4 1 0, 5 1 1, 6 1 2, 7 1 3,
                8 2 0, 9 2 1, 10 2 2, 11 2 3, 12 3 0, 13 3 1, 14 3 2, 15 3 3);
            let a = vshrq_n_s32::<1>(vshlq_n_s32::<1>(st));
            let b = vshrq_n_s32::<1>(vshlq_n_s32::<1>(sb));
            let s = vminq_s32(vmaxq_s32(vaddq_s32(a, b), lo), hi);
            vst1q_s32(p.add(exc + j0), vshlq_n_s32::<1>(s));
            j0 += 4;
        }
        let w = core::slice::from_raw_parts(p.add(x + OUTPUTS - 1), TAPS);
        let v = one(w, tab);
        *p.add(exc + OUTPUTS - 1) = v;
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86 {
    use super::*;
    #[cfg(target_arch = "x86")]
    use core::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;

    /// `fMultDiv2(a, b)` for 16-bit `b` (see the module documentation).
    #[inline(always)]
    unsafe fn md2v(a: __m256i, b: __m256i) -> __m256i {
        let hi = _mm256_mullo_epi32(_mm256_srai_epi32::<16>(a), b);
        let lo = _mm256_and_si256(a, _mm256_set1_epi32(0xFFFF));
        _mm256_add_epi32(hi, _mm256_srai_epi32::<16>(_mm256_mullo_epi32(lo, b)))
    }

    /// # Safety
    /// AVX2; `x + OUTPUTS - 1 + TAPS <= buf.len()`, `exc + OUTPUTS <= buf.len()`, `x + 38 < exc`.
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn pred_lt4(
        buf: &mut [FixpDbl],
        x: usize,
        exc: usize,
        tab: &[i32; L_INTERPOL2 as usize],
    ) {
        let mut ct = [_mm256_setzero_si256(); 16];
        let mut cb = [_mm256_setzero_si256(); 16];
        for k in 0..16 {
            ct[k] = _mm256_set1_epi32(((tab[k] >> 16) as i16) as i32);
            cb[k] = _mm256_set1_epi32((tab[k] as i16) as i32);
        }
        let lo = _mm256_set1_epi32(MINVAL_DBL >> 1);
        let hi = _mm256_set1_epi32(MAXVAL_DBL >> 1);
        let p = buf.as_mut_ptr();
        let mut j0 = 0;
        while j0 < OUTPUTS - 1 {
            let q = p.add(x + j0);
            let mut st = _mm256_setzero_si256();
            let mut sb = _mm256_setzero_si256();
            for k in 0..16 {
                let v0 = _mm256_loadu_si256(q.add(2 * k) as *const __m256i);
                let v1 = _mm256_loadu_si256(q.add(2 * k + 1) as *const __m256i);
                st = _mm256_add_epi32(st, md2v(v0, ct[k]));
                sb = _mm256_add_epi32(sb, md2v(v1, cb[k]));
            }
            let a = _mm256_srai_epi32::<1>(_mm256_slli_epi32::<1>(st));
            let b = _mm256_srai_epi32::<1>(_mm256_slli_epi32::<1>(sb));
            let s = _mm256_min_epi32(_mm256_max_epi32(_mm256_add_epi32(a, b), lo), hi);
            _mm256_storeu_si256(p.add(exc + j0) as *mut __m256i, _mm256_slli_epi32::<1>(s));
            j0 += 8;
        }
        let w = core::slice::from_raw_parts(p.add(x + OUTPUTS - 1), TAPS);
        let v = one(w, tab);
        *p.add(exc + OUTPUTS - 1) = v;
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
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() % (hi - lo + 1) as u32) as i32
        }
        fn val(&mut self, mode: u32) -> i32 {
            match mode {
                0 => [i32::MIN, i32::MIN + 1, i32::MAX, i32::MAX - 1, 0, -1, 1 << 30, -(1 << 30)]
                    [(self.next() % 8) as usize],
                1 => self.next() as i32,
                n => (self.next() as i32) >> (n * 3),
            }
        }
    }

    /// The SIMD blocks against the straight port: random and extreme excitations, every pitch
    /// lag around the block-width limits and up to the excitation memory, every fraction, every
    /// subframe offset of the decoder, and buffers too short for the blocks.
    #[test]
    fn pred_lt4_matches_straight() {
        let mut r = Rng(0x5eed_1234_abcd_0001);
        // PIT_MAX_MAX + L_INTERPOL + L_DIV + 1 (CLpd_AcelpDecode) and shorter buffers
        for it in 0..60_000u32 {
            let len = if it % 16 == 0 { r.range(64, 685) as usize } else { 685 };
            let mode = r.next() % 6;
            let buf: Vec<i32> = (0..len).map(|_| r.val(if mode == 5 { 0 } else { mode })).collect();
            let off = if it % 16 == 1 {
                r.range(0, len as i32) as usize
            } else {
                428 + 64 * (r.next() % 4) as usize
            };
            let t0 = match it % 4 {
                0 => r.range(16, 30),
                1 => r.range(16, 80),
                2 => r.range(-4, 420),
                _ => r.range(16, 411),
            };
            let frac = if it % 64 == 3 { r.range(-2, 6) } else { (r.next() % 4) as i32 };
            if off > len {
                continue;
            }
            let mut a = buf.clone();
            let mut b = buf;
            super::super::pred_lt4(&mut a, off, t0, frac);
            pred_lt4_ref(&mut b, off, t0, frac);
            assert_eq!(a, b, "it {it} len {len} off {off} t0 {t0} frac {frac}");
        }
    }
}
