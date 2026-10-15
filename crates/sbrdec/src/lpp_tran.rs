// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/lpp_tran.h, libSBRdec/src/lpp_tran.cpp.

//! Port of `lpp_tran.h` and `lpp_tran.cpp`: the low power profile transposer. It generates the
//! high band by copying ("patching") low band QMF data into the SBR range, with spectral
//! whitening by inverse filtering based on LPC coefficients.
//!
//! # Slots
//!
//! The QMF data (`FIXP_DBL **qmfBufferReal`) is a slice of time slots (`&mut [&mut [FixpDbl]]`,
//! one slice of 64 bands per slot, see [`crate::sbr_dec`]); a `NULL` imaginary buffer (low power
//! mode) is an empty slice.
//!
//! # Filter states
//!
//! The four LPC filter state arrays of `SBR_LPP_TRANS` are consecutive in memory
//! (`lpcFilterStatesRealLegSBR[14][32]`, `...ImagLegSBR[14][32]`, `...RealHBE[14][64]`,
//! `...ImagHBE[14][64]`); the port keeps them as one array ([`SbrLppTrans::lpc`]) with the same
//! layout, so an index past the end of a row lands where the C code reads.
//!
//! # Variants
//!
//! The `.ii` files of the two builds differ only in `SATURATE_LEFT_SHIFT` (`scale_arm.h` vs
//! `scale.h`), which is ported per variant in `fdk_aac_rs_fdk::scale`; the amounts used here
//! (`rescale` in 1..=3) are in range, so both give the same values.

use crate::hbe::HbeTransposer;
use crate::hfgen_pre_flat::sbr_decoder_calculate_gain_vec;
use crate::sbr_ram::pod_struct;
use crate::sbr_rom::*;
use crate::sbrdecoder::{SbrError, SBRDEC_OK, SBRDEC_UNSUPPORTED_CONFIG};
use crate::transcendent::f_div_norm_fast;
use fdk_aac_rs_fdk::autocorr2nd::{auto_corr2nd_cplx, auto_corr2nd_real, AcorrCoefs};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::qmf::QmfScaleFactor;

mod lp_bands;
#[cfg(test)]
mod tests;

/// `QMF_OUT_SCALE`
pub const QMF_OUT_SCALE: i32 = 8;

/// `MAX_NOISE_ENVELOPES`
pub const MAX_NOISE_ENVELOPES: usize = 2;
/// `MAX_NOISE_COEFFS`
pub const MAX_NOISE_COEFFS: usize = 5;
/// `MAX_NUM_NOISE_VALUES`
pub const MAX_NUM_NOISE_VALUES: usize = MAX_NOISE_ENVELOPES * MAX_NOISE_COEFFS;
/// `MAX_NUM_LIMITERS`
pub const MAX_NUM_LIMITERS: usize = 12;

/// `MAX_ENVELOPES_LEGACY`
pub const MAX_ENVELOPES_LEGACY: usize = 5;
/// `MAX_ENVELOPES_USAC`
pub const MAX_ENVELOPES_USAC: usize = 8;
/// `MAX_ENVELOPES`
pub const MAX_ENVELOPES: usize = MAX_ENVELOPES_USAC;

/// `MAX_FREQ_COEFFS_DUAL_RATE`
pub const MAX_FREQ_COEFFS_DUAL_RATE: usize = 48;
/// `MAX_FREQ_COEFFS_QUAD_RATE`
pub const MAX_FREQ_COEFFS_QUAD_RATE: usize = 56;
/// `MAX_FREQ_COEFFS`
pub const MAX_FREQ_COEFFS: usize = MAX_FREQ_COEFFS_QUAD_RATE;

/// `MAX_FREQ_COEFFS_FS44100`
pub const MAX_FREQ_COEFFS_FS44100: i32 = 35;
/// `MAX_FREQ_COEFFS_FS48000`
pub const MAX_FREQ_COEFFS_FS48000: i32 = 32;

/// `MAX_NUM_ENVELOPE_VALUES`
pub const MAX_NUM_ENVELOPE_VALUES: usize = MAX_ENVELOPES * MAX_FREQ_COEFFS;

/// `MAX_GAIN_EXP`: maximum gain will be sqrt(0.5 * 2^MAX_GAIN_EXP)
pub const MAX_GAIN_EXP: i32 = 34;
/// `MAX_GAIN_CONCEAL_EXP`: maximum gain in the concealment case
pub const MAX_GAIN_CONCEAL_EXP: i32 = 1;

/// `LPC_ORDER`
pub const LPC_ORDER: usize = 2;

/// `MAX_INVF_BANDS`
pub const MAX_INVF_BANDS: usize = MAX_NOISE_COEFFS;

/// `MAX_NUM_PATCHES`
pub const MAX_NUM_PATCHES: usize = 6;
/// `SHIFT_START_SB`: lowest subband of source range
pub const SHIFT_START_SB: i32 = 1;

/// `(3 * (4))`: the maximum number of QMF overlap slots.
pub const MAX_OV_COLS: usize = 3 * 4;
/// `((1024) / (32) * (4) / 2)`: the maximum number of QMF slots of a frame.
pub const MAX_COLS: usize = 1024 / 32 * 4 / 2;

/// `INVF_MODE`
pub type InvfMode = i32;
/// `INVF_OFF`
pub const INVF_OFF: InvfMode = 0;
/// `INVF_LOW_LEVEL`
pub const INVF_LOW_LEVEL: InvfMode = 1;
/// `INVF_MID_LEVEL`
pub const INVF_MID_LEVEL: InvfMode = 2;
/// `INVF_HIGH_LEVEL`
pub const INVF_HIGH_LEVEL: InvfMode = 3;
/// `INVF_SWITCHED`: not a real choice but used here to control behaviour
pub const INVF_SWITCHED: InvfMode = 4;

pod_struct! {
    /// `PATCH_PARAM`: parameter set for one single patch
    pub struct PatchParam {
        /// `sourceStartBand`: first band in lowbands where to take the samples from
        pub source_start_band: u8,
        /// `sourceStopBand`: first band in lowbands which is not included in the patch anymore
        pub source_stop_band: u8,
        /// `guardStartBand`: first band in highbands to be filled with zeros
        pub guard_start_band: u8,
        /// `targetStartBand`: first band in highbands to be filled with whitened lowband signal
        pub target_start_band: u8,
        /// `targetBandOffs`: difference between `startTargetBand` and `startSourceBand`
        pub target_band_offs: u8,
        /// `numBandsInPatch`: number of consecutive bands in this one patch
        pub num_bands_in_patch: u8,
    }
}

pod_struct! {
    /// `WHITENING_FACTORS`: whitening factors for different levels of whitening
    pub struct WhiteningFactors {
        /// `off`: bw factor for signal OFF
        pub off: FixpDbl,
        /// `transitionLevel`
        pub transition_level: FixpDbl,
        /// `lowLevel`: bw factor for signal LOW_LEVEL
        pub low_level: FixpDbl,
        /// `midLevel`: bw factor for signal MID_LEVEL
        pub mid_level: FixpDbl,
        /// `highLevel`: bw factor for signal HIGH_LEVEL
        pub high_level: FixpDbl,
    }
}

pod_struct! {
    /// `TRANSPOSER_SETTINGS`: the transposer settings are calculated on a header reset and are
    /// shared by both channels of an element.
    pub struct TransposerSettings {
        /// `nCols`: number subsamples of a codec frame
        pub n_cols: u8,
        /// `noOfPatches`: number of patches
        pub no_of_patches: u8,
        /// `lbStartPatching`: first band of lowbands that will be patched
        pub lb_start_patching: u8,
        /// `lbStopPatching`: first band that won't be patched anymore
        pub lb_stop_patching: u8,
        /// `bwBorders`: spectral bands with different inverse filtering levels
        pub bw_borders: [u8; MAX_NUM_NOISE_VALUES],
        /// `patchParam`: new parameter set for patching
        pub patch_param: [PatchParam; MAX_NUM_PATCHES + 1],
        /// `whFactors`: the pole moving factors for certain whitening levels
        pub wh_factors: WhiteningFactors,
        /// `overlap`: overlap size
        pub overlap: u8,
        pub _pad0: [u8; 3],
    }
}

/// Offset of `lpcFilterStatesRealLegSBR` in [`SbrLppTrans::lpc`].
pub const LPC_LEG_RE: usize = 0;
/// Offset of `lpcFilterStatesImagLegSBR` in [`SbrLppTrans::lpc`].
pub const LPC_LEG_IM: usize = LPC_LEG_RE + (LPC_ORDER + MAX_OV_COLS) * 32;
/// Offset of `lpcFilterStatesRealHBE` in [`SbrLppTrans::lpc`].
pub const LPC_HBE_RE: usize = LPC_LEG_IM + (LPC_ORDER + MAX_OV_COLS) * 32;
/// Offset of `lpcFilterStatesImagHBE` in [`SbrLppTrans::lpc`].
pub const LPC_HBE_IM: usize = LPC_HBE_RE + (LPC_ORDER + MAX_OV_COLS) * 64;
/// Size of [`SbrLppTrans::lpc`].
pub const LPC_STATES_SIZE: usize = LPC_HBE_IM + (LPC_ORDER + MAX_OV_COLS) * 64;
/// Number of columns of a row of the legacy SBR filter states.
pub const LPC_LEG_COLS: usize = 32;
/// Number of columns of a row of the HBE filter states.
pub const LPC_HBE_COLS: usize = 64;

pod_struct! {
    /// `SBR_LPP_TRANS`. `pSettings` points to the element's [`TransposerSettings`], which the
    /// functions take as a separate parameter.
    pub struct SbrLppTrans {
        /// `pSettings` (placeholder, see [`crate::sbr_ram`])
        pub _p_settings: usize,
        /// `bwVectorOld`: pole moving factors of past frame
        pub bw_vector_old: [FixpDbl; MAX_NUM_PATCHES],
        /// `lpcFilterStatesRealLegSBR[14][32]`, `lpcFilterStatesImagLegSBR[14][32]`,
        /// `lpcFilterStatesRealHBE[14][64]`, `lpcFilterStatesImagHBE[14][64]` (see the
        /// [module documentation](self))
        pub lpc: [FixpDbl; LPC_STATES_SIZE],
    }
}

impl SbrLppTrans {
    /// `lpcFilterStatesRealLegSBR[i]` (to the end of the states)
    pub fn leg_re(&mut self, i: usize) -> &mut [FixpDbl] {
        &mut self.lpc[(LPC_LEG_RE + i * LPC_LEG_COLS).min(LPC_STATES_SIZE)..]
    }
    /// `lpcFilterStatesImagLegSBR[i]` (to the end of the states)
    pub fn leg_im(&mut self, i: usize) -> &mut [FixpDbl] {
        &mut self.lpc[(LPC_LEG_IM + i * LPC_LEG_COLS).min(LPC_STATES_SIZE)..]
    }
    /// `lpcFilterStatesRealHBE[i]` (to the end of the states)
    pub fn hbe_re(&mut self, i: usize) -> &mut [FixpDbl] {
        &mut self.lpc[(LPC_HBE_RE + i * LPC_HBE_COLS).min(LPC_STATES_SIZE)..]
    }
    /// `lpcFilterStatesImagHBE[i]` (to the end of the states)
    pub fn hbe_im(&mut self, i: usize) -> &mut [FixpDbl] {
        &mut self.lpc[(LPC_HBE_IM + i * LPC_HBE_COLS).min(LPC_STATES_SIZE)..]
    }
    /// The value at `off` of the filter states, 0 outside of them (no defined result in the
    /// reference).
    #[inline(always)]
    fn at(&self, off: usize) -> FixpDbl {
        self.lpc.get(off).copied().unwrap_or(0)
    }
    /// Writes the value at `off` of the filter states; ignored outside of them.
    #[inline(always)]
    fn set(&mut self, off: usize, v: FixpDbl) {
        if let Some(p) = self.lpc.get_mut(off) {
            *p = v;
        }
    }
    /// The HBE filter states `(real, imag)` as two separate slices of 14 rows of 64 values.
    pub fn hbe_states_mut(&mut self) -> (&mut [FixpDbl], &mut [FixpDbl]) {
        let (_, hbe) = self.lpc.split_at_mut(LPC_HBE_RE);
        hbe.split_at_mut(LPC_HBE_IM - LPC_HBE_RE)
    }
}

/// `LPC_SCALE_FACTOR`
const LPC_SCALE_FACTOR: i32 = 2;

/// `mapInvfMode(mode, prevMode, whFactors)`: the bandwidth expansion factor of an inverse
/// filtering level. When switching from LOW to OFF, an additional level is inserted to achieve a
/// smooth transition.
fn map_invf_mode(mode: InvfMode, prev_mode: InvfMode, wh_factors: &WhiteningFactors) -> FixpDbl {
    match mode {
        INVF_LOW_LEVEL => {
            if prev_mode == INVF_OFF {
                wh_factors.transition_level
            } else {
                wh_factors.low_level
            }
        }
        INVF_MID_LEVEL => wh_factors.mid_level,
        INVF_HIGH_LEVEL => wh_factors.high_level,
        _ => {
            if prev_mode == INVF_LOW_LEVEL {
                wh_factors.transition_level
            } else {
                wh_factors.off
            }
        }
    }
}

/// `inverseFilteringLevelEmphasis()`: retrieves the bandwidth expansion factor and applies
/// smoothing for each filter band.
///
/// `nInvfBands` is at most 5 (`nNfb`); the port limits it to the arrays.
fn inverse_filtering_level_emphasis(
    h_lpp_trans: &SbrLppTrans,
    wh_factors: &WhiteningFactors,
    n_invf_bands: u8,
    sbr_invf_mode: &[InvfMode],
    sbr_invf_mode_prev: &[InvfMode],
    bw_vector: &mut [FixpDbl],
) {
    let n = (n_invf_bands as usize).min(MAX_INVF_BANDS);
    for i in 0..n {
        let bw_tmp = map_invf_mode(sbr_invf_mode[i], sbr_invf_mode_prev[i], wh_factors);

        let accu = if bw_tmp < h_lpp_trans.bw_vector_old[i] {
            f_mult_div2(fl2fxconst_dbl(0.75f32 as f64), bw_tmp)
                + f_mult_div2(fl2fxconst_dbl(0.25f32 as f64), h_lpp_trans.bw_vector_old[i])
        } else {
            f_mult_div2(fl2fxconst_dbl(0.90625f32 as f64), bw_tmp)
                + f_mult_div2(fl2fxconst_dbl(0.09375f32 as f64), h_lpp_trans.bw_vector_old[i])
        };

        if accu < fl2fxconst_dbl(0.015625f32 as f64) >> 1 {
            bw_vector[i] = 0;
        } else {
            bw_vector[i] = fix_min(accu << 1, fl2fxconst_dbl(0.99609375f32 as f64));
        }
    }
}

/// `calc_qmfBufferReal()`: the low power filtering of one band. `low_band_real[i]` is
/// `lowBandReal[i]` of the C function (which starts 2 values before the first output slot).
///
/// The straight port, kept as the reference of the optimized [`calc_qmf_buffer_real`].
#[allow(clippy::too_many_arguments)]
pub fn calc_qmf_buffer_real_straight(
    qmf_buffer_real: &mut [&mut [FixpDbl]],
    low_band_real: &[FixpDbl],
    start_sample: i32,
    stop_sample: i32,
    hi_band: u8,
    dynamic_scale: i32,
    a0r: FixpSgl,
    a1r: FixpSgl,
) {
    let dynscale = fix_max(0, dynamic_scale - 1) + 1;
    let rescale = -fix_min(0, dynamic_scale - 1) + 1;
    let descale = fix_min(DFRACT_BITS - 1, LPC_SCALE_FACTOR + dynamic_scale + rescale);

    for i in 0..(stop_sample - start_sample).max(0) as usize {
        let mut accu =
            f_mult_div2(a1r, low_band_real[i]).wrapping_add(f_mult_div2(a0r, low_band_real[i + 1]));
        accu = (low_band_real[i + 2] >> descale).wrapping_add(accu >> dynscale);

        qmf_buffer_real[i + start_sample as usize][hi_band as usize] =
            saturate_left_shift(accu, rescale, DFRACT_BITS);
    }
}

/// `calc_qmfBufferReal()`, optimized (see [`calc_qmf_buffer_real_straight`]): the slots are
/// independent; on aarch64 (the `aarch64` variant) four of them are filtered at a time with NEON
/// (see `neon_lpc`), otherwise in a loop over slices. Ranges outside the buffers and shift
/// amounts outside `0..32` fall back to the straight port.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn calc_qmf_buffer_real(
    qmf_buffer_real: &mut [&mut [FixpDbl]],
    low_band_real: &[FixpDbl],
    start_sample: i32,
    stop_sample: i32,
    hi_band: u8,
    dynamic_scale: i32,
    a0r: FixpSgl,
    a1r: FixpSgl,
) {
    let n = (stop_sample - start_sample).max(0) as usize;
    let dynscale = fix_max(0, dynamic_scale - 1) + 1;
    let rescale = -fix_min(0, dynamic_scale - 1) + 1;
    let descale = fix_min(DFRACT_BITS - 1, LPC_SCALE_FACTOR + dynamic_scale + rescale);
    if start_sample < 0
        || start_sample as usize + n > qmf_buffer_real.len()
        || n + 2 > low_band_real.len()
        || !(0..32).contains(&dynscale)
        || !(0..32).contains(&descale)
        || !(0..32).contains(&rescale)
    {
        return calc_qmf_buffer_real_straight(
            qmf_buffer_real,
            low_band_real,
            start_sample,
            stop_sample,
            hi_band,
            dynamic_scale,
            a0r,
            a1r,
        );
    }
    let s = start_sample as usize;
    let hb = hi_band as usize;
    let slots = &mut qmf_buffer_real[s..s + n];
    let (x2, x1, x0) = (&low_band_real[..n], &low_band_real[1..n + 1], &low_band_real[2..n + 2]);
    #[allow(unused_mut)]
    let mut k0 = 0;
    #[cfg(target_arch = "aarch64")]
    if matches!(fdk_aac_rs_fdk::ARCH, fdk_aac_rs_fdk::Arch::Aarch64) {
        k0 = n & !3;
        neon_lpc::real(slots, hb, x0, x1, x2, k0, [a0r, a1r], [descale, dynscale, rescale]);
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if fdk_aac_rs_fdk::simd_x86::avx2() {
        k0 = n & !7;
        // SAFETY: the CPU has AVX2.
        unsafe {
            x86_lpc::real(slots, hb, x0, x1, x2, k0, [a0r, a1r], [descale, dynscale, rescale])
        };
    }
    for k in k0..n {
        let accu = f_mult_div2(a1r, x2[k]).wrapping_add(f_mult_div2(a0r, x1[k]));
        let accu = (x0[k] >> descale).wrapping_add(accu >> dynscale);
        slots[k][hb] = saturate_left_shift(accu, rescale, DFRACT_BITS);
    }
}

/// Filter step 3 of `lppTransposer()` in high quality mode with a bandwidth factor above 0: the
/// second order LPC filtering of the source band into the slots `start_sample..stop_sample` of
/// the high band `hb`. `low_band_real`/`low_band_imag` start `LPC_ORDER` values before slot 0;
/// `a` holds `a0r, a0i, a1r, a1i`; `pre_whitening` the gain and exponent of the source band.
///
/// The straight port (the loop of the C function), kept as the reference of the optimized
/// [`lpp_filter_cplx`].
#[allow(clippy::too_many_arguments)]
pub fn lpp_filter_cplx_straight(
    qmf_buffer_real: &mut [&mut [FixpDbl]],
    qmf_buffer_imag: &mut [&mut [FixpDbl]],
    low_band_real: &[FixpDbl],
    low_band_imag: &[FixpDbl],
    start_sample: i32,
    stop_sample: i32,
    hb: usize,
    a: [FixpSgl; 4],
    dynamic_scale: i32,
    pre_whitening: Option<(FixpDbl, i32)>,
) {
    let [a0r, a0i, a1r, a1i] = a;
    let dynscale = fix_max(0, dynamic_scale - 2) + 1;
    let rescale = -fix_min(0, dynamic_scale - 2) + 1;
    let descale = fix_min(DFRACT_BITS - 1, LPC_SCALE_FACTOR + dynamic_scale + rescale);

    for i in start_sample..stop_sample {
        let iu = i as usize;
        let r1 = low_band_real[LPC_ORDER + iu - 1];
        let i1 = low_band_imag[LPC_ORDER + iu - 1];
        let r2 = low_band_real[LPC_ORDER + iu - 2];
        let i2 = low_band_imag[LPC_ORDER + iu - 2];
        let mut accu1 = (f_mult_div2(a0r, r1).wrapping_sub(f_mult_div2(a0i, i1)) >> 1)
            .wrapping_add(f_mult_div2(a1r, r2).wrapping_sub(f_mult_div2(a1i, i2)) >> 1);
        let mut accu2 = (f_mult_div2(a0i, r1).wrapping_add(f_mult_div2(a0r, i1)) >> 1)
            .wrapping_add(f_mult_div2(a1i, r2).wrapping_add(f_mult_div2(a1r, i2)) >> 1);

        accu1 = (low_band_real[LPC_ORDER + iu] >> descale).wrapping_add(accu1 >> dynscale);
        accu2 = (low_band_imag[LPC_ORDER + iu] >> descale).wrapping_add(accu2 >> dynscale);
        if let Some((gain, gain_exp)) = pre_whitening {
            qmf_buffer_real[iu][hb] =
                scale_value_saturate(f_mult_div2(accu1, gain), gain_exp + 1 + rescale);
            qmf_buffer_imag[iu][hb] =
                scale_value_saturate(f_mult_div2(accu2, gain), gain_exp + 1 + rescale);
        } else {
            qmf_buffer_real[iu][hb] = saturate_left_shift(accu1, rescale, DFRACT_BITS);
            qmf_buffer_imag[iu][hb] = saturate_left_shift(accu2, rescale, DFRACT_BITS);
        }
    }
}

/// [`lpp_filter_cplx_straight`], optimized: the slots are independent; on aarch64 (the
/// `aarch64` variant, without pre-whitening) four of them are filtered at a time with NEON (see
/// `neon_lpc`), otherwise in a loop over slices. Ranges outside the buffers and shift amounts
/// outside `0..32` fall back to the straight port.
#[allow(clippy::too_many_arguments)]
pub fn lpp_filter_cplx(
    qmf_buffer_real: &mut [&mut [FixpDbl]],
    qmf_buffer_imag: &mut [&mut [FixpDbl]],
    low_band_real: &[FixpDbl],
    low_band_imag: &[FixpDbl],
    start_sample: i32,
    stop_sample: i32,
    hb: usize,
    a: [FixpSgl; 4],
    dynamic_scale: i32,
    pre_whitening: Option<(FixpDbl, i32)>,
) {
    let n = (stop_sample - start_sample).max(0) as usize;
    let dynscale = fix_max(0, dynamic_scale - 2) + 1;
    let rescale = -fix_min(0, dynamic_scale - 2) + 1;
    let descale = fix_min(DFRACT_BITS - 1, LPC_SCALE_FACTOR + dynamic_scale + rescale);
    if start_sample < 0
        || stop_sample as usize > qmf_buffer_real.len().min(qmf_buffer_imag.len())
        || LPC_ORDER + stop_sample as usize > low_band_real.len().min(low_band_imag.len())
        || !(0..32).contains(&dynscale)
        || !(0..32).contains(&descale)
        || !(0..32).contains(&rescale)
    {
        return lpp_filter_cplx_straight(
            qmf_buffer_real,
            qmf_buffer_imag,
            low_band_real,
            low_band_imag,
            start_sample,
            stop_sample,
            hb,
            a,
            dynamic_scale,
            pre_whitening,
        );
    }
    let [a0r, a0i, a1r, a1i] = a;
    let s = start_sample as usize;
    let o = LPC_ORDER + s;
    let slots_r = &mut qmf_buffer_real[s..s + n];
    let slots_i = &mut qmf_buffer_imag[s..s + n];
    // x0[k] = lowBand[LPC_ORDER + s + k], x1[k] = x0[k - 1], x2[k] = x0[k - 2]
    let r = [
        &low_band_real[o..o + n],
        &low_band_real[o - 1..o - 1 + n],
        &low_band_real[o - 2..o - 2 + n],
    ];
    let i = [
        &low_band_imag[o..o + n],
        &low_band_imag[o - 1..o - 1 + n],
        &low_band_imag[o - 2..o - 2 + n],
    ];
    #[allow(unused_mut)]
    let mut k0 = 0;
    #[cfg(target_arch = "aarch64")]
    if pre_whitening.is_none() && matches!(fdk_aac_rs_fdk::ARCH, fdk_aac_rs_fdk::Arch::Aarch64) {
        k0 = n & !3;
        neon_lpc::cplx(slots_r, slots_i, hb, r, i, k0, a, [descale, dynscale, rescale]);
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if pre_whitening.is_none() && fdk_aac_rs_fdk::simd_x86::avx2() {
        k0 = n & !7;
        // SAFETY: the CPU has AVX2.
        unsafe { x86_lpc::cplx(slots_r, slots_i, hb, r, i, k0, a, [descale, dynscale, rescale]) };
    }
    for k in k0..n {
        let (r1, i1, r2, i2) = (r[1][k], i[1][k], r[2][k], i[2][k]);
        let accu1 = (f_mult_div2(a0r, r1).wrapping_sub(f_mult_div2(a0i, i1)) >> 1)
            .wrapping_add(f_mult_div2(a1r, r2).wrapping_sub(f_mult_div2(a1i, i2)) >> 1);
        let accu2 = (f_mult_div2(a0i, r1).wrapping_add(f_mult_div2(a0r, i1)) >> 1)
            .wrapping_add(f_mult_div2(a1i, r2).wrapping_add(f_mult_div2(a1r, i2)) >> 1);
        let accu1 = (r[0][k] >> descale).wrapping_add(accu1 >> dynscale);
        let accu2 = (i[0][k] >> descale).wrapping_add(accu2 >> dynscale);
        if let Some((gain, gain_exp)) = pre_whitening {
            slots_r[k][hb] = scale_value_saturate(f_mult_div2(accu1, gain), gain_exp + 1 + rescale);
            slots_i[k][hb] = scale_value_saturate(f_mult_div2(accu2, gain), gain_exp + 1 + rescale);
        } else {
            slots_r[k][hb] = saturate_left_shift(accu1, rescale, DFRACT_BITS);
            slots_i[k][hb] = saturate_left_shift(accu2, rescale, DFRACT_BITS);
        }
    }
}

/// NEON kernels of the LPC filters (aarch64 variant).
///
/// `fMultDiv2(FIXP_SGL a, FIXP_DBL x)` is `((a << 16) * x) >> 32`; `sqdmulh(x, a << 15)` computes
/// `(2 * x * (a << 15)) >> 32`, the same value, and cannot saturate (`a << 15` is never
/// `INT_MIN`). The shifts by register amounts are `sshl` (arithmetic right shifts by `0..32`,
/// wrapping left shifts by `0..32`), `SATURATE_LEFT_SHIFT` is the formula of `arm/scale_arm.h`.
#[cfg(target_arch = "aarch64")]
mod neon_lpc {
    use super::{FixpDbl, FixpSgl, MAXVAL_DBL};
    use core::arch::aarch64::*;

    #[inline(always)]
    unsafe fn ld(x: &[FixpDbl], k: usize) -> int32x4_t {
        debug_assert!(k + 4 <= x.len());
        vld1q_s32(x.as_ptr().add(k))
    }

    /// `fMultDiv2(a, x)` for four values of `x`; `a2` is `a << 15` in every lane.
    #[inline(always)]
    unsafe fn mdiv2(a2: int32x4_t, x: int32x4_t) -> int32x4_t {
        vqdmulhq_s32(x, a2)
    }

    /// `SATURATE_LEFT_SHIFT(v, scale, DFRACT_BITS)` of `arm/scale_arm.h`: `lim` is
    /// `MAXVAL_DBL >> scale`, `sh` is `scale` in every lane.
    #[inline(always)]
    unsafe fn sat_lshift(v: int32x4_t, lim: int32x4_t, sh: int32x4_t) -> int32x4_t {
        let sign = vshrq_n_s32::<31>(v);
        let over = vcgtq_s32(veorq_s32(v, sign), lim);
        vbslq_s32(over, veorq_s32(sign, vdupq_n_s32(MAXVAL_DBL)), vshlq_s32(v, sh))
    }

    #[inline(always)]
    fn dup_a(a: FixpSgl) -> i32 {
        (a as i32) << 15
    }

    /// Slots `0..k_end` (a multiple of 4) of `calc_qmf_buffer_real`; `sc` holds `descale`,
    /// `dynscale`, `rescale` (all in `0..32`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn real(
        slots: &mut [&mut [FixpDbl]],
        hb: usize,
        x0: &[FixpDbl],
        x1: &[FixpDbl],
        x2: &[FixpDbl],
        k_end: usize,
        a: [FixpSgl; 2],
        sc: [i32; 3],
    ) {
        assert!(
            k_end <= x0.len() && k_end <= x1.len() && k_end <= x2.len() && k_end <= slots.len()
        );
        // SAFETY: NEON is part of aarch64; the loads read x0/x1/x2[k..k + 4] with k + 4 <= k_end.
        unsafe {
            let (a0, a1) = (vdupq_n_s32(dup_a(a[0])), vdupq_n_s32(dup_a(a[1])));
            let (de, dy) = (vdupq_n_s32(-sc[0]), vdupq_n_s32(-sc[1]));
            let (lim, sh) = (vdupq_n_s32(MAXVAL_DBL >> sc[2]), vdupq_n_s32(sc[2]));
            let mut k = 0;
            while k < k_end {
                let accu = vaddq_s32(mdiv2(a1, ld(x2, k)), mdiv2(a0, ld(x1, k)));
                let accu = vaddq_s32(vshlq_s32(ld(x0, k), de), vshlq_s32(accu, dy));
                let out = sat_lshift(accu, lim, sh);
                slots[k][hb] = vgetq_lane_s32::<0>(out);
                slots[k + 1][hb] = vgetq_lane_s32::<1>(out);
                slots[k + 2][hb] = vgetq_lane_s32::<2>(out);
                slots[k + 3][hb] = vgetq_lane_s32::<3>(out);
                k += 4;
            }
        }
    }

    /// Slots `0..k_end` (a multiple of 4) of `lpp_filter_cplx` without pre-whitening; `r`/`i`
    /// are `x0`, `x1`, `x2` of the real and imaginary parts, `sc` holds `descale`, `dynscale`,
    /// `rescale` (all in `0..32`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn cplx(
        slots_r: &mut [&mut [FixpDbl]],
        slots_i: &mut [&mut [FixpDbl]],
        hb: usize,
        r: [&[FixpDbl]; 3],
        i: [&[FixpDbl]; 3],
        k_end: usize,
        a: [FixpSgl; 4],
        sc: [i32; 3],
    ) {
        assert!(r.iter().chain(i.iter()).all(|x| k_end <= x.len()));
        assert!(k_end <= slots_r.len() && k_end <= slots_i.len());
        // SAFETY: NEON is part of aarch64; the loads read x[k..k + 4] with k + 4 <= k_end.
        unsafe {
            let [a0r, a0i, a1r, a1i] = a.map(|v| vdupq_n_s32(dup_a(v)));
            let (de, dy) = (vdupq_n_s32(-sc[0]), vdupq_n_s32(-sc[1]));
            let (lim, sh) = (vdupq_n_s32(MAXVAL_DBL >> sc[2]), vdupq_n_s32(sc[2]));
            let mut k = 0;
            while k < k_end {
                let (r1, i1, r2, i2) = (ld(r[1], k), ld(i[1], k), ld(r[2], k), ld(i[2], k));
                let accu1 = vaddq_s32(
                    vshrq_n_s32::<1>(vsubq_s32(mdiv2(a0r, r1), mdiv2(a0i, i1))),
                    vshrq_n_s32::<1>(vsubq_s32(mdiv2(a1r, r2), mdiv2(a1i, i2))),
                );
                let accu2 = vaddq_s32(
                    vshrq_n_s32::<1>(vaddq_s32(mdiv2(a0i, r1), mdiv2(a0r, i1))),
                    vshrq_n_s32::<1>(vaddq_s32(mdiv2(a1i, r2), mdiv2(a1r, i2))),
                );
                let accu1 = vaddq_s32(vshlq_s32(ld(r[0], k), de), vshlq_s32(accu1, dy));
                let accu2 = vaddq_s32(vshlq_s32(ld(i[0], k), de), vshlq_s32(accu2, dy));
                let o1 = sat_lshift(accu1, lim, sh);
                let o2 = sat_lshift(accu2, lim, sh);
                slots_r[k][hb] = vgetq_lane_s32::<0>(o1);
                slots_r[k + 1][hb] = vgetq_lane_s32::<1>(o1);
                slots_r[k + 2][hb] = vgetq_lane_s32::<2>(o1);
                slots_r[k + 3][hb] = vgetq_lane_s32::<3>(o1);
                slots_i[k][hb] = vgetq_lane_s32::<0>(o2);
                slots_i[k + 1][hb] = vgetq_lane_s32::<1>(o2);
                slots_i[k + 2][hb] = vgetq_lane_s32::<2>(o2);
                slots_i[k + 3][hb] = vgetq_lane_s32::<3>(o2);
                k += 4;
            }
        }
    }
}

/// AVX2 kernels of the LPC filters (x86 hosts, selected at run time; every variant, since the
/// arithmetic is the same on all of them).
///
/// `fMultDiv2(FIXP_SGL a, FIXP_DBL x)` is `(a * x) >> 16`, computed exactly in 32 bit lanes as
/// `(x >> 16) * a + (((x & 0xFFFF) * a) >> 16)` (`|a * (x & 0xFFFF)| < 2^31`). The shifts by
/// the uniform amounts `0..32` are `vpsrad`/`vpslld` with a register count. `SATURATE_LEFT_SHIFT`
/// is `MAXVAL_DBL` above `MAXVAL_DBL >> scale`, `MINVAL_DBL` below `!(MAXVAL_DBL >> scale)` and
/// the shifted value otherwise (the generic form, which the `arm/scale_arm.h` form equals).
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86_lpc {
    use super::{FixpDbl, FixpSgl, MAXVAL_DBL, MINVAL_DBL};
    #[cfg(target_arch = "x86")]
    use core::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;

    #[inline(always)]
    unsafe fn ld(x: &[FixpDbl], k: usize) -> __m256i {
        debug_assert!(k + 8 <= x.len());
        _mm256_loadu_si256(x.as_ptr().add(k) as *const __m256i)
    }

    /// `fMultDiv2(a, x)` for eight values of `x`; `a` in every lane.
    #[inline(always)]
    unsafe fn mdiv2(a: __m256i, x: __m256i) -> __m256i {
        let hi = _mm256_mullo_epi32(_mm256_srai_epi32::<16>(x), a);
        let lo = _mm256_and_si256(x, _mm256_set1_epi32(0xFFFF));
        _mm256_add_epi32(hi, _mm256_srai_epi32::<16>(_mm256_mullo_epi32(lo, a)))
    }

    /// `SATURATE_LEFT_SHIFT(v, scale, DFRACT_BITS)`: `lim` is `MAXVAL_DBL >> scale` in every
    /// lane, `sh` the count `scale`.
    #[inline(always)]
    unsafe fn sat_lshift(v: __m256i, lim: __m256i, sh: __m128i) -> __m256i {
        let over = _mm256_cmpgt_epi32(v, lim);
        let under = _mm256_cmpgt_epi32(_mm256_xor_si256(lim, _mm256_set1_epi32(-1)), v);
        let r = _mm256_blendv_epi8(_mm256_sll_epi32(v, sh), _mm256_set1_epi32(MAXVAL_DBL), over);
        _mm256_blendv_epi8(r, _mm256_set1_epi32(MINVAL_DBL), under)
    }

    #[inline(always)]
    unsafe fn store(slots: &mut [&mut [FixpDbl]], k: usize, hb: usize, v: __m256i) {
        let mut t = [0 as FixpDbl; 8];
        _mm256_storeu_si256(t.as_mut_ptr() as *mut __m256i, v);
        for (l, &t) in t.iter().enumerate() {
            slots[k + l][hb] = t;
        }
    }

    /// Slots `0..k_end` (a multiple of 8) of `calc_qmf_buffer_real`; `sc` holds `descale`,
    /// `dynscale`, `rescale` (all in `0..32`).
    ///
    /// # Safety
    /// The CPU must have AVX2.
    #[allow(clippy::too_many_arguments)]
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn real(
        slots: &mut [&mut [FixpDbl]],
        hb: usize,
        x0: &[FixpDbl],
        x1: &[FixpDbl],
        x2: &[FixpDbl],
        k_end: usize,
        a: [FixpSgl; 2],
        sc: [i32; 3],
    ) {
        assert!(
            k_end <= x0.len() && k_end <= x1.len() && k_end <= x2.len() && k_end <= slots.len()
        );
        let (a0, a1) = (_mm256_set1_epi32(a[0] as i32), _mm256_set1_epi32(a[1] as i32));
        let (de, dy) = (_mm_cvtsi32_si128(sc[0]), _mm_cvtsi32_si128(sc[1]));
        let (lim, sh) = (_mm256_set1_epi32(MAXVAL_DBL >> sc[2]), _mm_cvtsi32_si128(sc[2]));
        let mut k = 0;
        while k < k_end {
            let accu = _mm256_add_epi32(mdiv2(a1, ld(x2, k)), mdiv2(a0, ld(x1, k)));
            let accu =
                _mm256_add_epi32(_mm256_sra_epi32(ld(x0, k), de), _mm256_sra_epi32(accu, dy));
            store(slots, k, hb, sat_lshift(accu, lim, sh));
            k += 8;
        }
    }

    /// Slots `0..k_end` (a multiple of 8) of `lpp_filter_cplx` without pre-whitening; `r`/`i`
    /// are `x0`, `x1`, `x2` of the real and imaginary parts, `sc` holds `descale`, `dynscale`,
    /// `rescale` (all in `0..32`).
    ///
    /// # Safety
    /// The CPU must have AVX2.
    #[allow(clippy::too_many_arguments)]
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn cplx(
        slots_r: &mut [&mut [FixpDbl]],
        slots_i: &mut [&mut [FixpDbl]],
        hb: usize,
        r: [&[FixpDbl]; 3],
        i: [&[FixpDbl]; 3],
        k_end: usize,
        a: [FixpSgl; 4],
        sc: [i32; 3],
    ) {
        assert!(r.iter().chain(i.iter()).all(|x| k_end <= x.len()));
        assert!(k_end <= slots_r.len() && k_end <= slots_i.len());
        let [a0r, a0i, a1r, a1i] = a.map(|v| _mm256_set1_epi32(v as i32));
        let (de, dy) = (_mm_cvtsi32_si128(sc[0]), _mm_cvtsi32_si128(sc[1]));
        let (lim, sh) = (_mm256_set1_epi32(MAXVAL_DBL >> sc[2]), _mm_cvtsi32_si128(sc[2]));
        let mut k = 0;
        while k < k_end {
            let (r1, i1, r2, i2) = (ld(r[1], k), ld(i[1], k), ld(r[2], k), ld(i[2], k));
            let accu1 = _mm256_add_epi32(
                _mm256_srai_epi32::<1>(_mm256_sub_epi32(mdiv2(a0r, r1), mdiv2(a0i, i1))),
                _mm256_srai_epi32::<1>(_mm256_sub_epi32(mdiv2(a1r, r2), mdiv2(a1i, i2))),
            );
            let accu2 = _mm256_add_epi32(
                _mm256_srai_epi32::<1>(_mm256_add_epi32(mdiv2(a0i, r1), mdiv2(a0r, i1))),
                _mm256_srai_epi32::<1>(_mm256_add_epi32(mdiv2(a1i, r2), mdiv2(a1r, i2))),
            );
            let accu1 =
                _mm256_add_epi32(_mm256_sra_epi32(ld(r[0], k), de), _mm256_sra_epi32(accu1, dy));
            let accu2 =
                _mm256_add_epi32(_mm256_sra_epi32(ld(i[0], k), de), _mm256_sra_epi32(accu2, dy));
            store(slots_r, k, hb, sat_lshift(accu1, lim, sh));
            store(slots_i, k, hb, sat_lshift(accu2, lim, sh));
            k += 8;
        }
    }
}

/// `lppTransposer()`: transposition by patching of subband samples, with spectral whitening by
/// inverse filtering. `p_settings` is `hLppTrans->pSettings`.
///
/// `use_lp` selects the low power (real valued) mode; `qmf_buffer_imag` is not used then (may be
/// empty). `degree_alias` is only used in low power mode.
///
/// In low power mode the band loop runs eight bands at a time where the configuration allows it
/// (see `lp_bands`); [`lpp_transposer_straight`] always runs the loop of the straight port.
#[allow(clippy::too_many_arguments)]
pub fn lpp_transposer(
    h_lpp_trans: &mut SbrLppTrans,
    p_settings: &TransposerSettings,
    sbr_scale_factor: &mut QmfScaleFactor,
    qmf_buffer_real: &mut [&mut [FixpDbl]],
    degree_alias: &mut [FixpDbl],
    qmf_buffer_imag: &mut [&mut [FixpDbl]],
    use_lp: bool,
    f_pre_whitening: i32,
    v_k_master0: i32,
    time_step: i32,
    first_slot_offs: i32,
    last_slot_offs: i32,
    n_invf_bands: i32,
    sbr_invf_mode: &[InvfMode],
    sbr_invf_mode_prev: &[InvfMode],
) {
    lpp_transposer_with(
        h_lpp_trans,
        p_settings,
        sbr_scale_factor,
        qmf_buffer_real,
        degree_alias,
        qmf_buffer_imag,
        use_lp,
        f_pre_whitening,
        v_k_master0,
        time_step,
        first_slot_offs,
        last_slot_offs,
        n_invf_bands,
        sbr_invf_mode,
        sbr_invf_mode_prev,
        true,
    )
}

/// [`lpp_transposer`] with the band loop of the straight port (the reference of the band
/// parallel loop, for tests).
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn lpp_transposer_straight(
    h_lpp_trans: &mut SbrLppTrans,
    p_settings: &TransposerSettings,
    sbr_scale_factor: &mut QmfScaleFactor,
    qmf_buffer_real: &mut [&mut [FixpDbl]],
    degree_alias: &mut [FixpDbl],
    qmf_buffer_imag: &mut [&mut [FixpDbl]],
    use_lp: bool,
    f_pre_whitening: i32,
    v_k_master0: i32,
    time_step: i32,
    first_slot_offs: i32,
    last_slot_offs: i32,
    n_invf_bands: i32,
    sbr_invf_mode: &[InvfMode],
    sbr_invf_mode_prev: &[InvfMode],
) {
    lpp_transposer_with(
        h_lpp_trans,
        p_settings,
        sbr_scale_factor,
        qmf_buffer_real,
        degree_alias,
        qmf_buffer_imag,
        use_lp,
        f_pre_whitening,
        v_k_master0,
        time_step,
        first_slot_offs,
        last_slot_offs,
        n_invf_bands,
        sbr_invf_mode,
        sbr_invf_mode_prev,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn lpp_transposer_with(
    h_lpp_trans: &mut SbrLppTrans,
    p_settings: &TransposerSettings,
    sbr_scale_factor: &mut QmfScaleFactor,
    qmf_buffer_real: &mut [&mut [FixpDbl]],
    degree_alias: &mut [FixpDbl],
    qmf_buffer_imag: &mut [&mut [FixpDbl]],
    use_lp: bool,
    f_pre_whitening: i32,
    v_k_master0: i32,
    time_step: i32,
    first_slot_offs: i32,
    last_slot_offs: i32,
    n_invf_bands: i32,
    sbr_invf_mode: &[InvfMode],
    sbr_invf_mode_prev: &[InvfMode],
    bands: bool,
) {
    let mut bw_index = [0i32; MAX_NUM_PATCHES];
    let mut bw_vector = [0 as FixpDbl; MAX_NUM_PATCHES];
    // DEVIATION: not initialized in the reference. sbrDecoder_calculateGainVec() only sets the
    // first v_k_master0 gains; for the others the reference has no defined result, the port
    // reads 0.
    let mut pre_whitening_gains = [0 as FixpDbl; 64 / 2];
    let mut pre_whitening_gains_exp = [0i32; 64 / 2];

    let patch_param = &p_settings.patch_param;

    let mut alphar: [FixpSgl; LPC_ORDER] = [0; LPC_ORDER];
    let mut alphai: [FixpSgl; LPC_ORDER] = [0; LPC_ORDER];
    let mut a0i: FixpSgl = 0;
    let mut a1i: FixpSgl = 0;

    let mut k1: FixpDbl;
    let mut k1_below: FixpDbl = 0;
    let mut k1_below2: FixpDbl = 0;

    let mut ac = AcorrCoefs::default();

    let start_sample = first_slot_offs * time_step;
    let stop_sample = p_settings.n_cols as i32 + last_slot_offs * time_step;
    fdk_assert!((last_slot_offs * time_step) <= p_settings.overlap as i32);

    inverse_filtering_level_emphasis(
        h_lpp_trans,
        &p_settings.wh_factors,
        n_invf_bands as u8,
        sbr_invf_mode,
        sbr_invf_mode_prev,
        &mut bw_vector,
    );

    let stop_sample_clear = stop_sample;

    let auto_corr_length = p_settings.n_cols as i32 + p_settings.overlap as i32;

    if p_settings.no_of_patches > 0 {
        // Set upper subbands to zero: this is required in case that the patches do not cover
        // the complete highband (because the last patch would be too short).
        let last = &patch_param[p_settings.no_of_patches as usize - 1];
        let target_stop_band = last.target_start_band as usize + last.num_bands_in_patch as usize;

        // Bounded: a target stop band above 64 clears nothing.
        for i in start_sample.max(0)..stop_sample_clear {
            if let Some(s) = qmf_buffer_real[i as usize].get_mut(target_stop_band..64) {
                s.fill(0);
            }
            if !use_lp {
                if let Some(s) = qmf_buffer_imag[i as usize].get_mut(target_stop_band..64) {
                    s.fill(0);
                }
            }
        }
    }

    // init bwIndex for each patch
    bw_index.fill(0);

    // Calc common low band scale factor
    let com_low_band_scale = fix_min(sbr_scale_factor.ov_lb_scale, sbr_scale_factor.lb_scale);

    let ov_low_band_shift = sbr_scale_factor.ov_lb_scale - com_low_band_scale;
    let low_band_shift = sbr_scale_factor.lb_scale - com_low_band_scale;

    if f_pre_whitening != 0 {
        sbr_decoder_calculate_gain_vec(
            qmf_buffer_real,
            qmf_buffer_imag,
            DFRACT_BITS - 1 - 16 - sbr_scale_factor.ov_lb_scale, // convert scale to exponent
            DFRACT_BITS - 1 - 16 - sbr_scale_factor.lb_scale,    // convert scale to exponent
            p_settings.overlap as i32,
            &mut pre_whitening_gains,
            &mut pre_whitening_gains_exp,
            v_k_master0,
            start_sample,
            stop_sample,
        );
    }

    // outer loop over bands to do analysis only once for each band
    let (start, stop) = if !use_lp {
        (p_settings.lb_start_patching as i32, p_settings.lb_stop_patching as i32)
    } else {
        (
            fix_max(1, p_settings.lb_start_patching as i32 - 2),
            patch_param[0].target_start_band as i32,
        )
    };

    let n_cols = p_settings.n_cols as i32;
    let overlap = p_settings.overlap as i32;
    let ord = LPC_ORDER as i32;
    let lp_bands_done = bands
        && use_lp
        && lp_bands::run(
            &lp_bands::Params {
                h_lpp_trans,
                p_settings,
                bw_vector: &bw_vector,
                start,
                stop,
                first_slot_offs,
                time_step,
                start_sample,
                stop_sample,
                com_low_band_scale,
                ov_low_band_shift,
                low_band_shift,
                v_k_master0,
            },
            qmf_buffer_real,
            degree_alias,
        );
    for lo_band in start..if lp_bands_done { start } else { stop } {
        let mut low_band_real = [0 as FixpDbl; MAX_COLS + MAX_OV_COLS + LPC_ORDER];
        let mut low_band_imag = [0 as FixpDbl; MAX_COLS + MAX_OV_COLS + LPC_ORDER];
        let mut p_low = 0usize;
        let mut p_low_i = 0usize;
        let mut p_qmf = (first_slot_offs * time_step) as usize;
        let mut reset_lpc_coeffs = false;
        let mut dynamic_scale = DFRACT_BITS - 1 - LPC_SCALE_FACTOR;
        let mut ac_det_scale = 0i32; // scaling of autocorrelation determinant

        for i in 0..(ord + first_slot_offs * time_step) as usize {
            low_band_real[p_low] = h_lpp_trans.at(LPC_LEG_RE + i * LPC_LEG_COLS + lo_band as usize);
            p_low += 1;
            if !use_lp {
                low_band_imag[p_low_i] =
                    h_lpp_trans.at(LPC_LEG_IM + i * LPC_LEG_COLS + lo_band as usize);
                p_low_i += 1;
            }
        }

        // Take old slope length qmf slot source values out of (overlap)qmf buffer
        if !use_lp {
            for _ in 0..(n_cols + overlap - first_slot_offs * time_step).max(0) {
                low_band_real[p_low] = qmf_buffer_real[p_qmf][lo_band as usize];
                low_band_imag[p_low_i] = qmf_buffer_imag[p_qmf][lo_band as usize];
                p_low += 1;
                p_low_i += 1;
                p_qmf += 1;
            }
        } else {
            // pSettings->overlap is always even
            fdk_assert!((overlap & 1) == 0);
            for _ in 0..((n_cols + overlap - first_slot_offs * time_step) >> 1).max(0) {
                low_band_real[p_low] = qmf_buffer_real[p_qmf][lo_band as usize];
                p_low += 1;
                p_qmf += 1;
                low_band_real[p_low] = qmf_buffer_real[p_qmf][lo_band as usize];
                p_low += 1;
                p_qmf += 1;
            }
            if n_cols & 1 != 0 {
                low_band_real[p_low] = qmf_buffer_real[p_qmf][lo_band as usize];
            }
        }

        let ov_end = (ord + overlap) as usize;
        let n_end = ov_end + n_cols as usize;

        // Determine dynamic scaling value.
        dynamic_scale =
            fix_min(dynamic_scale, get_scalefactor(&low_band_real[..ov_end]) + ov_low_band_shift);
        dynamic_scale =
            fix_min(dynamic_scale, get_scalefactor(&low_band_real[ov_end..n_end]) + low_band_shift);
        if !use_lp {
            dynamic_scale = fix_min(
                dynamic_scale,
                get_scalefactor(&low_band_imag[..ov_end]) + ov_low_band_shift,
            );
            dynamic_scale = fix_min(
                dynamic_scale,
                get_scalefactor(&low_band_imag[ov_end..n_end]) + low_band_shift,
            );
        }

        if dynamic_scale == 0 {
            // In this special case the available headroom bits as well as ovLowBandShift and
            // lowBandShift are zero. The spectrum is limited to prevent -1.0, so negative values
            // for dynamicScale can be avoided.
            for v in low_band_real[..n_end].iter_mut() {
                *v = fix_max(*v, 0x80000001u32 as i32);
            }
            if !use_lp {
                for v in low_band_imag[..n_end].iter_mut() {
                    *v = fix_max(*v, 0x80000001u32 as i32);
                }
            }
        } else {
            // one additional bit headroom to prevent -1.0
            dynamic_scale = fix_max(0, dynamic_scale - 1);
        }

        // Scale temporal QMF buffer.
        scale_values(&mut low_band_real[..ov_end], dynamic_scale - ov_low_band_shift);
        scale_values(&mut low_band_real[ov_end..n_end], dynamic_scale - low_band_shift);

        if !use_lp {
            scale_values(&mut low_band_imag[..ov_end], dynamic_scale - ov_low_band_shift);
            scale_values(&mut low_band_imag[ov_end..n_end], dynamic_scale - low_band_shift);
        }

        if !use_lp {
            ac_det_scale +=
                auto_corr2nd_cplx(&mut ac, &low_band_real, &low_band_imag, auto_corr_length);
        } else {
            ac_det_scale += auto_corr2nd_real(&mut ac, &low_band_real, auto_corr_length);
        }

        // Examine dynamic of determinant in autocorrelation.
        ac_det_scale += 2 * (com_low_band_scale + dynamic_scale);
        ac_det_scale *= 2; // two times reflection coefficent scaling
        ac_det_scale += ac.det_scale; // ac scaling of determinant

        // In case of determinant < 10^-38, resetLPCCoeffs=1 has to be enforced.
        if ac_det_scale > 126 {
            reset_lpc_coeffs = true;
        }

        alphar[1] = 0;
        if !use_lp {
            alphai[1] = 0;
        }

        if ac.det != 0 {
            let abs_det = fixp_abs(ac.det);

            let tmp = if !use_lp {
                (f_mult_div2(ac.r01r, ac.r12r) >> (LPC_SCALE_FACTOR - 1)).wrapping_sub(
                    f_mult_div2(ac.r01i, ac.r12i).wrapping_add(f_mult_div2(ac.r02r, ac.r11r))
                        >> (LPC_SCALE_FACTOR - 1),
                )
            } else {
                (f_mult_div2(ac.r01r, ac.r12r) >> (LPC_SCALE_FACTOR - 1))
                    .wrapping_sub(f_mult_div2(ac.r02r, ac.r11r) >> (LPC_SCALE_FACTOR - 1))
            };
            let abs_tmp = fixp_abs(tmp);

            // Quick check: is first filter coeff >= 1(4)
            {
                let mut scale = 0i32;
                let result = f_div_norm_fast(abs_tmp, abs_det, &mut scale);
                scale += ac.det_scale;

                if scale > 0 && result >= (MAXVAL_DBL >> scale) {
                    reset_lpc_coeffs = true;
                } else {
                    alphar[1] = fx_dbl2fx_sgl(scale_value_saturate(result, scale));
                    if (tmp < 0) ^ (ac.det < 0) {
                        alphar[1] = alphar[1].wrapping_neg();
                    }
                }
            }

            if !use_lp {
                let tmp = (f_mult_div2(ac.r01i, ac.r12r) >> (LPC_SCALE_FACTOR - 1)).wrapping_add(
                    f_mult_div2(ac.r01r, ac.r12i).wrapping_sub(f_mult_div2(ac.r02i, ac.r11r))
                        >> (LPC_SCALE_FACTOR - 1),
                );

                let abs_tmp = fixp_abs(tmp);

                // Quick check: is second filter coeff >= 1(4)
                let mut scale = 0i32;
                let result = f_div_norm_fast(abs_tmp, abs_det, &mut scale);
                scale += ac.det_scale;

                if scale > 0 && result >= (MAXVAL_DBL >> scale) {
                    reset_lpc_coeffs = true;
                } else {
                    alphai[1] = fx_dbl2fx_sgl(scale_value_saturate(result, scale));
                    if (tmp < 0) ^ (ac.det < 0) {
                        alphai[1] = alphai[1].wrapping_neg();
                    }
                }
            }
        }

        alphar[0] = 0;
        if !use_lp {
            alphai[0] = 0;
        }

        if ac.r11r != 0 {
            // ac.r11r is always >=0
            let tmp = if !use_lp {
                (ac.r01r >> (LPC_SCALE_FACTOR + 1)).wrapping_add(
                    f_mult_div2(alphar[1], ac.r12r).wrapping_add(f_mult_div2(alphai[1], ac.r12i)),
                )
            } else if ac.r01r >= 0 {
                (ac.r01r >> (LPC_SCALE_FACTOR + 1)).wrapping_add(f_mult_div2(alphar[1], ac.r12r))
            } else {
                // -((-ac.r01r) >> 3): ac.r01r == MINVAL_DBL negates to itself (wraps).
                (ac.r01r.wrapping_neg() >> (LPC_SCALE_FACTOR + 1))
                    .wrapping_neg()
                    .wrapping_add(f_mult_div2(alphar[1], ac.r12r))
            };

            let abs_tmp = fixp_abs(tmp);

            // Quick check: is first filter coeff >= 1(4)
            if abs_tmp >= (ac.r11r >> 1) {
                reset_lpc_coeffs = true;
            } else {
                let mut scale = 0i32;
                let result = f_div_norm_fast(abs_tmp, fixp_abs(ac.r11r), &mut scale);
                alphar[0] = fx_dbl2fx_sgl(scale_value_saturate(result, scale + 1));

                if (tmp > 0) ^ (ac.r11r < 0) {
                    alphar[0] = alphar[0].wrapping_neg();
                }
            }

            if !use_lp {
                let tmp = (ac.r01i >> (LPC_SCALE_FACTOR + 1)).wrapping_add(
                    f_mult_div2(alphai[1], ac.r12r).wrapping_sub(f_mult_div2(alphar[1], ac.r12i)),
                );

                let abs_tmp = fixp_abs(tmp);

                // Quick check: is second filter coeff >= 1(4)
                if abs_tmp >= (ac.r11r >> 1) {
                    reset_lpc_coeffs = true;
                } else {
                    let mut scale = 0i32;
                    let result = f_div_norm_fast(abs_tmp, fixp_abs(ac.r11r), &mut scale);
                    alphai[0] = fx_dbl2fx_sgl(scale_value_saturate(result, scale + 1));
                    if (tmp > 0) ^ (ac.r11r < 0) {
                        alphai[0] = alphai[0].wrapping_neg();
                    }
                }
            }
        }

        if !use_lp {
            // Now check the quadratic criteria
            if f_mult_div2(alphar[0], alphar[0]).wrapping_add(f_mult_div2(alphai[0], alphai[0]))
                >= fl2fxconst_dbl(0.5f32 as f64)
            {
                reset_lpc_coeffs = true;
            }
            if f_mult_div2(alphar[1], alphar[1]).wrapping_add(f_mult_div2(alphai[1], alphai[1]))
                >= fl2fxconst_dbl(0.5f32 as f64)
            {
                reset_lpc_coeffs = true;
            }
        }

        if reset_lpc_coeffs {
            alphar[0] = 0;
            alphar[1] = 0;
            if !use_lp {
                alphai[0] = 0;
                alphai[1] = 0;
            }
        }

        if use_lp {
            // Aliasing detection
            if ac.r11r == 0 {
                k1 = 0;
            } else if fixp_abs(ac.r01r) >= fixp_abs(ac.r11r) {
                if f_mult_div2(ac.r01r, ac.r11r) < 0 {
                    k1 = MAXVAL_DBL;
                } else {
                    // Since this value is squared later, it must not ever become -1.0f.
                    k1 = MINVAL_DBL + 1;
                }
            } else {
                let mut scale = 0i32;
                let result = f_div_norm_fast(fixp_abs(ac.r01r), fixp_abs(ac.r11r), &mut scale);
                k1 = scale_value_saturate(result, scale);

                if !((ac.r01r < 0) ^ (ac.r11r < 0)) {
                    k1 = k1.wrapping_neg();
                }
            }
            if lo_band > 1 && lo_band < v_k_master0 {
                // Check if the gain should be locked
                let deg = MAXVAL_DBL - f_pow2(k1_below);
                let lb = lo_band as usize;
                degree_alias[lb] = 0;
                if (lo_band & 1) == 0 && k1 < 0 {
                    if k1_below < 0 {
                        // 2-Ch Aliasing Detection
                        degree_alias[lb] = MAXVAL_DBL;
                        if k1_below2 > 0 {
                            // 3-Ch Aliasing Detection
                            degree_alias[lb - 1] = deg;
                        }
                    } else if k1_below2 > 0 {
                        // 3-Ch Aliasing Detection
                        degree_alias[lb] = deg;
                    }
                }
                if (lo_band & 1) == 1 && k1 > 0 {
                    if k1_below > 0 {
                        // 2-CH Aliasing Detection
                        degree_alias[lb] = MAXVAL_DBL;
                        if k1_below2 < 0 {
                            // 3-CH Aliasing Detection
                            degree_alias[lb - 1] = deg;
                        }
                    } else if k1_below2 < 0 {
                        // 3-CH Aliasing Detection
                        degree_alias[lb] = deg;
                    }
                }
            }
            // remember k1 values of the 2 QMF channels below the current channel
            k1_below2 = k1_below;
            k1_below = k1;
        }

        let mut patch = 0usize;

        while patch < p_settings.no_of_patches as usize {
            // inner loop over every patch
            let hi_band = lo_band + patch_param[patch].target_band_offs as i32;

            if lo_band < patch_param[patch].source_start_band as i32
                || lo_band >= patch_param[patch].source_stop_band as i32
            {
                // Lowband not in current patch - proceed
                patch += 1;
                continue;
            }

            fdk_assert!(hi_band < 64);
            // Bounded: the port skips a high band of 64 or more.
            let in_range = hi_band < 64;

            // bwIndex[patch] is already initialized with value from previous band inside this
            // patch
            while hi_band >= p_settings.bw_borders[bw_index[patch] as usize] as i32
                && bw_index[patch] < MAX_NUM_PATCHES as i32 - 1
            {
                bw_index[patch] += 1;
            }

            // Filter Step 2: add the left slope with the current filter to the buffer pure
            // source values are already in there
            let mut bw = fx_dbl2fx_sgl(bw_vector[bw_index[patch] as usize]);

            // Apply current bandwidth expansion factor
            let a0r = fx_dbl2fx_sgl(f_mult(bw, alphar[0]));

            if !use_lp {
                a0i = fx_dbl2fx_sgl(f_mult(bw, alphai[0]));
            }
            bw = fx_dbl2fx_sgl(f_pow2(bw));
            let a1r = fx_dbl2fx_sgl(f_mult(bw, alphar[1]));
            if !use_lp {
                a1i = fx_dbl2fx_sgl(f_mult(bw, alphai[1]));
            }

            // Filter Step 3: insert the middle part which won't be windowed
            let lb = lo_band as usize;
            let hb = hi_band as usize;
            if !in_range {
            } else if bw <= 0 {
                if !use_lp {
                    let descale = fix_min(DFRACT_BITS - 1, LPC_SCALE_FACTOR + dynamic_scale);
                    for i in start_sample..stop_sample {
                        let iu = i as usize;
                        let mut accu1 = low_band_real[LPC_ORDER + iu] >> descale;
                        let mut accu2 = low_band_imag[LPC_ORDER + iu] >> descale;
                        if f_pre_whitening != 0 {
                            accu1 = scale_value_saturate(
                                f_mult_div2(accu1, pre_whitening_gains[lb]),
                                pre_whitening_gains_exp[lb] + 1,
                            );
                            accu2 = scale_value_saturate(
                                f_mult_div2(accu2, pre_whitening_gains[lb]),
                                pre_whitening_gains_exp[lb] + 1,
                            );
                        }
                        qmf_buffer_real[iu][hb] = accu1;
                        qmf_buffer_imag[iu][hb] = accu2;
                    }
                } else {
                    let descale = fix_min(DFRACT_BITS - 1, LPC_SCALE_FACTOR + dynamic_scale);
                    for i in start_sample..stop_sample {
                        let iu = i as usize;
                        qmf_buffer_real[iu][hb] = low_band_real[LPC_ORDER + iu] >> descale;
                    }
                }
            } else if !use_lp {
                let pwg = if f_pre_whitening != 0 {
                    Some((pre_whitening_gains[lb], pre_whitening_gains_exp[lb]))
                } else {
                    None
                };
                lpp_filter_cplx(
                    qmf_buffer_real,
                    qmf_buffer_imag,
                    &low_band_real,
                    &low_band_imag,
                    start_sample,
                    stop_sample,
                    hb,
                    [a0r, a0i, a1r, a1i],
                    dynamic_scale,
                    pwg,
                );
            } else {
                fdk_assert!(dynamic_scale >= 0);
                calc_qmf_buffer_real(
                    qmf_buffer_real,
                    &low_band_real[(LPC_ORDER as i32 + start_sample - 2) as usize..],
                    start_sample,
                    stop_sample,
                    hi_band as u8,
                    dynamic_scale,
                    a0r,
                    a1r,
                );
            }

            patch += 1;
        } // inner loop over patches
    } // outer loop over bands (loBand)

    if use_lp {
        for lo_band in p_settings.lb_start_patching as i32..p_settings.lb_stop_patching as i32 {
            let mut patch = 0usize;
            while patch < p_settings.no_of_patches as usize {
                // UCHAR hiBand = loBand + targetBandOffs
                let hi_band = (lo_band + patch_param[patch].target_band_offs as i32) as u8;

                if lo_band < patch_param[patch].source_start_band as i32
                    || lo_band >= patch_param[patch].source_stop_band as i32
                    || hi_band >= 64
                // Highband out of range (biterror)
                {
                    // Lowband not in current patch or highband out of range (might be caused by
                    // biterrors)- proceed
                    patch += 1;
                    continue;
                }

                if hi_band != patch_param[patch].target_start_band {
                    // (see sbr_dec() for the values of degreeAlias[] the port reads)
                    degree_alias[hi_band as usize] = degree_alias[lo_band as usize];
                }

                patch += 1;
            }
        }
    }

    let n = (n_invf_bands.max(0) as usize).min(MAX_NUM_PATCHES);
    h_lpp_trans.bw_vector_old[..n].copy_from_slice(&bw_vector[..n]);

    // set high band scale factor
    sbr_scale_factor.hb_scale = com_low_band_scale - LPC_SCALE_FACTOR;
}

/// `lppTransposerHBE()`: the inverse filtering of the HBE patch (harmonic transposer) in place.
#[allow(clippy::too_many_arguments)]
pub fn lpp_transposer_hbe(
    h_lpp_trans: &mut SbrLppTrans,
    p_settings: &TransposerSettings,
    h_qmf_transposer: &HbeTransposer,
    sbr_scale_factor: &mut QmfScaleFactor,
    qmf_buffer_real: &mut [&mut [FixpDbl]],
    qmf_buffer_imag: &mut [&mut [FixpDbl]],
    time_step: i32,
    first_slot_offs: i32,
    last_slot_offs: i32,
    n_invf_bands: i32,
    sbr_invf_mode: &[InvfMode],
    sbr_invf_mode_prev: &[InvfMode],
) {
    let mut bw_vector = [0 as FixpDbl; MAX_NUM_PATCHES];

    let patch_param = &p_settings.patch_param;

    let mut alphar: [FixpSgl; LPC_ORDER];
    let mut alphai: [FixpSgl; LPC_ORDER];

    let mut ac = AcorrCoefs::default();

    let start_sample = first_slot_offs * time_step;
    let stop_sample = p_settings.n_cols as i32 + last_slot_offs * time_step;

    inverse_filtering_level_emphasis(
        h_lpp_trans,
        &p_settings.wh_factors,
        n_invf_bands as u8,
        sbr_invf_mode,
        sbr_invf_mode_prev,
        &mut bw_vector,
    );

    let stop_sample_clear = stop_sample;

    let auto_corr_length = p_settings.n_cols as i32 + p_settings.overlap as i32;

    if p_settings.no_of_patches > 0 {
        // Set upper subbands to zero (see lppTransposer()).
        let last = &patch_param[p_settings.no_of_patches as usize - 1];
        let target_stop_band = last.target_start_band as usize + last.num_bands_in_patch as usize;

        for i in start_sample.max(0)..stop_sample_clear {
            if let Some(s) = qmf_buffer_real[i as usize].get_mut(target_stop_band..64) {
                s.fill(0);
            }
            if let Some(s) = qmf_buffer_imag[i as usize].get_mut(target_stop_band..64) {
                s.fill(0);
            }
        }
    }

    // Calc common low band scale factor
    let com_band_scale = sbr_scale_factor.hb_scale;

    let ov_low_band_shift = sbr_scale_factor.hb_scale - com_band_scale;
    let low_band_shift = sbr_scale_factor.hb_scale - com_band_scale;

    // outer loop over bands to do analysis only once for each band
    let start = h_qmf_transposer.start_band;
    let stop = h_qmf_transposer.stop_band;

    let n_cols = p_settings.n_cols as i32;
    let overlap = p_settings.overlap as i32;
    let ord = LPC_ORDER as i32;
    for lo_band in start..stop {
        let lb = lo_band as usize;
        let mut bw_index = 0usize;

        let mut low_band_real = [0 as FixpDbl; MAX_COLS + MAX_OV_COLS + LPC_ORDER];
        let mut low_band_imag = [0 as FixpDbl; MAX_COLS + MAX_OV_COLS + LPC_ORDER];

        let mut reset_lpc_coeffs = false;
        let mut dynamic_scale = DFRACT_BITS - 1 - LPC_SCALE_FACTOR;
        let mut ac_det_scale = 0i32; // scaling of autocorrelation determinant

        let mut i = 0usize;
        while i < LPC_ORDER {
            low_band_real[i] = h_lpp_trans.at(LPC_HBE_RE + i * LPC_HBE_COLS + lb);
            low_band_imag[i] = h_lpp_trans.at(LPC_HBE_IM + i * LPC_HBE_COLS + lb);
            i += 1;
        }

        while (i as i32) < ord + first_slot_offs * time_step {
            low_band_real[i] = h_lpp_trans.at(LPC_HBE_RE + i * LPC_HBE_COLS + lb);
            low_band_imag[i] = h_lpp_trans.at(LPC_HBE_IM + i * LPC_HBE_COLS + lb);
            i += 1;
        }

        // Take old slope length qmf slot source values out of (overlap)qmf buffer
        for i in first_slot_offs * time_step..n_cols + overlap {
            let iu = i as usize;
            low_band_real[iu + LPC_ORDER] = qmf_buffer_real[iu][lb];
            low_band_imag[iu + LPC_ORDER] = qmf_buffer_imag[iu][lb];
        }

        // store unmodified values to buffer
        for i in 0..(ord + overlap) as usize {
            let src = (n_cols - ord) as usize + i;
            let re = qmf_buffer_real[src][lb];
            let im = qmf_buffer_imag[src][lb];
            h_lpp_trans.set(LPC_HBE_RE + i * LPC_HBE_COLS + lb, re);
            h_lpp_trans.set(LPC_HBE_IM + i * LPC_HBE_COLS + lb, im);
        }

        let ov_end = (ord + overlap) as usize;
        let n_end = ov_end + n_cols as usize;

        // Determine dynamic scaling value.
        dynamic_scale =
            fix_min(dynamic_scale, get_scalefactor(&low_band_real[..ov_end]) + ov_low_band_shift);
        dynamic_scale =
            fix_min(dynamic_scale, get_scalefactor(&low_band_real[ov_end..n_end]) + low_band_shift);
        dynamic_scale =
            fix_min(dynamic_scale, get_scalefactor(&low_band_imag[..ov_end]) + ov_low_band_shift);
        dynamic_scale =
            fix_min(dynamic_scale, get_scalefactor(&low_band_imag[ov_end..n_end]) + low_band_shift);

        // one additional bit headroom to prevent -1.0
        dynamic_scale -= 1;

        // Scale temporal QMF buffer.
        scale_values(&mut low_band_real[..ov_end], dynamic_scale - ov_low_band_shift);
        scale_values(&mut low_band_real[ov_end..n_end], dynamic_scale - low_band_shift);
        scale_values(&mut low_band_imag[..ov_end], dynamic_scale - ov_low_band_shift);
        scale_values(&mut low_band_imag[ov_end..n_end], dynamic_scale - low_band_shift);

        ac_det_scale +=
            auto_corr2nd_cplx(&mut ac, &low_band_real, &low_band_imag, auto_corr_length);

        // Examine dynamic of determinant in autocorrelation.
        ac_det_scale += 2 * (com_band_scale + dynamic_scale);
        ac_det_scale *= 2; // two times reflection coefficent scaling
        ac_det_scale += ac.det_scale; // ac scaling of determinant

        // In case of determinant < 10^-38, resetLPCCoeffs=1 has to be enforced.
        if ac_det_scale > 126 {
            reset_lpc_coeffs = true;
        }

        alphar = [0; LPC_ORDER];
        alphai = [0; LPC_ORDER];

        if ac.det != 0 {
            let abs_det = fixp_abs(ac.det);

            let tmp = (f_mult_div2(ac.r01r, ac.r12r) >> (LPC_SCALE_FACTOR - 1)).wrapping_sub(
                f_mult_div2(ac.r01i, ac.r12i).wrapping_add(f_mult_div2(ac.r02r, ac.r11r))
                    >> (LPC_SCALE_FACTOR - 1),
            );
            let abs_tmp = fixp_abs(tmp);

            // Quick check: is first filter coeff >= 1(4)
            {
                let mut scale = 0i32;
                let result = f_div_norm_fast(abs_tmp, abs_det, &mut scale);
                scale += ac.det_scale;

                if scale > 0 && result >= (MAXVAL_DBL >> scale) {
                    reset_lpc_coeffs = true;
                } else {
                    alphar[1] = fx_dbl2fx_sgl(scale_value_saturate(result, scale));
                    if (tmp < 0) ^ (ac.det < 0) {
                        alphar[1] = alphar[1].wrapping_neg();
                    }
                }
            }

            let tmp = (f_mult_div2(ac.r01i, ac.r12r) >> (LPC_SCALE_FACTOR - 1)).wrapping_add(
                f_mult_div2(ac.r01r, ac.r12i).wrapping_sub(f_mult_div2(ac.r02i, ac.r11r))
                    >> (LPC_SCALE_FACTOR - 1),
            );

            let abs_tmp = fixp_abs(tmp);

            // Quick check: is second filter coeff >= 1(4)
            {
                let mut scale = 0i32;
                let result = f_div_norm_fast(abs_tmp, abs_det, &mut scale);
                scale += ac.det_scale;

                if scale > 0 && result >= (MAXVAL_DBL >> scale) {
                    reset_lpc_coeffs = true;
                } else {
                    alphai[1] = fx_dbl2fx_sgl(scale_value_saturate(result, scale));
                    if (tmp < 0) ^ (ac.det < 0) {
                        alphai[1] = alphai[1].wrapping_neg();
                    }
                }
            }
        }

        alphar[0] = 0;
        alphai[0] = 0;

        if ac.r11r != 0 {
            // ac.r11r is always >=0
            let tmp = (ac.r01r >> (LPC_SCALE_FACTOR + 1)).wrapping_add(
                f_mult_div2(alphar[1], ac.r12r).wrapping_add(f_mult_div2(alphai[1], ac.r12i)),
            );

            let abs_tmp = fixp_abs(tmp);

            // Quick check: is first filter coeff >= 1(4)
            if abs_tmp >= (ac.r11r >> 1) {
                reset_lpc_coeffs = true;
            } else {
                let mut scale = 0i32;
                let result = f_div_norm_fast(abs_tmp, fixp_abs(ac.r11r), &mut scale);
                alphar[0] = fx_dbl2fx_sgl(scale_value_saturate(result, scale + 1));

                if (tmp > 0) ^ (ac.r11r < 0) {
                    alphar[0] = alphar[0].wrapping_neg();
                }
            }

            let tmp = (ac.r01i >> (LPC_SCALE_FACTOR + 1)).wrapping_add(
                f_mult_div2(alphai[1], ac.r12r).wrapping_sub(f_mult_div2(alphar[1], ac.r12i)),
            );

            let abs_tmp = fixp_abs(tmp);

            // Quick check: is second filter coeff >= 1(4)
            if abs_tmp >= (ac.r11r >> 1) {
                reset_lpc_coeffs = true;
            } else {
                let mut scale = 0i32;
                let result = f_div_norm_fast(abs_tmp, fixp_abs(ac.r11r), &mut scale);
                alphai[0] = fx_dbl2fx_sgl(scale_value_saturate(result, scale + 1));
                if (tmp > 0) ^ (ac.r11r < 0) {
                    alphai[0] = alphai[0].wrapping_neg();
                }
            }
        }

        // Now check the quadratic criteria
        if f_mult_div2(alphar[0], alphar[0]).wrapping_add(f_mult_div2(alphai[0], alphai[0]))
            >= fl2fxconst_dbl(0.5f32 as f64)
        {
            reset_lpc_coeffs = true;
        }
        if f_mult_div2(alphar[1], alphar[1]).wrapping_add(f_mult_div2(alphai[1], alphai[1]))
            >= fl2fxconst_dbl(0.5f32 as f64)
        {
            reset_lpc_coeffs = true;
        }

        if reset_lpc_coeffs {
            alphar = [0; LPC_ORDER];
            alphai = [0; LPC_ORDER];
        }

        while bw_index < MAX_NUM_PATCHES - 1 && lo_band >= p_settings.bw_borders[bw_index] as i32 {
            bw_index += 1;
        }

        // Filter Step 2: add the left slope with the current filter to the buffer pure source
        // values are already in there
        let mut bw = fx_dbl2fx_sgl(bw_vector[bw_index]);

        // Apply current bandwidth expansion factor
        let a0r = fx_dbl2fx_sgl(f_mult(bw, alphar[0]));
        let a0i = fx_dbl2fx_sgl(f_mult(bw, alphai[0]));
        bw = fx_dbl2fx_sgl(f_pow2(bw));
        let a1r = fx_dbl2fx_sgl(f_mult(bw, alphar[1]));
        let a1i = fx_dbl2fx_sgl(f_mult(bw, alphai[1]));

        // Filter Step 3: insert the middle part which won't be windowed
        if bw <= 0 {
            let descale = fix_min(DFRACT_BITS - 1, LPC_SCALE_FACTOR + dynamic_scale);
            for i in start_sample..stop_sample {
                let iu = i as usize;
                // dynamicScale can be -1 here (descale 1).
                qmf_buffer_real[iu][lb] = low_band_real[LPC_ORDER + iu] >> descale;
                qmf_buffer_imag[iu][lb] = low_band_imag[LPC_ORDER + iu] >> descale;
            }
        } else {
            let descale = fix_min(DFRACT_BITS - 1, LPC_SCALE_FACTOR + dynamic_scale);
            // prevent negativ scale factor due to 'one additional bit headroom'
            dynamic_scale += 1;

            for i in start_sample..stop_sample {
                let iu = i as usize;
                let r1 = low_band_real[LPC_ORDER + iu - 1];
                let i1 = low_band_imag[LPC_ORDER + iu - 1];
                let r2 = low_band_real[LPC_ORDER + iu - 2];
                let i2 = low_band_imag[LPC_ORDER + iu - 2];
                // `(...) >> dynamicScale`: dynamicScale is in 0..=29 here.
                let accu1 = f_mult_div2(a0r, r1)
                    .wrapping_sub(f_mult_div2(a0i, i1))
                    .wrapping_add(f_mult_div2(a1r, r2))
                    .wrapping_sub(f_mult_div2(a1i, i2))
                    .wrapping_shr(dynamic_scale as u32);
                let accu2 = f_mult_div2(a0i, r1)
                    .wrapping_add(f_mult_div2(a0r, i1))
                    .wrapping_add(f_mult_div2(a1i, r2))
                    .wrapping_add(f_mult_div2(a1r, i2))
                    .wrapping_shr(dynamic_scale as u32);

                qmf_buffer_real[iu][lb] =
                    (low_band_real[LPC_ORDER + iu] >> descale).wrapping_add(accu1 << (1 + 1));
                qmf_buffer_imag[iu][lb] =
                    (low_band_imag[LPC_ORDER + iu] >> descale).wrapping_add(accu2 << (1 + 1));
            }
        }
    } // outer loop over bands (loBand)

    let n = (n_invf_bands.max(0) as usize).min(MAX_NUM_PATCHES);
    h_lpp_trans.bw_vector_old[..n].copy_from_slice(&bw_vector[..n]);

    // set high band scale factor
    sbr_scale_factor.hb_scale = com_band_scale - LPC_SCALE_FACTOR;
}

/// `createLppTransposer()`: initializes one low power transposer instance. The common settings
/// (`pSettings`) are only initialized for channel 0.
#[allow(clippy::too_many_arguments)]
pub fn create_lpp_transposer(
    _hs: &mut SbrLppTrans,
    p_settings: &mut TransposerSettings,
    high_band_start_sb: i32,
    v_k_master: &[u8],
    num_master: i32,
    usb: i32,
    time_slots: i32,
    n_cols: i32,
    noise_band_table: &[u8],
    no_noise_bands: i32,
    fs: u32,
    chan: i32,
    overlap: i32,
) -> SbrError {
    // FB inverse filtering settings
    p_settings.n_cols = n_cols as u8;
    p_settings.overlap = overlap as u8;

    match time_slots {
        15 | 16 => {}
        _ => return SBRDEC_UNSUPPORTED_CONFIG, // Unimplemented
    }

    if chan == 0 {
        // Init common data only once
        p_settings.n_cols = n_cols as u8;

        return reset_lpp_transposer(
            p_settings,
            high_band_start_sb as u8,
            v_k_master,
            num_master as u8,
            noise_band_table,
            no_noise_bands as u8,
            usb as u8,
            fs,
        );
    }
    SBRDEC_OK
}

/// `findClosestEntry(goalSb, v_k_master, numMaster, direction)`
fn find_closest_entry(goal_sb: u8, v_k_master: &[u8], num_master: u8, direction: u8) -> i32 {
    let num_master = num_master as usize;
    // The master table has num_master + 1 valid entries; indices past the table read 0 in the
    // port.
    let at = |i: usize| v_k_master.get(i).copied().unwrap_or(0);

    if goal_sb <= at(0) {
        return at(0) as i32;
    }

    if goal_sb >= at(num_master) {
        return at(num_master) as i32;
    }

    let mut index: usize;
    if direction != 0 {
        index = 0;
        while at(index) < goal_sb {
            index += 1;
        }
    } else {
        index = num_master;
        while at(index) > goal_sb {
            index -= 1;
        }
    }

    at(index) as i32
}

/// `resetLppTransposer()`: resets the patching parameters and the whitening factors (the
/// settings shared by the channels of an element). Returns `SBRDEC_UNSUPPORTED_CONFIG` on error.
#[allow(clippy::too_many_arguments)]
pub fn reset_lpp_transposer(
    p_settings: &mut TransposerSettings,
    high_band_start_sb: u8,
    v_k_master: &[u8],
    num_master: u8,
    noise_band_table: &[u8],
    no_noise_bands: u8,
    usb: u8,
    fs: u32,
) -> SbrError {
    let at = |i: usize| v_k_master.get(i).copied().unwrap_or(0);

    // Start subband expressed in "non-critical" sampling terms
    let lsb = at(0) as i32;
    // Calculate distance in QMF bands between k0 and kx
    let xover_offset = high_band_start_sb as i32 - lsb;

    // Avoid endless loops (compare with float code).
    let usb = fix_min(usb as i32, at(num_master as usize) as i32) as u8;

    // Plausibility check
    if p_settings.n_cols == 64 {
        if lsb < 4 {
            // 4:1 SBR Requirement k0 >= 4 missed!
            return SBRDEC_UNSUPPORTED_CONFIG;
        }
    } else if lsb - SHIFT_START_SB < 4 {
        return SBRDEC_UNSUPPORTED_CONFIG;
    }

    // Initialize the patching parameter
    // ISO/IEC 14496-3 (Figure 4.48): goalSb = round( 2.048e6 / fs )
    // fs == 0: the result of the AArch64 division (udiv: 0) on every variant.
    let mut desired_border = ((((2048000u32 * 2).checked_div(fs).unwrap_or(0)) + 1) >> 1) as i32;

    // Adapt region to master-table
    desired_border = find_closest_entry(desired_border as u8, v_k_master, num_master, 1);

    // First patch
    let mut source_start_band = SHIFT_START_SB + xover_offset;
    let mut target_stop_band = lsb + xover_offset; // upperBand

    // Even (odd) numbered channel must be patched to even (odd) numbered channel
    let mut patch: i32 = 0;
    while target_stop_band < usb as i32 {
        // Too many patches? Allow MAX_NUM_PATCHES+1 patches here. we need to check later again,
        // since patch might be the highest patch AND contain less than 3 bands => actual number
        // of patches will be reduced by 1.
        if patch > MAX_NUM_PATCHES as i32 {
            return SBRDEC_UNSUPPORTED_CONFIG;
        }

        let pp = &mut p_settings.patch_param[patch as usize];
        pp.guard_start_band = target_stop_band as u8;
        pp.target_start_band = target_stop_band as u8;

        // Get the desired range of the patch
        let mut num_bands_in_patch = desired_border - target_stop_band;

        if num_bands_in_patch >= lsb - source_start_band {
            // Desired number bands are not available -> patch whole source range
            let mut patch_distance = target_stop_band - source_start_band; // Get the targetOffset
            patch_distance &= !1; // Rounding off odd numbers and make all even
                                  // Update number of bands to be patched
            num_bands_in_patch = lsb - (target_stop_band - patch_distance);
            // Adapt region to master-table
            num_bands_in_patch = find_closest_entry(
                (target_stop_band + num_bands_in_patch) as u8,
                v_k_master,
                num_master,
                0,
            ) - target_stop_band;
        }

        if p_settings.n_cols == 64 && num_bands_in_patch == 0 && source_start_band == SHIFT_START_SB
        {
            return SBRDEC_UNSUPPORTED_CONFIG;
        }

        // Desired number bands are available -> get the minimal even patching distance
        let mut patch_distance = num_bands_in_patch + target_stop_band - lsb; // minimal distance
        patch_distance = (patch_distance + 1) & !1; // Rounding up odd numbers and make all even

        if num_bands_in_patch > 0 {
            let pp = &mut p_settings.patch_param[patch as usize];
            pp.source_start_band = (target_stop_band - patch_distance) as u8;
            pp.target_band_offs = patch_distance as u8;
            pp.num_bands_in_patch = num_bands_in_patch as u8;
            pp.source_stop_band = pp.source_start_band.wrapping_add(num_bands_in_patch as u8);

            target_stop_band += pp.num_bands_in_patch as i32;
            patch += 1;
        }

        // All patches but first
        source_start_band = SHIFT_START_SB;

        // Check if we are close to desiredBorder
        if desired_border - target_stop_band < 3 {
            // MPEG doc
            desired_border = usb as i32;
        }
    }

    patch -= 1;

    // If highest patch contains less than three subband: skip it
    if patch > 0 && p_settings.patch_param[patch as usize].num_bands_in_patch < 3 {
        patch -= 1;
        let pp = &p_settings.patch_param[patch as usize];
        target_stop_band = pp.target_start_band as i32 + pp.num_bands_in_patch as i32;
    }

    // now check if we don't have one too many
    if patch >= MAX_NUM_PATCHES as i32 {
        return SBRDEC_UNSUPPORTED_CONFIG;
    }

    p_settings.no_of_patches = (patch + 1) as u8;

    // Check lowest and highest source subband
    p_settings.lb_start_patching = target_stop_band as u8;
    p_settings.lb_stop_patching = 0;
    for patch in 0..p_settings.no_of_patches as usize {
        p_settings.lb_start_patching =
            fix_min(p_settings.lb_start_patching, p_settings.patch_param[patch].source_start_band);
        p_settings.lb_stop_patching =
            fix_max(p_settings.lb_stop_patching, p_settings.patch_param[patch].source_stop_band);
    }

    let mut i = 0usize;
    while i < no_noise_bands as usize && i < MAX_NUM_NOISE_VALUES {
        p_settings.bw_borders[i] = noise_band_table.get(i + 1).copied().unwrap_or(0);
        i += 1;
    }
    while i < MAX_NUM_NOISE_VALUES {
        p_settings.bw_borders[i] = 255;
        i += 1;
    }

    // Choose whitening factors
    // Shift does a division by 2*(64)
    let start_freq_hz = (((lsb + xover_offset) as u32).wrapping_mul(fs) >> 7) as i32;

    let mut i = 1usize;
    while i < NUM_WHFACTOR_TABLE_ENTRIES {
        if start_freq_hz < FDK_SBR_DECODER_SBR_WH_FACTORS_INDEX[i] as i32 {
            break;
        }
        i += 1;
    }
    i -= 1;

    let t = &FDK_SBR_DECODER_SBR_WH_FACTORS_TABLE[i];
    p_settings.wh_factors.off = t[0];
    p_settings.wh_factors.transition_level = t[1];
    p_settings.wh_factors.low_level = t[2];
    p_settings.wh_factors.mid_level = t[3];
    p_settings.wh_factors.high_level = t[4];

    SBRDEC_OK
}
