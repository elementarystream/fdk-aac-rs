// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized prototype filters of libFDK/include/qmf_pcm.h.

//! NEON kernels of the QMF prototype filters (the same arithmetic on every variant).
//!
//! `fMultDiv2(FIXP_PFT, FIXP_DBL)` (`fixmuldiv2_SD`, the same on every variant) is
//! `(c * x) >> 16` for the 16-bit coefficient `c`; `vqdmulhq_s32(x, c << 15)` computes
//! `(2 * x * (c << 15)) >> 32`, the same value, and cannot saturate (`c << 15` is never
//! `i32::MIN`). `fMultDiv2(FIXP_SGL, FIXP_SGL)` (`fixmuldiv2_SS`) is the exact 32-bit product,
//! `vmull_s16`. The sums wrap like the scalar code (`add`).

use core::arch::aarch64::*;

/// Byte indices into `(fltm[0..8], flt[0..8])` (16-bit values) of the 10 coefficients of the
/// symmetric synthesis filter, in the order of the states: `fltm[0], flt[4], fltm[1], flt[3],
/// fltm[2], flt[2], fltm[3], flt[1]` and `fltm[4], flt[0]`.
const SYN_SYM_IDX: [[u8; 16]; 2] = [
    [0, 1, 24, 25, 2, 3, 22, 23, 4, 5, 20, 21, 6, 7, 18, 19],
    [8, 9, 16, 17, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255],
];
/// As [`SYN_SYM_IDX`] for the non-symmetric filter: `fltm[4], flt[4], fltm[3], flt[3], fltm[2],
/// flt[2], fltm[1], flt[1]` and `fltm[0], flt[0]`.
const SYN_NONSYM_IDX: [[u8; 16]; 2] = [
    [8, 9, 24, 25, 6, 7, 22, 23, 4, 5, 20, 21, 2, 3, 18, 19],
    [0, 1, 16, 17, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255],
];

/// One channel of the synthesis prototype filter (`qmfSynPrototypeFirSlot` or its
/// `_NonSymmetric` version): with `x = (real, imag, real, imag, ...)` and the coefficients `c`
/// in the order of [`SYN_SYM_IDX`] / [`SYN_NONSYM_IDX`], `t[k] = s[k] + fMultDiv2(c[k], x[k])`
/// for `k = 0..10` (`s[9] = 0`); returns `t[0]` (the output before the PCM formatting) and stores
/// `s[k] = t[k + 1]` for `k = 0..9`.
///
/// # Safety
///
/// `s` must be valid for 9 values, `flt` and `fltm` for 8 values each.
#[inline(always)]
pub(crate) unsafe fn syn_channel<const NONSYM: bool>(
    s: *mut i32,
    flt: *const i16,
    fltm: *const i16,
    real: i32,
    imag: i32,
) -> i32 {
    let idx = if NONSYM { &SYN_NONSYM_IDX } else { &SYN_SYM_IDX };
    let t =
        int8x16x2_t(vreinterpretq_s8_s16(vld1q_s16(fltm)), vreinterpretq_s8_s16(vld1q_s16(flt)));
    let ca = vreinterpretq_s16_s8(vqtbl2q_s8(t, vld1q_u8(idx[0].as_ptr())));
    let cb = vreinterpretq_s16_s8(vqtbl2q_s8(t, vld1q_u8(idx[1].as_ptr())));
    let c0 = vshll_n_s16::<15>(vget_low_s16(ca));
    let c1 = vshll_high_n_s16::<15>(ca);
    let c2 = vget_low_s32(vshll_n_s16::<15>(vget_low_s16(cb)));
    let x = vreinterpretq_s32_s64(vdupq_n_s64((real as u32 as i64) | ((imag as i64) << 32)));
    let t0 = vaddq_s32(vld1q_s32(s), vqdmulhq_s32(x, c0));
    let t1 = vaddq_s32(vld1q_s32(s.add(4)), vqdmulhq_s32(x, c1));
    let s8 = vld1_lane_s32::<0>(s.add(8), vdup_n_s32(0));
    let t2 = vadd_s32(s8, vqdmulh_s32(vget_low_s32(x), c2));
    vst1q_s32(s, vextq_s32::<1>(t0, t1));
    vst1q_s32(s.add(4), vextq_s32::<1>(t1, vcombine_s32(t2, t2)));
    vst1_lane_s32::<1>(s.add(8), t2);
    vgetq_lane_s32::<0>(t0)
}

/// [`syn_channel`] with the coefficients `c << 15` precomputed (12 values per channel, the
/// last 2 unused): `c` must be valid for 10 values, `s` for 9.
#[inline(always)]
pub(crate) unsafe fn syn_channel_pre(s: *mut i32, c: *const i32, real: i32, imag: i32) -> i32 {
    let x = vreinterpretq_s32_s64(vdupq_n_s64((real as u32 as i64) | ((imag as i64) << 32)));
    let t0 = vaddq_s32(vld1q_s32(s), vqdmulhq_s32(x, vld1q_s32(c)));
    let t1 = vaddq_s32(vld1q_s32(s.add(4)), vqdmulhq_s32(x, vld1q_s32(c.add(4))));
    let s8 = vld1_lane_s32::<0>(s.add(8), vdup_n_s32(0));
    let t2 = vadd_s32(s8, vqdmulh_s32(vget_low_s32(x), vld1_s32(c.add(8))));
    vst1q_s32(s, vextq_s32::<1>(t0, t1));
    vst1q_s32(s.add(4), vextq_s32::<1>(t1, vcombine_s32(t2, t2)));
    vst1_lane_s32::<1>(s.add(8), t2);
    vgetq_lane_s32::<0>(t0)
}

/// The coefficients of [`syn_channel`] as `c << 15` for [`syn_channel_pre`] (12 per channel).
pub(crate) fn syn_coefs<const NONSYM: bool>(
    p_filter: &[i16],
    flt0: usize,
    fltm0: usize,
    step: usize,
    nch: usize,
) -> Box<[i32]> {
    let mut out = vec![0i32; 12 * nch];
    for it in 0..nch {
        let flt = &p_filter[flt0 + it * step..][..5];
        let fltm = if NONSYM {
            &p_filter[fltm0 + it * step..][..5]
        } else {
            &p_filter[fltm0 - it * step..][..5]
        };
        let c: [i16; 10] = if NONSYM {
            [fltm[4], flt[4], fltm[3], flt[3], fltm[2], flt[2], fltm[1], flt[1], fltm[0], flt[0]]
        } else {
            [fltm[0], flt[4], fltm[1], flt[3], fltm[2], flt[2], fltm[3], flt[1], fltm[4], flt[0]]
        };
        for (o, &c) in out[12 * it..].iter_mut().zip(&c) {
            *o = (c as i32) << 15;
        }
    }
    out.into_boxed_slice()
}

/// The PCM formatting of the synthesis output of 4 channels (see `super::syn_fir`), for `D16`
/// (`SAMPLE_BITS_QMFOUT` 16) or 32-bit output:
///
/// - `fMult(are, gain)` if `gain` is given (as `gain << 15`): `((are * gain) >> 16) << 1`,
///   `vqdmulh(are, gain << 15) << 1` (see the module documentation);
/// - `right`: `SATURATE_RIGHT_SHIFT(are + rnd, scale, bits)` (the addition wraps), which is the
///   shifted value clamped to the output range (both forms of the macro, `0 <= scale <= 31`):
///   `vqmovn` of the arithmetic shift for 16 bits, no clamping needed for 32 bits;
/// - else `SATURATE_LEFT_SHIFT(are, -scale, bits)`, which is `are * 2^-scale` clamped to the
///   output range for `-scale <= bits - 1`: `vqshl`, then `vqmovn` for 16 bits. The caller
///   must not use this for 16 bits and `-scale > 15`.
///
/// Returns the 4 output values (within the output range).
#[inline(always)]
pub(crate) unsafe fn syn_format<const D16: bool>(
    are: [i32; 4],
    gain15: Option<i32>,
    right: bool,
    scale: i32,
    rnd: i32,
) -> [i32; 4] {
    let mut a = vld1q_s32(are.as_ptr());
    if let Some(g) = gain15 {
        a = vshlq_n_s32::<1>(vqdmulhq_s32(a, vdupq_n_s32(g)));
    }
    let v = if right {
        vshlq_s32(vaddq_s32(a, vdupq_n_s32(rnd)), vdupq_n_s32(-scale))
    } else {
        vqshlq_s32(a, vdupq_n_s32(-scale))
    };
    let v = if D16 { vmovl_s16(vqmovn_s32(v)) } else { v };
    let mut out = [0i32; 4];
    vst1q_s32(out.as_mut_ptr(), v);
    out
}

/// The analysis filter states: `FIXP_QAS` of the `INT_PCM` instantiation (`FIXP_SGL`, with
/// `fMultDiv2(FIXP_SGL, FIXP_SGL)` the exact product) or of the `LONG` one (`FIXP_DBL`, with
/// `fMultDiv2(FIXP_SGL, FIXP_DBL)`). The coefficients are stored as `c` (16 bit) or `c << 15`
/// (32 bit) respectively.
pub(crate) trait AnaState: Copy {
    type Coef: Copy + Default;
    fn coef(c: i16) -> Self::Coef;
    /// The 4 states from `p` (ascending).
    unsafe fn load(p: *const Self) -> int32x4_t;
    /// The 4 states from `p` in reverse order.
    unsafe fn load_rev(p: *const Self) -> int32x4_t;
    /// `acc + fMultDiv2(c, s)` per lane; `s` from [`AnaState::load`], `c` 4 coefficients.
    unsafe fn mac(acc: int32x4_t, s: int32x4_t, c: *const Self::Coef) -> int32x4_t;
}

impl AnaState for i16 {
    type Coef = i16;
    fn coef(c: i16) -> i16 {
        c
    }
    #[inline(always)]
    unsafe fn load(p: *const i16) -> int32x4_t {
        vmovl_s16(vld1_s16(p))
    }
    #[inline(always)]
    unsafe fn load_rev(p: *const i16) -> int32x4_t {
        vmovl_s16(vrev64_s16(vld1_s16(p)))
    }
    #[inline(always)]
    unsafe fn mac(acc: int32x4_t, s: int32x4_t, c: *const i16) -> int32x4_t {
        // the 16-bit states widened to 32 bits: the product is exact
        vmlaq_s32(acc, s, vmovl_s16(vld1_s16(c)))
    }
}

impl AnaState for i32 {
    type Coef = i32;
    fn coef(c: i16) -> i32 {
        (c as i32) << 15
    }
    #[inline(always)]
    unsafe fn load(p: *const i32) -> int32x4_t {
        vld1q_s32(p)
    }
    #[inline(always)]
    unsafe fn load_rev(p: *const i32) -> int32x4_t {
        crate::dct::neon::rev(vld1q_s32(p))
    }
    #[inline(always)]
    unsafe fn mac(acc: int32x4_t, s: int32x4_t, c: *const i32) -> int32x4_t {
        vaddq_s32(acc, vqdmulhq_s32(s, vld1q_s32(c)))
    }
}

/// The coefficient blocks of the symmetric analysis prototype filter (`qmfAnaPrototypeFirSlot`):
/// for every block of 4 channels `it0 = 4 b`, 5 vectors `p_filter[(it0 + l) * pflt_step + k]`
/// (`k = 0..5`, lanes `l`) and 5 vectors `p_filter[(it0 + l + 1) * pflt_step + k]`.
pub(crate) fn ana_sym_coefs<S: AnaState>(
    p_filter: &[i16],
    pflt_step: usize,
    nch: usize,
) -> Box<[S::Coef]> {
    let mut out = Vec::with_capacity(nch / 4 * 40);
    for b in 0..nch / 4 {
        for second in 0..2 {
            for k in 0..5 {
                for l in 0..4 {
                    out.push(S::coef(p_filter[(4 * b + l + second) * pflt_step + k]));
                }
            }
        }
    }
    out.into_boxed_slice()
}

/// The channels `it0 .. it0 + 4` of the symmetric analysis prototype filter (`nch` channels,
/// states `st` of `10 * nch` values, output `out` of `2 * nch` values), with the coefficient block
/// `c` of [`ana_sym_coefs`]:
/// `out[it] = (sum_k fMultDiv2(f_it[k], st[10 nch - 1 - it - 2 nch k])) << 1` and
/// `out[2 nch - 1 - it] = (sum_k fMultDiv2(f_(it+1)[k], st[it + 2 nch k])) << 1`.
#[inline(always)]
pub(crate) unsafe fn ana_sym_block<S: AnaState>(
    out: *mut i32,
    st: *const S,
    nch: usize,
    it0: usize,
    c: *const S::Coef,
) {
    let n2 = 2 * nch;
    let mut a = vdupq_n_s32(0);
    let mut b = vdupq_n_s32(0);
    for k in 0..5 {
        a = S::mac(a, S::load_rev(st.add(10 * nch - 4 - it0 - n2 * k)), c.add(4 * k));
        b = S::mac(b, S::load(st.add(it0 + n2 * k)), c.add(20 + 4 * k));
    }
    vst1q_s32(out.add(it0), vshlq_n_s32::<1>(a));
    vst1q_s32(out.add(n2 - 4 - it0), crate::dct::neon::rev(vshlq_n_s32::<1>(b)));
}

/// The coefficient blocks of the non-symmetric analysis prototype filter
/// (`qmfAnaPrototypeFirSlot_NonSymmetric`): for every block of 4 outputs `k0 = 4 b`, 5 vectors
/// of the coefficients `p` of the outputs `k0 + l`, at the indices of the C loop.
pub(crate) fn ana_nonsym_coefs<S: AnaState>(
    p_filter: &[i16],
    p_stride: usize,
    nch: usize,
) -> Box<[S::Coef]> {
    let poly = 5;
    let base = |k: usize| (k + 1) * poly * (p_stride - 1) + k * poly;
    let mut out = Vec::with_capacity(2 * nch / 4 * 20);
    for b in 0..2 * nch / 4 {
        for p in 0..poly {
            for l in 0..4 {
                out.push(S::coef(p_filter[base(4 * b + l) + p]));
            }
        }
    }
    out.into_boxed_slice()
}

/// The outputs `k0 .. k0 + 4` of the non-symmetric analysis prototype filter:
/// `out[2 nch - 1 - k] = (sum_p fMultDiv2(c_k[p], st[k + 2 nch p])) << 1`, with the coefficient
/// block `c` of [`ana_nonsym_coefs`].
#[inline(always)]
pub(crate) unsafe fn ana_nonsym_block<S: AnaState>(
    out: *mut i32,
    st: *const S,
    nch: usize,
    k0: usize,
    c: *const S::Coef,
) {
    let n2 = 2 * nch;
    let mut a = vdupq_n_s32(0);
    for p in 0..5 {
        a = S::mac(a, S::load(st.add(k0 + n2 * p)), c.add(4 * p));
    }
    vst1q_s32(out.add(n2 - 4 - k0), crate::dct::neon::rev(vshlq_n_s32::<1>(a)));
}
