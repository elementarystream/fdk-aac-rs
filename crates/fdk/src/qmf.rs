// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/qmf.h, libFDK/src/qmf.cpp.

//! Port of `qmf.h` and `qmf.cpp`: the complex (and low power, real valued) QMF analysis and
//! synthesis filter banks, including the CLDFB and the MPS low delay filter banks.
//!
//! The functions that depend on the PCM type (`qmf_pcm.h`, included twice by `qmf.cpp` for
//! `INT_PCM` and `LONG`) are in [`crate::qmf_pcm`]; this module has the modulations, the
//! initialization and the scaling functions.
//!
//! # Filter states
//!
//! `QMF_FILTER_BANK::FilterStates` is a pointer to memory owned by the caller (the QMF domain,
//! the SBR encoder, the HBE transposer, ...). The port does not keep that pointer:
//! [`QmfFilterBank::filter_states`] only records whether it is non-`NULL`, and every function
//! that reads or writes the states takes them as a slice. Callers pass the buffer that the C
//! code passed to `qmfInit*FilterBank`.
//!
//! # Variants
//!
//! The `.ii` files of the `aarch64` and `x86` builds are identical for `qmf.cpp` apart from
//! `FDK_ASSERT`. The variants differ only through the primitives (`fMult` in `cplxMult`, the
//! `cplxMultDiv2` assembly of `aarch64`, the `SATURATE_*` macros) and the DCT/DST
//! ([`crate::dct`]). No `FUNCTION_qmf*` override is defined in either build: the generic
//! functions are compiled.
//!
//! # Undefined behavior
//!
//! The additions, subtractions and negations of the modulations can overflow for full-scale
//! input (signed overflow); both builds compute them with 32 bit wrapping instructions (scalar
//! `add`/`sub`/`neg`, and `add.4s`/`sub.4s`/`neg.4s`/`ssra.4s` in the loops that `aarch64`
//! vectorizes; no saturating or halving instructions), so the port uses `wrapping_*`.

use crate::common_fix::*;
use crate::cplx_mul::{cplx_mult, cplx_mult_div2};
use crate::dct::{dct_ii, dct_iii, dct_iv, dst_iv};
use crate::scale::{scale_values, scale_values_saturate, scale_values_saturate_to};
use crate::tools_rom::*;

/// `FIXP_QSS`: the type of the synthesis filter states.
pub type FixpQss = FixpDbl;
/// `QSS_BITS`
pub const QSS_BITS: i32 = DFRACT_BITS;

/// `QMF_FLAG_LP`: low power mode (real valued filter bank).
pub const QMF_FLAG_LP: u32 = 1;
/// `QMF_FLAG_NONSYMMETRIC`: the prototype filter is not symmetric (set internally).
pub const QMF_FLAG_NONSYMMETRIC: u32 = 2;
/// `QMF_FLAG_CLDFB`: complex low delay filter bank.
pub const QMF_FLAG_CLDFB: u32 = 4;
/// `QMF_FLAG_KEEP_STATES`: keep the filter states at initialization.
pub const QMF_FLAG_KEEP_STATES: u32 = 8;
/// `QMF_FLAG_MPSLDFB`: complex low delay filter bank of the MPEG Surround encoder.
pub const QMF_FLAG_MPSLDFB: u32 = 16;
/// `QMF_FLAG_MPSLDFB_OPTIMIZE_MODULATION`
pub const QMF_FLAG_MPSLDFB_OPTIMIZE_MODULATION: u32 = 32;
/// `QMF_FLAG_DOWNSAMPLED`: HE-AAC downsampled SBR (decoder), adapted analysis post twiddling.
pub const QMF_FLAG_DOWNSAMPLED: u32 = 64;

/// `QMF_MAX_SYNTHESIS_BANDS`
pub const QMF_MAX_SYNTHESIS_BANDS: i32 = 64;

/// `ALGORITHMIC_SCALING_IN_ANALYSIS_FILTERBANK`
pub const ALGORITHMIC_SCALING_IN_ANALYSIS_FILTERBANK: i32 = 7;
/// `ALGORITHMIC_SCALING_IN_SYNTHESIS_FILTERBANK`
pub const ALGORITHMIC_SCALING_IN_SYNTHESIS_FILTERBANK: i32 = 1;

/// `QMF_SCALE_FACTOR`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QmfScaleFactor {
    /// `lb_scale`: scale of the low band area
    pub lb_scale: i32,
    /// `ov_lb_scale`: scale of the adjusted overlap low band area
    pub ov_lb_scale: i32,
    /// `hb_scale`: scale of the high band area
    pub hb_scale: i32,
    /// `ov_hb_scale`: scale of the adjusted overlap high band area
    pub ov_hb_scale: i32,
}

/// `struct QMF_FILTER_BANK` (`HANDLE_QMF_FILTER_BANK` is `&mut QmfFilterBank`).
///
/// The table pointers are slices of the static tables (empty for `NULL`). `FilterStates` is not
/// kept, see the [module documentation](self).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QmfFilterBank {
    /// `p_filter`: the prototype filter coefficients
    pub p_filter: &'static [FixpPft],
    /// `FilterStates != NULL`
    pub filter_states: bool,
    /// `FilterSize`: size of the prototype filter
    pub filter_size: i32,
    /// `t_cos`: modulation table
    pub t_cos: &'static [FixpQtw],
    /// `t_sin`: modulation table
    pub t_sin: &'static [FixpQtw],
    /// `filterScale`
    pub filter_scale: i32,
    /// `no_channels`: total number of channels (subbands)
    pub no_channels: i32,
    /// `no_col`: number of time slots
    pub no_col: i32,
    /// `lsb`: top of the low subbands
    pub lsb: i32,
    /// `usb`: top of the high subbands
    pub usb: i32,
    /// `synScalefactor`: scale factor of the synthesis QMF (synthesis only)
    pub syn_scalefactor: i32,
    /// `outScalefactor`: scale factor of the output data (synthesis only)
    pub out_scalefactor: i32,
    /// `outGain_m`: mantissa of the output gain (synthesis only; `0x80000000` is not applied)
    pub out_gain_m: FixpDbl,
    /// `outGain_e`: exponent of the output gain (synthesis only)
    pub out_gain_e: i32,
    /// `flags`
    pub flags: u32,
    /// `p_stride`: stride factor of the polyphase filters
    pub p_stride: u8,
}

impl Default for QmfFilterBank {
    /// The zero-initialized structure (`FDKmemclear`).
    fn default() -> Self {
        QmfFilterBank {
            p_filter: &[],
            filter_states: false,
            filter_size: 0,
            t_cos: &[],
            t_sin: &[],
            filter_scale: 0,
            no_channels: 0,
            no_col: 0,
            lsb: 0,
            usb: 0,
            syn_scalefactor: 0,
            out_scalefactor: 0,
            out_gain_m: 0,
            out_gain_e: 0,
            flags: 0,
            p_stride: 0,
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod neon;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod sse2;

/// The DCTs and the saturating scaling of the modulations: the optimized versions (`FAST`) or
/// the straight ports (for the `_ref` slot functions of [`crate::qmf_pcm`]).
#[inline(always)]
fn dct2<const FAST: bool>(x: &mut [FixpDbl], tmp: &mut [FixpDbl], l: i32, e: &mut i32) {
    if FAST {
        dct_ii(x, tmp, l, e)
    } else {
        crate::dct::dct_ii_ref(x, tmp, l, e)
    }
}

#[inline(always)]
fn dct3<const FAST: bool>(x: &mut [FixpDbl], tmp: &mut [FixpDbl], l: i32, e: &mut i32) {
    if FAST {
        dct_iii(x, tmp, l, e)
    } else {
        crate::dct::dct_iii_ref(x, tmp, l, e)
    }
}

#[inline(always)]
fn dct4<const FAST: bool>(x: &mut [FixpDbl], l: i32, e: &mut i32) {
    if FAST {
        dct_iv(x, l, e)
    } else {
        crate::dct::dct_iv_ref(x, l, e)
    }
}

#[inline(always)]
fn dst4<const FAST: bool>(x: &mut [FixpDbl], l: i32, e: &mut i32) {
    if FAST {
        dst_iv(x, l, e)
    } else {
        crate::dct::dst_iv_ref(x, l, e)
    }
}

/// `scaleValuesSaturate(dst, src, len, sf)`: NEON on `aarch64` targets if `FAST` (the same on
/// every variant, see `crate::mdct::neon::scale_values_saturate_to`).
#[inline(always)]
fn sat_to<const FAST: bool>(dst: &mut [FixpDbl], src: &[FixpDbl], scalefactor: i32) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return crate::mdct::neon::scale_values_saturate_to(dst, src, scalefactor);
    }
    scale_values_saturate_to(dst, src, scalefactor)
}

/// `scaleValuesSaturate(vector, len, sf)`, see [`sat_to`].
#[inline(always)]
fn sat<const FAST: bool>(vector: &mut [FixpDbl], scalefactor: i32) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return crate::mdct::neon::scale_values_saturate(vector, scalefactor);
    }
    scale_values_saturate(vector, scalefactor)
}

/// `qmfForwardModulationLP_even()`: real valued forward modulation of the time domain data of
/// `time_in` (`2 * L` values, also used as temporary buffer of the DCT-III) into `r_subband`.
pub(crate) fn qmf_forward_modulation_lp_even<const FAST: bool>(
    ana_qmf: &QmfFilterBank,
    time_in: &mut [FixpDbl],
    r_subband: &mut [FixpDbl],
) {
    let l = ana_qmf.no_channels as usize;
    let m = l >> 1;
    let mut scale = 0;

    r_subband[0] = time_in[3 * m] >> 1;

    if FAST && m >= 1 {
        // The loops below on slices (vectorized; the halves cannot overflow).
        let t = &time_in[..4 * m];
        let r = &mut r_subband[..l];
        for ((d, &a), &b) in
            r[1..m].iter_mut().zip(t[2 * m + 1..3 * m].iter().rev()).zip(&t[3 * m + 1..])
        {
            *d = (a >> 1) + (b >> 1);
        }
        for ((d, &a), &b) in r[m..l].iter_mut().zip(t[3 * m + 1 - l..2 * m + 1].iter().rev()).zip(t)
        {
            *d = (a >> 1) - (b >> 1);
        }
    } else {
        for k in 1..m {
            r_subband[k] = (time_in[3 * m - k] >> 1) + (time_in[3 * m + k] >> 1);
        }

        for k in 0..(l - m) {
            r_subband[m + k] = (time_in[2 * m - k] >> 1) - (time_in[k] >> 1);
        }
    }

    dct3::<FAST>(r_subband, time_in, l as i32, &mut scale);
}

/// `qmfForwardModulationLP_odd()`
pub(crate) fn qmf_forward_modulation_lp_odd<const FAST: bool>(
    ana_qmf: &QmfFilterBank,
    time_in: &[FixpDbl],
    r_subband: &mut [FixpDbl],
) {
    let l = ana_qmf.no_channels as usize;
    let m = l >> 1;
    let mut shift = (ana_qmf.no_channels >> 6) + 1;

    if FAST {
        // The loop below on slices (vectorized; the shifted values cannot overflow).
        let t = &time_in[..2 * l];
        let (lo, hi) = r_subband[..2 * m].split_at_mut(m);
        for ((d, &a), &b) in hi.iter_mut().zip(t[l - m..l].iter().rev()).zip(&t[..m]) {
            *d = (a >> 1) - (b >> shift);
        }
        for ((d, &a), &b) in lo.iter_mut().rev().zip(&t[l..l + m]).zip(t[2 * l - m..].iter().rev())
        {
            *d = (a >> 1) + (b >> shift);
        }
    } else {
        for i in 0..m {
            r_subband[m + i] = (time_in[l - 1 - i] >> 1) - (time_in[i] >> shift);
            r_subband[m - 1 - i] = (time_in[l + i] >> 1) + (time_in[2 * l - 1 - i] >> shift);
        }
    }

    dct4::<FAST>(r_subband, l as i32, &mut shift);
}

/// `qmfForwardModulationHQ()`: complex valued forward modulation of `time_in` (`2 * L` values)
/// into `r_subband` (real part) and `i_subband` (imaginary part).
pub(crate) fn qmf_forward_modulation_hq<const FAST: bool>(
    ana_qmf: &QmfFilterBank,
    time_in: &[FixpDbl],
    r_subband: &mut [FixpDbl],
    i_subband: &mut [FixpDbl],
) {
    let l = ana_qmf.no_channels as usize;
    let l2 = l << 1;
    let mut shift = 0;

    // Time advance by one sample, which is equivalent to the complex rotation at the end of the
    // analysis. Works only for STD mode.
    if l == 64 && (ana_qmf.flags & (QMF_FLAG_CLDFB | QMF_FLAG_MPSLDFB)) == 0 {
        // rSubband[0] = u[1] + u[0]
        // iSubband[0] = u[1] - u[0]
        let x = time_in[1] >> 1;
        let y = time_in[0];
        r_subband[0] = x + (y >> 1);
        i_subband[0] = x - (y >> 1);

        // rSubband[n] = u[n+1] - u[2M-n], n=1,...,M-1
        // iSubband[n] = u[n+1] + u[2M-n], n=1,...,M-1
        if FAST {
            // On slices (vectorized; the halves cannot overflow).
            let t = &time_in[..l2];
            for (((r, i), &x), &y) in r_subband[1..l]
                .iter_mut()
                .zip(&mut i_subband[1..l])
                .zip(&t[2..l + 1])
                .zip(t[l + 1..l2].iter().rev())
            {
                let x = x >> 1; // u[n+1]
                *r = x - (y >> 1);
                *i = x + (y >> 1);
            }
        } else {
            for i in 1..l {
                let x = time_in[i + 1] >> 1; // u[n+1]
                let y = time_in[l2 - i]; // u[2M-n]
                r_subband[i] = x - (y >> 1);
                i_subband[i] = x + (y >> 1);
            }
        }
    } else if FAST && l % 2 == 0 {
        // The pairs of the loop below are r[k] = (t[k] >> 1) - (t[2l-1-k] >> 1) and
        // i[k] = (t[k] >> 1) + (t[2l-1-k] >> 1) for k < l; on slices (vectorized).
        let t = &time_in[..l2];
        for (((r, i), &x), &y) in r_subband[..l]
            .iter_mut()
            .zip(&mut i_subband[..l])
            .zip(&t[..l])
            .zip(t[l..l2].iter().rev())
        {
            let x = x >> 1;
            *r = x - (y >> 1);
            *i = x + (y >> 1);
        }
    } else {
        let mut i = 0;
        while i < l {
            let x0 = time_in[i] >> 1;
            let x1 = time_in[i + 1] >> 1;
            let y0 = time_in[l2 - 1 - i];
            let y1 = time_in[l2 - 2 - i];

            r_subband[i] = x0 - (y0 >> 1);
            r_subband[i + 1] = x1 - (y1 >> 1);
            i_subband[i] = x0 + (y0 >> 1);
            i_subband[i + 1] = x1 + (y1 >> 1);
            i += 2;
        }
    }

    dct4::<FAST>(r_subband, l as i32, &mut shift);
    dst4::<FAST>(i_subband, l as i32, &mut shift);

    // Do the complex rotation except for the case of 64 bands (in STD mode).
    if l != 64 || (ana_qmf.flags & (QMF_FLAG_CLDFB | QMF_FLAG_MPSLDFB)) != 0 {
        if ana_qmf.flags & QMF_FLAG_MPSLDFB_OPTIMIZE_MODULATION != 0 {
            let n = f_min(ana_qmf.lsb, l as i32);
            let mut i = 0usize;
            while (i as i32) < n {
                let i_band = r_subband[i];
                r_subband[i] = i_subband[i].wrapping_neg();
                i_subband[i] = i_band;

                let i_band = r_subband[i + 1].wrapping_neg();
                r_subband[i + 1] = i_subband[i + 1];
                i_subband[i + 1] = i_band;
                i += 2;
            }
        } else {
            // was len = fMin(anaQmf->lsb, L) but in case of USAC the signal above lsb is
            // actually needed in some cases (HBE?)
            //
            // Deviation: without tables the reference has no defined result; the port rotates
            // only as many bands as the tables hold.
            let len = l.min(ana_qmf.t_cos.len()).min(ana_qmf.t_sin.len());
            let sbr_t_cos = ana_qmf.t_cos;
            let sbr_t_sin = ana_qmf.t_sin;

            if FAST {
                rotate_hq(&mut i_subband[..len], &mut r_subband[..len], sbr_t_cos, sbr_t_sin);
            } else {
                for i in 0..len {
                    let (re, im) =
                        cplx_mult(i_subband[i], r_subband[i], sbr_t_cos[i], sbr_t_sin[i]);
                    i_subband[i] = re;
                    r_subband[i] = im;
                }
            }
        }
    }
}

/// The bands `lo..hi` of `scaleValuesSaturate(&x[lo], hi - lo, sf)`, `FDKmemclear(&x[lo],
/// (hi - lo) * sizeof(FIXP_DBL))` and similar calls on a slot of `l` bands: empty for a negative
/// length (the C loops do nothing).
///
/// Deviation: bands beyond `l` are left out.
#[inline]
fn band_range(lo: i32, hi: i32, l: usize) -> core::ops::Range<usize> {
    let lo = (lo.max(0) as usize).min(l);
    let hi = (hi.max(0) as usize).clamp(lo, l);
    lo..hi
}

/// The final loop of `qmfInverseModulationHQ()` (`CLDFB`: without the negations) on slices
/// (vectorized): iteration `i < m` reads and writes `t_real[i]`, `t_real[l - 1 - i]`,
/// `t_imag[i]`, `t_imag[l - 1 - i]` only.
#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
fn hq_final<const CLDFB: bool>(t_real: &mut [FixpDbl], t_imag: &mut [FixpDbl], l: usize, m: usize) {
    let (rlo, rhi) = t_real[..l].split_at_mut(l - m);
    let (ilo, ihi) = t_imag[..l].split_at_mut(l - m);
    let neg = |x: FixpDbl| if CLDFB { x } else { x.wrapping_neg() };
    for (((rl, rh), il), ih) in rlo[..m]
        .iter_mut()
        .zip(rhi[..m].iter_mut().rev())
        .zip(ilo[..m].iter_mut())
        .zip(ihi[..m].iter_mut().rev())
    {
        let r1 = neg(*rl);
        let i2 = neg(*ih);
        let r2 = neg(*rh);
        let i1 = neg(*il);
        *rl = r1.wrapping_sub(i1) >> 1;
        *ih = r1.wrapping_add(i1).wrapping_neg() >> 1;
        *rh = r2.wrapping_sub(i2) >> 1;
        *il = r2.wrapping_add(i2).wrapping_neg() >> 1;
    }
}

/// `qmfInverseModulationLP_even()`: low power inverse modulation of `qmf_real` into
/// `p_time_out` (`2 * L` values).
pub(crate) fn qmf_inverse_modulation_lp_even<const FAST: bool>(
    syn_qmf: &QmfFilterBank,
    qmf_real: &[FixpDbl],
    scale_factor_low_band: i32,
    scale_factor_high_band: i32,
    p_time_out: &mut [FixpDbl],
) {
    let l = syn_qmf.no_channels as usize;
    let m = l >> 1;
    let mut scale = 0;
    let (t_real, t_imag) = p_time_out.split_at_mut(l);

    // Move input to output vector with offset
    let lo = band_range(0, syn_qmf.lsb, l);
    sat_to::<FAST>(&mut t_real[lo.clone()], &qmf_real[lo], scale_factor_low_band);
    let hi = band_range(syn_qmf.lsb, syn_qmf.usb, l);
    sat_to::<FAST>(&mut t_real[hi.clone()], &qmf_real[hi], scale_factor_high_band);
    t_real[band_range(syn_qmf.usb, l as i32, l)].fill(0);

    // Dct type-2 transform
    dct2::<FAST>(t_real, t_imag, l as i32, &mut scale);

    // Expand output and replace inplace the output buffers
    t_imag[0] = t_real[m];
    t_imag[m] = 0;
    t_real.swap(0, m);

    #[allow(unused_mut)]
    let mut i0 = 1usize;
    #[cfg(target_arch = "aarch64")]
    if FAST {
        assert!(t_imag.len() >= l);
        while i0 + 4 <= m / 2 {
            // SAFETY: the block reads and writes t_real and t_imag at i0 .. i0 + 4,
            // m - i0 - 3 .. m - i0 + 1, m + i0 .. m + i0 + 4 and l - i0 - 3 .. l - i0 + 1, inside
            // the l values of each for 1 <= i0 and i0 + 3 < m / 2.
            unsafe { neon::lp_even_expand_block(t_real.as_mut_ptr(), t_imag.as_mut_ptr(), m, i0) };
            i0 += 4;
        }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST {
        assert!(t_real.len() >= l && t_imag.len() >= l);
        while i0 + 4 <= m / 2 {
            // SAFETY: SSE2 is part of the x86 baselines; the block reads and writes t_real and
            // t_imag at i0 .. i0 + 4, m - i0 - 3 .. m - i0 + 1, m + i0 .. m + i0 + 4 and
            // l - i0 - 3 .. l - i0 + 1, inside the l values of each for 1 <= i0 and i0 + 3 < m / 2.
            unsafe { sse2::lp_even_expand_block(t_real.as_mut_ptr(), t_imag.as_mut_ptr(), m, i0) };
            i0 += 4;
        }
    }
    for i in i0..(m / 2) {
        // Imag
        let tmp = t_real[l - i];
        t_imag[m - i] = tmp;
        t_imag[i + m] = tmp.wrapping_neg();

        let tmp = t_real[m + i];
        t_imag[i] = tmp;
        t_imag[l - i] = tmp.wrapping_neg();

        // Real
        t_real[m + i] = t_real[i];
        t_real[l - i] = t_real[m - i];
        t_real.swap(i, m - i);
    }
    // Remaining odd terms
    let tmp = t_real[m + m / 2];
    t_imag[m / 2] = tmp;
    t_imag[m / 2 + m] = tmp.wrapping_neg();

    t_real[m + m / 2] = t_real[m / 2];
}

/// `qmfInverseModulationLP_odd()`
pub(crate) fn qmf_inverse_modulation_lp_odd<const FAST: bool>(
    syn_qmf: &QmfFilterBank,
    qmf_real: &[FixpDbl],
    scale_factor_low_band: i32,
    scale_factor_high_band: i32,
    p_time_out: &mut [FixpDbl],
) {
    let l = syn_qmf.no_channels as usize;
    let m = l >> 1;
    let mut shift = 0;

    // Move input to output vector with offset
    {
        let out = &mut p_time_out[m..];
        let lo = band_range(0, syn_qmf.lsb, l);
        sat_to::<FAST>(&mut out[lo.clone()], &qmf_real[lo], scale_factor_low_band);
        let hi = band_range(syn_qmf.lsb, syn_qmf.usb, l);
        sat_to::<FAST>(&mut out[hi.clone()], &qmf_real[hi], scale_factor_high_band);
        out[band_range(syn_qmf.usb, l as i32, l)].fill(0);
    }

    dct4::<FAST>(&mut p_time_out[m..], l as i32, &mut shift);
    #[allow(unused_mut)]
    let mut i0 = 0usize;
    #[cfg(target_arch = "aarch64")]
    if FAST {
        assert!(p_time_out.len() >= 2 * l);
        while i0 + 4 <= m {
            // SAFETY: the block reads l - 4 - i0 .. l - i0 and l + i0 .. l + i0 + 4 and writes
            // i0 .. i0 + 4 and 2 l - 4 - i0 .. 2 l - i0, inside the 2 l values for i0 + 3 < m.
            unsafe { neon::lp_odd_final_block(p_time_out.as_mut_ptr(), l, i0) };
            i0 += 4;
        }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST {
        assert!(p_time_out.len() >= 2 * l);
        while i0 + 4 <= m {
            // SAFETY: SSE2 is part of the x86 baselines; as for the NEON blocks.
            unsafe { sse2::lp_odd_final_block(p_time_out.as_mut_ptr(), l, i0) };
            i0 += 4;
        }
    }
    for i in i0..m {
        p_time_out[i] = p_time_out[l - 1 - i];
        p_time_out[2 * l - 1 - i] = p_time_out[l + i].wrapping_neg();
    }
}

crate::avx2_clone! {
/// The complex rotation of `qmfForwardModulationHQ()` on slices (vectorized; on x86 also compiled
/// with AVX2, whose `vpmuldq` gives the 64-bit products of `fMult`, the same values):
/// `(i, r) = cplxMult(i, r, cos, sin)` per band.
fn rotate_hq(i_subband: &mut [FixpDbl], r_subband: &mut [FixpDbl], cos: &[FixpQtw], sin: &[FixpQtw]) {
    for (((i, r), &c), &s) in i_subband.iter_mut().zip(r_subband.iter_mut()).zip(cos).zip(sin) {
        (*i, *r) = cplx_mult(*i, *r, c, s);
    }
}
}

crate::avx2_clone! {
/// The CLDFB rotation of `qmfInverseModulationHQ()` on slices (vectorized, see [`rotate_hq`]):
/// `(t_imag, t_real) = cplxMultDiv2(q_imag, q_real, cos, sin)` per band.
fn rotate_cldfb(
    t_imag: &mut [FixpDbl],
    t_real: &mut [FixpDbl],
    q_imag: &[FixpDbl],
    q_real: &[FixpDbl],
    cos: &[FixpQtw],
    sin: &[FixpQtw],
) {
    for ((((ti, tr), (&qi, &qr)), &c), &s) in t_imag
        .iter_mut()
        .zip(t_real.iter_mut())
        .zip(q_imag.iter().zip(q_real))
        .zip(cos)
        .zip(sin)
    {
        (*ti, *tr) = cplx_mult_div2(qi, qr, c, s);
    }
}
}

/// `qmfInverseModulationHQ()`: complex valued inverse modulation of `qmf_real` and `qmf_imag`
/// into `p_work_buffer` (`2 * L` values: real part, then imaginary part).
pub(crate) fn qmf_inverse_modulation_hq<const FAST: bool>(
    syn_qmf: &QmfFilterBank,
    qmf_real: &[FixpDbl],
    qmf_imag: &[FixpDbl],
    scale_factor_low_band: i32,
    scale_factor_high_band: i32,
    p_work_buffer: &mut [FixpDbl],
) {
    let l = syn_qmf.no_channels as usize;
    let m = l >> 1;
    let mut shift = 0;
    let (t_real, t_imag) = p_work_buffer.split_at_mut(l);
    let t_imag = &mut t_imag[..l];
    let lo = band_range(0, syn_qmf.lsb, l);
    let hi = band_range(syn_qmf.lsb, syn_qmf.usb, l);

    if syn_qmf.flags & QMF_FLAG_CLDFB != 0 {
        // Deviation: without tables the reference has no defined result; the port stops at the
        // end of the tables.
        let n = band_range(0, syn_qmf.usb, l).end.min(syn_qmf.t_cos.len()).min(syn_qmf.t_sin.len());
        if FAST {
            rotate_cldfb(
                &mut t_imag[..n],
                &mut t_real[..n],
                &qmf_imag[..n],
                &qmf_real[..n],
                syn_qmf.t_cos,
                syn_qmf.t_sin,
            );
        } else {
            for i in 0..n {
                let (re, im) =
                    cplx_mult_div2(qmf_imag[i], qmf_real[i], syn_qmf.t_cos[i], syn_qmf.t_sin[i]);
                t_imag[i] = re;
                t_real[i] = im;
            }
        }
        // `scaleFactorLowBand + 1` cannot overflow for the scale factors of the library.
        let sfl = scale_factor_low_band.wrapping_add(1);
        let sfh = scale_factor_high_band.wrapping_add(1);
        sat::<FAST>(&mut t_real[lo.clone()], sfl);
        sat::<FAST>(&mut t_real[hi.clone()], sfh);
        sat::<FAST>(&mut t_imag[lo.clone()], sfl);
        sat::<FAST>(&mut t_imag[hi.clone()], sfh);
    }

    if syn_qmf.flags & QMF_FLAG_CLDFB == 0 {
        sat_to::<FAST>(&mut t_real[lo.clone()], &qmf_real[lo.clone()], scale_factor_low_band);
        sat_to::<FAST>(&mut t_real[hi.clone()], &qmf_real[hi.clone()], scale_factor_high_band);
        sat_to::<FAST>(&mut t_imag[lo.clone()], &qmf_imag[lo.clone()], scale_factor_low_band);
        sat_to::<FAST>(&mut t_imag[hi.clone()], &qmf_imag[hi], scale_factor_high_band);
    }

    t_real[band_range(syn_qmf.usb, l as i32, l)].fill(0);
    t_imag[band_range(syn_qmf.usb, l as i32, l)].fill(0);

    dct4::<FAST>(t_real, l as i32, &mut shift);
    dst4::<FAST>(t_imag, l as i32, &mut shift);

    #[allow(unused_mut)]
    let mut i0 = 0usize;
    #[cfg(not(target_arch = "aarch64"))]
    if FAST {
        if syn_qmf.flags & QMF_FLAG_CLDFB != 0 {
            hq_final::<true>(t_real, t_imag, l, m);
        } else {
            hq_final::<false>(t_real, t_imag, l, m);
        }
        i0 = m;
    }
    #[cfg(target_arch = "aarch64")]
    if FAST {
        let cldfb = syn_qmf.flags & QMF_FLAG_CLDFB != 0;
        while i0 + 4 <= m {
            // SAFETY: the block reads and writes t_real and t_imag at i0 .. i0 + 4 and
            // l - 4 - i0 .. l - i0, inside the l values of each for i0 + 3 < m = l / 2.
            unsafe {
                if cldfb {
                    neon::hq_final_block::<true>(t_real.as_mut_ptr(), t_imag.as_mut_ptr(), l, i0);
                } else {
                    neon::hq_final_block::<false>(t_real.as_mut_ptr(), t_imag.as_mut_ptr(), l, i0);
                }
            }
            i0 += 4;
        }
    }
    if syn_qmf.flags & QMF_FLAG_CLDFB != 0 {
        for i in i0..m {
            let r1 = t_real[i];
            let i2 = t_imag[l - 1 - i];
            let r2 = t_real[l - i - 1];
            let i1 = t_imag[i];

            t_real[i] = r1.wrapping_sub(i1) >> 1;
            t_imag[l - 1 - i] = r1.wrapping_add(i1).wrapping_neg() >> 1;
            t_real[l - i - 1] = r2.wrapping_sub(i2) >> 1;
            t_imag[i] = r2.wrapping_add(i2).wrapping_neg() >> 1;
        }
    } else {
        // The array accesses are negative to compensate the missing minus sign in the low and
        // hi band gain.
        for i in i0..m {
            let r1 = t_real[i].wrapping_neg();
            let i2 = t_imag[l - 1 - i].wrapping_neg();
            let r2 = t_real[l - i - 1].wrapping_neg();
            let i1 = t_imag[i].wrapping_neg();

            t_real[i] = r1.wrapping_sub(i1) >> 1;
            t_imag[l - 1 - i] = r1.wrapping_add(i1).wrapping_neg() >> 1;
            t_real[l - i - 1] = r2.wrapping_sub(i2) >> 1;
            t_imag[i] = r2.wrapping_add(i2).wrapping_neg() >> 1;
        }
    }
}

/// `qmfInitFilterBank()`: sets up a filter bank; `filter_states` is `pFilterStates != NULL`.
/// Returns 0 on success, -1 for an unsupported number of channels.
pub(crate) fn qmf_init_filter_bank(
    h_qmf: &mut QmfFilterBank,
    filter_states: bool,
    no_cols: i32,
    lsb: i32,
    usb: i32,
    no_channels: i32,
    mut flags: u32,
    synflag: bool,
) -> i32 {
    *h_qmf = QmfFilterBank::default();

    if flags & QMF_FLAG_MPSLDFB != 0 {
        flags |= QMF_FLAG_NONSYMMETRIC;
        flags |= QMF_FLAG_MPSLDFB_OPTIMIZE_MODULATION;

        h_qmf.t_cos = &[];
        h_qmf.t_sin = &[];
        h_qmf.filter_scale = QMF_MPSLDFB_PFT_SCALE;
        h_qmf.p_stride = 1;

        match no_channels {
            64 => {
                h_qmf.p_filter = &QMF_MPSLDFB_640;
                h_qmf.filter_size = 640;
            }
            32 => {
                h_qmf.p_filter = &QMF_MPSLDFB_320;
                h_qmf.filter_size = 320;
            }
            _ => return -1,
        }
    }

    if flags & QMF_FLAG_MPSLDFB == 0 && flags & QMF_FLAG_CLDFB != 0 {
        flags |= QMF_FLAG_NONSYMMETRIC;
        h_qmf.filter_scale = QMF_CLDFB_PFT_SCALE;

        h_qmf.p_stride = 1;
        match no_channels {
            64 => {
                h_qmf.t_cos = &QMF_PHASESHIFT_COS64_CLDFB;
                h_qmf.t_sin = &QMF_PHASESHIFT_SIN64_CLDFB;
                h_qmf.p_filter = &QMF_CLDFB_640;
                h_qmf.filter_size = 640;
            }
            32 => {
                h_qmf.t_cos = if synflag {
                    &QMF_PHASESHIFT_COS32_CLDFB_SYN
                } else {
                    &QMF_PHASESHIFT_COS32_CLDFB_ANA
                };
                h_qmf.t_sin = &QMF_PHASESHIFT_SIN32_CLDFB;
                h_qmf.p_filter = &QMF_CLDFB_320;
                h_qmf.filter_size = 320;
            }
            16 => {
                h_qmf.t_cos = if synflag {
                    &QMF_PHASESHIFT_COS16_CLDFB_SYN
                } else {
                    &QMF_PHASESHIFT_COS16_CLDFB_ANA
                };
                h_qmf.t_sin = &QMF_PHASESHIFT_SIN16_CLDFB;
                h_qmf.p_filter = &QMF_CLDFB_160;
                h_qmf.filter_size = 160;
            }
            8 => {
                h_qmf.t_cos = if synflag {
                    &QMF_PHASESHIFT_COS8_CLDFB_SYN
                } else {
                    &QMF_PHASESHIFT_COS8_CLDFB_ANA
                };
                h_qmf.t_sin = &QMF_PHASESHIFT_SIN8_CLDFB;
                h_qmf.p_filter = &QMF_CLDFB_80;
                h_qmf.filter_size = 80;
            }
            _ => return -1,
        }
    }

    if flags & QMF_FLAG_MPSLDFB == 0 && flags & QMF_FLAG_CLDFB == 0 {
        match no_channels {
            64 => {
                h_qmf.p_filter = &QMF_PFILT640;
                h_qmf.t_cos = &QMF_PHASESHIFT_COS64;
                h_qmf.t_sin = &QMF_PHASESHIFT_SIN64;
                h_qmf.p_stride = 1;
                h_qmf.filter_size = 640;
                h_qmf.filter_scale = 0;
            }
            40 => {
                if !synflag {
                    h_qmf.p_filter = &QMF_PFILT400; // Scaling factor 0.8
                    h_qmf.t_cos = &QMF_PHASESHIFT_COS40;
                    h_qmf.t_sin = &QMF_PHASESHIFT_SIN40;
                    h_qmf.filter_scale = 1;
                    h_qmf.p_stride = 1;
                    h_qmf.filter_size = no_channels * 10;
                }
            }
            32 => {
                h_qmf.p_filter = &QMF_PFILT640;
                if flags & QMF_FLAG_DOWNSAMPLED != 0 {
                    h_qmf.t_cos = &QMF_PHASESHIFT_COS_DOWNSAMP32;
                    h_qmf.t_sin = &QMF_PHASESHIFT_SIN_DOWNSAMP32;
                } else {
                    h_qmf.t_cos = &QMF_PHASESHIFT_COS32;
                    h_qmf.t_sin = &QMF_PHASESHIFT_SIN32;
                }
                h_qmf.p_stride = 2;
                h_qmf.filter_size = 640;
                h_qmf.filter_scale = 0;
            }
            20 => {
                h_qmf.p_filter = &QMF_PFILT200;
                h_qmf.p_stride = 1;
                h_qmf.filter_size = 200;
                h_qmf.filter_scale = 0;
            }
            12 => {
                h_qmf.p_filter = &QMF_PFILT120;
                h_qmf.p_stride = 1;
                h_qmf.filter_size = 120;
                h_qmf.filter_scale = 0;
            }
            8 => {
                h_qmf.p_filter = &QMF_PFILT640;
                h_qmf.p_stride = 8;
                h_qmf.filter_size = 640;
                h_qmf.filter_scale = 0;
            }
            16 => {
                h_qmf.p_filter = &QMF_PFILT640;
                h_qmf.t_cos = &QMF_PHASESHIFT_COS16;
                h_qmf.t_sin = &QMF_PHASESHIFT_SIN16;
                h_qmf.p_stride = 4;
                h_qmf.filter_size = 640;
                h_qmf.filter_scale = 0;
            }
            24 => {
                h_qmf.p_filter = &QMF_PFILT240;
                h_qmf.t_cos = &QMF_PHASESHIFT_COS24;
                h_qmf.t_sin = &QMF_PHASESHIFT_SIN24;
                h_qmf.p_stride = 1;
                h_qmf.filter_size = 240;
                h_qmf.filter_scale = 1;
            }
            _ => return -1,
        }
    }

    h_qmf.syn_scalefactor = h_qmf.filter_scale;
    // DCT|DST dependency
    h_qmf.syn_scalefactor += match no_channels {
        128 => ALGORITHMIC_SCALING_IN_SYNTHESIS_FILTERBANK + 1,
        40 => ALGORITHMIC_SCALING_IN_SYNTHESIS_FILTERBANK - 1,
        64 => ALGORITHMIC_SCALING_IN_SYNTHESIS_FILTERBANK,
        8 => ALGORITHMIC_SCALING_IN_SYNTHESIS_FILTERBANK - 3,
        12 => ALGORITHMIC_SCALING_IN_SYNTHESIS_FILTERBANK,
        20 => ALGORITHMIC_SCALING_IN_SYNTHESIS_FILTERBANK + 1,
        32 => ALGORITHMIC_SCALING_IN_SYNTHESIS_FILTERBANK - 1,
        16 => ALGORITHMIC_SCALING_IN_SYNTHESIS_FILTERBANK - 2,
        24 => ALGORITHMIC_SCALING_IN_SYNTHESIS_FILTERBANK - 1,
        _ => return -1,
    };

    h_qmf.flags = flags;

    h_qmf.no_channels = no_channels;
    h_qmf.no_col = no_cols;

    h_qmf.lsb = f_min(lsb, h_qmf.no_channels);
    // was: h_Qmf->usb = fMin(usb, h_Qmf->no_channels);
    h_qmf.usb = if synflag { f_min(usb, h_qmf.no_channels) } else { usb };

    h_qmf.filter_states = filter_states;

    h_qmf.out_scalefactor =
        (ALGORITHMIC_SCALING_IN_ANALYSIS_FILTERBANK + h_qmf.filter_scale) + h_qmf.syn_scalefactor;

    h_qmf.out_gain_m = MINVAL_DBL; // default init value will be not applied
    h_qmf.out_gain_e = 0;

    0
}

/// The number of synthesis filter states, `no_channels * (QMF_NO_POLY * 2 - 1)`, limited to
/// the length of `states`.
///
/// Deviation: for a states buffer that is too short for the number of channels the reference
/// has no defined result; the port stops at its end.
#[inline]
pub(crate) fn syn_states_len(no_channels: i32, states_len: usize) -> usize {
    ((no_channels.max(0) * (QMF_NO_POLY * 2 - 1)) as usize).min(states_len)
}

/// `qmfAdaptFilterStates()`: scales the synthesis filter states by `2^scale_factor_diff`.
pub(crate) fn qmf_adapt_filter_states(
    syn_qmf: &QmfFilterBank,
    states: &mut [FixpQss],
    scale_factor_diff: i32,
) {
    if !syn_qmf.filter_states {
        return;
    }
    let n = syn_states_len(syn_qmf.no_channels, states.len());
    if scale_factor_diff > 0 {
        scale_values_saturate(&mut states[..n], scale_factor_diff);
    } else {
        scale_values(&mut states[..n], scale_factor_diff);
    }
}

/// `qmfInitSynthesisFilterBank(h_Qmf, pFilterStates, noCols, lsb, usb, no_channels, flags)`:
/// `filter_states` is `pFilterStates` (`None` for `NULL`). Returns 0 on success.
pub fn qmf_init_synthesis_filter_bank(
    h_qmf: &mut QmfFilterBank,
    filter_states: Option<&mut [FixpQss]>,
    no_cols: i32,
    lsb: i32,
    usb: i32,
    no_channels: i32,
    flags: u32,
) -> i32 {
    let old_out_scale = h_qmf.out_scalefactor;
    let err = qmf_init_filter_bank(
        h_qmf,
        filter_states.is_some(),
        no_cols,
        lsb,
        usb,
        no_channels,
        flags,
        true,
    );
    if let Some(states) = filter_states {
        if h_qmf.filter_states {
            if flags & QMF_FLAG_KEEP_STATES == 0 {
                let n = syn_states_len(h_qmf.no_channels, states.len());
                states[..n].fill(0);
            } else {
                // `oldOutScale - h_Qmf->outScalefactor` does not overflow for the scale factors
                // of the library.
                qmf_adapt_filter_states(
                    h_qmf,
                    states,
                    old_out_scale.wrapping_sub(h_qmf.out_scalefactor),
                );
            }
        }
    }

    fdk_assert!(h_qmf.no_channels >= h_qmf.lsb);
    fdk_assert!(h_qmf.no_channels >= h_qmf.usb);

    err
}

/// `qmfChangeOutScalefactor(synQmf, outScalefactor)`: changes the scale factor of the output
/// data and adapts the filter `states`.
pub fn qmf_change_out_scalefactor(
    syn_qmf: &mut QmfFilterBank,
    states: &mut [FixpQss],
    mut out_scalefactor: i32,
) {
    // Add internal filterbank scale
    out_scalefactor = out_scalefactor.wrapping_add(
        (ALGORITHMIC_SCALING_IN_ANALYSIS_FILTERBANK + syn_qmf.filter_scale)
            + syn_qmf.syn_scalefactor,
    );

    // adjust filter states when scale factor has been changed
    if syn_qmf.out_scalefactor != out_scalefactor {
        let diff = syn_qmf.out_scalefactor.wrapping_sub(out_scalefactor);

        qmf_adapt_filter_states(syn_qmf, states, diff);

        // save new scale factor
        syn_qmf.out_scalefactor = out_scalefactor;
    }
}

/// `qmfGetOutScalefactor(synQmf)`: the scale factor set by [`qmf_change_out_scalefactor`].
pub fn qmf_get_out_scalefactor(syn_qmf: &QmfFilterBank) -> i32 {
    if syn_qmf.out_scalefactor != 0 {
        syn_qmf.out_scalefactor.wrapping_sub(
            ALGORITHMIC_SCALING_IN_ANALYSIS_FILTERBANK
                + syn_qmf.filter_scale
                + syn_qmf.syn_scalefactor,
        )
    } else {
        0
    }
}

/// `qmfChangeOutGain(synQmf, outputGain, outputGainScale)`
pub fn qmf_change_out_gain(
    syn_qmf: &mut QmfFilterBank,
    output_gain: FixpDbl,
    output_gain_scale: i32,
) {
    syn_qmf.out_gain_m = output_gain;
    syn_qmf.out_gain_e = output_gain_scale;
}

pub use crate::qmf_pcm::{
    qmf_analysis_filtering, qmf_analysis_filtering_slot, qmf_init_analysis_filter_bank,
    qmf_synthesis_filtering, qmf_synthesis_filtering_slot, QmfPcm,
};
