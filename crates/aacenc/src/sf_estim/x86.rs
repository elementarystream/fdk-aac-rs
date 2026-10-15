// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/sf_estim.cpp and libFDK/include/x86/fixpoint_math_x86.h (optimized;
// no counterpart in the reference).

//! x86 AVX2 versions of the form factor sum of `FDKaacEnc_FDKaacEnc_CalcFormFactorChannel()`:
//! `sum(sqrtFixp(fixp_abs(spec[j])) >> FORM_FAC_SHIFT)` (wrapping), selected at run time
//! ([`crate::simd::avx2`]).
//!
//! - `x86`: `sqrtFixp(op)` is `(INT)(sqrtf((float)op) * 46340.9492f)` in single precision
//!   (`cvtsi2ss`, `sqrtss`, `mulss`, `cvttss2si`). The packed instructions (`vcvtdq2ps`,
//!   `vsqrtps`, `vmulps`, `vcvttps2dq`) round each lane the same way (round to nearest for the
//!   conversion, correctly rounded square root and product, truncation), eight lines at a time.
//! - `x86_32`: the x87 expression `(INT)(sqrt((long double)op) * 46340.9492f)`, decided in double
//!   precision as `fdk_aac_rs_fdk::x87::fast` does (four lines at a time with `vsqrtpd`, the same
//!   correctly rounded operations): the `f64` result `y` truncates to the x87 result unless its
//!   fraction lies within 2^-20 of an integer. A group with such a lane (other than 0, which is
//!   exact) calls `sqrtFixp()` for every line of the group, which gives the same result for every
//!   operand in all three x87 implementations.
//!
//! A group with `MINVAL_DBL` (whose `fixp_abs()` stays negative) takes the scalar code (which
//! also checks the `sqrtFixp()` assertion); the last lines that do not fill a group as well.

use super::FORM_FAC_SHIFT;
#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;
use fdk_aac_rs_fdk::common_fix::{fixp_abs, FixpDbl};
use fdk_aac_rs_fdk::fixpoint_math::sqrt_fixp;

/// `46340.9492f` of `sqrtFixp()` (`x86/fixpoint_math_x86.h`).
const SQRT_FIXP_SCALE: f32 = 46340.9492;
/// The fraction band where the `x86_32` lanes are not decided by the `f64` result (the bound of
/// `fdk_aac_rs_fdk::x87::fast::try_sqrt_fixp`).
const SQRT_FIXP_TOL: f64 = 1.0 / (1u32 << 20) as f64;

/// The scalar sum over `spec`.
#[inline(always)]
fn scalar(spec: &[FixpDbl]) -> FixpDbl {
    let mut sum: FixpDbl = 0;
    for &x in spec {
        sum = sum.wrapping_add(sqrt_fixp(fixp_abs(x)) >> FORM_FAC_SHIFT);
    }
    sum
}

/// The terms of the lines `g` (in `x`, zero lanes past them) added to `vsum`, or with the scalar
/// code to `sum` if a line is `MINVAL_DBL` (whose `fixp_abs()` stays negative).
#[inline(always)]
unsafe fn add8(g: &[FixpDbl], x: __m256i, scale: __m256, sum: &mut FixpDbl, vsum: &mut __m256i) {
    let op = _mm256_abs_epi32(x);
    if _mm256_movemask_ps(_mm256_castsi256_ps(op)) != 0 {
        *sum = sum.wrapping_add(scalar(g));
        return;
    }
    let r = _mm256_cvttps_epi32(_mm256_mul_ps(_mm256_sqrt_ps(_mm256_cvtepi32_ps(op)), scale));
    *vsum = _mm256_add_epi32(*vsum, _mm256_srai_epi32::<FORM_FAC_SHIFT>(r));
}

/// The form factor sum for the `x86` variant.
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn form_factor_sum_x86(spec: &[FixpDbl]) -> FixpDbl {
    let mut sum: FixpDbl = 0;
    let mut vsum = _mm256_setzero_si256();
    let scale = _mm256_set1_ps(SQRT_FIXP_SCALE);
    let groups = spec.chunks_exact(8);
    let mut tail = groups.remainder();
    for g in groups {
        add8(g, _mm256_loadu_si256(g.as_ptr() as *const __m256i), scale, &mut sum, &mut vsum);
    }
    if tail.len() >= 4 {
        // 4 remaining lines as a vector with 4 zero lanes: a zero line adds
        // sqrt(0) * 46340.9492 = 0
        let (g, t) = tail.split_at(4);
        let x = _mm256_zextsi128_si256(_mm_loadu_si128(g.as_ptr() as *const __m128i));
        add8(g, x, scale, &mut sum, &mut vsum);
        tail = t;
    }
    let s = _mm_add_epi32(_mm256_castsi256_si128(vsum), _mm256_extracti128_si256::<1>(vsum));
    let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b0100_1110>(s));
    let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b1011_0001>(s));
    sum.wrapping_add(_mm_cvtsi128_si32(s)).wrapping_add(scalar(tail))
}

/// The form factor sum for the `x86_32` variant.
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn form_factor_sum_x86_32(spec: &[FixpDbl]) -> FixpDbl {
    let mut sum: FixpDbl = 0;
    let mut vsum = _mm_setzero_si128();
    let scale = _mm256_set1_pd(SQRT_FIXP_SCALE as f64);
    let lo = _mm256_set1_pd(SQRT_FIXP_TOL);
    let hi = _mm256_set1_pd(1.0 - SQRT_FIXP_TOL);
    let groups = spec.chunks_exact(4);
    let tail = groups.remainder();
    for g in groups {
        let op = _mm_abs_epi32(_mm_loadu_si128(g.as_ptr() as *const __m128i));
        if _mm_movemask_ps(_mm_castsi128_ps(op)) == 0 {
            let y = _mm256_mul_pd(_mm256_sqrt_pd(_mm256_cvtepi32_pd(op)), scale);
            let t = _mm256_cvttpd_epi32(y);
            let frac = _mm256_sub_pd(y, _mm256_cvtepi32_pd(t));
            let decided = _mm256_and_pd(
                _mm256_cmp_pd::<_CMP_GT_OQ>(frac, lo),
                _mm256_cmp_pd::<_CMP_LT_OQ>(frac, hi),
            );
            // 0 is exact (and is excluded from the bound by the scalar code as well)
            let zero = _mm_movemask_ps(_mm_castsi128_ps(_mm_cmpeq_epi32(op, _mm_setzero_si128())));
            if _mm256_movemask_pd(decided) | zero == 0b1111 {
                vsum = _mm_add_epi32(vsum, _mm_srai_epi32::<FORM_FAC_SHIFT>(t));
                continue;
            }
        }
        sum = sum.wrapping_add(scalar(g));
    }
    let s = _mm_add_epi32(vsum, _mm_shuffle_epi32::<0b0100_1110>(vsum));
    let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b1011_0001>(s));
    sum.wrapping_add(_mm_cvtsi128_si32(s)).wrapping_add(scalar(tail))
}
