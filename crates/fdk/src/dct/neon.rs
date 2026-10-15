// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized pre and post twiddling of libFDK/src/dct.cpp (dct_IV, dst_IV).

//! NEON blocks of four iterations of the pre and post twiddling of `dct_IV` and `dst_IV`.
//!
//! The iterations of each loop are independent of each other once the loops are written per
//! pair of complex values (see `super`), so four of them run in the lanes of a vector. Every
//! lane computes exactly what the scalar code computes:
//!
//! - `cplxMultDiv2` of the `aarch64` variant (pre twiddling, `FIXP_WTP` window coefficients):
//!   the 64-bit sum of the two products of `a` and `b << 16`, bits 32..63 (`smull`,
//!   `smaddl`/`smsubl`, `asr #32`). These are bits 16..47 of the sum of the products with `b`:
//!   `vmull_s32`/`vmlal_s32`/`vmlsl_s32` with the coefficient widened to 32 bits and a narrowing
//!   shift by 16 (`vshrn_n_s64`), which keeps the low 32 bits, as the C code does. The sum of two
//!   products of a 32-bit and a 16-bit value cannot overflow 64 bits.
//! - `fMult(FIXP_DBL, FIXP_SGL)` (post twiddling, the same on every variant):
//!   `((a * b) >> 16) << 1`. `vqdmulhq_s32(a, b << 15)` is `(2 * a * (b << 15)) >> 32 =
//!   (a * b) >> 16`; it saturates only for two operands equal to `i32::MIN`, which `b << 15`
//!   never is. The left shift wraps like the C code.
//! - Additions, subtractions and negations wrap (`add`, `sub`, `neg`), like the scalar code.

use crate::archdef::{FixpStp, FixpWtp};
pub(super) use crate::fft::neon16::C;
use core::arch::aarch64::*;

/// The 4 lanes in reverse order.
#[inline(always)]
pub(crate) unsafe fn rev(v: int32x4_t) -> int32x4_t {
    let r = vrev64q_s32(v);
    vextq_s32::<2>(r, r)
}

/// `(a * p + b * q) >> 16` per lane, the sum in 64 bits, the result truncated to 32 bits.
#[inline(always)]
pub(crate) unsafe fn madd16(a: int32x4_t, p: int32x4_t, b: int32x4_t, q: int32x4_t) -> int32x4_t {
    let lo =
        vmlal_s32(vmull_s32(vget_low_s32(a), vget_low_s32(p)), vget_low_s32(b), vget_low_s32(q));
    let hi = vmlal_high_s32(vmull_high_s32(a, p), b, q);
    vshrn_high_n_s64::<16>(vshrn_n_s64::<16>(lo), hi)
}

/// `(a * p - b * q) >> 16` per lane, the difference in 64 bits, the result truncated to 32 bits.
#[inline(always)]
pub(crate) unsafe fn msub16(a: int32x4_t, p: int32x4_t, b: int32x4_t, q: int32x4_t) -> int32x4_t {
    let lo =
        vmlsl_s32(vmull_s32(vget_low_s32(a), vget_low_s32(p)), vget_low_s32(b), vget_low_s32(q));
    let hi = vmlsl_high_s32(vmull_high_s32(a, p), b, q);
    vshrn_high_n_s64::<16>(vshrn_n_s64::<16>(lo), hi)
}

/// The `aarch64` `cplxMultDiv2(&re, &im, a_re, a_im, c, s)` per lane: `(re, im)`.
#[inline(always)]
unsafe fn cmd2(
    a_re: int32x4_t,
    a_im: int32x4_t,
    c: int32x4_t,
    s: int32x4_t,
) -> (int32x4_t, int32x4_t) {
    (msub16(a_re, c, a_im, s), madd16(a_re, s, a_im, c))
}

/// The twiddles `tw[i0 + 2j]` (`.0`, `.1`: cos, sin) and `tw[i0 + 2j + 1]` (`.2`, `.3`) for
/// `j = 0..4`, widened to 32 bits.
#[inline(always)]
unsafe fn load_pre_twiddles(tw: *const FixpWtp) -> (int32x4_t, int32x4_t, int32x4_t, int32x4_t) {
    let t = vld4_s16(tw as *const i16);
    (vmovl_s16(t.0), vmovl_s16(t.1), vmovl_s16(t.2), vmovl_s16(t.3))
}

/// The iterations `i = i0, i0 + 2, i0 + 4, i0 + 6` of the pre twiddling loop of `dct_IV` on the
/// `l` values at `x` (`i0 + 6 < l / 2 - 1`).
///
/// Per iteration: `(a1, a2) = cplxMultDiv2(x[l-1-i], x[i], tw[i])`,
/// `(a3, a4) = cplxMultDiv2(x[l-2-i], x[i+1], tw[i+1])`; `x[i] = a2 >> 1`, `x[i+1] = a1 >> 1`,
/// `x[l-2-i] = a4 >> 1`, `x[l-1-i] = -(a3 >> 1)`.
#[inline(always)]
pub(super) unsafe fn dct_iv_pre_block(x: *mut i32, l: usize, i0: usize, tw: *const FixpWtp) {
    let (f, b) = dct_iv_pre_regs(x, l, i0, tw);
    vst2q_s32(x.add(i0), int32x4x2_t(f.0, f.1));
    vst2q_s32(x.add(l - 8 - i0), int32x4x2_t(b.0, b.1));
}

/// [`dct_iv_pre_block`] without the stores: the complex values `i0 / 2 .. i0 / 2 + 4` (front)
/// and `l / 2 - 4 - i0 / 2 .. l / 2 - i0 / 2` (back) that it writes, as complex vectors.
#[inline(always)]
pub(super) unsafe fn dct_iv_pre_regs(
    x: *const i32,
    l: usize,
    i0: usize,
    tw: *const FixpWtp,
) -> (C, C) {
    let f = vld2q_s32(x.add(i0));
    let b = vld2q_s32(x.add(l - 8 - i0));
    let (be, bo) = (rev(b.0), rev(b.1)); // x[l-2-i], x[l-1-i]
    let (c0, s0, c1, s1) = load_pre_twiddles(tw.add(i0));
    let (a1, a2) = cmd2(bo, f.0, c0, s0);
    let (a3, a4) = cmd2(be, f.1, c1, s1);
    (
        C(vshrq_n_s32::<1>(a2), vshrq_n_s32::<1>(a1)),
        C(rev(vshrq_n_s32::<1>(a4)), rev(vnegq_s32(vshrq_n_s32::<1>(a3)))),
    )
}

/// The iterations `i = i0, i0 + 2, i0 + 4, i0 + 6` of the pre twiddling loop of `dst_IV`
/// (`i0 + 6 < l / 2 - 1`).
///
/// Per iteration: `(a1, a2) = cplxMultDiv2(x[l-1-i] >> 1, -(x[i] >> 1), tw[i])`,
/// `(a3, a4) = cplxMultDiv2(-(x[l-2-i] >> 1), x[i+1] >> 1, tw[i+1])`; `x[i] = a2`,
/// `x[i+1] = a1`, `x[l-2-i] = a4`, `x[l-1-i] = -a3`.
#[inline(always)]
pub(super) unsafe fn dst_iv_pre_block(x: *mut i32, l: usize, i0: usize, tw: *const FixpWtp) {
    let (f, b) = dst_iv_pre_regs(x, l, i0, tw);
    vst2q_s32(x.add(i0), int32x4x2_t(f.0, f.1));
    vst2q_s32(x.add(l - 8 - i0), int32x4x2_t(b.0, b.1));
}

/// [`dst_iv_pre_block`] without the stores (see [`dct_iv_pre_regs`]).
#[inline(always)]
pub(super) unsafe fn dst_iv_pre_regs(
    x: *const i32,
    l: usize,
    i0: usize,
    tw: *const FixpWtp,
) -> (C, C) {
    let f = vld2q_s32(x.add(i0));
    let b = vld2q_s32(x.add(l - 8 - i0));
    let (be, bo) = (rev(b.0), rev(b.1)); // x[l-2-i], x[l-1-i]
    let (c0, s0, c1, s1) = load_pre_twiddles(tw.add(i0));
    let accu1 = vshrq_n_s32::<1>(bo);
    let accu2 = vnegq_s32(vshrq_n_s32::<1>(f.0));
    let accu3 = vshrq_n_s32::<1>(f.1);
    let accu4 = vnegq_s32(vshrq_n_s32::<1>(be));
    let (a1, a2) = cmd2(accu1, accu2, c0, s0);
    let (a3, a4) = cmd2(accu4, accu3, c1, s1);
    (C(a2, a1), C(rev(a4), rev(vnegq_s32(a3))))
}

/// `fMult(a, b)` per lane, with `b` given as `b << 15` (see the module documentation).
#[inline(always)]
unsafe fn fm(a: int32x4_t, b15: int32x4_t) -> int32x4_t {
    vshlq_n_s32::<1>(vqdmulhq_s32(a, b15))
}

/// `cplxMult(&re, &im, a_re, a_im, c, s)` with `FIXP_SGL` coefficients (given as `<< 15`).
#[inline(always)]
unsafe fn cm(
    a_re: int32x4_t,
    a_im: int32x4_t,
    c15: int32x4_t,
    s15: int32x4_t,
) -> (int32x4_t, int32x4_t) {
    (vsubq_s32(fm(a_re, c15), fm(a_im, s15)), vaddq_s32(fm(a_re, s15), fm(a_im, c15)))
}

/// `(re << 15, im << 15)` of 32-bit words `re | im << 16`.
#[inline(always)]
unsafe fn split15(v: int32x4_t) -> (int32x4_t, int32x4_t) {
    (
        vshrq_n_s32::<1>(vshlq_n_s32::<16>(v)),
        vshrq_n_s32::<1>(vandq_s32(v, vdupq_n_s32(0xFFFF_0000u32 as i32))),
    )
}

/// The pairs `j = j0 .. j0 + 4` of the post twiddling of `dct_IV` (`DST` false) or `dst_IV`
/// (`DST` true), including the special pairs (`K = (l / 2 + 1) / 2`, `l / 2` even):
///
/// - `FIRST` (`j0 = 0`): the pair 0 copies its front values (`dct_IV`: `x[l-1] = -x[1]`,
///   `x[0]` unchanged; `dst_IV`: `x[0] = x[1]`, `x[l-1] = -x[0]`) instead of the rotation with
///   `t[0]`;
/// - `LAST` (`j0 = K - 4`): the pair `K - 1` takes `fMult(B, W_PiFOURTH)` (the tail after the C
///   loop) instead of the rotation with `t[K * step]` (which is not read: its lane is loaded from
///   `t[(K - 1) * step]`).
///
/// Otherwise `1 <= j0` and `j0 + 3 <= K - 2`. See `super::dct_iv_post_pair`.
#[inline(always)]
pub(super) unsafe fn post_block<const DST: bool, const FIRST: bool, const LAST: bool>(
    x: *mut i32,
    l: usize,
    j0: usize,
    t: *const FixpStp,
    step: usize,
) {
    let f = vld2q_s32(x.add(2 * j0));
    let b = vld2q_s32(x.add(l - 8 - 2 * j0));
    post_block_regs::<DST, FIRST, LAST>(C(f.0, f.1), C(b.0, b.1), x, l, j0, t, step);
}

/// [`post_block`] with the values it reads given as complex vectors: `fv` the complex values
/// `j0 .. j0 + 4`, `bv` the complex values `l / 2 - 4 - j0 .. l / 2 - j0` of `x`.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(super) unsafe fn post_block_regs<const DST: bool, const FIRST: bool, const LAST: bool>(
    fv: C,
    bv: C,
    x: *mut i32,
    l: usize,
    j0: usize,
    t: *const FixpStp,
    step: usize,
) {
    let f = int32x4x2_t(fv.0, fv.1);
    let b = int32x4x2_t(bv.0, bv.1);
    let (be, bo) = (rev(b.0), rev(b.1));
    let tw = t as *const i32;
    let mut v = vdupq_n_s32(0);
    v = vld1q_lane_s32::<0>(tw.add(j0 * step), v);
    v = vld1q_lane_s32::<1>(tw.add((j0 + 1) * step), v);
    v = vld1q_lane_s32::<2>(tw.add((j0 + 2) * step), v);
    v = vld1q_lane_s32::<3>(tw.add((j0 + 3) * step), v);
    let w = vld1q_dup_s32(tw.add((j0 + if LAST { 3 } else { 4 }) * step));
    let ((c0, s0), (c1, s1)) = (split15(v), split15(vextq_s32::<1>(v, w)));
    let (a3, a4) = cm(f.1, f.0, c0, s0);
    let (b3, b4) = cm(be, bo, c1, s1);
    // outputs: front even / odd, back even / odd
    let (mut fe, mut fo, mut oe, mut oo) = if DST {
        (a3, vnegq_s32(b4), vnegq_s32(b3), vnegq_s32(a4))
    } else {
        (a4, b3, b4, vnegq_s32(a3))
    };
    if FIRST {
        let m = vreinterpretq_u32_s32(vsetq_lane_s32::<0>(-1, vdupq_n_s32(0)));
        let (ce, co) = if DST { (f.1, vnegq_s32(f.0)) } else { (f.0, vnegq_s32(f.1)) };
        fe = vbslq_s32(m, ce, fe);
        oo = vbslq_s32(m, co, oo);
    }
    if LAST {
        let m = vreinterpretq_u32_s32(vsetq_lane_s32::<3>(-1, vdupq_n_s32(0)));
        let w = vdupq_n_s32((super::W_PI_FOURTH as i32) << 15);
        let (a1, a2) = (fm(be, w), fm(bo, w));
        let (ce, co) = if DST {
            (vsubq_s32(a2, a1), vsubq_s32(vnegq_s32(a1), a2))
        } else {
            (vaddq_s32(a1, a2), vsubq_s32(a1, a2))
        };
        oe = vbslq_s32(m, ce, oe);
        fo = vbslq_s32(m, co, fo);
    }
    vst2q_s32(x.add(2 * j0), int32x4x2_t(fe, fo));
    vst2q_s32(x.add(l - 8 - 2 * j0), int32x4x2_t(rev(oe), rev(oo)));
}

/// The sine table entries `t[idx[j]]` as 32-bit words `re | im << 16`.
#[inline(always)]
unsafe fn gather4(t: *const FixpStp, idx: [usize; 4]) -> int32x4_t {
    let t = t as *const i32;
    let mut v = vdupq_n_s32(0);
    v = vld1q_lane_s32::<0>(t.add(idx[0]), v);
    v = vld1q_lane_s32::<1>(t.add(idx[1]), v);
    v = vld1q_lane_s32::<2>(t.add(idx[2]), v);
    vld1q_lane_s32::<3>(t.add(idx[3]), v)
}

/// `(re, im)` (sign-extended) of 32-bit words `re | im << 16`.
#[inline(always)]
unsafe fn split32(v: int32x4_t) -> (int32x4_t, int32x4_t) {
    (vshrq_n_s32::<16>(vshlq_n_s32::<16>(v)), vshrq_n_s32::<16>(v))
}

/// The coefficients of the iterations `i = i0 .. i0 + 4` of the loops of `dct_III` and `dct_II`
/// (`split`: the first `i` with `2 i >= m / 2`): `t[i * inc]`, `t[(m - i) * inc]`, and
/// `t[index(i)]` with `index(i) = 4 inc i` below `split`, `4 inc (m / 2 - i)` from `split` on
/// (see `super::dct_twiddle_index`), the latter as `(c, s)` swapped in the lanes from `split` on,
/// and the lanes from `split` on as a mask.
///
/// The two branches of the C loops compute `(r, x) = cplxMultDiv2(p, q, c, s)` (below `split`)
/// and `(-r', x') = cplxMultDiv2(q, p, c, s)` with the real part negated (from `split` on). With
/// `c` and `s` swapped the first form gives the products of the second: `x' = p c + q s` and
/// `-r'`, which is `-((q c - p s) >> 16) = (p s - q c + 0xFFFF) >> 16` (the negation of a
/// truncated 64-bit sum): the lanes from `split` on take the bias `0xFFFF` in the real part.
#[allow(clippy::type_complexity)]
#[inline(always)]
unsafe fn dct_ii_iii_twiddles(
    t: *const FixpStp,
    m: usize,
    i0: usize,
    inc: usize,
    split: usize,
) -> (int32x4_t, int32x4_t, (int32x4_t, int32x4_t), (int64x2_t, int64x2_t)) {
    let w1 = gather4(t, [i0 * inc, (i0 + 1) * inc, (i0 + 2) * inc, (i0 + 3) * inc]);
    let w2 =
        gather4(t, [(m - i0) * inc, (m - i0 - 1) * inc, (m - i0 - 2) * inc, (m - i0 - 3) * inc]);
    let k = |i: usize| 4 * inc * if i >= split { m / 2 - i } else { i };
    let w3 = gather4(t, [k(i0), k(i0 + 1), k(i0 + 2), k(i0 + 3)]);
    let iv = vaddq_u32(vdupq_n_u32(i0 as u32), vld1q_u32([0u32, 1, 2, 3].as_ptr()));
    let hi = vcgeq_u32(iv, vdupq_n_u32(split as u32));
    let (c3, s3) = split32(w3);
    let cs = (vbslq_s32(hi, s3, c3), vbslq_s32(hi, c3, s3));
    let him = vreinterpretq_s32_u32(hi);
    let f = vdupq_n_s64(0xFFFF);
    let bias = (vandq_s64(vmovl_s32(vget_low_s32(him)), f), vandq_s64(vmovl_high_s32(him), f));
    (w1, w2, cs, bias)
}

/// `(a * p - b * q + bias) >> 16` per lane, in 64 bits, truncated to 32 bits.
#[inline(always)]
unsafe fn msub16_b(
    a: int32x4_t,
    p: int32x4_t,
    b: int32x4_t,
    q: int32x4_t,
    bias: (int64x2_t, int64x2_t),
) -> int32x4_t {
    let lo = vmlsl_s32(
        vmlal_s32(bias.0, vget_low_s32(a), vget_low_s32(p)),
        vget_low_s32(b),
        vget_low_s32(q),
    );
    let hi = vmlsl_high_s32(vmlal_high_s32(bias.1, a, p), b, q);
    vshrn_high_n_s64::<16>(vshrn_n_s64::<16>(lo), hi)
}

/// The iterations `i = i0 .. i0 + 4` of the loop of `dct_III` (`1 <= i0`, `i0 + 3 < m / 2`;
/// either branch per lane, see [`dct_ii_iii_twiddles`]), reading `x` (`l = 2 m` values) and
/// writing `tmp[2i]`, `tmp[2i+1]`, `tmp[2(m-i)]`, `tmp[2(m-i)+1]`. See `super::dct_iii_iter` for
/// the operations.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(super) unsafe fn dct_iii_block(
    x: *const i32,
    tmp: *mut i32,
    l: usize,
    m: usize,
    i0: usize,
    t: *const FixpStp,
    inc: usize,
    split: usize,
) {
    let (f, b) = dct_iii_block_regs(x, l, m, i0, t, inc, split);
    vst2q_s32(tmp.add(2 * i0), int32x4x2_t(f.0, f.1));
    vst2q_s32(tmp.add(2 * (m - i0 - 3)), int32x4x2_t(b.0, b.1));
}

/// [`dct_iii_block`] without the stores: the complex values `i0 .. i0 + 4` (front) and
/// `m - i0 - 3 .. m - i0 + 1` (back) of `tmp` that it writes, as complex vectors.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(super) unsafe fn dct_iii_block_regs(
    x: *const i32,
    l: usize,
    m: usize,
    i0: usize,
    t: *const FixpStp,
    inc: usize,
    split: usize,
) -> (C, C) {
    let xi = vld1q_s32(x.add(i0));
    let xli = rev(vld1q_s32(x.add(l - i0 - 3)));
    let xmi = vld1q_s32(x.add(m + i0));
    let xm_i = rev(vld1q_s32(x.add(m - i0 - 3)));
    let (w1, w2, (c3, s3), bias) = dct_ii_iii_twiddles(t, m, i0, inc, split);
    let ((c1, s1), (c2, s2)) = (split32(w1), split32(w2));
    let (accu2, accu1) = cmd2(xli, xi, c1, s1);
    let (accu4, accu3) = cmd2(xmi, xm_i, c2, s2);
    let (accu3, accu4) = (vshrq_n_s32::<1>(accu3), vshrq_n_s32::<1>(accu4));
    let p = vsubq_s32(accu3, vshrq_n_s32::<1>(accu1));
    let q = vaddq_s32(vshrq_n_s32::<1>(accu2), accu4);
    let accu6 = msub16_b(p, c3, q, s3, bias);
    let accu5 = madd16(p, s3, q, c3);
    let xr = vshrq_n_s32::<1>(vaddq_s32(vshrq_n_s32::<1>(accu1), accu3));
    let xr2 = vshrq_n_s32::<1>(vsubq_s32(vshrq_n_s32::<1>(accu2), accu4));
    (
        C(vsubq_s32(xr, accu5), vsubq_s32(xr2, accu6)),
        C(rev(vaddq_s32(xr, accu5)), rev(vnegq_s32(vaddq_s32(xr2, accu6)))),
    )
}

/// The output permutation of `dct_III` for `k = k0 .. k0 + 4`: `x[4k] = tmp[2k]`,
/// `x[4k+1] = tmp[l-1-2k]`, `x[4k+2] = tmp[2k+1]`, `x[4k+3] = tmp[l-2-2k]`.
#[inline(always)]
pub(super) unsafe fn dct_iii_post_block(x: *mut i32, tmp: *const i32, l: usize, k0: usize) {
    let f = vld2q_s32(tmp.add(2 * k0));
    let b = vld2q_s32(tmp.add(l - 8 - 2 * k0));
    dct_iii_post_regs(x, k0, C(f.0, f.1), C(b.0, b.1));
}

/// [`dct_iii_post_block`] from the complex values `k0 .. k0 + 4` (`f`) and
/// `l / 2 - 4 - k0 .. l / 2 - k0` (`b`) of `tmp`.
#[inline(always)]
pub(super) unsafe fn dct_iii_post_regs(x: *mut i32, k0: usize, f: C, b: C) {
    vst4q_s32(x.add(4 * k0), int32x4x4_t(f.0, rev(b.1), f.1, rev(b.0)));
}

/// The input permutation of `dct_II` for `i = i0 .. i0 + 4`: `tmp[i] = x[2i] >> 2`,
/// `tmp[l-1-i] = x[2i+1] >> 2`.
#[inline(always)]
pub(super) unsafe fn dct_ii_pre_block(x: *const i32, tmp: *mut i32, l: usize, i0: usize) {
    let v = vld2q_s32(x.add(2 * i0));
    vst1q_s32(tmp.add(i0), vshrq_n_s32::<2>(v.0));
    vst1q_s32(tmp.add(l - 4 - i0), rev(vshrq_n_s32::<2>(v.1)));
}

/// The iterations `i = i0 .. i0 + 4` of the loop of `dct_II` after the FFT (conditions as for
/// [`dct_iii_block`]), reading `tmp` and writing `x[i]`, `x[l-i]`, `x[m+i]`, `x[m-i]`. See
/// `super::dct_ii_iter` for the operations.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(super) unsafe fn dct_ii_block(
    x: *mut i32,
    tmp: *const i32,
    l: usize,
    m: usize,
    i0: usize,
    t: *const FixpStp,
    inc: usize,
    split: usize,
) {
    let f = vld2q_s32(tmp.add(2 * i0)); // tmp[2i], tmp[2i+1]
    let b = vld2q_s32(tmp.add(2 * (m - i0 - 3)));
    dct_ii_block_regs(C(f.0, f.1), C(b.0, b.1), x, l, m, i0, t, inc, split);
}

/// [`dct_ii_block`] from the complex values `i0 .. i0 + 4` (`f`) and `m - i0 - 3 .. m - i0 + 1`
/// (`b`) of `tmp`.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(super) unsafe fn dct_ii_block_regs(
    f: C,
    b: C,
    x: *mut i32,
    l: usize,
    m: usize,
    i0: usize,
    t: *const FixpStp,
    inc: usize,
    split: usize,
) {
    let (be, bo) = (rev(b.0), rev(b.1)); // tmp[2(m-i)], tmp[2(m-i)+1]
    let (fe, fo) = (vshrq_n_s32::<1>(f.0), vshrq_n_s32::<1>(f.1));
    let (be, bo) = (vshrq_n_s32::<1>(be), vshrq_n_s32::<1>(bo));
    let (w1, w2, (c3, s3), bias) = dct_ii_iii_twiddles(t, m, i0, inc, split);
    let a1 = vaddq_s32(fo, bo);
    let a2 = vsubq_s32(be, fe);
    // below split: cplxMultDiv2(a2, a1, w); from split on: -re, im of cplxMultDiv2(a1, a2, w)
    let accu1 = msub16_b(a2, c3, a1, s3, bias);
    let accu2 = madd16(a2, s3, a1, c3);
    let (accu1, accu2) = (vshlq_n_s32::<1>(accu1), vshlq_n_s32::<1>(accu2));
    let a1 = vaddq_s32(fe, be);
    let a2 = vsubq_s32(fo, bo);
    let (c1, s1) = split15(w1);
    let (c2, s2) = split15(w2);
    let (p3, p4) = cm(vaddq_s32(accu1, a2), vaddq_s32(a1, accu2), c1, s1);
    let (q3, q4) = cm(vsubq_s32(accu1, a2), vsubq_s32(a1, accu2), c2, s2);
    vst1q_s32(x.add(i0), p4);
    vst1q_s32(x.add(l - i0 - 3), rev(vnegq_s32(p3)));
    vst1q_s32(x.add(m + i0), vnegq_s32(q3));
    vst1q_s32(x.add(m - i0 - 3), rev(q4));
}
