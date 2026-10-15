// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized pre and post twiddling of libFDK/src/dct.cpp (dct_IV, dst_IV) for the x86 variants.

//! AVX2 blocks of eight iterations of the pre and post twiddling of `dct_IV` and `dst_IV` with
//! the arithmetic of the x86, x86_32 and generic variants (see `super` for the per-pair
//! formulation; the NEON blocks of `super::neon` are the same with the `aarch64` arithmetic).
//!
//! - `cplxMultDiv2` (pre twiddling, `FIXP_WTP` coefficients) subtracts or adds the individually
//!   truncated products `M(x, w) = fMultDiv2(x, w) = (x * w) >> 16`, computed exactly in 32-bit
//!   lanes as `(x >> 16) * w + (((x & 0xFFFF) * w) >> 16)` (no intermediate value exceeds 31
//!   bits for 16-bit `w`).
//! - `cplxMult` (post twiddling, `FIXP_SGL` coefficients) uses `fMult(x, w) = M(x, w) << 1`
//!   (the same on every variant).
//! - Additions, subtractions, negations and shifts wrap like the scalar code.

use crate::archdef::{FixpStp, FixpWtp};
#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// `(x * w) >> 16` per lane for 16-bit `w` (see the module documentation).
#[inline(always)]
unsafe fn mul_div2(x: __m256i, w: __m256i) -> __m256i {
    let hi = _mm256_mullo_epi32(_mm256_srai_epi32::<16>(x), w);
    let lo = _mm256_and_si256(x, _mm256_set1_epi32(0xFFFF));
    _mm256_add_epi32(hi, _mm256_srai_epi32::<16>(_mm256_mullo_epi32(lo, w)))
}

/// `fMult(x, w)`: `((x * w) >> 16) << 1`.
#[inline(always)]
unsafe fn mul(x: __m256i, w: __m256i) -> __m256i {
    _mm256_slli_epi32::<1>(mul_div2(x, w))
}

#[inline(always)]
unsafe fn neg(x: __m256i) -> __m256i {
    _mm256_sub_epi32(_mm256_setzero_si256(), x)
}

/// Reverses the order of the 8 lanes.
#[inline(always)]
unsafe fn rev(x: __m256i) -> __m256i {
    _mm256_permutevar8x32_epi32(x, _mm256_setr_epi32(7, 6, 5, 4, 3, 2, 1, 0))
}

/// Loads the interleaved complex values `k .. k + 8` as (re, im).
#[inline(always)]
unsafe fn load_up(p: *const i32, k: usize) -> (__m256i, __m256i) {
    let lo = _mm256_loadu_si256(p.add(2 * k) as *const __m256i);
    let hi = _mm256_loadu_si256(p.add(2 * k + 8) as *const __m256i);
    // (r0 r1 i0 i1 | r2 r3 i2 i3), (r4 r5 i4 i5 | r6 r7 i6 i7)
    let lo = _mm256_shuffle_epi32::<0b11_01_10_00>(lo);
    let hi = _mm256_shuffle_epi32::<0b11_01_10_00>(hi);
    // (r0 r1 r4 r5 | r2 r3 r6 r7)
    let re = _mm256_unpacklo_epi64(lo, hi);
    let im = _mm256_unpackhi_epi64(lo, hi);
    (_mm256_permute4x64_epi64::<0b11_01_10_00>(re), _mm256_permute4x64_epi64::<0b11_01_10_00>(im))
}

/// Loads the interleaved complex values `k, k - 1, ..., k - 7` as (re, im).
#[inline(always)]
unsafe fn load_down(p: *const i32, k: usize) -> (__m256i, __m256i) {
    let (re, im) = load_up(p, k - 7);
    (rev(re), rev(im))
}

/// Stores the interleaved complex values `k .. k + 8`.
#[inline(always)]
unsafe fn store_up(p: *mut i32, k: usize, re: __m256i, im: __m256i) {
    // (r0 r1 r4 r5 | r2 r3 r6 r7)
    let re = _mm256_permute4x64_epi64::<0b11_01_10_00>(re);
    let im = _mm256_permute4x64_epi64::<0b11_01_10_00>(im);
    _mm256_storeu_si256(p.add(2 * k) as *mut __m256i, _mm256_unpacklo_epi32(re, im));
    _mm256_storeu_si256(p.add(2 * k + 8) as *mut __m256i, _mm256_unpackhi_epi32(re, im));
}

/// Stores the interleaved complex values `k, k - 1, ..., k - 7`.
#[inline(always)]
unsafe fn store_down(p: *mut i32, k: usize, re: __m256i, im: __m256i) {
    store_up(p, k - 7, rev(re), rev(im));
}

/// `cplxMultDiv2(a_re, a_im, c, s)` of the x86 variants:
/// `(M(a_re, c) - M(a_im, s), M(a_re, s) + M(a_im, c))`.
#[inline(always)]
unsafe fn cmd2(ar: __m256i, ai: __m256i, c: __m256i, s: __m256i) -> (__m256i, __m256i) {
    (
        _mm256_sub_epi32(mul_div2(ar, c), mul_div2(ai, s)),
        _mm256_add_epi32(mul_div2(ar, s), mul_div2(ai, c)),
    )
}

/// `(re, im)` (sign-extended) of 32-bit words `re | im << 16`.
#[inline(always)]
unsafe fn split(v: __m256i) -> (__m256i, __m256i) {
    (_mm256_srai_epi32::<16>(_mm256_slli_epi32::<16>(v)), _mm256_srai_epi32::<16>(v))
}

/// The iterations `i = 2t`, `t = t0 .. t0 + 8` of the pre twiddling of `dct_IV` (`DST` false) or
/// `dst_IV` (`DST` true) on the `l = 2m` values at `x` (`t0 + 8 <= m / 2`), see
/// `super::dct_iv_pre_iter`.
///
/// # Safety
///
/// AVX2 must be available; `x` holds `l` values, `tw` at least `2 (t0 + 8)`.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn pre_block<const DST: bool>(
    x: *mut i32,
    m: usize,
    t0: usize,
    tw: *const FixpWtp,
) {
    unsafe {
        let (fr, fi) = load_up(x, t0); // x[i], x[i+1]
        let (br, bi) = load_down(x, m - 1 - t0); // x[l-2-i], x[l-1-i]
                                                 // twiddle[2t] (even words) and twiddle[2t + 1] (odd words)
        let w = tw as *const i32;
        let perm = _mm256_setr_epi32(0, 2, 4, 6, 1, 3, 5, 7);
        let a =
            _mm256_permutevar8x32_epi32(_mm256_loadu_si256(w.add(2 * t0) as *const __m256i), perm);
        let b = _mm256_permutevar8x32_epi32(
            _mm256_loadu_si256(w.add(2 * t0 + 8) as *const __m256i),
            perm,
        );
        let (c0, s0) = split(_mm256_permute2x128_si256::<0x20>(a, b));
        let (c1, s1) = split(_mm256_permute2x128_si256::<0x31>(a, b));
        if !DST {
            let (accu1, accu2) = cmd2(bi, fr, c0, s0);
            let (accu3, accu4) = cmd2(br, fi, c1, s1);
            store_up(x, t0, _mm256_srai_epi32::<1>(accu2), _mm256_srai_epi32::<1>(accu1));
            store_down(
                x,
                m - 1 - t0,
                _mm256_srai_epi32::<1>(accu4),
                neg(_mm256_srai_epi32::<1>(accu3)),
            );
        } else {
            let accu1 = _mm256_srai_epi32::<1>(bi);
            let accu2 = neg(_mm256_srai_epi32::<1>(fr));
            let accu3 = _mm256_srai_epi32::<1>(fi);
            let accu4 = neg(_mm256_srai_epi32::<1>(br));
            let (a1, a2) = cmd2(accu1, accu2, c0, s0);
            let (a3, a4) = cmd2(accu4, accu3, c1, s1);
            store_up(x, t0, a2, a1);
            store_down(x, m - 1 - t0, a4, neg(a3));
        }
    }
}

/// 4-lane (128-bit) forms of the blocks for the transforms of 32 and 64 values with the values
/// kept in registers between the pre twiddling, the FFT and the post twiddling (see
/// `super::dct_iv_dst_iv_x86_n`): the same operations as [`pre_block`] and [`post_block`] (and the
/// NEON blocks `super::neon::dct_iv_pre_regs`, `post_block_regs`, with the arithmetic of these
/// variants: individually truncated products `M(x, w)`).
pub(super) mod lane4 {
    use super::*;
    pub(in crate::dct) use crate::fft::x86_16::C;
    use crate::fft::x86_16::{ld_at, st_at};

    /// `(x * w) >> 16` per lane for 16-bit `w` (see the module documentation).
    #[inline(always)]
    unsafe fn mul_div2(x: __m128i, w: __m128i) -> __m128i {
        let hi = _mm_mullo_epi32(_mm_srai_epi32::<16>(x), w);
        let lo = _mm_and_si128(x, _mm_set1_epi32(0xFFFF));
        _mm_add_epi32(hi, _mm_srai_epi32::<16>(_mm_mullo_epi32(lo, w)))
    }

    /// `fMult(x, w)`: `((x * w) >> 16) << 1`.
    #[inline(always)]
    unsafe fn mul(x: __m128i, w: __m128i) -> __m128i {
        _mm_slli_epi32::<1>(mul_div2(x, w))
    }

    #[inline(always)]
    unsafe fn neg(x: __m128i) -> __m128i {
        _mm_sub_epi32(_mm_setzero_si128(), x)
    }

    /// The 4 lanes in reverse order.
    #[inline(always)]
    unsafe fn rev(x: __m128i) -> __m128i {
        _mm_shuffle_epi32::<0b00_01_10_11>(x)
    }

    /// `cplxMultDiv2(a_re, a_im, c, s)` of the x86 variants.
    #[inline(always)]
    unsafe fn cmd2(ar: __m128i, ai: __m128i, c: __m128i, s: __m128i) -> (__m128i, __m128i) {
        (
            _mm_sub_epi32(mul_div2(ar, c), mul_div2(ai, s)),
            _mm_add_epi32(mul_div2(ar, s), mul_div2(ai, c)),
        )
    }

    /// `(re, im)` (sign-extended) of 32-bit words `re | im << 16`.
    #[inline(always)]
    unsafe fn split(v: __m128i) -> (__m128i, __m128i) {
        (_mm_srai_epi32::<16>(_mm_slli_epi32::<16>(v)), _mm_srai_epi32::<16>(v))
    }

    /// The iterations `i = i0, i0 + 2, i0 + 4, i0 + 6` of the pre twiddling of `dct_IV` (`DST`
    /// false) or `dst_IV` (`DST` true) on the `l` values at `x` (`i0 + 6 < l / 2 - 1`), without
    /// the stores: the complex values `i0 / 2 .. i0 / 2 + 4` (front) and
    /// `l / 2 - 4 - i0 / 2 .. l / 2 - i0 / 2` (back) that the iterations write.
    ///
    /// # Safety
    ///
    /// SSE4.1 (implied by AVX2); `x` holds `l` values, `tw` at least `i0 + 8`.
    #[inline(always)]
    pub(in crate::dct) unsafe fn pre_regs<const DST: bool>(
        x: *const i32,
        l: usize,
        i0: usize,
        tw: *const FixpWtp,
    ) -> (C, C) {
        let mu = l / 2;
        let t0 = i0 / 2;
        let f = ld_at(x, t0); // x[i], x[i+1]
        let b = ld_at(x, mu - 4 - t0);
        let (br, bi) = (rev(b.0), rev(b.1)); // x[l-2-i], x[l-1-i]
                                             // twiddle[i] (even words) and twiddle[i + 1] (odd words)
        let w = tw as *const f32;
        let w0 = _mm_loadu_ps(w.add(i0));
        let w1 = _mm_loadu_ps(w.add(i0 + 4));
        let (c0, s0) = split(_mm_castps_si128(_mm_shuffle_ps::<0b10_00_10_00>(w0, w1)));
        let (c1, s1) = split(_mm_castps_si128(_mm_shuffle_ps::<0b11_01_11_01>(w0, w1)));
        if !DST {
            let (accu1, accu2) = cmd2(bi, f.0, c0, s0);
            let (accu3, accu4) = cmd2(br, f.1, c1, s1);
            (
                C(_mm_srai_epi32::<1>(accu2), _mm_srai_epi32::<1>(accu1)),
                C(rev(_mm_srai_epi32::<1>(accu4)), rev(neg(_mm_srai_epi32::<1>(accu3)))),
            )
        } else {
            let accu1 = _mm_srai_epi32::<1>(bi);
            let accu2 = neg(_mm_srai_epi32::<1>(f.0));
            let accu3 = _mm_srai_epi32::<1>(f.1);
            let accu4 = neg(_mm_srai_epi32::<1>(br));
            let (a1, a2) = cmd2(accu1, accu2, c0, s0);
            let (a3, a4) = cmd2(accu4, accu3, c1, s1);
            (C(a2, a1), C(rev(a4), rev(neg(a3))))
        }
    }

    /// The pairs `j = j0 .. j0 + 4` of the post twiddling of `dct_IV` (`DST` false) or `dst_IV`
    /// (`DST` true) of `l` values, from the complex values `j0 .. j0 + 4` (`fv`) and
    /// `l / 2 - 4 - j0 .. l / 2 - j0` (`bv`), stored to `x`; `FIRST` (`j0 = 0`) and `LAST`
    /// (`j0 = K - 4`, `K = l / 4`) include the special pairs as `super::neon::post_block_regs`
    /// does (the copies of pair 0, `fMult(B, W_PiFOURTH)` for pair `K - 1`).
    ///
    /// # Safety
    ///
    /// SSE4.1 (implied by AVX2); `x` holds `l` values, `t` the twiddles `j * step` for
    /// `j <= j0 + 4` (`j0 + 3` for `LAST`).
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub(in crate::dct) unsafe fn post_regs<const DST: bool, const FIRST: bool, const LAST: bool>(
        fv: C,
        bv: C,
        x: *mut i32,
        l: usize,
        j0: usize,
        t: *const FixpStp,
        step: usize,
    ) {
        let mu = l / 2;
        let (fe, fo) = (fv.0, fv.1);
        let (be, bo) = (rev(bv.0), rev(bv.1));
        // the twiddles as 32-bit words `re | im << 16` (the table is 2-byte aligned)
        let tw = |j: usize| (t.add(j * step) as *const i32).read_unaligned();
        let v = _mm_setr_epi32(tw(j0), tw(j0 + 1), tw(j0 + 2), tw(j0 + 3));
        let w = tw(j0 + if LAST { 3 } else { 4 });
        let v1 = _mm_alignr_epi8::<4>(_mm_set1_epi32(w), v);
        let ((c0, s0), (c1, s1)) = (split(v), split(v1));
        // cplxMult(a_re, a_im, c, s) = (fMult(a_re, c) - fMult(a_im, s), fMult(a_re, s) + fMult(a_im, c))
        let a3 = _mm_sub_epi32(mul(fo, c0), mul(fe, s0));
        let a4 = _mm_add_epi32(mul(fo, s0), mul(fe, c0));
        let b3 = _mm_sub_epi32(mul(be, c1), mul(bo, s1));
        let b4 = _mm_add_epi32(mul(be, s1), mul(bo, c1));
        // outputs: front even / odd, back even / odd
        let (mut oe_f, mut oo_f, mut oe_b, mut oo_b) =
            if DST { (a3, neg(b4), neg(b3), neg(a4)) } else { (a4, b3, b4, neg(a3)) };
        if FIRST {
            let m = _mm_setr_epi32(-1, 0, 0, 0);
            let (ce, co) = if DST { (fo, neg(fe)) } else { (fe, neg(fo)) };
            oe_f = _mm_blendv_epi8(oe_f, ce, m);
            oo_b = _mm_blendv_epi8(oo_b, co, m);
        }
        if LAST {
            let m = _mm_setr_epi32(0, 0, 0, -1);
            let wp = _mm_set1_epi32(super::super::W_PI_FOURTH as i32);
            let (a1, a2) = (mul(be, wp), mul(bo, wp));
            let (ce, co) = if DST {
                (_mm_sub_epi32(a2, a1), _mm_sub_epi32(neg(a1), a2))
            } else {
                (_mm_add_epi32(a1, a2), _mm_sub_epi32(a1, a2))
            };
            oe_b = _mm_blendv_epi8(oe_b, ce, m);
            oo_f = _mm_blendv_epi8(oo_f, co, m);
        }
        st_at(x, j0, C(oe_f, oo_f));
        st_at(x, mu - 4 - j0, C(rev(oe_b), rev(oo_b)));
    }
}

/// The pairs `j = j0 .. j0 + 8` of the post twiddling of `dct_IV` (`DST` false) or `dst_IV`
/// (`DST` true) on the `l = 2m` values at `x` (`1 <= j0`, `j0 + 7 <= K - 2` with
/// `K = (m + 1) / 2`), see `super::dct_iv_post_pair`.
///
/// # Safety
///
/// AVX2 must be available; `x` holds `l` values, `t` at least `(j0 + 8) * step + 1`.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn post_block<const DST: bool>(
    x: *mut i32,
    m: usize,
    j0: usize,
    t: *const FixpStp,
    step: usize,
) {
    unsafe {
        let (fe, fo) = load_up(x, j0); // x[2j], x[2j+1]
        let (be, bo) = load_down(x, m - 1 - j0); // x[l-2-2j], x[l-1-2j]
        let st = step as i32;
        let idx = _mm256_mullo_epi32(
            _mm256_add_epi32(
                _mm256_set1_epi32(j0 as i32),
                _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7),
            ),
            _mm256_set1_epi32(st),
        );
        let tw = t as *const i32;
        let (c0, s0) = split(_mm256_i32gather_epi32::<4>(tw, idx));
        let (c1, s1) =
            split(_mm256_i32gather_epi32::<4>(tw, _mm256_add_epi32(idx, _mm256_set1_epi32(st))));
        // cplxMult(a_re, a_im, c, s) = (fMult(a_re, c) - fMult(a_im, s), fMult(a_re, s) + fMult(a_im, c))
        let a3 = _mm256_sub_epi32(mul(fo, c0), mul(fe, s0));
        let a4 = _mm256_add_epi32(mul(fo, s0), mul(fe, c0));
        let b3 = _mm256_sub_epi32(mul(be, c1), mul(bo, s1));
        let b4 = _mm256_add_epi32(mul(be, s1), mul(bo, c1));
        if !DST {
            store_up(x, j0, a4, b3);
            store_down(x, m - 1 - j0, b4, neg(a3));
        } else {
            store_up(x, j0, a3, neg(b4));
            store_down(x, m - 1 - j0, neg(b3), neg(a4));
        }
    }
}
