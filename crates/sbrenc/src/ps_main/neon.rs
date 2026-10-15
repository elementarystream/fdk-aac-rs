// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/ps_main.cpp and libFDK/include/fixpoint_math.h (optimized; no
// counterpart in the reference).

//! AArch64 NEON version of the hybrid band loop of `DownmixPSQmfData()` for the `aarch64`
//! variant, four bands at a time; every lane computes what [`super::downmix_band`] computes:
//!
//! - the normalization by the unsigned maximum of the four absolute values (`abs`, `umax`,
//!   `clz`), the energies `fPow2Div2(x)` = `sqdmulh(x, x) >> 1` and the cross terms
//!   `fMult(x, y)` = `sqdmulh(x, y)` (`(x * y) >> 31` on `aarch64`); the shifted values are below
//!   2^30 in magnitude unless the shift is 0, and a group with an input of `MINVAL_DBL` (the only
//!   operand that makes `sqdmulh` saturate) takes the scalar code;
//! - `schur_div(num, denum, 16)` of the bands that take the division: `floor(a * 2^15 / b)`
//!   (`a = num >> 1`, `b = denum >> 1`, see [`crate::sbr_misc::schur_div_fast`]) computed in
//!   double precision: `a * 2^15 < 2^45` and `b` are exact, and the correctly rounded quotient
//!   lies within `2^-38` of `a * 2^15 / b`, which is at least `1 / b > 2^-31` below the next
//!   integer, so truncating it gives the floor. `a == 0` gives 0, `a == b` sets all 15 bits; a
//!   group with another band outside `0 < a < b` takes the scalar code;
//! - `sqrtFixp()` with `invSqrtNorm2()` (as in `sf_estim/neon.rs`), the odd-shift correction and
//!   the final shifts as selects and register shifts modulo 32.
//!
//! The last `71 % 4` bands use the scalar code.

use super::downmix_band;
use core::arch::aarch64::*;
use fdk_aac_rs_fdk::common_fix::{FixpDbl, MAXVAL_DBL};
use fdk_aac_rs_fdk::fixpoint_math::INV_SQRT_TAB;

/// [`super::downmix_bands`] on NEON.
pub(super) fn downmix_bands(
    rows: [&[FixpDbl; 71]; 4],
    out_re: &mut [FixpDbl; 71],
    out_im: &mut [FixpDbl; 71],
    dyn_qmf_scale: i32,
) {
    let [lr, li, rr, ri] = rows;
    for g in 0..71 / 4 {
        let k = 4 * g;
        // SAFETY: NEON is part of the AArch64 baseline; the loads and stores access bands
        // k..k + 4 <= 68 of the 71-entry rows.
        let done = unsafe {
            group(
                [
                    vld1q_s32(lr.as_ptr().add(k)),
                    vld1q_s32(li.as_ptr().add(k)),
                    vld1q_s32(rr.as_ptr().add(k)),
                    vld1q_s32(ri.as_ptr().add(k)),
                ],
                dyn_qmf_scale,
            )
            .map(|(re, im)| {
                vst1q_s32(out_re.as_mut_ptr().add(k), re);
                vst1q_s32(out_im.as_mut_ptr().add(k), im);
            })
            .is_some()
        };
        if !done {
            for j in k..k + 4 {
                (out_re[j], out_im[j]) = downmix_band(lr[j], li[j], rr[j], ri[j], dyn_qmf_scale);
            }
        }
    }
    for j in 71 / 4 * 4..71 {
        (out_re[j], out_im[j]) = downmix_band(lr[j], li[j], rr[j], ri[j], dyn_qmf_scale);
    }
}

/// Four bands, or `None` if a band needs the scalar code.
#[inline(always)]
unsafe fn group(x: [int32x4_t; 4], dyn_qmf_scale: i32) -> Option<(int32x4_t, int32x4_t)> {
    let minval = vdupq_n_s32(i32::MIN);
    let mut bad = vdupq_n_u32(0);
    for v in x {
        bad = vorrq_u32(bad, vceqq_s32(v, minval));
    }
    // sc = max(0, clz(umax of |x|) - 2)
    let m = vmaxq_u32(
        vmaxq_u32(vreinterpretq_u32_s32(vabsq_s32(x[0])), vreinterpretq_u32_s32(vabsq_s32(x[1]))),
        vmaxq_u32(vreinterpretq_u32_s32(vabsq_s32(x[2])), vreinterpretq_u32_s32(vabsq_s32(x[3]))),
    );
    let sc =
        vmaxq_s32(vsubq_s32(vreinterpretq_s32_u32(vclzq_u32(m)), vdupq_n_s32(2)), vdupq_n_s32(0));
    let [lr, li, rr, ri] = x.map(|v| vshlq_s32(v, sc));
    let dyn_scale = vminq_s32(vsubq_s32(sc, vdupq_n_s32(dyn_qmf_scale)), vdupq_n_s32(31));

    let pow2div2 = |v: int32x4_t| vshrq_n_s32::<1>(vqdmulhq_s32(v, v));
    let ssf =
        vaddq_s32(vaddq_s32(pow2div2(lr), pow2div2(li)), vaddq_s32(pow2div2(rr), pow2div2(ri)));
    let tsf = vabsq_s32(vaddq_s32(vaddq_s32(ssf, vqdmulhq_s32(lr, rr)), vqdmulhq_s32(li, ri)));
    let take_div = vcltq_s32(vshrq_n_s32::<1>(ssf), vqdmulhq_s32(vdupq_n_s32(MAXVAL_DBL), tsf));

    // sc_num = CountLeadingBits(ssf), sc_denum = clz(tsf) - 1 (0 for 0)
    let sc_num = vbicq_s32(vclsq_s32(ssf), vreinterpretq_s32_u32(vceqzq_s32(ssf)));
    let sc_denum = vbicq_s32(
        vsubq_s32(vreinterpretq_s32_u32(vclzq_u32(vreinterpretq_u32_s32(tsf))), vdupq_n_s32(1)),
        vreinterpretq_s32_u32(vceqzq_s32(tsf)),
    );
    let mut sc = vsubq_s32(sc_denum, sc_num);
    let mask31 = vdupq_n_s32(31);
    // num = (ssf << sc_num) >> 1, denum = tsf << sc_denum (register shifts modulo 32)
    let num = vshrq_n_s32::<1>(vshlq_s32(ssf, vandq_s32(sc_num, mask31)));
    let denum = vshlq_s32(tsf, vandq_s32(sc_denum, mask31));
    let a = vshrq_n_s32::<1>(num);
    let b = vshrq_n_s32::<1>(denum);
    let a_zero = vceqzq_s32(a);
    let a_eq_b = vandq_u32(vceqq_s32(a, b), vcgtzq_s32(a));
    let a_in = vandq_u32(vcgtzq_s32(a), vcltq_s32(a, b));
    let ok = vorrq_u32(vorrq_u32(a_zero, a_eq_b), a_in);
    bad = vorrq_u32(bad, vbicq_u32(take_div, ok));
    if vmaxvq_u32(bad) != 0 {
        return None;
    }
    // floor(a * 2^15 / b) in double precision (lanes with b <= 0 are discarded)
    let bf = vmaxq_s32(b, vdupq_n_s32(1));
    let div2 = |a2: int32x2_t, b2: int32x2_t| -> int32x2_t {
        let af = vmulq_n_f64(vcvtq_f64_s64(vmovl_s32(a2)), 32768.0);
        let q = vdivq_f64(af, vcvtq_f64_s64(vmovl_s32(b2)));
        vmovn_s64(vcvtq_s64_f64(q))
    };
    let q = vcombine_s32(
        div2(vget_low_s32(a), vget_low_s32(bf)),
        div2(vget_high_s32(a), vget_high_s32(bf)),
    );
    let q = vbslq_s32(a_eq_b, vdupq_n_s32(0x7FFF), q);
    let q = vbicq_s32(q, vreinterpretq_s32_u32(a_zero));
    let mut quot = vshlq_n_s32::<16>(q);
    // prevent odd scaling for next sqrt calculation
    let odd = vtstq_s32(sc, vdupq_n_s32(1));
    sc = vsubq_s32(sc, vreinterpretq_s32_u32(odd)); // sc += 1 where odd
    quot = vbslq_s32(odd, vshrq_n_s32::<1>(quot), quot);
    let root = sqrt_fixp4(quot);
    // stereoScaleFactor <<= (sc >> 1) (modulo 32)
    let root = vshlq_s32(root, vandq_s32(vshrq_n_s32::<1>(sc), mask31));
    let ssf = vbslq_s32(take_div, root, vdupq_n_s32(MAXVAL_DBL));

    // fMultDiv2(stereoScaleFactor, l + r) >> dynScale (modulo 32)
    let sum_re = vaddq_s32(lr, rr);
    let sum_im = vaddq_s32(li, ri);
    let sat = vandq_u32(
        vceqq_s32(ssf, minval),
        vorrq_u32(vceqq_s32(sum_re, minval), vceqq_s32(sum_im, minval)),
    );
    if vmaxvq_u32(sat) != 0 {
        return None;
    }
    let shift = vnegq_s32(vandq_s32(dyn_scale, mask31));
    let re = vshlq_s32(vshrq_n_s32::<1>(vqdmulhq_s32(ssf, sum_re)), shift);
    let im = vshlq_s32(vshrq_n_s32::<1>(vqdmulhq_s32(ssf, sum_im)), shift);
    Some((re, im))
}

/// `sqrtFixp()` of four values `0 <= op < 2^31` (integer code of the `aarch64` variant):
/// `fMultDiv2(op << (e - 1), invSqrtNorm2(op, &e)) << 2`, 0 for `op == 0`.
#[inline(always)]
unsafe fn sqrt_fixp4(op: int32x4_t) -> int32x4_t {
    let fmd2 = |a: int32x4_t, b: int32x4_t| vshrq_n_s32::<1>(vqdmulhq_s32(a, b));
    let tab = INV_SQRT_TAB.as_ptr();
    let s = vsubq_s32(vclzq_s32(op), vdupq_n_s32(1));
    let val = vshlq_s32(op, s);
    let shift = vaddq_s32(s, vdupq_n_s32(2));
    let index = vandq_u32(vshrq_n_u32::<23>(vreinterpretq_u32_s32(val)), vdupq_n_u32(0x7f));
    let fract = vshlq_n_s32::<8>(vandq_s32(val, vdupq_n_s32(0x007F_FFFF)));
    let p: [*const i32; 4] = [
        tab.add(vgetq_lane_u32::<0>(index) as usize),
        tab.add(vgetq_lane_u32::<1>(index) as usize),
        tab.add(vgetq_lane_u32::<2>(index) as usize),
        tab.add(vgetq_lane_u32::<3>(index) as usize),
    ];
    let gather = |o: usize| -> int32x4_t {
        let mut v = vdupq_n_s32(0);
        v = vld1q_lane_s32::<0>(p[0].add(o), v);
        v = vld1q_lane_s32::<1>(p[1].add(o), v);
        v = vld1q_lane_s32::<2>(p[2].add(o), v);
        vld1q_lane_s32::<3>(p[3].add(o), v)
    };
    let (t0, t1, t2) = (gather(0), gather(1), gather(2));
    let diff = vsubq_s32(t1, t0);
    let reg1 = vaddq_s32(t0, vshlq_n_s32::<1>(fmd2(diff, fract)));
    let one_minus =
        vreinterpretq_s32_u32(vsubq_u32(vdupq_n_u32(0x8000_0000), vreinterpretq_u32_s32(fract)));
    let fract2 = vshlq_n_s32::<1>(fmd2(fract, one_minus));
    let diff2 = vsubq_s32(diff, vsubq_s32(t2, t1));
    let reg1 = vaddq_s32(reg1, fmd2(fract2, diff2));
    let odd = vtstq_s32(shift, vdupq_n_s32(1));
    let reg1 = vbslq_s32(odd, vshlq_n_s32::<2>(fmd2(reg1, vdupq_n_s32(0x5A82_7999))), reg1);
    let e = vshrq_n_s32::<1>(shift);
    let r = vshlq_n_s32::<2>(fmd2(vshlq_s32(op, vsubq_s32(e, vdupq_n_s32(1))), reg1));
    vbicq_s32(r, vreinterpretq_s32_u32(vceqzq_s32(op)))
}
