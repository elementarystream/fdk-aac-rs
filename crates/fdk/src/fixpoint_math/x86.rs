// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/fixpoint_math.h (optimized; no counterpart in the reference).

//! x86 AVX2 version of `CalcLdData()` (`fLog2(x, 0)`) for eight values at a time,
//! used by [`super::ld_data_vector`] when [`crate::simd::avx2`] holds.
//!
//! Every lane computes what [`super::f_log2_noexp`] computes for `x_e == 0` (see the NEON
//! version for the formulas); the products are exact 64-bit `vpmuldq`/`vpmuludq` products of the
//! even and odd lanes, of which the high words are kept:
//!
//! - `clz(x)` of `0 < x < 2^31` from the exponent of the `float` conversion of `x & !(x >> 1)`
//!   (clearing the bit below the leading one keeps the rounding from carrying into the next
//!   power of two), the same for `b - 1`;
//! - powers `floor(p * 2xm / 2^32)` unsigned (`2xm <= 2^31`); terms `floor((c << 16) * p / 2^32)`
//!   and `floor((c << 17) * q / 2^32)` signed (`c << 17` fits 32 bits for every coefficient but
//!   the first); the final `r + floor(r * C / 2^32)`;
//! - variable shifts `vpsravd`/`vpsllvd` with amounts in `0..=31`.

use super::{LD_COEFF, LD_PRECISION};
use crate::{Arch, ARCH};
#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// `floor(a * b / 2^32)` of signed lanes.
#[inline(always)]
unsafe fn mulhi_s(a: __m256i, b: __m256i) -> __m256i {
    let even = _mm256_srli_epi64::<32>(_mm256_mul_epi32(a, b));
    let odd = _mm256_mul_epi32(_mm256_srli_epi64::<32>(a), _mm256_srli_epi64::<32>(b));
    _mm256_blend_epi32::<0b1010_1010>(even, odd)
}

/// `floor(a * b / 2^32)` of unsigned lanes.
#[inline(always)]
unsafe fn mulhi_u(a: __m256i, b: __m256i) -> __m256i {
    let even = _mm256_srli_epi64::<32>(_mm256_mul_epu32(a, b));
    let odd = _mm256_mul_epu32(_mm256_srli_epi64::<32>(a), _mm256_srli_epi64::<32>(b));
    _mm256_blend_epi32::<0b1010_1010>(even, odd)
}

/// The leading zeros of every lane `0 <= a < 2^31` (32 for 0).
#[inline(always)]
unsafe fn clz31(a: __m256i) -> __m256i {
    let y = _mm256_andnot_si256(_mm256_srli_epi32::<1>(a), a);
    let e = _mm256_srli_epi32::<23>(_mm256_castps_si256(_mm256_cvtepi32_ps(y)));
    _mm256_min_epi32(_mm256_sub_epi32(_mm256_set1_epi32(158), e), _mm256_set1_epi32(32))
}

/// `CalcLdData()` of eight values.
///
/// # Safety
///
/// The CPU must support AVX2.
#[inline]
#[target_feature(enable = "avx2")]
pub(super) unsafe fn ld_data8(x: __m256i) -> __m256i {
    let one = _mm256_set1_epi32(1);
    let b = _mm256_sub_epi32(clz31(x), one);
    let x2 = _mm256_sllv_epi32(x, b);
    let xm = _mm256_sub_epi32(_mm256_set1_epi32(i32::MIN), x2);
    let xm2 = _mm256_add_epi32(xm, xm);
    let coef = |c: i16, s: u32| _mm256_set1_epi32((c as i32) << s);
    let mut r;
    if matches!(ARCH, Arch::Aarch64) {
        let mut p = xm;
        r = mulhi_s(p, coef(LD_COEFF[0], 16));
        for &c in LD_COEFF[1..LD_PRECISION].iter() {
            p = mulhi_u(p, xm2);
            r = _mm256_add_epi32(r, mulhi_s(p, coef(c, 16)));
        }
    } else {
        r = mulhi_s(xm, coef(LD_COEFF[0], 16));
        let mut q = mulhi_u(xm, xm);
        r = _mm256_add_epi32(r, mulhi_s(q, coef(LD_COEFF[1], 17)));
        for &c in LD_COEFF[2..LD_PRECISION].iter() {
            q = mulhi_u(q, xm2);
            r = _mm256_add_epi32(r, mulhi_s(q, coef(c, 17)));
        }
    }
    // fMultAddDiv2(r, r, FL2FXCONST_DBL(2 * 0.4426950408889634))
    r = _mm256_add_epi32(r, mulhi_s(r, _mm256_set1_epi32(super::LD_POLY_FINAL)));
    // e = 1 for b == 0, else 33 - clz(b - 1)
    let e = _mm256_sub_epi32(_mm256_set1_epi32(33), clz31(_mm256_sub_epi32(b, one)));
    let e = _mm256_blendv_epi8(e, one, _mm256_cmpeq_epi32(b, _mm256_setzero_si256()));
    let m = _mm256_add_epi32(
        _mm256_srav_epi32(r, _mm256_sub_epi32(e, one)),
        _mm256_sllv_epi32(
            _mm256_sub_epi32(_mm256_setzero_si256(), b),
            _mm256_sub_epi32(_mm256_set1_epi32(31), e),
        ),
    );
    let out = _mm256_srav_epi32(m, _mm256_sub_epi32(_mm256_set1_epi32(6), e));
    _mm256_blendv_epi8(
        _mm256_set1_epi32(i32::MIN),
        out,
        _mm256_cmpgt_epi32(x, _mm256_setzero_si256()),
    )
}
