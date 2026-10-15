// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACdec/src/sac_process.h, libSACdec/src/sac_process.cpp.

//! Port of `sac_process.h` and `sac_process.cpp`: the slot processing of the spatial decoder
//! (QMF and hybrid analysis, the M1/M2 matrices, the phase, the synthesis).
//!
//! The C functions take arrays of row pointers (`FIXP_DBL **qmfReal`, `**hybOutputRealDry`,
//! ...); every caller passes the arrays of the decoder itself, so the port's functions use
//! the members of [`SpatialDec`] directly (see the [`crate::sac_dec`] module documentation).
//!
//! # Undefined behavior
//!
//! The interpolations, the sums of the M2 products and the shifts `<< (1 + M2_exp)`, `<< s`,
//! `<<= PHASE_SCALE - 1` can overflow for extreme parameters and signals; both builds compile
//! them as 32 bit wrapping operations (`wrapping_*` in the port). The negations of
//! `interpolateParameter()` results and of the cosine/sine values wrap for `MINVAL_DBL`.

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::decorrelate::{fdk_decorrelate_apply, fdk_decorrelate_apply_in_place};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::hybrid::*;
use fdk_aac_rs_fdk::qmf::QMF_MAX_SYNTHESIS_BANDS;
use fdk_aac_rs_fdk::qmf_domain::*;
use fdk_aac_rs_fdk::syslib_channel_map_descr::{
    fdk_ch_map_descr_get_map_value, FdkChannelMapDescr,
};
use fdk_aac_rs_fdk::trig_fcts::inline_fixp_cos_sin;

use crate::sac_calc_m1_and_m2::*;
use crate::sac_dec::*;
use crate::sac_dec_interface::*;
use crate::sac_qmf::*;
use crate::sac_rom::*;
use crate::sac_tsd::*;

pub mod tables;
pub use tables::*;

/// `SAC_DEC_APPLY_M2_SCALE(spec, s)`: `(spec) >> (-(s))` (`s` is 0 or -1 in the library).
#[inline(always)]
fn sac_dec_apply_m2_scale(spec: FixpDbl, s: i32) -> FixpDbl {
    spec >> (-s)
}

/// `interpolateParameter(alpha, a, b)`: linear interpolation between two parameter values,
/// `a*alpha + b*(1-alpha) = a*alpha + b - b*alpha`.
#[inline(always)]
pub fn interpolate_parameter(alpha: FixpSgl, a: FixpDbl, b: FixpDbl) -> FixpDbl {
    b.wrapping_sub(f_mult(alpha, b)).wrapping_add(f_mult(alpha, a))
}

/// `mapChannel(self, ch)`: maps MPEG Surround channel indices to MPEG-4 PCE like channel
/// indices (`chanelIdx[0][ch]`; `ch` is an output channel index below 2; deviation: indices
/// outside of the row map to themselves).
pub fn map_channel(_self: &SpatialDec, ch: u32) -> u32 {
    let idx = 0;
    CHANEL_IDX[idx].get(ch as usize).map_or(ch, |&c| c as u32)
}

/// `getChGain(self, ch, scale)`: the gain of an output channel (always `0x80000000`, "no gain
/// modifier", with the scale 0). Returns `(gain, scale)`.
pub fn get_ch_gain(self_: &SpatialDec, ch: u32) -> (FixpDbl, i32) {
    // init no gain modifier
    let gain = 0x80000000u32 as FixpDbl;

    if !is_two_ch_mode(self_.upmix_type) && self_.upmix_type != UPMIXTYPE_BYPASS {
        if ch == 0 || ch == 1 || ch == 2 {
            // no modifier
        }
    }

    (gain, 0)
}

/// The input samples of [`spatial_dec_apply_frame`] (`inData`): `NULL`, a separate buffer, or
/// an offset in the output buffer (USAC with `stereoConfigIndex > 1` places the input samples
/// in the output buffer of the frame, which the slots are written to while the input is read).
#[derive(Clone, Copy, Debug)]
pub enum PcmMpsInput<'a> {
    /// `NULL`
    None,
    /// A separate buffer.
    Buffer(&'a [PcmMps]),
    /// The output buffer, from this offset.
    Output(usize),
}

/// `n` input samples from `pos` of the input (`inData + pos`), 0 outside of the buffer.
///
/// Deviation: positions outside of the caller's buffer read 0.
pub(crate) fn read_input(input: PcmMpsInput<'_>, out: &[PcmMps], pos: i64, dst: &mut [PcmMps]) {
    let src: &[PcmMps] = match input {
        PcmMpsInput::None => &[],
        PcmMpsInput::Buffer(b) => b,
        PcmMpsInput::Output(off) => out.get(off..).unwrap_or(&[]),
    };
    for (i, d) in dst.iter_mut().enumerate() {
        let p = pos + i as i64;
        *d = if p >= 0 { src.get(p as usize).copied().unwrap_or(0) } else { 0 };
    }
}

/// `pc_filterdelay` division of the QMF functions: `(qmfInputDelayBufPos + 1) %
/// pc_filterdelay` (deviation: a divisor 0 keeps the position).
fn advance_delay_buf_pos(self_: &mut SpatialDec) {
    if let Some(v) =
        (self_.qmf_input_delay_buf_pos.wrapping_add(1)).checked_rem(self_.pc_filterdelay)
    {
        self_.qmf_input_delay_buf_pos = v;
    }
}

/// `SpatialDecQMFAnalysis(self, inData, ts, bypassMode, qmfReal, qmfImag, numInputChannels)`:
/// QMF analysis of the downmix time data into `self->qmfInput{Real,Imag}__FDK`. `in_data` and
/// `time_out` give `inData` (see [`PcmMpsInput`]).
pub fn spatial_dec_qmf_analysis(
    self_: &mut SpatialDec,
    qd: &mut FdkQmfDomain,
    in_data: PcmMpsInput<'_>,
    time_out: &[PcmMps],
    ts: i32,
    bypass_mode: i32,
    num_input_channels: i32,
) -> SacdecError {
    let err = MPS_OK;

    let offset = qd.global_conf.n_bands_synthesis as i64 * qd.global_conf.n_qmf_time_slots as i64;

    for ch in 0..(num_input_channels.max(0) as usize).min(NUM_IN_CH) {
        let base = ts as i64 * qd.global_conf.n_bands_analysis as i64;
        let mut in_samples = [0 as PcmMps; 2 * NUM_QMF];
        read_input(in_data, time_out, base + ch as i64 * offset, &mut in_samples);

        // no delay in blind mode
        let qd_in = &mut qd.qmf_domain_in[ch];
        let states = qd_in.p_ana_qmf_states.as_deref_mut().unwrap_or(&mut []);
        calculate_space_analysis_qmf(
            Some((&qd_in.fb, states)),
            &in_samples,
            &mut self_.qmf_input_real__fdk[ch],
            &mut self_.qmf_input_imag__fdk[ch],
        );

        if !is_two_ch_mode(self_.upmix_type) && bypass_mode == 0 {
            for i in 0..(self_.qmf_bands.max(0) as usize).min(NUM_QMF) {
                self_.qmf_input_real__fdk[ch][i] = f_mult(
                    scale_value_saturate(
                        self_.qmf_input_real__fdk[ch][i],
                        self_.sac_in_data_headroom.wrapping_sub(1),
                    ),
                    self_.clip_protect_gain__fdk,
                );
                self_.qmf_input_imag__fdk[ch][i] = f_mult(
                    scale_value_saturate(
                        self_.qmf_input_imag__fdk[ch][i],
                        self_.sac_in_data_headroom.wrapping_sub(1),
                    ),
                    self_.clip_protect_gain__fdk,
                );
            }
        }
    }

    advance_delay_buf_pos(self_);

    err
}

/// `SpatialDecFeedQMF(self, qmfInDataReal, qmfInDataImag, ts, bypassMode, qmfReal__FDK,
/// qmfImag__FDK, numInputChannels)`: takes the downmix QMF data of slot `ts` from the QMF
/// domain (`qmfInDataReal`/`qmfInDataImag` are unused, `NULL` in the library).
pub fn spatial_dec_feed_qmf(
    self_: &mut SpatialDec,
    qd: &mut FdkQmfDomain,
    ts: i32,
    bypass_mode: i32,
    num_input_channels: i32,
) -> SacdecError {
    let err = MPS_OK;

    for ch in 0..(num_input_channels.max(0) as usize).min(NUM_IN_CH) {
        // no delay in blind mode
        let (re, im) = (&mut self_.qmf_input_real__fdk[ch], &mut self_.qmf_input_imag__fdk[ch]);

        // Write Input data to pQmfRealAnalysis.
        if self_.b_share_delay_with_sbr != 0 {
            fdk_qmf_domain_get_slot(
                qd,
                ch,
                ts + HYBRID_FILTER_DELAY,
                0,
                MAX_QMF_BANDS_TO_HYBRID,
                re,
                Some(im),
                15 + 1,
            );
            fdk_qmf_domain_get_slot(
                qd,
                ch,
                ts,
                MAX_QMF_BANDS_TO_HYBRID,
                self_.qmf_bands.min(NUM_QMF as i32),
                re,
                Some(im),
                15 + 1,
            );
        } else {
            fdk_qmf_domain_get_slot(
                qd,
                ch,
                ts,
                0,
                self_.qmf_bands.min(NUM_QMF as i32),
                re,
                Some(im),
                15 + 1,
            );
        }

        if ts == qd.global_conf.n_qmf_time_slots as i32 - 1 {
            // Is currently also needed in case we dont have any overlap. We need to save
            // lb_scale to ov_lb_scale
            fdk_qmf_domain_save_overlap(qd, ch, 0);
        }

        // Apply clip protection to output.
        if !is_two_ch_mode(self_.upmix_type) && bypass_mode == 0 {
            for i in 0..(self_.qmf_bands.max(0) as usize).min(NUM_QMF) {
                self_.qmf_input_real__fdk[ch][i] =
                    f_mult(self_.qmf_input_real__fdk[ch][i], self_.clip_protect_gain__fdk);
                self_.qmf_input_imag__fdk[ch][i] =
                    f_mult(self_.qmf_input_imag__fdk[ch][i], self_.clip_protect_gain__fdk);
            }
        }
    } // End of loop over numInputChannels

    advance_delay_buf_pos(self_);

    err
}

/// `SpatialDecHybridAnalysis(self, qmfInputReal, qmfInputImag, hybOutputReal, hybOutputImag,
/// ts, numInputChannels)`: hybrid analysis of `self->qmfInput*` into `self->hybInput*` (and
/// of the residual).
pub fn spatial_dec_hybrid_analysis(
    self_: &mut SpatialDec,
    cfg: &SpatialSpecificConfig,
    _ts: i32,
    num_input_channels: i32,
) -> SacdecError {
    let err = MPS_OK;

    // hybrid filtering for down-mix signals
    for ch in 0..(num_input_channels.max(0) as usize).min(NUM_IN_CH) {
        if cfg.syntax_flags & SACDEC_SYNTAX_LD != 0 {
            // No hybrid filtering. Just copy the QMF data.
            for k in 0..(self_.hybrid_bands.max(0) as usize).min(NUM_QMF) {
                self_.hyb_input_real__fdk[ch][k] = self_.qmf_input_real__fdk[ch][k];
                self_.hyb_input_imag__fdk[ch][k] = self_.qmf_input_imag__fdk[ch][k];
            }
        } else {
            self_.hybrid_analysis[ch].hf_mode = self_.b_share_delay_with_sbr as u8;
            if self_.stereo_config_index == 3 {
                fdk_assert!(self_.hybrid_analysis[ch].hf_mode == 0);
            }
            fdk_hybrid_analysis_apply(
                &mut self_.hybrid_analysis[ch],
                &self_.qmf_input_real__fdk[ch],
                &self_.qmf_input_imag__fdk[ch],
                &mut self_.hyb_input_real__fdk[ch],
                &mut self_.hyb_input_imag__fdk[ch],
            );
        }
    }

    if cfg.syntax_flags & SACDEC_SYNTAX_USAC != 0 && self_.residual_coding != 0 {
        // hybridAnalysis[numInputChannels]: numInputChannels is 1 in the decoder (deviation:
        // values above 1 use the residual filter bank as well).
        let n = (num_input_channels.max(0) as usize).min(NUM_IN_CH);
        let ha = &mut self_.hybrid_analysis[n];
        ha.hf_mode = 0;
        fdk_hybrid_analysis_apply(
            ha,
            &self_.qmf_residual_real__fdk[0][0],
            &self_.qmf_residual_imag__fdk[0][0],
            &mut self_.hyb_residual_real__fdk[0],
            &mut self_.hyb_residual_imag__fdk[0],
        );
    }

    err
}

/// `SpatialDecCreateX(self, hybInputReal, hybInputImag, pxReal, pxImag)`: the X signals are
/// the downmix hybrid signals (`pxReal[row] = hybInputReal[row]`); the port's
/// [`spatial_dec_apply_m1_create_w_mode212`] reads `self->hybInput*` directly.
pub fn spatial_dec_create_x(_self: &SpatialDec) -> SacdecError {
    MPS_OK
}

/// `M2ParamToKernelMult(pKernel, Mparam, MparamPrev, pWidth, alpha__FDK, nBands)`: writes the
/// interpolated parameters of `nBands` parameter bands, `pWidth[pb]` times each.
fn m2_param_to_kernel_mult(
    p_kernel: &mut [FixpSgl; MAX_HYBRID_BANDS],
    mparam: &[FixpDbl; MAX_PARAMETER_BANDS],
    mparam_prev: &[FixpDbl; MAX_PARAMETER_BANDS],
    p_width: &[i32; MAX_PARAMETER_BANDS],
    alpha__fdk: FixpSgl,
    n_bands: i32,
) {
    let mut k = 0usize;
    let mut put = |v: FixpSgl| {
        // The widths of the bands of the library sum up to hybridBands <= 71.
        if let Some(d) = p_kernel.get_mut(k) {
            *d = v;
        }
        k += 1;
    };
    for pb in 0..(n_bands.max(0) as usize).min(MAX_PARAMETER_BANDS) {
        let tmp = fx_dbl2fx_sgl(interpolate_parameter(alpha__fdk, mparam[pb], mparam_prev[pb]));

        let mut i = p_width[pb];
        if i & 1 != 0 {
            put(tmp);
        }
        if i & 2 != 0 {
            put(tmp);
            put(tmp);
        }
        // for (i >>= 2; i--;): the port stops for a negative width.
        i >>= 2;
        while i > 0 {
            i -= 1;
            put(tmp);
            put(tmp);
            put(tmp);
            put(tmp);
        }
    }
}

/// `SpatialDecApplyM1_CreateW_Mode212(self, frame, xReal, xImag, vReal, vImag)`: MPS212
/// version of applying M1 and creating the wet signal: `self->wReal__FDK[0]` (direct) and
/// `[1]` (wet) from `self->hybInput*` (the X signals).
pub fn spatial_dec_apply_m1_create_w_mode212(
    self_: &mut SpatialDec,
    frame: &SpatialBsFrame,
) -> SacdecError {
    let err = MPS_OK;

    // M1 does not do anything in 212 mode, so use simplified processing
    fdk_assert!(self_.num_v_channels == 2);
    fdk_assert!(self_.num_direkt_signals == 1);
    fdk_assert!(self_.num_decor_signals == 1);
    let hb = (self_.hybrid_bands.max(0) as usize).min(NUM_HYB);
    self_.w_real__fdk[0][..hb].copy_from_slice(&self_.hyb_input_real__fdk[0][..hb]);
    self_.w_imag__fdk[0][..hb].copy_from_slice(&self_.hyb_input_imag__fdk[0][..hb]);

    let mut decorr_in = TsdDecorrInput::Direct;
    if is_tsd_active(&frame.tsd_data[0]) != 0 {
        // Generate v_{x,nonTr} as input for allpass based decorrelator
        let [w0, w1] = &mut self_.w_real__fdk;
        let [i0, i1] = &mut self_.w_imag__fdk;
        decorr_in = tsd_generate_non_tr(
            self_.hybrid_bands,
            &frame.tsd_data[0],
            self_.tsd_ts,
            w0,
            i0,
            w1,
            i1,
        );
    }

    // - Decorrelate
    let res = spatial_dec_get_residual_index(self_, 1);
    let res_bands = self_.residual_bands.get(res as usize).copied().unwrap_or(0) as usize;
    let start = self_.param2hyb[res_bands.min(MAX_PARAMETER_BANDS)];
    let e = {
        let [w0, w1] = &mut self_.w_real__fdk;
        let [i0, i1] = &mut self_.w_imag__fdk;
        match decorr_in {
            TsdDecorrInput::Direct => {
                fdk_decorrelate_apply(&mut self_.ap_decor[0], &w0[..], &i0[..], w1, i1, start)
            }
            TsdDecorrInput::NonTr => {
                fdk_decorrelate_apply_in_place(&mut self_.ap_decor[0], w1, i1, start)
            }
        }
    };
    if e != 0 {
        return MPS_NOTOK;
    }

    if is_tsd_active(&frame.tsd_data[0]) != 0 {
        // Generate v_{x,Tr}, apply transient decorrelator and add to allpass based
        // decorrelator output
        let [w0, w1] = &mut self_.w_real__fdk;
        let [i0, i1] = &mut self_.w_imag__fdk;
        tsd_apply(
            self_.hybrid_bands,
            &frame.tsd_data[0],
            &mut self_.tsd_ts,
            w0, // input: v_x
            i0,
            w1, // input: d_{x,nonTr}; output: d_{x,nonTr} + d_{x,Tr}
            i1,
        );
    }

    // Write residual signal in appropriate parameter bands
    if res_bands > 0 {
        let stop_band = start;
        let n = (fix_min(stop_band, self_.hybrid_bands).max(0) as usize).min(NUM_HYB);
        // res is 0 (the only residual channel).
        let r = (res.max(0) as usize).min(NUM_RES_CH - 1);
        self_.w_real__fdk[1][..n].copy_from_slice(&self_.hyb_residual_real__fdk[r][..n]);
        self_.w_imag__fdk[1][..n].copy_from_slice(&self_.hyb_residual_imag__fdk[r][..n]);
    } // (self->residualBands[res]>0)

    err
}

/// The `do { ... } while (--i != 0)` loops over the hybrid bands of a parameter band: calls
/// `f` with each hybrid band index. A width of 0 runs the body once (deviation); indices past
/// the arrays end the loop.
#[inline(always)]
fn for_width(q: &mut usize, width: i32, mut f: impl FnMut(usize)) {
    let mut i = width;
    loop {
        if *q >= NUM_HYB {
            return;
        }
        f(*q);
        *q += 1;
        i = i.wrapping_sub(1);
        if i <= 0 {
            break;
        }
    }
}

/// `pb_max = self->kernels[self->hybridBands - 1] + 1`
fn pb_max_of(self_: &SpatialDec) -> usize {
    ((kernel_at(self_, self_.hybrid_bands - 1) + 1).max(0) as usize).min(MAX_PARAMETER_BANDS)
}

/// `SpatialDecApplyM2_Mode212(self, ps, alpha, wReal, wImag, hybOutputRealDry,
/// hybOutputImagDry)`: applies M2 for the 212 mode (mono to stereo upmix).
pub fn spatial_dec_apply_m2_mode212(
    self_: &mut SpatialDec,
    _ps: i32,
    alpha: FixpSgl,
) -> SacdecError {
    let err = MPS_OK;
    let p_width = self_.kernels_width;
    // for stereoConfigIndex == 3 case hybridBands is < 71
    let pb_max = pb_max_of(self_);
    let max_row = (self_.num_output_channels.max(0) as usize).min(NUM_OUT_CH);
    let m2_exp = if self_.residual_coding != 0 { 3 } else { 0 };

    for row in 0..max_row {
        let mparam0 = &self_.m2_real__fdk[row][0];
        let mparam1 = &self_.m2_real__fdk[row][1];
        let mparam_prev0 = &self_.m2_real_prev__fdk[row][0];
        let mparam_prev1 = &self_.m2_real_prev__fdk[row][1];
        let w_real0 = &self_.w_real__fdk[0];
        let w_real1 = &self_.w_real__fdk[1];
        let w_imag0 = &self_.w_imag__fdk[0];
        let w_imag1 = &self_.w_imag__fdk[1];
        let out_re = &mut self_.hyb_output_real_dry__fdk[row];
        let out_im = &mut self_.hyb_output_imag_dry__fdk[row];

        let mut q = 0usize;
        for pb in 0..pb_max {
            let tmp0 = interpolate_parameter(alpha, mparam0[pb], mparam_prev0[pb]);
            let tmp1 = interpolate_parameter(alpha, mparam1[pb], mparam_prev1[pb]);

            for_width(&mut q, p_width[pb], |k| {
                let var0 = w_real0[k];
                let var1 = w_real1[k];
                let mut real = f_mult_div2(var0, tmp0);
                let var0 = w_imag0[k];
                real = fixmadddiv2_dd(real, var1, tmp1);
                let var1 = w_imag1[k];
                let mut imag = f_mult_div2(var0, tmp0);
                out_re[k] = real.wrapping_shl(1 + m2_exp);
                imag = fixmadddiv2_dd(imag, var1, tmp1);
                out_im[k] = imag.wrapping_shl(1 + m2_exp);
            });
        }
    }
    err
}

/// `SpatialDecApplyM2_Mode212_ResidualsPlusPhaseCoding(self, ps, alpha, wReal, wImag,
/// hybOutputRealDry, hybOutputImagDry)`: applies M2 for the 212 mode with residual and phase
/// coding.
pub fn spatial_dec_apply_m2_mode212_residuals_plus_phase_coding(
    self_: &mut SpatialDec,
    cfg: &SpatialSpecificConfig,
    _ps: i32,
    alpha: FixpSgl,
) -> SacdecError {
    let err = MPS_OK;
    let p_width = self_.kernels_width;
    let pb_max = pb_max_of(self_);

    let scale_param_m2 = SCALE_PARAM_M2_212_PRED + SCALE_DATA_APPLY_M2;

    for row in 0..(self_.num_m2rows.max(0) as usize).min(NUM_OUT_CH) {
        let w_real0 = &self_.w_real__fdk[0];
        let w_imag0 = &self_.w_imag__fdk[0];
        let w_real1 = &self_.w_real__fdk[1];
        let w_imag1 = &self_.w_imag__fdk[1];
        let m_real0 = &self_.m2_real__fdk[row][0];
        let m_imag0 = &self_.m2_imag__fdk[row][0];
        let m_real1 = &self_.m2_real__fdk[row][1];
        let m_real_prev0 = &self_.m2_real_prev__fdk[row][0];
        let m_imag_prev0 = &self_.m2_imag_prev__fdk[row][0];
        let m_real_prev1 = &self_.m2_real_prev__fdk[row][1];
        let out_re = &mut self_.hyb_output_real_dry__fdk[row];
        let out_im = &mut self_.hyb_output_imag_dry__fdk[row];

        fdk_assert!(cfg.syntax_flags & SACDEC_SYNTAX_LD == 0);
        fdk_assert!((p_width[0] + p_width[1]) >= 3);

        let mut q = 0usize;
        let mut qs = 3;
        let mut pb = 0usize;
        while pb < 2 {
            let i_real0 = interpolate_parameter(alpha, m_real0[pb], m_real_prev0[pb]);
            let i_imag0 =
                interpolate_parameter(alpha, m_imag0[pb], m_imag_prev0[pb]).wrapping_neg();
            let i_real1 = interpolate_parameter(alpha, m_real1[pb], m_real_prev1[pb]);

            let mut max_val = f_abs(i_real0) | f_abs(i_imag0);
            max_val |= f_abs(i_real1);

            let s = f_min(cnt_leading_zeros(max_val) - 2, scale_param_m2);

            let m_real0v = scale_value(i_real0, s);
            let mut m_imag0v = scale_value(i_imag0, s);
            let m_real1v = scale_value(i_real1, s);

            let s = scale_param_m2 - s;

            for_width(&mut q, p_width[pb], |k| {
                let (wr0, wi0, wr1, wi1) = (w_real0[k], w_imag0[k], w_real1[k], w_imag1[k]);
                let (real, imag) = cplx_mult_div2(wr0, wi0, m_real0v, m_imag0v);
                out_re[k] = fixmadddiv2_dd(real, wr1, m_real1v).wrapping_shl(s as u32);
                out_im[k] = fixmadddiv2_dd(imag, wi1, m_real1v).wrapping_shl(s as u32);
                if qs > 0 {
                    m_imag0v = m_imag0v.wrapping_neg();
                    qs -= 1;
                }
            });
            pb += 1;
        }

        while pb < pb_max {
            let i_real0 = interpolate_parameter(alpha, m_real0[pb], m_real_prev0[pb]);
            let i_imag0 = interpolate_parameter(alpha, m_imag0[pb], m_imag_prev0[pb]);
            let i_real1 = interpolate_parameter(alpha, m_real1[pb], m_real_prev1[pb]);

            let mut max_val = f_abs(i_real0) | f_abs(i_imag0);
            max_val |= f_abs(i_real1);

            let s = f_min(cnt_leading_zeros(max_val) - 2, scale_param_m2);

            let m_real0v: FixpSgl = fx_dbl2fx_sgl(scale_value(i_real0, s));
            let m_imag0v: FixpSgl = fx_dbl2fx_sgl(scale_value(i_imag0, s));
            let m_real1v: FixpSgl = fx_dbl2fx_sgl(scale_value(i_real1, s));

            let s = scale_param_m2 - s;

            for_width(&mut q, p_width[pb], |k| {
                let (wr0, wi0, wr1, wi1) = (w_real0[k], w_imag0[k], w_real1[k], w_imag1[k]);
                let (real, imag) = cplx_mult_div2(wr0, wi0, m_real0v, m_imag0v);
                out_re[k] = fixmadddiv2_ds(real, wr1, m_real1v).wrapping_shl(s as u32);
                out_im[k] = fixmadddiv2_ds(imag, wi1, m_real1v).wrapping_shl(s as u32);
            });
            pb += 1;
        }
    }
    err
}

/// `SpatialDecApplyM2(self, ps, alpha, wReal, wImag, hybOutputRealDry, hybOutputImagDry,
/// hybOutputRealWet, hybOutputImagWet)`: applies M2 (the generic version).
pub fn spatial_dec_apply_m2(self_: &mut SpatialDec, _ps: i32, alpha: FixpSgl) -> SacdecError {
    let err = MPS_OK;

    // C_ALLOC_SCRATCH_START(pKernel, FIXP_SGL, MAX_HYBRID_BANDS): every value read is written
    // before (the widths of the parameter bands sum up to hybridBands).
    let mut p_kernel = [0 as FixpSgl; MAX_HYBRID_BANDS];

    let mut scale_param_m2 = 0;

    // The wet signal is added to the dry signal directly in applyM2 if GES and STP are
    // disabled
    let tools_disabled =
        if self_.temp_shape_config == 1 || self_.temp_shape_config == 2 { 0 } else { 1 };

    let complex_hyb_bands = (self_.hybrid_bands.max(0) as usize).min(NUM_HYB);
    let complex_par_bands = self_.num_parameter_bands;

    // FDKmemclear over the whole matrices (maxNumOutputChannels rows).
    self_.hyb_output_imag_dry__fdk = [[0; NUM_HYB]; NUM_OUT_CH];
    self_.hyb_output_real_dry__fdk = [[0; NUM_HYB]; NUM_OUT_CH];
    if tools_disabled == 0 {
        self_.hyb_output_real_wet__fdk = [[0; NUM_HYB]; NUM_OUT_CH];
        self_.hyb_output_imag_wet__fdk = [[0; NUM_HYB]; NUM_OUT_CH];
    }

    if self_.phase_coding == 3 {
        scale_param_m2 = -(SCALE_DATA_APPLY_M2_PC - 1);
    }
    let sc = |x: FixpDbl, k: FixpSgl| sac_dec_apply_m2_scale(f_mult_div2(x, k), scale_param_m2);

    let num_direkt = self_.num_direkt_signals.max(0) as usize;
    let num_v = (self_.num_v_channels.max(0) as usize).min(NUM_V_CH);
    let kernels_width = self_.kernels_width;
    let npb = self_.num_parameter_bands;

    // The rows of the current output channel: DR, DI (dry), and the wet rows WR, WI, which
    // are the dry rows if the tools are disabled (pHybOutRealWet = hybOutputRealDry[row]).
    const DR: usize = 0;
    const DI: usize = 1;
    let (wr, wi) = if tools_disabled != 0 { (0usize, 1usize) } else { (2, 3) };

    for row in 0..(self_.num_m2rows.max(0) as usize).min(NUM_OUT_CH) {
        let mut o = [
            self_.hyb_output_real_dry__fdk[row],
            self_.hyb_output_imag_dry__fdk[row],
            self_.hyb_output_real_wet__fdk[row],
            self_.hyb_output_imag_wet__fdk[row],
        ];

        let mut col = 0usize;
        while col < num_direkt.min(num_v) {
            // pActivM2ParamBands is NULL: default setting, calculate all rows and columns
            let p_w_real = &self_.w_real__fdk[col];
            let p_w_imag = &self_.w_imag__fdk[col];

            m2_param_to_kernel_mult(
                &mut p_kernel,
                &self_.m2_real__fdk[row][col],
                &self_.m2_real_prev__fdk[row][col],
                &kernels_width,
                alpha,
                npb,
            );

            if self_.phase_coding != 3 {
                // direct signals
                // only one sample will be assigned to each row, hence accumulation is not
                // necessary; that is valid for all configurations
                for qs in 0..complex_hyb_bands {
                    o[DR][qs] = f_mult(p_w_real[qs], p_kernel[qs]);
                    o[DI][qs] = f_mult(p_w_imag[qs], p_kernel[qs]);
                }
            } else {
                // isBinauralMode(self->upmixType)
                for qs in 0..complex_hyb_bands {
                    o[DR][qs] = o[DR][qs].wrapping_add(sc(p_w_real[qs], p_kernel[qs]));
                    o[DI][qs] = o[DI][qs].wrapping_add(sc(p_w_imag[qs], p_kernel[qs]));
                }

                m2_param_to_kernel_mult(
                    &mut p_kernel,
                    &self_.m2_imag__fdk[row][col],
                    &self_.m2_imag_prev__fdk[row][col],
                    &kernels_width,
                    alpha,
                    complex_par_bands,
                );

                // direct signals sign is -1 for qs = 0,2
                o[DR][0] = o[DR][0].wrapping_add(sc(p_w_imag[0], p_kernel[0]));
                o[DI][0] = o[DI][0].wrapping_sub(sc(p_w_real[0], p_kernel[0]));

                o[DR][2] = o[DR][2].wrapping_add(sc(p_w_imag[2], p_kernel[2]));
                o[DI][2] = o[DI][2].wrapping_sub(sc(p_w_real[2], p_kernel[2]));

                // direct signals sign is +1 for qs = 1,3,4,5,...,complexHybBands
                o[DR][1] = o[DR][1].wrapping_sub(sc(p_w_imag[1], p_kernel[1]));
                o[DI][1] = o[DI][1].wrapping_add(sc(p_w_real[1], p_kernel[1]));

                for qs in 3..complex_hyb_bands {
                    o[DR][qs] = o[DR][qs].wrapping_sub(sc(p_w_imag[qs], p_kernel[qs]));
                    o[DI][qs] = o[DI][qs].wrapping_add(sc(p_w_real[qs], p_kernel[qs]));
                }
            } // self->upmixType
            col += 1;
        } // self->numDirektSignals

        while col < num_v {
            // pActivM2ParamBands is NULL: activParamBands = 1
            let res_band_index = residual_bands_of(self_, col as i32);
            let res_hyb_index = (self_.param2hyb
                [(res_band_index.max(0) as usize).min(MAX_PARAMETER_BANDS)]
            .max(0) as usize)
                .min(NUM_HYB);
            let p_w_real = &self_.w_real__fdk[col];
            let p_w_imag = &self_.w_imag__fdk[col];

            m2_param_to_kernel_mult(
                &mut p_kernel,
                &self_.m2_real__fdk[row][col],
                &self_.m2_real_prev__fdk[row][col],
                &kernels_width,
                alpha,
                npb,
            );

            if self_.phase_coding != 3 {
                // residual signals
                let mut qs = 0usize;
                while qs < res_hyb_index {
                    o[DR][qs] = o[DR][qs].wrapping_add(f_mult(p_w_real[qs], p_kernel[qs]));
                    o[DI][qs] = o[DI][qs].wrapping_add(f_mult(p_w_imag[qs], p_kernel[qs]));
                    qs += 1;
                }
                // decor signals
                while qs < complex_hyb_bands {
                    o[wr][qs] = o[wr][qs].wrapping_add(f_mult(p_w_real[qs], p_kernel[qs]));
                    o[wi][qs] = o[wi][qs].wrapping_add(f_mult(p_w_imag[qs], p_kernel[qs]));
                    qs += 1;
                }
            } else {
                // self->upmixType
                // residual signals
                let mut qs = 0usize;
                while qs < res_hyb_index {
                    o[DR][qs] = o[DR][qs].wrapping_add(sc(p_w_real[qs], p_kernel[qs]));
                    o[DI][qs] = o[DI][qs].wrapping_add(sc(p_w_imag[qs], p_kernel[qs]));
                    qs += 1;
                }
                // decor signals
                while qs < complex_hyb_bands {
                    o[wr][qs] = o[wr][qs].wrapping_add(sc(p_w_real[qs], p_kernel[qs]));
                    o[wi][qs] = o[wi][qs].wrapping_add(sc(p_w_imag[qs], p_kernel[qs]));
                    qs += 1;
                }

                m2_param_to_kernel_mult(
                    &mut p_kernel,
                    &self_.m2_imag__fdk[row][col],
                    &self_.m2_imag_prev__fdk[row][col],
                    &kernels_width,
                    alpha,
                    complex_par_bands,
                );

                // direct signals sign is -1 for qs = 0,2
                // direct signals sign is +1 for qs = 1,3..
                if tools_disabled != 0 {
                    o[DR][0] = o[DR][0].wrapping_add(sc(p_w_imag[0], p_kernel[0]));
                    o[DI][0] = o[DI][0].wrapping_sub(sc(p_w_real[0], p_kernel[0]));

                    o[DR][1] = o[DR][1].wrapping_sub(sc(p_w_imag[1], p_kernel[1]));
                    o[DI][1] = o[DI][1].wrapping_add(sc(p_w_real[1], p_kernel[1]));

                    o[DR][2] = o[DR][2].wrapping_add(sc(p_w_imag[2], p_kernel[2]));
                    o[DI][2] = o[DI][2].wrapping_sub(sc(p_w_real[2], p_kernel[2]));
                } else {
                    // pHybOutReal/pHybOutImag point to the dry rows and switch to the wet
                    // rows at the residual border.
                    let (mut r, mut i) = (DR, DI);
                    if 0 == res_hyb_index {
                        (r, i) = (wr, wi);
                    }
                    o[r][0] = o[r][0].wrapping_add(sc(p_w_imag[0], p_kernel[0]));
                    o[i][0] = o[i][0].wrapping_sub(sc(p_w_real[0], p_kernel[0]));

                    if 1 == res_hyb_index {
                        (r, i) = (wr, wi);
                    }
                    o[r][1] = o[r][1].wrapping_sub(sc(p_w_imag[1], p_kernel[1]));
                    o[i][1] = o[i][1].wrapping_add(sc(p_w_real[1], p_kernel[1]));

                    if 2 == res_hyb_index {
                        (r, i) = (wr, wi);
                    }
                    o[r][2] = o[r][2].wrapping_add(sc(p_w_imag[2], p_kernel[2]));
                    o[i][2] = o[i][2].wrapping_sub(sc(p_w_real[2], p_kernel[2]));
                }

                let mut qs = 3usize;
                while qs < res_hyb_index {
                    o[DR][qs] = o[DR][qs].wrapping_sub(sc(p_w_imag[qs], p_kernel[qs]));
                    o[DI][qs] = o[DI][qs].wrapping_add(sc(p_w_real[qs], p_kernel[qs]));
                    qs += 1;
                }
                // decor signals
                while qs < complex_hyb_bands {
                    o[wr][qs] = o[wr][qs].wrapping_sub(sc(p_w_imag[qs], p_kernel[qs]));
                    o[wi][qs] = o[wi][qs].wrapping_add(sc(p_w_real[qs], p_kernel[qs]));
                    qs += 1;
                }
            } // self->upmixType
            col += 1;
        } // self->numVChannels

        if self_.phase_coding == 3 {
            let s = SCALE_PARAM_M2_212_PRED + SCALE_DATA_APPLY_M2_PC;
            scale_values_saturate(&mut o[DR][..complex_hyb_bands], s);
            scale_values_saturate(&mut o[DI][..complex_hyb_bands], s);
            if tools_disabled == 0 {
                scale_values_saturate(&mut o[2][..complex_hyb_bands], s);
                scale_values_saturate(&mut o[3][..complex_hyb_bands], s);
            }
        }

        self_.hyb_output_real_dry__fdk[row] = o[DR];
        self_.hyb_output_imag_dry__fdk[row] = o[DI];
        if tools_disabled == 0 {
            self_.hyb_output_real_wet__fdk[row] = o[2];
            self_.hyb_output_imag_wet__fdk[row] = o[3];
        }
    }

    err
}

/// `SpatialDecSynthesis(self, ts, hybOutputReal, hybOutputImag, timeOut, numInputChannels,
/// mapDescr)`: hybrid and QMF synthesis of slot `ts` of `self->hybOutput*Dry__FDK` into
/// `time_out` (or into the QMF work buffer of the SBR decoder for `stereoConfigIndex == 3`).
pub fn spatial_dec_synthesis(
    self_: &mut SpatialDec,
    qd: &mut FdkQmfDomain,
    cfg: &SpatialSpecificConfig,
    ts: i32,
    time_out: &mut [PcmMps],
    _num_input_channels: i32,
    map_descr: &FdkChannelMapDescr,
) -> SacdecError {
    let mut err = MPS_OK;

    let stride = self_.num_output_channels_at;
    let offset = 1;
    let base = stride as i64 * qd.global_conf.n_bands_synthesis as i64 * ts as i64;

    // C_ALLOC_SCRATCH_START(pQmfReal/pQmfImag, FIXP_DBL, QMF_MAX_SYNTHESIS_BANDS): the
    // synthesis reads what the hybrid synthesis (or the copy) wrote for consistent
    // configurations; other values read 0 in the port.
    let mut p_qmf_real = [0 as FixpDbl; QMF_MAX_SYNTHESIS_BANDS as usize];
    let mut p_qmf_imag = [0 as FixpDbl; QMF_MAX_SYNTHESIS_BANDS as usize];

    for ch in 0..(self_.num_output_channels_at.max(0) as usize).min(NUM_OUT_CH) {
        if cfg.syntax_flags & SACDEC_SYNTAX_LD != 0 {
            // No hybrid filtering. Just copy the QMF data.
            let n = (self_.hybrid_bands.max(0) as usize)
                .min(QMF_MAX_SYNTHESIS_BANDS as usize)
                .min(self_.hyb_output_real_dry__fdk[ch].len());
            p_qmf_real[..n].copy_from_slice(&self_.hyb_output_real_dry__fdk[ch][..n]);
            p_qmf_imag[..n].copy_from_slice(&self_.hyb_output_imag_dry__fdk[ch][..n]);
        } else {
            fdk_hybrid_synthesis_apply(
                &self_.hybrid_synthesis[ch],
                &self_.hyb_output_real_dry__fdk[ch],
                &self_.hyb_output_imag_dry__fdk[ch],
                &mut p_qmf_real,
                &mut p_qmf_imag,
            );
        }

        // Map channel indices from MPEG Surround -> PCE style -> channelMapping[]
        fdk_assert!(self_.num_output_channels_at <= 6);
        let out_ch = fdk_ch_map_descr_get_map_value(
            map_descr,
            map_channel(self_, ch as u32) as u8,
            self_.num_output_channels_at as u32,
        ) as usize;

        if out_ch >= QMF_DOMAIN_MAX_CHANNELS {
            // Deviation: guarded index of the QMF domain channels.
            continue;
        }

        if self_.stereo_config_index == 3 {
            // MPS -> SBR
            fdk_assert!(
                qd.qmf_domain_out[out_ch].fb.out_gain_m == 0x80000000u32 as FixpDbl
                    && qd.qmf_domain_out[out_ch].fb.out_gain_e == 0
            );
            let (p_work_buf_real, p_work_buf_imag) = fdk_qmf_domain_get_work_buffer(qd, out_ch, ts);
            fdk_assert!(self_.qmf_bands <= qd.qmf_domain_in[out_ch].work_buf_n_bands as i32);
            let n = (self_.qmf_bands.max(0) as usize).min(QMF_MAX_SYNTHESIS_BANDS as usize);
            {
                let d = qd.mem.slot_mut(p_work_buf_real);
                let m = n.min(d.len());
                d[..m].copy_from_slice(&p_qmf_real[..m]);
            }
            {
                let d = qd.mem.slot_mut(p_work_buf_imag);
                let m = n.min(d.len());
                d[..m].copy_from_slice(&p_qmf_imag[..m]);
            }
            let qin = &mut qd.qmf_domain_in[out_ch];
            qin.scaling.lb_scale = -7; // -ALGORITHMIC_SCALING_IN_ANALYSIS_FILTERBANK;
            qin.scaling.lb_scale = qin.scaling.lb_scale.wrapping_sub(qin.fb.filter_scale);
            qin.scaling.lb_scale =
                qin.scaling.lb_scale.wrapping_sub(self_.clip_protect_gain_sf__fdk as i32);

            qin.scaling.lb_scale = qin.scaling.lb_scale.wrapping_sub(1);
        } else {
            // Call the QMF synthesis for dry.
            let pos = base + (offset * out_ch) as i64;
            err = synthesis_into(
                &mut qd.qmf_domain_out[out_ch],
                &p_qmf_real,
                &p_qmf_imag,
                stride,
                time_out,
                pos,
            );
        }
        if err != MPS_OK {
            return err;
        }
    } // ch loop

    err
}

/// `CalculateSpaceSynthesisQmf(hQmfDomainOutCh, Sr, Si, stride, &timeOut[pos])`.
///
/// Deviation: output samples beyond `time_out` are dropped.
fn synthesis_into(
    qd_out: &mut FdkQmfDomainOut,
    sr: &[FixpDbl],
    si: &[FixpDbl],
    stride: i32,
    time_out: &mut [PcmMps],
    pos: i64,
) -> SacdecError {
    let n = qd_out.fb.no_channels.max(0) as i64;
    let stride_u = stride.max(1) as i64;
    let needed = if n > 0 { (n - 1) * stride_u + 1 } else { 0 };
    if pos >= 0 && pos + needed <= time_out.len() as i64 {
        calculate_space_synthesis_qmf(Some(qd_out), sr, si, stride, &mut time_out[pos as usize..])
    } else {
        let mut tmp = vec![0 as PcmMps; needed.max(0) as usize];
        // The synthesis writes every sample it produces, so the old content is not needed;
        // copy it anyway to keep the samples between the strides.
        for (i, t) in tmp.iter_mut().enumerate() {
            let p = pos + i as i64;
            if p >= 0 {
                if let Some(v) = time_out.get(p as usize) {
                    *t = *v;
                }
            }
        }
        let e = calculate_space_synthesis_qmf(Some(qd_out), sr, si, stride, &mut tmp);
        for (i, t) in tmp.iter().enumerate() {
            let p = pos + i as i64;
            if p >= 0 {
                if let Some(v) = time_out.get_mut(p as usize) {
                    *v = *t;
                }
            }
        }
        e
    }
}

/// `SpatialDecBufferMatrices(self)`: buffers the M2 matrices and the phases of the current
/// parameter set.
pub fn spatial_dec_buffer_matrices(self_: &mut SpatialDec) {
    let npb = (self_.num_parameter_bands.max(0) as usize).min(MAX_PARAMETER_BANDS);
    let complex_par_bands = npb;

    // buffer matrices M2
    for row in 0..(self_.num_m2rows.max(0) as usize).min(NUM_OUT_CH) {
        for col in 0..(self_.num_v_channels.max(0) as usize).min(NUM_V_CH) {
            let src = self_.m2_real__fdk[row][col];
            self_.m2_real_prev__fdk[row][col][..npb].copy_from_slice(&src[..npb]);
            if self_.phase_coding == 3 {
                let src = self_.m2_imag__fdk[row][col];
                self_.m2_imag_prev__fdk[row][col][..complex_par_bands]
                    .copy_from_slice(&src[..complex_par_bands]);
            }
        }
    }

    // buffer phase
    let src = self_.phase_left__fdk;
    self_.phase_prev_left__fdk[..npb].copy_from_slice(&src[..npb]);
    let src = self_.phase_right__fdk;
    self_.phase_prev_right__fdk[..npb].copy_from_slice(&src[..npb]);
}

/// `PHASE_SCALE`
const PHASE_SCALE: i32 = 2;

/// `interp_angle__FDK(angle1, angle2, alpha, pi_x2)`: for better precision, PI (`pi_x2`) is
/// already doubled.
fn interp_angle__fdk(
    mut angle1: FixpDbl,
    mut angle2: FixpDbl,
    alpha: FixpSgl,
    pi_x2: FixpDbl,
) -> FixpDbl {
    if angle2.wrapping_sub(angle1) > (pi_x2 >> 1) {
        angle2 = angle2.wrapping_sub(pi_x2);
    }

    if angle1.wrapping_sub(angle2) > (pi_x2 >> 1) {
        angle1 = angle1.wrapping_sub(pi_x2);
    }

    interpolate_parameter(alpha, angle2, angle1)
}

/// `SpatialDecApplyPhase(self, alpha__FDK, lastSlotOfParamSet)`
pub fn spatial_dec_apply_phase(
    self_: &mut SpatialDec,
    alpha__fdk: FixpSgl,
    _last_slot_of_param_set: i32,
) {
    // left real, imag - right real, imag interleaved (only the parameter bands below
    // numParameterBands are written and read)
    let mut ppb = [0 as FixpDbl; MAX_PARAMETER_BANDS * 4];
    let pi_x2 = PIX2__IPD;

    for pb in 0..(self_.num_parameter_bands.max(0) as usize).min(MAX_PARAMETER_BANDS) {
        let pl = interp_angle__fdk(
            self_.phase_prev_left__fdk[pb],
            self_.phase_left__fdk[pb],
            alpha__fdk,
            pi_x2,
        );
        let pr = interp_angle__fdk(
            self_.phase_prev_right__fdk[pb],
            self_.phase_right__fdk[pb],
            alpha__fdk,
            pi_x2,
        );
        let v = inline_fixp_cos_sin(pl, pr, IPD_SCALE);
        ppb[4 * pb..4 * pb + 4].copy_from_slice(&v);
    }

    let ppb_at = |pb: i32, k: usize| ppb.get(4 * (pb.max(0) as usize) + k).copied().unwrap_or(0);

    let [dry_real0, dry_real1] = &mut self_.hyb_output_real_dry__fdk;
    let [dry_imag0, dry_imag1] = &mut self_.hyb_output_imag_dry__fdk;
    let kernels = self_.kernels;
    let kern = |i: usize| kernels.and_then(|k| k.get(i)).map_or(0, |&v| v as i32);
    let mut q = 0usize;
    let mut step = |q: usize, pb: i32, neg: bool| {
        let (b1, b3) = if neg {
            (ppb_at(pb, 1).wrapping_neg(), ppb_at(pb, 3).wrapping_neg())
        } else {
            (ppb_at(pb, 1), ppb_at(pb, 3))
        };
        let (mut out_re, mut out_im) =
            cplx_mult_div2(dry_real0[q], dry_imag0[q], ppb_at(pb, 0), b1);
        out_re = out_re.wrapping_shl((PHASE_SCALE - 1) as u32);
        out_im = out_im.wrapping_shl((PHASE_SCALE - 1) as u32);
        dry_real0[q] = out_re;
        dry_imag0[q] = out_im;
        let (mut out_re, mut out_im) =
            cplx_mult_div2(dry_real1[q], dry_imag1[q], ppb_at(pb, 2), b3);
        out_re = out_re.wrapping_shl((PHASE_SCALE - 1) as u32);
        out_im = out_im.wrapping_shl((PHASE_SCALE - 1) as u32);
        dry_real1[q] = out_re;
        dry_imag1[q] = out_im;
    };

    // sign is -1 for qs = 0,2 and +1 for qs = 1
    for qs in (0..=2).rev() {
        let pb = kern(q);
        step(q, pb, qs != 1);
        q += 1;
    }

    // sign is +1 for qs >=3
    let n = self_.hybrid_bands - 3;
    for _ in 0..n.max(0) {
        if q >= NUM_HYB {
            break;
        }
        let pb = kern(q);
        step(q, pb, false);
        q += 1;
    }
}
