// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized version of libFDK/src/arm/fft_rad2_arm.cpp (aarch64 variant).

//! NEON implementation of [`super::dit_fft_arm`] for `SineTable512` (`trigDataSize` 512), the
//! table of every `dit_fft` call of the library (`fft()`, lengths 64 to 512).
//!
//! The butterflies of one stage are independent of each other, so they may be computed in any
//! order; every butterfly uses exactly the arithmetic of the straight port:
//!
//! - The bit reversal permutation (`scramble`) and the first two (radix-4) stages are fused into
//!   one out-of-place pass: the radix-4 butterfly of output group `g` reads the complex inputs
//!   `r`, `r + n/2`, `r + n/4` and `r + 3n/4` with `r = bitrev(g)`, so four consecutive `r` are
//!   processed in parallel and the results are scattered to the groups `bitrev(r)`.
//! - Each radix-2 butterfly with the span `mh` (complex values) and index `k < mh` computes
//!   `x1 = u + v`, `x2 = u - v` with `u = x1 >> 1` and
//!   `v = (sr * ((x2.re * p + x2.im * q) >> 16), si * ((x2.re * r + x2.im * t) >> 16))`.
//!   The C code handles the indices `k`, `mh/2 - k`, `mh/2 + k`, `mh - k` with four variants of
//!   the complex multiplication (swapped operands, negated sine) and different signs in the
//!   final additions; each variant maps to one set of coefficients `(p, q, r, t)` and signs
//!   `(sr, si)` that reproduces it exactly (see `lane`). The trivial twiddle factors `k = 0` and
//!   `k = mh/2` shift by 1 (`x >> 1 == (x * 32768) >> 16`).
//! - The `aarch64` `cplxMultDiv2` with a `FIXP_SPK` twiddle sums the two 64-bit products of
//!   `a` and `b << 16` and keeps bits 32..63 (`smull`, `smaddl`/`smsubl`, `asr #32`); these are
//!   bits 16..47 of the sum of the products with `b`, which is what `(a * p + b * q) >> 16`
//!   truncated to 32 bits gives (the sum of two products of a 32-bit and a 17-bit value cannot
//!   overflow 64 bits).
//! - The negation of a truncated product is folded into the multiplication: for a 64-bit sum of
//!   products `A`, `-(A >> 16) == (-A + 0xFFFF) >> 16`, so a lane with a negative sign uses the
//!   negated coefficients and the bias `0xFFFF` in the accumulator. With the signs folded,
//!   `(r, t) == (-q, p)` holds for all lanes, i.e. `v = x2 * (p - i q)` (checked when the table
//!   is built).
//! - The radix-2 stages are fused in pairs: the values `k`, `k + mh`, `k + 2 mh`, `k + 3 mh` of a
//!   group of `4 mh` values depend only on each other in the stages `mh` and `2 mh`.
//! - The additions wrap, like the straight port.

use crate::archdef::{stc, FixpStp};
use crate::common_fix::FixpDbl;
use crate::tools_rom::SINE_TABLE_512;
use core::arch::aarch64::*;
use core::mem::MaybeUninit;

/// Smallest supported `ldn` (the fused first pass handles 4 groups of 4 at a time).
pub(super) const MIN_LDN: i32 = 4;
/// Largest supported `ldn`.
pub(super) const MAX_LDN: i32 = 10;
const MAX_N: usize = 1 << MAX_LDN;

/// Coefficients of 4 butterflies with the signs folded in: `v = ((x2.re * p + x2.im * q + br) >>
/// 16, (x2.im * p - x2.re * q + bi) >> 16)`.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct BlockCoef {
    p: [i32; 4],
    q: [i32; 4],
    nq: [i32; 4],
    br: [i64; 4],
    bi: [i64; 4],
}

/// Number of blocks of the radix-2 stages `mh = 4, 8, ..., MAX_N / 2` (`mh / 4` blocks each).
const N_BLOCKS: usize = (MAX_N - 4) / 4;

/// `(cos, sin)` of `SineTable512[i * trigstep]`.
const fn cs(i: usize, trigstep: usize) -> (i32, i32) {
    let t: FixpStp = SINE_TABLE_512[i * trigstep];
    (t.re as i32, t.im as i32)
}

/// `(p, q, r, t, sr, si)` of the butterfly `k` of the stage with the span `mh` (`trigstep`:
/// the step in `SineTable512` of that stage), see the module documentation.
const fn lane(k: usize, mh: usize, trigstep: usize) -> [i32; 6] {
    let w = stc(0x5a82799a) as i32; // twiddle factor pi/4
    let q = mh / 4;
    if k == 0 {
        // ur + (vr >> 1), ui + (vi >> 1)
        [32768, 0, 0, 32768, 0, 0]
    } else if k < q {
        // (vi, vr) = cplxMultDiv2(x2.im, x2.re, c, s); x1 = u + v
        let (c, s) = cs(k, trigstep);
        [c, s, -s, c, 0, 0]
    } else if k == q {
        [w, w, -w, w, 0, 0]
    } else if k < 2 * q {
        // index mh/2 - k': (vi, vr) = cplxMultDiv2(x2.re, x2.im, c, s); x1 = (ur + vr, ui - vi)
        let (c, s) = cs(2 * q - k, trigstep);
        [s, c, c, -s, 0, -1]
    } else if k == 2 * q {
        // vr = x2.im >> 1, vi = x2.re >> 1; x1 = (ur + vr, ui - vi)
        [0, 32768, 32768, 0, 0, -1]
    } else if k < 3 * q {
        // index mh/2 + k': (vr, vi) = cplxMultDiv2(x2.im, x2.re, c, s); x1 = (ur + vr, ui - vi)
        let (c, s) = cs(k - 2 * q, trigstep);
        [-s, c, c, s, 0, -1]
    } else if k == 3 * q {
        [-w, w, w, w, 0, -1]
    } else {
        // index mh - k': (vr, vi) = cplxMultDiv2(x2.re, x2.im, c, s); x1 = (ur - vr, ui - vi)
        let (c, s) = cs(4 * q - k, trigstep);
        [c, -s, s, c, -1, -1]
    }
}

/// The coefficient blocks of the radix-2 stages; the stage `mh` starts at block `(mh - 4) / 4`.
static COEFS: [BlockCoef; N_BLOCKS] = {
    let zero = BlockCoef { p: [0; 4], q: [0; 4], nq: [0; 4], br: [0; 4], bi: [0; 4] };
    let mut out = [zero; N_BLOCKS];
    let mut blk = 0;
    let mut mh = 4usize;
    // The C code halves trigstep (starting with trigDataSize = 512) in every stage.
    let mut trigstep = 512 >> 1;
    while mh < MAX_N {
        let mut b = 0;
        while b < mh / 4 {
            let mut l = 0;
            while l < 4 {
                let [p, q, r, t, sr, si] = lane(4 * b + l, mh, trigstep);
                let (p, q) = if sr != 0 { (-p, -q) } else { (p, q) };
                let (r, t) = if si != 0 { (-r, -t) } else { (r, t) };
                assert!(r == -q && t == p, "FFT coefficient table");
                out[blk].p[l] = p;
                out[blk].q[l] = q;
                out[blk].nq[l] = -q;
                out[blk].br[l] = if sr != 0 { 0xFFFF } else { 0 };
                out[blk].bi[l] = if si != 0 { 0xFFFF } else { 0 };
                l += 1;
            }
            blk += 1;
            b += 1;
        }
        mh <<= 1;
        trigstep >>= 1;
    }
    out
};

/// [`super::dit_fft_arm`] with `SineTable512` and `trigDataSize` 512, `MIN_LDN <= ldn <= MAX_LDN`.
pub(super) fn dit_fft(x: &mut [FixpDbl], ldn: i32) {
    assert!((MIN_LDN..=MAX_LDN).contains(&ldn));
    let ldn = ldn as u32;
    let n = 1usize << ldn;
    let x = &mut x[..2 * n];
    let mut tmp = [MaybeUninit::<i32>::uninit(); 2 * MAX_N];
    // SAFETY: `x` holds `2 * n` values and `tmp` at least `2 * n`; the first pass reads `x` and
    // writes all `2 * n` values of `tmp` before the first stage reads them. The coefficients of
    // stage `mh` are `mh / 4` blocks from block `(mh - 4) / 4`, within the table for
    // `mh <= MAX_N / 2`.
    unsafe {
        let xp = x.as_mut_ptr();
        let tp = tmp.as_mut_ptr() as *mut i32;
        first_pass(xp, tp, ldn);
        let mut src = tp as *const i32;
        let mut mh = 4usize;
        while 2 * mh < n {
            stage2(src, xp, n, mh);
            src = xp;
            mh <<= 2;
        }
        if mh < n {
            stage(src, xp, n, mh, COEFS.as_ptr().add((mh - 4) / 4));
        }
    }
}

/// Bit reversal permutation and radix-4 butterflies (the first two stages) from `x` into `y`
/// (`2 * 2^ldn` values each, `ldn >= 4`).
#[inline(always)]
unsafe fn first_pass(x: *const i32, y: *mut i32, ldn: u32) {
    let q = 1usize << (ldn - 2);
    let shift = 32 - (ldn - 2);
    let mut r = 0usize;
    while r < q {
        let c0 = vld2q_s32(x.add(2 * r));
        let c1 = vld2q_s32(x.add(2 * (r + 2 * q)));
        let c2 = vld2q_s32(x.add(2 * (r + q)));
        let c3 = vld2q_s32(x.add(2 * (r + 3 * q)));

        let a00 = vshrq_n_s32::<1>(vaddq_s32(c0.0, c1.0));
        let a10 = vshrq_n_s32::<1>(vaddq_s32(c2.0, c3.0));
        let a20 = vshrq_n_s32::<1>(vaddq_s32(c0.1, c1.1));
        let a30 = vshrq_n_s32::<1>(vaddq_s32(c2.1, c3.1));

        let y0r = vaddq_s32(a00, a10);
        let y2r = vsubq_s32(a00, a10);
        let y0i = vaddq_s32(a20, a30);
        let y2i = vsubq_s32(a20, a30);

        let a00 = vsubq_s32(a00, c1.0);
        let a10 = vsubq_s32(a10, c3.0);
        let a20 = vsubq_s32(a20, c1.1);
        let a30 = vsubq_s32(a30, c3.1);

        let y1r = vaddq_s32(a00, a30);
        let y3r = vsubq_s32(a00, a30);
        let y1i = vsubq_s32(a20, a10);
        let y3i = vaddq_s32(a20, a10);

        let lo = int32x4x4_t(y0r, y0i, y1r, y1i);
        let hi = int32x4x4_t(y2r, y2i, y3r, y3i);
        let dst = |l: usize| y.add(8 * (((r + l) as u32).reverse_bits() >> shift) as usize);
        let d = dst(0);
        vst4q_lane_s32::<0>(d, lo);
        vst4q_lane_s32::<0>(d.add(4), hi);
        let d = dst(1);
        vst4q_lane_s32::<1>(d, lo);
        vst4q_lane_s32::<1>(d.add(4), hi);
        let d = dst(2);
        vst4q_lane_s32::<2>(d, lo);
        vst4q_lane_s32::<2>(d.add(4), hi);
        let d = dst(3);
        vst4q_lane_s32::<3>(d, lo);
        vst4q_lane_s32::<3>(d.add(4), hi);
        r += 4;
    }
}

/// `(bias + a * p + b * q) >> 16` per lane in 64 bits, truncated to 32 bits.
#[inline(always)]
unsafe fn mac2b(
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

/// Coefficients of 4 butterflies in registers.
#[derive(Clone, Copy)]
struct Coef {
    p: int32x4_t,
    q: int32x4_t,
    nq: int32x4_t,
    br: (int64x2_t, int64x2_t),
    bi: (int64x2_t, int64x2_t),
}

#[inline(always)]
unsafe fn load_coef(c: *const BlockCoef) -> Coef {
    let c = &*c;
    Coef {
        p: vld1q_s32(c.p.as_ptr()),
        q: vld1q_s32(c.q.as_ptr()),
        nq: vld1q_s32(c.nq.as_ptr()),
        br: (vld1q_s64(c.br.as_ptr()), vld1q_s64(c.br.as_ptr().add(2))),
        bi: (vld1q_s64(c.bi.as_ptr()), vld1q_s64(c.bi.as_ptr().add(2))),
    }
}

/// 4 radix-2 butterflies: returns `(x1 >> 1) + v` and `(x1 >> 1) - v` (see the module
/// documentation).
#[inline(always)]
unsafe fn bfly(x1: int32x4x2_t, x2: int32x4x2_t, c: &Coef) -> (int32x4x2_t, int32x4x2_t) {
    let ur = vshrq_n_s32::<1>(x1.0);
    let ui = vshrq_n_s32::<1>(x1.1);
    let vr = mac2b(c.br, x2.0, c.p, x2.1, c.q);
    let vi = mac2b(c.bi, x2.1, c.p, x2.0, c.nq);
    (
        int32x4x2_t(vaddq_s32(ur, vr), vaddq_s32(ui, vi)),
        int32x4x2_t(vsubq_s32(ur, vr), vsubq_s32(ui, vi)),
    )
}

/// One radix-2 stage with the span `mh` from `src` to `dst` (may be the same buffer). The
/// coefficients of a block of 4 butterflies are loaded once for all groups.
#[inline(always)]
unsafe fn stage(src: *const i32, dst: *mut i32, n: usize, mh: usize, coef: *const BlockCoef) {
    let mut k = 0usize;
    while k < mh {
        let c = load_coef(coef.add(k / 4));
        let mut g = k;
        while g < n {
            let a = vld2q_s32(src.add(2 * g));
            let b = vld2q_s32(src.add(2 * (g + mh)));
            let (a, b) = bfly(a, b, &c);
            vst2q_s32(dst.add(2 * g), a);
            vst2q_s32(dst.add(2 * (g + mh)), b);
            g += 2 * mh;
        }
        k += 4;
    }
}

/// The radix-2 stages with the spans `mh` and `2 * mh` in one pass: the complex values `k`,
/// `k + mh`, `k + 2 mh`, `k + 3 mh` of a group of `4 mh` values only depend on each other in
/// these two stages (stage `mh`: pairs `(k, k + mh)`, `(k + 2 mh, k + 3 mh)`; stage `2 mh`: pairs
/// `(k, k + 2 mh)`, `(k + mh, k + 3 mh)`).
#[inline(always)]
unsafe fn stage2(src: *const i32, dst: *mut i32, n: usize, mh: usize) {
    let c1_base = COEFS.as_ptr().add((mh - 4) / 4);
    let c2_base = COEFS.as_ptr().add((2 * mh - 4) / 4);
    let mut k = 0usize;
    while k < mh {
        let c1 = load_coef(c1_base.add(k / 4));
        let c2a = load_coef(c2_base.add(k / 4));
        let c2b = load_coef(c2_base.add((k + mh) / 4));
        let mut g = k;
        while g < n {
            let x0 = vld2q_s32(src.add(2 * g));
            let x1 = vld2q_s32(src.add(2 * (g + mh)));
            let x2 = vld2q_s32(src.add(2 * (g + 2 * mh)));
            let x3 = vld2q_s32(src.add(2 * (g + 3 * mh)));
            let (x0, x1) = bfly(x0, x1, &c1);
            let (x2, x3) = bfly(x2, x3, &c1);
            let (x0, x2) = bfly(x0, x2, &c2a);
            let (x1, x3) = bfly(x1, x3, &c2b);
            vst2q_s32(dst.add(2 * g), x0);
            vst2q_s32(dst.add(2 * (g + mh)), x1);
            vst2q_s32(dst.add(2 * (g + 2 * mh)), x2);
            vst2q_s32(dst.add(2 * (g + 3 * mh)), x3);
            g += 4 * mh;
        }
        k += 4;
    }
}
