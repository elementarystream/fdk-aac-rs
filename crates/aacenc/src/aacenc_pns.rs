// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/aacenc_pns.h, libAACenc/src/aacenc_pns.cpp.

//! Port of `aacenc_pns.h` and `aacenc_pns.cpp`: perceptual noise substitution.
//!
//! The functions are declared in `pns_func.h`; [`crate::pns_func`] re-exports them.

use crate::aacenc::{AacEncoderError, AAC_ENC_OK};
use crate::bit_cnt::CODE_BOOK_PNS_LAV;
use crate::interface::MS_SOME;
use crate::noisedet::fdk_aac_enc_noise_detect;
use crate::pnsparam::{
    fdk_aac_enc_get_pns_param, NoiseParams, IS_LOW_COMPLEXITY, JUST_LONG_WINDOW, USE_TNS_GAIN_THR,
    USE_TNS_PNS,
};
use crate::psy_const::{LOG_NORM_PCM, LONG_WINDOW, MAX_GROUPED_SFB, SHORT_WINDOW, TRUE};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fixpoint_math::{LD_COEFF, LD_DATA_SCALING, LD_DATA_SHIFT, LD_PRECISION};
use fdk_aac_rs_fdk::generic_stds::FDK_INT_MIN;

/// `NO_NOISE_PNS`
pub const NO_NOISE_PNS: i32 = FDK_INT_MIN;

/// `minCorrelationEnergy`: `(1.0e-10f)^2 ~ 2^-67 = 2^-47 * 2^-20`
/// (`FL2FXCONST_DBL(0.0)`, formerly `FL2FXCONST_DBL((float)FDKpow(2.0,-47))`)
const MIN_CORRELATION_ENERGY: FixpDbl = fl2fxconst_dbl(0.0);
/// `noiseCorrelationThresh = 0.6^2` (`FL2FXCONST_DBL(0.36)`)
const NOISE_CORRELATION_THRESH: FixpDbl = fl2fxconst_dbl(0.36);

/// `PNS_CONFIG` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PnsConfig {
    pub np: NoiseParams,
    pub min_correlation_energy: FixpDbl,
    pub noise_correlation_thresh: FixpDbl,
    pub use_pns: i32,
}

/// `PNS_DATA` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PnsData {
    pub noise_fuzzy_measure: [FixpSgl; MAX_GROUPED_SFB],
    pub noise_energy_correlation: [FixpDbl; MAX_GROUPED_SFB],
    pub pns_flag: [i32; MAX_GROUPED_SFB],
}

impl Default for PnsData {
    fn default() -> Self {
        PnsData {
            noise_fuzzy_measure: [0; MAX_GROUPED_SFB],
            noise_energy_correlation: [0; MAX_GROUPED_SFB],
            pns_flag: [0; MAX_GROUPED_SFB],
        }
    }
}

impl PnsData {
    /// `pnsFlag[i]` read through the C layout: `pnsFlag[-1]` is the word before the array,
    /// `noiseEnergyCorrelation[MAX_GROUPED_SFB - 1]` (reproduced from the C layout). Indices past
    /// the end of the structure read 0.
    fn pns_flag_at(&self, i: i32) -> i32 {
        if (0..MAX_GROUPED_SFB as i32).contains(&i) {
            self.pns_flag[i as usize]
        } else if i == -1 {
            self.noise_energy_correlation[MAX_GROUPED_SFB - 1]
        } else {
            0
        }
    }

    /// Writes `pnsFlag[i]`; writes outside the array are dropped.
    fn set_pns_flag_at(&mut self, i: i32, v: i32) {
        if (0..MAX_GROUPED_SFB as i32).contains(&i) {
            self.pns_flag[i as usize] = v;
        }
    }
}

/// `FDKaacEnc_InitPnsConfiguration(pnsConf, bitRate, sampleRate, usePns, sfbCnt, sfbOffset,
/// numChan, isLC)`: fill pnsConf with pns parameters.
pub fn fdk_aac_enc_init_pns_configuration(
    pns_conf: &mut PnsConfig,
    bit_rate: i32,
    sample_rate: i32,
    mut use_pns: i32,
    sfb_cnt: i32,
    sfb_offset: &[i32],
    num_chan: i32,
    is_lc: i32,
) -> AacEncoderError {
    // init noise detection
    let error_status = fdk_aac_enc_get_pns_param(
        &mut pns_conf.np,
        bit_rate,
        sample_rate,
        sfb_cnt,
        sfb_offset,
        &mut use_pns,
        num_chan,
        is_lc,
    );
    if error_status != AAC_ENC_OK {
        return error_status;
    }

    pns_conf.min_correlation_energy = MIN_CORRELATION_ENERGY;
    pns_conf.noise_correlation_thresh = NOISE_CORRELATION_THRESH;

    pns_conf.use_pns = use_pns;

    AAC_ENC_OK
}

/// `FDKaacEnc_PnsDetect(pnsConf, pnsData, lastWindowSequence, sfbActive, maxSfbPerGroup,
/// sfbThresholdLdData, sfbOffset, mdctSpectrum, sfbMaxScaleSpec, sfbtonality, tnsOrder,
/// tnsPredictionGain, tnsActive, sfbEnergyLdData, noiseNrg)`: do decision, if PNS shall used or
/// not.
///
/// Output: `pnsFlag` in `pns_data`, `noiseNrg[0..MAX_GROUPED_SFB]`.
///
/// `pnsFlag[-1]` is the last word of `noiseEnergyCorrelation` in the same `PNS_DATA`, reproduced
/// from the C layout (see `PnsData::pns_flag_at`).
pub fn fdk_aac_enc_pns_detect(
    pns_conf: &PnsConfig,
    pns_data: &mut PnsData,
    last_window_sequence: i32,
    sfb_active: i32,
    max_sfb_per_group: i32,
    sfb_threshold_ld_data: &[FixpDbl],
    sfb_offset: &[i32],
    mdct_spectrum: &[FixpDbl],
    sfb_max_scale_spec: &[i32],
    sfbtonality: &[FixpSgl],
    tns_order: i32,
    tns_prediction_gain: i32,
    tns_active: i32,
    sfb_energy_ld_data: &[FixpDbl],
    noise_nrg: &mut [i32],
) {
    // Reset pns info.
    pns_data.pns_flag = [0; MAX_GROUPED_SFB];
    for sfb in 0..MAX_GROUPED_SFB {
        noise_nrg[sfb] = NO_NOISE_PNS;
    }

    // Disable PNS and skip detection in certain cases.
    if pns_conf.use_pns == 0 {
        return;
    } else {
        // AAC - LC core encoder
        if (pns_conf.np.detection_algorithm_flags & IS_LOW_COMPLEXITY) != 0
            && (last_window_sequence == SHORT_WINDOW)
        {
            return;
        }
        // AAC - (E)LD core encoder
        if (pns_conf.np.detection_algorithm_flags & IS_LOW_COMPLEXITY) == 0
            && (pns_conf.np.detection_algorithm_flags & JUST_LONG_WINDOW) != 0
            && (last_window_sequence != LONG_WINDOW)
        {
            return;
        }
    }

    // call noise detection
    fdk_aac_enc_fdk_aac_enc_noise_detection(
        pns_conf,
        pns_data,
        sfb_active,
        sfb_offset,
        tns_order,
        tns_prediction_gain,
        tns_active,
        mdct_spectrum,
        sfb_max_scale_spec,
        sfbtonality,
    );

    // set startNoiseSfb (long)
    let start_noise_sfb = pns_conf.np.start_sfb as i32;

    // Set noise substitution status
    for sfb in 0..sfb_active.max(0) {
        let s = sfb as usize;
        // No PNS below startNoiseSfb
        if sfb < start_noise_sfb {
            pns_data.set_pns_flag_at(sfb, 0);
            continue;
        }

        // do noise substitution if
        //   fuzzy measure is high enough
        //   sfb freq > minimum sfb freq
        //   signal in coder band is not masked

        if (pns_data.noise_fuzzy_measure[s] > fl2fxconst_sgl(0.5))
            && (sfb_threshold_ld_data[s]
                .wrapping_add(fl2fxconst_dbl((0.5849625f32 / 64.0f32) as f64)) // thr * 1.5 = thrLd +ld(1.5)/64
                < sfb_energy_ld_data[s])
        {
            // mark in psyout flag array that we will code this band with PNS
            pns_data.set_pns_flag_at(sfb, 1); // PNS_ON
        } else {
            pns_data.set_pns_flag_at(sfb, 0); // PNS_OFF
        }

        // no PNS if LTP is active
    }

    // avoid PNS holes
    if (pns_data.noise_fuzzy_measure[0] > fl2fxconst_sgl(0.5)) && (pns_data.pns_flag[1] != 0) {
        pns_data.pns_flag[0] = 1;
    }

    for sfb in 1..max_sfb_per_group - 1 {
        if (pns_data.noise_fuzzy_measure[sfb as usize] > pns_conf.np.gap_fill_thr)
            && (pns_data.pns_flag_at(sfb - 1) != 0)
            && (pns_data.pns_flag_at(sfb + 1) != 0)
        {
            pns_data.set_pns_flag_at(sfb, 1);
        }
    }

    if max_sfb_per_group > 0 {
        // avoid PNS hole
        if (pns_data.noise_fuzzy_measure[(max_sfb_per_group - 1) as usize]
            > pns_conf.np.gap_fill_thr)
            && (pns_data.pns_flag_at(max_sfb_per_group - 2) != 0)
        {
            pns_data.set_pns_flag_at(max_sfb_per_group - 1, 1);
        }
        // avoid single PNS band
        if pns_data.pns_flag_at(max_sfb_per_group - 2) == 0 {
            pns_data.set_pns_flag_at(max_sfb_per_group - 1, 0);
        }
    }

    // avoid single PNS bands
    if pns_data.pns_flag[1] == 0 {
        pns_data.pns_flag[0] = 0;
    }

    for sfb in 1..max_sfb_per_group - 1 {
        if (pns_data.pns_flag_at(sfb - 1) == 0) && (pns_data.pns_flag_at(sfb + 1) == 0) {
            pns_data.set_pns_flag_at(sfb, 0);
        }
    }

    // calculate noiseNrg's
    fdk_aac_enc_calc_noise_nrgs(sfb_active, &pns_data.pns_flag, sfb_energy_ld_data, noise_nrg);
}

/// `FDKaacEnc_FDKaacEnc_noiseDetection(pnsConf, pnsData, sfbActive, sfbOffset, tnsOrder,
/// tnsPredictionGain, tnsActive, mdctSpectrum, sfbMaxScaleSpec, sfbtonality)`: wrapper for
/// noisedet.c.
///
/// Output: `noiseFuzzyMeasure` in `pns_data`.
pub fn fdk_aac_enc_fdk_aac_enc_noise_detection(
    pns_conf: &PnsConfig,
    pns_data: &mut PnsData,
    sfb_active: i32,
    sfb_offset: &[i32],
    tns_order: i32,
    tns_prediction_gain: i32,
    tns_active: i32,
    mdct_spectrum: &[FixpDbl],
    sfb_max_scale_spec: &[i32],
    sfbtonality: &[FixpSgl],
) {
    let mut condition = TRUE != 0;
    if (pns_conf.np.detection_algorithm_flags & IS_LOW_COMPLEXITY) == 0 {
        condition = tns_order > 3;
    }
    // no PNS if heavy TNS activity
    // clear pnsData->noiseFuzzyMeasure
    if (pns_conf.np.detection_algorithm_flags & USE_TNS_GAIN_THR) != 0
        && (tns_prediction_gain >= pns_conf.np.tns_gain_threshold)
        && condition
        && !((pns_conf.np.detection_algorithm_flags & USE_TNS_PNS) != 0
            && (tns_prediction_gain >= pns_conf.np.tns_pns_gain_threshold)
            && (tns_active != 0))
    {
        // clear all noiseFuzzyMeasure
        pns_data.noise_fuzzy_measure[..sfb_active.max(0) as usize].fill(0);
    } else {
        // call noise detection, output in pnsData->noiseFuzzyMeasure,
        // use real mdct spectral data
        fdk_aac_enc_noise_detect(
            mdct_spectrum,
            sfb_max_scale_spec,
            sfb_active,
            sfb_offset,
            &mut pns_data.noise_fuzzy_measure,
            &pns_conf.np,
            sfbtonality,
        );
    }
}

/// `FDKaacEnc_CalcNoiseNrgs(sfbActive, pnsFlag, sfbEnergyLdData, noiseNrg)`: calculate the
/// NoiseNrg's (in the bands with `pnsFlag` set).
pub fn fdk_aac_enc_calc_noise_nrgs(
    sfb_active: i32,
    pns_flag: &[i32],
    sfb_energy_ld_data: &[FixpDbl],
    noise_nrg: &mut [i32],
) {
    let tmp = (-LOG_NORM_PCM) << 2;

    for sfb in 0..sfb_active.max(0) as usize {
        if pns_flag[sfb] != 0 {
            let nrg = sfb_energy_ld_data[sfb]
                .wrapping_neg()
                .wrapping_add(fl2fxconst_dbl((0.5f32 / 64.0f32) as f64))
                >> (DFRACT_BITS - 1 - 7);
            noise_nrg[sfb] = tmp - nrg;
        }
    }
}

/// `FDKaacEnc_CodePnsChannel(sfbActive, pnsConf, pnsFlag, sfbEnergyLdData, noiseNrg,
/// sfbThresholdLdData)`: execute pns decission.
///
/// Sets `sfbThresholdLdData` high to code pe with 0; `noiseNrg` marks the bands for PNS coding.
pub fn fdk_aac_enc_code_pns_channel(
    sfb_active: i32,
    pns_conf: &PnsConfig,
    pns_flag: &[i32],
    sfb_energy_ld_data: &[FixpDbl],
    noise_nrg: &mut [i32],
    sfb_threshold_ld_data: &mut [FixpDbl],
) {
    let mut lasti_noise_energy: i32 = 0;
    let mut first_pns_band = 1; // TRUE for first PNS-coded band

    // no PNS
    if pns_conf.use_pns == 0 {
        for sfb in 0..sfb_active.max(0) as usize {
            // no PNS coding
            noise_nrg[sfb] = NO_NOISE_PNS;
        }
        return;
    }

    // code PNS
    for sfb in 0..sfb_active.max(0) as usize {
        if pns_flag[sfb] != 0 {
            // high sfbThreshold causes pe = 0
            if noise_nrg[sfb] != NO_NOISE_PNS {
                sfb_threshold_ld_data[sfb] = sfb_energy_ld_data[sfb]
                    .wrapping_add(fl2fxconst_dbl((1.0f32 / LD_DATA_SCALING) as f64));
            }

            // set noiseNrg in valid region
            if first_pns_band == 0 {
                let deltai_noise_energy = noise_nrg[sfb].wrapping_sub(lasti_noise_energy);

                if deltai_noise_energy > CODE_BOOK_PNS_LAV {
                    noise_nrg[sfb] = noise_nrg[sfb]
                        .wrapping_sub(deltai_noise_energy.wrapping_sub(CODE_BOOK_PNS_LAV));
                } else if deltai_noise_energy < -CODE_BOOK_PNS_LAV {
                    noise_nrg[sfb] = noise_nrg[sfb]
                        .wrapping_sub(deltai_noise_energy.wrapping_add(CODE_BOOK_PNS_LAV));
                }
            } else {
                first_pns_band = 0;
            }
            lasti_noise_energy = noise_nrg[sfb];
        } else {
            // no PNS coding
            noise_nrg[sfb] = NO_NOISE_PNS;
        }
    }
}

/// `CalcLdData(fixp_abs(MINVAL_DBL))` as compiled into `FDKaacEnc_PreProcessPnsChannelPair` by
/// all reference builds: `fLog2` without its `x_m <= 0` short cut, i.e. with `0x80000000` taken
/// as a positive number. The normalization shift `fNormz(x_m) - 1` is -1, which `lsl`/`shl`
/// take modulo 32 (ref-x86_64: `shl %cl` at 0x680).
fn calc_ld_data_of_min() -> FixpDbl {
    let x_m = MINVAL_DBL;
    let b_norm = fixnormz_d(x_m) - 1;
    let mut x2_m = x_m.wrapping_shl(b_norm as u32);
    let x_e = -b_norm;
    // map x from log(x) domain to log(1-x) domain.
    x2_m = x2_m.wrapping_add(fl2fxconst_dbl(-1.0)).wrapping_neg();
    // Taylor polynomial approximation of ln(1-x)
    let mut result_m = fl2fxconst_dbl(0.0);
    let mut px2_m = x2_m;
    for &coeff in LD_COEFF.iter().take(LD_PRECISION) {
        result_m = f_mult_add_div2(result_m, coeff, px2_m);
        px2_m = f_mult(px2_m, x2_m);
    }
    // Multiply result with 1/ln(2) = 1.0 + 0.442695040888 (get log2(x) from ln(x) result).
    result_m = f_mult_add_div2(
        result_m,
        result_m,
        fl2fxconst_dbl(2.0 * 0.4426950408889634073599246810019),
    );
    // Add exponent part (x_e == 1).
    let enorm = DFRACT_BITS - fixnorm_d(x_e);
    result_m = (result_m >> (enorm - 1)).wrapping_add(x_e << (DFRACT_BITS - 1 - enorm));
    scale_value(result_m, enorm - LD_DATA_SHIFT)
}

/// `FDKaacEnc_PreProcessPnsChannelPair(sfbActive, sfbEnergyLeft, sfbEnergyRight,
/// sfbEnergyLeftLD, sfbEnergyRightLD, sfbEnergyMid, pnsConfLeft, pnsDataLeft, pnsDataRight)`:
/// calculate the correlation of noise in a channel pair.
///
/// Output: `noiseEnergyCorrelation` in both `PNS_DATA`.
pub fn fdk_aac_enc_pre_process_pns_channel_pair(
    sfb_active: i32,
    sfb_energy_left: &[FixpDbl],
    sfb_energy_right: &[FixpDbl],
    sfb_energy_left_ld: &[FixpDbl],
    sfb_energy_right_ld: &[FixpDbl],
    sfb_energy_mid: &[FixpDbl],
    pns_conf: &PnsConfig,
    pns_data_left: &mut PnsData,
    pns_data_right: &mut PnsData,
) {
    if pns_conf.use_pns == 0 {
        return;
    }

    for sfb in 0..sfb_active.max(0) as usize {
        let quot = (sfb_energy_left_ld[sfb] >> 1) + (sfb_energy_right_ld[sfb] >> 1);

        let ccf = if quot < fl2fxconst_dbl((-32.0f32 / LD_DATA_SCALING) as f64) {
            fl2fxconst_dbl(0.0)
        } else {
            let mut accu = sfb_energy_mid[sfb]
                .wrapping_sub(((sfb_energy_left[sfb] >> 1) + (sfb_energy_right[sfb] >> 1)) >> 1);
            let sign = accu < fl2fxconst_dbl(0.0);
            accu = fixp_abs(accu);

            // ld(accu*2) = ld(accu) + 1
            let ld = if accu == MINVAL_DBL {
                // fixp_abs(MINVAL_DBL) overflows; all reference builds assume that the result
                // is positive (they test only accu == 0 before CalcLdData) and evaluate the
                // logarithm of 0x80000000 as a positive number (see calc_ld_data_of_min).
                calc_ld_data_of_min()
            } else {
                calc_ld_data(accu)
            };
            let ccf = ld
                .wrapping_add(fl2fxconst_dbl((1.0f32 / LD_DATA_SCALING) as f64))
                .wrapping_sub(quot);
            // With accu == 0, ccf = -1.0 + 1/64 - quot, which is negative unless the subtraction
            // overflows (quot > 1/64, i.e. an energy above 1.0; the encoder's LD energies are
            // not positive). ref-arm64, ref-x86_64 and ref-i686 test the sign of the wrapped
            // value like the source (ref-x86_64: `sub`, `add $0x2000000`, `jns` at
            // 0x7a5-0x7b7, also for accu == 0). Apple clang for x86-64 (ref-x86_64-mac) assumes
            // that it does not overflow and computes CalcInvLdData(0x82000000 - quot) for
            // accu == 0, which gives 0 for every positive ccf; the port does not reproduce that
            // build.
            if ccf >= fl2fxconst_dbl(0.0) {
                MAXVAL_DBL
            } else if sign {
                calc_inv_ld_data(ccf).wrapping_neg()
            } else {
                calc_inv_ld_data(ccf)
            }
        };

        pns_data_left.noise_energy_correlation[sfb] = ccf;
        pns_data_right.noise_energy_correlation[sfb] = ccf;
    }
}

/// `FDKaacEnc_PostProcessPnsChannelPair(sfbActive, pnsConf, pnsDataLeft, pnsDataRight, msMask,
/// msDigest)`: if PNS used at left and right channel, use msMask to flag correlation.
///
/// Output: `pnsFlag` in both `PNS_DATA`, `msMask` (flags correlation), `msDigest`.
pub fn fdk_aac_enc_post_process_pns_channel_pair(
    sfb_active: i32,
    pns_conf: &PnsConfig,
    pns_data_left: &mut PnsData,
    pns_data_right: &mut PnsData,
    ms_mask: &mut [i32],
    ms_digest: &mut i32,
) {
    if pns_conf.use_pns == 0 {
        return;
    }

    for sfb in 0..sfb_active.max(0) as usize {
        // MS post processing
        if ms_mask[sfb] != 0 {
            if (pns_data_left.pns_flag[sfb] != 0) && (pns_data_right.pns_flag[sfb] != 0) {
                // AAC only: Standard
                // do this to avoid ms flags in layers that should not have it
                if pns_data_left.noise_energy_correlation[sfb] <= pns_conf.noise_correlation_thresh
                {
                    ms_mask[sfb] = 0;
                    *ms_digest = MS_SOME;
                }
            } else {
                // No PNS coding
                pns_data_left.pns_flag[sfb] = 0;
                pns_data_right.pns_flag[sfb] = 0;
            }
        }

        // Use MS flag to signal noise correlation if pns is active in both channels
        if (pns_data_left.pns_flag[sfb] != 0) && (pns_data_right.pns_flag[sfb] != 0) {
            if pns_data_left.noise_energy_correlation[sfb] > pns_conf.noise_correlation_thresh {
                ms_mask[sfb] = 1;
                *ms_digest = MS_SOME;
            }
        }
    }
}
