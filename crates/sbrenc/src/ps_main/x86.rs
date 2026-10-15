// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/ps_main.cpp and libFDK/include/x86/fixpoint_math_x86.h (optimized;
// no counterpart in the reference).

//! x86 AVX2 version of the hybrid band loop of `DownmixPSQmfData()` for the `x86` and `x86_32`
//! variants, eight bands at a time (selected at run time, [`crate::simd::avx2`]); every lane
//! computes what [`super::downmix_band`] computes:
//!
//! - the normalization by the unsigned maximum of the four absolute values (`vpabsd`,
//!   `vpmaxud`, the leading zeros from the `float` exponent of `m & !(m >> 1)`, exact for
//!   `m < 2^31`; a group with a maximum of 2^30 or more takes the scalar code), the energies
//!   `fPow2Div2(x)` = `(x * x) >> 32`, the cross terms `fMult(x, y)` = `((x * y) >> 32) << 1` (the x86 variants) and `fMultDiv2()` from the 64-bit `vpmuldq`
//!   products of the even and odd lanes; the shifted values stay below 2^30 in magnitude, so
//!   `stereoScaleFactor` (a sum of four energies) lies in `0..2^30`;
//! - `schur_div(num, denum, 16)` of the bands that take the division, which is
//!   `((INT64)num << 31) / denum` on the x86 variants: there `0 <= num < 2^30 <= denum`, so the
//!   quotient is below 2^31. It is computed in double precision (`num * 2^31` and `denum` are
//!   exact, the correctly rounded quotient is within `2^31 * 2^-53` of the exact one) and
//!   truncated, then corrected by one where the exact 64-bit remainder `num * 2^31 - t * denum`
//!   is negative or not below `denum`;
//! - `sqrtFixp()`: `x86` as `vcvtdq2ps`, `vsqrtps`, `vmulps`, `vcvttps2dq` (the roundings of the
//!   scalar single-precision instructions); `x86_32` (the x87 expression) in double precision
//!   with the bound of `fdk_aac_rs_fdk::x87::fast` (a lane whose result is within 2^-20 of an
//!   integer is not decided);
//! - the odd-shift correction and the final shifts as selects and register shifts modulo 32.
//!
//! A group with an input of `MINVAL_DBL`, a division outside `0 <= num < denum` or an undecided
//! square root takes the scalar code, as do the last `71 % 8` bands.

use super::downmix_band;
#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;
use fdk_aac_rs_fdk::common_fix::{FixpDbl, MAXVAL_DBL};
use fdk_aac_rs_fdk::{Arch, ARCH};

/// `46340.9492f` of `sqrtFixp()` (`x86/fixpoint_math_x86.h`).
const SQRT_FIXP_SCALE: f32 = 46340.9492;
/// The fraction band where the `x86_32` square root is not decided by the `f64` result (the
/// bound of `fdk_aac_rs_fdk::x87::fast::try_sqrt_fixp`).
const SQRT_FIXP_TOL: f64 = 1.0 / (1u32 << 20) as f64;

/// [`super::downmix_bands`] with AVX2.
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn downmix_bands(
    rows: [&[FixpDbl; 71]; 4],
    out_re: &mut [FixpDbl; 71],
    out_im: &mut [FixpDbl; 71],
    dyn_qmf_scale: i32,
) {
    let [lr, li, rr, ri] = rows;
    for g in 0..71 / 8 {
        let k = 8 * g;
        // the loads and stores access bands k..k + 8 <= 64 of the 71-entry rows
        match group([ld(lr, k), ld(li, k), ld(rr, k), ld(ri, k)], dyn_qmf_scale) {
            Some((re, im)) => {
                _mm256_storeu_si256(out_re.as_mut_ptr().add(k) as *mut __m256i, re);
                _mm256_storeu_si256(out_im.as_mut_ptr().add(k) as *mut __m256i, im);
            }
            None => {
                for j in k..k + 8 {
                    (out_re[j], out_im[j]) =
                        downmix_band(lr[j], li[j], rr[j], ri[j], dyn_qmf_scale);
                }
            }
        }
    }
    for j in 71 / 8 * 8..71 {
        (out_re[j], out_im[j]) = downmix_band(lr[j], li[j], rr[j], ri[j], dyn_qmf_scale);
    }
}

/// Bands `k..k + 8` of a row (`k + 8 <= 71`).
#[inline(always)]
unsafe fn ld(r: &[FixpDbl; 71], k: usize) -> __m256i {
    debug_assert!(k + 8 <= 71);
    _mm256_loadu_si256(r.as_ptr().add(k) as *const __m256i)
}

/// `fMult(a, b)` of the x86 variants: `((a * b) >> 32) << 1`.
#[inline(always)]
unsafe fn f_mult(a: __m256i, b: __m256i) -> __m256i {
    _mm256_slli_epi32::<1>(mul_hi(a, b))
}

/// The leading zeros of every lane `0 <= a < 2^31` (32 for 0).
#[inline(always)]
unsafe fn clz31(a: __m256i) -> __m256i {
    let y = _mm256_andnot_si256(_mm256_srli_epi32::<1>(a), a);
    let e = _mm256_srli_epi32::<23>(_mm256_castps_si256(_mm256_cvtepi32_ps(y)));
    _mm256_min_epi32(_mm256_sub_epi32(_mm256_set1_epi32(158), e), _mm256_set1_epi32(32))
}

/// `(a * b) >> 32` of every lane.
#[inline(always)]
unsafe fn mul_hi(a: __m256i, b: __m256i) -> __m256i {
    let even = _mm256_srli_epi64::<32>(_mm256_mul_epi32(a, b));
    let odd = _mm256_mul_epi32(_mm256_srli_epi64::<32>(a), _mm256_srli_epi64::<32>(b));
    _mm256_blend_epi32::<0b1010_1010>(even, odd)
}

/// Whether any lane of a mask is set.
#[inline(always)]
unsafe fn any(m: __m256i) -> bool {
    _mm256_movemask_ps(_mm256_castsi256_ps(m)) != 0
}

/// `((INT64)num << 31) / denum` of four lanes with `0 <= num < denum`.
#[inline(always)]
unsafe fn div4(num: __m128i, den: __m128i) -> __m128i {
    let q = _mm256_div_pd(
        _mm256_mul_pd(_mm256_cvtepi32_pd(num), _mm256_set1_pd(2147483648.0)),
        _mm256_cvtepi32_pd(den),
    );
    let t = _mm256_cvtepi32_epi64(_mm256_cvttpd_epi32(q));
    let d = _mm256_cvtepi32_epi64(den);
    let n = _mm256_slli_epi64::<31>(_mm256_cvtepi32_epi64(num));
    let r = _mm256_sub_epi64(n, _mm256_mul_epu32(t, d));
    let below = _mm256_cmpgt_epi64(_mm256_setzero_si256(), r);
    let above = _mm256_cmpgt_epi64(r, _mm256_sub_epi64(d, _mm256_set1_epi64x(1)));
    // t - 1 where the remainder is negative, t + 1 where it is not below denum (masks are -1)
    let t = _mm256_add_epi64(_mm256_sub_epi64(t, above), below);
    let t = _mm256_permutevar8x32_epi32(t, _mm256_setr_epi32(0, 2, 4, 6, 0, 2, 4, 6));
    _mm256_castsi256_si128(t)
}

/// The x87 `sqrtFixp()` of four values `0 <= op < 2^31` in double precision, and a mask of the
/// lanes the bound does not decide.
#[inline(always)]
unsafe fn sqrt_fixp4_x87(op: __m128i) -> (__m128i, __m128i) {
    let y = _mm256_mul_pd(
        _mm256_sqrt_pd(_mm256_cvtepi32_pd(op)),
        _mm256_set1_pd(SQRT_FIXP_SCALE as f64),
    );
    let t = _mm256_cvttpd_epi32(y);
    let frac = _mm256_sub_pd(y, _mm256_cvtepi32_pd(t));
    let decided = _mm256_and_pd(
        _mm256_cmp_pd::<_CMP_GT_OQ>(frac, _mm256_set1_pd(SQRT_FIXP_TOL)),
        _mm256_cmp_pd::<_CMP_LT_OQ>(frac, _mm256_set1_pd(1.0 - SQRT_FIXP_TOL)),
    );
    // the four 64-bit masks as 32-bit lanes
    let d = _mm256_castsi256_si128(_mm256_permutevar8x32_epi32(
        _mm256_castpd_si256(decided),
        _mm256_setr_epi32(0, 2, 4, 6, 0, 2, 4, 6),
    ));
    // 0 is exact
    let ok = _mm_or_si128(d, _mm_cmpeq_epi32(op, _mm_setzero_si128()));
    (t, _mm_andnot_si128(ok, _mm_set1_epi32(-1)))
}

/// `sqrtFixp()` of eight values `0 <= op < 2^31` of the variant, and a mask of the lanes it does
/// not decide (`x86_32` only).
#[inline(always)]
unsafe fn sqrt_fixp8(op: __m256i) -> (__m256i, __m256i) {
    if matches!(ARCH, Arch::X86_32) {
        let (t0, u0) = sqrt_fixp4_x87(_mm256_castsi256_si128(op));
        let (t1, u1) = sqrt_fixp4_x87(_mm256_extracti128_si256::<1>(op));
        (_mm256_set_m128i(t1, t0), _mm256_set_m128i(u1, u0))
    } else {
        let r = _mm256_cvttps_epi32(_mm256_mul_ps(
            _mm256_sqrt_ps(_mm256_cvtepi32_ps(op)),
            _mm256_set1_ps(SQRT_FIXP_SCALE),
        ));
        (r, _mm256_setzero_si256())
    }
}

/// Eight bands, or `None` if a band needs the scalar code.
#[inline(always)]
unsafe fn group(x: [__m256i; 4], dyn_qmf_scale: i32) -> Option<(__m256i, __m256i)> {
    let minval = _mm256_set1_epi32(i32::MIN);
    let mut bad = _mm256_setzero_si256();
    for v in x {
        bad = _mm256_or_si256(bad, _mm256_cmpeq_epi32(v, minval));
    }
    if any(bad) {
        return None;
    }
    // sc = max(0, clz(umax of |x|) - 2); a maximum of 2^30 or more (not normalized below 2^30)
    // takes the scalar code
    let m = _mm256_max_epu32(
        _mm256_max_epu32(_mm256_abs_epi32(x[0]), _mm256_abs_epi32(x[1])),
        _mm256_max_epu32(_mm256_abs_epi32(x[2]), _mm256_abs_epi32(x[3])),
    );
    if any(_mm256_cmpgt_epi32(m, _mm256_set1_epi32(0x3FFF_FFFF))) {
        return None;
    }
    let sc =
        _mm256_max_epi32(_mm256_sub_epi32(clz31(m), _mm256_set1_epi32(2)), _mm256_setzero_si256());
    let lr = _mm256_sllv_epi32(x[0], sc);
    let li = _mm256_sllv_epi32(x[1], sc);
    let rr = _mm256_sllv_epi32(x[2], sc);
    let ri = _mm256_sllv_epi32(x[3], sc);
    let dyn_scale = _mm256_min_epi32(
        _mm256_sub_epi32(sc, _mm256_set1_epi32(dyn_qmf_scale)),
        _mm256_set1_epi32(31),
    );
    let mask31 = _mm256_set1_epi32(31);

    let ssf = _mm256_add_epi32(
        _mm256_add_epi32(mul_hi(lr, lr), mul_hi(li, li)),
        _mm256_add_epi32(mul_hi(rr, rr), mul_hi(ri, ri)),
    );
    let tsf =
        _mm256_abs_epi32(_mm256_add_epi32(_mm256_add_epi32(ssf, f_mult(lr, rr)), f_mult(li, ri)));
    let take_div =
        _mm256_cmpgt_epi32(f_mult(_mm256_set1_epi32(MAXVAL_DBL), tsf), _mm256_srai_epi32::<1>(ssf));

    let ssf_final = if any(take_div) {
        // sc_num = CountLeadingBits(ssf) (0 <= ssf < 2^30), sc_denum = clz(tsf) - 1 (0 < tsf <
        // 2^31 in the lanes that take the division; 0 for 0)
        let zero = _mm256_setzero_si256();
        let one = _mm256_set1_epi32(1);
        let sc_num =
            _mm256_andnot_si256(_mm256_cmpeq_epi32(ssf, zero), _mm256_sub_epi32(clz31(ssf), one));
        let tsf_pos = _mm256_max_epi32(tsf, zero);
        let sc_denum = _mm256_andnot_si256(
            _mm256_cmpeq_epi32(tsf_pos, zero),
            _mm256_sub_epi32(clz31(tsf_pos), one),
        );
        let mut sc = _mm256_sub_epi32(sc_denum, sc_num);
        // num = (ssf << sc_num) >> 1, denum = tsf << sc_denum
        let num = _mm256_srai_epi32::<1>(_mm256_sllv_epi32(ssf, _mm256_and_si256(sc_num, mask31)));
        let den = _mm256_sllv_epi32(tsf, _mm256_and_si256(sc_denum, mask31));
        let in_domain =
            _mm256_andnot_si256(_mm256_cmpgt_epi32(zero, num), _mm256_cmpgt_epi32(den, num));
        if any(_mm256_andnot_si256(in_domain, take_div)) {
            return None;
        }
        // lanes outside the division get num 0, denum 1 (their result is discarded)
        let num = _mm256_and_si256(num, take_div);
        let den = _mm256_blendv_epi8(one, den, take_div);
        let mut quot = _mm256_set_m128i(
            div4(_mm256_extracti128_si256::<1>(num), _mm256_extracti128_si256::<1>(den)),
            div4(_mm256_castsi256_si128(num), _mm256_castsi256_si128(den)),
        );
        // prevent odd scaling for next sqrt calculation
        let odd = _mm256_cmpeq_epi32(_mm256_and_si256(sc, one), one);
        sc = _mm256_sub_epi32(sc, odd); // sc += 1 where odd
        quot = _mm256_blendv_epi8(quot, _mm256_srai_epi32::<1>(quot), odd);
        let (root, undecided) = sqrt_fixp8(quot);
        if any(_mm256_and_si256(undecided, take_div)) {
            return None;
        }
        // stereoScaleFactor <<= (sc >> 1) (modulo 32)
        let root = _mm256_sllv_epi32(root, _mm256_and_si256(_mm256_srai_epi32::<1>(sc), mask31));
        _mm256_blendv_epi8(_mm256_set1_epi32(MAXVAL_DBL), root, take_div)
    } else {
        _mm256_set1_epi32(MAXVAL_DBL)
    };

    // fMultDiv2(stereoScaleFactor, l + r) >> dynScale (arithmetic, modulo 32)
    let shift = _mm256_and_si256(dyn_scale, mask31);
    let re = _mm256_srav_epi32(mul_hi(ssf_final, _mm256_add_epi32(lr, rr)), shift);
    let im = _mm256_srav_epi32(mul_hi(ssf_final, _mm256_add_epi32(li, ri)), shift);
    Some((re, im))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `div4` against the 64-bit division on random operands `0 <= num < 2^30 <= denum < 2^31`
    /// and on operands whose quotient lies within 2^-22 of an integer (`num = r / 2^31 mod
    /// denum` for odd `denum` and small `r` or `denum - r`), where the double-precision quotient
    /// needs the correction.
    #[test]
    fn div4_vs_division() {
        if !crate::simd::avx2() {
            return;
        }
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 32) as u32
        };
        // (2^31)^-1 mod d by the extended Euclidean algorithm
        let inv = |d: i64| -> i64 {
            let (mut r0, mut r1, mut s0, mut s1) = (d as i128, (1i128 << 31) % d as i128, 0, 1);
            while r1 != 0 {
                let q = r0 / r1;
                (r0, r1) = (r1, r0 - q * r1);
                (s0, s1) = (s1, s0 - q * s1);
            }
            s0.rem_euclid(d as i128) as i64
        };
        for it in 0..400_000 {
            let mut n = [0i32; 4];
            let mut d = [0i32; 4];
            for k in 0..4 {
                d[k] = (0x4000_0000 | (next() & 0x3FFF_FFFF)) as i32;
                n[k] = if it % 2 == 0 {
                    (next() & 0x3FFF_FFFF) as i32
                } else {
                    d[k] |= 1;
                    let dd = d[k] as i64;
                    let r = (next() % 300) as i64;
                    let r = if next() % 2 == 0 { r } else { dd - 1 - r };
                    ((r as i128 * inv(dd) as i128) % dd as i128) as i32
                };
                if n[k] >= d[k] {
                    n[k] = d[k] - 1;
                }
            }
            // SAFETY: the CPU supports AVX2 (checked above).
            let q = unsafe { div4_test(&n, &d) };
            for k in 0..4 {
                let e = (((n[k] as i64) << 31) / d[k] as i64) as i32;
                assert_eq!(q[k], e, "{} / {}", n[k], d[k]);
            }
        }
    }

    #[target_feature(enable = "avx2")]
    unsafe fn div4_test(n: &[i32; 4], d: &[i32; 4]) -> [i32; 4] {
        let r = div4(
            _mm_loadu_si128(n.as_ptr() as *const __m128i),
            _mm_loadu_si128(d.as_ptr() as *const __m128i),
        );
        let mut q = [0i32; 4];
        _mm_storeu_si128(q.as_mut_ptr() as *mut __m128i, r);
        q
    }
}
