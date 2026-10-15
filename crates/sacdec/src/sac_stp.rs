// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACdec/src/sac_stp.h, libSACdec/src/sac_stp.cpp.

//! Port of `sac_stp.h` and `sac_stp.cpp`: subband temporal processing (STP).
//!
//! `struct STP_DEC` is [`StpDec`]; `subbandTPCreate()` allocates it zeroed, the port keeps it as
//! a member of the spatial decoder. Its `BP` and `BP_GF` table pointers are flags (`NULL` until
//! `subbandTPInit()`).

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::scale::saturate_left_shift;

use crate::sac_bitdec::TREE_212;
use crate::sac_dec::*;
use crate::sac_dec_interface::*;
use crate::sac_rom::*;

/// `SF_FREQ_DOMAIN_HEADROOM`
const SF_FREQ_DOMAIN_HEADROOM: i32 = 2 * 1;

/// `BP_GF_START`
const BP_GF_START: usize = 6;
/// `BP_GF_SIZE`
const BP_GF_SIZE: usize = 25;
/// `HP_SIZE`
const HP_SIZE: usize = 9;
/// `STP_UPDATE_ENERGY_RATE`
const STP_UPDATE_ENERGY_RATE: i32 = 32;

/// `SF_WET`
const SF_WET: i32 = 5;
/// `SF_DRY`: SF_DRY == 2 would produce good conformance test results as well
const SF_DRY: i32 = 3;
/// `SF_DRY_NRG`: 8.495f = sum(BP_GF__FDK[i]) => energy calculation needs 4 bits headroom,
/// headroom can be reduced by 1 bit due to fPow2Div2() usage
const SF_DRY_NRG: i32 = 4 - 1;
/// `SF_WET_NRG`
const SF_WET_NRG: i32 = 4 - 1;
/// `SF_PRODUCT_BP_GF`
const SF_PRODUCT_BP_GF: i32 = 13;
/// `SF_PRODUCT_BP_GF_GF`
const SF_PRODUCT_BP_GF_GF: i32 = 26;
/// `SF_SCALE`
const SF_SCALE: i32 = 2;

/// `SF_SCALE_LD64`: `FL2FXCONST_DBL(0.03125)` (LD64((1<<SF_SCALE)))
pub const SF_SCALE_LD64: FixpDbl = fl2fxconst_dbl(0.03125);

/// `STP_LPF_COEFF1__FDK`: `FL2FXCONST_DBL(0.950f)`
pub const STP_LPF_COEFF1__FDK: FixpDbl = fl2fxconst_dbl(0.950f32 as f64);
/// `ONE_MINUS_STP_LPF_COEFF1__FDK`: `FL2FXCONST_DBL(0.05f)`
pub const ONE_MINUS_STP_LPF_COEFF1__FDK: FixpDbl = fl2fxconst_dbl(0.05f32 as f64);
/// `STP_LPF_COEFF2__FDK`: `FL2FXCONST_DBL(0.450f)`
pub const STP_LPF_COEFF2__FDK: FixpDbl = fl2fxconst_dbl(0.450f32 as f64);
/// `ONE_MINUS_STP_LPF_COEFF2__FDK`: `FL2FXCONST_DBL(1.0f - 0.450f)`
pub const ONE_MINUS_STP_LPF_COEFF2__FDK: FixpDbl = fl2fxconst_dbl((1.0f32 - 0.450f32) as f64);
/// `STP_SCALE_LIMIT__FDK`: `FL2FXCONST_DBL(2.82f / (float)(1 << SF_SCALE))`
pub const STP_SCALE_LIMIT__FDK: FixpDbl = fl2fxconst_dbl((2.82f32 / 4.0f32) as f64);
/// `ONE_DIV_STP_SCALE_LIMIT__FDK`: `FL2FXCONST_DBL(1.0f / 2.82f / (float)(1 << SF_SCALE))`
pub const ONE_DIV_STP_SCALE_LIMIT__FDK: FixpDbl =
    fl2fxconst_dbl((1.0f32 / 2.82f32 / 4.0f32) as f64);

/// `ABS_THR`: `(1e-9f * 32768 * 32768)` (a `float` product)
const ABS_THR: f32 = 1e-9f32 * 32768.0 * 32768.0;
/// `ABS_THR__FDK`: `FL2FXCONST_DBL(ABS_THR / ((float)(1 << (22 + 22 - 26))))`, scaled by 18 bits
pub const ABS_THR__FDK: FixpDbl = fl2fxconst_dbl((ABS_THR / 262144.0f32) as f64);
/// `ABS_THR2__FDK`: `FL2FXCONST_DBL(ABS_THR * 32.0f * 32.0f / ((float)(1 << (22 + 22 - 26))))`,
/// scaled by 10 bits
pub const ABS_THR2__FDK: FixpDbl =
    fl2fxconst_dbl((ABS_THR * 32.0f32 * 32.0f32 / 262144.0f32) as f64);

/// `STP_SCALE_LIMIT_HI`: `FL2FXCONST_DBL(3.02222222222 / (1 << SF_SCALE))` (a `double`)
pub const STP_SCALE_LIMIT_HI: FixpDbl = fl2fxconst_dbl(3.02222222222 / 4.0);
/// `STP_SCALE_LIMIT_LO`: `FL2FXCONST_DBL(0.28289992119 / (1 << SF_SCALE))`
pub const STP_SCALE_LIMIT_LO: FixpDbl = fl2fxconst_dbl(0.28289992119 / 4.0);
/// `STP_SCALE_LIMIT_HI_LD64`: `FL2FXCONST_DBL(0.04986280452)`
pub const STP_SCALE_LIMIT_HI_LD64: FixpDbl = fl2fxconst_dbl(0.04986280452);
/// `STP_SCALE_LIMIT_LO_LD64`: `FL2FXCONST_DBL(0.05692613500)`
pub const STP_SCALE_LIMIT_LO_LD64: FixpDbl = fl2fxconst_dbl(0.05692613500);

/// `FL2FXCONST_DBL(-0.484375f)`
const LD64_MIN: FixpDbl = fl2fxconst_dbl(-0.484375f32 as f64);

/// `struct STP_DEC`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StpDec {
    /// `runDryEner`
    pub run_dry_ener: [FixpDbl; MAX_INPUT_CHANNELS],
    /// `runWetEner`
    pub run_wet_ener: [FixpDbl; MAX_OUTPUT_CHANNELS],
    /// `oldDryEnerLD64`
    pub old_dry_ener_ld64: [FixpDbl; MAX_INPUT_CHANNELS],
    /// `oldWetEnerLD64`
    pub old_wet_ener_ld64: [FixpDbl; MAX_OUTPUT_CHANNELS],
    /// `prev_tp_scale`
    pub prev_tp_scale: [FixpDbl; MAX_OUTPUT_CHANNELS],
    /// `BP != NULL` (`BP__FDK`)
    pub bp: bool,
    /// `BP_GF != NULL` (`BP_GF__FDK`)
    pub bp_gf: bool,
    /// `update_old_ener`
    pub update_old_ener: i32,
}

/// `combineSignalCplx(hybOutputRealDry, hybOutputImagDry, hybOutputRealWet, hybOutputImagWet,
/// bands)`
fn combine_signal_cplx(
    hyb_output_real_dry: &mut [FixpDbl],
    hyb_output_imag_dry: &mut [FixpDbl],
    hyb_output_real_wet: &[FixpDbl],
    hyb_output_imag_wet: &[FixpDbl],
    bands: i32,
) {
    for n in 0..bands.max(0) as usize {
        hyb_output_real_dry[n] = f_add_saturate(hyb_output_real_dry[n], hyb_output_real_wet[n]);
        hyb_output_imag_dry[n] = f_add_saturate(hyb_output_imag_dry[n], hyb_output_imag_wet[n]);
    }
}

/// `combineSignalCplxScale1(hybOutputRealDry, hybOutputImagDry, hybOutputRealWet,
/// hybOutputImagWet, pBP, scaleX, bands)`
fn combine_signal_cplx_scale1(
    hyb_output_real_dry: &mut [FixpDbl],
    hyb_output_imag_dry: &mut [FixpDbl],
    hyb_output_real_wet: &[FixpDbl],
    hyb_output_imag_wet: &[FixpDbl],
    p_bp: &[FixpCfg],
    scale_x: FixpDbl,
    bands: i32,
) {
    for n in 0..bands.max(0) as usize {
        let scale_y = f_mult(scale_x, p_bp[n]);
        hyb_output_real_dry[n] = saturate_left_shift(
            (hyb_output_real_dry[n] >> SF_SCALE)
                .wrapping_add(f_mult(hyb_output_real_wet[n], scale_y)),
            SF_SCALE,
            DFRACT_BITS,
        );
        hyb_output_imag_dry[n] = saturate_left_shift(
            (hyb_output_imag_dry[n] >> SF_SCALE)
                .wrapping_add(f_mult(hyb_output_imag_wet[n], scale_y)),
            SF_SCALE,
            DFRACT_BITS,
        );
    }
}

/// `combineSignalCplxScale2(hybOutputRealDry, hybOutputImagDry, hybOutputRealWet,
/// hybOutputImagWet, scaleX, bands)`
fn combine_signal_cplx_scale2(
    hyb_output_real_dry: &mut [FixpDbl],
    hyb_output_imag_dry: &mut [FixpDbl],
    hyb_output_real_wet: &[FixpDbl],
    hyb_output_imag_wet: &[FixpDbl],
    scale_x: FixpDbl,
    bands: i32,
) {
    for n in 0..bands.max(0) as usize {
        hyb_output_real_dry[n] = saturate_left_shift(
            (hyb_output_real_dry[n] >> SF_SCALE)
                .wrapping_add(f_mult(hyb_output_real_wet[n], scale_x)),
            SF_SCALE,
            DFRACT_BITS,
        );
        hyb_output_imag_dry[n] = saturate_left_shift(
            (hyb_output_imag_dry[n] >> SF_SCALE)
                .wrapping_add(f_mult(hyb_output_imag_wet[n], scale_x)),
            SF_SCALE,
            DFRACT_BITS,
        );
    }
}

/// `subbandTPCreate(hStpDec)`: the structure is a zero-initialized member in the port.
pub fn subband_tp_create(h_stp_dec: &mut StpDec) -> SacdecError {
    *h_stp_dec = StpDec::default();
    MPS_OK
}

/// `subbandTPInit(self)`
pub fn subband_tp_init(self_: &mut StpDec) -> SacdecError {
    let err = MPS_OK;

    for ch in 0..MAX_OUTPUT_CHANNELS {
        self_.prev_tp_scale[ch] = fl2fxconst_dbl((1.0f32 / (1 << SF_SCALE) as f32) as f64);
        self_.old_wet_ener_ld64[ch] = 0;
    }
    for ch in 0..MAX_INPUT_CHANNELS {
        self_.old_dry_ener_ld64[ch] = 0;
    }

    self_.bp = true;
    self_.bp_gf = true;

    self_.update_old_ener = 0;

    err
}

/// `subbandTPDestroy(hStpDec)`: nothing to free in the port.
pub fn subband_tp_destroy(_h_stp_dec: &mut StpDec) {}

/// The `CALC_WET_SCALE(dryIdx, wetIdx)` macro.
#[inline]
fn calc_wet_scale(dry_ener_ld64: FixpDbl, wet_ener_ld64: FixpDbl) -> FixpDbl {
    if dry_ener_ld64.wrapping_sub(STP_SCALE_LIMIT_HI_LD64) > wet_ener_ld64 {
        STP_SCALE_LIMIT_HI
    } else if dry_ener_ld64 < wet_ener_ld64.wrapping_sub(STP_SCALE_LIMIT_LO_LD64) {
        STP_SCALE_LIMIT_LO
    } else {
        let tmp = ((dry_ener_ld64.wrapping_sub(wet_ener_ld64)) >> 1).wrapping_sub(SF_SCALE_LD64);
        calc_inv_ld_data(tmp)
    }
}

/// `subbandTPApply(self, frame)`
pub fn subband_tp_apply(self_: &mut SpatialDec, frame: &SpatialBsFrame) -> SacdecError {
    let mut dry_ener = [0 as FixpDbl; MAX_INPUT_CHANNELS];
    let mut scale = [0 as FixpDbl; MAX_OUTPUT_CHANNELS];
    let mut dry_ener_ld64 = [0 as FixpDbl; MAX_INPUT_CHANNELS];
    let mut wet_ener_ld64 = [0 as FixpDbl; MAX_OUTPUT_CHANNELS];
    let mut dry_ener0: FixpDbl = 0;
    let skip_channels = [0i32; MAX_OUTPUT_CHANNELS];

    let nrg_scale = 2 * self_.clip_protect_gain_sf__fdk as i32;

    // set scalefactor and loop counter
    fdk_assert!(SF_DRY >= 1);
    let cplx_bands = BP_GF_SIZE;
    let cplx_hyb_bands = self_.hybrid_bands;
    let dry_scale_dmx = if self_.tree_config == TREE_212 {
        2 // 2 bits to compensate fMultDiv2() and fPow2Div2() used in energy calculation
    } else {
        (2 * SF_DRY) - 2
    };
    let wet_scale_dmx = 2;

    // The pointers qmfOutput{Real,Imag}{Dry,Wet}[ch] = &hybOutput...[ch][7] are offsets of 7.
    const OFF: usize = 7;
    let n_out = (self_.num_output_channels.max(0) as usize).min(MAX_OUTPUT_CHANNELS);
    let n_in = (self_.num_input_channels.max(0) as usize).min(MAX_INPUT_CHANNELS);

    // clear skipping flag for all output channels (skipChannels), set scale values to zero
    // (scale): both arrays are local zeros.

    let h = &mut self_.h_stp_dec;
    // update normalisation energy with latest smoothed energy
    if h.update_old_ener == STP_UPDATE_ENERGY_RATE {
        h.update_old_ener = 1;
        for ch in 0..n_in {
            h.old_dry_ener_ld64[ch] =
                calc_ld_data(f_add_saturate(h.run_dry_ener[ch], ABS_THR__FDK));
        }
        for ch in 0..n_out {
            if self_.tree_config == TREE_212 {
                h.old_wet_ener_ld64[ch] =
                    calc_ld_data(f_add_saturate(h.run_wet_ener[ch], ABS_THR__FDK));
            } else {
                h.old_wet_ener_ld64[ch] =
                    calc_ld_data(f_add_saturate(h.run_wet_ener[ch], ABS_THR2__FDK));
            }
        }
    } else {
        h.update_old_ener += 1;
    }

    // get channel configuration
    let (i_lf, i_rf) = match self_.tree_config {
        TREE_212 => (0usize, 1usize),
        _ => return MPS_WRONG_TREECONFIG,
    };

    // form the 'direct' downmix signal
    // pBP = hStpDec->BP_GF - BP_GF_START: pBP[n] is BP_GF__FDK[n - BP_GF_START].
    let p_bp_gf = |n: usize| -> FixpCfg { BP_GF__FDK[n - BP_GF_START] };
    if self_.tree_config == TREE_212 {
        let lr = &self_.hyb_output_real_dry__fdk;
        let li = &self_.hyb_output_imag_dry__fdk;
        let s = OFF + BP_GF_START;
        let e = OFF + cplx_bands;
        let s_real = f_min(get_scalefactor(&lr[i_lf][s..e]), get_scalefactor(&lr[i_rf][s..e]));
        let s_imag = f_min(get_scalefactor(&li[i_lf][s..e]), get_scalefactor(&li[i_rf][s..e]));
        let s_min = f_min(s_real, s_imag) - 1;

        for n in BP_GF_START..cplx_bands {
            let dmx_real0 = scale_value(lr[i_lf][OFF + n], s_min)
                .wrapping_add(scale_value(lr[i_rf][OFF + n], s_min));
            let dmx_imag0 = scale_value(li[i_lf][OFF + n], s_min)
                .wrapping_add(scale_value(li[i_rf][OFF + n], s_min));

            dry_ener0 = dry_ener0.wrapping_add(
                f_mult_div2(f_pow2_div2(dmx_real0), p_bp_gf(n))
                    .wrapping_add(f_mult_div2(f_pow2_div2(dmx_imag0), p_bp_gf(n)))
                    >> SF_DRY_NRG,
            );
        }

        let s_norm = SF_FREQ_DOMAIN_HEADROOM + SF_DRY_NRG + dry_scale_dmx - (2 * s_min) + nrg_scale;
        dry_ener0 = scale_value_saturate(
            dry_ener0,
            f_max(f_min(s_norm, DFRACT_BITS - 1), -(DFRACT_BITS - 1)),
        );
    }
    dry_ener[0] = dry_ener0;

    let h = &mut self_.h_stp_dec;
    // normalise the 'direct' signals
    let mut ch = 0;
    while ch < n_in {
        if self_.tree_config != TREE_212 {
            dry_ener[ch] = dry_ener[ch].wrapping_shl(nrg_scale as u32);
        }
        h.run_dry_ener[ch] = f_mult(STP_LPF_COEFF1__FDK, h.run_dry_ener[ch])
            .wrapping_add(f_mult(ONE_MINUS_STP_LPF_COEFF1__FDK, dry_ener[ch]));
        if dry_ener[ch] != 0 {
            dry_ener_ld64[ch] =
                fix_max(calc_ld_data(dry_ener[ch]).wrapping_sub(h.old_dry_ener_ld64[ch]), LD64_MIN);
        } else {
            dry_ener_ld64[ch] = LD64_MIN;
        }
        ch += 1;
    }
    while ch < MAX_INPUT_CHANNELS {
        dry_ener_ld64[ch] = LD64_MIN;
        ch += 1;
    }

    // normalise the 'diffuse' signals
    for ch in 0..n_out {
        if skip_channels[ch] != 0 {
            continue;
        }

        let mut wet_ener_x: FixpDbl = 0;

        if self_.tree_config == TREE_212 {
            let wr = &self_.hyb_output_real_wet__fdk[ch];
            let wi = &self_.hyb_output_imag_wet__fdk[ch];
            let s = OFF + BP_GF_START;
            let e = OFF + cplx_bands;
            let s_min = f_min(get_scalefactor(&wr[s..e]), get_scalefactor(&wi[s..e]));

            for n in BP_GF_START..cplx_bands {
                wet_ener_x = wet_ener_x.wrapping_add(
                    f_mult_div2(f_pow2_div2(scale_value(wr[OFF + n], s_min)), p_bp_gf(n))
                        .wrapping_add(f_mult_div2(
                            f_pow2_div2(scale_value(wi[OFF + n], s_min)),
                            p_bp_gf(n),
                        ))
                        >> SF_WET_NRG,
                );
            }
            let s_norm =
                SF_FREQ_DOMAIN_HEADROOM + SF_WET_NRG + wet_scale_dmx - (2 * s_min) + nrg_scale;
            wet_ener_x = scale_value_saturate(
                wet_ener_x,
                f_max(f_min(s_norm, DFRACT_BITS - 1), -(DFRACT_BITS - 1)),
            );
        } else {
            fdk_assert!(self_.tree_config == TREE_212);
        }

        let h = &mut self_.h_stp_dec;
        h.run_wet_ener[ch] = f_mult(STP_LPF_COEFF1__FDK, h.run_wet_ener[ch])
            .wrapping_add(f_mult(ONE_MINUS_STP_LPF_COEFF1__FDK, wet_ener_x));

        if wet_ener_x == 0 {
            wet_ener_ld64[ch] = LD64_MIN;
        } else {
            wet_ener_ld64[ch] =
                fix_max(calc_ld_data(wet_ener_x).wrapping_sub(h.old_wet_ener_ld64[ch]), LD64_MIN);
        }
    }

    // compute scale factor for the 'diffuse' signals
    if self_.tree_config == TREE_212 && dry_ener[0] != 0 {
        scale[i_lf] = calc_wet_scale(dry_ener_ld64[0], wet_ener_ld64[i_lf]);
        scale[i_rf] = calc_wet_scale(dry_ener_ld64[0], wet_ener_ld64[i_rf]);
    }

    let damp = fl2fxconst_dbl((0.1f32 / (1 << SF_SCALE) as f32) as f64);
    let h = &mut self_.h_stp_dec;
    for ch in 0..n_out {
        // damp the scaling factor
        scale[ch] = damp.wrapping_add(f_mult(fl2fxconst_dbl(0.9f32 as f64), scale[ch]));

        // limiting the scale factor
        scale[ch] = scale[ch].clamp(ONE_DIV_STP_SCALE_LIMIT__FDK, STP_SCALE_LIMIT__FDK);

        // low pass filter the scaling factor
        scale[ch] = f_mult(STP_LPF_COEFF2__FDK, scale[ch])
            .wrapping_add(f_mult(ONE_MINUS_STP_LPF_COEFF2__FDK, h.prev_tp_scale[ch]));
        h.prev_tp_scale[ch] = scale[ch];
    }

    // combine 'direct' and scaled 'diffuse' signal
    fdk_assert!((HP_SIZE - 3 + 10 - 1) as i32 == PC_NUM_HYB_BANDS);
    let channl_index = ROW2CHANNEL_STP.get(self_.tree_config as usize).copied().unwrap_or([0, 0]);

    let border = self_.tp_hyb_band_border.max(0) as usize;
    const HB15: usize = HP_SIZE - 3 + 10 - 1;
    for ch in 0..n_out {
        let ci = (channl_index[ch].max(0) as usize).min(MAX_OUTPUT_CHANNELS - 1);
        let no_scaling = frame.temp_shape_enable_channel_stp[ci] == 0;
        let rd = &mut self_.hyb_output_real_dry__fdk[ch];
        let id = &mut self_.hyb_output_imag_dry__fdk[ch];
        let rw = &self_.hyb_output_real_wet__fdk[ch];
        let iw = &self_.hyb_output_imag_wet__fdk[ch];

        if no_scaling {
            combine_signal_cplx(
                &mut rd[border..],
                &mut id[border..],
                &rw[border..],
                &iw[border..],
                cplx_hyb_bands - self_.tp_hyb_band_border,
            );
        } else {
            let scale_x = scale[ch];
            // pBP = hStpDec->BP - self->tp_hybBandBorder: &pBP[tp_hybBandBorder] is BP__FDK.
            // Band[HP_SIZE-3+10-1] needs not to be processed in combineSignalCplxScale1(),
            // because pB[HP_SIZE-3+10-1] would be 1.0
            combine_signal_cplx_scale1(
                &mut rd[border..],
                &mut id[border..],
                &rw[border..],
                &iw[border..],
                &BP__FDK,
                scale_x,
                HB15 as i32 - self_.tp_hyb_band_border,
            );

            combine_signal_cplx_scale2(
                &mut rd[HB15..],
                &mut id[HB15..],
                &rw[HB15..],
                &iw[HB15..],
                scale_x,
                cplx_hyb_bands - HB15 as i32,
            );
        }
    }

    MPS_OK
}
