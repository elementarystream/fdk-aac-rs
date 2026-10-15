// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// NEON versions of fft_16 and fft_32 of libFDK/src/fft.cpp (aarch64 variant).

//! `fft_16` and `fft_32` of the `aarch64` variant with four butterflies per NEON operation.
//!
//! The unrolled C kernels are a radix-4 first pass (inputs `b`, `b + N/4`, `b + N/2`,
//! `b + 3N/4`, outputs `4 bitrev(b) .. 4 bitrev(b) + 3`) followed by radix-2 stages with the
//! spans 4, 8 (and 16); the complex values are kept as 4-lane vectors of real and imaginary
//! parts (`V[q]` = complex values `4q .. 4q + 3`).
//!
//! - The first pass computes the radix-4 butterflies of four `b` in the lanes and transposes
//!   the results (4x4) into the output groups.
//! - Every radix-2 butterfly computes `u = x1 >> 1`, `x1 = u + v`, `x2 = u - v`, with `v` from
//!   `x2` by one of the forms of the C code, chosen per lane:
//!   - the trivial twiddle (`v = x2 >> 1`), `-j` (`v = (x2.im >> 1, -(x2.re >> 1))`) and the
//!     `cplxMultDiv2` twiddles of both orientations (`(vi, vr) = cplxMultDiv2(x2.im, x2.re, w)`
//!     with `x1 = u + v`, or `(vr, vi) = ...` with `x1 = (u.re + vr, u.im - vi)`) as
//!     `v = ((x2.re p + x2.im q) >> 16, (x2.im p - x2.re q + b) >> 16)` in 64 bits (the
//!     `aarch64` `cplxMultDiv2`: bits 16..47 of the sum of the products); `x >> 1` is
//!     `(x * 32768) >> 16`, and a negated truncated product is `(-A + 0xFFFF) >> 16`
//!     (`b = 0xFFFF`);
//!   - `SUMDIFF_PIFOURTH` (the individually truncated products `fMultDiv2(x2.re, W)` and
//!     `fMultDiv2(x2.im, W)`, `vqdmulh` with `W << 15`), whose lanes replace the 64-bit form;
//!   - in the span-4 stage, all lanes: individually truncated products (`x >> 1` is
//!     `fMultDiv2(x, 32768)`), with per-lane coefficients and signs (negation after the
//!     truncation).
//! - All additions, subtractions and negations wrap, like the scalar code.

use super::{FFT16_W16, FFT32_W32, W_PI_FOURTH};
use crate::archdef::FixpStp;
use core::arch::aarch64::*;

/// The form of the radix-2 butterfly of a lane (see the module documentation).
#[derive(Clone, Copy)]
enum Bf {
    /// `v = x2 >> 1`
    Triv,
    /// `v = (x2.im >> 1, -(x2.re >> 1))`
    J,
    /// `(vi, vr) = cplxMultDiv2(x2.im, x2.re, w)`, `x1 = u + v`
    A(FixpStp),
    /// `(vr, vi) = cplxMultDiv2(x2.im, x2.re, w)`, `x1 = (u.re + vr, u.im - vi)`
    B(FixpStp),
    /// `(vi, vr) = SUMDIFF_PIFOURTH(x2.re, x2.im)`, `x1 = u + v`
    PiA,
    /// `(vr, vi) = SUMDIFF_PIFOURTH(x2.re, x2.im)`, `x1 = (u.re + vr, u.im - vi)`
    PiB,
}

/// The 64-bit form coefficients of 4 lanes: `p`, `q`, `-q`, the bias of the imaginary part.
#[derive(Clone, Copy)]
struct Tw {
    p: [i32; 4],
    q: [i32; 4],
    nq: [i32; 4],
    bi: [i64; 4],
}

const fn tw(l: [Bf; 4]) -> Tw {
    let mut t = Tw { p: [0; 4], q: [0; 4], nq: [0; 4], bi: [0; 4] };
    let mut i = 0;
    while i < 4 {
        let (p, q, b) = match l[i] {
            Bf::Triv => (32768, 0, 0),
            Bf::J => (0, 32768, 0xFFFF),
            Bf::A(w) => (w.re as i32, w.im as i32, 0),
            Bf::B(w) => (-(w.im as i32), w.re as i32, 0xFFFF),
            Bf::PiA | Bf::PiB => (0, 0, 0),
        };
        t.p[i] = p;
        t.q[i] = q;
        t.nq[i] = -q;
        t.bi[i] = b;
        i += 1;
    }
    t
}

const W0: FixpStp = FFT32_W32[0];
const W1: FixpStp = FFT32_W32[1];

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
pub(crate) struct C(pub(crate) int32x4_t, pub(crate) int32x4_t);

#[inline(always)]
unsafe fn ld(x: *const i32, q: usize) -> C {
    let v = vld2q_s32(x.add(8 * q));
    C(v.0, v.1)
}

#[inline(always)]
unsafe fn st(x: *mut i32, q: usize, c: C) {
    vst2q_s32(x.add(8 * q), int32x4x2_t(c.0, c.1));
}

/// `(a * p + b * q + bias) >> 16` per lane in 64 bits, truncated to 32 bits.
#[inline(always)]
unsafe fn mac(
    bias: (int64x2_t, int64x2_t),
    a: int32x4_t,
    p: int32x4_t,
    b: int32x4_t,
    q: int32x4_t,
) -> int32x4_t {
    let lo = vmlal_s32(
        vmlal_s32(bias.0, vget_low_s32(a), vget_low_s32(p)),
        vget_low_s32(b),
        vget_low_s32(q),
    );
    let hi = vmlal_high_s32(vmlal_high_s32(bias.1, a, p), b, q);
    vshrn_high_n_s64::<16>(vshrn_n_s64::<16>(lo), hi)
}

/// `v` of 4 lanes in the 64-bit form.
#[inline(always)]
unsafe fn v64(x2: C, t: &Tw) -> C {
    let p = vld1q_s32(t.p.as_ptr());
    let q = vld1q_s32(t.q.as_ptr());
    let nq = vld1q_s32(t.nq.as_ptr());
    let z = vdupq_n_s64(0);
    let bi = (vld1q_s64(t.bi.as_ptr()), vld1q_s64(t.bi.as_ptr().add(2)));
    C(mac((z, z), x2.0, p, x2.1, q), mac(bi, x2.1, p, x2.0, nq))
}

/// Replaces lane `L` of `v` by the `SUMDIFF_PIFOURTH` form (`B`: the second orientation).
#[inline(always)]
unsafe fn pi4_lane<const L: i32, const B: bool>(x2: C, v: C) -> C {
    let w = vdupq_n_s32((W_PI_FOURTH as i32) << 15);
    let wa = vqdmulhq_s32(x2.0, w);
    let wb = vqdmulhq_s32(x2.1, w);
    let (re, im) = if B {
        (vsubq_s32(wb, wa), vnegq_s32(vaddq_s32(wb, wa)))
    } else {
        (vaddq_s32(wb, wa), vsubq_s32(wb, wa))
    };
    let m = vreinterpretq_u32_s32(vsetq_lane_s32::<L>(-1, vdupq_n_s32(0)));
    C(vbslq_s32(m, re, v.0), vbslq_s32(m, im, v.1))
}

/// The radix-2 butterflies of 4 lanes: `(u + v, u - v)` with `u = x1 >> 1`.
#[inline(always)]
unsafe fn bf(x1: C, v: C) -> (C, C) {
    let ur = vshrq_n_s32::<1>(x1.0);
    let ui = vshrq_n_s32::<1>(x1.1);
    (C(vaddq_s32(ur, v.0), vaddq_s32(ui, v.1)), C(vsubq_s32(ur, v.0), vsubq_s32(ui, v.1)))
}

/// `(x ^ m) - m`: `x` negated in the lanes where `m` is -1.
#[inline(always)]
unsafe fn cneg(x: int32x4_t, m: int32x4_t) -> int32x4_t {
    vsubq_s32(veorq_s32(x, m), m)
}

/// The span-4 stage on the groups `(V[2g], V[2g + 1])`: lanes trivial, `PiA`, `-j`, `PiB`,
/// all with individually truncated products (see the module documentation).
#[inline(always)]
unsafe fn span4(x1: C, x2: C) -> (C, C) {
    let w = (W_PI_FOURTH as i32) << 15;
    let h = 1 << 30; // 32768 << 15: fMultDiv2(x, 32768) = x >> 1
    let a1 = vld1q_s32([h, w, 0, w].as_ptr());
    let a2 = vld1q_s32([0, w, h, w].as_ptr());
    let m1 = vld1q_s32([0, 0, 0, -1].as_ptr());
    let m3 = vld1q_s32([0, -1, -1, -1].as_ptr());
    let re = vaddq_s32(cneg(vqdmulhq_s32(x2.0, a1), m1), vqdmulhq_s32(x2.1, a2));
    let im = vaddq_s32(cneg(vqdmulhq_s32(x2.0, a2), m3), cneg(vqdmulhq_s32(x2.1, a1), m1));
    bf(x1, C(re, im))
}

/// The span-8 stage on `(V[4g], V[4g + 2])` (lanes `k = 0..4`) and `(V[4g + 1], V[4g + 3])`
/// (`k = 4..8`).
#[inline(always)]
unsafe fn span8(v: &mut [C], g: usize) {
    let (a, b) = (v[4 * g], v[4 * g + 2]);
    let w = pi4_lane::<2, false>(b, v64(b, &C8_LO));
    (v[4 * g], v[4 * g + 2]) = bf(a, w);
    let (a, b) = (v[4 * g + 1], v[4 * g + 3]);
    let w = pi4_lane::<2, true>(b, v64(b, &C8_HI));
    (v[4 * g + 1], v[4 * g + 3]) = bf(a, w);
}

/// The radix-4 first pass of 4 lanes `b`: inputs `q0 = x[b]`, `q1 = x[b + N/4]`,
/// `q2 = x[b + N/2]`, `q3 = x[b + 3N/4]`, with `A = (q0 + q2) >> 1` (`fft_32`, `SUM_FIRST`) or
/// `(q0 >> 1) + (q2 >> 1)` (`fft_16`) and `B = (q1 + q3) >> 1` (`SHIFT_A` empty, `SHIFT_B`
/// `>> 1`); returns the 4 output groups (transposed: group `l` holds the 4 outputs of lane `l`).
#[inline(always)]
unsafe fn radix4<const SUM_FIRST: bool>(q0: C, q1: C, q2: C, q3: C) -> [C; 4] {
    let (ar, ai) = if SUM_FIRST {
        (vshrq_n_s32::<1>(vaddq_s32(q0.0, q2.0)), vshrq_n_s32::<1>(vaddq_s32(q0.1, q2.1)))
    } else {
        (
            vaddq_s32(vshrq_n_s32::<1>(q0.0), vshrq_n_s32::<1>(q2.0)),
            vaddq_s32(vshrq_n_s32::<1>(q0.1), vshrq_n_s32::<1>(q2.1)),
        )
    };
    let br = vshrq_n_s32::<1>(vaddq_s32(q1.0, q3.0));
    let bi = vshrq_n_s32::<1>(vaddq_s32(q1.1, q3.1));
    let y0 = C(vaddq_s32(ar, br), vaddq_s32(ai, bi));
    let y2 = C(vsubq_s32(ar, br), vsubq_s32(ai, bi));
    let vr = vsubq_s32(ar, q2.0);
    let ur = vsubq_s32(ai, q2.1);
    let vi = vsubq_s32(br, q3.0);
    let ui = vsubq_s32(bi, q3.1);
    let y1 = C(vaddq_s32(ui, vr), vsubq_s32(ur, vi));
    let y3 = C(vsubq_s32(vr, ui), vaddq_s32(vi, ur));
    let (r0, r1, r2, r3) = transpose(y0.0, y1.0, y2.0, y3.0);
    let (i0, i1, i2, i3) = transpose(y0.1, y1.1, y2.1, y3.1);
    [C(r0, i0), C(r1, i1), C(r2, i2), C(r3, i3)]
}

/// The 4x4 transpose of `(q0, q1, q2, q3)`.
#[inline(always)]
unsafe fn transpose(
    q0: int32x4_t,
    q1: int32x4_t,
    q2: int32x4_t,
    q3: int32x4_t,
) -> (int32x4_t, int32x4_t, int32x4_t, int32x4_t) {
    let t0 = vtrn1q_s32(q0, q1);
    let t1 = vtrn2q_s32(q0, q1);
    let t2 = vtrn1q_s32(q2, q3);
    let t3 = vtrn2q_s32(q2, q3);
    let f = |a: int32x4_t| vreinterpretq_s64_s32(a);
    let g = |a: int64x2_t| vreinterpretq_s32_s64(a);
    (
        g(vtrn1q_s64(f(t0), f(t2))),
        g(vtrn1q_s64(f(t1), f(t3))),
        g(vtrn2q_s64(f(t0), f(t2))),
        g(vtrn2q_s64(f(t1), f(t3))),
    )
}

/// `fft_16` of the `aarch64` variant on the 32 values of `x`.
pub(crate) fn fft_16_arm(x: &mut [i32]) {
    let x = &mut x[..32];
    // SAFETY: x holds 32 values (vectors 0..4 of 8 values).
    unsafe {
        let p = x.as_mut_ptr();
        let v = fft_16_regs([ld(p, 0), ld(p, 1), ld(p, 2), ld(p, 3)]);
        for (q, c) in v.iter().enumerate() {
            st(p, q, *c);
        }
    }
}

/// [`fft_16_arm`] on vectors: `v[q]` holds the complex values `4q .. 4q + 3` (input and output).
///
/// # Safety
/// NEON (part of AArch64).
#[inline(always)]
pub(crate) unsafe fn fft_16_regs(v: [C; 4]) -> [C; 4] {
    let o = radix4::<false>(v[0], v[1], v[2], v[3]);
    // lane b -> output group bitrev2(b): 0, 2, 1, 3
    let mut v = [o[0], o[2], o[1], o[3]];
    for g in 0..2 {
        (v[2 * g], v[2 * g + 1]) = span4(v[2 * g], v[2 * g + 1]);
    }
    span8(&mut v, 0);
    v
}

/// `fft_32` of the `aarch64` variant on the 64 values of `x`.
pub(crate) fn fft_32_arm(x: &mut [i32]) {
    let x = &mut x[..64];
    // SAFETY: x holds 64 values (vectors 0..8 of 8 values).
    unsafe {
        let p = x.as_mut_ptr();
        let v = fft_32_regs(core::array::from_fn(|q| ld(p, q)));
        for (q, c) in v.iter().enumerate() {
            st(p, q, *c);
        }
    }
}

/// [`fft_32_arm`] on vectors: `v[q]` holds the complex values `4q .. 4q + 3` (input and output).
///
/// # Safety
/// NEON (part of AArch64).
#[inline(always)]
pub(crate) unsafe fn fft_32_regs(v: [C; 8]) -> [C; 8] {
    let lo = radix4::<true>(v[0], v[2], v[4], v[6]);
    let hi = radix4::<true>(v[1], v[3], v[5], v[7]);
    // lane b -> output group bitrev3(b): b = 0..4 -> 0, 4, 2, 6; b = 4..8 -> 1, 5, 3, 7
    let mut v = [lo[0], hi[0], lo[2], hi[2], lo[1], hi[1], lo[3], hi[3]];
    for g in 0..4 {
        (v[2 * g], v[2 * g + 1]) = span4(v[2 * g], v[2 * g + 1]);
    }
    span8(&mut v, 0);
    span8(&mut v, 1);
    for k in 0..4 {
        let (a, b) = (v[k], v[k + 4]);
        let mut w = v64(b, &C16[k]);
        if k == 1 {
            w = pi4_lane::<0, false>(b, w);
        } else if k == 3 {
            w = pi4_lane::<0, true>(b, w);
        }
        (v[k], v[k + 4]) = bf(a, w);
    }
    v
}
