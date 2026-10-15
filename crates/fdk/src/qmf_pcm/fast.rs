// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized prototype filters of libFDK/include/qmf_pcm.h (targets other than aarch64).

//! Portable kernels of the QMF analysis prototype filters that the compiler vectorizes (with
//! AVX2 if available, see [`crate::simd`]), for the targets without the NEON kernels.
//!
//! The arithmetic is the same on every variant: `fMultDiv2(FIXP_PFT, FIXP_QAS)` is the exact
//! 32-bit product for 16-bit states (`fixmuldiv2_SS`) and `(c * x) >> 16` for 32-bit states
//! (`fixmuldiv2_SD`), computed exactly in 32-bit lanes as
//! `(x >> 16) * c + (((x & 0xFFFF) * c) >> 16)` (with `x = xh * 2^16 + xl`,
//! `(c * x) >> 16 = xh * c + floor(xl * c / 2^16)`; no intermediate value exceeds 31 bits). The 5 products of an output are summed with wrapping
//! additions, so their order does not matter.
//!
//! The coefficients are rearranged per filter configuration (see [`ana_sym_coefs`] and
//! [`ana_nonsym_coefs`]) so that every term of the outputs is a product of two contiguous runs.

use crate::common_fix::FixpDbl;
use crate::tools_rom::FixpPft;

/// The analysis filter states (`FIXP_QAS`) with their `fMultDiv2(FIXP_PFT, FIXP_QAS)`.
pub(crate) trait AnaState: Copy {
    fn mul_div2(c: FixpPft, s: Self) -> FixpDbl;
}

impl AnaState for i16 {
    /// `fixmuldiv2_SS`: the exact product.
    #[inline(always)]
    fn mul_div2(c: FixpPft, s: i16) -> FixpDbl {
        c as i32 * s as i32
    }
}

impl AnaState for i32 {
    /// `fixmuldiv2_SD`: `(c * s) >> 16` in 32-bit operations (see the module documentation).
    #[inline(always)]
    fn mul_div2(c: FixpPft, s: i32) -> FixpDbl {
        let c = c as i32;
        (s >> 16).wrapping_mul(c).wrapping_add(((s & 0xFFFF) * c) >> 16)
    }
}

/// The coefficients of the symmetric analysis prototype filter (`qmfAnaPrototypeFirSlot`) in
/// rows `k = 0..5` of `nch + 1` values: `p_filter[i * pflt_step + k]`, `i = 0..=nch`. Channel `i`
/// uses `row_k[i]` for its first output and `row_k[i + 1]` for its second.
pub(crate) fn ana_sym_coefs(p_filter: &[FixpPft], pflt_step: usize, nch: usize) -> Box<[FixpPft]> {
    (0..5).flat_map(|k| (0..=nch).map(move |i| p_filter[i * pflt_step + k])).collect()
}

/// The coefficients of the non-symmetric analysis prototype filter
/// (`qmfAnaPrototypeFirSlot_NonSymmetric`) in rows `p = 0..5` of `2 * nch` values: the
/// coefficient `p` of output `k`, at the index of the C loop.
pub(crate) fn ana_nonsym_coefs(
    p_filter: &[FixpPft],
    p_stride: usize,
    nch: usize,
) -> Box<[FixpPft]> {
    let poly = 5;
    let base = |k: usize| k * poly * p_stride + poly * (p_stride - 1);
    (0..poly).flat_map(|p| (0..2 * nch).map(move |k| p_filter[base(k) + p])).collect()
}

/// All channels of the symmetric analysis prototype filter (`nch <= 64` channels, states `st` of
/// `10 * nch` values, output `out` of `2 * nch` values, coefficients `c` of [`ana_sym_coefs`]):
/// `out[i] = (sum_k fMultDiv2(row_k[i], st[10 nch - 1 - i - 2 nch k])) << 1` and
/// `out[2 nch - 1 - i] = (sum_k fMultDiv2(row_k[i + 1], st[i + 2 nch k])) << 1`.
#[inline(always)]
pub(crate) fn ana_sym<S: AnaState>(out: &mut [FixpDbl], st: &[S], c: &[FixpPft], nch: usize) {
    let mut a1 = [0 as FixpDbl; 64];
    let mut a2 = [0 as FixpDbl; 64];
    let (a1, a2) = (&mut a1[..nch], &mut a2[..nch]);
    let n1 = nch + 1;
    for k in 0..5 {
        let row = &c[k * n1..(k + 1) * n1];
        let s1 = &st[(9 - 2 * k) * nch..(10 - 2 * k) * nch];
        let s2 = &st[2 * k * nch..(2 * k + 1) * nch];
        for ((a, &c), &s) in a1.iter_mut().zip(&row[..nch]).zip(s1.iter().rev()) {
            *a = a.wrapping_add(S::mul_div2(c, s));
        }
        for ((a, &c), &s) in a2.iter_mut().zip(&row[1..]).zip(s2) {
            *a = a.wrapping_add(S::mul_div2(c, s));
        }
    }
    let (lo, hi) = out[..2 * nch].split_at_mut(nch);
    for (o, &a) in lo.iter_mut().zip(a1.iter()) {
        *o = a << 1;
    }
    for (o, &a) in hi.iter_mut().rev().zip(a2.iter()) {
        *o = a << 1;
    }
}

/// All outputs of the non-symmetric analysis prototype filter (`nch <= 64`, coefficients `c` of
/// [`ana_nonsym_coefs`]): `out[2 nch - 1 - k] = (sum_p fMultDiv2(row_p[k], st[k + 2 nch p])) <<
/// 1`.
#[inline(always)]
pub(crate) fn ana_nonsym<S: AnaState>(out: &mut [FixpDbl], st: &[S], c: &[FixpPft], nch: usize) {
    let n2 = 2 * nch;
    let mut acc = [0 as FixpDbl; 128];
    let acc = &mut acc[..n2];
    for p in 0..5 {
        let row = &c[p * n2..(p + 1) * n2];
        let s = &st[p * n2..(p + 1) * n2];
        for ((a, &c), &s) in acc.iter_mut().zip(row).zip(s) {
            *a = a.wrapping_add(S::mul_div2(c, s));
        }
    }
    for (o, &a) in out[..n2].iter_mut().rev().zip(acc.iter()) {
        *o = a << 1;
    }
}

crate::avx2_clone! {
/// [`ana_sym`] for 16-bit states.
pub(crate) fn ana_sym_i16(out: &mut [FixpDbl], st: &[i16], c: &[FixpPft], nch: usize) {
    ana_sym(out, st, c, nch)
}
}

crate::avx2_clone! {
/// [`ana_sym`] for 32-bit states.
pub(crate) fn ana_sym_i32(out: &mut [FixpDbl], st: &[i32], c: &[FixpPft], nch: usize) {
    ana_sym(out, st, c, nch)
}
}

crate::avx2_clone! {
/// [`ana_nonsym`] for 16-bit states.
pub(crate) fn ana_nonsym_i16(out: &mut [FixpDbl], st: &[i16], c: &[FixpPft], nch: usize) {
    ana_nonsym(out, st, c, nch)
}
}

crate::avx2_clone! {
/// [`ana_nonsym`] for 32-bit states.
pub(crate) fn ana_nonsym_i32(out: &mut [FixpDbl], st: &[i32], c: &[FixpPft], nch: usize) {
    ana_nonsym(out, st, c, nch)
}
}
