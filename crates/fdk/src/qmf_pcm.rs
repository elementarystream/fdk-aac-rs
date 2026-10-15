// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/qmf_pcm.h.

//! Port of `qmf_pcm.h`: the QMF functions that depend on the PCM type.
//!
//! `qmf.cpp` includes `qmf_pcm.h` twice, so the reference has two instantiations of every
//! function, overloaded by the type of the time signal:
//!
//! | Instantiation | Time signal (`INT_PCM_QMFIN`, `INT_PCM_QMFOUT`) | Analysis states (`FIXP_QAS`) | `SAMPLE_BITS_QMFOUT` |
//! |---|---|---|---|
//! | first | `INT_PCM` (`SHORT`) | `FIXP_PCM` (`FIXP_SGL`) | 16 |
//! | second | `LONG` (`INT`) | `FIXP_DBL` | 32 |
//!
//! The port has one generic function per C function; the trait [`QmfPcm`] (implemented for
//! `i16` and `i32`) selects the instantiation. The synthesis states (`FIXP_QSS`) are `FIXP_DBL`
//! in both.
//!
//! The pointer arrays of time slots (`FIXP_DBL **qmfReal`) are slices of slots (`&mut [R]` with
//! `R: AsMut<[FixpDbl]>`, e.g. `&mut [&mut [FixpDbl]]`); a `NULL` imaginary array (low power
//! mode) is an empty slice.

use crate::common_fix::*;
use crate::qmf::*;
use crate::scale::{saturate_left_shift, saturate_right_shift};
use crate::tools_rom::{FixpPft, QMF_NO_POLY};

/// The PCM type of a `qmf_pcm.h` instantiation (`i16`: `INT_PCM`, `i32`: `LONG`).
pub trait QmfPcm: Copy + Default {
    /// `FIXP_QAS`: the type of the analysis filter states.
    type Qas: Copy + Default;
    /// `SAMPLE_BITS_QMFOUT`
    const SAMPLE_BITS_QMFOUT: i32;
    /// `(FIXP_QAS)*timeIn`
    fn to_qas(self) -> Self::Qas;
    /// `fMultDiv2(FIXP_PFT, FIXP_QAS)`
    fn pft_mult_div2(p: FixpPft, s: Self::Qas) -> FixpDbl;
    /// `(INT_PCM_QMFOUT)x`
    fn from_dbl(x: FixpDbl) -> Self;
    /// The first channels (symmetric filter, `nonsym` false) or outputs (non-symmetric filter)
    /// of the analysis prototype filter in NEON (see [`ana_fir_neon`]); returns how many were
    /// computed. The default computes none.
    #[doc(hidden)]
    #[inline(always)]
    fn ana_fir_fast(
        _analysis_buffer: &mut [FixpDbl],
        _no_channels: usize,
        _p_filter: &'static [FixpPft],
        _p_stride: usize,
        _states: &[Self::Qas],
        _nonsym: bool,
    ) -> usize {
        0
    }
}

impl QmfPcm for i16 {
    type Qas = FixpSgl;
    const SAMPLE_BITS_QMFOUT: i32 = 16;
    #[inline(always)]
    fn to_qas(self) -> FixpSgl {
        self
    }
    /// `fMultDiv2(FIXP_SGL, FIXP_SGL)`: `fixmuldiv2_SS`
    #[inline(always)]
    fn pft_mult_div2(p: FixpPft, s: FixpSgl) -> FixpDbl {
        f_mult_div2(p, s)
    }
    #[inline(always)]
    fn from_dbl(x: FixpDbl) -> i16 {
        x as i16
    }
    #[cfg(target_arch = "aarch64")]
    #[inline(always)]
    fn ana_fir_fast(
        analysis_buffer: &mut [FixpDbl],
        no_channels: usize,
        p_filter: &'static [FixpPft],
        p_stride: usize,
        states: &[FixpSgl],
        nonsym: bool,
    ) -> usize {
        static CACHE: AnaCache<i16> = [const { std::sync::OnceLock::new() }; 32];
        ana_fir_neon(&CACHE, analysis_buffer, no_channels, p_filter, p_stride, states, nonsym)
    }
    #[cfg(not(target_arch = "aarch64"))]
    #[inline(always)]
    fn ana_fir_fast(
        analysis_buffer: &mut [FixpDbl],
        no_channels: usize,
        p_filter: &'static [FixpPft],
        p_stride: usize,
        states: &[FixpSgl],
        nonsym: bool,
    ) -> usize {
        static CACHE: AnaCache<FixpPft> = [const { std::sync::OnceLock::new() }; 32];
        let Some(c) = ana_coefs(&CACHE, no_channels, p_filter, p_stride, states.len(), nonsym)
        else {
            return 0;
        };
        if analysis_buffer.len() < 2 * no_channels {
            return 0;
        }
        if nonsym {
            fast::ana_nonsym_i16(analysis_buffer, states, c, no_channels);
            2 * no_channels
        } else {
            fast::ana_sym_i16(analysis_buffer, states, c, no_channels);
            no_channels
        }
    }
}

impl QmfPcm for i32 {
    type Qas = FixpDbl;
    const SAMPLE_BITS_QMFOUT: i32 = 32;
    #[inline(always)]
    fn to_qas(self) -> FixpDbl {
        self
    }
    /// `fMultDiv2(FIXP_SGL, FIXP_DBL)`: `fixmuldiv2_SD`
    #[inline(always)]
    fn pft_mult_div2(p: FixpPft, s: FixpDbl) -> FixpDbl {
        f_mult_div2(p, s)
    }
    #[inline(always)]
    fn from_dbl(x: FixpDbl) -> i32 {
        x
    }
    #[cfg(target_arch = "aarch64")]
    #[inline(always)]
    fn ana_fir_fast(
        analysis_buffer: &mut [FixpDbl],
        no_channels: usize,
        p_filter: &'static [FixpPft],
        p_stride: usize,
        states: &[FixpDbl],
        nonsym: bool,
    ) -> usize {
        static CACHE: AnaCache<i32> = [const { std::sync::OnceLock::new() }; 32];
        ana_fir_neon(&CACHE, analysis_buffer, no_channels, p_filter, p_stride, states, nonsym)
    }
    #[cfg(not(target_arch = "aarch64"))]
    #[inline(always)]
    fn ana_fir_fast(
        analysis_buffer: &mut [FixpDbl],
        no_channels: usize,
        p_filter: &'static [FixpPft],
        p_stride: usize,
        states: &[FixpDbl],
        nonsym: bool,
    ) -> usize {
        static CACHE: AnaCache<FixpPft> = [const { std::sync::OnceLock::new() }; 32];
        let Some(c) = ana_coefs(&CACHE, no_channels, p_filter, p_stride, states.len(), nonsym)
        else {
            return 0;
        };
        if analysis_buffer.len() < 2 * no_channels {
            return 0;
        }
        if nonsym {
            fast::ana_nonsym_i32(analysis_buffer, states, c, no_channels);
            2 * no_channels
        } else {
            fast::ana_sym_i32(analysis_buffer, states, c, no_channels);
            no_channels
        }
    }
}

/// `scale`, `rnd_val` of the PCM formatting of the synthesis prototype filters.
#[inline]
fn syn_output_scale<T: QmfPcm>(qmf: &QmfFilterBank) -> (i32, FixpDbl) {
    // The C expression cannot overflow for the scale factors and gains of the library; the
    // port wraps.
    let mut scale = ((DFRACT_BITS - T::SAMPLE_BITS_QMFOUT) - 1)
        .wrapping_sub(qmf.out_scalefactor)
        .wrapping_sub(qmf.out_gain_e);
    let mut rnd_val: FixpDbl = 0;
    if scale > 0 {
        if scale < DFRACT_BITS - 1 {
            rnd_val = 1 << (scale - 1);
        } else {
            scale = DFRACT_BITS - 1;
        }
    } else {
        scale = f_max(scale, -(DFRACT_BITS - 1));
    }
    (scale, rnd_val)
}

/// Whether the coefficient indices `lo..=hi` are inside `p_filter`.
#[inline]
fn filter_covers(p_filter: &[FixpPft], lo: i64, hi: i64) -> bool {
    lo >= 0 && hi < p_filter.len() as i64
}

/// `qmfSynPrototypeFirSlot()`: synthesis prototype filtering of one slot (symmetric prototype
/// filter). Takes `no_channels` values of `real_slot` and `imag_slot` and writes `no_channels`
/// samples to `time_out[j * stride]`.
///
/// The same as the straight port [`qmf_syn_prototype_fir_slot_ref`]; on `aarch64` targets the
/// filter of a channel runs in NEON (`neon::syn_channel`, any variant).
///
/// Deviation: for a filter bank without a valid configuration the reference has no defined
/// result; the port returns without output.
pub fn qmf_syn_prototype_fir_slot<T: QmfPcm>(
    qmf: &QmfFilterBank,
    states: &mut [FixpQss],
    real_slot: &[FixpDbl],
    imag_slot: &[FixpDbl],
    time_out: &mut [T],
    stride: i32,
) {
    syn_fir::<T, false>(qmf, states, real_slot, imag_slot, time_out, stride)
}

/// `qmfSynPrototypeFirSlot_NonSymmetric()`: as [`qmf_syn_prototype_fir_slot`], for the
/// non-symmetric prototype filters (CLDFB, MPS low delay filter bank). The same as the straight
/// port [`qmf_syn_prototype_fir_slot_non_symmetric_ref`].
pub fn qmf_syn_prototype_fir_slot_non_symmetric<T: QmfPcm>(
    qmf: &QmfFilterBank,
    states: &mut [FixpQss],
    real_slot: &[FixpDbl],
    imag_slot: &[FixpDbl],
    time_out: &mut [T],
    stride: i32,
) {
    syn_fir::<T, true>(qmf, states, real_slot, imag_slot, time_out, stride)
}

#[cfg(not(target_arch = "aarch64"))]
mod fast;
#[cfg(target_arch = "aarch64")]
mod neon;
#[cfg(test)]
mod tests;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86;

/// [`qmf_syn_prototype_fir_slot`] (`NONSYM` false) and
/// [`qmf_syn_prototype_fir_slot_non_symmetric`] (`NONSYM` true): the loops of the straight ports
/// with the filter of a channel computed by [`syn_channel`].
#[inline(always)]
fn syn_fir<T: QmfPcm, const NONSYM: bool>(
    qmf: &QmfFilterBank,
    states: &mut [FixpQss],
    real_slot: &[FixpDbl],
    imag_slot: &[FixpDbl],
    time_out: &mut [T],
    stride: i32,
) {
    let no_channels = qmf.no_channels;
    let p_filter = qmf.p_filter;
    let p_stride = qmf.p_stride as i32;
    let step = (p_stride * QMF_NO_POLY) as i64;
    let nch = no_channels.max(0) as i64;
    // symmetric: p_flt = p_Filter + p_stride * QMF_NO_POLY, p_fltm = p_Filter + (FilterSize / 2)
    // - p_stride * QMF_NO_POLY, decreasing; non-symmetric: p_flt = p_Filter, p_fltm = p_Filter +
    // FilterSize / 2, increasing.
    let (flt0, fltm0) = if NONSYM {
        (0, (qmf.filter_size / 2) as i64)
    } else {
        (step, (qmf.filter_size / 2) as i64 - step)
    };
    if nch == 0 {
        return;
    }
    let fltm_lo = if NONSYM { fltm0 } else { fltm0 - (nch - 1) * step };
    let fltm_hi = if NONSYM { fltm0 + (nch - 1) * step + 4 } else { fltm0 + 4 };
    if !filter_covers(p_filter, flt0, flt0 + (nch - 1) * step + 4)
        || !filter_covers(p_filter, fltm_lo, fltm_hi)
        || states.len() < (nch * 9) as usize
    {
        return;
    }
    let (flt0, fltm0, step) = (flt0 as usize, fltm0 as usize, step as usize);

    let gain = fx_dbl2fx_sgl(qmf.out_gain_m);
    let (scale, rnd_val) = syn_output_scale::<T>(qmf);
    // The formatting shifts right (with rounding) for scale >= 0 (symmetric) / scale > 0
    // (non-symmetric), else left.
    #[cfg_attr(
        not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")),
        allow(unused_variables)
    )]
    let right = if NONSYM { scale > 0 } else { scale >= 0 };
    let nch = nch as usize;
    #[allow(unused_mut)]
    let mut it0 = 0usize;

    // Groups of 4 channels with the formatting in NEON (see `neon::syn_format`), except for a
    // left shift beyond the range where SATURATE_LEFT_SHIFT clamps exactly (16-bit output). The
    // filter coefficients of the known configurations are precomputed (`syn_coefs_cached`).
    #[cfg(target_arch = "aarch64")]
    if T::SAMPLE_BITS_QMFOUT == 32 || right || -scale <= 15 {
        let gain15 = (gain != -32768).then_some((gain as i32) << 15);
        let stride = stride as usize;
        let coefs = syn_coefs_cached::<NONSYM>(p_filter, flt0, fltm0, step, nch);
        // The PCM formatting and the stores of 4 channels.
        let mut format = |it0: usize, are: [FixpDbl; 4]| {
            if cfg!(debug_assertions) && right {
                for &a in &are {
                    let a = if gain != -32768 { f_mult(a, gain) } else { a };
                    if NONSYM {
                        fdk_assert!(a < a.wrapping_add(rnd_val));
                    } else {
                        fdk_assert!(a <= a.wrapping_add(rnd_val));
                    }
                }
            }
            // SAFETY: NEON arithmetic on local arrays.
            let out = unsafe {
                if T::SAMPLE_BITS_QMFOUT == 16 {
                    neon::syn_format::<true>(are, gain15, right, scale, rnd_val)
                } else {
                    neon::syn_format::<false>(are, gain15, right, scale, rnd_val)
                }
            };
            for (l, &o) in out.iter().enumerate() {
                time_out[(nch - 1 - it0 - l) * stride] = T::from_dbl(o);
            }
        };
        if let Some(c) = coefs {
            // The channels it = 4 g + l (j = nch - 1 - it) of the groups g: 36 states, 48
            // coefficients, and the inputs j = nch - 4 - 4 g .. nch - 4 g in reverse order.
            let groups = nch / 4;
            let st = states[..36 * groups].chunks_exact_mut(36);
            let cc = c[..48 * groups].chunks_exact(48);
            let rs = real_slot[..nch].rchunks_exact(4);
            let is = imag_slot[..nch].rchunks_exact(4);
            for (g, (((s, c), r), im)) in st.zip(cc).zip(rs).zip(is).enumerate() {
                let mut are = [0 as FixpDbl; 4];
                for (l, a) in are.iter_mut().enumerate() {
                    let (s, c) = (&mut s[9 * l..9 * l + 9], &c[12 * l..12 * l + 10]);
                    // SAFETY: `s` holds 9 values, `c` 10.
                    *a = unsafe {
                        neon::syn_channel_pre(s.as_mut_ptr(), c.as_ptr(), r[3 - l], im[3 - l])
                    };
                }
                format(4 * g, are);
            }
            it0 = 4 * groups;
        } else {
            while it0 + 4 <= nch {
                let mut are = [0 as FixpDbl; 4];
                for (l, a) in are.iter_mut().enumerate() {
                    let it = it0 + l;
                    let j = nch - 1 - it;
                    let s = &mut states[9 * it..9 * it + 9];
                    let p_flt = flt0 + it * step;
                    let p_fltm = if NONSYM { fltm0 + it * step } else { fltm0 - it * step };
                    *a = syn_channel::<NONSYM>(
                        s,
                        p_filter,
                        p_flt,
                        p_fltm,
                        real_slot[j],
                        imag_slot[j],
                    );
                }
                format(it0, are);
                it0 += 4;
            }
        }
    }

    // All channels with the AVX2 kernel (`x86::syn_channels`) for the known filter
    // configurations, then the PCM formatting.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if nch <= 64 && crate::simd::avx2() {
        if let Some(c) = syn_coefs_cached::<NONSYM>(p_filter, flt0, fltm0, step, nch) {
            let mut are = [0 as FixpDbl; 64];
            // SAFETY: AVX2 is available.
            unsafe {
                x86::syn_channels(states, c, real_slot, imag_slot, &mut are[..nch]);
            }
            if cfg!(debug_assertions) && right {
                for &a in &are[..nch] {
                    let a = if gain != -32768 { f_mult(a, gain) } else { a };
                    if NONSYM {
                        fdk_assert!(a < a.wrapping_add(rnd_val));
                    } else {
                        fdk_assert!(a <= a.wrapping_add(rnd_val));
                    }
                }
            }
            syn_format_all(&mut are[..nch], gain, scale, rnd_val, right, T::SAMPLE_BITS_QMFOUT);
            for (it, &o) in are[..nch].iter().enumerate() {
                time_out[(nch - 1 - it) * stride as usize] = T::from_dbl(o);
            }
            return;
        }
    }

    for it in it0..nch {
        let j = nch - 1 - it; // no_channels-1 .. 0
        let imag = imag_slot[j];
        let real = real_slot[j];
        let p_flt = flt0 + it * step;
        let p_fltm = if NONSYM { fltm0 + it * step } else { fltm0 - it * step };
        let s = &mut states[9 * it..9 * it + 9]; // (2*QMF_NO_POLY-1) per channel
        let are = syn_channel::<NONSYM>(s, p_filter, p_flt, p_fltm, real, imag);
        time_out[j * stride as usize] = syn_format_scalar::<T, NONSYM>(are, gain, scale, rnd_val);
    }
}

crate::avx2_clone! {
/// The PCM formatting of [`syn_fir`] (see [`syn_format_scalar`]) of all values of `are`, with
/// `right` the direction of the shift and `bits` the output word length (vectorized; the
/// saturating shifts are selects).
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn syn_format_all(
    are: &mut [FixpDbl],
    gain: FixpSgl,
    scale: i32,
    rnd_val: FixpDbl,
    right: bool,
    bits: i32,
) {
    if gain != -32768 {
        are.iter_mut().for_each(|a| *a = f_mult(*a, gain));
    }
    if right {
        are.iter_mut()
            .for_each(|a| *a = saturate_right_shift(a.wrapping_add(rnd_val), scale, bits));
    } else {
        are.iter_mut().for_each(|a| *a = saturate_left_shift(*a, -scale, bits));
    }
}
}

/// The PCM formatting of [`syn_fir`] for one output value.
#[inline(always)]
fn syn_format_scalar<T: QmfPcm, const NONSYM: bool>(
    mut are: FixpDbl,
    gain: FixpSgl,
    scale: i32,
    rnd_val: FixpDbl,
) -> T {
    // This PCM formatting performs:
    //  - multiplication with 16-bit gain, if not -1.0f
    //  - rounding, if shift right is applied
    //  - apply shift left (or right) with saturation to 32 (or 16) bits
    //  - store output with --stride in 32 (or 16) bit format
    if gain != -32768 {
        // -1.0f
        are = f_mult(are, gain);
    }
    if (!NONSYM && scale >= 0) || (NONSYM && scale > 0) {
        // Round-addition must not overflow, might be equal for rnd_val=0
        if NONSYM {
            fdk_assert!(are < are.wrapping_add(rnd_val));
        } else {
            fdk_assert!(are <= are.wrapping_add(rnd_val));
        }
        T::from_dbl(saturate_right_shift(are.wrapping_add(rnd_val), scale, T::SAMPLE_BITS_QMFOUT))
    } else {
        T::from_dbl(saturate_left_shift(are, -scale, T::SAMPLE_BITS_QMFOUT))
    }
}

/// The precomputed coefficients (`neon::syn_coefs`, `x86::syn_coefs`) of a synthesis filter
/// bank configuration: one lazily built table per known prototype filter (and stride for
/// `qmf_pfilt640`), used if it was built for the same parameters; `None` for other
/// configurations.
#[cfg(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64"))]
fn syn_coefs_cached<const NONSYM: bool>(
    p_filter: &'static [FixpPft],
    flt0: usize,
    fltm0: usize,
    step: usize,
    nch: usize,
) -> Option<&'static [i32]> {
    use crate::tools_rom::*;
    use std::sync::OnceLock;
    type Key = (usize, usize, usize, usize, usize, usize, bool);
    static CACHE: [OnceLock<(Key, Box<[i32]>)>; 32] = [const { OnceLock::new() }; 32];
    let tables: [&'static [FixpPft]; 11] = [
        &QMF_PFILT640,
        &QMF_PFILT400,
        &QMF_PFILT200,
        &QMF_PFILT120,
        &QMF_PFILT240,
        &QMF_CLDFB_640,
        &QMF_CLDFB_320,
        &QMF_CLDFB_160,
        &QMF_CLDFB_80,
        &QMF_MPSLDFB_640,
        &QMF_MPSLDFB_320,
    ];
    let t = tables.iter().position(|t| core::ptr::eq(t.as_ptr(), p_filter.as_ptr()))?;
    // QMF_PFILT640 is used with the strides 1, 2, 4 and 8 (step 5 * p_stride).
    let sub = match step {
        5 => 0,
        10 => 1,
        20 => 2,
        40 => 3,
        _ => return None,
    };
    let slot = 2 * (if t == 0 { sub } else { 3 + t }) + NONSYM as usize;
    let key: Key = (p_filter.as_ptr() as usize, p_filter.len(), flt0, fltm0, step, nch, NONSYM);
    #[cfg(target_arch = "aarch64")]
    let build = neon::syn_coefs::<NONSYM>;
    #[cfg(not(target_arch = "aarch64"))]
    let build = x86::syn_coefs::<NONSYM>;
    let (k, c) = CACHE[slot].get_or_init(|| (key, build(p_filter, flt0, fltm0, step, nch)));
    (*k == key).then_some(&c[..])
}

/// The filter of one channel of [`syn_fir`]: updates the 9 states `s` and returns the output
/// before the PCM formatting (see the straight ports for the operations). NEON on `aarch64`
/// targets where the coefficients `p_filter[p_flt..p_flt + 8]` and
/// `p_filter[p_fltm..p_fltm + 8]` are inside the table (the kernel loads 8), else scalar.
#[inline(always)]
fn syn_channel<const NONSYM: bool>(
    s: &mut [FixpQss],
    p_filter: &[FixpPft],
    p_flt: usize,
    p_fltm: usize,
    real: FixpDbl,
    imag: FixpDbl,
) -> FixpDbl {
    let s = &mut s[..9];
    #[cfg(target_arch = "aarch64")]
    if p_flt + 8 <= p_filter.len() && p_fltm + 8 <= p_filter.len() {
        // SAFETY: `s` holds 9 values, the filter 8 values from both indices.
        return unsafe {
            neon::syn_channel::<NONSYM>(
                s.as_mut_ptr(),
                p_filter.as_ptr().add(p_flt),
                p_filter.as_ptr().add(p_fltm),
                real,
                imag,
            )
        };
    }
    let flt = &p_filter[p_flt..p_flt + 5];
    let fltm = &p_filter[p_fltm..p_fltm + 5];
    if NONSYM {
        let are = s[0].wrapping_add(f_mult_div2(fltm[4], real));
        s[0] = s[1].wrapping_add(f_mult_div2(flt[4], imag));
        s[1] = s[2].wrapping_add(f_mult_div2(fltm[3], real));
        s[2] = s[3].wrapping_add(f_mult_div2(flt[3], imag));
        s[3] = s[4].wrapping_add(f_mult_div2(fltm[2], real));
        s[4] = s[5].wrapping_add(f_mult_div2(flt[2], imag));
        s[5] = s[6].wrapping_add(f_mult_div2(fltm[1], real));
        s[6] = s[7].wrapping_add(f_mult_div2(flt[1], imag));
        s[7] = s[8].wrapping_add(f_mult_div2(fltm[0], real));
        s[8] = f_mult_div2(flt[0], imag);
        are
    } else {
        let are = f_mult_add_div2(s[0], fltm[0], real);
        s[0] = f_mult_add_div2(s[1], flt[4], imag);
        s[1] = f_mult_add_div2(s[2], fltm[1], real);
        s[2] = f_mult_add_div2(s[3], flt[3], imag);
        s[3] = f_mult_add_div2(s[4], fltm[2], real);
        s[4] = f_mult_add_div2(s[5], flt[2], imag);
        s[5] = f_mult_add_div2(s[6], fltm[3], real);
        s[6] = f_mult_add_div2(s[7], flt[1], imag);
        s[7] = f_mult_add_div2(s[8], fltm[4], real);
        s[8] = f_mult_div2(flt[0], imag);
        are
    }
}

/// `qmfSynPrototypeFirSlot()`: the straight port of [`qmf_syn_prototype_fir_slot`].
pub fn qmf_syn_prototype_fir_slot_ref<T: QmfPcm>(
    qmf: &QmfFilterBank,
    states: &mut [FixpQss],
    real_slot: &[FixpDbl],
    imag_slot: &[FixpDbl],
    time_out: &mut [T],
    stride: i32,
) {
    let no_channels = qmf.no_channels;
    let p_filter = qmf.p_filter;
    let p_stride = qmf.p_stride as i32;
    let step = (p_stride * QMF_NO_POLY) as i64;
    let nch = no_channels.max(0) as i64;
    // p_flt = p_Filter + p_stride * QMF_NO_POLY  (5th of 330)
    // p_fltm = p_Filter + (FilterSize / 2) - p_stride * QMF_NO_POLY  (5 + (320 - 2*5) = 315th of 330)
    let flt0 = step;
    let fltm0 = (qmf.filter_size / 2) as i64 - step;
    if nch == 0 {
        return;
    }
    if !filter_covers(p_filter, flt0, flt0 + (nch - 1) * step + 4)
        || !filter_covers(p_filter, fltm0 - (nch - 1) * step, fltm0 + 4)
        || states.len() < (nch * 9) as usize
    {
        return;
    }
    let mut p_flt = flt0 as usize;
    let mut p_fltm = fltm0 as usize;
    let step = step as usize;

    let gain = fx_dbl2fx_sgl(qmf.out_gain_m);
    let (scale, rnd_val) = syn_output_scale::<T>(qmf);

    let mut sta = 0usize;
    for j in (0..no_channels as usize).rev() {
        let imag = imag_slot[j]; // no_channels-1 .. 0
        let real = real_slot[j]; // no_channels-1 .. 0
        {
            let flt = &p_filter[p_flt..p_flt + 5];
            let fltm = &p_filter[p_fltm..p_fltm + 5];
            let s = &mut states[sta..sta + 9];
            let mut are = f_mult_add_div2(s[0], fltm[0], real);

            // This PCM formatting performs:
            //  - multiplication with 16-bit gain, if not -1.0f
            //  - rounding, if shift right is applied
            //  - apply shift left (or right) with saturation to 32 (or 16) bits
            //  - store output with --stride in 32 (or 16) bit format
            if gain != -32768 {
                // -1.0f
                are = f_mult(are, gain);
            }
            let tmp = if scale >= 0 {
                // Round-addition must not overflow, might be equal for rnd_val=0
                fdk_assert!(are <= are.wrapping_add(rnd_val));
                T::from_dbl(saturate_right_shift(
                    are.wrapping_add(rnd_val),
                    scale,
                    T::SAMPLE_BITS_QMFOUT,
                ))
            } else {
                T::from_dbl(saturate_left_shift(are, -scale, T::SAMPLE_BITS_QMFOUT))
            };

            time_out[j * stride as usize] = tmp;

            s[0] = f_mult_add_div2(s[1], flt[4], imag);
            s[1] = f_mult_add_div2(s[2], fltm[1], real);
            s[2] = f_mult_add_div2(s[3], flt[3], imag);
            s[3] = f_mult_add_div2(s[4], fltm[2], real);
            s[4] = f_mult_add_div2(s[5], flt[2], imag);
            s[5] = f_mult_add_div2(s[6], fltm[3], real);
            s[6] = f_mult_add_div2(s[7], flt[1], imag);
            s[7] = f_mult_add_div2(s[8], fltm[4], real);
            s[8] = f_mult_div2(flt[0], imag);
        }
        p_flt += step;
        p_fltm = p_fltm.wrapping_sub(step);
        sta += 9; // = (2*QMF_NO_POLY-1);
    }
}

/// `qmfSynPrototypeFirSlot_NonSymmetric()`: the straight port of
/// [`qmf_syn_prototype_fir_slot_non_symmetric`].
pub fn qmf_syn_prototype_fir_slot_non_symmetric_ref<T: QmfPcm>(
    qmf: &QmfFilterBank,
    states: &mut [FixpQss],
    real_slot: &[FixpDbl],
    imag_slot: &[FixpDbl],
    time_out: &mut [T],
    stride: i32,
) {
    let no_channels = qmf.no_channels;
    let p_filter = qmf.p_filter;
    let p_stride = qmf.p_stride as i32;
    let step = (p_stride * QMF_NO_POLY) as i64;
    let nch = no_channels.max(0) as i64;
    // p_flt: first half of the filter coefficients; p_fltm: at index 320, overall 640
    let fltm0 = (qmf.filter_size / 2) as i64;
    if nch == 0 {
        return;
    }
    if !filter_covers(p_filter, 0, (nch - 1) * step + 4)
        || !filter_covers(p_filter, fltm0, fltm0 + (nch - 1) * step + 4)
        || states.len() < (nch * 9) as usize
    {
        return;
    }
    let mut p_flt = 0usize;
    let mut p_fltm = fltm0 as usize;
    let step = step as usize;

    let gain = fx_dbl2fx_sgl(qmf.out_gain_m);
    let (scale, rnd_val) = syn_output_scale::<T>(qmf);

    let mut sta = 0usize;
    for j in (0..no_channels as usize).rev() {
        let imag = imag_slot[j]; // no_channels-1 .. 0
        let real = real_slot[j]; // no_channels-1 .. 0
        let flt = &p_filter[p_flt..p_flt + 5];
        let fltm = &p_filter[p_fltm..p_fltm + 5];
        let s = &mut states[sta..sta + 9];
        {
            let mut are = s[0].wrapping_add(f_mult_div2(fltm[4], real));

            // This PCM formatting performs:
            //  - multiplication with 16-bit gain, if not -1.0f
            //  - rounding, if shift right is applied
            //  - apply shift left (or right) with saturation to 32 (or 16) bits
            //  - store output with --stride in 32 (or 16) bit format
            if gain != -32768 {
                // -1.0f
                are = f_mult(are, gain);
            }
            let tmp = if scale > 0 {
                // Round-addition must not overflow
                fdk_assert!(are < are.wrapping_add(rnd_val));
                T::from_dbl(saturate_right_shift(
                    are.wrapping_add(rnd_val),
                    scale,
                    T::SAMPLE_BITS_QMFOUT,
                ))
            } else {
                T::from_dbl(saturate_left_shift(are, -scale, T::SAMPLE_BITS_QMFOUT))
            };
            time_out[j * stride as usize] = tmp;
        }

        s[0] = s[1].wrapping_add(f_mult_div2(flt[4], imag));
        s[1] = s[2].wrapping_add(f_mult_div2(fltm[3], real));
        s[2] = s[3].wrapping_add(f_mult_div2(flt[3], imag));

        s[3] = s[4].wrapping_add(f_mult_div2(fltm[2], real));
        s[4] = s[5].wrapping_add(f_mult_div2(flt[2], imag));
        s[5] = s[6].wrapping_add(f_mult_div2(fltm[1], real));
        s[6] = s[7].wrapping_add(f_mult_div2(flt[1], imag));

        s[7] = s[8].wrapping_add(f_mult_div2(fltm[0], real));
        s[8] = f_mult_div2(flt[0], imag);

        p_flt += step;
        p_fltm += step;
        sta += 9; // = (2*QMF_NO_POLY-1);
    }
}

/// `qmfSynthesisFilteringSlot(synQmf, realSlot, imagSlot, scaleFactorLowBand,
/// scaleFactorHighBand, timeOut, stride, pWorkBuffer)`: synthesis of one slot into
/// `time_out[j * stride]`, `j < no_channels`. `imag_slot` is not used in low power mode (may be
/// empty); `p_work_buffer` needs `2 * no_channels` values.
///
/// The same as the straight port [`qmf_synthesis_filtering_slot_ref`].
pub fn qmf_synthesis_filtering_slot<T: QmfPcm>(
    syn_qmf: &QmfFilterBank,
    states: &mut [FixpQss],
    real_slot: &[FixpDbl],
    imag_slot: &[FixpDbl],
    scale_factor_low_band: i32,
    scale_factor_high_band: i32,
    time_out: &mut [T],
    stride: i32,
    p_work_buffer: &mut [FixpDbl],
) {
    synthesis_slot::<T, true>(
        syn_qmf,
        states,
        real_slot,
        imag_slot,
        scale_factor_low_band,
        scale_factor_high_band,
        time_out,
        stride,
        p_work_buffer,
    )
}

/// `qmfSynthesisFilteringSlot()`: the straight port of [`qmf_synthesis_filtering_slot`].
pub fn qmf_synthesis_filtering_slot_ref<T: QmfPcm>(
    syn_qmf: &QmfFilterBank,
    states: &mut [FixpQss],
    real_slot: &[FixpDbl],
    imag_slot: &[FixpDbl],
    scale_factor_low_band: i32,
    scale_factor_high_band: i32,
    time_out: &mut [T],
    stride: i32,
    p_work_buffer: &mut [FixpDbl],
) {
    synthesis_slot::<T, false>(
        syn_qmf,
        states,
        real_slot,
        imag_slot,
        scale_factor_low_band,
        scale_factor_high_band,
        time_out,
        stride,
        p_work_buffer,
    )
}

/// [`qmf_synthesis_filtering_slot`] (`FAST`) and [`qmf_synthesis_filtering_slot_ref`].
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn synthesis_slot<T: QmfPcm, const FAST: bool>(
    syn_qmf: &QmfFilterBank,
    states: &mut [FixpQss],
    real_slot: &[FixpDbl],
    imag_slot: &[FixpDbl],
    scale_factor_low_band: i32,
    scale_factor_high_band: i32,
    time_out: &mut [T],
    stride: i32,
    p_work_buffer: &mut [FixpDbl],
) {
    if syn_qmf.flags & QMF_FLAG_LP == 0 {
        qmf_inverse_modulation_hq::<FAST>(
            syn_qmf,
            real_slot,
            imag_slot,
            scale_factor_low_band,
            scale_factor_high_band,
            p_work_buffer,
        );
    } else if syn_qmf.flags & QMF_FLAG_CLDFB != 0 {
        qmf_inverse_modulation_lp_odd::<FAST>(
            syn_qmf,
            real_slot,
            scale_factor_low_band,
            scale_factor_high_band,
            p_work_buffer,
        );
    } else {
        qmf_inverse_modulation_lp_even::<FAST>(
            syn_qmf,
            real_slot,
            scale_factor_low_band,
            scale_factor_high_band,
            p_work_buffer,
        );
    }

    let l = syn_qmf.no_channels.max(0) as usize;
    let (work_real, work_imag) = p_work_buffer.split_at(l);
    if syn_qmf.flags & QMF_FLAG_NONSYMMETRIC != 0 {
        if FAST {
            qmf_syn_prototype_fir_slot_non_symmetric(
                syn_qmf, states, work_real, work_imag, time_out, stride,
            );
        } else {
            qmf_syn_prototype_fir_slot_non_symmetric_ref(
                syn_qmf, states, work_real, work_imag, time_out, stride,
            );
        }
    } else if FAST {
        qmf_syn_prototype_fir_slot(syn_qmf, states, work_real, work_imag, time_out, stride);
    } else {
        qmf_syn_prototype_fir_slot_ref(syn_qmf, states, work_real, work_imag, time_out, stride);
    }
}

/// `qmfSynthesisFiltering(synQmf, QmfBufferReal, QmfBufferImag, scaleFactor, ov_len, timeOut,
/// stride, pWorkBuffer)`: synthesis of `no_col` slots; slot `i` is written to
/// `time_out[(i * no_channels + j) * stride]`. The first `ov_len` slots have the scale of the
/// overlap (`ov_lb_scale`). `qmf_buffer_imag` is not used in low power mode (may be empty).
pub fn qmf_synthesis_filtering<T: QmfPcm, R: AsRef<[FixpDbl]>>(
    syn_qmf: &QmfFilterBank,
    states: &mut [FixpQss],
    qmf_buffer_real: &[R],
    qmf_buffer_imag: &[R],
    scale_factor: &QmfScaleFactor,
    ov_len: i32,
    time_out: &mut [T],
    stride: i32,
    p_work_buffer: &mut [FixpDbl],
) {
    let l = syn_qmf.no_channels;

    fdk_assert!(syn_qmf.no_channels >= syn_qmf.lsb);
    fdk_assert!(syn_qmf.no_channels >= syn_qmf.usb);

    // adapt scaling (the scale factors of the library cannot overflow)
    let base = -ALGORITHMIC_SCALING_IN_ANALYSIS_FILTERBANK - syn_qmf.filter_scale;
    let scale_factor_high_band = base.wrapping_sub(scale_factor.hb_scale);
    let scale_factor_low_band_ov = base.wrapping_sub(scale_factor.ov_lb_scale);
    let scale_factor_low_band_no_ov = base.wrapping_sub(scale_factor.lb_scale);

    for i in 0..syn_qmf.no_col.max(0) as usize {
        let scale_factor_low_band = if (i as i32) < ov_len {
            scale_factor_low_band_ov
        } else {
            scale_factor_low_band_no_ov
        };

        let qmf_buffer_imag_slot: &[FixpDbl] =
            if syn_qmf.flags & QMF_FLAG_LP == 0 { qmf_buffer_imag[i].as_ref() } else { &[] };

        let off = i * (l as usize) * stride as usize;
        qmf_synthesis_filtering_slot(
            syn_qmf,
            states,
            qmf_buffer_real[i].as_ref(),
            qmf_buffer_imag_slot,
            scale_factor_low_band,
            scale_factor_high_band,
            &mut time_out[off..],
            stride,
            p_work_buffer,
        );
    }
}

/// `qmfInitAnalysisFilterBank(h_Qmf, pFilterStates, noCols, lsb, usb, no_channels, flags)`:
/// `filter_states` is `pFilterStates` (`None` for `NULL`), of the `FIXP_QAS` type of the
/// instantiation (`i16` or `i32`). Returns 0 on success.
pub fn qmf_init_analysis_filter_bank<Q: Copy + Default>(
    h_qmf: &mut QmfFilterBank,
    filter_states: Option<&mut [Q]>,
    no_cols: i32,
    lsb: i32,
    usb: i32,
    no_channels: i32,
    flags: u32,
) -> i32 {
    let err = qmf_init_filter_bank(
        h_qmf,
        filter_states.is_some(),
        no_cols,
        lsb,
        usb,
        no_channels,
        flags,
        false,
    );
    if flags & QMF_FLAG_KEEP_STATES == 0 && h_qmf.filter_states {
        if let Some(states) = filter_states {
            let n = syn_states_len(h_qmf.no_channels, states.len());
            states[..n].fill(Q::default());
        }
    }

    fdk_assert!(h_qmf.no_channels >= h_qmf.lsb);

    err
}

/// The analysis coefficient caches of `ana_fir_neon` and `ana_coefs` (one per PCM type).
type AnaCache<C> = [std::sync::OnceLock<(AnaKey, Box<[C]>)>; 32];
/// (filter address, filter length, p_stride, no_channels, non-symmetric)
type AnaKey = (usize, usize, usize, usize, bool);

/// The cache slot of an analysis filter configuration and its key; `None` for a filter that is
/// not one of the prototype filters of the library or another `p_stride`.
#[inline(always)]
fn ana_slot(
    p_filter: &'static [FixpPft],
    p_stride: usize,
    no_channels: usize,
    nonsym: bool,
) -> Option<(usize, AnaKey)> {
    use crate::tools_rom::*;
    let tables: [&'static [FixpPft]; 11] = [
        &QMF_PFILT640,
        &QMF_PFILT400,
        &QMF_PFILT200,
        &QMF_PFILT120,
        &QMF_PFILT240,
        &QMF_CLDFB_640,
        &QMF_CLDFB_320,
        &QMF_CLDFB_160,
        &QMF_CLDFB_80,
        &QMF_MPSLDFB_640,
        &QMF_MPSLDFB_320,
    ];
    let t = tables.iter().position(|t| core::ptr::eq(t.as_ptr(), p_filter.as_ptr()))?;
    let sub = match p_stride {
        1 => 0,
        2 => 1,
        4 => 2,
        8 => 3,
        _ => return None,
    };
    let slot = 2 * (if t == 0 { sub } else { 3 + t }) + nonsym as usize;
    Some((slot, (p_filter.as_ptr() as usize, p_filter.len(), p_stride, no_channels, nonsym)))
}

/// The coefficients of `fast::ana_sym_coefs` / `fast::ana_nonsym_coefs` for a known filter
/// configuration with `no_channels` in `1..=64` (built lazily and checked against the
/// parameters); `None` for other configurations or states too short for the kernels.
#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
fn ana_coefs(
    cache: &'static AnaCache<FixpPft>,
    nch: usize,
    p_filter: &'static [FixpPft],
    p_stride: usize,
    states_len: usize,
    nonsym: bool,
) -> Option<&'static [FixpPft]> {
    let poly = QMF_NO_POLY as usize;
    // The ranges the scalar code reads (checked by the caller) and that the kernels use.
    let needed = if nonsym { (2 * nch) * poly * p_stride } else { nch * poly * p_stride + 5 };
    if !(1..=64).contains(&nch) || p_filter.len() < needed || states_len < 10 * nch {
        return None;
    }
    let (slot, key) = ana_slot(p_filter, p_stride, nch, nonsym)?;
    let (k, c) = cache[slot].get_or_init(|| {
        let c = if nonsym {
            fast::ana_nonsym_coefs(p_filter, p_stride, nch)
        } else {
            fast::ana_sym_coefs(p_filter, poly * p_stride, nch)
        };
        (key, c)
    });
    (*k == key).then_some(&**c)
}

/// The NEON part of the analysis prototype filter: blocks of 4 channels (symmetric) or 4
/// outputs (non-symmetric) with coefficient tables precomputed per known filter configuration
/// (`neon::ana_sym_coefs`, `neon::ana_nonsym_coefs`, built lazily and checked against the
/// parameters). Returns the number of channels / outputs computed (0 for other
/// configurations); the caller computes the rest with the scalar code.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn ana_fir_neon<S: neon::AnaState>(
    cache: &'static AnaCache<S::Coef>,
    analysis_buffer: &mut [FixpDbl],
    no_channels: usize,
    p_filter: &'static [FixpPft],
    p_stride: usize,
    states: &[S],
    nonsym: bool,
) -> usize {
    let nch = no_channels;
    let poly = QMF_NO_POLY as usize;
    // The ranges the scalar code reads (checked by the caller) and that the blocks use.
    let needed = if nonsym { (2 * nch) * poly * p_stride } else { nch * poly * p_stride + 5 };
    if nch < 4
        || p_filter.len() < needed
        || states.len() < 10 * nch
        || analysis_buffer.len() < 2 * nch
    {
        return 0;
    }
    let Some((slot, key)) = ana_slot(p_filter, p_stride, nch, nonsym) else {
        return 0;
    };
    let (k, c) = cache[slot].get_or_init(|| {
        let c = if nonsym {
            neon::ana_nonsym_coefs::<S>(p_filter, p_stride, nch)
        } else {
            neon::ana_sym_coefs::<S>(p_filter, poly * p_stride, nch)
        };
        (key, c)
    });
    if *k != key {
        return 0;
    }
    let out = analysis_buffer.as_mut_ptr();
    let st = states.as_ptr();
    if nonsym {
        let blocks = 2 * nch / 4;
        for b in 0..blocks {
            // SAFETY: the outputs 2 nch - 4 - 4b .. 2 nch - 4b and the states 4b + 2 nch p ..
            // + 4 (p < 5) are inside the buffers (2 nch and 10 nch values); the block holds 20
            // coefficients.
            unsafe { neon::ana_nonsym_block(out, st, nch, 4 * b, c[20 * b..20 * b + 20].as_ptr()) };
        }
        4 * blocks
    } else {
        let blocks = nch / 4;
        for b in 0..blocks {
            // SAFETY: the outputs 4b .. 4b + 4 and 2 nch - 4 - 4b .. 2 nch - 4b, the states
            // 10 nch - 4 - 4b - 2 nch k .. + 4 and 4b + 2 nch k .. + 4 (k < 5) are inside the
            // buffers; the block holds 40 coefficients.
            unsafe { neon::ana_sym_block(out, st, nch, 4 * b, c[40 * b..40 * b + 40].as_ptr()) };
        }
        4 * blocks
    }
}

/// `qmfAnaPrototypeFirSlot()`: analysis prototype filtering of one slot (symmetric prototype
/// filter) into `analysis_buffer` (`2 * no_channels` values). The channels from `first` on are
/// computed (the straight port computes all, `first` 0).
fn qmf_ana_prototype_fir_slot<T: QmfPcm>(
    analysis_buffer: &mut [FixpDbl],
    no_channels: usize,
    p_filter: &[FixpPft],
    p_stride: usize,
    p_filter_states: &[T::Qas],
    first: usize,
) {
    let pflt_step = QMF_NO_POLY as usize * p_stride;
    let mut p_flt = first * pflt_step;
    let mut p_data_0 = 2 * no_channels - first; // one past the next write (pre-decrement)
    let mut p_data_1 = first;

    let mut sta_0 = first;
    let mut sta_1 = 2 * QMF_NO_POLY as usize * no_channels - 1 - first;
    let sta_step1 = no_channels << 1;

    // FIR filters 127..64 0..63
    for _ in first..no_channels {
        let f = &p_filter[p_flt..p_flt + 5];
        let mut accu = T::pft_mult_div2(f[0], p_filter_states[sta_1]);
        accu = accu.wrapping_add(T::pft_mult_div2(f[1], p_filter_states[sta_1 - sta_step1]));
        accu = accu.wrapping_add(T::pft_mult_div2(f[2], p_filter_states[sta_1 - 2 * sta_step1]));
        accu = accu.wrapping_add(T::pft_mult_div2(f[3], p_filter_states[sta_1 - 3 * sta_step1]));
        accu = accu.wrapping_add(T::pft_mult_div2(f[4], p_filter_states[sta_1 - 4 * sta_step1]));
        analysis_buffer[p_data_1] = accu << 1;
        p_data_1 += 1;
        // sta_1 -= 4 * staStep1; sta_1 += staStep2 (= (no_channels << 3) - 1)
        sta_1 -= 1;

        p_flt += pflt_step;
        let f = &p_filter[p_flt..p_flt + 5];
        let mut accu = T::pft_mult_div2(f[0], p_filter_states[sta_0]);
        accu = accu.wrapping_add(T::pft_mult_div2(f[1], p_filter_states[sta_0 + sta_step1]));
        accu = accu.wrapping_add(T::pft_mult_div2(f[2], p_filter_states[sta_0 + 2 * sta_step1]));
        accu = accu.wrapping_add(T::pft_mult_div2(f[3], p_filter_states[sta_0 + 3 * sta_step1]));
        accu = accu.wrapping_add(T::pft_mult_div2(f[4], p_filter_states[sta_0 + 4 * sta_step1]));
        p_data_0 -= 1;
        analysis_buffer[p_data_0] = accu << 1;
        // sta_0 += 4 * staStep1; sta_0 -= staStep2
        sta_0 += 1;
    }
}

/// `qmfAnaPrototypeFirSlot_NonSymmetric()`: the outputs from `first` on (the straight port
/// computes all, `first` 0).
fn qmf_ana_prototype_fir_slot_non_symmetric<T: QmfPcm>(
    analysis_buffer: &mut [FixpDbl],
    no_channels: usize,
    p_filter: &[FixpPft],
    p_stride: usize,
    p_filter_states: &[T::Qas],
    first: usize,
) {
    let poly = QMF_NO_POLY as usize;
    let mut p_flt = first * poly * p_stride;

    for k in first..2 * no_channels {
        let mut accu: FixpDbl = 0;

        p_flt += poly * (p_stride - 1);

        // Perform FIR-Filter
        for p in 0..poly {
            accu = accu.wrapping_add(T::pft_mult_div2(
                p_filter[p_flt],
                p_filter_states[k + 2 * no_channels * p],
            ));
            p_flt += 1;
        }
        analysis_buffer[2 * no_channels - 1 - k] = accu << 1;
    }
}

/// `qmfAnalysisFilteringSlot(anaQmf, qmfReal, qmfImag, timeIn, stride, pWorkBuffer)`: analysis
/// of one slot of `no_channels` samples `time_in[i * stride]`. `qmf_imag` is not used in low
/// power mode (may be empty); `p_work_buffer` needs `2 * no_channels` values.
///
/// Note: `anaQmf->lsb` can be greater than `anaQmf->no_channels` in case of implicit
/// resampling (USAC with reduced 3/4 core frame length).
///
/// Deviation: for a filter bank without a valid configuration or a states buffer that is too
/// short the reference has no defined result; the port returns without output.
///
/// The same as the straight port [`qmf_analysis_filtering_slot_ref`]; on `aarch64` targets the
/// prototype filter runs in NEON for the known filter configurations (`ana_fir_neon`).
pub fn qmf_analysis_filtering_slot<T: QmfPcm>(
    ana_qmf: &QmfFilterBank,
    states: &mut [T::Qas],
    qmf_real: &mut [FixpDbl],
    qmf_imag: &mut [FixpDbl],
    time_in: &[T],
    stride: i32,
    p_work_buffer: &mut [FixpDbl],
) {
    analysis_slot::<T, true>(ana_qmf, states, qmf_real, qmf_imag, time_in, stride, p_work_buffer)
}

/// `qmfAnalysisFilteringSlot()`: the straight port of [`qmf_analysis_filtering_slot`].
pub fn qmf_analysis_filtering_slot_ref<T: QmfPcm>(
    ana_qmf: &QmfFilterBank,
    states: &mut [T::Qas],
    qmf_real: &mut [FixpDbl],
    qmf_imag: &mut [FixpDbl],
    time_in: &[T],
    stride: i32,
    p_work_buffer: &mut [FixpDbl],
) {
    analysis_slot::<T, false>(ana_qmf, states, qmf_real, qmf_imag, time_in, stride, p_work_buffer)
}

/// [`qmf_analysis_filtering_slot`] (`FAST`) and [`qmf_analysis_filtering_slot_ref`].
#[inline(always)]
fn analysis_slot<T: QmfPcm, const FAST: bool>(
    ana_qmf: &QmfFilterBank,
    states: &mut [T::Qas],
    qmf_real: &mut [FixpDbl],
    qmf_imag: &mut [FixpDbl],
    time_in: &[T],
    stride: i32,
    p_work_buffer: &mut [FixpDbl],
) {
    if !analysis_slot_ok(ana_qmf, states.len()) {
        return;
    }
    let nch = ana_qmf.no_channels.max(0) as usize;
    let offset = nch * (QMF_NO_POLY as usize * 2 - 1);
    analysis_slot_window::<T, FAST>(
        ana_qmf,
        states,
        qmf_real,
        qmf_imag,
        time_in,
        stride,
        p_work_buffer,
    );

    // Shift filter states
    //
    // Should be realized with modulo addressing on a DSP instead of a true buffer shift
    states.copy_within(nch..nch + offset, 0);
}

/// Whether [`analysis_slot`] computes anything for a states buffer of `states_len` values:
/// not for a filter bank without a valid configuration or a states buffer that is too short
/// (see [`qmf_analysis_filtering_slot`]).
#[inline]
fn analysis_slot_ok(ana_qmf: &QmfFilterBank, states_len: usize) -> bool {
    let nch = ana_qmf.no_channels.max(0) as usize;
    let p_stride = ana_qmf.p_stride as usize;
    let offset = nch * (QMF_NO_POLY as usize * 2 - 1);
    let nonsymmetric = ana_qmf.flags & QMF_FLAG_NONSYMMETRIC != 0;
    let filter_needed = if nch == 0 {
        0
    } else if nonsymmetric {
        2 * nch * QMF_NO_POLY as usize * p_stride
    } else {
        nch * QMF_NO_POLY as usize * p_stride + 5
    };
    !(states_len < offset + nch
        || ana_qmf.p_filter.len() < filter_needed
        || (nonsymmetric && p_stride == 0))
}

/// [`analysis_slot`] without the checks and the shift of the states: the input is fed into
/// `states[9 nch..10 nch]`, the prototype filter reads `states[..10 nch]`.
#[inline(always)]
fn analysis_slot_window<T: QmfPcm, const FAST: bool>(
    ana_qmf: &QmfFilterBank,
    states: &mut [T::Qas],
    qmf_real: &mut [FixpDbl],
    qmf_imag: &mut [FixpDbl],
    time_in: &[T],
    stride: i32,
    p_work_buffer: &mut [FixpDbl],
) {
    let nch = ana_qmf.no_channels.max(0) as usize;
    let p_stride = ana_qmf.p_stride as usize;
    let offset = nch * (QMF_NO_POLY as usize * 2 - 1);
    let nonsymmetric = ana_qmf.flags & QMF_FLAG_NONSYMMETRIC != 0;

    // Feed time signal into oldest anaQmf->no_channels states
    {
        let stride = stride as usize;
        let tmp = &mut states[offset..offset + nch];
        // Feed and scale actual time in slot
        for i in 0..(nch >> 1) * 2 {
            // Place INT_PCM value left aligned in scaledTimeIn
            tmp[i] = time_in[i * stride].to_qas();
        }
    }

    let first = if FAST {
        T::ana_fir_fast(p_work_buffer, nch, ana_qmf.p_filter, p_stride, states, nonsymmetric)
    } else {
        0
    };
    if nonsymmetric {
        qmf_ana_prototype_fir_slot_non_symmetric::<T>(
            p_work_buffer,
            nch,
            ana_qmf.p_filter,
            p_stride,
            states,
            first,
        );
    } else {
        qmf_ana_prototype_fir_slot::<T>(
            p_work_buffer,
            nch,
            ana_qmf.p_filter,
            p_stride,
            states,
            first,
        );
    }

    if ana_qmf.flags & QMF_FLAG_LP != 0 {
        if ana_qmf.flags & QMF_FLAG_CLDFB != 0 {
            qmf_forward_modulation_lp_odd::<FAST>(ana_qmf, p_work_buffer, qmf_real);
        } else {
            qmf_forward_modulation_lp_even::<FAST>(ana_qmf, p_work_buffer, qmf_real);
        }
    } else {
        qmf_forward_modulation_hq::<FAST>(ana_qmf, p_work_buffer, qmf_real, qmf_imag);
    }
}

/// `qmfAnalysisFiltering(anaQmf, qmfReal, qmfImag, scaleFactor, timeIn, timeIn_e, stride,
/// pWorkBuffer)`: analysis of `no_col` slots; slot `i` reads `time_in[(i * no_channels + j) *
/// stride]`. `qmf_imag` is not used in low power mode (may be empty).
pub fn qmf_analysis_filtering<T: QmfPcm, R: AsMut<[FixpDbl]>>(
    ana_qmf: &QmfFilterBank,
    states: &mut [T::Qas],
    qmf_real: &mut [R],
    qmf_imag: &mut [R],
    scale_factor: &mut QmfScaleFactor,
    time_in: &[T],
    time_in_e: i32,
    stride: i32,
    p_work_buffer: &mut [FixpDbl],
) {
    let no_channels = ana_qmf.no_channels.max(0) as usize;

    scale_factor.lb_scale = (-ALGORITHMIC_SCALING_IN_ANALYSIS_FILTERBANK).wrapping_sub(time_in_e);
    scale_factor.lb_scale = scale_factor.lb_scale.wrapping_sub(ana_qmf.filter_scale);

    let no_col = ana_qmf.no_col.max(0) as usize;
    let nch = no_channels;
    let hist = nch * (QMF_NO_POLY as usize * 2 - 1);
    // The states of the slots in a window buffer: the slot i of a block of ANA_BLOCK slots
    // reads buf[i nch..i nch + 10 nch] (its input fed at buf[(9 + i) nch..]), so the states of
    // the next slot are the window moved by nch values, and the C shift of the states
    // (`states[0..9 nch] = states[nch..10 nch]`) happens once per block. After the last slot
    // the states are buf[k nch..(k + 9) nch] followed by the last input (the C shift keeps
    // states[9 nch..10 nch]). Only for an even number of channels (the C loop feeds
    // (nch >> 1) * 2 inputs).
    const ANA_BLOCK: usize = 16;
    if !analysis_slot_ok(ana_qmf, states.len())
        || nch == 0
        || nch % 2 != 0
        || nch > 64
        || no_col < 2
        || qmf_real.len() < no_col
        || (ana_qmf.flags & QMF_FLAG_LP == 0 && qmf_imag.len() < no_col)
    {
        let mut off = 0usize;
        for i in 0..no_col {
            let qmf_imag_slot: &mut [FixpDbl] =
                if ana_qmf.flags & QMF_FLAG_LP == 0 { qmf_imag[i].as_mut() } else { &mut [] };
            qmf_analysis_filtering_slot(
                ana_qmf,
                states,
                qmf_real[i].as_mut(),
                qmf_imag_slot,
                &time_in[off..],
                stride,
                p_work_buffer,
            );
            off += no_channels * stride as usize;
        }
        return;
    }
    let mut buf = [T::Qas::default(); (2 * QMF_NO_POLY as usize - 1 + ANA_BLOCK + 1) * 64];
    buf[..hist + nch].copy_from_slice(&states[..hist + nch]);
    let mut k = 0usize; // slots in the current block
    let mut off = 0usize;
    for i in 0..no_col {
        if k == ANA_BLOCK {
            buf.copy_within(k * nch..k * nch + hist, 0);
            k = 0;
        }
        let qmf_imag_slot: &mut [FixpDbl] =
            if ana_qmf.flags & QMF_FLAG_LP == 0 { qmf_imag[i].as_mut() } else { &mut [] };
        analysis_slot_window::<T, true>(
            ana_qmf,
            &mut buf[k * nch..k * nch + hist + nch],
            qmf_real[i].as_mut(),
            qmf_imag_slot,
            &time_in[off..],
            stride,
            p_work_buffer,
        );
        k += 1;
        off += no_channels * stride as usize;
    }
    states[..hist].copy_from_slice(&buf[k * nch..k * nch + hist]);
    states[hist..hist + nch].copy_from_slice(&buf[(k - 1) * nch + hist..k * nch + hist]);
}

/// `qmfAnalysisFiltering()`: the straight port of [`qmf_analysis_filtering`] (every slot with
/// [`qmf_analysis_filtering_slot_ref`]).
#[allow(clippy::too_many_arguments)]
pub fn qmf_analysis_filtering_ref<T: QmfPcm, R: AsMut<[FixpDbl]>>(
    ana_qmf: &QmfFilterBank,
    states: &mut [T::Qas],
    qmf_real: &mut [R],
    qmf_imag: &mut [R],
    scale_factor: &mut QmfScaleFactor,
    time_in: &[T],
    time_in_e: i32,
    stride: i32,
    p_work_buffer: &mut [FixpDbl],
) {
    let no_channels = ana_qmf.no_channels.max(0) as usize;

    scale_factor.lb_scale = (-ALGORITHMIC_SCALING_IN_ANALYSIS_FILTERBANK).wrapping_sub(time_in_e);
    scale_factor.lb_scale = scale_factor.lb_scale.wrapping_sub(ana_qmf.filter_scale);

    let mut off = 0usize;
    for i in 0..ana_qmf.no_col.max(0) as usize {
        let qmf_imag_slot: &mut [FixpDbl] =
            if ana_qmf.flags & QMF_FLAG_LP == 0 { qmf_imag[i].as_mut() } else { &mut [] };

        qmf_analysis_filtering_slot_ref(
            ana_qmf,
            states,
            qmf_real[i].as_mut(),
            qmf_imag_slot,
            &time_in[off..],
            stride,
            p_work_buffer,
        );

        off += no_channels * stride as usize;
    }
}
