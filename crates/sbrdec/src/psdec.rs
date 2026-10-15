// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/psdec.h, libSBRdec/src/psdec.cpp.

//! Port of `psdec.h` and `psdec.cpp`: the parametric stereo decoder (baseline version: hybrid
//! filter bank with 20 stereo bands, no IPD/OPD synthesis).
//!
//! # Memory
//!
//! `struct PS_DEC` is allocated (zeroed) by `GetRam_ps_dec`; the port allocates
//! [`PsDec::default`]. Its hybrid analysis filter bank owns its state memory
//! (`pHybridAnaStatesLFdmx`) and its decorrelator its buffer (`decorrBufferCplx`), see
//! `fdk_aac_rs_fdk::hybrid` and `fdk_aac_rs_fdk::decorrelate`.
//!
//! `specificTo.mpeg.pCoef` points to scratch memory of `sbrDecoder_DecodeElement()` (a
//! `PS_DEC_COEFFICIENTS` on its stack); the functions that use it take it as a parameter
//! ([`PsDecCoefficients`]). `DecodePs()` and `initSlotBasedRotation()` write every value that is
//! read later in the same call of `sbrDecoder_DecodeElement()`, so no stale content is involved.

use crate::sbr_ram::pod_struct;
use crate::sbr_rom::*;
use crate::sbrdecoder::{SbrError, SBRDEC_NOT_INITIALIZED, SBRDEC_OK};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::decorrelate::*;
use fdk_aac_rs_fdk::hybrid::*;
use fdk_aac_rs_fdk::trig_fcts::inline_fixp_cos_sin;

pub mod tables;
use tables::GROUP_TABLE;

/// `SCAL_HEADROOM`
pub const SCAL_HEADROOM: i32 = 2;
/// `PS_EXTENSION_SIZE_BITS`
pub const PS_EXTENSION_SIZE_BITS: u32 = 4;
/// `PS_EXTENSION_ESC_COUNT_BITS`
pub const PS_EXTENSION_ESC_COUNT_BITS: u32 = 8;
/// `NO_QMF_CHANNELS`
pub const NO_QMF_CHANNELS: usize = 64;
/// `MAX_NUM_COL`
pub const MAX_NUM_COL: i32 = 32;
/// `NO_QMF_BANDS_HYBRID20`
pub const NO_QMF_BANDS_HYBRID20: usize = 3;
/// `NO_SUB_QMF_CHANNELS`
pub const NO_SUB_QMF_CHANNELS: usize = 12;
/// `HYBRID_FILTER_DELAY`
pub const HYBRID_FILTER_DELAY: usize = 6;
/// `MAX_NO_PS_ENV`: +1 needed for VAR_BORDER
pub const MAX_NO_PS_ENV: usize = 4 + 1;
/// `NO_HI_RES_BINS`
pub const NO_HI_RES_BINS: usize = 34;
/// `NO_MID_RES_BINS`
pub const NO_MID_RES_BINS: usize = 20;
/// `NO_LOW_RES_BINS`
pub const NO_LOW_RES_BINS: usize = 10;
/// `NO_HI_RES_IID_BINS`
pub const NO_HI_RES_IID_BINS: usize = NO_HI_RES_BINS;
/// `NO_HI_RES_ICC_BINS`
pub const NO_HI_RES_ICC_BINS: usize = NO_HI_RES_BINS;
/// `SUBQMF_GROUPS`
pub const SUBQMF_GROUPS: usize = 10;
/// `QMF_GROUPS`
pub const QMF_GROUPS: usize = 12;
/// `NO_IID_GROUPS`
pub const NO_IID_GROUPS: usize = SUBQMF_GROUPS + QMF_GROUPS;
/// `NO_IID_STEPS`: 1 .. + 7
pub const NO_IID_STEPS: i32 = 7;
/// `NO_IID_STEPS_FINE`: 1 .. +15
pub const NO_IID_STEPS_FINE: i32 = 15;
/// `NO_ICC_STEPS`: 0 .. + 7
pub const NO_ICC_STEPS: i32 = 8;
/// `NO_IID_LEVELS`: - 7 .. + 7
pub const NO_IID_LEVELS: usize = 2 * NO_IID_STEPS as usize + 1;
/// `NO_IID_LEVELS_FINE`: -15 .. +15
pub const NO_IID_LEVELS_FINE: usize = 2 * NO_IID_STEPS_FINE as usize + 1;
/// `NO_ICC_LEVELS`: 0 .. + 7
pub const NO_ICC_LEVELS: usize = NO_ICC_STEPS as usize;
/// `FIXP_SQRT05`: 1/SQRT2
pub const FIXP_SQRT05: FixpDbl = 0x5a827980;

/// `NO_HYBRID_DATA_BANDS`
const NO_HYBRID_DATA_BANDS: usize = 71;

pod_struct! {
    /// `PS_DEC_COEFFICIENTS`
    pub struct PsDecCoefficients {
        /// `H11r`: coefficients of the sub-subband groups
        pub h11r: [FixpDbl; NO_IID_GROUPS],
        /// `H12r`
        pub h12r: [FixpDbl; NO_IID_GROUPS],
        /// `H21r`
        pub h21r: [FixpDbl; NO_IID_GROUPS],
        /// `H22r`
        pub h22r: [FixpDbl; NO_IID_GROUPS],
        /// `DeltaH11r`
        pub delta_h11r: [FixpDbl; NO_IID_GROUPS],
        /// `DeltaH12r`
        pub delta_h12r: [FixpDbl; NO_IID_GROUPS],
        /// `DeltaH21r`
        pub delta_h21r: [FixpDbl; NO_IID_GROUPS],
        /// `DeltaH22r`
        pub delta_h22r: [FixpDbl; NO_IID_GROUPS],
        /// `aaIidIndexMapped`: the mapped IID index for all envelopes and all IID bins
        pub aa_iid_index_mapped: [[i8; NO_HI_RES_IID_BINS]; MAX_NO_PS_ENV],
        /// `aaIccIndexMapped`: the mapped ICC index for all envelopes and all ICC bins
        pub aa_icc_index_mapped: [[i8; NO_HI_RES_ICC_BINS]; MAX_NO_PS_ENV],
    }
}

/// `PS_PAYLOAD_TYPE`
pub type PsPayloadType = i32;
/// `ppt_none`
pub const PPT_NONE: PsPayloadType = 0;
/// `ppt_mpeg`
pub const PPT_MPEG: PsPayloadType = 1;
/// `ppt_drm`
pub const PPT_DRM: PsPayloadType = 2;

pod_struct! {
    /// `MPEG_PS_BS_DATA`
    pub struct MpegPsBsData {
        /// `bPsHeaderValid`: set if new header is available from bitstream
        pub b_ps_header_valid: u8,
        /// `bEnableIid`: one bit denoting the presence of IID parameters
        pub b_enable_iid: u8,
        /// `bEnableIcc`: one bit denoting the presence of ICC parameters
        pub b_enable_icc: u8,
        /// `bEnableExt`: the PS extension layer is enabled
        pub b_enable_ext: u8,
        /// `modeIid`: the configuration of IID parameters
        pub mode_iid: u8,
        /// `modeIcc`: the configuration of ICC parameters
        pub mode_icc: u8,
        /// `freqResIid`: 0=low, 1=mid or 2=high frequency resolution for iid
        pub freq_res_iid: u8,
        /// `freqResIcc`: 0=low, 1=mid or 2=high frequency resolution for icc
        pub freq_res_icc: u8,
        /// `bFineIidQ`: use fine Iid quantisation
        pub b_fine_iid_q: u8,
        /// `bFrameClass`: uniformly spaced or variable parameter positions
        pub b_frame_class: u8,
        /// `noEnv`: the number of envelopes per frame
        pub no_env: u8,
        /// `aEnvStartStop`: the parameter positions of variable parameter spacing
        pub a_env_start_stop: [u8; MAX_NO_PS_ENV + 1],
        /// `abIidDtFlag`: deltacoding time/freq flag for IID, 0 => freq
        pub ab_iid_dt_flag: [i8; MAX_NO_PS_ENV],
        /// `abIccDtFlag`: deltacoding time/freq flag for ICC, 0 => freq
        pub ab_icc_dt_flag: [i8; MAX_NO_PS_ENV],
        /// `aaIidIndex`: the IID index for all envelopes and all IID bins
        pub aa_iid_index: [[i8; NO_HI_RES_IID_BINS]; MAX_NO_PS_ENV],
        /// `aaIccIndex`: the ICC index for all envelopes and all ICC bins
        pub aa_icc_index: [[i8; NO_HI_RES_ICC_BINS]; MAX_NO_PS_ENV],
    }
}

/// `struct PS_DEC` (`HANDLE_PS_DEC` is `&mut PsDec`). The union `bsData[].mpeg` and the
/// `shouldBeUnion specificTo.mpeg` have one member each and are flattened.
#[derive(Clone, Debug)]
pub struct PsDec {
    /// `noSubSamples`
    pub no_sub_samples: i8,
    /// `noChannels`
    pub no_channels: i8,
    /// `procFrameBased`: helper to detect switching from frame based to slot based processing
    pub proc_frame_based: i8,
    /// `bPsDataAvail`: set if new data available from bitstream
    pub b_ps_data_avail: [PsPayloadType; 1 + 1],
    /// `psDecodedPrv`: set if PS has been processed in the last frame
    pub ps_decoded_prv: u8,
    /// `bsLastSlot`: index of last read slot
    pub bs_last_slot: u8,
    /// `bsReadSlot`: index of current read slot for additional delay
    pub bs_read_slot: u8,
    /// `processSlot`: index of current slot for processing
    pub process_slot: u8,
    /// `bsData[].mpeg`: all MPEG specific PS data from the bitstream
    pub bs_data: [MpegPsBsData; 1 + 1],
    /// `specificTo.mpeg.aIidPrevFrameIndex`: the IID index for previous frame
    pub a_iid_prev_frame_index: [i8; NO_HI_RES_IID_BINS],
    /// `specificTo.mpeg.aIccPrevFrameIndex`: the ICC index for previous frame
    pub a_icc_prev_frame_index: [i8; NO_HI_RES_ICC_BINS],
    /// `specificTo.mpeg.bPrevFrameFineIidQ`: the IID quantization of the previous frame
    pub b_prev_frame_fine_iid_q: u8,
    /// `specificTo.mpeg.prevFreqResIid`
    pub prev_freq_res_iid: u8,
    /// `specificTo.mpeg.prevFreqResIcc`
    pub prev_freq_res_icc: u8,
    /// `specificTo.mpeg.lastUsb`: uppermost WMF delay band of last frame
    pub last_usb: u8,
    /// `specificTo.mpeg.hybridAnalysis` (owns `pHybridAnaStatesLFdmx`)
    pub hybrid_analysis: FdkAnaHybFilter,
    /// `specificTo.mpeg.hybridSynthesis`
    pub hybrid_synthesis: [FdkSynHybFilter; 2],
    /// `specificTo.mpeg.apDecor`: decorrelator instance (owns `decorrBufferCplx`)
    pub ap_decor: DecorrDec,
    /// `specificTo.mpeg.h11rPrev`: previous calculated h(xy) coefficients
    pub h11r_prev: [FixpDbl; NO_IID_GROUPS],
    /// `specificTo.mpeg.h12rPrev`
    pub h12r_prev: [FixpDbl; NO_IID_GROUPS],
    /// `specificTo.mpeg.h21rPrev`
    pub h21r_prev: [FixpDbl; NO_IID_GROUPS],
    /// `specificTo.mpeg.h22rPrev`
    pub h22r_prev: [FixpDbl; NO_IID_GROUPS],
}

impl Default for PsDec {
    /// The zeroed structure of `GetRam_ps_dec()` (`FDKcalloc`).
    fn default() -> Self {
        PsDec {
            no_sub_samples: 0,
            no_channels: 0,
            proc_frame_based: 0,
            b_ps_data_avail: [PPT_NONE; 2],
            ps_decoded_prv: 0,
            bs_last_slot: 0,
            bs_read_slot: 0,
            process_slot: 0,
            bs_data: [MpegPsBsData::ZERO; 2],
            a_iid_prev_frame_index: [0; NO_HI_RES_IID_BINS],
            a_icc_prev_frame_index: [0; NO_HI_RES_ICC_BINS],
            b_prev_frame_fine_iid_q: 0,
            prev_freq_res_iid: 0,
            prev_freq_res_icc: 0,
            last_usb: 0,
            hybrid_analysis: FdkAnaHybFilter::default(),
            hybrid_synthesis: [FdkSynHybFilter::default(); 2],
            ap_decor: DecorrDec::default(),
            h11r_prev: [0; NO_IID_GROUPS],
            h12r_prev: [0; NO_IID_GROUPS],
            h21r_prev: [0; NO_IID_GROUPS],
            h22r_prev: [0; NO_IID_GROUPS],
        }
    }
}

/// Size of `pHybridAnaStatesLFdmx` in values (`2 * 13 * NO_QMF_BANDS_HYBRID20`).
const HYBRID_ANA_STATES_LF_SIZE: usize = 2 * 13 * NO_QMF_BANDS_HYBRID20;

/// `CreatePsDec(h_PS_DEC, aacSamplesPerFrame)`: creates one PS decoder instance, or resets the
/// open instance in `*h_ps_dec`. Returns 0 on success, -1 on error.
///
/// On error the instance is deleted and the port sets `*h_ps_dec` to `None` (PORTING.md,
/// "Deviations from the reference").
pub fn create_ps_dec(h_ps_dec: &mut Option<Box<PsDec>>, aac_samples_per_frame: i32) -> i32 {
    // Get ps dec ram (zeroed), or reset an open instance.
    let mut h_ps_d = h_ps_dec.take().unwrap_or_default();

    // Create Analysis Hybrid filterbank.
    let lf_mem = core::mem::take(&mut h_ps_d.hybrid_analysis.p_lf_memory);
    let lf_mem = if lf_mem.len() == HYBRID_ANA_STATES_LF_SIZE {
        lf_mem
    } else {
        vec![0; HYBRID_ANA_STATES_LF_SIZE]
    };
    fdk_hybrid_analysis_open(
        &mut h_ps_d.hybrid_analysis,
        lf_mem,
        (HYBRID_ANA_STATES_LF_SIZE * 4) as u32,
        Vec::new(),
        0,
    );

    // initialisation
    h_ps_d.no_sub_samples = match aac_samples_per_frame {
        960 => 30,  // col
        1024 => 32, // col
        _ => -1,
    };

    if h_ps_d.no_sub_samples as i32 > MAX_NUM_COL || h_ps_d.no_sub_samples <= 0 {
        delete_ps_dec(&mut Some(h_ps_d));
        return -1;
    }
    h_ps_d.no_channels = NO_QMF_CHANNELS as i8; // row

    h_ps_d.ps_decoded_prv = 0;
    h_ps_d.proc_frame_based = -1;
    for i in 0..1 + 1 {
        h_ps_d.b_ps_data_avail[i] = PPT_NONE;
    }
    {
        let buf = h_ps_d
            .ap_decor
            .state_buffer_cplx
            .take()
            .filter(|b| b.len() == DECORR_BUFFER_LEN as usize)
            .unwrap_or_else(|| vec![0; DECORR_BUFFER_LEN as usize].into_boxed_slice());
        let error = fdk_decorrelate_open(&mut h_ps_d.ap_decor, buf);
        if error != 0 {
            delete_ps_dec(&mut Some(h_ps_d));
            return -1;
        }
    }

    for i in 0..1 + 1 {
        h_ps_d.bs_data[i] = MpegPsBsData::ZERO;
    }

    let error_info = reset_ps_dec(&mut h_ps_d);

    if error_info != SBRDEC_OK {
        delete_ps_dec(&mut Some(h_ps_d));
        return -1;
    }

    *h_ps_dec = Some(h_ps_d);

    0
}

/// `DeletePsDec(h_PS_DEC)`: deletes the instance. Returns -1 if there is none.
pub fn delete_ps_dec(h_ps_dec: &mut Option<Box<PsDec>>) -> i32 {
    let Some(h_ps_d) = h_ps_dec.as_mut() else {
        return -1;
    };

    fdk_decorrelate_close(&mut h_ps_d.ap_decor);

    *h_ps_dec = None;

    0
}

/// `ResetPsDec(h_ps_d)`: resets some values of the PS handle to default states.
pub fn reset_ps_dec(h_ps_d: &mut PsDec) -> SbrError {
    let error_info = SBRDEC_OK;

    // explicitly init state variables to safe values (until first ps header arrives)
    h_ps_d.last_usb = 0;

    // Initialize Analysis Hybrid filterbank.
    fdk_hybrid_analysis_init(
        &mut h_ps_d.hybrid_analysis,
        THREE_TO_TEN,
        NO_QMF_BANDS_HYBRID20 as i32,
        NO_QMF_BANDS_HYBRID20 as i32,
        1,
    );

    // Initialize Synthesis Hybrid filterbank.
    for i in 0..2 {
        fdk_hybrid_synthesis_init(
            &mut h_ps_d.hybrid_synthesis[i],
            THREE_TO_TEN,
            NO_QMF_CHANNELS as i32,
            NO_QMF_CHANNELS as i32,
        );
    }
    {
        let error = fdk_decorrelate_init(
            &mut h_ps_d.ap_decor,
            71,
            DECORR_PS,
            DUCKER_AUTOMATIC,
            0,
            0,
            0,
            0,
            1, // isLegacyPS
            1,
        );
        if error != 0 {
            return SBRDEC_NOT_INITIALIZED;
        }
    }

    for i in 0..NO_IID_GROUPS {
        h_ps_d.h11r_prev[i] = fl2fxconst_dbl(0.5f32 as f64);
        h_ps_d.h12r_prev[i] = fl2fxconst_dbl(0.5f32 as f64);
    }

    h_ps_d.h21r_prev.fill(0);
    h_ps_d.h22r_prev.fill(0);

    error_info
}

/// `PreparePsProcessing()`: feeds the delay lines when parametric stereo is switched on (from
/// frame based to slot based processing).
pub fn prepare_ps_processing(
    h_ps_d: &mut PsDec,
    r_int_buffer_left: &[&mut [FixpDbl]],
    i_int_buffer_left: &[&mut [FixpDbl]],
    scale_factor_low_band: i32,
) {
    if h_ps_d.proc_frame_based == 1 {
        // If we have switched from frame to slot based processing fill hybrid delay buffer.
        for i in 0..HYBRID_FILTER_DELAY {
            let mut qmf_input_data = [[0 as FixpDbl; NO_QMF_BANDS_HYBRID20]; 2];
            let mut hybrid_output_data = [[0 as FixpDbl; NO_SUB_QMF_CHANNELS]; 2];

            for j in 0..NO_QMF_BANDS_HYBRID20 {
                qmf_input_data[0][j] = scale_value(r_int_buffer_left[i][j], scale_factor_low_band);
                qmf_input_data[1][j] = scale_value(i_int_buffer_left[i][j], scale_factor_low_band);
            }

            let [out_re, out_im] = &mut hybrid_output_data;
            fdk_hybrid_analysis_apply(
                &mut h_ps_d.hybrid_analysis,
                &qmf_input_data[0],
                &qmf_input_data[1],
                out_re,
                out_im,
            );
        }
        h_ps_d.proc_frame_based = 0; // switch to slot based processing.
    }
}

/// `initSlotBasedRotation(h_ps_d, env, usb)`: dequantizes the IID and ICC parameters of
/// envelope `env` and prepares the interpolation of the mixing matrix.
pub fn init_slot_based_rotation(
    h_ps_d: &mut PsDec,
    p_coef: &mut PsDecCoefficients,
    env: i32,
    _usb: i32,
) {
    let env = env as usize;
    let bs = &h_ps_d.bs_data[h_ps_d.process_slot as usize];
    let (p_scale_factors, no_iid_steps): (&[FixpDbl], i32) = if bs.b_fine_iid_q != 0 {
        (&SCALE_FACTORS_FINE, NO_IID_STEPS_FINE) // values are shiftet right by one
    } else {
        (&SCALE_FACTORS, NO_IID_STEPS) // values are shiftet right by one
    };

    // dequantize and decode
    for group in 0..NO_IID_GROUPS {
        let bin = BINS2GROUP_MAP20[group] as usize;

        // type 'A' rotation: mixing procedure R_a, used in baseline version.
        // ScaleR and ScaleL are scaled by 1 shift right.
        let iid = p_coef.aa_iid_index_mapped[env.min(MAX_NO_PS_ENV - 1)][bin] as i32;
        let icc = p_coef.aa_icc_index_mapped[env.min(MAX_NO_PS_ENV - 1)][bin] as i32;
        // The indices are limited to -noIidSteps..=noIidSteps and 0..=7 by DecodePs(); a value
        // outside reads 0 in the port.
        let sf = |i: i32| p_scale_factors.get(i as usize).copied().unwrap_or(0);
        let alpha_at = |i: i32| ALPHAS.get(i as usize).copied().unwrap_or(0);
        let scale_r = sf(no_iid_steps + iid);
        let scale_l = sf(no_iid_steps - iid);

        let beta = f_mult(f_mult(alpha_at(icc), scale_r.wrapping_sub(scale_l)), FIXP_SQRT05);
        let alpha = alpha_at(icc) >> 1;

        // Alpha and Beta are now both scaled by 2 shifts right

        // calculate the coefficients h11... h22 from scale-factors and ICC parameters
        // h values are scaled by 1 shift right
        let trig_data = inline_fixp_cos_sin(beta.wrapping_add(alpha), beta.wrapping_sub(alpha), 2);
        let h11r = f_mult(scale_l, trig_data[0]);
        let h12r = f_mult(scale_r, trig_data[2]);
        let h21r = f_mult(scale_l, trig_data[1]);
        let h22r = f_mult(scale_r, trig_data[3]);

        // Interpolation of the matrices H11... H22:
        // H11(k,n) = H11(k,n[e]) + (n-n[e]) * (H11(k,n[e+1] - H11(k,n[e])) / (n[e+1] - n[e])

        // invL = 1/(length of envelope)
        let ess = &bs.a_env_start_stop;
        let len = ess.get(env + 1).copied().unwrap_or(0) as i32
            - ess.get(env).copied().unwrap_or(0) as i32;
        let inv_l = fx_dbl2fx_sgl(get_inv_int(len));

        p_coef.h11r[group] = h_ps_d.h11r_prev[group];
        p_coef.h12r[group] = h_ps_d.h12r_prev[group];
        p_coef.h21r[group] = h_ps_d.h21r_prev[group];
        p_coef.h22r[group] = h_ps_d.h22r_prev[group];

        p_coef.delta_h11r[group] = f_mult(h11r.wrapping_sub(p_coef.h11r[group]), inv_l);
        p_coef.delta_h12r[group] = f_mult(h12r.wrapping_sub(p_coef.h12r[group]), inv_l);
        p_coef.delta_h21r[group] = f_mult(h21r.wrapping_sub(p_coef.h21r[group]), inv_l);
        p_coef.delta_h22r[group] = f_mult(h22r.wrapping_sub(p_coef.h22r[group]), inv_l);

        // update prev coefficients for interpolation in next envelope
        h_ps_d.h11r_prev[group] = h11r;
        h_ps_d.h12r_prev[group] = h12r;
        h_ps_d.h21r_prev[group] = h21r;
        h_ps_d.h22r_prev[group] = h22r;
    }
}

/// `applySlotBasedRotation()`: constructs the stereo sub subband signals
/// `l_k(n) = H11(k,n) s_k(n) + H21(k,n) d_k(n)`, `r_k(n) = H12(k,n) s_k(n) + H22(k,n) d_k(n)`
/// with interpolated coefficients.
///
/// The straight port, kept as the reference of the optimized [`apply_slot_based_rotation`].
pub fn apply_slot_based_rotation_straight(
    p_coef: &mut PsDecCoefficients,
    m_hybrid_real_left: &mut [FixpDbl],
    m_hybrid_imag_left: &mut [FixpDbl],
    m_hybrid_real_right: &mut [FixpDbl],
    m_hybrid_imag_right: &mut [FixpDbl],
) {
    for group in 0..NO_IID_GROUPS {
        p_coef.h11r[group] = p_coef.h11r[group].wrapping_add(p_coef.delta_h11r[group]);
        p_coef.h12r[group] = p_coef.h12r[group].wrapping_add(p_coef.delta_h12r[group]);
        p_coef.h21r[group] = p_coef.h21r[group].wrapping_add(p_coef.delta_h21r[group]);
        p_coef.h22r[group] = p_coef.h22r[group].wrapping_add(p_coef.delta_h22r[group]);

        let start = GROUP_TABLE[group] as usize;
        let stop = GROUP_TABLE[group + 1] as usize;
        for subband in start..stop {
            let tmp_left = f_mult_add(
                f_mult_div2(p_coef.h11r[group], m_hybrid_real_left[subband]),
                p_coef.h21r[group],
                m_hybrid_real_right[subband],
            );
            let tmp_right = f_mult_add(
                f_mult_div2(p_coef.h12r[group], m_hybrid_real_left[subband]),
                p_coef.h22r[group],
                m_hybrid_real_right[subband],
            );
            m_hybrid_real_left[subband] = tmp_left;
            m_hybrid_real_right[subband] = tmp_right;

            let tmp_left = f_mult_add(
                f_mult_div2(p_coef.h11r[group], m_hybrid_imag_left[subband]),
                p_coef.h21r[group],
                m_hybrid_imag_right[subband],
            );
            let tmp_right = f_mult_add(
                f_mult_div2(p_coef.h12r[group], m_hybrid_imag_left[subband]),
                p_coef.h22r[group],
                m_hybrid_imag_right[subband],
            );
            m_hybrid_imag_left[subband] = tmp_left;
            m_hybrid_imag_right[subband] = tmp_right;
        }
    }
}

/// The number of groups of `GROUP_TABLE` with one band each at the start (band = group).
const SINGLE_BAND_GROUPS: usize = 16;
const _: () = {
    let mut g = 0;
    while g <= SINGLE_BAND_GROUPS {
        assert!(GROUP_TABLE[g] as usize == g);
        g += 1;
    }
    assert!(GROUP_TABLE[NO_IID_GROUPS] as usize == NO_HYBRID_DATA_BANDS);
};

/// `applySlotBasedRotation()`, optimized (see [`apply_slot_based_rotation_straight`]): the
/// coefficients of all groups are interpolated first (a group's coefficients do not depend on
/// the data), then the bands are rotated independently (every band is read before it is
/// written; the real and imaginary parts and the two channels are separate arrays). On aarch64
/// four bands are rotated at a time with NEON: the single-band groups with the coefficients of
/// four groups, the wider groups with their coefficients in every lane.
///
/// `fMultDiv2(FIXP_DBL, FIXP_DBL)` and `fMultAdd(x, a, b) = (x + fMultDiv2(a, b)) << 1` compute
/// the same values on every variant, so the NEON code (`smull`/`smull2`, the high halves with
/// `uzp2`) serves all of them.
pub fn apply_slot_based_rotation(
    p_coef: &mut PsDecCoefficients,
    m_hybrid_real_left: &mut [FixpDbl],
    m_hybrid_imag_left: &mut [FixpDbl],
    m_hybrid_real_right: &mut [FixpDbl],
    m_hybrid_imag_right: &mut [FixpDbl],
) {
    const N: usize = NO_HYBRID_DATA_BANDS;
    let c = p_coef;
    for g in 0..NO_IID_GROUPS {
        c.h11r[g] = c.h11r[g].wrapping_add(c.delta_h11r[g]);
        c.h12r[g] = c.h12r[g].wrapping_add(c.delta_h12r[g]);
        c.h21r[g] = c.h21r[g].wrapping_add(c.delta_h21r[g]);
        c.h22r[g] = c.h22r[g].wrapping_add(c.delta_h22r[g]);
    }
    let lr = &mut m_hybrid_real_left[..N];
    let li = &mut m_hybrid_imag_left[..N];
    let rr = &mut m_hybrid_real_right[..N];
    let ri = &mut m_hybrid_imag_right[..N];

    #[inline(always)]
    fn rot1(l: &mut FixpDbl, r: &mut FixpDbl, h: [FixpDbl; 4]) {
        let (lv, rv) = (*l, *r);
        *l = f_mult_add(f_mult_div2(h[0], lv), h[2], rv);
        *r = f_mult_add(f_mult_div2(h[1], lv), h[3], rv);
    }

    #[cfg(target_arch = "aarch64")]
    {
        use core::arch::aarch64::*;
        #[inline(always)]
        unsafe fn mdiv2(a: int32x4_t, x: int32x4_t) -> int32x4_t {
            let lo = vreinterpretq_s32_s64(vmull_s32(vget_low_s32(a), vget_low_s32(x)));
            let hi = vreinterpretq_s32_s64(vmull_high_s32(a, x));
            vuzp2q_s32(lo, hi)
        }
        /// Bands b..b + 4 of one channel pair (b + 4 <= N).
        #[inline(always)]
        unsafe fn rot4(l: &mut [FixpDbl], r: &mut [FixpDbl], b: usize, h: [int32x4_t; 4]) {
            debug_assert!(b + 4 <= l.len() && b + 4 <= r.len());
            let (pl, pr) = (l.as_mut_ptr().add(b), r.as_mut_ptr().add(b));
            let (lv, rv) = (vld1q_s32(pl), vld1q_s32(pr));
            let nl = vaddq_s32(mdiv2(h[0], lv), mdiv2(h[2], rv));
            let nr = vaddq_s32(mdiv2(h[1], lv), mdiv2(h[3], rv));
            vst1q_s32(pl, vshlq_n_s32::<1>(nl));
            vst1q_s32(pr, vshlq_n_s32::<1>(nr));
        }
        // SAFETY: NEON is part of aarch64; the loads and stores cover [b, b + 4) with b + 4 <= N
        // for the data (N values each) and b + 4 <= SINGLE_BAND_GROUPS for the coefficients.
        unsafe {
            for b in (0..SINGLE_BAND_GROUPS).step_by(4) {
                let h = [
                    vld1q_s32(c.h11r.as_ptr().add(b)),
                    vld1q_s32(c.h12r.as_ptr().add(b)),
                    vld1q_s32(c.h21r.as_ptr().add(b)),
                    vld1q_s32(c.h22r.as_ptr().add(b)),
                ];
                rot4(lr, rr, b, h);
                rot4(li, ri, b, h);
            }
            for g in SINGLE_BAND_GROUPS..NO_IID_GROUPS {
                let hs = [c.h11r[g], c.h12r[g], c.h21r[g], c.h22r[g]];
                let h = hs.map(|v| vdupq_n_s32(v));
                let (start, stop) = (GROUP_TABLE[g] as usize, GROUP_TABLE[g + 1] as usize);
                let mut b = start;
                while b + 4 <= stop {
                    rot4(lr, rr, b, h);
                    rot4(li, ri, b, h);
                    b += 4;
                }
                for b in b..stop {
                    rot1(&mut lr[b], &mut rr[b], hs);
                    rot1(&mut li[b], &mut ri[b], hs);
                }
            }
        }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if fdk_aac_rs_fdk::simd_x86::avx2() {
        // SAFETY: the CPU has AVX2.
        unsafe { x86_rot::rotate(c, lr, li, rr, ri) };
        return;
    }
    #[cfg(not(target_arch = "aarch64"))]
    for g in 0..NO_IID_GROUPS {
        let hs = [c.h11r[g], c.h12r[g], c.h21r[g], c.h22r[g]];
        for b in GROUP_TABLE[g] as usize..GROUP_TABLE[g + 1] as usize {
            rot1(&mut lr[b], &mut rr[b], hs);
            rot1(&mut li[b], &mut ri[b], hs);
        }
    }
}

/// The AVX2 form of the rotation of [`apply_slot_based_rotation`] (x86 hosts, every variant):
/// eight bands at a time, `fMultDiv2(FIXP_DBL, FIXP_DBL)` as the high halves of `vpmuldq` on
/// the even and odd lanes, `fMultAdd(fMultDiv2(a, l), b, r) = (fMultDiv2(a, l) +
/// fMultDiv2(b, r)) << 1` (wrapping).
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86_rot {
    use super::*;
    #[cfg(target_arch = "x86")]
    use core::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;

    #[inline(always)]
    unsafe fn mdiv2(a: __m256i, x: __m256i) -> __m256i {
        let even = _mm256_srli_epi64::<32>(_mm256_mul_epi32(a, x));
        let odd = _mm256_mul_epi32(_mm256_srli_epi64::<32>(a), _mm256_srli_epi64::<32>(x));
        _mm256_blend_epi32::<0b1010_1010>(even, odd)
    }

    /// Bands b..b + 8 of one channel pair (b + 8 <= N).
    #[inline(always)]
    unsafe fn rot8(l: &mut [FixpDbl], r: &mut [FixpDbl], b: usize, h: [__m256i; 4]) {
        assert!(b + 8 <= l.len() && b + 8 <= r.len());
        let (pl, pr) =
            (l.as_mut_ptr().add(b) as *mut __m256i, r.as_mut_ptr().add(b) as *mut __m256i);
        let (lv, rv) = (_mm256_loadu_si256(pl), _mm256_loadu_si256(pr));
        let nl = _mm256_add_epi32(mdiv2(h[0], lv), mdiv2(h[2], rv));
        let nr = _mm256_add_epi32(mdiv2(h[1], lv), mdiv2(h[3], rv));
        _mm256_storeu_si256(pl, _mm256_slli_epi32::<1>(nl));
        _mm256_storeu_si256(pr, _mm256_slli_epi32::<1>(nr));
    }

    /// # Safety
    /// The CPU must have AVX2.
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn rotate(
        c: &PsDecCoefficients,
        lr: &mut [FixpDbl],
        li: &mut [FixpDbl],
        rr: &mut [FixpDbl],
        ri: &mut [FixpDbl],
    ) {
        const _: () = assert!(SINGLE_BAND_GROUPS % 8 == 0);
        let ld = |v: &[FixpDbl], b: usize| {
            assert!(b + 8 <= v.len());
            _mm256_loadu_si256(v.as_ptr().add(b) as *const __m256i)
        };
        for b in (0..SINGLE_BAND_GROUPS).step_by(8) {
            let h = [ld(&c.h11r, b), ld(&c.h12r, b), ld(&c.h21r, b), ld(&c.h22r, b)];
            rot8(lr, rr, b, h);
            rot8(li, ri, b, h);
        }
        for g in SINGLE_BAND_GROUPS..NO_IID_GROUPS {
            let hs = [c.h11r[g], c.h12r[g], c.h21r[g], c.h22r[g]];
            let h = hs.map(|v| _mm256_set1_epi32(v));
            let (start, stop) = (GROUP_TABLE[g] as usize, GROUP_TABLE[g + 1] as usize);
            let mut b = start;
            while b + 8 <= stop {
                rot8(lr, rr, b, h);
                rot8(li, ri, b, h);
                b += 8;
            }
            for b in b..stop {
                let (l, r) = (lr[b], rr[b]);
                lr[b] = f_mult_add(f_mult_div2(hs[0], l), hs[2], r);
                rr[b] = f_mult_add(f_mult_div2(hs[1], l), hs[3], r);
                let (l, r) = (li[b], ri[b]);
                li[b] = f_mult_add(f_mult_div2(hs[0], l), hs[2], r);
                ri[b] = f_mult_add(f_mult_div2(hs[1], l), hs[3], r);
            }
        }
    }
}

/// `scaleValues(dst, src, len, scalefactor)` for a C length that may be negative (no-op).
fn scale_values_len(dst: &mut [FixpDbl], src: &[FixpDbl], len: i32, sf: i32) {
    if len <= 0 {
        return;
    }
    let n = (len as usize).min(dst.len()).min(src.len());
    scale_values_to(&mut dst[..n], &src[..n], sf);
}

/// `ApplyPsSlot()`: applies the IID and ICC parameters to one QMF slot.
///
/// `r_int_buffer_left`/`i_int_buffer_left` are the slot arrays starting at the current slot
/// (`pLowBandReal + i`): the hybrid analysis reads slot `HYBRID_FILTER_DELAY`, the output is
/// written to slot 0. The right channel is written to `r_int_buffer_right`/`i_int_buffer_right`
/// (one slot of 64 values each).
#[allow(clippy::too_many_arguments)]
pub fn apply_ps_slot(
    h_ps_d: &mut PsDec,
    p_coef: &mut PsDecCoefficients,
    r_int_buffer_left: &mut [&mut [FixpDbl]],
    i_int_buffer_left: &mut [&mut [FixpDbl]],
    r_int_buffer_right: &mut [FixpDbl],
    i_int_buffer_right: &mut [FixpDbl],
    scale_factor_low_band_no_ov: i32,
    scale_factor_low_band: i32,
    scale_factor_high_band: i32,
    lsb: i32,
    usb: i32,
) {
    let mut qmf_input_data = [[0 as FixpDbl; NO_QMF_BANDS_HYBRID20]; 2];
    // C_ALLOC_SCRATCH_START(pHybridData, FIXP_DBL, 4 * NO_HYBRID_DATA_BANDS): every value of the
    // left channel is written below before it is read, the right channel is written by the
    // decorrelator.
    let mut p_hybrid_data = [[0 as FixpDbl; NO_HYBRID_DATA_BANDS]; 4];

    // Hybrid analysis.

    // Get qmf input data and apply descaling
    for i in 0..NO_QMF_BANDS_HYBRID20 {
        qmf_input_data[0][i] =
            scale_value(r_int_buffer_left[HYBRID_FILTER_DELAY][i], scale_factor_low_band_no_ov);
        qmf_input_data[1][i] =
            scale_value(i_int_buffer_left[HYBRID_FILTER_DELAY][i], scale_factor_low_band_no_ov);
    }

    let [left_re, left_im, right_re, right_im] = &mut p_hybrid_data;

    // LF - part
    fdk_hybrid_analysis_apply(
        &mut h_ps_d.hybrid_analysis,
        &qmf_input_data[0],
        &qmf_input_data[1],
        left_re,
        left_im,
    );

    // HF - part
    // bands up to lsb
    let off = NO_SUB_QMF_CHANNELS - 2;
    let n = lsb - NO_QMF_BANDS_HYBRID20 as i32;
    scale_values_len(
        &mut left_re[off..],
        &r_int_buffer_left[0][NO_QMF_BANDS_HYBRID20..],
        n,
        scale_factor_low_band,
    );
    scale_values_len(
        &mut left_im[off..],
        &i_int_buffer_left[0][NO_QMF_BANDS_HYBRID20..],
        n,
        scale_factor_low_band,
    );

    // bands from lsb to usb
    let hoff = NO_SUB_QMF_CHANNELS - 2 - NO_QMF_BANDS_HYBRID20;
    if let (Some(dst_re), Some(src_re), Some(dst_im), Some(src_im)) = (
        left_re.get_mut((lsb.max(0) as usize) + hoff..),
        r_int_buffer_left[0].get(lsb.max(0) as usize..),
        left_im.get_mut((lsb.max(0) as usize) + hoff..),
        i_int_buffer_left[0].get(lsb.max(0) as usize..),
    ) {
        scale_values_len(dst_re, src_re, usb - lsb, scale_factor_high_band);
        scale_values_len(dst_im, src_im, usb - lsb, scale_factor_high_band);
    }

    // bands from usb to NO_SUB_QMF_CHANNELS which should be zero for non-overlap slots but can
    // be non-zero for overlap slots
    let u = usb.clamp(0, NO_QMF_CHANNELS as i32) as usize;
    left_re[u + hoff..u + hoff + (NO_QMF_CHANNELS - u)]
        .copy_from_slice(&r_int_buffer_left[0][u..NO_QMF_CHANNELS]);
    left_im[u + hoff..u + hoff + (NO_QMF_CHANNELS - u)]
        .copy_from_slice(&i_int_buffer_left[0][u..NO_QMF_CHANNELS]);

    // Decorrelation: by means of all-pass filtering and delaying, the (sub-)sub-band samples
    // s_k(n) are converted into de-correlated (sub-)sub-band samples d_k(n).
    fdk_decorrelate_apply(&mut h_ps_d.ap_decor, left_re, left_im, right_re, right_im, 0);

    // Stereo Processing: the sets of (sub-)sub-band samples s_k(n) and d_k(n) are processed
    // according to the stereo cues which are defined per stereo band.
    apply_slot_based_rotation(p_coef, left_re, left_im, right_re, right_im);

    // Hybrid synthesis.
    fdk_hybrid_synthesis_apply(
        &h_ps_d.hybrid_synthesis[0],
        left_re,
        left_im,
        r_int_buffer_left[0],
        i_int_buffer_left[0],
    );
    fdk_hybrid_synthesis_apply(
        &h_ps_d.hybrid_synthesis[1],
        right_re,
        right_im,
        r_int_buffer_right,
        i_int_buffer_right,
    );
}
