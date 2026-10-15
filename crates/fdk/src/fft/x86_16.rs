// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// AVX2 versions of fft_16 and fft_32 of libFDK/src/fft.cpp (x86, x86_32 and generic variants).

//! `fft_16` and `fft_32` of the variants other than `aarch64` with four butterflies per
//! vector operation, selected at run time ([`crate::simd::avx2`]; the 128-bit operations of
//! SSE4.1/SSSE3 are used, which AVX2 implies). The structure is that of `super::neon16`:
//!
//! - The unrolled C kernels are a radix-4 first pass (inputs `b`, `b + N/4`, `b + N/2`,
//!   `b + 3N/4`, outputs `4 bitrev(b) .. 4 bitrev(b) + 3`) followed by radix-2 stages with the
//!   spans 4, 8 (and 16); the complex values are kept as 4-lane vectors of real and imaginary
//!   parts (`V[q]` = complex values `4q .. 4q + 3`). The first pass computes the radix-4
//!   butterflies of four `b` in the lanes and transposes the results (4x4) into the output
//!   groups. In `fft_16` of these variants `SHIFT_A` halves before the sum:
//!   `A = (q0 >> 1) + (q2 >> 1)`, `B = (q1 >> 1) + (q3 >> 1)`; `fft_32` computes
//!   `A = (q0 + q2) >> 1`, `B = (q1 + q3) >> 1` on every variant.
//! - Every radix-2 butterfly computes `u = x1 >> 1`, `x1 = u + v`, `x2 = u - v`. With the
//!   individually truncated products `M(x, w) = (x * w) >> 16` of `cplxMultDiv2` and
//!   `SUMDIFF_PIFOURTH` in these variants, every form of the C code is
//!   `v.re = sa M(x2.re, a) + sb M(x2.im, b)`, `v.im = sc M(x2.re, c) + sd M(x2.im, d)` with
//!   per-lane coefficients and signs (a negated truncated product):
//!   - trivial (`v = x2 >> 1`, `x >> 1 = M(x, 32768)`): `a = d = 32768`;
//!   - `-j` (`v = (x2.im >> 1, -(x2.re >> 1))`): `b = 32768`, `c = 32768` negated;
//!   - `(vi, vr) = cplxMultDiv2(x2.im, x2.re, w)`, `x1 = u + v`:
//!     `v = (M(re, w.re) + M(im, w.im), -M(re, w.im) + M(im, w.re))`;
//!   - `(vr, vi) = cplxMultDiv2(x2.im, x2.re, w)`, `x1 = (u.re + vr, u.im - vi)`: with
//!     `v = (vr, -vi)`, `v = (-M(re, w.im) + M(im, w.re), -M(re, w.re) - M(im, w.im))`;
//!   - `(vi, vr) = SUMDIFF_PIFOURTH(x2.re, x2.im)`, `x1 = u + v`:
//!     `v = (M(re, W) + M(im, W), -M(re, W) + M(im, W))`;
//!   - `(vr, vi) = SUMDIFF_PIFOURTH(x2.re, x2.im)`, `x1 = (u.re + vr, u.im - vi)`:
//!     `v = (-M(re, W) + M(im, W), -M(re, W) - M(im, W))`.
//! - `M(x, w)` is computed exactly in 32-bit lanes as `(x >> 16) * w + (((x & 0xFFFF) * w) >> 16)`
//!   (no intermediate value exceeds 31 bits for `|w| <= 32768`); all additions, subtractions and
//!   negations wrap, like the scalar code.

use super::{FFT16_W16, FFT32_W32, W_PI_FOURTH};
use crate::archdef::FixpStp;
#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// The form of the radix-2 butterfly of a lane (see the module documentation).
#[derive(Clone, Copy)]
enum Bf {
    Triv,
    J,
    A(FixpStp),
    B(FixpStp),
    PiA,
    PiB,
}

/// The coefficients and signs of 4 lanes: `v.re = sa M(re, a) + sb M(im, b)`,
/// `v.im = sc M(re, c) + sd M(im, d)`.
#[derive(Clone, Copy)]
struct Tw {
    w: [[i32; 4]; 4],
    s: [[i32; 4]; 4],
}

const fn tw(l: [Bf; 4]) -> Tw {
    let mut t = Tw { w: [[0; 4]; 4], s: [[0; 4]; 4] };
    let p4 = W_PI_FOURTH as i32;
    let mut i = 0;
    while i < 4 {
        // (a, sa), (b, sb), (c, sc), (d, sd)
        let f: [(i32, i32); 4] = match l[i] {
            Bf::Triv => [(32768, 1), (0, 0), (0, 0), (32768, 1)],
            Bf::J => [(0, 0), (32768, 1), (32768, -1), (0, 0)],
            Bf::A(w) => [(w.re as i32, 1), (w.im as i32, 1), (w.im as i32, -1), (w.re as i32, 1)],
            Bf::B(w) => [(w.im as i32, -1), (w.re as i32, 1), (w.re as i32, -1), (w.im as i32, -1)],
            Bf::PiA => [(p4, 1), (p4, 1), (p4, -1), (p4, 1)],
            Bf::PiB => [(p4, -1), (p4, 1), (p4, -1), (p4, -1)],
        };
        let mut j = 0;
        while j < 4 {
            t.w[j][i] = f[j].0;
            t.s[j][i] = f[j].1;
            j += 1;
        }
        i += 1;
    }
    t
}

const W0: FixpStp = FFT32_W32[0];
const W1: FixpStp = FFT32_W32[1];

/// Span 4: the lanes `k = 0..4` of `(V[2g], V[2g + 1])`.
const C4: Tw = tw([Bf::Triv, Bf::PiA, Bf::J, Bf::PiB]);
/// Span 8: `x1 = V[4g]` lanes `k = 0..4`, `x1 = V[4g + 1]` lanes `k = 4..8`.
const C8_LO: Tw = tw([Bf::Triv, Bf::A(W0), Bf::PiA, Bf::A(W1)]);
const C8_HI: Tw = tw([Bf::J, Bf::B(W0), Bf::PiB, Bf::B(W1)]);
/// Span 16 (`fft_32`): lanes `k = 0..16` in four vectors.
const C16: [Tw; 4] = [
    tw([Bf::Triv, Bf::A(FFT32_W32[2]), Bf::A(W0), Bf::A(FFT32_W32[3])]),
    tw([Bf::PiA, Bf::A(FFT32_W32[4]), Bf::A(W1), Bf::A(FFT32_W32[5])]),
    tw([Bf::J, Bf::B(FFT32_W32[2]), Bf::B(W0), Bf::B(FFT32_W32[3])]),
    tw([Bf::PiB, Bf::B(FFT32_W32[4]), Bf::B(W1), Bf::B(FFT32_W32[5])]),
];

const _: () = {
    // fft_16 uses fft16_w16, which has the values of fft32_w32[0..2].
    assert!(FFT16_W16[0].re == W0.re && FFT16_W16[0].im == W0.im);
    assert!(FFT16_W16[1].re == W1.re && FFT16_W16[1].im == W1.im);
};

/// A complex vector: real and imaginary parts of 4 complex values.
#[derive(Clone, Copy)]
pub(crate) struct C(pub(crate) __m128i, pub(crate) __m128i);

#[inline(always)]
unsafe fn ld(x: *const i32, q: usize) -> C {
    let lo = _mm_shuffle_epi32::<0b11_01_10_00>(_mm_loadu_si128(x.add(8 * q) as *const __m128i));
    let hi =
        _mm_shuffle_epi32::<0b11_01_10_00>(_mm_loadu_si128(x.add(8 * q + 4) as *const __m128i));
    C(_mm_unpacklo_epi64(lo, hi), _mm_unpackhi_epi64(lo, hi))
}

#[inline(always)]
unsafe fn st(x: *mut i32, q: usize, c: C) {
    _mm_storeu_si128(x.add(8 * q) as *mut __m128i, _mm_unpacklo_epi32(c.0, c.1));
    _mm_storeu_si128(x.add(8 * q + 4) as *mut __m128i, _mm_unpackhi_epi32(c.0, c.1));
}

#[inline(always)]
unsafe fn ldv(a: &[i32; 4]) -> __m128i {
    _mm_loadu_si128(a.as_ptr() as *const __m128i)
}

/// `M(x, w) = (x * w) >> 16` per lane (see the module documentation).
#[inline(always)]
unsafe fn mul_div2(x: __m128i, w: __m128i) -> __m128i {
    let hi = _mm_mullo_epi32(_mm_srai_epi32::<16>(x), w);
    let lo = _mm_and_si128(x, _mm_set1_epi32(0xFFFF));
    _mm_add_epi32(hi, _mm_srai_epi32::<16>(_mm_mullo_epi32(lo, w)))
}

/// `sa M(re, a) + sb M(im, b)` for one coefficient pair of [`Tw`].
#[inline(always)]
unsafe fn term(re: __m128i, im: __m128i, t: &Tw, j: usize) -> __m128i {
    let p = _mm_sign_epi32(mul_div2(re, ldv(&t.w[j])), ldv(&t.s[j]));
    let q = _mm_sign_epi32(mul_div2(im, ldv(&t.w[j + 1])), ldv(&t.s[j + 1]));
    _mm_add_epi32(p, q)
}

/// `v` of 4 lanes.
#[inline(always)]
unsafe fn v(x2: C, t: &Tw) -> C {
    C(term(x2.0, x2.1, t, 0), term(x2.0, x2.1, t, 2))
}

/// The radix-2 butterflies of 4 lanes: `(u + v, u - v)` with `u = x1 >> 1`.
#[inline(always)]
unsafe fn bf(x1: C, v: C) -> (C, C) {
    let ur = _mm_srai_epi32::<1>(x1.0);
    let ui = _mm_srai_epi32::<1>(x1.1);
    (
        C(_mm_add_epi32(ur, v.0), _mm_add_epi32(ui, v.1)),
        C(_mm_sub_epi32(ur, v.0), _mm_sub_epi32(ui, v.1)),
    )
}

/// The span-8 stage on `(V[4g], V[4g + 2])` (lanes `k = 0..4`) and `(V[4g + 1], V[4g + 3])`
/// (`k = 4..8`).
#[inline(always)]
unsafe fn span8(v8: &mut [C], g: usize) {
    let (a, b) = (v8[4 * g], v8[4 * g + 2]);
    (v8[4 * g], v8[4 * g + 2]) = bf(a, v(b, &C8_LO));
    let (a, b) = (v8[4 * g + 1], v8[4 * g + 3]);
    (v8[4 * g + 1], v8[4 * g + 3]) = bf(a, v(b, &C8_HI));
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

/// The radix-4 first pass of 4 lanes `b`: inputs `q0 = x[b]`, `q1 = x[b + N/4]`,
/// `q2 = x[b + N/2]`, `q3 = x[b + 3N/4]`, with `A`, `B` as in the module documentation
/// (`SUM_FIRST`: `fft_32`); returns the 4 output groups (transposed: group `l` holds the 4
/// outputs of lane `l`).
#[inline(always)]
unsafe fn radix4<const SUM_FIRST: bool>(q0: C, q1: C, q2: C, q3: C) -> [C; 4] {
    let h = |a: __m128i| _mm_srai_epi32::<1>(a);
    let add = |a: __m128i, b: __m128i| _mm_add_epi32(a, b);
    let sub = |a: __m128i, b: __m128i| _mm_sub_epi32(a, b);
    let (ar, ai, br, bi) = if SUM_FIRST {
        (h(add(q0.0, q2.0)), h(add(q0.1, q2.1)), h(add(q1.0, q3.0)), h(add(q1.1, q3.1)))
    } else {
        (add(h(q0.0), h(q2.0)), add(h(q0.1), h(q2.1)), add(h(q1.0), h(q3.0)), add(h(q1.1), h(q3.1)))
    };
    let y0 = C(add(ar, br), add(ai, bi));
    let y2 = C(sub(ar, br), sub(ai, bi));
    let vr = sub(ar, q2.0);
    let ur = sub(ai, q2.1);
    let vi = sub(br, q3.0);
    let ui = sub(bi, q3.1);
    let y1 = C(add(ui, vr), sub(ur, vi));
    let y3 = C(sub(vr, ui), add(vi, ur));
    let (r0, r1, r2, r3) = transpose(y0.0, y1.0, y2.0, y3.0);
    let (i0, i1, i2, i3) = transpose(y0.1, y1.1, y2.1, y3.1);
    [C(r0, i0), C(r1, i1), C(r2, i2), C(r3, i3)]
}

/// `fft_16` of the variants other than `aarch64` on the 32 values of `x`.
///
/// # Safety
///
/// AVX2 must be available.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn fft_16(x: &mut [i32]) {
    let x = &mut x[..32];
    // SAFETY: x holds 32 values (vectors 0..4 of 8 values).
    unsafe {
        let p = x.as_mut_ptr();
        let v8 = fft_16_regs([ld(p, 0), ld(p, 1), ld(p, 2), ld(p, 3)]);
        for (q, c) in v8.iter().enumerate() {
            st(p, q, *c);
        }
    }
}

/// [`fft_16`] on vectors: `v[q]` holds the complex values `4q .. 4q + 3` (input and output).
///
/// # Safety
///
/// AVX2 must be available (the caller is compiled with it).
#[inline(always)]
pub(crate) unsafe fn fft_16_regs(input: [C; 4]) -> [C; 4] {
    let o = radix4::<false>(input[0], input[1], input[2], input[3]);
    // lane b -> output group bitrev2(b): 0, 2, 1, 3
    let mut v8 = [o[0], o[2], o[1], o[3]];
    for g in 0..2 {
        (v8[2 * g], v8[2 * g + 1]) = bf(v8[2 * g], v(v8[2 * g + 1], &C4));
    }
    span8(&mut v8, 0);
    v8
}

/// `fft_32` of the variants other than `aarch64` on the 64 values of `x`.
///
/// # Safety
///
/// AVX2 must be available.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn fft_32(x: &mut [i32]) {
    let x = &mut x[..64];
    // SAFETY: x holds 64 values (vectors 0..8 of 8 values).
    unsafe {
        let p = x.as_mut_ptr();
        let v8 = fft_32_regs(core::array::from_fn(|q| ld(p, q)));
        for (q, c) in v8.iter().enumerate() {
            st(p, q, *c);
        }
    }
}

/// [`fft_32`] on vectors: `v[q]` holds the complex values `4q .. 4q + 3` (input and output).
///
/// # Safety
///
/// AVX2 must be available (the caller is compiled with it).
#[inline(always)]
pub(crate) unsafe fn fft_32_regs(input: [C; 8]) -> [C; 8] {
    let lo = radix4::<true>(input[0], input[2], input[4], input[6]);
    let hi = radix4::<true>(input[1], input[3], input[5], input[7]);
    // lane b -> output group bitrev3(b): b = 0..4 -> 0, 4, 2, 6; b = 4..8 -> 1, 5, 3, 7
    let mut v8 = [lo[0], hi[0], lo[2], hi[2], lo[1], hi[1], lo[3], hi[3]];
    for g in 0..4 {
        (v8[2 * g], v8[2 * g + 1]) = bf(v8[2 * g], v(v8[2 * g + 1], &C4));
    }
    span8(&mut v8, 0);
    span8(&mut v8, 1);
    for k in 0..4 {
        let (a, b) = (v8[k], v8[k + 4]);
        (v8[k], v8[k + 4]) = bf(a, v(b, &C16[k]));
    }
    v8
}

/// The complex values `k .. k + 4` of the interleaved values at `x` as a vector.
///
/// # Safety
///
/// `x` holds `2 (k + 4)` values.
#[inline(always)]
pub(crate) unsafe fn ld_at(x: *const i32, k: usize) -> C {
    ld(x.add(2 * k), 0)
}

/// Stores the vector `c` as the interleaved complex values `k .. k + 4` at `x`.
///
/// # Safety
///
/// `x` holds `2 (k + 4)` values.
#[inline(always)]
pub(crate) unsafe fn st_at(x: *mut i32, k: usize, c: C) {
    st(x.add(2 * k), 0, c)
}
