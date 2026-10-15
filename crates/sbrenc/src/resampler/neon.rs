// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/resampler.cpp (optimized; no counterpart in the reference).

//! AArch64 NEON version of the biquad cascade of `AdvanceFilter()`.
//!
//! The sections of the cascade run as a wavefront: lane `i` of the state vectors is section `i`,
//! and at step `t` it filters sample `t - i`, so that all sections of a step are independent and
//! the output of section `i` at step `t` is the input of section `i + 1` at step `t + 1` (a lane
//! shift). During the first and the last `sections - 1` steps the lanes outside the block keep
//! their states (masked updates).
//!
//! Each lane computes what the straight port computes for its section and sample:
//!
//! ```text
//! y = x + fMult(x1, B1) + fMult(x2, B2) - fMult(y1, A1) - fMult(y2, A2)   (wrapping)
//! ```
//!
//! where `x1`, `x2` (`y1`, `y2`) are the input (output) of the section at the two previous
//! samples, shifted left by 1 (wrapping) as the reference stores them (`states`).
//! `fMult(FIXP_DBL, FIXP_SGL)` is `((a * c) >> 16) << 1` on every variant, i.e.
//! `floor(a * c / 2^16) << 1` = `sqdmulh(a, c << 15) << 1` (`sqdmulh` cannot saturate: `c << 15`
//! is never `MINVAL_DBL`). The wrapping sum does not depend on the order: the term of `y1` is
//! subtracted last, which keeps the recurrence of a section short.

use super::{LpFilter, BIQUAD_COEFSTEP, MAXNR_SECTIONS};
use core::arch::aarch64::*;
use fdk_aac_rs_fdk::common_fix::{FixpDbl, DFRACT_BITS};
use fdk_aac_rs_fdk::machine_type::{IntPcm, SAMPLE_BITS};

/// Filters `input` (the samples of the block) through the cascade of `f` and stores the output
/// of the last section for every sample in `ys`. Requires `1 <= sections <= MAXNR_SECTIONS`,
/// `f.coeffa` with `4 * sections` coefficients and `ys.len() == input.len()`.
pub(super) fn cascade(f: &mut LpFilter, sections: usize, input: &[IntPcm], ys: &mut [FixpDbl]) {
    match sections.div_ceil(4) {
        1 => cascade_v::<1>(f, sections, input, ys),
        2 => cascade_v::<2>(f, sections, input, ys),
        3 => cascade_v::<3>(f, sections, input, ys),
        _ => cascade_v::<4>(f, sections, input, ys),
    }
}

/// The state vectors of `V * 4` lanes.
struct Lanes<const V: usize> {
    /// the input of the current step
    x: [int32x4_t; V],
    /// the inputs and outputs of the previous two samples, shifted left by 1
    x1: [int32x4_t; V],
    x2: [int32x4_t; V],
    y1: [int32x4_t; V],
    y2: [int32x4_t; V],
}

/// The coefficients `<< 15` per lane (0 past the sections).
struct Coeffs<const V: usize> {
    b1: [int32x4_t; V],
    b2: [int32x4_t; V],
    a1: [int32x4_t; V],
    a2: [int32x4_t; V],
}

#[inline(always)]
fn cascade_v<const V: usize>(
    f: &mut LpFilter,
    sections: usize,
    input: &[IntPcm],
    ys: &mut [FixpDbl],
) {
    debug_assert!((1..=MAXNR_SECTIONS).contains(&sections) && sections <= 4 * V);
    debug_assert_eq!(input.len(), ys.len());
    let coeffa = f.coeffa.unwrap_or(&[]);
    let s1 = f.ptr as usize & 1;
    let s2 = s1 ^ 1;
    let mut c = [[0i32; 16]; 4];
    let mut st = [[0i32; 16]; 4];
    for i in 0..sections {
        for k in 0..BIQUAD_COEFSTEP {
            c[k][i] = (coeffa[i * BIQUAD_COEFSTEP + k] as i32) << 15;
        }
        st[0][i] = f.states[i][s1];
        st[1][i] = f.states[i][s2];
        st[2][i] = f.states[i + 1][s1];
        st[3][i] = f.states[i + 1][s2];
    }
    let n = input.len();
    let last = sections - 1;
    let shift = DFRACT_BITS - SAMPLE_BITS - super::BIQUAD_SCALE;
    let sample = |t: usize| -> i32 {
        if t < n {
            (input[t] as FixpDbl) << shift
        } else {
            0
        }
    };
    // SAFETY: NEON is part of the AArch64 baseline; the loads and stores access arrays of 16
    // words at offsets 4 * v < 16.
    unsafe {
        let load = |a: &[i32; 16]| -> [int32x4_t; V] {
            core::array::from_fn(|v| vld1q_s32(a.as_ptr().add(4 * v)))
        };
        let k = Coeffs::<V> { b1: load(&c[0]), b2: load(&c[1]), a1: load(&c[2]), a2: load(&c[3]) };
        let mut l = Lanes::<V> {
            x: [vdupq_n_s32(0); V],
            x1: load(&st[0]),
            x2: load(&st[1]),
            y1: load(&st[2]),
            y2: load(&st[3]),
        };
        let lane_index: [int32x4_t; V] = core::array::from_fn(|v| {
            vaddq_s32(vdupq_n_s32(4 * v as i32), vld1q_s32([0, 1, 2, 3].as_ptr()))
        });
        l.x[0] = vdupq_n_s32(sample(0));
        let mut out = [0i32; 4];
        let steps = n + last;
        // masked steps: lanes are active while 0 <= t - i < n
        let masked = |l: &mut Lanes<V>, t: usize| -> [int32x4_t; V] {
            let tv = vdupq_n_s32(t as i32);
            let lo = vdupq_n_s32(t as i32 - n as i32);
            let mask: [uint32x4_t; V] = core::array::from_fn(|v| {
                vandq_u32(vcleq_s32(lane_index[v], tv), vcgtq_s32(lane_index[v], lo))
            });
            step::<V, true>(l, &k, &mask, sample(t + 1))
        };
        let steady_start = last.min(steps);
        let steady_end = n.max(steady_start);
        for t in 0..steady_start {
            masked(&mut l, t);
        }
        let no_mask = [vdupq_n_u32(0); V];
        for t in steady_start..steady_end {
            let yn = step::<V, false>(&mut l, &k, &no_mask, sample(t + 1));
            vst1q_s32(out.as_mut_ptr(), yn[last / 4]);
            ys[t - last] = out[last % 4];
        }
        for t in steady_end..steps {
            let yn = masked(&mut l, t);
            vst1q_s32(out.as_mut_ptr(), yn[last / 4]);
            ys[t - last] = out[last % 4];
        }
        let mut st = [[0i32; 16]; 4];
        for v in 0..V {
            vst1q_s32(st[0].as_mut_ptr().add(4 * v), l.x1[v]);
            vst1q_s32(st[1].as_mut_ptr().add(4 * v), l.x2[v]);
            vst1q_s32(st[2].as_mut_ptr().add(4 * v), l.y1[v]);
            vst1q_s32(st[3].as_mut_ptr().add(4 * v), l.y2[v]);
        }
        // after n samples, ptr toggled n times: states[.][ptr'] is the latest sample
        let p1 = (f.ptr as usize ^ n) & 1;
        let p2 = p1 ^ 1;
        f.states[0][p1] = st[0][0];
        f.states[0][p2] = st[1][0];
        for i in 0..sections {
            f.states[i + 1][p1] = st[2][i];
            f.states[i + 1][p2] = st[3][i];
        }
        f.ptr ^= (n & 1) as i32;
    }
}

/// One step of the wavefront: returns the new outputs of all lanes and shifts them (with the next
/// input sample in lane 0) into the inputs of the next step.
#[inline(always)]
unsafe fn step<const V: usize, const MASKED: bool>(
    l: &mut Lanes<V>,
    k: &Coeffs<V>,
    mask: &[uint32x4_t; V],
    next: i32,
) -> [int32x4_t; V] {
    let mut yn = [vdupq_n_s32(0); V];
    for v in 0..V {
        // x + 2 * (sqdmulh(x1, B1) + sqdmulh(x2, B2) - sqdmulh(y2, A2)) - 2 * sqdmulh(y1, A1)
        let q = vsubq_s32(
            vaddq_s32(vqdmulhq_s32(l.x1[v], k.b1[v]), vqdmulhq_s32(l.x2[v], k.b2[v])),
            vqdmulhq_s32(l.y2[v], k.a2[v]),
        );
        let t = vaddq_s32(l.x[v], vshlq_n_s32::<1>(q));
        yn[v] = vsubq_s32(t, vshlq_n_s32::<1>(vqdmulhq_s32(l.y1[v], k.a1[v])));
        let x1 = vshlq_n_s32::<1>(l.x[v]);
        let y1 = vshlq_n_s32::<1>(yn[v]);
        if MASKED {
            l.x2[v] = vbslq_s32(mask[v], l.x1[v], l.x2[v]);
            l.x1[v] = vbslq_s32(mask[v], x1, l.x1[v]);
            l.y2[v] = vbslq_s32(mask[v], l.y1[v], l.y2[v]);
            l.y1[v] = vbslq_s32(mask[v], y1, l.y1[v]);
        } else {
            l.x2[v] = l.x1[v];
            l.x1[v] = x1;
            l.y2[v] = l.y1[v];
            l.y1[v] = y1;
        }
    }
    l.x[0] = vextq_s32::<3>(vdupq_n_s32(next), yn[0]);
    for v in 1..V {
        l.x[v] = vextq_s32::<3>(yn[v - 1], yn[v]);
    }
    yn
}
