// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/sf_estim.cpp and libFDK/include/fixpoint_math.h (optimized; no
// counterpart in the reference).

//! AArch64 NEON version of the form factor sum of `FDKaacEnc_FDKaacEnc_CalcFormFactorChannel()`:
//! `sum(sqrtFixp(fixp_abs(spec[j])) >> FORM_FAC_SHIFT)` (wrapping), four lines at a time.
//!
//! Only for the variants whose `sqrtFixp()` is the integer code (`aarch64`, `generic`): per lane,
//! `invSqrtNorm2()` normalizes the operand, interpolates `invSqrtTab` (the correction term is 0
//! for `fract == 0`, so it is added unconditionally), multiplies by `1/sqrt(2)` for an odd
//! exponent, and `sqrtFixp()` scales the result. Every `fMultDiv2(a, b)` = `(a * b) >> 32` is
//! `sqdmulh(a, b) >> 1`, which saturates only for `a = b = MINVAL_DBL`; no product here has two
//! such operands (the fractions are below 2^31, the table values and the normalized operand
//! positive). Lanes of 0 contribute 0 (as `sqrtFixp(0)`); a group with `MINVAL_DBL` (whose
//! `fixp_abs()` stays negative) takes the scalar code.

use super::FORM_FAC_SHIFT;
use core::arch::aarch64::*;
use fdk_aac_rs_fdk::common_fix::{fixp_abs, FixpDbl};
use fdk_aac_rs_fdk::fixpoint_math::{sqrt_fixp, INV_SQRT_TAB};

/// `sum(sqrtFixp(fixp_abs(spec[j])) >> FORM_FAC_SHIFT)` (wrapping).
pub(super) fn form_factor_sum(spec: &[FixpDbl]) -> FixpDbl {
    let mut sum: FixpDbl = 0;
    let mut vsum;
    // SAFETY: NEON is part of the AArch64 baseline; the loads read the chunks of 4 lines and
    // `INV_SQRT_TAB[index..index + 3]` with `index <= 127` (130 entries).
    unsafe {
        vsum = vdupq_n_s32(0);
        let tab = INV_SQRT_TAB.as_ptr();
        let fmd2 = |a: int32x4_t, b: int32x4_t| vshrq_n_s32::<1>(vqdmulhq_s32(a, b));
        for g in spec.chunks_exact(4) {
            let op = vabsq_s32(vld1q_s32(g.as_ptr()));
            if vminvq_s32(op) < 0 {
                for &x in g {
                    sum = sum.wrapping_add(sqrt_fixp(fixp_abs(x)) >> FORM_FAC_SHIFT);
                }
                continue;
            }
            // invSqrtNorm2(op, &shift); lanes of 0 are cleared at the end
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
            let one_minus = vreinterpretq_s32_u32(vsubq_u32(
                vdupq_n_u32(0x8000_0000),
                vreinterpretq_u32_s32(fract),
            ));
            let fract2 = vshlq_n_s32::<1>(fmd2(fract, one_minus));
            let diff2 = vsubq_s32(diff, vsubq_s32(t2, t1));
            let reg1 = vaddq_s32(reg1, fmd2(fract2, diff2));
            let odd = vtstq_s32(shift, vdupq_n_s32(1));
            let reg1 = vbslq_s32(odd, vshlq_n_s32::<2>(fmd2(reg1, vdupq_n_s32(0x5A82_7999))), reg1);
            let e = vshrq_n_s32::<1>(shift);
            // sqrtFixp: fMultDiv2(op << (e - 1), tmp_inv) << 2
            let r = vshlq_n_s32::<2>(fmd2(vshlq_s32(op, vsubq_s32(e, vdupq_n_s32(1))), reg1));
            let r = vshrq_n_s32::<{ FORM_FAC_SHIFT }>(r);
            vsum = vaddq_s32(vsum, vbicq_s32(r, vreinterpretq_s32_u32(vceqzq_s32(op))));
        }
        sum = sum.wrapping_add(vaddvq_s32(vsum));
    }
    for &x in spec.chunks_exact(4).remainder() {
        sum = sum.wrapping_add(sqrt_fixp(fixp_abs(x)) >> FORM_FAC_SHIFT);
    }
    sum
}
