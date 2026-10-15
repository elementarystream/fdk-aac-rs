// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized loops of libFDK/src/dct.cpp (dct_II, dct_III) for the x86 variants.

//! SSE2 blocks of four iterations of the loops of `dct_II` and `dct_III` with the arithmetic of
//! the x86, x86_32 and generic variants (SSE2 is part of the x86-64 and i686 targets, so no
//! run-time selection is needed). See `super::neon` for the `aarch64` blocks of the same loops;
//! the iterations are independent of each other (they read one buffer and write the other).
//!
//! - `cplxMultDiv2` (`FIXP_SGL` coefficients) subtracts or adds the individually truncated
//!   products `M(x, w) = (x * w) >> 16` (`crate::fft::x86::mul_div2_v`, exact, with a per-lane
//!   `w`).
//! - `cplxMult` uses `fMult(x, w) = M(x, w) << 1` (the same on every variant).
//! - The second branch of the loops, `(-r', x') = cplxMultDiv2(q, p, c, s)` with the real part
//!   negated, is the first, `(r, x) = cplxMultDiv2(p, q, c, s)`, with `c` and `s` swapped:
//!   `M(p, s) - M(q, c) = -(M(q, c) - M(p, s))` and `M(p, c) + M(q, s)` (wrapping), so the lanes
//!   from `split` on take swapped coefficients.
//! - Additions, subtractions, negations and shifts wrap like the scalar code.

use crate::archdef::FixpStp;
use crate::fft::x86::mul_div2_v as m;
pub(super) use crate::fft::x86_16::C;
#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// The 4 lanes in reverse order.
#[inline(always)]
unsafe fn rev(v: __m128i) -> __m128i {
    _mm_shuffle_epi32::<0b00_01_10_11>(v)
}

#[inline(always)]
unsafe fn neg(x: __m128i) -> __m128i {
    _mm_sub_epi32(_mm_setzero_si128(), x)
}

/// Loads the 4 interleaved complex values at `p` as (re, im).
#[inline(always)]
unsafe fn load2(p: *const i32) -> (__m128i, __m128i) {
    let lo = _mm_shuffle_epi32::<0b11_01_10_00>(_mm_loadu_si128(p as *const __m128i));
    let hi = _mm_shuffle_epi32::<0b11_01_10_00>(_mm_loadu_si128(p.add(4) as *const __m128i));
    (_mm_unpacklo_epi64(lo, hi), _mm_unpackhi_epi64(lo, hi))
}

/// Stores (re, im) as the 4 interleaved complex values at `p`.
#[inline(always)]
unsafe fn store2(p: *mut i32, re: __m128i, im: __m128i) {
    _mm_storeu_si128(p as *mut __m128i, _mm_unpacklo_epi32(re, im));
    _mm_storeu_si128(p.add(4) as *mut __m128i, _mm_unpackhi_epi32(re, im));
}

#[inline(always)]
unsafe fn ld(p: *const i32) -> __m128i {
    _mm_loadu_si128(p as *const __m128i)
}

#[inline(always)]
unsafe fn st(p: *mut i32, v: __m128i) {
    _mm_storeu_si128(p as *mut __m128i, v)
}

/// A 16-bit coefficient per lane in the form of `mul_div2_v`: `(w & 0xFFFF, w << 16)`.
type Co = (__m128i, __m128i);

/// The coefficients `(re, im)` of 4 words `re | im << 16`.
#[inline(always)]
unsafe fn split_co(v: __m128i) -> (Co, Co) {
    let hi16 = _mm_set1_epi32(0xFFFF_0000u32 as i32);
    let re = (_mm_srli_epi32::<16>(_mm_slli_epi32::<16>(v)), _mm_slli_epi32::<16>(v));
    let im = (_mm_srli_epi32::<16>(v), _mm_and_si128(v, hi16));
    (re, im)
}

/// `cplxMultDiv2(a_re, a_im, c, s)`: `(M(a_re, c) - M(a_im, s), M(a_re, s) + M(a_im, c))`.
#[inline(always)]
unsafe fn cmd2(ar: __m128i, ai: __m128i, c: Co, s: Co) -> (__m128i, __m128i) {
    (
        _mm_sub_epi32(m(ar, c.0, c.1), m(ai, s.0, s.1)),
        _mm_add_epi32(m(ar, s.0, s.1), m(ai, c.0, c.1)),
    )
}

/// `cplxMult(a_re, a_im, c, s)` with `fMult(x, w) = M(x, w) << 1`.
#[inline(always)]
unsafe fn cm(ar: __m128i, ai: __m128i, c: Co, s: Co) -> (__m128i, __m128i) {
    let f = |x: __m128i, w: Co| _mm_slli_epi32::<1>(m(x, w.0, w.1));
    (_mm_sub_epi32(f(ar, c), f(ai, s)), _mm_add_epi32(f(ar, s), f(ai, c)))
}

/// The twiddle words `re | im << 16` of the iterations `i` of the loops of `dct_III` and
/// `dct_II` (`half = m / 2` words each, index 0 unused): `t[i * inc]`, then `t[(m - i) * inc]`,
/// then `t[index(i)]` (see `super::dct_twiddle_index`) with `re` and `im` swapped from `split`
/// on (see the module documentation). Built once per configuration (up to 8, lazily, checked
/// against the parameters); `None` if the cache is full.
pub(super) fn twiddle_table(
    t: &'static [FixpStp],
    m: usize,
    inc: usize,
    split: usize,
) -> Option<&'static [i32]> {
    use std::sync::OnceLock;
    type Key = (usize, usize, usize, usize, usize);
    static CACHE: [OnceLock<(Key, Box<[i32]>)>; 8] = [const { OnceLock::new() }; 8];
    let half = m / 2;
    let key: Key = (t.as_ptr() as usize, t.len(), m, inc, split);
    let build = || {
        let w = |k: usize| (t[k].re as u16 as u32 | (t[k].im as u16 as u32) << 16) as i32;
        let mut v = vec![0i32; 3 * half];
        for i in 1..half {
            v[i] = w(i * inc);
            v[half + i] = w((m - i) * inc);
            let w3 = w(super::dct_twiddle_index(m, inc, i)) as u32;
            v[2 * half + i] = if i >= split { w3.rotate_left(16) } else { w3 } as i32;
        }
        v.into_boxed_slice()
    };
    for c in &CACHE {
        let (k, v) = c.get_or_init(|| (key, build()));
        if *k == key {
            return Some(v);
        }
    }
    None
}

/// The coefficients of the iterations `i = i0 .. i0 + 4` from the [`twiddle_table`] `tab`.
#[inline(always)]
unsafe fn twiddles(tab: *const i32, half: usize, i0: usize) -> ((Co, Co), (Co, Co), (Co, Co)) {
    let ld = |o: usize| _mm_loadu_si128(tab.add(o + i0) as *const __m128i);
    (split_co(ld(0)), split_co(ld(half)), split_co(ld(2 * half)))
}

/// The iterations `i = i0 .. i0 + 4` of the loop of `dct_III` (`1 <= i0`, `i0 + 3 < m / 2`;
/// coefficients from the [`twiddle_table`] `tab`), reading `x` (`l = 2 m` values) and writing
/// `tmp[2i]`, `tmp[2i+1]`, `tmp[2(m-i)]`, `tmp[2(m-i)+1]`. See `super::dct_iii_iter`.
///
/// # Safety
///
/// `x` and `tmp` hold `l` values; `tab` is the [`twiddle_table`] of the configuration.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(super) unsafe fn dct_iii_block(
    x: *const i32,
    tmp: *mut i32,
    l: usize,
    m: usize,
    i0: usize,
    tab: *const i32,
) {
    let (f, b) = dct_iii_block_regs(x, l, m, i0, tab);
    store2(tmp.add(2 * i0), f.0, f.1);
    store2(tmp.add(2 * (m - i0 - 3)), b.0, b.1);
}

/// [`dct_iii_block`] without the stores: the complex values `i0 .. i0 + 4` (front) and
/// `m - i0 - 3 .. m - i0 + 1` (back) of `tmp` that it writes, as complex vectors.
///
/// # Safety
///
/// As for [`dct_iii_block`].
#[inline(always)]
pub(super) unsafe fn dct_iii_block_regs(
    x: *const i32,
    l: usize,
    m: usize,
    i0: usize,
    tab: *const i32,
) -> (C, C) {
    let xi = ld(x.add(i0));
    let xli = rev(ld(x.add(l - i0 - 3)));
    let xmi = ld(x.add(m + i0));
    let xm_i = rev(ld(x.add(m - i0 - 3)));
    let ((c1, s1), (c2, s2), (c3, s3)) = twiddles(tab, m / 2, i0);
    let (accu2, accu1) = cmd2(xli, xi, c1, s1);
    let (accu4, accu3) = cmd2(xmi, xm_i, c2, s2);
    let (accu3, accu4) = (_mm_srai_epi32::<1>(accu3), _mm_srai_epi32::<1>(accu4));
    let p = _mm_sub_epi32(accu3, _mm_srai_epi32::<1>(accu1));
    let q = _mm_add_epi32(_mm_srai_epi32::<1>(accu2), accu4);
    let (accu6, accu5) = cmd2(p, q, c3, s3);
    let xr = _mm_srai_epi32::<1>(_mm_add_epi32(_mm_srai_epi32::<1>(accu1), accu3));
    let xr2 = _mm_srai_epi32::<1>(_mm_sub_epi32(_mm_srai_epi32::<1>(accu2), accu4));
    (
        C(_mm_sub_epi32(xr, accu5), _mm_sub_epi32(xr2, accu6)),
        C(rev(_mm_add_epi32(xr, accu5)), rev(neg(_mm_add_epi32(xr2, accu6)))),
    )
}

/// The output permutation of `dct_III` for `k = k0 .. k0 + 4`: `x[4k] = tmp[2k]`,
/// `x[4k+1] = tmp[l-1-2k]`, `x[4k+2] = tmp[2k+1]`, `x[4k+3] = tmp[l-2-2k]`.
///
/// # Safety
///
/// `x` and `tmp` hold `l` values, `k0 + 4 <= l / 4`.
#[inline(always)]
pub(super) unsafe fn dct_iii_post_block(x: *mut i32, tmp: *const i32, l: usize, k0: usize) {
    let (fe, fo) = load2(tmp.add(2 * k0));
    let (be, bo) = load2(tmp.add(l - 8 - 2 * k0));
    dct_iii_post_regs(x, k0, C(fe, fo), C(be, bo));
}

/// [`dct_iii_post_block`] from the complex values `k0 .. k0 + 4` (`f`) and
/// `l / 2 - 4 - k0 .. l / 2 - k0` (`b`) of `tmp`.
///
/// # Safety
///
/// `x` holds `4 (k0 + 4)` values.
#[inline(always)]
pub(super) unsafe fn dct_iii_post_regs(x: *mut i32, k0: usize, f: C, b: C) {
    let (fe, fo) = (f.0, f.1);
    let (b1, b0) = (rev(b.1), rev(b.0)); // tmp[l-1-2k], tmp[l-2-2k]
                                         // the rows k of (fe, b1, fo, b0), transposed
    let t0 = _mm_unpacklo_epi32(fe, b1);
    let t1 = _mm_unpacklo_epi32(fo, b0);
    let t2 = _mm_unpackhi_epi32(fe, b1);
    let t3 = _mm_unpackhi_epi32(fo, b0);
    st(x.add(4 * k0), _mm_unpacklo_epi64(t0, t1));
    st(x.add(4 * k0 + 4), _mm_unpackhi_epi64(t0, t1));
    st(x.add(4 * k0 + 8), _mm_unpacklo_epi64(t2, t3));
    st(x.add(4 * k0 + 12), _mm_unpackhi_epi64(t2, t3));
}

/// The input permutation of `dct_II` for `i = i0 .. i0 + 4`: `tmp[i] = x[2i] >> 2`,
/// `tmp[l-1-i] = x[2i+1] >> 2`.
///
/// # Safety
///
/// `x` and `tmp` hold `l` values, `i0 + 4 <= l / 2`.
#[inline(always)]
pub(super) unsafe fn dct_ii_pre_block(x: *const i32, tmp: *mut i32, l: usize, i0: usize) {
    let (e, o) = load2(x.add(2 * i0));
    st(tmp.add(i0), _mm_srai_epi32::<2>(e));
    st(tmp.add(l - 4 - i0), rev(_mm_srai_epi32::<2>(o)));
}

/// The iterations `i = i0 .. i0 + 4` of the loop of `dct_II` after the FFT (conditions as for
/// [`dct_iii_block`]), reading `tmp` and writing `x[i]`, `x[l-i]`, `x[m+i]`, `x[m-i]`. See
/// `super::dct_ii_iter`.
///
/// # Safety
///
/// As for [`dct_iii_block`].
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(super) unsafe fn dct_ii_block(
    x: *mut i32,
    tmp: *const i32,
    l: usize,
    m: usize,
    i0: usize,
    tab: *const i32,
) {
    let (fe, fo) = load2(tmp.add(2 * i0)); // tmp[2i], tmp[2i+1]
    let (be, bo) = load2(tmp.add(2 * (m - i0 - 3)));
    dct_ii_block_regs(C(fe, fo), C(be, bo), x, l, m, i0, tab);
}

/// [`dct_ii_block`] from the complex values `i0 .. i0 + 4` (`f`) and `m - i0 - 3 .. m - i0 + 1`
/// (`b`) of `tmp`.
///
/// # Safety
///
/// As for [`dct_iii_block`].
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(super) unsafe fn dct_ii_block_regs(
    f: C,
    b: C,
    x: *mut i32,
    l: usize,
    m: usize,
    i0: usize,
    tab: *const i32,
) {
    let (fe, fo) = (f.0, f.1);
    let (be, bo) = (rev(b.0), rev(b.1)); // tmp[2(m-i)], tmp[2(m-i)+1]
    let (fe, fo) = (_mm_srai_epi32::<1>(fe), _mm_srai_epi32::<1>(fo));
    let (be, bo) = (_mm_srai_epi32::<1>(be), _mm_srai_epi32::<1>(bo));
    let ((c1, s1), (c2, s2), (c3, s3)) = twiddles(tab, m / 2, i0);
    let a1 = _mm_add_epi32(fo, bo);
    let a2 = _mm_sub_epi32(be, fe);
    let (accu1, accu2) = cmd2(a2, a1, c3, s3);
    let (accu1, accu2) = (_mm_slli_epi32::<1>(accu1), _mm_slli_epi32::<1>(accu2));
    let a1 = _mm_add_epi32(fe, be);
    let a2 = _mm_sub_epi32(fo, bo);
    let (p3, p4) = cm(_mm_add_epi32(accu1, a2), _mm_add_epi32(a1, accu2), c1, s1);
    let (q3, q4) = cm(_mm_sub_epi32(accu1, a2), _mm_sub_epi32(a1, accu2), c2, s2);
    st(x.add(i0), p4);
    st(x.add(l - i0 - 3), rev(neg(p3)));
    st(x.add(m + i0), neg(q3));
    st(x.add(m - i0 - 3), rev(q4));
}
