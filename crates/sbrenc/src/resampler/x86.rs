// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/resampler.cpp (optimized; no counterpart in the reference).

//! x86 AVX2 version of the biquad cascade of `AdvanceFilter()` (selected at run time,
//! [`crate::simd::avx2`]).
//!
//! The same wavefront as the AArch64 version (`neon.rs`): lane `i` of the state vectors is
//! section `i`, and at step `t` it filters sample `t - i`, so that all sections of a step are
//! independent and the output of section `i` at step `t` is the input of section `i + 1` at step
//! `t + 1` (a lane shift across the eight lanes of a vector, and from lane 7 of one vector into
//! lane 0 of the next). During the first and the last `sections - 1` steps the lanes outside the
//! block keep their states (masked updates).
//!
//! Each lane computes what the straight port computes for its section and sample:
//!
//! ```text
//! y = x + fMult(x1, B1) + fMult(x2, B2) - fMult(y1, A1) - fMult(y2, A2)   (wrapping)
//! ```
//!
//! `fMult(FIXP_DBL, FIXP_SGL)` is `M(a, c) << 1` with `M(a, c) = floor(a * c / 2^16)` on every
//! variant: the high word of the 64-bit product `a * (c << 16)` (`vpmuldq` of the even and of the
//! odd lanes). The wrapping sum does not depend on the order.

use super::{LpFilter, BIQUAD_COEFSTEP, MAXNR_SECTIONS};
#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;
use fdk_aac_rs_fdk::common_fix::{FixpDbl, DFRACT_BITS};
use fdk_aac_rs_fdk::machine_type::{IntPcm, SAMPLE_BITS};

/// Filters `input` (the samples of the block) through the cascade of `f` and stores the output
/// of the last section for every sample in `ys`. Requires `1 <= sections <= MAXNR_SECTIONS`,
/// `f.coeffa` with `4 * sections` coefficients and `ys.len() == input.len()`.
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn cascade(
    f: &mut LpFilter,
    sections: usize,
    input: &[IntPcm],
    ys: &mut [FixpDbl],
) {
    if sections <= 8 {
        cascade_v::<1>(f, sections, input, ys)
    } else {
        cascade_v::<2>(f, sections, input, ys)
    }
}

/// The state vectors of `V * 8` lanes.
struct Lanes<const V: usize> {
    /// the input of the current step
    x: [__m256i; V],
    /// the inputs and outputs of the previous two samples, shifted left by 1
    x1: [__m256i; V],
    x2: [__m256i; V],
    y1: [__m256i; V],
    y2: [__m256i; V],
}

/// One coefficient per lane in the forms of [`mul`].
#[derive(Clone, Copy)]
struct Coef {
    /// `c << 16` (the even lanes)
    even: __m256i,
    /// `c << 16` of the odd lanes in the low words of the 64-bit lanes
    odd: __m256i,
}

struct Coeffs<const V: usize> {
    b1: [Coef; V],
    b2: [Coef; V],
    a1: [Coef; V],
    a2: [Coef; V],
}

/// `floor(a * c / 2^16)` of every lane.
#[inline(always)]
unsafe fn mul(a: __m256i, c: &Coef) -> __m256i {
    let even = _mm256_srli_epi64::<32>(_mm256_mul_epi32(a, c.even));
    let odd = _mm256_mul_epi32(_mm256_srli_epi64::<32>(a), c.odd);
    _mm256_blend_epi32::<0b1010_1010>(even, odd)
}

#[inline(always)]
unsafe fn cascade_v<const V: usize>(
    f: &mut LpFilter,
    sections: usize,
    input: &[IntPcm],
    ys: &mut [FixpDbl],
) {
    debug_assert!((1..=MAXNR_SECTIONS).contains(&sections) && sections <= 8 * V);
    debug_assert_eq!(input.len(), ys.len());
    let coeffa = f.coeffa.unwrap_or(&[]);
    let s1 = f.ptr as usize & 1;
    let s2 = s1 ^ 1;
    let mut c = [[0i32; 16]; 4];
    let mut st = [[0i32; 16]; 4];
    for i in 0..sections {
        for k in 0..BIQUAD_COEFSTEP {
            c[k][i] = coeffa[i * BIQUAD_COEFSTEP + k] as i32;
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
    // opaque to the optimizer, which would otherwise expand the signed products of the known
    // coefficient forms into slower unsigned multiplication sequences
    let k = core::hint::black_box(Coeffs::<V> {
        b1: coefs(&c[0]),
        b2: coefs(&c[1]),
        a1: coefs(&c[2]),
        a2: coefs(&c[3]),
    });
    let mut l = Lanes::<V> {
        x: [_mm256_setzero_si256(); V],
        x1: load(&st[0]),
        x2: load(&st[1]),
        y1: load(&st[2]),
        y2: load(&st[3]),
    };
    let mut lane_index = [_mm256_setzero_si256(); V];
    for (v, li) in lane_index.iter_mut().enumerate() {
        *li = _mm256_add_epi32(
            _mm256_set1_epi32(8 * v as i32),
            _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7),
        );
    }
    l.x[0] = _mm256_set1_epi32(sample(0));
    let mut out = [0i32; 8];
    let steps = n + last;
    let steady_start = last.min(steps);
    let steady_end = n.max(steady_start);
    for t in 0..steady_start {
        masked(&mut l, &k, &lane_index, t, n, sample(t + 1));
    }
    let no_mask = [_mm256_setzero_si256(); V];
    for t in steady_start..steady_end {
        let yn = step::<V, false>(&mut l, &k, &no_mask, sample(t + 1));
        _mm256_storeu_si256(
            out.as_mut_ptr() as *mut __m256i,
            if last >= 8 { yn[V - 1] } else { yn[0] },
        );
        ys[t - last] = out[last % 8];
    }
    for t in steady_end..steps {
        let yn = masked(&mut l, &k, &lane_index, t, n, sample(t + 1));
        _mm256_storeu_si256(
            out.as_mut_ptr() as *mut __m256i,
            if last >= 8 { yn[V - 1] } else { yn[0] },
        );
        ys[t - last] = out[last % 8];
    }
    let mut st = [[0i32; 16]; 4];
    for v in 0..V {
        _mm256_storeu_si256(st[0].as_mut_ptr().add(8 * v) as *mut __m256i, l.x1[v]);
        _mm256_storeu_si256(st[1].as_mut_ptr().add(8 * v) as *mut __m256i, l.x2[v]);
        _mm256_storeu_si256(st[2].as_mut_ptr().add(8 * v) as *mut __m256i, l.y1[v]);
        _mm256_storeu_si256(st[3].as_mut_ptr().add(8 * v) as *mut __m256i, l.y2[v]);
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

/// Loads 16 words as `V` vectors.
#[inline(always)]
unsafe fn load<const V: usize>(a: &[i32; 16]) -> [__m256i; V] {
    let mut r = [_mm256_setzero_si256(); V];
    for (v, x) in r.iter_mut().enumerate() {
        *x = _mm256_loadu_si256(a.as_ptr().add(8 * v) as *const __m256i);
    }
    r
}

/// The coefficients `a` (16 words) in the forms of [`mul`].
#[inline(always)]
unsafe fn coefs<const V: usize>(a: &[i32; 16]) -> [Coef; V] {
    let c = load::<V>(a);
    let mut r = [Coef { even: c[0], odd: c[0] }; V];
    for (x, &c) in r.iter_mut().zip(&c) {
        let c16 = _mm256_slli_epi32::<16>(c);
        *x = Coef { even: c16, odd: _mm256_srli_epi64::<32>(c16) };
    }
    r
}

/// A masked step `t` of a block of `n` samples: lanes are active while `0 <= t - i < n`.
#[inline(always)]
unsafe fn masked<const V: usize>(
    l: &mut Lanes<V>,
    k: &Coeffs<V>,
    lane_index: &[__m256i; V],
    t: usize,
    n: usize,
    next: i32,
) -> [__m256i; V] {
    let tv = _mm256_set1_epi32(t as i32 + 1);
    let lo = _mm256_set1_epi32(t as i32 - n as i32);
    let mut mask = [_mm256_setzero_si256(); V];
    for (m, &li) in mask.iter_mut().zip(lane_index) {
        *m = _mm256_and_si256(_mm256_cmpgt_epi32(tv, li), _mm256_cmpgt_epi32(li, lo));
    }
    step::<V, true>(l, k, &mask, next)
}

/// One step of the wavefront: returns the new outputs of all lanes and shifts them (with the next
/// input sample in lane 0) into the inputs of the next step.
#[inline(always)]
unsafe fn step<const V: usize, const MASKED: bool>(
    l: &mut Lanes<V>,
    k: &Coeffs<V>,
    mask: &[__m256i; V],
    next: i32,
) -> [__m256i; V] {
    // unrolled with constant indices, so that the states stay in registers
    let mut yn = [_mm256_setzero_si256(); V];
    yn[0] = section::<V, MASKED>(l, k, mask, 0);
    if V > 1 {
        yn[1] = section::<V, MASKED>(l, k, mask, 1);
    }
    // lane i of the next input is lane i - 1 of the outputs (lane 7 of the previous vector, or
    // the next sample for lane 0)
    let rot = _mm256_setr_epi32(7, 0, 1, 2, 3, 4, 5, 6);
    let r0 = _mm256_permutevar8x32_epi32(yn[0], rot);
    l.x[0] = _mm256_blend_epi32::<1>(r0, _mm256_set1_epi32(next));
    if V > 1 {
        let r1 = _mm256_permutevar8x32_epi32(yn[1], rot);
        l.x[1] = _mm256_blend_epi32::<1>(r1, r0);
    }
    yn
}

/// The sections of vector `v` in one step: returns their outputs and updates their states.
#[inline(always)]
unsafe fn section<const V: usize, const MASKED: bool>(
    l: &mut Lanes<V>,
    k: &Coeffs<V>,
    mask: &[__m256i; V],
    v: usize,
) -> __m256i {
    // x + 2 * (M(x1, B1) + M(x2, B2) - M(y2, A2)) - 2 * M(y1, A1)
    let q = _mm256_sub_epi32(
        _mm256_add_epi32(mul(l.x1[v], &k.b1[v]), mul(l.x2[v], &k.b2[v])),
        mul(l.y2[v], &k.a2[v]),
    );
    let t = _mm256_add_epi32(l.x[v], _mm256_slli_epi32::<1>(q));
    let yn = _mm256_sub_epi32(t, _mm256_slli_epi32::<1>(mul(l.y1[v], &k.a1[v])));
    let x1 = _mm256_slli_epi32::<1>(l.x[v]);
    let y1 = _mm256_slli_epi32::<1>(yn);
    if MASKED {
        l.x2[v] = _mm256_blendv_epi8(l.x2[v], l.x1[v], mask[v]);
        l.x1[v] = _mm256_blendv_epi8(l.x1[v], x1, mask[v]);
        l.y2[v] = _mm256_blendv_epi8(l.y2[v], l.y1[v], mask[v]);
        l.y1[v] = _mm256_blendv_epi8(l.y1[v], y1, mask[v]);
    } else {
        l.x2[v] = l.x1[v];
        l.x1[v] = x1;
        l.y2[v] = l.y1[v];
        l.y1[v] = y1;
    }
    yn
}
