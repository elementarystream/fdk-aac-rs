// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/aacenc_tns.cpp (optimized; no counterpart in the reference).

//! x86 AVX2 version of the sum of `FDKaacEnc_CalcAutoCorrValue()`, selected at run time
//! ([`crate::simd::avx2`]).
//!
//! The sum `sum(fMult(a[i], b[i]) >> scale)` (or `fPow2(a[i]) >> scale` for lag 0) is a wrapping
//! sum, so its order does not matter; eight products per step:
//!
//! - the 64-bit products of the even and odd lanes with `vpmuldq` (exact);
//! - `fMult(FIXP_DBL, FIXP_DBL)` of the variant: `(a * b) >> 31` (bits 31..62 of the product) on
//!   `aarch64`, `((a * b) >> 32) << 1` on the others; `fPow2(a)` = `((a * a) >> 32) << 1` on every
//!   variant;
//! - `>> scale` as `vpsrad` for `0 <= scale < 32` (the caller takes the scalar code otherwise).
//!
//! The lines that do not fill a group of eight are added by the scalar code.

#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;
use fdk_aac_rs_fdk::common_fix::{f_mult, f_pow2, FixpDbl};
use fdk_aac_rs_fdk::{Arch, ARCH};

/// `(a * b) >> 32` of every lane.
#[inline(always)]
unsafe fn mul_hi(a: __m256i, b: __m256i) -> __m256i {
    let even = _mm256_srli_epi64::<32>(_mm256_mul_epi32(a, b));
    let odd = _mm256_mul_epi32(_mm256_srli_epi64::<32>(a), _mm256_srli_epi64::<32>(b));
    _mm256_blend_epi32::<0b1010_1010>(even, odd)
}

/// `(a * b) >> 31` of every lane (the `aarch64` `fMult()`).
#[inline(always)]
unsafe fn mul_hi31(a: __m256i, b: __m256i) -> __m256i {
    let even = _mm256_srli_epi64::<31>(_mm256_mul_epi32(a, b));
    let odd = _mm256_slli_epi64::<1>(_mm256_mul_epi32(
        _mm256_srli_epi64::<32>(a),
        _mm256_srli_epi64::<32>(b),
    ));
    _mm256_blend_epi32::<0b1010_1010>(even, odd)
}

/// The wrapping sum of the eight lanes.
#[inline(always)]
unsafe fn hsum(v: __m256i) -> i32 {
    let s = _mm_add_epi32(_mm256_castsi256_si128(v), _mm256_extracti128_si256::<1>(v));
    let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b0100_1110>(s));
    let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b1011_0001>(s));
    _mm_cvtsi128_si32(s)
}

/// `sum(fMult(a[i], b[i]) >> scale)` (`pow2`: `sum(fPow2(a[i]) >> scale)`, `b` unused), wrapping,
/// for `0 <= scale < 32` and `a.len() == b.len()`.
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn auto_corr_sum(
    a: &[FixpDbl],
    b: &[FixpDbl],
    scale: i32,
    pow2: bool,
) -> FixpDbl {
    debug_assert!((0..32).contains(&scale) && a.len() == b.len());
    let cnt = _mm_cvtsi32_si128(scale);
    let mut acc = _mm256_setzero_si256();
    let ga = a.chunks_exact(8);
    let gb = b.chunks_exact(8);
    let (ta, tb) = (ga.remainder(), gb.remainder());
    if pow2 {
        for x in ga {
            let x = _mm256_loadu_si256(x.as_ptr() as *const __m256i);
            let p = _mm256_slli_epi32::<1>(mul_hi(x, x));
            acc = _mm256_add_epi32(acc, _mm256_sra_epi32(p, cnt));
        }
    } else {
        for (x, y) in ga.zip(gb) {
            let x = _mm256_loadu_si256(x.as_ptr() as *const __m256i);
            let y = _mm256_loadu_si256(y.as_ptr() as *const __m256i);
            let p = match ARCH {
                Arch::Aarch64 => mul_hi31(x, y),
                _ => _mm256_slli_epi32::<1>(mul_hi(x, y)),
            };
            acc = _mm256_add_epi32(acc, _mm256_sra_epi32(p, cnt));
        }
    }
    let mut sum = hsum(acc);
    for (&x, &y) in ta.iter().zip(tb) {
        let p = if pow2 { f_pow2(x) } else { f_mult(x, y) };
        sum = sum.wrapping_add(p >> scale);
    }
    sum
}
